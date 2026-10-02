//! C++ `Unit::CalculateSpellDamageTaken` for a player victim of a creature's
//! spell, inside the map phase that owns the victim's health write.
//!
//! C++ reaches this from `Spell::TargetInfo::DoDamageAndTriggers`
//! (`Spells/Spell.cpp:2960-2985`): `CalculateSpellDamageTaken`
//! (`Entities/Unit/Unit.cpp:1246-1360`) resolves the critical arm, then
//! `CalcAbsorbResist` (`:2080-2250`) takes the resist and the shields, and then
//! `DealSpellDamage` applies what is left. The arithmetic lives in
//! `session_rules`; this stage reads the canonical state those rules need and
//! commits the writes the map owns.
//!
//! The victim session still owns every publication: this returns what it must
//! send rather than sending anything.
//!
//! Its production caller is the creature spell tick, which executes the one
//! damage effect its topology gate admits and hands the result to the victim
//! session as a delivery command.

use super::*;

/// C++ `SPELL_ATTR0_SCALES_WITH_CREATURE_LEVEL` (`SharedDefines.h:472`): "For
/// non-player casts, scale impact and power cost with caster's level". It lives
/// here rather than in `wow_data::spell::attributes` because that module is a
/// #584 C4 file at its line ceiling and this is its only reader.
const SPELL_ATTR0_SCALES_WITH_CREATURE_LEVEL_LIKE_CPP: u32 = 0x0008_0000;

/// What one creature spell hit did to a player victim, and what the victim
/// session has to publish for it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::session) struct CreatureSpellDamageOutcomeLikeCpp {
    /// C++ `SpellNonMeleeDamage::originalDamage`, assigned after the critical arm
    /// and before `CalcAbsorbResist` (`Unit.cpp:1346-1347`).
    pub original_damage: u32,
    /// C++ `SpellNonMeleeDamage::damage`: what the victim actually took.
    pub damage: u32,
    /// C++ `SpellNonMeleeDamage::resist`.
    pub resisted: u32,
    /// C++ `SpellNonMeleeDamage::absorb`.
    pub absorbed: u32,
    /// The mana the mana-shield loop drained; the victim session publishes the
    /// resulting `SMSG_POWER_UPDATE` because it owns power publication.
    pub mana_spent: u32,
    /// C++ `SpellNonMeleeDamage::HitInfo`.
    pub hit_info: i32,
    /// Every shield this stage spent, in `AbsorbAuraOrderPred` order followed by
    /// the mana shields. The victim session owns the absorb-log publication and
    /// the aura transition.
    pub absorb_consumptions: Vec<crate::session_rules::RepresentedAbsorbConsumptionLikeCpp>,
    /// C++ `SpellNonMeleeDamage::preHitHealth`, read before `DealDamage` so the
    /// log can report the overkill.
    pub victim_health_before: u64,
    pub victim_health_after: u64,
    /// Whether this hit took the victim to zero.
    pub killed: bool,
}

/// Facts about the attacker that C++ reads off the caster rather than the victim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::session) struct CreatureSpellAttackerFactsLikeCpp {
    pub guid: ObjectGuid,
    /// C++ `caster->GetLevelForTarget(victim)`.
    pub level: u8,
    /// C++ `Unit::GetSpellModOwner()`: a creature with one is player-controlled,
    /// and only then can its spell crit at all (`Unit.cpp:7709-7711`).
    pub is_player_controlled: bool,
}

