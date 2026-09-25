//! The `Data/indices` set the Battle.net Agent keeps next to a CASC install:
//! the `.index` of every CDN config `archives` and `patch-archives` entry,
//! plus `archive-group`, `patch-archive-group`, `file-index` and
//! `patch-file-index` (the first key of each variable), all named
//! `<key>.index`. Observed on a real Agent install (76/76 patch indices, the
//! four extra files present); the 3.4.3 client reads the patch set at start
//! ("Constructing the patch group CDN index") and otherwise tries the CDN,
//! where Blizzard no longer serves them.
//!
//! CDN paths: `data/xx/yy/<key>.index` for archives and `file-index`,
//! `patch/xx/yy/<key>.index` for patch archives and `patch-file-index`. The
//! two groups exist on neither Blizzard's CDN nor the mirror and are built
//! locally ([`cdn_index::build_group`]); a built group is only kept when its
//! footer hashes to the configured key.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;
use std::thread;

use anyhow::{Context, Result, bail};

use crate::casc::write_atomic;
use crate::cdn_index::{self, IndexEntry};
use crate::config::ConfigFile;
use crate::progress::Progress;
use crate::remote::Remote;
use crate::util::{Key, hex, parse_key};

/// CDN directory an index is served from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdnKind {
    Data,
    Patch,
}

impl CdnKind {
    pub fn dir(self) -> &'static str {
        match self {
            Self::Data => "data",
            Self::Patch => "patch",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexSet {
    pub archives: Vec<Key>,
    pub patch_archives: Vec<Key>,
    pub file_index: Option<Key>,
    pub patch_file_index: Option<Key>,
    pub archive_group: Option<Key>,
    pub patch_archive_group: Option<Key>,
}

fn keys(cfg: &ConfigFile, name: &str) -> Result<Vec<Key>> {
    cfg.values(name)
        .unwrap_or_default()
        .iter()
        .map(|k| parse_key(k).with_context(|| format!("CDN config `{name}`: bad key {k}")))
        .collect()
}

impl IndexSet {
    pub fn from_cdn_config(cfg: &ConfigFile) -> Result<Self> {
        let set = Self {
            archives: keys(cfg, "archives")?,
            patch_archives: keys(cfg, "patch-archives")?,
            file_index: cfg.key("file-index", 0),
            patch_file_index: cfg.key("patch-file-index", 0),
            archive_group: cfg.key("archive-group", 0),
            patch_archive_group: cfg.key("patch-archive-group", 0),
        };
        if set.archives.is_empty() {
            bail!("CDN config lacks `archives`");
        }
        Ok(set)
    }

    /// Indices fetched from the CDN, deduplicated, in fetch order.
    pub fn downloads(&self) -> Vec<(Key, CdnKind)> {
        let mut seen = HashSet::new();
        self.archives
            .iter()
            .map(|k| (*k, CdnKind::Data))
            .chain(self.file_index.map(|k| (k, CdnKind::Data)))
            .chain(self.patch_archives.iter().map(|k| (*k, CdnKind::Patch)))
            .chain(self.patch_file_index.map(|k| (k, CdnKind::Patch)))
            .filter(|(k, _)| seen.insert(*k))
            .collect()
    }

    /// Every `Data/indices` file name of the set.
    pub fn file_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .downloads()
            .iter()
            .map(|(k, _)| k)
            .chain(self.archive_group.iter())
            .chain(self.patch_archive_group.iter())
            .map(|k| format!("{}.index", hex(k)))
            .collect();
        let mut seen = HashSet::new();
        names.retain(|n| seen.insert(n.clone()));
        names
    }
}

/// Counts of a completed [`ensure`] run.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    pub archives: usize,
    pub patch_archives: usize,
    pub file_indices: usize,
    pub groups: usize,
    /// Files that were missing or invalid and have been written now.
    pub written: usize,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} archive, {} patch-archive, {} file-index and {} group indices ({} newly written)",
            self.archives, self.patch_archives, self.file_indices, self.groups, self.written
        )
    }
}

/// Valid entries of `<dir>/<key>.index`, if present and intact.
fn read_local(dir: &Path, key: &Key) -> Option<Vec<IndexEntry>> {
    let data = std::fs::read(dir.join(format!("{}.index", hex(key)))).ok()?;
    cdn_index::parse(&data, Some(key)).ok()
}

/// Makes `dir` hold every index of `set`, fetching only missing or invalid
/// files, then building missing groups.
pub fn ensure(remote: &Remote, dir: &Path, set: &IndexSet, threads: usize) -> Result<Report> {
    std::fs::create_dir_all(dir)?;
    let downloads = set.downloads();
    let missing: Vec<(Key, CdnKind)> = downloads
        .iter()
        .filter(|(k, _)| read_local(dir, k).is_none())
        .copied()
        .collect();
    let mut report = Report::default();
    if !missing.is_empty() {
        println!("Fetching {} missing CDN indices", missing.len());
        fetch_all(remote, dir, &missing, threads)?;
        report.written += missing.len();
    }
    report.archives = set.archives.len();
    report.patch_archives = set.patch_archives.len();
    report.file_indices =
        usize::from(set.file_index.is_some()) + usize::from(set.patch_file_index.is_some());

    for (group, members, label) in [
        (set.archive_group, &set.archives, "archive-group"),
        (
            set.patch_archive_group,
            &set.patch_archives,
            "patch-archive-group",
        ),
    ] {
        let Some(group) = group else { continue };
        if read_local(dir, &group).is_some() {
            report.groups += 1;
            continue;
        }
        let lists = members
            .iter()
            .map(|k| read_local(dir, k).with_context(|| format!("{label}: {} missing", hex(k))))
            .collect::<Result<Vec<_>>>()?;
        let (data, name) = cdn_index::build_group(&lists);
        if name != group {
            bail!(
                "{label}: rebuilt index hashes to {}, CDN config says {}",
                hex(&name),
                hex(&group)
            );
        }
        write_atomic(&dir.join(format!("{}.index", hex(&group))), &data)?;
        println!("Built {label} {} ({} bytes)", hex(&group), data.len());
        report.groups += 1;
        report.written += 1;
    }
    if let Some(absent) = set.file_names().iter().find(|n| !dir.join(n).is_file()) {
        bail!("{absent} is still missing from {}", dir.display());
    }
    Ok(report)
}

