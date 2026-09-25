//! `StaticMapTree` and the loading half of `VMapManager2` — port of
//! `src/common/Collision/Maps/MapTree.{h,cpp}` (`InitMap`, `LoadMapTile`,
//! `UnloadMapTile`, `UnloadMap`, `getModelInstances`, `OpenMapTileFile`)
//! and `src/common/Collision/Management/VMapManager2.cpp` (`loadMap`,
//! `unloadMap`, `acquireModelInstance`, `releaseModelInstance`,
//! `InitializeThreadUnsafe`, `getParentMapId`).
//!
//! This is the access path used by `TerrainBuilder::loadVMap` in the
//! mmaps generator. Collision queries (line of sight, height, area info) are not
//! ported.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::bih::Bih;
use crate::definitions::{VMAP_MAGIC, VMO_EXTENSION, map_file_name, pack_tile_id, tile_file_name};
use crate::io::Reader;
use crate::model_instance::{ModelInstance, ModelSpawn};
use crate::world_model::WorldModel;

/// `VMAP::LoadResult` (IVMapManager.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadResult {
    Success,
    FileNotFound,
    VersionMismatch,
    ReadFromFileFailed,
    DisabledInConfig,
}

/// Model cache and parent-map table of `VMapManager2`
/// (`iLoadedModelFiles`, `iParentMapData`).
#[derive(Debug, Default)]
pub struct ModelRegistry {
    loaded_model_files: HashMap<String, (Arc<WorldModel>, i32)>,
    parent_map_data: HashMap<u32, u32>,
}

impl ModelRegistry {
    /// `VMapManager2::acquireModelInstance` — loads `<basepath><filename>.vmo`
    /// once and reference-counts it. `flags` is only used on first load.
    pub fn acquire_model_instance(
        &mut self,
        base_path: &Path,
        filename: &str,
        flags: u32,
    ) -> Option<Arc<WorldModel>> {
        if let Some((model, refs)) = self.loaded_model_files.get_mut(filename) {
            *refs += 1;
            return Some(Arc::clone(model));
        }
        let path = base_path.join(format!("{filename}{VMO_EXTENSION}"));
        let mut model = WorldModel::read_file(&path).ok()?;
        model.set_name(filename.to_owned());
        model.flags = flags;
        let model = Arc::new(model);
        self.loaded_model_files
            .insert(filename.to_owned(), (Arc::clone(&model), 1));
        Some(model)
    }

    /// `VMapManager2::releaseModelInstance`.
    pub fn release_model_instance(&mut self, filename: &str) {
        if let Some((_, refs)) = self.loaded_model_files.get_mut(filename) {
            *refs -= 1;
            if *refs == 0 {
                self.loaded_model_files.remove(filename);
            }
        }
    }

    /// Number of distinct `.vmo` files currently loaded.
    pub fn loaded_model_count(&self) -> usize {
        self.loaded_model_files.len()
    }

    /// `VMapManager2::getParentMapId` (`-1` when there is no parent).
    pub fn parent_map_id(&self, map_id: u32) -> i32 {
        self.parent_map_data.get(&map_id).map_or(-1, |&p| p as i32)
    }
}

/// `StaticMapTree::TileFileOpenResult`.
struct TileFileOpenResult {
    data: Option<Vec<u8>>,
    used_map_id: i32,
}

/// `StaticMapTree` — the map-wide spawn BIH (`.vmtree`) plus the model
/// instances of the loaded tiles (`.vmtile`).
#[derive(Debug)]
pub struct StaticMapTree {
    map_id: u32,
    tree: Bih,
    /// `iTreeValues` (`None` until `InitMap` read the tree successfully).
    tree_values: Option<Vec<ModelInstance>>,
    spawn_indices: HashMap<u32, u32>,
    loaded_tiles: HashMap<u32, bool>,
    loaded_spawns: HashMap<u32, u32>,
    base_path: PathBuf,
}

impl StaticMapTree {
    /// `StaticMapTree(mapID, basePath)`.
    pub fn new(map_id: u32, base_path: impl Into<PathBuf>) -> Self {
        Self {
            map_id,
            tree: Bih::default(),
            tree_values: None,
            spawn_indices: HashMap::new(),
            loaded_tiles: HashMap::new(),
            loaded_spawns: HashMap::new(),
            base_path: base_path.into(),
        }
    }

    pub fn map_id(&self) -> u32 {
        self.map_id
    }

    /// The map spawn BIH read from `NODE`.
    pub fn tree(&self) -> &Bih {
        &self.tree
    }

    /// `SIDX` spawn id -> tree slot.
    pub fn spawn_indices(&self) -> &HashMap<u32, u32> {
        &self.spawn_indices
    }

    /// `StaticMapTree::numLoadedTiles`.
    pub fn num_loaded_tiles(&self) -> u32 {
        self.loaded_tiles.len() as u32
    }

