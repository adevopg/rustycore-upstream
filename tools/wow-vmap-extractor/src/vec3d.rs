//! Port of `src/tools/vmap4_extractor/vec3d.h` (`Vec3D`, `AaBox3D`) and of the few G3D
//! (`dep/g3dlite`) operations `Doodad::ExtractSet` uses: `Matrix3::fromEulerAnglesZYX`,
//! `Matrix3(Quat)` (via `Quat::toRotationMatrix`), `Matrix3::operator*`,
//! `Matrix3::toEulerAnglesXYZ`, `toRadians(float)`, `toDegrees(float)`.
//!
//! All arithmetic is done in the same precision and operation order as the C++ so the
//! results are bit-identical (G3D's `cos`/`sin` on `float` resolve to `cosf`/`sinf`,
//! `aTan2`/`aSin` promote to `double`).

/// `Vec3D` (packed 3 floats).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec3D {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3D {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Reads 12 bytes (`x`, `y`, `z` little-endian floats).
    pub fn from_le(b: &[u8]) -> Self {
        Self::new(f32_at(b, 0), f32_at(b, 4), f32_at(b, 8))
    }

    /// `fwrite(&v, sizeof(Vec3D), 1, f)`.
    pub fn write_le(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        out.extend_from_slice(&self.z.to_le_bytes());
    }

    /// `Vec3D::operator+=`.
    pub fn add_assign(&mut self, v: Vec3D) {
        self.x += v.x;
        self.y += v.y;
        self.z += v.z;
    }
}

/// `AaBox3D`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AaBox3D {
    pub min: Vec3D,
    pub max: Vec3D,
}

impl AaBox3D {
    pub fn from_le(b: &[u8]) -> Self {
        Self {
            min: Vec3D::from_le(&b[0..12]),
            max: Vec3D::from_le(&b[12..24]),
        }
    }

    pub fn write_le(&self, out: &mut Vec<u8>) {
        self.min.write_le(out);
        self.max.write_le(out);
    }

    /// `AaBox3D::operator+=(Vec3D const& offset)`.
    pub fn add_assign(&mut self, offset: Vec3D) {
        self.min.add_assign(offset);
        self.max.add_assign(offset);
    }
}

pub fn f32_at(b: &[u8], off: usize) -> f32 {
    f32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

pub fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

pub fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

/// Little-endian `uint16` array (`size / 2` elements, like `new uint16[size / 2]`).
pub fn u16_vec(raw: &[u8]) -> Vec<u16> {
    raw.as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect()
}

/// Little-endian `uint32` array (`size / 4` elements).
pub fn u32_vec(raw: &[u8]) -> Vec<u32> {
    raw.as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect()
}

/// Little-endian `float` array (`size / 4` elements).
pub fn f32_vec(raw: &[u8]) -> Vec<f32> {
    raw.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect()
}

/// `G3D::pi()`.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
const G3D_PI: f64 = 3.141_592_653_589_8;
/// `G3D::halfPi()`.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
const G3D_HALF_PI: f64 = 1.570_796_33;

/// `G3D::toRadians(float)`: `deg * (float)pi() / 180.0f`.
pub fn to_radians(deg: f32) -> f32 {
    deg * (G3D_PI as f32) / 180.0
}

/// `G3D::toDegrees(float)`: `rad * 180.0f / (float)pi()`.
pub fn to_degrees(rad: f32) -> f32 {
    rad * 180.0 / (G3D_PI as f32)
}

/// `G3D::Matrix3` (row-major `elt[3][3]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix3 {
    pub elt: [[f32; 3]; 3],
}

