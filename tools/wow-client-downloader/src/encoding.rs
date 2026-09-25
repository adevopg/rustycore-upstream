//! ENCODING manifest: `CKey` -> (`EKey`, content size) and `EKey` -> encoded
//! size.
//!
//! Layout per wowdev `TACT#Encoding_table` and blizzget `NGDP::Encoding`
//! (`EncodingFileHeader`, `EncodingEntry`, `LayoutEntry`): a 22-byte big-endian
//! header, the `ESpec` string block, the `CKey` page table (`{first key, page
//! MD5}`) and pages, then the `EKey` spec page table and pages. Every page is
//! checked against its MD5 like blizzget does. Content and encoded sizes are
//! 40-bit big-endian values.

use anyhow::{Result, ensure};

use crate::util::{Key, be_uint, md5};

const HEADER_SIZE: usize = 22;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CKeyEntry {
    pub ckey: Key,
    /// First `EKey` of the entry (the one the client uses).
    pub ekey: Key,
    pub content_size: u64,
}

#[derive(Debug, Default)]
pub struct Encoding {
    /// Sorted by `CKey` (pages are sorted and in order).
    ckeys: Vec<CKeyEntry>,
    /// `(EKey, encoded size)` sorted by `EKey`.
    ekeys: Vec<(Key, u64)>,
}

impl Encoding {
    pub fn parse(data: &[u8]) -> Result<Self> {
        ensure!(
            data.len() >= HEADER_SIZE && &data[..2] == b"EN" && data[2] == 1,
            "ENCODING: bad signature or version"
        );
        let ckey_len = data[3] as usize;
        let ekey_len = data[4] as usize;
        ensure!(
            ckey_len == 16 && ekey_len == 16,
            "ENCODING: unsupported key sizes"
        );
        let ckey_page = be_uint(&data[5..7]) as usize * 1024;
        let ekey_page = be_uint(&data[7..9]) as usize * 1024;
        let ckey_pages = be_uint(&data[9..13]) as usize;
        let ekey_pages = be_uint(&data[13..17]) as usize;
        let espec_size = be_uint(&data[18..22]) as usize;

        let mut pos = HEADER_SIZE + espec_size;
        let ckey_table = take(data, &mut pos, ckey_pages * 32)?;
        let ckey_data = take(data, &mut pos, ckey_pages * ckey_page)?;
        let ekey_table = take(data, &mut pos, ekey_pages * 32)?;
        let ekey_data = take(data, &mut pos, ekey_pages * ekey_page)?;

        let mut ckeys = Vec::new();
        for (i, page) in ckey_data.chunks_exact(ckey_page).enumerate() {
            check_page(ckey_table, i, page, 6)?;
            let mut p = 0;
            while p + 6 + 16 <= page.len() {
                let count = page[p] as usize;
                if count == 0 {
                    break;
                }
                let len = 6 + 16 + count * 16;
                ensure!(
                    p + len <= page.len(),
                    "ENCODING: CKey entry crosses its page"
                );
                ckeys.push(CKeyEntry {
                    content_size: be_uint(&page[p + 1..p + 6]),
                    ckey: page[p + 6..p + 22].try_into().expect("16 bytes"),
                    ekey: page[p + 22..p + 38].try_into().expect("16 bytes"),
                });
                p += len;
            }
        }
        let mut ekeys = Vec::new();
        for (i, page) in ekey_data.chunks_exact(ekey_page).enumerate() {
            check_page(ekey_table, i, page, 0)?;
            for e in page.as_chunks::<25>().0 {
                let ekey: Key = e[..16].try_into().expect("16 bytes");
                if ekey == [0; 16] {
                    break;
                }
                ekeys.push((ekey, be_uint(&e[20..25])));
            }
        }
        ensure!(
            ckeys.is_sorted_by_key(|e| e.ckey),
            "ENCODING: CKeys are not sorted"
        );
        ekeys.sort_unstable();
        Ok(Self { ckeys, ekeys })
    }

    pub fn find(&self, ckey: &Key) -> Option<&CKeyEntry> {
        self.ckeys
            .binary_search_by(|e| e.ckey.cmp(ckey))
            .ok()
            .map(|i| &self.ckeys[i])
    }

    pub fn encoded_size(&self, ekey: &Key) -> Option<u64> {
        self.ekeys
            .binary_search_by(|(k, _)| k.cmp(ekey))
            .ok()
            .map(|i| self.ekeys[i].1)
    }

