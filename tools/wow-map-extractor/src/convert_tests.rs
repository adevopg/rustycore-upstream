//! Tests for `ConvertADT` on hand-built ADTs: MCNK/MCVT/MCLQ/MH2O/MFBO decoding, the
//! `.map` layout (offsets, sizes, flags, packing) and loading the result with
//! RustyCore's runtime reader (`wow_map::grid_map::GridMap`).

// Written bytes are compared bit-exactly on purpose; grid coordinates are tiny.
#![allow(clippy::float_cmp, clippy::cast_precision_loss)]

use std::collections::HashMap;

use super::*;
use crate::loadlib::test_util::{chunk, mver};
use crate::tables::LiquidTypeEntry;
use wow_map::grid_map::{GridMap, INVALID_HEIGHT};

// ── ADT builders ──────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Cell {
    area: u32,
    ypos: f32,
    flags: u32,
    holes: u32,
    high_res_holes: u64,
    mcvt: Option<Vec<f32>>,
    mclq: Option<Vec<u8>>,
}

impl Cell {
    fn flat(area: u32, ypos: f32) -> Self {
        Self {
            area,
            ypos,
            flags: 0,
            holes: 0,
            high_res_holes: 0,
            mcvt: None,
            mclq: None,
        }
    }
}

/// `adt_MCNK` chunk: 128 header bytes after the chunk header, then sub-chunks.
fn mcnk(ix: u32, iy: u32, cell: &Cell) -> Vec<u8> {
    let mut header = vec![0u8; 128];
    let mut put = |struct_offset: usize, bytes: &[u8]| {
        let at = struct_offset - 8;
        header[at..at + bytes.len()].copy_from_slice(bytes);
    };
    put(8, &cell.flags.to_le_bytes());
    put(12, &ix.to_le_bytes());
    put(16, &iy.to_le_bytes());
    put(28, &cell.high_res_holes.to_le_bytes());
    put(60, &cell.area.to_le_bytes());
    put(68, &cell.holes.to_le_bytes());
    let size_mclq = cell.mclq.as_ref().map_or(0, |m| m.len() as u32 + 8);
    put(108, &size_mclq.to_le_bytes());
    put(120, &cell.ypos.to_le_bytes());
    let mut payload = header;
    if let Some(mcvt) = &cell.mcvt {
        assert_eq!(mcvt.len(), 145);
        let bytes: Vec<u8> = mcvt.iter().flat_map(|h| h.to_le_bytes()).collect();
        payload.extend_from_slice(&chunk(b"MCVT", &bytes));
    }
    if let Some(mclq) = &cell.mclq {
        payload.extend_from_slice(&chunk(b"MCLQ", mclq));
    }
    payload.extend_from_slice(&[0; 8]); // never scanned (ptr < data + size)
    chunk(b"MCNK", &payload)
}

fn adt(cell: impl Fn(usize, usize) -> Cell, extra: &[Vec<u8>]) -> ChunkedFile {
    let mut bytes = mver();
    for iy in 0..16 {
        for ix in 0..16 {
            bytes.extend_from_slice(&mcnk(ix as u32, iy as u32, &cell(ix, iy)));
        }
    }
    for e in extra {
        bytes.extend_from_slice(e);
    }
    ChunkedFile::from_bytes(bytes).expect("valid ADT")
}

/// MCVT for a height plane `h(row, col)` over grid coordinates (V9 at integer points,
/// V8 at cell centres), relative to `ypos`.
fn plane_mcvt(ix: usize, iy: usize, ypos: f32, h: &impl Fn(f32, f32) -> f32) -> Vec<f32> {
    let mut v = Vec::with_capacity(145);
    for y in 0..=8 {
        for x in 0..=8 {
            v.push(h((iy * 8 + y) as f32, (ix * 8 + x) as f32) - ypos);
        }
        if y < 8 {
            for x in 0..8 {
                v.push(h((iy * 8 + y) as f32 + 0.5, (ix * 8 + x) as f32 + 0.5) - ypos);
            }
        }
    }
    v
}