/// Apply one creature spell hit to a canonical player victim.
///
/// Returns `None` when the map or the canonical player cannot be resolved, or
/// when C++ would not have dealt the damage at all: `DealSpellDamage` returns
/// early for a dead victim (`Unit.cpp:1371-1372`).
///
/// Boundaries, each a fact this port does not carry rather than a choice:
///
/// * a plain creature caster cannot crit, which is C++'s own rule rather than a
///   gap (`Unit.cpp:7709-7711`); the critical arm is still resolved through the
///   same rule the player path uses, so a player-controlled creature gets it.
/// * the caster's `SPELL_AURA_MOD_TARGET_RESISTANCE` and spell penetration are
///   zero, because no represented creature carries either.
/// * `SPELL_ATTR0_CU_BINARY_SPELL` is taken as unset, as on the player's own
///   cast path, so the level-based resistance term always applies.
/// * the split-damage families (`Unit.cpp:2252-2358`) stay with the melee path
///   that represents them.
#[allow(clippy::too_many_arguments)]
pub(in crate::session) fn apply_creature_spell_damage_to_canonical_player_like_cpp(
    canonical_manager: &mut wow_map::MapManager,
    map_id: u32,
    instance_id: u32,
    attacker: CreatureSpellAttackerFactsLikeCpp,
    victim_guid: ObjectGuid,
    spell_id: i32,
    damage: u32,
    spell_store: &wow_data::SpellStore,
    difficulty_id: u8,
    difficulty_store: Option<&wow_data::DifficultyStore>,
    // C++ `rand_norm()` for the resist bucket and `roll_chance_f` for the
    // critical, injected so a scenario can pin both.
    rolls: CreatureSpellDamageRollsLikeCpp,
) -> Option<CreatureSpellDamageOutcomeLikeCpp> {
    let metadata = spell_store.hit_metadata_for_difficulty_like_cpp(
        spell_id,
        difficulty_id,
        difficulty_store,
    )?;
    let school_mask = metadata.school_mask;

    // Everything the rules need off the victim, read once so the map lock is held
    // for one borrow rather than per stage.
    let (victim_resistance, victim_level, victim_is_alive, victim_health_before) = {
        let managed = canonical_manager.find_map_mut(map_id, instance_id)?;
        let player = managed.map_mut().get_typed_player_mut(victim_guid)?;
        let stats = player.effective_combat_stats_like_cpp();
        (
            wow_entities::resistance_for_school_mask_like_cpp(&stats.resistances, school_mask),
            u8::try_from(player.unit().data().level).unwrap_or(u8::MAX),
            player.unit().is_alive(),
            player.unit().data().health,
        )
    };
    // C++ `DealSpellDamage` returns before `DealDamage` for a victim that is not
    // alive, so nothing is published for it either.
    if !victim_is_alive {
        return None;
    }

    // C++ `Spell::PreprocessSpellLaunch` rolls one critical chance per target
    // before the hit, and `CalculateSpellDamageTaken` applies its arm.
    let crit_chance = crate::session_rules::represented_spell_crit_chance_taken_like_cpp(
        &crate::session_rules::RepresentedSpellCritChanceTakenFactsLikeCpp {
            spell_can_crit: spell_store.spell_can_crit_like_cpp(
                spell_id,
                difficulty_id,
                difficulty_store,
            ),
            defense_type: metadata.defense_type,
            spell_is_positive: false,
            attacker_spell_crit_chance_aura_pct: 0.0,
            attacker_spell_and_weapon_crit_chance_aura_pct: 0.0,
            resilience_crit_taken_pct: 0.0,
            unit_crit_chance_taken_pct: 0.0,
            crit_chance_for_caster_aura_pct: 0.0,
        },
        crate::session_rules::represented_spell_crit_chance_done_like_cpp(
            &crate::session_rules::RepresentedSpellCritChanceDoneFactsLikeCpp {
                // C++ `if (GetTypeId() == TYPEID_UNIT && !GetSpellModOwner())
                // return 0.0f`: a mob's spell cannot crit unless it is
                // player-controlled.
                caster_can_crit_at_all: attacker.is_player_controlled,
                caster_is_player: false,
                // C++ always crits a sitting target, but only for a player
                // caster, which this never is.
                victim_is_standing: true,
                spell_is_healing: false,
                spell_can_crit: spell_store.spell_can_crit_like_cpp(
                    spell_id,
                    difficulty_id,
                    difficulty_store,
                ),
                defense_type: metadata.defense_type,
                school_mask,
                player_spell_crit_pct: 0.0,
                unit_crit_chance_done_pct: 0.0,
                spell_crit_chance_school_aura_pct: 0.0,
                base_spell_crit_chance_pct: 0.0,
            },
        ),
    );
    let is_critical = crit_chance > 0.0 && rolls.critical < crit_chance;
    let original_damage = if is_critical {
        crate::session_rules::represented_spell_crit_damage_like_cpp(
            damage,
            metadata.defense_type,
            0.0,
        )
    } else {
        damage
    };

    // C++ `CalcAbsorbResist` runs `CalcSpellResistedDamage` first (`:2084`).
    let average_resist = crate::session_rules::represented_average_resist_reduction_like_cpp(
        &crate::session_rules::RepresentedAverageResistFactsLikeCpp {
            victim_resistance,
            caster_mod_target_resistance: 0,
            caster_spell_penetration: 0,
            school_mask,
            is_binary_spell: false,
            has_caster: true,
            victim_level,
            caster_level: attacker.level,
        },
    );
    let resisted = crate::session_rules::represented_spell_resisted_damage_like_cpp(
        original_damage,
        school_mask,
        // The victim is a player, so C++ returns zero for a holy school.
        false,
        average_resist,
        0,
        rolls.resist,
        None,
    )
    .min(original_damage);
    let post_resist_damage = original_damage - resisted;

    // Then both shield loops, through the stage the melee swing already uses.
    let absorb = super::player_victim_absorb::apply_absorb_stages_to_canonical_player_like_cpp(
        canonical_manager,
        map_id,
        instance_id,
        victim_guid,
        u32::from(school_mask),
        post_resist_damage,
        spell_store,
        difficulty_id,
        difficulty_store,
        // C++'s `auraAbsorbMod` comes from the attacker's
        // `SPELL_AURA_MOD_TARGET_ABSORB_SCHOOL`, which no represented creature
        // carries.
        0.0,
    );
    let (absorbed, damage_after_absorb, mana_spent, absorb_consumptions) = match absorb {
        Some(absorb) => absorb,
        // The victim vanished between the two borrows; publishing a hit it never
        // took would be worse than dropping it.
        None => return None,
    };

    // C++ `Unit::DealDamage` applies what the stages left, in the same map phase.
    let (victim_health_after, killed) = {
        let managed = canonical_manager.find_map_mut(map_id, instance_id)?;
        let player = managed.map_mut().get_typed_player_mut(victim_guid)?;
        let health_before = player.unit().data().health;
        let health_after = health_before.saturating_sub(u64::from(damage_after_absorb));
        player.unit_mut().set_health(health_after);
        if health_after == 0 {
            player
                .unit_mut()
                .set_death_state(wow_constants::DeathState::JustDied);
        }
        (health_after, health_after == 0)
    };

    let mut hit_info = if is_critical {
        crate::session_rules::SPELL_HIT_TYPE_CRIT_LIKE_CPP
    } else {
        0
    };
    hit_info |=
        crate::session_rules::represented_resist_hit_info_like_cpp(original_damage, resisted);

    Some(CreatureSpellDamageOutcomeLikeCpp {
        original_damage,
        damage: damage_after_absorb,
        resisted,
        absorbed,
        mana_spent,
        hit_info,
        absorb_consumptions,
        victim_health_before,
        victim_health_after,
        killed,
    })
}

