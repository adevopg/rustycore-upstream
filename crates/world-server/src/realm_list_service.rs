// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Worldserver `sRealmList` operations used by the Battle.net
//! `GameUtilitiesService` ("Change Realm" at character select).
//!
//! Source anchors (TrinityCore `78bcc3f52a1daa406851e7121c2b1af392fb4b3c`,
//! `src/server/shared/Realm/RealmList.cpp`): `RealmList::LoadBuildInfo`,
//! `RealmList::GetBuildInfo`, `RealmList::GetRealmList`,
//! `RealmList::WriteSubRegions`, `RealmList::JoinRealm`, and
//! `WorldSession::InitializeSession` (`GLOBAL_REALM_CHARACTER_COUNTS`,
//! `game/Server/WorldSession.cpp`).

use std::net::IpAddr;

use rand::RngCore;
use wow_proto::realm_list_json::{
    self, ClientVersion, IpAddress, RealmEntry, RealmIpAddressFamily, RealmListServerIpAddresses,
    RealmListUpdates, RealmState,
};
use wow_proto::status;
use wow_world::bnet_services::{
    RealmJoinFutureLikeCpp, RealmJoinGrantLikeCpp, RealmJoinRequestLikeCpp,
    WorldserverRealmListPortLikeCpp,
};

use super::*;

const REALM_FLAG_VERSION_MISMATCH_LIKE_CPP: u8 = 0x01;
const REALM_FLAG_OFFLINE_LIKE_CPP: u8 = 0x02;

/// One `build_info` row as C++ `RealmBuildInfo` uses it for the realm list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RealmBuildInfoLikeCpp {
    pub(super) major_version: u32,
    pub(super) minor_version: u32,
    pub(super) bugfix_version: u32,
    pub(super) build: u32,
}

/// C++ `RealmList::GetBuildInfo`: exact build match.
fn get_build_info_like_cpp(
    builds: &[RealmBuildInfoLikeCpp],
    build: u32,
) -> Option<&RealmBuildInfoLikeCpp> {
    builds.iter().find(|info| info.build == build)
}

/// C++ `Realm::GetConfigId`: `ConfigIdByType[Type]` = `Type + 1`
/// (`Type` was already normalized below `MAX_CLIENT_REALM_TYPE`).
fn realm_config_id_like_cpp(icon: u8) -> u32 {
    u32::from(icon) + 1
}

/// C++ `RealmList::GetRealmList(build, subRegion)`.
pub(super) fn realm_list_updates_like_cpp(
    snapshot: &RealmListSnapshotLikeCpp,
    builds: &[RealmBuildInfoLikeCpp],
    build: u32,
    sub_region: &str,
) -> RealmListUpdates {
    let updates = snapshot
        .realms
        .values()
        .filter(|realm| realm.id.sub_region_address_like_cpp() == sub_region)
        .map(|realm| {
            let mut flag = realm.flag;
            if realm.build != build {
                flag |= REALM_FLAG_VERSION_MISMATCH_LIKE_CPP;
            }

            let version = match get_build_info_like_cpp(builds, realm.build) {
                Some(info) => ClientVersion {
                    version_major: info.major_version,
                    version_minor: info.minor_version,
                    version_revision: info.bugfix_version,
                    version_build: info.build,
                },
                None => ClientVersion {
                    version_major: 6,
                    version_minor: 2,
                    version_revision: 4,
                    version_build: realm.build,
                },
            };

            RealmState {
                update: RealmEntry {
                    wow_realm_address: realm.id.address_like_cpp(),
                    cfg_timezones_id: 1,
                    population_state: if realm.flag & REALM_FLAG_OFFLINE_LIKE_CPP != 0 {
                        0
                    } else {
                        (realm.population as u32).max(1)
                    },
                    cfg_categories_id: u32::from(realm.timezone),
                    version,
                    cfg_realms_id: realm.id.realm,
                    flags: u32::from(flag),
                    name: realm.name.clone(),
                    cfg_configs_id: realm_config_id_like_cpp(realm.icon),
                    cfg_languages_id: 1,
                },
                deleting: false,
            }
        })
        .collect();
    RealmListUpdates { updates }
}

/// Why C++ `RealmList::JoinRealm` refuses before touching the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct JoinRealmTargetLikeCpp {
    pub(super) address: String,
    pub(super) local_address: String,
    pub(super) port: u16,
    pub(super) name: String,
    pub(super) realm_id: u32,
}