fn tile(allow_float_to_int: bool) -> TileInfo<'static> {
    TileInfo {
        map_name: "Test",
        gx: 32,
        gy: 48,
        build: 54261,
        ignore_deep_water: false,
        allow_float_to_int,
    }
}

fn convert(adt: &ChunkedFile, tables: &LiquidTables, allow_float_to_int: bool) -> Vec<u8> {
    AdtConverter::default()
        .convert(adt, tables, &tile(allow_float_to_int))
        .expect("convert")
}

// ── .map decoding helpers (MapDefines.h layout) ────────────────────────────────

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// `map_fileheader` as (area off, size, height off, size, liquid off, size, holes off, size).
fn file_header(b: &[u8]) -> [u32; 8] {
    assert_eq!(&b[0..4], b"MAPS");
    assert_eq!(u32_at(b, 4), 10);
    assert_eq!(u32_at(b, 8), 54261);
    std::array::from_fn(|i| u32_at(b, 12 + i * 4))
}

/// Every section is contiguous and the file ends at the last one.
fn assert_sections_contiguous(b: &[u8]) {
    let h = file_header(b);
    assert_eq!(h[0], 44);
    assert_eq!(h[2], h[0] + h[1]);
    let mut end = h[2] + h[3];
    if h[4] != 0 {
        assert_eq!(h[4], end);
        end += h[5];
    } else {
        assert_eq!(h[5], 0);
    }
    if h[6] != 0 {
        assert_eq!(h[6], end);
        assert_eq!(h[7], 2048);
        end += h[7];
    } else {
        assert_eq!(h[7], 0);
    }
    assert_eq!(end as usize, b.len());
    assert_eq!(&b[h[0] as usize..h[0] as usize + 4], b"AREA");
    assert_eq!(&b[h[2] as usize..h[2] as usize + 4], b"MHGT");
    if h[4] != 0 {
        assert_eq!(&b[h[4] as usize..h[4] as usize + 4], b"MLIQ");
    }
}

/// World coordinates of local grid point (row, col) inside tile (32, 32) as
/// `GridMap::get_height` maps them (`gx = 128 * (32 - x / SIZE_OF_GRIDS)`).
fn world(row: f32, col: f32) -> (f32, f32) {
    let size = 533.333_3f32;
    let x = (32.0 - (128.0 + row) / 128.0) * size;
    let y = (32.0 - (128.0 + col) / 128.0) * size;
    (x, y)
}

