//! BLTE containers as served by the CDN: `EKey` computation/verification and
//! decoding of the `N` (raw) and `Z` (zlib) frame modes.
//!
//! Layout per wowdev "BLTE" and `CascLib` `CascReadFile.cpp: ParseBlteHeader`
//! (`HeaderSize` counts from the `BLTE` signature, `0x0F` + 24-bit frame count,
//! 24-byte frame records `{EncodedSize BE, ContentSize BE, MD5}`); decoding
//! mirrors blizzget `NGDP::DecodeBLTE`. The `EKey` of a blob is the MD5 of its
//! BLTE header (signature through frame table) when it has frames, else the
//! MD5 of the whole blob; each frame's MD5 is checked as well so a verified
//! blob is fully covered.
//!
//! Encrypted (`E`) frames are not decoded here: the downloader only decodes
//! manifests and the loose install files, which are never encrypted; archive
//! entries are stored encoded, as the client expects.

use std::io::Read;

use anyhow::{Context, Result, bail, ensure};

use crate::util::{Key, be_uint, hex, md5};

const SIGNATURE: &[u8; 4] = b"BLTE";
const FRAME_RECORD: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub encoded_size: u32,
    /// 0 for the implicit frame of a frameless blob (size unknown).
    pub content_size: u32,
    pub hash: Option<Key>,
}

/// Parsed header: offset of the first frame's data and the frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub data_start: usize,
    pub frames: Vec<Frame>,
    /// `true` when the blob has a frame table (`HeaderSize != 0`).
    pub framed: bool,
}

pub fn parse_layout(blob: &[u8]) -> Result<Layout> {
    ensure!(
        blob.len() >= 8 && &blob[..4] == SIGNATURE,
        "missing BLTE signature"
    );
    let header_size = be_uint(&blob[4..8]) as usize;
    if header_size == 0 {
        return Ok(Layout {
            data_start: 8,
            frames: vec![Frame {
                encoded_size: (blob.len() - 8) as u32,
                content_size: 0,
                hash: None,
            }],
            framed: false,
        });
    }
    ensure!(
        blob.len() >= header_size && header_size >= 12,
        "BLTE header truncated"
    );
    ensure!(blob[8] == 0x0F, "BLTE header flags byte is not 0x0F");
    let count = be_uint(&blob[9..12]) as usize;
    ensure!(
        12 + count * FRAME_RECORD == header_size,
        "BLTE header size does not match its frame count"
    );
    let frames = blob[12..header_size]
        .as_chunks::<FRAME_RECORD>()
        .0
        .iter()
        .map(|f| Frame {
            encoded_size: be_uint(&f[0..4]) as u32,
            content_size: be_uint(&f[4..8]) as u32,
            hash: Some(f[8..24].try_into().expect("16 bytes")),
        })
        .collect();
    Ok(Layout {
        data_start: header_size,
        frames,
        framed: true,
    })
}

/// The `EKey` of a blob (MD5 of the header, or of the whole frameless blob).
#[cfg(test)]
pub fn encoded_key(blob: &[u8]) -> Result<Key> {
    let layout = parse_layout(blob)?;
    Ok(if layout.framed {
        md5(&blob[..layout.data_start])
    } else {
        md5(blob)
    })
}

/// Checks that `blob` is exactly the entry `ekey` names: header MD5, frame
/// hashes and total length.
pub fn verify(blob: &[u8], ekey: &Key) -> Result<()> {
    let layout = parse_layout(blob)?;
    let actual = if layout.framed {
        md5(&blob[..layout.data_start])
    } else {
        md5(blob)
    };
    ensure!(
        &actual == ekey,
        "EKey mismatch: expected {}, got {}",
        hex(ekey),
        hex(&actual)
    );
    let mut pos = layout.data_start;
    for (i, frame) in layout.frames.iter().enumerate() {
        let end = pos + frame.encoded_size as usize;
        ensure!(end <= blob.len(), "BLTE frame {i} exceeds the blob");
        if let Some(hash) = frame.hash {
            ensure!(md5(&blob[pos..end]) == hash, "BLTE frame {i} hash mismatch");
        }
        pos = end;
    }
    ensure!(pos == blob.len(), "BLTE blob has trailing bytes");
    Ok(())
}

