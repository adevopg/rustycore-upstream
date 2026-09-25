use super::off_mesh::{fgets_chunks, parse_off_mesh_connections};
use super::*;
use crate::map_defines::{MmapTileHeader, nav_area, nav_flag};
use crate::test_data::{build_fixture, fixture_data, fresh_dir};
use std::collections::HashMap;

fn opts() -> BuilderOptions {
    BuilderOptions {
        skip_junk_maps: true,
        mapid: -1,
        threads: 2,
        ..BuilderOptions::default()
    }
}

#[test]
fn tile_config_matches_cpp_constants() {
    let small = TileConfig::new(false);
    assert_eq!(small.base_unit_dim, 0.266_666_6);
    assert_eq!(
        (
            small.vertex_per_map,
            small.vertex_per_tile,
            small.tiles_per_map
        ),
        (2000, 80, 25)
    );
    let big = TileConfig::new(true);
    assert_eq!(big.base_unit_dim, 0.533_333_3);
    assert_eq!(
        (big.vertex_per_map, big.vertex_per_tile, big.tiles_per_map),
        (1000, 40, 25)
    );
}

#[test]
fn map_specific_config() {
    let dir = fresh_dir("mb-config");
    build_fixture(&dir);
    let mb = MapBuilder::new(opts(), &dir, fixture_data());
    let tc = TileConfig::new(false);
    let c = mb
        .shared
        .get_map_specific_config(1, &[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0], &tc);
    assert_eq!(c.bmin, [1.0, 2.0, 3.0]);
    assert_eq!(c.max_verts_per_poly, 6);
    assert_eq!((c.cs, c.ch), (0.266_666_6, 0.266_666_6));
    assert_eq!(
        (c.walkable_slope_angle, c.walkable_slope_angle_not_steep),
        (55.0, 55.0)
    );
    assert_eq!(
        (
            c.tile_size,
            c.walkable_radius,
            c.border_size,
            c.max_edge_len
        ),
        (80, 2, 5, 81)
    );
    assert_eq!((c.walkable_height, c.walkable_climb), (6, 6));
    assert_eq!((c.min_region_area, c.merge_region_area), (3600, 2500));
    assert_eq!(c.max_simplification_error, 1.8);
    assert_eq!(c.detail_sample_dist, 0.266_666_6 * 16.0);
    assert_eq!(c.detail_sample_max_error, 0.266_666_6);
    assert_eq!(
        mb.shared
            .get_map_specific_config(562, &[0.0; 3], &[0.0; 3], &tc)
            .walkable_radius,
        0
    );
    assert_eq!(
        mb.shared
            .get_map_specific_config(48, &[0.0; 3], &[0.0; 3], &tc)
            .ch,
        0.266_666_6 * 2.0
    );

    let mut o = opts();
    o.big_base_unit = true;
    o.max_walkable_angle = Some(70.0);
    o.max_walkable_angle_not_steep = Some(50.0);
    let mb = MapBuilder::new(o, &dir, fixture_data());
    let tc = TileConfig::new(true);
    let c = mb
        .shared
        .get_map_specific_config(1, &[0.0; 3], &[0.0; 3], &tc);
    assert_eq!(
        (c.walkable_slope_angle, c.walkable_slope_angle_not_steep),
        (70.0, 50.0)
    );
    assert_eq!(
        (
            c.tile_size,
            c.walkable_radius,
            c.border_size,
            c.max_edge_len
        ),
        (40, 1, 4, 41)
    );
    assert_eq!((c.walkable_height, c.walkable_climb), (3, 3));
}

#[test]
fn tile_bounds() {
    let (bmin, bmax) = Shared::get_tile_bounds(32, 31, None);
    assert_eq!(bmin, [-GRID_SIZE, f32::MIN_POSITIVE, 0.0]);
    assert_eq!(bmax, [0.0, f32::MAX, GRID_SIZE]);
    let (bmin, bmax) = Shared::get_tile_bounds(30, 34, Some(&[1.0, -5.0, 2.0, 3.0, 7.0, -1.0]));
    assert_eq!(bmin, [GRID_SIZE, -5.0, -2.0 * GRID_SIZE - GRID_SIZE]);
    assert_eq!(bmax, [2.0 * GRID_SIZE, 7.0, -2.0 * GRID_SIZE]);
}

