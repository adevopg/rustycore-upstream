//! Port of `src/tools/mmaps_generator/TerrainBuilder.{h,cpp}` (TDB343.24081):
//! `.map` terrain/liquid triangulation (`loadMap`), vmap model geometry
//! (`loadVMap`), `transform`/`copyVertices`/`copyIndices`/`cleanVertices`
//! and `loadOffMeshConnections`.
//!
//! All float expressions keep the C++ operand order and `float` precision so
//! the vertices handed to Recast are bit-identical.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wow_vmap::{LoadResult, MOD_M2, Matrix3, MeshTriangle, VMapManager, Vector3};

use crate::map_defines::{
    MAP_FILEHEADER_SIZE, MAP_HEIGHT_HEADER_SIZE, MAP_LIQUID_HEADER_SIZE, MAP_VERSION_MAGIC,
    MapFileHeader, MapHeightHeader, MapLiquidHeader, height_flags, liquid_header_flags,
    liquid_type_flags, nav_area,
};
use crate::path_common::GeneratorData;

/// `MMAP::Spot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Spot {
    Top = 1,
    Right = 2,
    Left = 3,
    Bottom = 4,
    Entire = 5,
}

impl Spot {
    fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Top,
            2 => Self::Right,
            3 => Self::Left,
            4 => Self::Bottom,
            _ => Self::Entire,
        }
    }
}

/// `MMAP::Grid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grid {
    V8,
    V9,
}

pub const V9_SIZE: i32 = 129;
pub const V9_SIZE_SQ: i32 = V9_SIZE * V9_SIZE;
pub const V8_SIZE: i32 = 128;
pub const V8_SIZE_SQ: i32 = V8_SIZE * V8_SIZE;
/// `GRID_SIZE` (533.3333f).
pub const GRID_SIZE: f32 = 533.3333_f32;
/// `GRID_PART_SIZE = GRID_SIZE / V8_SIZE`.
pub const GRID_PART_SIZE: f32 = GRID_SIZE / V8_SIZE as f32;
/// `INVALID_MAP_LIQ_HEIGHT`.
pub const INVALID_MAP_LIQ_HEIGHT: f32 = -2000.0;
/// `INVALID_MAP_LIQ_HEIGHT_MAX`.
pub const INVALID_MAP_LIQ_HEIGHT_MAX: f32 = 5000.0;

/// `MMAP::OffMeshData`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct OffMeshData {
    pub map_id: u32,
    pub tile_x: u32,
    pub tile_y: u32,
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub bidirectional: bool,
    pub radius: f32,
    pub area_id: u8,
    pub flags: u16,
}

/// `MMAP::MeshData`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshData {
    pub solid_verts: Vec<f32>,
    pub solid_tris: Vec<i32>,

    pub liquid_verts: Vec<f32>,
    pub liquid_tris: Vec<i32>,
    pub liquid_type: Vec<u8>,

    /// `[p0y,p0z,p0x,p1y,p1z,p1x]` per connection.
    pub off_mesh_connections: Vec<f32>,
    pub off_mesh_connection_rads: Vec<f32>,
    pub off_mesh_connection_dirs: Vec<u8>,
    pub off_mesh_connections_areas: Vec<u8>,
    pub off_mesh_connections_flags: Vec<u16>,
}

/// Minimal `FILE*` emulation over a fully read file (`fseek` may point past
/// the end; `fread` then reads nothing).
struct CFile {
    data: Vec<u8>,
    pos: usize,
}

impl CFile {
    fn open(path: &Path) -> Option<Self> {
        std::fs::read(path).ok().map(|data| Self { data, pos: 0 })
    }

    fn seek(&mut self, off: u32) {
        self.pos = off as usize;
    }

    /// `fread(buf, size, count)` — copies what is available, returns the
    /// number of complete items.
    fn read(&mut self, buf: &mut [u8], size: usize) -> usize {
        let avail = self.data.len().saturating_sub(self.pos);
        let n = buf.len().min(avail);
        buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        n.checked_div(size).unwrap_or(0)
    }

    fn read_array<const N: usize>(&mut self) -> Option<[u8; N]> {
        let mut b = [0u8; N];
        (self.read(&mut b, N) == 1).then_some(b)
    }
}

fn read_u16s(f: &mut CFile, out: &mut [u16]) -> usize {
    let mut raw = vec![0u8; out.len() * 2];
    let n = f.read(&mut raw, 2);
    for (o, c) in out.iter_mut().zip(raw.chunks_exact(2)) {
        *o = u16::from_le_bytes([c[0], c[1]]);
    }
    n
}

