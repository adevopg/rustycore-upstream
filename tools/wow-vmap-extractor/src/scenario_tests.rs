//! End-to-end extraction of a synthetic client data set (WDT/ADT/WMO/M2, parent map
//! cache, doodad sets, liquids, FileDataID and name references, gameobject models),
//! compared with the output of the TrinityCore C++ sources.
//!
//! The expected FNV-1a digests were produced by compiling the unmodified TDB343.24081
//! `vmap4_extractor` sources (`wmo.cpp`, `model.cpp`, `adtfile.cpp`, `wdtfile.cpp`, and
//! `ExtractSingleWmo` / `ParsMapFiles` / `GenerateUniqueObjectId` / the
//! `ExtractGameobjectModels` loop of `vmapexport.cpp` / `gameobject_extract.cpp`) with
//! g++ 13.3 against G3D from `dep/g3dlite`, with `CASCFile` replaced by a reader of a
//! directory holding the files dumped by this test (`VMAP_ORACLE_DUMP=<dir>`), and
//! hashing every file the C++ wrote to its `Buildings/` directory.
//! Set `VMAP_ORACLE_PRINT=1` to print the Rust digests.

#![allow(clippy::unreadable_literal, clippy::approx_constant)]

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use crate::cascfile::tests::MemSource;
use crate::modelheaders::MODEL_HEADER_SIZE;
use crate::vmapexport::{MapEntry, VmapExport};

