//! `MeshTriangle`, `WmoLiquid`, `GroupModel` and `WorldModel` — port of
//! `src/common/Collision/Models/WorldModel.{h,cpp}` (construction, BIH
//! building, `.vmo` write/read, liquid height lookup). Ray/point collision
//! queries are not ported.

use std::path::Path;

use crate::bih::Bih;
use crate::definitions::{LIQUID_TILE_SIZE, VMAP_MAGIC};
use crate::error::{OrFormat, Result, VmapError};
use crate::io::{Reader, Writer};
use crate::math::{AABox, Vector3};

/// `MeshTriangle` — three `u32` vertex indices (12 bytes on disk).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MeshTriangle {
    pub idx0: u32,
    pub idx1: u32,
    pub idx2: u32,
}

impl MeshTriangle {
    pub const fn new(idx0: u32, idx1: u32, idx2: u32) -> Self {
        Self { idx0, idx1, idx2 }
    }
}

/// `WmoLiquid` — liquid height grid of a WMO group (or a single flat height
/// when there are no tiles, in which case `flags` is `None`).
#[derive(Debug, Clone, PartialEq)]
pub struct WmoLiquid {
    tiles_x: u32,
    tiles_y: u32,
    corner: Vector3,
    liquid_type: u32,
    /// `(tiles_x + 1) * (tiles_y + 1)` heights, or one height if no tiles.
    heights: Vec<f32>,
    /// `tiles_x * tiles_y` tile flags, `None` if no tiles (`iFlags == nullptr`).
    flags: Option<Vec<u8>>,
}

impl WmoLiquid {
    /// `WmoLiquid(width, height, corner, type)`; storage is zero-filled
    /// (C++ leaves it uninitialised until filled by the caller).
    pub fn new(width: u32, height: u32, corner: Vector3, liquid_type: u32) -> Self {
        let (heights, flags) = if width != 0 && height != 0 {
            (
                vec![0.0; (width.wrapping_add(1)).wrapping_mul(height.wrapping_add(1)) as usize],
                Some(vec![0u8; width.wrapping_mul(height) as usize]),
            )
        } else {
            (vec![0.0; 1], None)
        };
        Self {
            tiles_x: width,
            tiles_y: height,
            corner,
            liquid_type,
            heights,
            flags,
        }
    }

    /// `WmoLiquid::GetType`.
    pub fn liquid_type(&self) -> u32 {
        self.liquid_type
    }

    /// `WmoLiquid::GetHeightStorage`.
    pub fn height_storage(&self) -> &[f32] {
        &self.heights
    }

    pub fn height_storage_mut(&mut self) -> &mut [f32] {
        &mut self.heights
    }

    /// `WmoLiquid::GetFlagsStorage` (`None` == `nullptr`).
    pub fn flags_storage(&self) -> Option<&[u8]> {
        self.flags.as_deref()
    }

    pub fn flags_storage_mut(&mut self) -> Option<&mut [u8]> {
        self.flags.as_deref_mut()
    }

    /// `WmoLiquid::getPosInfo` — `(tilesX, tilesY, corner)`.
    pub fn pos_info(&self) -> (u32, u32, Vector3) {
        (self.tiles_x, self.tiles_y, self.corner)
    }

    /// `WmoLiquid::GetLiquidHeight`.
    pub fn liquid_height(&self, pos: Vector3) -> Option<f32> {
        let Some(flags) = &self.flags else {
            return Some(self.heights[0]);
        };
        let col_f = (pos.x - self.corner.x) / LIQUID_TILE_SIZE;
        let tx = col_f as u32;
        if col_f < 0.0 || tx >= self.tiles_x {
            return None;
        }
        let row_f = (pos.y - self.corner.y) / LIQUID_TILE_SIZE;
        let ty = row_f as u32;
        if row_f < 0.0 || ty >= self.tiles_y {
            return None;
        }
        if (flags[(tx + ty * self.tiles_x) as usize] & 0x0F) == 0x0F {
            return None;
        }
        let dx = col_f - tx as f32;
        let dy = row_f - ty as f32;
        let row = self.tiles_x + 1;
        let h = |x: u32, y: u32| self.heights[(x + y * row) as usize];
        if dx > dy {
            let sx = h(tx + 1, ty) - h(tx, ty);
            let sy = h(tx + 1, ty + 1) - h(tx + 1, ty);
            Some(h(tx, ty) + dx * sx + dy * sy)
        } else {
            let sx = h(tx + 1, ty + 1) - h(tx, ty + 1);
            let sy = h(tx, ty + 1) - h(tx, ty);
            Some(h(tx, ty) + dx * sx + dy * sy)
        }
    }

