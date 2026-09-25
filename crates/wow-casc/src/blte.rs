//! BLTE decoding of one archive entry.
//!
//! Port of `CascLib` `dep/CascLib/src/CascReadFile.cpp`: `ParseBlteHeader`,
//! `CaptureBlteFileFrame`, `LoadSpanFrames`, `ReadFile_WholeFile`,
//! `DecodeFileFrame`; `CascDecompress.cpp`: `CascDecompress`;
//! `CascDecrypt.cpp`: `CascDecrypt`, `CascDirectCopy`.
//!
//! An entry in a local `data.###` archive starts with the 0x1E-byte
//! `BLTE_ENCODED_HEADER` prefix (`EKey`, size, flags, checksums), followed by the
//! `BLTE` header, the optional frame table and the frames. Loose (CDN style)
//! files start directly with `BLTE`.

use flate2::{Decompress, FlushDecompress, Status};

use crate::config::verify_md5;
use crate::keys::KeyMap;
use crate::salsa20;
use crate::{Error, Result};

/// `BLTE_HEADER_SIGNATURE` ('BLTE').
const BLTE_SIGNATURE: &[u8; 4] = b"BLTE";
/// `FIELD_OFFSET(BLTE_ENCODED_HEADER, Signature)`.
pub const BLTE_HEADER_DELTA: usize = 0x1E;
/// `sizeof(BLTE_FRAME)`.
const BLTE_FRAME_SIZE: usize = 0x18;

/// One frame of a BLTE stream (`CASC_FILE_FRAME`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub encoded_size: u32,
    pub content_size: u32,
    pub hash: [u8; 16],
}

/// Result of `ParseBlteHeader` + `LoadSpanFrames`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlteLayout {
    /// Offset of the first frame's data inside the entry
    /// (`CASC_FILE_SPAN::HeaderSize`).
    pub header_size: usize,
    pub frames: Vec<Frame>,
}

fn be_u32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0u32, |acc, &b| (acc << 8) | u32::from(b))
}

/// `ParseBlteHeader` + `LoadSpanFrames`. `entry` is the whole encoded entry
/// (`EncodedSize` bytes); `content_size` is the content size from ENCODING,
/// required for single-frame files ("dummy" frame).
pub fn parse_layout(entry: &[u8], content_size: Option<u32>) -> Result<BlteLayout> {
    let bad = |what: &str| Error::Corrupt(format!("BLTE: {what}"));
    let mut ex_header_size = 0;
    if entry.get(..4) != Some(BLTE_SIGNATURE) {
        // "There must be at least some bytes"
        if entry.len() < BLTE_HEADER_DELTA + 8 {
            return Err(bad("entry too short"));
        }
        // "Do NOT test anything else than the signature" (encoded header may be garbage)
        if &entry[BLTE_HEADER_DELTA..BLTE_HEADER_DELTA + 4] != BLTE_SIGNATURE {
            return Err(bad("missing BLTE signature"));
        }
        ex_header_size = BLTE_HEADER_DELTA;
    }
    let blte = &entry[ex_header_size..];
    if blte.len() < 8 {
        return Err(bad("header truncated"));
    }
    let header_size = be_u32(&blte[4..8]) as usize;
    if header_size == 0 {
        // Single frame: the rest of the entry, content size from ENCODING.
        let data_start = ex_header_size + 8;
        let content_size =
            content_size.ok_or_else(|| bad("single-frame file with unknown content size"))?;
        return Ok(BlteLayout {
            header_size: data_start,
            frames: vec![Frame {
                encoded_size: (entry.len() - data_start) as u32,
                content_size,
                hash: [0; 16],
            }],
        });
    }

    if blte.len() < 12 {
        return Err(bad("frame header truncated"));
    }
    if blte[8] != 0x0F {
        return Err(bad("MustBe0F mismatch"));
    }
    let frame_count = be_u32(&blte[9..12]) as usize;
    if 0x0C + frame_count * BLTE_FRAME_SIZE != header_size {
        return Err(bad("header size does not match frame count"));
    }
    let table_start = ex_header_size + 12;
    let table = entry
        .get(table_start..table_start + frame_count * BLTE_FRAME_SIZE)
        .ok_or_else(|| bad("frame table truncated"))?;
    let frames = table
        .as_chunks::<BLTE_FRAME_SIZE>()
        .0
        .iter()
        .map(|f| Frame {
            encoded_size: be_u32(&f[0..4]),
            content_size: be_u32(&f[4..8]),
            hash: f[8..24].try_into().expect("16 bytes"),
        })
        .collect();
    Ok(BlteLayout {
        header_size: table_start + frame_count * BLTE_FRAME_SIZE,
        frames,
    })
}

