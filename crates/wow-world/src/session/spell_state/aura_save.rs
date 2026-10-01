//! The Player half of C++ `Player::_SaveAuras` (`Player.cpp:20089-20146`):
//! turning the live aura state into the save candidates the pure
//! `Aura::CanBeSaved` filter and the persistence group consume.
//!
//! The filter itself lives in `wow_entities::aura_save`; this module only
//! resolves the facts C++ reads off `Aura`, `AuraEffect` and `SpellInfo` at the
//! same point.

use super::*;
use wow_entities::{
    PlayerAuraEffectSaveLikeCpp, PlayerAuraSaveCandidateLikeCpp, PlayerAuraSaveOperationLikeCpp,
    player_save_auras_plan_like_cpp,
};

/// The live auras a Player owns, in visible-slot order, with the `SpellInfo`
/// facts `Aura::CanBeSaved` consults.
///
/// C++ iterates `m_ownedAuras`. The complete live record here is the per-slot
/// runtime application map, which every path that puts an aura on a Player writes
/// to; `owned_auras` is maintained by only some of them, so it is consulted for
/// the cast-item GUID alone.
///
/// This takes the aura subsystem rather than reaching for it, because the save
/// projection runs with the canonical map lock already held: asking the session
/// for the Player again from there deadlocks.
pub(crate) fn player_aura_save_candidates_like_cpp(
    auras: &wow_entities::AuraSubsystem,
    spell_store: Option<&wow_data::SpellStore>,
    difficulty_store: Option<&wow_data::DifficultyStore>,
) -> Vec<PlayerAuraSaveCandidateLikeCpp> {
    {
        let mut applications: Vec<_> = auras.runtime_applications_like_cpp().values().collect();
        applications.sort_by_key(|application| application.slot);

        let mut candidates = Vec::with_capacity(applications.len());
        for application in applications {
            let spell_id = u32::try_from(application.spell_id).unwrap_or(0);
            if spell_id == 0 {
                continue;
            }
            let difficulty = application.difficulty_id;
            // C++ `Aura::IsPermanent` is `GetMaxDuration() == -1`. This port
            // stores an unsigned total where a permanent aura is zero
            // (`aura_application.rs:880-882`), and writes the C++ `-1` back so
            // the loader's own permanent branch sees it again.
            let is_permanent = application.duration_total == 0;
            let effect_base_points = spell_store.and_then(|store| {
                store.effects_for_difficulty_like_cpp(
                    application.spell_id,
                    difficulty,
                    difficulty_store,
                )
            });
            let effects = application
                .represented_effect_amounts
                .iter()
                .map(|effect| PlayerAuraEffectSaveLikeCpp {
                    effect_index: effect.effect_index,
                    amount: effect.amount,
                    // C++ `AuraEffect::AuraEffect` takes the stored base amount
                    // when one was loaded and the effect's `BasePoints`
                    // otherwise (`SpellAuraEffects.cpp:620`). Loaded base
                    // amounts are not retained by this port's `_LoadAuras`, so
                    // the data value is the one C++ would compute for a freshly
                    // cast aura.
                    base_amount: effect_base_points
                        .and_then(|effects| {
                            effects
                                .iter()
                                .find(|info| info.effect_index == u32::from(effect.effect_index))
                        })
                        .map_or(effect.amount, |info| info.effect_base_points),
                    // C++ `AuraEffect::m_canBeRecalculated` starts true
                    // (`SpellAuraEffects.cpp:622`) and is only cleared by a
                    // script amount handler, which this port does not run.
                    can_be_recalculated: true,
                })
                .collect();

            let aura_ref = wow_entities::AuraRef::new(spell_id, application.caster_guid);
            let item_caster_guid = auras
                .owned_auras
                .iter()
                .find(|owned| owned.aura_ref() == aura_ref)
                .and_then(|owned| owned.item_caster_guid)
                .unwrap_or(ObjectGuid::EMPTY);

            candidates.push(PlayerAuraSaveCandidateLikeCpp {
                spell_id,
                caster_guid: application.caster_guid,
                item_caster_guid,
                difficulty,
                stack_count: application.stack_count,
                max_duration_ms: if is_permanent {
                    -1
                } else {
                    i32::try_from(application.duration_total).unwrap_or(i32::MAX)
                },
                duration_ms: if is_permanent {
                    -1
                } else {
                    i32::try_from(application.duration_remaining).unwrap_or(i32::MAX)
                },
                // C++ writes `Aura::GetCharges`. Charge consumption is not
                // tracked for Player auras here, so the stored zero lets
                // `_LoadAuras` restore the spell's full `ProcCharges`, which is
                // the same branch it takes for a stored zero.
                charges: 0,
                // C++ writes `Aura::GetCastItemId`/`GetCastItemLevel`. The
                // represented aura carries neither, so an item-cast aura is
                // restored without its item identity.
                cast_item_id: 0,
                cast_item_level: 0,
                is_passive: spell_store
                    .is_some_and(|store| store.is_passive_like_cpp(application.spell_id)),
                is_channeled: spell_store
                    .is_some_and(|store| store.is_channeled_like_cpp(application.spell_id)),
                is_single_target: spell_store.is_some_and(|store| {
                    store.is_single_target_like_cpp(
                        application.spell_id,
                        difficulty,
                        difficulty_store,
                    )
                }),
                has_area_effect: spell_store.is_some_and(|store| {
                    store.has_area_effect_for_difficulty_like_cpp(
                        application.spell_id,
                        difficulty,
                        difficulty_store,
                    )
                }),
                cannot_be_saved_attribute: spell_store.is_some_and(|store| {
                    store.aura_cannot_be_saved_like_cpp(
                        application.spell_id,
                        difficulty,
                        difficulty_store,
                    )
                }),
                // C++ keeps `m_isUsingCharges` equal to `m_procCharges != 0` at
                // every write site (`SpellAuras.cpp:466`, `:999`, `:1288`), so
                // the charge gate in `CanBeSaved` follows the charge count.
                uses_charges: false,
                is_permanent,
                effects,
            });
        }

        candidates
    }
}

