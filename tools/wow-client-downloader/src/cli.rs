//! Command line parsing (hand-rolled like the repo's other tools) and the
//! user's platform/locale/region selection.
//!
//! Only Blizzard's desktop clients are accepted: Windows (`Windows` tag) and
//! macOS (`OSX` tag) on `x86_64`/`arm64`; whether a pair is really shipped is
//! checked against the install manifest ([`crate::plan::check_platform`]).
//! The region defaults from the locale the way the Agent groups them.

use std::net::IpAddr;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::product;

/// Locales of the 54261 manifests (tag type 3).
pub const LOCALES: [&str; 10] = [
    "deDE", "enUS", "esES", "esMX", "frFR", "koKR", "ptBR", "ruRU", "zhCN", "zhTW",
];
/// Regions of the manifests (tag type 4).
pub const REGIONS: [&str; 5] = ["CN", "EU", "KR", "TW", "US"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
}

impl Os {
    pub fn tag(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::MacOs => "OSX",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::MacOs => "macos",
        }
    }
    fn parse(text: &str) -> Result<Self> {
        match text.to_ascii_lowercase().as_str() {
            "windows" | "win" | "win64" => Ok(Self::Windows),
            "macos" | "mac" | "osx" => Ok(Self::MacOs),
            "android" | "ios" | "web" => bail!(
                "--os {text}: the manifests tag {text} content, but Blizzard ships no {text} client for this build; use windows or macos"
            ),
            _ => bail!("--os {text}: expected windows or macos"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Arm64,
}

impl Arch {
    pub fn tag(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Arm64 => "arm64",
        }
    }
    pub fn name(self) -> &'static str {
        self.tag()
    }
    fn parse(text: &str) -> Result<Self> {
        match text.to_ascii_lowercase().as_str() {
            "x86_64" | "x64" | "amd64" => Ok(Self::X86_64),
            "arm64" | "aarch64" => Ok(Self::Arm64),
            "x86_32" | "x86" | "i386" | "i686" | "win32" => {
                bail!("--arch {text}: 32-bit clients are no longer shipped; use x86_64 or arm64")
            }
            _ => bail!("--arch {text}: expected x86_64 or arm64"),
        }
    }
}

fn locale_tag(text: &str) -> Result<&'static str> {
    LOCALES
        .iter()
        .find(|l| l.eq_ignore_ascii_case(text))
        .copied()
        .with_context(|| format!("unknown locale {text}; available: {}", LOCALES.join(" ")))
}

/// Region the Agent uses for a locale.
pub fn default_region(locale: &str) -> &'static str {
    match locale {
        "enUS" | "esMX" | "ptBR" => "US",
        "koKR" => "KR",
        "zhTW" => "TW",
        "zhCN" => "CN",
        _ => "EU",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub os: Os,
    pub arch: Arch,
    pub speech: &'static str,
    pub text: &'static str,
    pub region: &'static str,
}

impl Selection {
    pub fn new(
        os: Os,
        arch: Arch,
        locale: &str,
        speech: Option<&str>,
        text: Option<&str>,
        region: Option<&str>,
    ) -> Result<Self> {
        let locale = locale_tag(locale)?;
        let region = match region {
            Some(r) => REGIONS
                .iter()
                .find(|x| x.eq_ignore_ascii_case(r))
                .copied()
                .with_context(|| format!("unknown region {r}; available: {}", REGIONS.join(" ")))?,
            None => default_region(locale),
        };
        Ok(Self {
            os,
            arch,
            speech: speech.map_or(Ok(locale), locale_tag)?,
            text: text.map_or(Ok(locale), locale_tag)?,
            region,
        })
    }

    /// The Agent's `.build.info` `Tags` value for this selection.
    pub fn agent_tags(&self) -> String {
        let (os, arch, region) = (self.os.tag(), self.arch.tag(), self.region);
        format!(
            "{os} {arch} {region}? {} speech?:{os} {arch} {region}? {} text?",
            self.speech, self.text
        )
    }

    /// `.build.info` branch (lower-case region).
    pub fn branch(&self) -> String {
        self.region.to_ascii_lowercase()
    }
}

#[derive(Debug, Clone)]
pub struct NetOptions {
    pub port: u16,
    pub mirror: Option<String>,
    pub cache: Option<PathBuf>,
    pub threads: usize,
}

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub output: PathBuf,
    pub selection: Selection,
    pub net: NetOptions,
    pub dry_run: bool,
    /// Hidden test options: cap the number/bytes of tagged files and force
    /// specific `FileDataIds` in.
    pub limit_files: Option<usize>,
    pub limit_bytes: Option<u64>,
    pub include_fdids: Vec<u32>,
    /// Only rewrite `.idx` files and complete `Data/indices` of an existing
    /// install (`data.###` untouched).
    pub indices_only: bool,
}

