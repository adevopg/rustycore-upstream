//! The represented spell absorb: spending the victim's shields on a spell hit.
//!
//! C++ `Unit::CalcAbsorbResist` (`Entities/Unit/Unit.cpp:2080-2250`) runs the
//! resist first, then the attacker's ignore-absorb share leaves the damage, then
//! the `SPELL_AURA_SCHOOL_ABSORB` loop spends each shield in
//! `Trinity::AbsorbAuraOrderPred` order, publishing one
//! `SMSG_SPELL_ABSORB_LOG` per consuming shield and removing a spent one. The
//! arithmetic is in `session_rules::rules_4`; this module reads the state it
//! needs and commits the depletion on the canonical creature.

use super::*;

/// One resolved spell absorb for a direct-damage hit on a creature.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::session) struct RepresentedSpellAbsorbLikeCpp {
    /// The damage left after the shields, which is what the victim takes.
    pub damage: u32,
    /// C++ `SpellNonMeleeDamage::absorb`, published on the combat log.
    pub absorbed: u32,
}

/// One shield's publication, resolved inside the canonical mutation and sent
/// outside it.
struct SpellAbsorbPublicationLikeCpp {
    slot: u8,
    absorb_spell_id: i32,
    absorb_caster: ObjectGuid,
    consumed: i32,
    removed: bool,
}

impl WorldSession {
    /// Spend a creature victim's school-absorb shields on one spell hit.
    ///
    /// C++ order is kept: the caller has already applied the critical and the
    /// resist, this stage runs before `DealDamage`, and its
    /// `SMSG_SPELL_ABSORB_LOG` publications precede
    /// `SMSG_SPELL_NON_MELEE_DAMAGE_LOG` exactly as `CalcAbsorbResist` precedes
    /// `SendSpellNonMeleeDamageLog`.
    ///
    /// The canonical creature owns both the aura amount and its published slot,
    /// so the depletion, the removal and the slot update are committed here
    /// rather than handed to another owner. A hit that absorbs nothing leaves
    /// the creature untouched and publishes nothing, like C++'s `if
    /// (currentAbsorb)` guard.
    ///
    /// Boundaries, each a fact this port does not carry rather than a choice:
    ///
    /// * `SPELL_AURA_MANA_SHIELD` has no creature-side projection, so C++'s
    ///   second loop (`Unit.cpp:2179-2248`) is empty here. It needs a creature
    ///   power write and its publication, which no represented path owns; the
    ///   melee creature victim has the same gap.
    /// * the absorb scripts (`CallScriptEffectAbsorbHandlers` and its
    ///   `defaultPrevented` escape, `:2140-2144`) are not ported, so an
    ///   infinite-absorb shield is clamped to zero as C++ does for safety.
    /// * `SPELL_AURA_SPLIT_DAMAGE_FLAT`/`_PCT` (`:2252-2358`) stay with the
    ///   melee path that already represents them.
    pub(in crate::session) fn represented_spell_absorb_for_damage_like_cpp(
        &mut self,
        spell_id: Option<i32>,
        attacker_guid: ObjectGuid,
        victim_guid: ObjectGuid,
        school_mask: u32,
        // C++ `DamageInfo::GetDamage()` before `ResistDamage`, which is the
        // basis of `CalculatePct(damage, auraAbsorbMod)` (`Unit.cpp:2106`).
        damage_before_resist: u32,
        // C++ `DamageInfo::GetDamage()` after `ResistDamage` (`:2111`).
        damage: u32,
        // C++ `DamageInfo::GetOriginalDamage()`, published on each absorb log.
        original_damage: u32,
    ) -> RepresentedSpellAbsorbLikeCpp {
        let unchanged = RepresentedSpellAbsorbLikeCpp {
            damage,
            absorbed: 0,
        };
        // C++ `CalcAbsorbResist` returns before any shield for a hit with no
        // damage (`Unit.cpp:2082`).
        if damage == 0 {
            return unchanged;
        }
        let Some(spell_store) = self.spell_store().cloned() else {
            return unchanged;
        };
        let difficulty_id = self.current_map_difficulty_id_like_cpp();
        let difficulty_store = self.difficulty_store().cloned();
        let ignore_absorb_pct =
            self.represented_attacker_ignore_absorb_pct_like_cpp(attacker_guid, school_mask);

        let Some((absorbed, remaining, publications)) =
            self.mutate_creature_aura_owner_like_cpp(victim_guid, |creature| {
                let shields = crate::session_rules::creature_absorb_shields_like_cpp(
                    &creature.unit().subsystems().auras,
                    &spell_store,
                    difficulty_id,
                    difficulty_store.as_deref(),
                    school_mask,
                );
                if shields.is_empty() {
                    return (0, damage, Vec::new());
                }
                let absorb = crate::session_rules::represented_absorb_stages_like_cpp(
                    &shields,
                    &[],
                    damage_before_resist,
                    damage,
                    0,
                    ignore_absorb_pct,
                );
                let mut publications = Vec::with_capacity(absorb.school_consumed.len());
                for consumption in &absorb.school_consumed {
                    let Some(applied) = creature
                        .unit()
                        .subsystems()
                        .auras
                        .applied_auras
                        .iter()
                        .find(|aura| {
                            aura.slot == consumption.slot
                                && 1_u32
                                    .checked_shl(u32::from(consumption.effect_index))
                                    .is_some_and(|bit| aura.effect_mask & bit != 0)
                        })
                        .copied()
                    else {
                        continue;
                    };
                    publications.push(SpellAbsorbPublicationLikeCpp {
                        slot: applied.slot,
                        absorb_spell_id: i32::try_from(applied.spell_id).unwrap_or(i32::MAX),
                        absorb_caster: applied.caster_guid,
                        consumed: consumption.consumed,
                        removed: consumption.removed,
                    });
                    if consumption.removed {
                        // C++ `absorbAurEff->GetBase()->Remove(AURA_REMOVE_BY_ENEMY_SPELL)`
                        // removes the whole aura, so every slot its application
                        // covers goes with it.
                        let aura_ref = applied.aura_ref();
                        let covered: Vec<_> = creature
                            .unit()
                            .subsystems()
                            .auras
                            .applied_auras
                            .iter()
                            .filter(|candidate| candidate.aura_ref() == aura_ref)
                            .copied()
                            .collect();
                        let auras = &mut creature.unit_mut().subsystems_mut().auras;
                        for covered in covered {
                            auras.unapply_aura(covered, 0);
                        }
                        let _ = auras.clear_visible(applied.slot);
                    } else if let Some(amount) = creature
                        .unit_mut()
                        .subsystems_mut()
                        .auras
                        .applied_aura_amounts
                        .get_mut(&applied)
                    {
                        // C++ `absorbAurEff->ChangeAmount(GetAmount() - currentAbsorb)`.
                        *amount = consumption.remaining.max(0);
                    }
                }
                (absorb.absorbed, absorb.damage, publications)
            })
        else {
            return unchanged;
        };

        // C++ publishes each shield's log inside the loop, before `DealDamage`
        // and therefore before `SendSpellNonMeleeDamageLog`
        // (`Unit.cpp:2166-2176`). `SendCombatLogMessage` reaches the victim's
        // whole visible set.
        for publication in &publications {
            if publication.consumed > 0 {
                let packet = wow_packet::packets::combat::SpellAbsorbLog {
                    attacker: attacker_guid,
                    victim: victim_guid,
                    absorbed_spell_id: spell_id.unwrap_or(0),
                    absorb_spell_id: publication.absorb_spell_id,
                    caster: publication.absorb_caster,
                    absorbed: publication.consumed,
                    original_damage: i32::try_from(original_damage).unwrap_or(i32::MAX),
                };
                self.send_packet(&packet);
                self.broadcast_creature_packet_to_visible_set_like_cpp(
                    victim_guid,
                    wow_packet::ServerPacket::to_bytes(&packet),
                );
            }
            if publication.removed {
                self.publish_creature_aura_slot_update_like_cpp(
                    victim_guid,
                    publication.slot,
                    false,
                );
            }
        }

        RepresentedSpellAbsorbLikeCpp {
            damage: remaining,
            absorbed,
        }
    }

