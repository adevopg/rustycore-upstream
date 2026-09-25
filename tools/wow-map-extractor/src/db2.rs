//! DB2 (WDC4) support for the extractor: the parts of TrinityCore
//! `src/common/DataStores/DB2FileLoader.cpp` that `map_extractor` relies on.
//!
//! * [`check_headers`] ports `DB2FileLoader::LoadHeaders` validation (used by
//!   `ExtractDB2File` with `loadInfo == nullptr` and by `TryLoadDB2`).
//! * [`rewrite_known_tact_ids`] ports the header copy of `ExtractDB2File`, which
//!   replaces the `TactId` of every section whose key is known with
//!   `DUMMY_KNOWN_TACT_ID`.
//! * [`Db2Table::load`] ports `TryLoadDB2` → `DB2FileLoader::Load` for the few tables the
//!   extractor reads (`Map`, `LiquidMaterial`, `LiquidObject`, `LiquidType`,
//!   `CinematicCamera`): layout-hash / field-count / size checks from the
//!   `ExtractorDB2LoadInfo.h` metadata, encrypted-section skipping, the copy table and
//!   `DB2FileLoaderRegularImpl::RecordGetString`. Numeric fields and record ids come from
//!   `wow_data::wdc4::Wdc4Reader`.
//!
//! Key checks use `wow_casc::Storage::has_tact_key` (`CASC::Storage::HasTactKey`),
//! passed in as a predicate so parsing stays independent of the storage.

use wow_data::wdc4::Wdc4Reader;

/// `DUMMY_KNOWN_TACT_ID` (DB2FileLoader.h): "TRINITY".
pub(crate) const DUMMY_KNOWN_TACT_ID: u64 = 0x5452_494E_4954_5900;

const WDC4_SIGNATURE: u32 = 0x3443_4457; // 'WDC4'
const HEADER_SIZE: usize = 72; // sizeof(DB2Header)
const SECTION_HEADER_SIZE: usize = 40; // sizeof(DB2SectionHeader)
const FIELD_ENTRY_SIZE: usize = 4; // sizeof(DB2FieldEntry)
const COLUMN_META_SIZE: usize = 24; // sizeof(DB2ColumnMeta)

// DB2ColumnCompression
const COMPRESSION_NONE: u32 = 0;
const COMPRESSION_IMMEDIATE: u32 = 1;
const COMPRESSION_COMMON_DATA: u32 = 2;
const COMPRESSION_PALLET: u32 = 3;
const COMPRESSION_PALLET_ARRAY: u32 = 4;
const COMPRESSION_SIGNED_IMMEDIATE: u32 = 5;

/// `DB2Meta` subset from `ExtractorDB2LoadInfo.h` (all five tables have
/// `IndexField == -1` and `ParentIndexField == -1`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Db2Meta {
    /// Name used in messages (`"Map.db2"`).
    pub(crate) name: &'static str,
    pub(crate) file_data_id: u32,
    pub(crate) layout_hash: u32,
    /// `DB2Meta::FieldCount` (physical file fields).
    pub(crate) field_count: u32,
}

fn rd_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn rd_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn rd_u64(b: &[u8], at: usize) -> u64 {
    u64::from(rd_u32(b, at)) | (u64::from(rd_u32(b, at + 4)) << 32)
}

/// `DB2Header` fields used here.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Header {
    pub(crate) record_count: u32,
    pub(crate) field_count: u32,
    pub(crate) record_size: u32,
    pub(crate) string_table_size: u32,
    pub(crate) layout_hash: u32,
    pub(crate) flags: u16,
    pub(crate) total_field_count: u32,
    pub(crate) packed_data_offset: u32,
    pub(crate) parent_lookup_count: u32,
    pub(crate) column_meta_size: u32,
    pub(crate) section_count: u32,
}

/// `DB2SectionHeader`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Section {
    pub(crate) tact_id: u64,
    pub(crate) file_offset: u32,
    pub(crate) record_count: u32,
    pub(crate) string_table_size: u32,
    pub(crate) id_table_size: u32,
    pub(crate) parent_lookup_data_size: u32,
    pub(crate) copy_table_count: u32,
}