    /// `WmoLiquid::GetFileSize`.
    pub fn file_size(&self) -> u32 {
        let base = 2 * 4 + 12 + 4;
        base + match &self.flags {
            Some(_) => (self.tiles_x.wrapping_add(1))
                .wrapping_mul(self.tiles_y.wrapping_add(1))
                .wrapping_mul(4)
                .wrapping_add(self.tiles_x.wrapping_mul(self.tiles_y)),
            None => 4,
        }
    }

    /// `WmoLiquid::writeToFile`.
    pub fn write_to(&self, w: &mut impl Writer) {
        w.put_u32(self.tiles_x);
        w.put_u32(self.tiles_y);
        w.put_vector3(self.corner);
        w.put_u32(self.liquid_type);
        if self.tiles_x != 0 && self.tiles_y != 0 {
            for &h in &self.heights {
                w.put_f32(h);
            }
            if let Some(flags) = &self.flags {
                w.put_bytes(flags);
            }
        } else {
            w.put_f32(self.heights[0]);
        }
    }

    /// `WmoLiquid::readFromFile`.
    pub fn read_from(r: &mut Reader<'_>) -> Result<Self> {
        let what = "WmoLiquid";
        let tiles_x = r.u32().or_format(what)?;
        let tiles_y = r.u32().or_format(what)?;
        let corner = r.vector3().or_format(what)?;
        let liquid_type = r.u32().or_format(what)?;
        let (heights, flags) = if tiles_x != 0 && tiles_y != 0 {
            let size = tiles_x
                .wrapping_add(1)
                .wrapping_mul(tiles_y.wrapping_add(1));
            let heights = r.f32_vec(size as usize).or_format(what)?;
            let size = tiles_x.wrapping_mul(tiles_y);
            let flags = r.bytes(size as usize).or_format(what)?.to_vec();
            (heights, Some(flags))
        } else {
            (vec![r.f32().or_format(what)?], None)
        };
        Ok(Self {
            tiles_x,
            tiles_y,
            corner,
            liquid_type,
            heights,
            flags,
        })
    }
}

/// `GroupModel` — one WMO group (or the single group of an M2) with its
/// collision mesh, mesh BIH and optional liquid.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroupModel {
    bound: AABox,
    mogp_flags: u32,
    group_wmo_id: u32,
    vertices: Vec<Vector3>,
    triangles: Vec<MeshTriangle>,
    mesh_tree: Bih,
    liquid: Option<WmoLiquid>,
}

impl GroupModel {
    /// `GroupModel(mogpFlags, groupWMOID, bound)`.
    pub fn new(mogp_flags: u32, group_wmo_id: u32, bound: AABox) -> Self {
        Self {
            bound,
            mogp_flags,
            group_wmo_id,
            ..Self::default()
        }
    }

    /// `GroupModel::setMeshData` — stores the mesh and builds the mesh BIH
    /// (leaf size 3) from per-triangle bounds (`TriBoundFunc`).
    pub fn set_mesh_data(
        &mut self,
        vertices: Vec<Vector3>,
        triangles: Vec<MeshTriangle>,
    ) -> Result<()> {
        let count = vertices.len();
        let vert = |i: u32| {
            vertices
                .get(i as usize)
                .copied()
                .ok_or(VmapError::VertexIndexOutOfRange { index: i, count })
        };
        let mut bounds = Vec::with_capacity(triangles.len());
        for tri in &triangles {
            let (v0, v1, v2) = (vert(tri.idx0)?, vert(tri.idx1)?, vert(tri.idx2)?);
            let lo = v0.min(v1).min(v2);
            let hi = v0.max(v1).max(v2);
            bounds.push(AABox::new(lo, hi));
        }
        self.vertices = vertices;
        self.triangles = triangles;
        self.mesh_tree.build(&bounds, 3)?;
        Ok(())
    }

