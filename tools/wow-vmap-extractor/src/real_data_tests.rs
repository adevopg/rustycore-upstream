//! Smoke test of the WDT/ADT/WMO/M2 parsers on a real local client install, without
//! the DB2 step (`#[ignore]`: needs a client). The storage is only read.
//!
//! `ParsMapFiles` opens `World\Maps\<dir>\<dir>.wdt` by name; newer roots have no name
//! hashes for those paths, so this test opens the WDT by FileDataID instead and then runs
//! the same per-map loop (`WDTFile::init`, `GetMap`/`ADTFile::init` for 64x64 tiles).
//!
//! Environment (defaults in parentheses):
//! - `VMAP_REAL_INSTALL` (the battlenet wine install used during development)
//! - `VMAP_REAL_PRODUCT` (`wow_classic_beta`)
//! - `VMAP_REAL_OUT`: output directory, gets `Buildings/` (required)
//! - `VMAP_REAL_WDTS`: `fdid:directory:mapId,...` (`775971:Azeroth:0,782779:Kalimdor:1`)
//!
//! `cargo test -p wow-vmap-extractor real_data -- --ignored --nocapture`

use std::path::{Path, PathBuf};

use wow_casc::Storage;

use crate::cascfile::CascFile;
use crate::vmapexport::VmapExport;
use crate::wdtfile::WdtFile;

const DEFAULT_INSTALL: &str =
    "/home/inna/Games/battlenet/drive_c/Program Files (x86)/World of Warcraft";

#[test]
#[ignore = "needs a local client install (VMAP_REAL_INSTALL / VMAP_REAL_OUT)"]
fn real_data_maps_by_wdt_file_data_id() {
    let install = std::env::var("VMAP_REAL_INSTALL").unwrap_or_else(|_| DEFAULT_INSTALL.into());
    let product = std::env::var("VMAP_REAL_PRODUCT").unwrap_or_else(|_| "wow_classic_beta".into());
    let out = PathBuf::from(std::env::var("VMAP_REAL_OUT").expect("set VMAP_REAL_OUT"));
    let wdts = std::env::var("VMAP_REAL_WDTS")
        .unwrap_or_else(|_| "775971:Azeroth:0,782779:Kalimdor:1".into());

    // GetInstalledLocalesMask, then open with the installed mask
    let installed = Storage::open(Path::new(&install), &product, 0)
        .expect("open storage")
        .installed_locales_mask();
    let storage = Storage::open(Path::new(&install), &product, installed).expect("open storage");
    println!(
        "build {} installed locales 0x{installed:X}",
        storage.build_number()
    );

    let work_dir = out.join("Buildings");
    std::fs::create_dir_all(&work_dir).expect("create Buildings");
    let mut ctx = VmapExport::new(&storage, work_dir.clone(), false);

    for spec in wdts.split(',') {
        let mut parts = spec.split(':');
        let fdid: u32 = parts.next().and_then(|v| v.parse().ok()).expect("fdid");
        let directory = parts.next().expect("directory").to_owned();
        let map_id: u32 = parts.next().and_then(|v| v.parse().ok()).expect("map id");

        let file = CascFile::open_id(&storage, fdid, &format!("WDT {fdid}"), true);
        let mut wdt = WdtFile::new(file, directory.clone(), false);
        assert!(
            wdt.init(&mut ctx, map_id),
            "WDT {fdid} ({directory}) init failed"
        );
        let start = std::time::Instant::now();
        let (mut tiles, mut ok) = (0, 0);
        for x in 0..64 {
            for y in 0..64 {
                if let Some(result) = wdt.init_adt(&mut ctx, x, y, map_id, map_id) {
                    tiles += 1;
                    ok += usize::from(result);
                }
            }
        }
        println!(
            "map {map_id} {directory}: {tiles} tiles in MAIN, {ok} obj0 ADTs parsed, {:.1}s",
            start.elapsed().as_secs_f32()
        );
    }

    let mut files = 0usize;
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(&work_dir)
        .expect("read Buildings")
        .flatten()
    {
        files += 1;
        let len = entry.metadata().map_or(0, |m| m.len());
        bytes += len;
        if entry.file_name() == "dir_bin" {
            println!("dir_bin: {len} bytes");
        }
    }
    println!("{files} files in Buildings ({bytes} bytes)");
    assert!(files > 1, "nothing extracted");
}
