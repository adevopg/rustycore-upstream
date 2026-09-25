//! Integration tests against a real local `WoW` install (read-only).
//!
//! Ignored by default. Run with
//! `cargo test -p wow-casc --release --test local_install -- --ignored --nocapture`.
//! The install path comes from `WOW_CASC_TEST_INSTALL` and the product from
//! `WOW_CASC_TEST_PRODUCT`; the defaults are the developer's Classic beta
//! install (`wow_classic_beta`, `.build.info` Version 1.60.1.70009, esES).
//! Assertions on specific `FileDataIds` only run for that default product.
//!
//! The storage is opened with `locale::ALL` (like `TrinityCore`'s
//! `GetInstalledLocalesMask` probe); locale-specific files are then read with
//! the installed locale mask, which narrows the selection the way opening the
//! storage for that locale would.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use wow_casc::{Error, Storage, locale};

const DEFAULT_INSTALL: &str =
    "/home/inna/Games/battlenet/drive_c/Program Files (x86)/World of Warcraft";
const DEFAULT_PRODUCT: &str = "wow_classic_beta";
/// `DBFilesClient\Map.db2`.
const MAP_DB2_FDID: u32 = 1_349_477;
/// `World\Maps\Azeroth\Azeroth.wdt`, `World\Maps\Kalimdor\Kalimdor.wdt`.
const WDT_FDIDS: [u32; 2] = [775_971, 782_779];
/// Other DB2 `FileDataIds` (`AreaTable` and two more client tables).
const OTHER_DB2_FDIDS: [u32; 2] = [1_353_545, 1_375_579];

fn install_path() -> PathBuf {
    std::env::var_os("WOW_CASC_TEST_INSTALL")
        .map_or_else(|| PathBuf::from(DEFAULT_INSTALL), PathBuf::from)
}

fn product() -> String {
    std::env::var("WOW_CASC_TEST_PRODUCT").unwrap_or_else(|_| DEFAULT_PRODUCT.to_owned())
}

fn is_default_install() -> bool {
    product() == DEFAULT_PRODUCT && std::env::var_os("WOW_CASC_TEST_INSTALL").is_none()
}

fn storage() -> &'static Storage {
    static STORAGE: OnceLock<Storage> = OnceLock::new();
    STORAGE.get_or_init(|| {
        let start = Instant::now();
        let storage = Storage::open(&install_path(), &product(), locale::ALL)
            .unwrap_or_else(|e| panic!("open {}: {e}", install_path().display()));
        println!(
            "opened {} ({}) in {:.2?}: build {}, installed locales {:#x}, {:?}",
            install_path().display(),
            storage.product(),
            start.elapsed(),
            storage.build_number(),
            storage.installed_locales_mask(),
            storage.stats()
        );
        storage
    })
}

fn magic(data: &[u8]) -> String {
    String::from_utf8_lossy(&data[..4.min(data.len())]).into_owned()
}

fn is_db2(data: &[u8]) -> bool {
    matches!(data.get(..4), Some(b"WDC3" | b"WDC4" | b"WDC5"))
}

/// Chunks of a WDT/ADT file: `(reversed id, payload)`.
fn chunks(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 8 <= data.len() {
        let id: [u8; 4] = data[pos..pos + 4].try_into().unwrap();
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let end = (pos + 8 + size).min(data.len());
        out.push((id, &data[pos + 8..end]));
        pos = end;
    }
    out
}

#[test]
#[ignore = "needs a local WoW install (WOW_CASC_TEST_INSTALL)"]
fn opens_and_reports_build() {
    let s = storage();
    if is_default_install() {
        assert_eq!(s.build_number(), 70009);
        assert_eq!(s.installed_locales_mask(), locale::ESES);
    }
    assert_ne!(s.build_number(), 0);
    assert_ne!(s.installed_locales_mask(), 0);
    assert_eq!(s.product(), product());
    let stats = s.stats();
    assert!(stats.index_entries > 0 && stats.root_entries > 0 && stats.root_name_hashes > 0);
    assert_eq!(stats.open_locale_mask, locale::ALL);

    // Opening for a product that is not in .build.info fails cleanly.
    assert!(matches!(
        Storage::open(&install_path(), "no_such_product", locale::ALL),
        Err(Error::ProductNotFound(_))
    ));
}

