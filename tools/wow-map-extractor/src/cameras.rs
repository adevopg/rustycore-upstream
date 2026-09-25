//! Cinematic camera extraction: port of `ReadCinematicCameraDBC` and
//! `ExtractCameraFiles` from `src/tools/map_extractor/System.cpp` (M2 files named
//! `cameras/FILE{FileDataID:08X}.xxx`).

use std::collections::BTreeSet;
use std::path::Path;

use crate::casc::Casc;
use crate::fsutil::{create_dir, open_and_extract};
use crate::tables::{CINEMATIC_CAMERA_FILE_DATA_ID, CINEMATIC_CAMERA_META, try_load_db2};

/// `ReadCinematicCameraDBC`: the `std::set<uint32> CameraFileDataIds` (sorted, unique).
pub(crate) fn read_cinematic_camera_dbc(casc: &Casc) -> anyhow::Result<BTreeSet<u32>> {
    println!("Read CinematicCamera.db2 file...");
    let db2 = try_load_db2(casc, &CINEMATIC_CAMERA_META)?;

    // get camera file list from DB2
    let ids: BTreeSet<u32> = db2
        .records()
        .map(|(_, idx)| db2.reader.get_field_u32(idx, CINEMATIC_CAMERA_FILE_DATA_ID))
        .collect();

    println!("Done! ({} CinematicCameras loaded)", ids.len());
    Ok(ids)
}

/// `FILE{:08X}.xxx`.
pub(crate) fn camera_file_name(file_data_id: u32) -> String {
    format!("FILE{file_data_id:08X}.xxx")
}

/// `ExtractCameraFiles`.
pub(crate) fn extract_camera_files(casc: &Casc, output_path: &Path) -> anyhow::Result<()> {
    println!("Extracting camera files...");

    let camera_file_data_ids = read_cinematic_camera_dbc(casc)?;

    let output_path = output_path.join("cameras");
    create_dir(&output_path)?;

    println!("output path {}", output_path.display());

    // extract M2s
    let mut count = 0u32;
    for camera_file_data_id in camera_file_data_ids {
        let file_path = output_path.join(camera_file_name(camera_file_data_id));
        match open_and_extract(casc, camera_file_data_id, &file_path) {
            Ok(true) => count += 1,
            Ok(false) => {}
            Err(error) => {
                println!("Unable to open file {camera_file_data_id} in the archive: {error}");
            }
        }
    }

    println!("Extracted {count} camera files");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn camera_file_name_is_upper_hex() {
        assert_eq!(super::camera_file_name(0x13_BF_1A), "FILE0013BF1A.xxx");
    }
}
