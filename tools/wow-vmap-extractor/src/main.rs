//! Port of TrinityCore `vmap4_extractor` (tag `TDB343.24081`, client 3.4.3.54261):
//! extracts WMO/M2 collision geometry and model spawns from a local CASC storage into
//! the raw `Buildings/` format read by the vmap assembler.
//!
//! This file ports the command line / storage part of
//! `src/tools/vmap4_extractor/vmapexport.cpp` (`processArgv`, `RetardCheck`,
//! `OpenCascStorage`, `GetInstalledLocalesMask`, `main`). The extractor is
//! single-threaded, like the C++ tool.

// The port keeps the C++ integer semantics (int/uint32 reinterpretation, float compares
// in G3D's Euler decomposition) on purpose; docs cite C++/CascLib identifiers verbatim.
#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::similar_names,
    clippy::too_many_lines
)]

mod adtfile;
mod cascfile;
mod db2;
mod gameobject_extract;
mod model;
mod modelheaders;
mod names;
mod std_unordered_set;
mod vec3d;
mod vmapexport;
mod wdtfile;
mod wmo;

#[cfg(test)]
mod real_data_tests;
#[cfg(test)]
mod scenario_tests;

use std::io::Write;
use std::path::{Path, PathBuf};

use wow_casc::{Storage, locale};

use crate::vmapexport::{SZ_WORK_DIR_WMO, VMAP_MAGIC, VmapExport, read_map_entries};

/// `TOTAL_LOCALES`.
const TOTAL_LOCALES: usize = 12;
/// `LOCALE_none`.
const LOCALE_NONE: usize = 9;

/// `localeNames` (`src/common/Common.cpp`).
const LOCALE_NAMES: [&str; TOTAL_LOCALES] = locale::TC_LOCALE_NAMES;

/// `WowLocaleToCascLocaleFlags` of `vmapexport.cpp` (a locale *mask* per
/// `LocaleConstant`, unlike `WowLocaleToCascLocaleBit`).
const WOW_LOCALE_TO_CASC_LOCALE_FLAGS: [u32; TOTAL_LOCALES] = [
    locale::ENUS | locale::ENGB,
    locale::KOKR,
    locale::FRFR,
    locale::DEDE,
    locale::ZHCN,
    locale::ZHTW,
    locale::ESES,
    locale::ESMX,
    locale::RURU,
    0,
    locale::PTBR | locale::PTPT,
    locale::ITIT,
];

/// Command line state of `processArgv`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    input_path: PathBuf,
    precise_vector_data: bool,
    casc_product: String,
    casc_region: String,
    use_remote_casc: bool,
    dbc_locale: u32,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            input_path: PathBuf::new(),
            precise_vector_data: false,
            casc_product: "wow_classic".to_owned(),
            casc_region: "eu".to_owned(),
            use_remote_casc: false,
            dbc_locale: 0,
        }
    }
}

/// `processArgv`: `None` when the usage was printed.
fn process_argv(argv: &[String], version_string: &str) -> Option<Args> {
    let mut args = Args::default();
    let mut result = true;
    let arg1 = argv.get(1).map(String::as_str);

    let mut i = 1;
    while i < argv.len() {
        let a = argv[i].as_str();
        if a == "-s" {
            args.precise_vector_data = false;
        } else if a == "-d" {
            if i + 1 < argv.len() {
                args.input_path = PathBuf::from(&argv[i + 1]);
                i += 1;
            } else {
                result = false;
            }
        } else if arg1 == Some("-?") {
            // the C++ compares argv[1] here, not argv[i]
            result = false;
        } else if a == "-l" {
            args.precise_vector_data = true;
        } else if a == "-p" {
            if i + 1 < argv.len() && !argv[i + 1].is_empty() {
                i += 1;
                args.casc_product.clone_from(&argv[i]);
            } else {
                result = false;
            }
        } else if a == "-c" {
            args.use_remote_casc = true;
        } else if a == "-r" {
            if i + 1 < argv.len() && !argv[i + 1].is_empty() {
                i += 1;
                args.casc_region.clone_from(&argv[i]);
            } else {
                result = false;
            }
        } else if a == "-dl" {
            if i + 1 < argv.len() && !argv[i + 1].is_empty() {
                for (l, name) in LOCALE_NAMES.iter().enumerate() {
                    if argv[i + 1] == *name {
                        args.dbc_locale = 1 << l;
                    }
                }
                i += 1;
            } else {
                result = false;
            }
        } else {
            result = false;
            break;
        }
        i += 1;
    }

    if !result {
        let argv0 = argv.first().map_or("wow-vmap-extractor", String::as_str);
        println!("Extract {version_string}.");
        println!("{argv0} [-?][-s][-l][-d <path>][-p <product>]");
        println!("   -s  : (default) small size (data size optimization), ~500MB less vmap data.");
        println!("   -l  : large size, ~500MB more vmap data. (might contain more details)");
        println!("   -d  <path>: Path to the vector data source folder.");
        println!("   -p  <product>: which installed product to open (wow/wowt/wow_beta)");
        println!("   -c  use remote casc");
        println!("   -r  set remote casc region - standard: eu");
        println!("   -dl dbc locale");
        println!("   -? : This message.");
        return None;
    }

    Some(args)
}