#[test]
#[ignore = "needs a local WoW install (WOW_CASC_TEST_INSTALL)"]
fn reads_map_db2() {
    let s = storage();
    let installed = s.installed_locales_mask();
    assert!(s.has_file_id(MAP_DB2_FDID, locale::ALL));
    assert!(s.has_file_id(MAP_DB2_FDID, locale::NONE));
    assert!(s.has_file_id(MAP_DB2_FDID, installed));

    // DB2 files are read with zero-filled encrypted sections, as TrinityCore's
    // DB2CascFileSource does (zerofillEncryptedParts = true).
    let map = s
        .read_file_by_id_zerofill_encrypted(MAP_DB2_FDID, installed)
        .unwrap()
        .expect("Map.db2 present for the installed locale");
    println!(
        "Map.db2 ({installed:#x}): {} bytes, magic {}",
        map.len(),
        magic(&map)
    );
    assert!(is_db2(&map), "unexpected DB2 magic {}", magic(&map));

    // The plain read either succeeds or reports the missing TACT key.
    match s.read_file_by_id(MAP_DB2_FDID, installed) {
        Ok(Some(data)) => assert_eq!(data, map),
        Ok(None) => panic!("Map.db2 vanished"),
        Err(Error::MissingKey(key)) => {
            println!("Map.db2 has sections encrypted with unknown key {key:016X}");
            assert!(!s.has_tact_key(key));
        }
        Err(e) => panic!("Map.db2: {e}"),
    }

    // By name: this root has no name hash for DB2 files, so the name path is
    // exercised through CascLib's `File<id>` fallback and the plain name must
    // agree with it whenever the root knows it.
    let by_file_name = s
        .read_file_by_name_zerofill_encrypted(&format!("File{MAP_DB2_FDID}.db2"), installed)
        .unwrap()
        .expect("Map.db2 by File<id> name");
    assert_eq!(by_file_name, map);
    match s.file_data_id_by_name("DBFilesClient\\Map.db2") {
        Some(id) => {
            assert_eq!(id, MAP_DB2_FDID);
            let by_name = s
                .read_file_by_name_zerofill_encrypted("dbfilesclient/map.db2", installed)
                .unwrap()
                .unwrap();
            assert_eq!(by_name, map);
        }
        None => println!("DBFilesClient\\Map.db2 has no name hash in this root"),
    }

    for id in OTHER_DB2_FDIDS {
        let data = s
            .read_file_by_id_zerofill_encrypted(id, installed)
            .unwrap()
            .expect("DB2 present");
        println!("FDID {id}: {} bytes, magic {}", data.len(), magic(&data));
        assert!(is_db2(&data));
    }

    // Unknown ids / names.
    assert!(
        s.read_file_by_id(u32::MAX - 1, locale::ALL)
            .unwrap()
            .is_none()
    );
    assert!(!s.has_file_id(u32::MAX - 1, locale::ALL));
    assert!(
        s.read_file_by_name("DBFilesClient\\DoesNotExist.db2", locale::ALL)
            .unwrap()
            .is_none()
    );
}

#[test]
#[ignore = "needs a local WoW install (WOW_CASC_TEST_INSTALL)"]
fn reads_by_name_hash() {
    let s = storage();
    // A file that does carry a name hash in the root.
    let name = "Fonts\\FRIZQT__.TTF";
    let id = s
        .file_data_id_by_name(name)
        .expect("FRIZQT__.TTF name hash");
    let by_name = s.read_file_by_name(name, locale::NONE).unwrap().unwrap();
    let by_norm = s
        .read_file_by_name("fonts/frizqt__.ttf", locale::NONE)
        .unwrap()
        .unwrap();
    let by_id = s.read_file_by_id(id, locale::NONE).unwrap().unwrap();
    println!(
        "{name}: FDID {id}, {} bytes, magic {:02x?}",
        by_id.len(),
        &by_id[..4]
    );
    assert_eq!(by_name, by_id);
    assert_eq!(by_norm, by_id);
    // TrueType signature.
    assert_eq!(&by_id[..4], &[0x00, 0x01, 0x00, 0x00]);
    if is_default_install() {
        assert_eq!(id, 615_960);
    }
}