/// Decodes the blob's content.
pub fn decode(blob: &[u8]) -> Result<Vec<u8>> {
    let layout = parse_layout(blob)?;
    let total: usize = layout.frames.iter().map(|f| f.content_size as usize).sum();
    let mut out = Vec::with_capacity(total);
    let mut pos = layout.data_start;
    for (i, frame) in layout.frames.iter().enumerate() {
        let end = pos + frame.encoded_size as usize;
        ensure!(end <= blob.len(), "BLTE frame {i} exceeds the blob");
        let start_len = out.len();
        decode_frame(&blob[pos..end], &mut out).with_context(|| format!("BLTE frame {i}"))?;
        if layout.framed {
            ensure!(
                out.len() - start_len == frame.content_size as usize,
                "BLTE frame {i} decoded to {} bytes, expected {}",
                out.len() - start_len,
                frame.content_size
            );
        }
        pos = end;
    }
    Ok(out)
}

fn decode_frame(frame: &[u8], out: &mut Vec<u8>) -> Result<()> {
    let (&mode, payload) = frame.split_first().context("empty frame")?;
    match mode {
        b'N' => out.extend_from_slice(payload),
        b'Z' => {
            flate2::read::ZlibDecoder::new(payload)
                .read_to_end(out)
                .context("zlib")?;
        }
        b'E' => bail!("encrypted frame (not supported for this file)"),
        other => bail!("unsupported frame mode {:?}", other as char),
    }
    Ok(())
}

/// Builds a BLTE blob from `(mode, content)` frames (`N` or `Z`). With one
/// frame and `framed == false` the blob has no frame table. Used by the tests
/// of every module that consumes BLTE.
#[cfg(test)]
pub fn encode(frames: &[(u8, &[u8])], with_table: bool) -> Vec<u8> {
    use std::io::Write;
    let encoded: Vec<Vec<u8>> = frames
        .iter()
        .map(|(mode, content)| {
            let mut f = vec![*mode];
            if *mode == b'Z' {
                let mut z =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                z.write_all(content).unwrap();
                f.extend_from_slice(&z.finish().unwrap());
            } else {
                f.extend_from_slice(content);
            }
            f
        })
        .collect();
    let mut out = SIGNATURE.to_vec();
    if !with_table {
        assert_eq!(frames.len(), 1);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&encoded[0]);
        return out;
    }
    out.extend_from_slice(&((12 + frames.len() * FRAME_RECORD) as u32).to_be_bytes());
    out.push(0x0F);
    out.extend_from_slice(&(frames.len() as u32).to_be_bytes()[1..]);
    for (enc, (_, content)) in encoded.iter().zip(frames) {
        out.extend_from_slice(&(enc.len() as u32).to_be_bytes());
        out.extend_from_slice(&(content.len() as u32).to_be_bytes());
        out.extend_from_slice(&md5(enc));
    }
    for enc in &encoded {
        out.extend_from_slice(enc);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framed_round_trip_and_verification() {
        let a: Vec<u8> = (0..10_000u32).map(|i| (i % 253) as u8).collect();
        let blob = encode(&[(b'Z', &a), (b'N', b"tail")], true);
        let ekey = encoded_key(&blob).unwrap();
        let layout = parse_layout(&blob).unwrap();
        assert_eq!(ekey, md5(&blob[..layout.data_start]));
        verify(&blob, &ekey).unwrap();
        let out = decode(&blob).unwrap();
        assert_eq!(&out[..10_000], &a[..]);
        assert_eq!(&out[10_000..], b"tail");

        // A corrupt data byte keeps the header (EKey) but fails the frame hash.
        let mut broken = blob.clone();
        *broken.last_mut().unwrap() ^= 1;
        assert_eq!(encoded_key(&broken).unwrap(), ekey);
        assert!(verify(&broken, &ekey).is_err());
        assert!(verify(&blob[..blob.len() - 1], &ekey).is_err());
        assert!(verify(&blob, &[0; 16]).is_err());
    }

    #[test]
    fn frameless_blobs() {
        let blob = encode(&[(b'Z', b"hello world")], false);
        assert_eq!(encoded_key(&blob).unwrap(), md5(&blob));
        verify(&blob, &md5(&blob)).unwrap();
        assert_eq!(decode(&blob).unwrap(), b"hello world");
        let raw = encode(&[(b'N', b"raw")], false);
        assert_eq!(decode(&raw).unwrap(), b"raw");
    }

    #[test]
    fn rejects_bad_input() {
        assert!(decode(b"NOPE\0\0\0\0N").is_err());
        let mut blob = encode(&[(b'N', b"x")], true);
        blob[8] = 0x0E;
        assert!(parse_layout(&blob).is_err());
        let enc = b"BLTE\0\0\0\0E\x08".to_vec();
        assert!(decode(&enc).is_err());
    }
}
