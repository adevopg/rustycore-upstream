//! Port of TrinityCore `mmaps_generator` (tag TDB343.24081, client
//! 3.4.3.54261): builds Recast/Detour navmesh tiles (`mmaps/<map>.mmap`,
//! `mmaps/<map><y><x>.mmtile`) from `maps/`, `vmaps/` and `dbc/`.
//!
//! This file ports `src/tools/mmaps_generator/PathGenerator.cpp`
//! (`handleArgs`, `checkDirectories`, `LoadLiquid`, `LoadMap`, `main`). Like
//! the C++ tool it works on the current working directory.

// Integer/float conversions intentionally mirror the C++ (uint32/int
// reinterpretation, float(int)), and float comparisons are exact on purpose.
#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::too_many_arguments,
    clippy::similar_names,
    clippy::many_single_char_names,
    // C++ identifiers in docs
    clippy::doc_markdown,
    // keep the C++ float expressions verbatim: `x * -1.f`, `(a + b) / 2`
    clippy::neg_multiply,
    clippy::manual_midpoint,
    clippy::format_push_string,
    clippy::implicit_hasher,
    clippy::chunks_exact_to_as_chunks
)]

mod intermediate_values;
mod map_builder;
mod map_defines;
mod path_common;
mod recast;
mod terrain_builder;

#[cfg(test)]
#[allow(clippy::naive_bytecount, clippy::too_many_lines)]
mod cross_check_tests;
#[cfg(test)]
mod test_data;

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use map_builder::{BuilderOptions, MapBuilder};
use path_common::{GeneratorData, ListFilesResult, MapEntry, c_atof, c_atoi, get_dir_contents};

/// `Info/readme.txt`.
const README: &str = include_str!("readme.txt");

/// Command line state of `main` / `handleArgs`.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    pub mapnum: i32,
    pub tile_x: i32,
    pub tile_y: i32,
    pub max_angle: Option<f32>,
    pub max_angle_not_steep: Option<f32>,
    pub skip_liquid: bool,
    pub skip_continents: bool,
    pub skip_junk_maps: bool,
    pub skip_battlegrounds: bool,
    pub debug_output: bool,
    pub silent: bool,
    pub big_base_unit: bool,
    pub off_mesh_input_path: Option<String>,
    pub file: Option<String>,
    pub threads: u32,
}

impl Args {
    /// Defaults of `main` (`threads = std::thread::hardware_concurrency()`).
    pub fn new(threads: u32) -> Self {
        Self {
            mapnum: -1,
            tile_x: -1,
            tile_y: -1,
            max_angle: None,
            max_angle_not_steep: None,
            skip_liquid: false,
            skip_continents: false,
            skip_junk_maps: true,
            skip_battlegrounds: false,
            debug_output: false,
            silent: false,
            big_base_unit: false,
            off_mesh_input_path: None,
            file: None,
            threads,
        }
    }
}

/// `finish` — prints the message and waits for a key.
fn finish(message: &str, return_value: i32) -> i32 {
    print!("{message}");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut b = [0u8; 1];
    let _ = std::io::stdin().read(&mut b); // Wait for user input
    return_value
}

fn parse_bool_opt(param: &str, target: &mut bool, name: &str, invalid_suffix: &str) {
    match param {
        "true" => *target = true,
        "false" => *target = false,
        _ => println!("invalid option for '{name}', using default{invalid_suffix}"),
    }
}

