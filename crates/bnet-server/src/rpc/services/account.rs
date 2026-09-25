//! Account service handler (hash 0x62DA0891).
//!
//! C++ reference (`TrinityCore` `wotlk_classic`): `src/server/bnetserver/Server/Session.cpp`
//! `Battlenet::Session::HandleGetAccountState` / `HandleGetGameAccountState`.
//! The `WoW` 3.4.3 client fills the login screen game-account dropdown from the
//! `GameLevelInfo.name` (`"WoW1"`) returned here for each `LogonResult.game_account_id`.

use anyhow::Result;
use prost::Message;
use wow_proto::bgs::protocol::account::v1::*;
use wow_proto::status;

use crate::rpc::session::{RpcSession, RpcStatusError};
use crate::state::AccountInfo;
use tokio::io::{AsyncRead, AsyncWrite};

/// `GameLevelInfo.program` / `GameStatus.program` value used by C++ (`5730135 // WoW`).
const PROGRAM_WOW_LIKE_CPP: u32 = 5_730_135;
const PRIVACY_INFO_TAG_LIKE_CPP: u32 = 0xD7CA_834D;
const GAME_LEVEL_INFO_TAG_LIKE_CPP: u32 = 0x5C46_D483;
const GAME_STATUS_TAG_LIKE_CPP: u32 = 0x98B7_5F99;

pub async fn handle<S: AsyncRead + AsyncWrite + Unpin>(
    session: &mut RpcSession<S>,
    method_id: u32,
    payload: &[u8],
) -> Result<Option<Vec<u8>>> {
    match method_id {
        30 => handle_get_account_state(session, payload).await,
        31 => handle_get_game_account_state(session, payload).await,
        _ => {
            tracing::warn!("AccountService: unknown method {method_id}");
            Ok(None)
        }
    }
}

/// Method 30: `GetAccountState`
async fn handle_get_account_state<S: AsyncRead + AsyncWrite + Unpin>(
    session: &mut RpcSession<S>,
    payload: &[u8],
) -> Result<Option<Vec<u8>>> {
    let request = GetAccountStateRequest::decode(payload)?;
    let response =
        get_account_state_like_cpp(session.authed, &request).map_err(RpcStatusError::new)?;
    Ok(Some(response.encode_to_vec()))
}

/// Method 31: `GetGameAccountState`
async fn handle_get_game_account_state<S: AsyncRead + AsyncWrite + Unpin>(
    session: &mut RpcSession<S>,
    payload: &[u8],
) -> Result<Option<Vec<u8>>> {
    let request = GetGameAccountStateRequest::decode(payload)?;
    let response =
        get_game_account_state_like_cpp(session.authed, session.account_info.as_ref(), &request)
            .map_err(RpcStatusError::new)?;
    tracing::debug!(
        "AccountService: GetGameAccountState game_account={:?} -> {:?}",
        request.game_account_id.as_ref().map(|id| id.low),
        response
    );
    Ok(Some(response.encode_to_vec()))
}

/// Mirrors `Session::HandleGetAccountState`: only `field_privacy_info` is answered.
fn get_account_state_like_cpp(
    authed: bool,
    request: &GetAccountStateRequest,
) -> std::result::Result<GetAccountStateResponse, u32> {
    if !authed {
        return Err(status::ERROR_DENIED);
    }

    let mut response = GetAccountStateResponse::default();
    let options = request.options.unwrap_or_default();
    if options.field_privacy_info() {
        response.state = Some(AccountState {
            privacy_info: Some(PrivacyInfo {
                is_using_rid: Some(false),
                is_visible_for_view_friends: Some(false),
                is_hidden_from_friend_finder: Some(true),
            }),
        });
        response.tags = Some(AccountFieldTags {
            privacy_info_tag: Some(PRIVACY_INFO_TAG_LIKE_CPP),
        });
    }

    Ok(response)
}