/// The realm-lookup and permission gate of C++ `RealmList::JoinRealm`.
pub(super) fn join_realm_target_like_cpp(
    snapshot: &RealmListSnapshotLikeCpp,
    realm_address: u32,
    build: u32,
) -> Result<JoinRealmTargetLikeCpp, u32> {
    // `GetRealm(Battlenet::RealmHandle(realmAddress))`: RealmHandle ordering
    // only compares the low 16-bit realm id.
    let Some(realm) = snapshot.get_realm_by_id_like_cpp(realm_address & 0xFFFF) else {
        return Err(status::ERROR_UTIL_SERVER_UNKNOWN_REALM);
    };
    if realm.flag & REALM_FLAG_OFFLINE_LIKE_CPP != 0 || realm.build != build {
        return Err(status::ERROR_USER_SERVER_NOT_PERMITTED_ON_REALM);
    }
    Ok(JoinRealmTargetLikeCpp {
        address: realm.address.clone(),
        local_address: realm.local_address.clone(),
        port: realm.port,
        name: realm.name.clone(),
        realm_id: realm.id.realm,
    })
}

/// C++ `JSONRealmListServerIPAddresses` for the address chosen by
/// `Realm::GetAddressForClient`.
pub(super) fn realm_server_addresses_blob_like_cpp(address: [u8; 4], port: u16) -> Vec<u8> {
    realm_list_json::compress_prefixed_like_cpp(
        realm_list_json::REALM_LIST_SERVER_IP_ADDRESSES_PREFIX,
        &RealmListServerIpAddresses {
            families: vec![RealmIpAddressFamily {
                family: 1,
                addresses: vec![IpAddress {
                    ip: format_ipv4(address),
                    port: u32::from(port),
                }],
            }],
        },
    )
}

/// C++ `LOGIN_UPD_BNET_GAME_ACCOUNT_LOGIN_INFO` bound by `RealmList::JoinRealm`.
pub(super) fn join_realm_login_info_statement_like_cpp(
    request: &RealmJoinRequestLikeCpp,
    client_address: &str,
    server_secret: &[u8; 32],
) -> wow_database::PreparedStatement {
    let mut key_data = Vec::with_capacity(64);
    key_data.extend_from_slice(&request.client_secret);
    key_data.extend_from_slice(server_secret);

    let mut stmt = wow_database::PreparedStatement::for_statement(
        LoginStatements::UPD_BNET_GAME_ACCOUNT_LOGIN_INFO,
    );
    stmt.set_bytes(0, key_data);
    stmt.set_string(1, client_address);
    stmt.set_u8(2, request.locale);
    stmt.set_string(3, request.os.clone());
    stmt.set_i16(4, request.timezone_offset_minutes);
    stmt.set_string(5, request.account_name.clone());
    stmt
}

/// Worldserver implementation of `WorldserverRealmListPortLikeCpp` over the
/// shared realm-list snapshot kept fresh by the `RealmsStateUpdateDelay` loop.
pub(super) struct WorldserverRealmListServiceLikeCpp {
    realm_list: SharedRealmListLikeCpp,
    builds: Vec<RealmBuildInfoLikeCpp>,
    current_realm_build: u32,
    login_db: Arc<LoginDatabase>,
}

impl WorldserverRealmListServiceLikeCpp {
    pub(super) fn new(
        realm_list: SharedRealmListLikeCpp,
        builds: Vec<RealmBuildInfoLikeCpp>,
        current_realm_build: u32,
        login_db: Arc<LoginDatabase>,
    ) -> Self {
        Self {
            realm_list,
            builds,
            current_realm_build,
            login_db,
        }
    }

    /// C++ `AccountInfoQueryHolder::GLOBAL_REALM_CHARACTER_COUNTS`
    /// (`LOGIN_SEL_BNET_CHARACTER_COUNTS_BY_ACCOUNT_ID`) as consumed by
    /// `InitializeSessionCallback`: `RealmHandle{Region, Battlegroup, id}.GetAddress()`
    /// -> `numchars`.
    pub(super) async fn load_realm_character_counts_like_cpp(
        &self,
        account_id: u32,
    ) -> Vec<(u32, u8)> {
        let mut stmt = self
            .login_db
            .prepare(LoginStatements::SEL_BNET_CHARACTER_COUNTS_BY_ACCOUNT_ID);
        stmt.set_u32(0, account_id);
        let mut result = match self.login_db.query(&stmt).await {
            Ok(result) => result,
            Err(error) => {
                warn!(account_id, "Failed to load realm character counts: {error}");
                return Vec::new();
            }
        };

        let mut counts = Vec::new();
        if result.is_empty() {
            return counts;
        }
        loop {
            let num_chars: u8 = result.try_read(1).unwrap_or(0);
            let realm_id: u32 = result.try_read(2).unwrap_or(0);
            let region: u8 = result.try_read(3).unwrap_or(0);
            let battlegroup: u8 = result.try_read(4).unwrap_or(0);
            counts.push((
                RealmHandleLikeCpp::new_like_cpp(region, battlegroup, realm_id).address_like_cpp(),
                num_chars,
            ));
            if !result.next_row() {
                break;
            }
        }
        counts
    }
}

