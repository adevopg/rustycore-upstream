//! Assembler tests on synthetic `Buildings/` directories. The writers below
//! mirror the `vmap4_extractor` output code (`Model::ConvertToVMAPModel`,
//! `Doodad::Extract`, `WMOMapObject::Extract` and the gameobject list in
//! `gameobject_extract.cpp`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::TileAssembler;
use wow_vmap::gameobject_models::read_game_object_models;
use wow_vmap::io::{Reader, Writer};
use wow_vmap::raw::{GroupModelRaw, WorldModelRaw};
use wow_vmap::{
    AABox, LoadResult, MOD_HAS_BOUND, MOD_M2, MOD_PARENT_SPAWN, MeshTriangle, ModelSpawn,
    RAW_VMAP_MAGIC, VMAP_MAGIC, VMapManager, Vector3, WmoLiquid, WorldModel,
};

const TILE: f32 = 533.333_33;

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    fn f(&mut self, range: f32) -> f32 {
        (self.next() % 1_000_000) as f32 / 1_000_000.0 * range
    }
}

fn temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "wow-vmap-asm-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `Model::ConvertToVMAPModel` (`vmap4_extractor/model.cpp)`: one group, zero
/// ids/flags, M2 collision box, index swap and Y/Z vertex fix-up.
fn m2_raw(bounds: AABox, indices: &[u32], vertices: &[Vector3]) -> Vec<u8> {
    let mut indices = indices.to_vec();
    let mut vertices = vertices.to_vec();
    let mut w = Vec::new();
    w.put_bytes(RAW_VMAP_MAGIC);
    let n_vertices = vertices.len() as u32;
    w.put_u32(n_vertices);
    w.put_u32(1); // nofgroups
    w.put_bytes(&[0; 12]); // rootwmoid, flags, groupid
    w.put_aabox(&bounds);
    w.put_u32(0); // liquidflags
    w.put_bytes(b"GRP ");
    let branches = 1u32;
    w.put_i32(4 + 4 * branches as i32);
    w.put_u32(branches);
    let n_indexes = indices.len() as u32;
    w.put_u32(n_indexes);
    w.put_bytes(b"INDX");
    w.put_i32(4 + 2 * n_indexes as i32);
    w.put_u32(n_indexes);
    if n_indexes > 0 {
        for i in 0..indices.len() {
            if (i % 3) == 1 && i + 1 < indices.len() {
                indices.swap(i, i + 1);
            }
        }
        for i in &indices {
            w.put_u32(*i);
        }
    }
    w.put_bytes(b"VERT");
    w.put_i32(4 + 12 * n_vertices as i32);
    w.put_u32(n_vertices);
    for v in &mut vertices {
        let tmp = v.y;
        v.y = -v.z;
        v.z = tmp;
        w.put_vector3(*v);
    }
    w
}

/// `Doodad::Extract` `dir_bin` record (no bound).
#[allow(clippy::too_many_arguments)]
fn dir_bin_m2(
    w: &mut Vec<u8>,
    map_id: u32,
    parent: bool,
    unique_id: u32,
    pos: Vector3,
    rot: Vector3,
    scale: f32,
    name: &str,
) {
    w.put_u32(map_id);
    w.put_u8(MOD_M2 | if parent { MOD_PARENT_SPAWN } else { 0 });
    w.put_u8(0); // nameSet
    w.put_u32(unique_id);
    w.put_vector3(pos);
    w.put_vector3(rot);
    w.put_f32(scale);
    w.put_u32(name.len() as u32);
    w.put_bytes(name.as_bytes());
}

/// `WMOMapObject::Extract` `dir_bin` record (with bound).
#[allow(clippy::too_many_arguments)]
fn dir_bin_wmo(
    w: &mut Vec<u8>,
    map_id: u32,
    parent: bool,
    name_set: u8,
    unique_id: u32,
    pos: Vector3,
    rot: Vector3,
    scale: f32,
    bounds: AABox,
    name: &str,
) {
    w.put_u32(map_id);
    w.put_u8(MOD_HAS_BOUND | if parent { MOD_PARENT_SPAWN } else { 0 });
    w.put_u8(name_set);
    w.put_u32(unique_id);
    w.put_vector3(pos);
    w.put_vector3(rot);
    w.put_f32(scale);
    w.put_aabox(&bounds);
    w.put_u32(name.len() as u32);
    w.put_bytes(name.as_bytes());
}

