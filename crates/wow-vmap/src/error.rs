//! Error type for VMAP file handling. The C++ code reports failures through
//! `bool`/`LoadResult` returns plus `printf`; the Rust port carries the
//! reason in [`VmapError`].

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum VmapError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{what}: malformed or truncated data")]
    Format { what: &'static str },
    #[error("BIH build failed: {0}")]
    Bih(#[from] crate::bih::BihBuildError),
    #[error("triangle references vertex {index} but only {count} vertices exist")]
    VertexIndexOutOfRange { index: u32, count: usize },
}

impl VmapError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn format(what: &'static str) -> Self {
        Self::Format { what }
    }
}

pub type Result<T, E = VmapError> = std::result::Result<T, E>;

/// Turns an `Option` from [`crate::io::Reader`] into a format error.
pub(crate) trait OrFormat<T> {
    fn or_format(self, what: &'static str) -> Result<T>;
}

impl<T> OrFormat<T> for Option<T> {
    fn or_format(self, what: &'static str) -> Result<T> {
        self.ok_or(VmapError::Format { what })
    }
}

impl OrFormat<()> for bool {
    fn or_format(self, what: &'static str) -> Result<()> {
        if self {
            Ok(())
        } else {
            Err(VmapError::Format { what })
        }
    }
}
