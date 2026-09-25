//! INSTALL (`IN`) and DOWNLOAD (`DL`) manifests with their tag bitmasks.
//!
//! Layouts per wowdev `TACT#Install_manifest`/`#Download_manifest` and
//! blizzget `data.cpp` (`ProgramData::loadTags`, the `'DL'`/`'IN'` readers of
//! `DownloadTask::run`):
//!
//! * install: `IN`, version, hash size, `u16` tag count, `u32` entry count,
//!   tags, then entries `{name\0, CKey, u32 size}`.
//! * download: `DL`, version, `EKey` size, checksum flag, `u32` entry count,
//!   `u16` tag count, [v2+: flag byte count], [v3+: 4 more bytes], entries
//!   `{EKey, u40 encoded size, u8 priority, [u32 checksum], [flags]}`, then tags.
//!
//! A tag is `{name\0, u16 type, bitmask}` with one bit per entry, most
//! significant bit first. All integers are big-endian.

use anyhow::{Result, ensure};

use crate::util::{Key, be_uint};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub kind: u16,
    pub mask: Vec<u8>,
}

impl Tag {
    /// Whether entry `index` carries this tag.
    pub fn has(&self, index: usize) -> bool {
        self.mask
            .get(index / 8)
            .is_some_and(|b| b & (0x80 >> (index % 8)) != 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallEntry {
    /// Relative path with `\` separators, as in the manifest.
    pub name: String,
    pub ckey: Key,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallManifest {
    pub tags: Vec<Tag>,
    pub entries: Vec<InstallEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadEntry {
    pub ekey: Key,
    /// Encoded (BLTE) size.
    pub size: u64,
    pub priority: i8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadManifest {
    pub tags: Vec<Tag>,
    pub entries: Vec<DownloadEntry>,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        let end = end.ok_or_else(|| anyhow::anyhow!("manifest truncated at {:#x}", self.pos))?;
        let out = &self.data[self.pos..end];
        self.pos = end;
        Ok(out)
    }
    fn uint(&mut self, n: usize) -> Result<u64> {
        Ok(be_uint(self.bytes(n)?))
    }
    fn cstr(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let len = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| anyhow::anyhow!("unterminated string at {:#x}", self.pos))?;
        let s = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.pos += len + 1;
        Ok(s)
    }
    fn tags(&mut self, count: usize, entries: usize) -> Result<Vec<Tag>> {
        let mask_len = entries.div_ceil(8);
        (0..count)
            .map(|_| {
                Ok(Tag {
                    name: self.cstr()?,
                    kind: self.uint(2)? as u16,
                    mask: self.bytes(mask_len)?.to_vec(),
                })
            })
            .collect()
    }
}

impl InstallManifest {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader { data, pos: 0 };
        ensure!(r.bytes(2)? == b"IN", "not an install manifest");
        let _version = r.uint(1)?;
        let hash_size = r.uint(1)? as usize;
        ensure!(hash_size == 16, "install manifest: hash size {hash_size}");
        let tag_count = r.uint(2)? as usize;
        let entry_count = r.uint(4)? as usize;
        let tags = r.tags(tag_count, entry_count)?;
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            entries.push(InstallEntry {
                name: r.cstr()?,
                ckey: r.bytes(16)?.try_into().expect("16 bytes"),
                size: r.uint(4)? as u32,
            });
        }
        Ok(Self { tags, entries })
    }
}

impl DownloadManifest {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader { data, pos: 0 };
        ensure!(r.bytes(2)? == b"DL", "not a download manifest");
        let version = r.uint(1)?;
        let ekey_size = r.uint(1)? as usize;
        ensure!(ekey_size == 16, "download manifest: EKey size {ekey_size}");
        let has_checksum = r.uint(1)? != 0;
        let entry_count = r.uint(4)? as usize;
        let tag_count = r.uint(2)? as usize;
        let mut flag_bytes = 0;
        if version >= 2 {
            flag_bytes = r.uint(1)? as usize;
            if version >= 3 {
                r.bytes(4)?;
            }
        }
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let ekey: Key = r.bytes(16)?.try_into().expect("16 bytes");
            let size = r.uint(5)?;
            let priority = (r.uint(1)? as u8).cast_signed();
            if has_checksum {
                r.bytes(4)?;
            }
            r.bytes(flag_bytes)?;
            entries.push(DownloadEntry {
                ekey,
                size,
                priority,
            });
        }
        let tags = r.tags(tag_count, entry_count)?;
        Ok(Self { tags, entries })
    }
}

