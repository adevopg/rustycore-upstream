//! `ModelSpawn`, `ModelMinimalData`, `ModelFlags` and `ModelInstance` —
//! port of `src/common/Collision/Models/ModelInstance.{h,cpp}` (file I/O and
//! transform setup; collision queries are not ported).

use std::sync::Arc;

use crate::error::{OrFormat, Result, VmapError};
use crate::io::{Reader, Writer};
use crate::math::{AABox, Matrix3, PIF, Vector3};
use crate::world_model::WorldModel;

/// `ModelFlags::MOD_M2`.
pub const MOD_M2: u8 = 1;
/// `ModelFlags::MOD_HAS_BOUND`.
pub const MOD_HAS_BOUND: u8 = 1 << 1;
/// `ModelFlags::MOD_PARENT_SPAWN`.
pub const MOD_PARENT_SPAWN: u8 = 1 << 2;

/// Maximum accepted name length in `ModelSpawn::readFromFile` (`nameBuff[500]`).
const MAX_NAME_LEN: u32 = 500;

/// `ModelSpawn` (`ModelMinimalData` + `iRot` + `name`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelSpawn {
    pub flags: u8,
    pub adt_id: u8,
    pub id: u32,
    pub pos: Vector3,
    pub rot: Vector3,
    pub scale: f32,
    pub bound: AABox,
    pub name: String,
}

impl ModelSpawn {
    /// `ModelMinimalData::getBounds`.
    pub fn bounds(&self) -> &AABox {
        &self.bound
    }

    /// `ModelSpawn::readFromFile`. Returns `Ok(None)` on a clean EOF before
    /// the first byte (the C++ returns `false` without error output there).
    pub fn read_from(r: &mut Reader<'_>) -> Result<Option<Self>> {
        let Some(flags) = r.u8() else {
            return Ok(None);
        };
        let what = "ModelSpawn";
        let adt_id = r.u8().or_format(what)?;
        let id = r.u32().or_format(what)?;
        let pos = r.vector3().or_format(what)?;
        let rot = r.vector3().or_format(what)?;
        let scale = r.f32().or_format(what)?;
        let mut bound = AABox::EMPTY;
        if flags & MOD_HAS_BOUND != 0 {
            let lo = r.vector3().or_format(what)?;
            let hi = r.vector3().or_format(what)?;
            bound = AABox::new(lo, hi);
        }
        let name_len = r.u32().or_format(what)?;
        if name_len > MAX_NAME_LEN {
            return Err(VmapError::format("ModelSpawn name too long"));
        }
        let name = r.bytes(name_len as usize).or_format(what)?;
        Ok(Some(Self {
            flags,
            adt_id,
            id,
            pos,
            rot,
            scale,
            bound,
            name: bytes_to_string(name),
        }))
    }

    /// `ModelSpawn::writeToFile`.
    pub fn write_to(&self, w: &mut impl Writer) {
        w.put_u8(self.flags);
        w.put_u8(self.adt_id);
        w.put_u32(self.id);
        w.put_vector3(self.pos);
        w.put_vector3(self.rot);
        w.put_f32(self.scale);
        if self.flags & MOD_HAS_BOUND != 0 {
            w.put_vector3(self.bound.low());
            w.put_vector3(self.bound.high());
        }
        w.put_u32(self.name.len() as u32);
        w.put_bytes(self.name.as_bytes());
    }
}

