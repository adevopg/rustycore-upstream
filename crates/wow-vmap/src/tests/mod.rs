//! Unit tests for the VMAP formats (round trips, BIH golden output, loading).

mod bih;
mod formats;
mod map_tree;

/// Deterministic xorshift32 generator shared by the tests.
pub(crate) struct Rng(pub u32);

impl Rng {
    pub fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Float in `[0, range)` with 1/64 resolution (exactly representable).
    pub fn coord(&mut self, range: u32) -> f32 {
        (self.next() % (range * 64)) as f32 / 64.0
    }
}

/// Unique scratch directory under the system temp dir.
pub(crate) fn temp_dir(tag: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "wow-vmap-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
