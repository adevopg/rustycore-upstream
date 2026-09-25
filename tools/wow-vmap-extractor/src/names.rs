//! File name helpers from `src/tools/vmap4_extractor/adtfile.cpp`: `GetPlainName`,
//! `FixNameCase`, `FixNameSpaces`, `NormalizeFileName`, plus the `FILE%08X.xxx` naming
//! (`Trinity::StringFormat("FILE{:08X}.xxx", id)`) used for every FileDataID-referenced
//! model.
//!
//! Names are handled as raw bytes like the C++ `char*` code.

/// `Trinity::StringFormat("FILE{:08X}.xxx", fileDataId)`.
pub fn file_data_id_name(file_data_id: u32) -> Vec<u8> {
    format!("FILE{file_data_id:08X}.xxx").into_bytes()
}

/// `GetPlainName`: the part after the last backslash (forward slashes are kept).
pub fn get_plain_name(name: &[u8]) -> &[u8] {
    match name.iter().rposition(|&b| b == b'\\') {
        Some(pos) => &name[pos + 1..],
        None => name,
    }
}

/// Offset of the plain name inside `name` (for in-place normalization).
pub fn plain_name_offset(name: &[u8]) -> usize {
    name.iter()
        .rposition(|&b| b == b'\\')
        .map_or(0, |pos| pos + 1)
}

/// `FixNameCase`: lower-case the extension, then capitalize the first letter of every
/// alphabetic run and lower-case the rest.
///
/// When the name has no `.`, the C++ loop that lower-cases the extension walks off the
/// start of the buffer (so the whole name gets `| 0x20` and the capitalization pass does
/// not run); only the in-name part of that behavior is observable and it is reproduced.
fn fix_name_case(name: &mut [u8]) {
    let Some(dot) = name.iter().rposition(|&b| b == b'.') else {
        for b in name.iter_mut() {
            *b |= 0x20;
        }
        return;
    };
    for b in &mut name[dot + 1..] {
        *b |= 0x20;
    }
    let mut i = dot as isize;
    while i >= 0 {
        let p = i as usize;
        let c = name[p];
        if p > 0 && c.is_ascii_uppercase() && name[p - 1].is_ascii_alphabetic() {
            name[p] |= 0x20;
        } else if (p == 0 || !name[p - 1].is_ascii_alphabetic()) && c.is_ascii_lowercase() {
            name[p] &= !0x20;
        }
        i -= 1;
    }
}

/// `FixNameSpaces`: spaces become underscores, except in the last three characters.
fn fix_name_spaces(name: &mut [u8]) {
    let len = name.len();
    if len < 3 {
        return;
    }
    for b in &mut name[..len - 3] {
        if *b == b' ' {
            *b = b'_';
        }
    }
}

/// `NormalizeFileName`: FileDataID-formatted names (`FILE...`) are left untouched.
pub fn normalize_file_name(name: &mut [u8]) {
    if name.len() >= 4 && &name[..4] == b"FILE" {
        return;
    }
    fix_name_case(name);
    fix_name_spaces(name);
}

/// `GetPlainName` + `NormalizeFileName` into a new buffer.
pub fn normalized_plain_name(name: &[u8]) -> Vec<u8> {
    let mut plain = get_plain_name(name).to_vec();
    normalize_file_name(&mut plain);
    plain
}

/// Lossy display/lookup string of a byte name.
pub fn lossy(name: &[u8]) -> String {
    String::from_utf8_lossy(name).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(s: &str) -> String {
        lossy(&normalized_plain_name(s.as_bytes()))
    }

    #[test]
    fn normalizes_like_cpp() {
        assert_eq!(
            norm("World\\wmo\\Dungeon\\AZ_Blackrock\\Blackrock.WMO"),
            "Blackrock.wmo"
        );
        assert_eq!(
            norm("world\\GENERIC\\human\\my tree_01.M2"),
            "My_Tree_01.m2"
        );
        assert_eq!(norm("a\\b\\stormwind LION.mdx"), "Stormwind_Lion.mdx");
        assert_eq!(norm("FILE0001ABCD.xxx"), "FILE0001ABCD.xxx");
        assert_eq!(norm("x\\ab c.m2"), "Ab_C.m2");
        assert_eq!(norm("x\\2bad.m2"), "2Bad.m2");
    }

    #[test]
    fn plain_name_uses_backslash_only() {
        assert_eq!(get_plain_name(b"a/b\\c/d.m2"), b"c/d.m2");
        assert_eq!(get_plain_name(b"plain.m2"), b"plain.m2");
        assert_eq!(file_data_id_name(0x1ABCD), b"FILE0001ABCD.xxx");
    }
}
