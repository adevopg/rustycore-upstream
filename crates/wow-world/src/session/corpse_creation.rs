// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Creating the dead player's corpse on the canonical map.
//!
//! Split out of `session/mod.rs`: the function and its scope contract are
//! unchanged, only their file moved.

use super::*;

/// C++ `Player::CreateCorpse` (`Entities/Player/Player.cpp:4346-4411`) plus the
/// `GetMap()->AddToMap(corpse)` step of `Player::BuildPlayerRepop` (`:4192`).
///
/// Creates the dead player's corpse on the canonical map and registers it, so a
/// released spirit leaves something behind. Returns `true` when a corpse was
/// registered.
///
/// **Scope contract — intentional, bounded departure.** Ported: the corpse
/// entity with its race/class/sex/faction identity, the map-generated
/// `HighGuid::Corpse` low guid (`:4354`), the position at the player's location
/// (`:4360`) and the map registration (`:4401`).
///
/// NOT ported, so none of it is silently invented:
///   * `SaveToDB` (`:4408`). `CharStatements::INS_CORPSE`,
///     `INS_CORPSE_CUSTOMIZATIONS` and `INS_CORPSE_PHASES` already exist with
///     SQL, but no adapter writes them, so a corpse does not survive a restart.
///   * the equipment display loop (`:4377-4398`). Its C++ body carries a local
///     fork patch at `Player.cpp:4382` ("alistar if player is not showing helm /
///     cloak hide them on corpse"), which is modified-fork behaviour and not
///     base-server parity evidence, so the whole loop is left out rather than
///     ported from a patched reference.
///   * the corpse flags from PvP/FFA/battleground state (`:4362-4371`),
///     `SetCustomizations` (`:4375`), `SetDisplayId` (`:4378`),
///     `UpdatePositionData`/`SetZoneScript` (`:4403-4404`) and the
///     `CORPSE_RESURRECTABLE_PVP` type selection (`:4350`).
pub(crate) fn create_player_corpse_on_map_like_cpp(
    manager: &SharedCanonicalMapManager,
    map_id: u32,
    instance_id: u32,
    realm_id: u16,
    player_guid: ObjectGuid,
    race: u8,
    class: u8,
    gender: u8,
    faction_template: i32,
    ghost_time: i64,
) -> Option<ObjectGuid> {
    let mut manager = manager.lock().ok()?;
    let map = manager.find_map_mut(map_id, instance_id)?;
    let map = map.map_mut();
    // C++ `CreateCorpse` does `_corpseLocation.WorldRelocate(*this)`: the corpse
    // goes exactly where the Player object is, so read the canonical Player
    // rather than a session-side mirror.
    let position = map.get_typed_player(player_guid)?.unit().world().position();
    let low_guid = map
        .generate_low_guid_like_cpp(wow_core::guid::HighGuid::Corpse)
        .ok()?;
    let map_id_u16 = u16::try_from(map_id).ok()?;
    let mut corpse =
        wow_entities::Corpse::new_at(wow_entities::CorpseType::ResurrectablePve, ghost_time);
    let corpse_guid = ObjectGuid::create_world_object(
        wow_core::guid::HighGuid::Corpse,
        0,
        realm_id,
        map_id_u16,
        0,
        0,
        low_guid,
    );
    corpse.world_mut().object_mut().create(corpse_guid);
    if corpse.world_mut().set_map(map_id, instance_id).is_err() {
        return None;
    }
    corpse.world_mut().relocate(position);
    corpse.set_race(race);
    corpse.set_class(class);
    corpse.set_sex(gender);
    corpse.set_faction_template(faction_template);
    corpse.world_mut().object_mut().add_to_world();
    // `register_loaded_corpse_like_cpp` returns whether the corpse became
    // active in world, which needs its destination grid loaded. The
    // registration itself succeeded either way, and a dormant corpse is still a
    // corpse: C++ `Map::AddCorpse` retains it by cell while `ObjectWorldLoader`
    // only calls `AddToWorld` once that grid loads. So only an `Err` is a
    // failure here.
    match map.register_loaded_corpse_like_cpp(corpse) {
        Ok(_) => Some(corpse_guid),
        Err(_) => None,
    }
}
