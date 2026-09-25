//! Blizzard patch-server tables (`/<product>/versions`, `/cdns`, `/bgdl`):
//! a header line of `Name!TYPE:size` columns, a `## seqn = N` line and
//! pipe-separated rows.
//!
//! Parsing follows blizzget `NGDP::NGDP` (skip `##` lines, header detected by
//! the `!` in its column names, columns split at `|`); rendering reproduces the
//! exact byte layout returned by `http://us.patch.battle.net:1119` (LF line ends,
//! trailing LF).

use std::fmt::Write as _;

use anyhow::{Context, Result, bail};

use crate::product::{self, REGIONS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// Full column specs, e.g. `BuildConfig!HEX:16`.
    pub header: Vec<String>,
    pub seqn: Option<u64>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn parse(text: &str) -> Result<Self> {
        let mut header: Option<Vec<String>> = None;
        let mut seqn = None;
        let mut rows = Vec::new();
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            if let Some(comment) = line.strip_prefix("##") {
                if let Some((name, value)) = comment.split_once('=')
                    && name.trim() == "seqn"
                {
                    seqn = value.trim().parse().ok();
                }
                continue;
            }
            let columns: Vec<String> = line.split('|').map(str::to_owned).collect();
            match &header {
                None => {
                    if !line.contains('!') {
                        bail!("table header expected, got {line:?}");
                    }
                    header = Some(columns);
                }
                Some(h) if h.len() == columns.len() => rows.push(columns),
                Some(h) => bail!(
                    "table row has {} columns, header has {}: {line:?}",
                    columns.len(),
                    h.len()
                ),
            }
        }
        Ok(Self {
            header: header.context("empty table")?,
            seqn,
            rows,
        })
    }

    /// Index of a column by its name (the part before `!`), ignoring case.
    pub fn column(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|h| {
            h.split('!')
                .next()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
    }

    pub fn get(&self, row: usize, column: &str) -> Option<&str> {
        let c = self.column(column)?;
        self.rows.get(row)?.get(c).map(String::as_str)
    }

    /// First row whose `column` equals `value` (case-insensitive).
    pub fn find_row(&self, column: &str, value: &str) -> Option<usize> {
        let c = self.column(column)?;
        self.rows
            .iter()
            .position(|r| r[c].eq_ignore_ascii_case(value))
    }

    pub fn render(&self) -> String {
        let mut out = self.header.join("|");
        out.push('\n');
        if let Some(seqn) = self.seqn {
            let _ = writeln!(out, "## seqn = {seqn}");
        }
        for row in &self.rows {
            out.push_str(&row.join("|"));
            out.push('\n');
        }
        out
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// `versions` advertising 3.4.3.54261 for every region (Blizzard's column
/// layout, including its `VersionsName!String:0` spelling).
pub fn versions_table(seqn: u64) -> Table {
    Table {
        header: strings(&[
            "Region!STRING:0",
            "BuildConfig!HEX:16",
            "CDNConfig!HEX:16",
            "KeyRing!HEX:16",
            "BuildId!DEC:4",
            "VersionsName!String:0",
            "ProductConfig!HEX:16",
        ]),
        seqn: Some(seqn),
        rows: REGIONS
            .iter()
            .map(|r| {
                vec![
                    r.name.to_owned(),
                    product::BUILD_CONFIG.to_owned(),
                    product::CDN_CONFIG.to_owned(),
                    String::new(),
                    product::BUILD_ID.to_string(),
                    product::VERSIONS_NAME.to_owned(),
                    product::PRODUCT_CONFIG.to_owned(),
                ]
            })
            .collect(),
    }
}

/// `cdns` whose hosts are the local server itself (`host:port`).
pub fn cdns_table(seqn: u64, host_port: &str) -> Table {
    Table {
        header: strings(&[
            "Name!STRING:0",
            "Path!STRING:0",
            "Hosts!STRING:0",
            "Servers!STRING:0",
            "ConfigPath!STRING:0",
        ]),
        seqn: Some(seqn),
        rows: REGIONS
            .iter()
            .map(|r| {
                vec![
                    r.name.to_owned(),
                    product::CDN_PATH.to_owned(),
                    host_port.to_owned(),
                    format!("http://{host_port}/?maxhosts=4"),
                    product::CONFIG_PATH.to_owned(),
                ]
            })
            .collect(),
    }
}

/// `bgdl` (background download): Blizzard serves the `versions` header with no
/// rows for `wow_classic`.
pub fn bgdl_table(seqn: u64) -> Table {
    let mut table = versions_table(seqn);
    table.rows.clear();
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL_CDNS: &str = "Name!STRING:0|Path!STRING:0|Hosts!STRING:0|Servers!STRING:0|ConfigPath!STRING:0\n\
## seqn = 4019526\n\
us|tpr/wow|level3.blizzard.com us.cdn.blizzard.com|http://level3.blizzard.com/?maxhosts=8|tpr/configs/data\n\
eu|tpr/wow|level3.blizzard.com|http://level3.blizzard.com/?maxhosts=8|tpr/configs/data\n";

    #[test]
    fn parses_real_cdns() {
        let t = Table::parse(REAL_CDNS).unwrap();
        assert_eq!(t.seqn, Some(4_019_526));
        assert_eq!(t.rows.len(), 2);
        let eu = t.find_row("Name", "EU").unwrap();
        assert_eq!(t.get(eu, "hosts"), Some("level3.blizzard.com"));
        assert_eq!(t.get(0, "ConfigPath"), Some("tpr/configs/data"));
        assert_eq!(t.render(), REAL_CDNS, "render reproduces Blizzard bytes");
    }

    #[test]
    fn versions_round_trip() {
        let text = versions_table(42).render();
        assert!(text.starts_with("Region!STRING:0|BuildConfig!HEX:16|CDNConfig!HEX:16|KeyRing!HEX:16|BuildId!DEC:4|VersionsName!String:0|ProductConfig!HEX:16\n## seqn = 42\nus|c91609c69ed2ab39d44039390a1be969|a838aeb3cda2e027e9c96bd9953944b3||54261|3.4.3.54261|fac7680539cd51bc0a791a88ade3da21\n"));
        let t = Table::parse(&text).unwrap();
        assert_eq!(t.rows.len(), 5);
        let kr = t.find_row("Region", "kr").unwrap();
        assert_eq!(t.get(kr, "VersionsName"), Some("3.4.3.54261"));

        let c = Table::parse(&cdns_table(42, "127.0.0.1:1119").render()).unwrap();
        assert_eq!(c.get(0, "Hosts"), Some("127.0.0.1:1119"));
        assert_eq!(c.get(0, "Path"), Some("tpr/wow"));
        assert!(bgdl_table(1).rows.is_empty());
    }

    #[test]
    fn rejects_garbage() {
        assert!(Table::parse("").is_err());
        assert!(Table::parse("no header\n").is_err());
        assert!(Table::parse("A!STRING:0|B!STRING:0\nx\n").is_err());
    }
}
