//! Port of `TileAssembler` and `ModelPosition` from
//! `src/tools/vmap4_assembler/TileAssembler.{h,cpp}`.
//!
//! Console messages follow the C++ `printf`/`std::cout` output.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use wow_vmap::definitions::{DIR_BIN, TEMP_GAMEOBJECT_MODELS, VMO_EXTENSION};
use wow_vmap::gameobject_models::MAX_MODEL_NAME_LEN;
use wow_vmap::io::{Reader, Writer};
use wow_vmap::math::PIF;
use wow_vmap::{
    AABox, Bih, GAMEOBJECT_MODELS, GroupModel, MOD_HAS_BOUND, MOD_M2, MOD_PARENT_SPAWN, Matrix3,
    ModelSpawn, RAW_VMAP_MAGIC, VMAP_MAGIC, Vector3, VmapError, WorldModel, WorldModelRaw,
    pack_tile_id, unpack_tile_id,
};

/// `ModelPosition` (TileAssembler.h).
#[derive(Debug, Clone, Copy)]
pub struct ModelPosition {
    rotation: Matrix3,
    pub dir: Vector3,
    pub scale: f32,
}

impl ModelPosition {
    pub fn new(dir: Vector3, scale: f32) -> Self {
        let mut p = Self {
            rotation: Matrix3::ZERO,
            dir,
            scale,
        };
        p.init();
        p
    }

    /// `ModelPosition::init`.
    pub fn init(&mut self) {
        self.rotation = Matrix3::from_euler_angles_zyx(
            PIF * self.dir.y / 180.0,
            PIF * self.dir.x / 180.0,
            PIF * self.dir.z / 180.0,
        );
    }

    /// `ModelPosition::transform`.
    pub fn transform(&self, p_in: Vector3) -> Vector3 {
        let out = p_in * self.scale;
        self.rotation * out
    }
}

/// `MapSpawns`. `TileSpawn` sets (ordered and deduplicated by id only) are
/// `BTreeMap<id, flags>` where the first inserted flags win (`emplace`).
#[derive(Debug, Default)]
pub struct MapSpawns {
    pub map_id: u32,
    pub unique_entries: BTreeMap<u32, ModelSpawn>,
    pub tile_entries: BTreeMap<u32, BTreeMap<u32, u8>>,
    pub parent_tile_entries: BTreeMap<u32, BTreeMap<u32, u8>>,
}

/// x86 `cvttss2si` semantics of the C++ `int16(float)` conversion:
/// truncate to `int32` (NaN/out of range -> `INT_MIN`), then keep 16 bits.
fn float_to_int16(f: f32) -> i16 {
    let i = if f.is_nan() || f >= 2_147_483_648.0 || f < -2_147_483_648.0 {
        i32::MIN
    } else {
        f as i32
    };
    i as i16
}

/// `TileAssembler`.
pub struct TileAssembler {
    dest_dir: PathBuf,
    src_dir: String,
    map_data: VecDeque<MapSpawns>,
    spawned_model_files: BTreeSet<String>,
}

impl TileAssembler {
    /// `TileAssembler::TileAssembler` — creates the destination directory.
    pub fn new(src_dir: &str, dest_dir: &str) -> std::io::Result<Self> {
        std::fs::create_dir_all(dest_dir)?;
        Ok(Self {
            dest_dir: PathBuf::from(dest_dir),
            src_dir: src_dir.to_owned(),
            map_data: VecDeque::new(),
            spawned_model_files: BTreeSet::new(),
        })
    }