fn sample_points() -> Vec<(f32, f32)> {
    vec![
        (3.3, 5.6),
        (64.25, 17.75),
        (100.9, 120.1),
        (12.5, 99.2),
        (127.4, 0.6),
    ]
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn flat_single_area_tile_is_minimal() {
    let adt = adt(|_, _| Cell::flat(12, 100.0), &[]);
    let b = convert(&adt, &LiquidTables::default(), true);
    assert_sections_contiguous(&b);
    let h = file_header(&b);
    assert_eq!(h, [44, 8, 52, 16, 0, 0, 0, 0]);
    assert_eq!(u16_at(&b, 48), AREA_NO_AREA);
    assert_eq!(u16_at(&b, 50), 12);
    assert_eq!(u32_at(&b, 56), HEIGHT_NO_HEIGHT);
    assert_eq!(f32_at(&b, 60), 100.0);
    assert_eq!(f32_at(&b, 64), 100.0);
    assert_eq!(b.len(), 68);

    let grid = GridMap::parse(&b).expect("RustyCore reader accepts the tile");
    let (x, y) = world(40.2, 77.7);
    assert_eq!(grid.get_height(x, y), 100.0);
}

#[test]
fn full_area_data_is_stored_row_major() {
    let adt = adt(|ix, iy| Cell::flat((iy * 16 + ix) as u32, 5.0), &[]);
    let b = convert(&adt, &LiquidTables::default(), true);
    assert_sections_contiguous(&b);
    let h = file_header(&b);
    assert_eq!(h[1], 8 + 512);
    assert_eq!(u16_at(&b, 48), 0);
    assert_eq!(u16_at(&b, 50), 0);
    for i in 0..256 {
        assert_eq!(u16_at(&b, 52 + i * 2), i as u16);
    }
}

fn plane_tile(h: impl Fn(f32, f32) -> f32) -> ChunkedFile {
    adt(
        |ix, iy| {
            let ypos = 10.0;
            Cell {
                mcvt: Some(plane_mcvt(ix, iy, ypos, &h)),
                ..Cell::flat(1, ypos)
            }
        },
        &[],
    )
}

fn check_plane(b: &[u8], h: &impl Fn(f32, f32) -> f32, tolerance: f32) {
    let grid = GridMap::parse(b).expect("RustyCore reader accepts the tile");
    for (row, col) in sample_points() {
        let (x, y) = world(row, col);
        let got = grid.get_height(x, y);
        let expected = h(row, col);
        assert!(
            (got - expected).abs() <= tolerance,
            "({row},{col}): got {got}, expected {expected}"
        );
    }
}

#[test]
fn float_heights_when_float_to_int_disabled() {
    let h = |r: f32, c: f32| 50.0 + 1.5 * r - 0.75 * c;
    let b = convert(&plane_tile(h), &LiquidTables::default(), false);
    assert_sections_contiguous(&b);
    let hdr = file_header(&b);
    assert_eq!(hdr[3], 16 + 129 * 129 * 4 + 128 * 128 * 4);
    let hh = hdr[2] as usize;
    assert_eq!(u32_at(&b, hh + 4), 0);
    assert_eq!(f32_at(&b, hh + 8), h(0.0, 128.0)); // min
    assert_eq!(f32_at(&b, hh + 12), h(128.0, 0.0)); // max
    // V9[0][0] then V8[0][0]
    assert_eq!(f32_at(&b, hh + 16), h(0.0, 0.0));
    assert_eq!(f32_at(&b, hh + 16 + 129 * 129 * 4), h(0.5, 0.5));
    check_plane(&b, &h, 1e-3);
}

#[test]
fn int16_heights() {
    let h = |r: f32, c: f32| -20.0 + 1.5 * r + 0.25 * c;
    let b = convert(&plane_tile(h), &LiquidTables::default(), true);
    assert_sections_contiguous(&b);
    let hdr = file_header(&b);
    assert_eq!(hdr[3], 16 + 129 * 129 * 2 + 128 * 128 * 2);
    let hh = hdr[2] as usize;
    assert_eq!(u32_at(&b, hh + 4), HEIGHT_AS_INT16);
    let (min, max) = (f32_at(&b, hh + 8), f32_at(&b, hh + 12));
    // uint16((V9 - min) * (65535 / diff) + 0.5)
    let step = 65535.0 / (max - min);
    assert_eq!(u16_at(&b, hh + 16), 0);
    let last_v9 = hh + 16 + (129 * 129 - 1) * 2;
    assert_eq!(u16_at(&b, last_v9), 65535);
    let v8_0 = u16_at(&b, hh + 16 + 129 * 129 * 2);
    assert_eq!(v8_0, ((h(0.5, 0.5) - min) * step + 0.5) as u16);
    check_plane(&b, &h, 0.01);
}

#[test]
fn int8_heights_and_height_limit() {
    let h = |r: f32, c: f32| -2500.0 + 0.005 * r + 0.004 * c;
    // Everything is below CONF_use_minHeight: clamped to -2000 -> flat.
    let b = convert(&plane_tile(h), &LiquidTables::default(), true);
    let hh = file_header(&b)[2] as usize;
    assert_eq!(u32_at(&b, hh + 4), HEIGHT_NO_HEIGHT);
    assert_eq!(f32_at(&b, hh + 8), -2000.0);
    assert_eq!(f32_at(&b, hh + 12), -2000.0);

    let h = |r: f32, c: f32| 3.0 + 0.005 * r + 0.004 * c;
    let b = convert(&plane_tile(h), &LiquidTables::default(), true);
    assert_sections_contiguous(&b);
    let hdr = file_header(&b);
    assert_eq!(hdr[3], 16 + 129 * 129 + 128 * 128);
    assert_eq!(u32_at(&b, hdr[2] as usize + 4), HEIGHT_AS_INT8);
    check_plane(&b, &h, 0.01);
}

#[test]
fn nearly_flat_surface_is_not_stored() {
    let h = |r: f32, _c: f32| 7.0 + 0.00001 * r;
    let b = convert(&plane_tile(h), &LiquidTables::default(), true);
    let hh = file_header(&b)[2] as usize;
    assert_eq!(u32_at(&b, hh + 4), HEIGHT_NO_HEIGHT);
    // ...unless float-to-int is disabled (only exact equality is flat then).
    let b = convert(&plane_tile(h), &LiquidTables::default(), false);
    let hh = file_header(&b)[2] as usize;
    assert_eq!(u32_at(&b, hh + 4), 0);
}

#[test]
fn flight_box_follows_heights() {
    let mut mfbo = Vec::new();
    for v in 1..=18i16 {
        mfbo.extend_from_slice(&v.to_le_bytes());
    }
    let adt = adt(|_, _| Cell::flat(1, 0.0), &[chunk(b"MFBO", &mfbo)]);
    let b = convert(&adt, &LiquidTables::default(), true);
    assert_sections_contiguous(&b);
    let hdr = file_header(&b);
    assert_eq!(hdr[3], 16 + 36);
    let hh = hdr[2] as usize;
    assert_eq!(
        u32_at(&b, hh + 4),
        HEIGHT_NO_HEIGHT | HEIGHT_HAS_FLIGHT_BOUNDS
    );
    assert_eq!(&b[hh + 16..hh + 52], &mfbo[..]);
    assert!(GridMap::parse(&b).is_some());
}

#[test]
fn transform_to_high_res_expands_quadrants() {
    let mut hi = [0u8; 8];
    assert!(transform_to_high_res(0x0001, &mut hi));
    assert_eq!(hi, [0b11, 0b11, 0, 0, 0, 0, 0, 0]);
    let mut hi = [0u8; 8];
    assert!(transform_to_high_res(0x8000, &mut hi));
    assert_eq!(hi, [0, 0, 0, 0, 0, 0, 0b1100_0000, 0b1100_0000]);
    let mut hi = [0u8; 8];
    assert!(!transform_to_high_res(0, &mut hi));
}

#[test]
fn holes_low_and_high_resolution() {
    let h = |r: f32, c: f32| 20.0 + 0.5 * r + 0.5 * c;
    let adt = adt(
        |ix, iy| {
            let mut cell = Cell {
                mcvt: Some(plane_mcvt(ix, iy, 0.0, &h)),
                ..Cell::flat(1, 0.0)
            };
            if (ix, iy) == (1, 0) {
                cell.holes = 0x0001; // low-res
            }
            if (ix, iy) == (2, 3) {
                cell.flags = 0x10000;
                cell.high_res_holes = 0x0000_0000_0000_8001; // row 0 bit 0 and row 1 bit 7
            }
            cell
        },
        &[],
    );
    let b = convert(&adt, &LiquidTables::default(), true);
    assert_sections_contiguous(&b);
    let hdr = file_header(&b);
    let holes = &b[hdr[6] as usize..];
    assert_eq!(holes.len(), 2048);
    let cell = |iy: usize, ix: usize| &holes[(iy * 16 + ix) * 8..(iy * 16 + ix) * 8 + 8];
    assert_eq!(cell(0, 1), &[0b11, 0b11, 0, 0, 0, 0, 0, 0]);
    assert_eq!(cell(3, 2), &[0x01, 0x80, 0, 0, 0, 0, 0, 0]);
    assert_eq!(holes.iter().filter(|&&v| v != 0).count(), 4);

    // GridMap: row index = cell row (iy) * 8 + hole row, column = ix * 8 + hole bit.
    let grid = GridMap::parse(&b).unwrap();
    let (x, y) = world(0.5, 8.5); // iy 0, ix 1, hole row 0 bit 0
    assert_eq!(grid.get_height(x, y), INVALID_HEIGHT);
    let (x, y) = world(25.5, 23.5); // iy 3 row 1, ix 2 bit 7
    assert_eq!(grid.get_height(x, y), INVALID_HEIGHT);
    let (x, y) = world(4.5, 12.5);
    assert!((grid.get_height(x, y) - h(4.5, 12.5)).abs() < 0.01);
}

/// `adt_MCLQ` payload: height1/2, 9x9 {light, height}, 8x8 flags, 84 bytes.
fn mclq(height: impl Fn(usize, usize) -> f32, flags: impl Fn(usize, usize) -> u8) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0f32.to_le_bytes());
    out.extend_from_slice(&0f32.to_le_bytes());
    for y in 0..9 {
        for x in 0..9 {
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&height(y, x).to_le_bytes());
        }
    }
    for y in 0..8 {
        for x in 0..8 {
            out.push(flags(y, x));
        }
    }
    out.extend_from_slice(&[0; 84]);
    out
}