    pub fn len(&self) -> usize {
        self.ckeys.len()
    }
}

fn take<'a>(data: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8]> {
    let end = pos
        .checked_add(len)
        .filter(|&e| e <= data.len())
        .ok_or_else(|| anyhow::anyhow!("ENCODING: truncated"))?;
    let out = &data[*pos..end];
    *pos = end;
    Ok(out)
}

/// Checks a page against its table record `{first key, MD5}`; the first key
/// sits at `key_offset` (after the count/size bytes of a `CKey` entry).
fn check_page(table: &[u8], index: usize, page: &[u8], key_offset: usize) -> Result<()> {
    let record = &table[index * 32..index * 32 + 32];
    ensure!(
        md5(page) == record[16..32],
        "ENCODING: page {index} MD5 mismatch"
    );
    ensure!(
        page.get(key_offset..key_offset + 16) == Some(&record[..16]),
        "ENCODING: page {index} does not start with its first key"
    );
    Ok(())
}

/// Builds an ENCODING file with 1 KiB pages from sorted
/// `(ckey, ekey, content size, encoded size)` entries.
#[cfg(test)]
pub fn build(entries: &[(Key, Key, u64, u64)], per_page: usize) -> Vec<u8> {
    let espec = b"z\0";
    let mut ckey_pages = Vec::new();
    for chunk in entries.chunks(per_page) {
        let mut p = Vec::new();
        for (ckey, ekey, size, _) in chunk {
            p.push(1);
            p.extend_from_slice(&size.to_be_bytes()[3..]);
            p.extend_from_slice(ckey);
            p.extend_from_slice(ekey);
        }
        p.resize(1024, 0);
        ckey_pages.push((chunk[0].0, p));
    }
    let mut sorted_e: Vec<_> = entries.iter().map(|e| (e.1, e.3)).collect();
    sorted_e.sort_unstable();
    let mut ekey_pages = Vec::new();
    for chunk in sorted_e.chunks(per_page) {
        let mut p = Vec::new();
        for (ekey, esize) in chunk {
            p.extend_from_slice(ekey);
            p.extend_from_slice(&0u32.to_be_bytes());
            p.extend_from_slice(&esize.to_be_bytes()[3..]);
        }
        p.resize(1024, 0);
        ekey_pages.push((chunk[0].0, p));
    }
    let mut out = b"EN".to_vec();
    out.extend_from_slice(&[1, 16, 16]);
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&(ckey_pages.len() as u32).to_be_bytes());
    out.extend_from_slice(&(ekey_pages.len() as u32).to_be_bytes());
    out.push(0);
    out.extend_from_slice(&(espec.len() as u32).to_be_bytes());
    out.extend_from_slice(espec);
    for pages in [&ckey_pages, &ekey_pages] {
        for (first, page) in pages {
            out.extend_from_slice(first);
            out.extend_from_slice(&md5(page));
        }
        for (_, page) in pages {
            out.extend_from_slice(page);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<(Key, Key, u64, u64)> {
        (0u8..60)
            .map(|i| {
                let mut ckey = [0u8; 16];
                ckey[0] = i * 4 + 1;
                let mut ekey = [0xEE; 16];
                ekey[0] = 255 - i;
                (
                    ckey,
                    ekey,
                    u64::from(i) * 1000 + 0x1_0000_0000,
                    u64::from(i) + 7,
                )
            })
            .collect()
    }

    #[test]
    fn lookups_across_pages() {
        let e = entries();
        let enc = Encoding::parse(&build(&e, 20)).unwrap();
        assert_eq!(enc.len(), 60);
        for (ckey, ekey, size, esize) in &e {
            let found = enc.find(ckey).unwrap();
            assert_eq!(&found.ekey, ekey);
            assert_eq!(found.content_size, *size, "40-bit sizes");
            assert_eq!(enc.encoded_size(ekey), Some(*esize));
        }
        assert!(enc.find(&[0; 16]).is_none());
        assert!(enc.encoded_size(&[1; 16]).is_none());
    }

    #[test]
    fn page_hash_is_checked() {
        let mut data = build(&entries(), 20);
        let last = data.len() - 1;
        data[last - 600] ^= 0xFF;
        assert!(Encoding::parse(&data).is_err());
        assert!(Encoding::parse(b"XX").is_err());
    }
}
