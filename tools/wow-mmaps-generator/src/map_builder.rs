//! Port of `src/tools/mmaps_generator/MapBuilder.{h,cpp}` (TDB343.24081):
//! `MapBuilder` (tile discovery, skip rules, `buildNavMesh` / `.mmap`,
//! off-mesh file parsing, map/tile scheduling) and `TileBuilder`
//! (`buildTile`, `buildMoveMapTile` -> `.mmtile`, `shouldSkipTile`).
//!
//! Threading: C++ feeds a `ProducerConsumerQueue<TileInfo>` consumed by one
//! `TileBuilder` thread each. Every tile is built from its own fresh
//! `dtNavMesh` and its own input files, so the output does not depend on the
//! scheduling. Deviation: C++ `buildMaps` cancels the queue as soon as it is
//! *empty* — a worker that just popped the last items can observe the
//! cancellation token and drop its tile (a race). Here the queue is closed
//! and every queued tile is built before the workers are joined.

use std::collections::{BTreeSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};

mod off_mesh;
mod tile_builder;

pub use off_mesh::parse_off_mesh_connections_file;
pub use tile_builder::TileBuilder;

use crate::path_common::{GeneratorData, c_atoi, get_dir_contents};
use crate::recast::{self, DT_POLY_BITS, DT_VERTS_PER_POLYGON, DtNavMeshParams, NavMesh, RcConfig};
use crate::terrain_builder::{GRID_SIZE, MeshData, OffMeshData, TerrainBuilder};

/// `StaticMapTree::packTileID`.
pub fn pack_tile_id(tile_x: u32, tile_y: u32) -> u32 {
    (tile_x << 16) | tile_y
}

/// `StaticMapTree::unpackTileID` (Y masked with `0xFF`, as in C++).
pub fn unpack_tile_id(id: u32) -> (u32, u32) {
    (id >> 16, id & 0xFF)
}

/// `MMAP::TileConfig`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileConfig {
    pub base_unit_dim: f32,
    pub vertex_per_map: i32,
    pub vertex_per_tile: i32,
    pub tiles_per_map: i32,
}

impl TileConfig {
    pub fn new(big_base_unit: bool) -> Self {
        // these are WORLD UNIT based metrics
        // this are basic unit dimentions
        // value have to divide GRID_SIZE(533.3333f) ( aka: 0.5333, 0.2666, 0.3333, 0.1333, etc )
        let base_unit_dim = if big_base_unit {
            0.533_333_3_f32
        } else {
            0.266_666_6_f32
        };
        // All are in UNIT metrics!
        let vertex_per_map = (GRID_SIZE / base_unit_dim + 0.5) as i32;
        let vertex_per_tile = if big_base_unit { 40 } else { 80 }; // must divide VERTEX_PER_MAP
        Self {
            base_unit_dim,
            vertex_per_map,
            vertex_per_tile,
            tiles_per_map: vertex_per_map / vertex_per_tile,
        }
    }
}

/// `MMAP::TileInfo`.
#[derive(Debug, Clone, Copy)]
struct TileInfo {
    map_id: u32,
    tile_x: u32,
    tile_y: u32,
    nav_mesh_params: DtNavMeshParams,
}

/// `MapBuilder` constructor options (PathGenerator.cpp `main`).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuilderOptions {
    pub max_walkable_angle: Option<f32>,
    pub max_walkable_angle_not_steep: Option<f32>,
    pub skip_liquid: bool,
    pub skip_continents: bool,
    pub skip_junk_maps: bool,
    pub skip_battlegrounds: bool,
    pub debug_output: bool,
    pub big_base_unit: bool,
    pub mapid: i32,
    pub off_mesh_file_path: Option<String>,
    pub threads: u32,
}

/// State `TileBuilder`s read from their `MapBuilder`.
struct Shared {
    opts: BuilderOptions,
    base: PathBuf,
    data: Arc<GeneratorData>,
    off_mesh_connections: Vec<OffMeshData>,
    total_tiles: AtomicU32,
    total_tiles_processed: AtomicU32,
}

