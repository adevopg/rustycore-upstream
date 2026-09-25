//! `JSON::RealmList` payloads shared by bnetserver and worldserver.
//!
//! TrinityCore keeps these messages in `src/server/proto/RealmList/RealmList.proto`
//! (package `JSON.RealmList`) and serializes them with `JSON::Serialize`
//! (`ProtobufJSON.cpp`), which names every member after the proto field name
//! and walks fields in field-number order. The client receives them inside a
//! protobuf `Variant.blob_value` as `"<Prefix>:<json>\0"`, zlib-compressed and
//! preceded by the uncompressed length (`RealmList::GetRealmList`,
//! `RealmList::JoinRealm`, `GameUtilitiesService::HandleRealmListRequest` at
//! TrinityCore `78bcc3f52a1daa406851e7121c2b1af392fb4b3c`).
//!
//! Both servers build their realm-list attributes from this one module so the
//! bnetserver and worldserver payloads cannot drift apart.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use serde::Serialize;

/// `JSON.RealmList.ClientVersion`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientVersion {
    #[serde(rename = "versionMajor")]
    pub version_major: u32,
    #[serde(rename = "versionMinor")]
    pub version_minor: u32,
    #[serde(rename = "versionRevision")]
    pub version_revision: u32,
    #[serde(rename = "versionBuild")]
    pub version_build: u32,
}

/// `JSON.RealmList.RealmEntry`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RealmEntry {
    #[serde(rename = "wowRealmAddress")]
    pub wow_realm_address: u32,
    #[serde(rename = "cfgTimezonesID")]
    pub cfg_timezones_id: u32,
    #[serde(rename = "populationState")]
    pub population_state: u32,
    #[serde(rename = "cfgCategoriesID")]
    pub cfg_categories_id: u32,
    pub version: ClientVersion,
    #[serde(rename = "cfgRealmsID")]
    pub cfg_realms_id: u32,
    pub flags: u32,
    pub name: String,
    #[serde(rename = "cfgConfigsID")]
    pub cfg_configs_id: u32,
    #[serde(rename = "cfgLanguagesID")]
    pub cfg_languages_id: u32,
}

/// `JSON.RealmList.RealmState`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RealmState {
    pub update: RealmEntry,
    pub deleting: bool,
}

/// `JSON.RealmList.RealmListUpdates`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RealmListUpdates {
    pub updates: Vec<RealmState>,
}

/// `JSON.RealmList.RealmCharacterCountEntry`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RealmCharacterCountEntry {
    #[serde(rename = "wowRealmAddress")]
    pub wow_realm_address: u32,
    pub count: u32,
}

/// `JSON.RealmList.RealmCharacterCountList`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RealmCharacterCountList {
    pub counts: Vec<RealmCharacterCountEntry>,
}

/// `JSON.RealmList.IPAddress`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IpAddress {
    pub ip: String,
    pub port: u32,
}

/// `JSON.RealmList.RealmIPAddressFamily`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RealmIpAddressFamily {
    pub family: u32,
    pub addresses: Vec<IpAddress>,
}

/// `JSON.RealmList.RealmListServerIPAddresses`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RealmListServerIpAddresses {
    pub families: Vec<RealmIpAddressFamily>,
}

pub const REALM_LIST_UPDATES_PREFIX: &str = "JSONRealmListUpdates";
pub const REALM_CHARACTER_COUNT_LIST_PREFIX: &str = "JSONRealmCharacterCountList";
pub const REALM_ENTRY_PREFIX: &str = "JamJSONRealmEntry";
pub const REALM_LIST_SERVER_IP_ADDRESSES_PREFIX: &str = "JSONRealmListServerIPAddresses";

/// C++ `"<prefix>:" + JSON::Serialize(message)`.
pub fn serialize_prefixed_like_cpp<T: Serialize>(prefix: &str, message: &T) -> String {
    format!(
        "{prefix}:{}",
        serde_json::to_string(message).unwrap_or_default()
    )
}