/// `DB2ColumnMeta` fields used here.
#[derive(Debug, Clone, Copy)]
struct ColumnMeta {
    bit_offset: u16,
    additional_data_size: u32,
    compression: u32,
    /// `CompressionData.immediate.BitOffset` / `CompressionData.pallet.BitOffset`.
    packed_bit_offset: u32,
}

/// Everything `DB2FileLoader::LoadHeaders` reads.
#[derive(Debug, Clone)]
pub(crate) struct Headers {
    pub(crate) header: Header,
    pub(crate) sections: Vec<Section>,
    columns: Vec<ColumnMeta>,
    /// Read position after the header blocks (`source->GetPosition()` after LoadHeaders).
    end: usize,
}

/// Port of `DB2FileLoader::LoadHeaders`; `file_name` is `DB2CascFileSource::GetFileName`
/// (`"FileDataId: <id>"`). The error string is the C++ exception's `what()`.
#[allow(clippy::too_many_lines)]
pub(crate) fn check_headers(
    bytes: &[u8],
    file_name: &str,
    meta: Option<&Db2Meta>,
) -> Result<Headers, String> {
    if bytes.len() < HEADER_SIZE {
        return Err("Failed to read header".to_owned());
    }
    let signature = rd_u32(bytes, 0);
    let header = Header {
        record_count: rd_u32(bytes, 4),
        field_count: rd_u32(bytes, 8),
        record_size: rd_u32(bytes, 12),
        string_table_size: rd_u32(bytes, 16),
        layout_hash: rd_u32(bytes, 24),
        flags: rd_u16(bytes, 40),
        total_field_count: rd_u32(bytes, 44),
        packed_data_offset: rd_u32(bytes, 48),
        parent_lookup_count: rd_u32(bytes, 52),
        column_meta_size: rd_u32(bytes, 56),
        section_count: rd_u32(bytes, 68),
    };

    if signature != WDC4_SIGNATURE {
        let c = signature.to_le_bytes();
        return Err(format!(
            "Incorrect file signature in {file_name}, expected 'WDC4', got {}{}{}{}",
            char::from(c[0]),
            char::from(c[1]),
            char::from(c[2]),
            char::from(c[3])
        ));
    }
    if let Some(meta) = meta
        && header.layout_hash != meta.layout_hash
    {
        return Err(format!(
            "Incorrect layout hash in {file_name}, expected 0x{:08X}, got 0x{:08X} (possibly wrong client version)",
            meta.layout_hash, header.layout_hash
        ));
    }
    if header.parent_lookup_count > 1 {
        return Err(format!(
            "Too many parent lookups in {file_name}, only one is allowed, got {}",
            header.parent_lookup_count
        ));
    }
    if let Some(meta) = meta {
        // ParentIndexField == -1 for every extractor table: no implicit parent field.
        if header.total_field_count != meta.field_count {
            return Err(format!(
                "Incorrect number of fields in {file_name}, expected {}, got {}",
                meta.field_count, header.total_field_count
            ));
        }
        if header.parent_lookup_count != 0 {
            return Err(format!("Unexpected parent lookup found in {file_name}"));
        }
    }

    let mut pos = HEADER_SIZE;
    let section_bytes = header.section_count as usize * SECTION_HEADER_SIZE;
    if header.section_count != 0 && pos + section_bytes > bytes.len() {
        return Err(format!("Unable to read section headers from {file_name}"));
    }
    let sections = (0..header.section_count as usize)
        .map(|i| parse_section(bytes, pos + i * SECTION_HEADER_SIZE))
        .collect();
    pos += section_bytes;

    let field_bytes = header.field_count as usize * FIELD_ENTRY_SIZE;
    if pos + field_bytes > bytes.len() {
        return Err(format!("Unable to read field information from {file_name}"));
    }
    pos += field_bytes;

    let mut columns = Vec::new();
    if header.column_meta_size != 0 {
        let meta_bytes = header.column_meta_size as usize;
        if pos + meta_bytes > bytes.len() {
            return Err(format!("Unable to read field metadata from {file_name}"));
        }
        let count = (header.total_field_count as usize).min(meta_bytes / COLUMN_META_SIZE);
        columns = (0..count)
            .map(|i| {
                let at = pos + i * COLUMN_META_SIZE;
                ColumnMeta {
                    bit_offset: rd_u16(bytes, at),
                    additional_data_size: rd_u32(bytes, at + 4),
                    compression: rd_u32(bytes, at + 8),
                    packed_bit_offset: rd_u32(bytes, at + 12),
                }
            })
            .collect::<Vec<_>>();
        pos += meta_bytes;

        for (kind, what) in [
            (COMPRESSION_PALLET, "pallet values"),
            (COMPRESSION_PALLET_ARRAY, "pallet array values"),
            (COMPRESSION_COMMON_DATA, "common values"),
        ] {
            for (i, column) in columns.iter().enumerate() {
                if column.compression != kind || column.additional_data_size == 0 {
                    continue;
                }
                let size = column.additional_data_size as usize;
                if pos + size > bytes.len() {
                    return Err(format!(
                        "Unable to read field {what} from {file_name} for field {i}"
                    ));
                }
                pos += size;
            }
        }
    }

    Ok(Headers {
        header,
        sections,
        columns,
        end: pos,
    })
}

