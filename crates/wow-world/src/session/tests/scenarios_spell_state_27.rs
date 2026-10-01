//! Session scenarios for the Player half of C++ `Player::_SaveAuras`.

use super::*;

/// One `SpellInfo` with the attribute words and single effect the save reads.
fn aura_save_spell_like_cpp(
    spell_id: i32,
    effect_base_points: i32,
    implicit_target_1: u32,
    effect: u32,
) -> SpellInfo {
    SpellInfo {
        spell_id,
        cast_time_ms: 0,
        cooldown_ms: 0,
        recovery_time_ms: 0,
        effect_type: effect,
        effect_base_points,
        effect_bonus_coefficient: 0.0,
        aura_type: Some(wow_data::spell::aura_types::SPELL_AURA_MOD_DAMAGE_DONE),
        display_flags: 0,
        requires_spell_focus: 0,
        power_costs: Vec::new(),
        effects: vec![wow_data::SpellEffectInfo {
            effect_index: 0,
            effect,
            effect_aura: wow_data::spell::aura_types::SPELL_AURA_MOD_DAMAGE_DONE,
            effect_base_points,
            implicit_target_1,
            ..Default::default()
        }],
    }
}

fn aura_save_session_like_cpp() -> (WorldSession, ObjectGuid) {
    let (mut session, _, _) = make_session();
    let player_guid = ObjectGuid::create_player(1, 77);
    session.set_player_guid(Some(player_guid));

    let mut spell_store = SpellStore::new();
    // 101: an ordinary single-target buff.
    spell_store.insert(
        101,
        aura_save_spell_like_cpp(
            101,
            44,
            0,
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
        ),
    );
    // 202: passive.
    spell_store.insert(
        202,
        aura_save_spell_like_cpp(
            202,
            0,
            0,
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
        ),
    );
    spell_store.insert_spell_misc_attributes_like_cpp(202, {
        let mut attributes = [0u32; 15];
        attributes[0] = wow_data::spell::attributes::SPELL_ATTR0_PASSIVE;
        attributes
    });
    // 303: channeled.
    spell_store.insert(
        303,
        aura_save_spell_like_cpp(
            303,
            0,
            0,
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
        ),
    );
    spell_store.insert_spell_misc_attributes_like_cpp(303, {
        let mut attributes = [0u32; 15];
        attributes[1] = wow_data::spell::attributes::SPELL_ATTR1_IS_CHANNELLED;
        attributes
    });
    // 404: an area-aura effect, saveable from its own caster only.
    spell_store.insert(
        404,
        aura_save_spell_like_cpp(
            404,
            0,
            0,
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AREA_AURA_PARTY,
        ),
    );
    // 505: a single-effect aura whose TargetA is an area target.
    spell_store.insert(
        505,
        aura_save_spell_like_cpp(
            505,
            0,
            33, // TARGET_UNIT_SRC_AREA_PARTY
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
        ),
    );
    // 606: SPELL_ATTR5_LIMIT_N, which C++ `SpellInfo::IsSingleTarget` reads.
    spell_store.insert(
        606,
        aura_save_spell_like_cpp(
            606,
            0,
            0,
            wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
        ),
    );
    spell_store.insert_spell_misc_attributes_like_cpp(606, {
        let mut attributes = [0u32; 15];
        attributes[5] = wow_data::spell::attributes::SPELL_ATTR5_LIMIT_N;
        attributes
    });
    // 707: an aura type `SpellMgr::LoadSpellInfoCustomAttributes` marks as one
    // that cannot be saved.
    let mut charm = aura_save_spell_like_cpp(
        707,
        0,
        0,
        wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
    );
    charm.effects[0].effect_aura = wow_data::spell::aura_types::SPELL_AURA_MOD_CHARM;
    spell_store.insert(707, charm);
    session.set_spell_store(Arc::new(spell_store));
    (session, player_guid)
}

