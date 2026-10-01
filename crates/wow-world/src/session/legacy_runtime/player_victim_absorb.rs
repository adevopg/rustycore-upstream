//! C++ `Unit::CalcAbsorbResist` for a player victim, inside the map phase that
//! owns the victim's health write.
//!
//! Extracted from the creature melee tick when the creature spell hit became its
//! second caller. C++ has one `CalcAbsorbResist`; so does this.

use super::*;

/// Commit one spent shield's `AuraEffect` remainder on the canonical player.
fn write_absorbed_shield_amount_like_cpp(
    player: &mut wow_entities::Player,
    consumption: &crate::session_rules::RepresentedAbsorbConsumptionLikeCpp,
) {
    crate::session::combat::write_absorbed_shield_amount_like_cpp(
        player,
        consumption.slot,
        consumption.effect_index,
        consumption.remaining,
    );
}

/// C++ `Unit::CalcAbsorbResist`'s represented stages for a player victim
/// (`Unit.cpp:2080-2250`), committed inside the same map-owned phase as the
/// victim's health write: the school-absorb loop, then the mana-shield loop.
///
/// C++ spends each shield effect's amount and the mana-shield drain while it
/// calculates the hit, so both are pool data and this map-owned stage is
/// their writer. Both the represented creature swing and the represented
/// creature spell hit are its callers, exactly as both reach one
/// `CalcAbsorbResist` in C++; the session's aura transition at delivery owns the *removal*
/// of an exhausted shield and its publication. A shield C++ would remove is
/// left at zero here, which the session removes through the same `remove_aura`
/// path it owns for every other aura.
///
/// Returns `None` when the map or the canonical player cannot be resolved,
/// which keeps the caller's pre-absorb damage unchanged. Otherwise the tuple is
/// `(absorbed, remaining damage, mana spent, consumptions)`, with the outcomes
/// in `AbsorbAuraOrderPred` order followed by the mana shields. Each caller maps
/// the consumptions into its own delivery command, because the victim session
/// owns the absorb-log publication and the aura transition.
#[allow(clippy::type_complexity)]
pub(in crate::session) fn apply_absorb_stages_to_canonical_player_like_cpp(
    canonical_manager: &mut wow_map::MapManager,
    map_id: u32,
    instance_id: u32,
    victim_guid: ObjectGuid,
    school_mask: u32,
    damage: u32,
    spell_store: &wow_data::SpellStore,
    difficulty_id: u8,
    difficulty_store: Option<&wow_data::DifficultyStore>,
    // C++ `CalcAbsorbResist`'s `auraAbsorbMod` from the attacker's
    // `SPELL_AURA_MOD_TARGET_ABSORB_SCHOOL`.
    ignore_absorb_pct: f32,
) -> Option<(
    u32,
    u32,
    u32,
    Vec<crate::session_rules::RepresentedAbsorbConsumptionLikeCpp>,
)> {
    let managed = canonical_manager.find_map_mut(map_id, instance_id)?;
    let player = managed.map_mut().get_typed_player_mut(victim_guid)?;
    let auras = player
        .unit()
        .subsystems()
        .auras
        .runtime_applications_like_cpp()
        .clone();
    let shields = crate::session_rules::player_absorb_shields_like_cpp(
        &auras,
        spell_store,
        difficulty_id,
        difficulty_store,
        school_mask,
    );
    // C++ runs the mana-shield loop after the school-absorb loop over the damage
    // the school shields left, with the attacker's ignore-absorb share held out
    // of both (`Unit.cpp:2112-2250`).
    let mana_shields =
        crate::session_rules::player_mana_shields_like_cpp(&auras, spell_store, school_mask);
    let mana_before = player
        .unit()
        .get_power(wow_constants::PowerType::Mana)
        .max(0);
    let absorb = crate::session_rules::represented_absorb_stages_like_cpp(
        &shields,
        &mana_shields,
        // A physical melee hit never resists (`Unit.cpp:1972-1974`), so the
        // pre-resist and post-resist damage are the same value here.
        damage,
        damage,
        mana_before as u32,
        ignore_absorb_pct,
    );
    let mut consumptions =
        Vec::with_capacity(absorb.school_consumed.len() + absorb.mana_consumed.len());
    for consumption in &absorb.school_consumed {
        write_absorbed_shield_amount_like_cpp(player, consumption);
        consumptions.push(*consumption);
    }
    if absorb.mana_spent > 0 {
        // `Unit::ModifyPower(POWER_MANA, -manaReduction)`: the same locked map
        // phase that commits the health write owns the drain, and the canonical
        // setter clamps it like C++.
        player.unit_mut().set_power(
            wow_constants::PowerType::Mana,
            mana_before - i32::try_from(absorb.mana_spent).unwrap_or(i32::MAX),
        );
    }
    for consumption in &absorb.mana_consumed {
        let consumption = crate::session_rules::RepresentedAbsorbConsumptionLikeCpp {
            slot: consumption.slot,
            effect_index: consumption.effect_index,
            consumed: consumption.consumed,
            remaining: consumption.remaining,
            removed: consumption.removed,
        };
        write_absorbed_shield_amount_like_cpp(player, &consumption);
        consumptions.push(consumption);
    }
    Some((
        absorb.absorbed,
        absorb.damage,
        absorb.mana_spent,
        consumptions,
    ))
}
