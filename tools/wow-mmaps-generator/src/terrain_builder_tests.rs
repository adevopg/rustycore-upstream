use super::*;
use crate::map_defines::nav_area;
use crate::test_data::{
    Heights, Liquid, build_fixture, fixture_data, fresh_dir, terrain_height, write_map_file,
};

fn builder(dir: &Path, skip_liquid: bool) -> TerrainBuilder {
    TerrainBuilder::new(skip_liquid, dir, fixture_data())
}

const HOLE_QUADS: usize = 8 + 32 + 2;

#[test]
fn float_tile_vertices_and_triangles() {
    let dir = fresh_dir("tb-float");
    write_map_file(&dir, 1, 32, 31, Heights::Float, Liquid::None, true);
    let tb = builder(&dir, false);
    let mut mesh = MeshData::default();
    assert!(tb.load_map_portion(1, 32, 31, &mut mesh, Spot::Entire));

    assert_eq!(
        mesh.solid_verts.len(),
        ((V9_SIZE_SQ + V8_SIZE_SQ) * 3) as usize
    );
    // vertex 0: V9 (row 0, col 0) -> (-(xoffset), height, -(yoffset))
    assert_eq!(mesh.solid_verts[0].to_bits(), (-0.0f32).to_bits());
    assert_eq!(
        mesh.solid_verts[1],
        terrain_height(32.0 * 128.0, 31.0 * 128.0)
    );
    assert_eq!(mesh.solid_verts[2], ((31.0f32 - 32.0) * GRID_SIZE) * -1.0);
    // first V8 vertex is offset by half a grid part
    let v8 = (V9_SIZE_SQ * 3) as usize;
    assert_eq!(
        mesh.solid_verts[v8],
        (0.0 + 0.0 * GRID_PART_SIZE + GRID_PART_SIZE / 2.0) * -1.0
    );

    // 4 triangles per quad, holes removed, no liquid
    assert_eq!(
        mesh.solid_tris.len() / 3,
        (V8_SIZE_SQ as usize - HOLE_QUADS) * 4
    );
    assert!(mesh.liquid_tris.is_empty() && mesh.liquid_verts.is_empty());
    // first quad: TOP triangle reversed = (V9_SQ + 0, 1, 0)
    assert_eq!(&mesh.solid_tris[0..3], &[V9_SIZE_SQ, 1, 0]);
}

#[test]
fn packed_heights_decode_like_cpp() {
    for (mode, div) in [(Heights::U16, 65535.0f32), (Heights::U8, 255.0f32)] {
        let dir = fresh_dir(&format!("tb-packed-{div}"));
        write_map_file(&dir, 1, 32, 32, mode, Liquid::None, false);
        // read the header values back to recompute the expected height
        let bytes = std::fs::read(dir.join("maps/0001_32_32.map")).unwrap();
        let hoff = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
        let lo = f32::from_le_bytes(bytes[hoff + 8..hoff + 12].try_into().unwrap());
        let hi = f32::from_le_bytes(bytes[hoff + 12..hoff + 16].try_into().unwrap());
        let raw0 = if div > 255.0 {
            f32::from(u16::from_le_bytes(
                bytes[hoff + 16..hoff + 18].try_into().unwrap(),
            ))
        } else {
            f32::from(bytes[hoff + 16])
        };
        let mult = (hi - lo) / div;

        let mut mesh = MeshData::default();
        assert!(builder(&dir, false).load_map_portion(1, 32, 32, &mut mesh, Spot::Entire));
        assert_eq!(mesh.solid_verts[1], raw0 * mult + lo);
        assert_eq!(mesh.solid_tris.len() / 3, V8_SIZE_SQ as usize * 4);
    }
}