/// The save row carries the live duration, stack and effect values, and the
/// effect's `BasePoints` as the base amount C++ `AuraEffect` holds for an aura
/// that was not restored with a stored one (`SpellAuraEffects.cpp:620`).
#[test]
fn player_aura_save_rows_carry_the_live_aura_values_like_cpp() {
    let (mut session, player_guid) = aura_save_session_like_cpp();
    let mut aura = test_visible_aura(0, 101);
    aura.caster_guid = player_guid;
    aura.duration_total = 60_000;
    aura.duration_remaining = 42_000;
    aura.stack_count = 3;
    aura.represented_effect_amounts = vec![RepresentedAuraEffectAmountLikeCpp {
        effect_index: 0,
        amount: 7,
    }];
    session.visible_auras.insert(0, aura);

    let rows = session
        .player_aura_save_rows_like_cpp()
        .expect("the live aura state is readable");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.spell_id, 101);
    assert_eq!(row.caster_guid_binary, player_guid.to_raw_bytes().to_vec());
    assert_eq!(
        row.item_guid_binary,
        ObjectGuid::EMPTY.to_raw_bytes().to_vec()
    );
    assert_eq!(row.stack_count, 3);
    assert_eq!(row.max_duration_ms, 60_000);
    assert_eq!(row.remain_time_ms, 42_000);
    // C++ `Aura::GenerateKey` derives both masks from the live effects, and a
    // runtime `AuraEffect` starts recalculable.
    assert_eq!(row.effect_mask, 0b1);
    assert_eq!(row.recalculate_mask, 0b1);
    assert_eq!(row.effects.len(), 1);
    assert_eq!(row.effects[0].effect_index, 0);
    assert_eq!(row.effects[0].amount, 7);
    assert_eq!(row.effects[0].base_amount, 44);
}

/// C++ writes `-1` for a permanent aura's duration, which is the marker
/// `_LoadAuras` reads back (`session/mod.rs:1586-1602`).
#[test]
fn a_permanent_player_aura_is_stored_with_the_cpp_minus_one_duration_like_cpp() {
    let (mut session, player_guid) = aura_save_session_like_cpp();
    let mut aura = test_visible_aura(0, 101);
    aura.caster_guid = player_guid;
    aura.duration_total = 0;
    aura.duration_remaining = 0;
    session.visible_auras.insert(0, aura);

    let rows = session
        .player_aura_save_rows_like_cpp()
        .expect("the live aura state is readable");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].max_duration_ms, -1);
    assert_eq!(rows[0].remain_time_ms, -1);
}

/// `Aura::CanBeSaved` (`SpellAuras.cpp:1172-1209`) refuses a passive or
/// channeled aura, and one whose spell carries the cannot-be-saved custom
/// attribute, whoever cast it.
#[test]
fn passive_channeled_and_unsaveable_player_auras_are_skipped_like_cpp() {
    let (mut session, player_guid) = aura_save_session_like_cpp();
    for (slot, spell_id) in [(0u8, 101i32), (1, 202), (2, 303), (3, 707)] {
        let mut aura = test_visible_aura(slot, spell_id);
        aura.caster_guid = player_guid;
        session.visible_auras.insert(slot, aura);
    }

    let rows = session
        .player_aura_save_rows_like_cpp()
        .expect("the live aura state is readable");
    assert_eq!(
        rows.iter().map(|row| row.spell_id).collect::<Vec<_>>(),
        vec![101]
    );
}

/// The area and single-target gates fire only for a foreign caster: C++ checks
/// `GetCasterGUID() != GetOwner()->GetGUID()` before it looks at the effects.
#[test]
fn a_foreign_casters_area_or_single_target_aura_is_skipped_like_cpp() {
    let stranger = ObjectGuid::create_player(1, 0xB2);
    for spell_id in [404i32, 505, 606] {
        let (mut session, player_guid) = aura_save_session_like_cpp();
        let mut own = test_visible_aura(0, spell_id);
        own.caster_guid = player_guid;
        session.visible_auras.insert(0, own);
        assert_eq!(
            session
                .player_aura_save_rows_like_cpp()
                .expect("the live aura state is readable")
                .len(),
            1,
            "the owner's own aura {spell_id} is saved"
        );

        let (mut session, _) = aura_save_session_like_cpp();
        let mut foreign = test_visible_aura(0, spell_id);
        foreign.caster_guid = stranger;
        session.visible_auras.insert(0, foreign);
        assert!(
            session
                .player_aura_save_rows_like_cpp()
                .expect("the live aura state is readable")
                .is_empty(),
            "another caster's aura {spell_id} is not"
        );
    }
}

/// A foreign caster's ordinary buff is saved: none of the gates apply to it.
#[test]
fn a_foreign_casters_ordinary_buff_is_saved_like_cpp() {
    let (mut session, _) = aura_save_session_like_cpp();
    let mut aura = test_visible_aura(0, 101);
    aura.caster_guid = ObjectGuid::create_player(1, 0xB2);
    session.visible_auras.insert(0, aura);

    let rows = session
        .player_aura_save_rows_like_cpp()
        .expect("the live aura state is readable");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].caster_guid_binary,
        ObjectGuid::create_player(1, 0xB2).to_raw_bytes().to_vec()
    );
}

