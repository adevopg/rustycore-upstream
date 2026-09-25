//! CDN archive indices (`data/xx/yy/<archive>.index`, also the `file-index`).
//!
//! Layout per wowdev `TACT#CDN_File_Organization` and blizzget
//! `DownloadTask::loadIndices`: 4 KiB blocks of `{EKey, u32 BE size, u32 BE
//! offset}` entries (zero padded), a table of contents (last key of each
//! block, then the first 8 bytes of each block's MD5) and a 28-byte footer
//! `{toc hash[8], version = 1, 0, 0, block size KiB, offset bytes, size bytes,
//! key bytes, checksum bytes = 8, u32 LE entry count, footer hash[8]}`.
//!
//! Checked like the Agent: the footer hash is `MD5(footer[8..20] + 8 zero
//! bytes)[..8]`, the archive name is `MD5(footer)`, every block matches its TOC
//! hash (verified on real 54261 `.index` files).
//!
//! The same format with other offset widths holds the `file-index` /
//! `patch-file-index` (no offset) and the `archive-group` /
//! `patch-archive-group` (an archive number of 1 or 2 bytes before the 4-byte
//! offset). The groups are not on the CDN (Blizzard 403, mirror 404): the
//! Agent builds them from the archive indices ([`build_group`]). The rule was
//! confirmed by regenerating both 54261 groups byte-exactly, i.e. their
//! footer MD5 equals the CDN config keys `5b60c64f...` (1386 archives, 2-byte
//! numbers, 5213121 entries) and `5111509f...` (155 patch archives, 1-byte
//! numbers, 2088691 entries); a real Agent `archive-group` footer also shows
//! 6 offset bytes and its `patch-archive-group` (76 archives) 5.

use anyhow::{Result, ensure};

use crate::util::{Key, be_uint, hex, md5};

const FOOTER_SIZE: usize = 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    pub ekey: Key,
    pub size: u32,
    /// Offset in the archive; 0 for a `file-index` (offset bytes = 0).
    pub offset: u32,
    /// Archive number of a group index entry; 0 otherwise.
    pub archive: u16,
}

/// Parses an index file. `name` is the archive key the file must hash to.
pub fn parse(data: &[u8], name: Option<&Key>) -> Result<Vec<IndexEntry>> {
    ensure!(data.len() >= FOOTER_SIZE, "index too short");
    let footer = &data[data.len() - FOOTER_SIZE..];
    let mut check = footer[8..20].to_vec();
    check.extend_from_slice(&[0; 8]);
    ensure!(
        md5(&check)[..8] == footer[20..28],
        "index footer checksum mismatch"
    );
    if let Some(name) = name {
        ensure!(
            &md5(footer) == name,
            "index footer does not hash to {}",
            hex(name)
        );
    }
    let (version, block_kb, offset_bytes, size_bytes, key_bytes, checksum_bytes) = (
        footer[8],
        footer[11] as usize,
        footer[12] as usize,
        footer[13] as usize,
        footer[14] as usize,
        footer[15] as usize,
    );
    ensure!(
        version == 1 && key_bytes == 16 && checksum_bytes == 8,
        "unsupported index version/key size"
    );
    ensure!(
        size_bytes == 4 && offset_bytes <= 6,
        "unsupported index size/offset widths"
    );
    let count = u32::from_le_bytes(footer[16..20].try_into().expect("4 bytes")) as usize;
    let block = block_kb * 1024;
    let body = data.len() - FOOTER_SIZE;
    let per_block = block + key_bytes + checksum_bytes;
    ensure!(
        block > 0 && body.is_multiple_of(per_block),
        "index size does not match its blocks"
    );
    let blocks = body / per_block;
    let hashes = &data[blocks * block + blocks * key_bytes..body];

    let entry_len = key_bytes + size_bytes + offset_bytes;
    let mut out = Vec::with_capacity(count);
    for b in 0..blocks {
        let page = &data[b * block..(b + 1) * block];
        ensure!(
            md5(page)[..8] == hashes[b * 8..b * 8 + 8],
            "index block {b} checksum mismatch"
        );
        for e in page.chunks_exact(entry_len) {
            if out.len() == count {
                break;
            }
            let ekey: Key = e[..16].try_into().expect("16 bytes");
            if ekey == [0; 16] {
                break;
            }
            let field = be_uint(&e[16 + size_bytes..entry_len]);
            out.push(IndexEntry {
                ekey,
                size: be_uint(&e[16..16 + size_bytes]) as u32,
                offset: field as u32,
                archive: (field >> 32) as u16,
            });
        }
    }
    ensure!(
        out.len() == count,
        "index lists {} entries, footer says {count}",
        out.len()
    );
    Ok(out)
}

