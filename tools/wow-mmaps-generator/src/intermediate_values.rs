//! Port of `src/tools/mmaps_generator/IntermediateValues.{h,cpp}`
//! (TDB343.24081): the `--debugOutput true` files under `meshes/`.
//!
//! `TileBuilder::buildMoveMapTile` only ever fills `polyMesh` and
//! `polyMeshDetail` (the heightfield, compact heightfield and contour set
//! members stay null), so only the `.pmesh` / `.dmesh` writers are reachable
//! and ported; `.obj`, `.map` and `.mesh` come from `generateObjFile`.

use std::io::Write;
use std::path::Path;

use crate::recast::{PolyMesh, PolyMeshDetail};
use crate::terrain_builder::{MeshData, TerrainBuilder};

/// `IntermediateValues` (only the reachable members).
#[derive(Default)]
pub struct IntermediateValues {
    pub poly_mesh: Option<PolyMesh>,
    pub poly_mesh_detail: Option<PolyMeshDetail>,
}

fn perror(message: &str, err: &std::io::Error) {
    eprintln!("{message}: {err}");
}

/// `printf("%f")`.
pub fn c_float(v: f32) -> String {
    let v = f64::from(v);
    if v.is_nan() {
        if v.is_sign_negative() {
            "-nan".into()
        } else {
            "nan".into()
        }
    } else if v.is_infinite() {
        if v < 0.0 { "-inf".into() } else { "inf".into() }
    } else {
        format!("{v:.6}")
    }
}

fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u16s(out: &mut Vec<u8>, v: &[u16]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

fn put_f32s(out: &mut Vec<u8>, v: &[f32]) {
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

impl IntermediateValues {
    /// `IntermediateValues::writeIV`.
    pub fn write_iv(&self, base: &Path, map_id: u32, tile_x: u32, tile_y: u32) {
        let tile_string = format!("[{tile_x:02},{tile_y:02}]: ");
        print!("{tile_string}Writing debug output...                       \r");

        let debug_write = |ext: &str, bytes: Vec<u8>| {
            let file_name = format!(
                "meshes/{map_id:04}{:02}{:02}.{ext}",
                tile_y as i32, tile_x as i32
            );
            match std::fs::File::create(base.join(&file_name)) {
                Ok(mut f) => {
                    let _ = f.write_all(&bytes);
                }
                Err(e) => perror(
                    &format!("{tile_string}Failed to open {file_name} for writing!\n"),
                    &e,
                ),
            }
            print!("{tile_string}Writing debug output...                       \r");
        };

        if let Some(pmesh) = &self.poly_mesh {
            debug_write("pmesh", Self::debug_write_poly_mesh(pmesh));
        }
        if let Some(dmesh) = &self.poly_mesh_detail {
            debug_write("dmesh", Self::debug_write_poly_mesh_detail(dmesh));
        }
    }

    /// `IntermediateValues::debugWrite(FILE*, rcPolyMesh const*)`.
    pub fn debug_write_poly_mesh(mesh: &PolyMesh) -> Vec<u8> {
        let m = mesh.get();
        let a = mesh.arrays();
        let mut out = Vec::new();
        put_f32s(&mut out, &[m.cs, m.ch]);
        put_i32(&mut out, m.nvp);
        put_f32s(&mut out, &m.bmin);
        put_f32s(&mut out, &m.bmax);
        put_i32(&mut out, m.nverts);
        put_u16s(&mut out, a.verts);
        put_i32(&mut out, m.npolys);
        put_u16s(&mut out, a.polys);
        put_u16s(&mut out, a.flags);
        out.extend_from_slice(a.areas);
        put_u16s(&mut out, a.regs);
        out
    }

    /// `IntermediateValues::debugWrite(FILE*, rcPolyMeshDetail const*)`.
    pub fn debug_write_poly_mesh_detail(mesh: &PolyMeshDetail) -> Vec<u8> {
        let m = mesh.get();
        let a = mesh.arrays();
        let mut out = Vec::new();
        put_i32(&mut out, m.nverts);
        put_f32s(&mut out, a.verts);
        put_i32(&mut out, m.ntris);
        out.extend_from_slice(a.tris);
        put_i32(&mut out, m.nmeshes);
        for v in a.meshes {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// `IntermediateValues::generateObjFile`.
    #[allow(clippy::unused_self)]
    pub fn generate_obj_file(
        &self,
        base: &Path,
        map_id: u32,
        tile_x: u32,
        tile_y: u32,
        mesh: &MeshData,
    ) {
        let obj_file_name = format!("meshes/map{map_id:04}{tile_y:02}{tile_x:02}.obj");
        let mut obj = match std::fs::File::create(base.join(&obj_file_name)) {
            Ok(f) => f,
            Err(e) => {
                perror(
                    &format!("Failed to open {obj_file_name} for writing!\n"),
                    &e,
                );
                return;
            }
        };

        let mut all_tris: Vec<i32> = mesh.liquid_tris.clone();
        let mut all_verts: Vec<f32> = mesh.liquid_verts.clone();
        TerrainBuilder::copy_indices_offset(
            &mesh.solid_tris,
            &mut all_tris,
            (all_verts.len() / 3) as i32,
        );
        all_verts.extend_from_slice(&mesh.solid_verts);

        let vert_count = (all_verts.len() / 3) as i32;
        let tri_count = (all_tris.len() / 3) as i32;

        let mut text = String::new();
        for v in all_verts.chunks_exact(3) {
            text.push_str(&format!(
                "v {} {} {}\n",
                c_float(v[0]),
                c_float(v[1]),
                c_float(v[2])
            ));
        }
        for t in all_tris.chunks_exact(3) {
            text.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
        }
        let _ = obj.write_all(text.as_bytes());
        drop(obj);

        let tile_string = format!("[{tile_y:02},{tile_x:02}]: ");
        print!("{tile_string}Writing debug output...                       \r");

        let map_name = format!("meshes/{map_id:04}.map");
        match std::fs::File::create(base.join(&map_name)) {
            Ok(mut f) => {
                let _ = f.write_all(&[0u8]);
            }
            Err(e) => {
                perror(&format!("Failed to open {map_name} for writing!\n"), &e);
                return;
            }
        }

        let mesh_name = format!("meshes/{map_id:04}{tile_y:02}{tile_x:02}.mesh");
        match std::fs::File::create(base.join(&mesh_name)) {
            Ok(mut f) => {
                let mut out = Vec::new();
                put_i32(&mut out, vert_count);
                put_f32s(&mut out, &all_verts);
                put_i32(&mut out, tri_count);
                for t in &all_tris {
                    out.extend_from_slice(&t.to_le_bytes());
                }
                let _ = f.write_all(&out);
            }
            Err(e) => perror(&format!("Failed to open {mesh_name} for writing!\n"), &e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::c_float;

    #[test]
    fn printf_f_format() {
        assert_eq!(c_float(1.5), "1.500000");
        assert_eq!(c_float(-0.0), "-0.000000");
        assert_eq!(c_float(-17_066.666), "-17066.666016");
        assert_eq!(c_float(f32::INFINITY), "inf");
        assert_eq!(c_float(f32::NAN), "nan");
    }
}
