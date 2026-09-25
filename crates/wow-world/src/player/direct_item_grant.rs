// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Direct store of a new item into planned inventory positions.
//!
//! Moved out of the quest reward owner (`store_quest_reward_item_like_cpp`) so the
//! in-game shop delivery can reuse the exact C++ `Player::StoreNewItem` runtime
//! projection; the body is unchanged apart from the item context parameter. The
//! caller owns the returned persistence rows and their durable write.

use tracing::warn;
use wow_constants::{InventoryResult, ItemBondingType, ItemContext, ItemFieldFlags};
use wow_core::ObjectGuid;
use wow_entities::{
    ItemPosCount, SendNewItemDelivery, SendNewItemDisplayText, SendNewItemInstancePlan,
    SendNewItemModifier, SendNewItemPlan, is_bag_pos,
};
use wow_packet::packets::update::{ItemCreateData, ItemEnchantmentValuesUpdate, UpdateObject};

use crate::session::{InventoryItem, WorldSession};

/// Rows staged by [`WorldSession::grant_new_inventory_item_stacks_like_cpp`].
///
/// `published` is false only when the runtime inventory was already mutated but
/// the closing `SMSG_ITEM_PUSH_RESULT` could not be built (the pre-move quest
/// function returned `false` after recording the rows in that case).
pub(crate) struct DirectItemGrantOutcomeLikeCpp {
    pub(crate) persistence: wow_persistence::QuestItemGrantPersistenceLikeCpp,
    pub(crate) published: bool,
}

