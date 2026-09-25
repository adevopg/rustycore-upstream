//! Minimal G3D math used by `TrinityCore`'s Collision code.
//!
//! Ports the subset of `dep/g3dlite` (G3D 9.0 as vendored by `TrinityCore`
//! `TDB343.24081`) that the VMAP formats, the BIH builder and the assembler
//! depend on: `G3D::Vector3`, `G3D::AABox`, `G3D::Matrix3`
//! (`fromEulerAnglesZYX`, `inverse`, both vector products), `G3D::Ray`,
//! `G3D::fuzzyEq`/`fuzzyNe` and `G3D::pif`.
//!
//! All arithmetic is kept in `f32` with the exact operation order of the C++
//! so that results (and therefore written files) are bit-identical.

use std::ops::{Add, Index, IndexMut, Mul, Sub};

/// `G3D::pif()` (g3dmath.h) — note the truncated literal.
#[allow(clippy::approx_constant)]
pub const PIF: f32 = 3.141_592_653_589_8_f32;

/// `fuzzyEpsilon32` (g3dmath.h).
const FUZZY_EPSILON_32: f32 = 0.000_01_f32;
/// `fuzzyEpsilon64` (g3dmath.h).
const FUZZY_EPSILON_64: f64 = 0.000_000_5;

/// `std::min<float>(a, b)`: `(b < a) ? b : a` (keeps `a` on NaN/ties).
#[inline]
pub fn std_min(a: f32, b: f32) -> f32 {
    if b < a { b } else { a }
}

/// `std::max<float>(a, b)`: `(a < b) ? b : a` (keeps `a` on NaN/ties).
#[inline]
pub fn std_max(a: f32, b: f32) -> f32 {
    if a < b { b } else { a }
}

/// `G3D::eps(float, float)` (g3dmath.h).
#[inline]
fn eps_f32(a: f32) -> f32 {
    let aa = a.abs() + 1.0;
    if aa == f32::INFINITY {
        FUZZY_EPSILON_32
    } else {
        FUZZY_EPSILON_32 * aa
    }
}

/// `G3D::fuzzyEq(float, float)` (g3dmath.h).
#[inline]
#[allow(clippy::float_cmp)]
pub fn fuzzy_eq_f32(a: f32, b: f32) -> bool {
    a == b || (a - b).abs() <= eps_f32(a)
}

/// `G3D::fuzzyEq(double, double)` (g3dmath.h).
#[inline]
#[allow(clippy::float_cmp)]
pub fn fuzzy_eq_f64(a: f64, b: f64) -> bool {
    let aa = a.abs() + 1.0;
    let eps = if aa == f64::INFINITY {
        FUZZY_EPSILON_64
    } else {
        FUZZY_EPSILON_64 * aa
    };
    a == b || (a - b).abs() <= eps
}

/// `G3D::fuzzyNe(double, double)`; C++ callers pass floats which promote.
#[inline]
pub fn fuzzy_ne(a: f32, b: f32) -> bool {
    !fuzzy_eq_f64(f64::from(a), f64::from(b))
}

/// `G3D::isFinite(float)`.
#[inline]
fn is_finite(x: f32) -> bool {
    !x.is_nan() && x < f32::INFINITY && x > -f32::INFINITY
}

/// `G3D::Vector3` — three packed little-endian `f32` on disk (12 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vector3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vector3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);

    #[inline]
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Vector with all components NaN (`G3D::fnan()` is the quiet NaN `0x7FC00000`).
    pub const NAN: Self = Self::new(f32::NAN, f32::NAN, f32::NAN);

    /// `Vector3::min` — `G3D::min(v.x, x)` per component.
    #[inline]
    #[must_use]
    pub fn min(self, v: Self) -> Self {
        Self::new(
            std_min(v.x, self.x),
            std_min(v.y, self.y),
            std_min(v.z, self.z),
        )
    }

    /// `Vector3::max` — `G3D::max(v.x, x)` per component.
    #[inline]
    #[must_use]
    pub fn max(self, v: Self) -> Self {
        Self::new(
            std_max(v.x, self.x),
            std_max(v.y, self.y),
            std_max(v.z, self.z),
        )
    }

    /// `Vector3::isNaN` — true if any component is NaN.
    #[inline]
    pub fn is_nan(self) -> bool {
        self.x.is_nan() || self.y.is_nan() || self.z.is_nan()
    }

    /// `Vector3::isFinite`.
    #[inline]
    pub fn is_finite(self) -> bool {
        is_finite(self.x) && is_finite(self.y) && is_finite(self.z)
    }

    /// `Vector3::primaryAxis` (Vector3.cpp): index of the largest |component|,
    /// ties resolved exactly like G3D (Y over X, Z over X/Y).
    pub fn primary_axis(self) -> usize {
        let nx = self.x.abs();
        let ny = self.y.abs();
        let nz = self.z.abs();
        if nx > ny {
            if nx > nz { 0 } else { 2 }
        } else if ny > nz {
            1
        } else {
            2
        }
    }

    /// `Vector3::dot`.
    #[inline]
    pub fn dot(self, v: Self) -> f32 {
        self.x * v.x + self.y * v.y + self.z * v.z
    }

    /// `Vector3::cross`.
    #[inline]
    #[must_use]
    pub fn cross(self, v: Self) -> Self {
        Self::new(
            self.y * v.z - self.z * v.y,
            self.z * v.x - self.x * v.z,
            self.x * v.y - self.y * v.x,
        )
    }

    /// `Vector3::magnitude`.
    #[inline]
    pub fn magnitude(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    /// Little-endian 12-byte representation (`fwrite(&v, sizeof(Vector3), 1)`).
    pub fn to_le_bytes(self) -> [u8; 12] {
        let mut out = [0u8; 12];
        out[0..4].copy_from_slice(&self.x.to_le_bytes());
        out[4..8].copy_from_slice(&self.y.to_le_bytes());
        out[8..12].copy_from_slice(&self.z.to_le_bytes());
        out
    }
}

