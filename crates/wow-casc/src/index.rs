//! Local index files (`Data/data/##########.idx`).
//!
//! Port of `CascLib` `dep/CascLib/src/CascIndexFiles.cpp`:
//! `IndexDirectory_OnFileFound` (newest sub-index per bucket),
//! `LoadLocalIndexFiles`, `CaptureIndexHeader_V2`, `CaptureGuardedBlock1/2/3`,
//! `LoadIndexFile_V2`, `InsertEncodingEKeyToMap` and `CopyEKeyEntry`.
//!
//! Only index version 2 (`IndexVersion == 7`, used by every `WoW` build since
//! 6.0) is supported; the Heroes-alpha `data.i##` v1 format is not.

use std::collections::HashMap;
use std::path::Path;

use crate::jenkins::{hashlittle, hashlittle2};
use crate::{Error, Result};

/// `CascLib` `CASC_INDEX_COUNT`.
const INDEX_COUNT: usize = 0x10;
/// `CascLib` `CASC_EKEY_SIZE`.
pub const EKEY_SIZE: usize = 9;
/// `CascLib` `FILE_INDEX_PAGE_SIZE`.
const FILE_INDEX_PAGE_SIZE: usize = 0x200;
/// `sizeof(FILE_INDEX_GUARDED_BLOCK)`.
const GUARDED_BLOCK_SIZE: usize = 8;
/// `sizeof(FILE_INDEX_HEADER_V2)`.
const HEADER_V2_SIZE: usize = 0x10;

/// Location of an encoded file inside the local data archives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    /// `StorageOffset` (big-endian 5 bytes): archive index in the high bits,
    /// archive offset in the low `file_offset_bits`.
    pub storage_offset: u64,
    /// `EncodedSize` (little-endian 4 bytes), including the 0x1E-byte
    /// encoded header in front of the BLTE data.
    pub encoded_size: u32,
}

/// Map of the first 9 bytes of an `EKey` to the local storage position
/// (`CascLib` `hs->IndexEKeyMap`).
#[derive(Debug, Default)]
pub struct LocalIndex {
    map: HashMap<[u8; EKEY_SIZE], IndexEntry>,
    /// `hs->FileOffsetBits`.
    pub file_offset_bits: u8,
}

impl LocalIndex {
    /// `CopyEKeyEntry`: looks up an `EKey` (only the first 9 bytes are used).
    pub fn find(&self, ekey: &[u8]) -> Option<IndexEntry> {
        let key: [u8; EKEY_SIZE] = ekey.get(..EKEY_SIZE)?.try_into().ok()?;
        self.map.get(&key).copied()
    }

