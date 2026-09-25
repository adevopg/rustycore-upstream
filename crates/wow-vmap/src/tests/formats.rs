use crate::gameobject_models::{GameObjectModelEntry, read_game_object_models};
use crate::io::{Reader, Writer};
use crate::math::{AABox, Vector3};
use crate::raw::{GroupModelRaw, WorldModelRaw};
use crate::{GroupModel, MeshTriangle, RAW_VMAP_MAGIC, VMAP_MAGIC, WmoLiquid, WorldModel};

fn tiled_liquid() -> WmoLiquid {
    let mut lq = WmoLiquid::new(2, 3, Vector3::new(-5.0, -6.0, 7.0), 13);
    for (i, h) in lq.height_storage_mut().iter_mut().enumerate() {
        *h = i as f32 * 0.5;
    }
    let flags = lq.flags_storage_mut().unwrap();
    flags.copy_from_slice(&[0, 1, 0x0F, 3, 4, 0x08]);
    lq
}

fn cube_group(offset: f32, liquid: Option<WmoLiquid>) -> GroupModel {
    let v = |x: f32, y: f32, z: f32| Vector3::new(x + offset, y, z);
    let vertices = vec![
        v(0.0, 0.0, 0.0),
        v(1.0, 0.0, 0.0),
        v(1.0, 1.0, 0.0),
        v(0.0, 1.0, 0.0),
        v(0.0, 0.0, 1.0),
        v(1.0, 0.0, 1.0),
        v(1.0, 1.0, 1.0),
        v(0.0, 1.0, 1.0),
    ];
    let triangles = vec![
        MeshTriangle::new(0, 1, 2),
        MeshTriangle::new(0, 2, 3),
        MeshTriangle::new(4, 5, 6),
        MeshTriangle::new(4, 6, 7),
        MeshTriangle::new(0, 1, 5),
        MeshTriangle::new(0, 5, 4),
    ];
    let bound = AABox::new(v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0));
    let mut g = GroupModel::new(0x8, 42, bound);
    g.set_mesh_data(vertices, triangles).unwrap();
    g.set_liquid_data(liquid);
    g
}

#[test]
fn wmo_liquid_roundtrip_and_size() {
    for lq in [tiled_liquid(), WmoLiquid::new(0, 0, Vector3::ZERO, 2)] {
        let mut buf = Vec::new();
        lq.write_to(&mut buf);
        assert_eq!(buf.len(), lq.file_size() as usize);
        let back = WmoLiquid::read_from(&mut Reader::new(&buf)).unwrap();
        assert_eq!(back, lq);
    }
}

#[test]
fn wmo_liquid_height_lookup() {
    let lq = tiled_liquid();
    let ts = crate::LIQUID_TILE_SIZE;
    // tile (0,0), dx > dy
    let h = lq
        .liquid_height(Vector3::new(-5.0 + ts * 0.5, -6.0 + ts * 0.25, 0.0))
        .unwrap();
    // heights row = 3 wide: h00=0, h10=0.5, h11=2.0 -> 0 + 0.5*0.5 + 0.25*1.5
    assert!((h - (0.25 + 0.375)).abs() < 1e-5);
    // tile (0,1) is disabled (flag 0x0F)
    assert!(
        lq.liquid_height(Vector3::new(-5.0 + ts * 0.5, -6.0 + ts * 1.5, 0.0))
            .is_none()
    );
    // outside
    assert!(lq.liquid_height(Vector3::new(-6.0, -6.0, 0.0)).is_none());
    let flat = WmoLiquid::new(0, 0, Vector3::ZERO, 1);
    assert_eq!(flat.liquid_height(Vector3::new(1e6, 0.0, 0.0)), Some(0.0));
}

#[test]
fn group_model_roundtrip() {
    let g = cube_group(0.0, Some(tiled_liquid()));
    let mut buf = Vec::new();
    g.write_to(&mut buf);
    let back = GroupModel::read_from(&mut Reader::new(&buf)).unwrap();
    assert_eq!(back, g);
    assert_eq!(back.liquid_type(), 13);
    let (verts, tris, liquid) = back.mesh_data();
    assert_eq!(verts.len(), 8);
    assert_eq!(tris.len(), 6);
    assert!(liquid.is_some());
    assert_eq!(back.mesh_tree().prim_count(), 6);
}

#[test]
fn group_model_layout() {
    let g = cube_group(0.0, None);
    let mut buf = Vec::new();
    g.write_to(&mut buf);
    // bound(24) mogp(4) id(4)
    assert_eq!(&buf[32..36], b"VERT");
    assert_eq!(
        u32::from_le_bytes(buf[36..40].try_into().unwrap()),
        4 + 12 * 8
    );
    let trim = 40 + 4 + 96;
    assert_eq!(&buf[trim..trim + 4], b"TRIM");
    assert_eq!(&buf[buf.len() - 8..buf.len() - 4], b"LIQU");
    assert_eq!(&buf[buf.len() - 4..], &[0, 0, 0, 0]);
}

#[test]
fn group_without_vertices_stops_after_vert_chunk() {
    let mut g = GroupModel::new(1, 2, AABox::new(Vector3::ZERO, Vector3::new(1.0, 1.0, 1.0)));
    g.set_mesh_data(Vec::new(), Vec::new()).unwrap();
    g.set_liquid_data(Some(WmoLiquid::new(0, 0, Vector3::ZERO, 1)));
    let mut buf = Vec::new();
    g.write_to(&mut buf);
    assert_eq!(buf.len(), 24 + 8 + 4 + 4 + 4);
    let back = GroupModel::read_from(&mut Reader::new(&buf)).unwrap();
    // the liquid is not written for groups without vertices (as in C++)
    assert!(back.liquid().is_none());
    assert!(back.vertices().is_empty());
}