/// C++ realm-list compression: a little-endian `uint32(json.length() + 1)`
/// followed by zlib `compress()` of the JSON text *including* its NUL
/// terminator (`RealmList::GetRealmList` / `JoinRealm`,
/// `GameUtilitiesService::HandleRealmListRequest`).
pub fn compress_json_like_cpp(json: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity(json.len() + 1);
    data.extend_from_slice(json.as_bytes());
    data.push(0);

    let mut compressed = (data.len() as u32).to_le_bytes().to_vec();
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&data)
        .expect("in-memory zlib write cannot fail");
    compressed.extend_from_slice(&encoder.finish().expect("in-memory zlib finish cannot fail"));
    compressed
}

/// Serialize with the C++ prefix and compress the result.
pub fn compress_prefixed_like_cpp<T: Serialize>(prefix: &str, message: &T) -> Vec<u8> {
    compress_json_like_cpp(&serialize_prefixed_like_cpp(prefix, message))
}

/// Inverse of [`compress_json_like_cpp`]: returns the JSON text without the
/// trailing NUL. Intended for tests and diagnostics.
pub fn decompress_json_like_cpp(blob: &[u8]) -> Option<String> {
    use std::io::Read;

    let declared = u32::from_le_bytes(blob.get(..4)?.try_into().ok()?) as usize;
    let mut decoder = flate2::read::ZlibDecoder::new(blob.get(4..)?);
    let mut data = Vec::new();
    decoder.read_to_end(&mut data).ok()?;
    if data.len() != declared || data.last() != Some(&0) {
        return None;
    }
    data.pop();
    String::from_utf8(data).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realm_entry_uses_cpp_proto_field_names_in_field_order() {
        let entry = RealmEntry {
            wow_realm_address: 0x0101_0001,
            cfg_timezones_id: 1,
            population_state: 1,
            cfg_categories_id: 1,
            version: ClientVersion {
                version_major: 3,
                version_minor: 4,
                version_revision: 3,
                version_build: 54261,
            },
            cfg_realms_id: 1,
            flags: 0,
            name: "Trinity".to_owned(),
            cfg_configs_id: 1,
            cfg_languages_id: 1,
        };
        assert_eq!(
            serialize_prefixed_like_cpp(REALM_ENTRY_PREFIX, &entry),
            concat!(
                "JamJSONRealmEntry:{\"wowRealmAddress\":16842753,\"cfgTimezonesID\":1,",
                "\"populationState\":1,\"cfgCategoriesID\":1,\"version\":{\"versionMajor\":3,",
                "\"versionMinor\":4,\"versionRevision\":3,\"versionBuild\":54261},",
                "\"cfgRealmsID\":1,\"flags\":0,\"name\":\"Trinity\",\"cfgConfigsID\":1,",
                "\"cfgLanguagesID\":1}"
            )
        );
    }

    #[test]
    fn compression_prefixes_length_including_nul_and_round_trips() {
        let json = serialize_prefixed_like_cpp(
            REALM_CHARACTER_COUNT_LIST_PREFIX,
            &RealmCharacterCountList {
                counts: vec![RealmCharacterCountEntry {
                    wow_realm_address: 0x0101_0001,
                    count: 3,
                }],
            },
        );
        let blob = compress_json_like_cpp(&json);
        assert_eq!(
            u32::from_le_bytes(blob[..4].try_into().unwrap()) as usize,
            json.len() + 1
        );
        assert_eq!(
            decompress_json_like_cpp(&blob).as_deref(),
            Some(json.as_str())
        );
        assert_eq!(
            json,
            "JSONRealmCharacterCountList:{\"counts\":[{\"wowRealmAddress\":16842753,\"count\":3}]}"
        );
    }

    #[test]
    fn server_addresses_match_cpp_shape() {
        let addresses = RealmListServerIpAddresses {
            families: vec![RealmIpAddressFamily {
                family: 1,
                addresses: vec![IpAddress {
                    ip: "127.0.0.1".to_owned(),
                    port: 8085,
                }],
            }],
        };
        assert_eq!(
            serialize_prefixed_like_cpp(REALM_LIST_SERVER_IP_ADDRESSES_PREFIX, &addresses),
            "JSONRealmListServerIPAddresses:{\"families\":[{\"family\":1,\"addresses\":[{\"ip\":\"127.0.0.1\",\"port\":8085}]}]}"
        );
    }
}