impl Shared {
    /// `MapBuilder::getTileBounds`.
    fn get_tile_bounds(tile_x: u32, tile_y: u32, verts: Option<&[f32]>) -> ([f32; 3], [f32; 3]) {
        let mut bmin = [0f32; 3];
        let mut bmax = [0f32; 3];
        // this is for elevation
        match verts {
            Some(v) if v.len() >= 3 => {
                recast::rc_calc_bounds(v, (v.len() / 3) as i32, &mut bmin, &mut bmax);
            }
            _ => {
                bmin[1] = f32::MIN_POSITIVE; // FLT_MIN
                bmax[1] = f32::MAX;
            }
        }
        // this is for width and depth
        bmax[0] = (32 - tile_x as i32) as f32 * GRID_SIZE;
        bmax[2] = (32 - tile_y as i32) as f32 * GRID_SIZE;
        bmin[0] = bmax[0] - GRID_SIZE;
        bmin[2] = bmax[2] - GRID_SIZE;
        (bmin, bmax)
    }

    /// `MapBuilder::GetMapSpecificConfig`.
    fn get_map_specific_config(
        &self,
        map_id: u32,
        bmin: &[f32; 3],
        bmax: &[f32; 3],
        tc: &TileConfig,
    ) -> RcConfig {
        let big = self.opts.big_base_unit;
        let mut config = RcConfig {
            bmin: *bmin,
            bmax: *bmax,
            max_verts_per_poly: DT_VERTS_PER_POLYGON,
            cs: tc.base_unit_dim,
            ch: tc.base_unit_dim,
            // Keeping these 2 slope angles the same reduces a lot the number of polys.
            // 55 should be the minimum, maybe 70 is ok (keep in mind blink uses mmaps), 85 is too much for players
            walkable_slope_angle: self.opts.max_walkable_angle.unwrap_or(55.0),
            walkable_slope_angle_not_steep: self.opts.max_walkable_angle_not_steep.unwrap_or(55.0),
            tile_size: tc.vertex_per_tile,
            walkable_radius: if big { 1 } else { 2 },
            ..RcConfig::default()
        };
        config.border_size = config.walkable_radius + 3;
        config.max_edge_len = tc.vertex_per_tile + 1; // anything bigger than tileSize
        config.walkable_height = if big { 3 } else { 6 };
        // a value >= 3|6 allows npcs to walk over some fences
        // a value >= 4|8 allows npcs to walk over all fences
        config.walkable_climb = if big { 3 } else { 6 };
        config.min_region_area = 60 * 60;
        config.merge_region_area = 50 * 50;
        config.max_simplification_error = 1.8; // eliminates most jagged edges (tiny polygons)
        config.detail_sample_dist = config.cs * 16.0;
        config.detail_sample_max_error = config.ch * 1.0;

        match map_id {
            // Blade's Edge Arena
            562 => {
                // This allows to walk on the ropes to the pillars
                config.walkable_radius = 0;
            }
            // Blackfathom Deeps
            48 => {
                // Reduce the chance to have underground levels
                config.ch *= 2.0;
            }
            _ => {}
        }
        config
    }

    /// `MapBuilder::percentageDone` / `currentPercentageDone`.
    fn current_percentage_done(&self) -> u32 {
        let total = self.total_tiles.load(Ordering::SeqCst);
        let done = self.total_tiles_processed.load(Ordering::SeqCst);
        percentage_done(total, done)
    }
}

/// `MapBuilder::percentageDone`.
pub fn percentage_done(total_tiles: u32, total_tiles_built: u32) -> u32 {
    total_tiles_built
        .wrapping_mul(100)
        .checked_div(total_tiles)
        .unwrap_or(0)
}

/// `ProducerConsumerQueue<TileInfo>`.
#[derive(Default)]
struct TileQueue {
    state: Mutex<(VecDeque<TileInfo>, bool)>,
    cond: Condvar,
}

impl TileQueue {
    fn push(&self, t: TileInfo) {
        self.state.lock().expect("queue lock").0.push_back(t);
        self.cond.notify_one();
    }

    fn close(&self) {
        self.state.lock().expect("queue lock").1 = true;
        self.cond.notify_all();
    }

    /// `WaitAndPop`; `None` once closed and drained.
    fn wait_and_pop(&self) -> Option<TileInfo> {
        let mut st = self.state.lock().expect("queue lock");
        loop {
            if let Some(t) = st.0.pop_front() {
                return Some(t);
            }
            if st.1 {
                return None;
            }
            st = self.cond.wait(st).expect("queue lock");
        }
    }
}

