//! End-to-end runs of the generator on the synthetic fixture
//! (`test_data.rs`), and the byte-for-byte comparison against the C++
//! reference build of TrinityCore's mmaps_generator.
//!
//! The reference (`MMAPS_CPP_REF`) is TrinityCore TDB343.24081's unmodified
//! `PathGenerator.cpp`, `MapBuilder.cpp`, `TerrainBuilder.cpp`,
//! `IntermediateValues.cpp`, Collision (VMapManager2/MapTree/ModelInstance/
//! WorldModel/BIH), G3D and `dep/recastnavigation`, compiled with `-O2
//! -DNDEBUG`; only logging, banner, timer and the DB2 loader are stubbed (the
//! stub reads the text stand-ins the fixture writes as `dbc/enUS/*.db2`).

use std::collections::BTreeMap;
use std::path::Path;

use crate::test_data::{build_fixture, fixture_data, fresh_dir};
use crate::{Args, check_directories, handle_args, run_builder};

fn run_rust(dir: &Path, args: &[&str]) {
    let mut argv = vec!["mmaps_generator".to_owned()];
    for a in args {
        // the off-mesh path is relative to the working directory in C++
        if *a == "offmesh.txt" {
            argv.push(dir.join(a).to_string_lossy().into_owned());
        } else {
            argv.push((*a).to_owned());
        }
    }
    let mut a = Args::new(4);
    assert!(handle_args(&argv, &mut a, false));
    let locales = check_directories(dir, a.debug_output).expect("fixture directories");
    assert_eq!(locales, vec!["enUS".to_owned()]);
    run_builder(&a, dir, fixture_data());
}

fn collect(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for sub in ["mmaps", "meshes"] {
        let Ok(rd) = std::fs::read_dir(dir.join(sub)) else {
            continue;
        };
        for e in rd {
            let e = e.unwrap();
            out.insert(
                format!("{sub}/{}", e.file_name().to_string_lossy()),
                std::fs::read(e.path()).unwrap(),
            );
        }
    }
    out
}

/// Checks the `.mmtile` header and the embedded `dtMeshHeader`.
fn check_tile(bytes: &[u8], uses_liquids: bool) -> i32 {
    let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    assert_eq!(u32_at(0), 0x4d4d_4150);
    assert_eq!((u32_at(4), u32_at(8)), (7, 15));
    assert_eq!(u32_at(12) as usize, bytes.len() - 20);
    assert_eq!(&bytes[16..20], &[u8::from(uses_liquids), 0, 0, 0]);
    // dtMeshHeader: magic 'DNAV', version, x, y, layer, userId, polyCount
    assert_eq!(u32_at(20), u32::from_be_bytes(*b"DNAV"));
    assert_eq!(u32_at(24), 7);
    u32_at(20 + 6 * 4) as i32
}

#[test]
fn rust_pipeline_builds_all_fixture_tiles() {
    let dir = fresh_dir("e2e");
    build_fixture(&dir);
    run_rust(
        &dir,
        &[
            "--silent",
            "--threads",
            "3",
            "--offMeshInput",
            "offmesh.txt",
        ],
    );
    let files = collect(&dir);
    let names: Vec<&str> = files.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        vec![
            "mmaps/0001.mmap",
            "mmaps/00013131.mmtile",
            "mmaps/00013132.mmtile",
            "mmaps/00013133.mmtile",
            "mmaps/00013232.mmtile",
        ]
    );
    assert_eq!(files["mmaps/0001.mmap"].len(), 28);
    for (name, bytes) in &files {
        if name.ends_with(".mmtile") {
            assert!(check_tile(bytes, true) > 0, "{name} has polygons");
        }
    }
    // an existing current tile is skipped: rerunning leaves the bytes alone
    run_rust(
        &dir,
        &[
            "--silent",
            "--threads",
            "1",
            "--offMeshInput",
            "offmesh.txt",
        ],
    );
    assert_eq!(collect(&dir), files);
    // thread count does not change the output
    let dir1 = fresh_dir("e2e-1thread");
    build_fixture(&dir1);
    run_rust(
        &dir1,
        &[
            "--silent",
            "--threads",
            "1",
            "--offMeshInput",
            "offmesh.txt",
        ],
    );
    assert_eq!(collect(&dir1), files);
}

/// Scenarios run by both implementations.
const SCENARIOS: [&[&str]; 5] = [
    &[
        "--silent",
        "--threads",
        "3",
        "--offMeshInput",
        "offmesh.txt",
    ],
    &[
        "2",
        "--tile",
        "32,31",
        "--silent",
        "--offMeshInput",
        "offmesh.txt",
    ],
    &[
        "1",
        "--bigBaseUnit",
        "true",
        "--skipLiquid",
        "true",
        "--maxAngle",
        "70",
        "--maxAngleNotSteep",
        "50",
        "--silent",
        "--threads",
        "2",
    ],
    &[
        "1",
        "--tile",
        "32,31",
        "--debugOutput",
        "true",
        "--silent",
        "--offMeshInput",
        "offmesh.txt",
    ],
    &[
        "1",
        "--maxAngle",
        "85",
        "--maxAngleNotSteep",
        "30",
        "--silent",
        "--threads",
        "4",
    ],
];

/// Byte-for-byte comparison with the C++ reference binary given by
/// `MMAPS_CPP_REF` (see the module docs).
#[test]
#[ignore = "needs the C++ reference build (MMAPS_CPP_REF)"]
fn matches_cpp_reference() {
    let reference = std::env::var("MMAPS_CPP_REF").expect("MMAPS_CPP_REF");
    for (i, args) in SCENARIOS.iter().enumerate() {
        let cpp_dir = fresh_dir(&format!("xcheck-cpp-{i}"));
        let rust_dir = fresh_dir(&format!("xcheck-rust-{i}"));
        build_fixture(&cpp_dir);
        build_fixture(&rust_dir);

        let out = std::process::Command::new(&reference)
            .args(*args)
            .current_dir(&cpp_dir)
            .stdin(std::process::Stdio::null())
            .output()
            .expect("run C++ reference");
        assert!(
            out.status.success(),
            "C++ reference failed: {}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout)
        );
        run_rust(&rust_dir, args);

        let cpp = collect(&cpp_dir);
        let rust = collect(&rust_dir);
        assert_eq!(
            cpp.keys().collect::<Vec<_>>(),
            rust.keys().collect::<Vec<_>>(),
            "scenario {i} {args:?}: file sets differ"
        );
        let mut tiles = 0;
        for (name, bytes) in &cpp {
            let r = &rust[name];
            if let Some(pos) = bytes.iter().zip(r.iter()).position(|(a, b)| a != b) {
                panic!(
                    "scenario {i} {args:?}: {name} differs at byte {pos} (C++ {} bytes, Rust {} bytes)",
                    bytes.len(),
                    r.len()
                );
            }
            assert_eq!(bytes.len(), r.len(), "scenario {i}: {name} length");
            if name.ends_with(".mmtile") {
                tiles += 1;
            }
        }
        assert!(tiles > 0, "scenario {i} produced no tiles");
        eprintln!(
            "scenario {i} {args:?}: {} files identical ({tiles} tiles, {} bytes)",
            cpp.len(),
            cpp.values().map(Vec::len).sum::<usize>()
        );
    }
}
