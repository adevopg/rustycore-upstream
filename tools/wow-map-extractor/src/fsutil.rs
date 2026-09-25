//! File helpers from `src/tools/map_extractor/System.cpp`: `CreateDir` and
//! `ExtractFile`.

use std::path::Path;

use crate::casc::{CASC_LOCALE_NONE, Casc, FileRead, FileRef, OpenFlags};

/// `CreateDir`: create one directory level unless it exists; failure is fatal
/// (the C++ throws an uncaught `std::runtime_error`).
pub(crate) fn create_dir(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        return Ok(());
    }
    std::fs::create_dir(path)
        .map_err(|e| anyhow::anyhow!("Unable to create directory{} ({e})", path.display()))
}

/// `CascStorage->OpenFile(fileDataId, CASC_LOCALE_NONE)` followed, when the target does
/// not exist yet, by `ExtractFile(file, filename)`.
///
/// Returns `None` when the file could not be opened (the caller prints its own
/// "Unable to open file" message with the error name), otherwise whether a file was
/// extracted.
pub(crate) fn open_and_extract(
    casc: &Casc,
    file_data_id: u32,
    file_path: &Path,
) -> Result<bool, &'static str> {
    // `OpenFile` succeeds without reading; use the root lookup so an existing output
    // file does not require decoding the CASC file.
    if !casc.has_file_id(file_data_id, CASC_LOCALE_NONE) {
        return Err("FILE_NOT_FOUND");
    }
    if file_path.exists() {
        return Ok(false);
    }
    let filename = file_path.display();
    match casc.read(
        FileRef::Id(file_data_id),
        CASC_LOCALE_NONE,
        OpenFlags::default(),
    ) {
        FileRead::Data(bytes) => {
            if std::fs::write(file_path, bytes).is_err() {
                println!("Can't create the output file '{filename}'");
                let _ = std::fs::remove_file(file_path);
                return Ok(false);
            }
            Ok(true)
        }
        FileRead::OpenFailed(error) => Err(error),
        FileRead::ReadFailed(_) => {
            println!("Can't read file '{filename}'");
            Ok(false)
        }
    }
}
