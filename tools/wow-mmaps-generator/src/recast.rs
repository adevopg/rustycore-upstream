//! FFI to TrinityCore's Recast/Detour (vendored, see `build.rs`) through
//! `vendor/shim/mmgen_recast_shim.cpp`.
//!
//! The `#[repr(C)]` structs mirror `Recast.h` (`rcConfig` with TrinityCore's
//! `walkableSlopeAngleNotSteep`, `rcPolyMesh`, `rcPolyMeshDetail`) and
//! `DetourNavMesh.h` / `DetourNavMeshBuilder.h` (`dtNavMeshParams`,
//! `dtNavMeshCreateParams`); their layout is checked against the C++
//! compiler in the tests (`mmgen_layout`).
#![allow(unsafe_code)]

use std::ffi::{c_int, c_uint, c_void};

/// `DT_NAVMESH_VERSION` (DetourNavMesh.h).
pub const DT_NAVMESH_VERSION: u32 = 7;
/// `DT_VERTS_PER_POLYGON`.
pub const DT_VERTS_PER_POLYGON: i32 = 6;
/// `DT_POLY_BITS` with TrinityCore's `DT_POLYREF64` patch.
pub const DT_POLY_BITS: u32 = 31;
/// `dtTileFlags::DT_TILE_FREE_DATA`.
pub const DT_TILE_FREE_DATA: i32 = 0x01;
/// `DT_SUCCESS` (DetourStatus.h).
pub const DT_SUCCESS: u32 = 1 << 30;

/// `dtTileRef` (64 bit with `DT_POLYREF64`).
pub type DtTileRef = u64;

/// `rcConfig` (Recast.h, TrinityCore patched).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RcConfig {
    pub width: c_int,
    pub height: c_int,
    pub tile_size: c_int,
    pub border_size: c_int,
    pub cs: f32,
    pub ch: f32,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub walkable_slope_angle: f32,
    pub walkable_slope_angle_not_steep: f32,
    pub walkable_height: c_int,
    pub walkable_climb: c_int,
    pub walkable_radius: c_int,
    pub max_edge_len: c_int,
    pub max_simplification_error: f32,
    pub min_region_area: c_int,
    pub merge_region_area: c_int,
    pub max_verts_per_poly: c_int,
    pub detail_sample_dist: f32,
    pub detail_sample_max_error: f32,
}

/// `rcPolyMesh` (allocated/freed by Recast only).
#[repr(C)]
#[derive(Debug)]
pub struct RcPolyMesh {
    pub verts: *mut u16,
    pub polys: *mut u16,
    pub regs: *mut u16,
    pub flags: *mut u16,
    pub areas: *mut u8,
    pub nverts: c_int,
    pub npolys: c_int,
    pub maxpolys: c_int,
    pub nvp: c_int,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub cs: f32,
    pub ch: f32,
    pub border_size: c_int,
    pub max_edge_error: f32,
}

/// `rcPolyMeshDetail`.
#[repr(C)]
#[derive(Debug)]
pub struct RcPolyMeshDetail {
    pub meshes: *mut c_uint,
    pub verts: *mut f32,
    pub tris: *mut u8,
    pub nmeshes: c_int,
    pub nverts: c_int,
    pub ntris: c_int,
}

/// `dtNavMeshParams` — also the on-disk `.mmap` content (28 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DtNavMeshParams {
    pub orig: [f32; 3],
    pub tile_width: f32,
    pub tile_height: f32,
    pub max_tiles: c_int,
    pub max_polys: c_int,
}