impl Index<usize> for Vector3 {
    type Output = f32;
    #[inline]
    fn index(&self, i: usize) -> &f32 {
        match i {
            0 => &self.x,
            1 => &self.y,
            2 => &self.z,
            _ => panic!("Vector3 index {i} out of range"),
        }
    }
}

impl IndexMut<usize> for Vector3 {
    #[inline]
    fn index_mut(&mut self, i: usize) -> &mut f32 {
        match i {
            0 => &mut self.x,
            1 => &mut self.y,
            2 => &mut self.z,
            _ => panic!("Vector3 index {i} out of range"),
        }
    }
}

impl Add for Vector3 {
    type Output = Self;
    #[inline]
    fn add(self, v: Self) -> Self {
        Self::new(self.x + v.x, self.y + v.y, self.z + v.z)
    }
}

impl Sub for Vector3 {
    type Output = Self;
    #[inline]
    fn sub(self, v: Self) -> Self {
        Self::new(self.x - v.x, self.y - v.y, self.z - v.z)
    }
}

impl Mul<f32> for Vector3 {
    type Output = Self;
    #[inline]
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Mul<Vector3> for f32 {
    type Output = Vector3;
    #[inline]
    fn mul(self, v: Vector3) -> Vector3 {
        Vector3::new(self * v.x, self * v.y, self * v.z)
    }
}

/// `G3D::AABox` — `lo`, `hi` (24 bytes on disk). The default box is empty
/// (all components NaN), matching `AABox::AABox()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AABox {
    lo: Vector3,
    hi: Vector3,
}

impl Default for AABox {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl AABox {
    /// `AABox()` — the empty (NaN) box.
    pub const EMPTY: Self = Self {
        lo: Vector3::NAN,
        hi: Vector3::NAN,
    };

    /// `AABox(low, high)` (no reordering, as in G3D release builds).
    #[inline]
    pub const fn new(lo: Vector3, hi: Vector3) -> Self {
        Self { lo, hi }
    }

    /// `AABox(point)` — zero-volume box.
    #[inline]
    pub const fn from_point(v: Vector3) -> Self {
        Self { lo: v, hi: v }
    }

    #[inline]
    pub fn low(&self) -> Vector3 {
        self.lo
    }

    #[inline]
    pub fn high(&self) -> Vector3 {
        self.hi
    }

    /// `AABox::set`.
    #[inline]
    pub fn set(&mut self, lo: Vector3, hi: Vector3) {
        self.lo = lo;
        self.hi = hi;
    }

    /// `AABox::isEmpty` — `lo.isNaN()`.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.lo.is_nan()
    }

    /// `AABox::isFinite`.
    #[inline]
    pub fn is_finite(&self) -> bool {
        self.is_empty() || (self.lo.is_finite() && self.hi.is_finite())
    }

    /// `AABox::merge(const AABox&)`.
    pub fn merge(&mut self, a: &AABox) {
        if self.is_empty() {
            self.lo = a.lo;
            self.hi = a.hi;
        } else if !a.is_empty() {
            self.lo = self.lo.min(a.lo);
            self.hi = self.hi.max(a.hi);
        }
    }

    /// `AABox::merge(const Point3&)`.
    pub fn merge_point(&mut self, a: Vector3) {
        if self.is_empty() {
            self.lo = a;
            self.hi = a;
        } else {
            self.lo = self.lo.min(a);
            self.hi = self.hi.max(a);
        }
    }

    /// `AABox::contains(const Point3&)`.
    pub fn contains(&self, p: Vector3) -> bool {
        p.x >= self.lo.x
            && p.y >= self.lo.y
            && p.z >= self.lo.z
            && p.x <= self.hi.x
            && p.y <= self.hi.y
            && p.z <= self.hi.z
    }

