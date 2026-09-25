//! Port of `src/tools/vmap4_extractor/wmo.{h,cpp}`: WMO root (`WMORoot::open`,
//! `WMORoot::ConvertToVMAPRootWmo`), WMO groups (`WMOGroup::open`,
//! `WMOGroup::ConvertToVMAPGroupWmo`, `WMOGroup::GetLiquidTypeId`, `WMOGroup::ShouldSkip`)
//! and the WMO spawn writer (`MapObject::Extract`).

use std::collections::HashSet;
use std::path::Path;

use crate::adtfile::{AdtOutputCache, Modf};
use crate::cascfile::{CascFile, CascSource, c_str, read_chunk_header};
use crate::names::file_data_id_name;
use crate::std_unordered_set::StdUnorderedSetU16;
use crate::vec3d::{AaBox3D, Vec3D, f32_at, f32_vec, u16_at, u16_vec, u32_at, u32_vec};
use crate::vmapexport::{
    MOD_HAS_BOUND, MOD_PARENT_SPAWN, RAW_VMAP_MAGIC, UniqueObjectIds, read_model_vertex_count,
};

// MOPY flags (`enum MopyFlags`)
pub const WMO_MATERIAL_DETAIL: u16 = 0x04;
pub const WMO_MATERIAL_COLLISION: u16 = 0x08;
pub const WMO_MATERIAL_RENDER: u16 = 0x20;

/// `WMO::MODS` (32 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub name: [u8; 20],
    /// index of first doodad instance in this set
    pub start_index: u32,
    /// number of doodad instances in this set
    pub count: u32,
}

pub const MODS_SIZE: usize = 32;

impl Mods {
    fn from_le(b: &[u8]) -> Self {
        let mut name = [0u8; 20];
        name.copy_from_slice(&b[0..20]);
        Self {
            name,
            start_index: u32_at(b, 20),
            count: u32_at(b, 24),
        }
    }
}

/// `WMO::MODD` (40 bytes; `NameIndex` is a 24-bit bitfield).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Modd {
    pub name_index: u32,
    pub position: Vec3D,
    /// `Quaternion Rotation` as `X, Y, Z, W`.
    pub rotation: [f32; 4],
    pub scale: f32,
    pub color: u32,
}

pub const MODD_SIZE: usize = 40;

impl Modd {
    fn from_le(b: &[u8]) -> Self {
        Self {
            name_index: u32_at(b, 0) & 0x00FF_FFFF,
            position: Vec3D::from_le(&b[4..16]),
            rotation: [f32_at(b, 16), f32_at(b, 20), f32_at(b, 24), f32_at(b, 28)],
            scale: f32_at(b, 32),
            color: u32_at(b, 36),
        }
    }
}

/// `533.33333f * 32`: offset applied to global (WDT) WMO placements.
#[allow(clippy::excessive_precision)]
pub const GLOBAL_WMO_OFFSET: f32 = 533.333_33 * 32.0;

/// `fixCoords`: "for whatever reason a certain company just can't stick to one
/// coordinate system..." — `(z, x, y)`.
pub fn fix_coords(v: Vec3D) -> Vec3D {
    Vec3D::new(v.z, v.x, v.y)
}

/// `WMODoodadData`.
#[derive(Debug, Clone, Default)]
pub struct WmoDoodadData {
    pub sets: Vec<Mods>,
    /// `MODN` chunk copy (doodads referenced by name offset).
    pub paths: Option<Vec<u8>>,
    /// `MODI` chunk (doodads referenced by FileDataID).
    pub file_data_ids: Option<Vec<u32>>,
    pub spawns: Vec<Modd>,
    pub references: StdUnorderedSetU16,
}

/// Port of `class WMORoot`.
pub struct WmoRoot {
    filename: String,
    pub color: u32,
    pub n_textures: u32,
    pub n_groups: u32,
    pub n_portals: u32,
    pub n_lights: u32,
    pub n_doodad_names: u32,
    pub n_doodad_defs: u32,
    pub n_doodad_sets: u32,
    pub root_wmo_id: u32,
    pub bbcorn1: [f32; 3],
    pub bbcorn2: [f32; 3],
    pub flags: u16,
    pub num_lod: u16,
    pub group_names: Vec<u8>,
    pub doodad_data: WmoDoodadData,
    pub valid_doodad_names: HashSet<u32>,
    pub group_file_data_ids: Vec<u32>,
}

impl WmoRoot {
    pub fn new(filename: String) -> Self {
        Self {
            filename,
            color: 0,
            n_textures: 0,
            n_groups: 0,
            n_portals: 0,
            n_lights: 0,
            n_doodad_names: 0,
            n_doodad_defs: 0,
            n_doodad_sets: 0,
            root_wmo_id: 0,
            bbcorn1: [0.0; 3],
            bbcorn2: [0.0; 3],
            flags: 0,
            num_lod: 0,
            group_names: Vec::new(),
            doodad_data: WmoDoodadData::default(),
            valid_doodad_names: HashSet::new(),
            group_file_data_ids: Vec::new(),
        }
    }

