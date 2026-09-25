//! Port of the extraction logic of `src/tools/vmap4_extractor/vmapexport.{h,cpp}`:
//! global state (`WmoDoodads`, `uniqueObjectIds`, `szWorkDirWmo`, `preciseVectorData`),
//! `GenerateUniqueObjectId`, `FileExists`, `ExtractSingleWmo`, the Map.db2 read of
//! `main` and `ParsMapFiles`. The command line and storage opening part of `main` is in
//! `main.rs`.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::adtfile::{AdtOutputCache, Modf};
use crate::cascfile::{CascFile, CascSource, c_str};
use crate::db2;
use crate::model;
use crate::names::{file_data_id_name, lossy, normalize_file_name, plain_name_offset};
use crate::wdtfile::WdtFile;
use crate::wmo::{self, WmoDoodadData, WmoGroup, WmoRoot};

/// `VMAP::RAW_VMAP_MAGIC` (`"VMAP04B"`, written with its NUL: 8 bytes).
pub const RAW_VMAP_MAGIC: &[u8; 8] = b"VMAP04B\0";
/// `VMAP::VMAP_MAGIC`.
pub const VMAP_MAGIC: &str = "VMAP_4.B";

// enum ModelFlags
pub const MOD_M2: u8 = 1;
pub const MOD_HAS_BOUND: u8 = 1 << 1;
pub const MOD_PARENT_SPAWN: u8 = 1 << 2;

/// `szWorkDirWmo`.
pub const SZ_WORK_DIR_WMO: &str = "./Buildings";

/// `uniqueObjectIds` / `GenerateUniqueObjectId`.
#[derive(Debug, Default)]
pub struct UniqueObjectIds {
    ids: HashMap<(u32, u16), u32>,
}

impl UniqueObjectIds {
    /// `GenerateUniqueObjectId(clientId, clientDoodadId)`: the first time a pair is seen
    /// it gets `size + 1`.
    pub fn generate(&mut self, client_id: u32, client_doodad_id: u16) -> u32 {
        let next = self.ids.len() as u32 + 1;
        *self
            .ids
            .entry((client_id, client_doodad_id))
            .or_insert(next)
    }
}

/// Path of `"%s/%s", szWorkDirWmo, name`.
pub fn work_file_path(work_dir: &Path, name: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        work_dir.join(std::ffi::OsStr::from_bytes(name))
    }
    #[cfg(not(unix))]
    {
        work_dir.join(lossy(name))
    }
}

/// `FileExists`: `fopen(file, "rb")` succeeds.
pub fn file_exists(path: &Path) -> bool {
    File::open(path).is_ok()
}

/// The `fopen("r+b")` / `fseek(8)` / `fread(&nVertices, 4)` sequence of
/// `Doodad::Extract`, `Doodad::ExtractSet` and `MapObject::Extract`.
///
/// `Err(path)` when the file cannot be opened, `Ok(None)` when the count cannot be read.
pub fn read_model_vertex_count(work_dir: &Path, name: &[u8]) -> Result<Option<i32>, PathBuf> {
    let path = work_file_path(work_dir, c_str(name));
    let Ok(mut input) = OpenOptions::new().read(true).write(true).open(&path) else {
        return Err(path);
    };
    let mut b = [0u8; 4];
    if input.seek(SeekFrom::Start(8)).is_err() || input.read_exact(&mut b).is_err() {
        return Ok(None);
    }
    Ok(Some(i32::from_le_bytes(b)))
}

/// `Buildings/dir_bin` opened with `fopen(dirname, "ab")`; the records of one
/// `ADTFile::init` / `WDTFile::init` are buffered and appended when it is dropped.
pub struct DirFile {
    file: File,
    pub data: Vec<u8>,
}

impl Drop for DirFile {
    fn drop(&mut self) {
        if !self.data.is_empty() {
            let _ = self.file.write_all(&self.data);
        }
    }
}

/// `MapEntry`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapEntry {
    pub id: u32,
    pub parent_map_id: i16,
    pub name: String,
    pub directory: String,
}

/// The extractor's global state.
pub struct VmapExport<'a> {
    /// `CascStorage`.
    pub casc: &'a dyn CascSource,
    /// `szWorkDirWmo`.
    pub work_dir: PathBuf,
    /// `preciseVectorData` (`-l`).
    pub precise_vector_data: bool,
    /// `WmoDoodads` (keyed by the normalized plain WMO name).
    pub wmo_doodads: HashMap<Vec<u8>, WmoDoodadData>,
    /// `uniqueObjectIds`.
    pub unique_ids: UniqueObjectIds,
}

impl<'a> VmapExport<'a> {
    pub fn new(casc: &'a dyn CascSource, work_dir: PathBuf, precise_vector_data: bool) -> Self {
        Self {
            casc,
            work_dir,
            precise_vector_data,
            wmo_doodads: HashMap::new(),
            unique_ids: UniqueObjectIds::default(),
        }
    }

