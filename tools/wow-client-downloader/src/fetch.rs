//! Locating and fetching encoded files: archive indices -> coalesced HTTP
//! range jobs -> verified blobs handed to the storage writer.
//!
//! blizzget `DownloadTask::loadIndices`/`loadFile` read every archive index
//! of the CDN config into one `EKey` -> `{archive, offset, size}` map and fall
//! back to the loose `data/xx/yy/<ekey>` path for keys in no archive; it
//! fetches 1 MiB-aligned blocks per file. Here, adjacent entries of one
//! archive are merged into one range request instead (gaps up to 64 KiB,
//! spans up to 8 MiB) to keep the number of requests to the CDN and the
//! community mirror low. Every blob is verified against its `EKey` before it
//! is stored.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use crate::blte;
use crate::casc::{LocalStorage, write_atomic};
use crate::cdn_index;
use crate::plan::Needed;
use crate::progress::Progress;
use crate::remote::Remote;
use crate::util::{Key, hex};

const MAX_GAP: u64 = 64 * 1024;
const MAX_SPAN: u64 = 8 * 1024 * 1024;
const JOB_ATTEMPTS: u32 = 3;

/// An archive entry to fetch: `(EKey, offset, size)`.
type Entry = (Key, u64, u64);
/// Verified blobs of a job.
type Blobs = Vec<(Key, Vec<u8>)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveLocation {
    pub archive: u16,
    pub offset: u32,
    pub size: u32,
}

/// Downloads (or reuses `Data/indices/<archive>.index`) every archive index
/// of the CDN config and returns the archive keys and the locations of the
/// `wanted` keys.
pub fn load_indices(
    remote: &Remote,
    storage: &LocalStorage,
    archives: &[Key],
    wanted: &HashSet<Key>,
    threads: usize,
) -> Result<HashMap<Key, ArchiveLocation>> {
    let next = Mutex::new(0usize);
    let found = Mutex::new(HashMap::new());
    let progress = Mutex::new(Progress::new("archive indices", archives.len(), 0));
    let errors = Mutex::new(Vec::new());
    thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = {
                        let mut n = next.lock().expect("lock");
                        let i = *n;
                        *n += 1;
                        i
                    };
                    let Some(archive) = archives.get(i) else {
                        break;
                    };
                    match load_one(remote, storage, archive) {
                        Ok(entries) => {
                            let mut f = found.lock().expect("lock");
                            for e in entries.iter().filter(|e| wanted.contains(&e.ekey)) {
                                f.entry(e.ekey).or_insert(ArchiveLocation {
                                    archive: i as u16,
                                    offset: e.offset,
                                    size: e.size,
                                });
                            }
                        }
                        Err(e) => errors
                            .lock()
                            .expect("lock")
                            .push(format!("{}: {e:#}", hex(archive))),
                    }
                    progress.lock().expect("lock").add(1, 0);
                }
            });
        }
    });
    progress.into_inner().expect("lock").finish();
    let errors = errors.into_inner().expect("lock");
    if !errors.is_empty() {
        return Err(anyhow!(
            "{} archive indices failed, first: {}",
            errors.len(),
            errors[0]
        ));
    }
    Ok(found.into_inner().expect("lock"))
}

fn load_one(
    remote: &Remote,
    storage: &LocalStorage,
    archive: &Key,
) -> Result<Vec<cdn_index::IndexEntry>> {
    let path = storage.index_path(&hex(archive));
    if let Ok(data) = std::fs::read(&path)
        && let Ok(entries) = cdn_index::parse(&data, Some(archive))
    {
        return Ok(entries);
    }
    let data = remote.index("data", archive)?;
    let entries = cdn_index::parse(&data, Some(archive))?;
    write_atomic(&path, &data)?;
    Ok(entries)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// Bytes `start..end` of archive `archive`, holding `entries`
    /// (`EKey`, offset, size).
    Range {
        archive: u16,
        start: u64,
        end: u64,
        entries: Vec<Entry>,
    },
    Loose {
        ekey: Key,
        size: u64,
    },
}

impl Job {
    fn files(&self) -> Vec<(Key, u64)> {
        match self {
            Job::Range { entries, .. } => entries.iter().map(|(k, _, s)| (*k, *s)).collect(),
            Job::Loose { ekey, size } => vec![(*ekey, *size)],
        }
    }
}

/// Groups `needed` keys into range jobs per archive (in first-needed order)
/// and loose jobs.
pub fn make_jobs(needed: &[Needed], locations: &HashMap<Key, ArchiveLocation>) -> Vec<Job> {
    let mut per_archive: Vec<(u16, Vec<Entry>)> = Vec::new();
    let mut archive_slot: HashMap<u16, usize> = HashMap::new();
    let mut loose = Vec::new();
    for n in needed {
        match locations.get(&n.ekey) {
            Some(loc) => {
                let slot = *archive_slot.entry(loc.archive).or_insert_with(|| {
                    per_archive.push((loc.archive, Vec::new()));
                    per_archive.len() - 1
                });
                per_archive[slot]
                    .1
                    .push((n.ekey, u64::from(loc.offset), u64::from(loc.size)));
            }
            None => loose.push(Job::Loose {
                ekey: n.ekey,
                size: n.size,
            }),
        }
    }
    let mut jobs = Vec::new();
    for (archive, mut entries) in per_archive {
        entries.sort_by_key(|e| e.1);
        let mut current: Option<Job> = None;
        for (ekey, offset, size) in entries {
            if let Some(Job::Range {
                start,
                end,
                entries,
                ..
            }) = &mut current
                && offset >= *end
                && offset - *end <= MAX_GAP
                && offset + size - *start <= MAX_SPAN
            {
                *end = offset + size;
                entries.push((ekey, offset, size));
                continue;
            }
            jobs.extend(current.take());
            current = Some(Job::Range {
                archive,
                start: offset,
                end: offset + size,
                entries: vec![(ekey, offset, size)],
            });
        }
        jobs.extend(current);
    }
    jobs.extend(loose);
    jobs
}

