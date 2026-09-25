//! Chunked client file parsing: port of `src/tools/map_extractor/loadlib.cpp` and
//! `loadlib/loadlib.h` (`ChunkedFile`, `FileChunk`, `file_MVER`, `InterestingChunks`).
//!
//! Chunks keep the C++ semantics exactly:
//! * top-level scan (`ChunkedFile::parseChunks`) walks byte by byte, accepts only the
//!   ten "interesting" FourCCs, keeps a chunk when `size <= data_size` and always
//!   advances by `size + 8` (u32 arithmetic) after an interesting header;
//! * sub-chunk scan (`FileChunk::parseSubChunks`) starts 8 bytes into the chunk, runs
//!   while `ptr < data + size` (the chunk `size` excludes its 8-byte header, so the
//!   last 8 payload bytes are never scanned — faithful) and accepts `subsize < size`;
//! * `GetChunk` / `GetSubChunk` return a chunk only when exactly one chunk has the name;
//! * chunks with the same name keep file order (`std::multimap` insertion order).
//!
//! Chunk structs are read through [`FileData`], which returns zero for bytes past the
//! end of the file instead of reading past the heap buffer like the C++ `As<T>()`.

/// FourCCs stored reversed on disk (`u_map_fcc InterestingChunks[]`).
const INTERESTING_CHUNKS: [[u8; 4]; 10] = [
    *b"REVM", *b"NIAM", *b"O2HM", *b"KNCM", *b"TVCM", *b"OMWM", *b"QLCM", *b"OBFM", *b"DHPM",
    *b"DIAM",
];

/// `FILE_FORMAT_VERSION` (loadlib.h).
pub(crate) const FILE_FORMAT_VERSION: u32 = 18;

/// `IsInterestingChunk`.
fn is_interesting_chunk(fcc: [u8; 4]) -> bool {
    INTERESTING_CHUNKS.contains(&fcc)
}

/// Raw file bytes with zero-filled out-of-range little-endian reads.
#[derive(Debug, Clone)]
pub(crate) struct FileData(Vec<u8>);

impl FileData {
    fn byte(&self, pos: usize) -> u8 {
        self.0.get(pos).copied().unwrap_or(0)
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn u8_at(&self, pos: usize) -> u8 {
        self.byte(pos)
    }

    pub(crate) fn fcc_at(&self, pos: usize) -> [u8; 4] {
        [
            self.byte(pos),
            self.byte(pos.wrapping_add(1)),
            self.byte(pos.wrapping_add(2)),
            self.byte(pos.wrapping_add(3)),
        ]
    }

    pub(crate) fn u16_at(&self, pos: usize) -> u16 {
        u16::from_le_bytes([self.byte(pos), self.byte(pos.wrapping_add(1))])
    }

    pub(crate) fn i16_at(&self, pos: usize) -> i16 {
        self.u16_at(pos).cast_signed()
    }

    pub(crate) fn u32_at(&self, pos: usize) -> u32 {
        u32::from_le_bytes(self.fcc_at(pos))
    }

    pub(crate) fn u64_at(&self, pos: usize) -> u64 {
        u64::from(self.u32_at(pos)) | (u64::from(self.u32_at(pos.wrapping_add(4))) << 32)
    }

    pub(crate) fn f32_at(&self, pos: usize) -> f32 {
        f32::from_bits(self.u32_at(pos))
    }
}

/// `FileChunk`: a chunk located at `offset` (its FourCC header) inside the file.
#[derive(Debug, Clone)]
pub(crate) struct FileChunk {
    /// Offset of the chunk header (`FileChunk::data`) in the file.
    pub(crate) offset: usize,
    /// `FileChunk::size` — the size field read from the header (payload size).
    pub(crate) size: u32,
    /// `FileChunk::subchunks`, in file order.
    subchunks: Vec<(String, FileChunk)>,
}

impl FileChunk {
    /// `FileChunk::parseSubChunks` (one level: nested sub-chunks of sub-chunks are
    /// never queried by map_extractor, so they are not materialised).
    fn parse_sub_chunks(&mut self, file: &FileData) {
        let end = self.offset + self.size as usize;
        let mut ptr = self.offset + 8; // skip self
        while ptr < end {
            let header = file.fcc_at(ptr);
            if is_interesting_chunk(header) {
                let subsize = file.u32_at(ptr + 4);
                if subsize < self.size {
                    let chunk = FileChunk {
                        offset: ptr,
                        size: subsize,
                        subchunks: Vec::new(),
                    };
                    self.subchunks.push((fcc_name(header), chunk));
                }
                // move to next chunk
                ptr += subsize.wrapping_add(8) as usize;
            } else {
                ptr += 1;
            }
        }
    }

