//! Port of `src/tools/vmap4_extractor/adtfile.{h,cpp}`: the `ADT::MDDF` / `ADT::MODF`
//! placement records, `ADTOutputCache` and `ADTFile` (`init`, `initFromCache`), which
//! walks an `_obj0.adt` and appends the doodad/WMO spawns to `Buildings/dir_bin`.
//! The name helpers of the same C++ file live in [`crate::names`].

use crate::cascfile::{CascFile, c_str, read_chunk_header};
use crate::model;
use crate::names::{file_data_id_name, normalized_plain_name};
use crate::vec3d::{AaBox3D, Vec3D, u16_at, u32_at};
use crate::vmapexport::{MOD_PARENT_SPAWN, VmapExport};

/// `ADT::MDDF` (packed, 36 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Mddf {
    pub id: u32,
    pub unique_id: u32,
    pub position: Vec3D,
    pub rotation: Vec3D,
    pub scale: u16,
    pub flags: u16,
}

pub const MDDF_SIZE: usize = 36;

impl Mddf {
    pub fn from_le(b: &[u8]) -> Self {
        Self {
            id: u32_at(b, 0),
            unique_id: u32_at(b, 4),
            position: Vec3D::from_le(&b[8..20]),
            rotation: Vec3D::from_le(&b[20..32]),
            scale: u16_at(b, 32),
            flags: u16_at(b, 34),
        }
    }
}

/// `ADT::MODF` (packed, 64 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Modf {
    pub id: u32,
    pub unique_id: u32,
    pub position: Vec3D,
    pub rotation: Vec3D,
    pub bounds: AaBox3D,
    pub flags: u16,
    /// can be larger than number of doodad sets in WMO
    pub doodad_set: u16,
    pub name_set: u16,
    pub scale: u16,
}

pub const MODF_SIZE: usize = 64;

impl Modf {
    pub fn from_le(b: &[u8]) -> Self {
        Self {
            id: u32_at(b, 0),
            unique_id: u32_at(b, 4),
            position: Vec3D::from_le(&b[8..20]),
            rotation: Vec3D::from_le(&b[20..32]),
            bounds: AaBox3D::from_le(&b[32..56]),
            flags: u16_at(b, 56),
            doodad_set: u16_at(b, 58),
            name_set: u16_at(b, 60),
            scale: u16_at(b, 62),
        }
    }
}

/// `ADTOutputCache`: one spawn record without its leading `mapID`, with the flags
/// stripped of `MOD_PARENT_SPAWN`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdtOutputCache {
    pub flags: u8,
    pub data: Vec<u8>,
}

/// Splits a string-list chunk (`MMDX`/`MWMO`) the way the C++ loops do:
/// `while (p < buf + size) { std::string path(p); ...; p += strlen(p) + 1; }`.
pub fn split_name_chunk(buf: &[u8]) -> Vec<Vec<u8>> {
    let mut names = Vec::new();
    let mut p = 0usize;
    while p < buf.len() {
        let name = c_str(&buf[p..]).to_vec();
        p += name.len() + 1;
        names.push(name);
    }
    names
}

/// Port of `class ADTFile`.
pub struct AdtFile {
    file: CascFile,
    cacheable: bool,
    dirfile_cache: Option<Vec<AdtOutputCache>>,
    pub wmo_instance_names: Vec<Vec<u8>>,
    pub model_instance_names: Vec<Vec<u8>>,
}

impl AdtFile {
    pub fn new(file: CascFile, cache: bool) -> Self {
        Self {
            file,
            cacheable: cache,
            dirfile_cache: None,
            wmo_instance_names: Vec::new(),
            model_instance_names: Vec::new(),
        }
    }

