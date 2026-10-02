//! The victim session's publication of one map-owned creature spell hit.
//!
//! Separated from the melee delivery handler because that one lives with the loot
//! handlers for historical reasons and this file has a boundary of its own: it
//! owns exactly the packets C++ sends for a spell hit the map already committed.

use crate::WorldSession;
use crate::session::mailbox::ApplyCreatureSpellDamageLikeCppCommand;

impl WorldSession {
    /// Deliver one committed map-owned creature spell hit to its player victim.
    ///
    /// C++ reaches this publication through `Spell::TargetInfo::DoDamageAndTriggers`
    /// (`Spells/Spell.cpp:2960-2985`): `CalcAbsorbResist` publishes one
    /// `SMSG_SPELL_ABSORB_LOG` per consuming shield while it calculates the hit,
    /// then `DealSpellDamage` applies the damage, then
    /// `SendSpellNonMeleeDamageLog` sends the combat log. The map-owned stage
    /// already did the first two; this keeps the publication in that order and
    /// then publishes the health, which is the order the melee hit's victim
    /// delivery uses for the same pair.
    ///
    /// Gated like the melee delivery: the wrong session, map, instance or a stale
    /// health revision drops it rather than showing the client a hit the server
    /// no longer believes in. Visibility gates only the attacker-facing combat
    /// log, never the authoritative health reconciliation.
    pub(crate) fn handle_apply_creature_spell_damage_like_cpp_command_like_cpp(
        &mut self,
        command: ApplyCreatureSpellDamageLikeCppCommand,
    ) {
        if self.state() != crate::session::SessionState::LoggedIn {
            return;
        }
        if self.player_guid() != Some(command.victim_guid) {
            return;
        }
        if self.player_map_id_like_cpp() != command.map_id {
            return;
        }
        let session_instance_id = self
            .current_canonical_player_map_key_like_cpp()
            .map(|key| key.instance_id)
            .unwrap_or(0);
        if session_instance_id != command.instance_id {
            return;
        }
        let Some(canonical_health) = self.present_committed_creature_melee_health_like_cpp(
            command.victim_health_state_revision_after,
        ) else {
            return;
        };

        self.publish_absorb_consumption_like_cpp(
            command.attacker_guid,
            command.victim_guid,
            command.spell_id,
            command.original_damage.min(i32::MAX as u32) as i32,
            command.mana_spent,
            &command.absorb_consumptions,
            &[],
        );
        if self
            .client_visible_guids_like_cpp
            .contains(&command.attacker_guid)
        {
            self.send_packet(&wow_packet::packets::combat::SpellNonMeleeDamageLog {
                target: command.victim_guid,
                caster: command.attacker_guid,
                cast_id: command.cast_id,
                spell_id: command.spell_id,
                visual_id: command.spell_visual_id.min(i32::MAX as u32) as i32,
                damage: command.damage.min(i32::MAX as u32) as i32,
                original_damage: command.original_damage.min(i32::MAX as u32) as i32,
                overkill: command.overkill,
                school_mask: command.school_mask,
                absorbed: command.absorbed.min(i32::MAX as u32) as i32,
                resisted: command.resisted.min(i32::MAX as u32) as i32,
                shield_block: 0,
                periodic: false,
                flags: command.hit_info,
            });
        }
        // A hit the shields swallowed whole commits no health transition, exactly
        // like an avoided swing.
        if command.damage > 0 {
            self.send_packet(&wow_packet::packets::combat::HealthUpdate {
                guid: command.victim_guid,
                health: canonical_health.min(i64::MAX as u64) as i64,
            });
        }
    }
}