#[test]
fn bad_triangle_index_is_rejected() {
    let mut g = GroupModel::new(0, 0, AABox::EMPTY);
    assert!(
        g.set_mesh_data(vec![Vector3::ZERO], vec![MeshTriangle::new(0, 0, 1)])
            .is_err()
    );
}

#[test]
fn world_model_roundtrip() {
    let mut m = WorldModel::new();
    m.set_root_wmo_id(1234);
    m.set_group_models(vec![
        cube_group(0.0, None),
        cube_group(5.0, Some(tiled_liquid())),
    ])
    .unwrap();
    let bytes = m.to_bytes();
    assert_eq!(&bytes[..8], VMAP_MAGIC);
    assert_eq!(&bytes[8..12], b"WMOD");
    assert_eq!(&bytes[20..24], b"GMOD");
    let back = WorldModel::from_bytes(&bytes).unwrap();
    assert_eq!(back, m);
    assert_eq!(back.to_bytes(), bytes);
    assert_eq!(back.group_tree().prim_count(), 2);

    let mut empty = WorldModel::new();
    empty.set_root_wmo_id(7);
    let bytes = empty.to_bytes();
    assert_eq!(bytes.len(), 8 + 4 + 4 + 4);
    let back = WorldModel::from_bytes(&bytes).unwrap();
    assert_eq!(back.root_wmo_id(), 7);
    assert!(back.group_models().is_empty());

    let mut bad = m.to_bytes();
    bad[3] = b'X';
    assert!(WorldModel::from_bytes(&bad).is_err());
}

fn raw_group(liquid_flags: u32) -> GroupModelRaw {
    let liquid = match liquid_flags & 3 {
        0 => None,
        f if f & 1 != 0 => Some(tiled_liquid()),
        _ => None,
    };
    GroupModelRaw {
        mogp_flags: 0x2000,
        group_wmo_id: 9,
        bounds: AABox::new(Vector3::new(-1.0, -2.0, -3.0), Vector3::new(4.0, 5.0, 6.0)),
        liquid_flags,
        branches: vec![3, 6],
        triangles: vec![MeshTriangle::new(0, 1, 2), MeshTriangle::new(2, 1, 0)],
        vertices: vec![
            Vector3::new(1.0, 2.0, 3.0),
            Vector3::new(4.0, 5.0, 6.0),
            Vector3::new(7.0, 8.0, 9.0),
        ],
        liquid,
    }
}

#[test]
fn raw_model_roundtrip() {
    let model = WorldModelRaw {
        n_vectors: 3,
        root_wmo_id: 77,
        groups: vec![raw_group(0), raw_group(1)],
    };
    let bytes = model.to_bytes();
    assert_eq!(&bytes[..8], RAW_VMAP_MAGIC);
    let back = WorldModelRaw::from_bytes(&bytes).unwrap();
    assert_eq!(back, model);
}

#[test]
fn raw_liquid_flag_2_uses_bound_height() {
    let mut g = raw_group(2);
    g.liquid = Some(WmoLiquid::new(0, 0, Vector3::ZERO, 5));
    let model = WorldModelRaw {
        n_vectors: 3,
        root_wmo_id: 1,
        groups: vec![g],
    };
    let back = WorldModelRaw::from_bytes(&model.to_bytes()).unwrap();
    let lq = back.groups[0].liquid.as_ref().unwrap();
    assert_eq!(lq.liquid_type(), 5);
    assert_eq!(lq.height_storage(), &[6.0]);
    assert!(lq.flags_storage().is_none());
}

#[test]
fn raw_rejects_bad_magic_and_truncation() {
    let model = WorldModelRaw {
        n_vectors: 3,
        root_wmo_id: 1,
        groups: vec![raw_group(1)],
    };
    let mut bytes = model.to_bytes();
    bytes[7] = b'X';
    assert!(WorldModelRaw::from_bytes(&bytes).is_err());
    let bytes = model.to_bytes();
    assert!(WorldModelRaw::from_bytes(&bytes[..bytes.len() - 1]).is_err());
}

#[test]
fn gameobject_models_list() {
    let mut buf = Vec::new();
    buf.put_bytes(VMAP_MAGIC);
    let a = GameObjectModelEntry {
        display_id: 10,
        is_wmo: true,
        name: "a.wmo".into(),
        bound_low: Vector3::new(-1.0, -1.0, -1.0),
        bound_high: Vector3::new(1.0, 1.0, 1.0),
    };
    let dup = GameObjectModelEntry {
        name: "dup.m2".into(),
        ..a.clone()
    };
    let nan = GameObjectModelEntry {
        display_id: 11,
        bound_low: Vector3::NAN,
        ..a.clone()
    };
    a.write_to(&mut buf);
    dup.write_to(&mut buf);
    nan.write_to(&mut buf);
    buf.extend_from_slice(&[1, 2]); // trailing garbage (short read = EOF)
    let list = read_game_object_models(&buf).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[&10], a);
    assert!(read_game_object_models(b"VMAP_4.X").is_err());
}