    /// `GroupModel::setLiquidData`.
    pub fn set_liquid_data(&mut self, liquid: Option<WmoLiquid>) {
        self.liquid = liquid;
    }

    /// `GroupModel::GetBound`.
    pub fn bound(&self) -> &AABox {
        &self.bound
    }

    /// `GroupModel::GetMogpFlags`.
    pub fn mogp_flags(&self) -> u32 {
        self.mogp_flags
    }

    /// `GroupModel::GetWmoID`.
    pub fn wmo_id(&self) -> u32 {
        self.group_wmo_id
    }

    pub fn vertices(&self) -> &[Vector3] {
        &self.vertices
    }

    pub fn triangles(&self) -> &[MeshTriangle] {
        &self.triangles
    }

    pub fn mesh_tree(&self) -> &Bih {
        &self.mesh_tree
    }

    pub fn liquid(&self) -> Option<&WmoLiquid> {
        self.liquid.as_ref()
    }

    /// `GroupModel::getMeshData` — vertices, triangles and liquid (borrowed).
    pub fn mesh_data(&self) -> (&[Vector3], &[MeshTriangle], Option<&WmoLiquid>) {
        (&self.vertices, &self.triangles, self.liquid.as_ref())
    }

    /// `GroupModel::GetLiquidLevel`.
    pub fn liquid_level(&self, pos: Vector3) -> Option<f32> {
        self.liquid.as_ref().and_then(|l| l.liquid_height(pos))
    }

    /// `GroupModel::GetLiquidType`.
    pub fn liquid_type(&self) -> u32 {
        self.liquid.as_ref().map_or(0, WmoLiquid::liquid_type)
    }

    /// `GroupModel::writeToFile`.
    pub fn write_to(&self, w: &mut impl Writer) {
        w.put_aabox(&self.bound);
        w.put_u32(self.mogp_flags);
        w.put_u32(self.group_wmo_id);

        // write vertices
        w.put_bytes(b"VERT");
        let count = self.vertices.len() as u32;
        w.put_u32(4 + 12 * count);
        w.put_u32(count);
        if count == 0 {
            // models without (collision) geometry end here
            return;
        }
        for v in &self.vertices {
            w.put_vector3(*v);
        }

        // write triangle mesh
        w.put_bytes(b"TRIM");
        let count = self.triangles.len() as u32;
        w.put_u32(4 + 12 * count);
        w.put_u32(count);
        for t in &self.triangles {
            w.put_u32(t.idx0);
            w.put_u32(t.idx1);
            w.put_u32(t.idx2);
        }

        // write mesh BIH
        w.put_bytes(b"MBIH");
        self.mesh_tree.write_to(w);

        // write liquid data
        w.put_bytes(b"LIQU");
        match &self.liquid {
            None => w.put_u32(0),
            Some(liquid) => {
                w.put_u32(liquid.file_size());
                liquid.write_to(w);
            }
        }
    }

    /// `GroupModel::readFromFile`.
    pub fn read_from(r: &mut Reader<'_>) -> Result<Self> {
        let what = "GroupModel";
        let mut g = Self {
            bound: r.aabox().or_format(what)?,
            mogp_flags: r.u32().or_format(what)?,
            group_wmo_id: r.u32().or_format(what)?,
            ..Self::default()
        };

        r.chunk(b"VERT").or_format("GroupModel VERT")?;
        let _chunk_size = r.u32().or_format(what)?;
        let count = r.u32().or_format(what)?;
        if count == 0 {
            return Ok(g);
        }
        g.vertices = r.vector3_vec(count as usize).or_format(what)?;

        r.chunk(b"TRIM").or_format("GroupModel TRIM")?;
        let _chunk_size = r.u32().or_format(what)?;
        let count = r.u32().or_format(what)?;
        let raw = r
            .u32_vec((count as usize).checked_mul(3).or_format(what)?)
            .or_format(what)?;
        g.triangles = raw
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| MeshTriangle::new(c[0], c[1], c[2]))
            .collect();

