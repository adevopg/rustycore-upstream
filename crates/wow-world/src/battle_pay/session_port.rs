//! [`BattlePaySessionLikeCpp`] for the production [`WorldSession`].

use wow_constants::{InventoryResult, ItemClass, ItemContext};
use wow_core::ObjectGuidGenerator;
use wow_packet::ServerPacket;
use wow_persistence::PlayerInventoryPersistenceRequestLikeCpp;

use super::constants::locale_index_from_name_like_cpp;
use super::flow::{BattlePayIdentityLikeCpp, BattlePayPlayerLikeCpp, BattlePaySessionLikeCpp};
use crate::session::{DirectInventoryStorageOverlayLikeCpp, WorldSession};

/// `ITEM_SUBCLASS_MISCELLANEOUS_MOUNT`.
const ITEM_SUBCLASS_MOUNT_LIKE_CPP: u32 = 5;
/// `ITEM_SPELLTRIGGER_LEARN_SPELL_ID` (`ItemEffect.TriggerType` 6).
const ITEM_SPELLTRIGGER_LEARN_SPELL_ID_LIKE_CPP: i8 = 6;

impl WorldSession {
    fn battle_pay_class_mask_like_cpp(&self) -> u32 {
        match self.player_class_like_cpp() {
            0 => 0,
            class => 1u32 << (class - 1),
        }
    }

    /// `(bag, slot, entry, count)` overlays of a planned store, merged onto the
    /// overlays of the products' earlier items.
    fn battle_pay_overlay_plan_like_cpp(
        &self,
        overlays: &mut Vec<DirectInventoryStorageOverlayLikeCpp>,
        entry_id: u32,
        dest: &[wow_entities::ItemPosCount],
    ) {
        for position in dest {
            let bag = (position.pos >> 8) as u8;
            let slot = (position.pos & 0x00FF) as u8;
            if let Some(overlay) = overlays
                .iter_mut()
                .find(|overlay| overlay.bag == bag && overlay.slot == slot)
            {
                overlay.count = overlay.count.saturating_add(position.count);
                continue;
            }
            let existing = self
                .get_inventory_item_by_pos(bag, slot)
                .and_then(|item| self.resolved_inventory_item_object_like_cpp(item.guid))
                .map_or(0, |item| item.count());
            overlays.push(DirectInventoryStorageOverlayLikeCpp {
                bag,
                slot,
                entry_id,
                count: existing.saturating_add(position.count),
            });
        }
    }
}

impl BattlePaySessionLikeCpp for WorldSession {
    fn battle_pay_identity(&self) -> BattlePayIdentityLikeCpp {
        BattlePayIdentityLikeCpp {
            account_id: self.account_id,
            battlenet_account_id: self.battlenet_account_id(),
            realm_id: u32::from(self.realm_id()),
            region_id: self.virtual_realm_address() >> 24,
            security: self.security,
            locale: locale_index_from_name_like_cpp(self.session_locale_name_like_cpp())
                .unwrap_or(0),
            ip: self
                .remote_address_like_cpp()
                .unwrap_or_default()
                .to_owned(),
            player: self.player_guid().map(|guid| BattlePayPlayerLikeCpp {
                guid,
                class_mask: self.battle_pay_class_mask_like_cpp(),
            }),
        }
    }

    fn send_battle_pay_packet<P: ServerPacket>(&self, packet: &P) {
        self.send_packet_realm(packet);
    }

    fn battle_pay_item_owned(&self, item_id: u32) -> bool {
        if self.player_guid().is_none() {
            return false;
        }
        let Some(effects) = self.item_effect_store() else {
            return false;
        };
        let learned: Vec<i32> = effects
            .values()
            .filter(|effect| {
                effect.parent_item_id == item_id
                    && effect.trigger_type == ITEM_SPELLTRIGGER_LEARN_SPELL_ID_LIKE_CPP
                    && effect.spell_id > 0
            })
            .map(|effect| effect.spell_id)
            .collect();
        if learned.is_empty() {
            return false;
        }
        let known = self.known_spells_like_cpp();
        learned.iter().any(|spell_id| known.contains(spell_id))
    }

    fn battle_pay_item_allowed(&self, item_id: u32) -> bool {
        let allowable_class = self
            .item_stats_store()
            .and_then(|store| store.sparse_template(item_id))
            .map_or(-1, |sparse| sparse.allowable_class);
        let class_mask = self.battle_pay_class_mask_like_cpp();
        allowable_class <= 0 || class_mask == 0 || (allowable_class as u32) & class_mask != 0
    }

    fn battle_pay_item_is_mount(&self, item_id: u32) -> bool {
        self.item_storage_template(item_id).is_some_and(|template| {
            template.class_id == ItemClass::Miscellaneous
                && template.subclass_id == ITEM_SUBCLASS_MOUNT_LIKE_CPP
        })
    }

    fn battle_pay_can_store(&self, items: &[(u32, u32)]) -> bool {
        if self.player_guid().is_none() {
            return false;
        }
        let mut overlays = Vec::new();
        for &(entry_id, count) in items {
            let Some((InventoryResult::Ok, dest, _)) = self
                .plan_store_new_direct_inventory_item_with_overlays_like_cpp(
                    entry_id,
                    count,
                    &overlays,
                    &[],
                )
            else {
                return false;
            };
            self.battle_pay_overlay_plan_like_cpp(&mut overlays, entry_id, &dest);
        }
        true
    }

    async fn battle_pay_grant_items(
        &mut self,
        item_guid_generator: &ObjectGuidGenerator,
        items: &[(u32, u32)],
    ) -> Option<Vec<PlayerInventoryPersistenceRequestLikeCpp>> {
        let mut rows = Vec::with_capacity(items.len());
        for &(entry_id, count) in items {
            let Some((InventoryResult::Ok, dest, _)) =
                self.plan_store_new_direct_inventory_item(entry_id, count)
            else {
                return None;
            };
            let grant = self
                .grant_new_inventory_item_stacks_like_cpp(
                    item_guid_generator,
                    entry_id,
                    count,
                    &dest,
                    ItemContext::InGameStore,
                )
                .await?;
            rows.push(PlayerInventoryPersistenceRequestLikeCpp::QuestItemGrant(
                grant.persistence,
            ));
        }
        Some(rows)
    }

    fn battle_pay_quarantine(&mut self, reason: &'static str) {
        self.kick(reason);
    }
}