    /// `FileChunk::GetSubChunk`: the sub-chunk only when exactly one has this name.
    pub(crate) fn get_sub_chunk(&self, name: &str) -> Option<&FileChunk> {
        unique(&self.subchunks, name)
    }
}

/// Reversed on-disk FourCC -> readable name (`std::swap` of bytes 0/3 and 1/2).
fn fcc_name(mut fcc: [u8; 4]) -> String {
    fcc.reverse();
    String::from_utf8_lossy(&fcc).into_owned()
}

fn unique<'a>(chunks: &'a [(String, FileChunk)], name: &str) -> Option<&'a FileChunk> {
    let mut found = chunks.iter().filter(|(n, _)| n == name).map(|(_, c)| c);
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// `ChunkedFile`: a parsed ADT/WDT.
#[derive(Debug, Clone)]
pub(crate) struct ChunkedFile {
    pub(crate) data: FileData,
    /// `ChunkedFile::chunks`, in file order.
    chunks: Vec<(String, FileChunk)>,
}

impl ChunkedFile {
    /// `parseChunks` + `prepareLoadedData` on bytes already read from CASC (the
    /// `loadFile` overloads). Returns `None` when `prepareLoadedData` fails; the caller
    /// prints `Error loading <name>` like the C++.
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        let mut file = ChunkedFile {
            data: FileData(bytes),
            chunks: Vec::new(),
        };
        file.parse_chunks();
        file.prepare_loaded_data().then_some(file)
    }

    /// `ChunkedFile::parseChunks`.
    fn parse_chunks(&mut self) {
        let data_size = self.data.len();
        let mut ptr = 0usize;
        // Make sure there's enough data to read u_map_fcc struct and the uint32 size after it
        while ptr + 8 <= data_size {
            let header = self.data.fcc_at(ptr);
            if is_interesting_chunk(header) {
                let size = self.data.u32_at(ptr + 4);
                if size as usize <= data_size {
                    let mut chunk = FileChunk {
                        offset: ptr,
                        size,
                        subchunks: Vec::new(),
                    };
                    chunk.parse_sub_chunks(&self.data);
                    self.chunks.push((fcc_name(header), chunk));
                }
                // move to next chunk
                ptr += size.wrapping_add(8) as usize;
            } else {
                ptr += 1;
            }
        }
    }

    /// `ChunkedFile::prepareLoadedData`: exactly one `MVER` with version 18.
    fn prepare_loaded_data(&self) -> bool {
        let Some(chunk) = self.get_chunk("MVER") else {
            return false;
        };
        // Check version (`file_MVER::fcc` always matches: the chunk was found by it)
        if self.data.fcc_at(chunk.offset) != *b"REVM" {
            return false;
        }
        self.data.u32_at(chunk.offset + 8) == FILE_FORMAT_VERSION
    }

    /// `ChunkedFile::GetChunk`: the chunk only when exactly one has this name.
    pub(crate) fn get_chunk(&self, name: &str) -> Option<&FileChunk> {
        unique(&self.chunks, name)
    }

    /// `MapEqualRange(chunks, name)`: every chunk with this name, in file order.
    pub(crate) fn chunks_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a FileChunk> {
        self.chunks
            .iter()
            .filter(move |(n, _)| n == name)
            .map(|(_, c)| c)
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    /// Builds a chunk: reversed FourCC, u32 payload size, payload (takes `b"NAME"`).
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub(crate) fn chunk(name: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(payload.len() + 8);
        let mut fcc = name.to_owned();
        fcc.reverse();
        out.extend_from_slice(&fcc);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    pub(crate) fn mver() -> Vec<u8> {
        chunk(b"MVER", &18u32.to_le_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::{chunk, mver};
    use super::*;

    #[test]
    fn parses_interesting_chunks_and_skips_unknown_bytes() {
        let mut bytes = mver();
        bytes.extend_from_slice(&chunk(b"MHDR", &[0; 16])); // not interesting: skipped bytewise
        bytes.extend_from_slice(&chunk(b"MFBO", &[1; 36]));
        let file = ChunkedFile::from_bytes(bytes).expect("valid MVER");
        let mfbo = file.get_chunk("MFBO").expect("MFBO");
        assert_eq!(mfbo.size, 36);
        assert_eq!(file.data.u8_at(mfbo.offset + 8), 1);
        assert!(file.get_chunk("MHDR").is_none());
    }

    #[test]
    fn rejects_missing_or_wrong_version() {
        assert!(ChunkedFile::from_bytes(chunk(b"MFBO", &[0; 36])).is_none());
        assert!(ChunkedFile::from_bytes(chunk(b"MVER", &17u32.to_le_bytes())).is_none());
        // Two MVER chunks: GetChunk returns null -> prepareLoadedData fails.
        let mut two = mver();
        two.extend_from_slice(&mver());
        assert!(ChunkedFile::from_bytes(two).is_none());
        assert!(ChunkedFile::from_bytes(Vec::new()).is_none());
    }

    #[test]
    fn duplicate_names_are_not_returned_but_iterate_in_order() {
        let mut bytes = mver();
        bytes.extend_from_slice(&chunk(b"MCNK", &[0; 8]));
        bytes.extend_from_slice(&chunk(b"MCNK", &[1; 8]));
        let file = ChunkedFile::from_bytes(bytes).unwrap();
        assert!(file.get_chunk("MCNK").is_none());
        let offsets: Vec<_> = file.chunks_named("MCNK").map(|c| c.offset).collect();
        assert_eq!(offsets, vec![12, 28]);
    }

    #[test]
    fn sub_chunks_are_found_inside_a_chunk() {
        // MCNK with a 128-byte header of zeros, then MCVT, then MCLQ; plus 8 trailing bytes
        // (the scan stops at data + size, i.e. 8 bytes before the payload end).
        let mut payload = vec![0u8; 120];
        payload.extend_from_slice(&chunk(b"MCVT", &[2; 580]));
        payload.extend_from_slice(&chunk(b"MCLQ", &[3; 16]));
        payload.extend_from_slice(&[0; 8]);
        let mut bytes = mver();
        bytes.extend_from_slice(&chunk(b"MCNK", &payload));
        let file = ChunkedFile::from_bytes(bytes).unwrap();
        let mcnk = file.get_chunk("MCNK").unwrap();
        let mcvt = mcnk.get_sub_chunk("MCVT").expect("MCVT");
        assert_eq!(mcvt.offset, mcnk.offset + 8 + 120);
        assert_eq!(mcvt.size, 580);
        assert!(mcnk.get_sub_chunk("MCLQ").is_some());
        assert!(mcnk.get_sub_chunk("MH2O").is_none());
    }

    #[test]
    fn sub_chunk_must_be_smaller_than_parent() {
        // A sub-chunk header claiming size >= parent size is skipped (but still jumped over).
        let mut payload = Vec::new();
        payload.extend_from_slice(&chunk(b"MCVT", &[0; 4]));
        let size = (payload.len() as u32) + 100;
        payload[4..8].copy_from_slice(&size.to_le_bytes());
        payload.extend_from_slice(&[0; 8]);
        let mut bytes = mver();
        bytes.extend_from_slice(&chunk(b"MCNK", &payload));
        let file = ChunkedFile::from_bytes(bytes).unwrap();
        assert!(
            file.get_chunk("MCNK")
                .unwrap()
                .get_sub_chunk("MCVT")
                .is_none()
        );
    }

    #[test]
    fn out_of_range_reads_are_zero() {
        let data = FileData(vec![1, 2]);
        assert_eq!(data.u16_at(0), 0x0201);
        assert_eq!(data.u32_at(0), 0x0201);
        assert_eq!(data.u64_at(10), 0);
    }
}
