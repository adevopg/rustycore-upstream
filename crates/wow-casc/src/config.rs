//! Build config (`Data/config/xx/yy/<build key>`) parsing.
//!
//! Port of `CascLib` `dep/CascLib/src/CascFiles.cpp`: `ParseFile_CdnBuild`,
//! `CheckConfigFileVariable`, `CaptureSingleString`, `CaptureSingleHash`,
//! `CaptureHashCount`, `CaptureDecimalInteger`, `LoadCKeyEntry`,
//! `LoadBuildNumber`, `FetchAndLoadConfigFile` (MD5 verification through
//! `ListFile_VerifyMD5` / `CascVerifyDataBlockHash`) and
//! `common/ListFile.cpp`: `ListFile_GetNextLine`.

use md5::{Digest, Md5};

use crate::build_info::parse_build_number;
use crate::{Error, Result};

/// `CascLib` `CASC_INVALID_SIZE`.
pub const INVALID_SIZE: u32 = 0xFFFF_FFFF;

/// The subset of `CascLib` `CASC_CKEY_ENTRY` filled from a build config line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigKeyEntry {
    pub ckey: Option<[u8; 16]>,
    pub ekey: Option<[u8; 16]>,
    pub content_size: u32,
    pub encoded_size: u32,
}

impl Default for ConfigKeyEntry {
    fn default() -> Self {
        Self {
            ckey: None,
            ekey: None,
            content_size: INVALID_SIZE,
            encoded_size: INVALID_SIZE,
        }
    }
}

/// Values of the build config that the local `WoW` reader needs.
#[derive(Debug, Clone, Default)]
pub struct BuildConfig {
    pub root: ConfigKeyEntry,
    pub encoding: ConfigKeyEntry,
    pub install: ConfigKeyEntry,
    pub vfs_root: ConfigKeyEntry,
    /// `LoadBuildNumber` over `build-name`, if it yielded a value.
    pub build_name_number: Option<u32>,
    /// `build-uid` (`CascLib` `LoadBuildProductId`).
    pub build_uid: Option<String>,
}

/// `CascLib` `IsWhiteSpace`: `0 <= ch <= 0x20` (signed char, so bytes >= 0x80
/// are *not* white space).
fn is_white_space(ch: u8) -> bool {
    ch <= 0x20
}

/// `CascLib` `CaptureSingleHash`: 32 hex digits followed by a white space or
/// the end of the data.
fn capture_single_hash(data: &[u8], mut pos: usize) -> Option<([u8; 16], usize)> {
    while pos < data.len() && is_white_space(data[pos]) {
        pos += 1;
    }
    let text = data.get(pos..pos + 32)?;
    let mut out = [0u8; 16];
    for (i, pair) in text.as_chunks::<2>().0.iter().enumerate() {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    pos += 32;
    if pos < data.len() && !is_white_space(data[pos]) {
        return None;
    }
    Some((out, pos))
}

/// `CascLib` `CaptureHashCount`: every token of the value must be a hash.
fn capture_hash_count(data: &[u8]) -> Option<usize> {
    let mut pos = 0;
    let mut count = 0;
    while pos < data.len() {
        let (_, next) = capture_single_hash(data, pos)?;
        pos = next;
        while pos < data.len() && is_white_space(data[pos]) {
            pos += 1;
        }
        count += 1;
    }
    Some(count)
}

/// `CascLib` `CaptureDecimalInteger`.
fn capture_decimal(data: &[u8], mut pos: usize) -> Option<(u32, usize)> {
    while pos < data.len() && data[pos] == b' ' {
        pos += 1;
    }
    let start = pos;
    let mut total: u32 = 0;
    while pos < data.len() && data[pos] != b' ' {
        if !data[pos].is_ascii_digit() {
            break;
        }
        total = total
            .wrapping_mul(10)
            .wrapping_add(u32::from(data[pos] - b'0'));
        pos += 1;
    }
    (pos != start).then_some((total, pos))
}

/// `CascLib` `LoadCKeyEntry`. Returns `false` for unrecognized values.
fn load_ckey_entry(name: &str, data: &[u8], entry: &mut ConfigKeyEntry) -> bool {
    if name.len() > 7 && name.ends_with("-config") {
        return true;
    }
    if name.len() > 5 && name.ends_with("-size") {
        let Some((content_size, pos)) = capture_decimal(data, 0) else {
            return false;
        };
        let encoded_size = capture_decimal(data, pos).map_or(INVALID_SIZE, |(v, _)| v);
        entry.content_size = content_size;
        entry.encoded_size = encoded_size;
        return true;
    }
    let Some(count) = capture_hash_count(data) else {
        return false;
    };
    let mut pos = 0;
    if count >= 1 {
        let Some((ckey, next)) = capture_single_hash(data, pos) else {
            return false;
        };
        entry.ckey = Some(ckey);
        pos = next;
    }
    if count == 2 {
        let Some((ekey, _)) = capture_single_hash(data, pos) else {
            return false;
        };
        entry.ekey = Some(ekey);
    }
    count == 1 || count == 2
}

/// `CascLib` `CascCheckWildCard` for the patterns used by `ParseFile_CdnBuild`
/// (a literal, optionally followed by a single trailing `*`), compared
/// case-insensitively like `AsciiToUpperTable_BkSlash`.
fn wildcard_match(name: &str, pattern: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => {
            name.len() >= prefix.len() && name[..prefix.len()].eq_ignore_ascii_case(prefix)
        }
        None => name.eq_ignore_ascii_case(pattern),
    }
}