/// C++ `RealmList::LoadBuildInfo` (the realm-list columns of `build_info`).
pub(super) async fn load_realm_build_info_like_cpp(
    login_db: &LoginDatabase,
) -> Result<Vec<RealmBuildInfoLikeCpp>> {
    let mut result = login_db
        .direct_query(
            "SELECT majorVersion, minorVersion, bugfixVersion, build FROM build_info ORDER BY build ASC",
        )
        .await
        .context("Failed to query build_info for RealmList")?;

    let mut builds = Vec::new();
    if result.is_empty() {
        return Ok(builds);
    }
    loop {
        let read_u32 = |column: usize| {
            result
                .try_read::<u32>(column)
                .or_else(|| result.try_read::<i32>(column).map(|value| value as u32))
                .unwrap_or(0)
        };
        builds.push(RealmBuildInfoLikeCpp {
            major_version: read_u32(0),
            minor_version: read_u32(1),
            bugfix_version: read_u32(2),
            build: read_u32(3),
        });
        if !result.next_row() {
            break;
        }
    }
    Ok(builds)
}

impl WorldserverRealmListPortLikeCpp for WorldserverRealmListServiceLikeCpp {
    fn sub_regions_like_cpp(&self) -> Vec<String> {
        let snapshot = self.realm_list.lock().expect("realm list mutex poisoned");
        snapshot.sub_regions.iter().cloned().collect()
    }

    fn current_realm_build_like_cpp(&self) -> u32 {
        self.current_realm_build
    }

    fn get_realm_list_like_cpp(&self, build: u32, sub_region: &str) -> Vec<u8> {
        let updates = {
            let snapshot = self.realm_list.lock().expect("realm list mutex poisoned");
            realm_list_updates_like_cpp(&snapshot, &self.builds, build, sub_region)
        };
        realm_list_json::compress_prefixed_like_cpp(
            realm_list_json::REALM_LIST_UPDATES_PREFIX,
            &updates,
        )
    }

    fn join_realm_like_cpp(&self, request: RealmJoinRequestLikeCpp) -> RealmJoinFutureLikeCpp<'_> {
        Box::pin(async move {
            let target = {
                let snapshot = self.realm_list.lock().expect("realm list mutex poisoned");
                join_realm_target_like_cpp(&snapshot, request.realm_address, request.build)?
            };

            // C++ resolves `address`/`localAddress` while loading the realm list
            // and drops realms that do not resolve; this snapshot keeps the raw
            // strings, so resolve here and treat a failure as an unknown realm.
            let external = resolve_realm_endpoint_address_like_cpp(
                "address",
                &target.address,
                &target.name,
                target.realm_id,
            )
            .await
            .map_err(|_| status::ERROR_UTIL_SERVER_UNKNOWN_REALM)?;
            let local = resolve_realm_endpoint_address_like_cpp(
                "localAddress",
                &target.local_address,
                &target.name,
                target.realm_id,
            )
            .await
            .map_err(|_| status::ERROR_UTIL_SERVER_UNKNOWN_REALM)?;

            let client_ip = request.client_address.parse::<IpAddr>().ok();
            let address_for_client = get_address_for_client(client_ip, external, local);
            let server_addresses =
                realm_server_addresses_blob_like_cpp(address_for_client, target.port);

            let mut server_secret = [0u8; 32];
            rand::rngs::OsRng.fill_bytes(&mut server_secret);

            let client_address = client_ip.map_or_else(
                || request.client_address.clone(),
                |address| address.to_string(),
            );
            let stmt =
                join_realm_login_info_statement_like_cpp(&request, &client_address, &server_secret);
            // C++ `LoginDatabase.DirectExecute(stmt)` ignores the result.
            if let Err(error) = self.login_db.execute(&stmt).await {
                warn!(
                    account = %request.account_name,
                    "JoinRealm failed to store the Battle.net session key: {error}"
                );
            }

            info!(
                account = %request.account_name,
                realm = %target.name,
                "Worldserver RealmList::JoinRealm granted"
            );
            Ok(RealmJoinGrantLikeCpp {
                server_addresses,
                join_secret: server_secret,
            })
        })
    }
}

#[cfg(test)]
#[path = "realm_list_service_tests.rs"]
mod tests;
