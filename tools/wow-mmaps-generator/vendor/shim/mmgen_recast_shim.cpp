// extern "C" shim over TrinityCore's vendored Recast + Detour (tag TDB343.24081,
// dep/recastnavigation) exposing exactly the calls made by
// src/tools/mmaps_generator/MapBuilder.cpp (TileBuilder::buildMoveMapTile,
// MapBuilder::buildNavMesh, TileBuilder::WorkerThread, MapBuilder::getTileBounds).
//
// Every wrapper forwards its arguments unchanged so the floating point behaviour
// is the one of the TrinityCore library code.

#include "Recast.h"
#include "DetourAlloc.h"
#include "DetourNavMesh.h"
#include "DetourNavMeshBuilder.h"

#include <cstddef>

extern "C" {

// ---------------------------------------------------------------- layout check
// Values checked by the Rust #[repr(C)] mirrors (recast.rs tests).
int mmgen_layout(int* out, int cap)
{
    int const values[] = {
        (int)sizeof(rcConfig),
        (int)offsetof(rcConfig, walkableSlopeAngleNotSteep),
        (int)offsetof(rcConfig, detailSampleMaxError),
        (int)sizeof(rcPolyMesh),
        (int)offsetof(rcPolyMesh, nverts),
        (int)offsetof(rcPolyMesh, maxEdgeError),
        (int)sizeof(rcPolyMeshDetail),
        (int)offsetof(rcPolyMeshDetail, ntris),
        (int)sizeof(dtNavMeshParams),
        (int)sizeof(dtNavMeshCreateParams),
        (int)offsetof(dtNavMeshCreateParams, offMeshConCount),
        (int)offsetof(dtNavMeshCreateParams, userId),
        (int)offsetof(dtNavMeshCreateParams, bmin),
        (int)offsetof(dtNavMeshCreateParams, buildBvTree),
        (int)sizeof(dtTileRef),
        DT_NAVMESH_VERSION,
        DT_VERTS_PER_POLYGON,
        (int)DT_POLY_BITS,
        DT_TILE_FREE_DATA,
    };
    int const n = (int)(sizeof(values) / sizeof(values[0]));
    for (int i = 0; i < n && i < cap; ++i)
        out[i] = values[i];
    return n;
}

// -------------------------------------------------------------------- context
void* mmgen_rc_context_new() { return new rcContext(false); }
void mmgen_rc_context_free(void* ctx) { delete static_cast<rcContext*>(ctx); }

// ------------------------------------------------------------ alloc / free
void* mmgen_rc_alloc_heightfield() { return rcAllocHeightfield(); }
void mmgen_rc_free_heightfield(void* p) { rcFreeHeightField(static_cast<rcHeightfield*>(p)); }
void* mmgen_rc_alloc_compact_heightfield() { return rcAllocCompactHeightfield(); }
void mmgen_rc_free_compact_heightfield(void* p) { rcFreeCompactHeightfield(static_cast<rcCompactHeightfield*>(p)); }
void* mmgen_rc_alloc_contour_set() { return rcAllocContourSet(); }
void mmgen_rc_free_contour_set(void* p) { rcFreeContourSet(static_cast<rcContourSet*>(p)); }
rcPolyMesh* mmgen_rc_alloc_poly_mesh() { return rcAllocPolyMesh(); }
void mmgen_rc_free_poly_mesh(rcPolyMesh* p) { rcFreePolyMesh(p); }
rcPolyMeshDetail* mmgen_rc_alloc_poly_mesh_detail() { return rcAllocPolyMeshDetail(); }
void mmgen_rc_free_poly_mesh_detail(rcPolyMeshDetail* p) { rcFreePolyMeshDetail(p); }

// -------------------------------------------------------------------- Recast
void mmgen_rc_calc_bounds(float const* verts, int nv, float* bmin, float* bmax)
{
    rcCalcBounds(verts, nv, bmin, bmax);
}

void mmgen_rc_calc_grid_size(float const* bmin, float const* bmax, float cs, int* w, int* h)
{
    rcCalcGridSize(bmin, bmax, cs, w, h);
}

bool mmgen_rc_create_heightfield(void* ctx, void* hf, int width, int height, float const* bmin, float const* bmax, float cs, float ch)
{
    return rcCreateHeightfield(static_cast<rcContext*>(ctx), *static_cast<rcHeightfield*>(hf), width, height, bmin, bmax, cs, ch);
}

void mmgen_rc_clear_unwalkable_triangles(void* ctx, float walkableSlopeAngle, float const* verts, int nv, int const* tris, int nt, unsigned char* areas)
{
    rcClearUnwalkableTriangles(static_cast<rcContext*>(ctx), walkableSlopeAngle, verts, nv, tris, nt, areas);
}

void mmgen_rc_mark_walkable_triangles(void* ctx, float walkableSlopeAngle, float const* verts, int nv, int const* tris, int nt, unsigned char* areas, unsigned char areaType)
{
    rcMarkWalkableTriangles(static_cast<rcContext*>(ctx), walkableSlopeAngle, verts, nv, tris, nt, areas, areaType);
}

bool mmgen_rc_rasterize_triangles(void* ctx, float const* verts, int nv, int const* tris, unsigned char const* areas, int nt, void* hf, int flagMergeThr)
{
    return rcRasterizeTriangles(static_cast<rcContext*>(ctx), verts, nv, tris, areas, nt, *static_cast<rcHeightfield*>(hf), flagMergeThr);
}

void mmgen_rc_filter_low_hanging_walkable_obstacles(void* ctx, int walkableClimb, void* hf)
{
    rcFilterLowHangingWalkableObstacles(static_cast<rcContext*>(ctx), walkableClimb, *static_cast<rcHeightfield*>(hf));
}

void mmgen_rc_filter_ledge_spans(void* ctx, int walkableHeight, int walkableClimb, void* hf)
{
    rcFilterLedgeSpans(static_cast<rcContext*>(ctx), walkableHeight, walkableClimb, *static_cast<rcHeightfield*>(hf));
}

void mmgen_rc_filter_walkable_low_height_spans(void* ctx, int walkableHeight, void* hf)
{
    rcFilterWalkableLowHeightSpans(static_cast<rcContext*>(ctx), walkableHeight, *static_cast<rcHeightfield*>(hf));
}

bool mmgen_rc_build_compact_heightfield(void* ctx, int walkableHeight, int walkableClimb, void* hf, void* chf)
{
    return rcBuildCompactHeightfield(static_cast<rcContext*>(ctx), walkableHeight, walkableClimb, *static_cast<rcHeightfield*>(hf), *static_cast<rcCompactHeightfield*>(chf));
}

bool mmgen_rc_erode_walkable_area(void* ctx, int radius, void* chf)
{
    return rcErodeWalkableArea(static_cast<rcContext*>(ctx), radius, *static_cast<rcCompactHeightfield*>(chf));
}

bool mmgen_rc_median_filter_walkable_area(void* ctx, void* chf)
{
    return rcMedianFilterWalkableArea(static_cast<rcContext*>(ctx), *static_cast<rcCompactHeightfield*>(chf));
}

bool mmgen_rc_build_distance_field(void* ctx, void* chf)
{
    return rcBuildDistanceField(static_cast<rcContext*>(ctx), *static_cast<rcCompactHeightfield*>(chf));
}

bool mmgen_rc_build_regions(void* ctx, void* chf, int borderSize, int minRegionArea, int mergeRegionArea)
{
    return rcBuildRegions(static_cast<rcContext*>(ctx), *static_cast<rcCompactHeightfield*>(chf), borderSize, minRegionArea, mergeRegionArea);
}

bool mmgen_rc_build_contours(void* ctx, void* chf, float maxError, int maxEdgeLen, void* cset)
{
    // default buildFlags (RC_CONTOUR_TESS_WALL_EDGES), as MapBuilder.cpp
    return rcBuildContours(static_cast<rcContext*>(ctx), *static_cast<rcCompactHeightfield*>(chf), maxError, maxEdgeLen, *static_cast<rcContourSet*>(cset));
}

bool mmgen_rc_build_poly_mesh(void* ctx, void* cset, int nvp, rcPolyMesh* mesh)
{
    return rcBuildPolyMesh(static_cast<rcContext*>(ctx), *static_cast<rcContourSet*>(cset), nvp, *mesh);
}

bool mmgen_rc_build_poly_mesh_detail(void* ctx, rcPolyMesh const* mesh, void* chf, float sampleDist, float sampleMaxError, rcPolyMeshDetail* dmesh)
{
    return rcBuildPolyMeshDetail(static_cast<rcContext*>(ctx), *mesh, *static_cast<rcCompactHeightfield*>(chf), sampleDist, sampleMaxError, *dmesh);
}

bool mmgen_rc_merge_poly_meshes(void* ctx, rcPolyMesh** meshes, int nmeshes, rcPolyMesh* mesh)
{
    return rcMergePolyMeshes(static_cast<rcContext*>(ctx), meshes, nmeshes, *mesh);
}

bool mmgen_rc_merge_poly_mesh_details(void* ctx, rcPolyMeshDetail** meshes, int nmeshes, rcPolyMeshDetail* mesh)
{
    return rcMergePolyMeshDetails(static_cast<rcContext*>(ctx), meshes, nmeshes, *mesh);
}

// -------------------------------------------------------------------- Detour
bool mmgen_dt_create_nav_mesh_data(dtNavMeshCreateParams* params, unsigned char** outData, int* outDataSize)
{
    return dtCreateNavMeshData(params, outData, outDataSize);
}

void mmgen_dt_free(void* ptr) { dtFree(ptr); }

void* mmgen_dt_alloc_nav_mesh() { return dtAllocNavMesh(); }
void mmgen_dt_free_nav_mesh(void* navMesh) { dtFreeNavMesh(static_cast<dtNavMesh*>(navMesh)); }

unsigned int mmgen_dt_nav_mesh_init(void* navMesh, dtNavMeshParams const* params)
{
    return static_cast<dtNavMesh*>(navMesh)->init(params);
}

dtNavMeshParams const* mmgen_dt_nav_mesh_get_params(void const* navMesh)
{
    return static_cast<dtNavMesh const*>(navMesh)->getParams();
}

unsigned int mmgen_dt_nav_mesh_add_tile(void* navMesh, unsigned char* data, int dataSize, int flags, dtTileRef lastRef, dtTileRef* result)
{
    return static_cast<dtNavMesh*>(navMesh)->addTile(data, dataSize, flags, lastRef, result);
}

unsigned int mmgen_dt_nav_mesh_remove_tile(void* navMesh, dtTileRef ref)
{
    return static_cast<dtNavMesh*>(navMesh)->removeTile(ref, nullptr, nullptr);
}

} // extern "C"
