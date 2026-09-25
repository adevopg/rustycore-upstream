//! `download` and `list` commands: the whole NGDP flow against the temporary
//! local server.
//!
//! Order (blizzget `DownloadTask::run`): versions/cdns -> build + CDN config
//! -> ENCODING -> install/download manifests -> tag selection -> archive
//! indices -> build manifests (ROOT, vfs, ...) -> tagged content -> local
//! indices + `.build.info` -> loose install files decoded into the product
//! subfolder.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::blte;
use crate::casc::build_info::{self, BuildInfoRow};
use crate::casc::{LocalStorage, write_atomic};
use crate::cli::{Arch, DownloadOptions, LOCALES, NetOptions, Os, Selection};
use crate::fetch::{self, Job};
use crate::http::HttpClient;
use crate::index_set::{self, IndexSet};
use crate::plan::{self, Kind, Manifests, Needed, Plan};
use crate::product;
use crate::progress::Progress;
use crate::remote::{self, BlobSource, Remote};
use crate::root;
use crate::server::{ServerOptions, TempServer};
use crate::util::{Key, hex, human_size, md5, parse_key};

impl BlobSource for LocalStorage {
    fn blob(&self, ekey: &Key) -> Option<Vec<u8>> {
        self.read(ekey).ok().flatten()
    }
}

/// Starts the temporary server, runs `f` against it and shuts it down.
fn with_server<T>(
    net: &NetOptions,
    region: &str,
    f: impl FnOnce(&Remote) -> Result<T>,
) -> Result<T> {
    let server = TempServer::start(ServerOptions::new(
        [127, 0, 0, 1].into(),
        net.port,
        net.mirror.clone(),
        net.cache.clone(),
    ))?;
    println!(
        "Temporary patch server: {}/{}/versions",
        server.base_url(),
        product::PRODUCT
    );
    let client = HttpClient::new(Some(Duration::from_mins(15)));
    let result = Remote::connect(client, &server.base_url(), region).and_then(|r| {
        println!(
            "{} {} (region {}): build config {}, CDN config {}, CDN {}",
            product::PRODUCT,
            r.version_name,
            r.region,
            hex(&r.build_key),
            hex(&r.cdn_key),
            r.cdn_base
        );
        f(&r)
    });
    println!("Patch server stopped; {}", server.stats().summary());
    server.shutdown();
    result
}

pub fn run(opts: &DownloadOptions) -> Result<()> {
    if opts.indices_only {
        return repair_indices(opts);
    }
    let region = opts.selection.branch();
    with_server(&opts.net, &region, |remote| download(remote, opts))
}

