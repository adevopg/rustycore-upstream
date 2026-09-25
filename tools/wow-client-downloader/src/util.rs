//! Small shared helpers: 16-byte keys (`CKey`/`EKey`) and their hex form,
//! MD5, big-endian readers and human-readable sizes.
//!
//! NGDP/TACT keys are MD5 digests written as 32 lower-case hex digits
//! (wowdev "TACT"; blizzget `NGDP::to_string`/`from_string`).

use md5::{Digest, Md5};

/// A content key (`CKey`) or encoded key (`EKey`).
pub type Key = [u8; 16];

/// Lower-case hex of any byte string.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Parses exactly 32 hex digits into a key.
pub fn parse_key(text: &str) -> Option<Key> {
    let bytes = text.trim().as_bytes();
    if bytes.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Some(out)
}

pub fn md5(data: &[u8]) -> Key {
    Md5::digest(data).into()
}

/// Big-endian unsigned integer of up to 8 bytes.
pub fn be_uint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b))
}

/// CDN relative path of a key: `<type>/xx/yy/<key>` (blizzget `NGDP::geturl`).
pub fn cdn_path(kind: &str, key: &str) -> String {
    format!("{kind}/{}/{}/{key}", &key[0..2], &key[2..4])
}

/// `12.34 GiB` style size.
#[allow(clippy::cast_precision_loss, reason = "display only")]
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_hex_round_trip() {
        let text = "c91609c69ed2ab39d44039390a1be969";
        let key = parse_key(text).unwrap();
        assert_eq!(key[0], 0xc9);
        assert_eq!(hex(&key), text);
        assert!(parse_key("c91609").is_none());
        assert!(parse_key("z91609c69ed2ab39d44039390a1be969").is_none());
    }

    #[test]
    fn helpers() {
        assert_eq!(be_uint(&[0x01, 0x02, 0x03]), 0x0001_0203);
        assert_eq!(
            cdn_path("config", "c91609c69ed2ab39d44039390a1be969"),
            "config/c9/16/c91609c69ed2ab39d44039390a1be969"
        );
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(3 * 1024 * 1024), "3.00 MiB");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
    }
}
