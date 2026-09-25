//! DB2 tables read by the extractor itself: port of `ReadMapDBC`,
//! `ReadLiquidMaterialTable`, `ReadLiquidObjectTable`, `ReadLiquidTypeTable` and the
//! shared `TryLoadDB2` from `src/tools/map_extractor/System.cpp`, with the table
//! metadata of `src/tools/extractor_common/ExtractorDB2LoadInfo.h`.
//!
//! Field indices are the physical WDC4 fields (`DB2Meta` fields; the `ID` column is the
//! id list because every table here has `IndexField == -1`):
//! * `Map` (22 fields): `Directory` = 0, `MapName` = 1 (same indices RustyCore's
//!   `wow_data::map` loader derives from `MapLoadInfo`).
//! * `LiquidMaterial` (2 fields): `Flags` = 0, `LVF` = 1.
//! * `LiquidObject` (5 fields): `FlowDirection` = 0, `FlowSpeed` = 1, `LiquidTypeID` = 2.
//! * `LiquidType` (21 fields): `SoundBank` = 3, `MaterialID` = 14 (matching
//!   `wow_data::maps_world::LiquidTypeStore`).
//! * `CinematicCamera` (4 fields): `FileDataID` = 3 (matching `CinematicCameraStore`).

use std::collections::HashMap;
use std::fmt;

use crate::casc::{CASC_LOCALE_NONE, Casc, FileRead, FileRef, OpenFlags};
use crate::db2::{Db2Meta, Db2Table};

pub(crate) const MAP_META: Db2Meta = Db2Meta {
    name: "Map.db2",
    file_data_id: 1_349_477,
    layout_hash: 0xBFC0_78A9,
    field_count: 22,
};
pub(crate) const LIQUID_MATERIAL_META: Db2Meta = Db2Meta {
    name: "LiquidMaterial.db2",
    file_data_id: 1_132_538,
    layout_hash: 0x2CFF_EA40,
    field_count: 2,
};
pub(crate) const LIQUID_OBJECT_META: Db2Meta = Db2Meta {
    name: "LiquidObject.db2",
    file_data_id: 1_308_058,
    layout_hash: 0x6CAE_B8A1,
    field_count: 5,
};
pub(crate) const LIQUID_TYPE_META: Db2Meta = Db2Meta {
    name: "LiquidType.db2",
    file_data_id: 1_371_380,
    layout_hash: 0xAFFF_C9E0,
    field_count: 21,
};
pub(crate) const CINEMATIC_CAMERA_META: Db2Meta = Db2Meta {
    name: "CinematicCamera.db2",
    file_data_id: 1_294_214,
    layout_hash: 0x744B_99BC,
    field_count: 4,
};

const MAP_DIRECTORY: usize = 0;
const MAP_MAP_NAME: usize = 1;
const LIQUID_MATERIAL_LVF: usize = 1;
const LIQUID_OBJECT_LIQUID_TYPE_ID: usize = 2;
const LIQUID_TYPE_SOUND_BANK: usize = 3;
const LIQUID_TYPE_MATERIAL_ID: usize = 14;
pub(crate) const CINEMATIC_CAMERA_FILE_DATA_ID: usize = 3;

/// A C++ fatal exit: the exact text the C++ prints before `exit(1)` / terminating.
#[derive(Debug)]
pub(crate) struct CppFatal(pub(crate) String);

impl fmt::Display for CppFatal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CppFatal {}

/// `TryLoadDB2`: `DB2CascFileSource(CascStorage, fileDataId)` (printErrors, zero-filled
/// encrypted parts, `CASC_LOCALE_NONE`) + `DB2FileLoader::Load`; any failure is fatal.
pub(crate) fn try_load_db2(casc: &Casc, meta: &Db2Meta) -> Result<Db2Table, CppFatal> {
    let fatal = |casc_error: &str, what: &str| {
        CppFatal(format!(
            "Fatal error: Invalid {} file format! {casc_error}\n{what}\n",
            meta.name
        ))
    };
    let flags = OpenFlags {
        print_errors: true,
        zerofill_encrypted: true,
    };
    match casc.read(FileRef::Id(meta.file_data_id), CASC_LOCALE_NONE, flags) {
        FileRead::Data(bytes) => Db2Table::load(bytes, meta, |tact_id| casc.has_tact_key(tact_id))
            .map_err(|what| fatal("SUCCESS", &what)),
        FileRead::OpenFailed(error) => Err(fatal(error, "No such file or directory")),
        FileRead::ReadFailed(error) => Err(fatal(error, "Failed to read header")),
    }
}

