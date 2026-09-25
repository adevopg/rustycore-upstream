//! ADT → `.map` conversion: port of `ConvertADT(ChunkedFile&, ...)`,
//! `TransformToHighRes`, `adt_MH2O::GetLiquidVertexFormat`, `selectUInt8StepStore` and
//! `selectUInt16StepStore` from `src/tools/map_extractor/System.cpp`, writing the
//! `MapDefines.h` format (`map_fileheader` "MAPS" v10, `map_areaHeader` "AREA",
//! `map_heightHeader` "MHGT", `map_liquidHeader` "MLIQ", holes).
//!
//! The C++ keeps the working grids in globals. All of them are reset per tile except
//! `liquid_height`, which carries values over from earlier tiles (only cells that the
//! current tile does not write and that are not reset by the liquid packing loop — row
//! and column 128 — can leak into the output). [`AdtConverter`] keeps that array alive
//! across calls so a whole extraction run produces the same bytes as the C++.

use crate::adt::{
    ADT_CELL_SIZE, ADT_CELLS_PER_GRID, ADT_GRID_SIZE, LIQUID_TYPE_MAGMA, LIQUID_TYPE_OCEAN,
    LIQUID_TYPE_SLIME, LIQUID_TYPE_WATER, LiquidInstance, Mclq, Mcnk, Mh2o, lvf, mcvt_height,
    mfbo_planes,
};
use crate::loadlib::ChunkedFile;
use crate::tables::{CppFatal, LiquidTables};

/// `MapMagic`.
pub(crate) const MAP_MAGIC: [u8; 4] = *b"MAPS";
/// `MapVersionMagic`.
pub(crate) const MAP_VERSION_MAGIC: u32 = 10;
const MAP_AREA_MAGIC: [u8; 4] = *b"AREA";
const MAP_HEIGHT_MAGIC: [u8; 4] = *b"MHGT";
const MAP_LIQUID_MAGIC: [u8; 4] = *b"MLIQ";

/// `map_areaHeaderFlags`.
pub(crate) const AREA_NO_AREA: u16 = 0x0001;
/// `map_heightHeaderFlags`.
pub(crate) const HEIGHT_NO_HEIGHT: u32 = 0x0001;
pub(crate) const HEIGHT_AS_INT16: u32 = 0x0002;
pub(crate) const HEIGHT_AS_INT8: u32 = 0x0004;
pub(crate) const HEIGHT_HAS_FLIGHT_BOUNDS: u32 = 0x0008;
/// `map_liquidHeaderFlags`.
pub(crate) const LIQUID_NO_TYPE: u8 = 0x01;
pub(crate) const LIQUID_NO_HEIGHT: u8 = 0x02;
/// `map_liquidHeaderTypeFlags`.
pub(crate) const LIQUID_TYPE_FLAG_WATER: u8 = 0x01;
pub(crate) const LIQUID_TYPE_FLAG_OCEAN: u8 = 0x02;
pub(crate) const LIQUID_TYPE_FLAG_MAGMA: u8 = 0x04;
pub(crate) const LIQUID_TYPE_FLAG_SLIME: u8 = 0x08;
pub(crate) const LIQUID_TYPE_FLAG_DARK_WATER: u8 = 0x10;

/// `sizeof(map_fileheader)`.
pub(crate) const FILE_HEADER_SIZE: u32 = 44;
const AREA_HEADER_SIZE: u32 = 8;
const HEIGHT_HEADER_SIZE: u32 = 16;
const LIQUID_HEADER_SIZE: u32 = 16;

// CONF_* (System.cpp); CONF_allow_height_limit has no command line switch.
const CONF_ALLOW_HEIGHT_LIMIT: bool = true;
const CONF_USE_MIN_HEIGHT: f32 = -2000.0;
const CONF_FLOAT_TO_INT8_LIMIT: f32 = 2.0;
const CONF_FLOAT_TO_INT16_LIMIT: f32 = 2048.0;
const CONF_FLAT_HEIGHT_DELTA_LIMIT: f32 = 0.005;
const CONF_FLAT_LIQUID_DELTA_LIMIT: f32 = 0.001;

