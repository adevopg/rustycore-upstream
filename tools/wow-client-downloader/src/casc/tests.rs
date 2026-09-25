//! Writer round trips read back through `wow_casc::Storage`.

use std::collections::BTreeMap;

use super::*;
use crate::util::md5;
use crate::{blte, encoding};

const MAP_DB2: u32 = 1_349_477;

/// A TSFM v1 root with one enUS group of `(fdid, ckey)` (sorted by fdid).
fn root(files: &[(u32, Key)]) -> Vec<u8> {
    let mut out = b"TSFM".to_vec();
    for v in [24u32, 1, files.len() as u32, 0, 0] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x1000_0000u32.to_le_bytes()); // no name hashes
    out.extend_from_slice(&2u32.to_le_bytes()); // enUS
    let mut prev: Option<u32> = None;
    for (id, _) in files {
        let delta = prev.map_or(*id, |p| id - p - 1);
        out.extend_from_slice(&delta.to_le_bytes());
        prev = Some(*id);
    }
    for (_, ckey) in files {
        out.extend_from_slice(ckey);
    }
    out
}

/// Builds a complete synthetic storage with `files` (`fdid -> content`) and
/// returns the storage writer (still open) and the file `EKeys`.
fn build_storage(dir: &Path, files: &BTreeMap<u32, Vec<u8>>, limit: u64) -> LocalStorage {
    let mut storage = LocalStorage::open_with_limit(dir, limit).unwrap();
    let mut enc_entries = Vec::new();
    let mut root_files = Vec::new();
    for (id, content) in files {
        let blob = blte::encode(&[(b'Z', content)], true);
        let ekey = blte::encoded_key(&blob).unwrap();
        let ckey = md5(content);
        storage.write(&ekey, &blob).unwrap();
        enc_entries.push((ckey, ekey, content.len() as u64, blob.len() as u64));
        root_files.push((*id, ckey));
    }
    let root_data = root(&root_files);
    let root_blob = blte::encode(&[(b'Z', &root_data)], true);
    let root_ekey = blte::encoded_key(&root_blob).unwrap();
    storage.write(&root_ekey, &root_blob).unwrap();
    enc_entries.push((
        md5(&root_data),
        root_ekey,
        root_data.len() as u64,
        root_blob.len() as u64,
    ));
    enc_entries.sort_unstable();
    let enc_data = encoding::build(&enc_entries, 10);
    let enc_blob = blte::encode(&[(b'Z', &enc_data)], true);
    let enc_ekey = blte::encoded_key(&enc_blob).unwrap();
    storage.write(&enc_ekey, &enc_blob).unwrap();

    let config = format!(
        "# Build Configuration\n\nroot = {}\nencoding = {} {}\nencoding-size = {} {}\nbuild-name = WOW-54261patch3.4.3_ClassicRetail\n",
        hex(&md5(&root_data)),
        hex(&md5(&enc_data)),
        hex(&enc_ekey),
        enc_data.len(),
        enc_blob.len()
    );
    let build_key = md5(config.as_bytes());
    storage.write_config(&build_key, config.as_bytes()).unwrap();
    storage.write_indices().unwrap();
    let row = build_info::BuildInfoRow {
        branch: "eu".into(),
        build_key: hex(&build_key),
        cdn_key: hex(&[0x11; 16]),
        cdn_path: "tpr/wow".into(),
        cdn_hosts: "level3.blizzard.com".into(),
        cdn_servers: String::new(),
        tags: "Windows x86_64 EU? esES speech?:Windows x86_64 EU? esES text?".into(),
        version: "3.4.3.54261".into(),
        product: "wow_classic".into(),
    };
    fs::write(dir.join(".build.info"), build_info::render(None, &row)).unwrap();
    storage
}

#[test]
fn round_trip_through_wow_casc() {
    let dir = tempfile::tempdir().unwrap();
    let mut files = BTreeMap::new();
    files.insert(MAP_DB2, b"WDC4 synthetic map table".repeat(40));
    files.insert(17, b"tiny".to_vec());
    let storage = build_storage(dir.path(), &files, MAX_ARCHIVE_SIZE);
    drop(storage);

    let casc = wow_casc::Storage::open(dir.path(), "wow_classic", wow_casc::locale::ALL).unwrap();
    assert_eq!(casc.build_number(), 54261);
    assert_eq!(casc.installed_locales_mask(), wow_casc::locale::ESES);
    for (id, content) in &files {
        let read = casc.read_file_by_id(*id, 0).unwrap().unwrap();
        assert_eq!(&read, content, "file {id}");
    }
    assert_eq!(
        &casc.read_file_by_id(MAP_DB2, 0).unwrap().unwrap()[..4],
        b"WDC4"
    );
    assert!(casc.read_file_by_id(12345, 0).unwrap().is_none());
    assert_eq!(
        fs::read_dir(dir.path().join("Data/data"))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|x| x == "idx")
            })
            .count(),
        16
    );
}