fn read_f32s(f: &mut CFile, out: &mut [f32]) -> usize {
    let mut raw = vec![0u8; out.len() * 4];
    let n = f.read(&mut raw, 4);
    for (o, c) in out.iter_mut().zip(raw.chunks_exact(4)) {
        *o = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    }
    n
}

/// `MMAP::TerrainBuilder`.
pub struct TerrainBuilder {
    skip_liquid: bool,
    /// Working directory holding `maps/` and `vmaps/` (C++ uses the CWD).
    base: PathBuf,
    data: Arc<GeneratorData>,
}

impl TerrainBuilder {
    pub fn new(skip_liquid: bool, base: impl AsRef<Path>, data: Arc<GeneratorData>) -> Self {
        Self {
            skip_liquid,
            base: base.as_ref().to_path_buf(),
            data,
        }
    }

    /// `TerrainBuilder::usesLiquids`.
    pub fn uses_liquids(&self) -> bool {
        !self.skip_liquid
    }

    /// `TerrainBuilder::getLoopVars`.
    fn get_loop_vars(portion: Spot) -> (i32, i32, i32) {
        match portion {
            Spot::Entire => (0, V8_SIZE_SQ, 1),
            Spot::Top => (0, V8_SIZE, 1),
            Spot::Left => (0, V8_SIZE_SQ - V8_SIZE + 1, V8_SIZE),
            Spot::Right => (V8_SIZE - 1, V8_SIZE_SQ, V8_SIZE),
            Spot::Bottom => (V8_SIZE_SQ - V8_SIZE, V8_SIZE_SQ, 1),
        }
    }

    /// `TerrainBuilder::loadMap(mapID, tileX, tileY, meshData)` — the tile
    /// plus the adjacent edge rows of its four neighbours.
    pub fn load_map(&self, map_id: u32, tile_x: u32, tile_y: u32, mesh: &mut MeshData) {
        if self.load_map_portion(map_id, tile_x, tile_y, mesh, Spot::Entire) {
            self.load_map_portion(map_id, tile_x.wrapping_add(1), tile_y, mesh, Spot::Left);
            self.load_map_portion(map_id, tile_x.wrapping_sub(1), tile_y, mesh, Spot::Right);
            self.load_map_portion(map_id, tile_x, tile_y.wrapping_add(1), mesh, Spot::Top);
            self.load_map_portion(map_id, tile_x, tile_y.wrapping_sub(1), mesh, Spot::Bottom);
        }
    }

