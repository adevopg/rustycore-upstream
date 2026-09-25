// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Initial items of a newly created character.
//!
//! C++ `Player::Create` (Player.cpp, "original items") calls
//! `StoreNewItemInBestSlots` for every `PlayerInfo::item`, then runs a second
//! pass over the backpack that equips what became equippable (e.g. an offhand
//! after the mainhand) or moves items to more appropriate bags. The new
//! character is then written by `CharacterHandler::HandleCharCreateOpcode`
//! through `Player::SaveToDB`, including the `equipmentCache` string.
//!
//! This module is a pure planner over the C++ inventory rules that matter for
//! an empty, level-1 character without dual wield or titan grip:
//! `FindEquipSlot`, `CanEquipItem`, `AutoUnequipOffhandIfNeed`,
//! `CanStoreItem`/`CanStoreItem_InInventorySlots`/`CanStoreItem_InBag` and
//! `StoreItem`/`_StoreItem`.
//!
//! Bounded simplifications (documented, not silent):
//! - `CanUseItem` skill/level/class checks are not re-evaluated: starting kits
//!   come from the client loadout for that class and level 1.
//! - Item limit categories and unique-equipped gem/category checks are not
//!   evaluated; `ItemTemplate::MaxCount` is.
//! - An offhand that `AutoUnequipOffhandIfNeed` cannot store is mailed by C++;
//!   here it is dropped and reported to the caller.
//!
//! NOTE: `wow_entities::player::equip_slot_candidates` maps `INVTYPE_RANGED`/
//! `INVTYPE_RANGEDRIGHT` to the mainhand (retail layout), which is wrong for
//! 3.4.3 (`EQUIPMENT_SLOT_RANGED`); this planner therefore carries its own
//! 3.4.3 `FindEquipSlot`.

use wow_constants::{
    InventoryType, ItemBondingType, ItemClass, ItemFieldFlags, ItemSubClassContainer,
    ItemSubClassWeapon,
};
use wow_entities::{
    EQUIPMENT_SLOT_BACK, EQUIPMENT_SLOT_BODY, EQUIPMENT_SLOT_CHEST, EQUIPMENT_SLOT_FEET,
    EQUIPMENT_SLOT_FINGER1, EQUIPMENT_SLOT_FINGER2, EQUIPMENT_SLOT_HANDS, EQUIPMENT_SLOT_HEAD,
    EQUIPMENT_SLOT_LEGS, EQUIPMENT_SLOT_MAINHAND, EQUIPMENT_SLOT_NECK, EQUIPMENT_SLOT_OFFHAND,
    EQUIPMENT_SLOT_RANGED, EQUIPMENT_SLOT_SHOULDERS, EQUIPMENT_SLOT_TABARD,
    EQUIPMENT_SLOT_TRINKET1, EQUIPMENT_SLOT_TRINKET2, EQUIPMENT_SLOT_WAIST, EQUIPMENT_SLOT_WRISTS,
    INVENTORY_DEFAULT_SIZE, INVENTORY_SLOT_BAG_0, INVENTORY_SLOT_BAG_END, INVENTORY_SLOT_BAG_START,
    INVENTORY_SLOT_ITEM_END, INVENTORY_SLOT_ITEM_START, Item, ItemStorageTemplate,
    REAGENT_BAG_SLOT_END, is_bag_pos, item_can_go_into_bag, make_item_pos,
};

/// C++ `INVENTORY_SLOT_ITEM_START + GetInventorySlotCount()` for a new player.
const INVENTORY_END_LIKE_CPP: u8 = INVENTORY_SLOT_ITEM_START + INVENTORY_DEFAULT_SIZE;

/// The `ItemTemplate` subset used by initial-item placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InitialItemTemplateLikeCpp {
    pub storage: ItemStorageTemplate,
    /// `ITEM_FLAG3_ALWAYS_ALLOW_DUAL_WIELD`.
    pub always_allow_dual_wield: bool,
    /// C++ `ItemTemplate::MaxDurability`, copied to `ItemData::Durability`
    /// by `Item::Create`.
    pub max_durability: u32,
}

/// One persisted initial item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlannedInitialItemLikeCpp {
    pub item_id: u32,
    pub count: u32,
    /// `INVENTORY_SLOT_BAG_0` for top-level slots, otherwise the inventory
    /// slot of the containing bag.
    pub bag: u8,
    pub slot: u8,
    pub dynamic_flags: u32,
    pub durability: u32,
    pub inventory_type: u32,
    pub subclass_id: u32,
}