/// `MMAP::MapBuilder`.
pub struct MapBuilder {
    shared: Arc<Shared>,
    terrain_builder: TerrainBuilder,
    /// `m_tiles` (`std::list<MapTiles>`, insertion order).
    tiles: Vec<(u32, BTreeSet<u32>)>,
}

impl MapBuilder {
    /// `MapBuilder::MapBuilder` — `base` is the working directory holding
    /// `maps/`, `vmaps/` and `mmaps/` (C++ uses the process CWD).
    pub fn new(mut opts: BuilderOptions, base: impl AsRef<Path>, data: Arc<GeneratorData>) -> Self {
        let base = base.as_ref().to_path_buf();
        // At least 1 thread is needed
        opts.threads = opts.threads.max(1);
        let off_mesh_file = opts.off_mesh_file_path.clone();
        let terrain_builder =
            TerrainBuilder::new(opts.skip_liquid, base.clone(), Arc::clone(&data));
        let shared = Shared {
            opts,
            base,
            data,
            off_mesh_connections: Vec::new(),
            total_tiles: AtomicU32::new(0),
            total_tiles_processed: AtomicU32::new(0),
        };
        let mut mb = Self {
            shared: Arc::new(shared),
            terrain_builder,
            tiles: Vec::new(),
        };
        mb.discover_tiles();
        let conns = parse_off_mesh_connections_file(off_mesh_file.as_deref());
        Arc::get_mut(&mut mb.shared)
            .expect("no TileBuilder exists yet")
            .off_mesh_connections = conns;
        mb
    }

    fn base(&self) -> &Path {
        &self.shared.base
    }

    fn find_map(&self, map_id: u32) -> Option<usize> {
        self.tiles.iter().position(|(id, _)| *id == map_id)
    }

    /// `MapBuilder::discoverTiles`.
    fn discover_tiles(&mut self) {
        let mut files = Vec::new();
        let mut count: u32 = 0;
        let substr = |s: &str, pos: usize, n: usize| -> String {
            s.get(pos.min(s.len())..(pos + n).min(s.len()))
                .unwrap_or("")
                .to_owned()
        };

        print!("Discovering maps... ");
        let _ = std::io::stdout().flush();
        get_dir_contents(&mut files, &self.base().join("maps"), "*");
        for f in &files {
            let map_id = c_atoi(&substr(f, 0, 4)) as u32;
            if self.find_map(map_id).is_none() {
                self.tiles.push((map_id, BTreeSet::new()));
                count += 1;
            }
        }

        files.clear();
        get_dir_contents(&mut files, &self.base().join("vmaps"), "*.vmtree");
        for f in &files {
            let map_id = c_atoi(&substr(f, 0, 4)) as u32;
            if self.find_map(map_id).is_none() {
                self.tiles.push((map_id, BTreeSet::new()));
                count += 1;
            }
        }
        println!("found {count}.");

        count = 0;
        print!("Discovering tiles... ");
        let _ = std::io::stdout().flush();
        for idx in 0..self.tiles.len() {
            let map_id = self.tiles[idx].0;

            files.clear();
            get_dir_contents(
                &mut files,
                &self.base().join("vmaps"),
                &format!("{map_id:04}_*.vmtile"),
            );
            for f in &files {
                let tile_x = c_atoi(&substr(f, 8, 2)) as u32;
                let tile_y = c_atoi(&substr(f, 5, 2)) as u32;
                let tile_id = pack_tile_id(tile_y, tile_x);
                self.tiles[idx].1.insert(tile_id);
                count += 1;
            }

            files.clear();
            get_dir_contents(
                &mut files,
                &self.base().join("maps"),
                &format!("{map_id:04}*"),
            );
            for f in &files {
                let tile_y = c_atoi(&substr(f, 5, 2)) as u32;
                let tile_x = c_atoi(&substr(f, 8, 2)) as u32;
                let tile_id = pack_tile_id(tile_x, tile_y);
                if self.tiles[idx].1.insert(tile_id) {
                    count += 1;
                }
            }

            // make sure we process maps which don't have tiles
            if self.tiles[idx].1.is_empty() {
                // convert coord bounds to grid bounds
                let (min_x, min_y, max_x, max_y) = self.get_grid_bounds(map_id);

                // add all tiles within bounds to tile list.
                let mut i = min_x;
                while i <= max_x {
                    let mut j = min_y;
                    while j <= max_y {
                        if self.tiles[idx].1.insert(pack_tile_id(i, j)) {
                            count += 1;
                        }
                        if j == u32::MAX {
                            break;
                        }
                        j += 1;
                    }
                    if i == u32::MAX {
                        break;
                    }
                    i += 1;
                }
            }
        }
        println!("found {count}.\n");

        // Calculate tiles to process in total
        let mut total = 0u32;
        for (id, tiles) in &self.tiles {
            if !self.should_skip_map(*id) {
                total = total.wrapping_add(tiles.len() as u32);
            }
        }
        self.shared.total_tiles.fetch_add(total, Ordering::SeqCst);
    }