#[test]
#[ignore = "needs a local WoW install (WOW_CASC_TEST_INSTALL)"]
fn reads_wdt_and_adt() {
    let s = storage();
    let installed = s.installed_locales_mask();
    for wdt_id in WDT_FDIDS {
        let wdt = s
            .read_file_by_id(wdt_id, installed)
            .unwrap()
            .expect("continent WDT");
        let ids: Vec<String> = chunks(&wdt)
            .iter()
            .map(|(id, data)| format!("{}({})", magic(id), data.len()))
            .collect();
        println!("WDT {wdt_id}: {} bytes, chunks {ids:?}", wdt.len());
        // Chunk ids are stored reversed: "REVM" = MVER.
        assert_eq!(&wdt[..4], b"REVM");

        // MAID: 64x64 tiles x 8 FileDataIds (root ADT first).
        let maid = chunks(&wdt)
            .into_iter()
            .find(|(id, _)| id == b"DIAM")
            .map(|(_, data)| data)
            .expect("MAID chunk");
        let root_adts: Vec<u32> = maid
            .as_chunks::<32>()
            .0
            .iter()
            .map(|tile| u32::from_le_bytes(tile[..4].try_into().unwrap()))
            .filter(|&id| id != 0)
            .collect();
        assert!(!root_adts.is_empty());
        let mut read = Vec::new();
        for &adt_id in root_adts.iter().take(3) {
            match s.read_file_by_id(adt_id, installed).unwrap() {
                Some(adt) => {
                    println!("  ADT {adt_id}: {} bytes, magic {}", adt.len(), magic(&adt));
                    assert_eq!(&adt[..4], b"REVM");
                    read.push(adt);
                }
                None => println!("  ADT {adt_id}: not local"),
            }
        }
        assert!(
            !read.is_empty(),
            "at least one ADT of WDT {wdt_id} is local"
        );
        // Different tiles have different terrain data.
        if read.len() >= 2 {
            assert_ne!(read[0], read[1]);
        }
    }
}

#[test]
#[ignore = "needs a local WoW install (WOW_CASC_TEST_INSTALL)"]
fn locale_masks() {
    let s = storage();
    let installed = s.installed_locales_mask();
    assert!(s.has_file_id(MAP_DB2_FDID, installed));
    // With ALL, CascLib keeps the first variant in root order, which need not
    // be an installed locale; narrowing to the installed locale finds the
    // local copy.
    let all = s.read_file_by_id_zerofill_encrypted(MAP_DB2_FDID, locale::ALL);
    println!(
        "Map.db2 with ALL: {:?}",
        all.as_ref().map(|d| d.as_ref().map(Vec::len))
    );
    for (tc, name) in locale::TC_LOCALE_NAMES.iter().enumerate() {
        let mask = locale::tc_locale_mask(tc);
        let present = s.has_file_id(MAP_DB2_FDID, mask);
        let local = s
            .read_file_by_id_zerofill_encrypted(MAP_DB2_FDID, mask)
            .map(|d| d.map(|d| d.len()));
        println!("Map.db2 {name} ({mask:#07x}): has_file_id = {present}, read = {local:?}");
        if mask & installed != 0 {
            assert!(matches!(local, Ok(Some(_))));
        }
    }
}

/// Encrypted content on real data: files encrypted with a built-in `CascLib`
/// key decode (OGG / BLP payloads below), unknown keys yield `MissingKey`
/// while the zero-filling read still succeeds.
#[test]
#[ignore = "needs a local WoW install (WOW_CASC_TEST_INSTALL)"]
fn encrypted_files() {
    let s = storage();
    let installed = s.installed_locales_mask();
    if is_default_install() {
        // Encrypted with key D134F430A45C1CF2 (in CascLib's static table).
        assert!(s.has_tact_key(0xD134_F430_A45C_1CF2));
        let ogg = s.read_file_by_id(3_149_387, installed).unwrap().unwrap();
        assert_eq!(&ogg[..4], b"OggS");
        let blp = s.read_file_by_id(3_182_725, installed).unwrap().unwrap();
        assert_eq!(&blp[..4], b"BLP2");
        println!(
            "decrypted: OggS {} bytes, BLP2 {} bytes",
            ogg.len(),
            blp.len()
        );
    }

    let mut ok = 0;
    let mut absent = 0;
    let mut missing = Vec::new();
    for fdid in (1..7_000_000).step_by(211) {
        if !s.has_file_id(fdid, installed) {
            continue;
        }
        match s.read_file_by_id(fdid, installed) {
            Ok(Some(_)) => ok += 1,
            Ok(None) => absent += 1,
            Err(Error::MissingKey(key)) => {
                let zero = s
                    .read_file_by_id_zerofill_encrypted(fdid, installed)
                    .unwrap()
                    .expect("zero-filled read");
                assert!(!s.has_tact_key(key));
                missing.push((fdid, format!("{key:016X}"), zero.len()));
            }
            Err(e) => panic!("FDID {fdid}: {e}"),
        }
    }
    println!(
        "sampled FDIDs: {ok} read, {absent} not local, {} with missing keys: {:?}",
        missing.len(),
        &missing[..missing.len().min(8)]
    );
    assert!(ok > 0);
}