    /// `TerrainBuilder::loadMap(mapID, tileX, tileY, meshData, portion)`.
    #[allow(clippy::too_many_lines)]
    pub fn load_map_portion(
        &self,
        map_id: u32,
        tile_x: u32,
        tile_y: u32,
        mesh: &mut MeshData,
        portion: Spot,
    ) -> bool {
        let mut map_file_name = format!("maps/{map_id:04}_{tile_y:02}_{tile_x:02}.map");
        let mut map_file = CFile::open(&self.base.join(&map_file_name));
        if map_file.is_none() {
            let mut parent_map_id = self.data.parent_map_id(map_id);
            while map_file.is_none() && parent_map_id != -1 {
                map_file_name = format!("maps/{parent_map_id:04}_{tile_y:02}_{tile_x:02}.map");
                map_file = CFile::open(&self.base.join(&map_file_name));
                parent_map_id = self.data.parent_map_id(parent_map_id as u32);
            }
        }
        let Some(mut map_file) = map_file else {
            return false;
        };

        let fheader = map_file
            .read_array::<MAP_FILEHEADER_SIZE>()
            .map(|b| MapFileHeader::parse(&b));
        let Some(fheader) = fheader.filter(|h| h.version_magic == MAP_VERSION_MAGIC) else {
            println!("{map_file_name} is the wrong version, please extract new .map files");
            return false;
        };

        map_file.seek(fheader.height_map_offset);
        let mut hheader = MapHeightHeader::default();
        let mut have_terrain = false;
        let mut have_liquid = false;
        if let Some(b) = map_file.read_array::<MAP_HEIGHT_HEADER_SIZE>() {
            hheader = MapHeightHeader::parse(&b);
            have_terrain = hheader.flags & height_flags::NO_HEIGHT == 0;
            have_liquid = fheader.liquid_map_offset != 0 && !self.skip_liquid;
        }

        // no data in this map file
        if !have_terrain && !have_liquid {
            return false;
        }

        // data used later
        let mut holes = [0u8; 16 * 16 * 8];
        let mut liquid_entry = [0u16; 16 * 16];
        let mut liquid_flags = [0u8; 16 * 16];
        let mut ltriangles: Vec<i32> = Vec::new();
        let mut ttriangles: Vec<i32> = Vec::new();

        // terrain data
        if have_terrain {
            let mut v9 = vec![0f32; V9_SIZE_SQ as usize];
            let mut v8 = vec![0f32; V8_SIZE_SQ as usize];
            let expected = (V9_SIZE_SQ + V8_SIZE_SQ) as usize;

            if hheader.flags & height_flags::HEIGHT_AS_INT8 != 0 {
                let mut b9 = vec![0u8; V9_SIZE_SQ as usize];
                let mut b8 = vec![0u8; V8_SIZE_SQ as usize];
                let mut count = map_file.read(&mut b9, 1);
                count += map_file.read(&mut b8, 1);
                if count != expected {
                    println!(
                        "TerrainBuilder::loadMap: Failed to read some data expected {expected}, read {count}"
                    );
                }
                let height_multiplier = (hheader.grid_max_height - hheader.grid_height) / 255.0;
                for (d, s) in v9.iter_mut().zip(&b9) {
                    *d = f32::from(*s) * height_multiplier + hheader.grid_height;
                }
                for (d, s) in v8.iter_mut().zip(&b8) {
                    *d = f32::from(*s) * height_multiplier + hheader.grid_height;
                }
            } else if hheader.flags & height_flags::HEIGHT_AS_INT16 != 0 {
                let mut s9 = vec![0u16; V9_SIZE_SQ as usize];
                let mut s8 = vec![0u16; V8_SIZE_SQ as usize];
                let mut count = read_u16s(&mut map_file, &mut s9);
                count += read_u16s(&mut map_file, &mut s8);
                if count != expected {
                    println!(
                        "TerrainBuilder::loadMap: Failed to read some data expected {expected}, read {count}"
                    );
                }
                let height_multiplier = (hheader.grid_max_height - hheader.grid_height) / 65535.0;
                for (d, s) in v9.iter_mut().zip(&s9) {
                    *d = f32::from(*s) * height_multiplier + hheader.grid_height;
                }
                for (d, s) in v8.iter_mut().zip(&s8) {
                    *d = f32::from(*s) * height_multiplier + hheader.grid_height;
                }
            } else {
                let mut count = read_f32s(&mut map_file, &mut v9);
                count += read_f32s(&mut map_file, &mut v8);
                if count != expected {
                    println!(
                        "TerrainBuilder::loadMap: Failed to read some data expected {expected}, read {count}"
                    );
                }
            }

            // hole data
            if fheader.holes_size != 0 {
                map_file.seek(fheader.holes_offset);
                let mut raw = vec![0u8; fheader.holes_size as usize];
                if map_file.read(&mut raw, fheader.holes_size as usize) != 1 {
                    println!(
                        "TerrainBuilder::loadMap: Failed to read some data expected 1, read 0"
                    );
                }
                let n = raw.len().min(holes.len());
                holes[..n].copy_from_slice(&raw[..n]);
            }

            let count = (mesh.solid_verts.len() / 3) as i32;
            let xoffset = (tile_x as f32 - 32.0) * GRID_SIZE;
            let yoffset = (tile_y as f32 - 32.0) * GRID_SIZE;

            for i in 0..V9_SIZE_SQ {
                let coord = Self::get_height_coord(i, Grid::V9, xoffset, yoffset, &v9);
                mesh.solid_verts
                    .extend_from_slice(&[coord[0], coord[2], coord[1]]);
            }
            for i in 0..V8_SIZE_SQ {
                let coord = Self::get_height_coord(i, Grid::V8, xoffset, yoffset, &v8);
                mesh.solid_verts
                    .extend_from_slice(&[coord[0], coord[2], coord[1]]);
            }

            let (loop_start, loop_end, loop_inc) = Self::get_loop_vars(portion);
            let mut i = loop_start;
            while i < loop_end {
                for j in Spot::Top as i32..=Spot::Bottom as i32 {
                    let indices = Self::get_height_triangle(i, Spot::from_i32(j), false);
                    ttriangles.push(indices[2] + count);
                    ttriangles.push(indices[1] + count);
                    ttriangles.push(indices[0] + count);
                }
                i += loop_inc;
            }
        }

        // liquid data
        if have_liquid {
            map_file.seek(fheader.liquid_map_offset);
            let lheader = if let Some(b) = map_file.read_array::<MAP_LIQUID_HEADER_SIZE>() {
                MapLiquidHeader::parse(&b)
            } else {
                println!("TerrainBuilder::loadMap: Failed to read some data expected 1, read 0");
                MapLiquidHeader::default()
            };

            if lheader.flags & liquid_header_flags::NO_TYPE == 0 {
                let mut raw = [0u8; 512];
                if map_file.read(&mut raw, 512) != 1 {
                    println!(
                        "TerrainBuilder::loadMap: Failed to read some data expected 1, read 0"
                    );
                }
                for (d, c) in liquid_entry.iter_mut().zip(raw.chunks_exact(2)) {
                    *d = u16::from_le_bytes([c[0], c[1]]);
                }
                if map_file.read(&mut liquid_flags, 256) != 1 {
                    println!(
                        "TerrainBuilder::loadMap: Failed to read some data expected 1, read 0"
                    );
                }
            } else {
                liquid_entry.fill(lheader.liquid_type);
                liquid_flags.fill(lheader.liquid_flags);
            }
            let _ = liquid_entry;

            let mut liquid_map: Vec<f32> = Vec::new();
            if lheader.flags & liquid_header_flags::NO_HEIGHT == 0 {
                let to_read = usize::from(lheader.width) * usize::from(lheader.height);
                liquid_map = vec![0f32; to_read];
                if read_f32s(&mut map_file, &mut liquid_map) != to_read {
                    println!(
                        "TerrainBuilder::loadMap: Failed to read some data expected 1, read 0"
                    );
                    // C++ drops the buffer and later dereferences a null
                    // height map (crash); we keep zero heights instead.
                    liquid_map.fill(0.0);
                }
            }

            let count = (mesh.liquid_verts.len() / 3) as i32;
            let xoffset = (tile_x as f32 - 32.0) * GRID_SIZE;
            let yoffset = (tile_y as f32 - 32.0) * GRID_SIZE;

            // generate coordinates
            if lheader.flags & liquid_header_flags::NO_HEIGHT == 0 {
                let mut j = 0usize;
                for i in 0..V9_SIZE_SQ {
                    let row = i / V9_SIZE;
                    let col = i % V9_SIZE;
                    let (ox, oy) = (i32::from(lheader.offset_x), i32::from(lheader.offset_y));
                    if row < oy
                        || row >= oy + i32::from(lheader.height)
                        || col < ox
                        || col >= ox + i32::from(lheader.width)
                    {
                        // dummy vert using invalid height
                        mesh.liquid_verts.extend_from_slice(&[
                            (xoffset + col as f32 * GRID_PART_SIZE) * -1.0,
                            INVALID_MAP_LIQ_HEIGHT,
                            (yoffset + row as f32 * GRID_PART_SIZE) * -1.0,
                        ]);
                        continue;
                    }
                    let coord = Self::get_liquid_coord(i, j, xoffset, yoffset, &liquid_map);
                    mesh.liquid_verts
                        .extend_from_slice(&[coord[0], coord[2], coord[1]]);
                    j += 1;
                }
            } else {
                for i in 0..V9_SIZE_SQ {
                    let row = i / V9_SIZE;
                    let col = i % V9_SIZE;
                    mesh.liquid_verts.extend_from_slice(&[
                        (xoffset + col as f32 * GRID_PART_SIZE) * -1.0,
                        lheader.liquid_level,
                        (yoffset + row as f32 * GRID_PART_SIZE) * -1.0,
                    ]);
                }
            }

            let tri_inc = Spot::Bottom as i32 - Spot::Top as i32;
            let (loop_start, loop_end, loop_inc) = Self::get_loop_vars(portion);

            // generate triangles
            let mut i = loop_start;
            while i < loop_end {
                let mut j = Spot::Top as i32;
                while j <= Spot::Bottom as i32 {
                    let indices = Self::get_height_triangle(i, Spot::from_i32(j), true);
                    ltriangles.push(indices[2] + count);
                    ltriangles.push(indices[1] + count);
                    ltriangles.push(indices[0] + count);
                    j += tri_inc;
                }
                i += loop_inc;
            }
        }

        // now that we have gathered the data, we can figure out which parts to keep:
        // liquid above ground, ground above liquid
        let t_tri_count = 4usize;

        if ltriangles.len() + ttriangles.len() == 0 {
            return false;
        }

        // make a copy of liquid vertices
        // used to pad right-bottom frame due to lost vertex data at extraction
        let lverts_copy = mesh.liquid_verts.clone();

        let mut lt = 0usize; // ltris offset
        let mut tt = 0usize; // ttris offset
        let (loop_start, loop_end, loop_inc) = Self::get_loop_vars(portion);
        let mut i = loop_start;
        while i < loop_end {
            for _j in 0..2 {
                // default is true, will change to false if needed
                let mut use_terrain = true;
                let mut use_liquid = true;
                let mut nav_liquid_type = nav_area::EMPTY;

                // if there is no liquid, don't use liquid
                if mesh.liquid_verts.is_empty() || ltriangles.is_empty() {
                    use_liquid = false;
                } else {
                    let liquid_type = Self::get_liquid_type(i, &liquid_flags);
                    if liquid_type & liquid_type_flags::DARK_WATER != 0 {
                        // players should not be here, so logically neither should creatures
                        use_terrain = false;
                        use_liquid = false;
                    } else if liquid_type & (liquid_type_flags::WATER | liquid_type_flags::OCEAN)
                        != 0
                    {
                        nav_liquid_type = nav_area::WATER;
                    } else if liquid_type & (liquid_type_flags::MAGMA | liquid_type_flags::SLIME)
                        != 0
                    {
                        nav_liquid_type = nav_area::MAGMA_SLIME;
                    } else {
                        use_liquid = false;
                    }
                }

                // if there is no terrain, don't use terrain
                if ttriangles.is_empty() {
                    use_terrain = false;
                }

                // while extracting ADT data we are losing right-bottom vertices
                // this code adds fair approximation of lost data
                if use_liquid {
                    let ltris = &ltriangles[lt..lt + 3];
                    let mut quad_height = 0f32;
                    let mut valid_count = 0u32;
                    for &v in ltris {
                        let h = lverts_copy[v as usize * 3 + 1];
                        if h != INVALID_MAP_LIQ_HEIGHT && h < INVALID_MAP_LIQ_HEIGHT_MAX {
                            quad_height += h;
                            valid_count += 1;
                        }
                    }

                    // update vertex height data
                    if valid_count > 0 && valid_count < 3 {
                        quad_height /= valid_count as f32;
                        for &v in ltris {
                            let h = &mut mesh.liquid_verts[v as usize * 3 + 1];
                            if *h == INVALID_MAP_LIQ_HEIGHT || *h > INVALID_MAP_LIQ_HEIGHT_MAX {
                                *h = quad_height;
                            }
                        }
                    }

                    // no valid vertexes - don't use this poly at all
                    if valid_count == 0 {
                        use_liquid = false;
                    }
                }

                // if there is a hole here, don't use the terrain
                if use_terrain && fheader.holes_size != 0 {
                    use_terrain = !Self::is_hole(i, &holes);
                }

                // we use only one terrain kind per quad - pick higher one
                if use_terrain && use_liquid {
                    let mut min_l_level = INVALID_MAP_LIQ_HEIGHT_MAX;
                    let mut max_l_level = INVALID_MAP_LIQ_HEIGHT;
                    for &v in &ltriangles[lt..lt + 3] {
                        let h = mesh.liquid_verts[v as usize * 3 + 1];
                        if min_l_level > h {
                            min_l_level = h;
                        }
                        if max_l_level < h {
                            max_l_level = h;
                        }
                    }

                    let mut max_t_level = INVALID_MAP_LIQ_HEIGHT;
                    let mut min_t_level = INVALID_MAP_LIQ_HEIGHT_MAX;
                    for &v in &ttriangles[tt..tt + 6] {
                        let h = mesh.solid_verts[v as usize * 3 + 1];
                        if max_t_level < h {
                            max_t_level = h;
                        }
                        if min_t_level > h {
                            min_t_level = h;
                        }
                    }

                    // terrain under the liquid?
                    if min_l_level > max_t_level {
                        use_terrain = false;
                    }

                    //liquid under the terrain?
                    if min_t_level > max_l_level {
                        use_liquid = false;
                    }
                }

                // store the result
                if use_liquid {
                    mesh.liquid_type.push(nav_liquid_type);
                    mesh.liquid_tris.extend_from_slice(&ltriangles[lt..lt + 3]);
                }

                if use_terrain {
                    let n = 3 * t_tri_count / 2;
                    mesh.solid_tris.extend_from_slice(&ttriangles[tt..tt + n]);
                }

                // advance to next set of triangles
                lt += 3;
                tt += 3 * t_tri_count / 2;
            }
            i += loop_inc;
        }

        !mesh.solid_tris.is_empty() || !mesh.liquid_tris.is_empty()
    }