/// With every aura gone the group is still present and empty, so the save
/// clears the stored rows instead of leaving the last login's auras behind.
#[test]
fn a_player_without_auras_still_reports_an_empty_group_like_cpp() {
    let (session, _) = aura_save_session_like_cpp();
    assert_eq!(
        session.player_aura_save_rows_like_cpp(),
        Some(Vec::new()),
        "an empty group is not a missing group"
    );
}

/// A player-cast magical spell of 100 base damage at a 500-HP creature, with the
/// caster's published fire spell-crit percentage set so the roll has something to
/// compare against.
async fn spell_crit_scenario_like_cpp(
    creature_id: i64,
    // `(spell id, amount, MiscValue school mask)` of a
    // `SPELL_AURA_SCHOOL_ABSORB` effect to apply to the victim, for the
    // scenarios that exercise `CalcAbsorbResist`'s shield loop.
    shield: Option<(i32, i32, i32)>,
) -> (
    WorldSession,
    crate::map_manager::SharedMapManager,
    ObjectGuid,
    i32,
    flume::Receiver<Vec<u8>>,
) {
    let (mut session, _, send_rx) = make_session();
    let manager = shared_map_manager();
    let canonical = shared_canonical_map_manager();
    let creature_guid = test_creature_guid(creature_id);
    let player_guid = ObjectGuid::create_player(1, 91);
    let spell_id = 133_i32; // any id; the fixture below supplies its metadata

    // The published spell-crit percentages live on the canonical Player, so the
    // roll needs one installed rather than a bare session guid.
    canonical.lock().unwrap().create_world_map(0, 0);
    session.set_canonical_map_manager(Arc::clone(&canonical));
    session.set_map_store(Arc::new(wow_data::MapStore::from_entries([
        wow_data::MapEntry {
            id: 0,
            instance_type: wow_data::map::MAP_COMMON,
            expansion_id: 0,
            parent_map_id: -1,
            cosmetic_parent_map_id: -1,
            flags1: 0,
            flags2: 0,
        },
    ])));
    session.attach_player_controller_like_cpp(SessionPlayerController::new(
        player_guid,
        "Critter".to_string(),
        Position::new(10.0, 10.0, 0.0, 0.0),
        0,
        1,
        1,
        20,
        0,
    ));
    let _ = session.ensure_canonical_world_map_for_current_player_like_cpp();
    register_test_creature(&mut session, manager.clone(), creature_guid, 500);

    let mut spell_store = wow_data::SpellStore::new();
    spell_store.insert(
        spell_id,
        wow_data::SpellInfo {
            spell_id,
            cast_time_ms: 0,
            cooldown_ms: 0,
            recovery_time_ms: 0,
            effect_type: wow_data::spell::spell_effect_types::SPELL_EFFECT_SCHOOL_DAMAGE,
            effect_base_points: 0,
            effect_bonus_coefficient: 0.0,
            aura_type: None,
            display_flags: 0,
            requires_spell_focus: 0,
            power_costs: Vec::new(),
            // C++ `SPELL_ATTR0_CU_CAN_CRIT` comes from the effect list
            // (`Spells/SpellMgr.cpp:3367-3381`).
            effects: vec![wow_data::SpellEffectInfo {
                effect_index: 0,
                effect: wow_data::spell::spell_effect_types::SPELL_EFFECT_SCHOOL_DAMAGE,
                ..Default::default()
            }],
        },
    );
    // `SpellInfo::DmgClass` is `SpellCategories::DefenseType`; 1 is
    // `SPELL_DAMAGE_CLASS_MAGIC`, and the school mask is fire.
    spell_store.insert_spell_hit_metadata_for_difficulty_like_cpp(
        spell_id,
        0,
        wow_data::SpellHitMetadataLikeCpp {
            category_id: 0,
            charge_category_id: 0,
            defense_type: 1,
            spell_mechanic: 0,
            school_mask: 0x04,
            effect_mechanics: BTreeMap::from([(0, 0)]),
        },
    );
    if let Some((shield_spell_id, amount, shield_school_mask)) = shield {
        spell_store.insert(
            shield_spell_id,
            wow_data::SpellInfo {
                spell_id: shield_spell_id,
                cast_time_ms: 0,
                cooldown_ms: 0,
                recovery_time_ms: 0,
                effect_type: wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
                effect_base_points: amount,
                effect_bonus_coefficient: 0.0,
                aura_type: Some(wow_data::spell::aura_types::SPELL_AURA_SCHOOL_ABSORB),
                display_flags: 0,
                requires_spell_focus: 0,
                power_costs: Vec::new(),
                effects: vec![wow_data::SpellEffectInfo {
                    effect_index: 0,
                    effect: wow_data::spell::spell_effect_types::SPELL_EFFECT_APPLY_AURA,
                    effect_aura: wow_data::spell::aura_types::SPELL_AURA_SCHOOL_ABSORB,
                    effect_base_points: amount,
                    // C++ `absorbAurEff->GetMiscValue()`: the schools the shield
                    // covers, matched against the hit's school mask.
                    effect_misc_value_1: shield_school_mask,
                    effect_misc_value_2: 0,
                    ..Default::default()
                }],
            },
        );
    }
    session.set_spell_store(Arc::new(spell_store));
    // `SpellInfo::GetSchoolMask()` comes from `SpellMisc`, which is the reader the
    // damage path passes to the critical roll.
    session.set_spell_misc_store(Arc::new(wow_data::SpellMiscStore::from_entries([
        wow_data::SpellMiscEntry {
            id: spell_id as u32,
            attributes: [0; 15],
            difficulty_id: 0,
            casting_time_index: 0,
            duration_index: 0,
            range_index: 0,
            school_mask: 0x04,
            speed: 0.0,
            launch_delay: 0.0,
            min_duration: 0.0,
            spell_icon_file_data_id: 0,
            active_icon_file_data_id: 0,
            content_tuning_id: 0,
            show_future_spell_player_condition_id: 0,
            spell_id: spell_id as u32,
        },
    ])));

    session
        .mutate_canonical_player_like_cpp(|player| {
            let mut stats = *player.effective_combat_stats_like_cpp();
            // `ActivePlayerData::SpellCritPercentage[SPELL_SCHOOL_FIRE]`.
            stats.spell_crit_pct[2] = 25.0;
            player.replace_effective_combat_stats_like_cpp(stats);
        })
        .expect("the canonical Player must own the published percentages");

    if let Some((shield_spell_id, _, _)) = shield {
        session
            .apply_creature_aura_like_cpp(shield_spell_id, player_guid, creature_guid, 1, 60_000)
            .expect("the victim's shield must apply");
    }

    (session, manager, creature_guid, spell_id, send_rx)
}