/// `MapEntry` (System.cpp).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MapEntry {
    pub(crate) id: u32,
    pub(crate) name: String,
    pub(crate) directory: String,
}

/// `ReadMapDBC`.
pub(crate) fn read_map_dbc(casc: &Casc) -> Result<Vec<MapEntry>, CppFatal> {
    println!("Read Map.db2 file...");
    let db2 = try_load_db2(casc, &MAP_META)?;
    let map_ids = map_entries(&db2);
    println!("Done! ({} maps loaded)", map_ids.len());
    Ok(map_ids)
}

/// Record + copy-table walk of `ReadMapDBC`.
pub(crate) fn map_entries(db2: &Db2Table) -> Vec<MapEntry> {
    let mut map_ids = Vec::with_capacity(db2.reader.record_count());
    let mut id_to_index = HashMap::new();
    for (id, idx) in db2.records() {
        id_to_index.insert(id, map_ids.len());
        map_ids.push(MapEntry {
            id,
            name: db2.get_string(idx, MAP_MAP_NAME),
            directory: db2.get_string(idx, MAP_DIRECTORY),
        });
    }
    for &(new_row_id, source_row_id) in db2.copies() {
        if let Some(&index) = id_to_index.get(&source_row_id) {
            let source: &MapEntry = &map_ids[index];
            let copy = MapEntry {
                id: new_row_id,
                ..source.clone()
            };
            map_ids.push(copy);
        }
    }
    map_ids
}

/// `LiquidTypeEntry` (System.cpp).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LiquidTypeEntry {
    pub(crate) sound_bank: u8,
    pub(crate) material_id: u8,
}

/// `LiquidMaterials`, `LiquidObjects`, `LiquidTypes` (System.cpp globals).
#[derive(Debug, Clone, Default)]
pub(crate) struct LiquidTables {
    /// `LiquidMaterialEntry::LVF` by id.
    pub(crate) materials: HashMap<u32, i8>,
    /// `LiquidObjectEntry::LiquidTypeID` by id (loaded but unused, like the C++).
    pub(crate) objects: HashMap<u32, i16>,
    pub(crate) types: HashMap<u32, LiquidTypeEntry>,
}

/// `map[copy.NewRowId] = map[copy.SourceRowId]` with `operator[]` semantics: a missing
/// source is default-inserted first.
fn apply_copies<V: Copy + Default>(map: &mut HashMap<u32, V>, copies: &[(u32, u32)]) {
    for &(new_row_id, source_row_id) in copies {
        let value = *map.entry(source_row_id).or_default();
        map.insert(new_row_id, value);
    }
}

impl LiquidTables {
    /// `ReadLiquidMaterialTable`, `ReadLiquidObjectTable`, `ReadLiquidTypeTable`.
    pub(crate) fn read(casc: &Casc) -> Result<Self, CppFatal> {
        let mut tables = Self::default();

        println!("Read LiquidMaterial.db2 file...");
        let db2 = try_load_db2(casc, &LIQUID_MATERIAL_META)?;
        tables.load_materials(&db2);
        println!("Done! ({} LiquidMaterials loaded)", tables.materials.len());

        println!("Read LiquidObject.db2 file...");
        let db2 = try_load_db2(casc, &LIQUID_OBJECT_META)?;
        tables.load_objects(&db2);
        println!("Done! ({} LiquidObjects loaded)", tables.objects.len());

        println!("Read LiquidType.db2 file...");
        let db2 = try_load_db2(casc, &LIQUID_TYPE_META)?;
        tables.load_types(&db2);
        println!("Done! ({} LiquidTypes loaded)", tables.types.len());

        Ok(tables)
    }