/// Serializes an index with 4 KiB blocks: entries `{EKey, u32 size, offset
/// field}` where the offset field is empty (`offset_bytes` 0), the 4-byte
/// offset (4) or the archive number in `offset_bytes - 4` bytes followed by
/// the offset (5, 6). Returns `(file, name)`, the name being `MD5(footer)`.
pub fn write(entries: &[IndexEntry], offset_bytes: u8) -> (Vec<u8>, Key) {
    const BLOCK: usize = 4096;
    let ob = usize::from(offset_bytes);
    let entry_len = 16 + 4 + ob;
    let per_block = BLOCK / entry_len;
    let mut out = Vec::with_capacity(entries.len().div_ceil(per_block.max(1)) * (BLOCK + 24) + 28);
    let mut last_keys = Vec::new();
    let mut block_hashes = Vec::new();
    for chunk in entries.chunks(per_block.max(1)) {
        let start = out.len();
        for e in chunk {
            out.extend_from_slice(&e.ekey);
            out.extend_from_slice(&e.size.to_be_bytes());
            if ob > 4 {
                out.extend_from_slice(&u32::from(e.archive).to_be_bytes()[8 - ob..]);
            }
            if ob >= 4 {
                out.extend_from_slice(&e.offset.to_be_bytes());
            }
        }
        out.resize(start + BLOCK, 0);
        block_hashes.push(md5(&out[start..])[..8].to_vec());
        last_keys.push(chunk.last().expect("non-empty chunk").ekey);
    }
    let toc_start = out.len();
    for k in &last_keys {
        out.extend_from_slice(k);
    }
    for h in &block_hashes {
        out.extend_from_slice(h);
    }
    let mut footer = md5(&out[toc_start..])[..8].to_vec();
    footer.extend_from_slice(&[1, 0, 0, 4, offset_bytes, 4, 16, 8]);
    footer.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    let mut check = footer[8..20].to_vec();
    check.extend_from_slice(&[0; 8]);
    footer.extend_from_slice(&md5(&check)[..8]);
    out.extend_from_slice(&footer);
    let name = md5(&footer);
    (out, name)
}

/// Builds an `archive-group`/`patch-archive-group` from the parsed indices of
/// the CDN config's archives, in config order (the archive number is the
/// position): all entries sorted by `EKey` (first archive wins on a
/// duplicate), 2-byte archive numbers when there are more than 255 archives,
/// else 1.
pub fn build_group(indices: &[Vec<IndexEntry>]) -> (Vec<u8>, Key) {
    let mut all: Vec<IndexEntry> = indices
        .iter()
        .enumerate()
        .flat_map(|(a, list)| {
            list.iter().map(move |e| IndexEntry {
                archive: a as u16,
                ..*e
            })
        })
        .collect();
    all.sort_by(|x, y| x.ekey.cmp(&y.ekey).then(x.archive.cmp(&y.archive)));
    all.dedup_by_key(|e| e.ekey);
    let offset_bytes = if indices.len() > 255 { 6 } else { 5 };
    write(&all, offset_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(n: usize) -> Vec<IndexEntry> {
        (0..n)
            .map(|i| {
                let mut ekey = [0u8; 16];
                ekey[..4].copy_from_slice(&(i as u32 + 1).to_be_bytes());
                IndexEntry {
                    ekey,
                    size: 100 + i as u32,
                    offset: i as u32 * 1000,
                    archive: 0,
                }
            })
            .collect()
    }

    #[test]
    fn round_trip_multi_block() {
        let e = entries(400); // 170 entries per 4 KiB block -> 3 blocks
        let (data, name) = write(&e, 4);
        assert_eq!(parse(&data, Some(&name)).unwrap(), e);
        assert!(parse(&data, Some(&[0; 16])).is_err(), "wrong archive name");
    }

    #[test]
    fn corruption_is_detected() {
        let (data, _) = write(&entries(10), 4);
        let mut broken = data.clone();
        broken[5] ^= 1;
        assert!(parse(&broken, None).is_err(), "block hash");
        let mut broken = data.clone();
        let n = broken.len();
        broken[n - 10] ^= 1;
        assert!(parse(&broken, None).is_err(), "footer hash");
    }

    #[test]
    fn file_index_without_offsets() {
        let mut e = entries(300);
        for x in &mut e {
            x.offset = 0;
        }
        let (data, name) = write(&e, 0);
        // 20-byte entries: 204 per block -> 2 blocks.
        assert_eq!(data.len(), 2 * (4096 + 24) + 28);
        assert_eq!(parse(&data, Some(&name)).unwrap(), e);
    }

    #[test]
    fn groups_merge_sort_and_number_archives() {
        let a = entries(3); // keys 1, 2, 3
        let mut b = entries(5)[3..].to_vec(); // keys 4, 5
        b.push(a[1]); // duplicate of key 2: the first archive wins
        b.sort_by_key(|e| e.ekey);
        let (data, name) = build_group(&[b.clone(), a.clone()]);
        assert_eq!(data[data.len() - 28 + 12], 5, "1-byte archive numbers");
        let merged = parse(&data, Some(&name)).unwrap();
        let keys: Vec<u8> = merged.iter().map(|e| e.ekey[3]).collect();
        assert_eq!(keys, [1, 2, 3, 4, 5]);
        assert_eq!(merged[1].archive, 0, "duplicate kept from archive 0");
        assert_eq!(merged[0].archive, 1);
        assert_eq!(merged[3].archive, 0);
        assert_eq!(merged[4].offset, 4000);

        let many: Vec<Vec<IndexEntry>> = (0..300).map(|_| Vec::new()).chain([a]).collect();
        let (data, name) = build_group(&many);
        assert_eq!(data[data.len() - 28 + 12], 6, "2-byte archive numbers");
        let merged = parse(&data, Some(&name)).unwrap();
        assert!(merged.iter().all(|e| e.archive == 300));
    }
}
