// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Worldserver Battle.net RPC services carried by `CMSG_BATTLENET_REQUEST`.
//!
//! Source anchors (TrinityCore `78bcc3f52a1daa406851e7121c2b1af392fb4b3c`,
//! the last `wotlk_classic` revision before the 2024-08 realm/auth refactors
//! that the 3.4.3.54261 fork still predates):
//! - `game/Services/WorldserverServiceDispatcher.cpp`: the registered
//!   services and the silent drop of unknown service hashes.
//! - `game/Services/WorldserverService.cpp`: `GameUtilitiesService`
//!   (`HandleProcessClientRequest`, `HandleRealmListRequest`,
//!   `HandleRealmJoinRequest`, `HandleGetAllValuesForAttribute`).
//! - `proto/Client/game_utilities_service.pb.cc`:
//!   `GameUtilitiesService::CallServerMethod` method table, the
//!   malformed-request / invalid-method statuses and response continuation.
//! - `game/Handlers/BattlenetHandler.cpp`: `SendBattlenetResponse`.
//!
//! The character-select "Change Realm" button sends
//! `GameUtilitiesService.GetAllValuesForAttribute("Command_RealmListRequest_v1")`
//! and `ProcessClientRequest(Command_RealmListRequest_v1 / Command_RealmJoinRequest_v1)`
//! to the worldserver. Answering them with `ERROR_RPC_NOT_IMPLEMENTED` (3015)
//! made the client drop to the login screen with `BLZ51903015`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use prost::Message;
use tracing::{debug, error, warn};
use wow_packet::packets::battlenet::BattlenetResponse;
use wow_proto::bgs::protocol::game_utilities::v1::{
    ClientRequest, ClientResponse, GameAccountOfflineNotification, GameAccountOnlineNotification,
    GetAllValuesForAttributeRequest, GetAllValuesForAttributeResponse,
    PresenceChannelCreatedRequest, RegisterUtilitiesRequest, ServerRequest,
    UnregisterUtilitiesRequest,
};
use wow_proto::bgs::protocol::{Attribute, Variant};
use wow_proto::realm_list_json::{self, RealmCharacterCountEntry, RealmCharacterCountList};
use wow_proto::{service_hash, status};

use crate::session::WorldSession;

/// `OriginalHash` of every service C++ `WorldserverServiceDispatcher`
/// registers. Every service except `GameUtilitiesService` is a plain
/// `WorldserverService<T>` whose generated handlers all return
/// `ERROR_RPC_NOT_IMPLEMENTED`.
pub(crate) const WORLDSERVER_SERVICE_HASHES_LIKE_CPP: [u32; 12] = [
    service_hash::ACCOUNT_SERVICE,
    service_hash::AUTHENTICATION_SERVICE,
    service_hash::CLUB_MEMBERSHIP_SERVICE,
    service_hash::CLUB_SERVICE,
    service_hash::CONNECTION_SERVICE,
    service_hash::FRIENDS_SERVICE,
    service_hash::GAME_UTILITIES_SERVICE,
    service_hash::PRESENCE_SERVICE,
    service_hash::REPORT_SERVICE,
    service_hash::REPORT_SERVICE_V2,
    service_hash::RESOURCES_SERVICE,
    service_hash::USER_MANAGER_SERVICE,
];

pub type RealmJoinFutureLikeCpp<'a> =
    Pin<Box<dyn Future<Output = Result<RealmJoinGrantLikeCpp, u32>> + Send + 'a>>;

/// Inputs of C++ `RealmList::JoinRealm` as called by the worldserver
/// `GameUtilitiesService::HandleRealmJoinRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmJoinRequestLikeCpp {
    pub realm_address: u32,
    /// C++ passes the global `realm.Build` (this worldserver's realm build).
    pub build: u32,
    /// `WorldSession::GetRemoteAddress()`.
    pub client_address: String,
    /// `WorldSession::GetRealmListSecret()` from `CMSG_CHANGE_REALM_TICKET`.
    pub client_secret: [u8; 32],
    /// `WorldSession::GetSessionDbcLocale()` as a `LocaleConstant`.
    pub locale: u8,
    pub os: String,
    pub timezone_offset_minutes: i16,
    pub account_name: String,
}

/// The realm-owned half of a successful C++ `RealmList::JoinRealm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealmJoinGrantLikeCpp {
    /// Compressed `JSONRealmListServerIPAddresses` blob.
    pub server_addresses: Vec<u8>,
    /// Random `serverSecret` also stored in `account.session_key_bnet`.
    pub join_secret: [u8; 32],
}

