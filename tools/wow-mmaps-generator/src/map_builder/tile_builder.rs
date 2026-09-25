//! `MMAP::TileBuilder` from `src/tools/mmaps_generator/MapBuilder.{h,cpp}`
//! (TDB343.24081): `WorkerThread`, `buildTile`, `buildMoveMapTile` (the
//! Recast/Detour pipeline writing `.mmtile`) and `shouldSkipTile`.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::{Shared, TileConfig, TileQueue};
use crate::intermediate_values::IntermediateValues;
use crate::map_defines::{MMAP_MAGIC, MMAP_VERSION, MmapTileHeader, nav_area, nav_flag};
use crate::recast::{
    self, CompactHeightfield, ContourSet, DT_NAVMESH_VERSION, DT_VERTS_PER_POLYGON,
    DtNavMeshCreateParams, Heightfield, NavMesh, PolyMesh, PolyMeshDetail, RcContext,
};
use crate::terrain_builder::{GRID_SIZE, MeshData, TerrainBuilder};

/// `MMAP::TileBuilder`.
pub struct TileBuilder {
    big_base_unit: bool,
    debug_output: bool,
    shared: Arc<Shared>,
    terrain_builder: TerrainBuilder,
    rc_context: RcContext,
}

/// `MMAP::Tile` — per sub-tile Recast intermediates.
#[derive(Default)]
struct Tile {
    chf: Option<CompactHeightfield>,
    solid: Option<Heightfield>,
    cset: Option<ContourSet>,
    pmesh: Option<PolyMesh>,
    dmesh: Option<PolyMeshDetail>,
}

impl TileBuilder {
    pub(super) fn new(shared: Arc<Shared>) -> Self {
        let opts = &shared.opts;
        Self {
            big_base_unit: opts.big_base_unit,
            debug_output: opts.debug_output,
            terrain_builder: TerrainBuilder::new(
                opts.skip_liquid,
                shared.base.clone(),
                Arc::clone(&shared.data),
            ),
            rc_context: RcContext::new(),
            shared,
        }
    }

    /// `TileBuilder::WorkerThread`.
    pub(super) fn worker_thread(self, queue: &TileQueue) {
        while let Some(tile_info) = queue.wait_and_pop() {
            let mut nav_mesh = NavMesh::alloc();
            // C++ `if (!navMesh->init(...))` on a dtStatus: only a 0 status
            // (never returned by Detour) counts as failure.
            if nav_mesh.init(&tile_info.nav_mesh_params) == 0 {
                println!(
                    "[Map {:04}] Failed creating navmesh for tile {},{} !",
                    tile_info.map_id as i32, tile_info.tile_x as i32, tile_info.tile_y as i32
                );
                return;
            }
            self.build_tile(
                tile_info.map_id,
                tile_info.tile_x,
                tile_info.tile_y,
                &mut nav_mesh,
            );
        }
    }

    /// `TileBuilder::buildTile`.
    pub fn build_tile(&self, map_id: u32, tile_x: u32, tile_y: u32, nav_mesh: &mut NavMesh) {
        let processed = &self.shared.total_tiles_processed;
        if self.should_skip_tile(map_id, tile_x, tile_y) {
            processed.fetch_add(1, Ordering::SeqCst);
            return;
        }

        println!(
            "{}% [Map {:04}] Building tile [{:02},{:02}]",
            self.shared.current_percentage_done(),
            map_id as i32,
            tile_x,
            tile_y
        );

        let mut mesh_data = MeshData::default();

        // get heightmap data
        self.terrain_builder
            .load_map(map_id, tile_x, tile_y, &mut mesh_data);

        // get model data
        self.terrain_builder
            .load_vmap(map_id, tile_y, tile_x, &mut mesh_data);

        // if there is no data, give up now
        if mesh_data.solid_verts.is_empty() && mesh_data.liquid_verts.is_empty() {
            processed.fetch_add(1, Ordering::SeqCst);
            return;
        }

        // remove unused vertices
        TerrainBuilder::clean_vertices(&mut mesh_data.solid_verts, &mut mesh_data.solid_tris);
        TerrainBuilder::clean_vertices(&mut mesh_data.liquid_verts, &mut mesh_data.liquid_tris);

        // gather all mesh data for final data check, and bounds calculation
        let mut all_verts = mesh_data.liquid_verts.clone();
        all_verts.extend_from_slice(&mesh_data.solid_verts);

        if all_verts.is_empty() {
            processed.fetch_add(1, Ordering::SeqCst);
            return;
        }

        // get bounds of current tile
        let (bmin, bmax) = Shared::get_tile_bounds(tile_x, tile_y, Some(&all_verts));

        TerrainBuilder::load_off_mesh_connections(
            map_id,
            tile_x,
            tile_y,
            &mut mesh_data,
            &self.shared.off_mesh_connections,
        );

        // build navmesh tile
        self.build_move_map_tile(
            map_id,
            tile_x,
            tile_y,
            &mut mesh_data,
            &bmin,
            &bmax,
            nav_mesh,
        );

        processed.fetch_add(1, Ordering::SeqCst);
    }