#[test]
fn typed_liquid_areas_and_invalid_heights() {
    let dir = fresh_dir("tb-liquid");
    write_map_file(&dir, 1, 32, 31, Heights::Float, Liquid::Typed, true);
    let mut mesh = MeshData::default();
    assert!(builder(&dir, false).load_map_portion(1, 32, 31, &mut mesh, Spot::Entire));

    assert_eq!(mesh.liquid_verts.len(), (V9_SIZE_SQ * 3) as usize);
    assert_eq!(mesh.liquid_tris.len() / 3, mesh.liquid_type.len());
    assert!(!mesh.liquid_type.is_empty());
    assert!(
        mesh.liquid_type
            .iter()
            .all(|&t| t == nav_area::WATER || t == nav_area::MAGMA_SLIME)
    );
    // outside the liquid rectangle the dummy vertices keep the invalid height
    assert_eq!(mesh.liquid_verts[1], INVALID_MAP_LIQ_HEIGHT);
    // no used liquid triangle keeps an invalid height (padded by the quad average)
    for t in &mesh.liquid_tris {
        let h = mesh.liquid_verts[*t as usize * 3 + 1];
        assert!(
            h != INVALID_MAP_LIQ_HEIGHT && h <= INVALID_MAP_LIQ_HEIGHT_MAX,
            "{h}"
        );
    }
    // dark water cells (flags 0x10: (row+col)%6 == 2) drop terrain and liquid
    let quad = |cr: i32, cc: i32| cr * 8 * 128 + cc * 8;
    let dark = quad(0, 2);
    let first_tri_of = |q: i32| [V9_SIZE_SQ + q, q + 1 + q / 128, q + q / 128];
    let has = |tris: &[i32], t: [i32; 3]| tris.chunks_exact(3).any(|c| c == t);
    assert!(!has(&mesh.solid_tris, first_tri_of(dark)));
    // a no-liquid cell ((row+col)%6 == 3) keeps its terrain
    assert!(has(&mesh.solid_tris, first_tri_of(quad(0, 3))));

    // skipLiquid ignores the liquid section entirely
    let mut mesh = MeshData::default();
    assert!(builder(&dir, true).load_map_portion(1, 32, 31, &mut mesh, Spot::Entire));
    assert!(mesh.liquid_verts.is_empty());
    assert_eq!(
        mesh.solid_tris.len() / 3,
        (V8_SIZE_SQ as usize - HOLE_QUADS) * 4
    );
}

#[test]
fn liquid_only_tile_and_flat_liquid() {
    let dir = fresh_dir("tb-liquid-only");
    write_map_file(&dir, 1, 31, 31, Heights::None, Liquid::Flat(0x04), false);
    let mut mesh = MeshData::default();
    assert!(builder(&dir, false).load_map_portion(1, 31, 31, &mut mesh, Spot::Entire));
    assert!(mesh.solid_verts.is_empty());
    assert_eq!(mesh.liquid_tris.len() / 3, V8_SIZE_SQ as usize * 2);
    assert!(mesh.liquid_type.iter().all(|&t| t == nav_area::MAGMA_SLIME));
    assert!(mesh.liquid_verts.chunks_exact(3).all(|v| v[1] == 2.5));
    // with skipLiquid there is nothing left
    let mut mesh = MeshData::default();
    assert!(!builder(&dir, true).load_map_portion(1, 31, 31, &mut mesh, Spot::Entire));
}

#[test]
fn neighbour_portions_and_parent_fallback() {
    let dir = fresh_dir("tb-neighbours");
    build_fixture(&dir);
    let tb = builder(&dir, false);
    let mut mesh = MeshData::default();
    tb.load_map(1, 32, 31, &mut mesh);
    // entire tile + LEFT of (33,31) + TOP of (32,32) + RIGHT of (31,31) (liquid only)
    let per_tile = ((V9_SIZE_SQ + V8_SIZE_SQ) * 3) as usize;
    assert_eq!(mesh.solid_verts.len(), per_tile * 3);
    assert_eq!(mesh.liquid_verts.len(), (V9_SIZE_SQ * 3) as usize * 3);

    // a portion loop only touches one edge row/column
    assert_eq!(
        TerrainBuilder::get_loop_vars(Spot::Left),
        (0, V8_SIZE_SQ - V8_SIZE + 1, V8_SIZE)
    );
    assert_eq!(
        TerrainBuilder::get_loop_vars(Spot::Right),
        (V8_SIZE - 1, V8_SIZE_SQ, V8_SIZE)
    );
    assert_eq!(TerrainBuilder::get_loop_vars(Spot::Top), (0, V8_SIZE, 1));
    assert_eq!(
        TerrainBuilder::get_loop_vars(Spot::Bottom),
        (V8_SIZE_SQ - V8_SIZE, V8_SIZE_SQ, 1)
    );

    // child map 2 falls back to the parent's .map files
    let mut child = MeshData::default();
    assert!(tb.load_map_portion(2, 32, 31, &mut child, Spot::Entire));
    let mut parent = MeshData::default();
    assert!(tb.load_map_portion(1, 32, 31, &mut parent, Spot::Entire));
    assert_eq!(child, parent);
}

