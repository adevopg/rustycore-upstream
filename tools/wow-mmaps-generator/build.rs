//! Compiles TrinityCore's Recast + Detour (the navmesh building half) and the
//! `extern "C"` shim used by `src/recast.rs`.
//!
//! `vendor/recastnavigation` is a verbatim copy of
//! `dep/recastnavigation` at TrinityCore tag TDB343.24081 (client 3.4.3.54261),
//! i.e. upstream recastnavigation with TrinityCore's local patches
//! (`recastnavigation.diff`, `recastnavigation_2_area_merge.diff`) already
//! applied; `License.txt` is the upstream zlib license. Only the sources
//! mmaps_generator links against are kept (Recast/*, Detour Alloc/Assert/
//! Common/NavMesh/NavMeshBuilder); `DetourNavMeshQuery` and `DetourNode.cpp`
//! (runtime path finding) were dropped.
//!
//! Compiler flags: TrinityCore's `cmake/compiler/{gcc,clang}/settings.cmake`
//! add no floating point flags on x86-64 (no `-ffast-math`, no `-march`), and
//! its release builds define `NDEBUG` (compiles `rcAssert`/`dtAssert` out).
//! We mirror that and additionally pin `-ffp-contract=off` so a
//! `-C target-cpu=native` RUSTFLAGS (which `cc` forwards as `-march`) can never
//! fuse multiply-adds into FMA and change the navmesh bytes. This also
//! matters on aarch64, where GCC/Clang contract to FMA by default: the output
//! then matches TrinityCore built for x86-64 (and the Rust TerrainBuilder
//! port, which never contracts), not a TrinityCore build made on aarch64.

#![allow(clippy::doc_markdown)]

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by Cargo"),
    );
    let rn = root.join("vendor/recastnavigation");
    let recast_inc = rn.join("Recast/Include");
    let recast_src = rn.join("Recast/Source");
    let detour_inc = rn.join("Detour/Include");
    let detour_src = rn.join("Detour/Source");
    let shim = root.join("vendor/shim/mmgen_recast_shim.cpp");

    let recast_files = [
        "Recast.cpp",
        "RecastAlloc.cpp",
        "RecastArea.cpp",
        "RecastAssert.cpp",
        "RecastContour.cpp",
        "RecastFilter.cpp",
        "RecastLayers.cpp",
        "RecastMesh.cpp",
        "RecastMeshDetail.cpp",
        "RecastRasterization.cpp",
        "RecastRegion.cpp",
    ];
    let detour_files = [
        "DetourAlloc.cpp",
        "DetourAssert.cpp",
        "DetourCommon.cpp",
        "DetourNavMesh.cpp",
        "DetourNavMeshBuilder.cpp",
    ];

    println!("cargo:rerun-if-changed={}", rn.display());
    println!("cargo:rerun-if-changed={}", shim.display());

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .include(&recast_inc)
        .include(&detour_inc)
        .define("NDEBUG", None)
        // Recast is unusably slow unoptimised; optimisation level does not
        // change IEEE results without fast-math / contraction.
        .opt_level(2)
        .flag_if_supported("-ffp-contract=off")
        .flag_if_supported("-fno-fast-math")
        .flag_if_supported("-Wno-class-memaccess")
        .flag_if_supported("-Wno-unused-parameter")
        .warnings(false);
    for f in recast_files {
        build.file(recast_src.join(f));
    }
    for f in detour_files {
        build.file(detour_src.join(f));
    }
    build.file(&shim);
    build.compile("mmgen_recastdetour");
}