fn fetch_all(remote: &Remote, dir: &Path, list: &[(Key, CdnKind)], threads: usize) -> Result<()> {
    let next = Mutex::new(0usize);
    let errors = Mutex::new(Vec::new());
    let progress = Mutex::new(Progress::new("CDN indices", list.len(), 0));
    thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = {
                        let mut n = next.lock().expect("lock");
                        *n += 1;
                        *n - 1
                    };
                    let Some((key, kind)) = list.get(i) else {
                        break;
                    };
                    let result = remote.index(kind.dir(), key).and_then(|data| {
                        cdn_index::parse(&data, Some(key))?;
                        write_atomic(&dir.join(format!("{}.index", hex(key))), &data)
                    });
                    if let Err(e) = result {
                        errors.lock().expect("lock").push(format!(
                            "{}/{}: {e:#}",
                            kind.dir(),
                            hex(key)
                        ));
                    }
                    progress.lock().expect("lock").add(1, 0);
                }
            });
        }
    });
    progress.into_inner().expect("lock").finish();
    let errors = errors.into_inner().expect("lock");
    if let Some(first) = errors.first() {
        bail!("{} indices failed, first: {first}", errors.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(b: u8) -> String {
        hex(&[b; 16])
    }

    #[test]
    fn selection_matches_the_agent_set() {
        let text = format!(
            "# CDN Configuration\narchives = {} {}\narchives-index-size = 1 2\narchive-group = {}\npatch-archives = {} {}\npatch-archives-index-size = 3 4\npatch-archive-group = {}\nfile-index = {}\nfile-index-size = 5\npatch-file-index = {}\npatch-file-index-size = 6\n",
            k(1),
            k(2),
            k(3),
            k(4),
            k(5),
            k(6),
            k(7),
            k(8)
        );
        let set = IndexSet::from_cdn_config(&ConfigFile::parse(text.as_bytes()).unwrap()).unwrap();
        let dl: Vec<(u8, CdnKind)> = set.downloads().iter().map(|(k, c)| (k[0], *c)).collect();
        assert_eq!(
            dl,
            [
                (1, CdnKind::Data),
                (2, CdnKind::Data),
                (7, CdnKind::Data),
                (4, CdnKind::Patch),
                (5, CdnKind::Patch),
                (8, CdnKind::Patch),
            ]
        );
        assert_eq!(set.archive_group, Some([3; 16]));
        assert_eq!(set.patch_archive_group, Some([6; 16]));
        let names = set.file_names();
        assert_eq!(names.len(), 8);
        assert!(names.contains(&format!("{}.index", k(3))));
        assert_eq!(CdnKind::Patch.dir(), "patch");
    }

    #[test]
    fn optional_fields_and_errors() {
        let only = format!("archives = {}\n", k(1));
        let set = IndexSet::from_cdn_config(&ConfigFile::parse(only.as_bytes()).unwrap()).unwrap();
        assert_eq!(set.downloads().len(), 1);
        assert!(set.patch_archives.is_empty() && set.archive_group.is_none());
        assert!(
            IndexSet::from_cdn_config(&ConfigFile::parse(b"file-index = 00").unwrap()).is_err()
        );
        let bad = format!("archives = {} zz\n", k(1));
        assert!(IndexSet::from_cdn_config(&ConfigFile::parse(bad.as_bytes()).unwrap()).is_err());
    }

    #[test]
    fn local_groups_are_built_and_verified() {
        let dir = tempfile::tempdir().unwrap();
        let entries = |base: u8| -> Vec<IndexEntry> {
            (0..3u8)
                .map(|i| IndexEntry {
                    ekey: [base + i; 16],
                    size: 10,
                    offset: u32::from(i) * 10,
                    archive: 0,
                })
                .collect()
        };
        let (a, ka) = cdn_index::write(&entries(10), 4);
        let (b, kb) = cdn_index::write(&entries(20), 4);
        std::fs::write(dir.path().join(format!("{}.index", hex(&ka))), &a).unwrap();
        std::fs::write(dir.path().join(format!("{}.index", hex(&kb))), &b).unwrap();
        assert_eq!(read_local(dir.path(), &ka).unwrap().len(), 3);
        assert!(read_local(dir.path(), &[9; 16]).is_none());
        // What `ensure` builds for a group of [a, b].
        let (_, group) = cdn_index::build_group(&[entries(10), entries(20)]);
        let lists = [ka, kb].map(|k| read_local(dir.path(), &k).unwrap());
        assert_eq!(cdn_index::build_group(&lists).1, group);
    }
}
