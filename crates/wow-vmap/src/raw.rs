//! Raw intermediate model format (`Buildings/<model>` files written by
//! `vmap4_extractor`) — port of `GroupModel_Raw::Read` and
//! `WorldModel_Raw::Read` from `src/tools/vmap4_assembler/TileAssembler.cpp`.
//!
//! The writer side mirrors `WMORoot::ConvertToVMAPRootWmo` and
//! `WMOGroup::ConvertToVMAPGroupWmo` (`src/tools/vmap4_extractor/wmo.cpp`)
//! and is provided for tests and tooling.

use std::path::Path;

use crate::definitions::RAW_VMAP_MAGIC;
use crate::error::{OrFormat, Result, VmapError};
use crate::io::{Reader, Writer};
use crate::math::{AABox, Vector3};
use crate::world_model::{MeshTriangle, WmoLiquid};

/// Packed size of `WMOLiquidHeader` (`#pragma pack(1)`: 4×i32, 3×f32, i16).
pub const WMO_LIQUID_HEADER_SIZE: usize = 30;

/// `WMOLiquidHeader` (TileAssembler.cpp / extractor wmo.h), packed.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WmoLiquidHeader {
    pub xverts: i32,
    pub yverts: i32,
    pub xtiles: i32,
    pub ytiles: i32,
    pub pos_x: f32,
    pub pos_y: f32,
    pub pos_z: f32,
    pub material: i16,
}

impl WmoLiquidHeader {
    fn read_from(r: &mut Reader<'_>) -> Option<Self> {
        // read as one 30-byte block like `fread(&hlq, sizeof(WMOLiquidHeader), 1)`
        let b = r.bytes(WMO_LIQUID_HEADER_SIZE)?;
        let mut h = Reader::new(b);
        Some(Self {
            xverts: h.i32()?,
            yverts: h.i32()?,
            xtiles: h.i32()?,
            ytiles: h.i32()?,
            pos_x: h.f32()?,
            pos_y: h.f32()?,
            pos_z: h.f32()?,
            material: h.i16()?,
        })
    }

    fn write_to(&self, w: &mut impl Writer) {
        w.put_i32(self.xverts);
        w.put_i32(self.yverts);
        w.put_i32(self.xtiles);
        w.put_i32(self.ytiles);
        w.put_f32(self.pos_x);
        w.put_f32(self.pos_y);
        w.put_f32(self.pos_z);
        w.put_i16(self.material);
    }
}

/// `GroupModel_Raw`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroupModelRaw {
    pub mogp_flags: u32,
    pub group_wmo_id: u32,
    pub bounds: AABox,
    pub liquid_flags: u32,
    /// Per-branch index counts of the `GRP ` block (read and ignored by the
    /// assembler; kept for writing).
    pub branches: Vec<u32>,
    pub triangles: Vec<MeshTriangle>,
    pub vertices: Vec<Vector3>,
    pub liquid: Option<WmoLiquid>,
}

