//! `GameObjectModels.dtree` — the list written by
//! `TileAssembler::exportGameobjectModels` and read by
//! `LoadGameObjectModelList` (`src/common/Collision/Models/GameObjectModel.cpp`).

use std::collections::HashMap;

use crate::definitions::VMAP_MAGIC;
use crate::error::{Result, VmapError};
use crate::io::{Reader, Writer};
use crate::math::Vector3;
use crate::model_instance::bytes_to_string;

/// `name_length >= sizeof(buff)` limit (`char buff[500]`).
pub const MAX_MODEL_NAME_LEN: u32 = 500;

/// One record of `GameObjectModels.dtree` (`GameobjectModelData`).
#[derive(Debug, Clone, PartialEq)]
pub struct GameObjectModelEntry {
    pub display_id: u32,
    pub is_wmo: bool,
    pub name: String,
    pub bound_low: Vector3,
    pub bound_high: Vector3,
}

impl GameObjectModelEntry {
    /// Record layout written by `exportGameobjectModels`
    /// (`displayId, isWmo, name_length, name, low, high`).
    pub fn write_to(&self, w: &mut impl Writer) {
        w.put_u32(self.display_id);
        w.put_u8(u8::from(self.is_wmo));
        w.put_u32(self.name.len() as u32);
        w.put_bytes(self.name.as_bytes());
        w.put_vector3(self.bound_low);
        w.put_vector3(self.bound_high);
    }
}

/// `LoadGameObjectModelList` — parses the file. Like the C++, a corrupted
/// record stops parsing (keeping earlier records), entries with NaN bounds
/// are skipped and the first entry of a duplicated display id wins.
pub fn read_game_object_models(data: &[u8]) -> Result<HashMap<u32, GameObjectModelEntry>> {
    let mut r = Reader::new(data);
    if !r.chunk(VMAP_MAGIC) {
        return Err(VmapError::format("GameObjectModels.dtree header"));
    }
    let mut out = HashMap::new();
    // a short read of the display id is the end of file
    while let Some(display_id) = r.u32() {
        let Some(is_wmo) = r.u8() else { break };
        let Some(name_length) = r.u32() else { break };
        if name_length >= MAX_MODEL_NAME_LEN {
            break;
        }
        let Some(name) = r.bytes(name_length as usize) else {
            break;
        };
        let Some(v1) = r.vector3() else { break };
        let Some(v2) = r.vector3() else { break };
        if v1.is_nan() || v2.is_nan() {
            continue;
        }
        out.entry(display_id).or_insert(GameObjectModelEntry {
            display_id,
            is_wmo: is_wmo != 0,
            name: bytes_to_string(name),
            bound_low: v1,
            bound_high: v2,
        });
    }
    Ok(out)
}
