//! The represented spell critical: resolving the facts C++ rolls one chance
//! from, and applying the critical arm to the damage.
//!
//! C++ rolls once per target in `Spell::PreprocessSpellLaunch`
//! (`Spells/Spell.cpp:8675-8684`) and applies the arm for the spell's damage
//! class in `Unit::CalculateSpellDamageTaken` (`Entities/Unit/Unit.cpp:1266-1332`).
//! The rules themselves are in `session_rules::rules_5`; this module only reads
//! the state they need.

use super::*;

/// One rolled spell critical for a direct-damage hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::session) struct RepresentedSpellCriticalLikeCpp {
    /// The damage after the critical arm, or the unchanged damage.
    pub damage: u32,
    /// C++ `SpellNonMeleeDamage::HitInfo`, which carries `SPELL_HIT_TYPE_CRIT`.
    pub hit_info: i32,
}

impl WorldSession {
    /// Roll C++'s one spell critical chance for this hit and apply its arm.
    ///
    /// Boundaries, each a fact this port does not carry rather than a choice:
    /// every aura term on both sides is zero, because the crit-chance aura
    /// families (`SPELL_AURA_MOD_SPELL_CRIT_CHANCE_SCHOOL`,
    /// `SPELL_AURA_MOD_ATTACKER_SPELL_CRIT_CHANCE`, the two crit-damage families
    /// and `SPELL_AURA_MOD_CRIT_PERCENT_VERSUS`) are not represented; resilience
    /// is zero for the same reason; `SpellInfo::IsPositive` is not represented,
    /// and this path is reached only by a damaging effect on a creature, so the
    /// spell is taken as harmful; and a creature victim is taken as standing,
    /// which is the stand state a represented creature has.
    pub(in crate::session) fn represented_spell_critical_for_damage_like_cpp(
        &self,
        spell_id: Option<i32>,
        caster_guid: ObjectGuid,
        caster_is_player_controlled: bool,
        school_mask: u32,
        damage: u32,
    ) -> RepresentedSpellCriticalLikeCpp {
        let unchanged = RepresentedSpellCriticalLikeCpp {
            damage,
            hit_info: 0,
        };
        // C++ reaches the critical roll only with a spell: a damage event with no
        // `SpellInfo` has no `DmgClass` and no `AttributesCu` to read.
        let Some(spell_id) = spell_id else {
            return unchanged;
        };
        let Some(spell_store) = self.spell_store().cloned() else {
            return unchanged;
        };
        let difficulty = self.current_map_difficulty_id_like_cpp();
        let difficulty_store = self.difficulty_store().cloned();
        let Some(metadata) = spell_store.hit_metadata_for_difficulty_like_cpp(
            spell_id,
            difficulty,
            difficulty_store.as_deref(),
        ) else {
            return unchanged;
        };
        let crit_chance = self.represented_spell_crit_chance_like_cpp(
            &spell_store,
            spell_id,
            difficulty,
            difficulty_store.as_deref(),
            caster_guid,
            caster_is_player_controlled,
            metadata.defense_type,
            u8::try_from(school_mask).unwrap_or(u8::MAX),
            false,
        );
        if !represented_spell_crit_roll_like_cpp(crit_chance) {
            return unchanged;
        }
        RepresentedSpellCriticalLikeCpp {
            damage: crate::session_rules::represented_spell_crit_damage_like_cpp(
                damage,
                metadata.defense_type,
                0.0,
            ),
            hit_info: crate::session_rules::SPELL_HIT_TYPE_CRIT_LIKE_CPP,
        }
    }
}