/// An initial item C++ `StoreNewItemInBestSlots` could neither equip nor
/// store (C++ logs an error and skips it), or an offhand displaced by a
/// two-hander that C++ would have mailed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DroppedInitialItemLikeCpp {
    pub item_id: u32,
    pub count: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct InitialItemPlanLikeCpp {
    pub items: Vec<PlannedInitialItemLikeCpp>,
    pub dropped: Vec<DroppedInitialItemLikeCpp>,
}

#[derive(Debug, Clone)]
struct PlacedItem {
    template: InitialItemTemplateLikeCpp,
    count: u32,
    object: Item,
}

impl PlacedItem {
    fn entry(&self) -> u32 {
        self.template.storage.entry
    }

    fn is_bag(&self) -> bool {
        // C++ `Item::IsBag`: `InventoryType == INVTYPE_BAG`.
        self.template.storage.inventory_type == InventoryType::Bag
    }
}

/// `(bag, slot)` with `bag == INVENTORY_SLOT_BAG_0` for top-level slots.
type Pos = (u8, u8);

struct InitialInventoryLikeCpp {
    top: Vec<Option<PlacedItem>>,
    /// Contents of the bag equipped in `INVENTORY_SLOT_BAG_START + index`.
    bags: Vec<Vec<Option<PlacedItem>>>,
}

impl InitialInventoryLikeCpp {
    fn new() -> Self {
        Self {
            top: vec![None; usize::from(INVENTORY_SLOT_ITEM_END)],
            bags: vec![Vec::new(); usize::from(INVENTORY_SLOT_BAG_END - INVENTORY_SLOT_BAG_START)],
        }
    }

    fn get(&self, (bag, slot): Pos) -> Option<&PlacedItem> {
        if bag == INVENTORY_SLOT_BAG_0 {
            self.top.get(usize::from(slot))?.as_ref()
        } else {
            self.bags
                .get(usize::from(bag.checked_sub(INVENTORY_SLOT_BAG_START)?))?
                .get(usize::from(slot))?
                .as_ref()
        }
    }

    fn get_mut(&mut self, (bag, slot): Pos) -> Option<&mut Option<PlacedItem>> {
        if bag == INVENTORY_SLOT_BAG_0 {
            self.top.get_mut(usize::from(slot))
        } else {
            self.bags
                .get_mut(usize::from(bag.checked_sub(INVENTORY_SLOT_BAG_START)?))?
                .get_mut(usize::from(slot))
        }
    }

    fn take(&mut self, pos: Pos) -> Option<PlacedItem> {
        let item = self.get_mut(pos)?.take()?;
        if pos.0 == INVENTORY_SLOT_BAG_0 && is_inventory_bag_slot(pos.1) {
            self.bags[usize::from(pos.1 - INVENTORY_SLOT_BAG_START)].clear();
        }
        Some(item)
    }

    /// C++ `Player::GetBagByPos`.
    fn bag_template(&self, bag: u8) -> Option<&InitialItemTemplateLikeCpp> {
        if !is_inventory_bag_slot(bag) {
            return None;
        }
        self.top[usize::from(bag)]
            .as_ref()
            .filter(|item| item.is_bag())
            .map(|item| &item.template)
    }

    fn place(&mut self, (bag, slot): Pos, item: PlacedItem) {
        if bag == INVENTORY_SLOT_BAG_0 && is_inventory_bag_slot(slot) && item.is_bag() {
            self.bags[usize::from(slot - INVENTORY_SLOT_BAG_START)] =
                vec![None; usize::from(item.template.storage.container_slots)];
        }
        if let Some(cell) = self.get_mut((bag, slot)) {
            *cell = Some(item);
        }
    }