fn parse_section(bytes: &[u8], at: usize) -> Section {
    Section {
        tact_id: rd_u64(bytes, at),
        file_offset: rd_u32(bytes, at + 8),
        record_count: rd_u32(bytes, at + 12),
        string_table_size: rd_u32(bytes, at + 16),
        id_table_size: rd_u32(bytes, at + 24),
        parent_lookup_data_size: rd_u32(bytes, at + 28),
        copy_table_count: rd_u32(bytes, at + 36),
    }
}

/// Port of the output half of `ExtractDB2File`: header + section headers (with
/// `TactId` replaced by `DUMMY_KNOWN_TACT_ID` when the key is known) + the rest of the
/// file copied verbatim from `posAfterHeaders`.
pub(crate) fn rewrite_known_tact_ids(
    bytes: &[u8],
    headers: &Headers,
    has_tact_key: impl Fn(u64) -> bool,
) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for (i, section) in headers.sections.iter().enumerate() {
        if section.tact_id != 0 && has_tact_key(section.tact_id) {
            let at = HEADER_SIZE + i * SECTION_HEADER_SIZE;
            out[at..at + 8].copy_from_slice(&DUMMY_KNOWN_TACT_ID.to_le_bytes());
        }
    }
    out
}

/// `IsKnownTactId` (DB2FileLoader.cpp).
fn is_known_tact_id(tact_id: u64) -> bool {
    tact_id == 0 || tact_id == DUMMY_KNOWN_TACT_ID
}

/// A loaded extractor table (`DB2FileLoader` after `TryLoadDB2`).
pub(crate) struct Db2Table {
    pub(crate) reader: Wdc4Reader,
    bytes: Vec<u8>,
    headers: Headers,
    known: Vec<bool>,
    /// Record index (in `reader` order) -> (section, index within section).
    locations: Vec<(usize, u32)>,
    /// `_copyTable` of all loaded sections, in file order: (NewRowId, SourceRowId).
    copies: Vec<(u32, u32)>,
}