/// The worldserver's view of C++ `sRealmList` plus the global `realm`.
pub trait WorldserverRealmListPortLikeCpp: Send + Sync {
    /// C++ `RealmList::WriteSubRegions`.
    fn sub_regions_like_cpp(&self) -> Vec<String>;

    /// C++ global `realm.Build`.
    fn current_realm_build_like_cpp(&self) -> u32;

    /// C++ `RealmList::GetRealmList(build, subRegion)`: compressed
    /// `JSONRealmListUpdates` blob (never empty on success).
    fn get_realm_list_like_cpp(&self, build: u32, sub_region: &str) -> Vec<u8>;

    /// C++ `RealmList::JoinRealm`, including the
    /// `LOGIN_UPD_BNET_GAME_ACCOUNT_LOGIN_INFO` write. `Err` carries the
    /// Battle.net status C++ returns.
    fn join_realm_like_cpp(&self, request: RealmJoinRequestLikeCpp) -> RealmJoinFutureLikeCpp<'_>;
}

/// What C++ `ServiceBase` sends back for one server method call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BattlenetReplyLikeCpp {
    /// `SendResponse(hash, methodId, token, message)` with `ERROR_OK`.
    Message(Vec<u8>),
    /// `SendResponse(hash, methodId, token, status)`.
    Status(u32),
}

/// C++ `removeSuffix`: strip from the last `_` (`Command_Foo_v1_b9` -> `Command_Foo_v1`).
fn remove_suffix_like_cpp(name: &str) -> &str {
    name.rfind('_').map_or(name, |pos| &name[..pos])
}

/// C++ `HandleProcessClientRequest` parameter scan: the last `Command_*`
/// attribute is the command, command values are keyed by their
/// suffix-stripped name and later duplicates overwrite earlier ones.
fn client_request_params_like_cpp(
    attributes: &[Attribute],
) -> (Option<&Attribute>, HashMap<&str, &Variant>) {
    let mut command = None;
    let mut params = HashMap::new();
    for attribute in attributes {
        if attribute.name.starts_with("Command_") {
            command = Some(attribute);
            params.insert(remove_suffix_like_cpp(&attribute.name), &attribute.value);
        } else {
            params.insert(attribute.name.as_str(), &attribute.value);
        }
    }
    (command, params)
}

fn blob_attribute(name: &str, value: Vec<u8>) -> Attribute {
    Attribute {
        name: name.to_owned(),
        value: Variant {
            blob_value: Some(value),
            ..Default::default()
        },
    }
}

/// C++ `ServerContinuation`: `ERROR_OK` sends the message, anything else the status.
fn continuation_like_cpp<M: Message>(result: Result<M, u32>) -> BattlenetReplyLikeCpp {
    match result {
        Ok(response) => BattlenetReplyLikeCpp::Message(response.encode_to_vec()),
        Err(status) => BattlenetReplyLikeCpp::Status(status),
    }
}

/// C++ generated `ParseAndHandle*` for a request that the worldserver does
/// not override: parse (or `ERROR_RPC_MALFORMED_REQUEST`), then the default
/// handler returns `ERROR_RPC_NOT_IMPLEMENTED`, which is always sent.
fn unimplemented_method_like_cpp<M: Message + Default>(
    method: &str,
    data: &[u8],
) -> BattlenetReplyLikeCpp {
    if M::decode(data).is_err() {
        debug!("Failed to parse request for {method}");
        return BattlenetReplyLikeCpp::Status(status::ERROR_RPC_MALFORMED_REQUEST);
    }
    debug!("Client tried to call not implemented method {method}");
    BattlenetReplyLikeCpp::Status(status::ERROR_RPC_NOT_IMPLEMENTED)
}

/// C++ `JSON::RealmList::RealmCharacterCountList` payload of `HandleRealmListRequest`.
pub(crate) fn realm_character_count_list_blob_like_cpp<'a>(
    counts: impl IntoIterator<Item = (&'a u32, &'a u8)>,
) -> Vec<u8> {
    let list = RealmCharacterCountList {
        counts: counts
            .into_iter()
            .map(|(&wow_realm_address, &count)| RealmCharacterCountEntry {
                wow_realm_address,
                count: u32::from(count),
            })
            .collect(),
    };
    realm_list_json::compress_prefixed_like_cpp(
        realm_list_json::REALM_CHARACTER_COUNT_LIST_PREFIX,
        &list,
    )
}

/// C++ `RealmList::JoinRealm` response attributes, in C++ order.
pub(crate) fn realm_join_response_attributes_like_cpp(
    account_name: &str,
    grant: RealmJoinGrantLikeCpp,
) -> Vec<Attribute> {
    vec![
        blob_attribute("Param_RealmJoinTicket", account_name.as_bytes().to_vec()),
        blob_attribute("Param_ServerAddresses", grant.server_addresses),
        blob_attribute("Param_JoinSecret", grant.join_secret.to_vec()),
    ]
}

