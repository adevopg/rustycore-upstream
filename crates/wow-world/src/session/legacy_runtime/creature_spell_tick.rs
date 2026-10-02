//! Legacy creature spell tick and the cast validation it appends.
//!
//! Moved out of the Session root under #619. Behaviour is preserved; the
//! canonical owner of this state is unchanged.

use super::*;

/// Runs the C++ stock `CombatAI`/`TurretAI` template-spell decision once.
///
/// Selection, cooldown mutation, target preconditions, and START/GO packet
/// construction stay under the global map-owned runtime. Effect execution is
/// intentionally not invented here: damage/heal calculation belongs to M3.2,
/// while this M2.6 slice proves the cast decision and wire topology.
pub fn run_legacy_creature_spell_tick_once_like_cpp(
    legacy_map_manager: &crate::map_manager::SharedMapManager,
    canonical_map_manager: Option<&SharedCanonicalMapManager>,
    config: &LegacyCreatureAggroConfigLikeCpp,
) -> LegacyCreatureSpellTickOutcomeLikeCpp {
    use crate::map_manager::RuntimeTickOwner;

    struct PendingCreatureSpellCastLikeCpp {
        command: CreatureSpellCastPlanLikeCpp,
        difficulty_id: u8,
        turret_ai: bool,
    }

    struct PendingCreatureSpellScheduleLikeCpp {
        caster_guid: ObjectGuid,
        map_id: u16,
        instance_id: u32,
        engagement_epoch: u64,
        slot: usize,
        minimum_ms: u64,
    }

    enum PendingCreatureSpellActionLikeCpp {
        Cast(PendingCreatureSpellCastLikeCpp),
        Schedule(PendingCreatureSpellScheduleLikeCpp),
        /// A TurretAI attempt C++ would still have made through `CastSpell`,
        /// whose swing reset depends only on the raw combat-range gate.
        TurretRejectedAttempt(TurretRejectedCastAttemptLikeCpp),
    }

    let mut outcome = LegacyCreatureSpellTickOutcomeLikeCpp::default();
    let Some(spell_store) = config.spell_store.as_ref() else {
        return outcome;
    };
    let mut map_difficulties = HashMap::new();
    if let Some(canonical_map_manager) = canonical_map_manager
        && let Ok(manager) = canonical_map_manager.lock()
    {
        manager.do_for_all_maps(|managed| {
            if let Ok(map_id) = u16::try_from(managed.map_id()) {
                map_difficulties.insert((map_id, managed.instance_id()), managed.difficulty());
            }
        });
    }
    let mut pending_actions = Vec::new();
    let mut manager = legacy_map_manager
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if manager.tick_owner() != RuntimeTickOwner::GlobalLegacy {
        outcome.skipped_owner_not_global = true;
        return outcome;
    }

    let map_keys = manager.active_map_keys();
    outcome.maps_seen = map_keys.len();
    for (map_id, instance_id) in map_keys {
        let difficulty_id = map_difficulties
            .get(&(map_id, instance_id))
            .copied()
            .unwrap_or(0);
        for guid in manager.creature_guids(map_id, instance_id) {
            let Some(creature) = manager.find_creature_mut(map_id, instance_id, guid) else {
                continue;
            };
            outcome.creatures_seen += 1;
            // `RUSTYCORE_CREATURE_SPELL_TRACE=1` also reports the per-creature
            // gate, because the aggregate counters cannot: a creature rejected
            // here increments nothing, so "entered combat but never cast" has no
            // other witness. Only a creature that is in combat or holds a combat
            // target is interesting, which keeps this quiet on a populated map.
            if (creature.state() == wow_entities::CreatureAiState::InCombat
                || creature.creature.ai_ownership().combat_target.is_some())
                && std::env::var_os("RUSTYCORE_CREATURE_SPELL_TRACE").is_some()
            {
                static GATE_TICKS: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(0);
                if GATE_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 100 == 0 {
                    tracing::info!(
                        creature = ?guid,
                        map_id,
                        instance_id,
                        alive = creature.is_alive(),
                        ai_state = ?creature.state(),
                        combat_target = ?creature.creature.ai_ownership().combat_target,
                        ai_kind = ?legacy_creature_ai_selection_decision_like_cpp(creature, config),
                        first_spell = creature.creature.spells()[0],
                        "creature spell gate"
                    );
                }
            }
            if !creature.is_alive() || creature.state() != wow_entities::CreatureAiState::InCombat {
                continue;
            }
            let Some(recipient_guid) = creature.creature.ai_ownership().combat_target else {
                continue;
            };
            // Creature-vs-creature effects need a future map-owned generic
            // spell executor and are intentionally not fabricated here.
            if !recipient_guid.is_player() {
                continue;
            }

            let ai_kind = legacy_creature_ai_selection_decision_like_cpp(creature, config);
            let spells = creature.creature.spells();

            match ai_kind {
                CreatureAiKindLikeCpp::CombatAI => {
                    if difficulty_id != 0 && spells.iter().any(|spell_id| *spell_id != 0) {
                        // Effects/range/cooldowns have difficulty-aware stores,
                        // but cast time, focus and SpellPower rows are still
                        // hydrated only from difficulty 0. Do not claim a
                        // successful active-difficulty cast from mixed metadata.
                        outcome.spell_effects_unrepresented += 1;
                        continue;
                    }
                    if creature_ai_has_temporally_unrepresented_noninstant_spell_like_cpp(
                        &spells,
                        difficulty_id,
                        config,
                    ) {
                        outcome.noninstant_casts_unrepresented += 1;
                        continue;
                    }
                    if !creature.creature_spell_schedule_initialized_like_cpp() {
                        creature.mark_creature_spell_schedule_initialized_like_cpp();
                        outcome.schedules_initialized += 1;
                        'spell_slots: for (slot, spell_id) in spells.into_iter().enumerate() {
                            if spell_id == 0 {
                                continue;
                            }
                            let Some(spell) = i32::try_from(spell_id)
                                .ok()
                                .and_then(|spell_id| spell_store.get(spell_id))
                            else {
                                outcome.missing_spell_metadata += 1;
                                continue;
                            };
                            let spell = creature_ai_effective_spell_info_like_cpp(
                                spell,
                                difficulty_id,
                                config,
                            );
                            match creature_ai_spell_condition_like_cpp(
                                spell_id,
                                difficulty_id,
                                config,
                            ) {
                                CreatureAiSpellConditionLikeCpp::Aggro => {
                                    // C++ `CombatAI::JustEngagedWith` casts
                                    // AICOND_AGGRO directly on `who`, bypassing
                                    // `UnitAI::DoCast` target classification.
                                    match creature_ai_spell_disable_decision_like_cpp(
                                        spell_id, map_id, creature, config,
                                    ) {
                                        CreatureSpellDisableDecisionLikeCpp::Enabled => {}
                                        CreatureSpellDisableDecisionLikeCpp::Disabled => {
                                            outcome.spells_disabled += 1;
                                            continue;
                                        }
                                        CreatureSpellDisableDecisionLikeCpp::ContextUnrepresented => {
                                            outcome.spell_disable_context_unrepresented += 1;
                                            creature.record_swing();
                                            break 'spell_slots;
                                        }
                                    }
                                    if !config
                                        .spell_has_no_unrepresented_runtime_hooks_like_cpp(spell_id)
                                    {
                                        outcome.spell_runtime_hooks_unrepresented += 1;
                                        creature.record_swing();
                                        break 'spell_slots;
                                    }
                                    if !config
                                        .spell_has_no_unrepresented_casting_requirements_like_cpp(
                                            spell_id,
                                        )
                                    {
                                        outcome.spell_casting_requirements_unrepresented += 1;
                                        creature.record_swing();
                                        break 'spell_slots;
                                    }
                                    if !config
                                        .spell_has_no_unrepresented_shapeshift_requirements_like_cpp(
                                            spell_id,
                                        )
                                    {
                                        outcome.spell_casting_requirements_unrepresented += 1;
                                        creature.record_swing();
                                        break 'spell_slots;
                                    }
                                    if !config
                                        .spell_has_no_unrepresented_aura_restrictions_like_cpp(
                                            spell_id,
                                            difficulty_id,
                                        )
                                    {
                                        outcome.spell_effects_unrepresented += 1;
                                        creature.record_swing();
                                        break 'spell_slots;
                                    }
                                    if !creature_ai_spell_has_represented_cooldown_semantics_like_cpp(
                                        spell_id,
                                        difficulty_id,
                                        config,
                                    ) {
                                        outcome.spell_effects_unrepresented += 1;
                                        creature.record_swing();
                                        break 'spell_slots;
                                    }
                                    if creature_ai_spell_is_combat_forbidden_like_cpp(
                                        spell_id,
                                        difficulty_id,
                                        config,
                                    ) {
                                        outcome.spell_effects_unrepresented += 1;
                                        creature.record_swing();
                                        break 'spell_slots;
                                    }
                                    if creature_ai_zero_power_rows_have_unrepresented_implicit_cost_like_cpp(
                                        &spell,
                                        difficulty_id,
                                        config,
                                        creature,
                                    ) {
                                        outcome.spell_effects_unrepresented += 1;
                                        creature.record_swing();
                                        break 'spell_slots;
                                    }
                                    if creature_ai_spell_has_unrepresented_target_restrictions_like_cpp(
                                        spell_id,
                                        difficulty_id,
                                        config,
                                    ) {
                                        outcome.spell_effects_unrepresented += 1;
                                        creature.record_swing();
                                        break 'spell_slots;
                                    }
                                    match creature_ai_spell_single_unit_topology_like_cpp(
                                        &spell,
                                        recipient_guid,
                                        recipient_guid,
                                        creature_ai_spell_requires_projectile_payload_like_cpp(
                                            spell_id,
                                            difficulty_id,
                                            config,
                                        ),
                                    ) {
                                        Ok(()) => {}
                                        Err(CreatureAiSpellRepresentationRejectionLikeCpp::NonInstant) => {
                                            outcome.noninstant_casts_unrepresented += 1;
                                            creature.record_swing();
                                            break 'spell_slots;
                                        }
                                        Err(CreatureAiSpellRepresentationRejectionLikeCpp::ProjectileOrAmmo) => {
                                            outcome.spell_projectiles_unrepresented += 1;
                                            creature.record_swing();
                                            break 'spell_slots;
                                        }
                                        Err(CreatureAiSpellRepresentationRejectionLikeCpp::EffectOrTarget) => {
                                            outcome.spell_effects_unrepresented += 1;
                                            creature.record_swing();
                                            break 'spell_slots;
                                        }
                                    }
                                    let Ok(command) = creature_ai_spell_plan_like_cpp(
                                        creature,
                                        guid,
                                        recipient_guid,
                                        map_id,
                                        instance_id,
                                        spell_id,
                                        &spell,
                                        difficulty_id,
                                        config,
                                    ) else {
                                        outcome.spell_visuals_unrepresented += 1;
                                        creature.record_swing();
                                        break 'spell_slots;
                                    };
                                    pending_actions.push(PendingCreatureSpellActionLikeCpp::Cast(
                                        PendingCreatureSpellCastLikeCpp {
                                            command,
                                            difficulty_id,
                                            turret_ai: false,
                                        },
                                    ));
                                    outcome.casts_ready += 1;
                                }
                                CreatureAiSpellConditionLikeCpp::Combat => {
                                    // Preserve the C++ slot order: an earlier
                                    // AICOND_AGGRO cast (and its hit roll) runs
                                    // before a later AICOND_COMBAT schedule
                                    // draws its randomized initial delay.
                                    let minimum = creature_ai_spell_initial_cooldown_like_cpp(
                                        spell_id,
                                        &spell,
                                        difficulty_id,
                                        config,
                                    );
                                    pending_actions.push(
                                        PendingCreatureSpellActionLikeCpp::Schedule(
                                            PendingCreatureSpellScheduleLikeCpp {
                                                caster_guid: guid,
                                                map_id,
                                                instance_id,
                                                engagement_epoch: creature
                                                    .creature_spell_engagement_epoch_like_cpp(),
                                                slot,
                                                minimum_ms: minimum,
                                            },
                                        ),
                                    );
                                }
                                CreatureAiSpellConditionLikeCpp::Die => {}
                            }
                        }
                    }

                    // C++ advances EventMap before this gate but executes no
                    // due event while the creature owns UNIT_STATE_CASTING.
                    if creature
                        .creature
                        .unit()
                        .has_unit_state(UnitState::CASTING.bits())
                    {
                        outcome.unit_state_casting_skips += 1;
                        continue;
                    }

                    if let Some(slot) = creature.first_due_creature_spell_slot_like_cpp() {
                        // C++ `EventMap::ExecuteEvent` removes the due event
                        // before `DoCast`. A represented repeat schedule below
                        // rearms it; an RNG tombstone deliberately leaves it
                        // absent instead of retrying the same event every tick.
                        creature.clear_creature_spell_slot_like_cpp(slot);
                        let spell_id = spells[slot];
                        let Some(spell) = i32::try_from(spell_id)
                            .ok()
                            .and_then(|spell_id| spell_store.get(spell_id))
                        else {
                            outcome.missing_spell_metadata += 1;
                            continue;
                        };
                        let spell =
                            creature_ai_effective_spell_info_like_cpp(spell, difficulty_id, config);
                        let target_kind = creature_ai_spell_target_like_cpp(
                            spell_id,
                            &spell,
                            difficulty_id,
                            config,
                        );
                        if target_kind.requires_random_threat_selection_like_cpp() {
                            // C++ Enemy/Debuff selection draws from the full
                            // non-offline threat list before CastSpell. This
                            // slice cannot yet prove every candidate's range,
                            // aura and player/NPC filters. Stop exact spell RNG
                            // accreditation instead of selecting the victim or
                            // drawing the later repeat delay out of order.
                            outcome.spell_effects_unrepresented += 1;
                            creature.invalidate_runtime_rng_authority_like_cpp();
                            continue;
                        }
                        let target_guid = target_kind
                            .resolve_for_single_player_threat_list_like_cpp(guid, recipient_guid);

                        // `CombatAI::UpdateAI` re-schedules after every
                        // `DoCast` attempt, including a failed one. Defer the
                        // delay draw until after a represented hit roll so the
                        // shared creature RNG follows C++ DoCast -> ScheduleEvent.
                        let minimum = creature_ai_spell_repeat_cooldown_like_cpp(
                            spell_id,
                            &spell,
                            difficulty_id,
                            config,
                        );
                        let schedule = PendingCreatureSpellScheduleLikeCpp {
                            caster_guid: guid,
                            map_id,
                            instance_id,
                            engagement_epoch: creature.creature_spell_engagement_epoch_like_cpp(),
                            slot,
                            minimum_ms: minimum,
                        };

                        match creature_ai_spell_disable_decision_like_cpp(
                            spell_id, map_id, creature, config,
                        ) {
                            CreatureSpellDisableDecisionLikeCpp::Enabled => {}
                            CreatureSpellDisableDecisionLikeCpp::Disabled => {
                                outcome.spells_disabled += 1;
                                pending_actions
                                    .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                                continue;
                            }
                            CreatureSpellDisableDecisionLikeCpp::ContextUnrepresented => {
                                outcome.spell_disable_context_unrepresented += 1;
                                pending_actions
                                    .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                                continue;
                            }
                        }
                        if !config.spell_has_no_unrepresented_runtime_hooks_like_cpp(spell_id) {
                            outcome.spell_runtime_hooks_unrepresented += 1;
                            pending_actions
                                .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                            continue;
                        }
                        if !config
                            .spell_has_no_unrepresented_casting_requirements_like_cpp(spell_id)
                        {
                            outcome.spell_casting_requirements_unrepresented += 1;
                            pending_actions
                                .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                            continue;
                        }
                        if !config
                            .spell_has_no_unrepresented_shapeshift_requirements_like_cpp(spell_id)
                        {
                            outcome.spell_casting_requirements_unrepresented += 1;
                            pending_actions
                                .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                            continue;
                        }
                        if !config.spell_has_no_unrepresented_aura_restrictions_like_cpp(
                            spell_id,
                            difficulty_id,
                        ) {
                            outcome.spell_effects_unrepresented += 1;
                            pending_actions
                                .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                            continue;
                        }
                        if !creature_ai_spell_has_represented_cooldown_semantics_like_cpp(
                            spell_id,
                            difficulty_id,
                            config,
                        ) {
                            outcome.spell_effects_unrepresented += 1;
                            pending_actions
                                .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                            continue;
                        }
                        if creature_ai_spell_is_combat_forbidden_like_cpp(
                            spell_id,
                            difficulty_id,
                            config,
                        ) {
                            outcome.spell_effects_unrepresented += 1;
                            pending_actions
                                .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                            continue;
                        }
                        if creature_ai_zero_power_rows_have_unrepresented_implicit_cost_like_cpp(
                            &spell,
                            difficulty_id,
                            config,
                            creature,
                        ) {
                            outcome.spell_effects_unrepresented += 1;
                            pending_actions
                                .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                            continue;
                        }
                        if creature_ai_spell_has_unrepresented_target_restrictions_like_cpp(
                            spell_id,
                            difficulty_id,
                            config,
                        ) {
                            outcome.spell_effects_unrepresented += 1;
                            pending_actions
                                .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                            continue;
                        }
                        match creature_ai_spell_single_unit_topology_like_cpp(
                            &spell,
                            target_guid,
                            recipient_guid,
                            creature_ai_spell_requires_projectile_payload_like_cpp(
                                spell_id,
                                difficulty_id,
                                config,
                            ),
                        ) {
                            Ok(()) => {}
                            Err(CreatureAiSpellRepresentationRejectionLikeCpp::NonInstant) => {
                                outcome.noninstant_casts_unrepresented += 1;
                                pending_actions
                                    .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                                continue;
                            }
                            Err(
                                CreatureAiSpellRepresentationRejectionLikeCpp::ProjectileOrAmmo,
                            ) => {
                                outcome.spell_projectiles_unrepresented += 1;
                                pending_actions
                                    .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                                continue;
                            }
                            Err(CreatureAiSpellRepresentationRejectionLikeCpp::EffectOrTarget) => {
                                outcome.spell_effects_unrepresented += 1;
                                pending_actions
                                    .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                                continue;
                            }
                        }
                        let Ok(command) = creature_ai_spell_plan_like_cpp(
                            creature,
                            guid,
                            target_guid,
                            map_id,
                            instance_id,
                            spell_id,
                            &spell,
                            difficulty_id,
                            config,
                        ) else {
                            outcome.spell_visuals_unrepresented += 1;
                            pending_actions
                                .push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                            continue;
                        };
                        pending_actions.push(PendingCreatureSpellActionLikeCpp::Cast(
                            PendingCreatureSpellCastLikeCpp {
                                command,
                                difficulty_id,
                                turret_ai: false,
                            },
                        ));
                        pending_actions.push(PendingCreatureSpellActionLikeCpp::Schedule(schedule));
                        outcome.casts_ready += 1;
                    }
                }
                CreatureAiKindLikeCpp::TurretAI => {
                    if difficulty_id != 0 && spells[0] != 0 {
                        // See the CombatAI gate above: TurretAI has the same
                        // unhydrated difficulty-specific cast metadata.
                        outcome.spell_effects_unrepresented += 1;
                        continue;
                    }
                    // C++ TurretAI only ever reads `m_spells[0]`; unrelated
                    // template slots cannot put it into UNIT_STATE_CASTING.
                    if creature_ai_has_temporally_unrepresented_noninstant_spell_like_cpp(
                        &spells[..1],
                        difficulty_id,
                        config,
                    ) {
                        outcome.noninstant_casts_unrepresented += 1;
                        continue;
                    }
                    if creature
                        .creature
                        .unit()
                        .has_unit_state(UnitState::CASTING.bits())
                    {
                        outcome.unit_state_casting_skips += 1;
                        continue;
                    }
                    let spell_id = spells[0];
                    if spell_id == 0 || !creature.can_swing() {
                        continue;
                    }
                    let Some(spell) = i32::try_from(spell_id)
                        .ok()
                        .and_then(|spell_id| spell_store.get(spell_id))
                    else {
                        outcome.missing_spell_metadata += 1;
                        continue;
                    };
                    let spell =
                        creature_ai_effective_spell_info_like_cpp(spell, difficulty_id, config);
                    match creature_ai_spell_disable_decision_like_cpp(
                        spell_id, map_id, creature, config,
                    ) {
                        CreatureSpellDisableDecisionLikeCpp::Enabled => {}
                        CreatureSpellDisableDecisionLikeCpp::Disabled => {
                            outcome.spells_disabled += 1;
                            // C++ reaches `CastSpell` and lets `Spell::CheckCast`
                            // reject the disabled spell, so BASE_ATTACK is still
                            // consumed whenever the raw combat-range gate in
                            // `DoSpellAttackIfReady` admitted the attempt.
                            // Deciding that here would use the pre-tick legacy
                            // position, so defer it to the canonical drain.
                            pending_actions.push(
                                PendingCreatureSpellActionLikeCpp::TurretRejectedAttempt(
                                    TurretRejectedCastAttemptLikeCpp {
                                        caster_guid: guid,
                                        target_guid: recipient_guid,
                                        map_id,
                                        instance_id,
                                        engagement_epoch: creature
                                            .creature_spell_engagement_epoch_like_cpp(),
                                        spell_id,
                                        difficulty_id,
                                    },
                                ),
                            );
                            continue;
                        }
                        CreatureSpellDisableDecisionLikeCpp::ContextUnrepresented => {
                            outcome.spell_disable_context_unrepresented += 1;
                            continue;
                        }
                    }
                    if !config.spell_has_no_unrepresented_runtime_hooks_like_cpp(spell_id) {
                        outcome.spell_runtime_hooks_unrepresented += 1;
                        continue;
                    }
                    if !config.spell_has_no_unrepresented_casting_requirements_like_cpp(spell_id) {
                        outcome.spell_casting_requirements_unrepresented += 1;
                        continue;
                    }
                    if !config.spell_has_no_unrepresented_shapeshift_requirements_like_cpp(spell_id)
                    {
                        outcome.spell_casting_requirements_unrepresented += 1;
                        creature.record_swing();
                        continue;
                    }
                    if !config.spell_has_no_unrepresented_aura_restrictions_like_cpp(
                        spell_id,
                        difficulty_id,
                    ) {
                        outcome.spell_effects_unrepresented += 1;
                        continue;
                    }
                    if !creature_ai_spell_has_represented_cooldown_semantics_like_cpp(
                        spell_id,
                        difficulty_id,
                        config,
                    ) {
                        outcome.spell_effects_unrepresented += 1;
                        continue;
                    }
                    if creature_ai_spell_is_combat_forbidden_like_cpp(
                        spell_id,
                        difficulty_id,
                        config,
                    ) {
                        outcome.spell_effects_unrepresented += 1;
                        // TurretAI resets BASE_ATTACK after its CastSpell call
                        // even when CheckCast rejects the peaceful-only spell.
                        creature.record_swing();
                        continue;
                    }
                    if creature_ai_zero_power_rows_have_unrepresented_implicit_cost_like_cpp(
                        &spell,
                        difficulty_id,
                        config,
                        creature,
                    ) {
                        outcome.spell_effects_unrepresented += 1;
                        continue;
                    }
                    if creature_ai_spell_has_unrepresented_target_restrictions_like_cpp(
                        spell_id,
                        difficulty_id,
                        config,
                    ) {
                        outcome.spell_effects_unrepresented += 1;
                        continue;
                    }
                    match creature_ai_spell_single_unit_topology_like_cpp(
                        &spell,
                        recipient_guid,
                        recipient_guid,
                        creature_ai_spell_requires_projectile_payload_like_cpp(
                            spell_id,
                            difficulty_id,
                            config,
                        ),
                    ) {
                        Ok(()) => {}
                        Err(CreatureAiSpellRepresentationRejectionLikeCpp::NonInstant) => {
                            outcome.noninstant_casts_unrepresented += 1;
                            continue;
                        }
                        Err(CreatureAiSpellRepresentationRejectionLikeCpp::ProjectileOrAmmo) => {
                            outcome.spell_projectiles_unrepresented += 1;
                            continue;
                        }
                        Err(CreatureAiSpellRepresentationRejectionLikeCpp::EffectOrTarget) => {
                            outcome.spell_effects_unrepresented += 1;
                            continue;
                        }
                    }
                    let Ok(command) = creature_ai_spell_plan_like_cpp(
                        creature,
                        guid,
                        recipient_guid,
                        map_id,
                        instance_id,
                        spell_id,
                        &spell,
                        difficulty_id,
                        config,
                    ) else {
                        outcome.spell_visuals_unrepresented += 1;
                        continue;
                    };
                    pending_actions.push(PendingCreatureSpellActionLikeCpp::Cast(
                        PendingCreatureSpellCastLikeCpp {
                            command,
                            difficulty_id,
                            turret_ai: true,
                        },
                    ));
                    outcome.casts_ready += 1;
                }
                _ => {}
            }
        }
    }

    drop(manager);
    for pending_action in pending_actions {
        let pending_cast = match pending_action {
            PendingCreatureSpellActionLikeCpp::Schedule(schedule) => {
                let mut manager = legacy_map_manager
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let Some(creature) = manager.find_creature_mut(
                    schedule.map_id,
                    schedule.instance_id,
                    schedule.caster_guid,
                ) else {
                    continue;
                };
                if creature.creature_spell_engagement_epoch_like_cpp() != schedule.engagement_epoch
                {
                    continue;
                }
                let Some(delay) = creature.random_creature_spell_delay_like_cpp(
                    schedule.minimum_ms,
                    schedule.minimum_ms.saturating_mul(2),
                ) else {
                    outcome.runtime_rng_authority_rejections += 1;
                    continue;
                };
                creature.schedule_creature_spell_slot_after_like_cpp(schedule.slot, delay);
                continue;
            }
            PendingCreatureSpellActionLikeCpp::TurretRejectedAttempt(attempt) => {
                let Some(canonical_map_manager) = canonical_map_manager else {
                    continue;
                };
                if apply_turret_rejected_cast_attempt_like_cpp(
                    canonical_map_manager,
                    legacy_map_manager,
                    &attempt,
                    config,
                ) {
                    outcome.turret_rejected_attempt_swings += 1;
                }
                continue;
            }
            PendingCreatureSpellActionLikeCpp::Cast(pending_cast) => pending_cast,
        };
        let Some(canonical_map_manager) = canonical_map_manager else {
            outcome.canonical_cast_missing_target += 1;
            outcome.casts_ready = outcome.casts_ready.saturating_sub(1);
            continue;
        };
        let validation = validate_and_append_creature_spell_cast_like_cpp(
            canonical_map_manager,
            legacy_map_manager,
            &pending_cast.command,
            pending_cast.difficulty_id,
            pending_cast.turret_ai,
            config,
            &mut outcome.plan,
        );

        match validation {
            CreatureSpellCastValidationResultLikeCpp::Ready {
                hit_result,
                cast_id,
            } => {
                outcome.canonical_cast_preconditions_passed += 1;
                match hit_result {
                    CreatureSpellTargetHitResultLikeCpp::Hit => {
                        outcome.spell_hits += 1;
                        // C++ runs the effect handlers for a HIT target in
                        // `Spell::handle_immediate`, after the GO this plan
                        // already carries (`Spells/Spell.cpp:4081-4120`). The
                        // topology gate upstream admitted only a single instant
                        // school-damage effect at one enemy unit, so that is the
                        // one effect executed here.
                        let command = {
                            let Ok(mut canonical) = canonical_map_manager.lock() else {
                                outcome.spell_damage_effects_unresolved += 1;
                                continue;
                            };
                            // C++ reads the caster's level and its
                            // `GetSpellModOwner()` off the unit itself; a creature
                            // has the latter only while a player charms or owns it.
                            let caster_facts = canonical
                                .find_map_mut(
                                    u32::from(pending_cast.command.map_id),
                                    pending_cast.command.instance_id,
                                )
                                .and_then(|managed| {
                                    managed
                                        .map_mut()
                                        .get_typed_creature_mut(pending_cast.command.caster_guid)
                                        .map(|creature| {
                                            (
                                                u8::try_from(creature.unit().data().level)
                                                    .unwrap_or(u8::MAX),
                                                creature
                                                    .unit()
                                                    .subsystems()
                                                    .control
                                                    .charmer_or_owner_guid()
                                                    .is_some_and(|guid| guid.is_player()),
                                            )
                                        })
                                });
                            let Some((caster_level, caster_is_player_controlled)) = caster_facts
                            else {
                                outcome.spell_damage_effects_unresolved += 1;
                                continue;
                            };
                            super::creature_spell_damage::resolve_creature_spell_damage_effect_like_cpp(
                                &mut canonical,
                                u32::from(pending_cast.command.map_id),
                                pending_cast.command.instance_id,
                                super::creature_spell_damage::CreatureSpellAttackerFactsLikeCpp {
                                    guid: pending_cast.command.caster_guid,
                                    level: caster_level,
                                    is_player_controlled: caster_is_player_controlled,
                                },
                                pending_cast.command.target_guid,
                                pending_cast.command.spell_id,
                                cast_id,
                                pending_cast.command.spell_x_spell_visual_id,
                                spell_store,
                                pending_cast.difficulty_id,
                                config.difficulty_store.as_deref(),
                                config.npc_mana_cost_scaler_table.as_deref(),
                            )
                        };
                        match command {
                            Some(command) => {
                                outcome.spell_damage_effects_executed += 1;
                                outcome.spell_damage_commands.push(command);
                            }
                            None => outcome.spell_damage_effects_unresolved += 1,
                        }
                    }
                    CreatureSpellTargetHitResultLikeCpp::Miss => outcome.spell_misses += 1,
                }
            }
            CreatureSpellCastValidationResultLikeCpp::OutOfRange => {
                outcome.spell_range_rejections += 1;
                outcome.casts_ready = outcome.casts_ready.saturating_sub(1);
                continue;
            }
            CreatureSpellCastValidationResultLikeCpp::LosRejected => {
                outcome.spell_los_rejections += 1;
                outcome.casts_ready = outcome.casts_ready.saturating_sub(1);
                continue;
            }
            CreatureSpellCastValidationResultLikeCpp::MissingTarget => {
                outcome.canonical_cast_missing_target += 1;
                outcome.casts_ready = outcome.casts_ready.saturating_sub(1);
                continue;
            }
            CreatureSpellCastValidationResultLikeCpp::TargetRejected => {
                outcome.canonical_cast_target_rejections += 1;
                outcome.casts_ready = outcome.casts_ready.saturating_sub(1);
                continue;
            }
            CreatureSpellCastValidationResultLikeCpp::CooldownRejected => {
                outcome.canonical_cast_cooldown_rejections += 1;
                outcome.casts_ready = outcome.casts_ready.saturating_sub(1);
                continue;
            }
            CreatureSpellCastValidationResultLikeCpp::HitResultUnrepresented => {
                outcome.spell_hit_results_unrepresented += 1;
                outcome.casts_ready = outcome.casts_ready.saturating_sub(1);
                continue;
            }
            CreatureSpellCastValidationResultLikeCpp::CasterIncarnationRejected => {
                outcome.caster_incarnation_rejections += 1;
                outcome.casts_ready = outcome.casts_ready.saturating_sub(1);
                continue;
            }
            CreatureSpellCastValidationResultLikeCpp::RuntimeRngAuthorityRejected => {
                outcome.runtime_rng_authority_rejections += 1;
                outcome.casts_ready = outcome.casts_ready.saturating_sub(1);
                continue;
            }
        }
    }

    outcome
}
