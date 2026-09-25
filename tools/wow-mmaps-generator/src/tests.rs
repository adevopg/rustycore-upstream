//! `PathGenerator.cpp` (`handleArgs`, `checkDirectories`, `LoadMap` record
//! logic, `secsToTimeString`) tests.

use super::*;
use crate::test_data::{build_fixture, fresh_dir};

fn parse(args: &[&str]) -> (bool, Args) {
    let mut argv = vec!["mmaps_generator".to_owned()];
    argv.extend(args.iter().map(|s| (*s).to_owned()));
    let mut a = Args::new(8);
    let ok = handle_args(&argv, &mut a, false);
    (ok, a)
}

#[test]
fn defaults() {
    let (ok, a) = parse(&[]);
    assert!(ok);
    assert_eq!(a, Args::new(8));
    assert_eq!((a.mapnum, a.tile_x, a.tile_y), (-1, -1, -1));
    assert!(a.skip_junk_maps && !a.skip_liquid && !a.skip_continents && !a.skip_battlegrounds);
    assert!(!a.debug_output && !a.silent && !a.big_base_unit);
    assert_eq!(
        (a.max_angle, a.max_angle_not_steep, a.threads),
        (None, None, 8)
    );
}

#[test]
fn all_options() {
    let (ok, a) = parse(&[
        "571",
        "--tile",
        "34,46",
        "--maxAngle",
        "70",
        "--maxAngleNotSteep",
        "45.5",
        "--skipLiquid",
        "true",
        "--skipContinents",
        "true",
        "--skipJunkMaps",
        "false",
        "--skipBattlegrounds",
        "true",
        "--debugOutput",
        "true",
        "--silent",
        "--bigBaseUnit",
        "true",
        "--offMeshInput",
        "off.txt",
        "--threads",
        "3",
        "--file",
        "mesh.bin",
        "--allowDebug",
    ]);
    assert!(ok);
    assert_eq!((a.mapnum, a.tile_x, a.tile_y), (571, 34, 46));
    assert_eq!(
        (a.max_angle, a.max_angle_not_steep),
        (Some(70.0), Some(45.5))
    );
    assert!(a.skip_liquid && a.skip_continents && !a.skip_junk_maps && a.skip_battlegrounds);
    assert!(a.debug_output && a.silent && a.big_base_unit);
    assert_eq!(a.off_mesh_input_path.as_deref(), Some("off.txt"));
    assert_eq!(a.file.as_deref(), Some("mesh.bin"));
    assert_eq!(a.threads, 3);
}

#[test]
fn invalid_values_keep_defaults() {
    let (ok, a) = parse(&[
        "--maxAngle",
        "95",
        "--maxAngleNotSteep",
        "-1",
        "--skipLiquid",
        "yes",
        "--skipJunkMaps",
        "no",
        "--threads",
        "-4",
    ]);
    assert!(ok);
    assert_eq!((a.max_angle, a.max_angle_not_steep), (None, None));
    assert!(!a.skip_liquid && a.skip_junk_maps);
    assert_eq!(a.threads, 0);
    // "0" and "0,0" are valid ids / tiles; atof-style prefixes are accepted
    let (ok, a) = parse(&["0", "--tile", "0,0", "--maxAngle", "60deg"]);
    assert!(ok);
    assert_eq!(
        (a.mapnum, a.tile_x, a.tile_y, a.max_angle),
        (0, 0, 0, Some(60.0))
    );
}

#[test]
fn invalid_arguments() {
    assert!(!parse(&["abc"]).0); // invalid map id
    assert!(!parse(&["-3"]).0);
    assert!(!parse(&["1", "--tile", "64,3"]).0);
    assert!(!parse(&["1", "--tile", "x,3"]).0);
    assert!(!parse(&["1", "--tile", "3"]).0);
    assert!(!parse(&["--maxAngle"]).0); // missing parameter
    assert!(!parse(&["--offMeshInput"]).0);
    let (ok, a) = parse(&["--help"]);
    assert!(!ok && a.silent);
    let (ok, a) = parse(&["-?"]);
    assert!(!ok && a.silent);
}

#[test]
fn debug_build_requires_allow_debug() {
    let argv: Vec<String> = ["g", "--allowDebug"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let mut a = Args::new(1);
    assert!(handle_args(&argv, &mut a, true));
    assert!(!a.silent);
}

#[test]
fn time_string() {
    assert_eq!(secs_to_time_string(0), "0 Second.");
    assert_eq!(secs_to_time_string(1), "1 Second.");
    assert_eq!(secs_to_time_string(59), "59 Seconds.");
    assert_eq!(secs_to_time_string(60), "1 Minute ");
    assert_eq!(secs_to_time_string(3725), "1 Hour 2 Minutes 5 Seconds.");
    assert_eq!(secs_to_time_string(2 * 86400 + 7200), "2 Days 2 Hours ");
}

#[test]
fn map_records_like_load_map() {
    let mut map_data = HashMap::new();
    let mut store = HashMap::new();
    let recs = [
        MapRecord {
            id: 1,
            flags1: 2,
            parent_map_id: 0xFFFF,
            cosmetic_parent_map_id: 0xFFFF,
            ..Default::default()
        },
        MapRecord {
            id: 5,
            map_type: 3,
            instance_type: 4,
            parent_map_id: 1,
            cosmetic_parent_map_id: 7,
            ..Default::default()
        },
        MapRecord {
            id: 6,
            parent_map_id: 0xFFFF,
            cosmetic_parent_map_id: 1,
            ..Default::default()
        },
    ];
    for r in &recs {
        apply_map_record(r, &mut map_data, &mut store);
    }
    let mut kids = map_data[&1].clone();
    kids.sort_unstable();
    assert_eq!(kids, vec![5, 6]);
    assert!(map_data[&5].is_empty() && map_data[&6].is_empty());
    assert_eq!(store[&1].parent_map_id, -1);
    assert_eq!(store[&1].flags, 2);
    assert_eq!(store[&5].parent_map_id, 1);
    assert_eq!((store[&5].map_type, store[&5].instance_type), (3, 4));
    assert_eq!(store[&6].parent_map_id, 1);
}

#[test]
fn directory_checks() {
    let dir = fresh_dir("cli-dirs");
    assert!(check_directories(&dir, false).is_none());
    build_fixture(&dir);
    assert_eq!(check_directories(&dir, true), Some(vec!["enUS".to_owned()]));
    assert!(dir.join("mmaps").is_dir() && dir.join("meshes").is_dir());
    std::fs::remove_file(dir.join("vmaps/0001.vmtree")).unwrap();
    std::fs::remove_file(dir.join("vmaps/0002.vmtree")).unwrap();
    assert!(check_directories(&dir, false).is_none());
}