    /// `TileBuilder::buildMoveMapTile`.
    #[allow(clippy::too_many_lines, clippy::too_many_arguments)]
    pub fn build_move_map_tile(
        &self,
        map_id: u32,
        tile_x: u32,
        tile_y: u32,
        mesh_data: &mut MeshData,
        bmin: &[f32; 3],
        bmax: &[f32; 3],
        nav_mesh: &mut NavMesh,
    ) {
        let ctx = &self.rc_context;
        // console output
        let tile_string = format!("[Map {map_id:04}] [{tile_x:02},{tile_y:02}]: ");
        println!("{tile_string} Building movemap tiles...");

        let mut iv = IntermediateValues::default();

        let t_verts = &mesh_data.solid_verts;
        let t_tris = &mesh_data.solid_tris;
        let t_tri_count = t_tris.len() / 3;
        let l_verts = &mesh_data.liquid_verts;
        let l_tris = &mesh_data.liquid_tris;
        let l_tri_flags = &mesh_data.liquid_type;

        let tile_config = TileConfig::new(self.big_base_unit);
        let tiles_per_map = tile_config.tiles_per_map;
        let base_unit_dim = tile_config.base_unit_dim;
        let mut config = self
            .shared
            .get_map_specific_config(map_id, bmin, bmax, &tile_config);

        // this sets the dimensions of the heightfield - should maybe happen before border padding
        let (w, h) = recast::rc_calc_grid_size(&config.bmin, &config.bmax, config.cs);
        config.width = w;
        config.height = h;

        // allocate subregions : tiles
        let mut tiles: Vec<Tile> = (0..tiles_per_map * tiles_per_map)
            .map(|_| Tile::default())
            .collect();

        // Initialize per tile config.
        let mut tile_cfg = config;
        tile_cfg.width = config.tile_size + config.border_size * 2;
        tile_cfg.height = config.tile_size + config.border_size * 2;

        // merge per tile poly and detail meshes
        let mut merge: Vec<usize> = Vec::new();
        // build all tiles
        for y in 0..tiles_per_map {
            for x in 0..tiles_per_map {
                let tile_index = (x + y * tiles_per_map) as usize;
                let tile = &mut tiles[tile_index];

                // Calculate the per tile bounding box.
                let step = config.tile_size as f32 * config.cs;
                tile_cfg.bmin[0] = config.bmin[0] + x as f32 * step;
                tile_cfg.bmin[2] = config.bmin[2] + y as f32 * step;
                tile_cfg.bmax[0] = config.bmin[0] + (x + 1) as f32 * step;
                tile_cfg.bmax[2] = config.bmin[2] + (y + 1) as f32 * step;

                tile_cfg.bmin[0] -= tile_cfg.border_size as f32 * tile_cfg.cs;
                tile_cfg.bmin[2] -= tile_cfg.border_size as f32 * tile_cfg.cs;
                tile_cfg.bmax[0] += tile_cfg.border_size as f32 * tile_cfg.cs;
                tile_cfg.bmax[2] += tile_cfg.border_size as f32 * tile_cfg.cs;

                // build heightfield
                tile.solid = Heightfield::alloc();
                let Some(solid) = tile.solid.as_ref().filter(|s| {
                    recast::create_heightfield(
                        ctx,
                        s,
                        tile_cfg.width,
                        tile_cfg.height,
                        &tile_cfg.bmin,
                        &tile_cfg.bmax,
                        tile_cfg.cs,
                        tile_cfg.ch,
                    )
                }) else {
                    println!("{tile_string} Failed building heightfield!            ");
                    continue;
                };

                // mark all walkable tiles, both liquids and solids

                /* we want to have triangles with slope less than walkableSlopeAngleNotSteep (<= 55) to have NAV_AREA_GROUND
                 * and with slope between walkableSlopeAngleNotSteep and walkableSlopeAngle (55 < .. <= 70) to have NAV_AREA_GROUND_STEEP.
                 * we achieve this using recast API: memset everything to NAV_AREA_GROUND_STEEP, call rcClearUnwalkableTriangles with 70 so
                 * any area above that will get RC_NULL_AREA (unwalkable), then call rcMarkWalkableTriangles with 55 to set NAV_AREA_GROUND
                 * on anything below 55 . Players and idle Creatures can use NAV_AREA_GROUND, while Creatures in combat can use NAV_AREA_GROUND_STEEP.
                 */
                let mut tri_flags = vec![nav_area::GROUND_STEEP; t_tri_count];
                recast::clear_unwalkable_triangles(
                    ctx,
                    tile_cfg.walkable_slope_angle,
                    t_verts,
                    t_tris,
                    &mut tri_flags,
                );
                recast::mark_walkable_triangles(
                    ctx,
                    tile_cfg.walkable_slope_angle_not_steep,
                    t_verts,
                    t_tris,
                    &mut tri_flags,
                    nav_area::GROUND,
                );
                recast::rasterize_triangles(
                    ctx,
                    t_verts,
                    t_tris,
                    &tri_flags,
                    solid,
                    config.walkable_climb,
                );
                drop(tri_flags);

                recast::filter_low_hanging_walkable_obstacles(ctx, config.walkable_climb, solid);
                recast::filter_ledge_spans(
                    ctx,
                    tile_cfg.walkable_height,
                    tile_cfg.walkable_climb,
                    solid,
                );
                recast::filter_walkable_low_height_spans(ctx, tile_cfg.walkable_height, solid);

                // add liquid triangles
                recast::rasterize_triangles(
                    ctx,
                    l_verts,
                    l_tris,
                    l_tri_flags,
                    solid,
                    config.walkable_climb,
                );

                // compact heightfield spans
                tile.chf = CompactHeightfield::alloc();
                let Some(chf) = tile.chf.as_ref().filter(|c| {
                    recast::build_compact_heightfield(
                        ctx,
                        tile_cfg.walkable_height,
                        tile_cfg.walkable_climb,
                        solid,
                        c,
                    )
                }) else {
                    println!("{tile_string} Failed compacting heightfield!            ");
                    continue;
                };

                // build polymesh intermediates
                if !recast::erode_walkable_area(ctx, config.walkable_radius, chf) {
                    println!("{tile_string} Failed eroding area!                    ");
                    continue;
                }

                if !recast::median_filter_walkable_area(ctx, chf) {
                    println!("{tile_string} Failed filtering area!                  ");
                    continue;
                }

                if !recast::build_distance_field(ctx, chf) {
                    println!("{tile_string} Failed building distance field!         ");
                    continue;
                }

                if !recast::build_regions(
                    ctx,
                    chf,
                    tile_cfg.border_size,
                    tile_cfg.min_region_area,
                    tile_cfg.merge_region_area,
                ) {
                    println!("{tile_string} Failed building regions!                ");
                    continue;
                }

                tile.cset = ContourSet::alloc();
                let Some(cset) = tile.cset.as_ref().filter(|cs| {
                    recast::build_contours(
                        ctx,
                        chf,
                        tile_cfg.max_simplification_error,
                        tile_cfg.max_edge_len,
                        cs,
                    )
                }) else {
                    println!("{tile_string} Failed building contours!               ");
                    continue;
                };

                // build polymesh
                tile.pmesh = PolyMesh::alloc();
                let Some(pmesh) = tile.pmesh.as_ref().filter(|pm| {
                    recast::build_poly_mesh(ctx, cset, tile_cfg.max_verts_per_poly, pm)
                }) else {
                    println!("{tile_string} Failed building polymesh!               ");
                    continue;
                };

                tile.dmesh = PolyMeshDetail::alloc();
                if !tile.dmesh.as_ref().is_some_and(|dm| {
                    recast::build_poly_mesh_detail(
                        ctx,
                        pmesh,
                        chf,
                        tile_cfg.detail_sample_dist,
                        tile_cfg.detail_sample_max_error,
                        dm,
                    )
                }) {
                    println!("{tile_string} Failed building polymesh detail!        ");
                    continue;
                }

                // free those up
                // we may want to keep them in the future for debug
                // but right now, we don't have the code to merge them
                tile.solid = None;
                tile.chf = None;
                tile.cset = None;

                merge.push(tile_index);
            }
        }

        let Some(poly_mesh) = PolyMesh::alloc() else {
            println!("{tile_string} alloc iv.polyMesh FAILED!");
            return;
        };
        {
            let pm: Vec<&PolyMesh> = merge
                .iter()
                .filter_map(|&i| tiles[i].pmesh.as_ref())
                .collect();
            recast::merge_poly_meshes(ctx, &pm, &poly_mesh);
        }
        iv.poly_mesh = Some(poly_mesh);

        let Some(poly_mesh_detail) = PolyMeshDetail::alloc() else {
            println!("{tile_string} alloc m_dmesh FAILED!");
            return;
        };
        {
            let dm: Vec<&PolyMeshDetail> = merge
                .iter()
                .filter_map(|&i| tiles[i].dmesh.as_ref())
                .collect();
            recast::merge_poly_mesh_details(ctx, &dm, &poly_mesh_detail);
        }
        iv.poly_mesh_detail = Some(poly_mesh_detail);

        // free things up
        drop(tiles);

        let poly_mesh = iv.poly_mesh.as_mut().expect("set above");
        // set polygons as walkable
        // TODO: special flags for DYNAMIC polygons, ie surfaces that can be turned on and off
        {
            let (areas, flags) = poly_mesh.areas_flags_mut();
            for (area, flag) in areas.iter().zip(flags.iter_mut()) {
                let area = area & nav_area::ALL_MASK;
                if area != 0 {
                    if area >= nav_area::MIN_VALUE {
                        let shift = i32::from(nav_area::MAX_VALUE) - i32::from(area);
                        *flag = 1u32.wrapping_shl(shift as u32) as u16;
                    } else {
                        *flag = nav_flag::GROUND; // TODO: these will be dynamic in future
                    }
                }
            }
        }

        let pm = poly_mesh.get();
        let dm = iv.poly_mesh_detail.as_ref().expect("set above").get();

        // setup mesh parameters
        let nav_params = nav_mesh.params();
        let mut params = DtNavMeshCreateParams {
            verts: pm.verts,
            vert_count: pm.nverts,
            polys: pm.polys,
            poly_areas: pm.areas,
            poly_flags: pm.flags,
            poly_count: pm.npolys,
            nvp: pm.nvp,
            detail_meshes: dm.meshes,
            detail_verts: dm.verts,
            detail_verts_count: dm.nverts,
            detail_tris: dm.tris,
            detail_tri_count: dm.ntris,

            off_mesh_con_verts: mesh_data.off_mesh_connections.as_ptr(),
            off_mesh_con_count: (mesh_data.off_mesh_connections.len() / 6) as i32,
            off_mesh_con_rad: mesh_data.off_mesh_connection_rads.as_ptr(),
            off_mesh_con_dir: mesh_data.off_mesh_connection_dirs.as_ptr(),
            off_mesh_con_areas: mesh_data.off_mesh_connections_areas.as_ptr(),
            off_mesh_con_flags: mesh_data.off_mesh_connections_flags.as_ptr(),

            walkable_height: base_unit_dim * config.walkable_height as f32, // agent height
            walkable_radius: base_unit_dim * config.walkable_radius as f32, // agent radius
            walkable_climb: base_unit_dim * config.walkable_climb as f32, // keep less that walkableHeight (aka agent height)!
            tile_x: (((bmin[0] + bmax[0]) / 2.0 - nav_params.orig[0]) / GRID_SIZE) as i32,
            tile_y: (((bmin[2] + bmax[2]) / 2.0 - nav_params.orig[2]) / GRID_SIZE) as i32,
            bmin: *bmin,
            bmax: *bmax,
            cs: config.cs,
            ch: config.ch,
            tile_layer: 0,
            build_bv_tree: true,
            ..DtNavMeshCreateParams::default()
        };

        'build: {
            // these values are checked within dtCreateNavMeshData - handle them here
            // so we have a clear error message
            if params.nvp > DT_VERTS_PER_POLYGON {
                println!("{tile_string} Invalid verts-per-polygon value!        ");
                break 'build;
            }
            if params.vert_count >= 0xffff {
                println!("{tile_string} Too many vertices!                      ");
                break 'build;
            }
            if params.vert_count == 0 || params.verts.is_null() {
                // occurs mostly when adjacent tiles have models
                // loaded but those models don't span into this tile

                // message is an annoyance
                //printf("%sNo vertices to build tile!              \n", tileString.c_str());
                break 'build;
            }
            if params.poly_count == 0 || params.polys.is_null() {
                // we have flat tiles with no actual geometry - don't build those, its useless
                // keep in mind that we do output those into debug info
                println!("{tile_string} No polygons to build on tile!              ");
                break 'build;
            }
            if params.detail_meshes.is_null()
                || params.detail_verts.is_null()
                || params.detail_tris.is_null()
            {
                println!("{tile_string} No detail mesh to build tile!           ");
                break 'build;
            }

            println!("{tile_string} Building navmesh tile...");
            let Some(nav_data) = recast::create_nav_mesh_data(&mut params) else {
                println!("{tile_string} Failed building navmesh tile!           ");
                break 'build;
            };
            let nav_data_size = nav_data.size();

            println!("{tile_string} Adding tile to navmesh...");
            // DT_TILE_FREE_DATA tells detour to unallocate memory when the tile
            // is removed via removeTile()
            let Ok((tile_ref, nav_bytes)) = nav_mesh.add_tile(nav_data) else {
                println!("{tile_string} Failed adding tile to navmesh!           ");
                break 'build;
            };

            // file output
            let file_name = format!(
                "mmaps/{map_id:04}{:02}{:02}.mmtile",
                tile_y as i32, tile_x as i32
            );
            let file = std::fs::File::create(self.shared.base.join(&file_name));
            let mut file = match file {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("[Map {map_id:04}] Failed to open {file_name} for writing!\n: {e}");
                    nav_mesh.remove_tile(tile_ref);
                    break 'build;
                }
            };

