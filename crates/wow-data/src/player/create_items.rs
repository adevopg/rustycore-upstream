// Copyright (c) 2026 alseif0x
// RustyCore - WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 - https://www.gnu.org/licenses/gpl-3.0.html

//! C++ `ObjectMgr::LoadPlayerInfo` "Load playercreate items" and
//! `playercreateinfo_item` override data (`PlayerInfo::item`).
//!
//! Source anchors (TrinityCore wotlk_classic 3.4.3):
//! - `ObjectMgr.cpp` `LoadPlayerInfo`, blocks "Loading Player Create Items
//!   Data..." and "Loading Player Create Items Override Data...".
//! - `ObjectMgr.cpp` `ObjectMgr::PlayerCreateInfoAddItemHelper`.
//! - `DB2Structure.h` `CharacterLoadoutEntry::IsForNewCharacter` (Purpose 9).

use std::collections::HashMap;

use super::create::{
    CLASS_WARRIOR_LIKE_CPP, MAX_CLASSES_LIKE_CPP, MAX_RACES_LIKE_CPP, RACE_HUMAN_LIKE_CPP,
};
use crate::character_progression::{CharacterLoadoutEntry, CharacterLoadoutItemEntry};
use crate::skill::race_mask_for_race_like_cpp;

/// C++ `CharacterLoadoutEntry::IsForNewCharacter` (`Purpose == 9`).
pub const CHARACTER_LOADOUT_PURPOSE_NEW_CHARACTER_LIKE_CPP: i32 = 9;

const ITEM_CLASS_CONSUMABLE_LIKE_CPP: u8 = 0;
const ITEM_SUBCLASS_FOOD_DRINK_LIKE_CPP: u8 = 5;
const SPELL_CATEGORY_FOOD_LIKE_CPP: u16 = 11;
const SPELL_CATEGORY_DRINK_LIKE_CPP: u16 = 59;
const CLASS_DEATH_KNIGHT_LIKE_CPP: u8 = 6;

/// The `ItemTemplate` subset consumed by the C++ create-item loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerCreateItemTemplateLikeCpp {
    /// C++ `ItemTemplate::GetClass`.
    pub class_id: u8,
    /// C++ `ItemTemplate::GetSubClass`.
    pub subclass_id: u8,
    /// C++ `ItemTemplate::GetBuyCount` (`max(VendorStackCount, 1)`).
    pub buy_count: u32,
    /// C++ `ItemTemplate::GetMaxStackSize`.
    pub max_stack_size: u32,
    /// C++ `ItemTemplate::Effects[0]->SpellCategoryID`; `None` when the
    /// template has no effects.
    pub first_effect_spell_category_id: Option<u16>,
}

/// One `SELECT race, class, itemid, amount FROM playercreateinfo_item` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerCreateInfoItemOverrideRowLikeCpp {
    pub race: u8,
    pub class: u8,
    pub item_id: u32,
    /// C++ reads the column with `GetInt8`.
    pub amount: i8,
}

/// C++ `PlayerCreateInfoItem`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerCreateInfoItemLikeCpp {
    pub item_id: u32,
    pub amount: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerCreateInfoItemLoadReportLikeCpp {
    /// Items appended from `CharacterLoadoutItem.db2`.
    pub loadout_items: usize,
    /// Accepted `playercreateinfo_item` rows (C++ `count`).
    pub override_rows: usize,
    pub skipped_invalid_race: usize,
    pub skipped_invalid_class: usize,
    pub skipped_unknown_item: usize,
    pub skipped_zero_amount: usize,
    /// C++ logs "Invalid count ... (use -1)" but still removes.
    pub invalid_remove_count: usize,
    /// C++ logs "... not found in db2!".
    pub remove_not_found: usize,
}

/// Process-owned C++ `PlayerInfo::item` / `PlayerInfo::itemContext`.
#[derive(Debug, Clone, Default)]
pub struct PlayerCreateInfoItemStoreLikeCpp {
    items_by_key: HashMap<(u8, u8), Vec<PlayerCreateInfoItemLikeCpp>>,
    item_context_by_key: HashMap<(u8, u8), u8>,
    load_report: PlayerCreateInfoItemLoadReportLikeCpp,
}

