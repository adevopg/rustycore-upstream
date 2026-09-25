use std::collections::HashMap;
use std::path::Path;

use super::temp_dir;
use crate::io::Writer;
use crate::math::{AABox, Vector3};
use crate::{
    Bih, GroupModel, LoadResult, MOD_HAS_BOUND, MOD_M2, MeshTriangle, ModelSpawn, VMAP_MAGIC,
    VMapManager, WorldModel, map_file_name, tile_file_name,
};

fn spawn(id: u32, name: &str, flags: u8, x: f32) -> ModelSpawn {
    ModelSpawn {
        flags: flags | MOD_HAS_BOUND,
        adt_id: 0,
        id,
        pos: Vector3::new(x, 10.0, 0.0),
        rot: Vector3::new(0.0, 90.0, 0.0),
        scale: 2.0,
        bound: AABox::new(
            Vector3::new(x - 1.0, 9.0, -1.0),
            Vector3::new(x + 1.0, 11.0, 1.0),
        ),
        name: name.into(),
    }
}

fn write_model(dir: &Path, name: &str) {
    let mut g = GroupModel::new(0, 1, AABox::new(Vector3::ZERO, Vector3::new(1.0, 1.0, 0.0)));
    g.set_mesh_data(
        vec![
            Vector3::ZERO,
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
        ],
        vec![MeshTriangle::new(0, 1, 2)],
    )
    .unwrap();
    let mut m = WorldModel::new();
    m.set_group_models(vec![g]).unwrap();
    m.write_file(dir.join(format!("{name}.vmo"))).unwrap();
}

fn write_tree(dir: &Path, map_id: u32, spawns: &[ModelSpawn]) {
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
    std::fs::write(dir.join(map_file_name(map_id)), buf).unwrap();
}

fn write_tile(dir: &Path, map_id: u32, x: u32, y: u32, spawns: &[&ModelSpawn]) {
    let mut buf = Vec::new();
    buf.put_bytes(VMAP_MAGIC);
    buf.put_u32(spawns.len() as u32);
    for s in spawns {
        s.write_to(&mut buf);
    }
    std::fs::write(dir.join(tile_file_name(map_id, x, y)), buf).unwrap();
}

#[test]
fn load_and_unload_tiles() {
    let dir = temp_dir("maptree");
    let a = spawn(100, "a.wmo", 0, 5.0);
    let b = spawn(200, "b.m2", MOD_M2, 50.0);
    let c = spawn(300, "a.wmo", 0, 90.0);
    write_tree(&dir, 1, &[a.clone(), b.clone(), c.clone()]);
    write_tile(&dir, 1, 3, 4, &[&a, &b]);
    write_tile(&dir, 1, 3, 5, &[&b, &c]);
    write_model(&dir, "a.wmo");
    write_model(&dir, "b.m2");

    let mut vm = VMapManager::new();
    assert_eq!(vm.load_map(&dir, 1, 3, 4), LoadResult::Success);
    let tree = vm.map_tree(1).unwrap();
    assert_eq!(tree.num_loaded_tiles(), 1);
    let instances = tree.model_instances();
    assert_eq!(instances.len(), 3);
    let loaded: Vec<u32> = instances
        .iter()
        .filter(|i| i.world_model().is_some())
        .map(|i| i.id)
        .collect();
    assert_eq!(loaded.len(), 2);
    assert!(loaded.contains(&100) && loaded.contains(&200));
    let m2 = instances.iter().find(|i| i.id == 200).unwrap();
    assert_eq!(m2.flags & MOD_M2, MOD_M2);
    assert_eq!(m2.scale, 2.0);
    let wm = m2.world_model().unwrap();
    assert_eq!(wm.name(), "b.m2");
    assert_eq!(wm.flags, u32::from(MOD_M2 | MOD_HAS_BOUND));
    assert_eq!(wm.group_models()[0].triangles().len(), 1);
    // rot.y is the yaw (Z rotation): 90 degrees maps X to Y, the inverse Y to X
    let r = *m2.inv_rot() * Vector3::new(0.0, 1.0, 0.0);
    assert!(
        (r - Vector3::new(1.0, 0.0, 0.0)).magnitude() < 1e-5,
        "{r:?}"
    );

    assert_eq!(vm.load_map(&dir, 1, 3, 5), LoadResult::Success);
    assert_eq!(vm.registry().loaded_model_count(), 2);
    let tree = vm.map_tree(1).unwrap();
    assert_eq!(
        tree.model_instances()
            .iter()
            .filter(|i| i.world_model().is_some())
            .count(),
        3
    );

    // missing tile file is not an error for the tree, but is reported
    assert_eq!(vm.load_map(&dir, 1, 9, 9), LoadResult::FileNotFound);

    vm.unload_map_tile(1, 3, 4);
    let tree = vm.map_tree(1).unwrap();
    let loaded: Vec<u32> = tree
        .model_instances()
        .iter()
        .filter(|i| i.world_model().is_some())
        .map(|i| i.id)
        .collect();
    assert_eq!(loaded.len(), 2);
    assert!(!loaded.contains(&100));
    vm.unload_map_tile(1, 9, 9);
    vm.unload_map_tile(1, 3, 5);
    assert!(vm.map_tree(1).is_none());
    assert_eq!(vm.registry().loaded_model_count(), 0);

    assert_eq!(vm.load_map(&dir, 2, 0, 0), LoadResult::FileNotFound);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn parent_map_tile_fallback_and_version_mismatch() {
    let dir = temp_dir("parent");
    let a = spawn(1, "a.wmo", 0, 5.0);
    write_tree(&dir, 0, std::slice::from_ref(&a));
    write_tree(&dir, 10, std::slice::from_ref(&a));
    write_tile(&dir, 0, 1, 1, &[&a]);
    write_model(&dir, "a.wmo");

    let mut vm = VMapManager::new();
    let mut parents = HashMap::new();
    parents.insert(0u32, vec![10u32]);
    vm.initialize_thread_unsafe(&parents);
    assert_eq!(vm.registry().parent_map_id(10), 0);
    // child map 10 has no own tile: falls back to map 0's tile
    assert_eq!(vm.load_map(&dir, 10, 1, 1), LoadResult::Success);
    assert!(
        vm.map_tree(10).unwrap().model_instances()[0]
            .world_model()
            .is_some()
    );

    let mut bytes = std::fs::read(dir.join(map_file_name(0))).unwrap();
    bytes[0] = b'X';
    std::fs::write(dir.join(map_file_name(3)), bytes).unwrap();
    assert_eq!(vm.load_map(&dir, 3, 1, 1), LoadResult::VersionMismatch);
    let _ = std::fs::remove_dir_all(dir);
}
