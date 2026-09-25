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

use anyhow::{Result, ensure};

use crate::util::{Key, be_uint, hex, md5};

const FOOTER_SIZE: usize = 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    pub ekey: Key,
    pub size: u32,
    /// Offset in the archive; 0 for a `file-index` (offset bytes = 0).
    pub offset: u32,
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
        size_bytes == 4 && offset_bytes <= 4,
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
            out.push(IndexEntry {
                ekey,
                size: be_uint(&e[16..16 + size_bytes]) as u32,
                offset: be_uint(&e[16 + size_bytes..entry_len]) as u32,
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

/// Builds an archive index (offset bytes = 4) from sorted entries; returns
/// `(file, archive name)`.
#[cfg(test)]
pub fn build(entries: &[IndexEntry], block_kb: u8) -> (Vec<u8>, Key) {
    let block = block_kb as usize * 1024;
    let mut blocks: Vec<Vec<u8>> = Vec::new();
    let mut last_keys = Vec::new();
    let mut current = Vec::new();
    let mut last = [0u8; 16];
    for e in entries {
        if current.len() + 24 > block {
            current.resize(block, 0);
            blocks.push(std::mem::take(&mut current));
            last_keys.push(last);
        }
        current.extend_from_slice(&e.ekey);
        current.extend_from_slice(&e.size.to_be_bytes());
        current.extend_from_slice(&e.offset.to_be_bytes());
        last = e.ekey;
    }
    current.resize(block, 0);
    blocks.push(current);
    last_keys.push(last);
    let mut out: Vec<u8> = blocks.concat();
    let mut toc = Vec::new();
    for k in &last_keys {
        toc.extend_from_slice(k);
    }
    for b in &blocks {
        toc.extend_from_slice(&md5(b)[..8]);
    }
    out.extend_from_slice(&toc);
    let mut footer = md5(&toc)[..8].to_vec();
    footer.extend_from_slice(&[1, 0, 0, block_kb, 4, 4, 16, 8]);
    footer.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    let mut check = footer[8..20].to_vec();
    check.extend_from_slice(&[0; 8]);
    footer.extend_from_slice(&md5(&check)[..8]);
    out.extend_from_slice(&footer);
    (out, md5(&footer))
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
                }
            })
            .collect()
    }

    #[test]
    fn round_trip_multi_block() {
        let e = entries(400); // 170 entries per 4 KiB block -> 3 blocks
        let (data, name) = build(&e, 4);
        assert_eq!(parse(&data, Some(&name)).unwrap(), e);
        assert!(parse(&data, Some(&[0; 16])).is_err(), "wrong archive name");
    }

    #[test]
    fn corruption_is_detected() {
        let (data, _) = build(&entries(10), 4);
        let mut broken = data.clone();
        broken[5] ^= 1;
        assert!(parse(&broken, None).is_err(), "block hash");
        let mut broken = data.clone();
        let n = broken.len();
        broken[n - 10] ^= 1;
        assert!(parse(&broken, None).is_err(), "footer hash");
    }
}
