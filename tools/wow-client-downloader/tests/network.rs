//! Network integration tests against the real Blizzard CDN and the
//! archive.wow.tools mirror. Ignored by default; run with
//! `cargo test -p wow-client-downloader -- --ignored`.
//!
//! The subset download fetches every archive `.index` (about 123 MiB) and
//! takes several minutes; set `WCD_TEST_OUTPUT` to reuse an output directory
//! (its `Data/indices` then act as a cache).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_wow-client-downloader");

fn run(args: &[&str]) -> String {
    let out = Command::new(BIN).args(args).output().expect("run binary");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{args:?} failed:\n{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    text
}

#[test]
#[ignore = "network"]
fn dry_run_windows_eses() {
    let dir = tempfile::tempdir().unwrap();
    let out_dir = dir.path().join("out");
    let text = run(&[
        "download",
        "--output",
        out_dir.to_str().unwrap(),
        "--locale",
        "esES",
        "--dry-run",
    ]);
    assert!(text.contains("Windows x86_64 EU? esES speech?:Windows x86_64 EU? esES text?"));
    assert!(text.contains("Dry run: "), "{text}");
    assert!(!out_dir.exists(), "dry run writes nothing");
}

/// Kills the `serve` child even when an assertion fails.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "network"]
fn serve_answers_versions_cdns_and_configs() {
    let mut child = KillOnDrop(
        Command::new(BIN)
            .args(["serve", "--port", "0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut first = String::new();
    let mut stdout = BufReader::new(child.0.stdout.take().unwrap());
    stdout.read_line(&mut first).unwrap();
    // Keep draining the request log so the pipe stays open.
    std::thread::spawn(move || std::io::copy(&mut stdout, &mut std::io::sink()));
    let base = first.trim().rsplit(' ').next().unwrap().to_owned();
    let get = |path: &str| {
        let mut resp = ureq::get(&format!("{base}{path}")).call().unwrap();
        resp.body_mut()
            .with_config()
            .limit(1 << 20)
            .read_to_vec()
            .unwrap()
    };
    let versions = String::from_utf8(get("/wow_classic/versions")).unwrap();
    assert!(versions.contains("c91609c69ed2ab39d44039390a1be969"));
    let cdns = String::from_utf8(get("/wow_classic/cdns")).unwrap();
    assert!(cdns.contains("tpr/configs/data"));
    let config = get("/tpr/wow/config/c9/16/c91609c69ed2ab39d44039390a1be969");
    assert!(String::from_utf8_lossy(&config).contains("WOW-54261patch3.4.3_ClassicRetail"));
}

#[test]
#[ignore = "network, several minutes"]
fn subset_download_reads_back_map_db2() {
    let tmp = tempfile::tempdir().unwrap();
    let out: PathBuf =
        std::env::var_os("WCD_TEST_OUTPUT").map_or_else(|| tmp.path().join("wow"), PathBuf::from);
    run(&[
        "download",
        "--output",
        out.to_str().unwrap(),
        "--locale",
        "esES",
        "--limit-files",
        "20",
        "--include-fdid",
        "1349477",
    ]);
    let storage = wow_casc::Storage::open(&out, "wow_classic", wow_casc::locale::ESES).unwrap();
    assert_eq!(storage.build_number(), 54261);
    let map = storage
        .read_file_by_id(1_349_477, 0)
        .unwrap()
        .expect("Map.db2");
    assert_eq!(&map[..4], b"WDC4");
    let by_name = storage
        .read_file_by_name("DBFilesClient\\Map.db2", 0)
        .unwrap()
        .expect("Map.db2 by name");
    assert_eq!(by_name, map);
    println!(
        "wow-casc: build {}, locales {:#x}, {:?}; Map.db2 {} bytes, magic {:?}",
        storage.build_number(),
        storage.installed_locales_mask(),
        storage.stats(),
        map.len(),
        String::from_utf8_lossy(&map[..4])
    );
    assert!(out.join("_classic_/.flavor.info").is_file());
}