/// Mirrors `Session::HandleGetGameAccountState`.
fn get_game_account_state_like_cpp(
    authed: bool,
    account: Option<&AccountInfo>,
    request: &GetGameAccountStateRequest,
) -> std::result::Result<GetGameAccountStateResponse, u32> {
    if !authed {
        return Err(status::ERROR_DENIED);
    }

    let mut response = GetGameAccountStateResponse::default();
    let options = request.options.unwrap_or_default();
    // C++: `_accountInfo->GameAccounts.find(request->game_account_id().low())` (uint32 key).
    let game_account_id = request
        .game_account_id
        .as_ref()
        .map_or(0, |id| id.low as u32);
    let game_account = account.and_then(|a| a.game_accounts.get(&game_account_id));

    if options.field_game_level_info() {
        let state = response.state.get_or_insert_with(Default::default);
        let level_info = state.game_level_info.get_or_insert_with(Default::default);
        if let Some(game_account) = game_account {
            level_info.name = Some(game_account.display_name.clone());
            level_info.program = Some(PROGRAM_WOW_LIKE_CPP);
        }
        response
            .tags
            .get_or_insert_with(Default::default)
            .game_level_info_tag = Some(GAME_LEVEL_INFO_TAG_LIKE_CPP);
    }

    if options.field_game_status() {
        let state = response.state.get_or_insert_with(Default::default);
        let game_status = state.game_status.get_or_insert_with(Default::default);
        if let Some(game_account) = game_account {
            game_status.is_suspended = Some(game_account.is_banned);
            game_status.is_banned = Some(game_account.is_permanently_banned);
            game_status.suspension_expires = Some(game_account.unban_date * 1_000_000);
        }
        game_status.program = Some(PROGRAM_WOW_LIKE_CPP);
        response
            .tags
            .get_or_insert_with(Default::default)
            .game_status_tag = Some(GAME_STATUS_TAG_LIKE_CPP);
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GameAccountInfo;
    use std::collections::HashMap;
    use wow_proto::bgs::protocol::EntityId;

    fn account_with_game_account() -> AccountInfo {
        let mut game_accounts = HashMap::new();
        game_accounts.insert(
            1,
            GameAccountInfo {
                id: 1,
                name: "1#1".to_string(),
                display_name: "WoW1".to_string(),
                unban_date: 0,
                is_permanently_banned: false,
                is_banned: false,
                security_level: 0,
                char_counts: HashMap::new(),
                last_played_chars: HashMap::new(),
            },
        );
        AccountInfo {
            id: 1,
            login: "INNA@INNA.CL".to_string(),
            is_locked_to_ip: false,
            lock_country: String::new(),
            last_ip: String::new(),
            failed_logins: 0,
            is_banned: false,
            is_permanently_banned: false,
            game_accounts,
        }
    }

    fn game_account_request(low: u64, level: bool, status: bool) -> GetGameAccountStateRequest {
        GetGameAccountStateRequest {
            game_account_id: Some(EntityId {
                high: 0x0200_0002_0057_6F57,
                low,
            }),
            options: Some(GameAccountFieldOptions {
                field_game_level_info: Some(level),
                field_game_status: Some(status),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn game_account_state_returns_display_name_like_cpp() {
        let account = account_with_game_account();
        let response = get_game_account_state_like_cpp(
            true,
            Some(&account),
            &game_account_request(1, true, false),
        )
        .unwrap();
        let level = response
            .state
            .as_ref()
            .unwrap()
            .game_level_info
            .as_ref()
            .unwrap();
        assert_eq!(level.name.as_deref(), Some("WoW1"));
        assert_eq!(level.program, Some(0x0057_6F57));
        assert!(response.state.as_ref().unwrap().game_status.is_none());
        assert_eq!(
            response.tags.unwrap().game_level_info_tag,
            Some(0x5C46_D483)
        );
    }

    // Wire layout must match TrinityCore account_types.pb.h: GetGameAccountStateResponse
    // state=1, GameAccountState game_level_info=1 / game_status=3, GameLevelInfo name=8 /
    // program=9 (fixed32), GameStatus is_suspended=4 / is_banned=5 / suspension_expires=6 /
    // program=7, GameAccountFieldTags game_level_info_tag=2 / game_status_tag=4.
    #[test]
    fn game_account_state_wire_bytes_match_cpp_field_numbers() {
        let account = account_with_game_account();
        let response = get_game_account_state_like_cpp(
            true,
            Some(&account),
            &game_account_request(1, true, true),
        )
        .unwrap();
        assert_eq!(
            response.encode_to_vec(),
            vec![
                0x0A, 0x1A, // state (1), len 26
                0x0A, 0x0B, // game_level_info (1), len 11
                0x42, 0x04, b'W', b'o', b'W', b'1', // name (8)
                0x4D, 0x57, 0x6F, 0x57, 0x00, // program (9), fixed32
                0x1A, 0x0B, // game_status (3), len 11
                0x20, 0x00, // is_suspended (4)
                0x28, 0x00, // is_banned (5)
                0x30, 0x00, // suspension_expires (6)
                0x3D, 0x57, 0x6F, 0x57, 0x00, // program (7), fixed32
                0x12, 0x0A, // tags (2), len 10
                0x15, 0x83, 0xD4, 0x46, 0x5C, // game_level_info_tag (2)
                0x25, 0x99, 0x5F, 0xB7, 0x98, // game_status_tag (4)
            ]
        );
    }

    #[test]
    fn game_account_request_options_decode_with_cpp_field_numbers() {
        // GameAccountFieldOptions: field_game_level_info=2, field_game_time_info=3,
        // field_game_status=4 (time info is not modelled and must not alias status).
        let options = GameAccountFieldOptions::decode(&[0x10, 0x01, 0x18, 0x01][..]).unwrap();
        assert!(options.field_game_level_info());
        assert!(!options.field_game_status());
        let options = GameAccountFieldOptions::decode(&[0x20, 0x01][..]).unwrap();
        assert!(options.field_game_status());
    }

    #[test]
    fn game_account_status_maps_ban_flags_like_cpp() {
        let mut account = account_with_game_account();
        let ga = account.game_accounts.get_mut(&1).unwrap();
        ga.is_banned = true;
        ga.unban_date = 1_700_000_000;
        let response = get_game_account_state_like_cpp(
            true,
            Some(&account),
            &game_account_request(1, false, true),
        )
        .unwrap();
        let state = response.state.unwrap();
        assert!(state.game_level_info.is_none());
        let status = state.game_status.unwrap();
        assert_eq!(status.is_suspended, Some(true));
        assert_eq!(status.is_banned, Some(false));
        assert_eq!(status.suspension_expires, Some(1_700_000_000_000_000));
        assert_eq!(status.program, Some(0x0057_6F57));
    }

    #[test]
    fn unknown_game_account_keeps_tags_but_no_name_like_cpp() {
        let account = account_with_game_account();
        let response = get_game_account_state_like_cpp(
            true,
            Some(&account),
            &game_account_request(7, true, true),
        )
        .unwrap();
        let state = response.state.unwrap();
        let level = state.game_level_info.unwrap();
        assert_eq!(level.name, None);
        assert_eq!(level.program, None);
        let status = state.game_status.unwrap();
        assert_eq!(status.is_suspended, None);
        assert_eq!(status.program, Some(0x0057_6F57));
        let tags = response.tags.unwrap();
        assert_eq!(tags.game_level_info_tag, Some(0x5C46_D483));
        assert_eq!(tags.game_status_tag, Some(0x98B7_5F99));
    }

    #[test]
    fn account_services_deny_unauthed_like_cpp() {
        assert_eq!(
            get_game_account_state_like_cpp(false, None, &game_account_request(1, true, true)),
            Err(status::ERROR_DENIED)
        );
        assert_eq!(
            get_account_state_like_cpp(false, &GetAccountStateRequest::default()),
            Err(status::ERROR_DENIED)
        );
    }

    #[test]
    fn account_state_privacy_info_wire_bytes_match_cpp_field_numbers() {
        // AccountFieldOptions.field_privacy_info = 3.
        let request = GetAccountStateRequest {
            options: Some(AccountFieldOptions::decode(&[0x18, 0x01][..]).unwrap()),
            ..Default::default()
        };
        let response = get_account_state_like_cpp(true, &request).unwrap();
        assert_eq!(
            response.encode_to_vec(),
            vec![
                0x0A, 0x08, // state (1)
                0x12, 0x06, // privacy_info (2)
                0x18, 0x00, 0x20, 0x00, 0x28, 0x01, // is_using_rid 3, visible 4, hidden 5
                0x12, 0x05, // tags (2)
                0x1D, 0x4D, 0x83, 0xCA, 0xD7, // privacy_info_tag (3)
            ]
        );

        let empty = get_account_state_like_cpp(true, &GetAccountStateRequest::default()).unwrap();
        assert!(empty.encode_to_vec().is_empty());
    }
}
