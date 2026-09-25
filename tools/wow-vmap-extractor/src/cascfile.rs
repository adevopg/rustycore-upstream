//! Port of `src/tools/vmap4_extractor/cascfile.{h,cpp}` (`CASCFile`, `flipcc`) plus the
//! parts of `extractor_common/CascHandles.cpp` (`CASC::Storage::OpenFile`,
//! `HumanReadableCASCError`) the vmap extractor goes through.
//!
//! The C++ `CASCFile` reads the whole file into memory and then exposes a `FILE*`-like
//! cursor (`read`, `seek`, `seekRelative`, `getPointer`, `isEof`). The quirks of that
//! cursor are part of the observable behavior of every chunk walker in the extractor
//! (partial reads at end of file set `eof` but still advance the pointer, a failed open
//! reports `eof` immediately, `close` sets `eof`), so they are reproduced exactly here.

use wow_casc::Storage;

/// Per-call locale mask passed to `wow-casc`. The C++ passes `CASC_LOCALE_ALL_WOW`
/// (`DB2CascFileSource`: `CASC_LOCALE_NONE`), which `CascOpenFile` ignores: the storage's
/// open mask decides. In `wow-casc` a mask of 0 is exactly that behavior.
const OPEN_LOCALE_MASK: u32 = 0;

/// CascLib `CASC_INVALID_ID`.
const CASC_INVALID_ID: u32 = 0xFFFF_FFFF;

/// Access to the files of a CASC storage, as used by the extractor.
///
/// Production code uses [`wow_casc::Storage`]; tests use in-memory maps. Errors are
/// already turned into the CascLib error names printed by the C++ tool
/// (`CASC::HumanReadableCASCError`).
pub trait CascSource {
    /// `CASC::Storage::OpenFile(char const* fileName, CASC_LOCALE_ALL_WOW)` followed by a
    /// full read. `Ok(None)` is CascLib `ERROR_FILE_NOT_FOUND`.
    fn open_by_name(&self, name: &str) -> Result<Option<Vec<u8>>, String>;

    /// `CASC::Storage::OpenFile(uint32 fileDataId, ...)` followed by a full read.
    /// `Ok(None)` is CascLib `ERROR_FILE_NOT_FOUND`.
    fn open_by_id(&self, file_data_id: u32) -> Result<Option<Vec<u8>>, String>;

    /// `DB2CascFileSource`: `OpenFile(fileDataId, CASC_LOCALE_NONE, printErrors,
    /// zerofillEncryptedParts = true)` followed by a full read; frames encrypted with an
    /// unknown key read as zeros.
    fn open_db2(&self, file_data_id: u32) -> Result<Option<Vec<u8>>, String>;

    /// `CASC::Storage::HasTactKey`.
    fn has_tact_key(&self, key_name: u64) -> bool;
}

/// CascLib `IsFileDataIdName` (`common/Common.cpp`): `CascOpenFile(CASC_OPEN_BY_NAME)`
/// accepts `File<decimal>.ext` and `FILE<8 hex digits>[.ext]` names as FileDataIDs.
/// The vmap extractor relies on this for every `FILE%08X.xxx` name it generates.
pub fn file_data_id_from_name(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    if let Some(rest) = bytes.strip_prefix(b"File") {
        let mut acc: u32 = 0;
        let mut i = 0;
        while i < rest.len() && rest[i] != b'.' {
            if !rest[i].is_ascii_digit() {
                break;
            }
            acc = acc.wrapping_mul(10).wrapping_add(u32::from(rest[i] - b'0'));
            i += 1;
        }
        if i == rest.len() || rest[i] == b'.' {
            return Some(acc);
        }
    }

    if bytes.starts_with(b"FILE") && bytes.len() >= 0x0C {
        let hex = &bytes[4..12];
        if hex.iter().all(u8::is_ascii_hexdigit) {
            // ConvertBytesToInteger_4 of the big-endian binary value.
            let text = std::str::from_utf8(hex).ok()?;
            let id = u32::from_str_radix(text, 16).ok()?;
            if bytes.len() == 0x0C || bytes[0x0C] == b'.' {
                return (id != CASC_INVALID_ID).then_some(id);
            }
        }
    }
    None
}