fn download(remote: &Remote, opts: &DownloadOptions) -> Result<()> {
    let sel = &opts.selection;
    println!("Selection: {}", sel.agent_tags());
    if opts.dry_run {
        let m = remote::load_manifests(remote, None, &mut Vec::new())?;
        let plan = plan::build(&m, sel)?;
        println!("Dry run: {}", plan.summary());
        return Ok(());
    }

    fs::create_dir_all(&opts.output)
        .with_context(|| format!("creating {}", opts.output.display()))?;
    let mut storage = LocalStorage::open(&opts.output)?;
    if storage.len() > 0 {
        println!("Resuming: {} files already stored", storage.len());
    }
    let mut fetched = Vec::new();
    let m = remote::load_manifests(remote, Some(&storage), &mut fetched)?;
    for (ekey, blob) in &fetched {
        storage.write(ekey, blob)?;
    }
    storage.write_config(&remote.build_key, &remote.config(&remote.build_key)?)?;
    storage.write_config(&remote.cdn_key, &remote.config(&remote.cdn_key)?)?;
    let subfolder = remote
        .product_config_text()
        .and_then(|t| product::subfolder_from_product_config(&t))
        .unwrap_or_else(|| product::DEFAULT_SUBFOLDER.to_owned());

    let plan = plan::build(&m, sel)?;
    println!("Plan: {}", plan.summary());

    let archives: Vec<Key> = m
        .cdn_config
        .values("archives")
        .context("CDN config lacks `archives`")?
        .iter()
        .map(|a| parse_key(a).context("bad archive key"))
        .collect::<Result<_>>()?;
    let wanted: HashSet<Key> = plan.needed.iter().map(|n| n.ekey).collect();
    let mut locations =
        fetch::load_indices(remote, &storage, &archives, &wanted, opts.net.threads)?;
    let pending = |kind_is_core: bool, storage: &LocalStorage| -> Vec<Needed> {
        plan.needed
            .iter()
            .filter(|n| (n.kind == Kind::Core) == kind_is_core && !storage.contains(&n.ekey))
            .copied()
            .collect()
    };

    // Build manifests first; the stored ROOT then resolves --include-fdid.
    let runner = Fetcher {
        remote,
        archives: &archives,
        threads: opts.net.threads,
    };
    let mut failures = runner.batch(
        "build manifests",
        pending(true, &storage),
        &locations,
        &mut storage,
    )?;
    let extra = include_fdids(&m, &storage, &opts.include_fdids)?;
    let unlocated: HashSet<Key> = extra
        .iter()
        .map(|n| n.ekey)
        .filter(|k| !locations.contains_key(k))
        .collect();
    if !unlocated.is_empty() {
        locations.extend(fetch::load_indices(
            remote,
            &storage,
            &archives,
            &unlocated,
            opts.net.threads,
        )?);
    }
    let rest = apply_limits(
        pending(false, &storage),
        &extra,
        opts.limit_files,
        opts.limit_bytes,
    );
    failures.extend(runner.batch("game data", rest, &locations, &mut storage)?);

    let set = IndexSet::from_cdn_config(&m.cdn_config)?;
    let indices = index_set::ensure(
        remote,
        &opts.output.join("Data/indices"),
        &set,
        opts.net.threads,
    )?;
    println!("Data/indices: {indices}");

    finalize(remote, sel, &mut storage, &plan, &opts.output, &subfolder)?;
    report(&failures, &plan, &storage, &opts.output)
}

fn report(
    failures: &[(Key, String)],
    plan: &Plan,
    storage: &LocalStorage,
    output: &Path,
) -> Result<()> {
    if !failures.is_empty() {
        for (ekey, reason) in failures.iter().take(10) {
            eprintln!("failed {}: {reason}", hex(ekey));
        }
        bail!(
            "{} files could not be downloaded; run the same command again to retry",
            failures.len()
        );
    }
    let missing = plan
        .needed
        .iter()
        .filter(|n| !storage.contains(&n.ekey))
        .count();
    if missing > 0 {
        println!("Partial download (limits): {missing} planned files not downloaded.");
    } else {
        println!(
            "Download complete: {} files in {}",
            storage.len(),
            output.display()
        );
    }
    Ok(())
}

/// `download --indices-only`: for an install written by this tool, rewrites
/// the local `.idx` files from the journal and fetches/builds the missing
/// `Data/indices` files. `data.###`, the journal, configs and `.build.info`
/// are left untouched.
fn repair_indices(opts: &DownloadOptions) -> Result<()> {
    let text = fs::read_to_string(opts.output.join(".build.info"))
        .with_context(|| format!("{}: no .build.info", opts.output.display()))?;
    let info = crate::tables::Table::parse(&text)?;
    let row = info
        .find_row("Product", product::PRODUCT)
        .with_context(|| format!(".build.info has no {} row", product::PRODUCT))?;
    let branch = info.get(row, "Branch").unwrap_or("us").to_owned();
    let cdn_key = info
        .get(row, "CDN Key")
        .and_then(parse_key)
        .context(".build.info: bad CDN Key")?;

    let mut storage = LocalStorage::open_index_only(&opts.output)?;
    storage.write_indices()?;
    println!(
        "Rewrote {} local .idx files for {} stored entries",
        crate::casc::idx::BUCKETS,
        storage.len()
    );
    with_server(&opts.net, &branch, |remote| {
        if remote.cdn_key != cdn_key {
            bail!(
                "install is for CDN config {}, the patch server offers {}",
                hex(&cdn_key),
                hex(&remote.cdn_key)
            );
        }
        let data = remote.config(&cdn_key)?;
        let set = IndexSet::from_cdn_config(&crate::config::ConfigFile::parse(&data)?)?;
        let report = index_set::ensure(
            remote,
            &opts.output.join("Data/indices"),
            &set,
            opts.net.threads,
        )?;
        println!("Data/indices: {report}");
        Ok(())
    })
}