impl DtNavMeshParams {
    /// `fwrite(&navMeshParams, sizeof(dtNavMeshParams), 1, file)`.
    pub fn to_bytes(self) -> [u8; 28] {
        let mut out = [0u8; 28];
        let words = [
            self.orig[0].to_bits(),
            self.orig[1].to_bits(),
            self.orig[2].to_bits(),
            self.tile_width.to_bits(),
            self.tile_height.to_bits(),
            self.max_tiles as u32,
            self.max_polys as u32,
        ];
        for (chunk, w) in out.chunks_exact_mut(4).zip(words) {
            chunk.copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

/// `dtNavMeshCreateParams` (`memset` to zero, then filled).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DtNavMeshCreateParams {
    pub verts: *const u16,
    pub vert_count: c_int,
    pub polys: *const u16,
    pub poly_flags: *const u16,
    pub poly_areas: *const u8,
    pub poly_count: c_int,
    pub nvp: c_int,
    pub detail_meshes: *const c_uint,
    pub detail_verts: *const f32,
    pub detail_verts_count: c_int,
    pub detail_tris: *const u8,
    pub detail_tri_count: c_int,
    pub off_mesh_con_verts: *const f32,
    pub off_mesh_con_rad: *const f32,
    pub off_mesh_con_flags: *const u16,
    pub off_mesh_con_areas: *const u8,
    pub off_mesh_con_dir: *const u8,
    pub off_mesh_con_user_id: *const c_uint,
    pub off_mesh_con_count: c_int,
    pub user_id: c_uint,
    pub tile_x: c_int,
    pub tile_y: c_int,
    pub tile_layer: c_int,
    pub bmin: [f32; 3],
    pub bmax: [f32; 3],
    pub walkable_height: f32,
    pub walkable_radius: f32,
    pub walkable_climb: f32,
    pub cs: f32,
    pub ch: f32,
    pub build_bv_tree: bool,
}

impl Default for DtNavMeshCreateParams {
    fn default() -> Self {
        Self {
            verts: std::ptr::null(),
            vert_count: 0,
            polys: std::ptr::null(),
            poly_flags: std::ptr::null(),
            poly_areas: std::ptr::null(),
            poly_count: 0,
            nvp: 0,
            detail_meshes: std::ptr::null(),
            detail_verts: std::ptr::null(),
            detail_verts_count: 0,
            detail_tris: std::ptr::null(),
            detail_tri_count: 0,
            off_mesh_con_verts: std::ptr::null(),
            off_mesh_con_rad: std::ptr::null(),
            off_mesh_con_flags: std::ptr::null(),
            off_mesh_con_areas: std::ptr::null(),
            off_mesh_con_dir: std::ptr::null(),
            off_mesh_con_user_id: std::ptr::null(),
            off_mesh_con_count: 0,
            user_id: 0,
            tile_x: 0,
            tile_y: 0,
            tile_layer: 0,
            bmin: [0.0; 3],
            bmax: [0.0; 3],
            walkable_height: 0.0,
            walkable_radius: 0.0,
            walkable_climb: 0.0,
            cs: 0.0,
            ch: 0.0,
            build_bv_tree: false,
        }
    }
}

unsafe extern "C" {
    #[cfg(test)]
    fn mmgen_layout(out: *mut c_int, cap: c_int) -> c_int;

    fn mmgen_rc_context_new() -> *mut c_void;
    fn mmgen_rc_context_free(ctx: *mut c_void);

    fn mmgen_rc_alloc_heightfield() -> *mut c_void;
    fn mmgen_rc_free_heightfield(p: *mut c_void);
    fn mmgen_rc_alloc_compact_heightfield() -> *mut c_void;
    fn mmgen_rc_free_compact_heightfield(p: *mut c_void);
    fn mmgen_rc_alloc_contour_set() -> *mut c_void;
    fn mmgen_rc_free_contour_set(p: *mut c_void);
    fn mmgen_rc_alloc_poly_mesh() -> *mut RcPolyMesh;
    fn mmgen_rc_free_poly_mesh(p: *mut RcPolyMesh);
    fn mmgen_rc_alloc_poly_mesh_detail() -> *mut RcPolyMeshDetail;
    fn mmgen_rc_free_poly_mesh_detail(p: *mut RcPolyMeshDetail);

    fn mmgen_rc_calc_bounds(verts: *const f32, nv: c_int, bmin: *mut f32, bmax: *mut f32);
    fn mmgen_rc_calc_grid_size(
        bmin: *const f32,
        bmax: *const f32,
        cs: f32,
        w: *mut c_int,
        h: *mut c_int,
    );
    fn mmgen_rc_create_heightfield(
        ctx: *mut c_void,
        hf: *mut c_void,
        width: c_int,
        height: c_int,
        bmin: *const f32,
        bmax: *const f32,
        cs: f32,
        ch: f32,
    ) -> bool;
    fn mmgen_rc_clear_unwalkable_triangles(
        ctx: *mut c_void,
        angle: f32,
        verts: *const f32,
        nv: c_int,
        tris: *const c_int,
        nt: c_int,
        areas: *mut u8,
    );
    fn mmgen_rc_mark_walkable_triangles(
        ctx: *mut c_void,
        angle: f32,
        verts: *const f32,
        nv: c_int,
        tris: *const c_int,
        nt: c_int,
        areas: *mut u8,
        area_type: u8,
    );
    fn mmgen_rc_rasterize_triangles(
        ctx: *mut c_void,
        verts: *const f32,
        nv: c_int,
        tris: *const c_int,
        areas: *const u8,
        nt: c_int,
        hf: *mut c_void,
        flag_merge_thr: c_int,
    ) -> bool;
    fn mmgen_rc_filter_low_hanging_walkable_obstacles(
        ctx: *mut c_void,
        climb: c_int,
        hf: *mut c_void,
    );
    fn mmgen_rc_filter_ledge_spans(ctx: *mut c_void, height: c_int, climb: c_int, hf: *mut c_void);
    fn mmgen_rc_filter_walkable_low_height_spans(ctx: *mut c_void, height: c_int, hf: *mut c_void);
    fn mmgen_rc_build_compact_heightfield(
        ctx: *mut c_void,
        height: c_int,
        climb: c_int,
        hf: *mut c_void,
        chf: *mut c_void,
    ) -> bool;
    fn mmgen_rc_erode_walkable_area(ctx: *mut c_void, radius: c_int, chf: *mut c_void) -> bool;
    fn mmgen_rc_median_filter_walkable_area(ctx: *mut c_void, chf: *mut c_void) -> bool;
    fn mmgen_rc_build_distance_field(ctx: *mut c_void, chf: *mut c_void) -> bool;
    fn mmgen_rc_build_regions(
        ctx: *mut c_void,
        chf: *mut c_void,
        border: c_int,
        min_area: c_int,
        merge_area: c_int,
    ) -> bool;
    fn mmgen_rc_build_contours(
        ctx: *mut c_void,
        chf: *mut c_void,
        max_error: f32,
        max_edge_len: c_int,
        cset: *mut c_void,
    ) -> bool;
    fn mmgen_rc_build_poly_mesh(
        ctx: *mut c_void,
        cset: *mut c_void,
        nvp: c_int,
        mesh: *mut RcPolyMesh,
    ) -> bool;
    fn mmgen_rc_build_poly_mesh_detail(
        ctx: *mut c_void,
        mesh: *const RcPolyMesh,
        chf: *mut c_void,
        sample_dist: f32,
        sample_max_error: f32,
        dmesh: *mut RcPolyMeshDetail,
    ) -> bool;
    fn mmgen_rc_merge_poly_meshes(
        ctx: *mut c_void,
        meshes: *mut *mut RcPolyMesh,
        n: c_int,
        mesh: *mut RcPolyMesh,
    ) -> bool;
    fn mmgen_rc_merge_poly_mesh_details(
        ctx: *mut c_void,
        meshes: *mut *mut RcPolyMeshDetail,
        n: c_int,
        mesh: *mut RcPolyMeshDetail,
    ) -> bool;

    fn mmgen_dt_create_nav_mesh_data(
        params: *mut DtNavMeshCreateParams,
        out_data: *mut *mut u8,
        out_size: *mut c_int,
    ) -> bool;
    fn mmgen_dt_free(ptr: *mut c_void);
    fn mmgen_dt_alloc_nav_mesh() -> *mut c_void;
    fn mmgen_dt_free_nav_mesh(nav: *mut c_void);
    fn mmgen_dt_nav_mesh_init(nav: *mut c_void, params: *const DtNavMeshParams) -> c_uint;
    fn mmgen_dt_nav_mesh_get_params(nav: *const c_void) -> *const DtNavMeshParams;
    fn mmgen_dt_nav_mesh_add_tile(
        nav: *mut c_void,
        data: *mut u8,
        size: c_int,
        flags: c_int,
        last_ref: DtTileRef,
        result: *mut DtTileRef,
    ) -> c_uint;
    fn mmgen_dt_nav_mesh_remove_tile(nav: *mut c_void, r: DtTileRef) -> c_uint;
}

/// Layout values reported by the C++ compiler (see `mmgen_layout`).
#[cfg(test)]
pub fn cpp_layout() -> Vec<i32> {
    let mut v = vec![0; 64];
    // SAFETY: the shim writes at most `cap` ints into the buffer.
    let n = unsafe { mmgen_layout(v.as_mut_ptr(), 64) };
    v.truncate(n as usize);
    v
}

/// `rcCalcBounds` (Recast.cpp).
pub fn rc_calc_bounds(verts: &[f32], nv: i32, bmin: &mut [f32; 3], bmax: &mut [f32; 3]) {
    assert!(verts.len() >= nv as usize * 3 && nv > 0);
    // SAFETY: `verts` holds `nv` vertices; outputs are 3 floats each.
    unsafe { mmgen_rc_calc_bounds(verts.as_ptr(), nv, bmin.as_mut_ptr(), bmax.as_mut_ptr()) }
}

/// `rcCalcGridSize` (Recast.cpp).
pub fn rc_calc_grid_size(bmin: &[f32; 3], bmax: &[f32; 3], cs: f32) -> (i32, i32) {
    let (mut w, mut h) = (0, 0);
    // SAFETY: plain value in/out parameters.
    unsafe { mmgen_rc_calc_grid_size(bmin.as_ptr(), bmax.as_ptr(), cs, &raw mut w, &raw mut h) };
    (w, h)
}

/// `rcContext(false)` owned by a tile builder.
pub struct RcContext(*mut c_void);

// SAFETY: an rcContext is only used by the thread owning it.
unsafe impl Send for RcContext {}

impl RcContext {
    pub fn new() -> Self {
        // SAFETY: returns a heap allocated rcContext.
        Self(unsafe { mmgen_rc_context_new() })
    }
    fn ptr(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for RcContext {
    fn drop(&mut self) {
        // SAFETY: allocated by `mmgen_rc_context_new`.
        unsafe { mmgen_rc_context_free(self.0) }
    }
}

macro_rules! owned_handle {
    ($name:ident, $ty:ty, $alloc:ident, $free:ident) => {
        /// Owning Recast allocation (freed with the matching `rcFree*`).
        pub struct $name(*mut $ty);
        impl $name {
            /// `rcAlloc*`; `None` when the allocation failed.
            pub fn alloc() -> Option<Self> {
                // SAFETY: Recast allocator.
                let p = unsafe { $alloc() };
                if p.is_null() { None } else { Some(Self(p)) }
            }
            pub fn as_ptr(&self) -> *mut $ty {
                self.0
            }
        }
        impl Drop for $name {
            fn drop(&mut self) {
                // SAFETY: allocated by the matching `rcAlloc*`.
                unsafe { $free(self.0) }
            }
        }
    };
}

owned_handle!(
    Heightfield,
    c_void,
    mmgen_rc_alloc_heightfield,
    mmgen_rc_free_heightfield
);
owned_handle!(
    CompactHeightfield,
    c_void,
    mmgen_rc_alloc_compact_heightfield,
    mmgen_rc_free_compact_heightfield
);
owned_handle!(
    ContourSet,
    c_void,
    mmgen_rc_alloc_contour_set,
    mmgen_rc_free_contour_set
);
owned_handle!(
    PolyMesh,
    RcPolyMesh,
    mmgen_rc_alloc_poly_mesh,
    mmgen_rc_free_poly_mesh
);
owned_handle!(
    PolyMeshDetail,
    RcPolyMeshDetail,
    mmgen_rc_alloc_poly_mesh_detail,
    mmgen_rc_free_poly_mesh_detail
);

impl PolyMesh {
    pub fn get(&self) -> &RcPolyMesh {
        // SAFETY: valid for the lifetime of the handle.
        unsafe { &*self.0 }
    }
    /// `verts` (`nverts * 3`).
    pub fn verts_mut(&mut self) -> &mut [u16] {
        let m = self.get();
        if m.verts.is_null() {
            return &mut [];
        }
        let n = m.nverts as usize * 3;
        // SAFETY: Recast allocates `nverts * 3` shorts.
        unsafe { std::slice::from_raw_parts_mut(m.verts, n) }
    }
    /// `(areas, flags)` of the `npolys` polygons.
    pub fn areas_flags_mut(&mut self) -> (&[u8], &mut [u16]) {
        let m = self.get();
        let n = m.npolys.max(0) as usize;
        if n == 0 {
            return (&[], &mut []);
        }
        // SAFETY: `areas`/`flags` hold at least `npolys` entries.
        unsafe {
            (
                std::slice::from_raw_parts(m.areas, n),
                std::slice::from_raw_parts_mut(m.flags, n),
            )
        }
    }
}

impl PolyMeshDetail {
    pub fn get(&self) -> &RcPolyMeshDetail {
        // SAFETY: valid for the lifetime of the handle.
        unsafe { &*self.0 }
    }
}

/// `(ptr, n)` as a slice (empty for null / zero).
///
/// # Safety
/// `p` must point to `n` valid elements (or be null / `n == 0`).
unsafe fn raw_slice<'a, T>(p: *const T, n: i32) -> &'a [T] {
    if p.is_null() || n <= 0 {
        &[]
    } else {
        // SAFETY: guaranteed by the caller.
        unsafe { std::slice::from_raw_parts(p, n as usize) }
    }
}

/// `rcPolyMesh` arrays (sized from `nverts`, `npolys`, `nvp`).
pub struct PolyMeshArrays<'a> {
    pub verts: &'a [u16],
    pub polys: &'a [u16],
    pub flags: &'a [u16],
    pub areas: &'a [u8],
    pub regs: &'a [u16],
}

impl PolyMesh {
    pub fn arrays(&self) -> PolyMeshArrays<'_> {
        let m = self.get();
        let np = m.npolys.max(0);
        // SAFETY: Recast allocates these with at least the used sizes.
        unsafe {
            PolyMeshArrays {
                verts: raw_slice(m.verts, m.nverts.max(0) * 3),
                polys: raw_slice(m.polys, np * m.nvp.max(0) * 2),
                flags: raw_slice(m.flags, np),
                areas: raw_slice(m.areas, np),
                regs: raw_slice(m.regs, np),
            }
        }
    }
}

/// `rcPolyMeshDetail` arrays (`meshes`, `verts`, `tris`).
pub struct PolyMeshDetailArrays<'a> {
    pub meshes: &'a [c_uint],
    pub verts: &'a [f32],
    pub tris: &'a [u8],
}

impl PolyMeshDetail {
    pub fn arrays(&self) -> PolyMeshDetailArrays<'_> {
        let m = self.get();
        // SAFETY: Recast allocates these with at least the used sizes.
        unsafe {
            PolyMeshDetailArrays {
                meshes: raw_slice(m.meshes, m.nmeshes.max(0) * 4),
                verts: raw_slice(m.verts, m.nverts.max(0) * 3),
                tris: raw_slice(m.tris, m.ntris.max(0) * 4),
            }
        }
    }
}