    fn all_positions(&self) -> impl Iterator<Item = Pos> + '_ {
        let top = (0..INVENTORY_SLOT_ITEM_END).map(|slot| (INVENTORY_SLOT_BAG_0, slot));
        let bags = self.bags.iter().enumerate().flat_map(|(index, contents)| {
            (0..contents.len())
                .map(move |slot| (INVENTORY_SLOT_BAG_START + index as u8, slot as u8))
        });
        top.chain(bags)
    }

    /// C++ `Player::GetItemCount(entry, true, skipItem)`.
    fn item_count(&self, entry: u32, skip: Option<Pos>) -> u32 {
        self.all_positions()
            .filter(|pos| Some(*pos) != skip)
            .filter_map(|pos| self.get(pos))
            .filter(|item| item.entry() == entry)
            .map(|item| item.count)
            .sum()
    }

    /// C++ `Player::CanTakeMoreSimilarItems` for `ItemTemplate::MaxCount`.
    fn can_take_more_similar(
        &self,
        template: &InitialItemTemplateLikeCpp,
        count: u32,
        skip: Option<Pos>,
    ) -> bool {
        let max_count = template.storage.max_count;
        max_count <= 0 || self.item_count(template.storage.entry, skip) + count <= max_count as u32
    }

    /// C++ `Player::IsTwoHandUsed` (no titan grip).
    fn is_two_hand_used(&self) -> bool {
        let Some(main) = self.get((INVENTORY_SLOT_BAG_0, EQUIPMENT_SLOT_MAINHAND)) else {
            return false;
        };
        let storage = &main.template.storage;
        storage.inventory_type == InventoryType::Weapon2Hand
            || storage.inventory_type == InventoryType::Ranged
            || (storage.inventory_type == InventoryType::RangedRight
                && storage.class_id == ItemClass::Weapon
                && storage.subclass_id != ItemSubClassWeapon::Wand as u32)
    }

    /// C++ `Player::FindEquipSlot(item, NULL_SLOT, swap=false)` for 3.4.3
    /// without dual wield / titan grip.
    fn find_equip_slot(&self, template: &InitialItemTemplateLikeCpp) -> Option<u8> {
        let candidates: &[u8] = match template.storage.inventory_type {
            InventoryType::Head => &[EQUIPMENT_SLOT_HEAD],
            InventoryType::Neck => &[EQUIPMENT_SLOT_NECK],
            InventoryType::Shoulders => &[EQUIPMENT_SLOT_SHOULDERS],
            InventoryType::Body => &[EQUIPMENT_SLOT_BODY],
            InventoryType::Chest | InventoryType::Robe => &[EQUIPMENT_SLOT_CHEST],
            InventoryType::Waist => &[EQUIPMENT_SLOT_WAIST],
            InventoryType::Legs => &[EQUIPMENT_SLOT_LEGS],
            InventoryType::Feet => &[EQUIPMENT_SLOT_FEET],
            InventoryType::Wrists => &[EQUIPMENT_SLOT_WRISTS],
            InventoryType::Hands => &[EQUIPMENT_SLOT_HANDS],
            InventoryType::Finger => &[EQUIPMENT_SLOT_FINGER1, EQUIPMENT_SLOT_FINGER2],
            InventoryType::Trinket => &[EQUIPMENT_SLOT_TRINKET1, EQUIPMENT_SLOT_TRINKET2],
            InventoryType::Cloak => &[EQUIPMENT_SLOT_BACK],
            // The offhand is suggested only when the player can dual wield.
            InventoryType::Weapon | InventoryType::Weapon2Hand | InventoryType::WeaponMainhand => {
                &[EQUIPMENT_SLOT_MAINHAND]
            }
            InventoryType::Shield | InventoryType::WeaponOffhand | InventoryType::Holdable => {
                &[EQUIPMENT_SLOT_OFFHAND]
            }
            InventoryType::Ranged
            | InventoryType::Thrown
            | InventoryType::RangedRight
            | InventoryType::Relic => &[EQUIPMENT_SLOT_RANGED],
            InventoryType::Tabard => &[EQUIPMENT_SLOT_TABARD],
            InventoryType::Bag => &[
                INVENTORY_SLOT_BAG_START,
                INVENTORY_SLOT_BAG_START + 1,
                INVENTORY_SLOT_BAG_START + 2,
                INVENTORY_SLOT_BAG_START + 3,
            ],
            // Profession slots need learned profession skills, which a new
            // character does not have; everything else is not equippable.
            _ => &[],
        };
        candidates.iter().copied().find(|slot| {
            self.get((INVENTORY_SLOT_BAG_0, *slot)).is_none()
                && (*slot != EQUIPMENT_SLOT_OFFHAND || !self.is_two_hand_used())
        })
    }

    /// C++ `Player::CanEquipItem(NULL_SLOT, dest, item, swap=false)`.
    fn can_equip(&self, template: &InitialItemTemplateLikeCpp, src: Option<Pos>) -> Option<u8> {
        if !self.can_take_more_similar(template, 1, src) {
            return None;
        }
        let eslot = self.find_equip_slot(template)?;
        let storage = &template.storage;

        if storage.class_id == ItemClass::Quiver
            && (INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END).any(|slot| {
                Some((INVENTORY_SLOT_BAG_0, slot)) != src
                    && self
                        .get((INVENTORY_SLOT_BAG_0, slot))
                        .is_some_and(|bag| bag.template.storage.class_id == storage.class_id)
            })
        {
            return None;
        }

        let inventory_type = storage.inventory_type;
        if eslot == EQUIPMENT_SLOT_OFFHAND {
            let allowed = match inventory_type {
                InventoryType::Weapon | InventoryType::Weapon2Hand => false,
                InventoryType::WeaponOffhand => template.always_allow_dual_wield,
                _ => true,
            };
            if !allowed || self.is_two_hand_used() {
                return None;
            }
        }

        if inventory_type == InventoryType::Weapon2Hand {
            if eslot != EQUIPMENT_SLOT_MAINHAND {
                return None;
            }
            let offhand = (INVENTORY_SLOT_BAG_0, EQUIPMENT_SLOT_OFFHAND);
            if let Some(off_item) = self.get(offhand) {
                self.plan_store(&off_item.template, off_item.count, Some(offhand), false)?;
            }
        }
        Some(eslot)
    }

    fn bag_accepts(
        &self,
        bag: u8,
        template: &InitialItemTemplateLikeCpp,
        src: Option<Pos>,
        non_specialized: bool,
    ) -> bool {
        // C++ `CanStoreItem_InBag` preconditions.
        if src == Some((INVENTORY_SLOT_BAG_0, bag)) {
            return false;
        }
        let Some(bag_template) = self.bag_template(bag) else {
            return false;
        };
        if src
            .and_then(|pos| self.get(pos))
            .is_some_and(|item| item.is_bag() && self.bag_has_items(pos_slot(src)))
        {
            return false;
        }
        let is_plain_container = bag_template.storage.class_id == ItemClass::Container
            && bag_template.storage.subclass_id == ItemSubClassContainer::Container as u32;
        non_specialized == is_plain_container
            && item_can_go_into_bag(&template.storage, &bag_template.storage)
    }

    fn bag_has_items(&self, slot: Option<u8>) -> bool {
        slot.filter(|slot| is_inventory_bag_slot(*slot))
            .is_some_and(|slot| {
                self.bags[usize::from(slot - INVENTORY_SLOT_BAG_START)]
                    .iter()
                    .any(Option::is_some)
            })
    }

    /// One `CanStoreItem_InInventorySlots`/`CanStoreItem_InBag` scan.
    fn scan_slots(
        &self,
        positions: impl Iterator<Item = Pos>,
        template: &InitialItemTemplateLikeCpp,
        count: &mut u32,
        merge: bool,
        src: Option<Pos>,
        dest: &mut Vec<(Pos, u32)>,
    ) {
        let max_stack = template.storage.max_stack_size;
        for pos in positions {
            if *count == 0 {
                return;
            }
            let existing = self.get(pos).filter(|_| Some(pos) != src);
            if existing.is_some() != merge {
                continue;
            }
            let mut need_space = max_stack;
            if let Some(existing) = existing {
                // `Item::CanBeMergedPartlyWith`.
                if existing.entry() != template.storage.entry || existing.count >= max_stack {
                    continue;
                }
                need_space -= existing.count;
            }
            need_space = need_space.min(*count);
            if dest.iter().any(|(dest_pos, _)| *dest_pos == pos) {
                continue;
            }
            dest.push((pos, need_space));
            *count -= need_space;
        }
    }

    fn bag_positions(&self, bag: u8) -> impl Iterator<Item = Pos> + '_ {
        let size = self.bags[usize::from(bag - INVENTORY_SLOT_BAG_START)].len();
        (0..size).map(move |slot| (bag, slot as u8))
    }

    /// C++ `Player::CanStoreItem(bag, NULL_SLOT, ...)` with `bag` either
    /// `INVENTORY_SLOT_BAG_0` (`backpack_first`) or `NULL_BAG`.
    fn plan_store(
        &self,
        template: &InitialItemTemplateLikeCpp,
        count: u32,
        src: Option<Pos>,
        backpack_first: bool,
    ) -> Option<Vec<(Pos, u32)>> {
        if !self.can_take_more_similar(template, count, src) {
            return None;
        }
        let storage = &template.storage;
        let mut count = count;
        let mut dest = Vec::new();
        let backpack = || {
            (INVENTORY_SLOT_ITEM_START..INVENTORY_END_LIKE_CPP).map(|s| (INVENTORY_SLOT_BAG_0, s))
        };
        let bags = INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END;

        if backpack_first {
            if storage.max_stack_size != 1 {
                self.scan_slots(backpack(), template, &mut count, true, src, &mut dest);
            }
            self.scan_slots(backpack(), template, &mut count, false, src, &mut dest);
        }

        if storage.max_stack_size != 1 {
            self.scan_slots(backpack(), template, &mut count, true, src, &mut dest);
            if !storage.bag_family.is_empty() {
                for bag in bags.clone() {
                    if self.bag_accepts(bag, template, src, false) {
                        let slots = self.bag_positions(bag);
                        self.scan_slots(slots, template, &mut count, true, src, &mut dest);
                    }
                }
            }
            for bag in bags.clone() {
                if self.bag_accepts(bag, template, src, true) {
                    let slots = self.bag_positions(bag);
                    self.scan_slots(slots, template, &mut count, true, src, &mut dest);
                }
            }
        }

        if !storage.bag_family.is_empty() {
            for bag in bags.clone() {
                if self.bag_accepts(bag, template, src, false) {
                    let slots = self.bag_positions(bag);
                    self.scan_slots(slots, template, &mut count, false, src, &mut dest);
                }
            }
        }

        if count > 0
            && src.is_some_and(|pos| pos.0 == INVENTORY_SLOT_BAG_0)
            && self.bag_has_items(pos_slot(src))
        {
            return None;
        }

        // New bags can be directly equipped.
        let search_start = if src.is_none()
            && storage.class_id == ItemClass::Container
            && storage.subclass_id == ItemSubClassContainer::Container as u32
            && matches!(
                storage.bonding,
                ItemBondingType::None | ItemBondingType::OnAcquire
            ) {
            INVENTORY_SLOT_BAG_START
        } else {
            INVENTORY_SLOT_ITEM_START
        };
        let free_top = (search_start..INVENTORY_END_LIKE_CPP).map(|s| (INVENTORY_SLOT_BAG_0, s));
        self.scan_slots(free_top, template, &mut count, false, src, &mut dest);

        for bag in bags {
            if self.bag_accepts(bag, template, src, true) {
                let slots = self.bag_positions(bag);
                self.scan_slots(slots, template, &mut count, false, src, &mut dest);
            }
        }

        (count == 0).then_some(dest)
    }

    /// C++ `Player::StoreItem(dest, item)` / `_StoreItem`: every destination
    /// but the last receives a clone (flags copied), merges add to the
    /// existing stack, and the storage binding rule applies at each
    /// destination.
    fn store(&mut self, dest: &[(Pos, u32)], item: PlacedItem) {
        for (pos, count) in dest {
            let bag_pos = is_bag_pos(make_item_pos(pos.0, pos.1));
            match self.get_mut(*pos).and_then(Option::as_mut) {
                Some(existing) => {
                    existing.object.bind_if_stored(bag_pos);
                    existing.count += count;
                }
                None => {
                    let mut placed = item.clone();
                    placed.count = *count;
                    placed.object.bind_if_stored(bag_pos);
                    self.place(*pos, placed);
                }
            }
        }
    }

    /// C++ `Player::EquipItem` (`VisualizeItem` binding).
    fn equip(&mut self, slot: u8, mut item: PlacedItem) {
        item.object.bind_if_visualized();
        self.place((INVENTORY_SLOT_BAG_0, slot), item);
    }

    /// C++ `Player::AutoUnequipOffhandIfNeed` without dual wield/titan grip.
    fn auto_unequip_offhand_if_need(&mut self, dropped: &mut Vec<DroppedInitialItemLikeCpp>) {
        let offhand = (INVENTORY_SLOT_BAG_0, EQUIPMENT_SLOT_OFFHAND);
        let Some(off_item) = self.get(offhand) else {
            return;
        };
        let off_type = off_item.template.storage.inventory_type;
        let force = (off_type == InventoryType::WeaponOffhand
            && !off_item.template.always_allow_dual_wield)
            || off_type == InventoryType::Weapon;
        if !force && off_type != InventoryType::Weapon2Hand && !self.is_two_hand_used() {
            return;
        }

        let plan = self.plan_store(&off_item.template, off_item.count, Some(offhand), false);
        let Some(off_item) = self.take(offhand) else {
            return;
        };
        match plan {
            Some(dest) => self.store(&dest, off_item),
            None => dropped.push(DroppedInitialItemLikeCpp {
                item_id: off_item.entry(),
                count: off_item.count,
            }),
        }
    }

    fn new_item(template: InitialItemTemplateLikeCpp, count: u32, stored: bool) -> PlacedItem {
        let mut object = Item::new(0);
        object.set_bonding(template.storage.bonding);
        if stored {
            // `Player::StoreNewItem`; `EquipNewItem` does not set it.
            object.set_item_flag(ItemFieldFlags::NEW_ITEM);
        }
        PlacedItem {
            template,
            count,
            object,
        }
    }

    /// C++ `Player::StoreNewItemInBestSlots`.
    fn store_new_item_in_best_slots(
        &mut self,
        template: InitialItemTemplateLikeCpp,
        mut amount: u32,
        dropped: &mut Vec<DroppedInitialItemLikeCpp>,
    ) {
        while amount > 0 {
            let Some(slot) = self.can_equip(&template, None) else {
                break;
            };
            self.equip(slot, Self::new_item(template, 1, false));
            self.auto_unequip_offhand_if_need(dropped);
            amount -= 1;
        }
        if amount == 0 {
            return;
        }

        // Store in the main bag to simplify the second pass.
        match self.plan_store(&template, amount, None, true) {
            Some(dest) => self.store(&dest, Self::new_item(template, amount, true)),
            None => dropped.push(DroppedInitialItemLikeCpp {
                item_id: template.storage.entry,
                count: amount,
            }),
        }
    }

    /// The second pass of C++ `Player::Create` over the backpack.
    fn second_pass(&mut self) {
        for slot in INVENTORY_SLOT_ITEM_START..INVENTORY_END_LIKE_CPP {
            let pos = (INVENTORY_SLOT_BAG_0, slot);
            let Some(item) = self.get(pos) else {
                continue;
            };
            let template = item.template;
            let count = item.count;
            if let Some(eslot) = self.can_equip(&template, Some(pos)) {
                if let Some(item) = self.take(pos) {
                    self.equip(eslot, item);
                }
            } else if let Some(dest) = self.plan_store(&template, count, Some(pos), false) {
                if let Some(item) = self.take(pos) {
                    self.store(&dest, item);
                }
            }
        }
    }

    fn into_plan(self, dropped: Vec<DroppedInitialItemLikeCpp>) -> InitialItemPlanLikeCpp {
        let items = self
            .all_positions()
            .filter_map(|(bag, slot)| {
                let item = self.get((bag, slot))?;
                Some(PlannedInitialItemLikeCpp {
                    item_id: item.entry(),
                    count: item.count,
                    bag,
                    slot,
                    dynamic_flags: item.object.item_flags_bits(),
                    durability: item.template.max_durability,
                    inventory_type: item.template.storage.inventory_type as u32,
                    subclass_id: item.template.storage.subclass_id,
                })
            })
            .collect();
        InitialItemPlanLikeCpp { items, dropped }
    }
}