#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub bind: IpAddr,
    pub port: u16,
    pub mirror: Option<String>,
    pub cache: Option<PathBuf>,
}

#[derive(Debug)]
pub enum Command {
    Download(DownloadOptions),
    Serve(ServeOptions),
    List(NetOptions),
    Help,
}

pub const USAGE: &str = "\
wow-client-downloader - download the WoW Classic 3.4.3.54261 client (wow_classic)

USAGE:
  wow-client-downloader download --output <dir> [options]
  wow-client-downloader list [--port P] [--mirror URL | --no-mirror] [--cache <dir>]
  wow-client-downloader serve [--port P] [--bind 127.0.0.1] [--mirror URL | --no-mirror] [--cache <dir>]

DOWNLOAD OPTIONS:
  --output <dir>      install directory (gets .build.info, Data/, _classic_/)
  --os windows|macos  client platform (default windows)
  --arch x86_64|arm64 client architecture (default x86_64)
  --locale <xxYY>     game locale (default enUS): deDE enUS esES esMX frFR koKR ptBR ruRU zhCN zhTW
  --speech <xxYY>     speech locale (default: --locale)
  --text <xxYY>       text locale (default: --locale)
  --region EU|US|KR|TW|CN  region (default derived from the locale)
  --threads N         parallel CDN connections (default 4)
  --dry-run           resolve manifests and print file count and size; write nothing
  --indices-only      existing install: rewrite the .idx files and fetch/build missing
                      Data/indices (archives, patch archives, groups, file indices);
                      data.### are not touched