const V9_SIDE: usize = ADT_GRID_SIZE + 1;
const CELLS: usize = ADT_CELLS_PER_GRID;

/// `selectUInt8StepStore`.
fn select_uint8_step_store(max_diff: f32) -> f32 {
    255.0 / max_diff
}

/// `selectUInt16StepStore`.
fn select_uint16_step_store(max_diff: f32) -> f32 {
    65535.0 / max_diff
}

/// `TransformToHighRes`: expands the 4x4 low-resolution hole mask into 8x8 bits.
pub(crate) fn transform_to_high_res(low_res_holes: u16, hi_res_holes: &mut [u8; 8]) -> bool {
    for i in 0..8u8 {
        for j in 0..8u8 {
            let hole_idx_l = (i / 2) * 4 + (j / 2);
            if (low_res_holes >> hole_idx_l) & 1 == 1 {
                hi_res_holes[i as usize] |= 1 << j;
            }
        }
    }
    u64::from_le_bytes(*hi_res_holes) != 0
}

/// `adt_MH2O::GetLiquidVertexFormat` (the `LiquidVertexFormatType` as its `uint16`).
pub(crate) fn get_liquid_vertex_format(
    liquid_instance: &LiquidInstance,
    tables: &LiquidTables,
) -> u16 {
    if liquid_instance.liquid_vertex_format < 42 {
        return liquid_instance.liquid_vertex_format;
    }
    if liquid_instance.liquid_type == 2 {
        return lvf::DEPTH;
    }
    if let Some(liquid_type) = tables.types.get(&u32::from(liquid_instance.liquid_type))
        && let Some(&lvf) = tables.materials.get(&u32::from(liquid_type.material_id))
    {
        // static_cast<LiquidVertexFormatType>(int8)
        return i16::from(lvf) as u16;
    }
    // static_cast<LiquidVertexFormatType>(-1)
    u16::MAX
}

/// Per-tile inputs of `ConvertADT` other than the ADT itself.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TileInfo<'a> {
    pub(crate) map_name: &'a str,
    pub(crate) gx: u32,
    pub(crate) gy: u32,
    pub(crate) build: u32,
    pub(crate) ignore_deep_water: bool,
    /// `CONF_allow_float_to_int` (`-f`).
    pub(crate) allow_float_to_int: bool,
}

/// The `ConvertADT` working grids (C++ globals).
pub(crate) struct AdtConverter {
    area_ids: [[u16; CELLS]; CELLS],
    v8: Vec<f32>,
    v9: Vec<f32>,
    liquid_entry: [[u16; CELLS]; CELLS],
    liquid_flags: [[u8; CELLS]; CELLS],
    liquid_show: Vec<bool>,
    /// `liquid_height[129][129]` — never reset between tiles (see module docs).
    liquid_height: Vec<f32>,
    holes: [[[u8; 8]; CELLS]; CELLS],
    flight_box_max: [i16; 9],
    flight_box_min: [i16; 9],
}

impl Default for AdtConverter {
    fn default() -> Self {
        Self {
            area_ids: [[0; CELLS]; CELLS],
            v8: vec![0.0; ADT_GRID_SIZE * ADT_GRID_SIZE],
            v9: vec![0.0; V9_SIDE * V9_SIDE],
            liquid_entry: [[0; CELLS]; CELLS],
            liquid_flags: [[0; CELLS]; CELLS],
            liquid_show: vec![false; ADT_GRID_SIZE * ADT_GRID_SIZE],
            liquid_height: vec![0.0; V9_SIDE * V9_SIDE],
            holes: [[[0; 8]; CELLS]; CELLS],
            flight_box_max: [0; 9],
            flight_box_min: [0; 9],
        }
    }
}

/// Sets `liquid_show[cy][cx]` when inside the 128x128 grid (C++ would write out of
/// bounds for malformed MH2O offsets; such writes are dropped here).
fn show(liquid_show: &mut [bool], cy: usize, cx: usize) {
    if cy < ADT_GRID_SIZE && cx < ADT_GRID_SIZE {
        liquid_show[cy * ADT_GRID_SIZE + cx] = true;
    }
}