impl Db2Table {
    /// `DB2FileLoader::Load(source, loadInfo)` for a regular (non-sparse) table.
    /// Errors carry the exception text for `TryLoadDB2`'s fatal message.
    /// Sections whose key is unknown are skipped (`IsKnownTactId` false and
    /// `HandleEncryptedSection` -> `Skip`); `has_tact_key` is `CascStorage->HasTactKey`.
    pub(crate) fn load(
        bytes: Vec<u8>,
        meta: &Db2Meta,
        has_tact_key: impl Fn(u64) -> bool,
    ) -> Result<Self, String> {
        let file_name = format!("FileDataId: {}", meta.file_data_id);
        let headers = check_headers(&bytes, &file_name, Some(meta))?;
        let header = headers.header;
        if header.flags & 0x1 != 0 {
            return Err(format!(
                "{file_name}: sparse (offset map) tables are not supported by the extractor port"
            ));
        }

        // Encrypted record id lists follow the header blocks, one per encrypted section.
        let mut pos = headers.end;
        for (i, section) in headers.sections.iter().enumerate() {
            if section.tact_id != 0 {
                if pos + 4 > bytes.len() {
                    return Err(format!(
                        "Unable to read number of encrypted records in {file_name} for section {i}"
                    ));
                }
                pos += 4 + 4 * rd_u32(&bytes, pos) as usize;
            }
        }

        let copy_size: usize = headers
            .sections
            .iter()
            .map(|s| s.copy_table_count as usize * 8)
            .sum();
        let parent_size: usize = headers
            .sections
            .iter()
            .map(|s| s.parent_lookup_data_size as usize)
            .sum();
        let expected = pos
            + header.record_size as usize * header.record_count as usize
            + header.string_table_size as usize
            + 4 * header.record_count as usize // IndexField == -1
            + copy_size
            + parent_size;
        if bytes.len() != expected {
            return Err(format!(
                "{file_name} failed size consistency check, expected {expected}, got {}",
                bytes.len()
            ));
        }

        let known: Vec<bool> = headers
            .sections
            .iter()
            .map(|s| is_known_tact_id(s.tact_id) || has_tact_key(s.tact_id))
            .collect();

        let mut copies = Vec::new();
        let mut locations = Vec::new();
        let mut patched = bytes.clone();
        for (i, section) in headers.sections.iter().enumerate() {
            if !known[i] {
                // HandleEncryptedSection -> Skip: no records, no copy table. Neutralise the
                // section header so Wdc4Reader ignores it too.
                let at = HEADER_SIZE + i * SECTION_HEADER_SIZE;
                patched[at + 12..at + 16].fill(0); // RecordCount
                patched[at + 24..at + 28].fill(0); // IdTableSize
                patched[at + 36..at + 40].fill(0); // CopyTableCount
                continue;
            }
            if section.id_table_size != 4 * section.record_count {
                return Err(format!(
                    "Unexpected id table size in {file_name} for section {i}, expected {}, got {}",
                    4 * section.record_count,
                    section.id_table_size
                ));
            }
            locations.extend((0..section.record_count).map(|r| (i, r)));
            let copy_at = section.file_offset as usize
                + section.record_count as usize * header.record_size as usize
                + section.string_table_size as usize
                + section.id_table_size as usize;
            for c in 0..section.copy_table_count as usize {
                let at = copy_at + c * 8;
                if at + 8 > bytes.len() {
                    return Err(format!(
                        "Unable to read section catalog data from {file_name} for section {i}"
                    ));
                }
                copies.push((rd_u32(&bytes, at), rd_u32(&bytes, at + 4)));
            }
        }

        let reader = Wdc4Reader::from_bytes(&patched).map_err(|e| format!("{file_name}: {e:#}"))?;
        if reader.record_count() != locations.len() {
            return Err(format!(
                "{file_name}: record count mismatch ({} != {})",
                reader.record_count(),
                locations.len()
            ));
        }

        Ok(Self {
            reader,
            bytes,
            headers,
            known,
            locations,
            copies,
        })
    }