/// `temp_gameobject_models` (`gameobject_extract.cpp`).
fn gameobject_list(entries: &[(u32, u8, &str)]) -> Vec<u8> {
    let mut w = Vec::new();
    w.put_bytes(RAW_VMAP_MAGIC);
    for (display_id, is_wmo, name) in entries {
        w.put_u32(*display_id);
        w.put_u8(*is_wmo);
        w.put_u32(name.len() as u32);
        w.put_bytes(name.as_bytes());
    }
    w
}

fn random_mesh(
    rng: &mut Rng,
    n_vertices: usize,
    n_triangles: usize,
    size: f32,
) -> (Vec<Vector3>, Vec<MeshTriangle>) {
    let vertices: Vec<Vector3> = (0..n_vertices)
        .map(|_| {
            Vector3::new(
                rng.f(size) - size / 2.0,
                rng.f(size) - size / 2.0,
                rng.f(size / 2.0),
            )
        })
        .collect();
    let n = n_vertices as u32;
    let triangles = (0..n_triangles)
        .map(|_| MeshTriangle::new(rng.next() % n, rng.next() % n, rng.next() % n))
        .collect();
    (vertices, triangles)
}

fn bounds_of(v: &[Vector3]) -> AABox {
    let mut b = AABox::EMPTY;
    for p in v {
        b.merge_point(*p);
    }
    b
}

fn random_wmo(rng: &mut Rng, root_id: u32, groups: usize) -> WorldModelRaw {
    let mut out = WorldModelRaw {
        n_vectors: 0,
        root_wmo_id: root_id,
        groups: Vec::new(),
    };
    for g in 0..groups {
        let nv = 3 + (rng.next() % 200) as usize;
        let nt = (rng.next() % 300) as usize;
        let (vertices, triangles) = random_mesh(rng, nv, nt, 60.0);
        out.n_vectors += nv as u32;
        let liquid_flags = rng.next() % 4;
        let bounds = bounds_of(&vertices);
        let liquid = match liquid_flags {
            1 | 3 => {
                let (tx, ty) = (1 + rng.next() % 6, 1 + rng.next() % 6);
                let mut lq = WmoLiquid::new(
                    tx,
                    ty,
                    Vector3::new(rng.f(10.0), rng.f(10.0), rng.f(5.0)),
                    rng.next() % 20,
                );
                for h in lq.height_storage_mut() {
                    *h = rng.f(3.0);
                }
                for f in lq.flags_storage_mut().unwrap() {
                    *f = (rng.next() % 16) as u8;
                }
                Some(lq)
            }
            2 => Some(WmoLiquid::new(0, 0, Vector3::ZERO, rng.next() % 20)),
            _ => None,
        };
        out.groups.push(GroupModelRaw {
            mogp_flags: rng.next(),
            group_wmo_id: g as u32 * 10 + root_id,
            bounds,
            liquid_flags,
            branches: vec![nt as u32 * 3],
            triangles,
            vertices,
            liquid,
        });
    }
    out
}

