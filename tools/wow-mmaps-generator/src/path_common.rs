//! Port of `src/tools/mmaps_generator/PathCommon.h` (TDB343.24081):
//! `matchWildcardFilter`, `getDirContents`, `MapEntry` plus the C library
//! number parsing (`atoi`/`atof`) the C++ tool relies on.

use std::collections::HashMap;
use std::path::Path;

/// `MMAP::matchWildcardFilter` — byte for byte, including its quirks
/// (a `*` only skips up to the first occurrence of the next filter byte).
pub fn match_wildcard_filter(filter: &str, s: &str) -> bool {
    let f = filter.as_bytes();
    let s = s.as_bytes();
    // C strings: reading past the end yields '\0'.
    let at = |b: &[u8], i: usize| b.get(i).copied().unwrap_or(0);
    let (mut fi, mut si) = (0usize, 0usize);

    while at(f, fi) != 0 && at(s, si) != 0 {
        if at(f, fi) == b'*' {
            fi += 1;
            if at(f, fi) == 0 {
                return true;
            }
            loop {
                if at(f, fi) == at(s, si) {
                    break;
                }
                if at(s, si) == 0 {
                    return false;
                }
                si += 1;
            }
        } else if at(f, fi) != at(s, si) {
            return false;
        }
        fi += 1;
        si += 1;
    }

    (at(f, fi) == 0
        || (at(f, fi) == b'*' && {
            fi += 1;
            at(f, fi) == 0
        }))
        && at(s, si) == 0
}

/// `MMAP::ListFilesResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFilesResult {
    DirectoryNotFound,
    Ok,
}

/// `MMAP::getDirContents` (POSIX branch): every entry except `.`/`..`
/// matching `filter`, in `readdir` order (`std::fs::read_dir` is `readdir`).
pub fn get_dir_contents(
    file_list: &mut Vec<String>,
    dirpath: &Path,
    filter: &str,
) -> ListFilesResult {
    let Ok(dir) = std::fs::read_dir(dirpath) else {
        return ListFilesResult::DirectoryNotFound;
    };
    for entry in dir {
        let Ok(entry) = entry else { break };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name != "." && name != ".." && match_wildcard_filter(filter, &name) {
            file_list.push(name);
        }
    }
    ListFilesResult::Ok
}

/// `MMAP::MapEntry` (filled from Map.db2 by `LoadMap`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapEntry {
    pub map_type: u8,
    pub instance_type: i8,
    pub parent_map_id: i16,
    pub flags: i32,
}

impl Default for MapEntry {
    fn default() -> Self {
        Self {
            map_type: 0,
            instance_type: 0,
            parent_map_id: -1,
            flags: 0,
        }
    }
}

/// The generator's process-wide DB2 data (`sMapStore`, `_liquidTypes`,
/// `_mapDataForVmapInitialization` in PathGenerator.cpp).
#[derive(Debug, Clone, Default)]
pub struct GeneratorData {
    pub map_store: HashMap<u32, MapEntry>,
    /// LiquidType id -> `SoundBank`.
    pub liquid_types: HashMap<u32, u8>,
    /// Parent map id -> child map ids (`VMapManager2::InitializeThreadUnsafe`).
    pub map_data_for_vmap: HashMap<u32, Vec<u32>>,
}

impl GeneratorData {
    /// `sMapStore[mapID].ParentMapID` (operator[] default-constructs `-1`).
    pub fn parent_map_id(&self, map_id: u32) -> i32 {
        self.map_store
            .get(&map_id)
            .map_or(-1, |e| i32::from(e.parent_map_id))
    }

    /// `VMapManager2::GetLiquidFlagsPtr` lambda from PathGenerator.cpp:
    /// `1 << SoundBank` (x86 shift semantics for counts >= 32), or 0.
    pub fn liquid_flags(&self, liquid_id: u32) -> u32 {
        self.liquid_types
            .get(&liquid_id)
            .map_or(0, |&sb| 1u32.wrapping_shl(u32::from(sb)))
    }
}

