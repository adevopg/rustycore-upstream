//! Little-endian binary helpers replacing the `fread`/`fwrite` calls of the
//! C++ Collision code. Files are read fully into memory; a short read is
//! reported as `None`, which is what `fread(...) != n` detects in C++.

use crate::math::{AABox, Vector3};

/// Cursor over an in-memory file with `fread`-like semantics.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Current offset in bytes.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// True once every byte was consumed (`feof` after a failed read).
    pub fn is_at_end(&self) -> bool {
        self.pos >= self.data.len()
    }

    /// Reads exactly `n` bytes. On a short read the remaining bytes are
    /// consumed (like `fread`) and `None` is returned.
    pub fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let remaining = self.data.len() - self.pos;
        if n > remaining {
            self.pos = self.data.len();
            return None;
        }
        let out = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Some(out)
    }

    pub fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.bytes(N).map(|b| b.try_into().expect("length checked"))
    }

    pub fn u8(&mut self) -> Option<u8> {
        self.array::<1>().map(|b| b[0])
    }

    pub fn u16(&mut self) -> Option<u16> {
        self.array().map(u16::from_le_bytes)
    }

    pub fn i16(&mut self) -> Option<i16> {
        self.array().map(i16::from_le_bytes)
    }

    pub fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }

    pub fn i32(&mut self) -> Option<i32> {
        self.array().map(i32::from_le_bytes)
    }

    pub fn f32(&mut self) -> Option<f32> {
        self.array().map(f32::from_le_bytes)
    }

    pub fn vector3(&mut self) -> Option<Vector3> {
        let b = self.bytes(12)?;
        Some(vector3_from_le(b))
    }

    /// `fread(&aabox, sizeof(G3D::AABox), 1)` — `lo` then `hi`.
    pub fn aabox(&mut self) -> Option<AABox> {
        let lo = self.vector3()?;
        let hi = self.vector3()?;
        Some(AABox::new(lo, hi))
    }

    /// Reads `count` little-endian `u32` values as a single block.
    pub fn u32_vec(&mut self, count: usize) -> Option<Vec<u32>> {
        let b = self.bytes(count.checked_mul(4)?)?;
        Some(
            b.as_chunks::<4>()
                .0
                .iter()
                .map(|c| u32::from_le_bytes(*c))
                .collect(),
        )
    }

    /// Reads `count` little-endian `f32` values as a single block.
    pub fn f32_vec(&mut self, count: usize) -> Option<Vec<f32>> {
        let b = self.bytes(count.checked_mul(4)?)?;
        Some(
            b.as_chunks::<4>()
                .0
                .iter()
                .map(|c| f32::from_le_bytes(*c))
                .collect(),
        )
    }

    /// Reads `count` packed `Vector3` values as a single block.
    pub fn vector3_vec(&mut self, count: usize) -> Option<Vec<Vector3>> {
        let b = self.bytes(count.checked_mul(12)?)?;
        Some(
            b.as_chunks::<12>()
                .0
                .iter()
                .map(|c| vector3_from_le(c))
                .collect(),
        )
    }

    /// `readChunk(rf, dest, compare, len)` (VMapManager2.cpp): reads `len`
    /// bytes and compares them with `compare`.
    pub fn chunk(&mut self, compare: &[u8]) -> bool {
        match self.bytes(compare.len()) {
            Some(b) => b == compare,
            None => false,
        }
    }
}

fn vector3_from_le(b: &[u8]) -> Vector3 {
    let f = |i: usize| f32::from_le_bytes(b[i..i + 4].try_into().expect("4 bytes"));
    Vector3::new(f(0), f(4), f(8))
}

/// Little-endian append helpers for building files in memory.
pub trait Writer {
    fn put_bytes(&mut self, b: &[u8]);
    fn put_u8(&mut self, v: u8) {
        self.put_bytes(&[v]);
    }
    fn put_u16(&mut self, v: u16) {
        self.put_bytes(&v.to_le_bytes());
    }
    fn put_i16(&mut self, v: i16) {
        self.put_bytes(&v.to_le_bytes());
    }
    fn put_u32(&mut self, v: u32) {
        self.put_bytes(&v.to_le_bytes());
    }
    fn put_i32(&mut self, v: i32) {
        self.put_bytes(&v.to_le_bytes());
    }
    fn put_f32(&mut self, v: f32) {
        self.put_bytes(&v.to_le_bytes());
    }
    fn put_vector3(&mut self, v: Vector3) {
        self.put_bytes(&v.to_le_bytes());
    }
    /// `fwrite(&aabox, sizeof(G3D::AABox), 1)` — `lo` then `hi`.
    fn put_aabox(&mut self, b: &AABox) {
        self.put_vector3(b.low());
        self.put_vector3(b.high());
    }
}

impl Writer for Vec<u8> {
    fn put_bytes(&mut self, b: &[u8]) {
        self.extend_from_slice(b);
    }
}