/// `boost::filesystem::canonical(input_path)` (an empty path is the current directory).
fn canonical_input(input_path: &Path) -> std::io::Result<PathBuf> {
    if input_path.as_os_str().is_empty() {
        std::env::current_dir()?.canonicalize()
    } else {
        input_path.canonicalize()
    }
}

/// `CASC::Storage::Open(canonical(input_path) / "Data", localeMask, product)`.
///
/// `wow-casc` takes the install directory (the one containing `.build.info` and
/// `Data/`), which is what CascLib derives from the `Data` path the C++ passes.
fn open_storage(args: &Args, locale_mask: u32) -> Result<Storage, String> {
    let install = canonical_input(&args.input_path).map_err(|e| e.to_string())?;
    let storage_dir = install.join("Data");
    match Storage::open(&install, &args.casc_product, locale_mask) {
        Ok(storage) => {
            println!("Opened casc storage '{}'", storage_dir.to_string_lossy());
            // CASC::Storage::LoadOnlineTactKeys downloads wowdev/TACTKeys; not ported.
            println!(
                "Failed to load additional online encryption keys, some files might not be extracted."
            );
            Ok(storage)
        }
        Err(e) => {
            println!(
                "Error opening casc storage '{}': {e}",
                storage_dir.to_string_lossy()
            );
            Err(format!("{}", storage_dir.to_string_lossy()))
        }
    }
}

/// `OpenCascStorage(locale)`.
fn open_casc_storage(args: &Args, locale: usize) -> Option<Storage> {
    if args.use_remote_casc {
        // CASC::Storage::OpenRemote is not supported by wow-casc
        println!("Unable to open remote casc fallback to local casc");
    }

    match canonical_input(&args.input_path) {
        Err(e) => {
            println!("error opening casc storage : {e}");
            None
        }
        Ok(_) => match open_storage(args, WOW_LOCALE_TO_CASC_LOCALE_FLAGS[locale]) {
            Ok(storage) => Some(storage),
            Err(storage_dir) => {
                println!(
                    "error opening casc storage '{storage_dir}' locale {}",
                    LOCALE_NAMES[locale]
                );
                None
            }
        },
    }
}

/// `GetInstalledLocalesMask`.
fn get_installed_locales_mask(args: &Args) -> u32 {
    if args.use_remote_casc {
        println!("Unable to open remote casc fallback to local casc");
    }
    if let Err(e) = canonical_input(&args.input_path) {
        println!("Unable to determine installed locales mask: {e}");
        return 0;
    }
    match open_storage(args, 0) {
        Ok(storage) => storage.installed_locales_mask(),
        Err(_) => 0,
    }
}

/// `RetardCheck`: refuse MPQ-based (3.3.5) clients.
fn retard_check(args: &Args) -> bool {
    if args.use_remote_casc {
        return true;
    }

    let storage_dir = match canonical_input(&args.input_path) {
        Ok(p) => p.join("Data"),
        Err(e) => {
            println!("Error checking client version: {e}");
            return true;
        }
    };
    let entries = match std::fs::read_dir(&storage_dir) {
        Ok(entries) => entries,
        Err(e) => {
            println!("Error checking client version: {e}");
            return true;
        }
    };
    for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "MPQ") {
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
    true
}