    /// `StaticMapTree::getModelInstances` — one slot per tree primitive;
    /// slots whose spawn is not in a loaded tile have no world model.
    pub fn model_instances(&self) -> &[ModelInstance] {
        self.tree_values.as_deref().unwrap_or(&[])
    }

    /// `StaticMapTree::InitMap(fname)` — reads `<basePath><fname>`.
    pub fn init_map(&mut self, fname: &str) -> LoadResult {
        let Ok(data) = std::fs::read(self.base_path.join(fname)) else {
            return LoadResult::FileNotFound;
        };
        self.init_map_from_bytes(&data)
    }

    /// `InitMap` on an in-memory `.vmtree`.
    pub fn init_map_from_bytes(&mut self, data: &[u8]) -> LoadResult {
        let mut r = Reader::new(data);
        let mut result = LoadResult::Success;
        if !r.chunk(VMAP_MAGIC) {
            result = LoadResult::VersionMismatch;
        }
        if result == LoadResult::Success
            && r.chunk(b"NODE")
            && let Ok(tree) = Bih::read_from(&mut r)
        {
            self.tree = tree;
            let n = self.tree.prim_count() as usize;
            self.tree_values = Some(vec![ModelInstance::default(); n]);
        }
        if result == LoadResult::Success {
            result = if r.chunk(b"SIDX") {
                LoadResult::Success
            } else {
                LoadResult::ReadFromFileFailed
            };
            let mut spawn_indices_size = 0;
            if result == LoadResult::Success {
                match r.u32() {
                    Some(n) => spawn_indices_size = n,
                    None => result = LoadResult::ReadFromFileFailed,
                }
            }
            let mut i = 0;
            while i < spawn_indices_size && result == LoadResult::Success {
                match r.u32() {
                    Some(spawn_id) => {
                        self.spawn_indices.insert(spawn_id, i);
                    }
                    None => result = LoadResult::ReadFromFileFailed,
                }
                i += 1;
            }
        }
        result
    }

    /// `StaticMapTree::OpenMapTileFile` — falls back to parent maps.
    fn open_map_tile_file(
        base_path: &Path,
        map_id: u32,
        tile_x: u32,
        tile_y: u32,
        vm: &ModelRegistry,
    ) -> TileFileOpenResult {
        let mut name = base_path.join(tile_file_name(map_id, tile_x, tile_y));
        let mut data = std::fs::read(&name).ok();
        let mut used_map_id = map_id as i32;
        if data.is_none() {
            let mut parent = vm.parent_map_id(map_id);
            while parent != -1 {
                name = base_path.join(tile_file_name(parent as u32, tile_x, tile_y));
                data = std::fs::read(&name).ok();
                used_map_id = parent;
                if data.is_some() {
                    break;
                }
                parent = vm.parent_map_id(parent as u32);
            }
        }
        TileFileOpenResult { data, used_map_id }
    }

    /// `StaticMapTree::LoadMapTile`.
    pub fn load_map_tile(
        &mut self,
        tile_x: u32,
        tile_y: u32,
        vm: &mut ModelRegistry,
    ) -> LoadResult {
        let Some(tree_values) = self.tree_values.as_mut() else {
            return LoadResult::ReadFromFileFailed;
        };
        let mut result = LoadResult::FileNotFound;
        let file = Self::open_map_tile_file(&self.base_path, self.map_id, tile_x, tile_y, vm);
        if let Some(data) = &file.data {
            let mut r = Reader::new(data);
            result = LoadResult::Success;
            if !r.chunk(VMAP_MAGIC) {
                result = LoadResult::VersionMismatch;
            }
            let mut num_spawns = 0;
            if result == LoadResult::Success {
                match r.u32() {
                    Some(n) => num_spawns = n,
                    None => result = LoadResult::ReadFromFileFailed,
                }
            }
            let mut i = 0;
            while i < num_spawns && result == LoadResult::Success {
                i += 1;
                let Ok(Some(spawn)) = ModelSpawn::read_from(&mut r) else {
                    result = LoadResult::ReadFromFileFailed;
                    continue;
                };
                // acquire model instance
                let model =
                    vm.acquire_model_instance(&self.base_path, &spawn.name, u32::from(spawn.flags));
                // update tree
                if let Some(&referenced_val) = self.spawn_indices.get(&spawn.id) {
                    if let Some(refs) = self.loaded_spawns.get_mut(&referenced_val) {
                        *refs += 1;
                    } else {
                        if referenced_val as usize >= tree_values.len() {
                            // invalid tree element referenced in tile
                            continue;
                        }
                        tree_values[referenced_val as usize] = ModelInstance::new(&spawn, model);
                        self.loaded_spawns.insert(referenced_val, 1);
                    }
                } else if self.map_id as i32 == file.used_map_id {
                    // unknown spawn in this map's own tile file
                    result = LoadResult::ReadFromFileFailed;
                }
            }
            self.loaded_tiles.insert(pack_tile_id(tile_x, tile_y), true);
        } else {
            self.loaded_tiles
                .insert(pack_tile_id(tile_x, tile_y), false);
        }
        result
    }

