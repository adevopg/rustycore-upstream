//! The temporary server against fake "Blizzard" and "mirror" upstreams.

use std::collections::HashMap;
use std::sync::atomic::AtomicU64;

use super::proxy::{parse_range, valid_path};
use super::*;
use crate::remote::Remote;
use crate::util::{hex, md5};

/// A minimal static file server with `Range` support.
struct Fake {
    server: Arc<tiny_http::Server>,
    url: String,
    hits: Arc<AtomicU64>,
}

impl Fake {
    fn start(files: HashMap<String, Vec<u8>>) -> Self {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let hits = Arc::new(AtomicU64::new(0));
        let (s, h) = (server.clone(), hits.clone());
        thread::spawn(move || {
            for req in s.incoming_requests() {
                h.fetch_add(1, Ordering::Relaxed);
                let path = req.url().to_owned();
                let Some(data) = files.get(&path) else {
                    let _ = req.respond(Response::empty(404));
                    continue;
                };
                let range = req
                    .headers()
                    .iter()
                    .find(|x| x.field.equiv("Range"))
                    .map(|x| x.value.as_str().to_owned());
                match parse_range(range.as_deref(), data.len() as u64) {
                    Ok(Some((a, b))) => {
                        let cr = format!("bytes {a}-{b}/{}", data.len());
                        let resp = Response::from_data(data[a as usize..=b as usize].to_vec())
                            .with_status_code(206)
                            .with_header(Header::from_bytes("Content-Range", cr).unwrap());
                        let _ = req.respond(resp);
                    }
                    _ => {
                        let _ = req.respond(Response::from_data(data.clone()));
                    }
                }
            }
        });
        Self { server, url, hits }
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.server.unblock();
    }
}

fn opts(blizzard: &Fake, mirror: Option<&Fake>, cache: Option<PathBuf>) -> ServerOptions {
    ServerOptions {
        bind: [127, 0, 0, 1].into(),
        port: 0,
        blizzard: vec![blizzard.url.clone()],
        mirror: mirror.map(|m| m.url.clone()),
        cache,
        verbose: false,
    }
}

#[test]
fn serves_tables_like_the_patch_server() {
    let blizzard = Fake::start(HashMap::new());
    let server = TempServer::start(opts(&blizzard, None, None)).unwrap();
    let client = HttpClient::new(Some(std::time::Duration::from_secs(10)));
    let base = server.base_url();
    let versions =
        String::from_utf8(client.get(&format!("{base}/wow_classic/versions")).unwrap()).unwrap();
    assert!(versions.starts_with("Region!STRING:0|BuildConfig!HEX:16|"));
    assert!(versions.lines().nth(1).unwrap().starts_with("## seqn = "));
    assert!(versions.contains("|54261|3.4.3.54261|"));
    let cdns = String::from_utf8(client.get(&format!("{base}/wow_classic/cdns")).unwrap()).unwrap();
    let host = base.trim_start_matches("http://");
    assert!(cdns.contains(&format!(
        "eu|tpr/wow|{host}|http://{host}/?maxhosts=4|tpr/configs/data"
    )));
    assert!(
        client
            .get_opt(&format!("{base}/wow_classic/bgdl"))
            .unwrap()
            .is_some()
    );
    assert!(
        client
            .get_opt(&format!("{base}/wow/versions"))
            .unwrap()
            .is_none()
    );

    // The downloader side reads them like blizzget.
    let remote = Remote::connect(client, &base, "eu").unwrap();
    assert_eq!(remote.cdn_base, format!("http://{host}/tpr/wow"));
    assert_eq!(hex(&remote.build_key), product::BUILD_CONFIG);
    assert_eq!(hex(&remote.cdn_key), product::CDN_CONFIG);
    assert_eq!(remote.version_name, "3.4.3.54261");
    server.shutdown();
}

