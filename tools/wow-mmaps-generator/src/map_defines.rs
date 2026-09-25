//! File format constants used by the generator:
//! `src/common/Collision/Maps/MapDefines.{h,cpp}` (`.map` v10 headers,
//! `map_liquidHeaderTypeFlags`) and `src/common/Collision/Maps/MMapDefines.h`
//! (`MmapTileHeader`, `NavArea`, `NavTerrainFlag`), TrinityCore TDB343.24081.

use crate::recast::DT_NAVMESH_VERSION;

/// `MapVersionMagic` (MapDefines.cpp).
pub const MAP_VERSION_MAGIC: u32 = 10;

/// `sizeof(map_fileheader)`.
pub const MAP_FILEHEADER_SIZE: usize = 44;
/// `sizeof(map_heightHeader)`.
pub const MAP_HEIGHT_HEADER_SIZE: usize = 16;
/// `sizeof(map_liquidHeader)`.
pub const MAP_LIQUID_HEADER_SIZE: usize = 16;

/// `map_heightHeaderFlags`.
pub mod height_flags {
    pub const NO_HEIGHT: u32 = 0x0001;
    pub const HEIGHT_AS_INT16: u32 = 0x0002;
    pub const HEIGHT_AS_INT8: u32 = 0x0004;
}

/// `map_liquidHeaderFlags`.
pub mod liquid_header_flags {
    pub const NO_TYPE: u8 = 0x01;
    pub const NO_HEIGHT: u8 = 0x02;
}

/// `map_liquidHeaderTypeFlags`.
#[allow(dead_code)]
pub mod liquid_type_flags {
    pub const NO_WATER: u8 = 0x00;
    pub const WATER: u8 = 0x01;
    pub const OCEAN: u8 = 0x02;
    pub const MAGMA: u8 = 0x04;
    pub const SLIME: u8 = 0x08;
    pub const DARK_WATER: u8 = 0x10;
}

/// `map_fileheader` (little endian, 44 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MapFileHeader {
    pub map_magic: [u8; 4],
    pub version_magic: u32,
    pub build_magic: u32,
    pub area_map_offset: u32,
    pub area_map_size: u32,
    pub height_map_offset: u32,
    pub height_map_size: u32,
    pub liquid_map_offset: u32,
    pub liquid_map_size: u32,
    pub holes_offset: u32,
    pub holes_size: u32,
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_bits(u32_at(b, o))
}

