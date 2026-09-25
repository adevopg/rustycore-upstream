//! What to download for a selection: manifests + tags -> the set of `EKeys`
//! to store and the loose install files, with their sizes.
//!
//! Follows blizzget `DownloadTask::run`: the build's own manifests
//! (ENCODING, install, download) are stored, then every download-manifest
//! entry selected by the tags, then the install-manifest files (loose game
//! binaries) selected by the same tags. Additionally stored: the ROOT, `size`,
//! `patch-index` and every `vfs-*` manifest named by the build config, which
//! the Agent keeps in its local storage as well.

use std::collections::HashSet;

use anyhow::{Context, Result, bail};

use crate::cli::Selection;
use crate::config::ConfigFile;
use crate::encoding::Encoding;
use crate::manifest::{DownloadManifest, InstallEntry, InstallManifest};
use crate::tags::{TagQuery, any_entry_with_all};
use crate::util::{Key, human_size};

/// Build config variables whose files are stored besides the tagged content.
const CORE_NAMES: &[&str] = &[
    "encoding",
    "root",
    "install",
    "download",
    "size",
    "patch-index",
];

/// The decoded manifests of the build.
pub struct Manifests {
    pub build_config: ConfigFile,
    pub cdn_config: ConfigFile,
    pub encoding: Encoding,
    pub install: InstallManifest,
    pub download: DownloadManifest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Core,
    Install,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Needed {
    pub ekey: Key,
    /// Encoded size (0 if unknown).
    pub size: u64,
    pub kind: Kind,
}

#[derive(Debug, Clone)]
pub struct Plan {
    /// Unique `EKeys`: core first, then install files, then download entries
    /// in manifest (priority) order.
    pub needed: Vec<Needed>,
    pub install_files: Vec<(InstallEntry, Key)>,
    pub download_selected: usize,
}

impl Plan {
    pub fn total_bytes(&self) -> u64 {
        self.needed.iter().map(|n| n.size).sum()
    }