/// Decoded `map_liquidHeader`.
#[derive(Debug, PartialEq)]
struct Liq {
    flags: u8,
    liquid_flags: u8,
    liquid_type: u16,
    offset_x: u8,
    offset_y: u8,
    width: u8,
    height: u8,
    level: f32,
}

fn liquid_header(b: &[u8]) -> (usize, Liq) {
    let at = file_header(b)[4] as usize;
    assert_ne!(at, 0);
    (
        at,
        Liq {
            flags: b[at + 4],
            liquid_flags: b[at + 5],
            liquid_type: u16_at(b, at + 6),
            offset_x: b[at + 8],
            offset_y: b[at + 9],
            width: b[at + 10],
            height: b[at + 11],
            level: f32_at(b, at + 12),
        },
    )
}

#[test]
fn mclq_liquid_in_one_cell() {
    let adt = adt(
        |ix, iy| {
            let mut cell = Cell::flat(1, 0.0);
            if (ix, iy) == (3, 2) {
                cell.flags = 1 << 2; // water
                // all shown except (0,0); (7,7) is dark water; heights vary
                cell.mclq = Some(mclq(
                    |y, x| 5.0 + (y * 9 + x) as f32 * 0.01,
                    |y, x| match (y, x) {
                        (0, 0) => 0x0F,
                        (7, 7) => 0x80,
                        _ => 0,
                    },
                ));
            }
            cell
        },
        &[],
    );
    let b = convert(&adt, &LiquidTables::default(), true);
    assert_sections_contiguous(&b);
    let (at, liq) = liquid_header(&b);
    assert_eq!(
        liq,
        Liq {
            flags: 0, // types differ between cells and heights are not flat
            liquid_flags: 0,
            liquid_type: 0,
            offset_x: 24,
            offset_y: 16,
            width: 9,
            height: 9,
            level: -2000.0, // non-shown cells count as CONF_use_minHeight
        }
    );
    // entries then flags (16x16 each), row-major [iy][ix]
    let entries = at + 16;
    let flags = entries + 512;
    assert_eq!(u16_at(&b, entries + (2 * 16 + 3) * 2), 1);
    assert_eq!(
        b[flags + 2 * 16 + 3],
        LIQUID_TYPE_FLAG_WATER | LIQUID_TYPE_FLAG_DARK_WATER
    );
    assert_eq!(b[flags], 0);
    // heights: 9x9 floats from [16][24]
    let heights = flags + 256;
    assert_eq!(file_header(&b)[5] as usize, 16 + 512 + 256 + 81 * 4);
    assert_eq!(f32_at(&b, heights), -2000.0); // not shown -> CONF_use_minHeight
    assert_eq!(f32_at(&b, heights + 4), 5.0 + 0.01); // (0,1)
    assert_eq!(f32_at(&b, heights + (7 * 9 + 7) * 4), 5.0 + 70.0 * 0.01); // (7,7)
    // (8,8) is row 24 / col 32: written by MCLQ but not shown, so reset by the packing loop
    assert_eq!(f32_at(&b, heights + (8 * 9 + 8) * 4), -2000.0);
}