/// Test builders producing the on-disk formats.
#[cfg(test)]
pub mod build {
    use super::Tag;

    fn tags(out: &mut Vec<u8>, tags: &[Tag]) {
        for t in tags {
            out.extend_from_slice(t.name.as_bytes());
            out.push(0);
            out.extend_from_slice(&t.kind.to_be_bytes());
            out.extend_from_slice(&t.mask);
        }
    }

    /// Tag whose mask has the given entry indices set.
    pub fn tag(name: &str, kind: u16, entries: usize, set: &[usize]) -> Tag {
        let mut mask = vec![0u8; entries.div_ceil(8)];
        for &i in set {
            mask[i / 8] |= 0x80 >> (i % 8);
        }
        Tag {
            name: name.to_owned(),
            kind,
            mask,
        }
    }

    pub fn install(t: &[Tag], entries: &[(&str, [u8; 16], u32)]) -> Vec<u8> {
        let mut out = b"IN\x01\x10".to_vec();
        out.extend_from_slice(&(t.len() as u16).to_be_bytes());
        out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        tags(&mut out, t);
        for (name, ckey, size) in entries {
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(ckey);
            out.extend_from_slice(&size.to_be_bytes());
        }
        out
    }

    /// Version-3 download manifest with checksums and one flag byte.
    pub fn download(t: &[Tag], entries: &[([u8; 16], u64)]) -> Vec<u8> {
        let mut out = b"DL\x03\x10\x01".to_vec();
        out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        out.extend_from_slice(&(t.len() as u16).to_be_bytes());
        out.push(1);
        out.extend_from_slice(&[0, 0, 0, 0]);
        for (ekey, size) in entries {
            out.extend_from_slice(ekey);
            out.extend_from_slice(&size.to_be_bytes()[3..]);
            out.push(0xFE);
            out.extend_from_slice(&[9, 9, 9, 9]);
            out.push(0x55);
        }
        tags(&mut out, t);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;

    #[test]
    fn install_round_trip() {
        let t = vec![tag("Windows", 1, 3, &[0, 2]), tag("OSX", 1, 3, &[1])];
        let data = install(
            &t,
            &[
                ("WowClassic.exe", [1; 16], 100),
                (
                    "World of Warcraft Classic.app\\Contents\\PkgInfo",
                    [2; 16],
                    8,
                ),
                ("Utils\\x.dll", [3; 16], 5),
            ],
        );
        let m = InstallManifest::parse(&data).unwrap();
        assert_eq!(m.tags, t);
        assert_eq!(m.entries.len(), 3);
        assert_eq!(m.entries[1].size, 8);
        assert_eq!(m.entries[2].ckey, [3; 16]);
        assert!(m.tags[0].has(2) && !m.tags[0].has(1) && !m.tags[0].has(99));
        assert!(InstallManifest::parse(&data[..data.len() - 1]).is_err());
    }

    #[test]
    fn download_round_trip() {
        let t = vec![tag("enUS", 3, 2, &[1])];
        let data = download(&t, &[([7; 16], 0x12_3456_789A), ([8; 16], 30)]);
        let m = DownloadManifest::parse(&data).unwrap();
        assert_eq!(m.entries[0].size, 0x12_3456_789A);
        assert_eq!(m.entries[0].priority, -2);
        assert_eq!(m.entries[1].ekey, [8; 16]);
        assert_eq!(m.tags, t);
        assert!(DownloadManifest::parse(b"IN").is_err());
    }

    #[test]
    fn download_v1_layout() {
        // Version 1 (the 54261 manifest): no flag bytes, no checksum.
        let mut out = b"DL\x01\x10\x00".to_vec();
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&[5; 16]);
        out.extend_from_slice(&[0, 0, 0, 0x10, 0]);
        out.push(0);
        out.extend_from_slice(b"text\0\x00\x05\x80");
        let m = DownloadManifest::parse(&out).unwrap();
        assert_eq!(m.entries[0].size, 0x1000);
        assert!(m.tags[0].has(0));
        assert_eq!(m.tags[0].kind, 5);
    }
}