// ------------------------------------------------------------------ Recast

pub fn create_heightfield(
    ctx: &RcContext,
    hf: &Heightfield,
    width: i32,
    height: i32,
    bmin: &[f32; 3],
    bmax: &[f32; 3],
    cs: f32,
    ch: f32,
) -> bool {
    // SAFETY: valid handles and 3-float bounds.
    unsafe {
        mmgen_rc_create_heightfield(
            ctx.ptr(),
            hf.as_ptr(),
            width,
            height,
            bmin.as_ptr(),
            bmax.as_ptr(),
            cs,
            ch,
        )
    }
}

fn tri_args(verts: &[f32], tris: &[i32]) -> (i32, i32) {
    ((verts.len() / 3) as i32, (tris.len() / 3) as i32)
}

pub fn clear_unwalkable_triangles(
    ctx: &RcContext,
    angle: f32,
    verts: &[f32],
    tris: &[i32],
    areas: &mut [u8],
) {
    let (nv, nt) = tri_args(verts, tris);
    assert!(areas.len() >= nt as usize);
    // SAFETY: slices sized as Recast expects.
    unsafe {
        mmgen_rc_clear_unwalkable_triangles(
            ctx.ptr(),
            angle,
            verts.as_ptr(),
            nv,
            tris.as_ptr(),
            nt,
            areas.as_mut_ptr(),
        );
    }
}

