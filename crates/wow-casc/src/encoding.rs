//! ENCODING manifest: `CKey` -> (`EKey`, content size).
//!
//! Port of `CascLib` `dep/CascLib/src/CascOpenStorage.cpp`:
//! `CaptureEncodingHeader`, `LoadEncodingManifest`, `LoadEncodingCKeyPage`
//! and the `InsertCKeyEntry(hs, PFILE_CKEY_ENTRY)` overload (first `EKey` of
//! each entry, big-endian 32-bit `ContentSize`).
//!
//! Instead of `CascLib`'s hash map of all `CKey` entries, the `CKey` pages are kept
//! as-is (they are sorted by `CKey`) and searched through the page table's
//! first keys. The page checks performed are the same as `CascLib`'s.

use crate::{Error, Result};

/// `sizeof(FILE_ENCODING_HEADER)`.
const HEADER_SIZE: usize = 0x16;
/// `sizeof(FILE_CKEY_PAGE)`.
const PAGE_HEADER_SIZE: usize = 0x20;
/// `CascLib` only supports 16-byte `CKeys` and `EKeys` in ENCODING.
const KEY_SIZE: usize = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodingEntry {
    pub ekey: [u8; 16],
    pub content_size: u32,
}

#[derive(Debug, Default)]
pub struct Encoding {
    /// First `CKey` of every `CKey` page (`FILE_CKEY_PAGE::FirstKey`).
    first_keys: Vec<[u8; 16]>,
    /// Concatenated `CKey` pages.
    pages: Vec<u8>,
    page_size: usize,
}

fn be(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |acc, &b| (acc << 8) | u32::from(b))
}

impl Encoding {
    /// `CaptureEncodingHeader` + the `CKey` page loop of `LoadEncodingManifest`.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let bad = |what: &str| Error::Corrupt(format!("ENCODING: {what}"));
        if data.len() < HEADER_SIZE || &data[0..2] != b"EN" || data[2] != 0x01 {
            return Err(bad("bad signature or version"));
        }
        if data[3] as usize != KEY_SIZE || data[4] as usize != KEY_SIZE {
            return Err(bad("unsupported key length"));
        }
        let page_size = be(&data[5..7]) as usize * 1024;
        let page_count = be(&data[9..13]) as usize;
        let espec_size = be(&data[18..22]) as usize;

        let table_start = HEADER_SIZE + espec_size;
        let pages_start = table_start + page_count * PAGE_HEADER_SIZE;
        let pages_end = pages_start + page_count * page_size;
        if pages_end > data.len() {
            return Err(Error::Corrupt(
                "ENCODING: CKey pages exceed the file".into(),
            ));
        }

