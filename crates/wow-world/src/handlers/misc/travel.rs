// Copyright (c) 2026 alseif0x
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Private travel capability handlers extracted from the legacy misc owner.

use tracing::{debug, info, warn};
use wow_constants::{ClientOpcodes, ConditionSourceType, ConditionType};
use wow_handler::{PacketProcessing, SessionStatus};

use crate::session::registry::PacketHandlerEntry;
use wow_packet::ClientPacket;
use wow_packet::packets::misc::{
    ActivateTaxi, ActivateTaxiReply, ERR_TAXITOOFARAWAY_LIKE_CPP, SetTaxiBenchmarkMode,
    TaxiNodeStatusPkt,
};

use crate::session::{AreaTriggerCatalogsLikeCpp, RepresentedActivateTaxiLikeCpp};

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::ActivateTaxi,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadSafe,
        handler_name: "handle_activate_taxi",
        handler: |session, _catalogs, pkt| Box::pin(async move { session.handle_activate_taxi(pkt).await }),
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::AreaTrigger,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::Inplace,
        handler_name: "handle_area_trigger",
        handler: |session, catalogs, pkt| {
            Box::pin(async move {
                session
                    .handle_area_trigger_with_catalogs_like_cpp(
                        catalogs.area_triggers.as_ref(),
                        catalogs.id_generators.item.as_ref(),
                        pkt,
                    )
                    .await
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::WorldPortResponse,
        status: SessionStatus::Transfer,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_world_port_response",
        handler: |session, catalogs, pkt| {
            Box::pin(async move {
                session
                    .handle_world_port_response_with_catalogs_like_cpp(
                        catalogs.creature_spawns.as_ref(),
                        catalogs.player_bootstrap.trait_node_entries.as_ref(),
                        catalogs.id_generators.item.as_ref(),
                        pkt,
                    )
                    .await
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::SuspendTokenResponse,
        status: SessionStatus::Transfer,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_suspend_token_response",
        handler: |session, _catalogs, pkt| {
            Box::pin(async move { session.handle_suspend_token_response(pkt).await })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::TaxiNodeStatusQuery,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadSafe,
        handler_name: "handle_taxi_node_status_query",
        handler: |session, _catalogs, pkt| {
            Box::pin(async move { session.handle_taxi_node_status_query(pkt).await })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::SetTaxiBenchmarkMode,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::Inplace,
        handler_name: "handle_set_taxi_benchmark_mode",
        handler: |session, _catalogs, pkt| {
            Box::pin(async move { session.handle_set_taxi_benchmark_mode(pkt).await })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::UpdateAreaTriggerVisual,
        status: SessionStatus::Authed,
        processing: PacketProcessing::Inplace,
        handler_name: "handle_update_area_trigger_visual",
        handler: |session, _catalogs, pkt| {
            Box::pin(async move { session.handle_update_area_trigger_visual(pkt).await })
        },
    }
}

impl crate::session::WorldSession {
    /// C++ `Map::SendInitSelf` (Map.cpp:1826), invoked by `Map::AddPlayerToMap(initPlayer=true)`
    /// on a non-seamless far teleport (HandleMoveWorldportAck -> AddPlayerToMap, Map.cpp:427-463).
    /// Re-sends the player's OWN object (ActivePlayer create block) so the client finishes the
    /// loading screen and enters the destination map. Sourced from session state; combat stats
    /// are placeholders here (health from the live value, the rest defaulted) and corrected by
    /// the `send_stat_update` that follows. Inventory item objects are not yet re-sent on
    /// teleport, unlike Player.cpp:3586-3608 — an open #NEXT.R8.ENTITIES.1229 parity gap.
    /// None means unavailable Player projection; Some reports channel acceptance only.
    /// A rejected send must not suppress the remaining native worldport effects.
    pub(super) async fn send_player_self_create_for_teleport_like_cpp(
        &mut self,
        trait_node_entries: &wow_data::trait_tree::TraitNodeEntryStore,
    ) -> Option<bool> {
        let _ = trait_node_entries; // Compatibility until the caller's catalog contract is narrowed.
        let Some(player_pkt) = self.prepare_player_self_create_for_teleport_like_cpp() else {
            return None;
        };
        let sent = self.send_packet(&player_pkt);
        info!(
            account = self.account_id,
            map = player_pkt.map_id,
            accepted = sent,
            "[FAR_TELEPORT] prepared SendInitSelf (player ActivePlayer create) for destination map"
        );
        Some(sent)
    }
    /// CMSG_SUSPEND_TOKEN_RESPONSE — client acknowledges SMSG_SUSPEND_TOKEN during a far
    /// teleport. C++ `WorldSession::HandleSuspendTokenResponse` (MovementHandler.cpp:239)
    /// replies with SMSG_NEW_WORLD so the client loads the destination map; only then does
    /// the client send CMSG_WORLD_PORT_RESPONSE. Without this step the client sits on the
    /// loading screen at 0% forever. #NEXT.R8.ENTITIES.1229.
    pub async fn handle_suspend_token_response(&mut self, _pkt: wow_packet::WorldPacket) {
        if self.state() == crate::session::SessionState::Disconnecting {
            return;
        }
        if !self.represented_far_teleport_pending_like_cpp() {
            return;
        }
        let Some((new_map, new_pos)) = self.pending_teleport_like_cpp() else {
            return;
        };
        let packet = wow_packet::packets::misc::NewWorld {
            map_id: new_map,
            pos: new_pos,
            reason: 16, // C++ Player.h NEW_WORLD_NORMAL (not the seamless value 21).
        };
        if self
            .realm_route_tx()
            .send(wow_packet::ServerPacket::to_bytes(&packet))
            .is_err()
        {
            self.kick("worldport NewWorld could not be queued");
            return;
        }
        self.recovery_new_world_sent_like_cpp();
        info!(
            account = self.account_id,
            map = new_map,
            "[FAR_TELEPORT] SuspendTokenResponse -> sent SMSG_NEW_WORLD (client now loads destination map)"
        );
    }

    /// CMSG_WORLD_PORT_RESPONSE — client confirms it has loaded the new map.
    /// Admission anchor: C++ `WorldSession::HandleMoveWorldportAck` in MovementHandler.cpp.
    /// Sent after SMSG_NEW_WORLD (which is emitted from handle_suspend_token_response).
    /// We respond with SMSG_RESUME_TOKEN and replay the after-add init.

    pub async fn handle_world_port_response_with_catalogs_like_cpp(
        &mut self,
        creature_spawn_catalogs: &crate::session::CreatureSpawnCatalogsLikeCpp,
        trait_node_entries: &wow_data::trait_tree::TraitNodeEntryStore,
        item_guid_generator: &wow_core::ObjectGuidGenerator,
        _pkt: wow_packet::WorldPacket,
    ) {
        use wow_packet::packets::misc::ResumeToken;

        if self.state() == crate::session::SessionState::Disconnecting
            || !self.recovery_worldport_ack_ready_like_cpp()
        {
            return;
        }
        if !self.represented_far_teleport_pending_like_cpp() {
            warn!(
                "WorldPortResponse from account {} but far teleport semaphore is not set",
                self.account_id
            );
            return;
        }
        let Some((new_map, new_pos)) = self.pending_teleport_like_cpp() else {
            warn!(
                "WorldPortResponse from account {} but no pending teleport",
                self.account_id
            );
            return;
        };
        // C++ MovementHandler.cpp:90-134 does not continue successful entry
        // after failed admission/add. Retain the pending transfer until the
        // same Player is attached; recovery must not save destination coordinates
        // paired with the old map or publish a false LoggedIn transition.
        if !self.try_attach_worldport_destination_like_cpp(new_map, new_pos) {
            warn!(
                account = self.account_id,
                "WorldPortResponse could not attach its Player; transfer remains pending"
            );
            self.recover_rejected_worldport_like_cpp().await;
            return;
        }
        self.set_represented_far_teleport_pending_like_cpp(false);
        if !self.set_pending_teleport_like_cpp(None) {
            return;
        }

        info!(
            account = self.account_id,
            "WorldPortResponse: completing teleport to map {} ({:.2}, {:.2}, {:.2})",
            new_map,
            new_pos.x,
            new_pos.y,
            new_pos.z
        );

        self.update_registry_position();

        // SMSG_NEW_WORLD was already sent from handle_suspend_token_response (C++ sends it in
        // HandleSuspendTokenResponse, BEFORE the client's worldport ack — MovementHandler.cpp:253);
        // it must NOT be resent here or the client never finishes loading. #NEXT.R8.ENTITIES.1229.

        // SMSG_RESUME_TOKEN — C++ HandleMoveWorldportAck sets SequenceIndex =
        // player->m_movementCounter (read here, before SendInitialPacketsBeforeAddToMap resets
        // it) and Reason = 1 for a non-seamless far teleport (MovementHandler.cpp:108-111).
        let Some(resume_seq) = self.movement_counter_like_cpp() else {
            self.kick("worldport lost its Player movement state");
            return;
        };
        self.send_packet(&ResumeToken {
            sequence_index: resume_seq,
            reason: 1,
        });
        info!(
            account = self.account_id,
            map = new_map,
            resume_seq,
            "[FAR_TELEPORT] worldport ack: sent ResumeToken(reason=1); NewWorld was sent at SuspendTokenResponse #NEXT.R8.ENTITIES.1229"
        );

        let Some(guid) = self.player_guid() else {
            self.kick("worldport lost its Player identity");
            return;
        };
        let updateobject_trace_enabled = std::env::var_os("RUSTYCORE_UPDATEOBJECT_TRACE").is_some();

        // Before-add control packets the client needs for the new map: C++
        // SendInitialPacketsBeforeAddToMap resets m_movementCounter (Player.cpp:23459) and
        // ends with SetMovedUnit -> SMSG_MOVE_SET_ACTIVE_MOVER, plus a fresh time sync. The
        // full before-add packet set (spells/factions/action bars/etc.) IS replayed by
        // C++ on non-seamless transfer. Rust still omits it: this is an open parity gap,
        // not proven client retention. Its DB-backed login helper needs separation.
        self.reset_movement_counter_like_cpp();
        self.send_packet(&wow_packet::packets::misc::MoveSetActiveMover { mover_guid: guid });
        self.send_time_sync();

        // C++ Map::AddPlayerToMap(initPlayer=true) -> SendInitSelf (Map.cpp:446): re-send the
        // player's OWN object (ActivePlayer create block) for the destination map. Without it
        // the client loads to 100% but never enters the world. #NEXT.R8.ENTITIES.1229.
        let Some(self_create_accepted) = self
            .send_player_self_create_for_teleport_like_cpp(trait_node_entries)
            .await
        else {
            self.kick("worldport self CREATE has incomplete Player state");
            return;
        };
        if !self.begin_worldport_post_add_like_cpp(new_map, new_pos) {
            self.kick("worldport post-add operation could not retain its Player");
            return;
        }

        // AddPlayerToMap-equivalent: refresh nearby world objects at the new position.
        self.send_nearby_creatures_with_catalogs_like_cpp(
            creature_spawn_catalogs,
            new_map as u16,
            &new_pos,
            0,
        )
        .await;
        self.send_nearby_gameobjects(new_map as u16, &new_pos, 0)
            .await;
        info!(
            account = self.account_id,
            map = new_map,
            visible = self.client_visible_guids_like_cpp.len(),
            "[FAR_TELEPORT] replayed before-add (MoveSetActiveMover + TimeSync) + refreshed \
             nearby objects; now sending after-add init"
        );

        // SendInitialPacketsAfterAddToMap: post-add phase shift, InitWorldStates resolved for
        // the destination map, the PhasingHandler::OnMapChange phase shift, CUF profiles, auras.
        self.send_initial_packets_after_add_to_map_with_catalogs_like_cpp(
            creature_spawn_catalogs,
            guid,
            &new_pos,
            new_map as i32,
            updateobject_trace_enabled,
        )
        .await;

        let Some((zone_id, area_id)) = self.player_zone_area_like_cpp() else {
            return;
        };
        // MovementHandler.cpp:156-234 completes after-add initialization and
        // zone updates before pet recovery and ProcessDelayedOperations.
        // In particular, self CREATE must precede the delayed resurrection.
        if !self.finish_worldport_native_before_disconnect_like_cpp() {
            self.kick("worldport native completion unavailable");
            return;
        }

        // Player.cpp:1494-1503: delayed resurrection precedes DELAYED_SAVE_PLAYER.
        // Do not replay the ACK or promote an unknown COMMIT back to LoggedIn.
        if let Some(outcome) = self
            .resume_deferred_player_save_with_generator_like_cpp(item_guid_generator)
            .await
        {
            // A known rollback preserves the existing SaveToDB failure policy:
            // keep dirty intent for the next scheduled save, without a hot retry.
            if !matches!(
                outcome,
                crate::session::PlayerSaveOutcomeLikeCpp::Applied
                    | crate::session::PlayerSaveOutcomeLikeCpp::Failed
            ) {
                self.kick("worldport deferred save did not complete; retain native intent");
                return;
            }
            if self.state() == crate::session::SessionState::Disconnecting {
                return;
            }
        }

        // Preserve server effects above even when output closes. Client readiness,
        // however, must not be reported after a failed terminal publication.
        let final_stat_accepted = self.send_stat_update();
        if !self_create_accepted || !final_stat_accepted {
            self.kick("worldport self CREATE delivery or final stat publication unavailable");
            return;
        }
        info!(
            account = self.account_id,
            map = new_map,
            zone = zone_id,
            area = area_id,
            resume_seq,
            "[FAR_TELEPORT] final stat packet accepted by output channel"
        );

        // Back to LoggedIn — handler dispatch resumes.
        self.set_state(crate::session::SessionState::LoggedIn);
    }

    #[cfg(test)]
    pub async fn handle_world_port_response(&mut self, pkt: wow_packet::WorldPacket) {
        let catalogs = self.creature_spawn_catalogs_for_test_like_cpp();
        let generators = self.id_generators_for_test_like_cpp();
        self.handle_world_port_response_with_catalogs_like_cpp(
            &catalogs,
            &wow_data::trait_tree::TraitNodeEntryStore::from_entries([]),
            generators.item.as_ref(),
            pkt,
        )
        .await;
    }

    /// CMSG_AREA_TRIGGER — player entered an area trigger.
    /// C++ ref: `WorldSession::HandleAreaTriggerOpcode`.

    pub async fn handle_area_trigger_with_catalogs_like_cpp(
        &mut self,
        catalogs: &AreaTriggerCatalogsLikeCpp,
        item_guid_generator: &wow_core::ObjectGuidGenerator,
        mut pkt: wow_packet::WorldPacket,
    ) {
        let Ok(trigger_id) = pkt.read_uint32() else {
            warn!(
                account = self.account_id,
                "AreaTrigger packet missing trigger ID"
            );
            return;
        };
        let Ok(entered) = pkt.read_bit() else {
            warn!(
                account = self.account_id,
                trigger_id, "AreaTrigger packet missing Entered bit"
            );
            return;
        };
        let Ok(_from_client) = pkt.read_bit() else {
            warn!(
                account = self.account_id,
                trigger_id, "AreaTrigger packet missing FromClient bit"
            );
            return;
        };

        info!(
            "AreaTrigger: account {} trigger_id={} entered={}",
            self.account_id, trigger_id, entered
        );

        if self.resolved_is_in_taxi_flight_like_cpp() != Some(false) {
            debug!(
                "Area trigger {} ignored because player is in taxi flight",
                trigger_id
            );
            return;
        }

        let Some(at_entry) = catalogs.db2.get(trigger_id).cloned() else {
            debug!("Unknown area trigger ID {}", trigger_id);
            return;
        };

        let player_in_area_trigger = self.player_is_in_area_trigger_radius_like_cpp(&at_entry);
        // Legacy1 validates radius only for an enter notification and is the
        // selected parity behavior. Legacy2 instead requires `entered` to
        // equal the current inside/outside result, so it rejects a leave that
        // arrives while the player is still inside. Keep the disagreement
        // explicit; a 3.4.3 client capture is still needed to adjudicate it.
        if entered && !player_in_area_trigger {
            debug!(
                "Area trigger {} ignored because player is too far",
                trigger_id
            );
            return;
        }

        if !self.area_trigger_client_conditions_meet_like_cpp(trigger_id) {
            debug!("Area trigger {} rejected by C++ conditions", trigger_id);
            return;
        }

        // C++ continues unless `ScriptMgr::OnAreaTrigger` returns true. A DB
        // binding alone therefore cannot consume the event.
        let bound_script_id = catalogs
            .scripts
            .get_script_id_like_cpp(trigger_id)
            .filter(|script_id| *script_id != wow_data::ScriptIdLikeCpp::NONE);
        if let Some(script_id) = bound_script_id {
            match self.dispatch_area_trigger_script_like_cpp(
                catalogs.script_dispatcher.as_ref(),
                script_id,
                trigger_id,
                entered,
            ) {
                Some(true) => return,
                Some(false) => {}
                None => warn!(
                    trigger_id,
                    entered,
                    ?script_id,
                    "Area trigger script dispatch is unrepresented; preserving prior continuation"
                ),
            }
        }

        // C++ credits area-trigger quests here, before the tavern branch and
        // only for a living player entering the trigger
        // (`Handlers/MiscHandler.cpp:530-574`). The tavern handling below
        // returns, so a trigger that is both would otherwise lose its quest.
        if entered {
            self.credit_represented_area_trigger_quests_like_cpp(
                item_guid_generator,
                catalogs.quest_relations.as_ref(),
                trigger_id,
            )
            .await;
        }

        if self.handle_represented_tavern_area_trigger_with_catalog_like_cpp(
            catalogs.taverns.as_ref(),
            trigger_id,
            entered,
        ) {
            return;
        }

        let Some(trigger) = catalogs.destinations.get_trigger(trigger_id).cloned() else {
            return;
        };

        // Lookup in represented teleport store
        info!(
            "AreaTrigger {} detected at map {} pos ({}, {}, {})",
            trigger_id, trigger.map_id, trigger.pos.x, trigger.pos.y, trigger.pos.z
        );

        if !entered {
            return;
        }

        if let Some(ref teleport) = trigger.teleport {
            let target_map = teleport.target_map;
            let target_pos = teleport.target_position;
            info!(
                "AreaTrigger {} → teleport to map {} ({:.2}, {:.2}, {:.2})",
                trigger_id, target_map, target_pos.x, target_pos.y, target_pos.z
            );
            self.teleport_to(target_map, target_pos).await;
        }
    }

    #[cfg(test)]
    pub async fn handle_area_trigger(&mut self, pkt: wow_packet::WorldPacket) {
        let catalogs = self.area_trigger_catalogs_for_test_like_cpp();
        let generators = self.id_generators_for_test_like_cpp();
        self.handle_area_trigger_with_catalogs_like_cpp(&catalogs, generators.item.as_ref(), pkt)
            .await;
    }

    /// C++ `WorldSession::HandleAreaTriggerOpcode`'s quest block
    /// (`Handlers/MiscHandler.cpp:530-574`).
    ///
    /// Deliberately not `Player::UpdateQuestObjectiveProgress`: C++ says why in
    /// its own comment at `:532` — a `quest_objectives.ObjectID` of `-1` means
    /// "any trigger bound by `areatrigger_involvedrelation`", which an ObjectID
    /// lookup cannot express. So the quests come from the relation store and the
    /// objective's own id is only a filter.
    ///
    /// Not ported here: `Player::isDebugAreaTriggers` chat output, and
    /// `IsQuestObjectiveComplete`'s live-state branches, which
    /// `represented_quest_objective_complete_like_cpp` still fails closed on.
    async fn credit_represented_area_trigger_quests_like_cpp(
        &mut self,
        item_guid_generator: &wow_core::ObjectGuidGenerator,
        quest_relations: &wow_data::QuestAreaTriggerStoreLikeCpp,
        trigger_id: u32,
    ) {
        use wow_packet::packets::quest::{QuestUpdateAddCreditSimple, QuestUpdateComplete};

        // C++ `player->IsAlive()` gates the whole block. An unresolved vital
        // state is not an alive player, so it fails closed like the rest of the
        // handler's gates.
        if self.resolved_player_is_alive_like_cpp() != Some(true) {
            return;
        }
        let Some(quest_ids) = quest_relations
            .quests_for_area_trigger_like_cpp(trigger_id)
            .map(|quests| quests.iter().copied().collect::<Vec<_>>())
        else {
            return;
        };
        let Some(store) = self.quests.store.clone() else {
            return;
        };

        self.invalidate_player_quest_status_authority_like_cpp();
        let mut any_objective_changed_completion_state = false;
        let mut quests_to_save = Vec::new();

        for quest_id in quest_ids {
            let Some(quest) = store.get(quest_id).cloned() else {
                continue;
            };
            let Some(status) = self
                .player_quest_gameplay_snapshot_like_cpp()
                .and_then(|state| state.statuses_like_cpp().get(&quest_id).cloned())
            else {
                continue;
            };
            // C++ needs a real quest-log slot and QUEST_STATUS_INCOMPLETE.
            if status.slot >= crate::handlers::quest::MAX_QUEST_LOG_SIZE_LIKE_CPP
                || status.status != crate::conditions::QUEST_STATUS_INCOMPLETE_LIKE_CPP
            {
                continue;
            }

            // C++ stops at the first objective it can credit.
            let credited = quest.objectives.iter().enumerate().find_map(
                |(objective_index, objective)| {
                    if objective.obj_type
                        != crate::handlers::quest::QUEST_OBJECTIVE_AREATRIGGER_LIKE_CPP_LOCAL
                        || !crate::handlers::quest_rules::represented_quest_objective_completable_like_cpp(
                            &status,
                            &quest,
                            objective_index,
                        )
                        || crate::handlers::quest_rules::represented_quest_objective_complete_like_cpp(
                            &status, &quest, objective,
                        )
                        || (objective.object_id != -1
                            && objective.object_id != i32::try_from(trigger_id).unwrap_or(i32::MAX))
                    {
                        return None;
                    }
                    let storage_index = usize::try_from(objective.storage_index).ok()?;
                    Some((storage_index, objective.id, objective.object_id, objective.obj_type))
                },
            );

            if let Some((storage_index, objective_id, object_id, objective_type)) = credited {
                // C++ `SetQuestObjectiveData(obj, 1)`; an areatrigger objective is
                // flag-storing, so any non-zero value completes it.
                let applied = self
                    .mutate_player_quest_gameplay_like_cpp(|state| {
                        let Some(status) = state.status_mut_like_cpp(quest_id) else {
                            return false;
                        };
                        if status.objective_counts.len() <= storage_index {
                            status.objective_counts.resize(storage_index + 1, 0);
                        }
                        status.objective_counts[storage_index] = 1;
                        true
                    })
                    .unwrap_or(false);
                if applied {
                    self.send_packet(&QuestUpdateAddCreditSimple {
                        quest_id,
                        object_id,
                        objective_type,
                    });
                    any_objective_changed_completion_state = true;
                    quests_to_save.push(quest_id);
                    self.complete_represented_area_trigger_quest_like_cpp(
                        item_guid_generator,
                        &quest,
                        Some(objective_id),
                    )
                    .await;
                }
            }

            // C++ `AreaExploredOrEventHappens(questId)` for a quest whose
            // completion is the trigger itself (`Player.cpp:16495-16513`).
            if (quest.flags & crate::handlers::quest::QUEST_FLAGS_COMPLETION_AREA_TRIGGER_LIKE_CPP)
                != 0
                && let Some((quest_is_in_log, should_send_event_complete)) = self
                    .mark_represented_quest_explored_like_cpp(
                        quest_id,
                        crate::conditions::QUEST_STATUS_FAILED_LIKE_CPP,
                    )
                && quest_is_in_log
            {
                if should_send_event_complete {
                    self.send_packet(&QuestUpdateComplete { quest_id });
                    quests_to_save.push(quest_id);
                }
                self.complete_represented_area_trigger_quest_like_cpp(
                    item_guid_generator,
                    &quest,
                    None,
                )
                .await;
            }
        }

        self.save_changed_represented_quest_statuses_like_cpp(&mut quests_to_save)
            .await;
        if any_objective_changed_completion_state {
            let _ = self.update_visible_gameobjects_or_spell_clicks_like_cpp();
        }
        self.sync_player_registry_state_like_cpp();
    }

    /// C++ `if (player->CanCompleteQuest(questId)) player->CompleteQuest(questId);`
    /// after each area-trigger credit (`Handlers/MiscHandler.cpp:566-567`).
    async fn complete_represented_area_trigger_quest_like_cpp(
        &mut self,
        item_guid_generator: &wow_core::ObjectGuidGenerator,
        quest: &wow_data::quest::QuestTemplate,
        objective_id: Option<u32>,
    ) {
        use wow_packet::packets::quest::QuestUpdateComplete;

        let quest_id = quest.id;
        let completed = match objective_id {
            Some(objective_id) => {
                self.complete_represented_quest_after_objective_with_generator_like_cpp(
                    item_guid_generator,
                    quest,
                    objective_id,
                )
                .await
            }
            None => {
                self.complete_represented_quest_after_add_with_generator_like_cpp(
                    item_guid_generator,
                    quest,
                )
                .await
            }
        };
        if completed
            && self.represented_player_quest_status_like_cpp(quest_id)
                == Some(Some(crate::conditions::QUEST_STATUS_COMPLETE_LIKE_CPP))
        {
            self.send_packet(&QuestUpdateComplete { quest_id });
        }
    }

    fn area_trigger_client_conditions_meet_like_cpp(&mut self, trigger_id: u32) -> bool {
        let Some(condition_store) = self.condition_store().cloned() else {
            return true;
        };
        let Some(player_object) = self.build_condition_player_object_like_cpp() else {
            return false;
        };

        let Some(player_unit_snapshot) = self.condition_player_unit_snapshot_like_cpp() else {
            return false;
        };
        let player_snapshot = self.condition_player_snapshot_like_cpp();
        let area_table_store = self.area_table_store().cloned();

        let mut source_info =
            crate::conditions::ConditionSourceInfo::from_targets(Some(&player_object), None, None);
        source_info.set_unit_target_snapshot(0, player_unit_snapshot);
        source_info.set_player_target_snapshot(0, player_snapshot);

        crate::conditions::is_object_meeting_not_grouped_conditions_like_cpp(
            condition_store.as_ref(),
            ConditionSourceType::AreaTriggerClientTriggered,
            trigger_id,
            &mut source_info,
            |condition, source_info| {
                // C++ combines the base condition with
                // `ScriptMgr::OnConditionCheck`. Rust does not yet have a
                // ConditionScript dispatcher, so allowing a scripted row
                // through would silently bypass its only custom predicate.
                if condition.script_id != 0 {
                    warn!(
                        trigger_id,
                        script_id = condition.script_id,
                        "Area trigger ConditionScript dispatch is unrepresented; failing closed"
                    );
                    return false;
                }

                let context_is_represented = match condition.condition_type {
                    ConditionType::None
                    | ConditionType::MapId
                    | ConditionType::ZoneId
                    | ConditionType::Class
                    | ConditionType::Team
                    | ConditionType::Race
                    | ConditionType::Gender
                    | ConditionType::Level
                    | ConditionType::Alive
                    | ConditionType::HpVal
                    | ConditionType::HpPct
                    | ConditionType::Taxi
                    | ConditionType::ObjectEntryGuid
                    | ConditionType::ObjectEntryGuidLegacy
                    | ConditionType::TypeMask
                    | ConditionType::TypeMaskLegacy => true,
                    ConditionType::AreaId => area_table_store.is_some(),
                    _ => false,
                };
                if !context_is_represented {
                    warn!(
                        trigger_id,
                        condition_type = ?condition.condition_type,
                        "Area trigger condition context is unrepresented; failing closed"
                    );
                    return false;
                }

                match crate::conditions::condition_meets_basic_like_cpp(
                    condition,
                    source_info,
                    |current_area, required_area| {
                        area_table_store.as_ref().is_some_and(|store| {
                            store.is_in_area_like_cpp(current_area, required_area)
                        })
                    },
                ) {
                    crate::conditions::ConditionMeetResult::Evaluated(value) => value,
                    crate::conditions::ConditionMeetResult::Unsupported => {
                        warn!(
                            trigger_id,
                            condition_type = ?condition.condition_type,
                            "Area trigger condition evaluation is unrepresented; failing closed"
                        );
                        false
                    }
                }
            },
        )
    }

    /// CMSG_ACTIVATE_TAXI.
    ///
    /// C++ resolves `GetNPCIfCanInteractWith(Vendor, UNIT_NPC_FLAG_FLIGHTMASTER)`,
    /// sends `ERR_TAXITOOFARAWAY` when that fails, then checks nearest taxi
    /// node, known taximask nodes, preferred mount display, `TaxiPathGraph`,
    /// and `Player::ActivateTaxiPathTo`.
    ///
    /// Rust currently has represented NPC interaction and mount display filters,
    /// but not `TaxiNodes.db2`, `TaxiPathGraph`, or live MotionMaster taxi
    /// flight. This handler preserves packet/dispatch and the first C++ failure
    /// reply, then records the accepted request for the future taxi runtime.
    pub async fn handle_activate_taxi(&mut self, mut pkt: wow_packet::WorldPacket) {
        let activate = match ActivateTaxi::read(&mut pkt) {
            Ok(activate) => activate,
            Err(error) => {
                warn!("Bad ActivateTaxi: {error}");
                return;
            }
        };

        const NPC_FLAG_FLIGHT_MASTER: u32 = 0x2000;
        let can_interact = self
            .represented_npc_can_interact_with_like_cpp(activate.vendor, NPC_FLAG_FLIGHT_MASTER, 0)
            .is_some()
            || self
                .mutate_world_creature(activate.vendor, |creature| {
                    creature.npc_flags() & NPC_FLAG_FLIGHT_MASTER != 0
                })
                .unwrap_or(false);

        if !can_interact {
            self.send_packet(&ActivateTaxiReply {
                reply: ERR_TAXITOOFARAWAY_LIKE_CPP,
            });
            return;
        }

        let preferred_mount_display = self
            .represented_taxi_usable_mount_displays_like_cpp(activate.flying_mount_id)
            .into_iter()
            .find_map(|display| u32::try_from(display).ok())
            .unwrap_or_default();

        self.record_represented_activate_taxi_like_cpp(RepresentedActivateTaxiLikeCpp {
            vendor: activate.vendor,
            node: activate.node,
            ground_mount_id: activate.ground_mount_id,
            flying_mount_id: activate.flying_mount_id,
            preferred_mount_display,
        });
    }

    /// CMSG_TAXI_NODE_STATUS_QUERY — client asks status of a taxi NPC.
    ///
    /// C# ref: `TaxiHandler.SendTaxiStatus`:
    ///   0 = None (no node found), 1 = Learned, 2 = Unlearned, 3 = NotEligible.
    ///
    /// Without a full taxi mask we default to:
    ///   - NPCFlags includes FlightMaster (0x2000) → `Unlearned` (2)
    ///     so the taxi icon shows as available.
    ///   - Otherwise → `None` (0).

    pub async fn handle_taxi_node_status_query(&mut self, mut pkt: wow_packet::WorldPacket) {
        let unit_guid = match pkt.read_packed_guid() {
            Ok(g) => g,
            Err(_) => {
                warn!("TaxiNodeStatusQuery: failed to read unit GUID");
                return;
            }
        };

        const NPC_FLAG_FLIGHT_MASTER: u32 = 0x2000;
        let is_flight_master = self
            .mutate_world_creature(unit_guid, |creature| {
                creature.npc_flags() & NPC_FLAG_FLIGHT_MASTER != 0
            })
            .unwrap_or(false);

        // TaxiNodeStatus: 0=None, 1=Learned, 2=Unlearned, 3=NotEligible
        let status: u8 = if is_flight_master { 2 } else { 0 };

        debug!(
            account = self.account_id,
            ?unit_guid,
            status,
            "TaxiNodeStatusQuery"
        );
        self.send_packet(&TaxiNodeStatusPkt { unit_guid, status });
    }

    pub async fn handle_set_taxi_benchmark_mode(&mut self, mut pkt: wow_packet::WorldPacket) {
        let packet = match SetTaxiBenchmarkMode::read(&mut pkt) {
            Ok(packet) => packet,
            Err(error) => {
                warn!(
                    account = self.account_id,
                    "SetTaxiBenchmarkMode parse failed: {error}"
                );
                return;
            }
        };

        self.represented_set_taxi_benchmark_mode_like_cpp(packet.enable);
    }

    pub async fn handle_update_area_trigger_visual(&mut self, _pkt: wow_packet::WorldPacket) {
        // C++ registers CMSG_UPDATE_AREA_TRIGGER_VISUAL as STATUS_UNHANDLED/Handle_NULL.
    }
}
