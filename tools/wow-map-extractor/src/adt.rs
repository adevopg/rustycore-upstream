//! ADT/WDT chunk layouts: port of `src/tools/map_extractor/adt.h` and `wdt.h`
//! (`adt_MCNK`, `adt_MCVT`, `adt_MCLQ`, `adt_MH2O`, `adt_liquid_instance`,
//! `adt_liquid_attributes`, `adt_MFBO`, `wdt_MPHD`, `wdt_MAIN`, `wdt_MAID`).
//!
//! The C++ casts the chunk pointer (which points at the FourCC header) to a
//! `#pragma pack(1)` struct; here each field is read at the same byte offset from
//! the chunk header through [`FileData`].

use crate::loadlib::{FileChunk, FileData};

/// `ADT_CELLS_PER_GRID`.
pub(crate) const ADT_CELLS_PER_GRID: usize = 16;
/// `ADT_CELL_SIZE`.
pub(crate) const ADT_CELL_SIZE: usize = 8;
/// `ADT_GRID_SIZE`.
pub(crate) const ADT_GRID_SIZE: usize = ADT_CELLS_PER_GRID * ADT_CELL_SIZE;
/// `WDT_MAP_SIZE`.
pub(crate) const WDT_MAP_SIZE: usize = 64;

/// `enum LiquidType` (adt.h) — `LiquidType.db2` `SoundBank` values.
pub(crate) const LIQUID_TYPE_WATER: u8 = 0;
pub(crate) const LIQUID_TYPE_OCEAN: u8 = 1;
pub(crate) const LIQUID_TYPE_MAGMA: u8 = 2;
pub(crate) const LIQUID_TYPE_SLIME: u8 = 3;

/// `adt_MCNK` field accessors (offsets include the 8-byte chunk header).
#[derive(Clone, Copy)]
pub(crate) struct Mcnk<'a> {
    file: &'a FileData,
    at: usize,
}

impl<'a> Mcnk<'a> {
    pub(crate) fn new(file: &'a FileData, chunk: &FileChunk) -> Self {
        Self {
            file,
            at: chunk.offset,
        }
    }

    pub(crate) fn flags(self) -> u32 {
        self.file.u32_at(self.at + 8)
    }
    pub(crate) fn ix(self) -> u32 {
        self.file.u32_at(self.at + 12)
    }
    pub(crate) fn iy(self) -> u32 {
        self.file.u32_at(self.at + 16)
    }
    /// `union_5_3_0.HighResHoles` (8 bytes at +28).
    pub(crate) fn high_res_holes(self) -> [u8; 8] {
        self.file.u64_at(self.at + 28).to_le_bytes()
    }
    pub(crate) fn areaid(self) -> u32 {
        self.file.u32_at(self.at + 60)
    }
    pub(crate) fn holes(self) -> u32 {
        self.file.u32_at(self.at + 68)
    }
    pub(crate) fn size_mclq(self) -> u32 {
        self.file.u32_at(self.at + 108)
    }
    /// `ypos` — the third `position` component (terrain base height).
    pub(crate) fn ypos(self) -> f32 {
        self.file.f32_at(self.at + 120)
    }
}

/// `adt_MCVT::height_map[i]` (145 floats after the header).
pub(crate) fn mcvt_height(file: &FileData, chunk: &FileChunk, i: usize) -> f32 {
    file.f32_at(chunk.offset + 8 + i * 4)
}

/// `adt_MCLQ` field accessors.
#[derive(Clone, Copy)]
pub(crate) struct Mclq<'a> {
    file: &'a FileData,
    at: usize,
}

impl<'a> Mclq<'a> {
    pub(crate) fn new(file: &'a FileData, chunk: &FileChunk) -> Self {
        Self {
            file,
            at: chunk.offset,
        }
    }
    /// `liquid[y][x].height` (`liquid_data` is `{u32 light; float height}` at +16).
    pub(crate) fn height(self, y: usize, x: usize) -> f32 {
        self.file
            .f32_at(self.at + 16 + (y * (ADT_CELL_SIZE + 1) + x) * 8 + 4)
    }
    /// `flags[y][x]` (after the 9x9 `liquid_data` array, at +664).
    pub(crate) fn flags(self, y: usize, x: usize) -> u8 {
        self.file
            .u8_at(self.at + 16 + 81 * 8 + y * ADT_CELL_SIZE + x)
    }
}

/// `LiquidVertexFormatType` (a `uint16` enum; unknown values fall to `default:`).
pub(crate) mod lvf {
    pub(crate) const HEIGHT_DEPTH: u16 = 0;
    pub(crate) const HEIGHT_TEXTURE_COORD: u16 = 1;
    pub(crate) const DEPTH: u16 = 2;
    pub(crate) const HEIGHT_DEPTH_TEXTURE_COORD: u16 = 3;
    pub(crate) const UNK4: u16 = 4;
    pub(crate) const UNK5: u16 = 5;
}

/// `adt_liquid_instance` (24 bytes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LiquidInstance {
    pub(crate) liquid_type: u16,
    pub(crate) liquid_vertex_format: u16,
    pub(crate) offset_x: u8,
    pub(crate) offset_y: u8,
    pub(crate) width: u8,
    pub(crate) height: u8,
    pub(crate) offset_exists_bitmap: u32,
    pub(crate) offset_vertex_data: u32,
}