/// The one `SMSG_SPELL_NON_MELEE_DAMAGE_LOG` the hit published, as
/// `(damage, original_damage, resisted, flags)`.
fn spell_non_melee_damage_log_values_like_cpp(packets: &[Vec<u8>]) -> (i32, i32, i32, u32) {
    let bytes = packets
        .iter()
        .find(|bytes| {
            wow_packet::WorldPacket::from_bytes(bytes).server_opcode()
                == Some(ServerOpcodes::SpellNonMeleeDamageLog)
        })
        .expect("the hit must publish one SMSG_SPELL_NON_MELEE_DAMAGE_LOG");
    let mut packet = wow_packet::WorldPacket::from_bytes(bytes);
    assert_eq!(
        packet.read_uint16().expect("opcode"),
        ServerOpcodes::SpellNonMeleeDamageLog as u16
    );
    packet.read_packed_guid().expect("target");
    packet.read_packed_guid().expect("caster");
    packet.read_packed_guid().expect("cast id");
    packet.read_int32().expect("spell id");
    packet.read_int32().expect("visual id");
    let damage = packet.read_int32().expect("damage");
    let original_damage = packet.read_int32().expect("original damage");
    packet.read_int32().expect("overkill");
    packet.read_uint8().expect("school mask");
    packet.read_int32().expect("absorbed");
    let resisted = packet.read_int32().expect("resisted");
    packet.read_int32().expect("shield block");
    packet.read_uint32().expect("world text viewers");
    packet.read_uint32().expect("supporters");
    // One `Periodic` bit, then the seven `HitInfo` bits.
    packet.read_bit().expect("periodic");
    // C++ writes `Flags` in seven bits (`CombatLogPackets.cpp:39`), so only
    // `HitInfo` bits below `0x80` reach the client at all.
    let flags = packet.read_bits(7).expect("hit info");
    (damage, original_damage, resisted, flags)
}

