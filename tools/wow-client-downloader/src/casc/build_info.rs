//! `<install>/.build.info`, in the column layout the current Battle.net Agent
//! writes (see the real `World of Warcraft/.build.info`):
//!
//! `Branch!STRING:0|Active!DEC:1|Build Key!HEX:16|CDN Key!HEX:16|Install
//! Key!HEX:16|IM Size!DEC:4|CDN Path!STRING:0|CDN Hosts!STRING:0|CDN
//! Servers!STRING:0|Tags!STRING:0|Armadillo!STRING:0|Last
//! Activated!STRING:0|Version!STRING:0|KeyRing!HEX:16|Product!STRING:0`
//!
//! Like the Agent, `Install Key`, `IM Size`, `Armadillo`, `Last Activated` and
//! `KeyRing` are left empty. Rows of other products in an existing file with
//! the same header are kept (one install folder can hold several flavors).

pub const HEADER: &str = "Branch!STRING:0|Active!DEC:1|Build Key!HEX:16|CDN Key!HEX:16|Install Key!HEX:16|IM Size!DEC:4|CDN Path!STRING:0|CDN Hosts!STRING:0|CDN Servers!STRING:0|Tags!STRING:0|Armadillo!STRING:0|Last Activated!STRING:0|Version!STRING:0|KeyRing!HEX:16|Product!STRING:0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfoRow {
    pub branch: String,
    pub build_key: String,
    pub cdn_key: String,
    pub cdn_path: String,
    pub cdn_hosts: String,
    pub cdn_servers: String,
    pub tags: String,
    pub version: String,
    pub product: String,
}

impl BuildInfoRow {
    fn render(&self) -> String {
        [
            self.branch.as_str(),
            "1",
            &self.build_key,
            &self.cdn_key,
            "",
            "",
            &self.cdn_path,
            &self.cdn_hosts,
            &self.cdn_servers,
            &self.tags,
            "",
            "",
            &self.version,
            "",
            &self.product,
        ]
        .join("|")
    }
}

/// New file content: `existing` rows of other products (if the header
/// matches) followed by `row`.
pub fn render(existing: Option<&str>, row: &BuildInfoRow) -> String {
    let mut out = format!("{HEADER}\n");
    if let Some(text) = existing {
        let mut lines = text.lines();
        if lines.next().map(str::trim_end) == Some(HEADER) {
            for line in lines.filter(|l| !l.trim().is_empty()) {
                let product = line.rsplit('|').next().unwrap_or_default();
                if !product.eq_ignore_ascii_case(&row.product) {
                    out.push_str(line.trim_end());
                    out.push('\n');
                }
            }
        }
    }
    out.push_str(&row.render());
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> BuildInfoRow {
        BuildInfoRow {
            branch: "eu".into(),
            build_key: "c91609c69ed2ab39d44039390a1be969".into(),
            cdn_key: "a838aeb3cda2e027e9c96bd9953944b3".into(),
            cdn_path: "tpr/wow".into(),
            cdn_hosts: "level3.blizzard.com".into(),
            cdn_servers: "http://level3.blizzard.com/?maxhosts=8".into(),
            tags: "Windows x86_64 EU? esES speech?:Windows x86_64 EU? esES text?".into(),
            version: "3.4.3.54261".into(),
            product: "wow_classic".into(),
        }
    }

    #[test]
    fn renders_agent_layout() {
        let text = render(None, &row());
        assert_eq!(
            text.lines().nth(1).unwrap(),
            "eu|1|c91609c69ed2ab39d44039390a1be969|a838aeb3cda2e027e9c96bd9953944b3|||tpr/wow|level3.blizzard.com|http://level3.blizzard.com/?maxhosts=8|Windows x86_64 EU? esES speech?:Windows x86_64 EU? esES text?|||3.4.3.54261||wow_classic"
        );
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn keeps_other_products() {
        let existing = format!(
            "{HEADER}\nus|1|a|b|||p|h|s|t|||1.2.3||wow_classic_era\nus|1|a|b|||p|h|s|t|||0||wow_classic\n"
        );
        let text = render(Some(&existing), &row());
        assert_eq!(text.lines().count(), 3);
        assert!(text.contains("wow_classic_era"));
        assert_eq!(text.matches("|wow_classic\n").count(), 1);
        // A foreign header is replaced.
        assert_eq!(render(Some("A|B\n1|2\n"), &row()).lines().count(), 2);
    }
}
