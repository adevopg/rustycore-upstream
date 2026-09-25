//! DB2 extraction: port of `ExtractDBFilesClient` and `ExtractDB2File` from
//! `src/tools/map_extractor/System.cpp` (files of `DBFilesClientList.h` copied to
//! `dbc/<locale>/`, with known `TactId`s replaced by `DUMMY_KNOWN_TACT_ID`).

use std::path::Path;

use crate::casc::{Casc, FileRead, FileRef, LOCALE_NAMES};
use crate::db_files_client_list::DB_FILES_CLIENT_LIST;
use crate::db2::{check_headers, rewrite_known_tact_ids};
use crate::fsutil::create_dir;

/// `ExtractDB2File(fileDataId, cascFileName, locale, outputPath)`.
pub(crate) fn extract_db2_file(
    casc: &Casc,
    file_data_id: u32,
    casc_file_name: &str,
    locale: usize,
    output_path: &Path,
) -> bool {
    // DB2CascFileSource(CascStorage, fileDataId, false): CASC_LOCALE_NONE, zero-filled
    // encrypted parts.
    let bytes = match casc.read(FileRef::Id(file_data_id), casc.locale_mask(), false) {
        FileRead::Data(bytes) => bytes,
        FileRead::OpenFailed(error) => {
            println!(
                "Unable to open file {casc_file_name} in the archive for locale {}: {error}",
                LOCALE_NAMES[locale]
            );
            return false;
        }
        FileRead::ReadFailed(_) => {
            // The C++ source zero-fills undecryptable blocks; wow-casc cannot return a
            // partially decrypted file, so this is reported as a read failure.
            println!("Can't read file '{}'", output_path.display());
            return false;
        }
    };

    let headers = match check_headers(&bytes, &format!("FileDataId: {file_data_id}"), None) {
        Ok(headers) => headers,
        Err(what) => {
            println!("Can't read DB2 headers of '{casc_file_name}': {what}");
            return false;
        }
    };

    if std::fs::write(output_path, rewrite_known_tact_ids(&bytes, &headers)).is_err() {
        println!("Can't create the output file '{}'", output_path.display());
        let _ = std::fs::remove_file(output_path);
        return false;
    }
    true
}

/// `ExtractDBFilesClient(l)`.
pub(crate) fn extract_db_files_client(
    casc: &Casc,
    output_path: &Path,
    locale: usize,
) -> anyhow::Result<()> {
    println!("Extracting dbc/db2 files...");

    let locale_path = output_path.join("dbc").join(LOCALE_NAMES[locale]);

    create_dir(&output_path.join("dbc"))?;
    create_dir(&locale_path)?;

    println!(
        "locale {} output path {}",
        LOCALE_NAMES[locale],
        locale_path.display()
    );

    let mut count = 0u32;
    for &(file_data_id, name) in DB_FILES_CLIENT_LIST {
        let file_path = locale_path.join(name);
        if !file_path.exists() && extract_db2_file(casc, file_data_id, name, locale, &file_path) {
            count += 1;
        }
    }

    println!("Extracted {count} files\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::db_files_client_list::DB_FILES_CLIENT_LIST;

    #[test]
    fn client_list_matches_cpp_header() {
        assert_eq!(DB_FILES_CLIENT_LIST.len(), 788);
        assert_eq!(DB_FILES_CLIENT_LIST[0], (1_260_179, "Achievement.db2"));
        assert_eq!(DB_FILES_CLIENT_LIST[787], (1_797_864, "ZoneStory.db2"));
        assert!(DB_FILES_CLIENT_LIST.contains(&(1_349_477, "Map.db2")));
    }
}
