// Copyright (c) 2026 alseif0x
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Character undelete handlers (TrinityCore 3.4.3, tag TDB343.24081):
//! `HandleCharUndeleteEnumOpcode`, `HandleGetUndeleteCooldownStatus` and
//! `HandleCharUndeleteOpcode` (`CharacterHandler.cpp:427-441, 2612-2735`). The
//! registrations match `Opcodes.cpp:444, 488, 986` (Authed, ThreadUnsafe). The
//! decisions and persistence order live in `crate::character_undelete`.

use wow_packet::ClientPacket;
use wow_packet::packets::character::{
    DeletedEnumCharactersResult, EnumCharactersResult, UndeleteCharacter, UndeleteCharacterResponse,
};
use wow_persistence::CharacterUndeletePersistencePortLikeCpp;

use super::*;
use crate::character_undelete::{
    CHAR_DELETE_REMOVE_LIKE_CPP, CharacterDeletionServiceLikeCpp, UndeleteRequestLikeCpp,
    delete_method_like_cpp, undelete_character_like_cpp, undelete_cooldown_status_like_cpp,
    unix_now_like_cpp,
};

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::EnumCharactersDeletedByClient,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_enum_characters_deleted_by_client",
        handler: |session, catalogs, _pkt| {
            Box::pin(async move {
                session
                    .handle_enum_characters_deleted_by_client_like_cpp(
                        catalogs.support_feature_policy.as_ref(),
                    )
                    .await
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::UndeleteCharacter,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_undelete_character",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                match UndeleteCharacter::read(&mut pkt) {
                    Ok(request) => {
                        session
                            .handle_undelete_character_like_cpp(
                                catalogs.character_deletion.as_ref(),
                                catalogs.support_feature_policy.as_ref(),
                                request,
                            )
                            .await
                    }
                    Err(e) => tracing::warn!("Failed to read UndeleteCharacter: {e}"),
                }
            })
        },
    }
}

impl WorldSession {
    pub(super) fn send_enum_characters_result_like_cpp(
        &self,
        deleted_characters: bool,
        result: EnumCharactersResult,
    ) {
        if deleted_characters {
            self.send_packet(&DeletedEnumCharactersResult(result));
        } else {
            self.send_packet(&result);
        }
    }

    /// `Player::DeleteFromDB` method selection; the unlink port is returned only
    /// when the unlink method was chosen and is available.
    pub(super) async fn select_char_delete_method_like_cpp(
        &self,
        deletion: &CharacterDeletionServiceLikeCpp,
        guid_low: u64,
    ) -> (
        u32,
        Option<Arc<dyn CharacterUndeletePersistencePortLikeCpp>>,
    ) {
        let config = deletion.config();
        let Some(port) = deletion.port() else {
            if config.delete_method != CHAR_DELETE_REMOVE_LIKE_CPP {
                warn!(
                    "CharDelete.Method {} configured without an undelete port; removing",
                    config.delete_method
                );
            }
            return (CHAR_DELETE_REMOVE_LIKE_CPP, None);
        };
        let candidate = match port.load_delete_candidate_like_cpp(guid_low).await {
            wow_persistence::CharacterAdministrationLoadOutcomeLikeCpp::Loaded(candidate) => {
                Some(candidate)
            }
            _ => None,
        };
        (
            delete_method_like_cpp(&config, candidate),
            Some(Arc::clone(port)),
        )
    }

    /// Handle CMSG_ENUM_CHARACTERS_DELETED_BY_CLIENT (`HandleCharUndeleteEnumOpcode`).
    pub async fn handle_enum_characters_deleted_by_client_like_cpp(
        &mut self,
        policy: &crate::session::SupportFeaturePolicyLikeCpp,
    ) {
        self.enum_characters_like_cpp(policy, true).await;
    }

    /// Handle CMSG_GET_UNDELETE_CHARACTER_COOLDOWN_STATUS
    /// (`HandleGetUndeleteCooldownStatus` + `HandleUndeleteCooldownStatusCallback`).
    pub async fn handle_get_undelete_cooldown_status(
        &mut self,
        deletion: &CharacterDeletionServiceLikeCpp,
    ) {
        let response = undelete_cooldown_status_like_cpp(
            deletion.port().map(|port| port.as_ref()),
            self.battlenet_account_id(),
            deletion.config().undelete_cooldown_secs,
            unix_now_like_cpp(),
        )
        .await;
        self.send_packet(&response);
    }

    /// Handle CMSG_UNDELETE_CHARACTER (`HandleCharUndeleteOpcode`).
    pub async fn handle_undelete_character_like_cpp(
        &mut self,
        deletion: &CharacterDeletionServiceLikeCpp,
        policy: &crate::session::SupportFeaturePolicyLikeCpp,
        request: UndeleteCharacter,
    ) {
        let result = undelete_character_like_cpp(
            deletion.port().map(|port| port.as_ref()),
            UndeleteRequestLikeCpp {
                guid_low: request.character_guid.counter() as u64,
                account_id: self.account_id,
                battlenet_account_id: self.battlenet_account_id(),
                enabled: policy.character_undelete_enabled,
                max_cooldown: deletion.config().undelete_cooldown_secs,
                characters_per_realm: policy.max_characters_per_realm,
                now: unix_now_like_cpp(),
            },
        )
        .await;
        info!(
            "Account {} undelete of {:?}: result {}",
            self.account_id, request.character_guid, result
        );
        self.send_packet(&UndeleteCharacterResponse {
            client_token: request.client_token,
            result,
            character_guid: request.character_guid,
        });
    }
}