impl WorldSession {
    pub(crate) async fn grant_new_inventory_item_stacks_like_cpp(
        &mut self,
        item_guid_generator: &wow_core::ObjectGuidGenerator,
        entry_id: u32,
        quantity: u32,
        dest: &[ItemPosCount],
        context: ItemContext,
    ) -> Option<DirectItemGrantOutcomeLikeCpp> {
        let player_guid = self.player_guid()?;
        if dest.is_empty() {
            return None;
        }

        #[derive(Clone, Copy)]
        struct ExistingStackUpdate {
            item_guid: ObjectGuid,
            new_count: u32,
            should_bind: bool,
            pos: u16,
        }

        #[derive(Clone, Copy)]
        struct NewStack {
            bag: u8,
            slot: u8,
            db_guid: u64,
            item_guid: ObjectGuid,
            stack_count: u32,
            max_durability: u32,
            item_flags: u32,
            contained_in: ObjectGuid,
        }

        let item_bonding = self
            .item_storage_template(entry_id)
            .map(|template| template.bonding);
        let mut existing_updates = Vec::new();
        let mut new_stacks = Vec::new();
        let mut persistence_existing_stacks = Vec::new();
        let mut persistence_new_stacks = Vec::new();
        let mut last_item_guid = ObjectGuid::EMPTY;
        let mut last_bag = u8::from(wow_entities::INVENTORY_SLOT_BAG_0);
        let mut last_slot = 0;
        let mut last_count_in_stack = 0;
        let new_item_count = dest
            .iter()
            .filter(|dest| {
                let bag = (dest.pos >> 8) as u8;
                let slot = (dest.pos & 0x00FF) as u8;
                self.get_inventory_item_by_pos(bag, slot).is_none()
            })
            .count();
        let Some(allocated_new_item_guids) = self
            .allocate_item_instance_guids_with_generator_like_cpp(
                item_guid_generator,
                new_item_count,
            )
        else {
            warn!(
                account = self.account_id,
                entry_id,
                count = new_item_count,
                "direct item grant: process-wide item GUID allocator is unavailable"
            );
            self.send_equip_error(InventoryResult::InvFull, None, None, 0, 0);
            return None;
        };
        let mut allocated_new_item_guids = allocated_new_item_guids.into_iter();

        for dest in dest {
            let bag = (dest.pos >> 8) as u8;
            let slot = (dest.pos & 0x00FF) as u8;

            if let Some(inv_item) = self.get_inventory_item_by_pos(bag, slot) {
                let Some(existing_item) =
                    self.resolved_inventory_item_object_like_cpp(inv_item.guid)
                else {
                    warn!(
                        account = self.account_id,
                        slot,
                        entry_id,
                        "direct item grant: missing runtime item object for reward item stack"
                    );
                    self.send_equip_error(InventoryResult::ItemNotFound, None, None, 0, 0);
                    return None;
                };
                let new_count = existing_item.count().saturating_add(dest.count);
                let existing_flags = existing_item.item_flags_bits();
                let should_bind = item_bonding.is_some_and(|bonding| {
                    matches!(bonding, ItemBondingType::OnAcquire | ItemBondingType::Quest)
                        || (bonding == ItemBondingType::OnEquip && is_bag_pos(dest.pos))
                });
                persistence_existing_stacks.push(
                    wow_persistence::QuestItemExistingStackPersistenceLikeCpp {
                        item_guid: inv_item.db_guid,
                        new_count,
                        dynamic_flags: (should_bind && !existing_item.is_soul_bound())
                            .then_some(existing_flags | ItemFieldFlags::SOULBOUND.bits()),
                    },
                );
                existing_updates.push(ExistingStackUpdate {
                    item_guid: inv_item.guid,
                    new_count,
                    should_bind,
                    pos: dest.pos,
                });
                last_item_guid = inv_item.guid;
                last_bag = bag;
                last_slot = slot;
                last_count_in_stack = new_count;
            } else {
                let (inventory_bag_db_guid, contained_in) = if bag
                    == u8::from(wow_entities::INVENTORY_SLOT_BAG_0)
                {
                    (0, player_guid)
                } else if let Some(bag_inventory_item) = self.resolved_inventory_item_like_cpp(bag)
                {
                    (bag_inventory_item.db_guid, bag_inventory_item.guid)
                } else {
                    warn!(
                        account = self.account_id,
                        bag,
                        slot,
                        entry_id,
                        "direct item grant: represented reward item destination references missing bag"
                    );
                    self.send_equip_error(InventoryResult::WrongBagType, None, None, 0, 0);
                    return None;
                };

                let Some((db_guid, item_guid)) = allocated_new_item_guids.next() else {
                    warn!(
                        account = self.account_id,
                        entry_id,
                        "direct item grant: preallocated item GUID count did not match store plan"
                    );
                    self.send_equip_error(InventoryResult::InvFull, None, None, 0, 0);
                    return None;
                };
                let max_durability = self.item_template_max_durability(entry_id);
                let should_bind = item_bonding.is_some_and(|bonding| {
                    matches!(bonding, ItemBondingType::OnAcquire | ItemBondingType::Quest)
                        || (bonding == ItemBondingType::OnEquip && is_bag_pos(dest.pos))
                });
                let item_flags = if should_bind {
                    ItemFieldFlags::SOULBOUND.bits()
                } else {
                    0
                };

                persistence_new_stacks.push(wow_persistence::QuestItemNewStackPersistenceLikeCpp {
                    item_guid: db_guid,
                    entry_id,
                    owner_guid: player_guid.counter() as u64,
                    count: dest.count,
                    max_durability,
                    dynamic_flags: item_flags,
                    bag_guid: inventory_bag_db_guid,
                    slot,
                });

                new_stacks.push(NewStack {
                    bag,
                    slot,
                    db_guid,
                    item_guid,
                    stack_count: dest.count,
                    max_durability,
                    item_flags,
                    contained_in,
                });
                last_item_guid = item_guid;
                last_bag = bag;
                last_slot = slot;
                last_count_in_stack = dest.count;
            }
        }

        // C++ `StoreNewItem` only mutates memory here; the caller owns the
        // durable write of these rows (quest reward: the operation's closing
        // `SaveToDB(false)`, Player.cpp:14867; shop delivery: the receipt batch).
        let persistence = wow_persistence::QuestItemGrantPersistenceLikeCpp {
            existing_stacks: persistence_existing_stacks,
            new_stacks: persistence_new_stacks,
        };

        for update in &existing_updates {
            let mut item_updates = vec![wow_entities::ItemObjectUpdateLikeCpp::SetCount(
                update.new_count,
            )];
            if let Some(bonding) = item_bonding {
                item_updates.push(wow_entities::ItemObjectUpdateLikeCpp::SetBonding(bonding));
                if update.should_bind {
                    item_updates.push(wow_entities::ItemObjectUpdateLikeCpp::BindIfStored(
                        is_bag_pos(update.pos),
                    ));
                }
            }
            let _ =
                self.apply_inventory_item_object_updates_like_cpp(update.item_guid, &item_updates);
        }

        let inventory_type = self.item_template_inventory_type(entry_id);
        for stack in &new_stacks {
            if stack.bag == u8::from(wow_entities::INVENTORY_SLOT_BAG_0) {
                self.insert_inventory_item_like_cpp(
                    stack.slot,
                    InventoryItem {
                        guid: stack.item_guid,
                        entry_id,
                        db_guid: stack.db_guid,
                        inventory_type,
                    },
                );
            }
            let mut item_object = self.make_inventory_item_object(
                stack.item_guid,
                entry_id,
                player_guid,
                stack.stack_count,
                stack.max_durability,
                context,
                stack.slot,
            );
            if stack.bag != u8::from(wow_entities::INVENTORY_SLOT_BAG_0) {
                item_object.set_container_guid_and_slot(stack.contained_in, stack.bag);
            }
            if let Some(bonding) = item_bonding {
                item_object.set_bonding(bonding);
                item_object.bind_if_stored(is_bag_pos(wow_entities::make_item_pos(
                    stack.bag, stack.slot,
                )));
            }
            self.insert_inventory_item_object(item_object);
        }

        let map_id = self.player_map_id_like_cpp();
        if !new_stacks.is_empty() {
            let item_creates = new_stacks
                .iter()
                .map(|stack| ItemCreateData {
                    item_guid: stack.item_guid,
                    entry_id: entry_id as i32,
                    owner_guid: player_guid,
                    contained_in: stack.contained_in,
                    stack_count: stack.stack_count,
                    dynamic_flags: stack.item_flags,
                    durability: stack.max_durability,
                    max_durability: stack.max_durability,
                    random_properties_seed: 0,
                    random_properties_id: 0,
                    enchantments: [ItemEnchantmentValuesUpdate::default(); 13],
                    gems: Vec::new(),
                    context: context as u8,
                    container_slots: 0,
                    container_item_guids: [ObjectGuid::EMPTY; 36],
                })
                .collect();
            self.send_packet(&UpdateObject::create_items(item_creates, map_id));
        }

        for update in &existing_updates {
            self.send_packet(&UpdateObject::item_stack_count_update(
                update.item_guid,
                map_id,
                update.new_count,
            ));
        }

        if !new_stacks.is_empty() {
            let changed_slots: Vec<_> = new_stacks
                .iter()
                .filter(|stack| stack.bag == u8::from(wow_entities::INVENTORY_SLOT_BAG_0))
                .map(|stack| (stack.slot, stack.item_guid))
                .collect();
            if !changed_slots.is_empty() {
                self.send_player_values_update_from_entity_bridge(
                    &changed_slots,
                    &[],
                    &[],
                    &[],
                    None,
                );
            }
        }

        let Some(inventory_item_counts) = self.represented_inventory_item_counts_like_cpp() else {
            return Some(DirectItemGrantOutcomeLikeCpp {
                persistence,
                published: false,
            });
        };
        let quantity_in_inventory = inventory_item_counts.get(&entry_id).copied().unwrap_or(0);
        self.send_new_item_plan(&SendNewItemPlan {
            player_guid,
            item_guid: last_item_guid,
            item_entry: entry_id,
            item_instance: SendNewItemInstancePlan {
                item_id: entry_id,
                random_properties_seed: 0,
                random_properties_id: 0,
                modifications: Vec::<SendNewItemModifier>::new(),
            },
            slot: last_bag,
            slot_in_bag: if last_count_in_stack == quantity {
                i16::from(last_slot)
            } else {
                -1
            },
            quest_log_item_id: 0,
            quantity,
            quantity_in_inventory,
            battle_pet_species_id: 0,
            battle_pet_breed_id: 0,
            battle_pet_breed_quality: 0,
            battle_pet_level: 0,
            pushed: true,
            created: false,
            display_text: SendNewItemDisplayText::Normal,
            dungeon_encounter_id: 0,
            is_encounter_loot: false,
            delivery: SendNewItemDelivery::Direct,
        });
        Some(DirectItemGrantOutcomeLikeCpp {
            persistence,
            published: true,
        })
    }
}
