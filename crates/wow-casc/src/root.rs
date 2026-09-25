//! World of Warcraft ROOT manifest (`FileDataId` / name hash -> `CKey`).
//!
//! Port of `CascLib` `dep/CascLib/src/CascRootFile_WoW.cpp`
//! (`TRootHandler_WoW::CaptureRootHeader_50893`, `CaptureRootHeader_30080`,
//! `CaptureRootHeader_18125`, `CaptureRootGroup`, `ParseWowRootFile_AddFiles_v1/v2`,
//! `ParseWowRootFile_Level2`, `ParseWowRootFile_Level1`, `Load`) together with
//! the lookup semantics of `common/FileTree.cpp` (`InsertById`: the first
//! entry inserted for a `FileDataId` wins; `InsertByHash` / `NameMap`: the first
//! node inserted for a name hash wins) and `common/RootHandler.cpp`
//! (`TFileTreeRoot::GetFile`).
//!
//! `CascLib` filters the root entries once, at storage-open time, with the open
//! locale mask. To also serve per-call locale masks, every entry that survives
//! the mask-independent filters (`CASC_CFLAG_DONT_LOAD`, `CASC_CFLAG_LOW_VIOLENCE`)
//! is kept, and [`WowRoot::select`] replays `CascLib`'s load order for a given
//! mask: audio pass 0 then 1 (`ContentFlags >> 31`), each pass with the mask
//! and then the enGB -> enUS / ptPT -> ptBR fallback, entries in file order.
//!
//! Extension beyond the `TrinityCore` `TDB343.24081` `CascLib` copy: the `TSFM`
//! header with `Version == 2` (`WoW` 11.1.0 build 58221+ and the 2025+ Classic
//! clients), whose root groups have a 17-byte header
//! `{NumberOfFiles, LocaleFlags, ContentFlags1, ContentFlags2, u8 ContentFlags3}`
//! with `ContentFlags = ContentFlags1 | ContentFlags2 | (ContentFlags3 << 17)`
//! (layout of upstream `CascLib` `FILE_ROOT_GROUP_HEADER_58221` / wowdev "Root").
//! The `TrinityCore` copy only accepts `Version == 1`. The layout was verified
//! against a real 1.60.1.70009 ROOT: all 1182 groups parse to the exact end of
//! the file and the file/name-hash totals equal the header counters.

use std::collections::HashMap;

use crate::locale;
use crate::{Error, Result};

/// `CASC_WOW_ROOT_SIGNATURE` ('TSFM' on disk).
const WOW_ROOT_SIGNATURE: u32 = 0x4D46_5354;
/// `CASC_CFLAG_LOW_VIOLENCE`.
pub const CFLAG_LOW_VIOLENCE: u32 = 0x80;
/// `CASC_CFLAG_DONT_LOAD`.
pub const CFLAG_DONT_LOAD: u32 = 0x100;
/// `CASC_CFLAG_NO_NAME_HASH`.
pub const CFLAG_NO_NAME_HASH: u32 = 0x1000_0000;
/// `sizeof(FILE_ROOT_GROUP_HEADER)`.
const GROUP_HEADER_SIZE: usize = 12;
/// Group header size of `TSFM` version 2 (build 58221+).
const GROUP_HEADER_SIZE_V2_HEADER: usize = 17;
/// `sizeof(FILE_ROOT_ENTRY)` (v1: `CKey` + name hash).
const ROOT_ENTRY_V1_SIZE: usize = 24;

/// `CascLib` `ROOT_FORMAT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootFormat {
    /// Since build 18125 (6.0.1): no header, interleaved `CKey` + name hash.
    V1,
    /// Since build 30080 (8.2.0, `TSFM` header) and 50893 (10.1.7, versioned
    /// `TSFM` header): separate `CKey` and optional name-hash arrays.
    V2,
}

/// One root entry kept after the mask-independent filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootRecord {
    pub file_data_id: u32,
    pub locale_flags: u32,
    pub content_flags: u32,
    pub ckey: [u8; 16],
}

impl RootRecord {
    /// `ParseWowRootFile_Level2` locale/audio check for one pass.
    fn matches(&self, audio: u32, mask: u32) -> bool {
        (self.content_flags >> 0x1F) == audio
            && (self.locale_flags == 0 || (self.locale_flags & mask) != 0)
    }
}

#[derive(Debug)]
pub struct WowRoot {
    pub format: RootFormat,
    /// Records sorted by `FileDataId`; records of one `FileDataId` stay in file order.
    records: Vec<RootRecord>,
    /// Name hash -> `FileDataId`, first node inserted in `CascLib` load order.
    names: HashMap<u64, u32>,
}