#[test]
fn bad_files_are_rejected() {
    let dir = fresh_dir("tb-bad");
    std::fs::create_dir_all(dir.join("maps")).unwrap();
    let mut header = crate::map_defines::MapFileHeader {
        map_magic: *b"MAPS",
        version_magic: 9,
        ..Default::default()
    };
    std::fs::write(dir.join("maps/0001_31_32.map"), header.to_bytes()).unwrap();
    let tb = builder(&dir, false);
    let mut mesh = MeshData::default();
    assert!(!tb.load_map_portion(1, 32, 31, &mut mesh, Spot::Entire));
    // right version but the height header is missing -> no data
    header.version_magic = 10;
    header.height_map_offset = 44;
    std::fs::write(dir.join("maps/0001_31_32.map"), header.to_bytes()).unwrap();
    assert!(!tb.load_map_portion(1, 32, 31, &mut mesh, Spot::Entire));
    // truncated heights: still triangulated (missing values read as 0)
    let mut bytes = header.to_bytes().to_vec();
    bytes.extend_from_slice(b"MHGT");
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 8]);
    bytes.extend_from_slice(&1.5f32.to_le_bytes());
    std::fs::write(dir.join("maps/0001_31_32.map"), bytes).unwrap();
    assert!(tb.load_map_portion(1, 32, 31, &mut mesh, Spot::Entire));
    assert_eq!(mesh.solid_verts[1], 1.5);
    assert_eq!(mesh.solid_verts[4], 0.0);
    // missing file
    assert!(!tb.load_map_portion(1, 10, 10, &mut MeshData::default(), Spot::Entire));
}

#[test]
fn triangle_indices_holes_and_liquid_cells() {
    assert_eq!(
        TerrainBuilder::get_height_triangle(0, Spot::Top, false),
        [0, 1, V9_SIZE_SQ]
    );
    assert_eq!(
        TerrainBuilder::get_height_triangle(0, Spot::Left, false),
        [0, V9_SIZE_SQ, 129]
    );
    assert_eq!(
        TerrainBuilder::get_height_triangle(0, Spot::Right, false),
        [1, 130, V9_SIZE_SQ]
    );
    assert_eq!(
        TerrainBuilder::get_height_triangle(0, Spot::Bottom, false),
        [V9_SIZE_SQ, 130, 129]
    );
    assert_eq!(
        TerrainBuilder::get_height_triangle(128, Spot::Top, false),
        [129, 130, V9_SIZE_SQ + 128]
    );
    assert_eq!(
        TerrainBuilder::get_height_triangle(0, Spot::Top, true),
        [0, 1, 130]
    );
    assert_eq!(
        TerrainBuilder::get_height_triangle(0, Spot::Bottom, true),
        [0, 130, 129]
    );
    assert_eq!(
        TerrainBuilder::get_height_triangle(0, Spot::Left, true),
        [0, 0, 0]
    );

    let mut holes = [0u8; 2048];
    holes[(3 * 16 + 5) * 8 + 2] = 0b0000_0100;
    // square row = 3*8+2, col = 5*8+2
    let sq = (3 * 8 + 2) * 128 + 5 * 8 + 2;
    assert!(TerrainBuilder::is_hole(sq, &holes));
    assert!(!TerrainBuilder::is_hole(sq + 1, &holes));

    let mut flags = [0u8; 256];
    flags[2 * 16 + 7] = 0x10;
    assert_eq!(
        TerrainBuilder::get_liquid_type((2 * 8 + 3) * 128 + 7 * 8 + 1, &flags),
        0x10
    );
    assert_eq!(TerrainBuilder::get_liquid_type(0, &flags), 0);
}

#[test]
fn clean_vertices_keeps_first_use_order() {
    let mut verts: Vec<f32> = (0..15).map(|i| i as f32).collect();
    let mut tris = vec![4, 2, 2, 0, 4, 2];
    TerrainBuilder::clean_vertices(&mut verts, &mut tris);
    assert_eq!(verts, vec![12.0, 13.0, 14.0, 6.0, 7.0, 8.0, 0.0, 1.0, 2.0]);
    assert_eq!(tris, vec![0, 1, 1, 2, 0, 1]);
}

