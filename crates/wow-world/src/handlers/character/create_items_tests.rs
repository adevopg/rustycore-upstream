//! C++ `Player::Create` initial-item placement regressions.

use super::*;
use wow_constants::{BagFamilyMask, ItemFlags};

const SOULBOUND: u32 = ItemFieldFlags::SOULBOUND.bits();
const NEW_ITEM: u32 = ItemFieldFlags::NEW_ITEM.bits();

fn template(
    entry: u32,
    class_id: ItemClass,
    subclass_id: u32,
    inventory_type: InventoryType,
) -> InitialItemTemplateLikeCpp {
    InitialItemTemplateLikeCpp {
        storage: ItemStorageTemplate {
            entry,
            class_id,
            subclass_id,
            inventory_type,
            bonding: ItemBondingType::None,
            bag_family: BagFamilyMask::NONE,
            max_stack_size: 1,
            max_count: 0,
            item_limit_category: 0,
            container_slots: 0,
            sell_price: 0,
            is_crafting_reagent: false,
            flags: ItemFlags::empty(),
        },
        always_allow_dual_wield: false,
        max_durability: 0,
    }
}

fn armor(entry: u32, inventory_type: InventoryType) -> InitialItemTemplateLikeCpp {
    let mut armor = template(entry, ItemClass::Armor, 1, inventory_type);
    armor.max_durability = 25;
    armor
}

fn weapon(entry: u32, inventory_type: InventoryType) -> InitialItemTemplateLikeCpp {
    template(entry, ItemClass::Weapon, 4, inventory_type)
}

fn bag(entry: u32, slots: u8) -> InitialItemTemplateLikeCpp {
    let mut bag = template(
        entry,
        ItemClass::Container,
        ItemSubClassContainer::Container as u32,
        InventoryType::Bag,
    );
    bag.storage.container_slots = slots;
    bag
}

fn stackable(entry: u32, max_stack: u32) -> InitialItemTemplateLikeCpp {
    let mut item = template(entry, ItemClass::Consumable, 5, InventoryType::NonEquip);
    item.storage.max_stack_size = max_stack;
    item
}

fn hearthstone() -> InitialItemTemplateLikeCpp {
    let mut item = template(6948, ItemClass::Miscellaneous, 0, InventoryType::NonEquip);
    item.storage.bonding = ItemBondingType::OnAcquire;
    item.storage.max_count = 1;
    item
}

fn lookup(
    templates: &[InitialItemTemplateLikeCpp],
) -> impl FnMut(u32) -> Option<InitialItemTemplateLikeCpp> + '_ {
    |item_id| {
        templates
            .iter()
            .find(|template| template.storage.entry == item_id)
            .copied()
    }
}

fn placement(plan: &InitialItemPlanLikeCpp) -> Vec<(u32, u32, u8, u8)> {
    plan.items
        .iter()
        .map(|item| (item.item_id, item.count, item.bag, item.slot))
        .collect()
}

const BAG0: u8 = INVENTORY_SLOT_BAG_0;

#[test]
fn armor_is_equipped_and_other_items_go_to_the_backpack_like_cpp() {
    let templates = [
        armor(38, InventoryType::Body),
        armor(39, InventoryType::Legs),
        armor(40, InventoryType::Feet),
        armor(41, InventoryType::Robe),
        hearthstone(),
    ];
    let plan = plan_initial_items_like_cpp(
        [(38, 1), (39, 1), (40, 1), (41, 1), (6948, 1)],
        lookup(&templates),
    );

    assert_eq!(
        placement(&plan),
        [
            (38, 1, BAG0, EQUIPMENT_SLOT_BODY),
            (41, 1, BAG0, EQUIPMENT_SLOT_CHEST),
            (39, 1, BAG0, EQUIPMENT_SLOT_LEGS),
            (40, 1, BAG0, EQUIPMENT_SLOT_FEET),
            (6948, 1, BAG0, INVENTORY_SLOT_ITEM_START),
        ]
    );
    assert!(plan.dropped.is_empty());
    // EquipNewItem sets no NEW_ITEM flag; StoreNewItem does, and the
    // BIND_ON_ACQUIRE hearthstone binds when stored.
    assert_eq!(plan.items[0].dynamic_flags, 0);
    assert_eq!(plan.items[0].durability, 25);
    assert_eq!(plan.items[4].dynamic_flags, NEW_ITEM | SOULBOUND);
}

