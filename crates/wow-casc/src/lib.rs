//! Local CASC storage reader for `WoW` client installs.
//!
//! Port of the subset of `TrinityCore` `src/tools/extractor_common` (`CASC::Storage`,
//! `CASC::File`) and `dep/CascLib` that the client-data extractors use, at
//! `TrinityCore` tag `TDB343.24081` (client 3.4.3.54261, product `wow_classic`).
//!
//! Only local installs are supported (`CASC::Storage::Open`); the remote/online
//! storage (`OpenRemote`) is not ported.
//!
//! Open sequence (`CascLib` `CascOpenStorage.cpp: LoadCascStorage`):
//! `.build.info` ([`build_info`]) -> build config ([`config`]) -> local
//! indices ([`index`]) -> ENCODING ([`encoding`]) -> `WoW` ROOT ([`root`]) ->
//! static TACT keys ([`keys`]). Files are read from `data.###` ([`data`]) and
//! decoded by [`blte`].
//!
//! # Locale masks
//!
//! `CascLib` selects one entry per `FileDataId` when the storage is opened, using
//! the open locale mask, and ignores the `dwLocaleFlags` argument of
//! `CascOpenFile` (`TrinityCore` even passes `CASC_LOCALE_NONE` for DB2 files).
//! The per-call `locale_mask` of this API narrows that selection: `0` means
//! "no extra restriction" (exact `CascLib` behaviour), any other value
//! re-runs `CascLib`'s selection as if the storage had been opened with
//! `open_mask & locale_mask`. Entries without locale flags match any mask.

use std::path::{Path, PathBuf};

pub mod locale;

mod blte;
mod build_info;
mod config;
mod data;
mod encoding;
mod index;
mod jenkins;
mod keys;
mod keys_table;
mod root;
mod salsa20;

pub use jenkins::file_name_hash;

use blte::DecodeOptions;
use data::DataArchives;
use encoding::Encoding;
use index::LocalIndex;
use keys::KeyMap;
use root::WowRoot;

/// Errors raised while opening a storage or reading a file from it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid CASC storage: {0}")]
    InvalidStorage(String),
    #[error("product `{0}` is not installed in this storage")]
    ProductNotFound(String),
    #[error("corrupt CASC data: {0}")]
    Corrupt(String),
    /// The file is encrypted with a TACT key that has not been loaded
    /// (`CascLib` `ERROR_FILE_ENCRYPTED`). Extractors skip such files.
    #[error("missing TACT decryption key {0:016X}")]
    MissingKey(u64),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Summary counters of an opened storage (diagnostics only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageStats {
    /// `EKeys` in the local `.idx` files.
    pub index_entries: usize,
    /// `CKey` pages of the ENCODING manifest.
    pub encoding_pages: usize,
    /// ROOT entries kept after the content-flag filters.
    pub root_entries: usize,
    /// Distinct name hashes registered for the open locale mask.
    pub root_name_hashes: usize,
    /// `true` for the `TSFM` (8.2.0+) root layout, `false` for the legacy one.
    pub root_has_tsfm_header: bool,
    /// Effective locale mask used when the ROOT was loaded.
    pub open_locale_mask: u32,
}

/// An opened local CASC storage (`TrinityCore` `CASC::Storage`).
pub struct Storage {
    product: String,
    build_number: u32,
    installed_locales: u32,
    /// Effective locale mask of the ROOT load (`LoadBuildManifest`).
    open_mask: u32,
    index: LocalIndex,
    encoding: Encoding,
    root: WowRoot,
    keys: KeyMap,
    archives: DataArchives,
}

/// `CascLib` `DataDirs[]` (`CascFiles.cpp`), in probing order.
const DATA_DIRS: &[&str] = &[
    "data/casc",
    "data",
    "Data",
    "SC2Data",
    "HeroesData",
    "BNTData",
];

/// `CascLib` `CheckCascBuildFileExact` + `CheckCascBuildFileDirs` for the
/// `.build.info` build file type: the path itself, or `.build.info` in the
/// directory or any parent directory.
fn find_build_info(install_path: &Path) -> Result<PathBuf> {
    let is_build_info = |p: &Path| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.len() >= 11 && n[n.len() - 11..].eq_ignore_ascii_case(".build.info"))
    };
    if install_path.is_file() && is_build_info(install_path) {
        return Ok(install_path.to_path_buf());
    }
    let mut dir = Some(install_path);
    while let Some(d) = dir {
        let candidate = d.join(".build.info");
        if candidate.is_file() {
            return Ok(candidate);
        }
        dir = d.parent();
    }
    Err(Error::InvalidStorage(format!(
        "no .build.info in {} or its parent directories",
        install_path.display()
    )))
}

