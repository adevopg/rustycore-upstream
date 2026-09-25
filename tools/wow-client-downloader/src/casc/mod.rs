//! Local CASC storage writer: `Data/data/data.###` archives, the 16 bucket
//! `.idx` files, `Data/config`, `Data/indices` and `.build.info`.
//!
//! Entries are appended to the newest `data.###` (a new archive is started
//! before one would exceed 1 GiB, the Agent's and blizzget's
//! `MaxDataSize = 0x40000000`, so every offset fits the 30 offset bits).
//! Each entry is the 0x1E-byte encoded header ([`header`]) followed by the
//! BLTE blob exactly as the CDN serves it.
//!
//! Resume: every stored entry is appended to `Data/data/.wcd-journal` (fixed
//! 26-byte records `{EKey, u16 archive, u32 offset, u32 size}` after an 8-byte
//! magic). On open, archives are truncated to the end of their last
//! journaled entry, which drops a half-written tail after a crash; the `.idx`
//! files are rebuilt from the journal by [`LocalStorage::write_indices`].
//! A `Data/data` holding archives without a journal (an Agent install) is
//! refused rather than modified.

pub mod build_info;
pub mod header;
pub mod idx;

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::util::{Key, hex};
use header::HEADER_SIZE;

const JOURNAL: &str = ".wcd-journal";
const JOURNAL_MAGIC: &[u8; 8] = b"WCDJRNL1";
const RECORD: usize = 26;
/// blizzget `DataStorage::MaxDataSize`.
pub const MAX_ARCHIVE_SIZE: u64 = 0x4000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub archive: u16,
    pub offset: u32,
    /// Encoded size including the 0x1E-byte header.
    pub size: u32,
}

impl Location {
    fn storage_offset(self) -> u64 {
        (u64::from(self.archive) << idx::OFFSET_BITS) | u64::from(self.offset)
    }
}

pub struct LocalStorage {
    root: PathBuf,
    data_dir: PathBuf,
    entries: HashMap<Key, Location>,
    journal: File,
    /// Newest archive and its current length.
    current: Option<(u16, File, u64)>,
    max_archive_size: u64,
    /// Opened by [`LocalStorage::open_index_only`]: archives and journal are
    /// never modified.
    read_only: bool,
}

impl LocalStorage {
    /// Opens (creating if needed) the storage under `root` (the directory that
    /// holds `.build.info` and `Data/`).
    pub fn open(root: &Path) -> Result<Self> {
        Self::open_with_limit(root, MAX_ARCHIVE_SIZE)
    }

    pub fn open_with_limit(root: &Path, max_archive_size: u64) -> Result<Self> {
        let data_dir = root.join("Data").join("data");
        for dir in [
            &data_dir,
            &root.join("Data/config"),
            &root.join("Data/indices"),
        ] {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let journal_path = data_dir.join(JOURNAL);
        let archives = list_archives(&data_dir)?;
        if !journal_path.exists() && !archives.is_empty() {
            bail!(
                "{} already holds data archives not written by this tool; use an empty --output",
                data_dir.display()
            );
        }
        let mut journal = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&journal_path)?;
        let mut raw = Vec::new();
        journal.read_to_end(&mut raw)?;
        if raw.is_empty() {
            journal.write_all(JOURNAL_MAGIC)?;
        } else {
            ensure!(raw.starts_with(JOURNAL_MAGIC), "{JOURNAL} has a bad magic");
        }

        let (entries, ends, dropped) = replay(&raw, &data_dir);
        // Truncate half-written tails; remove archives without entries.
        for a in archives {
            let path = data_dir.join(archive_name(a));
            match ends.get(&a) {
                Some(&end) => OpenOptions::new().write(true).open(&path)?.set_len(end)?,
                None => fs::remove_file(&path)?,
            }
        }
        let mut storage = Self {
            root: root.to_path_buf(),
            data_dir,
            entries,
            journal,
            current: None,
            max_archive_size,
            read_only: false,
        };
        if dropped {
            storage.rewrite_journal()?;
        }
        if let Some(&newest) = ends.keys().max() {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(storage.data_dir.join(archive_name(newest)))?;
            storage.current = Some((newest, file, ends[&newest]));
        }
        Ok(storage)
    }