#[test]
fn two_hander_blocks_offhand_and_displaces_an_equipped_shield_like_cpp() {
    let templates = [
        weapon(1, InventoryType::Weapon2Hand),
        template(2, ItemClass::Armor, 6, InventoryType::Shield),
    ];
    // Two-hander first: the shield cannot be equipped (IsTwoHandUsed).
    let plan = plan_initial_items_like_cpp([(1, 1), (2, 1)], lookup(&templates));
    assert_eq!(
        placement(&plan),
        [
            (1, 1, BAG0, EQUIPMENT_SLOT_MAINHAND),
            (2, 1, BAG0, INVENTORY_SLOT_ITEM_START),
        ]
    );

    // Shield first: AutoUnequipOffhandIfNeed moves it to the backpack.
    let plan = plan_initial_items_like_cpp([(2, 1), (1, 1)], lookup(&templates));
    assert_eq!(
        placement(&plan),
        [
            (1, 1, BAG0, EQUIPMENT_SLOT_MAINHAND),
            (2, 1, BAG0, INVENTORY_SLOT_ITEM_START),
        ]
    );
}

#[test]
fn one_hander_and_shield_share_hands_but_a_second_one_hander_is_stored() {
    let templates = [
        weapon(1, InventoryType::Weapon),
        template(2, ItemClass::Armor, 6, InventoryType::Shield),
    ];
    let plan = plan_initial_items_like_cpp([(2, 1), (1, 2)], lookup(&templates));
    assert_eq!(
        placement(&plan),
        [
            (1, 1, BAG0, EQUIPMENT_SLOT_MAINHAND),
            (2, 1, BAG0, EQUIPMENT_SLOT_OFFHAND),
            (1, 1, BAG0, INVENTORY_SLOT_ITEM_START),
        ],
        "no dual wield: FindEquipSlot never suggests the offhand for INVTYPE_WEAPON"
    );
}

#[test]
fn ranged_weapons_use_the_3_4_3_ranged_slot() {
    let templates = [
        weapon(1, InventoryType::Ranged),
        weapon(2, InventoryType::Thrown),
        weapon(3, InventoryType::Weapon),
        template(4, ItemClass::Armor, 6, InventoryType::Shield),
    ];
    let plan = plan_initial_items_like_cpp([(1, 1), (2, 1), (3, 1), (4, 1)], lookup(&templates));
    assert_eq!(
        placement(&plan),
        [
            (3, 1, BAG0, EQUIPMENT_SLOT_MAINHAND),
            (4, 1, BAG0, EQUIPMENT_SLOT_OFFHAND),
            (1, 1, BAG0, EQUIPMENT_SLOT_RANGED),
            (2, 1, BAG0, INVENTORY_SLOT_ITEM_START),
        ]
    );
}

#[test]
fn bags_fill_the_four_bag_slots_then_the_backpack() {
    let templates = [bag(4499, 12)];
    let plan = plan_initial_items_like_cpp([(4499, 5)], lookup(&templates));
    assert_eq!(
        placement(&plan),
        [
            (4499, 1, BAG0, INVENTORY_SLOT_BAG_START),
            (4499, 1, BAG0, INVENTORY_SLOT_BAG_START + 1),
            (4499, 1, BAG0, INVENTORY_SLOT_BAG_START + 2),
            (4499, 1, BAG0, INVENTORY_SLOT_BAG_START + 3),
            (4499, 1, BAG0, INVENTORY_SLOT_ITEM_START),
        ]
    );
}

#[test]
fn stacks_merge_before_using_free_backpack_slots() {
    let templates = [stackable(4540, 20)];
    let plan = plan_initial_items_like_cpp([(4540, 4), (4540, 4), (4540, 20)], lookup(&templates));
    assert_eq!(
        placement(&plan),
        [
            (4540, 20, BAG0, INVENTORY_SLOT_ITEM_START),
            (4540, 8, BAG0, INVENTORY_SLOT_ITEM_START + 1),
        ]
    );
    assert!(plan.items.iter().all(|item| item.dynamic_flags == NEW_ITEM));
}