    pub(crate) fn load_materials(&mut self, db2: &Db2Table) {
        for (id, idx) in db2.records() {
            // `record.GetUInt8("LVF")` stored into `int8 LVF`
            self.materials.insert(
                id,
                db2.reader
                    .get_field_u8(idx, LIQUID_MATERIAL_LVF)
                    .cast_signed(),
            );
        }
        apply_copies(&mut self.materials, db2.copies());
    }

    pub(crate) fn load_objects(&mut self, db2: &Db2Table) {
        for (id, idx) in db2.records() {
            // `record.GetUInt16("LiquidTypeID")` stored into `int16 LiquidTypeID`
            self.objects.insert(
                id,
                db2.reader
                    .get_field_u16(idx, LIQUID_OBJECT_LIQUID_TYPE_ID)
                    .cast_signed(),
            );
        }
        apply_copies(&mut self.objects, db2.copies());
    }

    pub(crate) fn load_types(&mut self, db2: &Db2Table) {
        for (id, idx) in db2.records() {
            self.types.insert(
                id,
                LiquidTypeEntry {
                    sound_bank: db2.reader.get_field_u8(idx, LIQUID_TYPE_SOUND_BANK),
                    material_id: db2.reader.get_field_u8(idx, LIQUID_TYPE_MATERIAL_ID),
                },
            );
        }
        apply_copies(&mut self.types, db2.copies());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db2::test_util::{SectionSpec, Wdc4Builder};

    #[test]
    fn map_entries_follow_read_map_dbc() {
        // 2 records of 22 u32 fields (88 bytes each): Directory (0) and MapName (1)
        // are string offsets relative to the field position.
        let mut strings = b"\0Azeroth\0Eastern Kingdoms\0".to_vec();
        strings.extend_from_slice(b"Kalimdor\0");
        let records_total = 2 * 88;
        let rel = |record: usize, field: usize, string_pos: usize| {
            (records_total + string_pos - (record * 88 + field * 4)) as u32
        };
        let mut r0 = vec![0u32; 22];
        r0[0] = rel(0, 0, 1);
        r0[1] = rel(0, 1, 9);
        let mut r1 = vec![0u32; 22];
        r1[0] = rel(1, 0, 26);
        r1[1] = rel(1, 1, 26);
        let bytes = Wdc4Builder {
            layout_hash: MAP_META.layout_hash,
            field_count: 22,
            sections: vec![SectionSpec {
                records: vec![(0, r0), (1, r1)],
                strings,
                copies: vec![(9000, 1), (9001, 12345)],
                ..SectionSpec::default()
            }],
        }
        .build();
        let db2 = Db2Table::load(bytes, &MAP_META, |_| false).unwrap();
        let maps = map_entries(&db2);
        let expect = |id: u32, name: &str, directory: &str| MapEntry {
            id,
            name: name.into(),
            directory: directory.into(),
        };
        assert_eq!(
            maps,
            vec![
                expect(0, "Eastern Kingdoms", "Azeroth"),
                expect(1, "Kalimdor", "Kalimdor"),
                expect(9000, "Kalimdor", "Kalimdor"),
            ]
        );
    }

    #[test]
    fn liquid_copies_use_operator_brackets() {
        let mut map = HashMap::from([(1u32, 7i8)]);
        apply_copies(&mut map, &[(2, 1), (3, 99)]);
        assert_eq!(map.get(&2), Some(&7));
        assert_eq!(map.get(&3), Some(&0));
        assert_eq!(map.get(&99), Some(&0)); // default-inserted source
    }

    #[test]
    fn liquid_type_fields() {
        let mut fields = vec![0u32; 21];
        fields[LIQUID_TYPE_SOUND_BANK] = 1;
        fields[LIQUID_TYPE_MATERIAL_ID] = 2;
        let bytes = Wdc4Builder {
            layout_hash: LIQUID_TYPE_META.layout_hash,
            field_count: 21,
            sections: vec![SectionSpec {
                records: vec![(14, fields)],
                ..SectionSpec::default()
            }],
        }
        .build();
        let db2 = Db2Table::load(bytes, &LIQUID_TYPE_META, |_| false).unwrap();
        let mut tables = LiquidTables::default();
        tables.load_types(&db2);
        assert_eq!(
            tables.types[&14],
            LiquidTypeEntry {
                sound_bank: 1,
                material_id: 2
            }
        );
    }
}