pub fn mark_walkable_triangles(
    ctx: &RcContext,
    angle: f32,
    verts: &[f32],
    tris: &[i32],
    areas: &mut [u8],
    area_type: u8,
) {
    let (nv, nt) = tri_args(verts, tris);
    assert!(areas.len() >= nt as usize);
    // SAFETY: slices sized as Recast expects.
    unsafe {
        mmgen_rc_mark_walkable_triangles(
            ctx.ptr(),
            angle,
            verts.as_ptr(),
            nv,
            tris.as_ptr(),
            nt,
            areas.as_mut_ptr(),
            area_type,
        );
    }
}

pub fn rasterize_triangles(
    ctx: &RcContext,
    verts: &[f32],
    tris: &[i32],
    areas: &[u8],
    hf: &Heightfield,
    flag_merge_thr: i32,
) -> bool {
    let (nv, nt) = tri_args(verts, tris);
    assert!(areas.len() >= nt as usize);
    // Triangle indices are dereferenced unchecked by Recast (as in C++).
    // SAFETY: slices sized as Recast expects.
    unsafe {
        mmgen_rc_rasterize_triangles(
            ctx.ptr(),
            verts.as_ptr(),
            nv,
            tris.as_ptr(),
            areas.as_ptr(),
            nt,
            hf.as_ptr(),
            flag_merge_thr,
        )
    }
}

