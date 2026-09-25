//! Bob Jenkins' lookup3 hashes (`dep/CascLib/src/jenkins/lookup3.c`:
//! `hashlittle`, `hashlittle2`) and `CascLib`'s file-name hash
//! (`dep/CascLib/src/common/Common.cpp`: `CalcFileNameHash`,
//! `CalcNormNameHash`, `NormalizeFileName_UpperBkSlash`).
//!
//! The byte-wise code path of lookup3 is ported; on little-endian hosts it is
//! defined to produce exactly the same values as the aligned-read path.

#[inline]
fn mix(a: &mut u32, b: &mut u32, c: &mut u32) {
    *a = a.wrapping_sub(*c);
    *a ^= c.rotate_left(4);
    *c = c.wrapping_add(*b);
    *b = b.wrapping_sub(*a);
    *b ^= a.rotate_left(6);
    *a = a.wrapping_add(*c);
    *c = c.wrapping_sub(*b);
    *c ^= b.rotate_left(8);
    *b = b.wrapping_add(*a);
    *a = a.wrapping_sub(*c);
    *a ^= c.rotate_left(16);
    *c = c.wrapping_add(*b);
    *b = b.wrapping_sub(*a);
    *b ^= a.rotate_left(19);
    *a = a.wrapping_add(*c);
    *c = c.wrapping_sub(*b);
    *c ^= b.rotate_left(4);
    *b = b.wrapping_add(*a);
}

#[inline]
fn final_mix(a: &mut u32, b: &mut u32, c: &mut u32) {
    *c ^= *b;
    *c = c.wrapping_sub(b.rotate_left(14));
    *a ^= *c;
    *a = a.wrapping_sub(c.rotate_left(11));
    *b ^= *a;
    *b = b.wrapping_sub(a.rotate_left(25));
    *c ^= *b;
    *c = c.wrapping_sub(b.rotate_left(16));
    *a ^= *c;
    *a = a.wrapping_sub(c.rotate_left(4));
    *b ^= *a;
    *b = b.wrapping_sub(a.rotate_left(14));
    *c ^= *b;
    *c = c.wrapping_sub(b.rotate_left(24));
}

/// Shared body of `hashlittle`/`hashlittle2`. Returns `(c, b)`.
fn hash_core(key: &[u8], pc: u32, pb: u32) -> (u32, u32) {
    let init = 0xdead_beef_u32
        .wrapping_add(key.len() as u32)
        .wrapping_add(pc);
    let (mut a, mut b, mut c) = (init, init, init.wrapping_add(pb));

    let mut k = key;
    while k.len() > 12 {
        a = a.wrapping_add(u32::from_le_bytes([k[0], k[1], k[2], k[3]]));
        b = b.wrapping_add(u32::from_le_bytes([k[4], k[5], k[6], k[7]]));
        c = c.wrapping_add(u32::from_le_bytes([k[8], k[9], k[10], k[11]]));
        mix(&mut a, &mut b, &mut c);
        k = &k[12..];
    }

    if k.is_empty() {
        // "zero length strings require no mixing"
        return (c, b);
    }

    // Last block: zero-pad to 12 bytes (equivalent to the fall-through switch).
    let mut tail = [0u8; 12];
    tail[..k.len()].copy_from_slice(k);
    a = a.wrapping_add(u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]));
    b = b.wrapping_add(u32::from_le_bytes([tail[4], tail[5], tail[6], tail[7]]));
    c = c.wrapping_add(u32::from_le_bytes([tail[8], tail[9], tail[10], tail[11]]));
    final_mix(&mut a, &mut b, &mut c);
    (c, b)
}

/// lookup3 `hashlittle(key, length, initval)`.
pub fn hashlittle(key: &[u8], initval: u32) -> u32 {
    hash_core(key, initval, 0).0
}

/// lookup3 `hashlittle2(key, length, &pc, &pb)`: updates the primary (`pc`)
/// and secondary (`pb`) values in place.
pub fn hashlittle2(key: &[u8], pc: &mut u32, pb: &mut u32) {
    let (c, b) = hash_core(key, *pc, *pb);
    *pc = c;
    *pb = b;
}

/// `CascLib` `CalcFileNameHash`: Jenkins96 (`hashlittle2`, both seeds 0) of the
/// file name upper-cased with `/` normalized to `\` (`AsciiToUpperTable_BkSlash`);
/// result is `(primary << 32) | secondary`.
pub fn file_name_hash(name: &str) -> u64 {
    // NormalizeFileName_UpperBkSlash truncates at MAX_PATH (260) characters.
    let norm: Vec<u8> = name
        .bytes()
        .take(260)
        .map(|ch| match ch {
            b'/' => b'\\',
            b'a'..=b'z' => ch - 0x20,
            _ => ch,
        })
        .collect();
    let (mut high, mut low) = (0u32, 0u32);
    hashlittle2(&norm, &mut high, &mut low);
    (u64::from(high) << 32) | u64::from(low)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test vectors from lookup3.c `driver5()`.
    #[test]
    fn lookup3_driver5_vectors() {
        let (mut c, mut b) = (0, 0);
        hashlittle2(b"", &mut c, &mut b);
        assert_eq!((c, b), (0xdead_beef, 0xdead_beef));

        let (mut c, mut b) = (0, 0xdead_beef);
        hashlittle2(b"", &mut c, &mut b);
        assert_eq!((c, b), (0xbd5b_7dde, 0xdead_beef));

        let (mut c, mut b) = (0xdead_beef, 0xdead_beef);
        hashlittle2(b"", &mut c, &mut b);
        assert_eq!((c, b), (0x9c09_3ccd, 0xbd5b_7dde));

        let s = b"Four score and seven years ago";
        let (mut c, mut b) = (0, 0);
        hashlittle2(s, &mut c, &mut b);
        assert_eq!((c, b), (0x1777_0551, 0xce72_26e6));

        let (mut c, mut b) = (0, 1);
        hashlittle2(s, &mut c, &mut b);
        assert_eq!((c, b), (0xe360_7cae, 0xbd37_1de4));

        let (mut c, mut b) = (1, 0);
        hashlittle2(s, &mut c, &mut b);
        assert_eq!((c, b), (0xcd62_8161, 0x6cbe_a4b3));

        assert_eq!(hashlittle(s, 0), 0x1777_0551);
        assert_eq!(hashlittle(s, 1), 0xcd62_8161);
    }

    #[test]
    fn file_name_hash_normalizes_case_and_slashes() {
        let a = file_name_hash("DBFilesClient\\Map.db2");
        assert_eq!(a, file_name_hash("dbfilesclient/map.db2"));
        assert_eq!(a, file_name_hash("DBFILESCLIENT\\MAP.DB2"));
        assert_ne!(a, file_name_hash("DBFilesClient\\Map.dbc"));
    }
}