COMMON OPTIONS:
  --port P            port of the temporary local server (default: random for download/list, 1119 for serve)
  --mirror URL        fallback mirror (default https://archive.wow.tools)
  --no-mirror         only use Blizzard's CDN
  --cache <dir>       on-disk cache of complete CDN files served by the local proxy
";

struct Args {
    items: Vec<String>,
    pos: usize,
}

impl Args {
    fn value(&mut self, flag: &str) -> Result<String> {
        self.pos += 1;
        self.items
            .get(self.pos)
            .cloned()
            .with_context(|| format!("{flag} needs a value"))
    }
}

fn parse_num<T: std::str::FromStr>(flag: &str, text: &str) -> Result<T> {
    text.parse()
        .map_err(|_| anyhow::anyhow!("{flag}: invalid number {text:?}"))
}

pub fn parse(argv: &[String]) -> Result<Command> {
    let Some(command) = argv.first() else {
        return Ok(Command::Help);
    };
    let mut cursor = Args {
        items: argv.to_vec(),
        pos: 0,
    };
    let mut output = None;
    let (mut os, mut arch) = (Os::Windows, Arch::X86_64);
    let mut locale = "enUS".to_owned();
    let (mut speech, mut text, mut region) = (None, None, None);
    let mut threads = 4usize;
    let mut port: Option<u16> = None;
    let mut bind: IpAddr = [127, 0, 0, 1].into();
    let mut mirror = Some(product::DEFAULT_MIRROR.to_owned());
    let mut cache = None;
    let mut dry_run = false;
    let mut indices_only = false;
    let (mut limit_files, mut limit_bytes, mut include_fdids) = (None, None, Vec::new());

    while cursor.pos + 1 < cursor.items.len() {
        cursor.pos += 1;
        let flag = cursor.items[cursor.pos].clone();
        match flag.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--output" | "-o" => output = Some(PathBuf::from(cursor.value(&flag)?)),
            "--os" => os = Os::parse(&cursor.value(&flag)?)?,
            "--arch" => arch = Arch::parse(&cursor.value(&flag)?)?,
            "--locale" => locale = cursor.value(&flag)?,
            "--speech" => speech = Some(cursor.value(&flag)?),
            "--text" => text = Some(cursor.value(&flag)?),
            "--region" => region = Some(cursor.value(&flag)?),
            "--threads" => threads = parse_num(&flag, &cursor.value(&flag)?)?,
            "--port" => port = Some(parse_num(&flag, &cursor.value(&flag)?)?),
            "--bind" => bind = parse_num(&flag, &cursor.value(&flag)?)?,
            "--mirror" => mirror = Some(cursor.value(&flag)?.trim_end_matches('/').to_owned()),
            "--no-mirror" => mirror = None,
            "--cache" => cache = Some(PathBuf::from(cursor.value(&flag)?)),
            "--dry-run" => dry_run = true,
            "--indices-only" => indices_only = true,
            "--limit-files" => limit_files = Some(parse_num(&flag, &cursor.value(&flag)?)?),
            "--limit-bytes" => limit_bytes = Some(parse_num(&flag, &cursor.value(&flag)?)?),
            "--include-fdid" => {
                for id in cursor.value(&flag)?.split(',') {
                    include_fdids.push(parse_num(&flag, id.trim())?);
                }
            }
            other => bail!("unknown option {other}\n\n{USAGE}"),
        }
    }
    if !(1..=32).contains(&threads) {
        bail!("--threads must be between 1 and 32");
    }
    let net = NetOptions {
        port: port.unwrap_or(0),
        mirror: mirror.clone(),
        cache: cache.clone(),
        threads,
    };
    match command.as_str() {
        "download" => Ok(Command::Download(DownloadOptions {
            output: output.context("download needs --output <dir>")?,
            selection: Selection::new(
                os,
                arch,
                &locale,
                speech.as_deref(),
                text.as_deref(),
                region.as_deref(),
            )?,
            net,
            dry_run,
            limit_files,
            limit_bytes,
            include_fdids,
            indices_only,
        })),
        "list" => Ok(Command::List(net)),
        "serve" => Ok(Command::Serve(ServeOptions {
            bind,
            port: port.unwrap_or(1119),
            mirror,
            cache,
        })),
        "help" | "-h" | "--help" => Ok(Command::Help),
        other => bail!("unknown command {other}\n\n{USAGE}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn download_defaults_and_tags() {
        let Command::Download(o) = parse(&argv("download --output /tmp/x --locale eses")).unwrap()
        else {
            panic!()
        };
        assert_eq!(o.selection.region, "EU");
        assert_eq!(
            o.selection.agent_tags(),
            "Windows x86_64 EU? esES speech?:Windows x86_64 EU? esES text?"
        );
        assert_eq!(o.selection.branch(), "eu");
        assert_eq!(o.net.threads, 4);
        assert_eq!(o.net.mirror.as_deref(), Some("https://archive.wow.tools"));
        assert!(!o.dry_run);
    }

    #[test]
    fn download_options() {
        let Command::Download(o) = parse(&argv(
            "download -o d --os macos --arch arm64 --locale deDE --text enUS --region US --threads 8 --no-mirror --dry-run --limit-files 5 --include-fdid 1349477,1",
        ))
        .unwrap() else {
            panic!()
        };
        assert_eq!(o.selection.os, Os::MacOs);
        assert_eq!(o.selection.arch, Arch::Arm64);
        assert_eq!((o.selection.speech, o.selection.text), ("deDE", "enUS"));
        assert_eq!(o.selection.region, "US");
        assert_eq!(
            o.selection.agent_tags(),
            "OSX arm64 US? deDE speech?:OSX arm64 US? enUS text?"
        );
        assert!(o.net.mirror.is_none() && o.dry_run);
        assert_eq!(o.limit_files, Some(5));
        assert_eq!(o.include_fdids, [1_349_477, 1]);
        assert!(!o.indices_only);
        let Command::Download(r) = parse(&argv("download -o d --indices-only")).unwrap() else {
            panic!()
        };
        assert!(r.indices_only);
    }

    #[test]
    fn rejections() {
        for bad in [
            "download --os android -o x",
            "download --os ios -o x",
            "download --os web -o x",
            "download --arch x86_32 -o x",
            "download --locale xxYY -o x",
            "download --region MARS -o x",
            "download",
            "download -o x --threads 0",
            "frobnicate",
        ] {
            assert!(parse(&argv(bad)).is_err(), "{bad}");
        }
        let err = parse(&argv("download --os android -o x"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("no android client"), "{err}");
    }

    #[test]
    fn region_defaults() {
        let r = |l| default_region(l);
        assert_eq!(
            [
                r("enUS"),
                r("esMX"),
                r("ptBR"),
                r("deDE"),
                r("frFR"),
                r("ruRU")
            ],
            ["US", "US", "US", "EU", "EU", "EU"]
        );
        assert_eq!([r("koKR"), r("zhTW"), r("zhCN")], ["KR", "TW", "CN"]);
    }

    #[test]
    fn serve_and_list() {
        let Command::Serve(s) = parse(&argv("serve --bind 0.0.0.0 --cache c")).unwrap() else {
            panic!()
        };
        assert_eq!(s.port, 1119);
        assert!(s.bind.is_unspecified());
        assert!(matches!(parse(&argv("list --port 5")).unwrap(), Command::List(n) if n.port == 5));
        assert!(matches!(parse(&[]).unwrap(), Command::Help));
    }
}
