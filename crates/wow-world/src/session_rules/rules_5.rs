// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Represented spell critical rules: the chance C++ rolls before a spell hit and
//! the damage its critical arm produces.
//!
//! C++ `Unit::SpellCritChanceDone` (`Entities/Unit/Unit.cpp:7706-7772`),
//! `Unit::SpellCritChanceTaken` (`:7774-7960`), the critical arms of
//! `Unit::CalculateSpellDamageTaken` (`:1266-1332`) and
//! `Unit::SpellCriticalDamageBonus` (`:7962-8003`). Separated from the melee
//! attack table in `rules_4` because a spell hit has no attack table: C++ rolls
//! one chance in `Spell::PreprocessSpellLaunch` (`Spells/Spell.cpp:8675-8684`).

/// C++ `SpellInfo::DmgClass`, which is `SpellCategories::DefenseType`
/// (`SharedDefines.h:1086-1092`).
pub(crate) const SPELL_DAMAGE_CLASS_NONE_LIKE_CPP: i8 = 0;
pub(crate) const SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP: i8 = 1;
pub(crate) const SPELL_DAMAGE_CLASS_MELEE_LIKE_CPP: i8 = 2;
pub(crate) const SPELL_DAMAGE_CLASS_RANGED_LIKE_CPP: i8 = 3;

/// C++ `SPELL_SCHOOL_MASK_NORMAL` (`SharedDefines.h:329`): the physical school.
pub(crate) const SPELL_SCHOOL_MASK_NORMAL_LIKE_CPP: u8 = 1;

/// C++ `SPELL_HIT_TYPE_CRIT` (`SharedDefines.h:2796`), the `HitInfo` bit the
/// critical arms set on `SpellNonMeleeDamage`.
pub(crate) const SPELL_HIT_TYPE_CRIT_LIKE_CPP: i32 = 0x02;

/// What C++ `Unit::SpellCritChanceDone` reads off the caster, the victim and the
/// `SpellInfo` (`Unit.cpp:7706-7772`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RepresentedSpellCritChanceDoneFactsLikeCpp {
    /// C++ `GetTypeId() == TYPEID_UNIT && !GetSpellModOwner()`: a creature with
    /// no player owner cannot crit with a spell at all.
    pub caster_can_crit_at_all: bool,
    /// C++ `IsPlayer()`, which selects the published per-school percentages.
    pub caster_is_player: bool,
    /// C++ `victim->IsStandState()`; true when there is no victim, because the
    /// sitting rule needs one.
    pub victim_is_standing: bool,
    /// C++ `spellInfo->IsHealingSpell()`.
    pub spell_is_healing: bool,
    /// C++ `SPELL_ATTR0_CU_CAN_CRIT`.
    pub spell_can_crit: bool,
    pub defense_type: i8,
    pub school_mask: u8,
    /// C++ `ActivePlayerData::SpellCritPercentage[GetFirstSchoolInMask(schoolMask)]`.
    pub player_spell_crit_pct: f32,
    /// C++ `GetUnitCriticalChanceDone(attackType)`.
    pub unit_crit_chance_done_pct: f32,
    /// C++ `GetTotalAuraModifierByMiscMask(SPELL_AURA_MOD_SPELL_CRIT_CHANCE_SCHOOL, schoolMask)`.
    pub spell_crit_chance_school_aura_pct: f32,
    /// C++ `m_baseSpellCritChance`, read only for a non-player caster.
    pub base_spell_crit_chance_pct: f32,
}