    /// `fopen(szWorkDirWmo "/dir_bin", "ab")`, printing the C++ error on failure.
    pub fn open_dirfile(&self) -> Option<DirFile> {
        let dirname = self.work_dir.join("dir_bin");
        if let Ok(file) = OpenOptions::new().append(true).create(true).open(&dirname) {
            Some(DirFile {
                file,
                data: Vec::new(),
            })
        } else {
            println!("Can't open dirfile!'{}'", dirname.to_string_lossy());
            None
        }
    }

    /// `ExtractSingleWmo(std::string& fname)` (normalizes the plain-name part of `fname`
    /// in place, like the C++).
    pub fn extract_single_wmo(&mut self, fname: &mut [u8]) -> bool {
        // Copy files from archive
        let original_name = fname.to_vec();

        let off = plain_name_offset(fname);
        let plain_len = c_str(&fname[off..]).len();
        normalize_file_name(&mut fname[off..off + plain_len]);
        let plain_name = fname[off..off + plain_len].to_vec();
        let sz_local_file = work_file_path(&self.work_dir, &plain_name);

        if file_exists(&sz_local_file) {
            return true;
        }

        // Select root wmo files
        if let Some(rchr) = plain_name.iter().rposition(|&b| b == b'_') {
            let p = (0..4)
                .filter(|i| plain_name.get(rchr + i).is_some_and(u8::is_ascii_digit))
                .count();
            if p == 3 {
                return true;
            }
        }

        let mut file_ok = true;
        let mut froot = WmoRoot::new(lossy(&original_name));
        let casc = self.casc;
        if !froot.open(casc, &mut |path| self.extract_single_model(path)) {
            println!("Couldn't open RootWmo!!!");
            return true;
        }
        let Ok(output) = File::create(&sz_local_file) else {
            println!(
                "couldn't open {} for writing!",
                sz_local_file.to_string_lossy()
            );
            return false;
        };
        let mut out = Vec::new();
        froot.convert_to_vmap_root_wmo(&mut out);
        let doodads = std::mem::take(&mut froot.doodad_data);
        let doodads = match self.wmo_doodads.entry(plain_name.clone()) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                e.insert(doodads);
                e.into_mut()
            }
            std::collections::hash_map::Entry::Vacant(e) => e.insert(doodads),
        };
        let mut wmo_n_vertices: i32 = 0;
        let mut group_count: u32 = 0;
        for &group_file_data_id in &froot.group_file_data_ids {
            let s = lossy(&file_data_id_name(group_file_data_id));
            let mut fgroup = WmoGroup::new(s);
            if !fgroup.open(casc, &froot) {
                println!("Could not open all Group file for: {}", lossy(&plain_name));
                file_ok = false;
                break;
            }

            if fgroup.should_skip(&froot) {
                continue;
            }

            wmo_n_vertices = wmo_n_vertices
                .wrapping_add(fgroup.convert_to_vmap_group_wmo(&mut out, self.precise_vector_data));
            group_count += 1;
            for &group_reference in &fgroup.doodad_references {
                let Some(spawn) = doodads.spawns.get(group_reference as usize) else {
                    continue;
                };
                if !froot.valid_doodad_names.contains(&spawn.name_index) {
                    continue;
                }
                doodads.references.insert(group_reference);
            }
        }

        // store the correct no of vertices
        out[8..12].copy_from_slice(&wmo_n_vertices.to_le_bytes());
        // store the correct no of groups
        out[12..16].copy_from_slice(&group_count.to_le_bytes());
        let mut output = output;
        let _ = output.write_all(&out);
        drop(output);

        // Delete the extracted file in the case of an error
        if !file_ok {
            let _ = std::fs::remove_file(&sz_local_file);
        }
        true
    }

    /// `MapObject::Extract` followed by
    /// `Doodad::ExtractSet(WmoDoodads[name], ...)` (the pair every `MODF` loop does).
    #[allow(clippy::too_many_arguments)]
    pub fn extract_map_object_and_doodads(
        &mut self,
        map_obj_def: &Modf,
        wmo_inst_name: &[u8],
        is_global_wmo: bool,
        map_id: u32,
        original_map_id: u32,
        dirfile: &mut Vec<u8>,
        mut dirfile_cache: Option<&mut Vec<AdtOutputCache>>,
    ) {
        wmo::map_object_extract(
            map_obj_def,
            wmo_inst_name,
            is_global_wmo,
            map_id,
            original_map_id,
            &self.work_dir,
            &mut self.unique_ids,
            dirfile,
            dirfile_cache.as_deref_mut(),
        );
        let doodads = self
            .wmo_doodads
            .entry(c_str(wmo_inst_name).to_vec())
            .or_default();
        model::extract_set(
            doodads,
            map_obj_def,
            is_global_wmo,
            map_id,
            original_map_id,
            &self.work_dir,
            &mut self.unique_ids,
            dirfile,
            dirfile_cache,
        );
    }

    /// `ParsMapFiles`.
    pub fn pars_map_files(&mut self, map_ids: &[MapEntry], maps_that_are_parents: &HashSet<u32>) {
        let mut wdts: HashMap<u32, WdtFile> = HashMap::new();
        // ids whose WDT failed to init (`wdts.erase(itr)`: retried on the next lookup)
        for map_entry in map_ids {
            if !self.get_wdt(&mut wdts, map_ids, maps_that_are_parents, map_entry.id) {
                continue;
            }
            let parent_id =
                (map_entry.parent_map_id >= 0).then_some(map_entry.parent_map_id as u32);
            let has_parent = parent_id.is_some_and(|parent| {
                self.get_wdt(&mut wdts, map_ids, maps_that_are_parents, parent)
            });
            println!("Processing Map {}\n[", map_entry.id);
            for x in 0..64 {
                for y in 0..64 {
                    let mut success = false;
                    let wdt = wdts.get_mut(&map_entry.id).expect("WDT loaded");
                    if let Some(result) = wdt.init_adt(self, x, y, map_entry.id, map_entry.id) {
                        success = result;
                    }
                    if !success
                        && has_parent
                        && let Some(parent) = parent_id
                    {
                        let parent_wdt = wdts.get_mut(&parent).expect("parent WDT loaded");
                        parent_wdt.init_adt(self, x, y, map_entry.id, parent);
                    }
                }
                print!("#");
                let _ = std::io::stdout().flush();
            }
            println!("]");
        }
    }

    /// The `getWDT` lambda of `ParsMapFiles`: loads and inits the WDT once; returns
    /// whether it is available.
    fn get_wdt(
        &mut self,
        wdts: &mut HashMap<u32, WdtFile>,
        map_ids: &[MapEntry],
        maps_that_are_parents: &HashSet<u32>,
        map_id: u32,
    ) -> bool {
        if wdts.contains_key(&map_id) {
            return true;
        }
        let Some(map_entry) = map_ids.iter().find(|m| m.id == map_id) else {
            return false;
        };
        let file_name = format!("World\\Maps\\{0}\\{0}.wdt", map_entry.directory);
        let file = CascFile::open_name(self.casc, &file_name, true);
        let mut wdt = WdtFile::new(
            file,
            map_entry.directory.clone(),
            maps_that_are_parents.contains(&map_id),
        );
        if !wdt.init(self, map_id) {
            return false;
        }
        wdts.insert(map_id, wdt);
        true
    }
}