#[test]
fn backpack_overflow_drops_the_whole_remaining_amount_like_cpp() {
    let templates = [
        template(1, ItemClass::Miscellaneous, 0, InventoryType::NonEquip),
        hearthstone(),
    ];
    let plan = plan_initial_items_like_cpp([(1, 17), (6948, 1), (6948, 1)], lookup(&templates));
    // CanStoreNewItem fails as a whole: none of the 17 is stored.
    assert_eq!(
        placement(&plan),
        [(6948, 1, BAG0, INVENTORY_SLOT_ITEM_START)]
    );
    assert_eq!(
        plan.dropped,
        [
            DroppedInitialItemLikeCpp {
                item_id: 1,
                count: 17
            },
            // ItemTemplate::MaxCount 1 rejects the second hearthstone.
            DroppedInitialItemLikeCpp {
                item_id: 6948,
                count: 1
            },
        ]
    );
}

#[test]
fn overflow_uses_equipped_bags_after_the_backpack() {
    let templates = [
        bag(4499, 4),
        template(1, ItemClass::Miscellaneous, 0, InventoryType::NonEquip),
    ];
    let plan = plan_initial_items_like_cpp([(4499, 1), (1, 18)], lookup(&templates));
    let placed = placement(&plan);
    assert_eq!(placed.len(), 19);
    assert_eq!(placed[0], (4499, 1, BAG0, INVENTORY_SLOT_BAG_START));
    assert!(placed.contains(&(1, 1, INVENTORY_SLOT_BAG_START, 0)));
    assert!(placed.contains(&(1, 1, INVENTORY_SLOT_BAG_START, 1)));
    assert!(!placed.contains(&(1, 1, INVENTORY_SLOT_BAG_START, 2)));
}

#[test]
fn second_pass_moves_ammo_into_the_quiver_and_compacts_the_backpack() {
    let mut quiver = template(2101, ItemClass::Quiver, 2, InventoryType::Bag);
    quiver.storage.container_slots = 6;
    let mut arrows = template(2512, ItemClass::Projectile, 2, InventoryType::Ammo);
    arrows.storage.max_stack_size = 1000;
    arrows.storage.bag_family = BagFamilyMask::ARROWS;
    let templates = [arrows, quiver, hearthstone()];

    let plan = plan_initial_items_like_cpp([(2512, 200), (2101, 1), (6948, 1)], lookup(&templates));
    assert_eq!(
        placement(&plan),
        [
            (2101, 1, BAG0, INVENTORY_SLOT_BAG_START),
            (6948, 1, BAG0, INVENTORY_SLOT_ITEM_START),
            (2512, 200, INVENTORY_SLOT_BAG_START, 0),
        ]
    );
}

#[test]
fn second_pass_equips_backpack_items_that_became_equippable() {
    let templates = [
        weapon(1, InventoryType::Weapon),
        template(2, ItemClass::Armor, 6, InventoryType::Shield),
    ];
    let mut inventory = InitialInventoryLikeCpp::new();
    inventory.place(
        (BAG0, INVENTORY_SLOT_ITEM_START),
        InitialInventoryLikeCpp::new_item(templates[1], 1, true),
    );
    inventory.place(
        (BAG0, INVENTORY_SLOT_ITEM_START + 1),
        InitialInventoryLikeCpp::new_item(templates[0], 1, true),
    );
    inventory.second_pass();
    let plan = inventory.into_plan(Vec::new());
    assert_eq!(
        placement(&plan),
        [
            (1, 1, BAG0, EQUIPMENT_SLOT_MAINHAND),
            (2, 1, BAG0, EQUIPMENT_SLOT_OFFHAND),
        ]
    );
    // NEW_ITEM from StoreNewItem survives the move to the equipment slot.
    assert!(plan.items.iter().all(|item| item.dynamic_flags == NEW_ITEM));
}