/// (row i, column j, `adt_liquid_instance` bytes, `adt_liquid_attributes`, vertex data).
type Mh2oSpec = (usize, usize, [u8; 24], Option<[u64; 2]>, Vec<u8>);

fn mh2o(instances: &[Mh2oSpec]) -> Vec<u8> {
    // payload: 16x16 {OffsetInstances, used, OffsetAttributes}, then per instance
    // instance (24) + attributes (16) + vertex data.
    let mut headers = vec![0u8; 256 * 12];
    let mut data = Vec::new();
    for (i, j, instance, attributes, vertex) in instances {
        let base = 256 * 12 + data.len();
        let at = (i * 16 + j) * 12;
        headers[at..at + 4].copy_from_slice(&(base as u32).to_le_bytes());
        headers[at + 4..at + 8].copy_from_slice(&1u32.to_le_bytes());
        let mut inst = *instance;
        let attr_off = if attributes.is_some() { base + 24 } else { 0 };
        headers[at + 8..at + 12].copy_from_slice(&(attr_off as u32).to_le_bytes());
        let vertex_off = base + 24 + if attributes.is_some() { 16 } else { 0 };
        if !vertex.is_empty() {
            inst[20..24].copy_from_slice(&(vertex_off as u32).to_le_bytes());
        }
        data.extend_from_slice(&inst);
        if let Some([fishable, deep]) = attributes {
            data.extend_from_slice(&fishable.to_le_bytes());
            data.extend_from_slice(&deep.to_le_bytes());
        }
        data.extend_from_slice(vertex);
    }
    headers.extend_from_slice(&data);
    chunk(b"MH2O", &headers)
}