#[allow(clippy::trivially_copy_pass_by_ref)]
fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = vec![id[3], id[2], id[1], id[0]];
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend_from_slice(data);
    v
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn u32s(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn u16s(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn mver() -> Vec<u8> {
    chunk(b"MVER", &18u32.to_le_bytes())
}

/// M2 with `prefix` before the MD20 header (offsets relative to MD20).
fn m2(prefix: &[u8], verts: &[[f32; 3]], tris: &[u16], collision: [f32; 6]) -> Vec<u8> {
    let mut hdr = vec![0u8; MODEL_HEADER_SIZE];
    hdr[0..4].copy_from_slice(b"MD20");
    hdr[0xBC..0xBC + 24].copy_from_slice(&f32s(&collision));
    let ofs_verts = MODEL_HEADER_SIZE as u32 + 16;
    let ofs_tris = ofs_verts + verts.len() as u32 * 12 + 4;
    hdr[0xD8..0xDC].copy_from_slice(&(tris.len() as u32).to_le_bytes());
    hdr[0xDC..0xE0].copy_from_slice(&ofs_tris.to_le_bytes());
    hdr[0xE0..0xE4].copy_from_slice(&(verts.len() as u32).to_le_bytes());
    hdr[0xE4..0xE8].copy_from_slice(&ofs_verts.to_le_bytes());
    let mut out = prefix.to_vec();
    out.extend_from_slice(&hdr);
    out.extend_from_slice(&[0xEEu8; 16]);
    for v in verts {
        out.extend_from_slice(&f32s(v));
    }
    out.extend_from_slice(&[0xEEu8; 4]);
    out.extend_from_slice(&u16s(tris));
    out
}

fn md21(verts: &[[f32; 3]], tris: &[u16], collision: [f32; 6]) -> Vec<u8> {
    let inner = m2(&[], verts, tris, collision);
    let mut out = chunk(b"12DM", &inner); // stored as "MD21"
    out.extend_from_slice(&chunk(b"DIFS", &[0u8; 8]));
    out
}

struct Modd {
    name_index: u32,
    pos: [f32; 3],
    quat: [f32; 4],
    scale: f32,
}

struct Root<'a> {
    n_groups: u32,
    wmo_id: u32,
    flags: u16,
    mogn: &'a [u8],
    mods: &'a [(u32, u32)],
    modn: Option<&'a [u8]>,
    modi: Option<&'a [u32]>,
    modd: &'a [Modd],
    gfid: &'a [u32],
}

fn wmo_root(r: &Root<'_>) -> Vec<u8> {
    let mut out = mver();
    let mut mohd = u32s(&[
        1,
        r.n_groups,
        0,
        0,
        3,
        r.modd.len() as u32,
        r.mods.len() as u32,
    ]);
    mohd.extend_from_slice(&u32s(&[0xFF80_4020, r.wmo_id]));
    mohd.extend_from_slice(&f32s(&[-10.0, -20.0, -30.0, 10.0, 20.0, 30.0]));
    mohd.extend_from_slice(&u16s(&[r.flags, 0]));
    out.extend_from_slice(&chunk(b"MOHD", &mohd));
    out.extend_from_slice(&chunk(b"MOTX", b"tex.blp\0"));
    out.extend_from_slice(&chunk(b"MOGN", r.mogn));
    let mut mods = Vec::new();
    for (i, (start, count)) in r.mods.iter().enumerate() {
        let mut name = [0u8; 20];
        name[0] = b'S';
        name[1] = b'0' + i as u8;
        mods.extend_from_slice(&name);
        mods.extend_from_slice(&u32s(&[*start, *count, 0]));
    }
    out.extend_from_slice(&chunk(b"MODS", &mods));
    if let Some(modn) = r.modn {
        out.extend_from_slice(&chunk(b"MODN", modn));
    }
    if let Some(modi) = r.modi {
        out.extend_from_slice(&chunk(b"MODI", &u32s(modi)));
    }
    let mut modd = Vec::new();
    for d in r.modd {
        modd.extend_from_slice(&(d.name_index | 0x2A00_0000).to_le_bytes());
        modd.extend_from_slice(&f32s(&d.pos));
        modd.extend_from_slice(&f32s(&d.quat));
        modd.extend_from_slice(&d.scale.to_le_bytes());
        modd.extend_from_slice(&0xFF00_FF00u32.to_le_bytes());
    }
    out.extend_from_slice(&chunk(b"MODD", &modd));
    out.extend_from_slice(&chunk(b"GFID", &u32s(r.gfid)));
    out
}

#[derive(Default)]
struct Group {
    name_ofs: i32,
    flags: i32,
    liquid: u32,
    wmo_id: u32,
    mopy: Option<Vec<u8>>,
    mpy2: Option<Vec<u16>>,
    movi: Option<Vec<u16>>,
    movx: Option<Vec<u32>>,
    movt: Vec<f32>,
    moba: Vec<u16>,
    modr: Vec<u16>,
    /// (xverts, yverts, xtiles, ytiles, heights, tile bytes)
    #[allow(clippy::type_complexity)]
    mliq: Option<(i32, i32, i32, i32, Vec<f32>, Vec<u8>)>,
}

fn wmo_group(g: &Group) -> Vec<u8> {
    let mut hdr = Vec::new();
    hdr.extend_from_slice(&g.name_ofs.to_le_bytes());
    hdr.extend_from_slice(&0i32.to_le_bytes());
    hdr.extend_from_slice(&g.flags.to_le_bytes());
    hdr.extend_from_slice(&f32s(&[-1.5, -2.5, -3.5, 1.5, 2.5, 3.5]));
    hdr.extend_from_slice(&u16s(&[0, 0, 1, 2]));
    hdr.extend_from_slice(&u32s(&[3, 0x0102_0304, g.liquid, g.wmo_id, 0, 0]));
    assert_eq!(hdr.len(), 68);
    let mut sub = Vec::new();
    if let Some(mopy) = &g.mopy {
        sub.extend_from_slice(&chunk(b"MOPY", mopy));
    }
    if let Some(mpy2) = &g.mpy2 {
        sub.extend_from_slice(&chunk(b"MPY2", &u16s(mpy2)));
    }
    if let Some(movi) = &g.movi {
        sub.extend_from_slice(&chunk(b"MOVI", &u16s(movi)));
    }
    if let Some(movx) = &g.movx {
        sub.extend_from_slice(&chunk(b"MOVX", &u32s(movx)));
    }
    sub.extend_from_slice(&chunk(b"MOVT", &f32s(&g.movt)));
    sub.extend_from_slice(&chunk(b"MONR", &[0u8; 12]));
    sub.extend_from_slice(&chunk(b"MOBA", &u16s(&g.moba)));
    if !g.modr.is_empty() {
        sub.extend_from_slice(&chunk(b"MODR", &u16s(&g.modr)));
    }
    if let Some((xv, yv, xt, yt, heights, bytes)) = &g.mliq {
        let mut m = Vec::new();
        for v in [*xv, *yv, *xt, *yt] {
            m.extend_from_slice(&v.to_le_bytes());
        }
        m.extend_from_slice(&f32s(&[100.0, 200.0, -5.25]));
        m.extend_from_slice(&7i16.to_le_bytes());
        for (i, h) in heights.iter().enumerate() {
            m.extend_from_slice(&u16s(&[i as u16, 0xABCD]));
            m.extend_from_slice(&h.to_le_bytes());
        }
        m.extend_from_slice(bytes);
        sub.extend_from_slice(&chunk(b"MLIQ", &m));
    }
    let mut mogp = hdr;
    mogp.extend_from_slice(&sub);
    let mut out = mver();
    out.extend_from_slice(&chunk(b"MOGP", &mogp));
    out
}

fn moba(counts: &[u16]) -> Vec<u16> {
    let mut v = Vec::new();
    for (i, c) in counts.iter().enumerate() {
        let mut e = [0u16; 12];
        e[6] = (i * 3) as u16;
        e[8] = *c;
        e[11] = 0x0501;
        v.extend_from_slice(&e);
    }
    v
}

fn mddf(id: u32, unique: u32, pos: [f32; 3], rot: [f32; 3], scale: u16, flags: u16) -> Vec<u8> {
    let mut v = u32s(&[id, unique]);
    v.extend_from_slice(&f32s(&pos));
    v.extend_from_slice(&f32s(&rot));
    v.extend_from_slice(&u16s(&[scale, flags]));
    v
}

#[allow(clippy::too_many_arguments)]
fn modf(
    id: u32,
    unique: u32,
    pos: [f32; 3],
    rot: [f32; 3],
    bounds: [f32; 6],
    flags: u16,
    doodad_set: u16,
    name_set: u16,
    scale: u16,
) -> Vec<u8> {
    let mut v = u32s(&[id, unique]);
    v.extend_from_slice(&f32s(&pos));
    v.extend_from_slice(&f32s(&rot));
    v.extend_from_slice(&f32s(&bounds));
    v.extend_from_slice(&u16s(&[flags, doodad_set, name_set, scale]));
    v
}

fn wdt(flags: u32, tiles: &[(usize, usize, u32)], globals: Option<(&[u8], Vec<u8>)>) -> Vec<u8> {
    let mut out = mver();
    let mut mphd = u32s(&[flags]);
    mphd.extend_from_slice(&[0u8; 28]);
    out.extend_from_slice(&chunk(b"MPHD", &mphd));
    let mut main = vec![0u8; 64 * 64 * 8];
    let mut maid = vec![0u8; 64 * 64 * 32];
    for &(x, y, obj0) in tiles {
        let i = y * 64 + x;
        main[i * 8..i * 8 + 4].copy_from_slice(&1u32.to_le_bytes());
        maid[i * 32 + 4..i * 32 + 8].copy_from_slice(&obj0.to_le_bytes());
    }
    out.extend_from_slice(&chunk(b"MAIN", &main));
    if flags & 0x200 != 0 {
        out.extend_from_slice(&chunk(b"MAID", &maid));
    }
    if let Some((mwmo, modf)) = globals {
        out.extend_from_slice(&chunk(b"MWMO", mwmo));
        out.extend_from_slice(&chunk(b"MODF", &modf));
    }
    out
}

fn tri_model(seed: f32) -> Vec<[f32; 3]> {
    vec![
        [seed, 0.5, -1.0],
        [1.0, seed * 2.0, 0.25],
        [-0.75, 3.0, seed],
        [2.0, -seed, 1.5],
    ]
}

/// The synthetic storage: `(name, bytes)` for name lookups and `(FileDataID, bytes)`.
fn build_source() -> MemSource {
    let mut src = MemSource::default();
    let mut name = |n: &str, d: Vec<u8>| {
        src.by_name.insert(n.to_owned(), d);
    };

    // ---- M2 models ----
    let tris = [0u16, 1, 2, 2, 3, 0, 1, 3, 2];
    name(
        "World\\Generic\\Tree Big.M2",
        m2(
            &[],
            &tri_model(1.25),
            &tris,
            [-1.0, -2.0, -3.0, 4.0, 5.0, 6.0],
        ),
    );
    name(
        "Doodads\\rock.m2",
        md21(&tri_model(-2.5), &tris[..6], [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
    );
    name(
        "World\\Generic\\Barrel.m2",
        m2(
            &[],
            &tri_model(0.125),
            &tris,
            [-0.5, -0.5, 0.0, 0.5, 0.5, 1.0],
        ),
    );
    name(
        "World\\Generic\\Crate.M2",
        m2(
            &[],
            &tri_model(7.0),
            &tris[..3],
            [-2.0, -2.0, 0.0, 2.0, 2.0, 2.0],
        ),
    );
    // no bounding triangles: never written
    name(
        "World\\Generic\\Flat.m2",
        m2(&[], &tri_model(3.0), &[], [0.0; 6]),
    );

    // ---- WMO "TownHall" (MODN doodads, 3 groups, one antiportal) ----
    let modn = b"World\\Generic\\Barrel.m2\0World\\Generic\\Crate.MDX\0World\\Generic\\Flat.m2\0\0";
    let barrel = 0u32;
    let crate_ = 24u32;
    let flat = 48u32;
    name(
        "World\\wmo\\Town\\TownHall.wmo",
        wmo_root(&Root {
            n_groups: 3,
            wmo_id: 4711,
            flags: 0,
            mogn: b"\0TownHall_000\0antiportal\0",
            mods: &[(0, 3), (3, 2)],
            modn: Some(modn),
            modi: None,
            modd: &[
                Modd {
                    name_index: barrel,
                    pos: [1.0, 2.0, 3.0],
                    quat: [0.1, 0.2, 0.3, 0.9],
                    scale: 1.0,
                },
                Modd {
                    name_index: crate_,
                    pos: [-4.5, 0.25, 10.0],
                    quat: [0.0, 0.0, 0.0, 1.0],
                    scale: 0.75,
                },
                Modd {
                    name_index: flat,
                    pos: [0.0, 0.0, 0.0],
                    quat: [0.0, 0.0, 0.0, 1.0],
                    scale: 1.0,
                },
                Modd {
                    name_index: barrel,
                    pos: [7.0, -8.0, 9.5],
                    quat: [0.5, -0.5, 0.5, 0.5],
                    scale: 2.0,
                },
                Modd {
                    name_index: barrel,
                    pos: [33.0, 44.0, 55.0],
                    quat: [0.0, 0.7071068, 0.0, 0.7071068],
                    scale: 1.5,
                },
            ],
            gfid: &[0x40_0000, 0x40_0001, 0x40_0002],
        }),
    );
    let movt: Vec<f32> = (0..8 * 3).map(|i| i as f32 * 1.5 - 7.0).collect();
    src.by_id.insert(
        0x40_0000,
        wmo_group(&Group {
            name_ofs: 1,
            flags: 0x0000_0008,
            liquid: 0,
            wmo_id: 77,
            mopy: Some(vec![0x08, 1, 0x20, 2, 0x24, 3, 0x00, 4, 0x28, 5]),
            movi: Some(vec![0, 1, 2, 2, 3, 4, 4, 5, 6, 6, 7, 0, 7, 6, 5]),
            movt: movt.clone(),
            moba: moba(&[6, 9]),
            modr: vec![0, 1, 2, 3, 4, 9, 3],
            mliq: Some((
                3,
                2,
                2,
                1,
                vec![1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
                vec![0x0F, 0x43],
            )),
            ..Group::default()
        }),
    );
    src.by_id.insert(
        0x40_0001,
        wmo_group(&Group {
            name_ofs: 14, // "antiportal"
            movi: Some(vec![0, 1, 2]),
            mopy: Some(vec![0x08, 0]),
            movt: movt[..9].to_vec(),
            moba: moba(&[3]),
            ..Group::default()
        }),
    );
    src.by_id.insert(
        0x40_0002,
        wmo_group(&Group {
            name_ofs: 0,
            flags: 0x0008_0000,
            liquid: 15,
            wmo_id: 78,
            mpy2: Some(vec![0x20, 9, 0x04, 9, 0x08, 9]),
            movx: Some(vec![3, 2, 1, 0, 1, 2, 3, 3, 3]),
            movt: movt[..12].to_vec(),
            moba: moba(&[9]),
            mliq: Some((2, 2, 1, 1, vec![-1.0, -2.0, -3.0, -4.0], vec![0x02])),
            ..Group::default()
        }),
    );

    // ---- WMO by FileDataID (MODI doodads, liquid type from MOGP, flags & 4) ----
    src.by_id.insert(
        0x30_0000,
        wmo_root(&Root {
            n_groups: 2,
            wmo_id: 900,
            flags: 4,
            mogn: b"\0g\0",
            mods: &[(0, 4)],
            modn: None,
            modi: Some(&[0x20_0000, 0, 0x20_0001, 0x20_0002]),
            modd: &[
                Modd {
                    name_index: 0,
                    pos: [0.5, 0.5, 0.5],
                    quat: [0.2, 0.1, 0.4, 0.8],
                    scale: 1.0,
                },
                Modd {
                    name_index: 2,
                    pos: [1.0, 1.0, 1.0],
                    quat: [0.0, 0.0, 0.0, 1.0],
                    scale: 1.0,
                },
                Modd {
                    name_index: 3,
                    pos: [-3.0, 2.0, -1.0],
                    quat: [0.0, 0.0, 0.3826834, 0.9238795],
                    scale: 0.5,
                },
                Modd {
                    name_index: 0,
                    pos: [9.0, 9.0, 9.0],
                    quat: [0.0, 0.0, 0.0, 1.0],
                    scale: 3.0,
                },
            ],
            gfid: &[0, 0x40_0010],
        }),
    );
    src.by_id.insert(
        0x40_0010,
        wmo_group(&Group {
            name_ofs: 1,
            liquid: 6,
            wmo_id: 1,
            mopy: Some(vec![0x20, 0, 0x08, 0]),
            movi: Some(vec![0, 1, 2, 1, 2, 3]),
            movt: movt[..12].to_vec(),
            moba: moba(&[6]),
            modr: vec![3, 2, 1, 0],
            ..Group::default()
        }),
    );
    src.by_id.insert(
        0x20_0000,
        md21(&tri_model(9.0), &tris, [-9.0, -9.0, -9.0, 9.0, 9.0, 9.0]),
    );
    src.by_id
        .insert(0x20_0002, m2(&[], &tri_model(0.5), &tris[..3], [0.0; 6]));

    // ---- global WMO whose second group is missing (file removed) ----
    name(
        "World\\wmo\\Global\\GlobalBuilding.wmo",
        wmo_root(&Root {
            n_groups: 2,
            wmo_id: 5,
            flags: 0,
            mogn: b"\0",
            mods: &[(0, 0)],
            modn: Some(b"\0"),
            modi: None,
            modd: &[],
            gfid: &[0x40_0000, 0x40_0099],
        }),
    );

    // ---- gameobject WMO by FileDataID, groups reused ----
    src.by_id.insert(
        0x30_0001,
        wmo_root(&Root {
            n_groups: 1,
            wmo_id: 12,
            flags: 0,
            mogn: b"\0",
            mods: &[],
            modn: None,
            modi: Some(&[]),
            modd: &[],
            gfid: &[0x40_0002],
        }),
    );
    // gameobject with an unknown header would abort; not included.

    // ---- maps ----
    let mut global_modf = modf(
        0,
        0x7000_0001,
        [100.0, 50.0, 200.0],
        [0.0, 90.0, 0.0],
        [90.0, 40.0, 190.0, 110.0, 60.0, 210.0],
        0,
        0,
        0,
        0,
    );
    global_modf.extend_from_slice(&modf(
        0,
        0x7000_0002,
        [10.0, 20.0, 30.0],
        [15.0, 30.0, 45.0],
        [0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        0x8,
        0,
        0,
        0,
    ));
    // (id 0x30_0000 via flag 8)
    let len = global_modf.len();
    global_modf[len - 64..len - 60].copy_from_slice(&0x30_0000u32.to_le_bytes());
    name(
        "World\\Maps\\Test\\Test.wdt",
        wdt(
            0x200,
            &[(0, 0, 0x10_0000), (1, 0, 0x10_0001), (5, 7, 0x10_0002)],
            Some((b"World\\wmo\\Global\\GlobalBuilding.wmo\0", global_modf)),
        ),
    );
    name("World\\Maps\\Child\\Child.wdt", wdt(0, &[(0, 0, 0)], None));

    let mut adt = mver();
    adt.extend_from_slice(&chunk(
        b"MMDX",
        b"World\\Generic\\Tree Big.MDX\0Doodads\\rock.m2\0",
    ));
    adt.extend_from_slice(&chunk(b"MMID", &u32s(&[0, 27])));
    adt.extend_from_slice(&chunk(b"MWMO", b"World\\wmo\\Town\\TownHall.wmo\0"));
    adt.extend_from_slice(&chunk(b"MWID", &u32s(&[0])));
    let mut d = mddf(0, 1001, [1000.0, 50.0, 2000.0], [0.0, 45.0, 0.0], 1024, 0);
    d.extend_from_slice(&mddf(
        1,
        1002,
        [1100.5, 51.25, 2100.75],
        [10.0, 20.0, 30.0],
        512,
        0,
    ));
    d.extend_from_slice(&mddf(
        0x20_0000,
        1003,
        [1200.0, 52.0, 2200.0],
        [-5.0, 0.0, 355.0],
        2048,
        0x40,
    ));
    d.extend_from_slice(&mddf(1, 1002, [1.0, 2.0, 3.0], [0.0, 0.0, 0.0], 1024, 0));
    adt.extend_from_slice(&chunk(b"MDDF", &d));
    let mut m = modf(
        0,
        2001,
        [1500.0, 60.0, 2500.0],
        [0.0, 180.0, 0.0],
        [1400.0, 50.0, 2400.0, 1600.0, 70.0, 2600.0],
        0x4,
        1,
        3,
        512,
    );
    m.extend_from_slice(&modf(
        0x30_0000,
        2002,
        [1700.0, 65.0, 2700.0],
        [30.0, 60.0, 90.0],
        [1690.0, 60.0, 2690.0, 1710.0, 70.0, 2710.0],
        0x8,
        0,
        0,
        0,
    ));
    m.extend_from_slice(&modf(0, 2003, [0.0; 3], [0.0; 3], [0.0; 6], 0x1, 0, 0, 0));
    m.extend_from_slice(&modf(
        0,
        2001,
        [1500.0, 60.0, 2500.0],
        [0.0, 180.0, 0.0],
        [1400.0, 50.0, 2400.0, 1600.0, 70.0, 2600.0],
        0x0,
        7,
        3,
        0,
    ));
    adt.extend_from_slice(&chunk(b"MODF", &m));
    src.by_id.insert(0x10_0000, adt);
    // tile (5, 7): ADT without placements
    src.by_id.insert(0x10_0002, mver());

    src
}

fn maps() -> (Vec<MapEntry>, HashSet<u32>) {
    let entry = |id: u32, parent: i16, dir: &str| MapEntry {
        id,
        parent_map_id: parent,
        name: dir.to_owned(),
        directory: dir.to_owned(),
    };
    (
        vec![
            entry(1, -1, "Test"),
            entry(2, 1, "Child"),
            entry(3, -1, "Lonely"),
        ],
        HashSet::from([1]),
    )
}

/// 64-bit FNV-1a.
fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wow-vmap-extractor-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn dump_inputs(src: &MemSource, dir: &Path) {
    std::fs::create_dir_all(dir).expect("dump dir");
    for (name, data) in &src.by_name {
        std::fs::write(dir.join(name), data).expect("dump");
    }
    for (id, data) in &src.by_id {
        std::fs::write(dir.join(format!("FILE{id:08X}.xxx")), data).expect("dump");
    }
}

fn run(precise: bool) -> BTreeMap<String, (usize, u64)> {
    let src = build_source();
    if let Ok(dir) = std::env::var("VMAP_ORACLE_DUMP") {
        dump_inputs(&src, Path::new(&dir));
    }
    let work = temp_dir(if precise { "precise" } else { "small" });
    let mut ctx = VmapExport::new(&src, work.clone(), precise);
    ctx.write_gameobject_models(&[
        (10, 0x20_0000),
        (11, 0x30_0001),
        (12, 0),
        (13, 0x99_9999),
        (14, 0x30_0000),
    ]);
    let (map_ids, parents) = maps();
    ctx.pars_map_files(&map_ids, &parents);

    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(&work).expect("read work dir") {
        let entry = entry.expect("entry");
        let data = std::fs::read(entry.path()).expect("read");
        out.insert(
            entry.file_name().to_string_lossy().into_owned(),
            (data.len(), fnv1a(&data)),
        );
    }
    if std::env::var("VMAP_ORACLE_PRINT").is_ok() {
        for (name, (len, hash)) in &out {
            println!("{precise} {name} {len} {hash:016x}");
        }
    }
    if let Ok(keep) = std::env::var("VMAP_ORACLE_KEEP") {
        let dest = Path::new(&keep).join(if precise { "precise" } else { "small" });
        let _ = std::fs::remove_dir_all(&dest);
        std::fs::create_dir_all(&dest).expect("keep dir");
        for entry in std::fs::read_dir(&work).expect("read work dir").flatten() {
            std::fs::copy(entry.path(), dest.join(entry.file_name())).expect("copy");
        }
    }
    let _ = std::fs::remove_dir_all(&work);
    out
}

fn expected(table: &[(&str, usize, u64)]) -> BTreeMap<String, (usize, u64)> {
    table
        .iter()
        .map(|(n, l, h)| ((*n).to_owned(), (*l, *h)))
        .collect()
}

#[test]
fn small_output_matches_cpp_extractor() {
    assert_eq!(run(false), expected(EXPECTED_SMALL));
}

#[test]
fn precise_output_matches_cpp_extractor() {
    assert_eq!(run(true), expected(EXPECTED_PRECISE));
}

/// Digests of the C++ oracle output (small / default mode).
const EXPECTED_SMALL: &[(&str, usize, u64)] = &[
    ("Barrel.m2", 180, 0x9e1c1b730ccedab8),
    ("Crate.m2", 156, 0xd64e631ef4970177),
    ("FILE00200000.xxx", 180, 0x83ca13e82d8ec7c9),
    ("FILE00200002.xxx", 156, 0x8db8df4ca7b729f4),
    ("FILE00300000.xxx", 180, 0xf41a41bbbcf7b11a),
    ("FILE00300001.xxx", 215, 0x8f3b7de9d78bf790),
    ("Rock.m2", 168, 0xc5aa9fa0b2412a22),
    ("Townhall.wmo", 495, 0x2f80c960bf1babc2),
    ("Tree_Big.m2", 180, 0xb043df391c61d4d5),
    ("dir_bin", 2208, 0x53f425aed493aaf3),
    ("temp_gameobject_models", 83, 0x8e821831a97ccae1),
];
/// Digests of the C++ oracle output (`-l`).
const EXPECTED_PRECISE: &[(&str, usize, u64)] = &[
    ("Barrel.m2", 180, 0x9e1c1b730ccedab8),
    ("Crate.m2", 156, 0xd64e631ef4970177),
    ("FILE00200000.xxx", 180, 0x83ca13e82d8ec7c9),
    ("FILE00200002.xxx", 156, 0x8db8df4ca7b729f4),
    ("FILE00300000.xxx", 180, 0xf41a41bbbcf7b11a),
    ("FILE00300001.xxx", 239, 0xc7c36b58aad7dd0a),
    ("Rock.m2", 168, 0xc5aa9fa0b2412a22),
    ("Townhall.wmo", 543, 0x6b0ce2c344dabb64),
    ("Tree_Big.m2", 180, 0xb043df391c61d4d5),
    ("dir_bin", 2208, 0x53f425aed493aaf3),
    ("temp_gameobject_models", 83, 0x8e821831a97ccae1),
];