impl WorldSession {
    /// C++ `WorldserverServiceDispatcher::Dispatch`.
    pub(crate) async fn dispatch_battlenet_service_like_cpp(
        &mut self,
        service_hash: u32,
        token: u32,
        method_id: u32,
        data: &[u8],
    ) {
        let reply = if service_hash == service_hash::GAME_UTILITIES_SERVICE {
            self.call_game_utilities_server_method_like_cpp(method_id, data)
                .await
        } else if WORLDSERVER_SERVICE_HASHES_LIKE_CPP.contains(&service_hash) {
            // Plain `WorldserverService<T>`: every generated handler returns
            // ERROR_RPC_NOT_IMPLEMENTED. The per-service method tables are not
            // generated here, so unknown method ids of these services also get
            // NOT_IMPLEMENTED instead of ERROR_RPC_INVALID_METHOD.
            debug!(
                account = self.account_id,
                "Client called not implemented Battle.net service 0x{service_hash:08X} method {method_id}"
            );
            BattlenetReplyLikeCpp::Status(status::ERROR_RPC_NOT_IMPLEMENTED)
        } else {
            debug!(
                account = self.account_id,
                "Account {} tried to call invalid service 0x{service_hash:X}", self.account_id
            );
            return;
        };

        self.send_battlenet_reply_like_cpp(service_hash, method_id, token, reply);
    }

    /// C++ `WorldSession::SendBattlenetResponse` (both overloads).
    fn send_battlenet_reply_like_cpp(
        &self,
        service_hash: u32,
        method_id: u32,
        token: u32,
        reply: BattlenetReplyLikeCpp,
    ) {
        let response = match reply {
            BattlenetReplyLikeCpp::Message(data) => {
                BattlenetResponse::message(service_hash, method_id, token, data)
            }
            BattlenetReplyLikeCpp::Status(status) => {
                BattlenetResponse::error(service_hash, method_id, token, status)
            }
        };
        self.send_packet(&response);
    }

    /// C++ `GameUtilitiesService::CallServerMethod`.
    pub(crate) async fn call_game_utilities_server_method_like_cpp(
        &mut self,
        method_id: u32,
        data: &[u8],
    ) -> BattlenetReplyLikeCpp {
        match method_id & 0x3FFF_FFFF {
            1 => match ClientRequest::decode(data) {
                Ok(request) => continuation_like_cpp(
                    self.handle_process_client_request_like_cpp(&request).await,
                ),
                Err(_) => BattlenetReplyLikeCpp::Status(status::ERROR_RPC_MALFORMED_REQUEST),
            },
            2 => unimplemented_method_like_cpp::<PresenceChannelCreatedRequest>(
                "GameUtilitiesService.PresenceChannelCreated",
                data,
            ),
            6 => unimplemented_method_like_cpp::<ServerRequest>(
                "GameUtilitiesService.ProcessServerRequest",
                data,
            ),
            7 => unimplemented_method_like_cpp::<GameAccountOnlineNotification>(
                "GameUtilitiesService.OnGameAccountOnline",
                data,
            ),
            8 => unimplemented_method_like_cpp::<GameAccountOfflineNotification>(
                "GameUtilitiesService.OnGameAccountOffline",
                data,
            ),
            10 => match GetAllValuesForAttributeRequest::decode(data) {
                Ok(request) => continuation_like_cpp(
                    self.handle_get_all_values_for_attribute_like_cpp(&request),
                ),
                Err(_) => BattlenetReplyLikeCpp::Status(status::ERROR_RPC_MALFORMED_REQUEST),
            },
            11 => unimplemented_method_like_cpp::<RegisterUtilitiesRequest>(
                "GameUtilitiesService.RegisterUtilities",
                data,
            ),
            12 => unimplemented_method_like_cpp::<UnregisterUtilitiesRequest>(
                "GameUtilitiesService.UnregisterUtilities",
                data,
            ),
            _ => {
                debug!("Client tried to call invalid GameUtilitiesService method {method_id}");
                BattlenetReplyLikeCpp::Status(status::ERROR_RPC_INVALID_METHOD)
            }
        }
    }