fn read_u32(data: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes(data[pos..pos + 4].try_into().expect("bounds checked"))
}

/// Layout of a root file: format, offset of the first group and size of the
/// group headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RootLayout {
    format: RootFormat,
    first_group: usize,
    group_header_size: usize,
}

/// `TRootHandler_WoW::CaptureRootHeader`.
fn capture_root_header(data: &[u8]) -> Option<RootLayout> {
    // CaptureRootHeader_50893
    if 20 < data.len() {
        let signature = read_u32(data, 0);
        let mut size_of_header = read_u32(data, 4) as usize;
        let version = read_u32(data, 8);
        let total = read_u32(data, 12);
        let named = read_u32(data, 16);
        // Version 2 is the build-58221 extension (see the module docs).
        if signature == WOW_ROOT_SIGNATURE && (version == 1 || version == 2) && named <= total {
            // "wow client doesn't seem to think this is a fatal error"
            if size_of_header < 4 {
                size_of_header = 4;
            }
            return Some(RootLayout {
                format: RootFormat::V2,
                first_group: size_of_header,
                group_header_size: if version == 2 {
                    GROUP_HEADER_SIZE_V2_HEADER
                } else {
                    GROUP_HEADER_SIZE
                },
            });
        }
    }
    // CaptureRootHeader_30080
    if 12 < data.len() {
        let signature = read_u32(data, 0);
        let total = read_u32(data, 4);
        let named = read_u32(data, 8);
        if signature == WOW_ROOT_SIGNATURE && named <= total {
            return Some(RootLayout {
                format: RootFormat::V2,
                first_group: 12,
                group_header_size: GROUP_HEADER_SIZE,
            });
        }
    }
    // CaptureRootHeader_18125: no header, validate the first group.
    if GROUP_HEADER_SIZE < data.len() {
        let count = read_u32(data, 0) as usize;
        let length = count.checked_mul(4 + ROOT_ENTRY_V1_SIZE)?;
        if GROUP_HEADER_SIZE.checked_add(length)? < data.len() {
            return Some(RootLayout {
                format: RootFormat::V1,
                first_group: 0,
                group_header_size: GROUP_HEADER_SIZE,
            });
        }
    }
    None
}

/// Parsed `FILE_ROOT_GROUP`.
struct RootGroup<'a> {
    content_flags: u32,
    locale_flags: u32,
    file_data_ids: &'a [u8],
    /// v2: `CKeys`; v1: interleaved `FILE_ROOT_ENTRY`s.
    entries: &'a [u8],
    /// v2 only, absent with `CASC_CFLAG_NO_NAME_HASH`.
    hashes: Option<&'a [u8]>,
    count: usize,
}

/// `TRootHandler_WoW::CaptureRootGroup`. Returns the group and the offset of
/// the next one.
fn capture_root_group(
    data: &[u8],
    mut pos: usize,
    layout: RootLayout,
) -> Option<(RootGroup<'_>, usize)> {
    let end = data.len();
    if pos + layout.group_header_size >= end {
        return None;
    }
    let count = read_u32(data, pos) as usize;
    let (content_flags, locale_flags) = if layout.group_header_size == GROUP_HEADER_SIZE_V2_HEADER {
        // {NumberOfFiles, LocaleFlags, ContentFlags1, ContentFlags2, ContentFlags3}
        let locale_flags = read_u32(data, pos + 4);
        let content_flags =
            read_u32(data, pos + 8) | read_u32(data, pos + 12) | (u32::from(data[pos + 16]) << 17);
        (content_flags, locale_flags)
    } else {
        // FILE_ROOT_GROUP_HEADER {NumberOfFiles, ContentFlags, LocaleFlags}
        (read_u32(data, pos + 4), read_u32(data, pos + 8))
    };
    pos += layout.group_header_size;

    let ids_len = count.checked_mul(4)?;
    if pos.checked_add(ids_len)? >= end {
        return None;
    }
    let file_data_ids = &data[pos..pos + ids_len];
    pos += ids_len;

    match layout.format {
        RootFormat::V2 => {
            let ckeys_len = count.checked_mul(16)?;
            if pos.checked_add(ckeys_len)? > end {
                return None;
            }
            let entries = &data[pos..pos + ckeys_len];
            pos += ckeys_len;
            let mut hashes = None;
            if content_flags & CFLAG_NO_NAME_HASH == 0 {
                let hashes_len = count.checked_mul(8)?;
                if pos.checked_add(hashes_len)? > end {
                    return None;
                }
                hashes = Some(&data[pos..pos + hashes_len]);
                pos += hashes_len;
            }
            Some((
                RootGroup {
                    content_flags,
                    locale_flags,
                    file_data_ids,
                    entries,
                    hashes,
                    count,
                },
                pos,
            ))
        }
        RootFormat::V1 => {
            let entries_len = count.checked_mul(ROOT_ENTRY_V1_SIZE)?;
            if pos.checked_add(entries_len)? > end {
                return None;
            }
            let entries = &data[pos..pos + entries_len];
            pos += entries_len;
            Some((
                RootGroup {
                    content_flags,
                    locale_flags,
                    file_data_ids,
                    entries,
                    hashes: None,
                    count,
                },
                pos,
            ))
        }
    }
}