    pub fn summary(&self) -> String {
        let count = |k| self.needed.iter().filter(|n| n.kind == k).count();
        let bytes = |k| {
            self.needed
                .iter()
                .filter(|n| n.kind == k)
                .map(|n| n.size)
                .sum::<u64>()
        };
        let install_bytes: u64 = self
            .install_files
            .iter()
            .map(|(e, _)| u64::from(e.size))
            .sum();
        format!(
            "{} files, {} to download ({} download-manifest entries selected, {} build manifests {}, {} extra install blobs {}); {} loose install files ({} decoded)",
            self.needed.len(),
            human_size(self.total_bytes()),
            self.download_selected,
            count(Kind::Core),
            human_size(bytes(Kind::Core)),
            count(Kind::Install),
            human_size(bytes(Kind::Install)),
            self.install_files.len(),
            human_size(install_bytes),
        )
    }
}

/// `EKeys` (and sizes) of the build's own manifests.
pub fn core_files(build_config: &ConfigFile, encoding: &Encoding) -> Vec<(String, Key, u64)> {
    let mut out = Vec::new();
    let names = build_config
        .names()
        .filter(|n| CORE_NAMES.contains(n) || (n.starts_with("vfs-") && !n.ends_with("-size")));
    for name in names {
        let ckey = build_config.key(name, 0);
        let ekey = build_config
            .key(name, 1)
            .or_else(|| ckey.and_then(|c| encoding.find(&c)).map(|e| e.ekey));
        let Some(ekey) = ekey else { continue };
        let size = build_config
            .values(&format!("{name}-size"))
            .and_then(|v| v.get(1))
            .and_then(|s| s.parse().ok())
            .or_else(|| encoding.encoded_size(&ekey))
            .unwrap_or(0);
        out.push((name.to_owned(), ekey, size));
    }
    out
}

/// Rejects platform/architecture pairs the build does not ship (no
/// install-manifest entry carries both tags).
pub fn check_platform(install: &InstallManifest, selection: &Selection) -> Result<()> {
    let (os, arch) = (selection.os.tag(), selection.arch.tag());
    if !any_entry_with_all(&install.tags, install.entries.len(), &[os, arch]) {
        let hint = if os == "OSX" {
            " (the macOS client is a universal binary: use --arch x86_64, it runs natively on Apple Silicon)"
        } else {
            ""
        };
        bail!(
            "build {} ships no {} {} client{hint}",
            crate::product::VERSIONS_NAME,
            selection.os.name(),
            selection.arch.name()
        );
    }
    Ok(())
}

pub fn build(m: &Manifests, selection: &Selection) -> Result<Plan> {
    check_platform(&m.install, selection)?;
    let query = TagQuery::parse_agent(&selection.agent_tags());
    let mut seen = HashSet::new();
    let mut needed = Vec::new();
    let mut push = |ekey: Key, size: u64, kind: Kind| {
        if seen.insert(ekey) {
            needed.push(Needed { ekey, size, kind });
        }
    };
    for (_, ekey, size) in core_files(&m.build_config, &m.encoding) {
        push(ekey, size, Kind::Core);
    }

    let dl_selected = query.select(&m.download.tags, m.download.entries.len());
    let dl_sizes: std::collections::HashMap<Key, u64> = m
        .download
        .entries
        .iter()
        .map(|e| (e.ekey, e.size))
        .collect();

    let in_selected = query.select(&m.install.tags, m.install.entries.len());
    let mut install_files = Vec::new();
    for (entry, _) in m
        .install
        .entries
        .iter()
        .zip(&in_selected)
        .filter(|(_, s)| **s)
    {
        let enc = m
            .encoding
            .find(&entry.ckey)
            .with_context(|| format!("install file {} is not in ENCODING", entry.name))?;
        let size = dl_sizes
            .get(&enc.ekey)
            .copied()
            .or_else(|| m.encoding.encoded_size(&enc.ekey))
            .unwrap_or(0);
        push(enc.ekey, size, Kind::Install);
        install_files.push((entry.clone(), enc.ekey));
    }

    let mut download_selected = 0;
    for (entry, _) in m
        .download
        .entries
        .iter()
        .zip(&dl_selected)
        .filter(|(_, s)| **s)
    {
        download_selected += 1;
        push(entry.ekey, entry.size, Kind::Download);
    }
    Ok(Plan {
        needed,
        install_files,
        download_selected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Arch, Os};
    use crate::manifest::build::{download, install, tag};
    use crate::util::{hex, md5};

    fn manifests() -> Manifests {
        // install: 0 WowClassic.exe (Windows x86_64), 1 mac binary (OSX x86_64)
        let exe = md5(b"exe");
        let mac = md5(b"mac");
        let install_tags = vec![
            tag("OSX", 1, 2, &[1]),
            tag("Windows", 1, 2, &[0]),
            tag("x86_64", 2, 2, &[0, 1]),
            tag("esES", 3, 2, &[0, 1]),
            tag("enUS", 3, 2, &[0, 1]),
            tag("EU", 4, 2, &[0, 1]),
            tag("speech", 5, 2, &[0, 1]),
            tag("text", 5, 2, &[0, 1]),
        ];
        let inst = install(
            &install_tags,
            &[("WowClassic.exe", exe, 3), ("Mac\\bin", mac, 3)],
        );
        // download: 0 common, 1 esES speech, 2 enUS text, 3 exe blob
        let dl_tags = vec![
            tag("Windows", 1, 4, &[0, 1, 2, 3]),
            tag("x86_64", 2, 4, &[0, 1, 2, 3]),
            tag("esES", 3, 4, &[0, 1, 3]),
            tag("enUS", 3, 4, &[0, 2, 3]),
            tag("EU", 4, 4, &[0, 1, 2, 3]),
            tag("speech", 5, 4, &[0, 1, 3]),
            tag("text", 5, 4, &[0, 2, 3]),
        ];
        let dl = download(
            &dl_tags,
            &[
                ([1; 16], 100),
                ([2; 16], 200),
                ([3; 16], 300),
                ([0xE0; 16], 40),
            ],
        );
        let mut enc = vec![
            (exe, [0xE0; 16], 3, 40),
            (mac, [0xE1; 16], 3, 41),
            ([0x0A; 16], [0xA0; 16], 5, 50),
        ];
        enc.sort_unstable();
        let build_config = format!(
            "root = {}\ninstall = {} {}\ninstall-size = 1 11\nencoding = {} {}\nencoding-size = 9 99\nvfs-1 = {} {}\nvfs-1-size = 1 7\n",
            hex(&[0x0A; 16]),
            hex(&[0x0B; 16]),
            hex(&[0xB0; 16]),
            hex(&[0x0C; 16]),
            hex(&[0xC0; 16]),
            hex(&[0x0D; 16]),
            hex(&[0xD0; 16])
        );
        Manifests {
            build_config: ConfigFile::parse(build_config.as_bytes()).unwrap(),
            cdn_config: ConfigFile::default(),
            encoding: Encoding::parse(&crate::encoding::build(&enc, 10)).unwrap(),
            install: InstallManifest::parse(&inst).unwrap(),
            download: DownloadManifest::parse(&dl).unwrap(),
        }
    }

    fn selection(os: Os, locale: &str) -> Selection {
        Selection::new(os, Arch::X86_64, locale, None, None, Some("EU")).unwrap()
    }

    #[test]
    fn windows_eses_plan() {
        let plan = build(&manifests(), &selection(Os::Windows, "esES")).unwrap();
        let keys: Vec<u8> = plan.needed.iter().map(|n| n.ekey[0]).collect();
        // core: root (via ENCODING), install, encoding, vfs-1; install exe;
        // download: common + esES speech (exe blob already counted).
        assert_eq!(keys, [0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 1, 2]);
        assert_eq!(plan.needed[0].size, 50);
        assert_eq!(plan.needed[1].size, 11);
        assert_eq!(
            plan.needed[4].size, 40,
            "install blob size from the download manifest"
        );
        assert_eq!(plan.download_selected, 3);
        assert_eq!(plan.install_files.len(), 1);
        assert_eq!(plan.total_bytes(), 50 + 11 + 99 + 7 + 40 + 100 + 200);
        assert!(plan.summary().contains("7 files"));
    }

    #[test]
    fn macos_arm64_is_rejected() {
        let m = manifests();
        let sel = Selection::new(Os::MacOs, Arch::Arm64, "enUS", None, None, None).unwrap();
        let err = build(&m, &sel).unwrap_err().to_string();
        assert!(err.contains("universal binary"), "{err}");
        let plan = build(&m, &selection(Os::MacOs, "enUS")).unwrap();
        assert_eq!(plan.install_files[0].0.name, "Mac\\bin");
    }
}
