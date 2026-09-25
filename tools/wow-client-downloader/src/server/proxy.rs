//! `/tpr/...` CDN proxy: Blizzard hosts, then the mirror; streaming,
//! `Range`/`HEAD` forwarding and the optional on-disk cache (see the parent
//! module docs for the policy).

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tiny_http::{Header, Method, Request, Response, StatusCode};

use super::{State, Stats, log, text_response};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Blizzard,
    Mirror,
}

/// Relative CDN paths only: `[A-Za-z0-9._-]` segments, no `..`.
pub(super) fn valid_path(rel: &str) -> bool {
    !rel.is_empty()
        && rel.split('/').all(|seg| {
            !seg.is_empty()
                && seg != "."
                && seg != ".."
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
        })
}

pub(super) fn handle(state: &Arc<State>, request: Request, method: &Method, rel: &str) {
    let path = format!("/{rel}");
    if !valid_path(rel) {
        let _ = request.respond(text_response(400, "bad path\n".into()));
        return;
    }
    let range = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str().to_owned());
    let head = *method == Method::Head;

    if let Some(dir) = &state.opts.cache {
        let cached = dir.join(rel);
        if cached.is_file() {
            serve_file(
                state,
                request,
                method,
                &path,
                &cached,
                range.as_deref(),
                head,
            );
            return;
        }
    }

    let agent = state.upstream.agent();
    let call = |url: &str| {
        let mut req = if head {
            agent.head(url)
        } else {
            agent.get(url)
        };
        if let Some(r) = &range {
            req = req.header("Range", r);
        }
        req.call()
    };
    let mut all_missing = true;

    let skip_blizzard = state
        .blizzard_missing
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(rel);
    if !skip_blizzard {
        let mut missing = 0;
        for base in &state.opts.blizzard {
            match call(&format!("{base}/{rel}")) {
                Ok(resp) => match resp.status().as_u16() {
                    200 | 206 | 416 => {
                        let target = cache_target(state, rel, range.as_deref(), head);
                        stream(
                            state,
                            request,
                            method,
                            &path,
                            resp,
                            Source::Blizzard,
                            target,
                        );
                        return;
                    }
                    403 | 404 | 410 => missing += 1,
                    _ => all_missing = false,
                },
                Err(_) => all_missing = false,
            }
        }
        if missing > 0 && missing == state.opts.blizzard.len() {
            state
                .blizzard_missing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(rel.to_owned());
        }
    }
    if let Some(mirror) = &state.opts.mirror {
        match call(&format!("{mirror}/{rel}")) {
            Ok(resp) => match resp.status().as_u16() {
                200 | 206 | 416 => {
                    let target = cache_target(state, rel, range.as_deref(), head);
                    stream(state, request, method, &path, resp, Source::Mirror, target);
                    return;
                }
                403 | 404 | 410 => {}
                _ => all_missing = false,
            },
            Err(_) => all_missing = false,
        }
    }
    let status = if all_missing { 404 } else { 502 };
    state.stats.not_found.fetch_add(1, Ordering::Relaxed);
    log(state, method, &path, status, "no upstream");
    let _ = request.respond(text_response(status, "not available upstream\n".into()));
}

/// Cache file for a complete GET answer.
fn cache_target(state: &State, rel: &str, range: Option<&str>, head: bool) -> Option<PathBuf> {
    let dir = state.opts.cache.as_ref()?;
    (range.is_none() && !head).then(|| dir.join(rel))
}

fn octet_stream() -> Header {
    Header::from_bytes("Content-Type", "application/octet-stream").expect("valid header")
}

fn stream(
    state: &Arc<State>,
    request: Request,
    method: &Method,
    path: &str,
    resp: ureq::http::Response<ureq::Body>,
    source: Source,
    cache: Option<PathBuf>,
) {
    let status = resp.status().as_u16();
    let mut headers = vec![octet_stream()];
    for name in ["content-range", "accept-ranges", "last-modified", "etag"] {
        if let Some(v) = resp.headers().get(name)
            && let Ok(h) = Header::from_bytes(name, v.as_bytes())
        {
            headers.push(h);
        }
    }
    let len: Option<u64> = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let hits = match source {
        Source::Blizzard => &state.stats.blizzard_hits,
        Source::Mirror => &state.stats.mirror_hits,
    };
    hits.fetch_add(1, Ordering::Relaxed);
    let label = match source {
        Source::Blizzard => "blizzard",
        Source::Mirror => "mirror",
    };
    log(state, method, path, status, label);
    let tee = if status == 200 {
        cache.and_then(|p| Tee::create(p, len))
    } else {
        None
    };
    let mut body = ProxyBody {
        inner: resp.into_body().into_reader(),
        stats: state.stats.clone(),
        source,
        tee,
    };
    let head = *method == Method::Head;
    if len.is_none() && !head {
        // Chunked upstream answer (the mirror over HTTP/1.1): buffer it so
        // the client gets a Content-Length, like from Blizzard's CDN.
        let mut data = Vec::new();
        if body.read_to_end(&mut data).is_err() {
            let _ = request.respond(text_response(502, "upstream read error\n".into()));
            return;
        }
        let mut response = Response::from_data(data)
            .with_status_code(StatusCode(status))
            .with_chunked_threshold(usize::MAX);
        for h in headers {
            response.add_header(h);
        }
        let _ = request.respond(response);
        return;
    }
    let response = Response::new(
        StatusCode(status),
        headers,
        body,
        len.map(|l| l as usize),
        None,
    )
    // Send Content-Length instead of tiny_http's chunking above 32 KiB.
    .with_chunked_threshold(usize::MAX);
    let _ = request.respond(response);
}