/// Runs one labelled batch of downloads.
struct Fetcher<'a> {
    remote: &'a Remote,
    archives: &'a [Key],
    threads: usize,
}

impl Fetcher<'_> {
    fn batch(
        &self,
        label: &str,
        batch: Vec<Needed>,
        locations: &HashMap<Key, fetch::ArchiveLocation>,
        storage: &mut LocalStorage,
    ) -> Result<Vec<(Key, String)>> {
        let batch: Vec<Needed> = batch
            .into_iter()
            .filter(|n| !storage.contains(&n.ekey))
            .collect();
        if batch.is_empty() {
            return Ok(Vec::new());
        }
        let jobs = fetch::make_jobs(&batch, locations);
        let loose = jobs
            .iter()
            .filter(|j| matches!(j, Job::Loose { .. }))
            .count();
        let bytes = batch.iter().map(|n| n.size).sum();
        println!(
            "Downloading {label}: {} files, {} in {} requests ({loose} loose)",
            batch.len(),
            human_size(bytes),
            jobs.len()
        );
        let mut progress = Progress::new(label, batch.len(), bytes);
        fetch::run_jobs(
            self.remote,
            self.archives,
            jobs,
            storage,
            self.threads,
            &mut progress,
        )
    }
}

/// Hidden test option: the files of the given `FileDataIds` (all locale
/// variants in ROOT), fetched in addition to the limited selection. Needs
/// the ROOT in the local storage.
fn include_fdids(m: &Manifests, storage: &LocalStorage, ids: &[u32]) -> Result<Vec<Needed>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let root_content = m
        .build_config
        .key("root", 0)
        .context("build config lacks root")?;
    let root_ekey = m
        .encoding
        .find(&root_content)
        .context("ROOT not in ENCODING")?
        .ekey;
    let blob = storage
        .read(&root_ekey)?
        .context("ROOT was not downloaded")?;
    let records = root::find(&blte::decode(&blob)?, &ids.iter().copied().collect())?;
    let mut out = Vec::new();
    for r in records {
        if let Some(e) = m.encoding.find(&r.ckey) {
            println!(
                "Including FileDataId {} (locale flags {:#x}): EKey {}",
                r.file_data_id,
                r.locale_flags,
                hex(&e.ekey)
            );
            out.push(Needed {
                ekey: e.ekey,
                size: m.encoding.encoded_size(&e.ekey).unwrap_or(0),
                kind: Kind::Download,
            });
        }
    }
    Ok(out)
}

fn apply_limits(
    rest: Vec<Needed>,
    extra: &[Needed],
    limit_files: Option<usize>,
    limit_bytes: Option<u64>,
) -> Vec<Needed> {
    if limit_files.is_none() && limit_bytes.is_none() && extra.is_empty() {
        return rest;
    }
    let mut out: Vec<Needed> = extra.to_vec();
    let mut bytes = 0u64;
    for n in rest {
        if limit_files.is_some_and(|l| out.len() >= l + extra.len())
            || limit_bytes.is_some_and(|l| bytes >= l)
        {
            break;
        }
        bytes += n.size;
        if !out.iter().any(|o| o.ekey == n.ekey) {
            out.push(n);
        }
    }
    out
}

