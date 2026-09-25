//! Synthetic generator input shared by the tests: `.map` v10 tiles (float,
//! uint16 and uint8 heights, holes, typed/untyped liquids, a liquid-only
//! tile), vmaps written with `wow-vmap` (a WMO with a liquid group and a
//! rotated M2, a child map re-using the parent's tiles), text stand-ins for
//! Map.db2 / LiquidType.db2 (the format read by the C++ reference's DB2 stub)
//! and an off-mesh connection file.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use wow_vmap::bih::Bih;
use wow_vmap::io::Writer;
use wow_vmap::{
    AABox, GroupModel, MOD_HAS_BOUND, MOD_M2, MeshTriangle, ModelSpawn, VMAP_MAGIC, Vector3,
    WmoLiquid, WorldModel, map_file_name, tile_file_name,
};

use crate::path_common::GeneratorData;
use crate::terrain_builder::{GRID_SIZE, V8_SIZE, V9_SIZE};
use crate::{MapRecord, apply_map_record};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heights {
    None,
    Float,
    U16,
    U8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liquid {
    None,
    /// Per-cell entry/flags and a partial height grid.
    Typed,
    /// `NoType | NoHeight`: header flags + level only.
    Flat(u8),
}

/// Terrain height at global grid position (continuous across tiles).
pub fn terrain_height(gx: f32, gy: f32) -> f32 {
    let mut h = 30.0 * (gx * 0.045).sin() * (gy * 0.03).cos() + 8.0 * (gy * 0.21).sin();
    // steep ridges every 97 grid units
    if (gx as i32).rem_euclid(97) < 3 {
        h += 25.0;
    }
    h
}

fn heights_for(tile_x: u32, tile_y: u32) -> (Vec<f32>, Vec<f32>) {
    let mut v9 = Vec::with_capacity((V9_SIZE * V9_SIZE) as usize);
    for i in 0..V9_SIZE * V9_SIZE {
        let (row, col) = (i / V9_SIZE, i % V9_SIZE);
        v9.push(terrain_height(
            (tile_x * 128) as f32 + col as f32,
            (tile_y * 128) as f32 + row as f32,
        ));
    }
    let mut v8 = Vec::with_capacity((V8_SIZE * V8_SIZE) as usize);
    for i in 0..V8_SIZE * V8_SIZE {
        let (row, col) = (i / V8_SIZE, i % V8_SIZE);
        v8.push(terrain_height(
            (tile_x * 128) as f32 + col as f32 + 0.5,
            (tile_y * 128) as f32 + row as f32 + 0.5,
        ));
    }
    (v9, v8)
}

/// Writes a `.map` v10 file for tile (x, y) (file name order Y then X).
#[allow(clippy::too_many_lines)]
pub fn write_map_file(
    dir: &Path,
    map_id: u32,
    tile_x: u32,
    tile_y: u32,
    heights: Heights,
    liquid: Liquid,
    holes: bool,
) {
    let mut body: Vec<u8> = Vec::new();
    // area header right after the file header (ignored by TerrainBuilder)
    let area_off = 44u32;
    body.extend_from_slice(b"AREA");
    body.extend_from_slice(&1u16.to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());

    let height_off = 44 + body.len() as u32;
    let (v9, v8) = heights_for(tile_x, tile_y);
    let (lo, hi) = v9
        .iter()
        .chain(&v8)
        .fold((f32::MAX, f32::MIN), |(a, b), &h| (a.min(h), b.max(h)));
    let flags = match heights {
        Heights::None => 1,
        Heights::Float => 0,
        Heights::U16 => 2,
        Heights::U8 => 4,
    };
    body.extend_from_slice(b"MHGT");
    body.extend_from_slice(&(flags as u32).to_le_bytes());
    body.extend_from_slice(&lo.to_le_bytes());
    body.extend_from_slice(&hi.to_le_bytes());
    match heights {
        Heights::None => {}
        Heights::Float => {
            for h in v9.iter().chain(&v8) {
                body.extend_from_slice(&h.to_le_bytes());
            }
        }
        Heights::U16 => {
            for h in v9.iter().chain(&v8) {
                let q = ((h - lo) / (hi - lo) * 65535.0).round() as u16;
                body.extend_from_slice(&q.to_le_bytes());
            }
        }
        Heights::U8 => {
            for h in v9.iter().chain(&v8) {
                body.push(((h - lo) / (hi - lo) * 255.0).round() as u8);
            }
        }
    }
    let height_size = 44 + body.len() as u32 - height_off;

    let mut liquid_off = 0u32;
    if liquid != Liquid::None {
        liquid_off = 44 + body.len() as u32;
        body.extend_from_slice(b"MLIQ");
        match liquid {
            Liquid::Typed => {
                let (ox, oy, w, h) = (10u8, 20u8, 70u8, 60u8);
                body.extend_from_slice(&[0, 0]);
                body.extend_from_slice(&0u16.to_le_bytes());
                body.extend_from_slice(&[ox, oy, w, h]);
                body.extend_from_slice(&0f32.to_le_bytes());
                for c in 0..256u16 {
                    body.extend_from_slice(&(c % 4 + 1).to_le_bytes());
                }
                for c in 0..256u32 {
                    let (cr, cc) = (c / 16, c % 16);
                    body.push(match (cr + cc) % 6 {
                        0 | 5 => 0x01,
                        1 => 0x04,
                        2 => 0x10,
                        3 => 0x00,
                        _ => 0x02 | 0x08,
                    });
                }
                for r in 0..u32::from(h) {
                    for c in 0..u32::from(w) {
                        let mut lh = 4.0 + 0.05 * c as f32 - 0.02 * r as f32;
                        if (r * 7 + c * 3) % 23 == 0 {
                            lh = -2000.0; // INVALID_MAP_LIQ_HEIGHT
                        }
                        if (r + c) % 31 == 0 {
                            lh = 6000.0; // above INVALID_MAP_LIQ_HEIGHT_MAX
                        }
                        body.extend_from_slice(&lh.to_le_bytes());
                    }
                }
            }
            Liquid::Flat(flags) => {
                body.extend_from_slice(&[0x01 | 0x02, flags]);
                body.extend_from_slice(&2u16.to_le_bytes());
                body.extend_from_slice(&[0, 0, 129, 129]);
                body.extend_from_slice(&2.5f32.to_le_bytes());
            }
            Liquid::None => unreachable!(),
        }
    }
    let liquid_size = if liquid_off != 0 {
        44 + body.len() as u32 - liquid_off
    } else {
        0
    };

    let (mut holes_off, mut holes_size) = (0u32, 0u32);
    if holes {
        holes_off = 44 + body.len() as u32;
        holes_size = 16 * 16 * 8;
        let mut h = vec![0u8; holes_size as usize];
        h[(3 * 16 + 5) * 8 + 2] = 0xFF;
        for k in 0..8 {
            h[(10 * 16 + 10) * 8 + k] = 0x0F;
        }
        h[(15 * 16 + 15) * 8 + 7] = 0x81;
        body.extend_from_slice(&h);
    }

    let header = crate::map_defines::MapFileHeader {
        map_magic: *b"MAPS",
        version_magic: 10,
        build_magic: 54261,
        area_map_offset: area_off,
        area_map_size: 8,
        height_map_offset: height_off,
        height_map_size: height_size,
        liquid_map_offset: liquid_off,
        liquid_map_size: liquid_size,
        holes_offset: holes_off,
        holes_size,
    };
    let mut file = header.to_bytes().to_vec();
    file.extend_from_slice(&body);
    std::fs::create_dir_all(dir.join("maps")).unwrap();
    std::fs::write(
        dir.join(format!("maps/{map_id:04}_{tile_y:02}_{tile_x:02}.map")),
        file,
    )
    .unwrap();
}

fn house_model() -> WorldModel {
    let v = Vector3::new;
    // group 0: platform with a ramp and a wall (model space, z up)
    let verts = vec![
        v(-20.0, -20.0, 0.0),
        v(20.0, -20.0, 0.0),
        v(20.0, 20.0, 0.0),
        v(-20.0, 20.0, 0.0),
        v(-45.0, -10.0, -12.0),
        v(-45.0, 10.0, -12.0),
        v(20.0, 20.0, 8.0),
        v(-20.0, 20.0, 8.0),
    ];
    let tris = vec![
        MeshTriangle::new(0, 1, 2),
        MeshTriangle::new(0, 2, 3),
        MeshTriangle::new(4, 0, 3),
        MeshTriangle::new(4, 3, 5),
        MeshTriangle::new(3, 2, 6),
        MeshTriangle::new(3, 6, 7),
    ];
    let bound = AABox::new(v(-45.0, -20.0, -12.0), v(20.0, 20.0, 8.0));
    let mut g0 = GroupModel::new(0, 1, bound);
    g0.set_mesh_data(verts, tris).unwrap();

    // group 1: pool basin with a liquid
    let pverts = vec![
        v(25.0, -20.0, -3.0),
        v(60.0, -20.0, -3.0),
        v(60.0, 10.0, -3.0),
        v(25.0, 10.0, -3.0),
    ];
    let ptris = vec![MeshTriangle::new(0, 1, 2), MeshTriangle::new(0, 2, 3)];
    let pbound = AABox::new(v(25.0, -20.0, -3.0), v(60.0, 10.0, 2.0));
    let mut g1 = GroupModel::new(0, 2, pbound);
    g1.set_mesh_data(pverts, ptris).unwrap();
    let mut liq = WmoLiquid::new(8, 6, v(25.0, -20.0, 0.0), 1);
    for (i, h) in liq.height_storage_mut().iter_mut().enumerate() {
        *h = 1.0 + (i % 5) as f32 * 0.1;
    }
    for (i, f) in liq.flags_storage_mut().unwrap().iter_mut().enumerate() {
        *f = if i % 7 == 3 { 0x0f } else { 0x00 };
    }
    g1.set_liquid_data(Some(liq));

    // group 2: magma strip (type 3) and an unknown liquid type (no area)
    let mut g2 = GroupModel::new(0, 3, AABox::new(v(-20.0, 25.0, -5.0), v(20.0, 45.0, 0.0)));
    g2.set_mesh_data(
        vec![
            v(-20.0, 25.0, -5.0),
            v(20.0, 25.0, -5.0),
            v(0.0, 45.0, -5.0),
        ],
        vec![MeshTriangle::new(0, 1, 2)],
    )
    .unwrap();
    let mut magma = WmoLiquid::new(5, 3, v(-20.0, 25.0, -1.0), 3);
    magma.height_storage_mut().fill(-1.0);
    g2.set_liquid_data(Some(magma));

    let mut wm = WorldModel::new();
    wm.set_root_wmo_id(77);
    wm.set_group_models(vec![g0, g1, g2]).unwrap();
    wm
}

fn rock_model() -> WorldModel {
    let v = Vector3::new;
    let verts = vec![
        v(-6.0, -6.0, 0.0),
        v(6.0, -6.0, 0.0),
        v(0.0, 7.0, 0.0),
        v(0.0, 0.0, 9.0),
    ];
    let tris = vec![
        MeshTriangle::new(0, 1, 3),
        MeshTriangle::new(1, 2, 3),
        MeshTriangle::new(2, 0, 3),
        MeshTriangle::new(0, 2, 1),
    ];
    let mut g = GroupModel::new(0, 0, AABox::new(v(-6.0, -6.0, 0.0), v(6.0, 7.0, 9.0)));
    g.set_mesh_data(verts, tris).unwrap();
    let mut wm = WorldModel::new();
    wm.set_group_models(vec![g]).unwrap();
    wm
}

fn spawns() -> Vec<ModelSpawn> {
    let g = GRID_SIZE;
    // map tile (x=32, y=31) spans internal y in [32G, 33G], x in [31G, 32G]
    vec![
        ModelSpawn {
            flags: MOD_HAS_BOUND,
            adt_id: 0,
            id: 1001,
            pos: Vector3::new(31.0 * g + 300.0, 32.0 * g + 260.0, 12.0),
            rot: Vector3::new(0.0, 30.0, 0.0),
            scale: 1.0,
            bound: AABox::new(
                Vector3::new(31.0 * g + 240.0, 32.0 * g + 200.0, -10.0),
                Vector3::new(31.0 * g + 370.0, 32.0 * g + 330.0, 25.0),
            ),
            name: "house.wmo".into(),
        },
        ModelSpawn {
            flags: MOD_HAS_BOUND | MOD_M2,
            adt_id: 0,
            id: 1002,
            pos: Vector3::new(31.0 * g + 150.0, 32.0 * g + 520.0, 3.0),
            rot: Vector3::new(10.0, 45.0, -5.0),
            scale: 1.7,
            bound: AABox::new(
                Vector3::new(31.0 * g + 135.0, 32.0 * g + 505.0, 0.0),
                Vector3::new(31.0 * g + 165.0, 32.0 * g + 545.0, 20.0),
            ),
            name: "rock.m2".into(),
        },
    ]
}

fn write_vmtree(path: &Path, spawns: &[ModelSpawn]) {
    let mut tree = Bih::default();
    let bounds: Vec<AABox> = spawns.iter().map(|s| s.bound).collect();
    tree.build(&bounds, 3).unwrap();
    let mut buf = Vec::new();
    buf.put_bytes(VMAP_MAGIC);
    buf.put_bytes(b"NODE");
    tree.write_to(&mut buf);
    buf.put_bytes(b"SIDX");
    buf.put_u32(spawns.len() as u32);
    for s in spawns {
        buf.put_u32(s.id);
    }
    std::fs::write(path, buf).unwrap();
}

fn write_vmtile(path: &Path, spawns: &[&ModelSpawn]) {
    let mut buf = Vec::new();
    buf.put_bytes(VMAP_MAGIC);
    buf.put_u32(spawns.len() as u32);
    for s in spawns {
        s.write_to(&mut buf);
    }
    std::fs::write(path, buf).unwrap();
}

/// Map.db2 stand-in rows: (id, MapType, InstanceType, ParentMapID, CosmeticParentMapID, Flags1).
const MAP_ROWS: [(u32, u8, u8, u16, u16, i32); 3] = [
    (1, 0, 0, 0xFFFF, 0xFFFF, 0),
    (2, 0, 1, 1, 0xFFFF, 0),
    (3, 3, 0, 0xFFFF, 0xFFFF, 0),
];
/// LiquidType.db2 stand-in rows: (id, SoundBank).
const LIQUID_ROWS: [(u32, u8); 4] = [(1, 0), (2, 1), (3, 2), (4, 4)];

/// Writes the whole fixture into `dir`.
pub fn build_fixture(dir: &Path) {
    // terrain
    write_map_file(dir, 1, 32, 31, Heights::Float, Liquid::Typed, true);
    write_map_file(dir, 1, 33, 31, Heights::U16, Liquid::Flat(0x01), false);
    write_map_file(dir, 1, 32, 32, Heights::U8, Liquid::None, true);
    write_map_file(dir, 1, 31, 31, Heights::None, Liquid::Flat(0x04), false);

    // vmaps
    let vm = dir.join("vmaps");
    std::fs::create_dir_all(&vm).unwrap();
    house_model().write_file(vm.join("house.wmo.vmo")).unwrap();
    rock_model().write_file(vm.join("rock.m2.vmo")).unwrap();
    let sp = spawns();
    write_vmtree(&vm.join(map_file_name(1)), &sp);
    // child map 2 knows the parent's spawns (vmtile fallback to the parent)
    write_vmtree(&vm.join(map_file_name(2)), &sp);
    // map tile (x, y) is loaded as vmap tile (y, x)
    write_vmtile(&vm.join(tile_file_name(1, 31, 32)), &[&sp[0], &sp[1]]);
    write_vmtile(&vm.join(tile_file_name(1, 31, 33)), &[&sp[1]]);

    // dbc stand-ins
    let dbc = dir.join("dbc/enUS");
    std::fs::create_dir_all(&dbc).unwrap();
    let mut map_txt = String::new();
    for (id, mt, it, p, cp, f) in MAP_ROWS {
        map_txt += &format!(
            "{id} MapType={mt} InstanceType={it} ParentMapID={p} CosmeticParentMapID={cp} Flags1={f}\n"
        );
    }
    std::fs::write(dbc.join("Map.db2"), map_txt).unwrap();
    let mut liq_txt = String::new();
    for (id, sb) in LIQUID_ROWS {
        liq_txt += &format!("{id} SoundBank={sb}\n");
    }
    std::fs::write(dbc.join("LiquidType.db2"), liq_txt).unwrap();

    // off-mesh connections (recast y = world x, recast x = world y)
    let h = |wx: f32, wy: f32| {
        // world (x, y) -> global grid position
        let gx = (32.0 * GRID_SIZE - wy) / GRID_SIZE * 128.0;
        let gy = (32.0 * GRID_SIZE - wx) / GRID_SIZE * 128.0;
        terrain_height(gx, gy)
    };
    let off = format!(
        "# comment line\n1 32,31 (100.0 -100.0 {:.3}) (104.0 -97.0 {:.3}) 1.5 11 1\n1 32,31 (200 -300 {:.3}) (206 -303 {:.3}) 2.0\n1 32,31 (300 -200 {:.3}) (303 -206 {:.3}) 1.0 9\n2 32,31 (100 -100 0) (101 -101 0)\n",
        h(100.0, -100.0),
        h(104.0, -97.0),
        h(200.0, -300.0),
        h(206.0, -303.0),
        h(300.0, -200.0),
        h(303.0, -206.0),
    );
    std::fs::write(dir.join("offmesh.txt"), off).unwrap();
}

/// `GeneratorData` equivalent to the fixture's DB2 stand-ins, built with the
/// same record logic as `LoadMap` / `LoadLiquid`.
pub fn fixture_data() -> Arc<GeneratorData> {
    let mut data = GeneratorData::default();
    let mut map_data: HashMap<u32, Vec<u32>> = HashMap::new();
    for (id, mt, it, p, cp, f) in MAP_ROWS {
        let r = MapRecord {
            id,
            map_type: mt,
            instance_type: it,
            parent_map_id: p,
            cosmetic_parent_map_id: cp,
            flags1: f,
        };
        apply_map_record(&r, &mut map_data, &mut data.map_store);
    }
    data.map_data_for_vmap = map_data;
    data.liquid_types = LIQUID_ROWS.into_iter().collect();
    Arc::new(data)
}

/// Fresh empty temp directory, removed on drop.
pub struct TestDir(std::path::PathBuf);

impl std::ops::Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Fresh empty directory under the system temp dir.
pub fn fresh_dir(name: &str) -> TestDir {
    let dir =
        std::env::temp_dir().join(format!("wow-mmaps-generator-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}