    /// Splits a storage offset into `(archive index, archive offset)`
    /// (`TCascFile::InitFileSpans`).
    pub fn split_offset(&self, storage_offset: u64) -> (u32, u64) {
        let bits = u32::from(self.file_offset_bits);
        let archive = (storage_offset >> bits) as u32;
        let offset = storage_offset & ((1u64 << bits) - 1);
        (archive, offset)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// `InsertEncodingEKeyToMap` -> `CASC_MAP::InsertObject`: a duplicate key
    /// is rejected, i.e. the first inserted entry wins.
    fn insert(&mut self, entry: &[u8], header: IndexHeader) {
        let key: [u8; EKEY_SIZE] = entry[..EKEY_SIZE].try_into().expect("entry length checked");
        let e = &entry[usize::from(header.ekey_length)..];
        let storage_offset = e[..5]
            .iter()
            .fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
        let encoded_size = u32::from_le_bytes([e[5], e[6], e[7], e[8]]);
        self.map.entry(key).or_insert(IndexEntry {
            storage_offset,
            encoded_size,
        });
    }

    /// `LoadLocalIndexFiles`: scans `index_dir` for `##########.idx` files,
    /// takes the newest sub-index of every bucket 0..15 and loads them in
    /// bucket order, stopping at the first missing bucket.
    pub fn load_dir(index_dir: &Path) -> Result<Self> {
        let mut newest: [u32; INDEX_COUNT] = [0; INDEX_COUNT];
        let mut found_any = false;
        for dir_entry in std::fs::read_dir(index_dir)? {
            let dir_entry = dir_entry?;
            let name = dir_entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some((bucket, version)) = parse_index_file_name(name) else {
                continue;
            };
            found_any = true;
            // "The index value must not be greater than 0x0F"
            if bucket as usize >= INDEX_COUNT {
                continue;
            }
            if version > newest[bucket as usize] {
                newest[bucket as usize] = version;
            }
        }
        if !found_any {
            return Err(Error::InvalidStorage(format!(
                "no .idx files in {}",
                index_dir.display()
            )));
        }

        let mut index = LocalIndex::default();
        for (bucket, &version) in newest.iter().enumerate() {
            let path = index_dir.join(format!("{bucket:02x}{version:08x}.idx"));
            let data = match std::fs::read(&path) {
                Ok(data) => data,
                // "Storages downloaded by Blizzget tool don't have all index files present"
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(e.into()),
            };
            index.load_file(&data, bucket as u8)?;
        }
        Ok(index)
    }

    /// `LoadIndexFile` for index version 2.
    pub fn load_file(&mut self, data: &[u8], bucket: u8) -> Result<()> {
        let header = capture_index_header_v2(data, bucket).ok_or_else(|| {
            Error::Corrupt(format!("index file for bucket {bucket:02x}: bad v2 header"))
        })?;
        // SaveFileOffsetBitsAndEKeyLength
        if self.file_offset_bits == 0 {
            self.file_offset_bits = header.file_offset_bits;
        }

        let entry_length = header.entry_length();
        let file_ptr = GUARDED_BLOCK_SIZE + HEADER_V2_SIZE + 8; // HeaderLength + HeaderPadding
        if let Some((start, size)) = capture_guarded_block2(data, file_ptr, entry_length) {
            // LoadIndexItems over the continuous block
            let block = &data[start..start + size];
            for entry in block.chunks_exact(entry_length) {
                self.insert(entry, header);
            }
            return Ok(());
        }

        // Second layout: entries in 0x200-byte pages from offset 0x1000, each
        // protected by its own 32-bit hash (CaptureGuardedBlock3).
        if data.len().saturating_sub(file_ptr) >= 0x7800 {
            let aligned = entry_length.div_ceil(4) * 4;
            let mut page = 0x1000;
            while page < data.len() {
                let page_end = page + FILE_INDEX_PAGE_SIZE;
                let mut pos = page;
                while pos < page_end {
                    let Some(entry_pos) = capture_guarded_block3(data, pos, page_end, entry_length)
                    else {
                        break;
                    };
                    self.insert(&data[entry_pos..entry_pos + entry_length], header);
                    pos = entry_pos + aligned;
                }
                page += FILE_INDEX_PAGE_SIZE;
            }
            return Ok(());
        }

        Err(Error::Corrupt(format!(
            "index file for bucket {bucket:02x}: no valid entry block"
        )))
    }
}

/// `IsIndexFileName_V2` + the `ConvertStringToInt` calls of
/// `IndexDirectory_OnFileFound`: `BBVVVVVVVV.idx` (hex digits).
fn parse_index_file_name(name: &str) -> Option<(u32, u32)> {
    if name.len() != 14 || !name[10..].eq_ignore_ascii_case(".idx") {
        return None;
    }
    let digits = &name[..10];
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let bucket = u32::from_str_radix(&digits[..2], 16).ok()?;
    let version = u32::from_str_radix(&digits[2..], 16).ok()?;
    Some((bucket, version))
}

#[derive(Debug, Clone, Copy)]
struct IndexHeader {
    encoded_size_length: u8,
    storage_offset_length: u8,
    ekey_length: u8,
    file_offset_bits: u8,
}

impl IndexHeader {
    fn entry_length(self) -> usize {
        usize::from(self.ekey_length)
            + usize::from(self.storage_offset_length)
            + usize::from(self.encoded_size_length)
    }
}

fn read_u32_le(data: &[u8], pos: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(pos..pos + 4)?.try_into().ok()?))
}

/// `CaptureGuardedBlock1` + `CaptureIndexHeader_V2`.
fn capture_index_header_v2(data: &[u8], bucket: u8) -> Option<IndexHeader> {
    // CaptureGuardedBlock1
    if GUARDED_BLOCK_SIZE >= data.len() {
        return None;
    }
    let block_size = read_u32_le(data, 0)? as usize;
    let block_hash = read_u32_le(data, 4)?;
    let body = data.get(GUARDED_BLOCK_SIZE..GUARDED_BLOCK_SIZE.checked_add(block_size)?)?;
    if hashlittle(body, 0) != block_hash {
        return None;
    }
    let h = data.get(GUARDED_BLOCK_SIZE..GUARDED_BLOCK_SIZE + HEADER_V2_SIZE)?;
    let index_version = u16::from_le_bytes([h[0], h[1]]);
    if index_version != 0x07 || h[2] != bucket || h[3] != 0 {
        return None;
    }
    if h[4] != 0x04 || h[5] != 0x05 || h[6] != 0x09 {
        return None;
    }
    Some(IndexHeader {
        encoded_size_length: h[4],
        storage_offset_length: h[5],
        ekey_length: h[6],
        file_offset_bits: h[7],
    })
}

