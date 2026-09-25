//! The NGDP client side: reads `versions`/`cdns` from a patch server (our
//! temporary one) and fetches configs, loose files and archive ranges from
//! the CDN host it advertises.
//!
//! Port of blizzget `NGDP::NGDP` (table parsing, region selection),
//! `NGDP::setRegion`/`geturl` (`http://<first host>/<path>/<type>/xx/yy/<hash>`)
//! and the manifest loading of `ProgramData::loadTags`/`DownloadTask::run`
//! (ENCODING from the build config's `EKey`, install/download via ENCODING).

use anyhow::{Context, Result, bail};

use crate::blte;
use crate::config::ConfigFile;
use crate::encoding::Encoding;
use crate::http::HttpClient;
use crate::manifest::{DownloadManifest, InstallManifest};
use crate::plan::Manifests;
use crate::product;
use crate::tables::Table;
use crate::util::{Key, cdn_path, hex, parse_key};

pub struct Remote {
    pub client: HttpClient,
    /// `http://<host>/<path>` of the CDN.
    pub cdn_base: String,
    /// `http://<host>/<config path>`.
    pub config_base: String,
    pub region: String,
    pub build_key: Key,
    pub cdn_key: Key,
    pub product_config: Option<Key>,
    pub version_name: String,
}

impl Remote {
    /// Reads `/<product>/versions` and `/cdns` from `patch_base` and selects
    /// `region` (lower-case branch name).
    pub fn connect(client: HttpClient, patch_base: &str, region: &str) -> Result<Self> {
        let get_table = |name: &str| -> Result<Table> {
            let url = format!("{patch_base}/{}/{name}", product::PRODUCT);
            let body = client.get(&url)?;
            Table::parse(&String::from_utf8_lossy(&body)).with_context(|| url.clone())
        };
        let cdns = get_table("cdns")?;
        let versions = get_table("versions")?;
        let crow = cdns
            .find_row("Name", region)
            .with_context(|| format!("cdns has no region {region}"))?;
        let vrow = versions
            .find_row("Region", region)
            .with_context(|| format!("versions has no region {region}"))?;
        let host = cdns
            .get(crow, "Hosts")
            .and_then(|h| h.split_whitespace().next())
            .context("cdns row has no host")?
            .to_owned();
        let path = cdns.get(crow, "Path").context("cdns row has no Path")?;
        let config_path = cdns.get(crow, "ConfigPath").unwrap_or(product::CONFIG_PATH);
        let key = |col: &str| {
            versions
                .get(vrow, col)
                .and_then(parse_key)
                .with_context(|| format!("versions: bad {col}"))
        };
        Ok(Self {
            cdn_base: format!("http://{host}/{path}"),
            config_base: format!("http://{host}/{config_path}"),
            region: region.to_owned(),
            build_key: key("BuildConfig")?,
            cdn_key: key("CDNConfig")?,
            product_config: versions.get(vrow, "ProductConfig").and_then(parse_key),
            version_name: versions
                .get(vrow, "VersionsName")
                .unwrap_or_default()
                .to_owned(),
            client,
        })
    }

    pub fn url(&self, kind: &str, key: &Key) -> String {
        format!("{}/{}", self.cdn_base, cdn_path(kind, &hex(key)))
    }

    /// `config/xx/yy/<key>`, MD5-checked.
    pub fn config(&self, key: &Key) -> Result<Vec<u8>> {
        let data = self.client.get(&self.url("config", key))?;
        if &crate::util::md5(&data) != key {
            bail!("config {} does not match its MD5", hex(key));
        }
        Ok(data)
    }

    /// Product config JSON (`tpr/configs/data/xx/yy/<key>`), if reachable.
    pub fn product_config_text(&self) -> Option<String> {
        let key = hex(&self.product_config?);
        let url = format!("{}/{}/{}/{key}", self.config_base, &key[0..2], &key[2..4]);
        let data = self.client.get_opt(&url).ok()??;
        Some(String::from_utf8_lossy(&data).into_owned())
    }

    /// A loose `data/xx/yy/<ekey>` blob, verified against its `EKey`.
    pub fn loose(&self, ekey: &Key) -> Result<Option<Vec<u8>>> {
        let Some(blob) = self.client.get_opt(&self.url("data", ekey))? else {
            return Ok(None);
        };
        blte::verify(&blob, ekey).with_context(|| format!("loose file {}", hex(ekey)))?;
        Ok(Some(blob))
    }

    /// `data/xx/yy/<archive>.index`.
    pub fn archive_index(&self, archive: &Key) -> Result<Vec<u8>> {
        self.client
            .get(&format!("{}.index", self.url("data", archive)))
    }

    /// Bytes `start..=end` of an archive.
    pub fn archive_range(&self, archive: &Key, start: u64, end: u64) -> Result<Vec<u8>> {
        self.client
            .get_range(&self.url("data", archive), start, end)
    }
}

/// Source of already-fetched blobs (the local storage when resuming).
pub trait BlobSource {
    fn blob(&self, ekey: &Key) -> Option<Vec<u8>>;
}

/// Fetches, verifies and decodes the build's manifests. `local` is consulted
/// before the CDN.
pub fn load_manifests(
    remote: &Remote,
    local: Option<&dyn BlobSource>,
    fetched: &mut Vec<(Key, Vec<u8>)>,
) -> Result<Manifests> {
    let build_config =
        ConfigFile::parse_verified(&remote.config(&remote.build_key)?, &remote.build_key)?;
    let cdn_config = ConfigFile::parse_verified(&remote.config(&remote.cdn_key)?, &remote.cdn_key)?;
    let mut get = |ekey: &Key, what: &str| -> Result<Vec<u8>> {
        if let Some(blob) = local.and_then(|l| l.blob(ekey))
            && blte::verify(&blob, ekey).is_ok()
        {
            return blte::decode(&blob);
        }
        let blob = remote
            .loose(ekey)?
            .with_context(|| format!("{what} ({}) is not on the CDN", hex(ekey)))?;
        let data = blte::decode(&blob).with_context(|| what.to_owned())?;
        fetched.push((*ekey, blob));
        Ok(data)
    };
    let enc_ekey = build_config
        .key("encoding", 1)
        .context("build config lacks the ENCODING EKey")?;
    let encoding = Encoding::parse(&get(&enc_ekey, "ENCODING")?)?;
    let manifest_ekey = |name: &str| -> Result<Key> {
        if let Some(k) = build_config.key(name, 1) {
            return Ok(k);
        }
        let ckey = build_config
            .key(name, 0)
            .with_context(|| format!("build config lacks `{name}`"))?;
        Ok(encoding
            .find(&ckey)
            .with_context(|| format!("{name} CKey is not in ENCODING"))?
            .ekey)
    };
    let install = InstallManifest::parse(&get(&manifest_ekey("install")?, "install manifest")?)?;
    let download =
        DownloadManifest::parse(&get(&manifest_ekey("download")?, "download manifest")?)?;
    Ok(Manifests {
        build_config,
        cdn_config,
        encoding,
        install,
        download,
    })
}