fn instance(liquid_type: u16, lvf: u16, off: (u8, u8), size: (u8, u8)) -> [u8; 24] {
    let mut b = [0u8; 24];
    b[0..2].copy_from_slice(&liquid_type.to_le_bytes());
    b[2..4].copy_from_slice(&lvf.to_le_bytes());
    b[12] = off.0;
    b[13] = off.1;
    b[14] = size.0;
    b[15] = size.1;
    b
}

fn tables() -> LiquidTables {
    LiquidTables {
        materials: HashMap::from([(3, 2i8), (4, -1i8)]),
        objects: HashMap::new(),
        types: HashMap::from([
            (
                2,
                LiquidTypeEntry {
                    sound_bank: 2,
                    material_id: 0,
                },
            ),
            (
                5,
                LiquidTypeEntry {
                    sound_bank: 1,
                    material_id: 0,
                },
            ),
            (
                7,
                LiquidTypeEntry {
                    sound_bank: 0,
                    material_id: 3,
                },
            ),
            (
                8,
                LiquidTypeEntry {
                    sound_bank: 9,
                    material_id: 4,
                },
            ),
        ]),
    }
}

#[test]
fn liquid_vertex_format_lookup() {
    let t = tables();
    let inst = |liquid_type, lvf| crate::adt::LiquidInstance {
        liquid_type,
        liquid_vertex_format: lvf,
        offset_x: 0,
        offset_y: 0,
        width: 0,
        height: 0,
        offset_exists_bitmap: 0,
        offset_vertex_data: 0,
    };
    assert_eq!(get_liquid_vertex_format(&inst(5, 1), &t), 1);
    assert_eq!(get_liquid_vertex_format(&inst(2, 42), &t), lvf::DEPTH);
    assert_eq!(get_liquid_vertex_format(&inst(7, 42), &t), 2); // material 3 -> LVF 2
    assert_eq!(get_liquid_vertex_format(&inst(8, 42), &t), u16::MAX); // LVF -1
    assert_eq!(get_liquid_vertex_format(&inst(99, 42), &t), u16::MAX); // unknown type
    let i = inst(9, 42);
    assert_eq!((i.get_width(), i.get_height(), i.get_offset_x()), (8, 8, 0));
}