    /// `TerrainBuilder::getHeightCoord` — returns `coord[3]` (x, y, height).
    fn get_height_coord(
        index: i32,
        grid: Grid,
        x_offset: f32,
        y_offset: f32,
        v: &[f32],
    ) -> [f32; 3] {
        // wow coords: x, y, height
        // coord is mirroed about the horizontal axes
        match grid {
            Grid::V9 => [
                (x_offset + (index % V9_SIZE) as f32 * GRID_PART_SIZE) * -1.0,
                (y_offset + (index / V9_SIZE) as f32 * GRID_PART_SIZE) * -1.0,
                v[index as usize],
            ],
            Grid::V8 => [
                (x_offset + (index % V8_SIZE) as f32 * GRID_PART_SIZE + GRID_PART_SIZE / 2.0)
                    * -1.0,
                (y_offset + (index / V8_SIZE) as f32 * GRID_PART_SIZE + GRID_PART_SIZE / 2.0)
                    * -1.0,
                v[index as usize],
            ],
        }
    }

    /// `TerrainBuilder::getHeightTriangle`.
    fn get_height_triangle(square: i32, triangle: Spot, liquid: bool) -> [i32; 3] {
        let row_offset = square / V8_SIZE;
        let mut indices = [0i32; 3];
        if liquid {
            match triangle {
                Spot::Top => {
                    indices[0] = square + row_offset;
                    indices[1] = square + 1 + row_offset;
                    indices[2] = square + V9_SIZE + 1 + row_offset;
                }
                Spot::Bottom => {
                    indices[0] = square + row_offset;
                    indices[1] = square + V9_SIZE + 1 + row_offset;
                    indices[2] = square + V9_SIZE + row_offset;
                }
                _ => {}
            }
        } else {
            match triangle {
                Spot::Top => {
                    indices[0] = square + row_offset;
                    indices[1] = square + 1 + row_offset;
                    indices[2] = V9_SIZE_SQ + square;
                }
                Spot::Left => {
                    indices[0] = square + row_offset;
                    indices[1] = V9_SIZE_SQ + square;
                    indices[2] = square + V9_SIZE + row_offset;
                }
                Spot::Right => {
                    indices[0] = square + 1 + row_offset;
                    indices[1] = square + V9_SIZE + 1 + row_offset;
                    indices[2] = V9_SIZE_SQ + square;
                }
                Spot::Bottom => {
                    indices[0] = V9_SIZE_SQ + square;
                    indices[1] = square + V9_SIZE + 1 + row_offset;
                    indices[2] = square + V9_SIZE + row_offset;
                }
                Spot::Entire => {}
            }
        }
        indices
    }