#[test]
fn many_archives_and_resume() {
    let dir = tempfile::tempdir().unwrap();
    // 8 KiB archives force entries into data.000 .. data.NNN; the content is
    // pseudo-random so zlib cannot shrink it.
    let mut seed = 0x1234_5678u32;
    let files: BTreeMap<u32, Vec<u8>> = (1..40u32)
        .map(|i| {
            let content = (0..900)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 24) as u8
                })
                .collect();
            (i * 3, content)
        })
        .collect();
    let storage = build_storage(dir.path(), &files, 8192);
    let keys: Vec<Key> = storage.entries.keys().copied().collect();
    let archives = storage.current.as_ref().unwrap().0;
    assert!(archives > 3, "entries spread over several archives");
    drop(storage);

    let casc = wow_casc::Storage::open(dir.path(), "wow_classic", wow_casc::locale::ENUS).unwrap();
    for (id, content) in &files {
        assert_eq!(&casc.read_file_by_id(*id, 0).unwrap().unwrap(), content);
    }

    // Crash simulation: garbage after the last entry and a torn journal record.
    let newest = dir.path().join("Data/data").join(archive_name(archives));
    let len = fs::metadata(&newest).unwrap().len();
    OpenOptions::new()
        .append(true)
        .open(&newest)
        .unwrap()
        .write_all(&[0xEE; 100])
        .unwrap();
    OpenOptions::new()
        .append(true)
        .open(dir.path().join("Data/data").join(JOURNAL))
        .unwrap()
        .write_all(&[1, 2, 3])
        .unwrap();
    let mut reopened = LocalStorage::open_with_limit(dir.path(), 8192).unwrap();
    assert_eq!(fs::metadata(&newest).unwrap().len(), len, "tail truncated");
    assert_eq!(reopened.len(), keys.len());
    for k in &keys {
        assert!(reopened.contains(k));
        let blob = reopened.read(k).unwrap().unwrap();
        blte::verify(&blob, k).unwrap();
    }
    // Appending continues in the newest archive.
    let blob = blte::encode(&[(b'N', b"more")], false);
    let k = blte::encoded_key(&blob).unwrap();
    reopened.write(&k, &blob).unwrap();
    assert_eq!(reopened.location(&k).unwrap().offset as u64, len);
}

#[test]
fn refuses_foreign_storage() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("Data/data")).unwrap();
    fs::write(dir.path().join("Data/data/data.000"), b"agent data").unwrap();
    assert!(LocalStorage::open(dir.path()).is_err());
}

#[test]
fn index_only_open_never_touches_archives() {
    let dir = tempfile::tempdir().unwrap();
    let mut files = BTreeMap::new();
    files.insert(MAP_DB2, b"WDC4 table".repeat(10));
    drop(build_storage(dir.path(), &files, MAX_ARCHIVE_SIZE));
    let data = dir.path().join("Data/data/data.000");
    // A tail the full open would truncate stays in index-only mode.
    OpenOptions::new()
        .append(true)
        .open(&data)
        .unwrap()
        .write_all(&[0xEE; 64])
        .unwrap();
    let len = fs::metadata(&data).unwrap().len();
    let journal = fs::read(dir.path().join("Data/data").join(JOURNAL)).unwrap();
    for f in fs::read_dir(dir.path().join("Data/data")).unwrap() {
        let p = f.unwrap().path();
        if p.extension().is_some_and(|e| e == "idx") {
            fs::remove_file(p).unwrap();
        }
    }
    let mut storage = LocalStorage::open_index_only(dir.path()).unwrap();
    storage.write_indices().unwrap();
    assert!(
        storage.write(&[1; 16], b"BLTE\0\0\0\0N").is_err(),
        "read-only"
    );
    assert_eq!(fs::metadata(&data).unwrap().len(), len);
    assert_eq!(
        fs::read(dir.path().join("Data/data").join(JOURNAL)).unwrap(),
        journal
    );
    let casc = wow_casc::Storage::open(dir.path(), "wow_classic", wow_casc::locale::ALL).unwrap();
    assert_eq!(
        &casc.read_file_by_id(MAP_DB2, 0).unwrap().unwrap()[..4],
        b"WDC4"
    );
    // Not a storage of this tool.
    let other = tempfile::tempdir().unwrap();
    assert!(LocalStorage::open_index_only(other.path()).is_err());
}
