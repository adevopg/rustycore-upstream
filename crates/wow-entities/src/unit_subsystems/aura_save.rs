// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! C++ `Player::_SaveAuras` and the `Aura::CanBeSaved` filter in front of it.

use wow_core::ObjectGuid;

/// One `AuraEffect` row, as C++ `_SaveAuras` writes it
/// (`Entities/Player/Player.cpp:20128-20143`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerAuraEffectSaveLikeCpp {
    pub effect_index: u8,
    pub amount: i32,
    pub base_amount: i32,
    /// C++ `AuraEffect::CanBeRecalculated`, read by `Aura::GenerateKey` to build
    /// the stored `recalculateMask`.
    pub can_be_recalculated: bool,
}

/// One owned aura offered to the save, with the facts C++ `Aura::CanBeSaved`
/// consults (`Spells/Auras/SpellAuras.cpp:1172-1209`) beside the row values.
///
/// The spell-info predicates are inputs rather than lookups: the aura subsystem
/// has no spell store, and C++ reads them off `GetSpellInfo()` at the same point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerAuraSaveCandidateLikeCpp {
    pub spell_id: u32,
    pub caster_guid: ObjectGuid,
    /// C++ `AuraKey::Item`; empty unless the aura was cast by an item.
    pub item_caster_guid: ObjectGuid,
    pub difficulty: u8,
    pub stack_count: u8,
    pub max_duration_ms: i32,
    pub duration_ms: i32,
    pub charges: u8,
    pub cast_item_id: u32,
    pub cast_item_level: i32,
    /// C++ `Aura::IsPassive`.
    pub is_passive: bool,
    /// C++ `SpellInfo::IsChanneled`.
    pub is_channeled: bool,
    /// C++ `Aura::IsSingleTarget() || GetSpellInfo()->IsSingleTarget()`.
    pub is_single_target: bool,
    /// Whether any real effect `IsTargetingArea()` or `IsAreaAuraEffect()`.
    pub has_area_effect: bool,
    /// C++ `SPELL_ATTR0_CU_AURA_CANNOT_BE_SAVED`.
    pub cannot_be_saved_attribute: bool,
    /// C++ `Aura::IsUsingCharges`.
    pub uses_charges: bool,
    /// C++ `Aura::IsPermanent`.
    pub is_permanent: bool,
    pub effects: Vec<PlayerAuraEffectSaveLikeCpp>,
}

/// C++ `AuraKey` masks, which `Aura::GenerateKey`
/// (`Spells/Auras/SpellAuras.cpp:1262-1281`) derives from the live effect list
/// rather than from anything stored. They are computed here for the same reason:
/// a caller cannot hand in a mask that disagrees with the effect rows it also
/// hands in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayerAuraSaveKeyMasksLikeCpp {
    pub effect_mask: u32,
    pub recalculate_mask: u32,
}

impl PlayerAuraSaveCandidateLikeCpp {
    /// C++ `Aura::GenerateKey` (`Spells/Auras/SpellAuras.cpp:1262-1281`).
    pub fn generate_key_masks_like_cpp(&self) -> PlayerAuraSaveKeyMasksLikeCpp {
        let mut masks = PlayerAuraSaveKeyMasksLikeCpp::default();
        for effect in &self.effects {
            masks.effect_mask |= 1 << effect.effect_index;
            if effect.can_be_recalculated {
                masks.recalculate_mask |= 1 << effect.effect_index;
            }
        }
        masks
    }

    /// C++ `Aura::CanBeSaved` (`Spells/Auras/SpellAuras.cpp:1172-1209`), in its
    /// own order. `owner_guid` is the aura's owner, which C++ compares against
    /// the caster before it looks at the area and single-target questions at all.
    pub fn can_be_saved_like_cpp(&self, owner_guid: ObjectGuid) -> bool {
        if self.is_passive || self.is_channeled {
            return false;
        }
        if self.caster_guid != owner_guid && (self.has_area_effect || self.is_single_target) {
            return false;
        }
        if self.cannot_be_saved_attribute {
            return false;
        }
        // C++: an aura the proc system has exhausted is not written back.
        if self.uses_charges && self.charges == 0 {
            return false;
        }
        // C++: a permanent item-triggered aura is recast on login if needed.
        if !self.item_caster_guid.is_empty() && self.is_permanent {
            return false;
        }
        true
    }
}