/// Options of a decode, mirroring `CascLib` file-handle flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct DecodeOptions {
    /// `bVerifyIntegrity` (`CASC_STRICT_DATA_CHECK`): check each frame's MD5.
    pub verify_frames: bool,
    /// `bOvercomeEncrypted` (`CASC_OVERCOME_ENCRYPTED`): zero-fill frames
    /// whose key is missing instead of failing.
    pub overcome_encrypted: bool,
}

/// Decodes a whole archive entry (`ReadFile_WholeFile`). `content_size` is the
/// file size from ENCODING (or the build config); the output has exactly that
/// length. When it is unknown (`CASC_INVALID_SIZE`), the frame sizes decide
/// (`LoadSpanFrames` fills `ContentSize` from the frames).
pub fn decode_entry(
    entry: &[u8],
    content_size: Option<u32>,
    keys: &KeyMap,
    options: DecodeOptions,
) -> Result<Vec<u8>> {
    let layout = parse_layout(entry, content_size)?;
    let total: u64 = layout
        .frames
        .iter()
        .map(|f| u64::from(f.content_size))
        .sum();
    let mut out = vec![0u8; total as usize];
    let mut in_pos = layout.header_size;
    let mut out_pos = 0usize;
    for (index, frame) in layout.frames.iter().enumerate() {
        let enc_end = in_pos
            .checked_add(frame.encoded_size as usize)
            .filter(|&end| end <= entry.len())
            .ok_or_else(|| Error::Corrupt("BLTE: frame exceeds entry".into()))?;
        let encoded = &entry[in_pos..enc_end];
        let decoded = &mut out[out_pos..out_pos + frame.content_size as usize];
        decode_frame(encoded, decoded, frame, index as u32, keys, options)?;
        in_pos = enc_end;
        out_pos += frame.content_size as usize;
    }
    // The file size is the ENCODING content size (TCascFile::ContentSize);
    // CascReadFile never returns bytes past it.
    if let Some(content_size) = content_size {
        if out.len() < content_size as usize {
            return Err(Error::Corrupt(format!(
                "BLTE: frames hold {} bytes, expected {content_size}",
                out.len()
            )));
        }
        out.truncate(content_size as usize);
    }
    Ok(out)
}

/// `DecodeFileFrame`.
fn decode_frame(
    encoded: &[u8],
    decoded: &mut [u8],
    frame: &Frame,
    frame_index: u32,
    keys: &KeyMap,
    options: DecodeOptions,
) -> Result<()> {
    if options.verify_frames && !verify_md5(encoded, &frame.hash) {
        return Err(Error::Corrupt(format!(
            "BLTE: frame {frame_index} hash mismatch"
        )));
    }
    match decode_frame_steps(encoded, decoded, frame_index, keys) {
        // "We overcome missing decryption key by zeroing the encrypted portions"
        Err(Error::MissingKey(_)) if options.overcome_encrypted => {
            decoded.fill(0);
            Ok(())
        }
        other => other,
    }
}

fn decode_frame_steps(
    encoded: &[u8],
    decoded: &mut [u8],
    frame_index: u32,
    keys: &KeyMap,
) -> Result<()> {
    let empty = || Error::Corrupt("BLTE: empty frame".into());
    let (&first_mode, first_payload) = encoded.split_first().ok_or_else(empty)?;
    // 'E' is always followed by exactly one more step on the decrypted data
    // ("There should never be a 3rd step").
    let decrypted;
    let (mode, payload) = if first_mode == b'E' {
        decrypted = decrypt(keys, first_payload, frame_index)?;
        let (&mode, payload) = decrypted.split_first().ok_or_else(empty)?;
        if mode == b'E' {
            return Err(Error::Corrupt("BLTE: nested encryption".into()));
        }
        (mode, payload)
    } else {
        (first_mode, first_payload)
    };

    match mode {
        b'Z' => {
            // CascDecompress; a short output is zero-filled.
            let written = inflate_into(payload, decoded)?;
            decoded[written..].fill(0);
            Ok(())
        }
        b'N' => {
            // CascDirectCopy
            if payload.len().saturating_sub(1) > decoded.len() {
                return Err(Error::Corrupt("BLTE: 'N' frame larger than content".into()));
            }
            let n = payload.len().min(decoded.len());
            decoded[..n].copy_from_slice(&payload[..n]);
            decoded[n..].fill(0);
            Ok(())
        }
        // 'F' (recursive frames) and anything else: ERROR_NOT_SUPPORTED
        other => Err(Error::Corrupt(format!(
            "BLTE: unsupported frame mode {:?}",
            other as char
        ))),
    }
}