pub fn filter_low_hanging_walkable_obstacles(ctx: &RcContext, climb: i32, hf: &Heightfield) {
    // SAFETY: valid handles.
    unsafe { mmgen_rc_filter_low_hanging_walkable_obstacles(ctx.ptr(), climb, hf.as_ptr()) }
}

pub fn filter_ledge_spans(ctx: &RcContext, height: i32, climb: i32, hf: &Heightfield) {
    // SAFETY: valid handles.
    unsafe { mmgen_rc_filter_ledge_spans(ctx.ptr(), height, climb, hf.as_ptr()) }
}

pub fn filter_walkable_low_height_spans(ctx: &RcContext, height: i32, hf: &Heightfield) {
    // SAFETY: valid handles.
    unsafe { mmgen_rc_filter_walkable_low_height_spans(ctx.ptr(), height, hf.as_ptr()) }
}

pub fn build_compact_heightfield(
    ctx: &RcContext,
    height: i32,
    climb: i32,
    hf: &Heightfield,
    chf: &CompactHeightfield,
) -> bool {
    // SAFETY: valid handles.
    unsafe {
        mmgen_rc_build_compact_heightfield(ctx.ptr(), height, climb, hf.as_ptr(), chf.as_ptr())
    }
}

pub fn erode_walkable_area(ctx: &RcContext, radius: i32, chf: &CompactHeightfield) -> bool {
    // SAFETY: valid handles.
    unsafe { mmgen_rc_erode_walkable_area(ctx.ptr(), radius, chf.as_ptr()) }
}