fn run_job(remote: &Remote, archives: &[Key], job: &Job) -> Result<Blobs> {
    match job {
        Job::Range {
            archive,
            start,
            end,
            entries,
        } => {
            let key = &archives[*archive as usize];
            let data = remote.archive_range(key, *start, end - 1)?;
            entries
                .iter()
                .map(|(ekey, offset, size)| {
                    let at = (offset - start) as usize;
                    let blob = &data[at..at + *size as usize];
                    blte::verify(blob, ekey)
                        .with_context(|| format!("{} in archive {}", hex(ekey), hex(key)))?;
                    Ok((*ekey, blob.to_vec()))
                })
                .collect()
        }
        Job::Loose { ekey, .. } => {
            let blob = remote
                .loose(ekey)?
                .with_context(|| format!("{} is in no archive and not a loose file", hex(ekey)))?;
            Ok(vec![(*ekey, blob)])
        }
    }
}

/// Runs `jobs` on `threads` workers and stores every verified blob. Returns
/// the keys that could not be fetched, with the reason.
pub fn run_jobs(
    remote: &Remote,
    archives: &[Key],
    jobs: Vec<Job>,
    storage: &mut LocalStorage,
    threads: usize,
    progress: &mut Progress,
) -> Result<Vec<(Key, String)>> {
    let queue = Mutex::new(VecDeque::from(jobs));
    let (tx, rx) = mpsc::sync_channel::<(Job, Result<Blobs>)>(threads * 2);
    let mut failures = Vec::new();
    let mut write_error = None;
    thread::scope(|s| {
        for _ in 0..threads {
            let tx = tx.clone();
            let queue = &queue;
            s.spawn(move || {
                loop {
                    let Some(job) = queue.lock().expect("lock").pop_front() else {
                        break;
                    };
                    let mut result = run_job(remote, archives, &job);
                    for attempt in 1..JOB_ATTEMPTS {
                        if result.is_ok() {
                            break;
                        }
                        thread::sleep(Duration::from_secs(u64::from(attempt) * 2));
                        result = run_job(remote, archives, &job);
                    }
                    if tx.send((job, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for (job, result) in rx {
            match result {
                Ok(blobs) => {
                    for (ekey, blob) in blobs {
                        if write_error.is_none()
                            && let Err(e) = storage.write(&ekey, &blob)
                        {
                            write_error = Some(e);
                            queue.lock().expect("lock").clear();
                        }
                        progress.add(1, blob.len() as u64);
                    }
                }
                Err(e) => {
                    let reason = format!("{e:#}");
                    for (ekey, size) in job.files() {
                        failures.push((ekey, reason.clone()));
                        progress.add(1, size);
                    }
                }
            }
        }
    });
    progress.finish();
    storage.sync()?;
    if let Some(e) = write_error {
        return Err(e.context("writing the local storage"));
    }
    Ok(failures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Kind;

    fn needed(k: u8) -> Needed {
        Needed {
            ekey: [k; 16],
            size: 10,
            kind: Kind::Download,
        }
    }

    #[test]
    fn coalesces_adjacent_ranges() {
        let loc = |archive, offset, size| ArchiveLocation {
            archive,
            offset,
            size,
        };
        let mut locations = HashMap::new();
        locations.insert([1; 16], loc(3, 1000, 100));
        locations.insert([2; 16], loc(3, 0, 1000)); // adjacent, sorted before [1]
        locations.insert([3; 16], loc(3, 1100 + 70_000, 10)); // gap > 64 KiB
        locations.insert([4; 16], loc(0, 5, 5));
        let n: Vec<Needed> = [1, 2, 3, 4, 5].map(needed).to_vec();
        let jobs = make_jobs(&n, &locations);
        assert_eq!(jobs.len(), 4);
        assert_eq!(
            jobs[0],
            Job::Range {
                archive: 3,
                start: 0,
                end: 1100,
                entries: vec![([2; 16], 0, 1000), ([1; 16], 1000, 100)],
            }
        );
        assert!(matches!(&jobs[1], Job::Range { start: 71_100, .. }));
        assert!(matches!(&jobs[2], Job::Range { archive: 0, .. }));
        assert_eq!(
            jobs[3],
            Job::Loose {
                ekey: [5; 16],
                size: 10
            }
        );
    }

    #[test]
    fn span_limit_splits() {
        let mut locations = HashMap::new();
        for i in 0..20u8 {
            locations.insert(
                [i; 16],
                ArchiveLocation {
                    archive: 0,
                    offset: u32::from(i) * 1_000_000,
                    size: 1_000_000,
                },
            );
        }
        let n: Vec<Needed> = (0..20).map(needed).collect();
        let jobs = make_jobs(&n, &locations);
        assert_eq!(jobs.len(), 3, "20 MB in spans of at most 8 MiB");
        for j in &jobs {
            let Job::Range { start, end, .. } = j else {
                panic!()
            };
            assert!(end - start <= MAX_SPAN);
        }
    }
}
