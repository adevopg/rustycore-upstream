//! `.build.info` parsing and product selection.
//!
//! Port of `CascLib` `dep/CascLib/src/common/Csv.cpp` (`CASC_CSV`: pipe-separated
//! columns, header line with `Name!TYPE:size` column names, lines whose column
//! count differs from the header are ignored, at most 0x40 lines) and
//! `dep/CascLib/src/CascFiles.cpp`: `ParseFile_BuildInfo`, `LoadQueryKey`,
//! `GetDefaultLocaleMask`, `GetLocaleValue`, `LoadBuildNumber`.

use crate::locale;
use crate::{Error, Result};

/// `CascLib` `CASC_CSV(0x40, true)` line limit used by `LoadCsvFile`.
const CSV_MAX_LINES: usize = 0x40;

/// A parsed pipe-separated table with a header line (`CASC_CSV`).
#[derive(Debug, Clone)]
pub struct Csv {
    header: Vec<String>,
    lines: Vec<Vec<String>>,
}

impl Csv {
    /// `CASC_CSV::Load` + `ParseCsvData` with `bHasHeader = true`.
    pub fn parse(text: &str) -> Option<Self> {
        // "Overwatch ROOT's header begins with '#'"
        let text = text.strip_prefix('#').unwrap_or(text);
        // NextLine_Default: lines end at CR/LF; empty lines are skipped.
        let mut raw_lines = text.split(['\n', '\r']).filter(|l| !l.is_empty());
        let header: Vec<String> = raw_lines.next()?.split('|').map(str::to_owned).collect();
        let mut lines = Vec::new();
        for line in raw_lines {
            if lines.len() >= CSV_MAX_LINES {
                break;
            }
            let columns: Vec<String> = line.split('|').map(str::to_owned).collect();
            // "In the case of mismatched column count, ignore the line"
            if columns.len() == header.len() {
                lines.push(columns);
            }
        }
        Some(Self { header, lines })
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// `CASC_CSV_LINE::operator[](const char *)`: exact column-name match.
    pub fn get(&self, line: usize, column: &str) -> Option<&str> {
        let index = self.header.iter().position(|h| h == column)?;
        self.lines.get(line)?.get(index).map(String::as_str)
    }

    pub fn has_column(&self, column: &str) -> bool {
        self.header.iter().any(|h| h == column)
    }
}

/// The values `CascLib` takes from the selected `.build.info` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    /// `hs->CdnBuildKey` (key of the build config in `Data/config`).
    pub build_key: [u8; 16],
    /// `hs->CdnConfigKey` (key of the CDN config in `Data/config`).
    pub cdn_key: [u8; 16],
    /// `hs->dwDefaultLocale` from the `Tags` column.
    pub default_locale: u32,
    /// Build number from the `Version` column (`LoadBuildNumber`), 0 if none.
    pub build_number: u32,
    /// Row index that was selected.
    pub selected_row: usize,
}

/// Parses a 32-character hex string into a 16-byte key (`CascLib`
/// `LoadQueryKey`: the column must be exactly `MD5_STRING_SIZE` long).
pub fn parse_md5_hex(text: &str) -> Option<[u8; 16]> {
    let bytes = text.as_bytes();
    if bytes.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Some(out)
}

/// `CascLib` `GetLocaleValue` for a 4-character tag.
fn locale_value(tag: &[u8]) -> u32 {
    match tag {
        b"enUS" => locale::ENUS,
        b"enGB" => locale::ENGB,
        b"enCN" => locale::ENCN,
        b"enTW" => locale::ENTW,
        b"esES" => locale::ESES,
        b"esMX" => locale::ESMX,
        b"ptBR" => locale::PTBR,
        b"ptPT" => locale::PTPT,
        b"zhCN" => locale::ZHCN,
        b"zhTW" => locale::ZHTW,
        b"koKR" => locale::KOKR,
        b"frFR" => locale::FRFR,
        b"deDE" => locale::DEDE,
        b"ruRU" => locale::RURU,
        b"itIT" => locale::ITIT,
        _ => locale::NONE,
    }
}

/// `CascLib` `GetDefaultLocaleMask`: scans the `Tags` string for any 4-char
/// locale code (case-sensitive), anywhere in the text.
pub fn default_locale_mask(tags: &str) -> u32 {
    let bytes = tags.as_bytes();
    let mut mask = 0;
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let value = locale_value(&bytes[i..i + 4]);
        if value == locale::NONE {
            i += 1;
        } else {
            mask |= value;
            i += 4;
        }
    }
    mask
}

/// `CascLib` `LoadBuildNumber`: the largest decimal run in the string, accepted
/// only if it is at least 100. Returns `None` when no such number exists.
pub fn parse_build_number(text: &str) -> Option<u32> {
    let mut current: u32 = 0;
    let mut max_value: u32 = 0;
    for ch in text.bytes() {
        if ch.is_ascii_digit() {
            current = current.wrapping_mul(10).wrapping_add(u32::from(ch - b'0'));
            max_value = max_value.max(current);
        } else {
            current = 0;
        }
    }
    (max_value >= 100).then_some(max_value)
}

const COL_ACTIVE: &str = "Active!DEC:1";
const COL_PRODUCT: &str = "Product!STRING:0";
const COL_BUILD_KEY: &str = "Build Key!HEX:16";
const COL_CDN_KEY: &str = "CDN Key!HEX:16";
const COL_TAGS: &str = "Tags!STRING:0";
const COL_VERSION: &str = "Version!STRING:0";

