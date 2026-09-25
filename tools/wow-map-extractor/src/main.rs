//! Port of TrinityCore `map_extractor` (`src/tools/map_extractor`, tag TDB343.24081,
//! client 3.4.3.54261): extracts `dbc/<locale>/*.db2`, `gt/*.txt`, `cameras/*.xxx`
//! and `maps/*.map` + `maps/*.tilelist` from a WoW Classic CASC install.
//!
//! `main` ports `main`, `OpenCascStorage`, `GetInstalledLocalesMask` and `RetardCheck`
//! from `System.cpp`.
//!
//! Usage (same switches as the C++): `wow-map-extractor [-i <client dir>] [-o <output dir>]
//! [-e <mask: 1 maps, 2 dbc, 4 cameras, 8 gt>] [-f 0|1] [-l <locale>] [-p <product>]
//! [-c 0|1] [-r <region>]`.

// Docs cite C++/CascLib identifiers (map_extractor, CascLib, FourCC...) in prose.
#![allow(clippy::doc_markdown)]

mod adt;
mod cameras;
mod casc;
mod cli;
mod convert;
mod db2;
mod db_files_client_list;
mod dbc;
mod fsutil;
mod gametables;
mod loadlib;
mod maps;
mod tables;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use casc::{
    CASC_LOCALE_ALL_WOW, Casc, LOCALE_NAMES, LOCALE_NONE, TOTAL_LOCALES,
    WOW_LOCALE_TO_CASC_LOCALE_FLAGS,
};
use cli::{Config, EXTRACT_CAMERA, EXTRACT_DBC, EXTRACT_GT, EXTRACT_MAP};
use tables::CppFatal;

/// `boost::filesystem::canonical(input_path)`.
fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path)
}

/// Remote CASC (`CASC::Storage::OpenRemote`) is not available in `wow-casc`; the C++
/// then falls back to the local storage with this message.
fn remote_casc_unavailable() {
    println!("Error opening remote casc storage: NOT_SUPPORTED");
    println!("Unable to open remote casc fallback to local casc");
}

/// `OpenCascStorage(locale)`.
fn open_casc_storage(config: &Config, locale: usize) -> Option<Casc> {
    if config.use_remote_casc {
        remote_casc_unavailable();
    }
    let input = match canonical(&config.input_path) {
        Ok(input) => input,
        Err(e) => {
            println!("Error opening CASC storage: {e}");
            return None;
        }
    };
    let storage = Casc::open(
        &input,
        WOW_LOCALE_TO_CASC_LOCALE_FLAGS[locale],
        &config.product,
    );
    if storage.is_none() {
        println!(
            "error opening casc storage '{}' locale {}",
            input.join("Data").display(),
            LOCALE_NAMES[locale]
        );
    }
    storage
}

/// `GetInstalledLocalesMask()`.
fn get_installed_locales_mask(config: &Config) -> u32 {
    if config.use_remote_casc {
        remote_casc_unavailable();
    }
    match canonical(&config.input_path) {
        Ok(input) => Casc::open(&input, CASC_LOCALE_ALL_WOW, &config.product)
            .map_or(0, |s| s.installed_locales_mask()),
        Err(e) => {
            println!("Unable to determine installed locales mask: {e}");
            0
        }
    }
}

/// `RetardCheck()`: refuse MPQ-based (3.3.5) clients.
fn retard_check(config: &Config) -> bool {
    if config.use_remote_casc {
        return true;
    }
    let entries = canonical(&config.input_path).and_then(|p| std::fs::read_dir(p.join("Data")));
    match entries {
        Ok(entries) => {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|e| e == "MPQ") {
                    println!("MPQ files found in Data directory!");
                    println!("This tool works only with World of Warcraft: Battle for Azeroth");
                    println!();
                    println!(
                        "To extract maps for Wrath of the Lich King, rebuild tools using 3.3.5 branch!"
                    );
                    println!();
                    println!("Press ENTER to exit...");
                    let mut line = String::new();
                    let _ = std::io::stdin().read_line(&mut line);
                    return false;
                }
            }
        }
        Err(e) => println!("Error checking client version: {e}"),
    }
    true
}

/// `main`.
fn run(config: &Config, print_progress: bool) -> anyhow::Result<i32> {
    if !retard_check(config) {
        return Ok(1);
    }

    let installed_locales_mask = get_installed_locales_mask(config);
    let mut first_installed_locale: Option<usize> = None;
    let mut build = 0u32;

    for i in 0..TOTAL_LOCALES {
        if config.locale != 0 && config.locale & (1 << i) == 0 {
            continue;
        }
        if i == LOCALE_NONE {
            continue;
        }
        if installed_locales_mask & WOW_LOCALE_TO_CASC_LOCALE_FLAGS[i] == 0 {
            continue;
        }
        let Some(storage) = open_casc_storage(config, i) else {
            continue;
        };

        if config.extract & EXTRACT_DBC == 0 {
            first_installed_locale = Some(i);
            build = storage.build_number();
            if build == 0 {
                continue;
            }
            println!("Detected client build: {build}\n");
            break;
        }

        // Extract DBC files
        let temp_build = storage.build_number();
        if temp_build == 0 {
            continue;
        }

        println!(
            "Detected client build {temp_build} for locale {}\n",
            LOCALE_NAMES[i]
        );
        dbc::extract_db_files_client(&storage, &config.output_path, i)?;
        drop(storage);

        if first_installed_locale.is_none() {
            first_installed_locale = Some(i);
            build = temp_build;
        }
    }

    let Some(first_installed_locale) = first_installed_locale else {
        println!("No locales detected");
        return Ok(0);
    };

    // The C++ ignores a failed re-open here and then dereferences the null storage.
    let reopen = || {
        open_casc_storage(config, first_installed_locale)
            .ok_or_else(|| anyhow::anyhow!("unable to reopen the CASC storage"))
    };

    if config.extract & EXTRACT_CAMERA != 0 {
        cameras::extract_camera_files(&reopen()?, &config.output_path)?;
    }

    if config.extract & EXTRACT_GT != 0 {
        gametables::extract_game_tables(&reopen()?, &config.output_path)?;
    }

    if config.extract & EXTRACT_MAP != 0 {
        let storage = reopen()?;
        maps::extract_maps(
            &storage,
            &maps::MapExtractOptions {
                output_path: &config.output_path,
                build,
                allow_float_to_int: config.allow_float_to_int,
                print_progress,
            },
        )?;
    }

    Ok(0)
}

fn main() {
    // Trinity::Banner::Show("Map & DBC Extractor", ...)
    println!(
        "RustyCore {} {} (Map & DBC Extractor)",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );
    println!("<Ctrl-C> to stop.\n");

    let print_progress = std::io::stdout().is_terminal();
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let args: Vec<String> = std::env::args().collect();
    let prg = args.first().map_or("wow-map-extractor", String::as_str);

    let Ok(config) = cli::handle_args(&args, Config::new(current_dir)) else {
        print!("{}", cli::usage(prg));
        std::process::exit(1);
    };

    let code = match run(&config, print_progress) {
        Ok(code) => code,
        Err(e) => {
            if let Some(fatal) = e.downcast_ref::<CppFatal>() {
                print!("{fatal}");
            } else {
                eprintln!("Fatal error: {e:#}");
            }
            1
        }
    };
    std::process::exit(code);
}
