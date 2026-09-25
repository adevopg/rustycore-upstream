//! Port of `src/tools/vmap4_extractor/model.{h,cpp}`: M2 collision data (`Model::open`,
//! `Model::ConvertToVMAPModel`, `fixCoordSystem`) and the doodad spawn writers
//! (`Doodad::Extract`, `Doodad::ExtractSet`).

use std::path::Path;

use crate::adtfile::{AdtOutputCache, Mddf, Modf};
use crate::cascfile::{CascFile, CascSource, c_str};
use crate::modelheaders::{MODEL_HEADER_SIZE, ModelHeader};
use crate::names::{get_plain_name, normalize_file_name};
use crate::vec3d::{AaBox3D, Matrix3, Vec3D, to_degrees, to_radians, u16_vec};
use crate::vmapexport::{
    MOD_M2, MOD_PARENT_SPAWN, RAW_VMAP_MAGIC, UniqueObjectIds, read_model_vertex_count,
};
use crate::wmo::{GLOBAL_WMO_OFFSET, WmoDoodadData, fix_coords};

/// `fixCoordSystem`: `(x, z, -y)`.
pub fn fix_coord_system(v: Vec3D) -> Vec3D {
    Vec3D::new(v.x, v.z, -v.y)
}

/// Port of `class Model`.
pub struct Model {
    filename: String,
    pub header: ModelHeader,
    pub vertices: Vec<Vec3D>,
    pub indices: Vec<u32>,
    pub bounds: AaBox3D,
}

impl Model {
    pub fn new(filename: String) -> Self {
        Self {
            filename,
            header: ModelHeader::default(),
            vertices: Vec::new(),
            indices: Vec::new(),
            bounds: AaBox3D::default(),
        }
    }

    /// `Model::open`.
    pub fn open(&mut self, casc: &dyn CascSource) -> bool {
        let f = CascFile::open_name(casc, &self.filename, true);
        self.open_file(f)
    }

    /// Body of `Model::open` once the file is loaded.
    pub fn open_file(&mut self, mut f: CascFile) -> bool {
        if f.is_eof() {
            f.close();
            return false;
        }

        self.vertices.clear();
        self.indices.clear();

        let size = f.get_size();
        let mut m2start = 0usize;
        while m2start + 4 < size && &f.get_buffer()[m2start..m2start + 4] != b"MD20" {
            m2start += 1;
            if m2start + MODEL_HEADER_SIZE > size {
                return false;
            }
        }

        self.header = ModelHeader::from_bytes(f.get_buffer().get(m2start..).unwrap_or(&[]));
        self.bounds = self.header.collision_box;
        if self.header.n_bounding_triangles > 0 {
            f.seek(m2start);
            f.seek_relative(self.header.ofs_bounding_vertices as i32);
            let n_vertices = self.header.n_bounding_vertices as usize;
            let raw = f.read_vec(n_vertices * 12);
            self.vertices = raw
                .as_chunks::<12>()
                .0
                .iter()
                .map(|c| fix_coord_system(Vec3D::from_le(c)))
                .collect();

            f.seek(m2start);
            f.seek_relative(self.header.ofs_bounding_triangles as i32);
            let n_triangles = self.header.n_bounding_triangles as usize;
            let raw = f.read_vec(n_triangles * 2);
            self.indices = u16_vec(&raw).into_iter().map(u32::from).collect();
            f.close();
        } else {
            f.close();
            return false;
        }
        true
    }

    /// `Model::ConvertToVMAPModel`: writes the raw model file.
    pub fn convert_to_vmap_model(&mut self, outfilename: &Path) -> bool {
        let data = self.convert_to_vmap_model_bytes();
        if std::fs::write(outfilename, data).is_err() {
            println!(
                "Can't create the output file '{}'",
                outfilename.to_string_lossy()
            );
            return false;
        }
        true
    }