    /// `ADTFile::init`.
    pub fn init(&mut self, ctx: &mut VmapExport<'_>, map_num: u32, original_map_id: u32) -> bool {
        if self.dirfile_cache.is_some() {
            return self.init_from_cache(ctx, map_num, original_map_id);
        }

        if self.file.is_eof() {
            return false;
        }

        let Some(mut dirfile) = ctx.open_dirfile() else {
            return false;
        };

        if self.cacheable {
            self.dirfile_cache = Some(Vec::new());
        }

        let mut size = 0u32;
        let mut fourcc = [0u8; 4];
        while !self.file.is_eof() {
            read_chunk_header(&mut self.file, &mut fourcc, &mut size);
            let nextpos = self.file.get_pos() + size as usize;

            match &fourcc {
                b"MMDX" if size != 0 => {
                    let buf = self.file.read_vec(size as usize);
                    for mut path in split_name_chunk(&buf) {
                        self.model_instance_names.push(normalized_plain_name(&path));
                        ctx.extract_single_model(&mut path);
                    }
                }
                b"MWMO" if size != 0 => {
                    let buf = self.file.read_vec(size as usize);
                    for mut path in split_name_chunk(&buf) {
                        self.wmo_instance_names.push(normalized_plain_name(&path));
                        ctx.extract_single_wmo(&mut path);
                    }
                }
                b"MDDF" if size != 0 => {
                    let doodad_count = size as usize / MDDF_SIZE;
                    for _ in 0..doodad_count {
                        let doodad_def = Mddf::from_le(&self.file.read_vec(MDDF_SIZE));
                        if doodad_def.flags & 0x40 == 0 {
                            // C++ indexes ModelInstanceNames[Id] unchecked
                            let Some(name) = self.model_instance_names.get(doodad_def.id as usize)
                            else {
                                continue;
                            };
                            model::extract(
                                &doodad_def,
                                name,
                                map_num,
                                original_map_id,
                                &ctx.work_dir,
                                &mut ctx.unique_ids,
                                &mut dirfile.data,
                                self.dirfile_cache.as_mut(),
                            );
                        } else {
                            let mut file_name = file_data_id_name(doodad_def.id);
                            ctx.extract_single_model(&mut file_name);
                            model::extract(
                                &doodad_def,
                                &file_name,
                                map_num,
                                original_map_id,
                                &ctx.work_dir,
                                &mut ctx.unique_ids,
                                &mut dirfile.data,
                                self.dirfile_cache.as_mut(),
                            );
                        }
                    }

                    self.model_instance_names.clear();
                }
                b"MODF" if size != 0 => {
                    let map_object_count = size as usize / MODF_SIZE;
                    for _ in 0..map_object_count {
                        let map_obj_def = Modf::from_le(&self.file.read_vec(MODF_SIZE));
                        let name = if map_obj_def.flags & 0x8 == 0 {
                            // C++ indexes WmoInstanceNames[Id] unchecked
                            let Some(name) = self.wmo_instance_names.get(map_obj_def.id as usize)
                            else {
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
                            false,
                            map_num,
                            original_map_id,
                            &mut dirfile.data,
                            self.dirfile_cache.as_mut(),
                        );
                    }

                    self.wmo_instance_names.clear();
                }
                _ => {}
            }

            self.file.seek(nextpos);
        }

        self.file.close();
        drop(dirfile); // fclose(dirfile)
        true
    }

    /// `ADTFile::initFromCache`.
    fn init_from_cache(
        &mut self,
        ctx: &mut VmapExport<'_>,
        map_num: u32,
        original_map_id: u32,
    ) -> bool {
        let cache = self.dirfile_cache.as_ref().expect("cache present");
        if cache.is_empty() {
            return true;
        }

        let Some(mut dirfile) = ctx.open_dirfile() else {
            return false;
        };

        for cached in cache {
            dirfile.data.extend_from_slice(&map_num.to_le_bytes());
            let mut flags = cached.flags;
            if map_num != original_map_id {
                flags |= MOD_PARENT_SPAWN;
            }
            dirfile.data.push(flags);
            dirfile.data.extend_from_slice(&cached.data);
        }

        drop(dirfile); // fclose(dirfile)
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_records_decode() {
        let mut b = Vec::new();
        b.extend_from_slice(&7u32.to_le_bytes());
        b.extend_from_slice(&99u32.to_le_bytes());
        for f in [1.0f32, 2.0, 3.0, 10.0, 20.0, 30.0] {
            b.extend_from_slice(&f.to_le_bytes());
        }
        b.extend_from_slice(&2048u16.to_le_bytes());
        b.extend_from_slice(&0x40u16.to_le_bytes());
        assert_eq!(b.len(), MDDF_SIZE);
        let d = Mddf::from_le(&b);
        assert_eq!(d.id, 7);
        assert_eq!(d.unique_id, 99);
        assert_eq!(d.position, Vec3D::new(1.0, 2.0, 3.0));
        assert_eq!(d.rotation, Vec3D::new(10.0, 20.0, 30.0));
        assert_eq!((d.scale, d.flags), (2048, 0x40));

        let mut b = Vec::new();
        b.extend_from_slice(&3u32.to_le_bytes());
        b.extend_from_slice(&0xDEADu32.to_le_bytes());
        for i in 0..12 {
            b.extend_from_slice(&(i as f32).to_le_bytes());
        }
        for v in [0x8u16, 2, 1, 1024] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(b.len(), MODF_SIZE);
        let m = Modf::from_le(&b);
        assert_eq!(m.bounds.min, Vec3D::new(6.0, 7.0, 8.0));
        assert_eq!(m.bounds.max, Vec3D::new(9.0, 10.0, 11.0));
        assert_eq!(
            (m.flags, m.doodad_set, m.name_set, m.scale),
            (8, 2, 1, 1024)
        );
    }

    #[test]
    fn name_chunks_split_on_nul_including_padding() {
        let names = split_name_chunk(b"a\\b.m2\0c.m2\0\0");
        assert_eq!(
            names,
            vec![b"a\\b.m2".to_vec(), b"c.m2".to_vec(), Vec::new()]
        );
    }
}