/// `CaptureGuardedBlock2`: returns `(start of entries, block size)`. The block
/// hash is `hashlittle2` chained over the entries (Blizzard) or `hashlittle`
/// chained over the entries (`BlizzGet`).
fn capture_guarded_block2(data: &[u8], pos: usize, entry_length: usize) -> Option<(usize, usize)> {
    if pos + GUARDED_BLOCK_SIZE >= data.len() {
        return None;
    }
    let block_size = read_u32_le(data, pos)? as usize;
    let block_hash = read_u32_le(data, pos + 4)?;
    let start = pos + GUARDED_BLOCK_SIZE;
    if block_size == 0 || start + block_size > data.len() {
        return None;
    }
    let entries = &data[start..start + block_size];

    let (mut high, mut low) = (0u32, 0u32);
    for entry in entries.chunks_exact(entry_length) {
        hashlittle2(entry, &mut high, &mut low);
    }
    if high == block_hash {
        return Some((start, block_size));
    }

    let mut blizzget = 0u32;
    for entry in entries.chunks_exact(entry_length) {
        blizzget = hashlittle(entry, blizzget);
    }
    (blizzget == block_hash).then_some((start, block_size))
}

/// `CaptureGuardedBlock3`: a 32-bit hash followed by one entry; the hash
/// covers the entry plus one byte, with the top bit forced. Returns the
/// position of the entry.
fn capture_guarded_block3(
    data: &[u8],
    pos: usize,
    end: usize,
    entry_length: usize,
) -> Option<usize> {
    let end = end.min(data.len());
    if pos + 4 + entry_length >= end {
        return None;
    }
    let stored = read_u32_le(data, pos)?;
    if stored == 0 {
        return None;
    }
    let hashed = data.get(pos + 4..pos + 4 + entry_length + 1)?;
    if hashlittle(hashed, 0) | 0x8000_0000 != stored {
        return None;
    }
    Some(pos + 4)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a synthetic v2 `.idx` file with the given
    /// `(ekey9, storage_offset, encoded_size)` entries, hashed the way the
    /// Blizzard client does (`hashlittle2` chained over entries).
    pub(crate) fn build_idx(bucket: u8, entries: &[([u8; 9], u64, u32)]) -> Vec<u8> {
        let mut header = vec![0x07, 0x00, bucket, 0x00, 0x04, 0x05, 0x09, 30];
        header.extend_from_slice(&0x4000_0000u64.to_le_bytes());
        let mut out = Vec::new();
        out.extend_from_slice(&(header.len() as u32).to_le_bytes());
        out.extend_from_slice(&hashlittle(&header, 0).to_le_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(&[0u8; 8]); // padding

        let mut block = Vec::new();
        let (mut high, mut low) = (0u32, 0u32);
        for (ekey, offset, size) in entries {
            let mut e = Vec::new();
            e.extend_from_slice(ekey);
            e.extend_from_slice(&offset.to_be_bytes()[3..]);
            e.extend_from_slice(&size.to_le_bytes());
            hashlittle2(&e, &mut high, &mut low);
            block.extend_from_slice(&e);
        }
        out.extend_from_slice(&(block.len() as u32).to_le_bytes());
        out.extend_from_slice(&high.to_le_bytes());
        out.extend_from_slice(&block);
        out
    }

    #[test]
    fn parses_synthetic_idx() {
        let a = [1, 2, 3, 4, 5, 6, 7, 8, 9];
        let b = [9, 8, 7, 6, 5, 4, 3, 2, 1];
        let data = build_idx(
            3,
            &[(a, (5u64 << 30) | 0x1234, 777), (b, 0x10, 42), (a, 0, 1)],
        );
        let mut index = LocalIndex::default();
        index.load_file(&data, 3).unwrap();
        assert_eq!(index.len(), 2);
        assert_eq!(index.file_offset_bits, 30);
        let e = index
            .find(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 0xAA, 0xBB])
            .unwrap();
        assert_eq!(e.encoded_size, 777, "first inserted entry wins");
        assert_eq!(index.split_offset(e.storage_offset), (5, 0x1234));
        assert_eq!(index.find(&b).unwrap().encoded_size, 42);
        assert!(index.find(&[0; 16]).is_none());
    }

    #[test]
    fn rejects_wrong_bucket_and_bad_hash() {
        let data = build_idx(3, &[([1; 9], 0, 1)]);
        let mut index = LocalIndex::default();
        assert!(index.load_file(&data, 4).is_err());
        let mut broken = data.clone();
        *broken.last_mut().unwrap() ^= 0xFF;
        assert!(index.load_file(&broken, 3).is_err());
    }

    #[test]
    fn index_file_names() {
        assert_eq!(parse_index_file_name("0100000030.idx"), Some((1, 0x30)));
        assert_eq!(parse_index_file_name("0f0000002B.IDX"), Some((0x0f, 0x2b)));
        assert_eq!(parse_index_file_name("0000000000000004.lru"), None);
        assert_eq!(parse_index_file_name("data.001"), None);
    }
}