/// Writes a random but valid Buildings directory; returns the expected
/// number of maps.
fn write_random_buildings(src: &Path, seed: u32) -> usize {
    let mut rng = Rng(seed);
    let mut dir_bin = Vec::new();
    let n_wmo = 6;
    let n_m2 = 8;
    for i in 0..n_wmo {
        let groups = 1 + (rng.next() % 4) as usize;
        std::fs::write(
            src.join(format!("wmo_{i}.wmo")),
            random_wmo(&mut rng, 1000 + i, groups).to_bytes(),
        )
        .unwrap();
    }
    for i in 0..n_m2 {
        let nv = 3 + (rng.next() % 60) as usize;
        let nt = 1 + (rng.next() % 80) as usize;
        let (vertices, triangles) = random_mesh(&mut rng, nv, nt, 8.0);
        let indices: Vec<u32> = triangles
            .iter()
            .flat_map(|t| [t.idx0, t.idx1, t.idx2])
            .collect();
        let b = bounds_of(&vertices);
        std::fs::write(
            src.join(format!("m2_{i}.m2")),
            m2_raw(b, &indices, &vertices),
        )
        .unwrap();
    }
    let mut unique = 1u32;
    for map_id in [0u32, 1, 571] {
        for _ in 0..40 {
            let x = 16.0 * TILE + rng.f(32.0 * TILE);
            let y = 16.0 * TILE + rng.f(32.0 * TILE);
            let pos = Vector3::new(x, y, rng.f(100.0));
            let rot = Vector3::new(rng.f(360.0) - 180.0, rng.f(360.0), rng.f(20.0) - 10.0);
            let parent = rng.next().is_multiple_of(5);
            unique += 1 + rng.next() % 3;
            if rng.next().is_multiple_of(2) {
                let ext = Vector3::new(10.0 + rng.f(1200.0), 10.0 + rng.f(1200.0), rng.f(50.0));
                let bounds = AABox::new(pos - ext * 0.5, pos + ext * 0.5);
                let name = format!("wmo_{}.wmo", rng.next() % n_wmo);
                let scale = if rng.next().is_multiple_of(3) {
                    0.5 + rng.f(2.0)
                } else {
                    1.0
                };
                dir_bin_wmo(
                    &mut dir_bin,
                    map_id,
                    parent,
                    (rng.next() % 3) as u8,
                    unique,
                    pos,
                    rot,
                    scale,
                    bounds,
                    &name,
                );
                // the same spawn is listed again by neighbouring ADTs
                if rng.next().is_multiple_of(3) {
                    dir_bin_wmo(
                        &mut dir_bin,
                        map_id,
                        parent,
                        0,
                        unique,
                        pos,
                        rot,
                        scale,
                        bounds,
                        &name,
                    );
                }
            } else {
                // m2 index n_m2 does not exist: its bound cannot be computed
                let name = format!("m2_{}.m2", rng.next() % (n_m2 + 1));
                dir_bin_m2(
                    &mut dir_bin,
                    map_id,
                    parent,
                    unique,
                    pos,
                    rot,
                    0.25 + rng.f(3.0),
                    &name,
                );
            }
        }
    }
    std::fs::write(src.join("dir_bin"), dir_bin).unwrap();

    // gameobject models: one normal, one missing file, one without vertices
    std::fs::write(
        src.join("go_empty.m2"),
        m2_raw(AABox::new(Vector3::ZERO, Vector3::ZERO), &[], &[]),
    )
    .unwrap();
    std::fs::write(
        src.join("temp_gameobject_models"),
        gameobject_list(&[
            (1, 0, "m2_0.m2"),
            (2, 1, "wmo_1.wmo"),
            (3, 0, "missing.m2"),
            (4, 0, "go_empty.m2"),
            (5, 0, "go_only.m2"),
        ]),
    )
    .unwrap();
    std::fs::write(
        src.join("go_only.m2"),
        m2_raw(
            AABox::new(Vector3::ZERO, Vector3::new(1.0, 1.0, 1.0)),
            &[0, 1, 2],
            &[
                Vector3::ZERO,
                Vector3::new(1.0, 0.0, 0.0),
                Vector3::new(0.0, 0.0, 1.0),
            ],
        ),
    )
    .unwrap();
    3
}