impl WorldSession {
    /// C++ `Spell::PreprocessSpellLaunch`'s one chance
    /// (`Spells/Spell.cpp:8675-8684`): `SpellCritChanceDone` on the caster, then
    /// `SpellCritChanceTaken` on the victim.
    #[allow(clippy::too_many_arguments)]
    fn represented_spell_crit_chance_like_cpp(
        &self,
        spell_store: &wow_data::SpellStore,
        spell_id: i32,
        difficulty: u8,
        difficulty_store: Option<&wow_data::DifficultyStore>,
        caster_guid: ObjectGuid,
        caster_is_player_controlled: bool,
        defense_type: i8,
        school_mask: u8,
        spell_is_healing: bool,
    ) -> f32 {
        let caster_is_player = caster_guid.is_player();
        let stats = caster_is_player
            .then(|| {
                self.canonical_player_snapshot_like_cpp(|player| {
                    let stats = player.effective_combat_stats_like_cpp();
                    (stats.crit_pct, stats.spell_crit_pct)
                })
            })
            .flatten();
        let (unit_crit_chance_done_pct, player_spell_crit_pct) = match stats {
            Some((crit_pct, spell_crit_pct)) => (
                crit_pct,
                represented_first_school_in_mask_crit_pct_like_cpp(school_mask, &spell_crit_pct),
            ),
            None => (0.0, 0.0),
        };
        let done = crate::session_rules::RepresentedSpellCritChanceDoneFactsLikeCpp {
            caster_can_crit_at_all: caster_is_player || caster_is_player_controlled,
            caster_is_player,
            victim_is_standing: true,
            spell_is_healing,
            spell_can_crit: spell_store.spell_can_crit_like_cpp(
                spell_id,
                difficulty,
                difficulty_store,
            ),
            defense_type,
            school_mask,
            player_spell_crit_pct,
            unit_crit_chance_done_pct,
            spell_crit_chance_school_aura_pct: 0.0,
            base_spell_crit_chance_pct: BASE_SPELL_CRIT_CHANCE_LIKE_CPP,
        };
        let done_chance = crate::session_rules::represented_spell_crit_chance_done_like_cpp(&done);
        let taken = crate::session_rules::RepresentedSpellCritChanceTakenFactsLikeCpp {
            spell_can_crit: done.spell_can_crit,
            defense_type,
            // C++ `SpellInfo::IsPositive` is not represented; a heal is positive
            // and a damage hit on a creature is not, which is what the two
            // callers pass.
            spell_is_positive: spell_is_healing,
            attacker_spell_crit_chance_aura_pct: 0.0,
            attacker_spell_and_weapon_crit_chance_aura_pct: 0.0,
            resilience_crit_taken_pct: 0.0,
            // C++ `GetUnitCriticalChanceTaken(caster, attackType, crit_chance)`
            // returns the done chance plus the victim's crit-taken auras, none of
            // which is represented.
            unit_crit_chance_taken_pct: done_chance,
            crit_chance_for_caster_aura_pct: 0.0,
        };
        crate::session_rules::represented_spell_crit_chance_taken_like_cpp(&taken, done_chance)
    }

    /// Roll C++'s one spell critical chance for a heal and apply
    /// `SpellCriticalHealingBonus` (`Entities/Unit/Unit.cpp:8005-8036`), which
    /// `Spell::DoAllEffectOnTarget` runs before `HealBySpell`
    /// (`Spells/Spell.cpp:2930-2940`) and therefore before the heal absorb.
    ///
    /// Boundaries are the same as the damage roll's, plus the two heal-only aura
    /// multipliers (`SPELL_AURA_MOD_CRIT_PERCENT_VERSUS` and
    /// `SPELL_AURA_MOD_CRITICAL_HEALING_AMOUNT`), both left at `1.0`.
    pub(in crate::session) fn represented_spell_critical_for_heal_like_cpp(
        &self,
        spell_id: Option<i32>,
        healer_guid: ObjectGuid,
        healer_is_player_controlled: bool,
        heal: u32,
    ) -> (u32, bool) {
        let Some(spell_id) = spell_id else {
            return (heal, false);
        };
        let Some(spell_store) = self.spell_store().cloned() else {
            return (heal, false);
        };
        let difficulty = self.current_map_difficulty_id_like_cpp();
        let difficulty_store = self.difficulty_store().cloned();
        let Some(metadata) = spell_store.hit_metadata_for_difficulty_like_cpp(
            spell_id,
            difficulty,
            difficulty_store.as_deref(),
        ) else {
            return (heal, false);
        };
        let chance = self.represented_spell_crit_chance_like_cpp(
            &spell_store,
            spell_id,
            difficulty,
            difficulty_store.as_deref(),
            healer_guid,
            healer_is_player_controlled,
            metadata.defense_type,
            metadata.school_mask,
            // C++ passes `spellInfo->IsHealingSpell()`, which keeps the
            // always-crit-sitting-target rule off a heal.
            true,
        );
        if !represented_spell_crit_roll_like_cpp(chance) {
            return (heal, false);
        }
        (
            crate::session_rules::represented_spell_critical_healing_bonus_like_cpp(
                heal,
                metadata.defense_type,
                1.0,
                1.0,
            ),
            true,
        )
    }
}

