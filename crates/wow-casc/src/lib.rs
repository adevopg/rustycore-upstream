//! Local CASC storage reader for WoW client installs.
//!
//! Port of the subset of TrinityCore `src/tools/extractor_common` (`CASC::Storage`,
//! `CASC::File`) and `dep/CascLib` that the client-data extractors use, at
//! TrinityCore tag `TDB343.24081` (client 3.4.3.54261, product `wow_classic`).
//!
//! Only local installs are supported (`CASC::Storage::Open`); the remote/online
//! storage (`OpenRemote`) is not ported.

use std::path::Path;

pub mod locale;

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
    /// (CascLib `ERROR_FILE_ENCRYPTED`). Extractors skip such files.
    #[error("missing TACT decryption key {0:016X}")]
    MissingKey(u64),
}

pub type Result<T> = std::result::Result<T, Error>;

/// An opened local CASC storage (TrinityCore `CASC::Storage`).
pub struct Storage {
    _private: (),
}

impl Storage {
    /// Opens the local storage at `install_path` (the directory that contains
    /// `.build.info` and `Data/`) for `product` (e.g. `wow_classic`), restricted to
    /// files whose locale flags intersect `locale_mask` (a combination of
    /// [`locale`] CASC bits; use [`locale::ALL`] for every locale).
    pub fn open(install_path: &Path, product: &str, locale_mask: u32) -> Result<Self> {
        let _ = (install_path, product, locale_mask);
        todo!("wow-casc: Storage::open")
    }

    /// Client build number from the build config (`GetBuildNumber`).
    pub fn build_number(&self) -> u32 {
        todo!("wow-casc: Storage::build_number")
    }

    /// Locale mask of the installed locales (`GetInstalledLocalesMask`), in
    /// [`locale`] CASC bits.
    pub fn installed_locales_mask(&self) -> u32 {
        todo!("wow-casc: Storage::installed_locales_mask")
    }

    /// Product this storage was opened for.
    pub fn product(&self) -> &str {
        todo!("wow-casc: Storage::product")
    }

    /// Returns whether the root manifest has an entry for `file_data_id` whose
    /// locale flags intersect `locale_mask`.
    pub fn has_file_id(&self, file_data_id: u32, locale_mask: u32) -> bool {
        let _ = (file_data_id, locale_mask);
        todo!("wow-casc: Storage::has_file_id")
    }

    /// Reads and fully decodes (BLTE, decryption) the file with `file_data_id`.
    /// Returns `Ok(None)` when the file does not exist for `locale_mask` or its
    /// data is not present locally (TrinityCore `OpenFile` returning null).
    pub fn read_file_by_id(&self, file_data_id: u32, locale_mask: u32) -> Result<Option<Vec<u8>>> {
        let _ = (file_data_id, locale_mask);
        todo!("wow-casc: Storage::read_file_by_id")
    }

    /// Same as [`Storage::read_file_by_id`] but resolves a client path such as
    /// `DBFilesClient\\Map.db2` through the root name hashes (Jenkins96 of the
    /// upper-cased path with `/` normalized to `\\`).
    pub fn read_file_by_name(&self, name: &str, locale_mask: u32) -> Result<Option<Vec<u8>>> {
        let _ = (name, locale_mask);
        todo!("wow-casc: Storage::read_file_by_name")
    }

    /// Adds TACT keys from text in the wowdev `TACTKeys` format: one
    /// `<16 hex key name> <32 hex key>` pair per line (`#` comments allowed).
    /// Returns the number of keys added.
    pub fn add_tact_keys_from_str(&mut self, text: &str) -> usize {
        let _ = text;
        todo!("wow-casc: Storage::add_tact_keys_from_str")
    }
}