/// `CASC::HumanReadableCASCError` for the errors `wow-casc` can report.
fn human_readable_casc_error(error: &wow_casc::Error) -> String {
    match error {
        wow_casc::Error::MissingKey(_) => "FILE_ENCRYPTED".to_owned(),
        wow_casc::Error::Corrupt(_) => "FILE_CORRUPT".to_owned(),
        other => other.to_string(),
    }
}

impl CascSource for Storage {
    fn open_by_name(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        // CascOpenFile tries the root name hash first and then `IsFileDataIdName`; the
        // generated `FILE%08X.xxx` names never have a name hash, so resolving them as
        // FileDataIDs first is equivalent.
        let result = match file_data_id_from_name(name) {
            Some(id) => self.read_file_by_id(id, OPEN_LOCALE_MASK),
            None => self.read_file_by_name(name, OPEN_LOCALE_MASK),
        };
        result.map_err(|e| human_readable_casc_error(&e))
    }

    fn open_by_id(&self, file_data_id: u32) -> Result<Option<Vec<u8>>, String> {
        self.read_file_by_id(file_data_id, OPEN_LOCALE_MASK)
            .map_err(|e| human_readable_casc_error(&e))
    }

    fn open_db2(&self, file_data_id: u32) -> Result<Option<Vec<u8>>, String> {
        self.read_file_by_id_zerofill_encrypted(file_data_id, OPEN_LOCALE_MASK)
            .map_err(|e| human_readable_casc_error(&e))
    }

    fn has_tact_key(&self, key_name: u64) -> bool {
        Storage::has_tact_key(self, key_name)
    }
}

/// Port of `CASCFile`.
pub struct CascFile {
    eof: bool,
    buffer: Vec<u8>,
    pointer: usize,
}

impl CascFile {
    /// `CASCFile(casc, filename, warnNoExist)`.
    pub fn open_name(source: &dyn CascSource, filename: &str, warn_no_exist: bool) -> Self {
        Self::from_result(source.open_by_name(filename), filename, warn_no_exist)
    }

    /// `CASCFile(casc, fileDataId, description, warnNoExist)`.
    pub fn open_id(
        source: &dyn CascSource,
        file_data_id: u32,
        description: &str,
        warn_no_exist: bool,
    ) -> Self {
        Self::from_result(source.open_by_id(file_data_id), description, warn_no_exist)
    }

    fn from_result(
        result: Result<Option<Vec<u8>>, String>,
        description: &str,
        warn_no_exist: bool,
    ) -> Self {
        match result {
            Ok(Some(buffer)) => Self::from_bytes(buffer),
            Ok(None) => {
                if warn_no_exist {
                    eprintln!("Can't open {description}: FILE_NOT_FOUND");
                }
                Self::failed()
            }
            Err(error) => {
                eprintln!("Can't open {description}: {error}");
                Self::failed()
            }
        }
    }

    /// A file whose content was read successfully (`CASCFile::init`).
    pub fn from_bytes(buffer: Vec<u8>) -> Self {
        Self {
            eof: false,
            buffer,
            pointer: 0,
        }
    }

    fn failed() -> Self {
        Self {
            eof: true,
            buffer: Vec::new(),
            pointer: 0,
        }
    }

    /// `CASCFile::read`: copies up to `dest.len()` bytes; a read crossing the end copies
    /// the remaining bytes, sets `eof` and still moves the pointer by the full amount.
    pub fn read(&mut self, dest: &mut [u8]) -> usize {
        if self.eof {
            return 0;
        }
        let mut bytes = dest.len();
        let rpos = self.pointer + bytes;
        if rpos > self.buffer.len() {
            bytes = self.buffer.len() - self.pointer;
            self.eof = true;
        }
        dest[..bytes].copy_from_slice(&self.buffer[self.pointer..self.pointer + bytes]);
        self.pointer = rpos;
        bytes
    }

    /// Reads a little-endian `u32` over `value` (bytes not read keep their old value,
    /// like `f.read(&value, 4)` in C++).
    pub fn read_u32(&mut self, value: &mut u32) {
        let mut b = value.to_le_bytes();
        self.read(&mut b);
        *value = u32::from_le_bytes(b);
    }

    pub fn read_i32(&mut self, value: &mut i32) {
        let mut b = value.to_le_bytes();
        self.read(&mut b);
        *value = i32::from_le_bytes(b);
    }

    pub fn read_u16(&mut self, value: &mut u16) {
        let mut b = value.to_le_bytes();
        self.read(&mut b);
        *value = u16::from_le_bytes(b);
    }

