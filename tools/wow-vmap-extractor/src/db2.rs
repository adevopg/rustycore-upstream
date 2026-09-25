//! DB2 access for the extractor: the relevant parts of
//! `src/tools/extractor_common/ExtractorDB2LoadInfo.h` (`MapLoadInfo`,
//! `GameobjectDisplayInfoLoadInfo`), `DB2CascFileSource` and the header validation of
//! `DB2FileLoader::LoadHeaders` (`src/common/DataStores/DB2FileLoader.cpp`), on top of
//! [`wow_data::wdc4::Wdc4Reader`].
//!
//! Field indices are the physical WDC4 fields, i.e. the index into
//! `DB2Meta::MetaFields` (the `ID` field is not stored in the record for these two
//! tables, `IndexField == -1`). They match RustyCore's own loaders for the same 3.4.3
//! tables (`crates/wow-data/src/map.rs` `MapStore::load`,
//! `crates/wow-data/src/entities_movement.rs` `GameObjectDisplayInfoStore::load`).

use wow_data::wdc4::Wdc4Reader;

use crate::cascfile::CascSource;

/// The `DB2Meta` values checked by `DB2FileLoader::LoadHeaders`.
pub struct Db2Meta {
    pub file_data_id: u32,
    pub layout_hash: u32,
    /// `DB2Meta::FieldCount` (= expected `TotalFieldCount`, no parent index field).
    pub field_count: u32,
}

/// `MapLoadInfo::MetaInstance{ 1349477, -1, 22, 22, 0xBFC078A9, MetaFields, -1 }`.
pub const MAP_META: Db2Meta = Db2Meta {
    file_data_id: 1_349_477,
    layout_hash: 0xBFC0_78A9,
    field_count: 22,
};

/// `Map.db2` `Directory` (`MetaFields[0]`, `FT_STRING_NOT_LOCALIZED`).
pub const MAP_FIELD_DIRECTORY: usize = 0;
/// `Map.db2` `MapName` (`MetaFields[1]`, `FT_STRING`).
pub const MAP_FIELD_MAP_NAME: usize = 1;
/// `Map.db2` `ParentMapID` (`MetaFields[12]`, signed `FT_SHORT`).
pub const MAP_FIELD_PARENT_MAP_ID: usize = 12;
/// `Map.db2` `CosmeticParentMapID` (`MetaFields[13]`, signed `FT_SHORT`).
pub const MAP_FIELD_COSMETIC_PARENT_MAP_ID: usize = 13;

/// `GameobjectDisplayInfoLoadInfo::MetaInstance{ 1266277, -1, 6, 6, 0xB59CF0B2, ... }`.
pub const GAMEOBJECT_DISPLAY_INFO_META: Db2Meta = Db2Meta {
    file_data_id: 1_266_277,
    layout_hash: 0xB59C_F0B2,
    field_count: 6,
};

/// `GameObjectDisplayInfo.db2` `FileDataID` (`MetaFields[2]`, signed `FT_INT`).
pub const GAMEOBJECT_DISPLAY_INFO_FIELD_FILE_DATA_ID: usize = 2;

const WDC4_SIGNATURE: u32 = 0x3443_4457; // 'WDC4'

fn header_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