fn is_inventory_bag_slot(slot: u8) -> bool {
    (INVENTORY_SLOT_BAG_START..INVENTORY_SLOT_BAG_END).contains(&slot)
}

fn pos_slot(pos: Option<Pos>) -> Option<u8> {
    pos.filter(|pos| pos.0 == INVENTORY_SLOT_BAG_0)
        .map(|pos| pos.1)
}

/// Plan C++ `Player::Create` initial items: `StoreNewItemInBestSlots` for
/// each `(item_id, amount)` in `PlayerInfo::item` order, then the backpack
/// second pass. Unknown templates are skipped like a failed
/// `Item::CreateItem`.
pub(crate) fn plan_initial_items_like_cpp(
    items: impl IntoIterator<Item = (u32, u32)>,
    mut template: impl FnMut(u32) -> Option<InitialItemTemplateLikeCpp>,
) -> InitialItemPlanLikeCpp {
    let mut inventory = InitialInventoryLikeCpp::new();
    let mut dropped = Vec::new();
    for (item_id, amount) in items {
        match template(item_id) {
            Some(template) => {
                inventory.store_new_item_in_best_slots(template, amount, &mut dropped)
            }
            None => dropped.push(DroppedInitialItemLikeCpp {
                item_id,
                count: amount,
            }),
        }
    }
    inventory.second_pass();
    inventory.into_plan(dropped)
}

