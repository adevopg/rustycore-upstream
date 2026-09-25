//! The temporary local patch server + CDN proxy.
//!
//! Blizzard's patch server no longer lists 3.4.3.54261, so this server plays
//! its role for `wow_classic`: `/<product>/versions`, `/cdns` and `/bgdl` in
//! Blizzard's exact table format ([`crate::tables`]), with `cdns` pointing at
//! the server itself. Everything under `/tpr/...` is proxied: the Blizzard CDN
//! hosts first, then (only when they answer 403/404/5xx or fail) the mirror.
//! Bodies are streamed, `Range` headers are forwarded (206 answers pass
//! through), `HEAD` is forwarded as `HEAD`. A path Blizzard reported missing
//! is remembered so later requests (e.g. many ranges of one archive) go
//! straight to the mirror. With a cache directory, complete (non-range) 200
//! answers are stored under `<cache>/tpr/...` and later served locally,
//! including ranges.
//!
//! The downloader starts this server on 127.0.0.1 and reads versions/cdns
//! from it exactly like blizzget's `NGDP::NGDP` reads Blizzard's server.

mod proxy;

use std::io::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use tiny_http::{Header, Method, Response, StatusCode};

use crate::http::HttpClient;
use crate::product;
use crate::tables;

#[derive(Debug, Clone)]
pub struct ServerOptions {
    pub bind: IpAddr,
    /// 0 picks a free port.
    pub port: u16,
    /// Blizzard CDN base URLs tried first, e.g. `http://level3.blizzard.com`.
    pub blizzard: Vec<String>,
    /// Mirror base URL (`<mirror>/tpr/wow/...`), `None` to disable.
    pub mirror: Option<String>,
    pub cache: Option<PathBuf>,
    /// Print one line per request.
    pub verbose: bool,
}

/// Request/byte counters by source.
#[derive(Debug, Default)]
pub struct Stats {
    pub requests: AtomicU64,
    pub blizzard_bytes: AtomicU64,
    pub blizzard_hits: AtomicU64,
    pub mirror_bytes: AtomicU64,
    pub mirror_hits: AtomicU64,
    pub cache_bytes: AtomicU64,
    pub cache_hits: AtomicU64,
    pub not_found: AtomicU64,
}

impl Stats {
    pub fn summary(&self) -> String {
        let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
        format!(
            "{} requests: Blizzard CDN {} ({}), mirror {} ({}), cache {} ({}), not found {}",
            g(&self.requests),
            g(&self.blizzard_hits),
            crate::util::human_size(g(&self.blizzard_bytes)),
            g(&self.mirror_hits),
            crate::util::human_size(g(&self.mirror_bytes)),
            g(&self.cache_hits),
            crate::util::human_size(g(&self.cache_bytes)),
            g(&self.not_found),
        )
    }
}

impl ServerOptions {
    /// Default upstreams: [`product::BLIZZARD_HOSTS`] over plain HTTP, like the
    /// Agent's `http://<host>/?maxhosts=` servers.
    pub fn new(bind: IpAddr, port: u16, mirror: Option<String>, cache: Option<PathBuf>) -> Self {
        Self {
            bind,
            port,
            blizzard: product::BLIZZARD_HOSTS
                .iter()
                .map(|h| format!("http://{h}"))
                .collect(),
            mirror: mirror.map(|m| m.trim_end_matches('/').to_owned()),
            cache,
            verbose: false,
        }
    }
}

pub(crate) struct State {
    opts: ServerOptions,
    host_port: String,
    seqn: u64,
    upstream: HttpClient,
    /// Paths Blizzard's hosts answered 403/404 for.
    blizzard_missing: Mutex<std::collections::HashSet<String>>,
    stats: Arc<Stats>,
}

pub struct TempServer {
    server: Arc<tiny_http::Server>,
    thread: Option<JoinHandle<()>>,
    addr: SocketAddr,
    stats: Arc<Stats>,
}