pub fn median_filter_walkable_area(ctx: &RcContext, chf: &CompactHeightfield) -> bool {
    // SAFETY: valid handles.
    unsafe { mmgen_rc_median_filter_walkable_area(ctx.ptr(), chf.as_ptr()) }
}

pub fn build_distance_field(ctx: &RcContext, chf: &CompactHeightfield) -> bool {
    // SAFETY: valid handles.
    unsafe { mmgen_rc_build_distance_field(ctx.ptr(), chf.as_ptr()) }
}

pub fn build_regions(
    ctx: &RcContext,
    chf: &CompactHeightfield,
    border: i32,
    min_area: i32,
    merge_area: i32,
) -> bool {
    // SAFETY: valid handles.
    unsafe { mmgen_rc_build_regions(ctx.ptr(), chf.as_ptr(), border, min_area, merge_area) }
}

pub fn build_contours(
    ctx: &RcContext,
    chf: &CompactHeightfield,
    max_error: f32,
    max_edge_len: i32,
    cset: &ContourSet,
) -> bool {
    // SAFETY: valid handles.
    unsafe {
        mmgen_rc_build_contours(
            ctx.ptr(),
            chf.as_ptr(),
            max_error,
            max_edge_len,
            cset.as_ptr(),
        )
    }
}