    /// `MapBuilder::getTileList` (creates an empty entry when missing).
    fn get_tile_list(&mut self, map_id: u32) -> &BTreeSet<u32> {
        let idx = if let Some(i) = self.find_map(map_id) {
            i
        } else {
            self.tiles.push((map_id, BTreeSet::new()));
            self.tiles.len() - 1
        };
        &self.tiles[idx].1
    }

    /// `MapBuilder::getGridBounds` — `(minX, minY, maxX, maxY)`.
    fn get_grid_bounds(&self, map_id: u32) -> (u32, u32, u32, u32) {
        let mut max_x = i32::MAX as u32;
        let mut max_y = i32::MAX as u32;
        let mut min_x = i32::MIN as u32;
        let mut min_y = i32::MIN as u32;

        let mut bmin = [0f32; 3];
        let mut bmax = [0f32; 3];
        let mut lmin = [0f32; 3];
        let mut lmax = [0f32; 3];
        let mut mesh_data = MeshData::default();

        // make sure we process maps which don't have tiles
        // initialize the static tree, which loads WDT models
        if !self
            .terrain_builder
            .load_vmap(map_id, 64, 64, &mut mesh_data)
        {
            return (min_x, min_y, max_x, max_y);
        }

        // get the coord bounds of the model data
        let (sv, lv) = (&mesh_data.solid_verts, &mesh_data.liquid_verts);
        if sv.len() + lv.len() == 0 {
            return (min_x, min_y, max_x, max_y);
        }

        // get the coord bounds of the model data
        if !sv.is_empty() && !lv.is_empty() {
            recast::rc_calc_bounds(sv, (sv.len() / 3) as i32, &mut bmin, &mut bmax);
            recast::rc_calc_bounds(lv, (lv.len() / 3) as i32, &mut lmin, &mut lmax);
            for k in 0..3 {
                bmin[k] = bmin[k].min(lmin[k]); // rcVmin
                bmax[k] = bmax[k].max(lmax[k]); // rcVmax
            }
        } else if !sv.is_empty() {
            recast::rc_calc_bounds(sv, (sv.len() / 3) as i32, &mut bmin, &mut bmax);
        } else {
            // C++ computes the liquid bounds into lmin/lmax and then uses the
            // untouched (zero) bmin/bmax below.
            recast::rc_calc_bounds(lv, (lv.len() / 3) as i32, &mut lmin, &mut lmax);
        }

        // convert coord bounds to grid bounds (float -> uint32 as x86-64 does)
        let to_u32 = |f: f32| (f as i64) as u32;
        max_x = to_u32(32.0 - bmin[0] / GRID_SIZE);
        max_y = to_u32(32.0 - bmin[2] / GRID_SIZE);
        min_x = to_u32(32.0 - bmax[0] / GRID_SIZE);
        min_y = to_u32(32.0 - bmax[2] / GRID_SIZE);
        (min_x, min_y, max_x, max_y)
    }