/// `handleArgs`. `argv[0]` is the program name. `debug_build` enables the
/// `#ifndef NDEBUG` "--allowDebug" guard.
pub fn handle_args(argv: &[String], a: &mut Args, debug_build: bool) -> bool {
    let mut allow_debug = false;
    let mut i = 1;
    // `param = argv[++i]; if (!param) return false;`
    macro_rules! next_param {
        () => {{
            i += 1;
            match argv.get(i) {
                Some(p) => p.as_str(),
                None => return false,
            }
        }};
    }
    while i < argv.len() {
        let arg = argv[i].as_str();
        match arg {
            "--maxAngle" | "--maxAngleNotSteep" => {
                let param = next_param!();
                let maxangle = c_atof(param) as f32;
                if (0.0..=90.0).contains(&maxangle) {
                    if arg == "--maxAngle" {
                        a.max_angle = Some(maxangle);
                    } else {
                        a.max_angle_not_steep = Some(maxangle);
                    }
                } else {
                    println!("invalid option for '{arg}', using default");
                }
            }
            "--threads" => {
                let param = next_param!();
                a.threads = c_atoi(param).max(0) as u32;
            }
            "--file" => {
                a.file = Some(next_param!().to_owned());
            }
            "--tile" => {
                let param = next_param!();
                // strtok(param, ",") twice
                let mut toks = param.split(',').filter(|t| !t.is_empty());
                let stile_x = toks.next();
                let stile_y = toks.next();
                // C++ passes a null token to atoi (crash); treat it as invalid.
                let (Some(stile_x), Some(stile_y)) = (stile_x, stile_y) else {
                    println!("invalid tile coords.");
                    return false;
                };
                let tilex = c_atoi(stile_x);
                let tiley = c_atoi(stile_y);

                if (tilex > 0 && tilex < 64) || (tilex == 0 && stile_x == "0") {
                    a.tile_x = tilex;
                }
                if (tiley > 0 && tiley < 64) || (tiley == 0 && stile_y == "0") {
                    a.tile_y = tiley;
                }

                if a.tile_x < 0 || a.tile_y < 0 {
                    println!("invalid tile coords.");
                    return false;
                }
            }
            "--skipLiquid" => parse_bool_opt(next_param!(), &mut a.skip_liquid, arg, ""),
            "--skipContinents" => parse_bool_opt(next_param!(), &mut a.skip_continents, arg, ""),
            "--skipJunkMaps" => parse_bool_opt(next_param!(), &mut a.skip_junk_maps, arg, ""),
            "--skipBattlegrounds" => {
                parse_bool_opt(next_param!(), &mut a.skip_battlegrounds, arg, "");
            }
            "--debugOutput" => parse_bool_opt(next_param!(), &mut a.debug_output, arg, " true"),
            "--silent" => a.silent = true,
            "--bigBaseUnit" => parse_bool_opt(next_param!(), &mut a.big_base_unit, arg, " false"),
            "--offMeshInput" => a.off_mesh_input_path = Some(next_param!().to_owned()),
            "--allowDebug" => allow_debug = true,
            "--help" | "-?" => {
                println!("{README}");
                a.silent = true;
                return false;
            }
            _ => {
                let map = c_atoi(arg);
                if map > 0 || (map == 0 && arg == "0") {
                    a.mapnum = map;
                } else {
                    println!("invalid map id");
                    return false;
                }
            }
        }
        i += 1;
    }

    if debug_build && !allow_debug {
        finish(
            "Build mmaps_generator in RelWithDebInfo or Release mode or it will take hours to complete!!!\nUse '--allowDebug' argument if you really want to run this tool in Debug.\n",
            -2,
        );
        a.silent = true;
        return false;
    }

    true
}

/// `checkDirectories` — returns the `dbc/` entries (locales) on success.
pub fn check_directories(base: &Path, debug_output: bool) -> Option<Vec<String>> {
    let mut dbc_locales = Vec::new();
    if get_dir_contents(&mut dbc_locales, &base.join("dbc"), "*")
        == ListFilesResult::DirectoryNotFound
        || dbc_locales.is_empty()
    {
        println!("'dbc' directory is empty or does not exist");
        return None;
    }

    let mut dir_files = Vec::new();
    if get_dir_contents(&mut dir_files, &base.join("maps"), "*")
        == ListFilesResult::DirectoryNotFound
        || dir_files.is_empty()
    {
        println!("'maps' directory is empty or does not exist");
        return None;
    }

    dir_files.clear();
    if get_dir_contents(&mut dir_files, &base.join("vmaps"), "*.vmtree")
        == ListFilesResult::DirectoryNotFound
        || dir_files.is_empty()
    {
        println!("'vmaps' directory is empty or does not exist");
        return None;
    }

    dir_files.clear();
    if get_dir_contents(&mut dir_files, &base.join("mmaps"), "*")
        == ListFilesResult::DirectoryNotFound
        && std::fs::create_dir(base.join("mmaps")).is_err()
    {
        println!("'mmaps' directory does not exist and failed to create it");
        return None;
    }

    dir_files.clear();
    if debug_output
        && get_dir_contents(&mut dir_files, &base.join("meshes"), "*")
            == ListFilesResult::DirectoryNotFound
        && std::fs::create_dir(base.join("meshes")).is_err()
    {
        println!(
            "'meshes' directory does not exist and failed to create it (no place to put debugOutput files)"
        );
        return None;
    }

    Some(dbc_locales)
}