impl GroupModelRaw {
    /// `GroupModel_Raw::Read`.
    pub fn read_from(r: &mut Reader<'_>) -> Result<Self> {
        let what = "GroupModel_Raw";
        let mogp_flags = r.u32().or_format(what)?;
        let group_wmo_id = r.u32().or_format(what)?;
        let vec1 = r.vector3().or_format(what)?;
        let vec2 = r.vector3().or_format(what)?;
        let mut bounds = AABox::EMPTY;
        bounds.set(vec1, vec2);
        let liquid_flags = r.u32().or_format(what)?;

        r.chunk(b"GRP ").or_format("GroupModel_Raw GRP")?;
        let _blocksize = r.i32().or_format(what)?;
        let n_branches = r.u32().or_format(what)?;
        let mut branches = Vec::new();
        for _ in 0..n_branches {
            branches.push(r.u32().or_format(what)?);
        }

        // ---- indexes
        r.chunk(b"INDX").or_format("GroupModel_Raw INDX")?;
        let _blocksize = r.i32().or_format(what)?;
        let n_indexes = r.u32().or_format(what)?;
        let mut triangles = Vec::new();
        if n_indexes > 0 {
            let idx = r.u32_vec(n_indexes as usize).or_format(what)?;
            triangles.reserve(idx.len() / 3);
            // C++ reads past the array for a trailing partial triangle; the
            // port uses 0 for those (malformed input only).
            let at = |i: usize| idx.get(i).copied().unwrap_or(0);
            for i in (0..idx.len()).step_by(3) {
                triangles.push(MeshTriangle::new(at(i), at(i + 1), at(i + 2)));
            }
        }

        // ---- vectors
        r.chunk(b"VERT").or_format("GroupModel_Raw VERT")?;
        let _blocksize = r.i32().or_format(what)?;
        let n_vectors = r.u32().or_format(what)?;
        let mut vertices = Vec::new();
        if n_vectors > 0 {
            vertices = r.vector3_vec(n_vectors as usize).or_format(what)?;
        }

        // ----- liquid
        let mut liquid = None;
        if liquid_flags & 3 != 0 {
            r.chunk(b"LIQU").or_format("GroupModel_Raw LIQU")?;
            let _blocksize = r.i32().or_format(what)?;
            let liquid_type = r.u32().or_format(what)?;
            if liquid_flags & 1 != 0 {
                let hlq = WmoLiquidHeader::read_from(r).or_format(what)?;
                // A zero-sized fread block fails in C++ (READ_OR_RETURN), so
                // xtiles*ytiles == 0 is an error there as well; negative tile
                // counts crash the C++ allocation and are rejected here.
                if hlq.xtiles <= 0 || hlq.ytiles <= 0 {
                    return Err(VmapError::format("GroupModel_Raw liquid tiles"));
                }
                let mut lq = WmoLiquid::new(
                    hlq.xtiles as u32,
                    hlq.ytiles as u32,
                    Vector3::new(hlq.pos_x, hlq.pos_y, hlq.pos_z),
                    liquid_type,
                );
                let size = hlq.xverts.wrapping_mul(hlq.yverts) as u32;
                if size == 0 {
                    return Err(VmapError::format("GroupModel_Raw liquid heights"));
                }
                let heights = r.f32_vec(size as usize).or_format(what)?;
                // C++ reads `xverts*yverts` floats into a buffer of
                // `(xtiles+1)*(ytiles+1)`; they match for valid data. On a
                // mismatch the port truncates / zero-fills instead of UB.
                let storage = lq.height_storage_mut();
                let n = storage.len().min(heights.len());
                storage[..n].copy_from_slice(&heights[..n]);
                let size = (hlq.xtiles as u32).wrapping_mul(hlq.ytiles as u32);
                let flags = r.bytes(size as usize).or_format(what)?;
                lq.flags_storage_mut()
                    .expect("tiles > 0")
                    .copy_from_slice(flags);
                liquid = Some(lq);
            } else {
                let mut lq = WmoLiquid::new(0, 0, Vector3::ZERO, liquid_type);
                lq.height_storage_mut()[0] = bounds.high().z;
                liquid = Some(lq);
            }
        }

        Ok(Self {
            mogp_flags,
            group_wmo_id,
            bounds,
            liquid_flags,
            branches,
            triangles,
            vertices,
            liquid,
        })
    }

