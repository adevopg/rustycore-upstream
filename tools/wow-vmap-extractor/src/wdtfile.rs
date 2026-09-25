//! Port of `src/tools/vmap4_extractor/wdtfile.{h,cpp}` (`WDTFile`): reads the map `.wdt`
//! (`MPHD`, `MAIN`, `MAID`, global `MWMO`/`MODF`) and hands out the per-tile
//! `_obj0.adt` files, optionally caching them for maps that are parents of other maps.

use crate::adtfile::{AdtFile, MODF_SIZE, Modf, split_name_chunk};
use crate::cascfile::{CascFile, read_chunk_header};
use crate::names::{file_data_id_name, normalized_plain_name};
use crate::vec3d::u32_at;
use crate::vmapexport::VmapExport;

/// `sizeof(WDT::MPHD)`.
pub const MPHD_SIZE: usize = 32;
/// `sizeof(WDT::MAIN)`: `SMAreaInfo Data[64][64]` of `{ Flag, AsyncId }`.
pub const MAIN_SIZE: usize = 64 * 64 * 8;
/// `sizeof(WDT::MAID)`: `SMAreaFileIDs Data[64][64]` of 8 FileDataIDs.
pub const MAID_SIZE: usize = 64 * 64 * 32;

/// Port of `class WDTFile`.
pub struct WdtFile {
    file: CascFile,
    /// `WDT::MPHD::Flags` (the rest of `MPHD` is not used).
    header_flags: u32,
    /// `WDT::MAIN`.
    adt_info: Vec<u8>,
    /// `WDT::MAID`.
    adt_file_data_ids: Option<Vec<u8>>,
    map_name: String,
    wmo_names: Vec<Vec<u8>>,
    /// `ADTCache` (64x64, indexed `[x][y]`), present when the WDT is cacheable.
    adt_cache: Option<Vec<Option<AdtFile>>>,
}

impl WdtFile {
    /// `WDTFile::WDTFile(fileName, mapName, cache)`.
    pub fn new(file: CascFile, map_name: String, cache: bool) -> Self {
        Self {
            file,
            header_flags: 0,
            adt_info: vec![0u8; MAIN_SIZE],
            adt_file_data_ids: None,
            map_name,
            wmo_names: Vec::new(),
            adt_cache: cache.then(|| (0..64 * 64).map(|_| None).collect()),
        }
    }

    /// `WDTFile::init`.
    pub fn init(&mut self, ctx: &mut VmapExport<'_>, map_id: u32) -> bool {
        if self.file.is_eof() {
            return false;
        }

        let Some(mut dirfile) = ctx.open_dirfile() else {
            return false;
        };

        let mut size = 0u32;
        let mut fourcc = [0u8; 4];
        while !self.file.is_eof() {
            read_chunk_header(&mut self.file, &mut fourcc, &mut size);
            let nextpos = self.file.get_pos() + size as usize;

            match &fourcc {
                b"MPHD" => {
                    assert!(
                        size as usize == MPHD_SIZE,
                        "ASSERT(size == sizeof(WDT::MPHD))"
                    );
                    let raw = self.file.read_vec(MPHD_SIZE);
                    self.header_flags = u32_at(&raw, 0);
                }
                b"MAIN" => {
                    assert!(
                        size as usize == MAIN_SIZE,
                        "ASSERT(size == sizeof(WDT::MAIN))"
                    );
                    self.adt_info = self.file.read_vec(MAIN_SIZE);
                }
                b"MAID" => {
                    assert!(
                        size as usize == MAID_SIZE,
                        "ASSERT(size == sizeof(WDT::MAID))"
                    );
                    self.adt_file_data_ids = Some(self.file.read_vec(MAID_SIZE));
                }
                b"MWMO" if size != 0 => {
                    // global map objects
                    let buf = self.file.read_vec(size as usize);
                    for mut path in split_name_chunk(&buf) {
                        self.wmo_names.push(normalized_plain_name(&path));
                        ctx.extract_single_wmo(&mut path);
                    }
                }
                b"MODF" if size != 0 => {
                    // global wmo instance data
                    let map_object_count = size as usize / MODF_SIZE;
                    for _ in 0..map_object_count {
                        let map_obj_def = Modf::from_le(&self.file.read_vec(MODF_SIZE));
                        let name = if map_obj_def.flags & 0x8 == 0 {
                            // C++ indexes _wmoNames[Id] unchecked
                            let Some(name) = self.wmo_names.get(map_obj_def.id as usize) else {
                                continue;
                            };
                            name.clone()
                        } else {
                            let mut file_name = file_data_id_name(map_obj_def.id);
                            ctx.extract_single_wmo(&mut file_name);
                            file_name
                        };
                        ctx.extract_map_object_and_doodads(
                            &map_obj_def,
                            &name,
                            true,
                            map_id,
                            map_id,
                            &mut dirfile.data,
                            None,
                        );
                    }
                }
                _ => {}
            }
            self.file.seek(nextpos);
        }

        self.file.close();
        drop(dirfile); // fclose(dirfile)
        true
    }

    /// `WDTFile::GetMap` + `ADTFile::init` + `WDTFile::FreeADT`: runs `init` on the
    /// tile's ADT and returns its result, `None` when `GetMap` returns null.
    pub fn init_adt(
        &mut self,
        ctx: &mut VmapExport<'_>,
        x: i32,
        y: i32,
        map_num: u32,
        original_map_id: u32,
    ) -> Option<bool> {
        if !(0..64).contains(&x) || !(0..64).contains(&y) {
            return None;
        }
        let cache_index = (x * 64 + y) as usize;

        if let Some(cache) = &mut self.adt_cache
            && let Some(adt) = &mut cache[cache_index]
        {
            return Some(adt.init(ctx, map_num, original_map_id));
        }

        let info_index = (y as usize * 64 + x as usize) * 8;
        if u32_at(&self.adt_info, info_index) & 1 == 0 {
            return None;
        }

        let name = format!(
            "World\\Maps\\{0}\\{0}_{1}_{2}_obj0.adt",
            self.map_name, x, y
        );
        let cacheable = self.adt_cache.is_some();
        let file = if self.header_flags & 0x200 != 0 {
            // C++ dereferences _adtFileDataIds unchecked
            let obj0 = self.adt_file_data_ids.as_ref().map_or(0, |maid| {
                u32_at(maid, (y as usize * 64 + x as usize) * 32 + 4)
            });
            CascFile::open_id(ctx.casc, obj0, &name, false)
        } else {
            CascFile::open_name(ctx.casc, &name, false)
        };
        let mut adt = AdtFile::new(file, cacheable);
        let result = adt.init(ctx, map_num, original_map_id);

        if let Some(cache) = &mut self.adt_cache {
            cache[cache_index] = Some(adt);
        }
        // FreeADT: non-cached ADTs are dropped here
        Some(result)
    }
}