/// The locale masks of one `ParseWowRootFile_Level1` call, in order.
fn level1_masks(mask: u32) -> impl Iterator<Item = u32> {
    // "If we wanted enGB, we also load enUS for the missing files"
    let extra = match mask {
        locale::ENGB => Some(locale::ENUS),
        locale::PTPT => Some(locale::PTBR),
        _ => None,
    };
    std::iter::once(mask).chain(extra)
}

/// `TRootHandler_WoW::Load` pass order: `(audio, mask)` pairs.
fn load_passes(mask: u32) -> impl Iterator<Item = (u32, u32)> {
    (0..2u32).flat_map(move |audio| level1_masks(mask).map(move |m| (audio, m)))
}

impl WowRoot {
    /// `RootHandler_CreateWoW`: parses the whole manifest. `open_mask` is the
    /// storage-open locale mask (used for the name-hash map, as `CascLib` only
    /// registers names of the entries it loaded).
    pub fn parse(data: &[u8], open_mask: u32) -> Result<Self> {
        let layout = capture_root_header(data)
            .ok_or_else(|| Error::Corrupt("ROOT: unrecognized WoW root header".into()))?;
        let format = layout.format;
        let mut pos = layout.first_group;

        let mut records = Vec::new();
        let mut hashes: Vec<u64> = Vec::new();
        while pos < data.len() {
            let (group, next) = capture_root_group(data, pos, layout).ok_or_else(|| {
                Error::Corrupt(format!("ROOT: bad root group at offset {pos:#x}"))
            })?;
            pos = next;

            // "Entries with flag 0x100 set are skipped"
            if group.content_flags & CFLAG_DONT_LOAD != 0 {
                continue;
            }
            // "Entries with flag 0x80 set are skipped if overrideArchive CVAR is
            // set to FALSE" (CascLib always passes bOverrideLowViolence = false)
            if group.content_flags & CFLAG_LOW_VIOLENCE != 0 {
                continue;
            }

            // ParseWowRootFile_AddFiles_v1/v2: FileDataId delta coding
            let mut file_data_id: u32 = 0;
            for i in 0..group.count {
                file_data_id = file_data_id.wrapping_add(read_u32(group.file_data_ids, i * 4));
                let (ckey, hash) = match format {
                    RootFormat::V2 => {
                        let ckey: [u8; 16] = group.entries[i * 16..i * 16 + 16]
                            .try_into()
                            .expect("16 bytes");
                        let hash = group.hashes.map_or(0, |h| {
                            u64::from_le_bytes(h[i * 8..i * 8 + 8].try_into().expect("8 bytes"))
                        });
                        (ckey, hash)
                    }
                    RootFormat::V1 => {
                        let e = &group.entries[i * ROOT_ENTRY_V1_SIZE..][..ROOT_ENTRY_V1_SIZE];
                        let ckey: [u8; 16] = e[..16].try_into().expect("16 bytes");
                        let hash = u64::from_le_bytes(e[16..24].try_into().expect("8 bytes"));
                        (ckey, hash)
                    }
                };
                records.push(RootRecord {
                    file_data_id,
                    locale_flags: group.locale_flags,
                    content_flags: group.content_flags,
                    ckey,
                });
                hashes.push(hash);
                file_data_id = file_data_id.wrapping_add(1);
            }
        }

        // Name hash -> FileDataId in CascLib load order for the open mask.
        let mut names = HashMap::new();
        for (audio, mask) in load_passes(open_mask) {
            for (record, &hash) in records.iter().zip(&hashes) {
                if hash != 0 && record.matches(audio, mask) {
                    names.entry(hash).or_insert(record.file_data_id);
                }
            }
        }
        drop(hashes);

        // Stable sort keeps file order among the entries of one FileDataId.
        records.sort_by_key(|r| r.file_data_id);
        records.shrink_to_fit();
        Ok(Self {
            format,
            records,
            names,
        })
    }