/// C++ rolls one spell critical chance per target before the hit
/// (`Spells/Spell.cpp:8675-8684`) and applies the magical arm in
/// `Unit::CalculateSpellDamageTaken` (`Entities/Unit/Unit.cpp:1319-1332`), where
/// `SpellCriticalDamageBonus` adds half again. The creature takes the critical
/// value and `SpellNonMeleeDamage::HitInfo` carries `SPELL_HIT_TYPE_CRIT`.
#[tokio::test]
async fn a_critical_spell_hit_adds_half_again_and_flags_the_log_like_cpp() {
    let (mut session, manager, creature_guid, spell_id, send_rx) =
        spell_crit_scenario_like_cpp(27_001, None).await;
    // The resolved chance is the published 25%; a draw below it crits.
    let _pinned = crate::session::spell_effects::PinnedSpellCritRollLikeCpp::pin(10.0);

    let _ = drain_server_packet_bytes(&send_rx);
    session
        .apply_damage(Some(spell_id), creature_guid, 100)
        .await
        .expect("the represented spell damage must apply");
    let packets = drain_server_packet_bytes(&send_rx);

    let hp = manager
        .read()
        .unwrap()
        .find_creature(0, 0, creature_guid)
        .map(|creature| creature.current_hp())
        .expect("the creature must still be registered");
    assert_eq!(hp, 500 - 150, "a magical critical adds half again");

    let (damage, original_damage, _, flags) = spell_non_melee_damage_log_values_like_cpp(&packets);
    assert_eq!(damage, 150);
    assert_eq!(
        original_damage, 150,
        "C++ assigns originalDamage after the critical arm"
    );
    assert_eq!(flags, 0x02, "SPELL_HIT_TYPE_CRIT");
}

/// The same hit with a draw above the chance is an ordinary hit: no bonus, no flag.
#[tokio::test]
async fn a_non_critical_spell_hit_keeps_its_damage_and_flags_like_cpp() {
    let (mut session, manager, creature_guid, spell_id, send_rx) =
        spell_crit_scenario_like_cpp(27_002, None).await;
    let _pinned = crate::session::spell_effects::PinnedSpellCritRollLikeCpp::pin(90.0);

    let _ = drain_server_packet_bytes(&send_rx);
    session
        .apply_damage(Some(spell_id), creature_guid, 100)
        .await
        .expect("the represented spell damage must apply");
    let packets = drain_server_packet_bytes(&send_rx);

    let hp = manager
        .read()
        .unwrap()
        .find_creature(0, 0, creature_guid)
        .map(|creature| creature.current_hp())
        .expect("the creature must still be registered");
    assert_eq!(hp, 500 - 100);

    let (damage, _, _, flags) = spell_non_melee_damage_log_values_like_cpp(&packets);
    assert_eq!(damage, 100);
    assert_eq!(flags, 0);
}

/// C++ `Unit::CalcAbsorbResist` takes the resisted share out of the hit before the
/// victim sees it (`Entities/Unit/Unit.cpp:2080-2111`), and
/// `CalculateSpellDamageTaken` publishes it as `resist` with a partial- or
/// full-resist `HitInfo` bit (`:1346-1360`).
///
/// The creature's fire resistance is what C++ seeds from
/// `creature_template_resistance`. With 100 resistance at level 20 on both sides
/// the average reduction is `100 / (100 + 100)`, whose table puts half the damage
/// in bucket five, so a draw inside it resists exactly half.
#[tokio::test]
async fn a_resisted_spell_hit_loses_that_share_and_reports_it_like_cpp() {
    let (mut session, manager, creature_guid, spell_id, send_rx) =
        spell_crit_scenario_like_cpp(27_003, None).await;
    session
        .mutate_world_creature(creature_guid, |creature| {
            creature
                .creature
                .set_resistances_like_cpp([0, 0, 100, 0, 0, 0, 0]);
            creature.creature.unit_mut().set_level(20);
        })
        .expect("the creature must be registered");
    // No critical, and a resist draw inside the half-damage bucket.
    let _no_crit = crate::session::spell_effects::PinnedSpellCritRollLikeCpp::pin(90.0);
    let _resist = crate::session::spell_effects::PinnedResistRollLikeCpp::pin(0.5);

    let _ = drain_server_packet_bytes(&send_rx);
    session
        .apply_damage(Some(spell_id), creature_guid, 200)
        .await
        .expect("the represented spell damage must apply");
    let packets = drain_server_packet_bytes(&send_rx);

    let hp = manager
        .read()
        .unwrap()
        .find_creature(0, 0, creature_guid)
        .map(|creature| creature.current_hp())
        .expect("the creature must still be registered");
    assert_eq!(hp, 500 - 100, "half the hit was resisted");

    let (damage, original_damage, resisted, flags) =
        spell_non_melee_damage_log_values_like_cpp(&packets);
    assert_eq!(damage, 100, "the log reports the damage after the resist");
    assert_eq!(
        original_damage, 200,
        "C++ assigns originalDamage before CalcAbsorbResist"
    );
    assert_eq!(
        resisted, 100,
        "the client learns the resist from this field"
    );
    // `HITINFO_PARTIAL_RESIST` is `0x100` and C++ writes `Flags` in seven bits, so
    // the bit it sets on the server never reaches the client on this packet. The
    // truncation is C++'s, not this port's.
    assert_eq!(flags, 0);
}