    /// The exact byte sequence `Model::ConvertToVMAPModel` `fwrite`s.
    pub fn convert_to_vmap_model_bytes(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(RAW_VMAP_MAGIC);
        let n_vertices = self.header.n_bounding_vertices;
        out.extend_from_slice(&n_vertices.to_le_bytes());
        let nofgroups: u32 = 1;
        out.extend_from_slice(&nofgroups.to_le_bytes());
        out.extend_from_slice(&[0u8; 12]); // rootwmoid, flags, groupid
        self.bounds.write_le(&mut out); // bbox, only needed for WMO currently
        out.extend_from_slice(&[0u8; 4]); // liquidflags
        out.extend_from_slice(b"GRP ");
        let branches: u32 = 1;
        let wsize: i32 = 4 + 4 * branches as i32;
        out.extend_from_slice(&wsize.to_le_bytes());
        out.extend_from_slice(&branches.to_le_bytes());
        let n_indexes = self.header.n_bounding_triangles;
        out.extend_from_slice(&n_indexes.to_le_bytes());
        out.extend_from_slice(b"INDX");
        let wsize = 4u32.wrapping_add(2u32.wrapping_mul(n_indexes));
        out.extend_from_slice(&wsize.to_le_bytes());
        out.extend_from_slice(&n_indexes.to_le_bytes());
        if n_indexes > 0 {
            let n = n_indexes as usize;
            for i in 0..n {
                if i % 3 == 1 && i + 1 < n {
                    self.indices.swap(i, i + 1);
                }
            }
            for index in &self.indices {
                out.extend_from_slice(&index.to_le_bytes());
            }
        }

        out.extend_from_slice(b"VERT");
        let wsize = 4u32.wrapping_add(12u32.wrapping_mul(n_vertices));
        out.extend_from_slice(&wsize.to_le_bytes());
        out.extend_from_slice(&n_vertices.to_le_bytes());
        if n_vertices > 0 {
            for v in &mut self.vertices {
                let tmp = v.y;
                v.y = -v.z;
                v.z = tmp;
            }
            for v in &self.vertices {
                v.write_le(&mut out);
            }
        }
        out
    }
}

/// Appends the cached copy of a spawn record (everything after `mapID` and `flags`).
fn push_cache(cache: Option<&mut Vec<AdtOutputCache>>, flags: u8, data: &[u8]) {
    if let Some(cache) = cache {
        cache.push(AdtOutputCache {
            flags: flags & !MOD_PARENT_SPAWN,
            data: data.to_vec(),
        });
    }
}

/// `Doodad::Extract`.
#[allow(clippy::too_many_arguments)]
pub fn extract(
    doodad_def: &Mddf,
    model_inst_name: &[u8],
    map_id: u32,
    original_map_id: u32,
    work_dir: &Path,
    unique_ids: &mut UniqueObjectIds,
    dirfile: &mut Vec<u8>,
    dirfile_cache: Option<&mut Vec<AdtOutputCache>>,
) {
    let Ok(Some(n_vertices)) = read_model_vertex_count(work_dir, model_inst_name) else {
        return;
    };
    if n_vertices == 0 {
        return;
    }

    // scale factor - divide by 1024. blizzard devs must be on crack, why not just use a float?
    let sc = f32::from(doodad_def.scale) / 1024.0;
    let position = fix_coords(doodad_def.position);

    let name_set: u8 = 0; // not used for models
    let unique_id = unique_ids.generate(doodad_def.unique_id, 0);
    let mut tcflags = MOD_M2;
    if map_id != original_map_id {
        tcflags |= MOD_PARENT_SPAWN;
    }

    let name = c_str(model_inst_name);
    let mut data = Vec::with_capacity(46 + name.len());
    data.push(name_set);
    data.extend_from_slice(&unique_id.to_le_bytes());
    position.write_le(&mut data);
    doodad_def.rotation.write_le(&mut data);
    data.extend_from_slice(&sc.to_le_bytes());
    data.extend_from_slice(&(name.len() as u32).to_le_bytes());
    data.extend_from_slice(name);

    // write mapID, Flags, NameSet, UniqueId, Pos, Rot, Scale, name
    dirfile.extend_from_slice(&map_id.to_le_bytes());
    dirfile.push(tcflags);
    dirfile.extend_from_slice(&data);

    push_cache(dirfile_cache, tcflags, &data);
}