impl PlayerCreateInfoItemStoreLikeCpp {
    /// Build the per-(race, class) initial item lists exactly in C++ order.
    ///
    /// `has_player_info(race, class)` must answer whether C++ `_playerInfo`
    /// holds that pair (a validated `playercreateinfo` row), and
    /// `item_template(id)` whether `ObjectMgr::GetItemTemplate` resolves.
    pub fn build_like_cpp<'a>(
        loadouts: impl IntoIterator<Item = &'a CharacterLoadoutEntry>,
        loadout_items: impl IntoIterator<Item = &'a CharacterLoadoutItemEntry>,
        overrides: impl IntoIterator<Item = PlayerCreateInfoItemOverrideRowLikeCpp>,
        mut has_player_info: impl FnMut(u8, u8) -> bool,
        mut item_template: impl FnMut(u32) -> Option<PlayerCreateItemTemplateLikeCpp>,
    ) -> Self {
        let mut store = Self::default();

        // C++ DB2 storages iterate in ascending ID order.
        let mut loadout_items: Vec<_> = loadout_items.into_iter().collect();
        loadout_items.sort_by_key(|entry| entry.id);
        let mut items_by_loadout: HashMap<u32, Vec<(u32, PlayerCreateItemTemplateLikeCpp)>> =
            HashMap::new();
        for loadout_item in loadout_items {
            if let Some(template) = item_template(loadout_item.item_id) {
                items_by_loadout
                    .entry(loadout_item.character_loadout_id)
                    .or_default()
                    .push((loadout_item.item_id, template));
            }
        }

        let mut loadouts: Vec<_> = loadouts.into_iter().collect();
        loadouts.sort_by_key(|entry| entry.id);
        for loadout in loadouts {
            if loadout.purpose != CHARACTER_LOADOUT_PURPOSE_NEW_CHARACTER_LIKE_CPP {
                continue;
            }
            let Some(items) = items_by_loadout.get(&loadout.id) else {
                continue;
            };
            let Ok(class) = u8::try_from(loadout.chr_class_id) else {
                continue;
            };

            for race in RACE_HUMAN_LIKE_CPP..MAX_RACES_LIKE_CPP {
                if loadout.race_mask & race_mask_for_race_like_cpp(race) == 0 {
                    continue;
                }
                if !has_player_info(race, class) {
                    continue;
                }

                store
                    .item_context_by_key
                    .insert((race, class), loadout.item_context as u8);
                let list = store.items_by_key.entry((race, class)).or_default();
                for (item_id, template) in items {
                    list.push(PlayerCreateInfoItemLikeCpp {
                        item_id: *item_id,
                        amount: loadout_item_count_like_cpp(template, class),
                    });
                    store.load_report.loadout_items += 1;
                }
            }
        }

        for row in overrides {
            if row.race >= MAX_RACES_LIKE_CPP {
                store.load_report.skipped_invalid_race += 1;
                continue;
            }
            if row.class >= MAX_CLASSES_LIKE_CPP {
                store.load_report.skipped_invalid_class += 1;
                continue;
            }
            if item_template(row.item_id).is_none() {
                store.load_report.skipped_unknown_item += 1;
                continue;
            }
            if row.amount == 0 {
                store.load_report.skipped_zero_amount += 1;
                continue;
            }

            if row.race == 0 || row.class == 0 {
                let (min_race, max_race) = if row.race != 0 {
                    (row.race, row.race + 1)
                } else {
                    (RACE_HUMAN_LIKE_CPP, MAX_RACES_LIKE_CPP)
                };
                let (min_class, max_class) = if row.class != 0 {
                    (row.class, row.class + 1)
                } else {
                    (CLASS_WARRIOR_LIKE_CPP, MAX_CLASSES_LIKE_CPP)
                };
                for race in min_race..max_race {
                    for class in min_class..max_class {
                        store.add_item_helper_like_cpp(
                            race,
                            class,
                            row.item_id,
                            i32::from(row.amount),
                            &mut has_player_info,
                        );
                    }
                }
            } else {
                store.add_item_helper_like_cpp(
                    row.race,
                    row.class,
                    row.item_id,
                    i32::from(row.amount),
                    &mut has_player_info,
                );
            }
            store.load_report.override_rows += 1;
        }

        store
    }

    /// C++ `ObjectMgr::PlayerCreateInfoAddItemHelper`.
    fn add_item_helper_like_cpp(
        &mut self,
        race: u8,
        class: u8,
        item_id: u32,
        count: i32,
        has_player_info: &mut impl FnMut(u8, u8) -> bool,
    ) {
        if !has_player_info(race, class) {
            return;
        }

        if count > 0 {
            self.items_by_key
                .entry((race, class))
                .or_default()
                .push(PlayerCreateInfoItemLikeCpp {
                    item_id,
                    amount: count as u32,
                });
            return;
        }

        if count < -1 {
            self.load_report.invalid_remove_count += 1;
        }
        let items = self.items_by_key.entry((race, class)).or_default();
        let before = items.len();
        items.retain(|item| item.item_id != item_id);
        if items.len() == before {
            self.load_report.remove_not_found += 1;
        }
    }

    /// C++ `PlayerInfo::item` for a race/class pair (empty if unknown).
    pub fn items_like_cpp(&self, race: u8, class: u8) -> &[PlayerCreateInfoItemLikeCpp] {
        self.items_by_key
            .get(&(race, class))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// C++ `PlayerInfo::itemContext` (defaults to `ItemContext::NONE`).
    pub fn item_context_like_cpp(&self, race: u8, class: u8) -> u8 {
        self.item_context_by_key
            .get(&(race, class))
            .copied()
            .unwrap_or(0)
    }

    /// Number of race/class pairs with at least one initial item.
    pub fn len(&self) -> usize {
        self.items_by_key
            .values()
            .filter(|items| !items.is_empty())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn load_report_like_cpp(&self) -> &PlayerCreateInfoItemLoadReportLikeCpp {
        &self.load_report
    }
}

/// C++ per-item count in the loadout loop: `GetBuyCount()`, replaced for
/// food/drink by 4 (10 for death knights) / 2 and clamped to the maximum
/// stack size inside that food/drink branch only.
fn loadout_item_count_like_cpp(template: &PlayerCreateItemTemplateLikeCpp, class: u8) -> u32 {
    let mut count = template.buy_count;
    if template.class_id == ITEM_CLASS_CONSUMABLE_LIKE_CPP
        && template.subclass_id == ITEM_SUBCLASS_FOOD_DRINK_LIKE_CPP
    {
        match template.first_effect_spell_category_id {
            Some(SPELL_CATEGORY_FOOD_LIKE_CPP) => {
                count = if class == CLASS_DEATH_KNIGHT_LIKE_CPP {
                    10
                } else {
                    4
                };
            }
            Some(SPELL_CATEGORY_DRINK_LIKE_CPP) => count = 2,
            _ => {}
        }
        if template.max_stack_size < count {
            count = template.max_stack_size;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loadout(id: u32, race_mask: i64, class: i8, purpose: i32) -> CharacterLoadoutEntry {
        CharacterLoadoutEntry {
            id,
            race_mask,
            chr_class_id: class,
            purpose,
            item_context: 75,
        }
    }

    fn loadout_item(id: u32, loadout_id: u32, item_id: u32) -> CharacterLoadoutItemEntry {
        CharacterLoadoutItemEntry {
            id,
            character_loadout_id: loadout_id,
            item_id,
        }
    }

    fn plain(buy_count: u32) -> PlayerCreateItemTemplateLikeCpp {
        PlayerCreateItemTemplateLikeCpp {
            class_id: 4,
            subclass_id: 1,
            buy_count,
            max_stack_size: 1,
            first_effect_spell_category_id: None,
        }
    }

    fn food(category: u16, max_stack: u32) -> PlayerCreateItemTemplateLikeCpp {
        PlayerCreateItemTemplateLikeCpp {
            class_id: ITEM_CLASS_CONSUMABLE_LIKE_CPP,
            subclass_id: ITEM_SUBCLASS_FOOD_DRINK_LIKE_CPP,
            buy_count: 5,
            max_stack_size: max_stack,
            first_effect_spell_category_id: Some(category),
        }
    }

    fn templates(item_id: u32) -> Option<PlayerCreateItemTemplateLikeCpp> {
        match item_id {
            38 | 39 | 40 | 6948 => Some(plain(1)),
            2512 => Some(PlayerCreateItemTemplateLikeCpp {
                class_id: 6,
                subclass_id: 2,
                buy_count: 200,
                max_stack_size: 1000,
                first_effect_spell_category_id: None,
            }),
            4540 => Some(food(SPELL_CATEGORY_FOOD_LIKE_CPP, 20)),
            159 => Some(food(SPELL_CATEGORY_DRINK_LIKE_CPP, 20)),
            117 => Some(food(SPELL_CATEGORY_FOOD_LIKE_CPP, 3)),
            // Food/drink subclass without effects keeps the buy count.
            118 => Some(PlayerCreateItemTemplateLikeCpp {
                first_effect_spell_category_id: None,
                ..food(0, 20)
            }),
            40582 => Some(plain(1)),
            _ => None,
        }
    }

    #[test]
    fn only_new_character_loadouts_for_existing_player_info_are_used() {
        let loadouts = [
            loadout(10, 1, 1, 9),
            // Not IsForNewCharacter.
            loadout(11, 1, 1, 10),
            // Blood elf (race 10 -> bit 9) warrior has no player info.
            loadout(12, 1 << 9, 1, 9),
        ];
        let items = [
            loadout_item(3, 10, 39),
            loadout_item(1, 10, 38),
            loadout_item(2, 11, 6948),
            loadout_item(4, 12, 40),
            // Unknown item template is skipped like C++ GetItemTemplate.
            loadout_item(5, 10, 999_999),
        ];
        let store = PlayerCreateInfoItemStoreLikeCpp::build_like_cpp(
            &loadouts,
            &items,
            [],
            |race, class| (race, class) == (1, 1),
            templates,
        );

        assert_eq!(
            store.items_like_cpp(1, 1),
            &[
                PlayerCreateInfoItemLikeCpp {
                    item_id: 38,
                    amount: 1
                },
                PlayerCreateInfoItemLikeCpp {
                    item_id: 39,
                    amount: 1
                },
            ]
        );
        assert!(store.items_like_cpp(10, 1).is_empty());
        assert_eq!(store.item_context_like_cpp(1, 1), 75);
        assert_eq!(store.item_context_like_cpp(10, 1), 0);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn race_mask_expands_to_every_matching_race_with_player_info() {
        // Human (bit 0) + Dwarf (bit 2) + Draenei (race 11 -> bit 10).
        let loadouts = [loadout(1, 1 | (1 << 2) | (1 << 10), 1, 9)];
        let items = [loadout_item(1, 1, 38)];
        let store = PlayerCreateInfoItemStoreLikeCpp::build_like_cpp(
            &loadouts,
            &items,
            [],
            |race, class| class == 1 && matches!(race, 1 | 3 | 11),
            templates,
        );
        for race in [1, 3, 11] {
            assert_eq!(store.items_like_cpp(race, 1).len(), 1, "race {race}");
        }
        assert!(store.items_like_cpp(2, 1).is_empty());
    }

    #[test]
    fn food_and_drink_counts_follow_cpp_categories_and_clamp() {
        let loadouts = [loadout(1, 1, 1, 9), loadout(2, 1, 6, 9)];
        let items = [
            loadout_item(1, 1, 4540),
            loadout_item(2, 1, 159),
            loadout_item(3, 1, 117),
            loadout_item(4, 1, 118),
            loadout_item(5, 1, 2512),
            loadout_item(6, 2, 4540),
        ];
        let store = PlayerCreateInfoItemStoreLikeCpp::build_like_cpp(
            &loadouts,
            &items,
            [],
            |_, _| true,
            templates,
        );
        let counts: Vec<_> = store
            .items_like_cpp(1, 1)
            .iter()
            .map(|item| (item.item_id, item.amount))
            .collect();
        assert_eq!(
            counts,
            [(4540, 4), (159, 2), (117, 3), (118, 5), (2512, 200)],
            "food 4, drink 2, clamp to max stack, no-effect food keeps BuyCount, \
             non-food keeps BuyCount without clamp"
        );
        assert_eq!(
            store.items_like_cpp(1, 6),
            &[PlayerCreateInfoItemLikeCpp {
                item_id: 4540,
                amount: 10
            }]
        );
    }

    #[test]
    fn overrides_add_remove_and_expand_zero_race_or_class_like_cpp() {
        let loadouts = [loadout(1, 1, 6, 9), loadout(2, 1 << 1, 6, 9)];
        let items = [
            loadout_item(1, 1, 40582),
            loadout_item(2, 1, 38),
            loadout_item(3, 2, 40582),
        ];
        let overrides = [
            // Live world row: (0, 6, 40582, -1) removes for every race.
            PlayerCreateInfoItemOverrideRowLikeCpp {
                race: 0,
                class: 6,
                item_id: 40582,
                amount: -1,
            },
            PlayerCreateInfoItemOverrideRowLikeCpp {
                race: 1,
                class: 0,
                item_id: 6948,
                amount: 1,
            },
            PlayerCreateInfoItemOverrideRowLikeCpp {
                race: 1,
                class: 6,
                item_id: 39,
                amount: -2,
            },
            PlayerCreateInfoItemOverrideRowLikeCpp {
                race: 1,
                class: 6,
                item_id: 999_999,
                amount: 1,
            },
            PlayerCreateInfoItemOverrideRowLikeCpp {
                race: 1,
                class: 6,
                item_id: 38,
                amount: 0,
            },
            PlayerCreateInfoItemOverrideRowLikeCpp {
                race: MAX_RACES_LIKE_CPP,
                class: 6,
                item_id: 38,
                amount: 1,
            },
            PlayerCreateInfoItemOverrideRowLikeCpp {
                race: 1,
                class: MAX_CLASSES_LIKE_CPP,
                item_id: 38,
                amount: 1,
            },
        ];
        let store = PlayerCreateInfoItemStoreLikeCpp::build_like_cpp(
            &loadouts,
            &items,
            overrides,
            |race, class| matches!((race, class), (1, 6) | (2, 6) | (1, 1)),
            templates,
        );

        assert_eq!(
            store.items_like_cpp(1, 6),
            &[
                PlayerCreateInfoItemLikeCpp {
                    item_id: 38,
                    amount: 1
                },
                PlayerCreateInfoItemLikeCpp {
                    item_id: 6948,
                    amount: 1
                },
            ]
        );
        assert!(store.items_like_cpp(2, 6).is_empty());
        assert_eq!(
            store.items_like_cpp(1, 1),
            &[PlayerCreateInfoItemLikeCpp {
                item_id: 6948,
                amount: 1
            }],
            "an override may seed a pair without loadout items"
        );
        assert!(store.items_like_cpp(3, 6).is_empty());

        let report = store.load_report_like_cpp();
        assert_eq!(report.override_rows, 3);
        assert_eq!(report.skipped_unknown_item, 1);
        assert_eq!(report.skipped_zero_amount, 1);
        assert_eq!(report.skipped_invalid_race, 1);
        assert_eq!(report.skipped_invalid_class, 1);
        assert_eq!(report.invalid_remove_count, 1);
        // Race 0 expands over every race; only pairs with player info count,
        // and (1,6)/(2,6) both contained 40582. The -2 row finds no item 39.
        assert_eq!(report.remove_not_found, 1);
    }
}