#[test]
fn discovery_nav_mesh_params_and_skip_rules() {
    let dir = fresh_dir("mb-discover");
    build_fixture(&dir);
    std::fs::create_dir_all(dir.join("mmaps")).unwrap();
    let mut mb = MapBuilder::new(opts(), &dir, fixture_data());
    let list: HashMap<u32, Vec<(u32, u32)>> = mb
        .tile_list()
        .iter()
        .map(|(id, t)| (*id, t.iter().map(|&i| unpack_tile_id(i)).collect()))
        .collect();
    assert_eq!(list[&1], vec![(31, 31), (32, 31), (32, 32), (33, 31)]);
    assert!(list[&2].is_empty());
    assert_eq!(mb.shared.total_tiles.load(Ordering::SeqCst), 4);

    // child map uses the parent's tile set for dtNavMeshParams
    let nav = mb.build_nav_mesh(2).expect("navmesh");
    let p = nav.params();
    assert_eq!(
        p.orig,
        [
            (32.0 - 33.0) * GRID_SIZE - GRID_SIZE,
            f32::MIN_POSITIVE,
            0.0 - GRID_SIZE
        ]
    );
    assert_eq!(
        (p.tile_width, p.tile_height, p.max_tiles, p.max_polys),
        (GRID_SIZE, GRID_SIZE, 4, i32::MIN)
    );
    assert_eq!(
        std::fs::read(dir.join("mmaps/0002.mmap")).unwrap(),
        p.to_bytes()
    );

    // skip rules
    assert!(!mb.should_skip_map(1));
    let mut data = (*fixture_data()).clone();
    data.map_store.get_mut(&1).unwrap().flags = 2;
    data.map_store.get_mut(&2).unwrap().instance_type = 3;
    let mut o = opts();
    o.skip_battlegrounds = true;
    let mb2 = MapBuilder::new(o.clone(), &dir, Arc::new(data.clone()));
    assert!(mb2.should_skip_map(1)); // dev map
    assert!(mb2.should_skip_map(2)); // battleground
    assert!(mb2.should_skip_map(3)); // transport
    assert!(!mb2.should_skip_map(4));
    o.skip_junk_maps = false;
    o.skip_battlegrounds = false;
    o.skip_continents = true;
    let mb3 = MapBuilder::new(o.clone(), &dir, Arc::new(data.clone()));
    assert!(mb3.should_skip_map(1)); // Kalimdor is a continent
    assert!(!mb3.should_skip_map(2) && !mb3.should_skip_map(3));
    assert!(mb3.should_skip_map(571));
    o.mapid = 2;
    let mb4 = MapBuilder::new(o, &dir, Arc::new(data));
    assert!(mb4.should_skip_map(1) && !mb4.should_skip_map(2) && mb4.should_skip_map(571));
    assert!(
        [0, 1, 530, 571, 870, 1116, 1220, 1642, 1643, 2222]
            .iter()
            .all(|&m| is_continent_map(m))
    );
    assert!(!is_continent_map(2));
}

#[test]
fn off_mesh_file_parsing() {
    let text = b"# comment\n\
        1 32,31 (1.5 2.5 3.5) (4 5 6) 1.25 9 4\n\
        2 3,4 (1 2 3) (4 5 6) 2.0\n\
        5 6,7 (1 2 3) (4 5 6) 0.5 8\n\
        5 6, 7 (1 2 3) (4 5 6) 0.5\n\
        530 30,20 (1e1 -2 .5) (4 5 6) 3\n\
        1 2,3 (1 2 3) (4 5 6)\n\
        -1 2,3 (1 2 3) (4 5 6) 1 300 70000\n";
    let conns = parse_off_mesh_connections(text);
    assert_eq!(conns.len(), 6);
    assert_eq!(
        conns[0],
        OffMeshData {
            map_id: 1,
            tile_x: 32,
            tile_y: 31,
            from: [1.5, 2.5, 3.5],
            to: [4.0, 5.0, 6.0],
            bidirectional: true,
            radius: 1.25,
            area_id: 9,
            flags: 4,
        }
    );
    assert_eq!(
        (conns[1].area_id, conns[1].flags),
        (nav_area::GROUND, nav_flag::GROUND)
    );
    assert_eq!((conns[2].area_id, conns[2].flags), (8, nav_flag::GROUND));
    // "%u,%u" skips whitespace before the second number
    assert_eq!((conns[3].tile_x, conns[3].tile_y), (6, 7));
    assert_eq!(conns[4].from, [10.0, -2.0, 0.5]);
    // strtoul semantics: "-1" wraps, %hhu/%hu truncate
    assert_eq!(
        (conns[5].map_id, conns[5].area_id, conns[5].flags),
        (u32::MAX, 44, 4464)
    );

    // fgets(512) splits long lines
    let long = vec![b'x'; 1200];
    let chunks = fgets_chunks(&long);
    assert_eq!(
        chunks.iter().map(|c| c.len()).collect::<Vec<_>>(),
        vec![511, 511, 178]
    );
    assert!(parse_off_mesh_connections_file(None).is_empty());
    assert!(parse_off_mesh_connections_file(Some("/nonexistent/offmesh.txt")).is_empty());
}

#[test]
fn helpers() {
    assert_eq!(pack_tile_id(32, 31), (32 << 16) | 31);
    assert_eq!(unpack_tile_id((32 << 16) | 0x1FF), (32, 0xFF));
    assert_eq!(percentage_done(0, 5), 0);
    assert_eq!(percentage_done(8, 2), 25);
}

#[test]
fn should_skip_tile_checks_existing_header() {
    let dir = fresh_dir("mb-skip");
    build_fixture(&dir);
    std::fs::create_dir_all(dir.join("mmaps")).unwrap();
    let mb = MapBuilder::new(opts(), &dir, fixture_data());
    let tb = TileBuilder::new(Arc::clone(&mb.shared));
    assert!(!tb.should_skip_tile(1, 32, 31));
    let path = dir.join("mmaps/00013132.mmtile");
    std::fs::write(&path, MmapTileHeader::default().to_bytes()).unwrap();
    assert!(tb.should_skip_tile(1, 32, 31));
    let old = MmapTileHeader {
        mmap_version: 14,
        ..MmapTileHeader::default()
    };
    std::fs::write(&path, old.to_bytes()).unwrap();
    assert!(!tb.should_skip_tile(1, 32, 31));
    std::fs::write(&path, [1u8, 2, 3]).unwrap();
    assert!(!tb.should_skip_tile(1, 32, 31));
}