/// C++ `Unit::m_baseSpellCritChance` (`Entities/Unit/Unit.cpp:468`), read only
/// for a non-player caster.
const BASE_SPELL_CRIT_CHANCE_LIKE_CPP: f32 = 5.0;

/// C++ `GetFirstSchoolInMask` (`Miscellaneous/SharedDefines.h:377-387`) indexing
/// `ActivePlayerData::SpellCritPercentage`.
fn represented_first_school_in_mask_crit_pct_like_cpp(
    school_mask: u8,
    spell_crit_pct: &[f32; 7],
) -> f32 {
    for (school, percentage) in spell_crit_pct.iter().enumerate() {
        if school_mask & (1 << school) != 0 {
            return *percentage;
        }
    }
    // C++ returns `SPELL_SCHOOL_NORMAL` for an empty mask.
    spell_crit_pct[0]
}

/// C++ `roll_chance_f(critChance)` (`Spells/Spell.cpp:8683`), which is
/// `rand_chance() < chance` over `[0, 100)`. Pinnable so a scenario can assert
/// both outcomes of the same hit.
fn represented_spell_crit_roll_like_cpp(chance: f32) -> bool {
    #[cfg(test)]
    if let Some(pinned) = PINNED_SPELL_CRIT_ROLL_LIKE_CPP.with(|cell| cell.get()) {
        return pinned < chance;
    }
    wow_core::roll_chance_f_like_cpp(chance)
}

#[cfg(test)]
thread_local! {
    static PINNED_SPELL_CRIT_ROLL_LIKE_CPP: std::cell::Cell<Option<f32>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(in crate::session) struct PinnedSpellCritRollLikeCpp;

#[cfg(test)]
impl PinnedSpellCritRollLikeCpp {
    /// Pin C++'s `roll_chance_f` draw for this thread: the roll succeeds when the
    /// pinned draw is below the resolved chance, exactly like `rand_chance() <
    /// chance` with a known draw.
    pub(in crate::session) fn pin(draw: f32) -> Self {
        PINNED_SPELL_CRIT_ROLL_LIKE_CPP.with(|cell| cell.set(Some(draw)));
        Self
    }
}

#[cfg(test)]
impl Drop for PinnedSpellCritRollLikeCpp {
    fn drop(&mut self) {
        PINNED_SPELL_CRIT_ROLL_LIKE_CPP.with(|cell| cell.set(None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C++ `GetFirstSchoolInMask` takes the lowest set bit, and an empty mask
    /// falls back to the physical school.
    #[test]
    fn first_school_in_mask_picks_the_lowest_set_bit_like_cpp() {
        let percentages = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        assert_eq!(
            represented_first_school_in_mask_crit_pct_like_cpp(0b0000_0001, &percentages),
            1.0
        );
        assert_eq!(
            represented_first_school_in_mask_crit_pct_like_cpp(0b0000_0100, &percentages),
            3.0
        );
        assert_eq!(
            represented_first_school_in_mask_crit_pct_like_cpp(0b0010_1000, &percentages),
            4.0
        );
        assert_eq!(
            represented_first_school_in_mask_crit_pct_like_cpp(0, &percentages),
            1.0
        );
    }
}