impl MapFileHeader {
    pub fn parse(b: &[u8; MAP_FILEHEADER_SIZE]) -> Self {
        Self {
            map_magic: [b[0], b[1], b[2], b[3]],
            version_magic: u32_at(b, 4),
            build_magic: u32_at(b, 8),
            area_map_offset: u32_at(b, 12),
            area_map_size: u32_at(b, 16),
            height_map_offset: u32_at(b, 20),
            height_map_size: u32_at(b, 24),
            liquid_map_offset: u32_at(b, 28),
            liquid_map_size: u32_at(b, 32),
            holes_offset: u32_at(b, 36),
            holes_size: u32_at(b, 40),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn to_bytes(self) -> [u8; MAP_FILEHEADER_SIZE] {
        let mut out = [0u8; MAP_FILEHEADER_SIZE];
        out[0..4].copy_from_slice(&self.map_magic);
        let words = [
            self.version_magic,
            self.build_magic,
            self.area_map_offset,
            self.area_map_size,
            self.height_map_offset,
            self.height_map_size,
            self.liquid_map_offset,
            self.liquid_map_size,
            self.holes_offset,
            self.holes_size,
        ];
        for (i, w) in words.into_iter().enumerate() {
            out[4 + i * 4..8 + i * 4].copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

/// `map_heightHeader` (16 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MapHeightHeader {
    pub height_magic: [u8; 4],
    pub flags: u32,
    pub grid_height: f32,
    pub grid_max_height: f32,
}

impl MapHeightHeader {
    pub fn parse(b: &[u8; MAP_HEIGHT_HEADER_SIZE]) -> Self {
        Self {
            height_magic: [b[0], b[1], b[2], b[3]],
            flags: u32_at(b, 4),
            grid_height: f32_at(b, 8),
            grid_max_height: f32_at(b, 12),
        }
    }
}

/// `map_liquidHeader` (16 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MapLiquidHeader {
    pub liquid_magic: [u8; 4],
    pub flags: u8,
    pub liquid_flags: u8,
    pub liquid_type: u16,
    pub offset_x: u8,
    pub offset_y: u8,
    pub width: u8,
    pub height: u8,
    pub liquid_level: f32,
}

impl MapLiquidHeader {
    pub fn parse(b: &[u8; MAP_LIQUID_HEADER_SIZE]) -> Self {
        Self {
            liquid_magic: [b[0], b[1], b[2], b[3]],
            flags: b[4],
            liquid_flags: b[5],
            liquid_type: u16::from_le_bytes([b[6], b[7]]),
            offset_x: b[8],
            offset_y: b[9],
            width: b[10],
            height: b[11],
            liquid_level: f32_at(b, 12),
        }
    }
}

/// `MMAP_MAGIC` ('MMAP').
pub const MMAP_MAGIC: u32 = 0x4d4d_4150;
/// `MMAP_VERSION`.
pub const MMAP_VERSION: u32 = 15;

/// `MmapTileHeader` (20 bytes, padding zeroed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MmapTileHeader {
    pub mmap_magic: u32,
    pub dt_version: u32,
    pub mmap_version: u32,
    pub size: u32,
    pub uses_liquids: bool,
}

impl Default for MmapTileHeader {
    /// `MmapTileHeader::MmapTileHeader()`.
    fn default() -> Self {
        Self {
            mmap_magic: MMAP_MAGIC,
            dt_version: DT_NAVMESH_VERSION,
            mmap_version: MMAP_VERSION,
            size: 0,
            uses_liquids: true,
        }
    }
}

impl MmapTileHeader {
    pub const SIZE: usize = 20;

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut out = [0u8; Self::SIZE];
        out[0..4].copy_from_slice(&self.mmap_magic.to_le_bytes());
        out[4..8].copy_from_slice(&self.dt_version.to_le_bytes());
        out[8..12].copy_from_slice(&self.mmap_version.to_le_bytes());
        out[12..16].copy_from_slice(&self.size.to_le_bytes());
        out[16] = u8::from(self.uses_liquids);
        out
    }

    pub fn parse(b: &[u8; Self::SIZE]) -> Self {
        Self {
            mmap_magic: u32_at(b, 0),
            dt_version: u32_at(b, 4),
            mmap_version: u32_at(b, 8),
            size: u32_at(b, 12),
            uses_liquids: b[16] != 0,
        }
    }
}

/// `NavArea`.
pub mod nav_area {
    pub const EMPTY: u8 = 0;
    pub const GROUND: u8 = 11;
    pub const GROUND_STEEP: u8 = 10;
    pub const WATER: u8 = 9;
    pub const MAGMA_SLIME: u8 = 8;
    pub const MAX_VALUE: u8 = GROUND;
    pub const MIN_VALUE: u8 = MAGMA_SLIME;
    pub const ALL_MASK: u8 = 0x3F;
}

/// `NavTerrainFlag`.
#[allow(dead_code)]
pub mod nav_flag {
    use super::nav_area;
    pub const EMPTY: u16 = 0x00;
    pub const GROUND: u16 = 1 << (nav_area::MAX_VALUE - nav_area::GROUND);
    pub const GROUND_STEEP: u16 = 1 << (nav_area::MAX_VALUE - nav_area::GROUND_STEEP);
    pub const WATER: u16 = 1 << (nav_area::MAX_VALUE - nav_area::WATER);
    pub const MAGMA_SLIME: u16 = 1 << (nav_area::MAX_VALUE - nav_area::MAGMA_SLIME);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_header_default_bytes() {
        let h = MmapTileHeader {
            size: 0x0102_0304,
            ..MmapTileHeader::default()
        };
        let b = h.to_bytes();
        assert_eq!(&b[0..4], &[0x50, 0x41, 0x4d, 0x4d]);
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 15);
        assert_eq!(&b[12..20], &[4, 3, 2, 1, 1, 0, 0, 0]);
        assert_eq!(MmapTileHeader::parse(&b), h);
    }

    #[test]
    fn nav_flags() {
        assert_eq!(nav_flag::GROUND, 1);
        assert_eq!(nav_flag::GROUND_STEEP, 2);
        assert_eq!(nav_flag::WATER, 4);
        assert_eq!(nav_flag::MAGMA_SLIME, 8);
        assert_eq!(nav_flag::EMPTY, 0);
    }
}