impl Matrix3 {
    /// `Matrix3::fromEulerAnglesZYX(fYAngle, fPAngle, fRAngle)`.
    pub fn from_euler_angles_zyx(y_angle: f32, p_angle: f32, r_angle: f32) -> Self {
        let (c, s) = (y_angle.cos(), y_angle.sin());
        let z_mat = Self {
            elt: [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
        };
        let (c, s) = (p_angle.cos(), p_angle.sin());
        let y_mat = Self {
            elt: [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
        };
        let (c, s) = (r_angle.cos(), r_angle.sin());
        let x_mat = Self {
            elt: [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
        };
        z_mat.mul(&y_mat.mul(&x_mat))
    }

    /// `Matrix3::Matrix3(const Quat&)` (unitizes the quaternion first).
    pub fn from_quat(qx: f32, qy: f32, qz: f32, qw: f32) -> Self {
        // Quat::unitize: *this *= rsq(dot(*this)); rsq(x) = 1.0f / sqrtf(x)
        let dot = (qx * qx) + (qy * qy) + (qz * qz) + (qw * qw);
        let inv = 1.0f32 / dot.sqrt();
        let (x, y, z, w) = (qx * inv, qy * inv, qz * inv, qw * inv);
        let xx = 2.0f32 * x * x;
        let xy = 2.0f32 * x * y;
        let xz = 2.0f32 * x * z;
        let xw = 2.0f32 * x * w;
        let yy = 2.0f32 * y * y;
        let yz = 2.0f32 * y * z;
        let yw = 2.0f32 * y * w;
        let zz = 2.0f32 * z * z;
        let zw = 2.0f32 * z * w;
        Self {
            elt: [
                [1.0 - yy - zz, xy - zw, xz + yw],
                [xy + zw, 1.0 - xx - zz, yz - xw],
                [xz - yw, yz + xw, 1.0 - xx - yy],
            ],
        }
    }

    /// `Matrix3::operator*(const Matrix3&)`.
    pub fn mul(&self, rhs: &Matrix3) -> Matrix3 {
        let mut prod = [[0.0f32; 3]; 3];
        for (r, row) in prod.iter_mut().enumerate() {
            for (c, cell) in row.iter_mut().enumerate() {
                *cell = self.elt[r][0] * rhs.elt[0][c]
                    + self.elt[r][1] * rhs.elt[1][c]
                    + self.elt[r][2] * rhs.elt[2][c];
            }
        }
        Matrix3 { elt: prod }
    }

    /// `Matrix3::operator*(const Vector3&)`.
    pub fn mul_vec(&self, v: Vec3D) -> Vec3D {
        let e = &self.elt;
        Vec3D::new(
            e[0][0] * v.x + e[0][1] * v.y + e[0][2] * v.z,
            e[1][0] * v.x + e[1][1] * v.y + e[1][2] * v.z,
            e[2][0] * v.x + e[2][1] * v.y + e[2][2] * v.z,
        )
    }

    /// `Matrix3::toEulerAnglesXYZ(rfXAngle, rfYAngle, rfZAngle)`, returned as `(x, y, z)`.
    pub fn to_euler_angles_xyz(self) -> (f32, f32, f32) {
        let e = &self.elt;
        if e[0][2] < 1.0 {
            if e[0][2] > -1.0 {
                let x = f64::from(-e[1][2]).atan2(f64::from(e[2][2])) as f32;
                // G3D::aSin(double): the argument is strictly inside (-1, 1) here.
                let y = f64::from(e[0][2]).asin() as f32;
                let z = f64::from(-e[0][1]).atan2(f64::from(e[0][0])) as f32;
                (x, y, z)
            } else {
                let x = -(f64::from(e[1][0]).atan2(f64::from(e[1][1])) as f32);
                (x, -(G3D_HALF_PI as f32), 0.0)
            }
        } else {
            let x = f64::from(e[1][0]).atan2(f64::from(e[1][1])) as f32;
            (x, G3D_HALF_PI as f32, 0.0)
        }
    }
}

#[cfg(test)]
#[allow(clippy::approx_constant)]
mod tests {
    use super::*;

    /// `Doodad::ExtractSet` math; expected bits from G3D (`dep/g3dlite` Matrix3.cpp /
    /// Quat.cpp) compiled with g++ 13.3 -O2 on x86_64.
    fn extract_set_math(
        wp: [f32; 3],
        wr: [f32; 3],
        dp: [f32; 3],
        q: [f32; 4],
        global: bool,
    ) -> [u32; 6] {
        let mut wmo_position = Vec3D::new(wp[2], wp[0], wp[1]);
        let wmo_rotation =
            Matrix3::from_euler_angles_zyx(to_radians(wr[1]), to_radians(wr[0]), to_radians(wr[2]));
        if global {
            wmo_position.add_assign(Vec3D::new(
                crate::wmo::GLOBAL_WMO_OFFSET,
                crate::wmo::GLOBAL_WMO_OFFSET,
                0.0,
            ));
        }
        let r = wmo_rotation.mul_vec(Vec3D::new(dp[0], dp[1], dp[2]));
        let pos = Vec3D::new(
            wmo_position.x + r.x,
            wmo_position.y + r.y,
            wmo_position.z + r.z,
        );
        let (rz, rx, ry) = Matrix3::from_quat(q[0], q[1], q[2], q[3])
            .mul(&wmo_rotation)
            .to_euler_angles_xyz();
        [
            pos.x.to_bits(),
            pos.y.to_bits(),
            pos.z.to_bits(),
            to_degrees(rx).to_bits(),
            to_degrees(ry).to_bits(),
            to_degrees(rz).to_bits(),
        ]
    }

    #[test]
    fn extract_set_math_matches_g3d() {
        assert_eq!(
            extract_set_math(
                [100.5, 20.25, -3000.75],
                [10.0, 45.0, 190.0],
                [1.5, -2.25, 3.0],
                [0.1, 0.2, 0.3, 0.9],
                false
            ),
            [
                0xC53B_9F49,
                0x42CE_5473,
                0x418B_B7D5,
                0xC215_6CA0,
                0xC2AA_FAF2,
                0x432D_C8BD
            ]
        );
        assert_eq!(
            extract_set_math(
                [-1234.0, 55.5, 17066.66],
                [0.0, 270.0, 0.0],
                [0.0; 3],
                [0.0, 0.0, 0.0, 1.0],
                true
            ),
            [
                0x4705_5554,
                0x4677_62AA,
                0x425E_0000,
                0x0000_0000,
                0xC2B4_0000,
                0x8000_0000
            ]
        );
        assert_eq!(
            extract_set_math(
                [5.0, 6.0, 7.0],
                [-33.3, 90.0, 12.0],
                [100.0, -200.0, 5.0],
                [0.5, -0.5, 0.5, 0.5],
                true
            ),
            [
                0x4686_ECAC,
                0x4686_2EC8,
                0x41F1_E19E,
                0x4201_ECEC,
                0xC32C_391B,
                0x4297_767C
            ]
        );
        assert_eq!(
            extract_set_math(
                [5.0, 6.0, 7.0],
                [0.0; 3],
                [1.0, 2.0, 3.0],
                [0.0, 0.707_106_8, 0.0, 0.707_106_8],
                false
            ),
            [
                0x4100_0000,
                0x40E0_0000,
                0x4110_0000,
                0x42B4_0000,
                0x0000_0000,
                0x0000_0000
            ]
        );
    }
}