fn db2_error_exit(e: &anyhow::Error, silent: bool, code: i32) -> ! {
    if silent {
        std::process::exit(code);
    }
    std::process::exit(finish(&format!("{e:#}"), code));
}

/// `LoadLiquid` — LiquidType id -> `SoundBank` (meta field 3).
pub fn load_liquid(base: &Path, locale: &str) -> anyhow::Result<HashMap<u32, u8>> {
    let path = base.join("dbc").join(locale).join("LiquidType.db2");
    let db2 = wow_data::wdc4::Wdc4Reader::open(&path)?;
    let mut liquid_data = HashMap::new();
    for idx in 0..db2.record_count() {
        liquid_data.insert(db2.record_id(idx), db2.get_field_u8(idx, 3));
    }
    Ok(liquid_data)
}

/// One Map.db2 record as read by `LoadMap`.
#[derive(Debug, Clone, Copy, Default)]
pub struct MapRecord {
    pub id: u32,
    pub map_type: u8,
    pub instance_type: u8,
    pub parent_map_id: u16,
    pub cosmetic_parent_map_id: u16,
    pub flags1: i32,
}

/// Loop body of `LoadMap`.
pub fn apply_map_record(
    r: &MapRecord,
    map_data: &mut HashMap<u32, Vec<u32>>,
    map_store: &mut HashMap<u32, MapEntry>,
) {
    map_data.entry(r.id).or_default();
    let mut parent_map_id = r.parent_map_id as i16;
    if parent_map_id < 0 {
        parent_map_id = r.cosmetic_parent_map_id as i16;
    }
    if parent_map_id != -1 {
        map_data
            .entry(i32::from(parent_map_id) as u32)
            .or_default()
            .push(r.id);
    }

    let map = map_store.entry(r.id).or_default();
    map.map_type = r.map_type;
    map.instance_type = r.instance_type as i8;
    map.parent_map_id = parent_map_id;
    map.flags = r.flags1;
}

/// `LoadMap` — fills `sMapStore` and the vmap parent/child table. Meta field
/// indices (`MapLoadInfo`): MapType 6, InstanceType 7, ParentMapID 12,
/// CosmeticParentMapID 13, Flags[0] 21.
pub fn load_map(
    base: &Path,
    locale: &str,
    map_store: &mut HashMap<u32, MapEntry>,
) -> anyhow::Result<HashMap<u32, Vec<u32>>> {
    let path = base.join("dbc").join(locale).join("Map.db2");
    let db2 = wow_data::wdc4::Wdc4Reader::open(&path)?;
    let mut map_data: HashMap<u32, Vec<u32>> = HashMap::new();
    for idx in 0..db2.record_count() {
        let record = MapRecord {
            id: db2.record_id(idx),
            map_type: db2.get_field_u8(idx, 6),
            // Signed meta fields: `DB2FileLoader::RecordGetVarInt` sign-extends
            // `SignedImmediate` (bitpacked signed) columns before the
            // `GetUInt*` truncation, so e.g. an 11-bit CosmeticParentMapID of
            // -1 reads as 0xFFFF, not 0x7FF.
            instance_type: db2.get_field_i8(idx, 7) as u8,
            parent_map_id: db2.get_field_i16(idx, 12) as u16,
            cosmetic_parent_map_id: db2.get_field_i16(idx, 13) as u16,
            flags1: db2.get_field_i32(idx, 21),
        };
        apply_map_record(&record, &mut map_data, map_store);
    }
    Ok(map_data)
}

/// `Trinity::Banner::Show`.
fn show_banner() {
    println!(
        "RustyCore wow-mmaps-generator {} (MMAP generator)",
        env!("CARGO_PKG_VERSION")
    );
    println!("<Ctrl-C> to stop.\n");
}