#[test]
fn proxy_falls_back_to_the_mirror_and_caches() {
    let config = b"# Build Configuration\nroot = 00\n".to_vec();
    let key = hex(&md5(&config));
    let config_path = format!("/tpr/wow/config/{}/{}/{key}", &key[0..2], &key[2..4]);
    let archive: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
    let on_blizzard = HashMap::from([("/tpr/wow/data/aa/bb/archive".to_owned(), archive.clone())]);
    let on_mirror = HashMap::from([
        (config_path.clone(), config.clone()),
        ("/tpr/wow/data/aa/bb/archive".to_owned(), vec![0; 5]),
    ]);
    let blizzard = Fake::start(on_blizzard);
    let mirror = Fake::start(on_mirror);
    let cache = tempfile::tempdir().unwrap();
    let server = TempServer::start(opts(
        &blizzard,
        Some(&mirror),
        Some(cache.path().to_path_buf()),
    ))
    .unwrap();
    let client = HttpClient::new(Some(std::time::Duration::from_secs(10)));
    let base = server.base_url();

    // Missing on Blizzard -> mirror; the second request skips Blizzard and,
    // being cached, the mirror too.
    assert_eq!(client.get(&format!("{base}{config_path}")).unwrap(), config);
    assert!(cache.path().join(&config_path[1..]).is_file());
    let (b0, m0) = (
        blizzard.hits.load(Ordering::Relaxed),
        mirror.hits.load(Ordering::Relaxed),
    );
    assert_eq!((b0, m0), (1, 1));
    assert_eq!(client.get(&format!("{base}{config_path}")).unwrap(), config);
    assert_eq!(blizzard.hits.load(Ordering::Relaxed), 1);
    assert_eq!(mirror.hits.load(Ordering::Relaxed), 1);
    // Ranges are served from the cached file too.
    let part = client
        .get_range(&format!("{base}{config_path}"), 2, 10)
        .unwrap();
    assert_eq!(part, &config[2..=10]);

    // Present on Blizzard: served from there (range forwarded, 206).
    let url = format!("{base}/tpr/wow/data/aa/bb/archive");
    assert_eq!(
        client.get_range(&url, 1000, 1999).unwrap(),
        &archive[1000..2000]
    );
    assert_eq!(mirror.hits.load(Ordering::Relaxed), 1, "mirror not asked");

    // Missing everywhere -> 404 for the client.
    assert!(
        client
            .get_opt(&format!("{base}/tpr/wow/data/00/00/nothing"))
            .unwrap()
            .is_none()
    );
    assert!(
        client.get_opt(&format!("{base}/tpr/wow/../etc")).is_err(),
        "400 bad path"
    );
    let stats = server.stats();
    assert_eq!(stats.mirror_hits.load(Ordering::Relaxed), 1);
    assert_eq!(stats.blizzard_hits.load(Ordering::Relaxed), 1);
    assert_eq!(stats.cache_hits.load(Ordering::Relaxed), 2);
    server.shutdown();
}

#[test]
fn no_mirror_means_blizzard_only() {
    let blizzard = Fake::start(HashMap::new());
    let server = TempServer::start(opts(&blizzard, None, None)).unwrap();
    let client = HttpClient::new(Some(std::time::Duration::from_secs(10)));
    let url = format!("{}/tpr/wow/config/00/00/x", server.base_url());
    assert!(client.get_opt(&url).unwrap().is_none());
    assert_eq!(blizzard.hits.load(Ordering::Relaxed), 1);
}

#[test]
fn range_parsing() {
    assert_eq!(parse_range(None, 10), Ok(None));
    assert_eq!(parse_range(Some("bytes=2-5"), 10), Ok(Some((2, 5))));
    assert_eq!(parse_range(Some("bytes=2-"), 10), Ok(Some((2, 9))));
    assert_eq!(parse_range(Some("bytes=-3"), 10), Ok(Some((7, 9))));
    assert_eq!(parse_range(Some("bytes=5-100"), 10), Ok(Some((5, 9))));
    assert_eq!(parse_range(Some("bytes=10-"), 10), Err(()));
    assert_eq!(parse_range(Some("bytes=1-2,4-5"), 10), Ok(None));
    assert_eq!(parse_range(Some("items=1-2"), 10), Ok(None));
    assert!(valid_path("tpr/wow/data/aa/bb/x.index"));
    assert!(!valid_path("tpr/../x") && !valid_path("tpr//x") && !valid_path("tpr/a b"));
}
