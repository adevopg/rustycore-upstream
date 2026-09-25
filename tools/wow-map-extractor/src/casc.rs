//! CASC access for the extractor: port of `src/tools/extractor_common/CascHandles.cpp`
//! (`CASC::Storage::Open`, `LoadOnlineTactKeys`, `OpenFile` + `File::ReadFile`,
//! `HumanReadableCASCError`) on top of the `wow-casc` crate, plus the locale tables
//! from `map_extractor/System.cpp` (`WowLocaleToCascLocaleFlags`) and `Common.cpp`
//! (`localeNames`).

use std::path::Path;
use std::sync::OnceLock;

use wow_casc::locale as cl;

/// `localeNames[TOTAL_LOCALES]` (Common.cpp).
pub(crate) const LOCALE_NAMES: [&str; 12] = cl::TC_LOCALE_NAMES;
/// `TOTAL_LOCALES`.
pub(crate) const TOTAL_LOCALES: usize = 12;
/// `LOCALE_none`.
pub(crate) const LOCALE_NONE: usize = 9;

/// CascLib `CASC_LOCALE_ALL_WOW`.
pub(crate) const CASC_LOCALE_ALL_WOW: u32 = cl::ENUS
    | cl::KOKR
    | cl::FRFR
    | cl::DEDE
    | cl::ZHCN
    | cl::ESES
    | cl::ZHTW
    | cl::ENGB
    | cl::ESMX
    | cl::RURU
    | cl::PTBR
    | cl::ITIT
    | cl::PTPT;

/// `WowLocaleToCascLocaleFlags[12]` (map_extractor System.cpp — note enUS also
/// covers enGB and ptBR also covers ptPT, unlike `WowLocaleToCascLocaleBit`).
pub(crate) const WOW_LOCALE_TO_CASC_LOCALE_FLAGS: [u32; TOTAL_LOCALES] = [
    cl::ENUS | cl::ENGB,
    cl::KOKR,
    cl::FRFR,
    cl::DEDE,
    cl::ZHCN,
    cl::ZHTW,
    cl::ESES,
    cl::ESMX,
    cl::RURU,
    0,
    cl::PTBR | cl::PTPT,
    cl::ITIT,
];

/// `LoadOnlineTactKeys` source (`DownloadFile("raw.githubusercontent.com", 443, ...)`).
const TACT_KEYS_URL: &str = "https://raw.githubusercontent.com/wowdev/TACTKeys/master/WoW.txt";

/// `CASC::HumanReadableCASCError` for the error a `wow-casc` call reported.
pub(crate) fn human_readable_error(error: &wow_casc::Error) -> &'static str {
    match error {
        wow_casc::Error::Io(e) if e.kind() == std::io::ErrorKind::NotFound => "FILE_NOT_FOUND",
        wow_casc::Error::Io(_) => "CAN_NOT_COMPLETE",
        wow_casc::Error::InvalidStorage(_) => "BAD_FORMAT",
        wow_casc::Error::ProductNotFound(_) => "FILE_NOT_FOUND",
        wow_casc::Error::Corrupt(_) => "FILE_CORRUPT",
        wow_casc::Error::MissingKey(_) => "FILE_ENCRYPTED",
    }
}

/// Result of `OpenFile` followed by a full `ReadFile`.
#[derive(Debug)]
pub(crate) enum FileRead {
    /// The whole file.
    Data(Vec<u8>),
    /// `OpenFile` returned null; carries `HumanReadableCASCError(GetCascError())`.
    OpenFailed(&'static str),
    /// The file opened but `ReadFile` failed (e.g. encrypted with an unknown key).
    ReadFailed(&'static str),
}

/// What to open: `OpenFile(uint32 fileDataId, ...)` or `OpenFile(char const* fileName, ...)`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum FileRef<'a> {
    Id(u32),
    Name(&'a str),
}

/// CascLib `CASC_LOCALE_NONE`. `wow-casc` treats a per-call mask of 0 like CascLib:
/// the entry selected with the storage's open mask.
pub(crate) const CASC_LOCALE_NONE: u32 = 0;

/// `CASC::Storage::OpenFile` flags.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct OpenFlags {
    /// `printErrors`.
    pub(crate) print_errors: bool,
    /// `zerofillEncryptedParts` (`CASC_OVERCOME_ENCRYPTED`).
    pub(crate) zerofill_encrypted: bool,
}

/// `CASC::Storage` opened for one locale mask.
pub(crate) struct Casc {
    storage: wow_casc::Storage,
}

/// `LoadOnlineTactKeys`: the list is downloaded once per process (`static` in C++).
fn online_tact_keys() -> Option<&'static str> {
    static KEYS: OnceLock<Option<String>> = OnceLock::new();
    KEYS.get_or_init(|| {
        let output = std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "60", TACT_KEYS_URL])
            .output()
            .ok()?;
        if !output.status.success() {
            print!(
                "Downloading tact key list failed: {}",
                String::from_utf8_lossy(&output.stderr).trim_end()
            );
            return None;
        }
        String::from_utf8(output.stdout).ok()
    })
    .as_deref()
}

