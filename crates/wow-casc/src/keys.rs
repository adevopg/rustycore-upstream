//! TACT encryption key store.
//!
//! Port of `CascLib` `dep/CascLib/src/CascDecrypt.cpp`: `CASC_KEY_MAP::AddKey` /
//! `FindKey` (an existing key is never replaced), `CascLoadEncryptionKeys`
//! (the built-in [`STATIC_CASC_KEYS`] table) and `CascImportKeysFromString`.

use std::collections::HashMap;

use crate::keys_table::STATIC_CASC_KEYS;

/// `CascLib` `CASC_KEY_LENGTH`.
pub const KEY_LENGTH: usize = 0x10;

#[derive(Debug, Clone)]
pub struct KeyMap {
    keys: HashMap<u64, [u8; KEY_LENGTH]>,
}

impl KeyMap {
    /// `CascLoadEncryptionKeys`: a key map preloaded with the static table.
    pub fn with_static_keys() -> Self {
        let mut map = Self {
            keys: HashMap::with_capacity(STATIC_CASC_KEYS.len() * 2),
        };
        for &(name, key) in STATIC_CASC_KEYS {
            map.add(name, key.to_be_bytes());
        }
        map
    }

    /// `CASC_KEY_MAP::AddKey`: inserts the key unless one with that name
    /// already exists. Returns `true` when a new key was inserted.
    pub fn add(&mut self, name: u64, key: [u8; KEY_LENGTH]) -> bool {
        use std::collections::hash_map::Entry;
        match self.keys.entry(name) {
            Entry::Occupied(_) => false,
            Entry::Vacant(v) => {
                v.insert(key);
                true
            }
        }
    }

    /// `CASC_KEY_MAP::FindKey`.
    pub fn find(&self, name: u64) -> Option<&[u8; KEY_LENGTH]> {
        self.keys.get(&name)
    }

    /// Imports keys from the wowdev `TACTKeys` text format, one
    /// `<key name hex> <32 hex digit key>` pair per line. Returns the number of
    /// keys that were not already present.
    ///
    /// Line syntax follows `CascImportKeysFromString` (key name parsed with
    /// `ConvertStringToInt`, up to 16 hex digits; spaces/tabs; 32 hex digits
    /// of key; the rest of the line ignored; a key name of 0 ends the list).
    /// Deviation: `CascLib` aborts the whole import on the first malformed
    /// line; here empty lines and `#` comment lines are skipped and other
    /// malformed lines are ignored individually.
    pub fn import_from_str(&mut self, text: &str) -> usize {
        let mut added = 0;
        for line in text.split(['\n', '\r']) {
            let line = line.trim_start_matches([' ', '\t']);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((name, key)) = parse_key_line(line) else {
                continue;
            };
            // "TACT key list downloaded from wow.tools ends with a single zero"
            if name == 0 {
                break;
            }
            if self.add(name, key) {
                added += 1;
            }
        }
        added
    }
}

fn hex_value(ch: u8) -> Option<u8> {
    match ch {
        b'0'..=b'9' => Some(ch - b'0'),
        b'a'..=b'f' => Some(ch - b'a' + 10),
        b'A'..=b'F' => Some(ch - b'A' + 10),
        _ => None,
    }
}

fn parse_key_line(line: &str) -> Option<(u64, [u8; KEY_LENGTH])> {
    let bytes = line.as_bytes();
    let mut pos = 0;
    let mut name: u64 = 0;
    // ConvertStringToInt(szKeyList, 0, KeyName): up to 16 digits, stops at <= 0x20.
    while pos < bytes.len() && pos < 16 && bytes[pos] > 0x20 {
        name = (name << 4) | u64::from(hex_value(bytes[pos])?);
        pos += 1;
    }
    if pos == 0 {
        return None;
    }
    if name == 0 {
        return Some((0, [0; KEY_LENGTH]));
    }
    // "We only expect spaces and tabs at this point"
    if pos < bytes.len() && bytes[pos] > 0x20 {
        return None;
    }
    while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
        pos += 1;
    }
    // BinaryFromString(szKeyList, CASC_KEY_LENGTH * 2, KeyValue)
    let hex = bytes.get(pos..pos + KEY_LENGTH * 2)?;
    let mut key = [0u8; KEY_LENGTH];
    for (i, pair) in hex.as_chunks::<2>().0.iter().enumerate() {
        key[i] = (hex_value(pair[0])? << 4) | hex_value(pair[1])?;
    }
    Some((name, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_table_is_loaded() {
        let map = KeyMap::with_static_keys();
        // First entry of CascLib StaticCascKeys (Battle.net App Alpha 1.5.0).
        assert_eq!(
            map.find(0x2C54_7F26_A261_3E01),
            Some(&[
                0x37, 0xC5, 0x0C, 0x10, 0x2D, 0x4C, 0x9E, 0x3A, 0x5A, 0xC0, 0x69, 0xF0, 0x72, 0xB1,
                0x41, 0x7D
            ])
        );
        // CascLib's table lists one key name twice; AddKey keeps the first.
        let distinct: std::collections::HashSet<u64> =
            STATIC_CASC_KEYS.iter().map(|&(name, _)| name).collect();
        assert_eq!(map.keys.len(), distinct.len());
        for &(name, key) in STATIC_CASC_KEYS {
            let first = STATIC_CASC_KEYS
                .iter()
                .find(|&&(n, _)| n == name)
                .unwrap()
                .1;
            if key == first {
                assert_eq!(map.find(name), Some(&key.to_be_bytes()));
            }
        }
    }

    #[test]
    fn import_text_keys() {
        let mut map = KeyMap::with_static_keys();
        let text = "# comment\n\
                    1122334455667788 00112233445566778899AABBCCDDEEFF extra words\r\n\
                    \n\
                    2C547F26A2613E01 FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF\n\
                    bad line\n\
                    99 0102030405060708090a0b0c0d0e0f10\n";
        assert_eq!(map.import_from_str(text), 2);
        assert_eq!(map.find(0x1122_3344_5566_7788).unwrap()[15], 0xFF);
        assert_eq!(map.find(0x99).unwrap()[0], 0x01);
        // Existing keys are never replaced (CASC_KEY_MAP::AddKey).
        assert_eq!(map.find(0x2C54_7F26_A261_3E01).unwrap()[0], 0x37);
    }

    #[test]
    fn zero_key_name_ends_list() {
        let mut map = KeyMap::with_static_keys();
        let text = "0\n1122334455667788 00112233445566778899AABBCCDDEEFF\n";
        assert_eq!(map.import_from_str(text), 0);
    }
}