/// `CascLib` `CheckArchiveFilesDirectories`: returns `(data path, index path)`.
fn find_data_dirs(root_path: &Path) -> Result<(PathBuf, PathBuf)> {
    for sub in DATA_DIRS {
        let data_path = root_path.join(sub);
        if !data_path.is_dir() || !data_path.join("config").is_dir() {
            continue;
        }
        for index_sub in ["data", "darch"] {
            let index_path = data_path.join(index_sub);
            if index_path.is_dir() {
                return Ok((data_path, index_path));
            }
        }
    }
    Err(Error::InvalidStorage(format!(
        "no data/config directories under {}",
        root_path.display()
    )))
}

fn hex_lower(key: &[u8; 16]) -> String {
    use std::fmt::Write;
    key.iter().fold(String::with_capacity(32), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// `CascLib` `IsFileDataIdName`: `File%u.ext` (decimal) or `FILE%08X.ext`.
fn file_data_id_name(name: &str) -> Option<u32> {
    if let Some(rest) = name.strip_prefix("File") {
        let digits = rest.split('.').next().unwrap_or_default();
        if digits.bytes().all(|b| b.is_ascii_digit()) {
            let mut acc: u32 = 0;
            for b in digits.bytes() {
                acc = acc.wrapping_mul(10).wrapping_add(u32::from(b - b'0'));
            }
            return Some(acc);
        }
    }
    if let Some(rest) = name.strip_prefix("FILE")
        && name.len() >= 0x0C
    {
        let (hex, tail) = (rest.get(..8)?, rest.get(8..)?);
        if (tail.is_empty() || tail.starts_with('.')) && hex.bytes().all(|b| b.is_ascii_hexdigit())
        {
            let id = u32::from_str_radix(hex, 16).ok()?;
            return (id != u32::MAX).then_some(id);
        }
    }
    None
}

impl Storage {
    /// Opens the local storage at `install_path` (the directory that contains
    /// `.build.info` and `Data/`) for `product` (e.g. `wow_classic`), restricted to
    /// files whose locale flags intersect `locale_mask` (a combination of
    /// [`locale`] CASC bits; use [`locale::ALL`] for every locale).
    ///
    /// As in `CascLib`, a `locale_mask` of 0 means the installed locales
    /// (`.build.info` tags), and if those are empty too, every locale.
    pub fn open(install_path: &Path, product: &str, locale_mask: u32) -> Result<Self> {
        // LoadMainFile -> ParseFile_BuildInfo
        let build_info_path = find_build_info(install_path)?;
        let root_path = build_info_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let (data_path, index_path) = find_data_dirs(&root_path)?;
        let text = std::fs::read(&build_info_path)?;
        let csv = build_info::Csv::parse(&String::from_utf8_lossy(&text))
            .ok_or_else(|| Error::InvalidStorage(".build.info is empty".into()))?;
        let info = build_info::select_build_info(&csv, product)?;

        // LoadCdnConfigFile: the CDN config only matters for online storages
        // (archive indices); CascLib ignores its failure for local storages,
        // so it is not loaded here.

        // LoadCdnBuildFile -> FetchAndLoadConfigFile -> ParseFile_CdnBuild
        let key_hex = hex_lower(&info.build_key);
        let config_path = data_path
            .join("config")
            .join(&key_hex[0..2])
            .join(&key_hex[2..4])
            .join(&key_hex);
        let config_data = std::fs::read(&config_path).map_err(|e| {
            Error::InvalidStorage(format!("build config {}: {e}", config_path.display()))
        })?;
        if !config::verify_md5(&config_data, &info.build_key) {
            return Err(Error::Corrupt(format!(
                "build config {key_hex} does not match its MD5"
            )));
        }
        let build_config = config::parse_build_config(&config_data)?;

        // dwBuildNumber: .build.info Version, else build-name, else
        // "21742 + InstallCKey.ContentSize".
        let build_number = if info.build_number != 0 {
            info.build_number
        } else if let Some(n) = build_config.build_name_number {
            n
        } else {
            21742u32.wrapping_add(build_config.install.content_size)
        };

        // LoadIndexFiles -> LoadLocalIndexFiles
        let index = LocalIndex::load_dir(&index_path)?;
        let archives = DataArchives::new(index_path);
        let keys = KeyMap::with_static_keys();

        // LoadEncodingManifest (LoadInternalFileToMemory with CASC_STRICT_DATA_CHECK)
        let enc_cfg = build_config.encoding;
        let enc_ekey = enc_cfg.ekey.expect("checked by parse_build_config");
        let encoding_data = read_internal(
            &index,
            &archives,
            &keys,
            &enc_ekey,
            known_size(enc_cfg.content_size),
        )?
        .ok_or_else(|| Error::InvalidStorage("ENCODING is not present locally".into()))?;
        let encoding = Encoding::parse(&encoding_data)?;
        drop(encoding_data);

        // LoadBuildManifest. The effective mask: open mask, else the
        // installed locales, else everything.
        let mut open_mask = if locale_mask != 0 {
            locale_mask
        } else {
            info.default_locale
        };
        if open_mask == 0 {
            open_mask = locale::ALL;
        }
        // Deviation: CascLib first loads the TVFS `vfs-root` when present and
        // reparses the legacy ROOT when TVFS holds WoW-style entries (always
        // the case for WoW); only the legacy WoW ROOT is loaded here.
        let root_ckey = build_config
            .root
            .ckey
            .ok_or_else(|| Error::InvalidStorage("build config lacks `root`".into()))?;
        let (root_encoded_key, root_size) = match encoding.find(&root_ckey) {
            Some(e) => (e.ekey, Some(e.content_size)),
            None => match build_config.root.ekey {
                Some(ekey) => (ekey, known_size(build_config.root.content_size)),
                None => {
                    return Err(Error::InvalidStorage("ROOT CKey is not in ENCODING".into()));
                }
            },
        };
        let root_data = read_internal(&index, &archives, &keys, &root_encoded_key, root_size)?
            .ok_or_else(|| Error::InvalidStorage("ROOT is not present locally".into()))?;
        // "Ignore ROOT files that contain just a MD5 hash"
        if root_data.len() <= 32 {
            return Err(Error::InvalidStorage("ROOT file is too small".into()));
        }
        let root = WowRoot::parse(&root_data, open_mask)?;
        drop(root_data);

        Ok(Self {
            // hs->szCodeName: the caller's product name takes precedence.
            product: product.to_owned(),
            build_number,
            installed_locales: info.default_locale,
            open_mask,
            index,
            encoding,
            root,
            keys,
            archives,
        })
    }

    /// Client build number from the build config (`GetBuildNumber`).
    ///
    /// Like `CascLib`: the largest number in the `.build.info` `Version`
    /// column, else in the build config `build-name`.
    pub fn build_number(&self) -> u32 {
        self.build_number
    }

    /// Locale mask of the installed locales (`GetInstalledLocalesMask`), in
    /// [`locale`] CASC bits.
    ///
    /// `CascLib` derives it from the locale codes in the `.build.info` `Tags`
    /// column of the selected product.
    pub fn installed_locales_mask(&self) -> u32 {
        self.installed_locales
    }

    /// Summary counters of the loaded manifests.
    pub fn stats(&self) -> StorageStats {
        StorageStats {
            index_entries: self.index.len(),
            encoding_pages: self.encoding.page_count(),
            root_entries: self.root.record_count(),
            root_name_hashes: self.root.name_count(),
            root_has_tsfm_header: self.root.format == root::RootFormat::V2,
            open_locale_mask: self.open_mask,
        }
    }

    /// Product this storage was opened for.
    pub fn product(&self) -> &str {
        &self.product
    }

    /// Effective mask for a per-call locale mask (see the crate docs).
    fn effective_mask(&self, locale_mask: u32) -> u32 {
        if locale_mask == 0 {
            self.open_mask
        } else {
            self.open_mask & locale_mask
        }
    }

    /// Selects the `CKey` for a `FileDataId` (`TFileTreeRoot::GetFile(FileDataId)`).
    fn select_ckey(&self, file_data_id: u32, locale_mask: u32) -> Option<[u8; 16]> {
        let mask = self.effective_mask(locale_mask);
        self.root
            .select(file_data_id, mask, |ckey| {
                self.encoding.find(ckey).is_some()
            })
            .map(|r| r.ckey)
    }

    /// Returns whether the root manifest has an entry for `file_data_id` whose
    /// locale flags intersect `locale_mask`.
    ///
    /// Entries without locale flags always match, and a `locale_mask` of 0
    /// uses the storage's open mask (see the crate docs). Only entries whose
    /// `CKey` is listed in ENCODING count, as in `CascLib`'s file tree; whether
    /// the data is present locally is not checked.
    pub fn has_file_id(&self, file_data_id: u32, locale_mask: u32) -> bool {
        self.select_ckey(file_data_id, locale_mask).is_some()
    }

    /// Reads and fully decodes (BLTE, decryption) the file with `file_data_id`.
    /// Returns `Ok(None)` when the file does not exist for `locale_mask` or its
    /// data is not present locally (`TrinityCore` `OpenFile` returning null).
    ///
    /// A frame encrypted with an unknown TACT key fails with
    /// [`Error::MissingKey`]; see [`Storage::read_file_by_id_zerofill_encrypted`].
    pub fn read_file_by_id(&self, file_data_id: u32, locale_mask: u32) -> Result<Option<Vec<u8>>> {
        self.read_by_id(file_data_id, locale_mask, false)
    }

    /// [`Storage::read_file_by_id`] with `TrinityCore`'s
    /// `zerofillEncryptedParts = true` (`CascLib` `CASC_OVERCOME_ENCRYPTED`):
    /// frames encrypted with an unknown key are returned as zeros instead of
    /// failing. `TrinityCore` uses this for DB2 files (`DB2CascFileSource`).
    pub fn read_file_by_id_zerofill_encrypted(
        &self,
        file_data_id: u32,
        locale_mask: u32,
    ) -> Result<Option<Vec<u8>>> {
        self.read_by_id(file_data_id, locale_mask, true)
    }

    /// Same as [`Storage::read_file_by_id`] but resolves a client path such as
    /// `DBFilesClient\\Map.db2` through the root name hashes (Jenkins96 of the
    /// upper-cased path with `/` normalized to `\\`).
    ///
    /// Like `CascLib` `CascOpenFile(CASC_OPEN_BY_NAME)`, a name that is not in
    /// the root falls back to `File<decimal id>` / `FILE<8 hex id>` names and
    /// to a 32-hex-digit `CKey`.
    pub fn read_file_by_name(&self, name: &str, locale_mask: u32) -> Result<Option<Vec<u8>>> {
        self.read_by_name(name, locale_mask, false)
    }

    /// [`Storage::read_file_by_name`] with `zerofillEncryptedParts = true`.
    pub fn read_file_by_name_zerofill_encrypted(
        &self,
        name: &str,
        locale_mask: u32,
    ) -> Result<Option<Vec<u8>>> {
        self.read_by_name(name, locale_mask, true)
    }

    /// `FileDataId` for a client path through the root name hashes, if known.
    pub fn file_data_id_by_name(&self, name: &str) -> Option<u32> {
        self.root.file_data_id_by_hash(file_name_hash(name))
    }

    /// Adds TACT keys from text in the wowdev `TACTKeys` format: one
    /// `<16 hex key name> <32 hex key>` pair per line (`#` comments allowed).
    /// Returns the number of keys added.
    pub fn add_tact_keys_from_str(&mut self, text: &str) -> usize {
        self.keys.import_from_str(text)
    }

    /// Adds one TACT key (`CascLib` `CascAddEncryptionKey`). An existing key
    /// with the same name is kept; returns whether the key was new.
    pub fn add_tact_key(&mut self, key_name: u64, key: [u8; 16]) -> bool {
        self.keys.add(key_name, key)
    }

    /// `TrinityCore` `CASC::Storage::HasTactKey` (`CascFindEncryptionKey`).
    pub fn has_tact_key(&self, key_name: u64) -> bool {
        self.keys.find(key_name).is_some()
    }

    fn read_by_id(
        &self,
        file_data_id: u32,
        locale_mask: u32,
        overcome_encrypted: bool,
    ) -> Result<Option<Vec<u8>>> {
        match self.select_ckey(file_data_id, locale_mask) {
            Some(ckey) => self.read_ckey(&ckey, overcome_encrypted),
            None => Ok(None),
        }
    }

    fn read_by_name(
        &self,
        name: &str,
        locale_mask: u32,
        overcome_encrypted: bool,
    ) -> Result<Option<Vec<u8>>> {
        if name.is_empty() {
            return Ok(None);
        }
        // First chance: the root name hash.
        if let Some(id) = self.file_data_id_by_name(name)
            && let Some(ckey) = self.select_ckey(id, locale_mask)
        {
            return self.read_ckey(&ckey, overcome_encrypted);
        }
        // Second chance: "File%u" / "FILE%08X" names.
        if let Some(id) = file_data_id_name(name)
            && let Some(ckey) = self.select_ckey(id, locale_mask)
        {
            return self.read_ckey(&ckey, overcome_encrypted);
        }
        // Third chance: a CKey string.
        if let Some(ckey) = build_info::parse_md5_hex(name)
            && self.encoding.find(&ckey).is_some()
        {
            return self.read_ckey(&ckey, overcome_encrypted);
        }
        Ok(None)
    }

    /// Opens the file by its `CKey` entry and reads it whole (`CascReadFile`
    /// with the file size as the byte count).
    fn read_ckey(&self, ckey: &[u8; 16], overcome_encrypted: bool) -> Result<Option<Vec<u8>>> {
        let Some(entry) = self.encoding.find(ckey) else {
            return Ok(None);
        };
        // CascReadFile / CascGetFileSize64 short-circuit empty files.
        if entry.content_size == 0 {
            return Ok(Some(Vec::new()));
        }
        read_entry(
            &self.index,
            &self.archives,
            &self.keys,
            &entry.ekey,
            Some(entry.content_size),
            DecodeOptions {
                verify_frames: false,
                overcome_encrypted,
            },
        )
    }
}

/// `CascLib` `CASC_INVALID_SIZE` -> `None`.
fn known_size(size: u32) -> Option<u32> {
    (size != config::INVALID_SIZE).then_some(size)
}

/// `LoadInternalFileToMemory`: open by `EKey` with `CASC_STRICT_DATA_CHECK`.
fn read_internal(
    index: &LocalIndex,
    archives: &DataArchives,
    keys: &KeyMap,
    ekey: &[u8; 16],
    content_size: Option<u32>,
) -> Result<Option<Vec<u8>>> {
    read_entry(
        index,
        archives,
        keys,
        ekey,
        content_size,
        DecodeOptions {
            verify_frames: true,
            overcome_encrypted: false,
        },
    )
}

/// `OpenDataStream` + `LoadEncodedHeaderAndSpanFrames` + `ReadFile_WholeFile`.
/// `Ok(None)` when the `EKey` is not in the local indices (`ERROR_FILE_OFFLINE`)
/// or its archive does not exist.
fn read_entry(
    index: &LocalIndex,
    archives: &DataArchives,
    keys: &KeyMap,
    ekey: &[u8; 16],
    content_size: Option<u32>,
    options: DecodeOptions,
) -> Result<Option<Vec<u8>>> {
    let Some(location) = index.find(ekey) else {
        return Ok(None);
    };
    let (archive, offset) = index.split_offset(location.storage_offset);
    let Some(encoded) = archives.read(archive, offset, location.encoded_size as usize)? else {
        return Ok(None);
    };
    blte::decode_entry(&encoded, content_size, keys, options).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_data_id_names() {
        assert_eq!(file_data_id_name("File1349477.db2"), Some(1_349_477));
        assert_eq!(file_data_id_name("File42"), Some(42));
        assert_eq!(file_data_id_name("FILE00149705.db2"), Some(0x0014_9705));
        assert_eq!(file_data_id_name("FILE0014970G.db2"), None);
        assert_eq!(file_data_id_name("Files\\x.db2"), None);
        assert_eq!(file_data_id_name("DBFilesClient\\Map.db2"), None);
    }

    #[test]
    fn storage_is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Storage>();
    }
}