    /// `StaticMapTree::UnloadMapTile`.
    pub fn unload_map_tile(&mut self, tile_x: u32, tile_y: u32, vm: &mut ModelRegistry) {
        let tile_id = pack_tile_id(tile_x, tile_y);
        let Some(&has_file) = self.loaded_tiles.get(&tile_id) else {
            return;
        };
        if has_file {
            let file = Self::open_map_tile_file(&self.base_path, self.map_id, tile_x, tile_y, vm);
            if let Some(data) = &file.data {
                let mut r = Reader::new(data);
                let mut result = r.chunk(VMAP_MAGIC);
                let num_spawns = r.u32();
                if num_spawns.is_none() {
                    result = false;
                }
                let num_spawns = num_spawns.unwrap_or(0);
                let mut i = 0;
                while i < num_spawns && result {
                    i += 1;
                    let Ok(Some(spawn)) = ModelSpawn::read_from(&mut r) else {
                        result = false;
                        continue;
                    };
                    vm.release_model_instance(&spawn.name);
                    if let Some(&node) = self.spawn_indices.get(&spawn.id) {
                        if let Some(refs) = self.loaded_spawns.get_mut(&node) {
                            *refs -= 1;
                            if *refs == 0 {
                                if let Some(v) = self
                                    .tree_values
                                    .as_mut()
                                    .and_then(|t| t.get_mut(node as usize))
                                {
                                    v.set_unloaded();
                                }
                                self.loaded_spawns.remove(&node);
                            }
                        }
                    } else if self.map_id as i32 == file.used_map_id {
                        result = false;
                    }
                }
            }
        }
        self.loaded_tiles.remove(&tile_id);
    }

    /// `StaticMapTree::UnloadMap`.
    pub fn unload_map(&mut self, vm: &mut ModelRegistry) {
        for (&idx, &refs) in &self.loaded_spawns {
            if let Some(v) = self
                .tree_values
                .as_mut()
                .and_then(|t| t.get_mut(idx as usize))
            {
                if let Some(model) = v.world_model() {
                    let name = model.name().to_owned();
                    for _ in 0..refs {
                        vm.release_model_instance(&name);
                    }
                }
                v.set_unloaded();
            }
        }
        self.loaded_spawns.clear();
        self.loaded_tiles.clear();
    }
}

/// Loading part of `VMapManager2`: owns one `StaticMapTree` per map and the
/// shared model cache.
#[derive(Debug, Default)]
pub struct VMapManager {
    instance_map_trees: HashMap<u32, Option<StaticMapTree>>,
    registry: ModelRegistry,
}

impl VMapManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// `VMapManager2::InitializeThreadUnsafe` — registers the map ids and
    /// their child maps (used for parent-map tile fallback).
    pub fn initialize_thread_unsafe(&mut self, map_data: &HashMap<u32, Vec<u32>>) {
        for (&map_id, children) in map_data {
            self.instance_map_trees.entry(map_id).or_insert(None);
            for &child in children {
                self.registry.parent_map_data.insert(child, map_id);
            }
        }
    }

    /// `VMapManager2::loadMap(basePath, mapId, x, y)`.
    pub fn load_map(
        &mut self,
        base_path: impl AsRef<Path>,
        map_id: u32,
        x: u32,
        y: u32,
    ) -> LoadResult {
        let slot = self.instance_map_trees.entry(map_id).or_insert(None);
        if slot.is_none() {
            let mut tree = StaticMapTree::new(map_id, base_path.as_ref());
            let init = tree.init_map(&map_file_name(map_id));
            if init != LoadResult::Success {
                return init;
            }
            *slot = Some(tree);
        }
        slot.as_mut()
            .expect("tree initialised above")
            .load_map_tile(x, y, &mut self.registry)
    }

    /// `VMapManager2::unloadMap(mapId, x, y)`.
    pub fn unload_map_tile(&mut self, map_id: u32, x: u32, y: u32) {
        if let Some(slot) = self.instance_map_trees.get_mut(&map_id)
            && let Some(tree) = slot.as_mut()
        {
            tree.unload_map_tile(x, y, &mut self.registry);
            if tree.num_loaded_tiles() == 0 {
                *slot = None;
            }
        }
    }

    /// `VMapManager2::unloadMap(mapId)`.
    pub fn unload_map(&mut self, map_id: u32) {
        if let Some(slot) = self.instance_map_trees.get_mut(&map_id)
            && let Some(tree) = slot.as_mut()
        {
            tree.unload_map(&mut self.registry);
            if tree.num_loaded_tiles() == 0 {
                *slot = None;
            }
        }
    }

    /// `getInstanceMapTree()[mapId]` — the loaded tree of a map.
    pub fn map_tree(&self, map_id: u32) -> Option<&StaticMapTree> {
        self.instance_map_trees
            .get(&map_id)
            .and_then(Option::as_ref)
    }

    pub fn registry(&self) -> &ModelRegistry {
        &self.registry
    }
}