/// C++ `Unit::SpellCritChanceDone` (`Entities/Unit/Unit.cpp:7706-7772`).
///
/// Two deliberate omissions, both named rather than silently folded in:
/// the `SpellModOp::CritChance` spellmod, which needs the talent spellmod owner;
/// and the reference fork's `alistar:`-marked warlock healthstone special case in
/// `getPhysicalCritChance` (`:7729-7736`), which is a patched region and so not
/// parity evidence for this build.
pub(crate) fn represented_spell_crit_chance_done_like_cpp(
    facts: &RepresentedSpellCritChanceDoneFactsLikeCpp,
) -> f32 {
    // Mobs can't crit with spells unless player controlled.
    if !facts.caster_can_crit_at_all {
        return 0.0;
    }
    // Always crit sitting targets.
    if facts.caster_is_player && !facts.victim_is_standing && !facts.spell_is_healing {
        return 100.0;
    }
    if !facts.spell_can_crit {
        return 0.0;
    }

    let mut crit_chance = 0.0f32;
    match facts.defense_type {
        SPELL_DAMAGE_CLASS_NONE_LIKE_CPP | SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP => {
            if facts.school_mask & SPELL_SCHOOL_MASK_NORMAL_LIKE_CPP != 0 {
                crit_chance = crit_chance
                    .max(facts.unit_crit_chance_done_pct + facts.spell_crit_chance_school_aura_pct);
            }
            if facts.school_mask & !SPELL_SCHOOL_MASK_NORMAL_LIKE_CPP != 0 {
                let magic = if facts.caster_is_player {
                    facts.player_spell_crit_pct
                } else {
                    facts.base_spell_crit_chance_pct + facts.spell_crit_chance_school_aura_pct
                };
                crit_chance = crit_chance.max(magic);
            }
        }
        SPELL_DAMAGE_CLASS_MELEE_LIKE_CPP | SPELL_DAMAGE_CLASS_RANGED_LIKE_CPP => {
            crit_chance += facts.unit_crit_chance_done_pct;
            crit_chance += facts.spell_crit_chance_school_aura_pct;
        }
        _ => return 0.0,
    }
    crit_chance.max(0.0)
}

/// What C++ `Unit::SpellCritChanceTaken` reads off the victim (`Unit.cpp:7774-7960`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RepresentedSpellCritChanceTakenFactsLikeCpp {
    /// C++ `SPELL_ATTR0_CU_CAN_CRIT`, checked again on the taken side.
    pub spell_can_crit: bool,
    pub defense_type: i8,
    /// C++ `spellInfo->IsPositive()`.
    pub spell_is_positive: bool,
    /// The victim's `SPELL_AURA_MOD_ATTACKER_SPELL_CRIT_CHANCE` sum for this school.
    pub attacker_spell_crit_chance_aura_pct: f32,
    /// The victim's `SPELL_AURA_MOD_ATTACKER_SPELL_AND_WEAPON_CRIT_CHANCE` sum,
    /// which C++ adds *after* resilience.
    pub attacker_spell_and_weapon_crit_chance_aura_pct: f32,
    /// What `Unit::ApplyResilience(CR_CRIT_TAKEN_SPELL)` subtracts; zero when the
    /// caster cannot apply resilience.
    pub resilience_crit_taken_pct: f32,
    /// C++ `GetUnitCriticalChanceTaken(caster, attackType, crit_chance)` for the
    /// melee and ranged arms, already resolved by the owner.
    pub unit_crit_chance_taken_pct: f32,
    /// The victim's `SPELL_AURA_MOD_CRIT_CHANCE_FOR_CASTER` sum for this caster
    /// and spell, which C++ adds for every damage class.
    pub crit_chance_for_caster_aura_pct: f32,
}

