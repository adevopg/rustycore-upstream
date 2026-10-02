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
    // Nothing logged the tick's own refusals, which made a creature that never
    // casts indistinguishable from one the gates rejected.
    // `RUSTYCORE_CREATURE_SPELL_TRACE=1` turns every gate into a readable number,
    // in the same env-var-gated, throttled shape as the melee trace beside it.
    //
    // It speaks on every tick that saw a creature, not only on one where a
    // counter moved: "saw thirty creatures and did nothing, with every gate at
    // zero" is the single most useful line this trace can print, and the first
    // version of it stayed silent for exactly that case.
    if outcome.creatures_seen > 0 && std::env::var_os("RUSTYCORE_CREATURE_SPELL_TRACE").is_some() {
        static TRACE_TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let tick = TRACE_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if tick % 100 == 0 {
            tracing::info!(
                creatures_seen = outcome.creatures_seen,
                casts_ready = outcome.casts_ready,
                schedules_initialized = outcome.schedules_initialized,
                spell_hits = outcome.spell_hits,
                spell_misses = outcome.spell_misses,
                damage_executed = outcome.spell_damage_effects_executed,
                damage_unresolved = outcome.spell_damage_effects_unresolved,
                effects_unrepresented = outcome.spell_effects_unrepresented,
                noninstant = outcome.noninstant_casts_unrepresented,
                projectiles = outcome.spell_projectiles_unrepresented,
                missing_spell_metadata = outcome.missing_spell_metadata,
                range_rejections = outcome.spell_range_rejections,
                los_rejections = outcome.spell_los_rejections,
                hit_results_unrepresented = outcome.spell_hit_results_unrepresented,
                target_rejections = outcome.canonical_cast_target_rejections,
                missing_target = outcome.canonical_cast_missing_target,
                cooldown_rejections = outcome.canonical_cast_cooldown_rejections,
                disabled = outcome.spells_disabled,
                casting_requirements = outcome.spell_casting_requirements_unrepresented,
                incarnation_rejections = outcome.caster_incarnation_rejections,
                unit_state_skips = outcome.unit_state_casting_skips,
                "creature spell tick"
            );
        }
    }
    // START and GO are committed map events and enter every eligible
    // observer's durable FIFO in plan order.
    let plan_delivery = deliver_runtime_plan_like_cpp(&outcome.plan, registry);
    // The damage effect's own publication goes to the victim session, after the
    // GO it belongs to, exactly as C++ sends the combat log after SendSpellGo.
    let damage_delivery =
        deliver_creature_spell_damage_commands_like_cpp(&outcome.spell_damage_commands, registry);
    (outcome, plan_delivery, damage_delivery)
}