    /// `AABox::corner(int)` (AABox.cpp), same corner numbering.
    pub fn corner(&self, index: usize) -> Vector3 {
        let (lo, hi) = (self.lo, self.hi);
        match index {
            0 => Vector3::new(lo.x, lo.y, hi.z),
            1 => Vector3::new(hi.x, lo.y, hi.z),
            2 => Vector3::new(hi.x, hi.y, hi.z),
            3 => Vector3::new(lo.x, hi.y, hi.z),
            4 => Vector3::new(lo.x, lo.y, lo.z),
            5 => Vector3::new(hi.x, lo.y, lo.z),
            6 => Vector3::new(hi.x, hi.y, lo.z),
            7 => Vector3::new(lo.x, hi.y, lo.z),
            _ => panic!("AABox corner index {index} out of range"),
        }
    }
}

impl Add<Vector3> for AABox {
    type Output = AABox;
    /// `AABox::operator+(const Vector3&)`.
    fn add(self, v: Vector3) -> AABox {
        AABox::new(self.lo + v, self.hi + v)
    }
}

/// `G3D::Matrix3`, row-major `elt[row][col]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix3 {
    pub elt: [[f32; 3]; 3],
}

impl Default for Matrix3 {
    /// G3D's default constructor leaves the matrix uninitialized; `TrinityCore`
    /// only default-constructs `ModelInstance::iInvRot`, we use zero.
    fn default() -> Self {
        Self::ZERO
    }
}

impl Matrix3 {
    pub const ZERO: Self = Self { elt: [[0.0; 3]; 3] };
    pub const IDENTITY: Self = Self {
        elt: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };

    /// Nine-float row-major constructor.
    #[allow(clippy::too_many_arguments, clippy::similar_names)]
    pub const fn new(
        e00: f32,
        e01: f32,
        e02: f32,
        e10: f32,
        e11: f32,
        e12: f32,
        e20: f32,
        e21: f32,
        e22: f32,
    ) -> Self {
        Self {
            elt: [[e00, e01, e02], [e10, e11, e12], [e20, e21, e22]],
        }
    }

    /// `Matrix3::fromEulerAnglesZYX(yaw, pitch, roll)` (Matrix3.cpp).
    ///
    /// Uses `f32::cos`/`sin` (C `cosf`/`sinf`), which is what the C++ float
    /// overloads resolve to.
    pub fn from_euler_angles_zyx(y_angle: f32, p_angle: f32, r_angle: f32) -> Self {
        let (c, s) = (y_angle.cos(), y_angle.sin());
        let z_mat = Self::new(c, -s, 0.0, s, c, 0.0, 0.0, 0.0, 1.0);
        let (c, s) = (p_angle.cos(), p_angle.sin());
        let y_mat = Self::new(c, 0.0, s, 0.0, 1.0, 0.0, -s, 0.0, c);
        let (c, s) = (r_angle.cos(), r_angle.sin());
        let x_mat = Self::new(1.0, 0.0, 0.0, 0.0, c, -s, 0.0, s, c);
        z_mat * (y_mat * x_mat)
    }

    /// `Matrix3::inverse(float fTolerance = 1e-06f)`: cofactor inverse,
    /// returns the zero matrix when the determinant is within tolerance.
    #[must_use]
    pub fn inverse(&self) -> Self {
        let e = &self.elt;
        let mut inv = [[0.0f32; 3]; 3];
        inv[0][0] = e[1][1] * e[2][2] - e[1][2] * e[2][1];
        inv[0][1] = e[0][2] * e[2][1] - e[0][1] * e[2][2];
        inv[0][2] = e[0][1] * e[1][2] - e[0][2] * e[1][1];
        inv[1][0] = e[1][2] * e[2][0] - e[1][0] * e[2][2];
        inv[1][1] = e[0][0] * e[2][2] - e[0][2] * e[2][0];
        inv[1][2] = e[0][2] * e[1][0] - e[0][0] * e[1][2];
        inv[2][0] = e[1][0] * e[2][1] - e[1][1] * e[2][0];
        inv[2][1] = e[0][1] * e[2][0] - e[0][0] * e[2][1];
        inv[2][2] = e[0][0] * e[1][1] - e[0][1] * e[1][0];

        let det = e[0][0] * inv[0][0] + e[0][1] * inv[1][0] + e[0][2] * inv[2][0];
        if det.abs() <= 1e-06_f32 {
            // The bool overload returns false after having stored the
            // (unscaled) cofactors into kInverse; the value overload returns
            // that matrix as-is.
            return Self { elt: inv };
        }
        let inv_det = 1.0f32 / det;
        for row in &mut inv {
            for v in row.iter_mut() {
                *v *= inv_det;
            }
        }
        Self { elt: inv }
    }
}