fn read_dir(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

fn run_rust(src: &Path, dest: &Path) -> bool {
    let mut ta = TileAssembler::new(src.to_str().unwrap(), dest.to_str().unwrap()).unwrap();
    ta.convert_world2()
}

#[test]
fn assembles_small_hand_made_buildings() {
    let root = temp_dir("small");
    let src = root.join("Buildings");
    let dest = root.join("vmaps");
    std::fs::create_dir_all(&src).unwrap();

    // WMO with two groups, one with a tiled liquid
    let mut wmo = WorldModelRaw {
        n_vectors: 6,
        root_wmo_id: 55,
        groups: Vec::new(),
    };
    let tri_vertices = vec![
        Vector3::ZERO,
        Vector3::new(4.0, 0.0, 0.0),
        Vector3::new(0.0, 4.0, 0.0),
    ];
    let mut lq = WmoLiquid::new(1, 1, Vector3::new(0.0, 0.0, 1.0), 2);
    lq.height_storage_mut()
        .copy_from_slice(&[1.0, 1.5, 2.0, 2.5]);
    for (i, liquid) in [None, Some(lq.clone())].into_iter().enumerate() {
        wmo.groups.push(GroupModelRaw {
            mogp_flags: 0x8,
            group_wmo_id: i as u32,
            bounds: AABox::new(Vector3::ZERO, Vector3::new(4.0, 4.0, 0.0)),
            liquid_flags: u32::from(liquid.is_some()),
            branches: vec![3],
            triangles: vec![MeshTriangle::new(0, 1, 2)],
            vertices: tri_vertices.clone(),
            liquid,
        });
    }
    std::fs::write(src.join("castle.wmo"), wmo.to_bytes()).unwrap();
    std::fs::write(
        src.join("tree.m2"),
        m2_raw(
            AABox::new(Vector3::new(-1.0, -1.0, 0.0), Vector3::new(1.0, 1.0, 2.0)),
            &[0, 1, 2],
            &tri_vertices,
        ),
    )
    .unwrap();

    let mut dir_bin = Vec::new();
    // spans tiles x 32..33, y 32 (bounds 17066..17333 / 533.33 = 32.0..32.5)
    let wmo_pos = Vector3::new(17_200.0, 17_100.0, 0.0);
    let wmo_bounds = AABox::new(
        Vector3::new(17_000.0, 17_070.0, -5.0),
        Vector3::new(17_700.0, 17_130.0, 5.0),
    );
    dir_bin_wmo(
        &mut dir_bin,
        1,
        false,
        0,
        10,
        wmo_pos,
        Vector3::ZERO,
        1.0,
        wmo_bounds,
        "castle.wmo",
    );
    dir_bin_m2(
        &mut dir_bin,
        1,
        false,
        20,
        Vector3::new(17_100.0, 17_100.0, 3.0),
        Vector3::new(0.0, 90.0, 0.0),
        2.0,
        "tree.m2",
    );
    dir_bin_m2(
        &mut dir_bin,
        1,
        false,
        30,
        Vector3::new(17_100.0, 17_100.0, 3.0),
        Vector3::ZERO,
        1.0,
        "no_such.m2",
    );
    // parent spawn on map 2 in the same tile, plus a normal spawn
    dir_bin_wmo(
        &mut dir_bin,
        2,
        true,
        0,
        40,
        wmo_pos,
        Vector3::ZERO,
        1.0,
        wmo_bounds,
        "castle.wmo",
    );
    dir_bin_wmo(
        &mut dir_bin,
        2,
        false,
        1,
        41,
        wmo_pos,
        Vector3::ZERO,
        1.0,
        wmo_bounds,
        "castle.wmo",
    );
    std::fs::write(src.join("dir_bin"), dir_bin).unwrap();
    std::fs::write(
        src.join("temp_gameobject_models"),
        gameobject_list(&[(7, 0, "tree.m2"), (8, 1, "gone.wmo")]),
    )
    .unwrap();

    assert!(run_rust(&src, &dest));
    let files = read_dir(&dest);
    let names: Vec<&str> = files.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "0001.vmtree",
            "0001_32_31.vmtile",
            "0001_32_32.vmtile",
            "0001_32_33.vmtile",
            "0002.vmtree",
            "0002_32_31.vmtile",
            "0002_32_32.vmtile",
            "0002_32_33.vmtile",
            "GameObjectModels.dtree",
            "castle.wmo.vmo",
            "tree.m2.vmo",
        ]
    );

    // vmtree: magic, NODE + BIH, SIDX with the spawns that got bounds
    let tree = &files["0001.vmtree"];
    assert_eq!(&tree[..8], VMAP_MAGIC);
    assert_eq!(&tree[8..12], b"NODE");
    let mut r = Reader::new(&tree[12..]);
    let bih = wow_vmap::Bih::read_from(&mut r).unwrap();
    assert_eq!(bih.prim_count(), 2);
    assert!(r.chunk(b"SIDX"));
    assert_eq!(r.u32(), Some(2));
    assert_eq!((r.u32(), r.u32()), (Some(10), Some(20)));
    assert!(r.is_at_end());

    // M2 got a computed bound: tree.m2 box (-1,-1,0)-(1,1,2), scale 2, yaw 90
    let tile = &files["0001_32_32.vmtile"];
    let mut r = Reader::new(tile);
    assert!(r.chunk(VMAP_MAGIC));
    assert_eq!(r.u32(), Some(2));
    let s10 = ModelSpawn::read_from(&mut r).unwrap().unwrap();
    let s20 = ModelSpawn::read_from(&mut r).unwrap().unwrap();
    assert_eq!((s10.id, s20.id), (10, 20));
    assert_eq!(s20.flags, MOD_M2 | MOD_HAS_BOUND);
    let lo = s20.bound.low() - Vector3::new(17_100.0, 17_100.0, 3.0);
    let hi = s20.bound.high() - Vector3::new(17_100.0, 17_100.0, 3.0);
    assert!(
        (lo - Vector3::new(-2.0, -2.0, 0.0)).magnitude() < 1e-2,
        "{lo:?}"
    );
    assert!(
        (hi - Vector3::new(2.0, 2.0, 4.0)).magnitude() < 1e-2,
        "{hi:?}"
    );

    // map 2: tile holds the normal spawn first, then the parent spawn
    let tile = &files["0002_32_32.vmtile"];
    let mut r = Reader::new(tile);
    assert!(r.chunk(VMAP_MAGIC));
    assert_eq!(r.u32(), Some(2));
    assert_eq!(ModelSpawn::read_from(&mut r).unwrap().unwrap().id, 41);
    let parent = ModelSpawn::read_from(&mut r).unwrap().unwrap();
    assert_eq!(
        (parent.id, parent.flags),
        (40, MOD_HAS_BOUND | MOD_PARENT_SPAWN)
    );

    // vmo files parse back
    let castle = WorldModel::from_bytes(&files["castle.wmo.vmo"]).unwrap();
    assert_eq!(castle.root_wmo_id(), 55);
    assert_eq!(castle.group_models().len(), 2);
    assert_eq!(castle.group_models()[1].liquid(), Some(&lq));
    let tree_m2 = WorldModel::from_bytes(&files["tree.m2.vmo"]).unwrap();
    let g = &tree_m2.group_models()[0];
    // extractor swapped indices 1/2 and fixed Y/Z
    assert_eq!(g.triangles(), &[MeshTriangle::new(0, 2, 1)]);
    assert_eq!(g.vertices()[2], Vector3::new(0.0, 0.0, 4.0));

    // gameobject list: only existing models with vertices
    let go = read_game_object_models(&files["GameObjectModels.dtree"]).unwrap();
    assert_eq!(go.len(), 1);
    assert_eq!(go[&7].name, "tree.m2");
    assert_eq!(go[&7].bound_high, Vector3::new(4.0, 0.0, 4.0));

    // runtime loading path used by the mmaps generator
    let mut vm = VMapManager::new();
    assert_eq!(vm.load_map(&dest, 1, 32, 32), LoadResult::Success);
    let instances = vm.map_tree(1).unwrap().model_instances();
    assert_eq!(instances.len(), 2);
    assert!(instances.iter().all(|i| i.world_model().is_some()));
    assert_eq!(vm.load_map(&dest, 2, 32, 32), LoadResult::Success);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn missing_dir_bin_fails_and_missing_model_is_an_error() {
    let root = temp_dir("fail");
    let src = root.join("Buildings");
    std::fs::create_dir_all(&src).unwrap();
    assert!(!run_rust(&src, &root.join("vmaps")));

    let mut dir_bin = Vec::new();
    let b = AABox::new(
        Vector3::new(100.0, 100.0, 0.0),
        Vector3::new(110.0, 110.0, 1.0),
    );
    dir_bin_wmo(
        &mut dir_bin,
        0,
        false,
        0,
        1,
        Vector3::new(105.0, 105.0, 0.0),
        Vector3::ZERO,
        1.0,
        b,
        "absent.wmo",
    );
    std::fs::write(src.join("dir_bin"), dir_bin).unwrap();
    assert!(!run_rust(&src, &root.join("vmaps2")));
    // tree and tile are still written before model conversion fails
    assert!(root.join("vmaps2/0000.vmtree").exists());
    assert!(root.join("vmaps2/0000_00_00.vmtile").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn random_buildings_assemble_and_load() {
    let root = temp_dir("random");
    let src = root.join("Buildings");
    std::fs::create_dir_all(&src).unwrap();
    write_random_buildings(&src, 12345);
    let dest = root.join("vmaps");
    assert!(run_rust(&src, &dest));
    let files = read_dir(&dest);
    for (name, bytes) in &files {
        if Path::new(name).extension().is_some_and(|e| e == "vmo") {
            let m = WorldModel::from_bytes(bytes).unwrap();
            assert_eq!(&m.to_bytes(), bytes, "{name} re-serialises identically");
        }
    }
    assert!(files.contains_key("go_only.m2.vmo"));
    assert!(files.contains_key("0571.vmtree"));
    let mut vm = VMapManager::new();
    let tiles: Vec<(u32, u32)> = files
        .keys()
        .filter_map(|n| {
            n.strip_prefix("0000_")
                .and_then(|r| r.strip_suffix(".vmtile"))
        })
        .map(|r| {
            let (y, x) = r.split_once('_').unwrap();
            (x.parse().unwrap(), y.parse().unwrap())
        })
        .collect();
    assert!(!tiles.is_empty());
    for (x, y) in tiles {
        assert_eq!(
            vm.load_map(&dest, 0, x, y),
            LoadResult::Success,
            "tile {x} {y}"
        );
    }
    let _ = std::fs::remove_dir_all(root);
}

/// Byte-for-byte comparison with the C++ `TileAssembler` when
/// `VMAP_CPP_ASSEMBLER` points to a reference build taking `<src> <dest>`.
#[test]
#[ignore = "needs the C++ reference assembler (VMAP_CPP_ASSEMBLER)"]
fn random_buildings_match_cpp_reference() {
    let cpp = std::env::var("VMAP_CPP_ASSEMBLER").expect("VMAP_CPP_ASSEMBLER");
    for seed in [1u32, 7, 12345, 99_999, 424_242] {
        let root = temp_dir("cpp");
        let src = root.join("Buildings");
        std::fs::create_dir_all(&src).unwrap();
        write_random_buildings(&src, seed);
        let rust_out = root.join("rust");
        let cpp_out = root.join("cpp");
        let rust_ok = run_rust(&src, &rust_out);
        let status = std::process::Command::new(&cpp)
            .arg(&src)
            .arg(&cpp_out)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(rust_ok, status.success(), "seed {seed}: exit status");
        let a = read_dir(&rust_out);
        let b = read_dir(&cpp_out);
        assert_eq!(
            a.keys().collect::<Vec<_>>(),
            b.keys().collect::<Vec<_>>(),
            "seed {seed}: file set"
        );
        for (name, bytes) in &a {
            assert!(bytes == &b[name], "seed {seed}: {name} differs");
        }
        println!("seed {seed}: {} files identical", a.len());
        let _ = std::fs::remove_dir_all(root);
    }
}