    /// `TerrainBuilder::getLiquidCoord`.
    fn get_liquid_coord(
        index: i32,
        index2: usize,
        x_offset: f32,
        y_offset: f32,
        v: &[f32],
    ) -> [f32; 3] {
        [
            (x_offset + (index % V9_SIZE) as f32 * GRID_PART_SIZE) * -1.0,
            (y_offset + (index / V9_SIZE) as f32 * GRID_PART_SIZE) * -1.0,
            v[index2],
        ]
    }

    /// `TerrainBuilder::isHole` (`holes[16][16][8]` flattened).
    fn is_hole(square: i32, holes: &[u8; 16 * 16 * 8]) -> bool {
        let row = square / 128;
        let col = square % 128;
        let cell_row = row / 8; // 8 squares per cell
        let cell_col = col / 8;
        let hole_row = row % 8;
        let hole_col = col % 8;
        (holes[(cell_row * 128 + cell_col * 8 + hole_row) as usize] & (1 << hole_col)) != 0
    }

    /// `TerrainBuilder::getLiquidType` (`liquid_flags[16][16]` flattened).
    fn get_liquid_type(square: i32, liquid_type: &[u8; 256]) -> u8 {
        let row = square / 128;
        let col = square % 128;
        let cell_row = row / 8;
        let cell_col = col / 8;
        liquid_type[(cell_row * 16 + cell_col) as usize]
    }

