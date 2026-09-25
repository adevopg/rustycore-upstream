//! Blocking HTTP client (ureq) with the retry policy used against the CDN
//! and the community mirror: transport errors, 429 and 5xx are retried with
//! exponential backoff (1, 2, 4, 8 s), 403/404/410 mean "not there" and are
//! not retried. Bodies are taken as raw bytes (no content decoding), as
//! blizzget's `HttpRequest` does.

use std::io::Read;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

pub const USER_AGENT: &str = concat!("wow-client-downloader/", env!("CARGO_PKG_VERSION"));

/// Number of attempts for retryable failures.
const ATTEMPTS: u32 = 5;

#[derive(Clone)]
pub struct HttpClient {
    agent: ureq::Agent,
}

/// Outcome of one attempt.
pub enum Attempt {
    Body {
        status: u16,
        content_range: Option<String>,
        body: Vec<u8>,
    },
    NotFound(u16),
}

impl HttpClient {
    /// `total_timeout` bounds a whole request including its body.
    pub fn new(total_timeout: Option<Duration>) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_global(total_timeout)
            .max_idle_connections_per_host(16)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    pub fn agent(&self) -> &ureq::Agent {
        &self.agent
    }

    fn attempt(&self, url: &str, range: Option<(u64, u64)>) -> Result<Attempt, AttemptError> {
        let mut req = self.agent.get(url);
        if let Some((start, end)) = range {
            req = req.header("Range", format!("bytes={start}-{end}"));
        }
        let resp = req.call().map_err(|e| AttemptError::Retry(anyhow!(e)))?;
        let status = resp.status().as_u16();
        match status {
            200 | 206 => {
                let content_range = resp
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                let mut body = Vec::new();
                resp.into_body()
                    .into_reader()
                    .read_to_end(&mut body)
                    .map_err(|e| AttemptError::Retry(anyhow!(e)))?;
                Ok(Attempt::Body {
                    status,
                    content_range,
                    body,
                })
            }
            403 | 404 | 410 => Ok(Attempt::NotFound(status)),
            429 | 500..=599 => Err(AttemptError::Retry(anyhow!("HTTP {status}"))),
            _ => Err(AttemptError::Fatal(anyhow!("HTTP {status}"))),
        }
    }

    fn with_retries(&self, url: &str, range: Option<(u64, u64)>) -> Result<Attempt> {
        let mut delay = Duration::from_secs(1);
        let mut last = None;
        for attempt in 1..=ATTEMPTS {
            match self.attempt(url, range) {
                Ok(a) => return Ok(a),
                Err(AttemptError::Fatal(e)) => return Err(e.context(url.to_owned())),
                Err(AttemptError::Retry(e)) => {
                    last = Some(e);
                    if attempt < ATTEMPTS {
                        thread::sleep(delay);
                        delay *= 2;
                    }
                }
            }
        }
        Err(last
            .unwrap_or_else(|| anyhow!("no attempt"))
            .context(format!("{url} (after {ATTEMPTS} attempts)")))
    }

    /// Whole body, `None` when the server says the file does not exist.
    pub fn get_opt(&self, url: &str) -> Result<Option<Vec<u8>>> {
        match self.with_retries(url, None)? {
            Attempt::Body { body, .. } => Ok(Some(body)),
            Attempt::NotFound(_) => Ok(None),
        }
    }

    pub fn get(&self, url: &str) -> Result<Vec<u8>> {
        self.get_opt(url)?
            .with_context(|| format!("{url}: not found"))
    }

    /// Bytes `start..=end`. A server that ignores `Range` and answers 200
    /// with the whole file is handled by slicing.
    pub fn get_range(&self, url: &str, start: u64, end: u64) -> Result<Vec<u8>> {
        let len = (end - start + 1) as usize;
        match self.with_retries(url, Some((start, end)))? {
            Attempt::NotFound(status) => bail!("{url}: HTTP {status}"),
            Attempt::Body {
                status: 206,
                content_range,
                body,
            } => {
                let expected = format!("bytes {start}-{end}/");
                if let Some(cr) = content_range
                    && !cr.starts_with(&expected)
                {
                    bail!("{url}: unexpected Content-Range {cr:?}");
                }
                if body.len() != len {
                    bail!("{url}: range returned {} bytes, expected {len}", body.len());
                }
                Ok(body)
            }
            Attempt::Body { body, .. } => body
                .get(start as usize..start as usize + len)
                .map(<[u8]>::to_vec)
                .with_context(|| format!("{url}: file shorter than the requested range")),
        }
    }
}

enum AttemptError {
    Retry(anyhow::Error),
    Fatal(anyhow::Error),
}
