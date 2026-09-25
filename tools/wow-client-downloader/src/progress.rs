//! Concise progress lines: files, bytes, rate and ETA. On a terminal the
//! line is redrawn in place about once per second, otherwise a new line is
//! printed every 15 seconds.

use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

use crate::util::human_size;

pub struct Progress {
    label: String,
    total_files: usize,
    total_bytes: u64,
    pub done_files: usize,
    pub done_bytes: u64,
    start: Instant,
    last: Instant,
    tty: bool,
}

impl Progress {
    pub fn new(label: &str, total_files: usize, total_bytes: u64) -> Self {
        let now = Instant::now();
        Self {
            label: label.to_owned(),
            total_files,
            total_bytes,
            done_files: 0,
            done_bytes: 0,
            start: now,
            last: now,
            tty: std::io::stdout().is_terminal(),
        }
    }

    pub fn add(&mut self, files: usize, bytes: u64) {
        self.done_files += files;
        self.done_bytes += bytes;
        let every = if self.tty {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(15)
        };
        if self.last.elapsed() >= every {
            self.print(false);
        }
    }

    #[allow(clippy::cast_precision_loss, reason = "display only")]
    pub fn line(&self) -> String {
        let secs = self.start.elapsed().as_secs_f64().max(0.001);
        if self.total_bytes == 0 {
            // Count-only progress (e.g. archive indices).
            let rate = self.done_files as f64 / secs;
            let left = self.total_files.saturating_sub(self.done_files) as f64;
            let eta = if rate > 0.0 {
                format_hms((left / rate) as u64)
            } else {
                "--:--:--".to_owned()
            };
            return format!(
                "{}: {}/{} files, {rate:.1} files/s, ETA {eta}",
                self.label, self.done_files, self.total_files
            );
        }
        let rate = self.done_bytes as f64 / secs;
        let pct = if self.total_bytes > 0 {
            self.done_bytes as f64 * 100.0 / self.total_bytes as f64
        } else {
            100.0
        };
        let eta = if rate > 0.0 && self.total_bytes > self.done_bytes {
            format_hms(((self.total_bytes - self.done_bytes) as f64 / rate) as u64)
        } else {
            "--:--:--".to_owned()
        };
        format!(
            "{}: {}/{} files, {}/{} ({pct:.1}%), {}/s, ETA {eta}",
            self.label,
            self.done_files,
            self.total_files,
            human_size(self.done_bytes),
            human_size(self.total_bytes),
            human_size(rate as u64),
        )
    }

    pub fn print(&mut self, last: bool) {
        self.last = Instant::now();
        let mut out = std::io::stdout().lock();
        if self.tty {
            let _ = write!(out, "\r\x1b[2K{}", self.line());
            if last {
                let _ = writeln!(out);
            }
        } else {
            let _ = writeln!(out, "{}", self.line());
        }
        let _ = out.flush();
    }

    pub fn finish(&mut self) {
        self.print(true);
    }
}

pub fn format_hms(secs: u64) -> String {
    format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_format() {
        let mut p = Progress::new("data", 10, 2048);
        p.done_files = 5;
        p.done_bytes = 1024;
        let line = p.line();
        assert!(
            line.starts_with("data: 5/10 files, 1.00 KiB/2.00 KiB (50.0%)"),
            "{line}"
        );
        assert_eq!(format_hms(3725), "01:02:05");
        let count_only = Progress::new("indices", 4, 0).line();
        assert!(
            count_only.starts_with("indices: 0/4 files, "),
            "{count_only}"
        );
    }
}