impl Casc {
    /// `CASC::Storage::Open(canonical(input)/"Data", localeMask, product)`.
    ///
    /// `install_dir` is the canonical input path (the directory holding `.build.info`
    /// and `Data/`); messages name `<install_dir>/Data` like the C++.
    pub(crate) fn open(install_dir: &Path, locale_mask: u32, product: &str) -> Option<Self> {
        let storage_dir = install_dir.join("Data");
        let mut storage = match wow_casc::Storage::open(install_dir, product, locale_mask) {
            Ok(storage) => storage,
            Err(e) => {
                println!(
                    "Error opening casc storage '{}': {}",
                    storage_dir.display(),
                    human_readable_error(&e)
                );
                return None;
            }
        };
        println!("Opened casc storage '{}'", storage_dir.display());

        match online_tact_keys() {
            Some(keys) => {
                storage.add_tact_keys_from_str(keys);
            }
            None => println!(
                "Failed to load additional online encryption keys, some files might not be extracted."
            ),
        }

        Some(Self { storage })
    }

    /// `GetBuildNumber`.
    pub(crate) fn build_number(&self) -> u32 {
        self.storage.build_number()
    }

    /// `GetInstalledLocalesMask`.
    pub(crate) fn installed_locales_mask(&self) -> u32 {
        self.storage.installed_locales_mask()
    }

    /// `HasTactKey` (`CascFindEncryptionKey`).
    pub(crate) fn has_tact_key(&self, key_name: u64) -> bool {
        self.storage.has_tact_key(key_name)
    }

    /// Whether `OpenFile(fileDataId, localeMask)` would succeed.
    pub(crate) fn has_file_id(&self, file_data_id: u32, locale_mask: u32) -> bool {
        self.storage.has_file_id(file_data_id, locale_mask)
    }

    /// `OpenFile(file, localeMask, printErrors, zerofillEncryptedParts)` + `ReadFile` of
    /// the whole file. `locale_mask` is the value the C++ passes (`CASC_LOCALE_NONE` or
    /// `CASC_LOCALE_ALL_WOW`); CascLib ignores it, and `wow-casc` only narrows the
    /// open-time selection with it, which is a no-op for both values.
    /// With `print_errors` an open failure is reported on stderr like `CASC::Storage::OpenFile`.
    pub(crate) fn read(&self, file: FileRef<'_>, locale_mask: u32, flags: OpenFlags) -> FileRead {
        let s = &self.storage;
        let result = match (file, flags.zerofill_encrypted) {
            (FileRef::Id(id), false) => s.read_file_by_id(id, locale_mask),
            (FileRef::Id(id), true) => s.read_file_by_id_zerofill_encrypted(id, locale_mask),
            (FileRef::Name(name), false) => s.read_file_by_name(name, locale_mask),
            (FileRef::Name(name), true) => {
                s.read_file_by_name_zerofill_encrypted(name, locale_mask)
            }
        };
        let outcome = match result {
            Ok(Some(data)) => return FileRead::Data(data),
            Ok(None) => FileRead::OpenFailed("FILE_NOT_FOUND"),
            Err(e @ (wow_casc::Error::MissingKey(_) | wow_casc::Error::Corrupt(_))) => {
                return FileRead::ReadFailed(human_readable_error(&e));
            }
            Err(e) => FileRead::OpenFailed(human_readable_error(&e)),
        };
        if flags.print_errors
            && let FileRead::OpenFailed(error) = outcome
        {
            match file {
                FileRef::Id(id) => {
                    eprintln!("Failed to open 'FileDataId {id}' in CASC storage: {error}");
                }
                FileRef::Name(name) => {
                    eprintln!("Failed to open '{name}' in CASC storage: {error}");
                }
            }
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_wow_mask_matches_casclib() {
        assert_eq!(CASC_LOCALE_ALL_WOW, 0x0001_F3F6);
    }

    #[test]
    fn locale_flags_follow_system_cpp() {
        assert_eq!(WOW_LOCALE_TO_CASC_LOCALE_FLAGS[0], 0x2 | 0x200);
        assert_eq!(WOW_LOCALE_TO_CASC_LOCALE_FLAGS[LOCALE_NONE], 0);
        assert_eq!(WOW_LOCALE_TO_CASC_LOCALE_FLAGS[10], 0x4000 | 0x1_0000);
        assert_eq!(LOCALE_NAMES[LOCALE_NONE], "none");
    }

    #[test]
    fn error_names() {
        assert_eq!(
            human_readable_error(&wow_casc::Error::MissingKey(1)),
            "FILE_ENCRYPTED"
        );
        let nf = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(
            human_readable_error(&wow_casc::Error::Io(nf)),
            "FILE_NOT_FOUND"
        );
    }
}