pub fn build_poly_mesh(ctx: &RcContext, cset: &ContourSet, nvp: i32, mesh: &PolyMesh) -> bool {
    // SAFETY: valid handles.
    unsafe { mmgen_rc_build_poly_mesh(ctx.ptr(), cset.as_ptr(), nvp, mesh.as_ptr()) }
}

pub fn build_poly_mesh_detail(
    ctx: &RcContext,
    mesh: &PolyMesh,
    chf: &CompactHeightfield,
    sample_dist: f32,
    sample_max_error: f32,
    dmesh: &PolyMeshDetail,
) -> bool {
    // SAFETY: valid handles.
    unsafe {
        mmgen_rc_build_poly_mesh_detail(
            ctx.ptr(),
            mesh.as_ptr(),
            chf.as_ptr(),
            sample_dist,
            sample_max_error,
            dmesh.as_ptr(),
        )
    }
}

pub fn merge_poly_meshes(ctx: &RcContext, meshes: &[&PolyMesh], out: &PolyMesh) -> bool {
    let mut ptrs: Vec<*mut RcPolyMesh> = meshes.iter().map(|m| m.as_ptr()).collect();
    // SAFETY: array of valid meshes.
    unsafe {
        mmgen_rc_merge_poly_meshes(
            ctx.ptr(),
            ptrs.as_mut_ptr(),
            ptrs.len() as i32,
            out.as_ptr(),
        )
    }
}

pub fn merge_poly_mesh_details(
    ctx: &RcContext,
    meshes: &[&PolyMeshDetail],
    out: &PolyMeshDetail,
) -> bool {
    let mut ptrs: Vec<*mut RcPolyMeshDetail> = meshes.iter().map(|m| m.as_ptr()).collect();
    // SAFETY: array of valid meshes.
    unsafe {
        mmgen_rc_merge_poly_mesh_details(
            ctx.ptr(),
            ptrs.as_mut_ptr(),
            ptrs.len() as i32,
            out.as_ptr(),
        )
    }
}

// ------------------------------------------------------------------ Detour

/// Nav mesh data returned by `dtCreateNavMeshData` (dtAlloc'ed).
pub struct NavData {
    ptr: *mut u8,
    size: i32,
}

impl NavData {
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `size` bytes allocated by Detour.
        unsafe { std::slice::from_raw_parts(self.ptr, self.size as usize) }
    }
    pub fn size(&self) -> i32 {
        self.size
    }
}

impl Drop for NavData {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: allocated with dtAlloc.
            unsafe { mmgen_dt_free(self.ptr.cast()) }
        }
    }
}

/// `dtCreateNavMeshData`.
pub fn create_nav_mesh_data(params: &mut DtNavMeshCreateParams) -> Option<NavData> {
    let mut data = std::ptr::null_mut();
    let mut size = 0;
    // SAFETY: params point to buffers kept alive by the caller.
    let ok = unsafe { mmgen_dt_create_nav_mesh_data(params, &raw mut data, &raw mut size) };
    ok.then_some(NavData { ptr: data, size })
}

/// `dtNavMesh` (dtAllocNavMesh / dtFreeNavMesh).
pub struct NavMesh(*mut c_void);

// SAFETY: a dtNavMesh is only used by one thread at a time.
unsafe impl Send for NavMesh {}

