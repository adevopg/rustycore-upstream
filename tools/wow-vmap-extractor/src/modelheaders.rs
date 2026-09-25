//! Port of `src/tools/vmap4_extractor/modelheaders.h` (`ModelHeader`, `#pragma pack(1)`).
//!
//! Only the fields the extractor reads are decoded; the byte offsets are those of the
//! packed C++ struct (the M2 `MD20` header): `id[4]`, `version[4]`, then 38 `uint32`
//! (`nameLength` .. `ofsTexAnimLookup`), `boundingBox` (0xA0), `boundingSphereRadius`,
//! `collisionBox` (0xBC), `collisionSphereRadius`, `nBoundingTriangles` (0xD8) ..
//! `ofsParticleEmitters`; `sizeof(ModelHeader) == 304`.

use crate::vec3d::{AaBox3D, u32_at};

/// `sizeof(ModelHeader)`.
pub const MODEL_HEADER_SIZE: usize = 304;

const OFS_COLLISION_BOX: usize = 0xBC;
const OFS_N_BOUNDING_TRIANGLES: usize = 0xD8;
const OFS_OFS_BOUNDING_TRIANGLES: usize = 0xDC;
const OFS_N_BOUNDING_VERTICES: usize = 0xE0;
const OFS_OFS_BOUNDING_VERTICES: usize = 0xE4;

/// The `ModelHeader` fields used by `Model::open` / `Model::ConvertToVMAPModel`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ModelHeader {
    pub id: [u8; 4],
    pub collision_box: AaBox3D,
    pub n_bounding_triangles: u32,
    pub ofs_bounding_triangles: u32,
    pub n_bounding_vertices: u32,
    pub ofs_bounding_vertices: u32,
}

impl ModelHeader {
    /// `memcpy(&header, buffer, sizeof(ModelHeader))`; bytes past the end of the file
    /// read as zero.
    pub fn from_bytes(src: &[u8]) -> Self {
        let mut b = [0u8; MODEL_HEADER_SIZE];
        let n = src.len().min(MODEL_HEADER_SIZE);
        b[..n].copy_from_slice(&src[..n]);
        Self {
            id: [b[0], b[1], b[2], b[3]],
            collision_box: AaBox3D::from_le(&b[OFS_COLLISION_BOX..OFS_COLLISION_BOX + 24]),
            n_bounding_triangles: u32_at(&b, OFS_N_BOUNDING_TRIANGLES),
            ofs_bounding_triangles: u32_at(&b, OFS_OFS_BOUNDING_TRIANGLES),
            n_bounding_vertices: u32_at(&b, OFS_N_BOUNDING_VERTICES),
            ofs_bounding_vertices: u32_at(&b, OFS_OFS_BOUNDING_VERTICES),
        }
    }
}