/// The Map.db2 part of `main`: record order, copy rows, parent partitioning.
///
/// Returns `(map_ids, maps_that_are_parents)`.
pub(crate) fn read_map_entries(db2: &db2::Db2Table) -> (Vec<MapEntry>, HashSet<u32>) {
    let reader = &db2.reader;
    let mut map_ids: Vec<MapEntry> = Vec::with_capacity(reader.record_count());
    let mut maps_that_are_parents = HashSet::new();
    let mut id_to_index: HashMap<u32, usize> = HashMap::new();

    for (id, record_idx) in db2.records() {
        let mut map = MapEntry {
            id,
            // int16(record.GetUInt16("ParentMapID")): RecordGetVarInt sign-extends
            // SignedImmediate columns
            parent_map_id: reader.get_field_i16(record_idx, db2::MAP_FIELD_PARENT_MAP_ID),
            name: db2.get_string(record_idx, db2::MAP_FIELD_MAP_NAME),
            directory: db2.get_string(record_idx, db2::MAP_FIELD_DIRECTORY),
        };

        if map.parent_map_id < 0 {
            map.parent_map_id =
                reader.get_field_i16(record_idx, db2::MAP_FIELD_COSMETIC_PARENT_MAP_ID);
        }

        if map.parent_map_id >= 0 {
            maps_that_are_parents.insert(map.parent_map_id as u32);
        }

        id_to_index.insert(map.id, map_ids.len());
        map_ids.push(map);
    }

    for &(new_row_id, source_row_id) in db2.copies() {
        if let Some(&source) = id_to_index.get(&source_row_id) {
            let src = map_ids[source].clone();
            map_ids.push(MapEntry {
                id: new_row_id,
                ..src
            });
        }
    }

    // force parent maps to be extracted first (std::stable_partition)
    let (mut parents, others): (Vec<MapEntry>, Vec<MapEntry>) = map_ids
        .into_iter()
        .partition(|m| maps_that_are_parents.contains(&m.id));
    parents.extend(others);
    (parents, maps_that_are_parents)
}