fn set_liquid_height(liquid_height: &mut [f32], cy: usize, cx: usize, value: f32) {
    if cy < V9_SIDE && cx < V9_SIDE {
        liquid_height[cy * V9_SIDE + cx] = value;
    }
}

struct Out(Vec<u8>);

impl Out {
    fn bytes(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }
    fn i16(&mut self, v: i16) {
        self.bytes(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }
}

impl AdtConverter {
    /// `ConvertADT(ChunkedFile& adt, mapName, outputPath, gx, gy, build, ignoreDeepWater)`
    /// up to (not including) opening the output file: returns the `.map` file bytes.
    ///
    /// Errors only where the C++ would terminate (`LiquidTypes.at()` throwing).
    #[allow(clippy::too_many_lines)]
    pub(crate) fn convert(
        &mut self,
        adt: &ChunkedFile,
        tables: &LiquidTables,
        tile: &TileInfo<'_>,
    ) -> Result<Vec<u8>, CppFatal> {
        let file = &adt.data;

        // Get area flags data
        self.area_ids = [[0; CELLS]; CELLS];
        self.v9.fill(0.0);
        self.v8.fill(0.0);
        self.liquid_show.fill(false);
        self.liquid_flags = [[0; CELLS]; CELLS];
        self.liquid_entry = [[0; CELLS]; CELLS];
        self.holes = [[[0; 8]; CELLS]; CELLS];

        let mut has_holes = false;
        let mut has_flight_box = false;

        for raw_chunk in adt.chunks_named("MCNK") {
            let mcnk = Mcnk::new(file, raw_chunk);
            let (ix, iy) = (mcnk.ix() as usize, mcnk.iy() as usize);
            if ix >= CELLS || iy >= CELLS {
                // Out-of-range cell index: undefined behaviour in C++; ignored here.
                continue;
            }

            // Area data
            self.area_ids[iy][ix] = mcnk.areaid() as u16;

            // Set map height as grid height
            let ypos = mcnk.ypos();
            for y in 0..=ADT_CELL_SIZE {
                // edge V9s are overlapping between cells
                let cy = iy * ADT_CELL_SIZE + y;
                for x in 0..=ADT_CELL_SIZE {
                    let cx = ix * ADT_CELL_SIZE + x;
                    self.v9[cy * V9_SIDE + cx] = ypos;
                }
            }
            for y in 0..ADT_CELL_SIZE {
                let cy = iy * ADT_CELL_SIZE + y;
                for x in 0..ADT_CELL_SIZE {
                    let cx = ix * ADT_CELL_SIZE + x;
                    self.v8[cy * ADT_GRID_SIZE + cx] = ypos;
                }
            }

            // Get custom height
            if let Some(chunk) = raw_chunk.get_sub_chunk("MCVT") {
                // get V9 height map
                for y in 0..=ADT_CELL_SIZE {
                    let cy = iy * ADT_CELL_SIZE + y;
                    for x in 0..=ADT_CELL_SIZE {
                        let cx = ix * ADT_CELL_SIZE + x;
                        self.v9[cy * V9_SIDE + cx] +=
                            mcvt_height(file, chunk, y * (ADT_CELL_SIZE * 2 + 1) + x);
                    }
                }
                // get V8 height map
                for y in 0..ADT_CELL_SIZE {
                    let cy = iy * ADT_CELL_SIZE + y;
                    for x in 0..ADT_CELL_SIZE {
                        let cx = ix * ADT_CELL_SIZE + x;
                        self.v8[cy * ADT_GRID_SIZE + cx] += mcvt_height(
                            file,
                            chunk,
                            y * (ADT_CELL_SIZE * 2 + 1) + ADT_CELL_SIZE + 1 + x,
                        );
                    }
                }
            }

            // Liquid data
            if mcnk.size_mclq() > 8
                && let Some(chunk) = raw_chunk.get_sub_chunk("MCLQ")
            {
                let liquid = Mclq::new(file, chunk);
                let mut count = 0;
                for y in 0..ADT_CELL_SIZE {
                    let cy = iy * ADT_CELL_SIZE + y;
                    for x in 0..ADT_CELL_SIZE {
                        let cx = ix * ADT_CELL_SIZE + x;
                        let flags = liquid.flags(y, x);
                        if flags != 0x0F {
                            self.liquid_show[cy * ADT_GRID_SIZE + cx] = true;
                            if !tile.ignore_deep_water && flags & (1 << 7) != 0 {
                                self.liquid_flags[iy][ix] |= LIQUID_TYPE_FLAG_DARK_WATER;
                            }
                            count += 1;
                        }
                    }
                }

                let c_flag = mcnk.flags();
                if c_flag & (1 << 2) != 0 {
                    self.liquid_entry[iy][ix] = 1;
                    self.liquid_flags[iy][ix] |= LIQUID_TYPE_FLAG_WATER; // water
                }
                if c_flag & (1 << 3) != 0 {
                    self.liquid_entry[iy][ix] = 2;
                    self.liquid_flags[iy][ix] |= LIQUID_TYPE_FLAG_OCEAN; // ocean
                }
                if c_flag & (1 << 4) != 0 {
                    self.liquid_entry[iy][ix] = 3;
                    self.liquid_flags[iy][ix] |= LIQUID_TYPE_FLAG_MAGMA; // magma/slime
                }

                if count == 0 && self.liquid_flags[iy][ix] != 0 {
                    eprint!("Wrong liquid detect in MCLQ chunk");
                }

                for y in 0..=ADT_CELL_SIZE {
                    let cy = iy * ADT_CELL_SIZE + y;
                    for x in 0..=ADT_CELL_SIZE {
                        let cx = ix * ADT_CELL_SIZE + x;
                        self.liquid_height[cy * V9_SIDE + cx] = liquid.height(y, x);
                    }
                }
            }

            // Hole data
            if mcnk.flags() & 0x10000 == 0 {
                let hole = mcnk.holes() as u16;
                if hole != 0 && transform_to_high_res(hole, &mut self.holes[iy][ix]) {
                    has_holes = true;
                }
            } else {
                self.holes[iy][ix] = mcnk.high_res_holes();
                if u64::from_le_bytes(self.holes[iy][ix]) != 0 {
                    has_holes = true;
                }
            }
        }

        // Get liquid map for grid (in WOTLK used MH2O chunk)
        if let Some(chunk) = adt.get_chunk("MH2O") {
            self.read_mh2o(Mh2o::new(file, chunk), tables, tile)?;
        }

        if let Some(chunk) = adt.get_chunk("MFBO") {
            (self.flight_box_max, self.flight_box_min) = mfbo_planes(file, chunk);
            has_flight_box = true;
        }

        Ok(self.write(tile, has_holes, has_flight_box))
    }