/// Splits one config line into `(variable name, value bytes)` like
/// `CheckConfigFileVariable` (name up to white space or `=`, then white space
/// and `=` skipped). Returns `None` when there is no value.
fn split_variable(line: &[u8]) -> Option<(String, &[u8])> {
    let mut pos = 0;
    while pos < line.len() && is_white_space(line[pos]) {
        pos += 1;
    }
    let start = pos;
    // CaptureSingleString copies at most MAX_VAR_NAME - 1 (79) characters.
    while pos < line.len() && !is_white_space(line[pos]) && line[pos] != b'=' && pos - start < 79 {
        pos += 1;
    }
    let name = String::from_utf8_lossy(&line[start..pos]).into_owned();
    while pos < line.len() && (is_white_space(line[pos]) || line[pos] == b'=') {
        pos += 1;
    }
    if pos >= line.len() {
        return None;
    }
    Some((name, &line[pos..]))
}

/// `ListFile_GetNextLine` over the whole file: lines are split at CR, LF or
/// 0x85, leading control/space characters are skipped.
fn config_lines(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    data.split(|&b| b == b'\n' || b == b'\r' || b == 0x85)
        .map(|line| {
            let skip = line.iter().take_while(|&&b| b <= 0x20).count();
            &line[skip..]
        })
        .filter(|line| !line.is_empty())
}

/// `CascVerifyDataBlockHash`: an all-zero expected hash is not verified.
pub fn verify_md5(data: &[u8], expected: &[u8; 16]) -> bool {
    if expected.iter().all(|&b| b == 0) {
        return true;
    }
    Md5::digest(data).as_slice() == expected
}

/// `ParseFile_CdnBuild` (after `FetchAndLoadConfigFile` verified the MD5).
pub fn parse_build_config(data: &[u8]) -> Result<BuildConfig> {
    let mut cfg = BuildConfig::default();
    for line in config_lines(data) {
        let Some((name, value)) = split_variable(line) else {
            continue;
        };
        // The order of the checks is the order in ParseFile_CdnBuild; the
        // first handler that accepts the variable wins.
        if wildcard_match(&name, "build-uid") {
            if cfg.build_uid.is_none() {
                cfg.build_uid = Some(String::from_utf8_lossy(value).into_owned());
            }
            continue;
        }
        if wildcard_match(&name, "build-name") {
            // LoadBuildNumber; the caller applies it only when .build.info
            // gave no build number (`hs->dwBuildNumber == 0`).
            if cfg.build_name_number.is_none() {
                cfg.build_name_number = parse_build_number(&String::from_utf8_lossy(value));
            }
            continue;
        }
        if wildcard_match(&name, "root*") && load_ckey_entry(&name, value, &mut cfg.root) {
            continue;
        }
        if wildcard_match(&name, "install*") && load_ckey_entry(&name, value, &mut cfg.install) {
            continue;
        }
        if wildcard_match(&name, "download*") {
            continue;
        }
        if wildcard_match(&name, "encoding*") && load_ckey_entry(&name, value, &mut cfg.encoding) {
            continue;
        }
        if wildcard_match(&name, "vfs-root*") {
            load_ckey_entry(&name, value, &mut cfg.vfs_root);
        }
    }

    // "Both CKey and EKey of ENCODING file is required"
    if cfg.encoding.ckey.is_none() || cfg.encoding.ekey.is_none() {
        return Err(Error::InvalidStorage(
            "build config lacks the ENCODING CKey + EKey".into(),
        ));
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Build Configuration\n\
\n\
root = 051a7757e36b884b1f8f9fd368b662e8\n\
install = c2b638576a07c3ebce59bf4ca28a3820 8d7fa7d8544305bf0491e534e0b8e9aa\n\
install-size = 19333 18532\n\
encoding = d8e6b4d7ee686bb4d62a0d3b8c8e0726 e103d037f40e72c1b4cabd19a2f83bfe\n\
encoding-size = 160535468 160259972\n\
build-name = WOW-70009patch1.60.1_ForeverBeta\n\
build-uid = wow_classic_beta\n\
vfs-root = acae8f42a9a6b6b1d342a4dfec55d8ee b555c93a67dfdbf660cf9d4ae53380f4\n\
vfs-root-size = 52937 33351\n";

    #[test]
    fn parses_build_config() {
        let cfg = parse_build_config(SAMPLE.as_bytes()).unwrap();
        assert_eq!(cfg.root.ckey.unwrap()[0], 0x05);
        assert_eq!(cfg.root.ekey, None);
        assert_eq!(cfg.encoding.ckey.unwrap()[0], 0xd8);
        assert_eq!(cfg.encoding.ekey.unwrap()[15], 0xfe);
        assert_eq!(cfg.encoding.content_size, 160_535_468);
        assert_eq!(cfg.encoding.encoded_size, 160_259_972);
        assert_eq!(cfg.install.content_size, 19333);
        assert_eq!(cfg.build_name_number, Some(70009));
        assert_eq!(cfg.build_uid.as_deref(), Some("wow_classic_beta"));
        assert_eq!(cfg.vfs_root.content_size, 52937);
    }

    #[test]
    fn encoding_is_required() {
        let text = "root = 051a7757e36b884b1f8f9fd368b662e8\nencoding = d8e6b4d7ee686bb4d62a0d3b8c8e0726\n";
        assert!(parse_build_config(text.as_bytes()).is_err());
    }

    #[test]
    fn md5_verification() {
        let data = b"abc";
        let md5_abc = [
            0x90, 0x01, 0x50, 0x98, 0x3c, 0xd2, 0x4f, 0xb0, 0xd6, 0x96, 0x3f, 0x7d, 0x28, 0xe1,
            0x7f, 0x72,
        ];
        assert!(verify_md5(data, &md5_abc));
        assert!(!verify_md5(b"abd", &md5_abc));
        assert!(verify_md5(b"anything", &[0; 16]));
    }
}