#[test]
fn transform_and_copy_helpers() {
    let src = [Vector3::new(1.0, 2.0, 3.0)];
    let rot = Matrix3 {
        elt: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    };
    let out = TerrainBuilder::transform(&src, 2.0, &rot, Vector3::new(10.0, 20.0, 30.0));
    // row vector * matrix: (1*0 + 2*-1, 1*1 + 2*0, 3) = (-2, 1, 3) *2 + pos, mirrored x/y
    assert_eq!(out[0], Vector3::new(-6.0, -22.0, 36.0));
    let mut dest = Vec::new();
    TerrainBuilder::copy_vertices(&out, &mut dest);
    assert_eq!(dest, vec![-22.0, 36.0, -6.0]);

    let tris = [MeshTriangle::new(0, 1, 2)];
    let mut d = Vec::new();
    TerrainBuilder::copy_indices(&tris, &mut d, 5, false);
    TerrainBuilder::copy_indices(&tris, &mut d, 5, true);
    assert_eq!(d, vec![5, 6, 7, 7, 6, 5]);
    let mut d2 = vec![1];
    TerrainBuilder::copy_indices_offset(&[0, 3], &mut d2, 10);
    assert_eq!(d2, vec![1, 10, 13]);
}

#[test]
fn load_vmap_models_and_liquids() {
    let dir = fresh_dir("tb-vmap");
    build_fixture(&dir);
    let tb = builder(&dir, false);
    let mut mesh = MeshData::default();
    // map tile (32,31) is requested as vmap tile (31,32)
    assert!(tb.load_vmap(1, 31, 32, &mut mesh));
    // house: 8 + 4 + 3 vertices, rock: 4
    assert_eq!(mesh.solid_verts.len() / 3, 19);
    assert_eq!(mesh.solid_tris.len() / 3, 6 + 2 + 1 + 4);
    // water pool: 48 - 7 flagged tiles, magma: 15 tiles, 2 triangles each
    assert_eq!(mesh.liquid_tris.len() / 3, (41 + 15) * 2);
    assert_eq!(
        mesh.liquid_type
            .iter()
            .filter(|&&t| t == nav_area::WATER)
            .count(),
        82
    );
    assert_eq!(
        mesh.liquid_type
            .iter()
            .filter(|&&t| t == nav_area::MAGMA_SLIME)
            .count(),
        30
    );
    assert_eq!(mesh.liquid_verts.len() / 3, 9 * 7 + 6 * 4);
    // the M2 (second instance) has its triangle winding flipped
    let rock_first = &mesh.solid_tris[9 * 3..9 * 3 + 3];
    assert_eq!(rock_first, &[15 + 3, 15 + 1, 15]);

    // positions: recast x = -(pos.y - 32G + ...) lies inside the tile
    let xs: Vec<f32> = mesh.solid_verts.chunks_exact(3).map(|v| v[0]).collect();
    assert!(
        xs.iter().all(|&x| (-GRID_SIZE - 50.0..=50.0).contains(&x)),
        "{xs:?}"
    );

    // child map 2 resolves the spawns from the parent's vmtile
    let mut child = MeshData::default();
    assert!(tb.load_vmap(2, 31, 32, &mut child));
    assert_eq!(child, mesh);

    // tile without vmtile
    assert!(!tb.load_vmap(1, 5, 5, &mut MeshData::default()));
}

#[test]
fn off_mesh_connections_filtered_by_tile() {
    let conns = [
        OffMeshData {
            map_id: 1,
            tile_x: 32,
            tile_y: 31,
            from: [1.0, 2.0, 3.0],
            to: [4.0, 5.0, 6.0],
            bidirectional: true,
            radius: 1.5,
            area_id: 11,
            flags: 1,
        },
        OffMeshData {
            map_id: 1,
            tile_x: 31,
            tile_y: 32,
            ..Default::default()
        },
    ];
    let mut mesh = MeshData::default();
    TerrainBuilder::load_off_mesh_connections(1, 32, 31, &mut mesh, &conns);
    assert_eq!(
        mesh.off_mesh_connections,
        vec![2.0, 3.0, 1.0, 5.0, 6.0, 4.0]
    );
    assert_eq!(mesh.off_mesh_connection_dirs, vec![1]);
    assert_eq!(mesh.off_mesh_connection_rads, vec![1.5]);
    assert_eq!(mesh.off_mesh_connections_areas, vec![11]);
    assert_eq!(mesh.off_mesh_connections_flags, vec![1]);
}