    /// The `MH2O` block of `ConvertADT`.
    fn read_mh2o(
        &mut self,
        h2o: Mh2o<'_>,
        tables: &LiquidTables,
        tile: &TileInfo<'_>,
    ) -> Result<(), CppFatal> {
        for i in 0..CELLS {
            for j in 0..CELLS {
                let Some(h) = h2o.get_liquid_instance(i, j) else {
                    continue;
                };

                let attrs = h2o.get_liquid_attributes(i, j);
                let vertex_format = get_liquid_vertex_format(&h, tables);

                let mut count = 0;
                let mut exists_mask = h2o.get_liquid_exists_bitmap(&h);
                for y in 0..usize::from(h.get_height()) {
                    let cy = i * ADT_CELL_SIZE + y + usize::from(h.get_offset_y());
                    for x in 0..usize::from(h.get_width()) {
                        let cx = j * ADT_CELL_SIZE + x + usize::from(h.get_offset_x());
                        if exists_mask & 1 != 0 {
                            show(&mut self.liquid_show, cy, cx);
                            count += 1;
                        }
                        exists_mask >>= 1;
                    }
                }

                self.liquid_entry[i][j] = Mh2o::get_liquid_type(&h, vertex_format);
                let Some(liquid_type) = tables.types.get(&u32::from(self.liquid_entry[i][j]))
                else {
                    // LiquidTypes.at() throws std::out_of_range -> std::terminate
                    return Err(CppFatal(format!(
                        "terminate called after throwing an instance of 'std::out_of_range'\n  what():  unordered_map::at (LiquidType {} not in LiquidType.db2, map {} [{},{}] chunk {},{})\n",
                        self.liquid_entry[i][j], tile.map_name, tile.gx, tile.gy, i, j
                    )));
                };
                match liquid_type.sound_bank {
                    LIQUID_TYPE_WATER => self.liquid_flags[i][j] |= LIQUID_TYPE_FLAG_WATER,
                    LIQUID_TYPE_OCEAN => {
                        self.liquid_flags[i][j] |= LIQUID_TYPE_FLAG_OCEAN;
                        if !tile.ignore_deep_water && attrs.deep != 0 {
                            self.liquid_flags[i][j] |= LIQUID_TYPE_FLAG_DARK_WATER;
                        }
                    }
                    LIQUID_TYPE_MAGMA => self.liquid_flags[i][j] |= LIQUID_TYPE_FLAG_MAGMA,
                    LIQUID_TYPE_SLIME => self.liquid_flags[i][j] |= LIQUID_TYPE_FLAG_SLIME,
                    _ => print!(
                        "\nCan't find Liquid type {} for map {} [{},{}]\nchunk {},{}\n",
                        h.liquid_type, tile.map_name, tile.gx, tile.gy, i, j
                    ),
                }

                if count == 0 && self.liquid_flags[i][j] != 0 {
                    print!("Wrong liquid detect in MH2O chunk");
                }

                let mut pos = 0;
                for y in 0..=usize::from(h.get_height()) {
                    let cy = i * ADT_CELL_SIZE + y + usize::from(h.get_offset_y());
                    for x in 0..=usize::from(h.get_width()) {
                        let cx = j * ADT_CELL_SIZE + x + usize::from(h.get_offset_x());
                        let height = h2o.get_liquid_height(&h, vertex_format, pos);
                        set_liquid_height(&mut self.liquid_height, cy, cx, height);
                        pos += 1;
                    }
                }
            }
        }
        Ok(())
    }