    /// C++ `Unit::CalcAbsorbResist`'s `auraAbsorbMod` (`Unit.cpp:2086-2104`) for
    /// the hit's attacker, whichever side of the represented split it lives on.
    ///
    /// Boundary: `SPELL_AURA_MOD_TARGET_ABILITY_ABSORB_SCHOOL`'s
    /// `IsAffectingSpell` arm (`:2092-2101`) has no represented spell-family
    /// projection, so only the school-wide family is read.
    fn represented_attacker_ignore_absorb_pct_like_cpp(
        &mut self,
        attacker_guid: ObjectGuid,
        school_mask: u32,
    ) -> f32 {
        let Some(spell_store) = self.spell_store().cloned() else {
            return 0.0;
        };
        let effects = if attacker_guid.is_player() {
            let Some(auras) = self.canonical_player_snapshot_like_cpp(|player| {
                player
                    .unit()
                    .subsystems()
                    .auras
                    .runtime_applications_like_cpp()
                    .clone()
            }) else {
                return 0.0;
            };
            crate::session_rules::player_aura_effects_full_by_spell_aura_type_like_cpp(
                &auras,
                &spell_store,
                wow_data::spell::aura_types::SPELL_AURA_MOD_TARGET_ABSORB_SCHOOL,
            )
            .iter()
            .map(|effect| effect.as_applied_like_cpp())
            .collect()
        } else {
            let difficulty_id = self.current_map_difficulty_id_like_cpp();
            let difficulty_store = self.difficulty_store().cloned();
            let Some(applied) = self
                .mutate_canonical_creature_by_guid_like_cpp(attacker_guid, |creature| {
                    creature.unit().subsystems().auras.applied_auras.clone()
                })
            else {
                return 0.0;
            };
            crate::session_rules::creature_aura_effects_like_cpp(
                &applied,
                &spell_store,
                difficulty_id,
                difficulty_store.as_deref(),
            )
        };
        crate::session_rules::represented_ignore_absorb_pct_like_cpp(&effects, school_mask)
    }
}