    /// C++ `Battlenet::GameUtilitiesService::HandleProcessClientRequest`.
    async fn handle_process_client_request_like_cpp(
        &mut self,
        request: &ClientRequest,
    ) -> Result<ClientResponse, u32> {
        let (command, params) = client_request_params_like_cpp(&request.attribute);
        let Some(command) = command else {
            error!(
                account = self.account_id,
                "Account {} sent ClientRequest with no command.", self.account_id
            );
            return Err(status::ERROR_RPC_MALFORMED_REQUEST);
        };

        match remove_suffix_like_cpp(&command.name) {
            "Command_RealmListRequest_v1" => self.handle_realm_list_request_like_cpp(&params),
            "Command_RealmJoinRequest_v1" => self.handle_realm_join_request_like_cpp(&params).await,
            other => {
                error!(
                    account = self.account_id,
                    "Account {} sent ClientRequest with unknown command {other}.", self.account_id
                );
                Err(status::ERROR_RPC_NOT_IMPLEMENTED)
            }
        }
    }

    fn worldserver_realm_list_or_status_like_cpp(
        &self,
    ) -> Result<std::sync::Arc<dyn WorldserverRealmListPortLikeCpp>, u32> {
        self.worldserver_realm_list_like_cpp()
            .map(std::sync::Arc::clone)
            .ok_or_else(|| {
                warn!(
                    account = self.account_id,
                    "Worldserver RealmList capability is not installed"
                );
                status::ERROR_RPC_NOT_IMPLEMENTED
            })
    }

    /// C++ `Battlenet::GameUtilitiesService::HandleRealmListRequest`.
    fn handle_realm_list_request_like_cpp(
        &self,
        params: &HashMap<&str, &Variant>,
    ) -> Result<ClientResponse, u32> {
        let realm_list = self.worldserver_realm_list_or_status_like_cpp()?;
        let sub_region = params
            .get("Command_RealmListRequest_v1")
            .and_then(|value| value.string_value.as_deref())
            .unwrap_or_default();

        let compressed = realm_list
            .get_realm_list_like_cpp(realm_list.current_realm_build_like_cpp(), sub_region);
        if compressed.is_empty() {
            return Err(status::ERROR_UTIL_SERVER_FAILED_TO_SERIALIZE_RESPONSE);
        }

        Ok(ClientResponse {
            attribute: vec![
                blob_attribute("Param_RealmList", compressed),
                blob_attribute(
                    "Param_CharacterCountList",
                    realm_character_count_list_blob_like_cpp(
                        self.realm_character_counts_like_cpp(),
                    ),
                ),
            ],
        })
    }

    /// C++ `Battlenet::GameUtilitiesService::HandleRealmJoinRequest`.
    async fn handle_realm_join_request_like_cpp(
        &self,
        params: &HashMap<&str, &Variant>,
    ) -> Result<ClientResponse, u32> {
        let Some(realm_address) = params.get("Param_RealmAddress") else {
            return Err(status::ERROR_WOW_SERVICES_INVALID_JOIN_TICKET);
        };
        let realm_list = self.worldserver_realm_list_or_status_like_cpp()?;
        let request = RealmJoinRequestLikeCpp {
            realm_address: realm_address.uint_value.unwrap_or_default() as u32,
            build: realm_list.current_realm_build_like_cpp(),
            client_address: self
                .remote_address_like_cpp()
                .unwrap_or_default()
                .to_owned(),
            client_secret: *self.realm_list_secret_like_cpp(),
            locale: crate::battle_pay::locale_index_from_name_like_cpp(
                self.session_locale_name_like_cpp(),
            )
            .unwrap_or(0),
            os: self.os_like_cpp().to_owned(),
            timezone_offset_minutes: self.timezone_offset_minutes_like_cpp(),
            account_name: self.account_name.clone(),
        };
        let grant = realm_list.join_realm_like_cpp(request).await?;
        Ok(ClientResponse {
            attribute: realm_join_response_attributes_like_cpp(&self.account_name, grant),
        })
    }

    /// C++ `Battlenet::GameUtilitiesService::HandleGetAllValuesForAttribute`.
    fn handle_get_all_values_for_attribute_like_cpp(
        &self,
        request: &GetAllValuesForAttributeRequest,
    ) -> Result<GetAllValuesForAttributeResponse, u32> {
        if !request
            .attribute_key
            .as_deref()
            .unwrap_or_default()
            .starts_with("Command_RealmListRequest_v1")
        {
            return Err(status::ERROR_RPC_NOT_IMPLEMENTED);
        }

        let realm_list = self.worldserver_realm_list_or_status_like_cpp()?;
        Ok(GetAllValuesForAttributeResponse {
            attribute_value: realm_list
                .sub_regions_like_cpp()
                .into_iter()
                .map(|sub_region| Variant {
                    string_value: Some(sub_region),
                    ..Default::default()
                })
                .collect(),
        })
    }
}

#[cfg(test)]
#[path = "bnet_services_tests.rs"]
mod tests;