/// `CascDecompress`: one zlib inflate into a buffer of the expected size.
/// Returns the number of bytes produced.
fn inflate_into(input: &[u8], output: &mut [u8]) -> Result<usize> {
    let mut z = Decompress::new(true);
    loop {
        let in_pos = z.total_in() as usize;
        let out_pos = z.total_out() as usize;
        let status = z
            .decompress(
                &input[in_pos..],
                &mut output[out_pos..],
                FlushDecompress::None,
            )
            .map_err(|e| Error::Corrupt(format!("BLTE: zlib: {e}")))?;
        let progressed = z.total_in() as usize != in_pos || z.total_out() as usize != out_pos;
        match status {
            Status::StreamEnd => break,
            _ if z.total_out() as usize == output.len() || !progressed => break,
            _ => {}
        }
    }
    Ok(z.total_out() as usize)
}

/// `CascDecrypt`: decrypts the payload of an 'E' frame (after the mode byte).
fn decrypt(keys: &KeyMap, input: &[u8], frame_index: u32) -> Result<Vec<u8>> {
    let corrupt = || Error::Corrupt("BLTE: truncated encryption header".into());
    let unsupported = |what: &str| Error::Corrupt(format!("BLTE: unsupported encryption {what}"));
    let end = input.len();
    let mut pos = 0;

    // Key name size (0 or 8) and key name (little-endian u64)
    if pos >= end {
        return Err(corrupt());
    }
    let key_name_size = input[pos] as usize;
    if key_name_size != 0 && key_name_size != 8 {
        return Err(unsupported("key name size"));
    }
    pos += 1;
    if pos + key_name_size >= end {
        return Err(corrupt());
    }
    let mut name_bytes = [0u8; 8];
    name_bytes[..key_name_size].copy_from_slice(&input[pos..pos + key_name_size]);
    let key_name = u64::from_le_bytes(name_bytes);
    pos += key_name_size;

    // IV size (4 or 8) and IV, zero-padded to 8 bytes
    if pos >= end {
        return Err(corrupt());
    }
    let iv_size = input[pos] as usize;
    if iv_size != 4 && iv_size != 8 {
        return Err(unsupported("IV size"));
    }
    pos += 1;
    if pos + iv_size >= end {
        return Err(corrupt());
    }
    let mut vector = [0u8; 8];
    vector[..iv_size].copy_from_slice(&input[pos..pos + iv_size]);
    pos += iv_size;

    // Encryption type
    if pos >= end {
        return Err(corrupt());
    }
    let encryption_type = input[pos];
    if encryption_type != b'S' && encryption_type != b'A' {
        return Err(unsupported("type"));
    }
    pos += 1;

    let Some(key) = keys.find(key_name) else {
        return Err(Error::MissingKey(key_name));
    };

    // "Shuffle the Vector with the block index"
    for (i, b) in frame_index.to_le_bytes().iter().enumerate() {
        vector[i] ^= b;
    }

    match encryption_type {
        b'S' => {
            let mut out = input[pos..].to_vec();
            salsa20::apply_keystream(key, vector, &mut out);
            Ok(out)
        }
        // CascLib has no ARC4 implementation ('A' ends in assert/NOT_SUPPORTED).
        _ => Err(unsupported("type 'A' (ARC4)")),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    pub(crate) fn zlib(data: &[u8]) -> Vec<u8> {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    /// Wraps BLTE bytes into an archive entry with a (garbage) 0x1E-byte
    /// encoded header.
    pub(crate) fn archive_entry(blte: &[u8]) -> Vec<u8> {
        let mut out = vec![0xAB; BLTE_HEADER_DELTA];
        out.extend_from_slice(blte);
        out
    }

    /// Builds a multi-frame BLTE stream from encoded frames (mode byte included).
    pub(crate) fn blte_multi(frames: &[(Vec<u8>, u32)]) -> Vec<u8> {
        let mut out = b"BLTE".to_vec();
        out.extend_from_slice(&((0x0C + frames.len() * 0x18) as u32).to_be_bytes());
        out.push(0x0F);
        out.extend_from_slice(&(frames.len() as u32).to_be_bytes()[1..]);
        for (enc, content) in frames {
            out.extend_from_slice(&(enc.len() as u32).to_be_bytes());
            out.extend_from_slice(&content.to_be_bytes());
            out.extend_from_slice(&md5::Md5::digest(enc));
        }
        for (enc, _) in frames {
            out.extend_from_slice(enc);
        }
        out
    }

    use md5::Digest;

    #[test]
    fn single_frame_raw() {
        let mut blte = b"BLTE\0\0\0\0N".to_vec();
        blte.extend_from_slice(b"hello");
        let entry = archive_entry(&blte);
        let keys = KeyMap::with_static_keys();
        let out = decode_entry(&entry, Some(5), &keys, DecodeOptions::default()).unwrap();
        assert_eq!(out, b"hello");
        // Loose file without the encoded header.
        let out = decode_entry(&blte, Some(5), &keys, DecodeOptions::default()).unwrap();
        assert_eq!(out, b"hello");
    }

    #[test]
    fn multi_frame_zlib_and_raw_with_verification() {
        let part1: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let part2 = b"tail".to_vec();
        let mut f1 = vec![b'Z'];
        f1.extend_from_slice(&zlib(&part1));
        let mut f2 = vec![b'N'];
        f2.extend_from_slice(&part2);
        let blte = blte_multi(&[(f1, 5000), (f2, 4)]);
        let entry = archive_entry(&blte);
        let keys = KeyMap::with_static_keys();
        let opts = DecodeOptions {
            verify_frames: true,
            overcome_encrypted: false,
        };
        let out = decode_entry(&entry, None, &keys, opts).unwrap();
        assert_eq!(&out[..5000], &part1[..]);
        assert_eq!(&out[5000..], b"tail");

        let layout = parse_layout(&entry, None).unwrap();
        assert_eq!(layout.frames.len(), 2);
        assert_eq!(layout.header_size, BLTE_HEADER_DELTA + 12 + 2 * 0x18);

        // Corrupt one byte of frame 2 data -> hash mismatch under strict check.
        let mut broken = entry.clone();
        *broken.last_mut().unwrap() ^= 1;
        assert!(decode_entry(&broken, Some(5004), &keys, opts).is_err());
    }

    #[test]
    fn bad_headers_are_rejected() {
        let keys = KeyMap::with_static_keys();
        assert!(decode_entry(&[0u8; 64], Some(1), &keys, DecodeOptions::default()).is_err());
        let mut blte = b"BLTE\0\0\0\x24\x0E\0\0\x01".to_vec();
        blte.extend_from_slice(&[0u8; 0x18]);
        assert!(parse_layout(&blte, Some(0)).is_err(), "MustBe0F");
    }

    /// Builds an encrypted 'E' frame around `inner` (mode byte included).
    pub(crate) fn encrypt_frame(
        key_name: u64,
        key: &[u8; 16],
        iv: [u8; 4],
        frame_index: u32,
        inner: &[u8],
    ) -> Vec<u8> {
        let mut out = vec![b'E', 8];
        out.extend_from_slice(&key_name.to_le_bytes());
        out.push(4);
        out.extend_from_slice(&iv);
        out.push(b'S');
        let mut vector = [0u8; 8];
        vector[..4].copy_from_slice(&iv);
        for (i, b) in frame_index.to_le_bytes().iter().enumerate() {
            vector[i] ^= b;
        }
        let mut payload = inner.to_vec();
        salsa20::apply_keystream(key, vector, &mut payload);
        out.extend_from_slice(&payload);
        out
    }

    #[test]
    fn encrypted_frames() {
        let mut keys = KeyMap::with_static_keys();
        let key_name = 0x1122_3344_5566_7788;
        let key = [7u8; 16];
        let mut inner0 = vec![b'Z'];
        inner0.extend_from_slice(&zlib(b"first frame"));
        let inner1 = b"Nsecond".to_vec();
        let f0 = encrypt_frame(key_name, &key, [1, 2, 3, 4], 0, &inner0);
        let f1 = encrypt_frame(key_name, &key, [1, 2, 3, 4], 1, &inner1);
        let entry = archive_entry(&blte_multi(&[(f0, 11), (f1, 6)]));

        // Missing key -> MissingKey(name), or zeros when overcoming.
        match decode_entry(&entry, Some(17), &keys, DecodeOptions::default()) {
            Err(Error::MissingKey(name)) => assert_eq!(name, key_name),
            other => panic!("unexpected {other:?}"),
        }
        let zeroed = decode_entry(
            &entry,
            Some(17),
            &keys,
            DecodeOptions {
                verify_frames: false,
                overcome_encrypted: true,
            },
        )
        .unwrap();
        assert_eq!(zeroed, vec![0u8; 17]);

        assert!(keys.add(key_name, key));
        let out = decode_entry(&entry, Some(17), &keys, DecodeOptions::default()).unwrap();
        assert_eq!(out, b"first framesecond");
    }

    #[test]
    fn short_zlib_output_is_zero_filled() {
        let mut blte = b"BLTE\0\0\0\0Z".to_vec();
        blte.extend_from_slice(&zlib(b"abc"));
        let keys = KeyMap::with_static_keys();
        let out = decode_entry(&blte, Some(6), &keys, DecodeOptions::default()).unwrap();
        assert_eq!(out, b"abc\0\0\0");
    }
}