/// A creature with no resistance row resists nothing, and the physical school is
/// never resisted at all: C++ returns before the roll for both.
#[tokio::test]
async fn an_unresisted_spell_hit_reports_no_resist_like_cpp() {
    let (mut session, manager, creature_guid, spell_id, send_rx) =
        spell_crit_scenario_like_cpp(27_004, None).await;
    session
        .mutate_world_creature(creature_guid, |creature| {
            creature.creature.unit_mut().set_level(20);
        })
        .expect("the creature must be registered");
    let _no_crit = crate::session::spell_effects::PinnedSpellCritRollLikeCpp::pin(90.0);
    let _resist = crate::session::spell_effects::PinnedResistRollLikeCpp::pin(0.99);

    let _ = drain_server_packet_bytes(&send_rx);
    session
        .apply_damage(Some(spell_id), creature_guid, 200)
        .await
        .expect("the represented spell damage must apply");
    let packets = drain_server_packet_bytes(&send_rx);

    let hp = manager
        .read()
        .unwrap()
        .find_creature(0, 0, creature_guid)
        .map(|creature| creature.current_hp())
        .expect("the creature must still be registered");
    assert_eq!(hp, 500 - 200);
    let (damage, original_damage, resisted, flags) =
        spell_non_melee_damage_log_values_like_cpp(&packets);
    assert_eq!(damage, 200);
    assert_eq!(original_damage, 200);
    assert_eq!(resisted, 0);
    assert_eq!(flags, 0);
}

/// The one `SMSG_SPELL_ABSORB_LOG` the hit published, as
/// `(absorbed_spell_id, absorb_spell_id, absorbed, original_damage)`.
fn spell_absorb_log_values_like_cpp(packets: &[Vec<u8>]) -> (i32, i32, i32, i32) {
    let bytes = packets
        .iter()
        .find(|bytes| {
            wow_packet::WorldPacket::from_bytes(bytes).server_opcode()
                == Some(ServerOpcodes::SpellAbsorbLog)
        })
        .expect("a consuming shield must publish one SMSG_SPELL_ABSORB_LOG");
    let mut packet = wow_packet::WorldPacket::from_bytes(bytes);
    assert_eq!(
        packet.read_uint16().expect("opcode"),
        ServerOpcodes::SpellAbsorbLog as u16
    );
    packet.read_packed_guid().expect("attacker");
    packet.read_packed_guid().expect("victim");
    let absorbed_spell_id = packet.read_int32().expect("absorbed spell id");
    let absorb_spell_id = packet.read_int32().expect("absorb spell id");
    packet.read_packed_guid().expect("absorb caster");
    let absorbed = packet.read_int32().expect("absorbed");
    let original_damage = packet.read_int32().expect("original damage");
    (
        absorbed_spell_id,
        absorb_spell_id,
        absorbed,
        original_damage,
    )
}

/// The victim's surviving `SPELL_AURA_SCHOOL_ABSORB` amount, read from the
/// canonical aura subsystem the depletion writes to.
fn creature_shield_amount_like_cpp(
    session: &mut WorldSession,
    creature_guid: ObjectGuid,
) -> Option<i32> {
    session
        .mutate_creature_aura_owner_like_cpp(creature_guid, |creature| {
            let auras = &creature.unit().subsystems().auras;
            auras
                .applied_auras
                .first()
                .and_then(|applied| auras.applied_aura_amounts.get(applied).copied())
        })
        .flatten()
}