/// The `character_aura` rows for one Player's save.
pub(crate) fn player_aura_save_rows_like_cpp(
    owner_guid: ObjectGuid,
    auras: &wow_entities::AuraSubsystem,
    spell_store: Option<&wow_data::SpellStore>,
    difficulty_store: Option<&wow_data::DifficultyStore>,
) -> Vec<wow_persistence::PlayerAuraSaveLikeCpp> {
    {
        let candidates = player_aura_save_candidates_like_cpp(auras, spell_store, difficulty_store);
        let mut rows: Vec<wow_persistence::PlayerAuraSaveLikeCpp> = Vec::new();
        for operation in player_save_auras_plan_like_cpp(owner_guid, &candidates) {
            match operation {
                PlayerAuraSaveOperationLikeCpp::DeleteAuraEffects
                | PlayerAuraSaveOperationLikeCpp::DeleteAuras => {}
                PlayerAuraSaveOperationLikeCpp::InsertAura {
                    caster_guid,
                    item_guid,
                    spell_id,
                    effect_mask,
                    recalculate_mask,
                    difficulty,
                    stack_count,
                    max_duration_ms,
                    duration_ms,
                    charges,
                    cast_item_id,
                    cast_item_level,
                } => rows.push(wow_persistence::PlayerAuraSaveLikeCpp {
                    caster_guid_binary: caster_guid.to_raw_bytes().to_vec(),
                    item_guid_binary: item_guid.to_raw_bytes().to_vec(),
                    spell_id,
                    effect_mask,
                    recalculate_mask,
                    difficulty,
                    stack_count,
                    max_duration_ms,
                    remain_time_ms: duration_ms,
                    remain_charges: charges,
                    cast_item_id,
                    cast_item_level,
                    effects: Vec::new(),
                }),
                PlayerAuraSaveOperationLikeCpp::InsertAuraEffect {
                    effect_index,
                    amount,
                    base_amount,
                    ..
                } => {
                    if let Some(row) = rows.last_mut() {
                        row.effects
                            .push(wow_persistence::PlayerAuraEffectSaveRowLikeCpp {
                                effect_index,
                                amount,
                                base_amount,
                            });
                    }
                }
            }
        }
        rows
    }
}

impl WorldSession {
    /// The `character_aura` rows for this session's Player, or `None` when the
    /// canonical aura state could not be read: C++ only reaches `_SaveAuras` with
    /// a live Player, and an unread aura map must not clear stored rows.
    ///
    /// Only for callers that do **not** hold the canonical map lock. The save
    /// projection already holds it and uses the free function above with the
    /// Player it was given.
    pub(crate) fn player_aura_save_rows_like_cpp(
        &self,
    ) -> Option<Vec<wow_persistence::PlayerAuraSaveLikeCpp>> {
        let owner_guid = self.player_guid()?;
        let auras = self.player_aura_subsystem_snapshot_like_cpp()?;
        let spell_store = self.spell_store().cloned();
        let difficulty_store = self.difficulty_store().cloned();
        Some(player_aura_save_rows_like_cpp(
            owner_guid,
            &auras,
            spell_store.as_deref(),
            difficulty_store.as_deref(),
        ))
    }
}