    /// Writes one group like `WMOGroup::ConvertToVMAPGroupWmo` (the
    /// collision-filtered branch). With `liquid_flags & 1` the liquid header
    /// is derived from the stored liquid (`xverts = xtiles + 1`, material 0).
    pub fn write_to(&self, w: &mut impl Writer) {
        w.put_u32(self.mogp_flags);
        w.put_u32(self.group_wmo_id);
        w.put_vector3(self.bounds.low());
        w.put_vector3(self.bounds.high());
        w.put_u32(self.liquid_flags);

        w.put_bytes(b"GRP ");
        let n_branches = self.branches.len() as i32;
        w.put_i32(n_branches * 4 + 4);
        w.put_i32(n_branches);
        for &b in &self.branches {
            w.put_u32(b);
        }

        let n_col_triangles = self.triangles.len() as i32;
        w.put_u32(0x5844_4E49); // "INDX"
        w.put_i32(n_col_triangles * 6 + 4);
        w.put_i32(n_col_triangles * 3);
        for t in &self.triangles {
            w.put_u32(t.idx0);
            w.put_u32(t.idx1);
            w.put_u32(t.idx2);
        }

        let n_vertices = self.vertices.len() as u32;
        w.put_u32(0x5452_4556); // "VERT"
        w.put_u32(n_vertices * 3 * 4 + 4);
        w.put_u32(n_vertices);
        for v in &self.vertices {
            w.put_vector3(*v);
        }

        if self.liquid_flags & 3 != 0 {
            let liquid_type = self.liquid.as_ref().map_or(0, WmoLiquid::liquid_type);
            let mut total = 4i32;
            let header = self
                .liquid
                .as_ref()
                .filter(|_| self.liquid_flags & 1 != 0)
                .map(|lq| {
                    let (tx, ty, corner) = lq.pos_info();
                    WmoLiquidHeader {
                        xverts: tx as i32 + 1,
                        yverts: ty as i32 + 1,
                        xtiles: tx as i32,
                        ytiles: ty as i32,
                        pos_x: corner.x,
                        pos_y: corner.y,
                        pos_z: corner.z,
                        material: 0,
                    }
                });
            if let Some(h) = &header {
                total +=
                    WMO_LIQUID_HEADER_SIZE as i32 + h.xverts * h.yverts * 4 + h.xtiles * h.ytiles;
            }
            w.put_u32(0x5551_494C); // "LIQU"
            w.put_i32(total);
            w.put_u32(liquid_type);
            if let (Some(h), Some(lq)) = (header, &self.liquid) {
                h.write_to(w);
                for &height in lq.height_storage() {
                    w.put_f32(height);
                }
                w.put_bytes(lq.flags_storage().unwrap_or(&[]));
            }
        }
    }
}

/// `WorldModel_Raw`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorldModelRaw {
    /// Vertex count stored after the magic (`tempNVectors`, skipped by the
    /// assembler; the extractor reads it to skip empty models).
    pub n_vectors: u32,
    pub root_wmo_id: u32,
    pub groups: Vec<GroupModelRaw>,
}

impl WorldModelRaw {
    /// `WorldModel_Raw::Read` on an in-memory file.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        // `strcmp(ident, RAW_VMAP_MAGIC)` on the 8 bytes + terminator
        r.chunk(RAW_VMAP_MAGIC).or_format("WorldModel_Raw magic")?;
        let n_vectors = r.u32().or_format("WorldModel_Raw")?;
        let n_groups = r.u32().or_format("WorldModel_Raw")?;
        let root_wmo_id = r.u32().or_format("WorldModel_Raw")?;
        let mut groups = Vec::new();
        for _ in 0..n_groups {
            groups.push(GroupModelRaw::read_from(&mut r)?);
        }
        Ok(Self {
            n_vectors,
            root_wmo_id,
            groups,
        })
    }

    /// `WorldModel_Raw::Read(path)`.
    pub fn read_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let data = std::fs::read(path).map_err(|e| VmapError::io(path, e))?;
        Self::from_bytes(&data)
    }

    /// Writes the file like `WMORoot::ConvertToVMAPRootWmo` followed by
    /// `ConvertToVMAPGroupWmo` for every group.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Vec::new();
        w.put_bytes(RAW_VMAP_MAGIC);
        w.put_u32(self.n_vectors);
        w.put_u32(self.groups.len() as u32);
        w.put_u32(self.root_wmo_id);
        for g in &self.groups {
            g.write_to(&mut w);
        }
        w
    }
}