/// C++ `Unit::CalcAbsorbResist`'s school-absorb loop
/// (`Entities/Unit/Unit.cpp:2114-2178`) spends the victim's
/// `SPELL_AURA_SCHOOL_ABSORB` amount on the hit, publishes one
/// `SMSG_SPELL_ABSORB_LOG` per consuming shield and reports the total as
/// `SpellNonMeleeDamage::absorb`.
///
/// A shield bigger than the hit survives with the remainder, which is what makes
/// the amount real state rather than a per-hit calculation.
#[tokio::test]
async fn an_absorbed_spell_hit_spends_the_shield_and_reports_it_like_cpp() {
    let shield_spell_id = 91_830_i32;
    let (mut session, manager, creature_guid, spell_id, send_rx) =
        spell_crit_scenario_like_cpp(27_005, Some((shield_spell_id, 300, 0x04))).await;
    let _no_crit = crate::session::spell_effects::PinnedSpellCritRollLikeCpp::pin(90.0);

    let _ = drain_server_packet_bytes(&send_rx);
    session
        .apply_damage(Some(spell_id), creature_guid, 100)
        .await
        .expect("the represented spell damage must apply");
    let packets = drain_server_packet_bytes(&send_rx);

    let hp = manager
        .read()
        .unwrap()
        .find_creature(0, 0, creature_guid)
        .map(|creature| creature.current_hp())
        .expect("the creature must still be registered");
    assert_eq!(hp, 500, "the shield covered the whole hit");

    let (damage, original_damage, _, flags) = spell_non_melee_damage_log_values_like_cpp(&packets);
    assert_eq!(damage, 0, "the log reports the damage after the shields");
    assert_eq!(
        original_damage, 100,
        "C++ assigns originalDamage before CalcAbsorbResist"
    );
    assert_eq!(flags, 0);

    let (absorbed_spell_id, absorb_spell_id, absorbed, log_original_damage) =
        spell_absorb_log_values_like_cpp(&packets);
    assert_eq!(absorbed_spell_id, spell_id, "the spell being absorbed");
    assert_eq!(absorb_spell_id, shield_spell_id, "the shield's own spell");
    assert_eq!(absorbed, 100);
    assert_eq!(log_original_damage, 100);

    assert_eq!(
        creature_shield_amount_like_cpp(&mut session, creature_guid),
        Some(200),
        "C++ ChangeAmount leaves the shield with what the hit did not spend"
    );
}

/// A shield smaller than the hit absorbs what it can, is removed at zero
/// (`AURA_REMOVE_BY_ENEMY_SPELL`, `Unit.cpp:2170-2172`), and the rest of the hit
/// lands.
#[tokio::test]
async fn a_spent_spell_absorb_shield_is_removed_and_the_rest_lands_like_cpp() {
    let shield_spell_id = 91_831_i32;
    let (mut session, manager, creature_guid, spell_id, send_rx) =
        spell_crit_scenario_like_cpp(27_006, Some((shield_spell_id, 40, 0x04))).await;
    let _no_crit = crate::session::spell_effects::PinnedSpellCritRollLikeCpp::pin(90.0);

    let _ = drain_server_packet_bytes(&send_rx);
    session
        .apply_damage(Some(spell_id), creature_guid, 100)
        .await
        .expect("the represented spell damage must apply");
    let packets = drain_server_packet_bytes(&send_rx);

    let hp = manager
        .read()
        .unwrap()
        .find_creature(0, 0, creature_guid)
        .map(|creature| creature.current_hp())
        .expect("the creature must still be registered");
    assert_eq!(hp, 500 - 60, "the shield took 40 of the 100");

    let (damage, _, _, _) = spell_non_melee_damage_log_values_like_cpp(&packets);
    assert_eq!(damage, 60);
    let (_, _, absorbed, _) = spell_absorb_log_values_like_cpp(&packets);
    assert_eq!(absorbed, 40);

    assert_eq!(
        creature_shield_amount_like_cpp(&mut session, creature_guid),
        None,
        "a spent shield is removed, not left at zero"
    );
    let removed = packets.iter().any(|bytes| {
        wow_packet::WorldPacket::from_bytes(bytes).server_opcode()
            == Some(ServerOpcodes::AuraUpdate)
    });
    assert!(
        removed,
        "the removal must reach the client as an aura update"
    );
}

