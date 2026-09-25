//! Port of `src/tools/vmap4_extractor/gameobject_extract.cpp`: `ExtractSingleModel`,
//! `GetHeaderMagic` and `ExtractGameobjectModels` (writes
//! `Buildings/temp_gameobject_models` from GameObjectDisplayInfo.db2).

use std::fs::File;
use std::io::Write;

use crate::cascfile::c_str;
use crate::db2;
use crate::model::Model;
use crate::names::{file_data_id_name, lossy, normalize_file_name, plain_name_offset};
use crate::vmapexport::{RAW_VMAP_MAGIC, VmapExport, file_exists, work_file_path};

impl VmapExport<'_> {
    /// `ExtractSingleModel(std::string& fname)`: `.mdx`/`.mdl` become `.m2` and the
    /// plain-name part is normalized in `fname` itself, like the C++.
    pub fn extract_single_model(&mut self, fname: &mut Vec<u8>) -> bool {
        if fname.len() < 4 {
            return false;
        }

        let extension = &fname[fname.len() - 4..];
        if matches!(extension, b".mdx" | b".MDX" | b".mdl" | b".MDL") {
            fname.truncate(fname.len() - 2);
            fname.push(b'2');
        }

        let original_name = fname.clone();

        let off = plain_name_offset(fname);
        let plain_len = c_str(&fname[off..]).len();
        normalize_file_name(&mut fname[off..off + plain_len]);
        let name = &fname[off..off + plain_len];

        let output = work_file_path(&self.work_dir, name);

        if file_exists(&output) {
            return true;
        }

        let mut mdl = Model::new(lossy(&original_name));
        if !mdl.open(self.casc) {
            return false;
        }

        mdl.convert_to_vmap_model(&output)
    }

    /// `GetHeaderMagic`: the first 4 bytes of the file, `None` when it cannot be opened
    /// or is shorter than 4 bytes.
    fn get_header_magic(&self, file_name: &str) -> Option<[u8; 4]> {
        let data = self.casc.open_by_name(file_name).ok()??;
        data.get(..4).map(|b| [b[0], b[1], b[2], b[3]])
    }

    /// `ExtractGameobjectModels`.
    pub fn extract_gameobject_models(&mut self) {
        println!("Extracting GameObject models...");

        let db2 = match db2::load(self.casc, &db2::GAMEOBJECT_DISPLAY_INFO_META) {
            Ok(db2) => db2,
            Err(e) => {
                println!(
                    "Fatal error: Invalid GameObjectDisplayInfo.db2 file format!\n{}",
                    e.what
                );
                std::process::exit(1);
            }
        };

        let records: Vec<(u32, u32)> = db2
            .records()
            .map(|(id, rec)| {
                (
                    id,
                    db2.reader
                        .get_field_u32(rec, db2::GAMEOBJECT_DISPLAY_INFO_FIELD_FILE_DATA_ID),
                )
            })
            .collect();
        self.write_gameobject_models(&records);

        println!("Done!");
    }

    /// The `temp_gameobject_models` part of `ExtractGameobjectModels`, over
    /// `(record id, FileDataID)` of every GameObjectDisplayInfo record in file order.
    pub fn write_gameobject_models(&mut self, records: &[(u32, u32)]) {
        let model_list_path = self.work_dir.join("temp_gameobject_models");
        let Ok(mut model_list) = File::create(&model_list_path) else {
            println!(
                "Fatal error: Could not open file {}",
                model_list_path.to_string_lossy()
            );
            return;
        };

        let mut out = Vec::new();
        out.extend_from_slice(RAW_VMAP_MAGIC);

        for &(display_id, file_id) in records {
            if file_id == 0 {
                continue;
            }

            let mut file_name = file_data_id_name(file_id);
            let Some(header) = self.get_header_magic(&lossy(&file_name)) else {
                continue;
            };

            let mut is_wmo: u8 = 0;
            let result = if &header == b"REVM" {
                is_wmo = 1;
                self.extract_single_wmo(&mut file_name)
            } else if &header == b"MD20" || &header == b"MD21" {
                self.extract_single_model(&mut file_name)
            } else {
                let h = u32::from_le_bytes(header);
                eprintln!(
                    "ABORT_MSG: {} header: {} - {}{}{}{}",
                    lossy(&file_name),
                    h as i32,
                    char::from((h >> 24) as u8),
                    char::from((h >> 16) as u8),
                    char::from((h >> 8) as u8),
                    char::from(h as u8)
                );
                std::process::abort();
            };

            if result {
                out.extend_from_slice(&display_id.to_le_bytes());
                out.push(is_wmo);
                out.extend_from_slice(&(file_name.len() as u32).to_le_bytes());
                out.extend_from_slice(&file_name);
            }
        }

        let _ = model_list.write_all(&out);
        drop(model_list);
    }
}