    /// `WMORoot::open`. `extract_single_model` is `ExtractSingleModel` (called for every
    /// doodad model named by `MODN`/`MODI`).
    pub fn open(
        &mut self,
        casc: &dyn CascSource,
        extract_single_model: &mut dyn FnMut(&mut Vec<u8>) -> bool,
    ) -> bool {
        let f = CascFile::open_name(casc, &self.filename, true);
        self.open_file(f, extract_single_model)
    }

    /// Body of `WMORoot::open` once the file is loaded.
    pub fn open_file(
        &mut self,
        mut f: CascFile,
        extract_single_model: &mut dyn FnMut(&mut Vec<u8>) -> bool,
    ) -> bool {
        if f.is_eof() {
            println!("No such file.");
            return false;
        }

        let mut size = 0u32;
        let mut fourcc = [0u8; 4];

        while !f.is_eof() {
            read_chunk_header(&mut f, &mut fourcc, &mut size);
            let nextpos = f.get_pos() + size as usize;

            match &fourcc {
                b"MOHD" => {
                    f.read_u32(&mut self.n_textures);
                    f.read_u32(&mut self.n_groups);
                    f.read_u32(&mut self.n_portals);
                    f.read_u32(&mut self.n_lights);
                    f.read_u32(&mut self.n_doodad_names);
                    f.read_u32(&mut self.n_doodad_defs);
                    f.read_u32(&mut self.n_doodad_sets);
                    f.read_u32(&mut self.color);
                    f.read_u32(&mut self.root_wmo_id);
                    for v in &mut self.bbcorn1 {
                        f.read_f32(v);
                    }
                    for v in &mut self.bbcorn2 {
                        f.read_f32(v);
                    }
                    f.read_u16(&mut self.flags);
                    f.read_u16(&mut self.num_lod);
                }
                b"MODS" => {
                    let raw = f.read_vec(size as usize);
                    self.doodad_data.sets = raw
                        .as_chunks::<MODS_SIZE>()
                        .0
                        .iter()
                        .map(|c| Mods::from_le(c))
                        .collect();
                }
                b"MODN" => {
                    assert!(
                        self.doodad_data.file_data_ids.is_none(),
                        "ASSERT(!DoodadData.FileDataIds)"
                    );
                    let chunk_start = f.get_pos();
                    let buffer = f.get_buffer();
                    let end = chunk_start + size as usize;
                    let copy_end = end.min(buffer.len());
                    let mut paths = buffer.get(chunk_start..copy_end).unwrap_or(&[]).to_vec();
                    paths.resize(size as usize, 0);
                    let mut ptr = chunk_start;
                    let mut names = Vec::new();
                    while ptr < end {
                        let path = c_str(buffer.get(ptr..).unwrap_or(&[])).to_vec();
                        let doodad_name_index = (ptr - chunk_start) as u32;
                        ptr += path.len() + 1;
                        names.push((doodad_name_index, path));
                    }
                    self.doodad_data.paths = Some(paths);
                    for (doodad_name_index, mut path) in names {
                        if extract_single_model(&mut path) {
                            self.valid_doodad_names.insert(doodad_name_index);
                        }
                    }
                }
                b"MODI" => {
                    assert!(
                        self.doodad_data.paths.is_none(),
                        "ASSERT(!DoodadData.Paths)"
                    );
                    // f.read(FileDataIds, size); size / sizeof(uint32) ids
                    let raw = f.read_vec(size as usize);
                    let ids: Vec<u32> = u32_vec(&raw);
                    for (i, &id) in ids.iter().enumerate() {
                        if id == 0 {
                            continue;
                        }
                        let mut path = file_data_id_name(id);
                        if extract_single_model(&mut path) {
                            self.valid_doodad_names.insert(i as u32);
                        }
                    }
                    self.doodad_data.file_data_ids = Some(ids);
                }
                b"MODD" => {
                    let raw = f.read_vec(size as usize);
                    self.doodad_data.spawns = raw
                        .as_chunks::<MODD_SIZE>()
                        .0
                        .iter()
                        .map(|c| Modd::from_le(c))
                        .collect();
                }
                b"MOGN" => {
                    self.group_names = f.read_vec(size as usize);
                }
                b"GFID" => {
                    // only the first (most detailed) LOD is read
                    for _ in 0..self.n_groups {
                        let mut file_data_id = 0u32;
                        f.read_u32(&mut file_data_id);
                        if file_data_id != 0 {
                            self.group_file_data_ids.push(file_data_id);
                        }
                    }
                }
                _ => {}
            }
            f.seek(nextpos);
        }
        f.close();
        true
    }