    /// All kept entries of `file_data_id`, in file order.
    pub fn records_of(&self, file_data_id: u32) -> &[RootRecord] {
        let start = self
            .records
            .partition_point(|r| r.file_data_id < file_data_id);
        let end = self.records[start..].partition_point(|r| r.file_data_id == file_data_id);
        &self.records[start..start + end]
    }

    /// The entry `CascLib`'s file tree would hold for `file_data_id` if the
    /// storage had been opened with `mask`: the first entry in `Load` pass
    /// order whose `CKey` is known (`FindCKeyEntry_CKey`, via `ckey_known`).
    pub fn select(
        &self,
        file_data_id: u32,
        mask: u32,
        ckey_known: impl Fn(&[u8; 16]) -> bool,
    ) -> Option<&RootRecord> {
        let candidates = self.records_of(file_data_id);
        if candidates.is_empty() {
            return None;
        }
        load_passes(mask).find_map(|(audio, m)| {
            candidates
                .iter()
                .find(|r| r.matches(audio, m) && ckey_known(&r.ckey))
        })
    }

    /// Name hash -> `FileDataId` (`CASC_FILE_TREE::Find(FileNameHash)`).
    pub fn file_data_id_by_hash(&self, name_hash: u64) -> Option<u32> {
        self.names.get(&name_hash).copied()
    }

    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    pub fn name_count(&self) -> usize {
        self.names.len()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) struct Group {
        pub content: u32,
        pub locale: u32,
        /// `(file data id, ckey, name hash)`, ids ascending.
        pub files: Vec<(u32, [u8; 16], u64)>,
    }