    /// `iSrcDir + "/" + name`.
    fn src_path(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("{}/{}", self.src_dir, name))
    }

    /// `TileAssembler::convertWorld2`.
    pub fn convert_world2(&mut self) -> bool {
        let mut success = self.read_map_spawns();
        if !success {
            return false;
        }

        let inv_tile_size: f32 = 1.0f32 / 533.333_33_f32;

        // export Map data
        while let Some(mut data) = self.map_data.pop_front() {
            // build global map tree
            let mut map_spawns: Vec<u32> = Vec::with_capacity(data.unique_entries.len());
            println!("Calculating model bounds for map {}...", data.map_id);
            let ids: Vec<u32> = data.unique_entries.keys().copied().collect();
            for id in ids {
                let spawn = data.unique_entries.get_mut(&id).expect("key from map");
                // M2 models don't have a bound set in WDT/ADT placement data,
                // they're not used for LoS but are needed for pathfinding
                if spawn.flags & MOD_M2 != 0 && !self.calculate_transformed_bound(spawn) {
                    continue;
                }

                map_spawns.push(id);
                self.spawned_model_files.insert(spawn.name.clone());

                let tile_entries = if spawn.flags & MOD_PARENT_SPAWN != 0 {
                    &mut data.parent_tile_entries
                } else {
                    &mut data.tile_entries
                };

                let bounds = spawn.bound;
                let low_x = float_to_int16(bounds.low().x * inv_tile_size);
                let low_y = float_to_int16(bounds.low().y * inv_tile_size);
                let high_x = float_to_int16(bounds.high().x * inv_tile_size);
                let high_y = float_to_int16(bounds.high().y * inv_tile_size);
                for x in i32::from(low_x)..=i32::from(high_x) {
                    for y in i32::from(low_y)..=i32::from(high_y) {
                        tile_entries
                            .entry(pack_tile_id(x as u32, y as u32))
                            .or_default()
                            .entry(spawn.id)
                            .or_insert(spawn.flags);
                    }
                }
            }

            println!("Creating map tree for map {}...", data.map_id);
            let mut tree = Bih::default();
            let bounds: Vec<AABox> = map_spawns
                .iter()
                .map(|id| data.unique_entries[id].bound)
                .collect();
            if let Err(e) = tree.build(&bounds, 3) {
                print!("Exception {e} when calling pTree.build");
                let _ = std::io::stdout().flush();
                return false;
            }

            // write map tree file
            let map_file_name = self.dest_dir.join(format!("{:04}.vmtree", data.map_id));
            let mut buf = Vec::new();
            buf.put_bytes(VMAP_MAGIC);
            buf.put_bytes(b"NODE");
            tree.write_to(&mut buf);
            buf.put_bytes(b"SIDX");
            buf.put_u32(map_spawns.len() as u32);
            for id in &map_spawns {
                buf.put_u32(*id);
            }
            if std::fs::write(&map_file_name, &buf).is_err() {
                println!("Cannot open {}", map_file_name.display());
                success = false;
                break;
            }

            // write map tile files, similar to ADT files, only with extra BIH tree node info
            for (&tile_id, spawns) in &data.tile_entries {
                let (x, y) = unpack_tile_id(tile_id);
                let tile_file_name = self
                    .dest_dir
                    .join(format!("{:04}_{:02}_{:02}.vmtile", data.map_id, y, x));
                let parent_spawns = data.parent_tile_entries.entry(tile_id).or_default();
                let n_spawns = (spawns.len() + parent_spawns.len()) as u32;
                let mut buf = Vec::new();
                buf.put_bytes(VMAP_MAGIC);
                buf.put_u32(n_spawns);
                for id in spawns.keys().chain(parent_spawns.keys()) {
                    data.unique_entries[id].write_to(&mut buf);
                }
                // a tile file that cannot be created is skipped silently
                let _ = std::fs::write(&tile_file_name, &buf);
            }
        }

        // add an object models, listed in temp_gameobject_models file
        self.export_gameobject_models();
        // export objects
        println!("\nConverting Model Files");
        let files: Vec<String> = self.spawned_model_files.iter().cloned().collect();
        for spawned_model_file in files {
            println!("Converting {spawned_model_file}");
            if let Err(e) = self.convert_raw_file(&spawned_model_file) {
                println!("{e}");
                println!("error converting {spawned_model_file}");
                success = false;
                break;
            }
        }

        success
    }

    /// `TileAssembler::readMapSpawns`.
    pub fn read_map_spawns(&mut self) -> bool {
        let Ok(dir_bin) = std::fs::read(self.src_path(DIR_BIN)) else {
            println!("Could not read dir_bin file!");
            return false;
        };
        println!("Read coordinate mapping...");
        let mut data: BTreeMap<u32, MapSpawns> = BTreeMap::new();
        let mut r = Reader::new(&dir_bin);
        // read mapID, Flags, NameSet, UniqueId, Pos, Rot, Scale, Bound_lo, Bound_hi, name
        // (a short read of the map id is the end of file)
        while let Some(map_id) = r.u32() {
            let spawn = match ModelSpawn::read_from(&mut r) {
                Ok(Some(spawn)) => spawn,
                Ok(None) => break,
                Err(VmapError::Format { what }) if what.contains("too long") => {
                    println!("Error reading ModelSpawn, file name too long!");
                    break;
                }
                Err(_) => {
                    println!("Error reading ModelSpawn!");
                    break;
                }
            };
            let entry = data.entry(map_id).or_insert_with(|| {
                println!("spawning Map {map_id}");
                MapSpawns {
                    map_id,
                    ..MapSpawns::default()
                }
            });
            entry.unique_entries.entry(spawn.id).or_insert(spawn);
        }
        self.map_data = data.into_values().collect();
        true
    }

    /// `TileAssembler::calculateTransformedBound`.
    pub fn calculate_transformed_bound(&self, spawn: &mut ModelSpawn) -> bool {
        let model_filename = self.src_path(&spawn.name);
        let model_position = ModelPosition::new(spawn.rot, spawn.scale);

        let Some(raw_model) = read_raw(&model_filename) else {
            return false;
        };

        if raw_model.groups.len() != 1 {
            println!(
                "Warning: '{}' does not seem to be a M2 model!",
                model_filename.display()
            );
        }
        // C++ indexes groupsArray[0] unconditionally (UB when empty)
        let Some(group) = raw_model.groups.first() else {
            return false;
        };

        let mut rotated_bounds = AABox::EMPTY;
        for i in 0..8 {
            rotated_bounds.merge_point(model_position.transform(group.bounds.corner(i)));
        }

        spawn.bound = rotated_bounds + spawn.pos;
        spawn.flags |= MOD_HAS_BOUND;
        true
    }

    /// `TileAssembler::convertRawFile`.
    pub fn convert_raw_file(&self, model_filename: &str) -> Result<(), VmapError> {
        let filename = if self.src_dir.is_empty() {
            PathBuf::from(model_filename)
        } else {
            self.src_path(model_filename)
        };
        let raw_model = read_raw(&filename).ok_or(VmapError::Format {
            what: "raw model file",
        })?;

        // write WorldModel
        let mut model = WorldModel::new();
        model.set_root_wmo_id(raw_model.root_wmo_id);
        if !raw_model.groups.is_empty() {
            let mut groups = Vec::with_capacity(raw_model.groups.len());
            for raw_group in raw_model.groups {
                let mut g = GroupModel::new(
                    raw_group.mogp_flags,
                    raw_group.group_wmo_id,
                    raw_group.bounds,
                );
                g.set_mesh_data(raw_group.vertices, raw_group.triangles)?;
                g.set_liquid_data(raw_group.liquid);
                groups.push(g);
            }
            model.set_group_models(groups)?;
        }

        model.write_file(
            self.dest_dir
                .join(format!("{model_filename}{VMO_EXTENSION}")),
        )
    }

    /// `TileAssembler::exportGameobjectModels`.
    pub fn export_gameobject_models(&mut self) {
        let Ok(list) = std::fs::read(self.src_path(TEMP_GAMEOBJECT_MODELS)) else {
            return;
        };
        let mut r = Reader::new(&list);
        if !r.chunk(RAW_VMAP_MAGIC) {
            return;
        }

        let Ok(mut model_list_copy) = std::fs::File::create(self.dest_dir.join(GAMEOBJECT_MODELS))
        else {
            return;
        };

        let mut out = Vec::new();
        out.put_bytes(VMAP_MAGIC);

        // EOF flag is only set after failed reading attempt
        while let Some(display_id) = r.u32() {
            let is_wmo = r.u8();
            let name_length = r.u32();
            let name = match (is_wmo, name_length) {
                (Some(_), Some(len)) if len < MAX_MODEL_NAME_LEN => r.bytes(len as usize),
                _ => None,
            };
            let (Some(is_wmo), Some(name)) = (is_wmo, name) else {
                println!("\nFile 'temp_gameobject_models' seems to be corrupted");
                break;
            };
            let model_name = String::from_utf8_lossy(name).into_owned();

            let Some(raw_model) = read_raw(&self.src_path(&model_name)) else {
                continue;
            };

            self.spawned_model_files.insert(model_name.clone());
            let mut bounds = AABox::EMPTY;
            for group in &raw_model.groups {
                for v in &group.vertices {
                    bounds.merge_point(*v);
                }
            }

            if bounds.is_empty() {
                println!("\nModel {model_name} has empty bounding box");
                continue;
            }

            if !bounds.is_finite() {
                println!("\nModel {model_name} has invalid bounding box");
                continue;
            }

            // same layout as `GameObjectModelEntry::write_to`, but with the
            // name bytes exactly as read
            out.put_u32(display_id);
            out.put_u8(is_wmo);
            out.put_u32(name.len() as u32);
            out.put_bytes(name);
            out.put_vector3(bounds.low());
            out.put_vector3(bounds.high());
        }

        let _ = model_list_copy.write_all(&out);
    }
}

/// `WorldModel_Raw::Read` with the C++ console diagnostics.
fn read_raw(path: &Path) -> Option<WorldModelRaw> {
    match WorldModelRaw::read_file(path) {
        Ok(m) => Some(m),
        Err(VmapError::Io { .. }) => {
            println!("ERROR: Can't open raw model file: {}", path.display());
            None
        }
        Err(e) => {
            println!("WorldModel_Raw::Read readfail: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests;