/// C `atoi` (via `strtol`, base 10, saturating like glibc's cast of the long).
pub fn c_atoi(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || (b'\t'..=b'\r').contains(&b[i])) {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        neg = b[i] == b'-';
        i += 1;
    }
    let mut v: i64 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        // strtol saturates at LONG_MAX / LONG_MIN.
        v = v.saturating_mul(10).saturating_add(i64::from(b[i] - b'0'));
        i += 1;
    }
    let v = if neg {
        if v == i64::MAX { i64::MIN } else { -v }
    } else {
        v
    };
    // strtol returns a long; atoi truncates it to int.
    v as i32
}

/// C `atof` (decimal subset of `strtod`: sign, digits, fraction, exponent,
/// `inf`/`nan`); returns 0 when nothing parses.
pub fn c_atof(s: &str) -> f64 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || (b'\t'..=b'\r').contains(&b[i])) {
        i += 1;
    }
    let start = i;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let rest = &s[i..];
    let lower = rest.to_ascii_lowercase();
    for word in ["infinity", "inf", "nan"] {
        if lower.starts_with(word) {
            return s[start..i + word.len()].parse::<f64>().unwrap_or(0.0);
        }
    }
    let digits_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    let mut mantissa_digits = i - digits_start;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        mantissa_digits += i - frac_start;
    }
    if mantissa_digits == 0 {
        return 0.0;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_start {
            i = j;
        }
    }
    let text = &s[start..i];
    let text = text.strip_suffix('.').unwrap_or(text);
    text.parse::<f64>().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_filter_matches_cpp() {
        assert!(match_wildcard_filter("*", "anything"));
        assert!(match_wildcard_filter("*.vmtree", "0000.vmtree"));
        assert!(!match_wildcard_filter("*.vmtree", "0000_31_31.vmtile"));
        assert!(match_wildcard_filter("0001_*.vmtile", "0001_31_32.vmtile"));
        assert!(!match_wildcard_filter("0001_*.vmtile", "0001.vmtree"));
        assert!(match_wildcard_filter("0000*", "0000_31_31.map"));
        assert!(!match_wildcard_filter("0000*", "0001_31_31.map"));
        assert!(match_wildcard_filter("abc", "abc"));
        assert!(!match_wildcard_filter("abc", "abcd"));
        // C++ quirk: '*' stops at the first '.', so the rest must match exactly.
        assert!(!match_wildcard_filter("*.b", "a.c.b"));
        assert!(match_wildcard_filter("a*", "a"));
        assert!(!match_wildcard_filter("", "a"));
        assert!(match_wildcard_filter("*", ""));
    }

    #[test]
    fn atoi_atof_like_c() {
        assert_eq!(c_atoi("0000_31_32.map"), 0);
        assert_eq!(c_atoi("0530"), 530);
        assert_eq!(c_atoi("  -12abc"), -12);
        assert_eq!(c_atoi("x"), 0);
        assert_eq!(c_atoi("31"), 31);
        assert!((c_atof("55.5deg") - 55.5).abs() < 1e-12);
        assert!((c_atof("1e1") - 10.0).abs() < 1e-12);
        assert!((c_atof("3.") - 3.0).abs() < 1e-12);
        assert_eq!(c_atof("abc"), 0.0);
        assert_eq!(c_atof("-"), 0.0);
    }

    #[test]
    fn liquid_flags_shift() {
        let mut d = GeneratorData::default();
        d.liquid_types.insert(1, 0);
        d.liquid_types.insert(2, 3);
        d.liquid_types.insert(3, 33);
        assert_eq!(d.liquid_flags(1), 1);
        assert_eq!(d.liquid_flags(2), 8);
        assert_eq!(d.liquid_flags(3), 2);
        assert_eq!(d.liquid_flags(9), 0);
    }
}