    /// `MapBuilder::buildMeshFromFile` (`--file`).
    ///
    /// Deviation: the C++ function never cancels the queue its local
    /// `TileBuilder` worker waits on, so the process hangs when it returns;
    /// here it simply returns.
    pub fn build_mesh_from_file(&mut self, name: &str) {
        let Ok(bytes) = std::fs::read(name) else {
            return;
        };
        let mut pos = 0usize;
        let mut read_u32 = || -> Option<u32> {
            let b = bytes.get(pos..pos + 4)?;
            pos += 4;
            Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };

        println!("Building mesh from file");
        let Some(map_id) = read_u32() else { return };
        let Some(tile_x) = read_u32() else { return };
        let Some(tile_y) = read_u32() else { return };

        let Some(mut nav_mesh) = self.build_nav_mesh(map_id) else {
            println!("Failed creating navmesh!              ");
            return;
        };

        let Some(vertices_count) = read_u32() else {
            return;
        };
        let Some(indices_count) = read_u32() else {
            return;
        };

        let mut data = MeshData::default();
        for _ in 0..vertices_count {
            let Some(v) = read_u32() else { return };
            data.solid_verts.push(f32::from_bits(v));
        }
        for _ in 0..indices_count {
            let Some(v) = read_u32() else { return };
            data.solid_tris.push(v as i32);
        }

        TerrainBuilder::clean_vertices(&mut data.solid_verts, &mut data.solid_tris);
        // get bounds of current tile
        let (bmin, bmax) = Shared::get_tile_bounds(tile_x, tile_y, Some(&data.solid_verts));

        // build navmesh tile
        let tile_builder = TileBuilder::new(Arc::clone(&self.shared));
        tile_builder.build_move_map_tile(
            map_id,
            tile_x,
            tile_y,
            &mut data,
            &bmin,
            &bmax,
            &mut nav_mesh,
        );
    }

    /// `MapBuilder::buildSingleTile`.
    pub fn build_single_tile(&mut self, map_id: u32, tile_x: u32, tile_y: u32) {
        let Some(mut nav_mesh) = self.build_nav_mesh(map_id) else {
            println!("Failed creating navmesh!              ");
            return;
        };

        // ToDo: delete the old tile as the user clearly wants to rebuild it

        let tile_builder = TileBuilder::new(Arc::clone(&self.shared));
        tile_builder.build_tile(map_id, tile_x, tile_y, &mut nav_mesh);
    }