#[test]
fn unknown_templates_are_reported_and_skipped() {
    let plan = plan_initial_items_like_cpp([(7, 1)], |_| None);
    assert!(plan.items.is_empty());
    assert_eq!(
        plan.dropped,
        [DroppedInitialItemLikeCpp {
            item_id: 7,
            count: 1
        }]
    );
}

#[test]
fn equipment_cache_writes_five_fields_for_each_slot_below_reagent_bag_end() {
    let items = [
        PlannedInitialItemLikeCpp {
            item_id: 38,
            count: 1,
            bag: BAG0,
            slot: EQUIPMENT_SLOT_BODY,
            dynamic_flags: 0,
            durability: 0,
            inventory_type: InventoryType::Body as u32,
            subclass_id: 0,
        },
        PlannedInitialItemLikeCpp {
            item_id: 4499,
            count: 1,
            bag: BAG0,
            slot: INVENTORY_SLOT_BAG_START,
            dynamic_flags: 0,
            durability: 0,
            inventory_type: InventoryType::Bag as u32,
            subclass_id: 0,
        },
        // Backpack and bag contents are not part of the cache.
        PlannedInitialItemLikeCpp {
            item_id: 6948,
            count: 1,
            bag: BAG0,
            slot: INVENTORY_SLOT_ITEM_START,
            dynamic_flags: 0,
            durability: 0,
            inventory_type: 0,
            subclass_id: 0,
        },
    ];
    let cache = initial_equipment_cache_like_cpp(&items, |item_id| item_id * 10);
    let fields: Vec<&str> = cache.split(' ').collect();
    // 35 slots x 5 fields, each followed by a space.
    assert_eq!(fields.len(), usize::from(REAGENT_BAG_SLOT_END) * 5 + 1);
    assert_eq!(fields.last(), Some(&""));
    let slot = |index: usize| &fields[index * 5..index * 5 + 5];
    assert_eq!(slot(0), ["0", "0", "0", "0", "0"]);
    assert_eq!(
        slot(usize::from(EQUIPMENT_SLOT_BODY)),
        ["4", "380", "0", "0", "0"]
    );
    assert_eq!(
        slot(usize::from(INVENTORY_SLOT_BAG_START)),
        ["18", "44990", "0", "0", "0"]
    );
    assert!(cache.starts_with("0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 4 380 0 0 0 "));
}

#[test]
fn initial_character_rows_link_bag_contents_to_the_bag_guid() {
    let mut quiver = template(2101, ItemClass::Quiver, 2, InventoryType::Bag);
    quiver.storage.container_slots = 6;
    let mut arrows = template(2512, ItemClass::Projectile, 2, InventoryType::Ammo);
    arrows.storage.max_stack_size = 1000;
    arrows.storage.bag_family = BagFamilyMask::ARROWS;
    let templates = [arrows, quiver, armor(38, InventoryType::Body)];
    let items = [
        wow_data::PlayerCreateInfoItemLikeCpp {
            item_id: 2512,
            amount: 200,
        },
        wow_data::PlayerCreateInfoItemLikeCpp {
            item_id: 2101,
            amount: 1,
        },
        wow_data::PlayerCreateInfoItemLikeCpp {
            item_id: 38,
            amount: 1,
        },
    ];

    let initial = initial_character_items_like_cpp(
        &items,
        75,
        lookup(&templates),
        |item_id| item_id + 1,
        |count| Some((100..100 + count as u64).collect()),
    )
    .expect("guids allocated");

    let rows: Vec<_> = initial
        .rows
        .iter()
        .map(|row| {
            (
                row.item_guid,
                row.item_id,
                row.count,
                row.bag_guid,
                row.slot,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (100, 38, 1, 0, EQUIPMENT_SLOT_BODY),
            (101, 2101, 1, 0, INVENTORY_SLOT_BAG_START),
            (102, 2512, 200, 101, 0),
        ]
    );
    assert!(initial.rows.iter().all(|row| row.item_context == 75));
    assert_eq!(initial.rows[0].durability, 25);
    assert!(initial.equipment_cache.contains(" 4 39 0 1 0 "));
    assert!(initial.dropped.is_empty());

    assert!(
        initial_character_items_like_cpp(&items, 0, lookup(&templates), |_| 0, |_| None).is_none()
    );
}
