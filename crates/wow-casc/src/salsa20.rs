//! Salsa20/20 stream decryption as used for TACT 'S' encrypted BLTE frames.
//!
//! Port of `CascLib` `dep/CascLib/src/CascDecrypt.cpp`: `Initialize`, `Decrypt`,
//! `Decrypt_Salsa20`. `CascLib` always passes a 16-byte key ("expand 16-byte k"
//! constants, key words repeated) and an 8-byte nonce; the 64-bit block
//! counter starts at zero.

const SIGMA16: &[u8; 16] = b"expand 16-byte k";
const SIGMA32: &[u8; 16] = b"expand 32-byte k";

#[inline]
fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// `CascLib` `Initialize`: builds the Salsa20 input state. `key` must be 16 or
/// 32 bytes, `vector` 8 bytes.
fn initialize(key: &[u8], vector: [u8; 8]) -> [u32; 16] {
    let constants = if key.len() == 32 { SIGMA32 } else { SIGMA16 };
    let key_index = key.len() - 0x10;
    [
        word(constants, 0x00),
        word(key, 0x00),
        word(key, 0x04),
        word(key, 0x08),
        word(key, 0x0C),
        word(constants, 0x04),
        word(&vector, 0x00),
        word(&vector, 0x04),
        0,
        0,
        word(constants, 0x08),
        word(key, key_index),
        word(key, key_index + 0x04),
        word(key, key_index + 0x08),
        word(key, key_index + 0x0C),
        word(constants, 0x0C),
    ]
}

/// Salsa20 double-round loop from `CascLib` `Decrypt` (20 rounds).
fn block(state: &[u32; 16]) -> [u8; 64] {
    let mut x = *state;
    macro_rules! qr {
        ($d:expr, $s1:expr, $s2:expr, $r:expr) => {
            x[$d] ^= x[$s1].wrapping_add(x[$s2]).rotate_left($r);
        };
    }
    for _ in (0..20).step_by(2) {
        qr!(0x04, 0x00, 0x0C, 7);
        qr!(0x08, 0x04, 0x00, 9);
        qr!(0x0C, 0x08, 0x04, 13);
        qr!(0x00, 0x0C, 0x08, 18);

        qr!(0x09, 0x05, 0x01, 7);
        qr!(0x0D, 0x09, 0x05, 9);
        qr!(0x01, 0x0D, 0x09, 13);
        qr!(0x05, 0x01, 0x0D, 18);

        qr!(0x0E, 0x0A, 0x06, 7);
        qr!(0x02, 0x0E, 0x0A, 9);
        qr!(0x06, 0x02, 0x0E, 13);
        qr!(0x0A, 0x06, 0x02, 18);

        qr!(0x03, 0x0F, 0x0B, 7);
        qr!(0x07, 0x03, 0x0F, 9);
        qr!(0x0B, 0x07, 0x03, 13);
        qr!(0x0F, 0x0B, 0x07, 18);

        qr!(0x01, 0x00, 0x03, 7);
        qr!(0x02, 0x01, 0x00, 9);
        qr!(0x03, 0x02, 0x01, 13);
        qr!(0x00, 0x03, 0x02, 18);

        qr!(0x06, 0x05, 0x04, 7);
        qr!(0x07, 0x06, 0x05, 9);
        qr!(0x04, 0x07, 0x06, 13);
        qr!(0x05, 0x04, 0x07, 18);

        qr!(0x0B, 0x0A, 0x09, 7);
        qr!(0x08, 0x0B, 0x0A, 9);
        qr!(0x09, 0x08, 0x0B, 13);
        qr!(0x0A, 0x09, 0x08, 18);

        qr!(0x0C, 0x0F, 0x0E, 7);
        qr!(0x0D, 0x0C, 0x0F, 9);
        qr!(0x0E, 0x0D, 0x0C, 13);
        qr!(0x0F, 0x0E, 0x0D, 18);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[i * 4..i * 4 + 4].copy_from_slice(&x[i].wrapping_add(state[i]).to_le_bytes());
    }
    out
}

/// `CascLib` `Decrypt_Salsa20`: XORs `data` in place with the Salsa20 keystream.
pub fn apply_keystream(key: &[u8], vector: [u8; 8], data: &mut [u8]) {
    let mut state = initialize(key, vector);
    for chunk in data.chunks_mut(0x40) {
        let ks = block(&state);
        for (b, k) in chunk.iter_mut().zip(ks.iter()) {
            *b ^= k;
        }
        state[8] = state[8].wrapping_add(1);
        if state[8] == 0 {
            state[9] = state[9].wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// ECRYPT Salsa20/20 128-bit key test vector: Set 1, vector #0
    /// (key = 80 00..00, IV = 0), stream[0..63].
    #[test]
    fn ecrypt_set1_vector0() {
        let mut key = [0u8; 16];
        key[0] = 0x80;
        let mut data = [0u8; 64];
        apply_keystream(&key, [0u8; 8], &mut data);
        let expected = hex(concat!(
            "4DFA5E481DA23EA09A31022050859936",
            "DA52FCEE218005164F267CB65F5CFD7F",
            "2B4F97E0FF16924A52DF269515110A07",
            "F9E460BC65EF95DA58F740B7D1DBB0AA"
        ));
        assert_eq!(&data[..], &expected[..]);
    }

    #[test]
    fn keystream_is_involutive_and_crosses_blocks() {
        let key: Vec<u8> = (0u8..16).collect();
        let iv = [1, 2, 3, 4, 5, 6, 7, 8];
        let plain: Vec<u8> = (0..200u32).map(|i| (i * 7) as u8).collect();
        let mut buf = plain.clone();
        apply_keystream(&key, iv, &mut buf);
        assert_ne!(buf, plain);
        apply_keystream(&key, iv, &mut buf);
        assert_eq!(buf, plain);
    }
}
