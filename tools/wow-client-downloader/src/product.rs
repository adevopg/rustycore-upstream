//! The one build this tool installs: `wow_classic` 3.4.3.54261
//! (`WOW-54261patch3.4.3_ClassicRetail`), plus the real Blizzard patch-server
//! values the temporary server imitates.
//!
//! Keys were taken from wago.tools and the build config MD5 was checked
//! against its key. The CDN rows are the `wow_classic/cdns` table returned by
//! `http://us.patch.battle.net:1119` (2026-09-25); `Hosts`/`Servers` are what
//! the Battle.net Agent writes into `.build.info`.

pub const PRODUCT: &str = "wow_classic";
pub const BUILD_CONFIG: &str = "c91609c69ed2ab39d44039390a1be969";
pub const CDN_CONFIG: &str = "a838aeb3cda2e027e9c96bd9953944b3";
pub const PRODUCT_CONFIG: &str = "fac7680539cd51bc0a791a88ade3da21";
pub const BUILD_ID: u32 = 54261;
pub const VERSIONS_NAME: &str = "3.4.3.54261";

/// `Path` column of `cdns`.
pub const CDN_PATH: &str = "tpr/wow";
/// `ConfigPath` column of `cdns` (product configs).
pub const CONFIG_PATH: &str = "tpr/configs/data";

/// Blizzard CDN hosts tried (in order) before the mirror.
pub const BLIZZARD_HOSTS: &[&str] = &["level3.blizzard.com", "us.cdn.blizzard.com"];
/// Community mirror of old CDN content (`archive.wow.tools`).
pub const DEFAULT_MIRROR: &str = "https://archive.wow.tools";

/// `product config` `shared_container_default_subfolder` fallback.
pub const DEFAULT_SUBFOLDER: &str = "_classic_";

/// One row of Blizzard's `cdns` table.
#[derive(Debug, Clone, Copy)]
pub struct RegionCdn {
    pub name: &'static str,
    pub hosts: &'static str,
    pub servers: &'static str,
}

/// Regions in Blizzard's `versions`/`cdns` order.
pub const REGIONS: [RegionCdn; 5] = [
    RegionCdn {
        name: "us",
        hosts: "level3.blizzard.com us.cdn.blizzard.com",
        servers: "http://level3.blizzard.com/?maxhosts=8 http://us.cdn.blizzard.com/?maxhosts=4&fallback=1 https://level3.ssl.blizzard.com/?maxhosts=4&fallback=1 https://us.cdn.blizzard.com/?maxhosts=4&fallback=1",
    },
    RegionCdn {
        name: "eu",
        hosts: "level3.blizzard.com",
        servers: "http://level3.blizzard.com/?maxhosts=8 https://level3.ssl.blizzard.com/?maxhosts=8&fallback=1",
    },
    RegionCdn {
        name: "cn",
        hosts: "blzdist-wow.necdn.leihuo.netease.com",
        servers: "http://blzdist-wow.necdn.leihuo.netease.com/?maxhosts=6 https://blzdist-wow.necdn.leihuo.netease.com/?maxhosts=4&fallback=1",
    },
    RegionCdn {
        name: "kr",
        hosts: "level3.blizzard.com kr.cdn.blizzard.com blizzard.gcdn.cloudn.co.kr",
        servers: "http://blizzard.gcdn.cloudn.co.kr/?fallback=1 http://kr.cdn.blizzard.com/?maxhosts=2 http://level3.blizzard.com/?maxhosts=4 https://blizzard.gcdn.cloudn.co.kr/?maxhosts=4&fallback=1 https://kr.cdn.blizzard.com/?maxhosts=4&fallback=1 https://level3.ssl.blizzard.com/?maxhosts=4&fallback=1",
    },
    RegionCdn {
        name: "tw",
        hosts: "level3.blizzard.com us.cdn.blizzard.com",
        servers: "http://level3.blizzard.com/?maxhosts=8 http://us.cdn.blizzard.com/?maxhosts=4&fallback=1 https://level3.ssl.blizzard.com/?maxhosts=4&fallback=1 https://us.cdn.blizzard.com/?maxhosts=4&fallback=1",
    },
];

/// Region row by (case-insensitive) name.
pub fn region_cdn(name: &str) -> Option<&'static RegionCdn> {
    REGIONS.iter().find(|r| r.name.eq_ignore_ascii_case(name))
}

/// `shared_container_default_subfolder` from the product config JSON, found
/// by a plain text search (the file is one JSON object; no JSON parser needed).
pub fn subfolder_from_product_config(text: &str) -> Option<String> {
    let key = "\"shared_container_default_subfolder\"";
    let rest = &text[text.find(key)? + key.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let value = &rest[..rest.find('"')?];
    let valid = !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    valid.then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subfolder_is_extracted() {
        let json = r#"{"all":{"config":{"data_dir":"Data/","shared_container_default_subfolder": "_classic_","x":1}}}"#;
        assert_eq!(
            subfolder_from_product_config(json).as_deref(),
            Some("_classic_")
        );
        assert_eq!(subfolder_from_product_config("{}"), None);
        assert_eq!(
            subfolder_from_product_config(r#"{"shared_container_default_subfolder":"../x"}"#),
            None
        );
    }

    #[test]
    fn regions() {
        assert_eq!(region_cdn("EU").unwrap().hosts, "level3.blizzard.com");
        assert!(region_cdn("xx").is_none());
    }
}