/// The two draws C++ makes for one spell hit, injected so a scenario can pin
/// them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::session) struct CreatureSpellDamageRollsLikeCpp {
    /// C++ `roll_chance_f(crit_chance)`'s draw (`Spells/Spell.cpp:8675-8684`),
    /// as a percentage.
    pub critical: f32,
    /// C++ `rand_norm()` for the resist bucket (`Unit.cpp:1994`).
    pub resist: f32,
}

impl CreatureSpellDamageRollsLikeCpp {
    /// The live draws the server makes outside a scenario.
    pub(in crate::session) fn live_like_cpp() -> Self {
        Self {
            critical: wow_core::rand_chance_like_cpp(),
            resist: wow_core::rand_norm_like_cpp(),
        }
    }
}

/// C++ `Spell::_handle_immediate_phase` → `DoProcessTargetContainer` →
/// `TargetInfo::DoTargetSpellHit`/`DoDamageAndTriggers` for the one effect this
/// slice represents (`Spells/Spell.cpp:4258-4276`, `:2794-2985`).
///
/// The topology gate upstream has already narrowed the cast to a single instant
/// `SPELL_EFFECT_SCHOOL_DAMAGE` effect with `TARGET_UNIT_TARGET_ENEMY`, so this
/// resolves that effect's `CalcValue` with the creature caster and hands it to
/// the hit chain. Returns `None` when there is nothing to resolve it from, which
/// the caller counts rather than guessing a value.
#[allow(clippy::too_many_arguments)]
pub(in crate::session) fn resolve_creature_spell_damage_effect_like_cpp(
    canonical_manager: &mut wow_map::MapManager,
    map_id: u32,
    instance_id: u32,
    attacker: CreatureSpellAttackerFactsLikeCpp,
    victim_guid: ObjectGuid,
    spell_id: i32,
    cast_id: ObjectGuid,
    spell_visual_id: u32,
    spell_store: &wow_data::SpellStore,
    difficulty_id: u8,
    difficulty_store: Option<&wow_data::DifficultyStore>,
    npc_mana_cost_scaler: Option<&wow_data::NpcManaCostScalerGameTableLikeCpp>,
) -> Option<crate::session::mailbox::ApplyCreatureSpellDamageLikeCppCommand> {
    let effects =
        spell_store.effects_for_difficulty_like_cpp(spell_id, difficulty_id, difficulty_store)?;
    let effect = effects.iter().find(|effect| {
        effect.effect == wow_data::spell::spell_effect_types::SPELL_EFFECT_SCHOOL_DAMAGE
    })?;
    let levels =
        spell_store.spell_levels_for_difficulty_like_cpp(spell_id, difficulty_id, difficulty_store);
    // C++ hands `EffectSchoolDMG` the effect's `CalcValue(caster)`, never the raw
    // `EffectBasePoints`; for a creature caster that includes the
    // `NpcManaCostScaler` arm under `SPELL_ATTR0_SCALES_WITH_CREATURE_LEVEL`.
    let damage = effect
        .calc_value_with_caster_and_die_roll_like_cpp(
            levels,
            Some(wow_data::spell::CalcValueCasterLikeCpp {
                level: u32::from(attacker.level),
                // No represented creature tracks combo points.
                combo_points: 0,
                is_controlled_by_player: attacker.is_player_controlled,
                scales_with_creature_level: spell_store.has_attribute_for_difficulty_like_cpp(
                    spell_id,
                    difficulty_id,
                    difficulty_store,
                    0,
                    SPELL_ATTR0_SCALES_WITH_CREATURE_LEVEL_LIKE_CPP,
                ),
            }),
            npc_mana_cost_scaler,
            |min, max| wow_core::irand_like_cpp(min, max),
        )
        .max(0) as u32;

    let outcome = apply_creature_spell_damage_to_canonical_player_like_cpp(
        canonical_manager,
        map_id,
        instance_id,
        attacker,
        victim_guid,
        spell_id,
        damage,
        spell_store,
        difficulty_id,
        difficulty_store,
        CreatureSpellDamageRollsLikeCpp::live_like_cpp(),
    )?;
    let metadata = spell_store.hit_metadata_for_difficulty_like_cpp(
        spell_id,
        difficulty_id,
        difficulty_store,
    )?;
    Some(
        crate::session::mailbox::ApplyCreatureSpellDamageLikeCppCommand {
            attacker_guid: attacker.guid,
            victim_guid,
            map_id: u16::try_from(map_id).unwrap_or(0),
            instance_id,
            spell_id,
            cast_id,
            spell_visual_id,
            school_mask: metadata.school_mask,
            damage: outcome.damage,
            original_damage: outcome.original_damage,
            // C++ `if (log->damage > log->preHitHealth) Overkill = damage -
            // preHitHealth; else Overkill = -1` (`Unit.cpp:5890-5893`).
            overkill: if u64::from(outcome.damage) > outcome.victim_health_before {
                i32::try_from(u64::from(outcome.damage) - outcome.victim_health_before)
                    .unwrap_or(i32::MAX)
            } else {
                -1
            },
            resisted: outcome.resisted,
            absorbed: outcome.absorbed,
            mana_spent: outcome.mana_spent,
            hit_info: outcome.hit_info,
            absorb_consumptions: outcome
                .absorb_consumptions
                .iter()
                .map(
                    |consumption| crate::session::mailbox::CreatureMeleeAbsorbConsumptionLikeCpp {
                        slot: consumption.slot,
                        consumed: consumption.consumed,
                        removed: consumption.removed,
                    },
                )
                .collect(),
            victim_health_after: outcome.victim_health_after,
            victim_health_state_revision_after: creature_spell_victim_health_revision_like_cpp(
                canonical_manager,
                map_id,
                instance_id,
                victim_guid,
            )?,
            killed: outcome.killed,
        },
    )
}

/// The victim's health-state revision after the commit, which the delivery gate
/// compares against the session's own canonical view.
fn creature_spell_victim_health_revision_like_cpp(
    canonical_manager: &mut wow_map::MapManager,
    map_id: u32,
    instance_id: u32,
    victim_guid: ObjectGuid,
) -> Option<u64> {
    let managed = canonical_manager.find_map_mut(map_id, instance_id)?;
    let player = managed.map_mut().get_typed_player_mut(victim_guid)?;
    Some(player.unit().health_state_revision_like_cpp())
}