            println!("{tile_string} Writing to file...");

            // write header
            let header = MmapTileHeader {
                uses_liquids: self.terrain_builder.uses_liquids(),
                size: nav_data_size as u32,
                ..MmapTileHeader::default()
            };
            let _ = file.write_all(&header.to_bytes());

            // write data
            let _ = file.write_all(&nav_bytes);
            drop(file);

            // now that tile is written to disk, we can unload it
            nav_mesh.remove_tile(tile_ref);
        }

        if self.debug_output {
            // restore padding so that the debug visualization is correct
            let border = config.border_size as u16;
            for v in iv
                .poly_mesh
                .as_mut()
                .expect("set above")
                .verts_mut()
                .chunks_exact_mut(3)
            {
                v[0] = v[0].wrapping_add(border);
                v[2] = v[2].wrapping_add(border);
            }

            iv.generate_obj_file(&self.shared.base, map_id, tile_x, tile_y, mesh_data);
            iv.write_iv(&self.shared.base, map_id, tile_x, tile_y);
        }
    }

    /// `TileBuilder::shouldSkipTile` — an existing `.mmtile` with a current
    /// header is not rebuilt.
    pub fn should_skip_tile(&self, map_id: u32, tile_x: u32, tile_y: u32) -> bool {
        let file_name = format!(
            "mmaps/{map_id:04}{:02}{:02}.mmtile",
            tile_y as i32, tile_x as i32
        );
        let Ok(bytes) = std::fs::read(self.shared.base.join(file_name)) else {
            return false;
        };
        let Some(head) = bytes.get(..MmapTileHeader::SIZE) else {
            return false;
        };
        let header = MmapTileHeader::parse(head.try_into().expect("20 bytes"));
        if header.mmap_magic != MMAP_MAGIC || header.dt_version != DT_NAVMESH_VERSION {
            return false;
        }
        header.mmap_version == MMAP_VERSION
    }
}