        let mut first_keys = Vec::with_capacity(page_count);
        for i in 0..page_count {
            let header = &data[table_start + i * PAGE_HEADER_SIZE..][..PAGE_HEADER_SIZE];
            let first_key: [u8; 16] = header[..16].try_into().expect("16 bytes");
            let page = &data[pages_start + i * page_size..][..page_size];
            // "Check if the CKey matches with the expected first value"
            if page.len() < 6 + KEY_SIZE || page[6..6 + KEY_SIZE] != first_key {
                return Err(bad("CKey page does not start with its first key"));
            }
            first_keys.push(first_key);
        }
        Ok(Self {
            first_keys,
            pages: data[pages_start..pages_end].to_vec(),
            page_size,
        })
    }

    /// Iterates the entries of one page (`LoadEncodingCKeyPage`), yielding
    /// `(ckey, entry)`.
    fn page_entries(page: &[u8]) -> impl Iterator<Item = (&[u8], EncodingEntry)> {
        let mut pos = 0;
        std::iter::from_fn(move || {
            // FILE_CKEY_ENTRY header: USHORT EKeyCount (little endian, as
            // CascLib reads it), BYTE ContentSize[4] (big endian), CKey, EKeys.
            let header = page.get(pos..pos + 6 + KEY_SIZE + KEY_SIZE)?;
            let ekey_count = u16::from_le_bytes([header[0], header[1]]) as usize;
            if ekey_count == 0 {
                return None;
            }
            let entry_len = 2 + 4 + KEY_SIZE + ekey_count * KEY_SIZE;
            if pos + entry_len > page.len() {
                return None;
            }
            let ckey = &header[6..6 + KEY_SIZE];
            let entry = EncodingEntry {
                ekey: header[6 + KEY_SIZE..6 + 2 * KEY_SIZE]
                    .try_into()
                    .expect("16 bytes"),
                content_size: be(&header[2..6]),
            };
            pos += entry_len;
            Some((ckey, entry))
        })
    }

    /// `FindCKeyEntry_CKey`.
    pub fn find(&self, ckey: &[u8; 16]) -> Option<EncodingEntry> {
        let page_index = self
            .first_keys
            .partition_point(|k| k <= ckey)
            .checked_sub(1)?;
        let page = &self.pages[page_index * self.page_size..][..self.page_size];
        Self::page_entries(page)
            .find(|(k, _)| *k == ckey.as_slice())
            .map(|(_, e)| e)
    }

    pub fn page_count(&self) -> usize {
        self.first_keys.len()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds an ENCODING file with 1 KB `CKey` pages from sorted entries
    /// `(ckey, ekey, content size)`, `per_page` entries per page.
    type TestEntry = ([u8; 16], [u8; 16], u32);

    pub(crate) fn build_encoding(entries: &[TestEntry], per_page: usize) -> Vec<u8> {
        let espec = b"z\0";
        let pages: Vec<&[TestEntry]> = entries.chunks(per_page).collect();
        let mut out = b"EN".to_vec();
        out.extend_from_slice(&[1, 16, 16]);
        out.extend_from_slice(&1u16.to_be_bytes()); // CKey page size (KB)
        out.extend_from_slice(&1u16.to_be_bytes()); // EKey page size (KB)
        out.extend_from_slice(&(pages.len() as u32).to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.push(0);
        out.extend_from_slice(&(espec.len() as u32).to_be_bytes());
        out.extend_from_slice(espec);
        for page in &pages {
            out.extend_from_slice(&page[0].0);
            out.extend_from_slice(&[0u8; 16]);
        }
        for page in &pages {
            let mut p = Vec::new();
            for (ckey, ekey, size) in *page {
                p.push(1);
                p.push(0);
                p.extend_from_slice(&size.to_be_bytes());
                p.extend_from_slice(ckey);
                p.extend_from_slice(ekey);
            }
            p.resize(1024, 0);
            out.extend_from_slice(&p);
        }
        out
    }

    #[test]
    fn lookup_across_pages() {
        let entries: Vec<_> = (0u8..50)
            .map(|i| {
                let mut ckey = [0u8; 16];
                ckey[0] = i * 4 + 1;
                let mut ekey = [0xEE; 16];
                ekey[0] = i;
                (ckey, ekey, u32::from(i) * 1000)
            })
            .collect();
        let data = build_encoding(&entries, 20);
        let enc = Encoding::parse(&data).unwrap();
        assert_eq!(enc.page_count(), 3);
        for (ckey, ekey, size) in &entries {
            let e = enc.find(ckey).unwrap();
            assert_eq!(&e.ekey, ekey);
            assert_eq!(e.content_size, *size);
        }
        assert!(enc.find(&[0u8; 16]).is_none());
        assert!(enc.find(&[2u8; 16]).is_none());
        assert!(enc.find(&[0xFF; 16]).is_none());
    }

    #[test]
    fn rejects_bad_first_key() {
        let entries = [([1u8; 16], [2u8; 16], 5u32)];
        let mut data = build_encoding(&entries, 1);
        let table = HEADER_SIZE + 2;
        data[table] ^= 0xFF;
        assert!(Encoding::parse(&data).is_err());
    }
}