/// The resist runs before the shields in C++ (`CalcSpellResistedDamage` at
/// `Unit.cpp:2084`, the loop at `:2114`), so the shield only ever sees what the
/// resist left, and the log carries both shares.
#[tokio::test]
async fn a_resisted_spell_hit_absorbs_only_what_the_resist_left_like_cpp() {
    let shield_spell_id = 91_832_i32;
    let (mut session, manager, creature_guid, spell_id, send_rx) =
        spell_crit_scenario_like_cpp(27_007, Some((shield_spell_id, 300, 0x04))).await;
    session
        .mutate_world_creature(creature_guid, |creature| {
            creature
                .creature
                .set_resistances_like_cpp([0, 0, 100, 0, 0, 0, 0]);
            creature.creature.unit_mut().set_level(20);
        })
        .expect("the creature must be registered");
    let _no_crit = crate::session::spell_effects::PinnedSpellCritRollLikeCpp::pin(90.0);
    let _resist = crate::session::spell_effects::PinnedResistRollLikeCpp::pin(0.5);

    let _ = drain_server_packet_bytes(&send_rx);
    session
        .apply_damage(Some(spell_id), creature_guid, 200)
        .await
        .expect("the represented spell damage must apply");
    let packets = drain_server_packet_bytes(&send_rx);

    let hp = manager
        .read()
        .unwrap()
        .find_creature(0, 0, creature_guid)
        .map(|creature| creature.current_hp())
        .expect("the creature must still be registered");
    assert_eq!(hp, 500, "half resisted, the other half absorbed");

    let (damage, original_damage, resisted, _) =
        spell_non_melee_damage_log_values_like_cpp(&packets);
    assert_eq!(damage, 0);
    assert_eq!(original_damage, 200);
    assert_eq!(resisted, 100);
    let (_, _, absorbed, log_original_damage) = spell_absorb_log_values_like_cpp(&packets);
    assert_eq!(absorbed, 100, "the shield only saw the post-resist half");
    assert_eq!(
        log_original_damage, 200,
        "the absorb log carries GetOriginalDamage, not the absorbed share"
    );
    assert_eq!(
        creature_shield_amount_like_cpp(&mut session, creature_guid),
        Some(200)
    );
}

/// A shield whose `MiscValue` does not cover the hit's school is skipped
/// entirely (`if (!(absorbAurEff->GetMiscValue() & damageInfo.GetSchoolMask()))
/// continue;`, `Unit.cpp:2127-2128`): the hit lands whole and the amount is
/// untouched.
#[tokio::test]
async fn a_shield_of_another_school_absorbs_nothing_like_cpp() {
    let shield_spell_id = 91_833_i32;
    // The shield covers frost (`0x10`) while the hit stays fire (`0x04`).
    let (mut session, manager, creature_guid, spell_id, send_rx) =
        spell_crit_scenario_like_cpp(27_008, Some((shield_spell_id, 300, 0x10))).await;
    let _no_crit = crate::session::spell_effects::PinnedSpellCritRollLikeCpp::pin(90.0);

    let _ = drain_server_packet_bytes(&send_rx);
    session
        .apply_damage(Some(spell_id), creature_guid, 100)
        .await
        .expect("the represented spell damage must apply");
    let packets = drain_server_packet_bytes(&send_rx);

    let hp = manager
        .read()
        .unwrap()
        .find_creature(0, 0, creature_guid)
        .map(|creature| creature.current_hp())
        .expect("the creature must still be registered");
    assert_eq!(hp, 500 - 100, "a frost shield does not stop a fire hit");
    let (damage, _, _, _) = spell_non_melee_damage_log_values_like_cpp(&packets);
    assert_eq!(damage, 100);
    assert!(
        !packets.iter().any(|bytes| {
            wow_packet::WorldPacket::from_bytes(bytes).server_opcode()
                == Some(ServerOpcodes::SpellAbsorbLog)
        }),
        "a shield that absorbs nothing publishes nothing"
    );
    assert_eq!(
        creature_shield_amount_like_cpp(&mut session, creature_guid),
        Some(300)
    );
}

/// A creature aura must survive a legacy-runtime mutation of the same creature.
///
/// `sync_canonical_creature_entity_like_cpp` replaces the canonical creature
/// wholesale from its legacy mirror, so an aura written only to the canonical
/// side was discarded by the next `mutate_world_creature` — a no-op one was
/// enough. The hit path mutates the legacy creature to read resistances and to
/// apply damage, so the shield had to survive that for the absorb loop to see it
/// at all.
#[tokio::test]
async fn a_creature_aura_survives_a_legacy_mirror_mutation_like_cpp() {
    let shield_spell_id = 91_834_i32;
    let (mut session, _manager, creature_guid, _spell_id, _send_rx) =
        spell_crit_scenario_like_cpp(27_009, Some((shield_spell_id, 300, 0x04))).await;
    assert_eq!(
        creature_shield_amount_like_cpp(&mut session, creature_guid),
        Some(300),
        "the shield applies"
    );
    session
        .mutate_world_creature(creature_guid, |_creature| {})
        .expect("the creature must be registered");
    assert_eq!(
        creature_shield_amount_like_cpp(&mut session, creature_guid),
        Some(300),
        "a legacy mutation must not discard it"
    );
}