/// Upstream body reader that counts bytes and optionally tees into the cache.
struct ProxyBody<R> {
    inner: R,
    stats: Arc<Stats>,
    source: Source,
    tee: Option<Tee>,
}

impl<R: Read> Read for ProxyBody<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        let counter = match self.source {
            Source::Blizzard => &self.stats.blizzard_bytes,
            Source::Mirror => &self.stats.mirror_bytes,
        };
        counter.fetch_add(n as u64, Ordering::Relaxed);
        if let Some(tee) = &mut self.tee {
            let ok = if n == 0 {
                tee.commit()
            } else {
                tee.write(&buf[..n])
            };
            if !ok || n == 0 {
                self.tee = None;
            }
        }
        Ok(n)
    }
}

/// A cache file being written; renamed into place only when complete.
struct Tee {
    file: Option<File>,
    tmp: PathBuf,
    target: PathBuf,
    expected: Option<u64>,
    written: u64,
}

static TEE_SEQ: AtomicU64 = AtomicU64::new(0);

impl Tee {
    fn create(target: PathBuf, expected: Option<u64>) -> Option<Self> {
        fs::create_dir_all(target.parent()?).ok()?;
        let seq = TEE_SEQ.fetch_add(1, Ordering::Relaxed);
        let tmp = target.with_extension(format!("part{}-{seq}", std::process::id()));
        let file = File::create(&tmp).ok()?;
        Some(Self {
            file: Some(file),
            tmp,
            target,
            expected,
            written: 0,
        })
    }

    fn write(&mut self, data: &[u8]) -> bool {
        let ok = self
            .file
            .as_mut()
            .is_some_and(|f| f.write_all(data).is_ok());
        self.written += data.len() as u64;
        ok
    }

    fn commit(&mut self) -> bool {
        let complete = self.expected.is_none_or(|e| e == self.written);
        if let Some(file) = self.file.take()
            && complete
            && file.sync_all().is_ok()
        {
            drop(file);
            return fs::rename(&self.tmp, &self.target).is_ok();
        }
        false
    }
}

impl Drop for Tee {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.tmp);
    }
}

/// Single-range `Range` header. `Ok(None)`: serve the whole file (no or
/// unsupported header, as RFC 9110 allows); `Err`: unsatisfiable.
pub(super) fn parse_range(header: Option<&str>, total: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return Ok(None);
    };
    if spec.contains(',') {
        return Ok(None);
    }
    let Some((a, b)) = spec.split_once('-') else {
        return Ok(None);
    };
    let (a, b) = (a.trim(), b.trim());
    let range = if a.is_empty() {
        let Ok(n) = b.parse::<u64>() else {
            return Ok(None);
        };
        if n == 0 || total == 0 {
            return Err(());
        }
        (total - n.min(total), total - 1)
    } else {
        let Ok(start) = a.parse::<u64>() else {
            return Ok(None);
        };
        let end = if b.is_empty() {
            total.saturating_sub(1)
        } else {
            match b.parse::<u64>() {
                Ok(e) if e >= start => e.min(total.saturating_sub(1)),
                _ => return Ok(None),
            }
        };
        if start >= total {
            return Err(());
        }
        (start, end)
    };
    Ok(Some(range))
}

fn serve_file(
    state: &State,
    request: Request,
    method: &Method,
    path: &str,
    file_path: &Path,
    range: Option<&str>,
    head: bool,
) {
    let opened = File::open(file_path).and_then(|f| f.metadata().map(|m| (f, m.len())));
    let Ok((mut file, total)) = opened else {
        let _ = request.respond(text_response(500, "cache read error\n".into()));
        return;
    };
    state.stats.cache_hits.fetch_add(1, Ordering::Relaxed);
    let mut headers = vec![
        octet_stream(),
        Header::from_bytes("Accept-Ranges", "bytes").expect("valid header"),
    ];
    let (status, start, len) = match parse_range(range, total) {
        Ok(None) => (200, 0, total),
        Ok(Some((a, b))) => {
            headers.push(
                Header::from_bytes("Content-Range", format!("bytes {a}-{b}/{total}"))
                    .expect("valid header"),
            );
            (206, a, b - a + 1)
        }
        Err(()) => {
            headers.push(
                Header::from_bytes("Content-Range", format!("bytes */{total}"))
                    .expect("valid header"),
            );
            log(state, method, path, 416, "cache");
            let _ = request.respond(Response::empty(416).with_header(headers.remove(2)));
            return;
        }
    };
    if file.seek(SeekFrom::Start(start)).is_err() {
        let _ = request.respond(text_response(500, "cache read error\n".into()));
        return;
    }
    if !head {
        state.stats.cache_bytes.fetch_add(len, Ordering::Relaxed);
    }
    log(state, method, path, status, "cache");
    let response = Response::new(
        StatusCode(status),
        headers,
        file.take(len),
        Some(len as usize),
        None,
    )
    // Send Content-Length instead of tiny_http's chunking above 32 KiB.
    .with_chunked_threshold(usize::MAX);
    let _ = request.respond(response);
}
