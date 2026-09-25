//! Constants and file-name helpers from `VMapDefinitions.h`,
//! `VMapManager2.{h,cpp}` (`getMapFileName`) and `MapTree.{h,cpp}`
//! (`getTileFileName`, `packTileID`, `unpackTileID`).

/// `VMAP_MAGIC` — header of runtime `.vmtree`/`.vmtile`/`.vmo`/`.dtree` files
/// (written with `fwrite(VMAP_MAGIC, 1, 8)`, i.e. without the terminator).
pub const VMAP_MAGIC: &[u8; 8] = b"VMAP_4.B";

/// `RAW_VMAP_MAGIC` — header of raw extractor output in `Buildings/`.
/// The C++ array is `"VMAP04B"` plus its NUL terminator: 8 bytes on disk.
pub const RAW_VMAP_MAGIC: &[u8; 8] = b"VMAP04B\0";

/// `GAMEOBJECT_MODELS` — game object model list written into the vmaps dir.
pub const GAMEOBJECT_MODELS: &str = "GameObjectModels.dtree";

/// `LIQUID_TILE_SIZE` — `533.333f / 128.f`.
pub const LIQUID_TILE_SIZE: f32 = 533.333_f32 / 128.0_f32;

/// `MAP_FILENAME_EXTENSION2` (VMapManager2.h).
pub const MAP_FILENAME_EXTENSION2: &str = ".vmtree";

/// Raw-directory list of all map spawns (`TileAssembler::readMapSpawns`).
pub const DIR_BIN: &str = "dir_bin";

/// Raw game object model list (`TileAssembler::exportGameobjectModels`).
pub const TEMP_GAMEOBJECT_MODELS: &str = "temp_gameobject_models";

/// Extension appended to model names for converted `WorldModel` files.
pub const VMO_EXTENSION: &str = ".vmo";

/// `VMapManager2::getMapFileName` — `"%04u.vmtree"`.
pub fn map_file_name(map_id: u32) -> String {
    format!("{map_id:04}{MAP_FILENAME_EXTENSION2}")
}

/// `StaticMapTree::getTileFileName` — `"%04u_%02u_%02u.vmtile"` with **Y
/// before X**, as in the C++.
pub fn tile_file_name(map_id: u32, tile_x: u32, tile_y: u32) -> String {
    format!("{map_id:04}_{tile_y:02}_{tile_x:02}.vmtile")
}

/// `StaticMapTree::packTileID` — `tileX << 16 | tileY` (wrapping `u32`).
pub fn pack_tile_id(tile_x: u32, tile_y: u32) -> u32 {
    (tile_x << 16) | tile_y
}

/// `StaticMapTree::unpackTileID` — note the C++ masks Y with `0xFF`.
pub fn unpack_tile_id(id: u32) -> (u32, u32) {
    (id >> 16, id & 0xFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_match_cpp_formats() {
        assert_eq!(map_file_name(1), "0001.vmtree");
        assert_eq!(map_file_name(12345), "12345.vmtree");
        assert_eq!(tile_file_name(530, 3, 45), "0530_45_03.vmtile");
    }

    #[test]
    fn tile_id_packing() {
        assert_eq!(pack_tile_id(32, 48), 0x0020_0030);
        assert_eq!(unpack_tile_id(pack_tile_id(32, 48)), (32, 48));
        assert_eq!(unpack_tile_id(pack_tile_id(1, 0x1FF)), (1, 0xFF));
    }
}