/// `mkdir(szWorkDirWmo, 0711)`; `true` when created or already existing.
fn create_work_dir(path: &Path) -> bool {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o711);
    }
    match builder.create(path) {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::AlreadyExists,
    }
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    // Trinity::Banner::Show("VMAP data extractor", ...)
    println!("VMAP data extractor (RustyCore port of TrinityCore vmap4_extractor, TDB343.24081)");
    println!("<Ctrl-C> to stop.");

    let mut success = true;

    let argv: Vec<String> = std::env::args().collect();
    // Use command line arguments, when some
    let Some(args) = process_argv(&argv, VMAP_MAGIC) else {
        return 1;
    };

    if !retard_check(&args) {
        return 1;
    }

    // some simple check if working dir is dirty
    let work_dir = PathBuf::from(SZ_WORK_DIR_WMO);
    if std::fs::metadata(work_dir.join("dir")).is_ok()
        || std::fs::metadata(work_dir.join("dir_bin")).is_ok()
    {
        println!("Your output directory seems to be polluted, please use an empty directory!");
        print!("<press return to exit>");
        let _ = std::io::stdout().flush();
        let mut garbage = [0u8; 1];
        // `return scanf("%c", garbage)`: 1 when a character was read, EOF (-1) otherwise
        return match std::io::Read::read(&mut std::io::stdin(), &mut garbage) {
            Ok(1) => 1,
            _ => -1,
        };
    }

    println!("Extract {VMAP_MAGIC}. Beginning work ....");
    // Create the working directory
    if !create_work_dir(&work_dir) {
        success = false;
    }

    let installed_locales_mask = get_installed_locales_mask(&args);
    let mut first_locale: i32 = -1;
    let mut casc_storage: Option<Storage> = None;
    for i in 0..TOTAL_LOCALES {
        if args.dbc_locale != 0 && args.dbc_locale & (1 << i) == 0 {
            continue;
        }

        if i == LOCALE_NONE {
            continue;
        }

        if installed_locales_mask & WOW_LOCALE_TO_CASC_LOCALE_FLAGS[i] == 0 {
            continue;
        }

        let Some(storage) = open_casc_storage(&args, i) else {
            continue;
        };

        first_locale = i as i32;
        let build = storage.build_number();
        if build == 0 {
            continue;
        }

        println!(
            "Detected client build {build} for locale {}\n",
            LOCALE_NAMES[i]
        );
        casc_storage = Some(storage);
        break;
    }

    match casc_storage {
        Some(storage) if first_locale != -1 => extract(&storage, &args, work_dir, success),
        // FirstLocale == -1 (the C++ would crash instead when the last opened locale
        // reported build 0 and reset the storage)
        _ => {
            println!("FATAL ERROR: No locales defined, unable to continue.");
            1
        }
    }
}

/// The extraction part of `main` after the storage is open.
fn extract(storage: &Storage, args: &Args, work_dir: PathBuf, success: bool) -> i32 {
    let mut ctx = VmapExport::new(storage, work_dir, args.precise_vector_data);

    // Extract models, listed in GameObjectDisplayInfo.dbc
    ctx.extract_gameobject_models();

    //map.dbc
    if success {
        print!("Read Map.dbc file... ");
        let _ = std::io::stdout().flush();

        let db2 = match db2::load(storage, &db2::MAP_META) {
            Ok(db2) => db2,
            Err(e) => {
                println!(
                    "Fatal error: Invalid Map.db2 file format! {}\n{}",
                    e.casc_error, e.what
                );
                std::process::exit(1);
            }
        };

        let (map_ids, maps_that_are_parents) = read_map_entries(&db2);

        println!("Done! ({} maps loaded)", map_ids.len());
        ctx.pars_map_files(&map_ids, &maps_that_are_parents);
    }

    println!();
    if !success {
        println!(
            "ERROR: Extract {VMAP_MAGIC}. Work NOT complete.\n   Precise vector data={}.\nPress any key.",
            i32::from(args.precise_vector_data)
        );
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    }

    println!("Extract {VMAP_MAGIC}. Work complete. No errors.");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(a: &[&str]) -> Vec<String> {
        std::iter::once("vmap4extractor")
            .chain(a.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn process_argv_defaults_and_flags() {
        let args = process_argv(&argv(&[]), VMAP_MAGIC).expect("valid");
        assert_eq!(args, Args::default());
        assert_eq!(args.casc_product, "wow_classic");

        let args = process_argv(
            &argv(&[
                "-l",
                "-d",
                "/wow",
                "-p",
                "wow_classic_era",
                "-c",
                "-r",
                "us",
                "-dl",
                "deDE",
            ]),
            VMAP_MAGIC,
        )
        .expect("valid");
        assert!(args.precise_vector_data);
        assert_eq!(args.input_path, PathBuf::from("/wow"));
        assert_eq!(args.casc_product, "wow_classic_era");
        assert!(args.use_remote_casc);
        assert_eq!(args.casc_region, "us");
        assert_eq!(args.dbc_locale, 1 << 3);

        // -s after -l resets to small
        let args = process_argv(&argv(&["-l", "-s"]), VMAP_MAGIC).expect("valid");
        assert!(!args.precise_vector_data);
        // unknown -dl locale keeps "all locales"
        let args = process_argv(&argv(&["-dl", "xxXX"]), VMAP_MAGIC).expect("valid");
        assert_eq!(args.dbc_locale, 0);
    }

    #[test]
    fn process_argv_rejects() {
        assert!(process_argv(&argv(&["-?"]), VMAP_MAGIC).is_none());
        assert!(process_argv(&argv(&["-d"]), VMAP_MAGIC).is_none());
        assert!(process_argv(&argv(&["-p", ""]), VMAP_MAGIC).is_none());
        assert!(process_argv(&argv(&["-x"]), VMAP_MAGIC).is_none());
        assert!(process_argv(&argv(&["-l", "-?"]), VMAP_MAGIC).is_none());
    }
}