impl Mul for Matrix3 {
    type Output = Matrix3;
    /// `Matrix3::operator*(const Matrix3&)`.
    fn mul(self, m: Matrix3) -> Matrix3 {
        let mut out = Matrix3::ZERO;
        for r in 0..3 {
            for c in 0..3 {
                out.elt[r][c] = self.elt[r][0] * m.elt[0][c]
                    + self.elt[r][1] * m.elt[1][c]
                    + self.elt[r][2] * m.elt[2][c];
            }
        }
        out
    }
}

impl Mul<Vector3> for Matrix3 {
    type Output = Vector3;
    /// `Matrix3::operator*(const Vector3&)` — matrix × column vector.
    fn mul(self, v: Vector3) -> Vector3 {
        let mut out = Vector3::ZERO;
        for r in 0..3 {
            out[r] = self.elt[r][0] * v[0] + self.elt[r][1] * v[1] + self.elt[r][2] * v[2];
        }
        out
    }
}

impl Mul<Matrix3> for Vector3 {
    type Output = Vector3;
    /// `operator*(const Vector3&, const Matrix3&)` — row vector × matrix
    /// (used by `TerrainBuilder::transform`).
    fn mul(self, m: Matrix3) -> Vector3 {
        let mut out = Vector3::ZERO;
        for r in 0..3 {
            out[r] = self[0] * m.elt[0][r] + self[1] * m.elt[1][r] + self[2] * m.elt[2][r];
        }
        out
    }
}

/// `G3D::Ray` — only the members the BIH traversal needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    origin: Vector3,
    direction: Vector3,
    inv_direction: Vector3,
}

impl Ray {
    /// `Ray::fromOriginAndDirection` / `Ray::set`: `invDirection = one / direction`.
    pub fn from_origin_and_direction(origin: Vector3, direction: Vector3) -> Self {
        Self {
            origin,
            direction,
            inv_direction: Vector3::new(1.0 / direction.x, 1.0 / direction.y, 1.0 / direction.z),
        }
    }

    #[inline]
    pub fn origin(&self) -> Vector3 {
        self.origin
    }

    #[inline]
    pub fn direction(&self) -> Vector3 {
        self.direction
    }

    #[inline]
    pub fn inv_direction(&self) -> Vector3 {
        self.inv_direction
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_box_is_nan_and_merges() {
        let mut b = AABox::default();
        assert!(b.is_empty());
        assert_eq!(b.low().x.to_bits(), 0x7FC0_0000);
        b.merge_point(Vector3::new(1.0, 2.0, 3.0));
        b.merge_point(Vector3::new(-1.0, 5.0, 0.0));
        assert_eq!(b.low(), Vector3::new(-1.0, 2.0, 0.0));
        assert_eq!(b.high(), Vector3::new(1.0, 5.0, 3.0));
        assert!(b.contains(Vector3::new(0.0, 3.0, 1.0)));
    }

    #[test]
    fn primary_axis_ties_follow_g3d() {
        assert_eq!(Vector3::new(1.0, 1.0, 1.0).primary_axis(), 2);
        assert_eq!(Vector3::new(2.0, 2.0, 1.0).primary_axis(), 1);
        assert_eq!(Vector3::new(3.0, 2.0, 3.0).primary_axis(), 2);
        assert_eq!(Vector3::new(3.0, 2.0, 1.0).primary_axis(), 0);
    }

    #[test]
    fn euler_zero_is_identity_and_inverse_roundtrips() {
        assert_eq!(
            Matrix3::from_euler_angles_zyx(0.0, 0.0, 0.0),
            Matrix3::IDENTITY
        );
        let m = Matrix3::from_euler_angles_zyx(0.3, -1.1, 2.0);
        let v = Vector3::new(1.0, 2.0, 3.0);
        let back = m.inverse() * (m * v);
        assert!((back - v).magnitude() < 1e-5);
        // rotation: v * M == M^T * v == M^-1 * v
        let a = v * m;
        let b = m.inverse() * v;
        assert!((a - b).magnitude() < 1e-5);
    }

    #[test]
    fn yaw_90_rotates_x_to_y() {
        let m = Matrix3::from_euler_angles_zyx(PIF * 90.0 / 180.0, 0.0, 0.0);
        let r = m * Vector3::new(1.0, 0.0, 0.0);
        assert!((r - Vector3::new(0.0, 1.0, 0.0)).magnitude() < 1e-6);
    }

    #[test]
    fn fuzzy_compare() {
        assert!(fuzzy_eq_f32(1.0, 1.000_005));
        assert!(!fuzzy_eq_f32(1.0, 1.0001));
        assert!(!fuzzy_ne(0.0, 0.0));
        assert!(fuzzy_ne(0.001, 0.0));
    }
}