    /// `TerrainBuilder::loadVMap` — every loaded model instance of the tile,
    /// transformed into Recast space, plus WMO liquids.
    ///
    /// Aborts like `VMapManager2::loadMap`'s `ABORT_MSG` when `map_id` was not
    /// registered through `InitializeThreadUnsafe` (i.e. is not in Map.db2).
    pub fn load_vmap(&self, map_id: u32, tile_x: u32, tile_y: u32, mesh: &mut MeshData) -> bool {
        let mut vm = VMapManager::new();
        vm.initialize_thread_unsafe(&self.data.map_data_for_vmap);
        if !self.data.map_data_for_vmap.contains_key(&map_id) {
            eprintln!(
                "Invalid mapId {map_id} tile [{tile_x}, {tile_y}] passed to VMapManager2 after startup in thread unsafe environment"
            );
            std::process::abort();
        }
        let result = vm.load_map(self.base.join("vmaps"), map_id, tile_x, tile_y);
        let mut retval = false;

        if result == LoadResult::Success
            && let Some(tree) = vm.map_tree(map_id)
        {
            for instance in tree.model_instances() {
                // model instances exist in tree even though there are instances of that model in this tile
                let Some(world_model) = instance.world_model() else {
                    continue;
                };

                // now we have a model to add to the meshdata
                retval = true;

                // all M2s need to have triangle indices reversed
                let is_m2 = instance.flags & MOD_M2 != 0;

                // transform data
                let scale = instance.scale;
                let rotation = *instance.inv_rot();
                let mut position = instance.pos;
                position.x -= 32.0 * GRID_SIZE;
                position.y -= 32.0 * GRID_SIZE;

                for group in world_model.group_models() {
                    let (temp_vertices, temp_triangles, liquid) = group.mesh_data();

                    // first handle collision mesh
                    let transformed = Self::transform(temp_vertices, scale, &rotation, position);

                    let offset = (mesh.solid_verts.len() / 3) as i32;

                    Self::copy_vertices(&transformed, &mut mesh.solid_verts);
                    Self::copy_indices(temp_triangles, &mut mesh.solid_tris, offset, is_m2);

                    // now handle liquid data
                    let Some(liquid) = liquid else { continue };
                    let Some(flags) = liquid.flags_storage() else {
                        continue;
                    };
                    let (tiles_x, tiles_y, corner) = liquid.pos_info();
                    let verts_x = tiles_x + 1;
                    let verts_y = tiles_y + 1;
                    let data = liquid.height_storage();
                    let mut ty = nav_area::EMPTY;

                    // convert liquid type to NavTerrain
                    let liquid_flags = self.data.liquid_flags(liquid.liquid_type()) as u8;
                    if liquid_flags & (liquid_type_flags::WATER | liquid_type_flags::OCEAN) != 0 {
                        ty = nav_area::WATER;
                    } else if liquid_flags & (liquid_type_flags::MAGMA | liquid_type_flags::SLIME)
                        != 0
                    {
                        ty = nav_area::MAGMA_SLIME;
                    }

                    // indexing is weird...
                    // after a lot of trial and error, this is what works:
                    // vertex = y*vertsX+x
                    // tile   = x*tilesY+y
                    // flag   = y*tilesY+x
                    let mut liq_verts: Vec<Vector3> = Vec::new();
                    for x in 0..verts_x {
                        for y in 0..verts_y {
                            let mut vert = Vector3::new(
                                corner.x + x as f32 * GRID_PART_SIZE,
                                corner.y + y as f32 * GRID_PART_SIZE,
                                data[(y * verts_x + x) as usize],
                            );
                            vert = vert * rotation * scale + position;
                            vert.x *= -1.0;
                            vert.y *= -1.0;
                            liq_verts.push(vert);
                        }
                    }

                    let mut liq_tris: Vec<i32> = Vec::new();
                    for x in 0..tiles_x {
                        for y in 0..tiles_y {
                            if (flags[(x + y * tiles_x) as usize] & 0x0f) != 0x0f {
                                let square = x * tiles_y + y;
                                let idx1 = (square + x) as i32;
                                let idx2 = (square + 1 + x) as i32;
                                let idx3 = (square + tiles_y + 1 + 1 + x) as i32;
                                let idx4 = (square + tiles_y + 1 + x) as i32;

                                // top triangle
                                liq_tris.extend_from_slice(&[idx3, idx2, idx1]);
                                // bottom triangle
                                liq_tris.extend_from_slice(&[idx4, idx3, idx1]);
                            }
                        }
                    }

                    let liq_offset = (mesh.liquid_verts.len() / 3) as i32;
                    for v in &liq_verts {
                        mesh.liquid_verts.extend_from_slice(&[v.y, v.z, v.x]);
                    }

                    for t in liq_tris.chunks_exact(3) {
                        mesh.liquid_tris.extend_from_slice(&[
                            t[1].wrapping_add(liq_offset),
                            t[2].wrapping_add(liq_offset),
                            t[0].wrapping_add(liq_offset),
                        ]);
                        mesh.liquid_type.push(ty);
                    }
                }
            }
        }

        vm.unload_map_tile(map_id, tile_x, tile_y);

        retval
    }

