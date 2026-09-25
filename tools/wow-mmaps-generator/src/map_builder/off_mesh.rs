//! `MapBuilder::ParseOffMeshConnectionsFile` from
//! `src/tools/mmaps_generator/MapBuilder.cpp` (TDB343.24081), with the
//! `fgets` / `sscanf` semantics it relies on.

use crate::map_defines::{nav_area, nav_flag};
use crate::terrain_builder::OffMeshData;

/// `MapBuilder::ParseOffMeshConnectionsFile` — one connection per line,
/// `sscanf("%u %u,%u (%f %f %f) (%f %f %f) %f %hhu %hu")`, read with
/// `fgets(buf, 512)`.
pub fn parse_off_mesh_connections_file(path: Option<&str>) -> Vec<OffMeshData> {
    // no meshfile input given?
    let Some(path) = path else { return Vec::new() };
    let Ok(bytes) = std::fs::read(path) else {
        println!(" loadOffMeshConnections:: input file {path} not found!");
        return Vec::new();
    };
    parse_off_mesh_connections(&bytes)
}

/// `fgets(buf, 512, fp)` chunks (at most 511 bytes, newline included).
pub(super) fn fgets_chunks(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let mut end = pos;
        while end < bytes.len() && end - pos < 511 {
            end += 1;
            if bytes[end - 1] == b'\n' {
                break;
            }
        }
        out.push(&bytes[pos..end]);
        pos = end;
    }
    out
}

/// Parses the content of an off-mesh connections file.
pub fn parse_off_mesh_connections(bytes: &[u8]) -> Vec<OffMeshData> {
    let mut out = Vec::new();
    for line in fgets_chunks(bytes) {
        // fgets stops at an embedded NUL for sscanf's purposes
        let line = line.split(|&b| b == 0).next().unwrap_or(&[]);
        let mut sc = Scanner { s: line, pos: 0 };
        let mut off = OffMeshData::default();
        let scanned = sc.scan_off_mesh(&mut off);
        if scanned < 10 {
            continue;
        }
        off.bidirectional = true;
        if scanned < 12 {
            off.flags = nav_flag::GROUND;
        }
        if scanned < 11 {
            off.area_id = nav_area::GROUND;
        }
        out.push(off);
    }
    out
}

/// Tiny `sscanf` for the off-mesh format.
struct Scanner<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Scanner<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while self
            .peek()
            .is_some_and(|c| c == b' ' || (b'\t'..=b'\r').contains(&c))
        {
            self.pos += 1;
        }
    }

    fn literal(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// `%u` (strtoul: optional sign, wraps on negation, saturates).
    fn unsigned(&mut self) -> Option<u64> {
        self.skip_ws();
        let start = self.pos;
        let mut neg = false;
        if let Some(c @ (b'+' | b'-')) = self.peek() {
            neg = c == b'-';
            self.pos += 1;
        }
        let digits = self.pos;
        let mut v: u64 = 0;
        let mut overflow = false;
        while let Some(c) = self.peek().filter(u8::is_ascii_digit) {
            match v
                .checked_mul(10)
                .and_then(|x| x.checked_add(u64::from(c - b'0')))
            {
                Some(n) => v = n,
                None => overflow = true,
            }
            self.pos += 1;
        }
        if self.pos == digits {
            self.pos = start;
            return None;
        }
        if overflow {
            v = u64::MAX;
        }
        Some(if neg { v.wrapping_neg() } else { v })
    }

    /// `%f` (strtof on the longest decimal prefix).
    fn float(&mut self) -> Option<f32> {
        self.skip_ws();
        let start = self.pos;
        let mut i = self.pos;
        if matches!(self.s.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        let rest = String::from_utf8_lossy(&self.s[i..]).to_ascii_lowercase();
        for word in ["infinity", "inf", "nan"] {
            if rest.starts_with(word) {
                let end = i + word.len();
                self.pos = end;
                let txt = String::from_utf8_lossy(&self.s[start..end]).into_owned();
                return txt.parse::<f32>().ok();
            }
        }
        let mut digits = 0;
        while self.s.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
            digits += 1;
        }
        if self.s.get(i) == Some(&b'.') {
            i += 1;
            while self.s.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return None;
        }
        if matches!(self.s.get(i), Some(b'e' | b'E')) {
            let mut j = i + 1;
            if matches!(self.s.get(j), Some(b'+' | b'-')) {
                j += 1;
            }
            let es = j;
            while self.s.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
            if j > es {
                i = j;
            }
        }
        self.pos = i;
        let txt = String::from_utf8_lossy(&self.s[start..i]).into_owned();
        let txt = txt.replace(".e", ".0e").replace(".E", ".0E");
        let txt = txt.strip_suffix('.').unwrap_or(&txt);
        txt.parse::<f32>().ok()
    }

    /// Whitespace directive in the format: skips any amount of whitespace.
    fn ws(&mut self) {
        self.skip_ws();
    }

    fn scan_off_mesh(&mut self, o: &mut OffMeshData) -> i32 {
        let mut n = 0;
        macro_rules! conv {
            ($e:expr) => {
                match $e {
                    Some(v) => v,
                    None => return n,
                }
            };
        }
        macro_rules! lit {
            ($c:expr) => {
                if !self.literal($c) {
                    return n;
                }
            };
        }
        o.map_id = conv!(self.unsigned()) as u32;
        n += 1;
        self.ws();
        o.tile_x = conv!(self.unsigned()) as u32;
        n += 1;
        lit!(b',');
        o.tile_y = conv!(self.unsigned()) as u32;
        n += 1;
        self.ws();
        lit!(b'(');
        for k in 0..3 {
            o.from[k] = conv!(self.float());
            n += 1;
            if k < 2 {
                self.ws();
            }
        }
        lit!(b')');
        self.ws();
        lit!(b'(');
        for k in 0..3 {
            o.to[k] = conv!(self.float());
            n += 1;
            if k < 2 {
                self.ws();
            }
        }
        lit!(b')');
        self.ws();
        o.radius = conv!(self.float());
        n += 1;
        self.ws();
        o.area_id = conv!(self.unsigned()) as u8;
        n += 1;
        self.ws();
        o.flags = conv!(self.unsigned()) as u16;
        n += 1;
        n
    }
}