        r.chunk(b"MBIH").or_format("GroupModel MBIH")?;
        g.mesh_tree = Bih::read_from(r)?;

        r.chunk(b"LIQU").or_format("GroupModel LIQU")?;
        let chunk_size = r.u32().or_format(what)?;
        if chunk_size > 0 {
            g.liquid = Some(WmoLiquid::read_from(r)?);
        }
        Ok(g)
    }
}

/// `WorldModel` — a converted M2 or WMO in its own coordinate space
/// (`.vmo` file).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WorldModel {
    /// `WorldModel::Flags` — set from the spawn flags when a model is
    /// acquired by `VMapManager2::acquireModelInstance`.
    pub flags: u32,
    root_wmo_id: u32,
    group_models: Vec<GroupModel>,
    group_tree: Bih,
    name: String,
}

impl WorldModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// `WorldModel::setRootWmoID`.
    pub fn set_root_wmo_id(&mut self, id: u32) {
        self.root_wmo_id = id;
    }

    pub fn root_wmo_id(&self) -> u32 {
        self.root_wmo_id
    }

    /// `WorldModel::setGroupModels` — stores the groups and builds the group
    /// BIH (leaf size 1) from the group bounds.
    pub fn set_group_models(&mut self, models: Vec<GroupModel>) -> Result<()> {
        self.group_models = models;
        let bounds: Vec<AABox> = self.group_models.iter().map(|g| g.bound).collect();
        self.group_tree.build(&bounds, 1)?;
        Ok(())
    }

    /// `WorldModel::getGroupModels` (borrowed instead of copied).
    pub fn group_models(&self) -> &[GroupModel] {
        &self.group_models
    }

    pub fn group_tree(&self) -> &Bih {
        &self.group_tree
    }

    /// `WorldModel::GetName`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `WorldModel::SetName`.
    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    /// Serialises exactly what `WorldModel::writeFile` writes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Vec::new();
        w.put_bytes(VMAP_MAGIC);
        w.put_bytes(b"WMOD");
        w.put_u32(4 + 4);
        w.put_u32(self.root_wmo_id);
        if !self.group_models.is_empty() {
            w.put_bytes(b"GMOD");
            w.put_u32(self.group_models.len() as u32);
            for g in &self.group_models {
                g.write_to(&mut w);
            }
            w.put_bytes(b"GBIH");
            self.group_tree.write_to(&mut w);
        }
        w
    }

    /// `WorldModel::writeFile`.
    pub fn write_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        std::fs::write(path, self.to_bytes()).map_err(|e| VmapError::io(path, e))
    }

    /// Parses what `WorldModel::readFile` reads.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let mut m = Self::default();
        r.chunk(VMAP_MAGIC).or_format("WorldModel magic")?;
        r.chunk(b"WMOD").or_format("WorldModel WMOD")?;
        let _chunk_size = r.u32().or_format("WorldModel")?;
        m.root_wmo_id = r.u32().or_format("WorldModel")?;
        // a missing GMOD chunk is not an error (models without groups)
        if r.chunk(b"GMOD") {
            let count = r.u32().or_format("WorldModel GMOD")?;
            let mut groups = Vec::new();
            for _ in 0..count {
                groups.push(GroupModel::read_from(&mut r)?);
            }
            m.group_models = groups;
            r.chunk(b"GBIH").or_format("WorldModel GBIH")?;
            m.group_tree = Bih::read_from(&mut r)?;
        }
        Ok(m)
    }

    /// `WorldModel::readFile`.
    pub fn read_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let data = std::fs::read(path).map_err(|e| VmapError::io(path, e))?;
        Self::from_bytes(&data)
    }
}