    /// `TerrainBuilder::transform` — `v * rotation * scale + position`, then
    /// mirrored along the horizontal axes.
    pub fn transform(
        source: &[Vector3],
        scale: f32,
        rotation: &Matrix3,
        position: Vector3,
    ) -> Vec<Vector3> {
        source
            .iter()
            .map(|&it| {
                let mut v = it * *rotation * scale + position;
                v.x *= -1.0;
                v.y *= -1.0;
                v
            })
            .collect()
    }

    /// `TerrainBuilder::copyVertices` — (y, z, x) order.
    pub fn copy_vertices(source: &[Vector3], dest: &mut Vec<f32>) {
        for v in source {
            dest.extend_from_slice(&[v.y, v.z, v.x]);
        }
    }

    /// `TerrainBuilder::copyIndices(std::vector<MeshTriangle>&, ...)`.
    pub fn copy_indices(source: &[MeshTriangle], dest: &mut Vec<i32>, offset: i32, flip: bool) {
        let add = |i: u32| (i as i32).wrapping_add(offset);
        for t in source {
            if flip {
                dest.extend_from_slice(&[add(t.idx2), add(t.idx1), add(t.idx0)]);
            } else {
                dest.extend_from_slice(&[add(t.idx0), add(t.idx1), add(t.idx2)]);
            }
        }
    }