/// C++ `Unit::SpellCritChanceTaken` (`Entities/Unit/Unit.cpp:7774-7960`).
///
/// Note the `SPELL_DAMAGE_CLASS_NONE` arm: it returns zero, so a spell of that
/// class never crits however large a chance the done side produced.
///
/// Omitted deliberately and named: the scripted class blocks at `:7799-7930`
/// (Shatter, Glyph of Shadowburn, Renewed Hope and the per-family cases), which
/// need `SPELL_AURA_OVERRIDE_CLASS_SCRIPTS` effects and aura-state reads this
/// port does not represent.
pub(crate) fn represented_spell_crit_chance_taken_like_cpp(
    facts: &RepresentedSpellCritChanceTakenFactsLikeCpp,
    done_chance: f32,
) -> f32 {
    if !facts.spell_can_crit {
        return 0.0;
    }
    let mut crit_chance = done_chance;
    match facts.defense_type {
        SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP => {
            if !facts.spell_is_positive {
                crit_chance += facts.attacker_spell_crit_chance_aura_pct;
                crit_chance -= facts.resilience_crit_taken_pct;
                crit_chance += facts.attacker_spell_and_weapon_crit_chance_aura_pct;
            }
        }
        SPELL_DAMAGE_CLASS_MELEE_LIKE_CPP | SPELL_DAMAGE_CLASS_RANGED_LIKE_CPP => {
            crit_chance = facts.unit_crit_chance_taken_pct;
        }
        _ => return 0.0,
    }
    crit_chance += facts.crit_chance_for_caster_aura_pct;
    crit_chance.max(0.0)
}

/// C++ `Unit::SpellCriticalDamageBonus` (`Entities/Unit/Unit.cpp:7962-8003`),
/// the magical arm's whole critical damage.
///
/// `crit_mod_pct` is the caster's
/// `(GetTotalAuraMultiplierByMiscMask(SPELL_AURA_MOD_CRIT_DAMAGE_BONUS, school) - 1) * 100`
/// plus `GetTotalAuraModifierByMiscMask(SPELL_AURA_MOD_CRIT_PERCENT_VERSUS, creatureTypeMask)`.
/// The `SpellModOp::CritDamageAndHealing` spellmod is not represented.
pub(crate) fn represented_spell_critical_damage_bonus_like_cpp(
    damage: u32,
    defense_type: i8,
    crit_mod_pct: f32,
) -> u32 {
    let damage_i64 = i64::from(damage);
    let mut crit_bonus = match defense_type {
        // For melee based spells it is 100%.
        SPELL_DAMAGE_CLASS_MELEE_LIKE_CPP | SPELL_DAMAGE_CLASS_RANGED_LIKE_CPP => {
            damage_i64 + damage_i64
        }
        // For spells it is 50%.
        _ => damage_i64 + damage_i64 / 2,
    };
    // C++ `AddPct` applies the modifier to the whole value, then the function
    // reduces to the bonus, applies the spellmod and adds the base back.
    if crit_bonus != 0 {
        crit_bonus += (crit_bonus as f64 * f64::from(crit_mod_pct) / 100.0) as i64;
    }
    crit_bonus -= damage_i64;
    crit_bonus += damage_i64;
    crit_bonus.clamp(0, i64::from(u32::MAX)) as u32
}

/// C++ `Unit::SpellCriticalHealingBonus` (`Entities/Unit/Unit.cpp:8005-8036`).
///
/// It is not the damage function with another name: the bonus is computed alone,
/// `SPELL_AURA_MOD_CRIT_PERCENT_VERSUS` multiplies *that bonus* rather than being
/// added as a percentage, the bonus is added only when positive, and
/// `SPELL_AURA_MOD_CRITICAL_HEALING_AMOUNT` then multiplies the whole heal.
///
/// `crit_percent_versus_multiplier` and `critical_healing_amount_multiplier` are
/// those two aura multipliers, both `1.0` when unrepresented.
pub(crate) fn represented_spell_critical_healing_bonus_like_cpp(
    heal: u32,
    defense_type: i8,
    crit_percent_versus_multiplier: f32,
    critical_healing_amount_multiplier: f32,
) -> u32 {
    let heal_i64 = i64::from(heal);
    let mut crit_bonus = match defense_type {
        SPELL_DAMAGE_CLASS_MELEE_LIKE_CPP | SPELL_DAMAGE_CLASS_RANGED_LIKE_CPP => heal_i64,
        _ => heal_i64 / 2,
    };
    crit_bonus = (crit_bonus as f64 * f64::from(crit_percent_versus_multiplier)) as i64;
    let mut healed = heal_i64;
    if crit_bonus > 0 {
        healed += crit_bonus;
    }
    healed = (healed as f64 * f64::from(critical_healing_amount_multiplier)) as i64;
    healed.clamp(0, i64::from(u32::MAX)) as u32
}