/// `secsToTimeString(secs, TimeFormat::FullText)`.
pub fn secs_to_time_string(time_in_secs: u64) -> String {
    let secs = time_in_secs % 60;
    let minutes = time_in_secs % 3600 / 60;
    let hours = time_in_secs % 86400 / 3600;
    let days = time_in_secs / 86400;
    let mut ss = String::new();
    if days != 0 {
        ss += &format!("{days}{}", if days == 1 { " Day " } else { " Days " });
    }
    if hours != 0 {
        ss += &format!("{hours}{}", if hours <= 1 { " Hour " } else { " Hours " });
    }
    if minutes != 0 {
        ss += &format!(
            "{minutes}{}",
            if minutes == 1 {
                " Minute "
            } else {
                " Minutes "
            }
        );
    }
    if secs != 0 || (days == 0 && hours == 0 && minutes == 0) {
        ss += &format!("{secs}{}", if secs <= 1 { " Second." } else { " Seconds." });
    }
    ss
}

/// Runs the generator in `base` with already parsed arguments and loaded
/// DB2 data (the part of `main` after `LoadMap`).
pub fn run_builder(a: &Args, base: &Path, data: Arc<GeneratorData>) {
    let opts = BuilderOptions {
        max_walkable_angle: a.max_angle,
        max_walkable_angle_not_steep: a.max_angle_not_steep,
        skip_liquid: a.skip_liquid,
        skip_continents: a.skip_continents,
        skip_junk_maps: a.skip_junk_maps,
        skip_battlegrounds: a.skip_battlegrounds,
        debug_output: a.debug_output,
        big_base_unit: a.big_base_unit,
        mapid: a.mapnum,
        off_mesh_file_path: a.off_mesh_input_path.clone(),
        threads: a.threads,
    };
    let mut builder = MapBuilder::new(opts, base, data);

    let start = Instant::now();
    if let Some(file) = &a.file {
        builder.build_mesh_from_file(file);
    } else if a.tile_x > -1 && a.tile_y > -1 && a.mapnum >= 0 {
        builder.build_single_tile(a.mapnum as u32, a.tile_x as u32, a.tile_y as u32);
    } else if a.mapnum >= 0 {
        builder.build_maps(Some(a.mapnum as u32));
    } else {
        builder.build_maps(None);
    }

    if !a.silent {
        println!(
            "Finished. MMAPS were built in {}",
            secs_to_time_string(start.elapsed().as_secs())
        );
    }
}

fn real_main() -> i32 {
    show_banner();

    let threads = std::thread::available_parallelism().map_or(0, |n| n.get() as u32);
    let mut a = Args::new(threads);
    let argv: Vec<String> = std::env::args().collect();

    if !handle_args(&argv, &mut a, cfg!(debug_assertions)) {
        return if a.silent {
            -1
        } else {
            finish("You have specified invalid parameters", -1)
        };
    }

    if a.mapnum == -1 && a.debug_output {
        if a.silent {
            return -2;
        }
        println!("You have specifed debug output, but didn't specify a map to generate.");
        println!("This will generate debug output for ALL maps.");
        print!("Are you sure you want to continue? (y/n) ");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut b = [0u8; 1];
        if std::io::stdin().read(&mut b).unwrap_or(0) != 1 || b[0] != b'y' {
            return 0;
        }
    }

    let base = Path::new(".");
    let Some(dbc_locales) = check_directories(base, a.debug_output) else {
        return if a.silent {
            -3
        } else {
            finish("Press ENTER to close...", -3)
        };
    };

    let mut data = GeneratorData::default();
    data.liquid_types = match load_liquid(base, &dbc_locales[0]) {
        Ok(d) => d,
        Err(e) => db2_error_exit(&e, a.silent, -5),
    };
    data.map_data_for_vmap = match load_map(base, &dbc_locales[0], &mut data.map_store) {
        Ok(d) => d,
        Err(e) => db2_error_exit(&e, a.silent, -4),
    };

    run_builder(&a, base, Arc::new(data));
    0
}

fn main() {
    std::process::exit(real_main());
}

#[cfg(test)]
mod tests;