    /// `TerrainBuilder::copyIndices(G3D::Array<int>&, ...)`.
    pub fn copy_indices_offset(source: &[i32], dest: &mut Vec<i32>, offset: i32) {
        dest.extend(source.iter().map(|&i| i.wrapping_add(offset)));
    }

    /// `TerrainBuilder::cleanVertices` — drops unreferenced vertices, keeping
    /// first-reference order, and remaps the triangle indices.
    pub fn clean_vertices(verts: &mut Vec<f32>, tris: &mut [i32]) {
        let mut vert_map: HashMap<i32, i32> = HashMap::new();
        let mut clean_verts: Vec<f32> = Vec::new();
        let mut count = 0;
        // collect all the vertex indices from triangle
        for &t in tris.iter() {
            if vert_map.contains_key(&t) {
                continue;
            }
            vert_map.insert(t, count);
            let index = t as usize;
            clean_verts.extend_from_slice(&verts[index * 3..index * 3 + 3]);
            count += 1;
        }

        *verts = clean_verts;

        // update triangles to use new indices
        for t in tris.iter_mut() {
            if let Some(&n) = vert_map.get(t) {
                *t = n;
            }
        }
    }

    /// `TerrainBuilder::loadOffMeshConnections`.
    pub fn load_off_mesh_connections(
        map_id: u32,
        tile_x: u32,
        tile_y: u32,
        mesh: &mut MeshData,
        off_mesh_connections: &[OffMeshData],
    ) {
        for c in off_mesh_connections {
            if map_id != c.map_id || tile_x != c.tile_x || tile_y != c.tile_y {
                continue;
            }

            mesh.off_mesh_connections
                .extend_from_slice(&[c.from[1], c.from[2], c.from[0]]);
            mesh.off_mesh_connections
                .extend_from_slice(&[c.to[1], c.to[2], c.to[0]]);

            mesh.off_mesh_connection_dirs
                .push(u8::from(c.bidirectional));
            mesh.off_mesh_connection_rads.push(c.radius); // agent size equivalent
            // can be used same way as polygon flags
            mesh.off_mesh_connections_areas.push(c.area_id);
            mesh.off_mesh_connections_flags.push(c.flags);
        }
    }
}

#[cfg(test)]
#[allow(clippy::naive_bytecount)]
#[path = "terrain_builder_tests.rs"]
mod tests;