/// `DB2FileLoader::LoadHeaders` checks against the load info.
pub fn check_header(data: &[u8], meta: &Db2Meta) -> Result<(), String> {
    if data.len() < 72 {
        return Err("Failed to read header".to_owned());
    }
    let signature = header_u32(data, 0);
    if signature != WDC4_SIGNATURE {
        return Err(format!(
            "Incorrect file signature in FileDataId: {}, expected 'WDC4', got {}",
            meta.file_data_id,
            String::from_utf8_lossy(&data[0..4])
        ));
    }
    let layout_hash = header_u32(data, 24);
    if layout_hash != meta.layout_hash {
        return Err(format!(
            "Incorrect layout hash in FileDataId: {}, expected 0x{:08X}, got 0x{:08X} (possibly wrong client version)",
            meta.file_data_id, meta.layout_hash, layout_hash
        ));
    }
    let parent_lookup_count = header_u32(data, 52);
    if parent_lookup_count > 1 {
        return Err(format!(
            "Too many parent lookups in FileDataId: {}, only one is allowed, got {}",
            meta.file_data_id, parent_lookup_count
        ));
    }
    let total_field_count = header_u32(data, 44);
    if total_field_count != meta.field_count {
        return Err(format!(
            "Incorrect number of fields in FileDataId: {}, expected {}, got {}",
            meta.file_data_id, meta.field_count, total_field_count
        ));
    }
    if parent_lookup_count != 0 {
        return Err(format!(
            "Unexpected parent lookup found in FileDataId: {}",
            meta.file_data_id
        ));
    }
    Ok(())
}

/// `DB2CascFileSource(storage, fileDataId)` + `DB2FileLoader::Load`.
///
/// The `Err` carries the `e.what()` text (and the CASC error name when the file could
/// not be opened, which `vmapexport.cpp` prints for Map.db2).
pub fn load(casc: &dyn CascSource, meta: &Db2Meta) -> Result<Wdc4Reader, Db2LoadError> {
    let data = match casc.open_by_id(meta.file_data_id) {
        Ok(Some(data)) => data,
        Ok(None) => {
            eprintln!(
                "Failed to open 'FileDataId {}' in CASC storage: FILE_NOT_FOUND",
                meta.file_data_id
            );
            return Err(Db2LoadError {
                casc_error: "FILE_NOT_FOUND".to_owned(),
                what: "No such file or directory".to_owned(),
            });
        }
        Err(error) => {
            eprintln!(
                "Failed to open 'FileDataId {}' in CASC storage: {error}",
                meta.file_data_id
            );
            return Err(Db2LoadError {
                casc_error: error,
                what: "No such file or directory".to_owned(),
            });
        }
    };
    let fail = |what: String| Db2LoadError {
        casc_error: "SUCCESS".to_owned(),
        what,
    };
    check_header(&data, meta).map_err(fail)?;
    Wdc4Reader::from_bytes(&data).map_err(|e| fail(format!("{e:#}")))
}

/// A failed DB2 load.
#[derive(Debug)]
pub struct Db2LoadError {
    /// `CASC::HumanReadableCASCError(GetCascError())`.
    pub casc_error: String,
    /// `exception::what()`.
    pub what: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn header(sig: &[u8; 4], layout: u32, total_fields: u32, parent_lookups: u32) -> Vec<u8> {
        let mut h = vec![0u8; 72];
        h[0..4].copy_from_slice(sig);
        h[24..28].copy_from_slice(&layout.to_le_bytes());
        h[44..48].copy_from_slice(&total_fields.to_le_bytes());
        h[52..56].copy_from_slice(&parent_lookups.to_le_bytes());
        h
    }

    #[test]
    fn header_checks_follow_db2_file_loader() {
        assert!(check_header(&header(b"WDC4", 0xBFC0_78A9, 22, 0), &MAP_META).is_ok());
        assert!(check_header(&header(b"WDC3", 0xBFC0_78A9, 22, 0), &MAP_META).is_err());
        assert!(check_header(&header(b"WDC4", 0x1234_5678, 22, 0), &MAP_META).is_err());
        assert!(check_header(&header(b"WDC4", 0xBFC0_78A9, 21, 0), &MAP_META).is_err());
        assert!(check_header(&header(b"WDC4", 0xBFC0_78A9, 22, 1), &MAP_META).is_err());
        assert!(
            check_header(
                &header(b"WDC4", 0xB59C_F0B2, 6, 0),
                &GAMEOBJECT_DISPLAY_INFO_META
            )
            .is_ok()
        );
    }
}
