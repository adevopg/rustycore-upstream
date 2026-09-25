//! Local index files `Data/data/BBVVVVVVVV.idx` (index version 2,
//! `IndexVersion == 7`).
//!
//! Written in the exact layout `CascLib` `CascIndexFiles.cpp` (and `wow-casc`
//! `index.rs`) validates, with the values of a real Agent-written file:
//!
//! * guarded block 1: `u32 size = 0x10`, `u32 hashlittle(header, 0)`, then the
//!   header `{u16 7, u8 bucket, u8 0, 4 size bytes, 5 offset bytes, 9 key bytes,
//!   30 offset bits, u64 LE 0xFF_C000_0000}` and 8 bytes of padding;
//! * guarded block 2: `u32 size`, `u32 hash` = `hashlittle2` primary value
//!   chained over the 18-byte entries (the Blizzard variant of
//!   `CaptureGuardedBlock2`), entries sorted by key: 9-byte `EKey` prefix,
//!   5-byte big-endian storage offset `(archive << 30) | offset`, 4-byte LE size;
//! * a zero-filled update section of at least [`UPDATE_SECTION_MIN`] bytes,
//!   then zero padding to a 64 KiB multiple. The client's TACT rejects a file
//!   whose update section is shorter ("Truncated updated section in KMT file
//!   detected" -> `NeedsRepair`); every Agent-written file (16/16 checked)
//!   has `size == align_up(entries_end + 0x7800, 0x10000)`.
//!
//! The bucket of a key is `CascLib`'s: `x = xor(key[0..9])`,
//! `(x & 0x0F) ^ (x >> 4)` (checked against a real Agent install, where each
//! file additionally holds 64 keys of the previous bucket).

use crate::jenkins::{hashlittle, hashlittle2};

pub const BUCKETS: u8 = 16;
pub const OFFSET_BITS: u8 = 30;
const ENTRY_SIZE: usize = 18;
/// `FILE_INDEX_HEADER_V2::SegmentSize` of the Agent's files.
const MAX_FILE_OFFSET: u64 = 0xFF_C000_0000;

pub fn bucket(ekey: &[u8]) -> u8 {
    let x = ekey[..9].iter().fold(0u8, |acc, b| acc ^ b);
    (x & 0x0F) ^ (x >> 4)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct IdxEntry {
    pub key: [u8; 9],
    pub storage_offset: u64,
    pub size: u32,
}

/// `BBVVVVVVVV.idx` file name.
pub fn file_name(bucket: u8, version: u32) -> String {
    format!("{bucket:02x}{version:08x}.idx")
}

/// Serializes one bucket's index file.
pub fn build(bucket: u8, entries: &[IdxEntry]) -> Vec<u8> {
    let mut sorted = entries.to_vec();
    sorted.sort_unstable();
    sorted.dedup_by_key(|e| e.key);

    let mut header = vec![0x07, 0x00, bucket, 0x00, 4, 5, 9, OFFSET_BITS];
    header.extend_from_slice(&MAX_FILE_OFFSET.to_le_bytes());
    let mut out = Vec::with_capacity(0x28 + sorted.len() * ENTRY_SIZE);
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&hashlittle(&header, 0).to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&[0u8; 8]);

    let mut block = Vec::with_capacity(sorted.len() * ENTRY_SIZE);
    let (mut high, mut low) = (0u32, 0u32);
    for e in &sorted {
        let start = block.len();
        block.extend_from_slice(&e.key);
        block.extend_from_slice(&e.storage_offset.to_be_bytes()[3..]);
        block.extend_from_slice(&e.size.to_le_bytes());
        hashlittle2(&block[start..], &mut high, &mut low);
    }
    out.extend_from_slice(&(block.len() as u32).to_le_bytes());
    out.extend_from_slice(&high.to_le_bytes());
    out.extend_from_slice(&block);
    out.resize(
        (out.len() + UPDATE_SECTION_MIN).div_ceil(0x1_0000) * 0x1_0000,
        0,
    );
    out
}

/// Minimum size of the (empty) KMT update section that follows the entries.
pub(crate) const UPDATE_SECTION_MIN: usize = 0x7800;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_function() {
        // xor of the prefix = 0x5A -> 0xA ^ 0x5 = 0xF
        let mut key = [0u8; 16];
        key[0] = 0x5A;
        assert_eq!(bucket(&key), 0x0F);
        key[1] = 0x5A;
        assert_eq!(bucket(&key), 0);
        key[9] = 0xFF; // beyond the 9-byte prefix
        assert_eq!(bucket(&key), 0);
    }

    #[test]
    fn layout() {
        let e = IdxEntry {
            key: [1; 9],
            storage_offset: (3 << 30) | 0x1234,
            size: 99,
        };
        let data = build(2, &[e, e]);
        assert_eq!(data.len(), 0x1_0000);
        // entries end at 0x28 + 36; the update section must still fit after them
        assert!(data.len() - (0x28 + 36) >= UPDATE_SECTION_MIN);
        assert_eq!(&data[8..16], &[7, 0, 2, 0, 4, 5, 9, 30]);
        assert_eq!(&data[16..24], &[0, 0, 0, 0xC0, 0xFF, 0, 0, 0]);
        assert_eq!(u32::from_le_bytes(data[0x20..0x24].try_into().unwrap()), 18);
        assert_eq!(&data[0x28..0x31], &[1; 9]);
        assert_eq!(&data[0x31..0x36], &[0, 0xC0, 0, 0x12, 0x34]);
        assert_eq!(file_name(0x0f, 1), "0f00000001.idx");
    }

    #[test]
    fn update_section_is_never_truncated() {
        // Entries that end just below a 64 KiB boundary (as in a real bucket of
        // the 3.4.3 client) must push the file to the next boundary.
        let e = IdxEntry {
            key: [2; 9],
            storage_offset: 0,
            size: 1,
        };
        let n = (0x1_0000 - 0x28 - 100) / 18;
        let mut entries = Vec::new();
        for i in 0..n {
            let mut k = e;
            k.key[..4].copy_from_slice(&(i as u32).to_be_bytes());
            entries.push(k);
        }
        let data = build(0, &entries);
        let end = 0x28 + n * 18;
        assert_eq!(
            data.len(),
            (end + UPDATE_SECTION_MIN).div_ceil(0x1_0000) * 0x1_0000
        );
        assert!(data[end..].iter().all(|&b| b == 0));
    }
}
