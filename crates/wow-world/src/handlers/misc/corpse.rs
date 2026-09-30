// Copyright (c) 2026 alseif0x
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Private corpse capability handlers extracted from the legacy misc owner.

use tracing::{info, warn};
use wow_constants::{ClientOpcodes, ConditionType};
use wow_handler::{PacketProcessing, SessionStatus};

use crate::session::registry::PacketHandlerEntry;
use wow_packet::ClientPacket;
use wow_packet::packets::misc::{
    PortGraveyard, ReclaimCorpse, RepopRequest, RequestCemeteryListResponse, ResurrectResponse,
};

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::ResurrectResponse,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_resurrect_response",
        handler: |session, _catalogs, pkt| {
            Box::pin(async move { session.handle_resurrect_response(pkt).await })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::RepopRequest,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_repop_request",
        handler: |session, catalogs, pkt| {
            Box::pin(async move {
                session
                    .handle_repop_request(catalogs.graveyards.as_ref(), pkt)
                    .await
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::ReclaimCorpse,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_reclaim_corpse",
        handler: |session, _catalogs, pkt| Box::pin(async move { session.handle_reclaim_corpse(pkt).await }),
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::RequestCemeteryList,
        status: SessionStatus::LoggedIn,
        processing: PacketProcessing::Inplace,
        handler_name: "handle_request_cemetery_list",
        handler: |session, catalogs, pkt| {
            Box::pin(async move {
                session
                    .handle_request_cemetery_list_with_catalog_like_cpp(
                        catalogs.graveyards.as_ref(),
                        pkt,
                    )
                    .await
            })
        },
    }
}

impl crate::session::WorldSession {
    /// CMSG_REQUEST_CEMETERY_LIST — client asks for graveyards in zone.
    /// C++ ref: `WorldSession::HandleRequestCemeteryList`.
    pub(crate) async fn handle_request_cemetery_list_with_catalog_like_cpp(
        &mut self,
        graveyard_store: &wow_data::GraveyardStore,
        _pkt: wow_packet::WorldPacket,
    ) {
        if std::env::var_os("RUSTYCORE_PACKET_SEQUENCE_TRACE").is_some() {
            info!(
                account = self.account_id,
                state = ?self.state(),
                "RUST_CEMETERY_TRACE handler entry"
            );
        }
        let Some((zone_id, area_id)) = self.player_zone_area_like_cpp() else {
            return;
        };
        if std::env::var_os("RUSTYCORE_PACKET_SEQUENCE_TRACE").is_some() {
            info!(
                account = self.account_id,
                state = ?self.state(),
                zone = zone_id,
                area = area_id,
                map_id = self.player_map_id_like_cpp(),
                player = ?self.player_guid(),
                "RUST_CEMETERY_TRACE handler resolved zone_area"
            );
        }
        let Some(graveyards) = graveyard_store.graveyards_for_zone(zone_id) else {
            info!(
                zone = zone_id,
                area = area_id,
                map_id = self.player_map_id_like_cpp(),
                player = ?self.player_guid(),
                "No graveyards found in CMSG_REQUEST_CEMETERY_LIST"
            );
            return;
        };

        let mut cemetery_ids = Vec::new();
        for graveyard in graveyards {
            if cemetery_ids.len() >= 16 {
                break;
            }
            if self.graveyard_conditions_meet_like_cpp(&graveyard.conditions) {
                cemetery_ids.push(graveyard.safe_loc_id);
            }
        }

        if cemetery_ids.is_empty() {
            info!(
                zone = zone_id,
                area = area_id,
                map_id = self.player_map_id_like_cpp(),
                candidate_count = graveyards.len(),
                player = ?self.player_guid(),
                "No graveyards passed conditions in CMSG_REQUEST_CEMETERY_LIST"
            );
            return;
        }

        info!(
            zone = zone_id,
            area = area_id,
            map_id = self.player_map_id_like_cpp(),
            candidate_count = graveyards.len(),
            accepted_count = cemetery_ids.len(),
            cemetery_ids = ?cemetery_ids,
            player = ?self.player_guid(),
            "Sending C++ RequestCemeteryListResponse"
        );
        self.send_packet(&RequestCemeteryListResponse {
            is_gossip_triggered: false,
            cemetery_ids,
        });
    }

    #[cfg(test)]
    pub async fn handle_request_cemetery_list(&mut self, pkt: wow_packet::WorldPacket) {
        let store = self
            .graveyard_store()
            .cloned()
            .unwrap_or_else(|| std::sync::Arc::new(wow_data::GraveyardStore::default()));
        self.handle_request_cemetery_list_with_catalog_like_cpp(store.as_ref(), pkt)
            .await;
    }

    fn graveyard_conditions_meet_like_cpp(
        &mut self,
        conditions_ref: &wow_data::ConditionsReference,
    ) -> bool {
        let Some(conditions) = conditions_ref.upgrade() else {
            return true;
        };
        if conditions.is_empty() {
            return true;
        }

        let Some(condition_store) = self.condition_store().cloned() else {
            warn!("Cemetery condition check failed closed: missing condition store");
            return false;
        };
        let Some(player_object) = self.build_condition_player_object_like_cpp() else {
            warn!("Cemetery condition check failed closed: missing player object");
            return false;
        };

        let Some(player_unit_snapshot) = self.condition_player_unit_snapshot_like_cpp() else {
            return false;
        };
        let player_snapshot = self.condition_player_snapshot_like_cpp();
        let needs_player_condition_context = conditions.iter().any(|condition| {
            condition.reference_id != 0
                || condition.condition_type == ConditionType::PlayerCondition
        });
        let player_condition_store = needs_player_condition_context
            .then(|| self.player_condition_store().cloned())
            .flatten();
        let player_condition_context = needs_player_condition_context
            .then(|| self.represented_player_condition_context_like_cpp())
            .flatten();

        let mut source_info =
            crate::conditions::ConditionSourceInfo::from_targets(Some(&player_object), None, None);
        source_info.set_unit_target_snapshot(0, player_unit_snapshot);
        source_info.set_player_target_snapshot(0, player_snapshot);
        if let (Some(store), Some(context)) = (
            player_condition_store.as_ref(),
            player_condition_context.as_ref(),
        ) {
            source_info.set_player_condition_store(store.as_ref());
            if let Some(context) = context.as_context(self) {
                source_info.set_player_condition_context(0, context);
            }
        }

        crate::conditions::is_object_meet_to_conditions_like_cpp(
            &mut source_info,
            conditions.as_slice(),
            condition_store.as_ref(),
            |condition, source_info| match crate::conditions::condition_meets_basic_like_cpp(
                condition,
                source_info,
                |current_area, required_area| current_area == required_area,
            ) {
                crate::conditions::ConditionMeetResult::Evaluated(value) => value,
                crate::conditions::ConditionMeetResult::Unsupported => {
                    warn!(
                        "Cemetery condition check failed closed: unsupported {:?}",
                        condition.condition_type
                    );
                    false
                }
            },
        )
    }

    /// CMSG_RESURRECT_RESPONSE — answer to a pending resurrection request.
    /// C++ ref: `WorldSession::HandleResurrectResponse`.

    pub async fn handle_resurrect_response(&mut self, mut pkt: wow_packet::WorldPacket) {
        let response = match ResurrectResponse::read(&mut pkt) {
            Ok(response) => response,
            Err(error) => {
                warn!(
                    account = self.account_id,
                    "ResurrectResponse parse failed: {error}"
                );
                return;
            }
        };

        if self.resolved_player_is_alive_like_cpp() != Some(false) {
            return;
        }

        if response.response != 0 {
            self.clear_represented_resurrection_request_like_cpp();
            return;
        }

        let Some(request) = self
            .take_represented_resurrection_request_if_requested_by_like_cpp(response.resurrecter)
        else {
            return;
        };

        // C++ teleports to resurrection request location before applying the
        // resurrected state. InstanceScript combat-res charges, aura original
        // caster, and SpawnCorpseBones remain represented gaps.
        self.teleport_to(request.map_id, request.position).await;
        if self.pending_teleport_like_cpp().is_some() || self.near_teleport_pending_like_cpp() {
            self.schedule_represented_resurrection_after_teleport_like_cpp(request);
        } else {
            self.apply_represented_resurrection_health_like_cpp(request.health);
        }
    }

    /// C++ `Player::RepopAtGraveyard` (`Entities/Player/Player.cpp:4642-4690`).
    ///
    /// Resolves the closest eligible graveyard for the player's zone and team
    /// and teleports there, which is what turns "released spirit" into a state
    /// the player can actually recover from. Returns `true` when a graveyard was
    /// found and the teleport was issued.
    ///
    /// **Scope contract — intentional, bounded departure.** Ported: the
    /// `sObjectMgr->GetClosestGraveyard(*this, GetTeam(), this)` lookup
    /// (`:4668`), the "if no grave found, stay at the current location"
    /// behaviour (`:4674-4676`) and the teleport (`:4677`).
    ///
    /// NOT ported, so none of it is silently invented:
    ///   * `shouldResurrect` and its `SpawnCorpseBones()` call (`:4652-4656`):
    ///     the zone `NoGhostOnRelease` flag, dungeon/raid maps, transports and
    ///     the below-min-height case. RustyCore has no corpse writer yet, so
    ///     there are no bones to spawn.
    ///   * the `Battleground`/`Battlefield` graveyard overrides (`:4659-4666`);
    ///     neither manager exists in RustyCore.
    ///   * `SMSG_DEATH_RELEASE_LOC` (`:4679-4685`): the opcode is enumerated
    ///     (`ServerOpcodes::DeathReleaseLoc = 0x26d3`) but `wow-packet` has no
    ///     body for it, so the spirit-healer location is not shown yet.
    ///   * `m_deathTimer = 0` (`:4672`) and
    ///     `RemovePlayerFlag(PLAYER_FLAGS_IS_OUT_OF_BOUNDS)` (`:4688`).
    ///   * the `m_homebind` fallback for a below-min-height player (`:4686`).
    pub(crate) async fn repop_at_graveyard_like_cpp(
        &mut self,
        graveyards: &wow_data::GraveyardStore,
    ) -> bool {
        let Some(position) = self.player_position_like_cpp() else {
            return false;
        };
        let map_id = u32::from(self.player_map_id_like_cpp());
        let Some(safe_locs) = self.world_safe_loc_store_like_cpp().cloned() else {
            return false;
        };

        // C++ `GetClosestGraveyard` keys off the player's zone, resolved from
        // the terrain area like `Player::UpdateZoneAndAreaId`.
        let zone_id = match crate::map_manager::zone_and_area_for_position_like_cpp(
            &self.mmap_runtime_config_like_cpp().data_dir,
            map_id,
            position.x,
            position.y,
            self.area_table_store().map(|store| store.as_ref()),
            |lookup_map_id| {
                self.map_store()
                    .as_deref()
                    .map(|store| u32::from(store.area_table_id_like_cpp(lookup_map_id)))
                    .unwrap_or(0)
            },
        ) {
            Ok((zone_id, _area_id)) => zone_id,
            Err(_) => 0,
        };

        let team =
            u32::from(crate::session::player_team_for_race_cpp(self.player_race_like_cpp()) as u8);
        let context = wow_data::GraveyardLookupContextLikeCpp {
            map_id,
            x: position.x,
            y: position.y,
            z: position.z,
            team,
            parent_map_id: None,
            corpse_map_id: None,
            is_battleground_or_arena: false,
        };
        let Some(safe_loc_id) =
            graveyards.closest_graveyard_in_zone_like_cpp(zone_id, context, safe_locs.as_ref())
        else {
            // C++: "if no grave found, stay at the current location".
            return false;
        };
        let Some(safe_loc) = safe_locs.get(safe_loc_id) else {
            return false;
        };
        let (target_map, target_position) = (safe_loc.map_id, safe_loc.position);
        self.teleport_to(target_map, target_position).await;
        true
    }

    /// The corpse half of C++ `Player::BuildPlayerRepop` (`Player.cpp:4190-4193`):
    /// create the player's corpse at the death location and register it on the
    /// map before the graveyard teleport.
    ///
    /// C++ guards against a second corpse on the current map (`:4182-4187`);
    /// RustyCore has no `GetCorpseLocation()` equivalent yet, so that guard is
    /// not reproduced. See the scope contract on
    /// `create_player_corpse_on_map_like_cpp` for what the corpse omits.
    pub(crate) fn create_player_corpse_for_repop_like_cpp(
        &mut self,
    ) -> Option<wow_core::ObjectGuid> {
        let player_guid = self.player_guid()?;
        let manager = self
            .canonical_map_manager
            .as_ref()
            .map(std::sync::Arc::clone)?;
        let (legacy_map_id, instance_id) = self.current_legacy_runtime_map_key_like_cpp();
        let map_id = u32::from(legacy_map_id);
        let realm_id = self.realm_id();
        let race = self.player_race_like_cpp();
        let class = self.player_class_like_cpp();
        let gender = self.player_gender_like_cpp();
        let faction_template = self
            .player_faction_template_id_like_cpp()
            .and_then(|id| i32::try_from(id).ok())
            .unwrap_or(0);
        let ghost_time = crate::session::unix_now_like_cpp();
        crate::session::create_player_corpse_on_map_like_cpp(
            &manager,
            map_id,
            instance_id,
            realm_id,
            player_guid,
            race,
            class,
            gender,
            faction_template,
            ghost_time,
        )
    }

    /// C++ `Corpse::SaveToDB` (`Entities/Corpse/Corpse.cpp`), reached from
    /// `Player::CreateCorpse` (`Player.cpp:4407-4408`) only when the map is not
    /// instanceable.
    ///
    /// Writes the row keyed by the owner's guid counter, which is what the login
    /// corpse loader reads back, so a corpse now survives a restart.
    ///
    /// Not written: the phase and customization rows of the same C++ function.
    /// `create_player_corpse_on_map_like_cpp` sets neither a phase shift nor
    /// customizations, so there is nothing to persist; the adapter still issues
    /// their deletes so no stale row survives.
    async fn persist_created_corpse_like_cpp(&mut self) -> bool {
        let Some(port) = self
            .map_corpse_persistence_port_like_cpp()
            .map(std::sync::Arc::clone)
        else {
            return false;
        };
        let Some(player_guid) = self.player_guid() else {
            return false;
        };
        let Some(position) = self.player_position_like_cpp() else {
            return false;
        };
        let (legacy_map_id, instance_id) = self.current_legacy_runtime_map_key_like_cpp();
        let owner_guid = match u64::try_from(player_guid.counter()) {
            Ok(counter) => counter,
            Err(_) => return false,
        };
        let row = wow_persistence::MapCorpseSaveRowLikeCpp {
            owner_guid,
            pos_x: position.x,
            pos_y: position.y,
            pos_z: position.z,
            orientation: position.orientation,
            map_id: legacy_map_id,
            display_id: 0,
            item_cache: String::new(),
            race: self.player_race_like_cpp(),
            class: self.player_class_like_cpp(),
            sex: self.player_gender_like_cpp(),
            flags: 0,
            dynamic_flags: 0,
            ghost_time: u32::try_from(crate::session::unix_now_like_cpp()).unwrap_or(0),
            corpse_type: wow_entities::CorpseType::ResurrectablePve as u8,
            instance_id,
        };
        matches!(
            port.persist_corpse_like_cpp(row).await,
            wow_persistence::MapCorpseSaveOutcomeLikeCpp::Saved
        )
    }

    /// CMSG_REPOP_REQUEST — release spirit.
    /// C++ ref: `WorldSession::HandleRepopRequest`.

    pub async fn handle_repop_request(
        &mut self,
        graveyards: &wow_data::GraveyardStore,
        mut pkt: wow_packet::WorldPacket,
    ) {
        let _request = match RepopRequest::read(&mut pkt) {
            Ok(request) => request,
            Err(error) => {
                warn!(
                    account = self.account_id,
                    "RepopRequest parse failed: {error}"
                );
                return;
            }
        };

        if self.resolved_player_is_alive_like_cpp() != Some(false)
            || self.player_has_ghost_flag_like_cpp()
        {
            return;
        }

        // C++ also blocks `SPELL_AURA_PREVENT_RESURRECTION`, handles JUST_DIED
        // promotion through KillPlayer, removes the pet, builds the corpse, and
        // teleports to the graveyard. Rust has only the represented death/ghost
        // seam here; full corpse/graveyard runtime remains open.
        self.set_player_alive_like_cpp(false);
        self.set_player_ghost_flag_like_cpp(true);
        // C++ `HandleRepopRequest` ends in `BuildPlayerRepop()` then
        // `RepopAtGraveyard()`. The graveyard teleport is now real; the
        // `PREVENT_RESURRECTION` aura gate, the KillPlayer JUST_DIED promotion,
        // pet removal and the `BuildPlayerRepop` ghost transition remain open
        // (see the scope contracts on `repop_at_graveyard_like_cpp` and
        // `create_player_corpse_on_map_like_cpp`).
        //
        // C++ runs `BuildPlayerRepop()` (`Player.cpp:4167`), which creates the
        // corpse and adds it to the map, BEFORE `RepopAtGraveyard()` teleports
        // the ghost away. Order matters: the corpse belongs where the player
        // died, not at the graveyard.
        if self.create_player_corpse_for_repop_like_cpp().is_some() {
            // C++ `Corpse::SaveToDB` runs inside `CreateCorpse`
            // (`Entities/Player/Player.cpp:4408`), guarded by
            // `if (!GetMap()->Instanceable())` — instance corpses are not saved.
            self.persist_created_corpse_like_cpp().await;
        }
        let ported = self.repop_at_graveyard_like_cpp(graveyards).await;
        #[cfg(test)]
        {
            self.represented_repop_at_graveyard_count =
                self.represented_repop_at_graveyard_count.saturating_add(1);
        }
        let _ = ported;
    }

    /// CMSG_CLIENT_PORT_GRAVEYARD — manually teleport ghost to graveyard.
    /// C++ ref: `WorldSession::HandlePortGraveyard`.
    pub async fn try_handle_client_port_graveyard_like_cpp(
        &mut self,
        graveyards: &wow_data::GraveyardStore,
        mut pkt: wow_packet::WorldPacket,
    ) -> bool {
        if PortGraveyard::read(&mut pkt).is_err() {
            return false;
        }

        if self.resolved_player_is_alive_like_cpp() != Some(false)
            || !self.player_has_ghost_flag_like_cpp()
        {
            return true;
        }

        // C++ `WorldSession::HandlePortGraveyard` calls
        // `Player::RepopAtGraveyard()`. This used to be a `#[cfg(test)]`-only
        // counter, so in production the packet was consumed and nothing
        // happened at all.
        let ported = self.repop_at_graveyard_like_cpp(graveyards).await;
        #[cfg(test)]
        {
            self.represented_repop_at_graveyard_count =
                self.represented_repop_at_graveyard_count.saturating_add(1);
        }
        let _ = ported;
        true
    }

    /// CMSG_RECLAIM_CORPSE — resurrect at corpse.
    /// C++ ref: `WorldSession::HandleReclaimCorpse`.

    pub async fn handle_reclaim_corpse(&mut self, mut pkt: wow_packet::WorldPacket) {
        let _request = match ReclaimCorpse::read(&mut pkt) {
            Ok(request) => request,
            Err(error) => {
                warn!(
                    account = self.account_id,
                    "ReclaimCorpse parse failed: {error}"
                );
                return;
            }
        };

        if self.resolved_player_is_alive_like_cpp() != Some(false) {
            return;
        }

        if !self.player_has_ghost_flag_like_cpp() {
            return;
        }

        // C++ checks arena, live corpse existence, reclaim delay, and distance
        // before `ResurrectPlayer(0.5f)` + `SpawnCorpseBones`. Those require the
        // full player-corpse runtime; this represented slice only clears the
        // ghost/dead state when the already-known C++ gates pass.
        self.set_player_ghost_flag_like_cpp(false);
        let restore_percent = if self.player_in_represented_battleground_like_cpp() {
            1.0
        } else {
            0.5
        };
        self.apply_represented_resurrection_percent_like_cpp(restore_percent);
    }
}