/// `CascLib` `ParseFile_BuildInfo` with a product code name given by the caller
/// (`TrinityCore` always passes `szCodeName`): picks the first *active* row whose
/// `Product` equals `product` case-insensitively; if the file has no `Product`
/// column the first active row is taken.
pub fn select_build_info(csv: &Csv, product: &str) -> Result<BuildInfo> {
    // CascLib compares against a copy truncated to a 0x20-char buffer.
    let wanted: String = product.chars().take(0x1F).collect();
    let has_product = csv.has_column(COL_PRODUCT);
    let mut selected = None;
    for i in 0..csv.line_count() {
        if csv.get(i, COL_ACTIVE) != Some("1") {
            continue;
        }
        if has_product {
            let row_product: String = csv
                .get(i, COL_PRODUCT)
                .unwrap_or_default()
                .chars()
                .take(0x1F)
                .collect();
            if !row_product.eq_ignore_ascii_case(&wanted) {
                continue;
            }
        }
        selected = Some(i);
        break;
    }
    let Some(row) = selected else {
        return Err(Error::ProductNotFound(product.to_owned()));
    };

    let key = |column: &str| {
        csv.get(row, column)
            .and_then(parse_md5_hex)
            .ok_or_else(|| Error::InvalidStorage(format!(".build.info: bad `{column}` column")))
    };
    let build_key = key(COL_BUILD_KEY)?;
    let cdn_key = key(COL_CDN_KEY)?;
    let default_locale = csv.get(row, COL_TAGS).map_or(0, default_locale_mask);
    let build_number = csv
        .get(row, COL_VERSION)
        .filter(|v| !v.is_empty())
        .and_then(parse_build_number)
        .unwrap_or(0);
    Ok(BuildInfo {
        build_key,
        cdn_key,
        default_locale,
        build_number,
        selected_row: row,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Branch!STRING:0|Active!DEC:1|Build Key!HEX:16|CDN Key!HEX:16|Install Key!HEX:16|IM Size!DEC:4|CDN Path!STRING:0|CDN Hosts!STRING:0|CDN Servers!STRING:0|Tags!STRING:0|Armadillo!STRING:0|Last Activated!STRING:0|Version!STRING:0|KeyRing!HEX:16|Product!STRING:0\n\
us|1|05215079e3905ef5922ae0b03ffefb73|9b3c456dbb837d133a026d380c7c13e9|||tpr/wow|level3.blizzard.com us.cdn.blizzard.com|http://level3.blizzard.com/?maxhosts=8|Windows x86_64 US? acct-ESP? geoip-ES? esES speech?:Windows x86_64 US? acct-ESP? geoip-ES? esES text?|||1.60.1.70009||wow_classic_beta\n\
eu|1|00000000000000000000000000000001|00000000000000000000000000000002|||tpr/wow|h|s|Windows x86_64 EU? enUS speech?:Windows x86_64 EU? enGB deDE text?|||3.4.3.54261||wow_classic\n\
eu|0|00000000000000000000000000000003|00000000000000000000000000000004|||tpr/wow|h|s|x|||3.4.3.54261||wow_classicera\n\
short|line\n";

    #[test]
    fn parses_header_and_rows() {
        let csv = Csv::parse(SAMPLE).unwrap();
        assert_eq!(csv.line_count(), 3, "mismatched column count line ignored");
        assert_eq!(csv.get(0, "Product!STRING:0"), Some("wow_classic_beta"));
        assert_eq!(csv.get(1, "Version!STRING:0"), Some("3.4.3.54261"));
        assert_eq!(csv.get(0, "Nope"), None);
    }

    #[test]
    fn selects_product_row() {
        let csv = Csv::parse(SAMPLE).unwrap();
        let info = select_build_info(&csv, "wow_classic_beta").unwrap();
        assert_eq!(info.selected_row, 0);
        assert_eq!(info.build_key[0], 0x05);
        assert_eq!(info.cdn_key[15], 0xe9);
        assert_eq!(info.build_number, 70009);
        assert_eq!(info.default_locale, locale::ESES);

        let info = select_build_info(&csv, "WOW_CLASSIC").unwrap();
        assert_eq!(info.selected_row, 1);
        assert_eq!(info.build_number, 54261);
        assert_eq!(
            info.default_locale,
            locale::ENUS | locale::ENGB | locale::DEDE
        );
    }

    #[test]
    fn inactive_or_missing_product_is_not_found() {
        let csv = Csv::parse(SAMPLE).unwrap();
        assert!(matches!(
            select_build_info(&csv, "wow_classic_era"),
            Err(Error::ProductNotFound(_))
        ));
        assert!(matches!(
            select_build_info(&csv, "wow"),
            Err(Error::ProductNotFound(_))
        ));
    }

    #[test]
    fn no_product_column_takes_first_active() {
        let text = "Active!DEC:1|Build Key!HEX:16|CDN Key!HEX:16\n\
                    0|00000000000000000000000000000001|00000000000000000000000000000002\n\
                    1|00000000000000000000000000000003|00000000000000000000000000000004\n";
        let csv = Csv::parse(text).unwrap();
        let info = select_build_info(&csv, "anything").unwrap();
        assert_eq!(info.selected_row, 1);
        assert_eq!(info.build_number, 0);
    }

    #[test]
    fn build_number_rules() {
        assert_eq!(
            parse_build_number("WOW-70009patch1.60.1_ForeverBeta"),
            Some(70009)
        );
        assert_eq!(parse_build_number("B29049"), Some(29049));
        assert_eq!(parse_build_number("30013_Win32_2_2_0_Ptr_ptr"), Some(30013));
        assert_eq!(parse_build_number("1.2.3"), None);
    }
}