/// C++ `Player::SaveToDB` "cache equipment": for every top-level slot below
/// `REAGENT_BAG_SLOT_END`, `"InventoryType DisplayID EnchantVisual Subclass
/// SecondaryModifiedAppearanceID "`, or `"0 0 0 0 0 "` when empty. New items
/// have no visible enchantment and no secondary appearance.
pub(crate) fn initial_equipment_cache_like_cpp(
    items: &[PlannedInitialItemLikeCpp],
    mut display_id: impl FnMut(u32) -> u32,
) -> String {
    use std::fmt::Write;

    let mut cache = String::new();
    for slot in 0..REAGENT_BAG_SLOT_END {
        match items
            .iter()
            .find(|item| item.bag == INVENTORY_SLOT_BAG_0 && item.slot == slot)
        {
            Some(item) => {
                let _ = write!(
                    cache,
                    "{} {} 0 {} 0 ",
                    item.inventory_type,
                    display_id(item.item_id),
                    item.subclass_id
                );
            }
            None => cache.push_str("0 0 0 0 0 "),
        }
    }
    cache
}

/// Persistence rows and `equipmentCache` of a new character's items.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InitialCharacterItemsLikeCpp {
    pub rows: Vec<wow_persistence::CharacterCreateItemPersistenceLikeCpp>,
    pub equipment_cache: String,
    pub dropped: Vec<DroppedInitialItemLikeCpp>,
}