    /// Opens an existing storage written by this tool without modifying its
    /// archives or journal (no tail truncation, no appends): for rewriting
    /// the `.idx` files and `Data/indices` only. Journal records whose bytes
    /// are not on disk are left out of the indices.
    pub fn open_index_only(root: &Path) -> Result<Self> {
        let data_dir = root.join("Data").join("data");
        let journal_path = data_dir.join(JOURNAL);
        if !journal_path.is_file() {
            bail!(
                "{} has no {JOURNAL}: not a storage written by this tool",
                data_dir.display()
            );
        }
        let raw = fs::read(&journal_path)?;
        ensure!(raw.starts_with(JOURNAL_MAGIC), "{JOURNAL} has a bad magic");
        let (entries, _, dropped) = replay(&raw, &data_dir);
        if dropped {
            println!("warning: some journal records are not on disk and are not indexed");
        }
        Ok(Self {
            root: root.to_path_buf(),
            journal: File::open(&journal_path)?,
            data_dir,
            entries,
            current: None,
            max_archive_size: MAX_ARCHIVE_SIZE,
            read_only: true,
        })
    }

    fn rewrite_journal(&mut self) -> Result<()> {
        let mut records: Vec<_> = self.entries.iter().collect();
        records.sort_by_key(|(_, l)| (l.archive, l.offset));
        let mut data = JOURNAL_MAGIC.to_vec();
        for (k, l) in records {
            data.extend_from_slice(&record(k, *l));
        }
        let path = self.data_dir.join(JOURNAL);
        fs::write(&path, data)?;
        self.journal = OpenOptions::new().read(true).append(true).open(&path)?;
        Ok(())
    }

    pub fn contains(&self, ekey: &Key) -> bool {
        self.entries.contains_key(ekey)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn location(&self, ekey: &Key) -> Option<Location> {
        self.entries.get(ekey).copied()
    }

    /// Appends one verified BLTE blob. Already stored keys are skipped.
    pub fn write(&mut self, ekey: &Key, blte: &[u8]) -> Result<()> {
        if self.contains(ekey) {
            return Ok(());
        }
        ensure!(!self.read_only, "storage opened for index repair only");
        let total = (blte.len() + HEADER_SIZE) as u64;
        ensure!(
            total <= self.max_archive_size,
            "entry larger than an archive"
        );
        let needs_new = self
            .current
            .as_ref()
            .is_none_or(|(_, _, len)| len + total > self.max_archive_size);
        if needs_new {
            let next = self.current.as_ref().map_or(0, |(a, _, _)| a + 1);
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(self.data_dir.join(archive_name(next)))?;
            self.current = Some((next, file, 0));
        }
        let (archive, file, len) = self.current.as_mut().expect("set above");
        let loc = Location {
            archive: *archive,
            offset: *len as u32,
            size: total as u32,
        };
        let head = header::encoded_header(ekey, blte.len() as u32, loc.storage_offset());
        file.seek(SeekFrom::Start(*len))?;
        file.write_all(&head)?;
        file.write_all(blte)?;
        *len += total;
        self.journal.write_all(&record(ekey, loc))?;
        self.entries.insert(*ekey, loc);
        Ok(())
    }

    /// The BLTE blob of a stored entry (without its encoded header).
    pub fn read(&self, ekey: &Key) -> Result<Option<Vec<u8>>> {
        let Some(loc) = self.location(ekey) else {
            return Ok(None);
        };
        let mut file = File::open(self.data_dir.join(archive_name(loc.archive)))?;
        file.seek(SeekFrom::Start(u64::from(loc.offset) + HEADER_SIZE as u64))?;
        let mut buf = vec![0u8; loc.size as usize - HEADER_SIZE];
        file.read_exact(&mut buf)?;
        Ok(Some(buf))
    }

    /// Flushes archives and the journal to disk.
    pub fn sync(&mut self) -> Result<()> {
        if let Some((_, file, _)) = &self.current {
            file.sync_data()?;
        }
        if !self.read_only {
            self.journal.sync_data()?;
        }
        Ok(())
    }

    /// Replaces all `.idx` files with version-1 files for the 16 buckets.
    pub fn write_indices(&mut self) -> Result<()> {
        self.sync()?;
        for entry in fs::read_dir(&self.data_dir)? {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("idx"))
            {
                fs::remove_file(path)?;
            }
        }
        let mut buckets: Vec<Vec<idx::IdxEntry>> = vec![Vec::new(); idx::BUCKETS as usize];
        for (ekey, loc) in &self.entries {
            buckets[idx::bucket(ekey) as usize].push(idx::IdxEntry {
                key: ekey[..9].try_into().expect("9 bytes"),
                storage_offset: loc.storage_offset(),
                size: loc.size,
            });
        }
        for (b, list) in buckets.iter().enumerate() {
            let path = self.data_dir.join(idx::file_name(b as u8, 1));
            write_atomic(&path, &idx::build(b as u8, list))?;
        }
        Ok(())
    }

