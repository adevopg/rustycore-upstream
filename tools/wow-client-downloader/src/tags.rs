//! Tag selection over install/download manifests, and the Agent's `Tags`
//! string (`.build.info`).
//!
//! The Agent stores its selection as tag sets separated by `:`, each a
//! space-separated list where a trailing `?` marks an optional tag (e.g.
//! `Windows x86_64 EU? esES speech?:Windows x86_64 EU? esES text?`). blizzget
//! (`ProgramData::downloadMask`) implements the same model: every tag set
//! yields `AND` of its tags' masks and the result is the `OR` of the sets.
//!
//! Here each set is evaluated per tag *type*: an entry matches a set when, for
//! every tag type that has tags in the set, it carries at least one of them
//! (`OR` inside a type, `AND` across types); the selection is the `OR` of the
//! sets. With at most one tag per type in a set (what the Agent and blizzget's
//! combo boxes produce) this is exactly blizzget's `AND`. Names that are not
//! tags of the manifest (`acct-ESP?`, `geoip-ES?`) are ignored.

use crate::manifest::Tag;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagQuery {
    pub sets: Vec<Vec<String>>,
}

impl TagQuery {
    /// Parses an Agent `Tags` string.
    pub fn parse_agent(text: &str) -> Self {
        Self {
            sets: text
                .split(':')
                .map(|set| {
                    set.split_whitespace()
                        .map(|t| t.trim_end_matches('?').to_owned())
                        .filter(|t| !t.is_empty())
                        .collect()
                })
                .filter(|s: &Vec<String>| !s.is_empty())
                .collect(),
        }
    }

    /// Selection bitmap over `entries` entries of a manifest with `tags`.
    pub fn select(&self, tags: &[Tag], entries: usize) -> Vec<bool> {
        let mut selected = vec![false; entries];
        for set in &self.sets {
            // Group this set's known tags by type.
            let mut kinds: Vec<(u16, Vec<&Tag>)> = Vec::new();
            for name in set {
                let Some(tag) = tags.iter().find(|t| &t.name == name) else {
                    continue;
                };
                match kinds.iter_mut().find(|(k, _)| *k == tag.kind) {
                    Some((_, list)) => list.push(tag),
                    None => kinds.push((tag.kind, vec![tag])),
                }
            }
            for (i, sel) in selected.iter_mut().enumerate() {
                if !*sel && kinds.iter().all(|(_, list)| list.iter().any(|t| t.has(i))) {
                    *sel = true;
                }
            }
        }
        selected
    }
}

/// Whether any entry carries every one of `names` (used to reject platform
/// and architecture combinations the build does not ship).
pub fn any_entry_with_all(tags: &[Tag], entries: usize, names: &[&str]) -> bool {
    let wanted: Option<Vec<&Tag>> = names
        .iter()
        .map(|n| tags.iter().find(|t| t.name == *n))
        .collect();
    wanted.is_some_and(|w| (0..entries).any(|i| w.iter().all(|t| t.has(i))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::build::tag;

    fn tags() -> Vec<Tag> {
        // entries: 0 win x64 all-locales text+speech, 1 win x64 deDE speech,
        // 2 win x64 esES text, 3 OSX x64, 4 win arm64, 5 win x64 deDE text
        vec![
            tag("Windows", 1, 6, &[0, 1, 2, 4, 5]),
            tag("OSX", 1, 6, &[3]),
            tag("arm64", 2, 6, &[4]),
            tag("x86_64", 2, 6, &[0, 1, 2, 3, 5]),
            tag("deDE", 3, 6, &[0, 1, 3, 4, 5]),
            tag("esES", 3, 6, &[0, 2, 3, 4]),
            tag("EU", 4, 6, &[0, 1, 2, 3, 4, 5]),
            tag("speech", 5, 6, &[0, 1, 3, 4]),
            tag("text", 5, 6, &[0, 2, 3, 4, 5]),
        ]
    }

    #[test]
    fn agent_string_semantics() {
        let q = TagQuery::parse_agent(
            "Windows x86_64 EU? acct-ESP? geoip-ES? esES speech?:Windows x86_64 EU? esES text?",
        );
        assert_eq!(q.sets.len(), 2);
        assert_eq!(q.sets[0][2], "EU");
        let sel = q.select(&tags(), 6);
        assert_eq!(sel, [true, false, true, false, false, false]);

        // Different speech and text locales: (deDE AND speech) OR (esES AND text).
        let q = TagQuery::parse_agent("Windows x86_64 deDE speech:Windows x86_64 esES text");
        assert_eq!(
            q.select(&tags(), 6),
            [true, true, true, false, false, false],
            "deDE text (entry 5) is not selected"
        );
    }

    #[test]
    fn or_within_a_type() {
        let q = TagQuery::parse_agent("Windows x86_64 arm64 deDE");
        assert_eq!(q.select(&tags(), 6), [true, true, false, false, true, true]);
        // No known tags: everything matches (blizzget's all-ones mask).
        assert!(
            TagQuery::parse_agent("foo")
                .select(&tags(), 6)
                .iter()
                .all(|&s| s)
        );
        assert!(
            TagQuery::parse_agent("")
                .select(&tags(), 6)
                .iter()
                .all(|&s| !s)
        );
    }

    #[test]
    fn platform_combinations() {
        let t = tags();
        assert!(any_entry_with_all(&t, 6, &["Windows", "arm64"]));
        assert!(!any_entry_with_all(&t, 6, &["OSX", "arm64"]));
        assert!(!any_entry_with_all(&t, 6, &["Linux"]));
    }
}