    /// `WMORoot::ConvertToVMAPRootWmo`.
    pub fn convert_to_vmap_root_wmo(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(RAW_VMAP_MAGIC);
        let n_vectors: u32 = 0;
        out.extend_from_slice(&n_vectors.to_le_bytes()); // will be filled later
        out.extend_from_slice(&self.n_groups.to_le_bytes());
        out.extend_from_slice(&self.root_wmo_id.to_le_bytes());
    }
}

/// `WMOLiquidHeader` (packed, 30 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WmoLiquidHeader {
    pub xverts: i32,
    pub yverts: i32,
    pub xtiles: i32,
    pub ytiles: i32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub material: i16,
}

pub const WMO_LIQUID_HEADER_SIZE: usize = 30;
/// `sizeof(WMOLiquidVert)`.
const WMO_LIQUID_VERT_SIZE: i32 = 8;

impl WmoLiquidHeader {
    fn from_le(b: &[u8]) -> Self {
        Self {
            xverts: u32_at(b, 0) as i32,
            yverts: u32_at(b, 4) as i32,
            xtiles: u32_at(b, 8) as i32,
            ytiles: u32_at(b, 12) as i32,
            pos_x: f32_at(b, 16),
            pos_y: f32_at(b, 20),
            pos_z: f32_at(b, 24),
            material: u16_at(b, 28) as i16,
        }
    }

    fn write_le(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.xverts.to_le_bytes());
        out.extend_from_slice(&self.yverts.to_le_bytes());
        out.extend_from_slice(&self.xtiles.to_le_bytes());
        out.extend_from_slice(&self.ytiles.to_le_bytes());
        out.extend_from_slice(&self.pos_x.to_le_bytes());
        out.extend_from_slice(&self.pos_y.to_le_bytes());
        out.extend_from_slice(&self.pos_z.to_le_bytes());
        out.extend_from_slice(&self.material.to_le_bytes());
    }
}

/// Port of `class WMOGroup`.
pub struct WmoGroup {
    filename: String,
    /// `MPY2` (from `MOPY` bytes widened to `uint16`, or the `MPY2` chunk).
    pub mpy2: Vec<u16>,
    /// `MOVX` (from `MOVI` widened to `uint32`, or the `MOVX` chunk).
    pub movx: Vec<u32>,
    pub movt: Vec<f32>,
    pub moba: Vec<u16>,
    pub hlq: Option<WmoLiquidHeader>,
    /// `WMOLiquidVert` heights (the only field used).
    pub liqu_ex_heights: Vec<f32>,
    pub liqu_bytes: Vec<u8>,
    pub group_name: i32,
    pub desc_group_name: i32,
    pub mogp_flags: i32,
    pub bbcorn1: [f32; 3],
    pub bbcorn2: [f32; 3],
    pub mopr_idx: u16,
    pub mopr_n_items: u16,
    pub n_batch_a: u16,
    pub n_batch_b: u16,
    pub n_batch_c: u32,
    pub fog_idx: u32,
    pub group_liquid: u32,
    pub group_wmo_id: u32,
    pub moba_size: i32,
    pub liqu_ex_size: i32,
    /// number when loaded
    pub n_vertices: u32,
    /// number when loaded
    pub n_triangles: i32,
    pub liquflags: u32,
    pub doodad_references: Vec<u16>,
}

impl WmoGroup {
    pub fn new(filename: String) -> Self {
        Self {
            filename,
            mpy2: Vec::new(),
            movx: Vec::new(),
            movt: Vec::new(),
            moba: Vec::new(),
            hlq: None,
            liqu_ex_heights: Vec::new(),
            liqu_bytes: Vec::new(),
            group_name: 0,
            desc_group_name: 0,
            mogp_flags: 0,
            bbcorn1: [0.0; 3],
            bbcorn2: [0.0; 3],
            mopr_idx: 0,
            mopr_n_items: 0,
            n_batch_a: 0,
            n_batch_b: 0,
            n_batch_c: 0,
            fog_idx: 0,
            group_liquid: 0,
            group_wmo_id: 0,
            moba_size: 0,
            liqu_ex_size: 0,
            n_vertices: 0,
            n_triangles: 0,
            liquflags: 0,
            doodad_references: Vec::new(),
        }
    }

    /// `WMOGroup::open`.
    pub fn open(&mut self, casc: &dyn CascSource, root: &WmoRoot) -> bool {
        let f = CascFile::open_name(casc, &self.filename, true);
        self.open_file(f, root)
    }