    /// `Data/config/xx/yy/<key>`.
    pub fn write_config(&self, key: &Key, data: &[u8]) -> Result<()> {
        let k = hex(key);
        let dir = self.root.join("Data/config").join(&k[0..2]).join(&k[2..4]);
        fs::create_dir_all(&dir)?;
        write_atomic(&dir.join(&k), data)
    }

    /// `Data/indices/<archive>.index`.
    pub fn index_path(&self, archive: &str) -> PathBuf {
        self.root
            .join("Data/indices")
            .join(format!("{archive}.index"))
    }
}

/// Journal replay: the entries whose bytes are really on disk, the end of the
/// last entry per archive, and whether records were dropped (torn or beyond
/// an archive's length).
fn replay(raw: &[u8], data_dir: &Path) -> (HashMap<Key, Location>, HashMap<u16, u64>, bool) {
    let archive_len =
        |a: u16| -> u64 { fs::metadata(data_dir.join(archive_name(a))).map_or(0, |m| m.len()) };
    let mut entries = HashMap::new();
    let mut ends: HashMap<u16, u64> = HashMap::new();
    let mut lengths: HashMap<u16, u64> = HashMap::new();
    let mut dropped = false;
    for r in raw.get(8..).unwrap_or_default().chunks(RECORD) {
        if r.len() < RECORD {
            dropped = true;
            break;
        }
        let ekey: Key = r[..16].try_into().expect("16 bytes");
        let loc = Location {
            archive: u16::from_le_bytes([r[16], r[17]]),
            offset: u32::from_le_bytes(r[18..22].try_into().expect("4 bytes")),
            size: u32::from_le_bytes(r[22..26].try_into().expect("4 bytes")),
        };
        let end = u64::from(loc.offset) + u64::from(loc.size);
        let len = *lengths
            .entry(loc.archive)
            .or_insert_with(|| archive_len(loc.archive));
        if end > len {
            dropped = true;
            continue;
        }
        let e = ends.entry(loc.archive).or_default();
        *e = (*e).max(end);
        entries.insert(ekey, loc);
    }
    (entries, ends, dropped)
}

fn archive_name(archive: u16) -> String {
    format!("data.{archive:03}")
}

fn list_archives(dir: &Path) -> Result<Vec<u16>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name();
        if let Some(n) = name.to_str().and_then(|n| n.strip_prefix("data."))
            && let Ok(a) = n.parse::<u16>()
        {
            out.push(a);
        }
    }
    Ok(out)
}

fn record(ekey: &Key, loc: Location) -> [u8; RECORD] {
    let mut r = [0u8; RECORD];
    r[..16].copy_from_slice(ekey);
    r[16..18].copy_from_slice(&loc.archive.to_le_bytes());
    r[18..22].copy_from_slice(&loc.offset.to_le_bytes());
    r[22..26].copy_from_slice(&loc.size.to_le_bytes());
    r
}

/// Writes through a temporary file and renames it into place.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("wcd-tmp");
    fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