#[test]
fn mh2o_ocean_with_heights_exists_mask_and_deep_flag() {
    // Cell (i=2, j=3): ocean (type 5), HeightDepth, offset (1,2), 3x2 quads, exists mask
    // with bits 0,1,5 set; heights (4x3 = 12 floats) then depths.
    let mut vertex = Vec::new();
    for k in 0..12 {
        vertex.extend_from_slice(&(30.0f32 + k as f32).to_le_bytes());
    }
    vertex.extend_from_slice(&[0; 12]);
    let mut inst = instance(5, 0, (1, 2), (3, 2));
    let mask: u64 = 0b10_0011;
    // exists bitmap stored right after the instance: use the attributes-less layout
    // and put the mask in the vertex blob's tail instead.
    let mask_off_in_vertex = vertex.len();
    vertex.extend_from_slice(&mask.to_le_bytes());
    let base = 256 * 12;
    let vertex_off = base + 24 + 16;
    inst[16..20].copy_from_slice(&((vertex_off + mask_off_in_vertex) as u32).to_le_bytes());

    let adt = adt(
        |_, _| Cell::flat(1, 0.0),
        &[mh2o(&[(2, 3, inst, Some([0, 1]), vertex)])],
    );
    let b = convert(&adt, &tables(), true);
    assert_sections_contiguous(&b);
    let (at, liq) = liquid_header(&b);
    // shown cells: (y=0,x=0),(0,1),(1,2) -> rows 16+2.., cols 24+1..
    assert_eq!(
        liq,
        Liq {
            flags: 0,
            liquid_flags: 0,
            liquid_type: 0,
            offset_x: 25,
            offset_y: 18,
            width: 4,
            height: 3,
            level: -2000.0, // every non-shown cell of the grid counts as CONF_use_minHeight
        }
    );
    let entries = at + 16;
    let flags = entries + 512;
    assert_eq!(u16_at(&b, entries + (2 * 16 + 3) * 2), 5);
    assert_eq!(
        b[flags + 2 * 16 + 3],
        LIQUID_TYPE_FLAG_OCEAN | LIQUID_TYPE_FLAG_DARK_WATER
    );
    let heights = flags + 256;
    // Row 18 (y=0): x=25..28 -> positions 0..3 of the instance vertex grid.
    assert_eq!(f32_at(&b, heights), 30.0);
    assert_eq!(f32_at(&b, heights + 4), 31.0);
    // cols 27 and 28 of row 18 are not shown (< 128) -> reset to CONF_use_minHeight
    assert_eq!(f32_at(&b, heights + 8), -2000.0);
    assert_eq!(f32_at(&b, heights + 12), -2000.0);
    // Row 19 (y=1): (19,25) and (19,26) not shown -> -2000, (19,27) shown -> pos 4+2
    assert_eq!(f32_at(&b, heights + 16), -2000.0);
    assert_eq!(f32_at(&b, heights + 16 + 8), 36.0);
}