/// Local indices, `.build.info`, `.flavor.info` and the loose install files.
fn finalize(
    remote: &Remote,
    sel: &Selection,
    storage: &mut LocalStorage,
    plan: &Plan,
    output: &Path,
    subfolder: &str,
) -> Result<()> {
    storage.write_indices()?;
    let cdn = product::region_cdn(&remote.region).unwrap_or(&product::REGIONS[0]);
    let row = BuildInfoRow {
        branch: sel.branch(),
        build_key: hex(&remote.build_key),
        cdn_key: hex(&remote.cdn_key),
        cdn_path: product::CDN_PATH.to_owned(),
        cdn_hosts: cdn.hosts.to_owned(),
        cdn_servers: cdn.servers.to_owned(),
        tags: sel.agent_tags(),
        version: product::VERSIONS_NAME.to_owned(),
        product: product::PRODUCT.to_owned(),
    };
    let path = output.join(".build.info");
    let existing = fs::read_to_string(&path).ok();
    write_atomic(
        &path,
        build_info::render(existing.as_deref(), &row).as_bytes(),
    )?;

    let product_dir = output.join(subfolder);
    fs::create_dir_all(&product_dir)?;
    write_atomic(
        &product_dir.join(".flavor.info"),
        format!("Product Flavor!STRING:0\n{}\n", product::PRODUCT).as_bytes(),
    )?;
    let (mut written, mut skipped) = (0usize, 0usize);
    let mut dirs = HashMap::new();
    for (entry, ekey) in &plan.install_files {
        let target = install_path(&product_dir, &entry.name, &mut dirs)?;
        if fs::read(&target).is_ok_and(|d| d.len() == entry.size as usize && md5(&d) == entry.ckey)
        {
            written += 1;
            continue;
        }
        let Some(blob) = storage.read(ekey)? else {
            skipped += 1;
            continue;
        };
        let data = blte::decode(&blob).with_context(|| format!("decoding {}", entry.name))?;
        if md5(&data) != entry.ckey {
            bail!("install file {} does not match its CKey", entry.name);
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        write_atomic(&target, &data)?;
        #[cfg(unix)]
        if is_executable(&entry.name) {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
        }
        written += 1;
    }
    println!(
        "Wrote .build.info, {} local indices and {written} install files to {}{}",
        crate::casc::idx::BUCKETS,
        product_dir.display(),
        if skipped > 0 {
            format!(" ({skipped} not downloaded yet)")
        } else {
            String::new()
        }
    );
    Ok(())
}

/// macOS bundle binaries (`Contents/MacOS/...`, `.dylib`) get the executable bit.
fn is_executable(name: &str) -> bool {
    let n = name.replace('\\', "/");
    n.contains("/Contents/MacOS/")
        || Path::new(&n)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dylib"))
}

/// `<product dir>/<manifest path>` with `\` turned into `/`; rejects paths
/// that would escape the directory.
///
/// The manifest spells one directory in several cases (`Utils\icudtl.dat`,
/// `UTILS\LIBCEF.DLL`); Windows merges them, so on case-sensitive file
/// systems every directory keeps the first spelling seen (`dirs` maps the
/// lower-cased relative directory to it).
fn install_path(
    product_dir: &Path,
    name: &str,
    dirs: &mut HashMap<String, PathBuf>,
) -> Result<PathBuf> {
    let rel = PathBuf::from(name.replace('\\', "/"));
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        bail!("unsafe install path {name:?}");
    }
    let parts: Vec<&str> = name.split('\\').collect();
    let (file, parents) = parts.split_last().context("empty install path")?;
    let mut actual = PathBuf::new();
    let mut key = String::new();
    for part in parents {
        key.push_str(&part.to_lowercase());
        key.push('/');
        let next = actual.join(part);
        actual.clone_from(dirs.entry(key.clone()).or_insert(next));
    }
    Ok(product_dir.join(actual).join(file))
}

