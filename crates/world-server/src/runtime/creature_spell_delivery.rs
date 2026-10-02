//! The creature spell hit's delivery: its game table, its victim routing and the
//! tick driver that produces both.
//!
//! Separated from `delivery.rs` when the spell hit gained a damage effect: that
//! file holds the melee and visibility rails and was already at its line budget,
//! and this is one responsibility with one reader.

use super::*;

/// C++ `sNpcManaCostScalerGameTable`, loaded with the other GameTables.
///
/// `SpellEffectInfo::CalcValue` reads it to scale a creature spell's value from
/// the spell's own level to the caster's (`SpellInfo.cpp:586-592`). A missing file
/// leaves the arm unscaled, which is what C++'s `if (spellScaler && casterScaler)`
/// does with a missing row, so this returns `None` rather than refusing to start.
pub(crate) fn load_npc_mana_cost_scaler_game_table_like_cpp(
    data_dir: impl AsRef<std::path::Path>,
) -> Option<std::sync::Arc<wow_data::NpcManaCostScalerGameTableLikeCpp>> {
    wow_data::NpcManaCostScalerGameTableLikeCpp::load(data_dir)
        .ok()
        .map(std::sync::Arc::new)
}

/// Deliver map-owned creature spell hits to their exact victim sessions.
///
/// The same routing the melee hit uses, and for the same reason: the canonical
/// player's health, shield amounts and mana are already committed, so the hit is
/// published to the durable FIFO rail rather than dropped. The per-session gate
/// in the handler re-checks the map, instance and health revision.
pub(crate) fn deliver_creature_spell_damage_commands_like_cpp(
    commands: &[wow_world::session::mailbox::ApplyCreatureSpellDamageLikeCppCommand],
    registry: &wow_world::session::directory::PlayerRegistry,
) -> RuntimeCreatureMeleeDeliverySummaryLikeCpp {
    let mut summary = RuntimeCreatureMeleeDeliverySummaryLikeCpp::default();
    for command in commands {
        summary.commands_seen += 1;
        let Some(recipient) = registry.runtime_recipient(command.victim_guid) else {
            summary.candidates_skipped_missing_victim += 1;
            continue;
        };
        summary.candidates_seen += 1;
        if !recipient.is_in_world {
            summary.candidates_skipped_not_in_world += 1;
            continue;
        }
        if recipient.map_id != command.map_id {
            summary.candidates_skipped_wrong_map += 1;
            continue;
        }
        if recipient.instance_id != command.instance_id {
            summary.candidates_skipped_wrong_instance += 1;
            continue;
        }
        if registry.publish_current_creature_spell_damage(recipient.registration, command.clone()) {
            summary.candidates_queued += 1;
        } else {
            summary.send_failed += 1;
        }
    }
    summary
}

pub(crate) fn run_legacy_creature_spell_tick_and_deliver_once_like_cpp(
    legacy_map_manager: &SharedMapManager,
    canonical_map_manager: Option<&wow_world::session::SharedCanonicalMapManager>,
    registry: &wow_world::session::directory::PlayerRegistry,
    config: &wow_world::session::LegacyCreatureAggroConfigLikeCpp,
) -> (
    wow_world::session::LegacyCreatureSpellTickOutcomeLikeCpp,
    RuntimeDeliverySummaryLikeCpp,
    RuntimeCreatureMeleeDeliverySummaryLikeCpp,
) {
    let outcome = wow_world::session::run_legacy_creature_spell_tick_once_like_cpp(
        legacy_map_manager,
        canonical_map_manager,
        config,
    );
    // START and GO are committed map events and enter every eligible
    // observer's durable FIFO in plan order.
    let plan_delivery = deliver_runtime_plan_like_cpp(&outcome.plan, registry);
    // The damage effect's own publication goes to the victim session, after the
    // GO it belongs to, exactly as C++ sends the combat log after SendSpellGo.
    let damage_delivery =
        deliver_creature_spell_damage_commands_like_cpp(&outcome.spell_damage_commands, registry);
    (outcome, plan_delivery, damage_delivery)
}