    pub fn read_f32(&mut self, value: &mut f32) {
        let mut b = value.to_le_bytes();
        self.read(&mut b);
        *value = f32::from_le_bytes(b);
    }

    /// Reads `count` bytes into a zero-initialized vector (C++ `make_unique<T[]>` or a
    /// buffer the extractor only reads back what was filled).
    pub fn read_vec(&mut self, count: usize) -> Vec<u8> {
        let mut v = vec![0u8; count];
        self.read(&mut v);
        v
    }

    pub fn get_size(&self) -> usize {
        self.buffer.len()
    }

    pub fn get_pos(&self) -> usize {
        self.pointer
    }

    pub fn get_buffer(&self) -> &[u8] {
        &self.buffer
    }

    pub fn is_eof(&self) -> bool {
        self.eof
    }

    /// `CASCFile::seek(int offset)`.
    pub fn seek(&mut self, offset: usize) {
        self.pointer = offset;
        self.eof = self.pointer >= self.buffer.len();
    }

    /// `CASCFile::seekRelative(int offset)`; the offset is a C `int`, so values above
    /// `i32::MAX` wrap to negative and the `size_t` pointer wraps around.
    pub fn seek_relative(&mut self, offset: i32) {
        self.pointer = self.pointer.wrapping_add(offset as isize as usize);
        self.eof = self.pointer >= self.buffer.len();
    }

    /// `CASCFile::close`.
    pub fn close(&mut self) {
        self.buffer = Vec::new();
        self.eof = true;
    }
}

/// Reads a chunk header (`f.read(fourcc, 4); f.read(&size, 4); flipcc(fourcc);`).
/// `fourcc`/`size` keep bytes that could not be read, as the C++ locals do.
pub fn read_chunk_header(f: &mut CascFile, fourcc: &mut [u8; 4], size: &mut u32) {
    f.read(fourcc);
    f.read_u32(size);
    flipcc(fourcc);
}

/// `flipcc`: chunk ids are stored reversed (`REVM` for `MVER`).
pub fn flipcc(fcc: &mut [u8; 4]) {
    fcc.swap(0, 3);
    fcc.swap(1, 2);
}

/// `strlen`-style C string at the start of `bytes` (up to the first NUL or the end).
pub fn c_str(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    &bytes[..end]
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::collections::HashMap;

    /// In-memory CASC storage: names (including `FILE%08X.xxx`) and FileDataIDs.
    #[derive(Default)]
    pub struct MemSource {
        pub by_name: HashMap<String, Vec<u8>>,
        pub by_id: HashMap<u32, Vec<u8>>,
    }

    impl CascSource for MemSource {
        fn open_by_name(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
            if let Some(id) = file_data_id_from_name(name) {
                return self.open_by_id(id);
            }
            Ok(self.by_name.get(name).cloned())
        }

        fn open_by_id(&self, file_data_id: u32) -> Result<Option<Vec<u8>>, String> {
            Ok(self.by_id.get(&file_data_id).cloned())
        }

        fn open_db2(&self, file_data_id: u32) -> Result<Option<Vec<u8>>, String> {
            self.open_by_id(file_data_id)
        }

        fn has_tact_key(&self, _key_name: u64) -> bool {
            false
        }
    }

    #[test]
    fn file_data_id_names_follow_casclib() {
        assert_eq!(file_data_id_from_name("FILE0001ABCD.xxx"), Some(0x1ABCD));
        assert_eq!(file_data_id_from_name("FILE0001abcd"), Some(0x1ABCD));
        assert_eq!(file_data_id_from_name("File123.m2"), Some(123));
        assert_eq!(file_data_id_from_name("FILEFFFFFFFF.xxx"), None);
        assert_eq!(file_data_id_from_name("FILE0001ABCDx"), None);
        assert_eq!(file_data_id_from_name("World\\Maps\\A\\A.wdt"), None);
    }

    #[test]
    fn partial_read_sets_eof_and_advances_pointer() {
        let mut f = CascFile::from_bytes(vec![1, 2, 3]);
        let mut v = 0xAABB_CCDD_u32;
        f.read_u32(&mut v);
        assert!(f.is_eof());
        assert_eq!(f.get_pos(), 4);
        // bytes that were not read keep their previous value
        assert_eq!(v, 0xAA03_0201);
        f.seek(1);
        assert!(!f.is_eof());
        f.seek_relative(5);
        assert!(f.is_eof());
    }
}