impl LiquidInstance {
    pub(crate) fn get_offset_x(&self) -> u8 {
        if self.liquid_vertex_format < 42 {
            self.offset_x
        } else {
            0
        }
    }
    pub(crate) fn get_offset_y(&self) -> u8 {
        if self.liquid_vertex_format < 42 {
            self.offset_y
        } else {
            0
        }
    }
    pub(crate) fn get_width(&self) -> u8 {
        if self.liquid_vertex_format < 42 {
            self.width
        } else {
            8
        }
    }
    pub(crate) fn get_height(&self) -> u8 {
        if self.liquid_vertex_format < 42 {
            self.height
        } else {
            8
        }
    }
}

/// `adt_liquid_attributes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LiquidAttributes {
    pub(crate) fishable: u64,
    pub(crate) deep: u64,
}

/// `adt_MH2O` accessors; `base` is `(uint8*)this + 8` (all MH2O offsets are relative to it).
#[derive(Clone, Copy)]
pub(crate) struct Mh2o<'a> {
    file: &'a FileData,
    base: usize,
}

impl<'a> Mh2o<'a> {
    pub(crate) fn new(file: &'a FileData, chunk: &FileChunk) -> Self {
        Self {
            file,
            base: chunk.offset + 8,
        }
    }

    /// `liquid[x][y]` = `{OffsetInstances, used, OffsetAttributes}`.
    fn header(self, x: usize, y: usize) -> (u32, u32, u32) {
        let at = self.base + (x * ADT_CELLS_PER_GRID + y) * 12;
        (
            self.file.u32_at(at),
            self.file.u32_at(at + 4),
            self.file.u32_at(at + 8),
        )
    }

    /// `adt_MH2O::GetLiquidInstance`.
    pub(crate) fn get_liquid_instance(self, x: usize, y: usize) -> Option<LiquidInstance> {
        let (offset_instances, used, _) = self.header(x, y);
        if used == 0 || offset_instances == 0 {
            return None;
        }
        let at = self.base + offset_instances as usize;
        Some(LiquidInstance {
            liquid_type: self.file.u16_at(at),
            liquid_vertex_format: self.file.u16_at(at + 2),
            offset_x: self.file.u8_at(at + 12),
            offset_y: self.file.u8_at(at + 13),
            width: self.file.u8_at(at + 14),
            height: self.file.u8_at(at + 15),
            offset_exists_bitmap: self.file.u32_at(at + 16),
            offset_vertex_data: self.file.u32_at(at + 20),
        })
    }

    /// `adt_MH2O::GetLiquidAttributes`.
    pub(crate) fn get_liquid_attributes(self, x: usize, y: usize) -> LiquidAttributes {
        let (_, used, offset_attributes) = self.header(x, y);
        if used != 0 {
            if offset_attributes != 0 {
                let at = self.base + offset_attributes as usize;
                return LiquidAttributes {
                    fishable: self.file.u64_at(at),
                    deep: self.file.u64_at(at + 8),
                };
            }
            return LiquidAttributes {
                fishable: u64::MAX,
                deep: u64::MAX,
            };
        }
        LiquidAttributes {
            fishable: 0,
            deep: 0,
        }
    }

    /// `adt_MH2O::GetLiquidType`; `vertex_format` is `GetLiquidVertexFormat(h)`.
    pub(crate) fn get_liquid_type(h: &LiquidInstance, vertex_format: u16) -> u16 {
        if vertex_format == lvf::DEPTH {
            return 2;
        }
        h.liquid_type
    }

    /// `adt_MH2O::GetLiquidHeight`; `vertex_format` is `GetLiquidVertexFormat(h)`.
    pub(crate) fn get_liquid_height(
        self,
        h: &LiquidInstance,
        vertex_format: u16,
        pos: usize,
    ) -> f32 {
        if h.offset_vertex_data == 0 {
            return 0.0;
        }
        let data = self.base + h.offset_vertex_data as usize;
        match vertex_format {
            lvf::HEIGHT_DEPTH | lvf::HEIGHT_TEXTURE_COORD | lvf::HEIGHT_DEPTH_TEXTURE_COORD => {
                self.file.f32_at(data + pos * 4)
            }
            lvf::UNK4 | lvf::UNK5 => self.file.f32_at(data + 4 + pos * 2 * 4),
            // Depth and unknown formats
            _ => 0.0,
        }
    }

    /// `adt_MH2O::GetLiquidExistsBitmap`.
    pub(crate) fn get_liquid_exists_bitmap(self, h: &LiquidInstance) -> u64 {
        if h.offset_exists_bitmap != 0 {
            self.file
                .u64_at(self.base + h.offset_exists_bitmap as usize)
        } else {
            u64::MAX
        }
    }
}

/// `adt_MFBO`: `max` then `min` planes of 9 `int16` each.
pub(crate) fn mfbo_planes(file: &FileData, chunk: &FileChunk) -> ([i16; 9], [i16; 9]) {
    let max = std::array::from_fn(|i| file.i16_at(chunk.offset + 8 + i * 2));
    let min = std::array::from_fn(|i| file.i16_at(chunk.offset + 8 + 18 + i * 2));
    (max, min)
}

/// `wdt_MPHD::flags`.
pub(crate) fn mphd_flags(file: &FileData, chunk: &FileChunk) -> u32 {
    file.u32_at(chunk.offset + 8)
}

/// `wdt_MAIN::adt_list[y][x].flag`.
pub(crate) fn main_flag(file: &FileData, chunk: &FileChunk, y: usize, x: usize) -> u32 {
    file.u32_at(chunk.offset + 8 + (y * WDT_MAP_SIZE + x) * 8)
}

/// `wdt_MAID::adt_files[y][x].rootADT`.
pub(crate) fn maid_root_adt(file: &FileData, chunk: &FileChunk, y: usize, x: usize) -> u32 {
    file.u32_at(chunk.offset + 8 + (y * WDT_MAP_SIZE + x) * 32)
}