    /// Packing + `outFile.write` sequence of `ConvertADT`.
    #[allow(clippy::too_many_lines, clippy::float_cmp)]
    fn write(&mut self, tile: &TileInfo<'_>, has_holes: bool, has_flight_box: bool) -> Vec<u8> {
        //============================================
        // Try pack area data
        //============================================
        let area_id = self.area_ids[0][0];
        let full_area_data = self.area_ids.iter().flatten().any(|&a| a != area_id);

        let area_map_offset = FILE_HEADER_SIZE;
        let mut area_map_size = AREA_HEADER_SIZE;
        let mut area_flags = 0u16;
        let grid_area;
        if full_area_data {
            grid_area = 0;
            area_map_size += (CELLS * CELLS * 2) as u32;
        } else {
            area_flags |= AREA_NO_AREA;
            grid_area = area_id;
        }

        //============================================
        // Try pack height data
        //============================================
        let mut max_height = -20000.0f32;
        let mut min_height = 20000.0f32;
        for &h in self.v8.iter().chain(self.v9.iter()) {
            if max_height < h {
                max_height = h;
            }
            if min_height > h {
                min_height = h;
            }
        }

        // Check for allow limit minimum height (not store height in deep ochean - allow save some memory)
        if CONF_ALLOW_HEIGHT_LIMIT && min_height < CONF_USE_MIN_HEIGHT {
            for h in self.v8.iter_mut().chain(self.v9.iter_mut()) {
                if *h < CONF_USE_MIN_HEIGHT {
                    *h = CONF_USE_MIN_HEIGHT;
                }
            }
            if min_height < CONF_USE_MIN_HEIGHT {
                min_height = CONF_USE_MIN_HEIGHT;
            }
            if max_height < CONF_USE_MIN_HEIGHT {
                max_height = CONF_USE_MIN_HEIGHT;
            }
        }

        let height_map_offset = area_map_offset + area_map_size;
        let mut height_map_size = HEIGHT_HEADER_SIZE;

        let mut height_flags = 0u32;
        let grid_height = min_height;
        let grid_max_height = max_height;

        if max_height == min_height {
            height_flags |= HEIGHT_NO_HEIGHT;
        }

        // Not need store if flat surface
        if tile.allow_float_to_int && (max_height - min_height) < CONF_FLAT_HEIGHT_DELTA_LIMIT {
            height_flags |= HEIGHT_NO_HEIGHT;
        }

        if has_flight_box {
            height_flags |= HEIGHT_HAS_FLIGHT_BOUNDS;
            height_map_size += 18 + 18;
        }

        let v9_count = (V9_SIDE * V9_SIDE) as u32;
        let v8_count = (ADT_GRID_SIZE * ADT_GRID_SIZE) as u32;

        // Try store as packed in uint16 or uint8 values
        let mut step = 0.0f32;
        if height_flags & HEIGHT_NO_HEIGHT == 0 {
            // Try Store as uint values
            if tile.allow_float_to_int {
                let diff = max_height - min_height;
                if diff < CONF_FLOAT_TO_INT8_LIMIT {
                    // As uint8 (max accuracy = CONF_float_to_int8_limit/256)
                    height_flags |= HEIGHT_AS_INT8;
                    step = select_uint8_step_store(diff);
                } else if diff < CONF_FLOAT_TO_INT16_LIMIT {
                    // As uint16 (max accuracy = CONF_float_to_int16_limit/65536)
                    height_flags |= HEIGHT_AS_INT16;
                    step = select_uint16_step_store(diff);
                }
            }

            if height_flags & HEIGHT_AS_INT8 != 0 {
                height_map_size += v9_count + v8_count;
            } else if height_flags & HEIGHT_AS_INT16 != 0 {
                height_map_size += 2 * (v9_count + v8_count);
            } else {
                height_map_size += 4 * (v9_count + v8_count);
            }
        }

        //============================================
        // Pack liquid data
        //============================================
        let first_liquid_type = self.liquid_entry[0][0];
        let first_liquid_flag = self.liquid_flags[0][0];
        let full_type = (0..CELLS).any(|y| {
            (0..CELLS).any(|x| {
                self.liquid_entry[y][x] != first_liquid_type
                    || self.liquid_flags[y][x] != first_liquid_flag
            })
        });

        let mut liquid_map_offset = 0u32;
        let mut liquid_map_size = 0u32;
        let mut liquid_header = None;

        // no water data (if all grid have 0 liquid type)
        if first_liquid_flag != 0 || full_type {
            let (mut min_x, mut min_y) = (255i32, 255i32);
            let (mut max_x, mut max_y) = (0i32, 0i32);
            max_height = -20000.0;
            min_height = 20000.0;
            for (y, yi) in (0..ADT_GRID_SIZE).zip(0i32..) {
                for (x, xi) in (0..ADT_GRID_SIZE).zip(0i32..) {
                    if self.liquid_show[y * ADT_GRID_SIZE + x] {
                        if min_x > xi {
                            min_x = xi;
                        }
                        if max_x < xi {
                            max_x = xi;
                        }
                        if min_y > yi {
                            min_y = yi;
                        }
                        if max_y < yi {
                            max_y = yi;
                        }
                        let h = self.liquid_height[y * V9_SIDE + x];
                        if max_height < h {
                            max_height = h;
                        }
                        if min_height > h {
                            min_height = h;
                        }
                    } else {
                        self.liquid_height[y * V9_SIDE + x] = CONF_USE_MIN_HEIGHT;
                        if min_height > CONF_USE_MIN_HEIGHT {
                            min_height = CONF_USE_MIN_HEIGHT;
                        }
                    }
                }
            }
            liquid_map_offset = height_map_offset + height_map_size;
            liquid_map_size = LIQUID_HEADER_SIZE;
            let mut header = LiquidHeader {
                flags: 0,
                liquid_flags: 0,
                liquid_type: 0,
                offset_x: min_x as u8,
                offset_y: min_y as u8,
                width: (max_x - min_x + 1 + 1) as u8,
                height: (max_y - min_y + 1 + 1) as u8,
                liquid_level: min_height,
            };

            if max_height == min_height {
                header.flags |= LIQUID_NO_HEIGHT;
            }

            // Not need store if flat surface
            if tile.allow_float_to_int && (max_height - min_height) < CONF_FLAT_LIQUID_DELTA_LIMIT {
                header.flags |= LIQUID_NO_HEIGHT;
            }

            if !full_type {
                header.flags |= LIQUID_NO_TYPE;
            }

            if header.flags & LIQUID_NO_TYPE != 0 {
                header.liquid_flags = first_liquid_flag;
                header.liquid_type = first_liquid_type;
            } else {
                liquid_map_size += (CELLS * CELLS * 2 + CELLS * CELLS) as u32;
            }

            if header.flags & LIQUID_NO_HEIGHT == 0 {
                liquid_map_size += 4 * u32::from(header.width) * u32::from(header.height);
            }
            liquid_header = Some(header);
        }

        let (holes_offset, holes_size) = if has_holes {
            let offset = if liquid_map_offset != 0 {
                liquid_map_offset + liquid_map_size
            } else {
                height_map_offset + height_map_size
            };
            (offset, (CELLS * CELLS * 8) as u32)
        } else {
            (0, 0)
        };

        // Ok all data prepared - store it
        let mut out = Out(Vec::new());
        out.bytes(&MAP_MAGIC);
        out.u32(MAP_VERSION_MAGIC);
        out.u32(tile.build);
        out.u32(area_map_offset);
        out.u32(area_map_size);
        out.u32(height_map_offset);
        out.u32(height_map_size);
        out.u32(liquid_map_offset);
        out.u32(liquid_map_size);
        out.u32(holes_offset);
        out.u32(holes_size);

        // Store area data
        out.bytes(&MAP_AREA_MAGIC);
        out.u16(area_flags);
        out.u16(grid_area);
        if area_flags & AREA_NO_AREA == 0 {
            for &a in self.area_ids.iter().flatten() {
                out.u16(a);
            }
        }

        // Store height data
        out.bytes(&MAP_HEIGHT_MAGIC);
        out.u32(height_flags);
        out.f32(grid_height);
        out.f32(grid_max_height);
        if height_flags & HEIGHT_NO_HEIGHT == 0 {
            let heights = self.v9.iter().chain(self.v8.iter());
            if height_flags & HEIGHT_AS_INT16 != 0 {
                for &h in heights {
                    out.u16(((h - grid_height) * step + 0.5) as u16);
                }
            } else if height_flags & HEIGHT_AS_INT8 != 0 {
                for &h in heights {
                    out.u8(((h - grid_height) * step + 0.5) as u8);
                }
            } else {
                for &h in heights {
                    out.f32(h);
                }
            }
        }

        if height_flags & HEIGHT_HAS_FLIGHT_BOUNDS != 0 {
            for v in self.flight_box_max.iter().chain(self.flight_box_min.iter()) {
                out.i16(*v);
            }
        }

        // Store liquid data if need
        if let Some(header) = liquid_header {
            out.bytes(&MAP_LIQUID_MAGIC);
            out.u8(header.flags);
            out.u8(header.liquid_flags);
            out.u16(header.liquid_type);
            out.u8(header.offset_x);
            out.u8(header.offset_y);
            out.u8(header.width);
            out.u8(header.height);
            out.f32(header.liquid_level);
            if header.flags & LIQUID_NO_TYPE == 0 {
                for &e in self.liquid_entry.iter().flatten() {
                    out.u16(e);
                }
                for &f in self.liquid_flags.iter().flatten() {
                    out.u8(f);
                }
            }

            if header.flags & LIQUID_NO_HEIGHT == 0 {
                for y in 0..usize::from(header.height) {
                    // &liquid_height[y + offsetY][offsetX], `width` floats (flat indexing)
                    let start =
                        (y + usize::from(header.offset_y)) * V9_SIDE + usize::from(header.offset_x);
                    for k in 0..usize::from(header.width) {
                        out.f32(self.liquid_height.get(start + k).copied().unwrap_or(0.0));
                    }
                }
            }
        }

        // store hole data
        if has_holes {
            for cell in self.holes.iter().flatten() {
                out.bytes(cell);
            }
        }

        out.0
    }
}

/// `map_liquidHeader` minus the magic.
#[derive(Debug, Clone, Copy)]
struct LiquidHeader {
    flags: u8,
    liquid_flags: u8,
    liquid_type: u16,
    offset_x: u8,
    offset_y: u8,
    width: u8,
    height: u8,
    liquid_level: f32,
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod tests;