/// `list` command.
pub fn list(net: &NetOptions) -> Result<()> {
    with_server(net, "us", |remote| {
        let m = remote::load_manifests(remote, None, &mut Vec::new())?;
        println!(
            "Build {} ({}): {} ENCODING entries, {} install files, {} download entries",
            m.build_config.first("build-name").unwrap_or("?"),
            product::VERSIONS_NAME,
            m.encoding.len(),
            m.install.entries.len(),
            m.download.entries.len()
        );
        let combos = [
            (Os::Windows, Arch::X86_64),
            (Os::Windows, Arch::Arm64),
            (Os::MacOs, Arch::X86_64),
            (Os::MacOs, Arch::Arm64),
        ];
        let mut shipped = Vec::new();
        for (os, arch) in combos {
            let sel = Selection::new(os, arch, "enUS", None, None, None)?;
            match plan::check_platform(&m.install, &sel) {
                Ok(()) => shipped.push((os, arch)),
                Err(e) => println!("Not available: {e:#}"),
            }
        }
        let names: Vec<String> = shipped
            .iter()
            .map(|(o, a)| format!("{}/{}", o.name(), a.name()))
            .collect();
        println!("Platforms: {}", names.join(", "));
        println!("Locales: {}", LOCALES.join(" "));
        println!(
            "Regions: {} (default from the locale)",
            crate::cli::REGIONS.join(" ")
        );
        println!(
            "Download size (speech = text = locale, default region; build manifests included):"
        );
        let mut header = format!("  {:<6} {:<6}", "locale", "region");
        for n in &names {
            let _ = write!(header, " {n:>16}");
        }
        println!("{header}");
        for locale in LOCALES {
            let mut line = format!("  {locale:<6} {:<6}", crate::cli::default_region(locale));
            for (os, arch) in &shipped {
                let sel = Selection::new(*os, *arch, locale, None, None, None)?;
                let plan = plan::build(&m, &sel)?;
                let _ = write!(line, " {:>16}", human_size(plan.total_bytes()));
            }
            println!("{line}");
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_paths() {
        let dir = Path::new("/x/_classic_");
        let mut dirs = HashMap::new();
        assert_eq!(
            install_path(dir, "Utils\\icudtl.dat", &mut dirs).unwrap(),
            Path::new("/x/_classic_/Utils/icudtl.dat")
        );
        assert_eq!(
            install_path(dir, "UTILS\\LOCALES\\KO.PAK", &mut dirs).unwrap(),
            Path::new("/x/_classic_/Utils/LOCALES/KO.PAK"),
            "directory case merged like on Windows"
        );
        assert_eq!(
            install_path(dir, "Utils\\locales\\it.pak", &mut dirs).unwrap(),
            Path::new("/x/_classic_/Utils/LOCALES/it.pak")
        );
        assert!(install_path(dir, "..\\evil", &mut dirs).is_err());
        assert!(install_path(dir, "/etc/passwd", &mut dirs).is_err());
        assert!(is_executable(
            "World of Warcraft Classic.app\\Contents\\MacOS\\World of Warcraft Classic"
        ));
        assert!(!is_executable("WowClassic.exe"));
    }

    #[test]
    fn limits() {
        let n = |k: u8, size| Needed {
            ekey: [k; 16],
            size,
            kind: Kind::Download,
        };
        let rest = vec![n(1, 10), n(2, 10), n(3, 10)];
        assert_eq!(apply_limits(rest.clone(), &[], None, None).len(), 3);
        assert_eq!(apply_limits(rest.clone(), &[], Some(2), None).len(), 2);
        assert_eq!(apply_limits(rest.clone(), &[], None, Some(15)).len(), 2);
        let out = apply_limits(rest, &[n(9, 1), n(2, 10)], Some(1), None);
        let keys: Vec<u8> = out.iter().map(|x| x.ekey[0]).collect();
        assert_eq!(keys, [9, 2, 1], "extras first, duplicates skipped");
    }
}