/// Plan `PlayerInfo::item`, allocate one item GUID per planned item
/// (C++ `Item::CreateItem` consumes the generator in the same way) and build
/// the rows written by `Player::SaveToDB`. Bag contents reference the GUID
/// of the bag equipped in their bag slot (`character_inventory.bag`).
///
/// Returns `None` only when the GUID allocation fails.
pub(crate) fn initial_character_items_like_cpp(
    items: &[wow_data::PlayerCreateInfoItemLikeCpp],
    item_context: u8,
    template: impl FnMut(u32) -> Option<InitialItemTemplateLikeCpp>,
    display_id: impl FnMut(u32) -> u32,
    allocate_guids: impl FnOnce(usize) -> Option<Vec<u64>>,
) -> Option<InitialCharacterItemsLikeCpp> {
    let plan = plan_initial_items_like_cpp(
        items.iter().map(|item| (item.item_id, item.amount)),
        template,
    );
    let guids = allocate_guids(plan.items.len())?;
    if guids.len() != plan.items.len() {
        return None;
    }

    let bag_guid = |bag: u8| {
        plan.items
            .iter()
            .zip(&guids)
            .find(|(item, _)| item.bag == INVENTORY_SLOT_BAG_0 && item.slot == bag)
            .map(|(_, guid)| *guid)
            .unwrap_or(0)
    };
    let rows = plan
        .items
        .iter()
        .zip(&guids)
        .map(
            |(item, item_guid)| wow_persistence::CharacterCreateItemPersistenceLikeCpp {
                item_guid: *item_guid,
                item_id: item.item_id,
                count: item.count,
                durability: item.durability,
                dynamic_flags: item.dynamic_flags,
                item_context,
                bag_guid: if item.bag == INVENTORY_SLOT_BAG_0 {
                    0
                } else {
                    bag_guid(item.bag)
                },
                slot: item.slot,
            },
        )
        .collect();
    let equipment_cache = initial_equipment_cache_like_cpp(&plan.items, display_id);
    Some(InitialCharacterItemsLikeCpp {
        rows,
        equipment_cache,
        dropped: plan.dropped,
    })
}

#[cfg(test)]
#[path = "create_items_tests.rs"]
mod tests;