/// `Doodad::ExtractSet`.
#[allow(clippy::too_many_arguments)]
pub fn extract_set(
    doodad_data: &WmoDoodadData,
    wmo: &Modf,
    is_global_wmo: bool,
    map_id: u32,
    original_map_id: u32,
    work_dir: &Path,
    unique_ids: &mut UniqueObjectIds,
    dirfile: &mut Vec<u8>,
    mut dirfile_cache: Option<&mut Vec<AdtOutputCache>>,
) {
    if doodad_data.sets.is_empty() {
        return;
    }

    let mut wmo_position = Vec3D::new(wmo.position.z, wmo.position.x, wmo.position.y);
    let wmo_rotation = Matrix3::from_euler_angles_zyx(
        to_radians(wmo.rotation.y),
        to_radians(wmo.rotation.x),
        to_radians(wmo.rotation.z),
    );

    if is_global_wmo {
        wmo_position.add_assign(Vec3D::new(GLOBAL_WMO_OFFSET, GLOBAL_WMO_OFFSET, 0.0));
    }

    let mut doodad_id: u16 = 0;
    let mut extract_single_set = |start_index: u32, count: u32| {
        for doodad_index in doodad_data.references.iter() {
            let idx = u32::from(doodad_index);
            if idx < start_index || idx >= start_index.wrapping_add(count) {
                continue;
            }

            let doodad = &doodad_data.spawns[doodad_index as usize];

            let mut model_inst_name: Vec<u8> = if let Some(paths) = &doodad_data.paths {
                let start = (doodad.name_index as usize).min(paths.len());
                get_plain_name(c_str(&paths[start..])).to_vec()
            } else if let Some(ids) = &doodad_data.file_data_ids {
                let Some(&id) = ids.get(doodad.name_index as usize) else {
                    continue;
                };
                format!("FILE{id:08X}.xxx").into_bytes()
            } else {
                panic!("ASSERT(false): WMO doodad data has neither MODN nor MODI");
            };

            let nlen = model_inst_name.len();
            normalize_file_name(&mut model_inst_name);
            if nlen > 3 {
                let extension = &model_inst_name[nlen - 4..];
                if extension == b".mdx" || extension == b".mdl" {
                    // `ModelInstName[nlen - 1] = '\0'` while `nlen` keeps its old value:
                    // the file lookup sees "*.m2" but `nlen` bytes (with the NUL) are
                    // written to dir_bin.
                    model_inst_name[nlen - 2] = b'2';
                    model_inst_name[nlen - 1] = 0;
                }
            }

            let Ok(Some(n_vertices)) = read_model_vertex_count(work_dir, &model_inst_name) else {
                continue;
            };
            if n_vertices == 0 {
                continue;
            }

            assert!(doodad_id < u16::MAX, "ASSERT(doodadId < max uint16)");
            doodad_id += 1;

            let position = {
                let rotated = wmo_rotation.mul_vec(doodad.position);
                Vec3D::new(
                    wmo_position.x + rotated.x,
                    wmo_position.y + rotated.y,
                    wmo_position.z + rotated.z,
                )
            };

            let q = doodad.rotation;
            let (rz, rx, ry) = Matrix3::from_quat(q[0], q[1], q[2], q[3])
                .mul(&wmo_rotation)
                .to_euler_angles_xyz();
            let rotation = Vec3D::new(to_degrees(rx), to_degrees(ry), to_degrees(rz));

            let name_set: u8 = 0; // not used for models
            let unique_id = unique_ids.generate(wmo.unique_id, doodad_id);
            let mut tcflags = MOD_M2;
            if map_id != original_map_id {
                tcflags |= MOD_PARENT_SPAWN;
            }

            let mut data = Vec::with_capacity(46 + nlen);
            data.push(name_set);
            data.extend_from_slice(&unique_id.to_le_bytes());
            position.write_le(&mut data);
            rotation.write_le(&mut data);
            data.extend_from_slice(&doodad.scale.to_le_bytes());
            data.extend_from_slice(&(nlen as u32).to_le_bytes());
            data.extend_from_slice(&model_inst_name[..nlen]);

            // write mapID, Flags, NameSet, UniqueId, Pos, Rot, Scale, name
            dirfile.extend_from_slice(&map_id.to_le_bytes());
            dirfile.push(tcflags);
            dirfile.extend_from_slice(&data);

            push_cache(dirfile_cache.as_deref_mut(), tcflags, &data);
        }
    };

    // first doodad set is always active
    let first = &doodad_data.sets[0];
    extract_single_set(first.start_index, first.count);

    if wmo.doodad_set != 0 && (wmo.doodad_set as usize) < doodad_data.sets.len() {
        let set = &doodad_data.sets[wmo.doodad_set as usize];
        extract_single_set(set.start_index, set.count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cascfile::CascFile;

    /// Builds an M2 file: `prefix` bytes, then an MD20 header with the bounding data
    /// placed after it.
    pub fn build_m2(
        prefix: &[u8],
        verts: &[[f32; 3]],
        tris: &[u16],
        collision: [f32; 6],
    ) -> Vec<u8> {
        let mut hdr = vec![0u8; MODEL_HEADER_SIZE];
        hdr[0..4].copy_from_slice(b"MD20");
        for (i, v) in collision.iter().enumerate() {
            hdr[0xBC + i * 4..0xBC + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let ofs_verts = MODEL_HEADER_SIZE as u32;
        let ofs_tris = ofs_verts + verts.len() as u32 * 12;
        hdr[0xD8..0xDC].copy_from_slice(&(tris.len() as u32).to_le_bytes());
        hdr[0xDC..0xE0].copy_from_slice(&ofs_tris.to_le_bytes());
        hdr[0xE0..0xE4].copy_from_slice(&(verts.len() as u32).to_le_bytes());
        hdr[0xE4..0xE8].copy_from_slice(&ofs_verts.to_le_bytes());
        let mut out = prefix.to_vec();
        out.extend_from_slice(&hdr);
        for v in verts {
            for c in v {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        for t in tris {
            out.extend_from_slice(&t.to_le_bytes());
        }
        out
    }

    fn le_f32s(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }

    #[test]
    fn model_open_finds_md20_inside_md21_and_converts() {
        let verts = [
            [1.0, 2.0, 3.0],
            [4.0, 5.0, 6.0],
            [7.0, 8.0, 9.0],
            [-1.0, -2.0, -3.0],
        ];
        let tris = [0u16, 1, 2, 1, 2, 3];
        // MD21 chunk header before the MD20 data: offsets are relative to MD20.
        let mut prefix = b"MD21".to_vec();
        prefix.extend_from_slice(&0u32.to_le_bytes());
        let file = build_m2(&prefix, &verts, &tris, [-1.0, -2.0, -3.0, 4.0, 5.0, 6.0]);

        let mut m = Model::new("x".into());
        assert!(m.open_file(CascFile::from_bytes(file)));
        // fixCoordSystem(x, z, -y)
        assert_eq!(m.vertices[0], Vec3D::new(1.0, 3.0, -2.0));
        assert_eq!(m.indices, vec![0, 1, 2, 1, 2, 3]);

        let out = m.convert_to_vmap_model_bytes();
        let mut exp = Vec::new();
        exp.extend_from_slice(b"VMAP04B\0");
        exp.extend_from_slice(&4u32.to_le_bytes()); // nVertices
        exp.extend_from_slice(&1u32.to_le_bytes()); // nofgroups
        exp.extend_from_slice(&[0u8; 12]);
        exp.extend_from_slice(&le_f32s(&[-1.0, -2.0, -3.0, 4.0, 5.0, 6.0])); // collisionBox as-is
        exp.extend_from_slice(&[0u8; 4]);
        exp.extend_from_slice(b"GRP ");
        exp.extend_from_slice(&8i32.to_le_bytes());
        exp.extend_from_slice(&1u32.to_le_bytes());
        exp.extend_from_slice(&6u32.to_le_bytes());
        exp.extend_from_slice(b"INDX");
        exp.extend_from_slice(&(4 + 2 * 6u32).to_le_bytes());
        exp.extend_from_slice(&6u32.to_le_bytes());
        // (i % 3) == 1 swaps with i + 1: 0 2 1 1 3 2
        for i in [0u32, 2, 1, 1, 3, 2] {
            exp.extend_from_slice(&i.to_le_bytes());
        }
        exp.extend_from_slice(b"VERT");
        exp.extend_from_slice(&(4 + 12 * 4u32).to_le_bytes());
        exp.extend_from_slice(&4u32.to_le_bytes());
        // fixCoordSystem then (x, -z, y): back to the original (x, y, z)
        for v in verts {
            exp.extend_from_slice(&le_f32s(&v));
        }
        assert_eq!(out, exp);
    }

    #[test]
    fn model_without_bounding_triangles_is_rejected() {
        let file = build_m2(&[], &[[1.0, 2.0, 3.0]], &[], [0.0; 6]);
        let mut m = Model::new("x".into());
        assert!(!m.open_file(CascFile::from_bytes(file)));
    }

    #[test]
    fn model_scan_fails_when_md20_missing() {
        let mut m = Model::new("x".into());
        assert!(!m.open_file(CascFile::from_bytes(vec![0u8; 400])));
    }
}
