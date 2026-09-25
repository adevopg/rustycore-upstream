//! The 0x1E-byte `BLTE_ENCODED_HEADER` in front of every entry of a local
//! `data.###` archive.
//!
//! Field layout: `CascLib` `CascStructs.h: BLTE_ENCODED_HEADER` (`EKey`
//! byte-reversed, `EncodedSize` LE incl. this header, `field_14`, `field_15`,
//! `JenkinsHash[4]`, `Checksum[4]`). Values follow the Battle.net Agent, as
//! verified on a real Agent-written `wow_classic_beta` install (thousands of
//! entries across `data.005`, `data.030`, `data.063`):
//!
//! * `EKey`: only the 9-byte prefix the local indices use, reversed, so the
//!   first 7 bytes are zero;
//! * `field_14` = `field_15` = 0 for data entries (the Agent writes 1 for
//!   empty placeholder spans);
//! * `JenkinsHash` = `hashlittle(header[0..0x16], 0x3D6BE971)` (`CascLib`
//!   `VerifyHeaderSpan`);
//! * `Checksum`: `CascLib` `VerifyHeaderSpan` (from `Agent.exe` 2.15) with one
//!   correction found on the real files: the running XOR of `header[0..0x1A]`
//!   is keyed by the byte's absolute position `(offset + i) & 3`, not `i & 3`,
//!   and `HeaderOffset` is the full storage offset `(archive << 30) | offset`.
//!   `CascLib` never checks it (its check is compiled out), so this only
//!   matters for the official client.

use crate::jenkins::hashlittle;
use crate::util::Key;

pub const HEADER_SIZE: usize = 0x1E;

/// `CascLib` `table_16C57A8`.
const OFFSET_TABLE: [u32; 16] = [
    0x0493_96B8,
    0x72A8_2A9B,
    0xEE62_6CCA,
    0x9917_754F,
    0x15DE_40B1,
    0xF5A8_A9B6,
    0x421E_AC7E,
    0xA9D5_5C9A,
    0x317F_D40C,
    0x04FA_F80D,
    0x3D6B_E971,
    0x5293_3CFD,
    0x27F6_4B7D,
    0xC6F5_C11B,
    0xD575_7E3A,
    0x6C38_8745,
];

/// Header of an entry whose BLTE data is `blte_len` bytes long and which
/// starts at `storage_offset` (`(archive << 30) | offset`).
pub fn encoded_header(ekey: &Key, blte_len: u32, storage_offset: u64) -> [u8; HEADER_SIZE] {
    let mut h = [0u8; HEADER_SIZE];
    for i in 0..9 {
        h[15 - i] = ekey[i];
    }
    h[16..20].copy_from_slice(&(blte_len + HEADER_SIZE as u32).to_le_bytes());
    // h[20] (field_14) and h[21] (field_15) stay 0.
    let jenkins = hashlittle(&h[..0x16], 0x3D6B_E971);
    h[22..26].copy_from_slice(&jenkins.to_le_bytes());

    let mut encoded = (storage_offset as u32).wrapping_add(HEADER_SIZE as u32);
    encoded ^= OFFSET_TABLE[(encoded & 0x0F) as usize];
    let encoded = encoded.to_le_bytes();
    let base = storage_offset as usize;
    let mut hashed = [0u8; 4];
    for (i, &b) in h[..0x1A].iter().enumerate() {
        hashed[(base + i) & 3] ^= b;
    }
    for j in 0..4 {
        let k = (base + 0x1A + j) & 3;
        h[0x1A + j] = hashed[k] ^ encoded[k];
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::parse_key;

    /// Two real headers from an Agent-written `data.005` (archive 5).
    #[test]
    fn matches_real_agent_headers() {
        let cases = [
            (
                0x000e_010f_u64,
                189_106u32,
                "0000000000000007a12fa058723f9ca8b2e2020000008a09f5dd74051219",
            ),
            (
                0x0010_e3c1,
                24_971,
                "000000000000004b4ccef6c82b5d5ed98b610000000000b768308477e0ea",
            ),
        ];
        for (offset, total, expected) in cases {
            let bytes: Vec<u8> = (0..expected.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&expected[i..i + 2], 16).unwrap())
                .collect();
            // Recover the 9-byte EKey prefix from the reversed field.
            let mut ekey = [0x77u8; 16];
            for i in 0..9 {
                ekey[i] = bytes[15 - i];
            }
            let h = encoded_header(&ekey, total - HEADER_SIZE as u32, (5 << 30) | offset);
            assert_eq!(h.as_slice(), bytes.as_slice(), "offset {offset:#x}");
        }
    }

    #[test]
    fn layout() {
        let ekey = parse_key("53a60f42b994c2ca732fa86aeb4f2f8a").unwrap();
        let h = encoded_header(&ekey, 100, 0);
        assert_eq!(&h[..7], &[0; 7]);
        assert_eq!(h[15], 0x53);
        assert_eq!(h[7], 0x73, "ninth key byte");
        assert_eq!(u32::from_le_bytes(h[16..20].try_into().unwrap()), 130);
        assert_eq!(&h[20..22], &[0, 0]);
    }
}