impl NavMesh {
    pub fn alloc() -> Self {
        // SAFETY: Detour allocator.
        Self(unsafe { mmgen_dt_alloc_nav_mesh() })
    }

    /// `dtNavMesh::init(const dtNavMeshParams*)` — returns the `dtStatus`.
    pub fn init(&mut self, params: &DtNavMeshParams) -> u32 {
        // SAFETY: valid mesh, params copied by Detour.
        unsafe { mmgen_dt_nav_mesh_init(self.0, params) }
    }

    pub fn params(&self) -> DtNavMeshParams {
        // SAFETY: returns a pointer to the mesh's own params.
        unsafe { *mmgen_dt_nav_mesh_get_params(self.0) }
    }

    /// `addTile(data, size, DT_TILE_FREE_DATA, 0, &tileRef)`.
    ///
    /// On success the mesh owns the buffer (freed by `remove_tile`) and the
    /// returned bytes are the buffer *after* Detour wired the tile links —
    /// exactly what MapBuilder writes to the `.mmtile`. On failure the buffer
    /// is returned in `Err` together with the status.
    pub fn add_tile(&mut self, data: NavData) -> Result<(DtTileRef, Vec<u8>), (u32, DtTileRef)> {
        let mut tile_ref: DtTileRef = 0;
        // SAFETY: data was produced by dtCreateNavMeshData.
        let status = unsafe {
            mmgen_dt_nav_mesh_add_tile(
                self.0,
                data.ptr,
                data.size,
                DT_TILE_FREE_DATA,
                0,
                &raw mut tile_ref,
            )
        };
        if tile_ref == 0 || status != DT_SUCCESS {
            // C++ leaks navData here; the mesh does not own it unless
            // the status is a success, so freeing it changes nothing observable.
            if status & DT_SUCCESS == 0 {
                drop(data);
            } else {
                std::mem::forget(data);
            }
            return Err((status, tile_ref));
        }
        let bytes = data.bytes().to_vec();
        std::mem::forget(data);
        Ok((tile_ref, bytes))
    }

    /// `removeTile(ref, nullptr, nullptr)` (frees the data, DT_TILE_FREE_DATA).
    pub fn remove_tile(&mut self, tile_ref: DtTileRef) -> u32 {
        // SAFETY: valid mesh.
        unsafe { mmgen_dt_nav_mesh_remove_tile(self.0, tile_ref) }
    }
}

impl Drop for NavMesh {
    fn drop(&mut self) {
        // SAFETY: allocated by dtAllocNavMesh.
        unsafe { mmgen_dt_free_nav_mesh(self.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn repr_c_layout_matches_cpp() {
        let expected = vec![
            size_of::<RcConfig>() as i32,
            offset_of!(RcConfig, walkable_slope_angle_not_steep) as i32,
            offset_of!(RcConfig, detail_sample_max_error) as i32,
            size_of::<RcPolyMesh>() as i32,
            offset_of!(RcPolyMesh, nverts) as i32,
            offset_of!(RcPolyMesh, max_edge_error) as i32,
            size_of::<RcPolyMeshDetail>() as i32,
            offset_of!(RcPolyMeshDetail, ntris) as i32,
            size_of::<DtNavMeshParams>() as i32,
            size_of::<DtNavMeshCreateParams>() as i32,
            offset_of!(DtNavMeshCreateParams, off_mesh_con_count) as i32,
            offset_of!(DtNavMeshCreateParams, user_id) as i32,
            offset_of!(DtNavMeshCreateParams, bmin) as i32,
            offset_of!(DtNavMeshCreateParams, build_bv_tree) as i32,
            size_of::<DtTileRef>() as i32,
            DT_NAVMESH_VERSION as i32,
            DT_VERTS_PER_POLYGON,
            DT_POLY_BITS as i32,
            DT_TILE_FREE_DATA,
        ];
        assert_eq!(cpp_layout(), expected);
    }

    #[test]
    fn calc_grid_size_matches_recast() {
        let (w, h) = rc_calc_grid_size(&[0.0, 0.0, 0.0], &[10.0, 5.0, 3.0], 0.5);
        assert_eq!((w, h), (20, 6));
    }
}