    /// Loaded records as `(GetId(), record index)`, in file order.
    pub(crate) fn records(&self) -> impl Iterator<Item = (u32, usize)> + '_ {
        (0..self.reader.record_count()).map(|idx| (self.reader.record_id(idx), idx))
    }

    /// `GetRecordCopy(0..GetRecordCopyCount())` as `(NewRowId, SourceRowId)`.
    pub(crate) fn copies(&self) -> &[(u32, u32)] {
        &self.copies
    }

    /// `DB2FileLoaderRegularImpl::GetFieldOffset`.
    fn field_offset(&self, field: usize) -> usize {
        let Some(column) = self.headers.columns.get(field) else {
            return 0;
        };
        match column.compression {
            COMPRESSION_NONE => column.bit_offset as usize / 8,
            COMPRESSION_IMMEDIATE
            | COMPRESSION_SIGNED_IMMEDIATE
            | COMPRESSION_PALLET
            | COMPRESSION_PALLET_ARRAY => {
                column.packed_bit_offset as usize / 8
                    + self.headers.header.packed_data_offset as usize
            }
            _ => 0xFFFF,
        }
    }

    /// Byte `pos` of TrinityCore's in-memory `_data` buffer: every section's records
    /// back to back, then every section's string table (`LoadTableData`); skipped
    /// (encrypted) sections stay zero.
    fn data_byte(&self, mut pos: usize) -> u8 {
        let header = &self.headers.header;
        let record_size = header.record_size as usize;
        let records_total = record_size * header.record_count as usize;
        let in_strings = pos >= records_total;
        if in_strings {
            pos -= records_total;
        }
        for (i, section) in self.headers.sections.iter().enumerate() {
            let size = if in_strings {
                section.string_table_size as usize
            } else {
                section.record_count as usize * record_size
            };
            if pos < size {
                if !self.known[i] {
                    return 0;
                }
                let base = section.file_offset as usize
                    + if in_strings {
                        section.record_count as usize * record_size
                    } else {
                        0
                    };
                return self.bytes.get(base + pos).copied().unwrap_or(0);
            }
            pos -= size;
        }
        0
    }

    /// `DB2Record::GetString(field)` (`DB2FileLoaderRegularImpl::RecordGetString`): the
    /// string lives at `record + fieldOffset + stringOffset` in the `_data` buffer.
    /// A zero offset (C++ `nullptr`) yields an empty string.
    pub(crate) fn get_string(&self, idx: usize, field: usize) -> String {
        let string_offset = self.reader.get_field_u32(idx, field) as usize;
        if string_offset == 0 {
            return String::new();
        }
        let (section, in_section) = self.locations[idx];
        let record_size = self.headers.header.record_size as usize;
        let record_start: usize = self.headers.sections[..section]
            .iter()
            .map(|s| s.record_count as usize * record_size)
            .sum::<usize>()
            + in_section as usize * record_size;
        let mut pos = record_start + self.field_offset(field) + string_offset;
        let mut out = Vec::new();
        loop {
            let b = self.data_byte(pos);
            if b == 0 {
                break;
            }
            out.push(b);
            pos += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    /// A regular WDC4 file builder with uncompressed 32-bit fields.
    pub(crate) struct Wdc4Builder {
        pub(crate) layout_hash: u32,
        pub(crate) field_count: u32,
        /// Each section: tact id, records (id, fields), strings, copies.
        pub(crate) sections: Vec<SectionSpec>,
    }

    #[derive(Default, Clone)]
    pub(crate) struct SectionSpec {
        pub(crate) tact_id: u64,
        pub(crate) records: Vec<(u32, Vec<u32>)>,
        pub(crate) strings: Vec<u8>,
        pub(crate) copies: Vec<(u32, u32)>,
        pub(crate) zero_data: bool,
    }

    impl Wdc4Builder {
        pub(crate) fn build(&self) -> Vec<u8> {
            let fields = self.field_count as usize;
            let record_size = 4 * fields;
            let record_count: usize = self.sections.iter().map(|s| s.records.len()).sum();
            let string_size: usize = self.sections.iter().map(|s| s.strings.len()).sum();
            let encrypted = self.sections.iter().filter(|s| s.tact_id != 0).count();
            let mut out = Vec::new();
            let push = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
            push(&mut out, 0x3443_4457);
            push(&mut out, record_count as u32);
            push(&mut out, self.field_count);
            push(&mut out, record_size as u32);
            push(&mut out, string_size as u32);
            push(&mut out, 0xAABB_CCDD); // table hash
            push(&mut out, self.layout_hash);
            push(&mut out, 0); // min id
            push(&mut out, 0); // max id
            push(&mut out, 0); // locale
            out.extend_from_slice(&0x4u16.to_le_bytes()); // flags: has id list
            out.extend_from_slice(&(-1i16).to_le_bytes()); // index field
            push(&mut out, self.field_count); // total field count
            push(&mut out, record_size as u32); // packed data offset
            push(&mut out, 0); // parent lookup count
            push(&mut out, (fields * 24) as u32); // column meta size
            push(&mut out, 0); // common
            push(&mut out, 0); // pallet
            push(&mut out, self.sections.len() as u32);
            let sections_at = out.len();
            out.resize(out.len() + 40 * self.sections.len(), 0);
            for f in 0..fields {
                out.extend_from_slice(&0i16.to_le_bytes()); // shift
                out.extend_from_slice(&((f * 4) as u16).to_le_bytes());
            }
            for f in 0..fields {
                out.extend_from_slice(&((f * 32) as u16).to_le_bytes());
                out.extend_from_slice(&32u16.to_le_bytes());
                push(&mut out, 0); // additional data size
                push(&mut out, 0); // compression none
                out.extend_from_slice(&[0; 12]);
            }
            for _ in 0..encrypted {
                push(&mut out, 0); // encrypted id count
            }
            for (i, s) in self.sections.iter().enumerate() {
                let file_offset = out.len();
                let data_start = out.len();
                for (_, values) in &s.records {
                    for v in values {
                        push(&mut out, *v);
                    }
                }
                out.extend_from_slice(&s.strings);
                for (id, _) in &s.records {
                    push(&mut out, *id);
                }
                if s.zero_data {
                    out[data_start..].fill(0);
                }
                for (new, src) in &s.copies {
                    push(&mut out, *new);
                    push(&mut out, *src);
                }
                let at = sections_at + i * 40;
                let mut hdr = Vec::new();
                hdr.extend_from_slice(&s.tact_id.to_le_bytes());
                push(&mut hdr, file_offset as u32);
                push(&mut hdr, s.records.len() as u32);
                push(&mut hdr, s.strings.len() as u32);
                push(&mut hdr, 0);
                push(&mut hdr, 4 * s.records.len() as u32);
                push(&mut hdr, 0);
                push(&mut hdr, 0);
                push(&mut hdr, s.copies.len() as u32);
                out[at..at + 40].copy_from_slice(&hdr);
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::{SectionSpec, Wdc4Builder};
    use super::*;

    const META: Db2Meta = Db2Meta {
        name: "Test.db2",
        file_data_id: 42,
        layout_hash: 0x1234_5678,
        field_count: 2,
    };

    /// Field 0 is a string offset relative to the field itself (WDC4 regular layout).
    fn two_record_file() -> Vec<u8> {
        // records: 2 records x 8 bytes = 16 bytes, strings follow.
        // record 0 field 0 at data offset 0 -> "Azeroth" at strings offset 1 => 16 + 1 - 0
        // record 1 field 0 at data offset 8 -> "Kalimdor" at strings offset 9 => 16 + 9 - 8
        let strings = b"\0Azeroth\0Kalimdor\0".to_vec();
        Wdc4Builder {
            layout_hash: META.layout_hash,
            field_count: 2,
            sections: vec![SectionSpec {
                records: vec![(0, vec![17, 7]), (1, vec![17, 9])],
                strings,
                copies: vec![(530, 1), (531, 99)],
                ..SectionSpec::default()
            }],
        }
        .build()
    }

    #[test]
    fn loads_ids_numeric_fields_strings_and_copies() {
        let table = Db2Table::load(two_record_file(), &META, |_| false).expect("valid table");
        let records: Vec<_> = table.records().collect();
        assert_eq!(records, vec![(0, 0), (1, 1)]);
        assert_eq!(table.get_string(0, 0), "Azeroth");
        assert_eq!(table.get_string(1, 0), "Kalimdor");
        assert_eq!(table.reader.get_field_u32(1, 1), 9);
        assert_eq!(table.copies(), &[(530, 1), (531, 99)]);
    }

    #[test]
    fn rejects_layout_hash_field_count_and_size_mismatch() {
        let bytes = two_record_file();
        let wrong_hash = Db2Meta {
            layout_hash: 1,
            ..META
        };
        let err = Db2Table::load(bytes.clone(), &wrong_hash, |_| false)
            .err()
            .unwrap();
        assert!(
            err.starts_with("Incorrect layout hash in FileDataId: 42"),
            "{err}"
        );
        let wrong_fields = Db2Meta {
            field_count: 3,
            ..META
        };
        let err = Db2Table::load(bytes.clone(), &wrong_fields, |_| false)
            .err()
            .unwrap();
        assert_eq!(
            err,
            "Incorrect number of fields in FileDataId: 42, expected 3, got 2"
        );
        let mut longer = bytes;
        longer.push(0);
        let err = Db2Table::load(longer, &META, |_| false).err().unwrap();
        assert!(err.contains("failed size consistency check"), "{err}");
    }

    #[test]
    fn signature_check_matches_cpp_message() {
        let mut bytes = two_record_file();
        bytes[0..4].copy_from_slice(b"WDC3");
        let err = check_headers(&bytes, "FileDataId: 1", None).err().unwrap();
        assert_eq!(
            err,
            "Incorrect file signature in FileDataId: 1, expected 'WDC4', got WDC3"
        );
        assert_eq!(
            check_headers(&[0; 10], "x", None).err().unwrap(),
            "Failed to read header"
        );
    }

    fn encrypted_file(zero: bool) -> Vec<u8> {
        Wdc4Builder {
            layout_hash: META.layout_hash,
            field_count: 2,
            sections: vec![
                SectionSpec {
                    records: vec![(5, vec![0, 50])],
                    ..SectionSpec::default()
                },
                SectionSpec {
                    tact_id: 0x1122_3344_5566_7788,
                    records: vec![(6, vec![0, 60])],
                    copies: vec![(7, 6)],
                    zero_data: zero,
                    ..SectionSpec::default()
                },
            ],
        }
        .build()
    }

    const KEY: u64 = 0x1122_3344_5566_7788;

    #[test]
    fn unknown_key_section_is_skipped() {
        // Zero-filled like CascLib `CASC_OVERCOME_ENCRYPTED` output for a missing key.
        let table = Db2Table::load(encrypted_file(true), &META, |_| false).unwrap();
        assert_eq!(table.records().collect::<Vec<_>>(), vec![(5, 0)]);
        assert!(table.copies().is_empty());
    }

    #[test]
    fn known_key_section_is_processed() {
        let table = Db2Table::load(encrypted_file(false), &META, |k| k == KEY).unwrap();
        let ids: Vec<_> = table.records().map(|(id, _)| id).collect();
        assert_eq!(ids, vec![5, 6]);
        assert_eq!(table.reader.get_field_u32(1, 1), 60);
        assert_eq!(table.copies(), &[(7, 6)]);
    }

    #[test]
    fn extraction_rewrites_only_known_tact_ids() {
        for (known, expected) in [(true, DUMMY_KNOWN_TACT_ID), (false, KEY)] {
            let bytes = encrypted_file(!known);
            let headers = check_headers(&bytes, "FileDataId: 42", None).unwrap();
            let out = rewrite_known_tact_ids(&bytes, &headers, |k| known && k == KEY);
            assert_eq!(out.len(), bytes.len());
            let second = HEADER_SIZE + SECTION_HEADER_SIZE;
            assert_eq!(rd_u64(&out, second), expected);
            assert_eq!(rd_u64(&out, HEADER_SIZE), 0);
            assert_eq!(out[second + 8..], bytes[second + 8..]);
        }
    }
}
