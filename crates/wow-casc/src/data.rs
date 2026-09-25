//! Local data archives (`Data/data/data.###`).
//!
//! Port of `CascLib` `dep/CascLib/src/CascReadFile.cpp`: `OpenDataStream` (lazy,
//! cached, read-only open of `data.%03u` in the index directory) and the
//! `FileStream_Read` of the whole encoded entry with `STREAM_FLAG_FILL_MISSING`
//! (bytes past the end of a truncated archive read as zeros).

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct DataArchives {
    dir: PathBuf,
    open: Mutex<HashMap<u32, Arc<File>>>,
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

impl DataArchives {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            open: Mutex::new(HashMap::new()),
        }
    }

    /// Opens (once) `data.%03u`. `Ok(None)` when the archive does not exist
    /// (`OpenDataStream` returning `ERROR_FILE_NOT_FOUND`).
    fn archive(&self, index: u32) -> io::Result<Option<Arc<File>>> {
        let mut open = self
            .open
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(file) = open.get(&index) {
            return Ok(Some(file.clone()));
        }
        let path = self.dir.join(format!("data.{index:03}"));
        match File::open(&path) {
            Ok(file) => {
                let file = Arc::new(file);
                open.insert(index, file.clone());
                Ok(Some(file))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Reads `size` bytes at `offset` of archive `index`; missing trailing
    /// bytes are zero (`STREAM_FLAG_FILL_MISSING`). `Ok(None)` when the
    /// archive file does not exist.
    pub fn read(&self, index: u32, offset: u64, size: usize) -> io::Result<Option<Vec<u8>>> {
        let Some(file) = self.archive(index)? else {
            return Ok(None);
        };
        let mut buf = vec![0u8; size];
        let mut done = 0;
        while done < size {
            match read_at(&file, &mut buf[done..], offset + done as u64) {
                Ok(0) => break,
                Ok(n) => done += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(Some(buf))
    }
}