#[test]
fn mh2o_deep_water_ignored_and_depth_format() {
    // Type 2 (magma, sound bank 2) with LVF >= 42: whole cell (8x8), Depth format -> 0 heights.
    let inst = instance(2, 42, (5, 5), (1, 1));
    let adt = adt(
        |_, _| Cell::flat(1, 0.0),
        &[mh2o(&[(0, 0, inst, None, vec![1; 81])])],
    );
    let mut converter = AdtConverter::default();
    let mut info = tile(true);
    info.ignore_deep_water = true;
    let b = converter.convert(&adt, &tables(), &info).unwrap();
    assert_sections_contiguous(&b);
    let (_, liq) = liquid_header(&b);
    assert_eq!(
        (liq.offset_x, liq.offset_y, liq.width, liq.height),
        (0, 0, 9, 9)
    );
    assert_eq!(liq.level, -2000.0);
    let at = file_header(&b)[4] as usize;
    assert_eq!(u16_at(&b, at + 16), 2);
    assert_eq!(b[at + 16 + 512], LIQUID_TYPE_FLAG_MAGMA);
}

#[test]
fn mh2o_unknown_liquid_type_is_fatal() {
    let inst = instance(77, 0, (0, 0), (1, 1));
    let adt = adt(
        |_, _| Cell::flat(1, 0.0),
        &[mh2o(&[(0, 0, inst, None, Vec::new())])],
    );
    let err = AdtConverter::default()
        .convert(&adt, &tables(), &tile(true))
        .expect_err("LiquidTypes.at throws");
    assert!(err.0.contains("std::out_of_range"), "{}", err.0);
}

#[test]
fn uniform_liquid_uses_no_type_and_flat_level() {
    // Every cell fully covered by water (MCLQ, flat height 12) -> NoType + NoHeight.
    let adt = adt(
        |_, _| Cell {
            flags: 1 << 2,
            mclq: Some(mclq(|_, _| 12.0, |_, _| 0)),
            ..Cell::flat(1, 0.0)
        },
        &[],
    );
    let b = convert(&adt, &LiquidTables::default(), true);
    assert_sections_contiguous(&b);
    let (_, liq) = liquid_header(&b);
    assert_eq!(
        liq,
        Liq {
            flags: LIQUID_NO_TYPE | LIQUID_NO_HEIGHT,
            liquid_flags: LIQUID_TYPE_FLAG_WATER,
            liquid_type: 1,
            offset_x: 0,
            offset_y: 0,
            width: 129,
            height: 129,
            level: 12.0,
        }
    );
    assert_eq!(file_header(&b)[5], 16);
}

#[test]
fn liquid_height_persists_between_tiles_like_the_cpp_global() {
    let water = |h: f32| Cell {
        flags: 1 << 2,
        mclq: Some(mclq(move |_, _| h, |_, _| 0)),
        ..Cell::flat(1, 0.0)
    };
    // Tile A: liquid in cell (ix 5, iy 15) -> writes liquid_height row 128, cols 40..=48.
    let a = adt(
        |ix, iy| {
            if (ix, iy) == (5, 15) {
                water(9.0)
            } else {
                Cell::flat(1, 0.0)
            }
        },
        &[],
    );
    // Tile B: liquid in cells (0, 15) and (7, 0): the liquid box spans rows 0..=128 and
    // cols 0..=64, but B itself writes row 128 only at cols 0..=8.
    let b_tile = adt(
        |ix, iy| match (ix, iy) {
            (0, 15) => water(4.0),
            (7, 0) => water(4.5),
            _ => Cell::flat(1, 0.0),
        },
        &[],
    );
    let row128_col40 = |b: &[u8]| {
        let (at, liq) = liquid_header(b);
        assert_eq!(
            (liq.offset_x, liq.offset_y, liq.width, liq.height),
            (0, 0, 65, 129)
        );
        let heights = at + 16 + 512 + 256;
        f32_at(b, heights + (128 * 65 + 40) * 4)
    };

    let fresh = AdtConverter::default()
        .convert(&b_tile, &LiquidTables::default(), &tile(true))
        .unwrap();
    assert_eq!(row128_col40(&fresh), 0.0);

    let mut reused = AdtConverter::default();
    reused
        .convert(&a, &LiquidTables::default(), &tile(true))
        .unwrap();
    let after_a = reused
        .convert(&b_tile, &LiquidTables::default(), &tile(true))
        .unwrap();
    assert_eq!(row128_col40(&after_a), 9.0);
}