/// Model names are raw bytes in C++ (`std::string(nameBuff, nameLen)`); they
/// are ASCII paths in practice. Invalid UTF-8 is replaced lossily.
pub(crate) fn bytes_to_string(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// `ModelInstance` — a spawn placed in a `StaticMapTree` slot, with its
/// inverse rotation/scale and the shared `WorldModel`.
#[derive(Debug, Clone, Default)]
pub struct ModelInstance {
    pub flags: u8,
    pub adt_id: u8,
    pub id: u32,
    pub pos: Vector3,
    pub scale: f32,
    pub bound: AABox,
    /// Model file name (only kept under `VMAP_DEBUG` in C++).
    pub name: String,
    inv_rot: Matrix3,
    inv_scale: f32,
    model: Option<Arc<WorldModel>>,
}

impl ModelInstance {
    /// `ModelInstance::ModelInstance(ModelSpawn const&, WorldModel*)`.
    pub fn new(spawn: &ModelSpawn, model: Option<Arc<WorldModel>>) -> Self {
        let inv_rot = Matrix3::from_euler_angles_zyx(
            PIF * spawn.rot.y / 180.0,
            PIF * spawn.rot.x / 180.0,
            PIF * spawn.rot.z / 180.0,
        )
        .inverse();
        Self {
            flags: spawn.flags,
            adt_id: spawn.adt_id,
            id: spawn.id,
            pos: spawn.pos,
            scale: spawn.scale,
            bound: spawn.bound,
            name: spawn.name.clone(),
            inv_rot,
            inv_scale: 1.0 / spawn.scale,
            model,
        }
    }

    /// `ModelInstance::setUnloaded`.
    pub fn set_unloaded(&mut self) {
        self.model = None;
    }

    /// `ModelInstance::GetInvRot`.
    pub fn inv_rot(&self) -> &Matrix3 {
        &self.inv_rot
    }

    /// `iInvScale`.
    pub fn inv_scale(&self) -> f32 {
        self.inv_scale
    }

    /// `ModelInstance::getWorldModel` (`None` for unloaded tree slots).
    pub fn world_model(&self) -> Option<&Arc<WorldModel>> {
        self.model.as_ref()
    }

    /// `ModelMinimalData::getBounds`.
    pub fn bounds(&self) -> &AABox {
        &self.bound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(flags: u8) -> ModelSpawn {
        ModelSpawn {
            flags,
            adt_id: 7,
            id: 0xDEAD_BEEF,
            pos: Vector3::new(1.0, 2.0, 3.0),
            rot: Vector3::new(10.0, 20.0, 30.0),
            scale: 1.5,
            bound: if flags & MOD_HAS_BOUND != 0 {
                AABox::new(Vector3::new(-1.0, -2.0, -3.0), Vector3::new(4.0, 5.0, 6.0))
            } else {
                AABox::EMPTY
            },
            name: "World/wmo/test.wmo".into(),
        }
    }

    #[test]
    fn roundtrip_with_and_without_bound() {
        for flags in [MOD_HAS_BOUND, MOD_M2, MOD_HAS_BOUND | MOD_PARENT_SPAWN] {
            let s = sample(flags);
            let mut buf = Vec::new();
            s.write_to(&mut buf);
            let expected_len = 1
                + 1
                + 4
                + 12
                + 12
                + 4
                + if flags & MOD_HAS_BOUND != 0 { 24 } else { 0 }
                + 4
                + s.name.len();
            assert_eq!(buf.len(), expected_len);
            let mut r = Reader::new(&buf);
            let back = ModelSpawn::read_from(&mut r).unwrap().unwrap();
            if flags & MOD_HAS_BOUND != 0 {
                assert_eq!(back, s);
            } else {
                assert!(back.bound.is_empty());
                assert_eq!(back.name, s.name);
            }
            assert!(ModelSpawn::read_from(&mut r).unwrap().is_none());
        }
    }

    #[test]
    fn truncated_and_too_long_names_fail() {
        let mut buf = Vec::new();
        sample(MOD_HAS_BOUND).write_to(&mut buf);
        buf.pop();
        assert!(ModelSpawn::read_from(&mut Reader::new(&buf)).is_err());

        let mut s = sample(MOD_M2);
        s.name = "x".repeat(501);
        let mut buf = Vec::new();
        s.write_to(&mut buf);
        assert!(ModelSpawn::read_from(&mut Reader::new(&buf)).is_err());
    }

    #[test]
    fn instance_inverse_rotation() {
        let s = sample(MOD_HAS_BOUND);
        let inst = ModelInstance::new(&s, None);
        let rot = Matrix3::from_euler_angles_zyx(
            PIF * 20.0 / 180.0,
            PIF * 10.0 / 180.0,
            PIF * 30.0 / 180.0,
        );
        let v = Vector3::new(3.0, -1.0, 2.0);
        assert!((*inst.inv_rot() * (rot * v) - v).magnitude() < 1e-5);
        assert!((inst.inv_scale() - 1.0 / 1.5).abs() < f32::EPSILON);
        assert!(inst.world_model().is_none());
    }
}