/// One statement C++ `_SaveAuras` appends, in the order it appends them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerAuraSaveOperationLikeCpp {
    DeleteAuraEffects,
    DeleteAuras,
    InsertAura {
        caster_guid: ObjectGuid,
        item_guid: ObjectGuid,
        spell_id: u32,
        effect_mask: u32,
        recalculate_mask: u32,
        difficulty: u8,
        stack_count: u8,
        max_duration_ms: i32,
        duration_ms: i32,
        charges: u8,
        cast_item_id: u32,
        cast_item_level: i32,
    },
    InsertAuraEffect {
        caster_guid: ObjectGuid,
        item_guid: ObjectGuid,
        spell_id: u32,
        effect_mask: u32,
        effect_index: u8,
        amount: i32,
        base_amount: i32,
    },
}

/// C++ `Player::_SaveAuras` (`Entities/Player/Player.cpp:20089-20146`): both
/// deletes first, then one aura row per saveable owned aura followed by its own
/// effect rows.
pub fn player_save_auras_plan_like_cpp(
    owner_guid: ObjectGuid,
    candidates: &[PlayerAuraSaveCandidateLikeCpp],
) -> Vec<PlayerAuraSaveOperationLikeCpp> {
    let mut operations = vec![
        PlayerAuraSaveOperationLikeCpp::DeleteAuraEffects,
        PlayerAuraSaveOperationLikeCpp::DeleteAuras,
    ];

    for candidate in candidates {
        if !candidate.can_be_saved_like_cpp(owner_guid) {
            continue;
        }
        let masks = candidate.generate_key_masks_like_cpp();
        operations.push(PlayerAuraSaveOperationLikeCpp::InsertAura {
            caster_guid: candidate.caster_guid,
            item_guid: candidate.item_caster_guid,
            spell_id: candidate.spell_id,
            effect_mask: masks.effect_mask,
            recalculate_mask: masks.recalculate_mask,
            difficulty: candidate.difficulty,
            stack_count: candidate.stack_count,
            max_duration_ms: candidate.max_duration_ms,
            duration_ms: candidate.duration_ms,
            charges: candidate.charges,
            cast_item_id: candidate.cast_item_id,
            cast_item_level: candidate.cast_item_level,
        });
        for effect in &candidate.effects {
            operations.push(PlayerAuraSaveOperationLikeCpp::InsertAuraEffect {
                caster_guid: candidate.caster_guid,
                item_guid: candidate.item_caster_guid,
                spell_id: candidate.spell_id,
                effect_mask: masks.effect_mask,
                effect_index: effect.effect_index,
                amount: effect.amount,
                base_amount: effect.base_amount,
            });
        }
    }

    operations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(spell_id: u32, caster: ObjectGuid) -> PlayerAuraSaveCandidateLikeCpp {
        PlayerAuraSaveCandidateLikeCpp {
            spell_id,
            caster_guid: caster,
            item_caster_guid: ObjectGuid::EMPTY,
            difficulty: 0,
            stack_count: 1,
            max_duration_ms: 60_000,
            duration_ms: 42_000,
            charges: 0,
            cast_item_id: 0,
            cast_item_level: 0,
            is_passive: false,
            is_channeled: false,
            is_single_target: false,
            has_area_effect: false,
            cannot_be_saved_attribute: false,
            uses_charges: false,
            is_permanent: false,
            effects: vec![
                PlayerAuraEffectSaveLikeCpp {
                    effect_index: 0,
                    amount: 7,
                    base_amount: 5,
                    can_be_recalculated: false,
                },
                PlayerAuraEffectSaveLikeCpp {
                    effect_index: 2,
                    amount: -3,
                    base_amount: -3,
                    can_be_recalculated: true,
                },
            ],
        }
    }

    fn owner() -> ObjectGuid {
        ObjectGuid::create_player(1, 0xA1)
    }

    /// C++ `Aura::CanBeSaved` refuses on each of its own gates and on nothing else.
    #[test]
    fn can_be_saved_follows_every_cpp_gate() {
        let owner_guid = owner();
        assert!(candidate(101, owner_guid).can_be_saved_like_cpp(owner_guid));

        let mut passive = candidate(101, owner_guid);
        passive.is_passive = true;
        assert!(!passive.can_be_saved_like_cpp(owner_guid));

        let mut channeled = candidate(101, owner_guid);
        channeled.is_channeled = true;
        assert!(!channeled.can_be_saved_like_cpp(owner_guid));

        let mut attribute = candidate(101, owner_guid);
        attribute.cannot_be_saved_attribute = true;
        assert!(!attribute.can_be_saved_like_cpp(owner_guid));

        // An aura the proc system has used up is not written back; one that does
        // not use charges at all is unaffected by holding none.
        let mut exhausted = candidate(101, owner_guid);
        exhausted.uses_charges = true;
        exhausted.charges = 0;
        assert!(!exhausted.can_be_saved_like_cpp(owner_guid));
        exhausted.charges = 1;
        assert!(exhausted.can_be_saved_like_cpp(owner_guid));

        // A permanent item-triggered aura is recast on login instead.
        let mut from_item = candidate(101, owner_guid);
        from_item.item_caster_guid = ObjectGuid::create_item(1, 55);
        from_item.is_permanent = true;
        assert!(!from_item.can_be_saved_like_cpp(owner_guid));
        from_item.is_permanent = false;
        assert!(from_item.can_be_saved_like_cpp(owner_guid));
    }

    /// The area and single-target gates apply only when the caster is someone
    /// else: C++ checks `GetCasterGUID() != GetOwner()->GetGUID()` first, because
    /// an area aura's owner *is* its caster.
    #[test]
    fn area_and_single_target_gates_only_apply_to_a_foreign_caster_like_cpp() {
        let owner_guid = owner();
        let stranger = ObjectGuid::create_player(1, 0xB2);

        for mutate in [
            |c: &mut PlayerAuraSaveCandidateLikeCpp| c.has_area_effect = true,
            |c: &mut PlayerAuraSaveCandidateLikeCpp| c.is_single_target = true,
        ] {
            let mut own = candidate(101, owner_guid);
            mutate(&mut own);
            assert!(
                own.can_be_saved_like_cpp(owner_guid),
                "the owner's own aura is kept"
            );

            let mut foreign = candidate(101, stranger);
            mutate(&mut foreign);
            assert!(
                !foreign.can_be_saved_like_cpp(owner_guid),
                "another caster's area or single-target aura is not"
            );
        }
    }

    /// Both deletes first, then each kept aura followed by its own effects.
    #[test]
    fn the_plan_deletes_before_it_inserts_like_cpp() {
        let owner_guid = owner();
        let mut skipped = candidate(202, owner_guid);
        skipped.is_passive = true;
        let plan =
            player_save_auras_plan_like_cpp(owner_guid, &[candidate(101, owner_guid), skipped]);

        assert_eq!(plan[0], PlayerAuraSaveOperationLikeCpp::DeleteAuraEffects);
        assert_eq!(plan[1], PlayerAuraSaveOperationLikeCpp::DeleteAuras);
        assert_eq!(plan.len(), 5, "two deletes, one aura, its two effects");
        assert!(matches!(
            plan[2],
            PlayerAuraSaveOperationLikeCpp::InsertAura {
                spell_id: 101,
                effect_mask: 0b101,
                recalculate_mask: 0b100,
                stack_count: 1,
                max_duration_ms: 60_000,
                duration_ms: 42_000,
                ..
            }
        ));
        assert!(matches!(
            plan[3],
            PlayerAuraSaveOperationLikeCpp::InsertAuraEffect {
                spell_id: 101,
                effect_index: 0,
                amount: 7,
                base_amount: 5,
                ..
            }
        ));
        assert!(matches!(
            plan[4],
            PlayerAuraSaveOperationLikeCpp::InsertAuraEffect {
                effect_index: 2,
                amount: -3,
                ..
            }
        ));
    }

    /// With nothing saveable the plan is still the two deletes: C++ appends them
    /// before it looks at the aura map at all, so a cleared aura list clears the
    /// rows too.
    #[test]
    fn an_empty_aura_list_still_clears_the_rows_like_cpp() {
        assert_eq!(
            player_save_auras_plan_like_cpp(owner(), &[]),
            vec![
                PlayerAuraSaveOperationLikeCpp::DeleteAuraEffects,
                PlayerAuraSaveOperationLikeCpp::DeleteAuras,
            ]
        );
    }
}
