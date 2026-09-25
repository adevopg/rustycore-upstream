//! Build config / CDN config text files (`name = value value ...`).
//!
//! Same rules as blizzget `NGDP::ParseConfig` (lines starting with `#` are
//! comments, `name = value`), with values split on white space as the TACT
//! consumers do (`split(buildConfig["encoding"])[1]` in blizzget `data.cpp`).

use anyhow::{Result, bail};

use crate::util::{Key, md5, parse_key};

#[derive(Debug, Clone, Default)]
pub struct ConfigFile {
    entries: Vec<(String, Vec<String>)>,
}

impl ConfigFile {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let text = String::from_utf8_lossy(data);
        if text.trim_start().starts_with('<') {
            bail!("config file is an HTML/XML error page");
        }
        let mut entries = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            entries.push((
                name.trim().to_owned(),
                value.split_whitespace().map(str::to_owned).collect(),
            ));
        }
        Ok(Self { entries })
    }

    /// Parses after checking the file's MD5 against its key (configs are
    /// content-addressed).
    pub fn parse_verified(data: &[u8], key: &Key) -> Result<Self> {
        if &md5(data) != key {
            bail!("config file does not match its MD5 key");
        }
        Self::parse(data)
    }

    pub fn values(&self, name: &str) -> Option<&[String]> {
        self.entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
    }

    pub fn first(&self, name: &str) -> Option<&str> {
        self.values(name)?.first().map(String::as_str)
    }

    /// Key number `index` of a hash-valued variable.
    pub fn key(&self, name: &str, index: usize) -> Option<Key> {
        parse_key(self.values(name)?.get(index)?)
    }

    /// Every variable name, in file order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(n, _)| n.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Build Configuration\n\nroot = 28f965732c7762e2ef3d1c00d9d2f801\n\
install = 8c3b9bb3248a8d718f76a7092ac0cc24 59bebcea12d0a9ceba2af8859653bcc6\n\
install-size = 17491 16957\n\
build-name = WOW-54261patch3.4.3_ClassicRetail\n";

    #[test]
    fn parses_values() {
        let cfg = ConfigFile::parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(cfg.key("root", 0).unwrap()[0], 0x28);
        assert!(cfg.key("root", 1).is_none());
        assert_eq!(cfg.key("install", 1).unwrap()[15], 0xc6);
        assert_eq!(cfg.values("install-size").unwrap(), ["17491", "16957"]);
        assert_eq!(
            cfg.first("build-name"),
            Some("WOW-54261patch3.4.3_ClassicRetail")
        );
        assert_eq!(cfg.names().count(), 4);
    }

    #[test]
    fn md5_is_checked() {
        let key = md5(SAMPLE.as_bytes());
        assert!(ConfigFile::parse_verified(SAMPLE.as_bytes(), &key).is_ok());
        assert!(ConfigFile::parse_verified(b"root = 00", &key).is_err());
        assert!(ConfigFile::parse(b"<?xml version=\"1.0\"?><Error/>").is_err());
    }
}
