// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! `CMSG_CHAR_RACE_OR_FACTION_CHANGE` (C++ `WorldSession::HandleCharRaceOrFactionChangeOpcode`
//! + `HandleCharRaceOrFactionChangeCallback` + `SendCharFactionChange`,
//! `Handlers/CharacterHandler.cpp:2010-2558,2800-2816`, TDB343.24081;
//! `Opcodes.cpp:299` STATUS_AUTHED / PROCESS_THREADUNSAFE).
//!
//! The rules live in `crate::character_race_faction_change`; this file owns the
//! packet, the persistence port and the group runtime. RustyCore has no arena
//! teams, so the `CHAR_CREATE_CHARACTER_ARENA_LEADER` check and
//! `Player::LeaveAllArenaTeams` have nothing to act on.

use std::collections::HashMap;

use wow_packet::packets::character::{
    CharFactionChangeDisplayInfo, CharFactionChangeResult, CharRaceOrFactionChange,
};
use wow_persistence::{
    CharacterAdministrationLoadOutcomeLikeCpp as LoadOutcome,
    CharacterAdministrationMutationOutcomeLikeCpp as MutationOutcome,
    CharacterCustomizationPersistenceLikeCpp,
};

use crate::character_race_faction_change::{
    CHAR_CREATE_ERROR_LIKE_CPP, CHAR_CREATE_NAME_IN_USE_LIKE_CPP, RESPONSE_SUCCESS_LIKE_CPP,
    RaceFactionChangeCatalogLikeCpp, RaceFactionChangeCommitInputLikeCpp,
    RaceFactionChangeRequestLikeCpp, build_commit_like_cpp, plan_race_or_faction_change_like_cpp,
    reputation_pairs_like_cpp,
};

use super::*;

/// `AccountTypes::SEC_MODERATOR`: stands in for the RBAC permissions 16/17
/// `RBAC_PERM_SKIP_CHECK_CHARACTER_CREATION_RACEMASK` / `_RESERVEDNAME`, linked to
/// role 194 "Sec Level Moderator" (and inherited by GM/Admin) in TC's default
/// `rbac_linked_permissions`; RustyCore has no RBAC runtime.
const SEC_MODERATOR_LIKE_CPP: u8 = 1;

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::CharRaceOrFactionChange,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_char_race_or_faction_change",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                match CharRaceOrFactionChange::read(&mut pkt) {
                    Ok(request) => {
                        session
                            .handle_char_race_or_faction_change_like_cpp(
                                catalogs.race_faction_change.as_ref(),
                                catalogs.player_bootstrap.create_info.as_ref(),
                                request,
                            )
                            .await
                    }
                    Err(e) => tracing::warn!("Failed to read CharRaceOrFactionChange: {e}"),
                }
            })
        },
    }
}

impl WorldSession {
    /// C++ `SendCharFactionChange`: the display block only on success.
    fn send_char_faction_change_like_cpp(
        &self,
        result: u8,
        request: &CharRaceOrFactionChange,
        name: &str,
    ) {
        self.send_packet(&CharFactionChangeResult {
            result,
            guid: request.guid,
            display: (result == RESPONSE_SUCCESS_LIKE_CPP).then(|| CharFactionChangeDisplayInfo {
                name: name.to_owned(),
                sex_id: request.sex_id,
                race_id: request.race_id,
                customizations: request.customizations.clone(),
            }),
        });
    }

