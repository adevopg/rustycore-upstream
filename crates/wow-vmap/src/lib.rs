//! VMAP collision file formats — port of `TrinityCore`'s
//! `src/common/Collision` (tag TDB343.24081, client 3.4.3.54261).
//!
//! * Raw intermediate model files in `Buildings/` ([`raw`]).
//! * Runtime files in `vmaps/`: `NNNN.vmtree` (map spawn BIH + spawn index),
//!   `NNNN_YY_XX.vmtile` (tile spawns), `<model>.vmo` ([`WorldModel`]) and
//!   `GameObjectModels.dtree` ([`gameobject_models`]).
//! * The bit-exact BIH builder ([`Bih`]) and the G3D math it needs ([`math`]).
//! * Loading API equivalent to `VMapManager2::loadMap` +
//!   `StaticMapTree::getModelInstances` ([`VMapManager`], [`StaticMapTree`]).
//!
//! All formats are little-endian with the C++ struct packing and write order.

// Integer/float conversions intentionally mirror the C++ (`int`/`uint32`
// reinterpretation, `float(uint32)`), and the BIH/assembler math must keep
// the exact C++ float expressions (e.g. `(a + b) * 0.5f`, not `midpoint`).
#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::manual_midpoint,
    clippy::excessive_precision,
    clippy::too_many_lines
)]
#![cfg_attr(test, allow(clippy::float_cmp))]

pub mod bih;
pub mod definitions;
pub mod error;
pub mod gameobject_models;
pub mod io;
pub mod map_tree;
pub mod math;
pub mod model_instance;
pub mod raw;
pub mod world_model;

pub use bih::{Bih, BihBuildError};
pub use definitions::{
    GAMEOBJECT_MODELS, LIQUID_TILE_SIZE, RAW_VMAP_MAGIC, VMAP_MAGIC, map_file_name, pack_tile_id,
    tile_file_name, unpack_tile_id,
};
pub use error::{Result, VmapError};
pub use map_tree::{LoadResult, ModelRegistry, StaticMapTree, VMapManager};
pub use math::{AABox, Matrix3, Ray, Vector3};
pub use model_instance::{MOD_HAS_BOUND, MOD_M2, MOD_PARENT_SPAWN, ModelInstance, ModelSpawn};
pub use raw::{GroupModelRaw, WorldModelRaw};
pub use world_model::{GroupModel, MeshTriangle, WmoLiquid, WorldModel};

#[cfg(test)]
mod tests;