    /// Body of `WMOGroup::open` once the file is loaded.
    pub fn open_file(&mut self, mut f: CascFile, root: &WmoRoot) -> bool {
        if f.is_eof() {
            println!("No such file.");
            return false;
        }
        let mut size = 0u32;
        let mut fourcc = [0u8; 4];
        while !f.is_eof() {
            read_chunk_header(&mut f, &mut fourcc, &mut size);
            if &fourcc == b"MOGP" {
                // size specified in MOGP chunk is all the other chunks combined, adjust to read MOGP-only
                size = 68;
            }

            let nextpos = f.get_pos() + size as usize;
            match &fourcc {
                b"MOGP" => {
                    f.read_i32(&mut self.group_name);
                    f.read_i32(&mut self.desc_group_name);
                    f.read_i32(&mut self.mogp_flags);
                    for v in &mut self.bbcorn1 {
                        f.read_f32(v);
                    }
                    for v in &mut self.bbcorn2 {
                        f.read_f32(v);
                    }
                    f.read_u16(&mut self.mopr_idx);
                    f.read_u16(&mut self.mopr_n_items);
                    f.read_u16(&mut self.n_batch_a);
                    f.read_u16(&mut self.n_batch_b);
                    f.read_u32(&mut self.n_batch_c);
                    f.read_u32(&mut self.fog_idx);
                    f.read_u32(&mut self.group_liquid);
                    f.read_u32(&mut self.group_wmo_id);

                    // according to WoW.Dev Wiki:
                    if root.flags & 4 != 0 {
                        self.group_liquid = self.get_liquid_type_id(self.group_liquid);
                    } else if self.group_liquid == 15 {
                        self.group_liquid = 0;
                    } else {
                        self.group_liquid =
                            self.get_liquid_type_id(self.group_liquid.wrapping_add(1));
                    }

                    if self.group_liquid != 0 {
                        self.liquflags |= 2;
                    }
                }
                b"MOPY" => {
                    let mopy = f.read_vec(size as usize);
                    self.n_triangles = size as i32 / 2;
                    self.mpy2 = mopy.iter().map(|&b| u16::from(b)).collect();
                }
                b"MPY2" => {
                    let raw = f.read_vec(size as usize);
                    self.n_triangles = size as i32 / 4;
                    self.mpy2 = u16_vec(&raw);
                }
                b"MOVI" => {
                    let raw = f.read_vec(size as usize);
                    self.movx = u16_vec(&raw).into_iter().map(u32::from).collect();
                }
                b"MOVX" => {
                    let raw = f.read_vec(size as usize);
                    self.movx = u32_vec(&raw);
                }
                b"MOVT" => {
                    let raw = f.read_vec(size as usize);
                    self.movt = f32_vec(&raw);
                    self.n_vertices = size / 12;
                }
                b"MOBA" => {
                    let raw = f.read_vec(size as usize);
                    self.moba = u16_vec(&raw);
                    self.moba_size = (size / 2) as i32;
                }
                b"MODR" => {
                    let raw = f.read_vec(size as usize);
                    self.doodad_references = u16_vec(&raw);
                }
                b"MLIQ" => {
                    self.liquflags |= 1;
                    let hlq = WmoLiquidHeader::from_le(&f.read_vec(WMO_LIQUID_HEADER_SIZE));
                    let n_verts = hlq.xverts.wrapping_mul(hlq.yverts).max(0);
                    self.liqu_ex_size = WMO_LIQUID_VERT_SIZE.wrapping_mul(n_verts);
                    let raw = f.read_vec(n_verts as usize * WMO_LIQUID_VERT_SIZE as usize);
                    self.liqu_ex_heights = raw
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|c| f32_at(c, 4))
                        .collect();
                    let n_liqu_bytes = hlq.xtiles.wrapping_mul(hlq.ytiles).max(0);
                    self.liqu_bytes = f.read_vec(n_liqu_bytes as usize);
                    self.hlq = Some(hlq);

                    // Determine legacy liquid type
                    if self.group_liquid == 0 {
                        for i in 0..n_liqu_bytes as usize {
                            let b = self.liqu_bytes[i] & 0xF;
                            if b != 15 {
                                self.group_liquid = self.get_liquid_type_id(u32::from(b) + 1);
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
            f.seek(nextpos);
        }
        f.close();
        true
    }

    /// `WMOGroup::ConvertToVMAPGroupWmo`: appends the group record, returns the number of
    /// collision triangles written.
    pub fn convert_to_vmap_group_wmo(&self, out: &mut Vec<u8>, precise_vector_data: bool) -> i32 {
        out.extend_from_slice(&self.mogp_flags.to_le_bytes());
        out.extend_from_slice(&self.group_wmo_id.to_le_bytes());
        // group bound
        for v in self.bbcorn1.iter().chain(self.bbcorn2.iter()) {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.liquflags.to_le_bytes());

        // GRP: batch index counts (MOBA entries are 12 uint16, [8] is the index count)
        out.extend_from_slice(b"GRP ");
        let moba_batch = self.moba_size / 12;
        let moba_ex: Vec<i32> = (8..self.moba_size.max(0) as usize)
            .step_by(12)
            .map(|i| i32::from(self.moba.get(i).copied().unwrap_or(0)))
            .collect();
        let moba_size_grp = moba_batch * 4 + 4;
        out.extend_from_slice(&moba_size_grp.to_le_bytes());
        out.extend_from_slice(&moba_batch.to_le_bytes());
        for v in &moba_ex {
            out.extend_from_slice(&v.to_le_bytes());
        }

        let n_col_triangles;
        if precise_vector_data {
            let n_indexes = (self.n_triangles * 3) as u32;
            out.extend_from_slice(b"INDX");
            let wsize = 4u32.wrapping_add(2u32.wrapping_mul(n_indexes));
            out.extend_from_slice(&wsize.to_le_bytes());
            out.extend_from_slice(&n_indexes.to_le_bytes());
            for i in 0..n_indexes as usize {
                let v = self.movx.get(i).copied().unwrap_or(0);
                out.extend_from_slice(&v.to_le_bytes());
            }

            out.extend_from_slice(b"VERT");
            let wsize = 4u32.wrapping_add(12u32.wrapping_mul(self.n_vertices));
            out.extend_from_slice(&wsize.to_le_bytes());
            out.extend_from_slice(&self.n_vertices.to_le_bytes());
            for i in 0..self.n_vertices as usize * 3 {
                let v = self.movt.get(i).copied().unwrap_or(0.0);
                out.extend_from_slice(&v.to_le_bytes());
            }

            n_col_triangles = self.n_triangles;
        } else {
            //-------INDX------------------------------------
            //-------MOPY/MPY2--------
            let n_triangles = self.n_triangles.max(0) as usize;
            let n_vertices = self.n_vertices as usize;
            let mut movx_ex: Vec<u32> = Vec::with_capacity(n_triangles * 3);
            let mut index_renum: Vec<i32> = vec![-1; n_vertices];
            for i in 0..n_triangles {
                let flags = self.mpy2.get(2 * i).copied().unwrap_or(0);
                // Skip no collision triangles
                let is_render_face =
                    (flags & WMO_MATERIAL_RENDER) != 0 && (flags & WMO_MATERIAL_DETAIL) == 0;
                let is_collision = (flags & WMO_MATERIAL_COLLISION) != 0 || is_render_face;
                if !is_collision {
                    continue;
                }

                // Use this triangle
                for j in 0..3 {
                    let index = self.movx.get(3 * i + j).copied().unwrap_or(0);
                    assert!(
                        (index as usize) < n_vertices,
                        "ASSERT(MovxEx[i] < nVertices) in {}",
                        self.filename
                    );
                    index_renum[index as usize] = 1;
                    movx_ex.push(index);
                }
            }
            n_col_triangles = (movx_ex.len() / 3) as i32;

            // assign new vertex index numbers
            let mut n_col_vertices: u32 = 0;
            for r in &mut index_renum {
                if *r == 1 {
                    *r = n_col_vertices as i32;
                    n_col_vertices += 1;
                }
            }

            // translate triangle indices to new numbers
            for v in &mut movx_ex {
                *v = index_renum[*v as usize] as u32;
            }

            // write triangle indices
            out.extend_from_slice(&0x5844_4E49_i32.to_le_bytes()); // "INDX"
            out.extend_from_slice(&(n_col_triangles * 6 + 4).to_le_bytes());
            out.extend_from_slice(&(n_col_triangles * 3).to_le_bytes());
            for v in &movx_ex {
                out.extend_from_slice(&v.to_le_bytes());
            }

            // write vertices
            out.extend_from_slice(&0x5452_4556_u32.to_le_bytes()); // "VERT"
            out.extend_from_slice(&(n_col_vertices * 12 + 4).to_le_bytes());
            out.extend_from_slice(&n_col_vertices.to_le_bytes());
            for (i, r) in index_renum.iter().enumerate() {
                if *r >= 0 {
                    for k in 0..3 {
                        let v = self.movt.get(3 * i + k).copied().unwrap_or(0.0);
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }

        //------LIQU------------------------
        if self.liquflags & 3 != 0 {
            let mut liqu_total_size: i32 = 4;
            let hlq = self.hlq.unwrap_or_default();
            if self.liquflags & 1 != 0 {
                liqu_total_size += WMO_LIQUID_HEADER_SIZE as i32;
                liqu_total_size += self.liqu_ex_size / WMO_LIQUID_VERT_SIZE * 4;
                liqu_total_size += hlq.xtiles.wrapping_mul(hlq.ytiles);
            }
            out.extend_from_slice(&0x5551_494C_i32.to_le_bytes()); // "LIQU"
            out.extend_from_slice(&liqu_total_size.to_le_bytes());

            out.extend_from_slice(&self.group_liquid.to_le_bytes());
            if self.liquflags & 1 != 0 {
                hlq.write_le(out);
                // only need height values, the other values are unknown anyway
                for h in &self.liqu_ex_heights {
                    out.extend_from_slice(&h.to_le_bytes());
                }
                // todo: compress to bit field
                out.extend_from_slice(&self.liqu_bytes);
            }
        }

        n_col_triangles
    }

    /// `WMOGroup::GetLiquidTypeId`.
    pub fn get_liquid_type_id(&self, liquid_type_id: u32) -> u32 {
        if liquid_type_id < 21 && liquid_type_id != 0 {
            match ((liquid_type_id as u8).wrapping_sub(1)) & 3 {
                0 => return u32::from((self.mogp_flags & 0x80000) != 0) + 13,
                1 => return 14,
                2 => return 19,
                3 => return 20,
                _ => {}
            }
        }
        liquid_type_id
    }

    /// `WMOGroup::ShouldSkip`.
    pub fn should_skip(&self, root: &WmoRoot) -> bool {
        // skip unreachable
        if self.mogp_flags & 0x80 != 0 {
            return true;
        }

        // skip antiportals
        if self.mogp_flags & 0x0400_0000 != 0 {
            return true;
        }

        if self.group_name >= 0
            && (self.group_name as usize) < root.group_names.len()
            && c_str(&root.group_names[self.group_name as usize..]) == b"antiportal"
        {
            return true;
        }

        false
    }
}

/// `MapObject::Extract`.
#[allow(clippy::too_many_arguments)]
pub fn map_object_extract(
    map_obj_def: &Modf,
    wmo_inst_name: &[u8],
    is_global_wmo: bool,
    map_id: u32,
    original_map_id: u32,
    work_dir: &Path,
    unique_ids: &mut UniqueObjectIds,
    dirfile: &mut Vec<u8>,
    dirfile_cache: Option<&mut Vec<AdtOutputCache>>,
) {
    // destructible wmo, do not dump. we can handle the vmap for these
    // in dynamic tree (gameobject vmaps)
    if map_obj_def.flags & 0x1 != 0 {
        return;
    }

    //-----------add_in _dir_file----------------
    let n_vertices = match read_model_vertex_count(work_dir, wmo_inst_name) {
        Ok(Some(n)) => n,
        Ok(None) => return,
        Err(path) => {
            println!(
                "WMOInstance::WMOInstance: couldn't open {}",
                path.to_string_lossy()
            );
            return;
        }
    };
    if n_vertices == 0 {
        return;
    }

    let mut position = fix_coords(map_obj_def.position);
    let mut bounds = AaBox3D {
        min: fix_coords(map_obj_def.bounds.min),
        max: fix_coords(map_obj_def.bounds.max),
    };

    if is_global_wmo {
        let offset = Vec3D::new(GLOBAL_WMO_OFFSET, GLOBAL_WMO_OFFSET, 0.0);
        position.add_assign(offset);
        bounds.add_assign(offset);
    }

    let mut scale = 1.0f32;
    if map_obj_def.flags & 0x4 != 0 {
        scale = f32::from(map_obj_def.scale) / 1024.0;
    }
    let unique_id = unique_ids.generate(map_obj_def.unique_id, 0);
    let mut flags = MOD_HAS_BOUND;
    let name_set = map_obj_def.name_set as u8;
    if map_id != original_map_id {
        flags |= MOD_PARENT_SPAWN;
    }

    let name = c_str(wmo_inst_name);
    let mut data = Vec::with_capacity(70 + name.len());
    data.push(name_set);
    data.extend_from_slice(&unique_id.to_le_bytes());
    position.write_le(&mut data);
    map_obj_def.rotation.write_le(&mut data);
    data.extend_from_slice(&scale.to_le_bytes());
    bounds.write_le(&mut data);
    data.extend_from_slice(&(name.len() as u32).to_le_bytes());
    data.extend_from_slice(name);

    // write mapID, Flags, NameSet, UniqueId, Pos, Rot, Scale, Bound_lo, Bound_hi, name
    dirfile.extend_from_slice(&map_id.to_le_bytes());
    dirfile.push(flags);
    dirfile.extend_from_slice(&data);

    if let Some(cache) = dirfile_cache {
        cache.push(AdtOutputCache {
            flags: flags & !MOD_PARENT_SPAWN,
            data,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = vec![id[3], id[2], id[1], id[0]];
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    fn le<T: Copy, const N: usize>(v: &[T], f: impl Fn(T) -> [u8; N]) -> Vec<u8> {
        v.iter().flat_map(|x| f(*x)).collect()
    }

    fn root() -> WmoRoot {
        let mut mohd = le(
            &[2u32, 3, 0, 0, 2, 2, 1, 0xAABB_CCDD, 321],
            u32::to_le_bytes,
        );
        mohd.extend(le(&[-1.0f32, -2.0, -3.0, 1.0, 2.0, 3.0], f32::to_le_bytes));
        mohd.extend(le(&[0u16, 0], u16::to_le_bytes));
        let mut file = chunk(b"MVER", &17u32.to_le_bytes());
        file.extend(chunk(b"MOHD", &mohd));
        file.extend(chunk(b"MOGN", b"\0antiportal\0"));
        let mut mods = vec![0u8; 20];
        mods.extend(le(&[0u32, 2, 0], u32::to_le_bytes));
        file.extend(chunk(b"MODS", &mods));
        file.extend(chunk(
            b"MODI",
            &le(&[0x1234u32, 0, 0x5678], u32::to_le_bytes),
        ));
        let mut modd = Vec::new();
        for name_index in [0xFF00_0000u32, 2] {
            modd.extend(le(&[name_index], u32::to_le_bytes));
            modd.extend(le(
                &[1.0f32, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0, 1.5],
                f32::to_le_bytes,
            ));
            modd.extend(le(&[0u32], u32::to_le_bytes));
        }
        file.extend(chunk(b"MODD", &modd));
        file.extend(chunk(b"GFID", &le(&[0x10u32, 0, 0x11], u32::to_le_bytes)));

        let mut r = WmoRoot::new("root".into());
        let mut asked = Vec::new();
        assert!(r.open_file(CascFile::from_bytes(file), &mut |p| {
            asked.push(p.clone());
            p.as_slice() == b"FILE00001234.xxx"
        }));
        assert_eq!(
            asked,
            vec![b"FILE00001234.xxx".to_vec(), b"FILE00005678.xxx".to_vec()]
        );
        r
    }

    #[test]
    fn root_chunks_parse() {
        let r = root();
        assert_eq!((r.n_groups, r.root_wmo_id, r.color), (3, 321, 0xAABB_CCDD));
        assert_eq!(r.bbcorn2, [1.0, 2.0, 3.0]);
        assert_eq!(r.doodad_data.sets.len(), 1);
        assert_eq!(r.doodad_data.sets[0].count, 2);
        assert_eq!(r.doodad_data.file_data_ids, Some(vec![0x1234, 0, 0x5678]));
        // NameIndex is a 24-bit bitfield
        assert_eq!(r.doodad_data.spawns[0].name_index, 0);
        assert_eq!(r.doodad_data.spawns[1].name_index, 2);
        assert_eq!(r.doodad_data.spawns[1].scale, 1.5);
        assert_eq!(r.valid_doodad_names, HashSet::from([0]));
        // GFID: nGroups ids, zero skipped
        assert_eq!(r.group_file_data_ids, vec![0x10, 0x11]);

        let mut out = Vec::new();
        r.convert_to_vmap_root_wmo(&mut out);
        let mut exp = b"VMAP04B\0".to_vec();
        exp.extend(le(&[0u32, 3, 321], u32::to_le_bytes));
        assert_eq!(out, exp);
    }

    fn group_file(name_ofs: i32) -> Vec<u8> {
        let mut hdr = le(&[name_ofs, 0, 0x10], i32::to_le_bytes);
        hdr.extend(le(&[-1.0f32, -1.0, -1.0, 1.0, 1.0, 1.0], f32::to_le_bytes));
        hdr.extend(le(&[0u16; 4], u16::to_le_bytes));
        // nBatchC, fogIdx, groupLiquid (0 -> +1 -> 13 ocean-less water), groupWMOID, pad
        hdr.extend(le(&[0u32, 0, 0, 55, 0, 0], u32::to_le_bytes));
        let mut sub = chunk(b"MOPY", &[0x08, 0, 0x24, 0, 0x20, 0]);
        sub.extend(chunk(
            b"MOVI",
            &le(&[0u16, 1, 2, 2, 1, 3, 3, 4, 1], u16::to_le_bytes),
        ));
        let verts: Vec<f32> = (0..15).map(|i| i as f32).collect();
        sub.extend(chunk(b"MOVT", &le(&verts, f32::to_le_bytes)));
        let mut moba = [0u16; 12];
        moba[8] = 6;
        sub.extend(chunk(b"MOBA", &le(&moba, u16::to_le_bytes)));
        sub.extend(chunk(b"MODR", &le(&[1u16, 0, 7], u16::to_le_bytes)));
        let mut mliq = le(&[2i32, 1, 1, 1], i32::to_le_bytes);
        mliq.extend(le(&[10.0f32, 20.0, 30.0], f32::to_le_bytes));
        mliq.extend(le(&[5i16], i16::to_le_bytes));
        mliq.extend(le(&[0u16, 0], u16::to_le_bytes));
        mliq.extend(le(&[4.5f32], f32::to_le_bytes));
        mliq.extend(le(&[0u16, 0], u16::to_le_bytes));
        mliq.extend(le(&[5.5f32], f32::to_le_bytes));
        mliq.push(0x0F);
        sub.extend(chunk(b"MLIQ", &mliq));
        let mut mogp = hdr;
        mogp.extend(sub);
        let mut file = chunk(b"MVER", &17u32.to_le_bytes());
        file.extend(chunk(b"MOGP", &mogp));
        file
    }

    fn open_group(name_ofs: i32, root: &WmoRoot) -> WmoGroup {
        let mut g = WmoGroup::new("group".into());
        assert!(g.open_file(CascFile::from_bytes(group_file(name_ofs)), root));
        g
    }

    #[test]
    fn group_chunks_parse_and_skip_rules() {
        let r = root();
        let g = open_group(0, &r);
        assert_eq!((g.n_triangles, g.n_vertices, g.moba_size), (3, 5, 12));
        assert_eq!(g.mpy2, vec![0x08, 0, 0x24, 0, 0x20, 0]);
        assert_eq!(g.doodad_references, vec![1, 0, 7]);
        // groupLiquid 0 with root flags & 4 == 0: GetLiquidTypeId(0 + 1) = 13
        assert_eq!(g.group_liquid, 13);
        assert_eq!(g.liquflags, 3);
        assert!(!g.should_skip(&r));
        assert!(open_group(1, &r).should_skip(&r)); // "antiportal"
    }

    #[test]
    fn group_writer_small_layout() {
        let r = root();
        let g = open_group(0, &r);
        let mut out = Vec::new();
        let n = g.convert_to_vmap_group_wmo(&mut out, false);
        // triangle 1 (flags 0x24: render + detail) is not collision
        assert_eq!(n, 2);

        let mut exp = le(&[0x10i32, 55], i32::to_le_bytes);
        exp.extend(le(&[-1.0f32, -1.0, -1.0, 1.0, 1.0, 1.0], f32::to_le_bytes));
        exp.extend(le(&[3u32], u32::to_le_bytes)); // liquflags
        exp.extend(b"GRP ");
        exp.extend(le(&[8i32, 1, 6], i32::to_le_bytes));
        exp.extend(le(&[0x5844_4E49i32, 2 * 6 + 4, 6], i32::to_le_bytes));
        // triangles 0 (0,1,2) and 2 (3,4,1), vertices 0..=4 all used -> identity renumbering
        exp.extend(le(&[0u32, 1, 2, 3, 4, 1], u32::to_le_bytes));
        exp.extend(le(&[0x5452_4556u32, 5 * 12 + 4, 5], u32::to_le_bytes));
        let verts: Vec<f32> = (0..15).map(|i| i as f32).collect();
        exp.extend(le(&verts, f32::to_le_bytes));
        // LIQU: 4 + 30 + 2 heights * 4 + 1 tile byte
        exp.extend(le(&[0x5551_494Ci32, 4 + 30 + 8 + 1], i32::to_le_bytes));
        exp.extend(le(&[13u32], u32::to_le_bytes));
        exp.extend(le(&[2i32, 1, 1, 1], i32::to_le_bytes));
        exp.extend(le(&[10.0f32, 20.0, 30.0], f32::to_le_bytes));
        exp.extend(le(&[5i16], i16::to_le_bytes));
        exp.extend(le(&[4.5f32, 5.5], f32::to_le_bytes));
        exp.push(0x0F);
        assert_eq!(out, exp);
    }

    #[test]
    fn group_writer_precise_layout() {
        let r = root();
        let g = open_group(0, &r);
        let mut out = Vec::new();
        assert_eq!(g.convert_to_vmap_group_wmo(&mut out, true), 3);
        let mut exp = le(&[0x10i32, 55], i32::to_le_bytes);
        exp.extend(le(&[-1.0f32, -1.0, -1.0, 1.0, 1.0, 1.0], f32::to_le_bytes));
        exp.extend(le(&[3u32], u32::to_le_bytes));
        exp.extend(b"GRP ");
        exp.extend(le(&[8i32, 1, 6], i32::to_le_bytes));
        exp.extend(b"INDX");
        // wsize claims 2 bytes per index but 4-byte indices follow (as in the C++)
        exp.extend(le(&[4 + 2 * 9u32, 9], u32::to_le_bytes));
        exp.extend(le(&[0u32, 1, 2, 2, 1, 3, 3, 4, 1], u32::to_le_bytes));
        exp.extend(b"VERT");
        exp.extend(le(&[4 + 12 * 5u32, 5], u32::to_le_bytes));
        let verts: Vec<f32> = (0..15).map(|i| i as f32).collect();
        exp.extend(le(&verts, f32::to_le_bytes));
        let liqu_start = out.len() - (8 + 4 + 30 + 8 + 1);
        assert_eq!(&out[..liqu_start], exp.as_slice());
    }

    #[test]
    fn liquid_type_mapping() {
        let mut g = WmoGroup::new(String::new());
        assert_eq!(g.get_liquid_type_id(1), 13);
        g.mogp_flags = 0x80000;
        assert_eq!(g.get_liquid_type_id(1), 14);
        assert_eq!(g.get_liquid_type_id(2), 14);
        assert_eq!(g.get_liquid_type_id(3), 19);
        assert_eq!(g.get_liquid_type_id(4), 20);
        assert_eq!(g.get_liquid_type_id(5), 14);
        assert_eq!(g.get_liquid_type_id(21), 21);
        assert_eq!(g.get_liquid_type_id(0), 0);
    }
}