    /// Handle CMSG_CHAR_RACE_OR_FACTION_CHANGE.
    pub async fn handle_char_race_or_faction_change_like_cpp(
        &mut self,
        catalog: &RaceFactionChangeCatalogLikeCpp,
        create_info: &wow_data::PlayerCreateInfoStoreLikeCpp,
        request: CharRaceOrFactionChange,
    ) {
        if !self.is_legit_character(&request.guid) {
            warn!(
                "Account {} tried to factionchange character {:?}, but it does not belong to their account!",
                self.account_id, request.guid
            );
            self.kick(
                "WorldSession::HandleCharFactionOrRaceChange Trying to change faction of character of another account",
            );
            return;
        }
        let error = |session: &Self, result: u8| {
            session.send_char_faction_change_like_cpp(result, &request, &request.name);
        };
        let Some(port) = self.character_administration_persistence_port_like_cpp() else {
            return error(self, CHAR_CREATE_ERROR_LIKE_CPP);
        };
        let guid = request.guid.counter() as u64;
        let candidate = match port
            .load_race_or_faction_change_candidate_like_cpp(guid)
            .await
        {
            LoadOutcome::Loaded(candidate) => candidate,
            LoadOutcome::NotFound => return error(self, CHAR_CREATE_ERROR_LIKE_CPP),
            LoadOutcome::Failed { reason } => {
                warn!("Character race/faction change query failed: {reason}");
                return error(self, CHAR_CREATE_ERROR_LIKE_CPP);
            }
        };

        let plan = match plan_race_or_faction_change_like_cpp(
            catalog,
            &candidate,
            RaceFactionChangeRequestLikeCpp {
                faction_change: request.faction_change,
                race: request.race_id,
                name: &request.name,
            },
            create_info.get(request.race_id, candidate.class).is_some(),
            self.security >= SEC_MODERATOR_LIKE_CPP,
        ) {
            Ok(plan) => plan,
            Err(result) => return error(self, result),
        };

        // `sCharacterCache->GetCharacterGuidByName`: only another character's name
        // is in use; the stored names are normalized, so the own name is `old_name`.
        if plan.name != candidate.name {
            match port.find_character_name_like_cpp(&plan.name).await {
                LoadOutcome::Loaded(()) => return error(self, CHAR_CREATE_NAME_IN_USE_LIKE_CPP),
                LoadOutcome::NotFound => {}
                LoadOutcome::Failed { reason } => {
                    warn!("Character race/faction change name query failed: {reason}");
                    return error(self, CHAR_CREATE_ERROR_LIKE_CPP);
                }
            }
        }

        // C++ reads each old standing (synchronously) while building the transaction.
        let mut old_standings = HashMap::new();
        if request.faction_change && candidate.race != request.race_id {
            for (_, old_faction) in reputation_pairs_like_cpp(catalog, plan.new_team) {
                match port
                    .load_reputation_standing_like_cpp(guid, old_faction)
                    .await
                {
                    LoadOutcome::Loaded(standing) => {
                        old_standings.insert(old_faction, standing);
                    }
                    LoadOutcome::NotFound => {}
                    LoadOutcome::Failed { reason } => {
                        warn!("Character faction change reputation query failed: {reason}");
                        return error(self, CHAR_CREATE_ERROR_LIKE_CPP);
                    }
                }
            }
        }

        let commit = match build_commit_like_cpp(
            catalog,
            guid,
            &candidate,
            RaceFactionChangeCommitInputLikeCpp {
                faction_change: request.faction_change,
                race: request.race_id,
                sex: request.sex_id,
                customizations: request
                    .customizations
                    .iter()
                    .map(|choice| CharacterCustomizationPersistenceLikeCpp {
                        option_id: choice.option_id,
                        choice_id: choice.choice_id,
                    })
                    .collect(),
            },
            &plan,
            &old_standings,
        ) {
            Ok(commit) => commit,
            Err(result) => {
                warn!(
                    "Could not find language data for race ({}).",
                    request.race_id
                );
                return error(self, result);
            }
        };

        // `group->RemoveMember(guid)` runs while C++ builds the transaction.
        if commit.faction.is_some()
            && candidate.group_id != 0
            && !catalog.allow_two_side_interaction_group
        {
            self.remove_offline_group_member_like_cpp(candidate.group_id, request.guid)
                .await;
        }

        if let MutationOutcome::Failed { reason } =
            port.commit_race_or_faction_change_like_cpp(commit).await
        {
            warn!("Character race/faction change transaction failed: {reason}");
            return error(self, CHAR_CREATE_ERROR_LIKE_CPP);
        }

        tracing::debug!(
            "Account {} changed race of {:?} from {} to {}",
            self.account_id,
            request.guid,
            candidate.race,
            request.race_id
        );
        self.send_char_faction_change_like_cpp(RESPONSE_SUCCESS_LIKE_CPP, &request, &plan.name);
    }
}

#[cfg(test)]
#[path = "race_faction_change/tests.rs"]
mod tests;