    /// `MapBuilder::buildMaps`.
    pub fn build_maps(&mut self, map_id: Option<u32>) {
        let threads = self.shared.opts.threads;
        println!("Using {threads} threads to generate mmaps");

        let queue = Arc::new(TileQueue::default());
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                let builder = TileBuilder::new(Arc::clone(&self.shared));
                let queue = Arc::clone(&queue);
                std::thread::spawn(move || builder.worker_thread(&queue))
            })
            .collect();

        if let Some(map_id) = map_id {
            self.build_map(map_id, &queue);
        } else {
            // Build all maps if no map id has been specified
            let ids: Vec<u32> = self.tiles.iter().map(|(id, _)| *id).collect();
            for id in ids {
                if !self.should_skip_map(id) {
                    self.build_map(id, &queue);
                }
            }
        }

        queue.close();
        for w in workers {
            let _ = w.join();
        }
    }

    /// `MapBuilder::buildMap`.
    fn build_map(&mut self, map_id: u32, queue: &TileQueue) {
        let tiles: Vec<u32> = self.get_tile_list(map_id).iter().copied().collect();
        if tiles.is_empty() {
            return;
        }

        // build navMesh
        let Some(nav_mesh) = self.build_nav_mesh(map_id) else {
            println!("[Map {:04}] Failed creating navmesh!", map_id as i32);
            self.shared
                .total_tiles_processed
                .fetch_add(tiles.len() as u32, Ordering::SeqCst);
            return;
        };

        // now start building mmtiles for each tile
        println!(
            "[Map {:04}] We have {} tiles.                          ",
            map_id as i32,
            tiles.len()
        );
        let params = nav_mesh.params();
        for id in tiles {
            // unpack tile coords
            let (tile_x, tile_y) = unpack_tile_id(id);
            queue.push(TileInfo {
                map_id,
                tile_x,
                tile_y,
                nav_mesh_params: params,
            });
        }
    }

    /// `MapBuilder::buildNavMesh` — computes the map's `dtNavMeshParams`
    /// (from the root parent map's tiles) and writes `mmaps/<map>.mmap`.
    fn build_nav_mesh(&mut self, map_id: u32) -> Option<NavMesh> {
        // if map has a parent we use that to generate dtNavMeshParams - worldserver will load all missing tiles from that map
        let mut nav_mesh_params_map_id = map_id as i32;
        let mut parent_map_id = self.shared.data.parent_map_id(map_id);
        while parent_map_id != -1 {
            nav_mesh_params_map_id = parent_map_id;
            parent_map_id = self.shared.data.parent_map_id(parent_map_id as u32);
        }

        let tiles = self.get_tile_list(nav_mesh_params_map_id as u32).clone();

        let poly_bits = DT_POLY_BITS;
        let max_tiles = tiles.len() as i32;
        let max_polys_per_tile = 1i32.wrapping_shl(poly_bits);

        /***          calculate bounds of map         ***/
        let (mut tile_x_min, mut tile_y_min, mut tile_x_max, mut tile_y_max) =
            (64u32, 64u32, 0u32, 0u32);
        for id in &tiles {
            let (tile_x, tile_y) = unpack_tile_id(*id);
            if tile_x > tile_x_max {
                tile_x_max = tile_x;
            } else if tile_x < tile_x_min {
                tile_x_min = tile_x;
            }
            if tile_y > tile_y_max {
                tile_y_max = tile_y;
            } else if tile_y < tile_y_min {
                tile_y_min = tile_y;
            }
        }
        let _ = (tile_x_min, tile_y_min);

        // use Max because '32 - tileX' is negative for values over 32
        let (bmin, _bmax) = Shared::get_tile_bounds(tile_x_max, tile_y_max, None);

        /***       now create the navmesh       ***/

        // navmesh creation params
        let nav_mesh_params = DtNavMeshParams {
            orig: bmin,
            tile_width: GRID_SIZE,
            tile_height: GRID_SIZE,
            max_tiles,
            max_polys: max_polys_per_tile,
        };

        let mut nav_mesh = NavMesh::alloc();
        println!("[Map {map_id:04}] Creating navMesh...");
        if nav_mesh.init(&nav_mesh_params) == 0 {
            println!("[Map {map_id:04}] Failed creating navmesh!                ");
            return Some(nav_mesh);
        }

        let file_name = format!("mmaps/{map_id:04}.mmap");
        let mut file = match std::fs::File::create(self.base().join(&file_name)) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[Map {map_id:04}] Failed to open {file_name} for writing!\n: {e}");
                return None;
            }
        };

        // now that we know navMesh params are valid, we can write them to file
        let _ = file.write_all(&nav_mesh_params.to_bytes());
        Some(nav_mesh)
    }

    /// `MapBuilder::shouldSkipMap`.
    fn should_skip_map(&self, map_id: u32) -> bool {
        let o = &self.shared.opts;
        if o.mapid >= 0 {
            return o.mapid as u32 != map_id;
        }
        if o.skip_continents && is_continent_map(map_id) {
            return true;
        }
        if o.skip_junk_maps {
            if self.is_dev_map(map_id) {
                return true;
            }
            if self.is_transport_map(map_id) {
                return true;
            }
        }
        if o.skip_battlegrounds && self.is_battleground_map(map_id) {
            return true;
        }
        false
    }

    /// `MapBuilder::isTransportMap`.
    fn is_transport_map(&self, map_id: u32) -> bool {
        self.shared
            .data
            .map_store
            .get(&map_id)
            .is_some_and(|m| m.map_type == 3)
    }

    /// `MapBuilder::isDevMap`.
    fn is_dev_map(&self, map_id: u32) -> bool {
        self.shared
            .data
            .map_store
            .get(&map_id)
            .is_some_and(|m| m.flags & 0x2 != 0)
    }

    /// `MapBuilder::isBattlegroundMap`.
    fn is_battleground_map(&self, map_id: u32) -> bool {
        self.shared
            .data
            .map_store
            .get(&map_id)
            .is_some_and(|m| m.instance_type == 3)
    }

    /// Discovered maps and tiles (for tests).
    #[cfg(test)]
    fn tile_list(&self) -> &[(u32, BTreeSet<u32>)] {
        &self.tiles
    }
}

/// `MapBuilder::isContinentMap`.
pub fn is_continent_map(map_id: u32) -> bool {
    matches!(
        map_id,
        0 | 1 | 530 | 571 | 870 | 1116 | 1220 | 1642 | 1643 | 2222
    )
}

#[cfg(test)]
#[path = "map_builder_tests.rs"]
mod tests;
