//! Minimal `WoW` ROOT (`TSFM`) reader: the `CKeys` of given `FileDataIds`.
//!
//! Only used by the hidden `--include-fdid` test option to put specific game
//! files (e.g. `DBFilesClient\Map.db2`, 1349477) into a partial download.
//! Layouts as ported in `wow-casc` `root.rs` (`CascLib`
//! `TRootHandler_WoW::CaptureRootHeader_50893/30080/18125` and
//! `CaptureRootGroup`):
//!
//! * `TSFM` (8.2+): optional versioned header, then groups `{u32 count, u32
//!   content flags, u32 locale flags}` (17-byte header for version 2), `count`
//!   delta coded `FileDataIds`, `count` `CKeys` and, without `0x10000000` in
//!   the content flags, `count` name hashes;
//! * headerless (6.0+, used by the 3.4.3.54261 ROOT): the same group header
//!   and deltas followed by `count` interleaved `{CKey, name hash}` records.

use std::collections::HashSet;

use anyhow::{Result, bail, ensure};

use crate::util::Key;

const SIGNATURE: &[u8; 4] = b"TSFM";
const NO_NAME_HASH: u32 = 0x1000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootRecord {
    pub file_data_id: u32,
    pub locale_flags: u32,
    pub content_flags: u32,
    pub ckey: Key,
}

fn u32_at(data: &[u8], pos: usize) -> Result<u32> {
    match data.get(pos..pos + 4) {
        Some(b) => Ok(u32::from_le_bytes(b.try_into().expect("4 bytes"))),
        None => bail!("ROOT truncated at {pos:#x}"),
    }
}

/// Every record of the wanted `FileDataIds`, in file order.
pub fn find(data: &[u8], wanted: &HashSet<u32>) -> Result<Vec<RootRecord>> {
    let tsfm = data.get(..4) == Some(SIGNATURE);
    let (mut pos, group_header) = if tsfm {
        let header_size = u32_at(data, 4)? as usize;
        let version = u32_at(data, 8)?;
        if (version == 1 || version == 2) && (4..64).contains(&header_size) {
            (header_size, if version == 2 { 17 } else { 12 })
        } else {
            (12, 12)
        }
    } else {
        (0, 12)
    };
    // Per-entry stride of the CKey array: 16, or 24 for interleaved records.
    let stride = if tsfm { 16 } else { 24 };
    let mut out = Vec::new();
    while pos < data.len() {
        let count = u32_at(data, pos)? as usize;
        let (content_flags, locale_flags) = if group_header == 17 {
            let cf = u32_at(data, pos + 8)?
                | u32_at(data, pos + 12)?
                | (u32::from(*data.get(pos + 16).unwrap_or(&0)) << 17);
            (cf, u32_at(data, pos + 4)?)
        } else {
            (u32_at(data, pos + 4)?, u32_at(data, pos + 8)?)
        };
        pos += group_header;
        let ids = pos;
        let ckeys = ids + count * 4;
        pos = ckeys + count * stride;
        if tsfm && content_flags & NO_NAME_HASH == 0 {
            pos += count * 8;
        }
        ensure!(pos <= data.len(), "ROOT group exceeds the file");
        let mut fdid = 0u32;
        for i in 0..count {
            fdid = fdid.wrapping_add(u32_at(data, ids + i * 4)?);
            if wanted.contains(&fdid) {
                out.push(RootRecord {
                    file_data_id: fdid,
                    locale_flags,
                    content_flags,
                    ckey: data[ckeys + i * stride..ckeys + i * stride + 16]
                        .try_into()
                        .expect("16 bytes"),
                });
            }
            fdid = fdid.wrapping_add(1);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Version-1 `TSFM` root with groups of `(locale, content flags, [(fdid, ckey)])`.
    /// `(locale flags, content flags, [(fdid, ckey byte)])`.
    type Group = (u32, u32, Vec<(u32, u8)>);

    fn build(groups: &[Group]) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        for v in [24u32, 1, 0, 0, 0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for (locale, flags, files) in groups {
            out.extend_from_slice(&(files.len() as u32).to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&locale.to_le_bytes());
            let mut prev: Option<u32> = None;
            for (id, _) in files {
                let delta = prev.map_or(*id, |p| id - p - 1);
                out.extend_from_slice(&delta.to_le_bytes());
                prev = Some(*id);
            }
            for (_, k) in files {
                out.extend_from_slice(&[*k; 16]);
            }
            if flags & NO_NAME_HASH == 0 {
                out.extend(std::iter::repeat_n(0xAB, files.len() * 8));
            }
        }
        out
    }

    #[test]
    fn finds_records() {
        let data = build(&[
            (2, 0, vec![(10, 1), (1_349_477, 2)]),
            (0x80, NO_NAME_HASH, vec![(5, 3), (1_349_477, 4)]),
        ]);
        let wanted: HashSet<u32> = [1_349_477, 5].into();
        let found = find(&data, &wanted).unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].ckey, [2; 16]);
        assert_eq!(found[0].locale_flags, 2);
        assert_eq!(found[1].file_data_id, 5);
        assert_eq!(found[2].ckey, [4; 16]);
        assert!(
            find(
                b"TSFM\x18\0\0\0\x01\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\x09\0\0\0",
                &wanted
            )
            .is_err()
        );
    }

    #[test]
    fn headerless_interleaved_layout() {
        // One group: count 2, content flags, locale enUS, deltas, {CKey, hash} x2.
        let mut data = Vec::new();
        for v in [2u32, 0x80, 2, 1_349_476, 0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        for k in [7u8, 8] {
            data.extend_from_slice(&[k; 16]);
            data.extend_from_slice(&[0xAB; 8]);
        }
        let found = find(&data, &[1_349_477u32].into()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].ckey, [8; 16]);
        assert_eq!(found[0].content_flags, 0x80);
    }
}