/// C++ `Unit::CalculateSpellDamageTaken`'s melee and ranged critical arm
/// (`Entities/Unit/Unit.cpp:1266-1298`), which does **not** call
/// `SpellCriticalDamageBonus`: it doubles the damage and then applies the
/// victim's and caster's critical-damage percentages.
///
/// `crit_pct_damage_mod` is the victim's
/// `SPELL_AURA_MOD_ATTACKER_MELEE_CRIT_DAMAGE` (or the ranged one) plus the
/// caster's `SPELL_AURA_MOD_CRIT_DAMAGE_BONUS` multiplier term and
/// `SPELL_AURA_MOD_CRIT_PERCENT_VERSUS`. The
/// `SpellModOp::CritDamageAndHealing` spellmod is not represented.
pub(crate) fn represented_weapon_spell_critical_damage_like_cpp(
    damage: u32,
    crit_pct_damage_mod: f32,
) -> u32 {
    let mut damage_i64 = i64::from(damage) + i64::from(damage);
    if crit_pct_damage_mod != 0.0 {
        damage_i64 += (damage_i64 as f64 * f64::from(crit_pct_damage_mod) / 100.0) as i64;
    }
    damage_i64.clamp(0, i64::from(u32::MAX)) as u32
}

/// The critical damage for one represented spell hit, by damage class: C++ runs
/// two different arms inside `Unit::CalculateSpellDamageTaken` (`:1266-1332`).
pub(crate) fn represented_spell_crit_damage_like_cpp(
    damage: u32,
    defense_type: i8,
    crit_mod_pct: f32,
) -> u32 {
    match defense_type {
        SPELL_DAMAGE_CLASS_MELEE_LIKE_CPP | SPELL_DAMAGE_CLASS_RANGED_LIKE_CPP => {
            represented_weapon_spell_critical_damage_like_cpp(damage, crit_mod_pct)
        }
        _ => represented_spell_critical_damage_bonus_like_cpp(damage, defense_type, crit_mod_pct),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done_facts() -> RepresentedSpellCritChanceDoneFactsLikeCpp {
        RepresentedSpellCritChanceDoneFactsLikeCpp {
            caster_can_crit_at_all: true,
            caster_is_player: true,
            victim_is_standing: true,
            spell_is_healing: false,
            spell_can_crit: true,
            defense_type: SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP,
            school_mask: 0x04, // SPELL_SCHOOL_MASK_FIRE
            player_spell_crit_pct: 12.5,
            unit_crit_chance_done_pct: 4.0,
            spell_crit_chance_school_aura_pct: 1.0,
            base_spell_crit_chance_pct: 5.0,
        }
    }

    fn taken_facts() -> RepresentedSpellCritChanceTakenFactsLikeCpp {
        RepresentedSpellCritChanceTakenFactsLikeCpp {
            spell_can_crit: true,
            defense_type: SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP,
            spell_is_positive: false,
            attacker_spell_crit_chance_aura_pct: 0.0,
            attacker_spell_and_weapon_crit_chance_aura_pct: 0.0,
            resilience_crit_taken_pct: 0.0,
            unit_crit_chance_taken_pct: 0.0,
            crit_chance_for_caster_aura_pct: 0.0,
        }
    }

    /// The three early returns, in C++ order, and the published per-school
    /// percentage a player's magic spell uses.
    #[test]
    fn crit_chance_done_follows_the_cpp_gates_and_branches() {
        let facts = done_facts();
        assert_eq!(represented_spell_crit_chance_done_like_cpp(&facts), 12.5);

        // A creature with no player owner cannot crit with a spell at all, and
        // that return comes before the sitting-target rule.
        let mut creature = facts;
        creature.caster_can_crit_at_all = false;
        creature.victim_is_standing = false;
        assert_eq!(represented_spell_crit_chance_done_like_cpp(&creature), 0.0);

        // A player's harmful spell on a sitting victim always crits, before the
        // CAN_CRIT gate is consulted.
        let mut sitting = facts;
        sitting.victim_is_standing = false;
        sitting.spell_can_crit = false;
        assert_eq!(represented_spell_crit_chance_done_like_cpp(&sitting), 100.0);
        // A heal on a sitting victim does not.
        sitting.spell_is_healing = true;
        assert_eq!(represented_spell_crit_chance_done_like_cpp(&sitting), 0.0);

        let mut cannot_crit = facts;
        cannot_crit.spell_can_crit = false;
        assert_eq!(
            represented_spell_crit_chance_done_like_cpp(&cannot_crit),
            0.0
        );
    }

    /// The magic branch takes the larger of the physical and magical chances when
    /// the school mask covers both, and a non-player caster reads
    /// `m_baseSpellCritChance` plus its school aura instead of the published
    /// percentages.
    #[test]
    fn crit_chance_done_takes_the_larger_school_branch_like_cpp() {
        let mut mixed = done_facts();
        mixed.school_mask = SPELL_SCHOOL_MASK_NORMAL_LIKE_CPP | 0x04;
        // physical = 4 + 1 = 5, magical = 12.5
        assert_eq!(represented_spell_crit_chance_done_like_cpp(&mixed), 12.5);
        mixed.player_spell_crit_pct = 2.0;
        assert_eq!(represented_spell_crit_chance_done_like_cpp(&mixed), 5.0);

        let mut pet = done_facts();
        pet.caster_is_player = false;
        assert_eq!(represented_spell_crit_chance_done_like_cpp(&pet), 6.0);

        // A melee-class spell adds both terms instead of choosing one.
        let mut weapon = done_facts();
        weapon.defense_type = SPELL_DAMAGE_CLASS_MELEE_LIKE_CPP;
        assert_eq!(represented_spell_crit_chance_done_like_cpp(&weapon), 5.0);
    }

    /// `SPELL_DAMAGE_CLASS_NONE` returns zero on the taken side, so such a spell
    /// never crits however large the done chance was.
    #[test]
    fn crit_chance_taken_refuses_damage_class_none_like_cpp() {
        let mut none = taken_facts();
        none.defense_type = SPELL_DAMAGE_CLASS_NONE_LIKE_CPP;
        assert_eq!(
            represented_spell_crit_chance_taken_like_cpp(&none, 80.0),
            0.0
        );
        assert_eq!(
            represented_spell_crit_chance_taken_like_cpp(&taken_facts(), 80.0),
            80.0
        );
    }

    /// The magic arm applies the victim's aura, then resilience, then the
    /// after-resilience aura, and only for a harmful spell.
    #[test]
    fn crit_chance_taken_applies_resilience_between_the_two_auras_like_cpp() {
        let mut facts = taken_facts();
        facts.attacker_spell_crit_chance_aura_pct = 5.0;
        facts.resilience_crit_taken_pct = 3.0;
        facts.attacker_spell_and_weapon_crit_chance_aura_pct = 1.0;
        assert_eq!(
            represented_spell_crit_chance_taken_like_cpp(&facts, 10.0),
            13.0
        );

        // A positive spell skips the whole block.
        facts.spell_is_positive = true;
        assert_eq!(
            represented_spell_crit_chance_taken_like_cpp(&facts, 10.0),
            10.0
        );

        // The caster-specific aura is added for every class, and the result never
        // goes below zero.
        let mut negative = taken_facts();
        negative.resilience_crit_taken_pct = 50.0;
        negative.crit_chance_for_caster_aura_pct = 2.0;
        assert_eq!(
            represented_spell_crit_chance_taken_like_cpp(&negative, 10.0),
            0.0
        );

        // A melee-class spell replaces the chance with the taken value.
        let mut weapon = taken_facts();
        weapon.defense_type = SPELL_DAMAGE_CLASS_RANGED_LIKE_CPP;
        weapon.unit_crit_chance_taken_pct = 7.5;
        assert_eq!(
            represented_spell_crit_chance_taken_like_cpp(&weapon, 10.0),
            7.5
        );

        let mut cannot_crit = taken_facts();
        cannot_crit.spell_can_crit = false;
        assert_eq!(
            represented_spell_crit_chance_taken_like_cpp(&cannot_crit, 10.0),
            0.0
        );
    }

    /// The heal arm computes the bonus alone, so the multipliers land differently
    /// from the damage arm.
    #[test]
    fn critical_healing_applies_its_two_multipliers_like_cpp() {
        assert_eq!(
            represented_spell_critical_healing_bonus_like_cpp(
                100,
                SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP,
                1.0,
                1.0
            ),
            150
        );
        assert_eq!(
            represented_spell_critical_healing_bonus_like_cpp(
                100,
                SPELL_DAMAGE_CLASS_MELEE_LIKE_CPP,
                1.0,
                1.0
            ),
            200
        );
        // `MOD_CRIT_PERCENT_VERSUS` multiplies the bonus only: 100 + 50 * 1.5.
        assert_eq!(
            represented_spell_critical_healing_bonus_like_cpp(
                100,
                SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP,
                1.5,
                1.0
            ),
            175
        );
        // `MOD_CRITICAL_HEALING_AMOUNT` multiplies the whole heal: (100 + 50) * 1.2.
        assert_eq!(
            represented_spell_critical_healing_bonus_like_cpp(
                100,
                SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP,
                1.0,
                1.2
            ),
            180
        );
        // A one-point heal gains nothing from the magical arm, because the bonus
        // truncates to zero and C++ adds it only when positive.
        assert_eq!(
            represented_spell_critical_healing_bonus_like_cpp(
                1,
                SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP,
                1.0,
                1.0
            ),
            1
        );
    }

    /// A magical critical adds half again; a weapon-based one doubles.
    #[test]
    fn crit_damage_uses_the_cpp_arm_for_its_damage_class() {
        assert_eq!(
            represented_spell_crit_damage_like_cpp(100, SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP, 0.0),
            150
        );
        assert_eq!(
            represented_spell_crit_damage_like_cpp(100, SPELL_DAMAGE_CLASS_NONE_LIKE_CPP, 0.0),
            150
        );
        assert_eq!(
            represented_spell_crit_damage_like_cpp(100, SPELL_DAMAGE_CLASS_MELEE_LIKE_CPP, 0.0),
            200
        );
        assert_eq!(
            represented_spell_crit_damage_like_cpp(100, SPELL_DAMAGE_CLASS_RANGED_LIKE_CPP, 0.0),
            200
        );

        // C++ `AddPct` applies the modifier to the whole critical value, not to
        // the bonus alone: 150 + 10% of 150.
        assert_eq!(
            represented_spell_crit_damage_like_cpp(100, SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP, 10.0),
            165
        );
        assert_eq!(
            represented_spell_crit_damage_like_cpp(100, SPELL_DAMAGE_CLASS_MELEE_LIKE_CPP, 10.0),
            220
        );

        // Odd damage truncates like C++ integer division.
        assert_eq!(
            represented_spell_crit_damage_like_cpp(101, SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP, 0.0),
            151
        );
        assert_eq!(
            represented_spell_crit_damage_like_cpp(0, SPELL_DAMAGE_CLASS_MAGIC_LIKE_CPP, 50.0),
            0
        );
    }
}