impl TempServer {
    pub fn start(opts: ServerOptions) -> Result<Self> {
        let server = tiny_http::Server::http(SocketAddr::new(opts.bind, opts.port))
            .map_err(|e| anyhow!("cannot listen on {}:{}: {e}", opts.bind, opts.port))?;
        let addr = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| anyhow!("server has no IP address"))?;
        let server = Arc::new(server);
        let counters = Arc::new(Stats::default());
        let host = if opts.bind.is_unspecified() {
            "127.0.0.1".to_owned()
        } else {
            opts.bind.to_string()
        };
        let state = Arc::new(State {
            host_port: format!("{host}:{}", addr.port()),
            // Newer than any real seqn, stable for the server's lifetime.
            seqn: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(1, |d| d.as_secs()),
            upstream: HttpClient::new(None),
            blizzard_missing: Mutex::default(),
            stats: counters.clone(),
            opts,
        });
        let accept = server.clone();
        let thread = thread::Builder::new()
            .name("wcd-server".into())
            .spawn(move || {
                for request in accept.incoming_requests() {
                    let state = state.clone();
                    let _ = thread::Builder::new()
                        .name("wcd-request".into())
                        .spawn(move || handle(&state, request));
                }
            })?;
        Ok(Self {
            server,
            thread: Some(thread),
            addr,
            stats: counters,
        })
    }

    /// `http://127.0.0.1:<port>` (loopback even when bound to 0.0.0.0).
    pub fn base_url(&self) -> String {
        let ip = if self.addr.ip().is_unspecified() {
            "127.0.0.1".to_owned()
        } else {
            self.addr.ip().to_string()
        };
        format!("http://{ip}:{}", self.addr.port())
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.server.unblock();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for TempServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `serve` command: runs until the process is interrupted.
pub fn serve(opts: ServerOptions) -> Result<()> {
    let mirror = opts.mirror.clone();
    let server = TempServer::start(opts)?;
    let base = server.base_url();
    let p = product::PRODUCT;
    println!(
        "Temporary patch server for {p} {} listening on {base}",
        product::VERSIONS_NAME
    );
    println!("  versions: {base}/{p}/versions");
    println!("  cdns:     {base}/{p}/cdns");
    println!("  bgdl:     {base}/{p}/bgdl");
    println!(
        "  CDN:      {base}/{}/  (Blizzard: {})",
        product::CDN_PATH,
        product::BLIZZARD_HOSTS.join(", ")
    );
    match mirror {
        Some(m) => println!("  fallback mirror: {m}"),
        None => println!("  fallback mirror: disabled"),
    }
    println!("Press Ctrl-C to stop.");
    if let Some(t) = &server.thread {
        while !t.is_finished() {
            thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    Ok(())
}

fn text_response(status: u16, body: String) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(StatusCode(status))
        .with_header(
            Header::from_bytes("Content-Type", "text/plain; charset=utf-8").expect("valid header"),
        )
}

fn handle(state: &Arc<State>, request: tiny_http::Request) {
    state.stats.requests.fetch_add(1, Ordering::Relaxed);
    let method = request.method().clone();
    if method != Method::Get && method != Method::Head {
        let _ = request.respond(text_response(405, "method not allowed\n".into()));
        return;
    }
    let url = request.url().to_owned();
    let path = url.split(['?', '#']).next().unwrap_or_default().to_owned();
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let table = match segments.as_slice() {
        [prod, endpoint] if prod.eq_ignore_ascii_case(product::PRODUCT) => match *endpoint {
            "versions" => Some(tables::versions_table(state.seqn)),
            "cdns" => Some(tables::cdns_table(state.seqn, &state.host_port)),
            "bgdl" => Some(tables::bgdl_table(state.seqn)),
            _ => None,
        },
        _ => None,
    };
    if let Some(table) = table {
        log(state, &method, &path, 200, "tables");
        let _ = request.respond(text_response(200, table.render()));
        return;
    }
    if path.starts_with("/tpr/") {
        proxy::handle(state, request, &method, &path[1..]);
        return;
    }
    if path == "/" {
        let body = format!(
            "wow-client-downloader temporary patch server for {} {}\n/{}/versions\n/{}/cdns\n/tpr/...\n",
            product::PRODUCT,
            product::VERSIONS_NAME,
            product::PRODUCT,
            product::PRODUCT
        );
        let _ = request.respond(text_response(200, body));
        return;
    }
    state.stats.not_found.fetch_add(1, Ordering::Relaxed);
    log(state, &method, &path, 404, "unknown");
    let _ = request.respond(text_response(404, "not found\n".into()));
}

fn log(state: &State, method: &Method, path: &str, status: u16, source: &str) {
    if state.opts.verbose {
        // A closed stdout must not take the request thread down.
        let _ = writeln!(std::io::stdout(), "{method} {path} -> {status} ({source})");
    }
}

#[cfg(test)]
mod tests;