    fn deltas(files: &[(u32, [u8; 16], u64)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut next = 0u32;
        for (id, _, _) in files {
            out.extend_from_slice(&id.wrapping_sub(next).to_le_bytes());
            next = id + 1;
        }
        out
    }

    /// `header_version`: 0 = 8.2.0 `TSFM` header, 1 / 2 = versioned header.
    pub(crate) fn build_root_v2(groups: &[Group], header_version: u32) -> Vec<u8> {
        let total: usize = groups.iter().map(|g| g.files.len()).sum();
        let mut out = b"TSFM".to_vec();
        if header_version != 0 {
            out.extend_from_slice(&24u32.to_le_bytes()); // SizeOfHeader
            out.extend_from_slice(&header_version.to_le_bytes()); // Version
            out.extend_from_slice(&(total as u32).to_le_bytes());
            out.extend_from_slice(&(total as u32).to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes()); // padding up to SizeOfHeader
        } else {
            out.extend_from_slice(&(total as u32).to_le_bytes());
            out.extend_from_slice(&(total as u32).to_le_bytes());
        }
        for g in groups {
            out.extend_from_slice(&(g.files.len() as u32).to_le_bytes());
            if header_version == 2 {
                // Split the flags over the three fields like the real files do.
                out.extend_from_slice(&g.locale.to_le_bytes());
                out.extend_from_slice(&(g.content & 0x00FF_FFFF).to_le_bytes());
                out.extend_from_slice(&(g.content & 0xFF00_0000).to_le_bytes());
                out.push(((g.content >> 17) & 0x7F) as u8);
            } else {
                out.extend_from_slice(&g.content.to_le_bytes());
                out.extend_from_slice(&g.locale.to_le_bytes());
            }
            out.extend_from_slice(&deltas(&g.files));
            for (_, ckey, _) in &g.files {
                out.extend_from_slice(ckey);
            }
            if g.content & CFLAG_NO_NAME_HASH == 0 {
                for (_, _, hash) in &g.files {
                    out.extend_from_slice(&hash.to_le_bytes());
                }
            }
        }
        out
    }

    fn build_root_v1(groups: &[Group]) -> Vec<u8> {
        let mut out = Vec::new();
        for g in groups {
            out.extend_from_slice(&(g.files.len() as u32).to_le_bytes());
            out.extend_from_slice(&g.content.to_le_bytes());
            out.extend_from_slice(&g.locale.to_le_bytes());
            out.extend_from_slice(&deltas(&g.files));
            for (_, ckey, hash) in &g.files {
                out.extend_from_slice(ckey);
                out.extend_from_slice(&hash.to_le_bytes());
            }
        }
        out
    }

    fn ck(b: u8) -> [u8; 16] {
        [b; 16]
    }

    fn sample_groups() -> Vec<Group> {
        vec![
            // koKR variant first in file order
            Group {
                content: 0,
                locale: locale::KOKR,
                files: vec![(100, ck(1), 0xAAAA), (105, ck(2), 0)],
            },
            // enUS variant
            Group {
                content: 0,
                locale: locale::ENUS,
                files: vec![(100, ck(3), 0xAAAA)],
            },
            // audio-locale group (bit 31): loaded in the second pass only
            Group {
                content: 0x8000_0000 | CFLAG_NO_NAME_HASH,
                locale: locale::ENGB,
                files: vec![(100, ck(4), 0), (200, ck(5), 0)],
            },
            // skipped groups
            Group {
                content: CFLAG_LOW_VIOLENCE,
                locale: 0,
                files: vec![(300, ck(6), 0xBBBB)],
            },
            Group {
                content: CFLAG_DONT_LOAD,
                locale: 0,
                files: vec![(301, ck(7), 0xCCCC)],
            },
            // locale-neutral group
            Group {
                content: 0,
                locale: 0,
                files: vec![(1_349_477, ck(8), 0x1234_5678_9ABC_DEF0)],
            },
        ]
    }

    #[test]
    fn selection_follows_casclib_load_order() {
        for header_version in [0, 1, 2] {
            let data = build_root_v2(&sample_groups(), header_version);
            let root = WowRoot::parse(&data, locale::ALL).unwrap();
            assert_eq!(root.format, RootFormat::V2);
            let any = |_: &[u8; 16]| true;

            // ALL: first in file order among audio pass 0.
            assert_eq!(root.select(100, locale::ALL, any).unwrap().ckey, ck(1));
            assert_eq!(root.select(100, locale::ENUS, any).unwrap().ckey, ck(3));
            // enGB exact: pass (0, enGB) finds nothing, (0, enUS) fallback wins
            // over the audio group of pass (1, enGB).
            assert_eq!(root.select(100, locale::ENGB, any).unwrap().ckey, ck(3));
            // Only the audio group has id 200.
            assert_eq!(root.select(200, locale::ENGB, any).unwrap().ckey, ck(5));
            assert!(root.select(200, locale::ENUS, any).is_none());
            // CKey unknown to ENCODING -> next candidate.
            assert_eq!(
                root.select(100, locale::ALL, |k| *k != ck(1)).unwrap().ckey,
                ck(3)
            );
            // Skipped content flags.
            assert!(root.records_of(300).is_empty());
            assert!(root.records_of(301).is_empty());
            // Locale-neutral entries match any mask, even 0.
            assert_eq!(
                root.select(1_349_477, locale::FRFR, any).unwrap().ckey,
                ck(8)
            );
            assert_eq!(root.select(1_349_477, 0, any).unwrap().ckey, ck(8));
            // FileDataId delta decoding.
            assert_eq!(root.records_of(105).len(), 1);

            assert_eq!(root.file_data_id_by_hash(0xAAAA), Some(100));
            assert_eq!(
                root.file_data_id_by_hash(0x1234_5678_9ABC_DEF0),
                Some(1_349_477)
            );
            assert_eq!(root.file_data_id_by_hash(0xBBBB), None);
        }
    }

    #[test]
    fn name_map_respects_open_mask() {
        let data = build_root_v2(&sample_groups(), 0);
        let root = WowRoot::parse(&data, locale::FRFR).unwrap();
        // 0xAAAA only exists in koKR / enUS groups, not loaded for frFR.
        assert_eq!(root.file_data_id_by_hash(0xAAAA), None);
        assert_eq!(
            root.file_data_id_by_hash(0x1234_5678_9ABC_DEF0),
            Some(1_349_477)
        );
    }

    #[test]
    fn legacy_v1_root() {
        let groups = vec![
            Group {
                content: 0,
                locale: locale::ENUS,
                files: vec![(10, ck(1), 0x11), (12, ck(2), 0x22)],
            },
            Group {
                content: 0,
                locale: locale::DEDE,
                files: vec![(10, ck(3), 0x11)],
            },
        ];
        let data = build_root_v1(&groups);
        let root = WowRoot::parse(&data, locale::ALL).unwrap();
        assert_eq!(root.format, RootFormat::V1);
        let any = |_: &[u8; 16]| true;
        assert_eq!(root.select(10, locale::DEDE, any).unwrap().ckey, ck(3));
        assert_eq!(root.select(12, locale::ALL, any).unwrap().ckey, ck(2));
        assert_eq!(root.file_data_id_by_hash(0x22), Some(12));
    }

    #[test]
    fn truncated_root_is_rejected() {
        let mut data = build_root_v2(&sample_groups(), 0);
        data.truncate(data.len() - 3);
        assert!(WowRoot::parse(&data, locale::ALL).is_err());
    }
}
