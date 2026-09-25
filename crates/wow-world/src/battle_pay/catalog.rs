//! Immutable BattlePay catalog and the product-list projection.
//!
//! Port of LegionCore `BattlePayDataStoreMgr` (`Globals/BattlePayData.cpp`) and of
//! `BattlepayManager::{SendProductList, ProductFilter, WriteProduct,
//! WriteDisplayInfo}` (`BattlePay/BattlePayMgr.cpp`), adapted to the 3.4.3
//! codec in `wow_packet::packets::battlepay`.

use std::collections::{BTreeMap, HashMap};

use wow_packet::packets::battlepay::{
    BattlePayDisplayInfo, BattlePayGetProductListResponse, BattlePayProduct, BattlePayProductGroup,
    BattlePayProductInfo, BattlePayProductItem, BattlePayShopEntry, BattlePayVisual,
};
use wow_persistence::BattlePayCatalogRowsLikeCpp;

use super::constants::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattlePayProductItemLikeCpp {
    pub id: u32,
    pub item_id: u32,
    pub quantity: u32,
    pub display_info_id: u32,
    pub pet_result: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattlePayProductLikeCpp {
    pub product_id: u32,
    /// Client fixed point (units * 10000).
    pub normal_price: u64,
    pub current_price: u64,
    pub product_type: u8,
    pub website_type: u8,
    pub choice_type: u8,
    pub flags: u32,
    pub display_info_id: u32,
    pub class_mask: u32,
    pub script_name: String,
    pub items: Vec<BattlePayProductItemLikeCpp>,
}

impl BattlePayProductLikeCpp {
    /// Products RustyCore can deliver: LegionCore `ProcessDelivery` `WebsiteType::Item`
    /// plus `ItemMount`, whose item is delivered the same way (the LegionCore switch
    /// had no `ItemMount` arm). Every other type (boosts, services, game time, pets,
    /// WoW Token) is dropped from the 3.4.3 port.
    pub(crate) fn is_deliverable_like_cpp(&self) -> bool {
        self.product_type != PRODUCT_TYPE_WOW_TOKEN_LIKE_CPP
            && self.script_name.is_empty()
            && !self.items.is_empty()
            && matches!(
                self.website_type,
                WEBSITE_TYPE_ITEM_LIKE_CPP | WEBSITE_TYPE_ITEM_MOUNT_LIKE_CPP
            )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattlePayProductGroupLikeCpp {
    pub group_id: u32,
    pub name: String,
    pub icon_file_data_id: u32,
    pub display_type: u8,
    pub ordering: u32,
    pub flags: u32,
    pub token_type: u8,
    pub ingame_only: bool,
    pub owns_tokens_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattlePayShopEntryLikeCpp {
    pub entry_id: u32,
    pub group_id: u32,
    pub product_id: u32,
    pub ordering: i32,
    pub flags: u32,
    pub banner_type: u8,
    pub display_info_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BattlePayDisplayInfoLikeCpp {
    pub file_data_id: u32,
    pub flags: u32,
    pub names: [String; 4],
    /// `(DisplayId, VisualId, ProductName)`.
    pub visuals: Vec<(u32, u32, String)>,
}

/// Load report for startup logging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BattlePayCatalogLoadReportLikeCpp {
    pub skipped_products: usize,
    pub skipped_items: usize,
    pub skipped_locales: usize,
}

/// The loaded shop catalog; immutable after startup.
#[derive(Debug, Clone, Default)]
pub struct BattlePayCatalogLikeCpp {
    products: BTreeMap<u32, BattlePayProductLikeCpp>,
    groups: Vec<BattlePayProductGroupLikeCpp>,
    shop_entries: Vec<BattlePayShopEntryLikeCpp>,
    display_infos: HashMap<u32, BattlePayDisplayInfoLikeCpp>,
    group_names: HashMap<(u32, u8), String>,
    display_names: HashMap<(u32, u8), [String; 4]>,
    token_names: HashMap<u8, String>,
}

/// Everything the product list depends on besides the catalog.
pub(crate) struct ProductListViewerLikeCpp<'a> {
    pub in_world: bool,
    pub locale: u8,
    /// `1 << (class - 1)`, 0 without a player.
    pub class_mask: u32,
    pub web_checkout: bool,
    pub currency_id: u32,
    pub token_balances: &'a HashMap<u8, i64>,
    /// LegionCore `AlreadyOwnProduct(itemId)`.
    pub owned: &'a dyn Fn(u32) -> bool,
    /// `ItemTemplate::AllowableClass` (and the other per-player item gates).
    pub item_allowed: &'a dyn Fn(u32) -> bool,
}

impl ProductListViewerLikeCpp<'_> {
    fn balance(&self, token_type: u8) -> i64 {
        self.token_balances.get(&token_type).copied().unwrap_or(0)
    }

    /// Group visibility shared by the group, entry and product loops.
    fn group_visible(&self, group: &BattlePayProductGroupLikeCpp) -> bool {
        if !self.in_world && group.ingame_only {
            return false;
        }
        if group.group_id == WOW_TOKEN_GROUP_ID_LIKE_CPP {
            return false; // WoW Token is not ported
        }
        !(group.owns_tokens_only && !self.web_checkout && self.balance(group.token_type) <= 0)
    }
}

/// Truncate to at most `max` bytes on a char boundary (codec bit-length limits).
fn clipped(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_owned();
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

impl BattlePayCatalogLikeCpp {
    /// LegionCore `BattlePayDataStoreMgr::Initialize` validation over typed rows.
    pub fn from_rows_like_cpp(
        rows: BattlePayCatalogRowsLikeCpp,
        item_exists: impl Fn(u32) -> bool,
    ) -> (Self, BattlePayCatalogLoadReportLikeCpp) {
        let mut report = BattlePayCatalogLoadReportLikeCpp::default();
        let mut catalog = Self::default();

        for row in rows.display_infos {
            catalog.display_infos.insert(
                row.display_info_id,
                BattlePayDisplayInfoLikeCpp {
                    file_data_id: row.file_data_id,
                    flags: row.flags,
                    names: row.names,
                    visuals: Vec::new(),
                },
            );
        }
        for row in rows.visuals {
            // C++ keeps visuals of unknown display infos; they are simply never read.
            if let Some(info) = catalog.display_infos.get_mut(&row.display_info_id) {
                info.visuals
                    .push((row.display_id, row.visual_id, row.product_name));
            }
        }
        for row in rows.products {
            if row.website_type >= WEBSITE_TYPE_MAX_LIKE_CPP {
                report.skipped_products += 1;
                continue;
            }
            catalog.products.insert(
                row.product_id,
                BattlePayProductLikeCpp {
                    product_id: row.product_id,
                    normal_price: cents_to_fixed_point_like_cpp(row.normal_price_cents),
                    current_price: cents_to_fixed_point_like_cpp(row.current_price_cents),
                    product_type: row.product_type,
                    website_type: row.website_type,
                    choice_type: row.choice_type,
                    flags: row.flags,
                    display_info_id: row.display_info_id,
                    class_mask: row.class_mask,
                    script_name: row.script_name,
                    items: Vec::new(),
                },
            );
        }
        for row in rows.product_items {
            let display_ok = row.display_info_id == 0
                || catalog.display_infos.contains_key(&row.display_info_id);
            let Some(product) = catalog.products.get_mut(&row.product_id) else {
                report.skipped_items += 1;
                continue;
            };
            if !display_ok || !item_exists(row.item_id) {
                report.skipped_items += 1;
                continue;
            }
            product.items.push(BattlePayProductItemLikeCpp {
                id: row.id,
                item_id: row.item_id,
                quantity: row.quantity,
                display_info_id: row.display_info_id,
                pet_result: row.pet_result,
            });
        }
        catalog.groups = rows
            .groups
            .into_iter()
            .map(|row| BattlePayProductGroupLikeCpp {
                group_id: row.group_id,
                name: row.name,
                icon_file_data_id: row.icon_file_data_id,
                display_type: row.display_type,
                ordering: row.ordering,
                flags: row.flags,
                token_type: row.token_type,
                ingame_only: row.ingame_only,
                owns_tokens_only: row.owns_tokens_only,
            })
            .collect();
        catalog.shop_entries = rows
            .shop_entries
            .into_iter()
            .map(|row| BattlePayShopEntryLikeCpp {
                entry_id: row.entry_id,
                group_id: row.group_id,
                product_id: row.product_id,
                ordering: row.ordering,
                flags: row.flags,
                banner_type: row.banner_type,
                display_info_id: row.display_info_id,
            })
            .collect();
        // The Locale column is the numeric LocaleConstant; LegionCore passed it to
        // GetLocaleByName, which cannot parse "6" and silently fell back to enUS.
        for row in rows.group_locales {
            if !is_valid_locale_index_like_cpp(row.locale) {
                report.skipped_locales += 1;
                continue;
            }
            catalog
                .group_names
                .insert((row.group_id, row.locale as u8), row.name);
        }
        for row in rows.display_info_locales {
            if !is_valid_locale_index_like_cpp(row.locale) {
                report.skipped_locales += 1;
                continue;
            }
            catalog
                .display_names
                .insert((row.display_info_id, row.locale as u8), row.names);
        }
        for row in rows.token_types {
            catalog.token_names.insert(row.token_type, row.name);
        }
        (catalog, report)
    }

    pub fn product_count(&self) -> usize {
        self.products.len()
    }

    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    pub fn shop_entry_count(&self) -> usize {
        self.shop_entries.len()
    }

    pub(crate) fn product(&self, product_id: u32) -> Option<&BattlePayProductLikeCpp> {
        self.products.get(&product_id)
    }

    /// LegionCore `GetProductGroupForProductId`: the group of the first shop entry
    /// placing the product.
    pub(crate) fn group_for_product(
        &self,
        product_id: u32,
    ) -> Option<&BattlePayProductGroupLikeCpp> {
        let group_id = self
            .shop_entries
            .iter()
            .find(|entry| entry.product_id == product_id)?
            .group_id;
        self.groups.iter().find(|group| group.group_id == group_id)
    }

    /// LegionCore `ObjectMgr::GetLocaleString`: a non-empty translation wins.
    fn localized<'a>(default: &'a str, translation: Option<&'a str>) -> &'a str {
        translation
            .filter(|value| !value.is_empty())
            .unwrap_or(default)
    }

    /// LegionCore `WriteDisplayInfo` (without the pack-description override, which
    /// only distributions used).
    pub(crate) fn display_info_like_cpp(
        &self,
        display_info_id: u32,
        locale: u8,
    ) -> Option<BattlePayDisplayInfo> {
        if display_info_id == 0 {
            return None;
        }
        let info = self.display_infos.get(&display_info_id)?;
        let translation = self.display_names.get(&(display_info_id, locale));
        let name = |index: usize, max: usize| {
            clipped(
                Self::localized(
                    &info.names[index],
                    translation.map(|names| names[index].as_str()),
                ),
                max,
            )
        };
        Some(BattlePayDisplayInfo {
            file_data_id: (info.file_data_id != 0).then_some(info.file_data_id),
            // Always present: the 54261 store Lua does `bit.band(sharedData.flags, ...)`
            // on every card (Blizzard_StoreUISecure.lua StoreFrame_UpdateCard,
            // StoreFrame_FilterEntries); LegionCore omitted a zero value.
            flags: Some(info.flags),
            name1: name(0, (1 << 10) - 1),
            name2: name(1, (1 << 10) - 1),
            name3: name(2, (1 << 13) - 1),
            name4: name(3, (1 << 13) - 1),
            visuals: info
                .visuals
                .iter()
                .map(|(display_id, visual_id, product_name)| BattlePayVisual {
                    display_id: *display_id,
                    visual_id: *visual_id,
                    unk: 0,
                    name: clipped(product_name, (1 << 10) - 1),
                })
                .collect(),
            ..BattlePayDisplayInfo::default()
        })
    }

    fn group_name(&self, group: &BattlePayProductGroupLikeCpp, locale: u8) -> String {
        let translation = self
            .group_names
            .get(&(group.group_id, locale))
            .map(String::as_str);
        clipped(Self::localized(&group.name, translation), 255)
    }

    /// LegionCore `BattlepayManager::ProductFilter` restricted to deliverable types.
    pub(crate) fn product_visible_like_cpp(
        &self,
        product: &BattlePayProductLikeCpp,
        viewer: &ProductListViewerLikeCpp<'_>,
    ) -> bool {
        if !product.is_deliverable_like_cpp() {
            return false;
        }
        if !viewer.in_world {
            // Glue store: of the deliverable types LegionCore only lists ItemMount there.
            return product.website_type == WEBSITE_TYPE_ITEM_MOUNT_LIKE_CPP;
        }
        if product.class_mask != 0 && product.class_mask & viewer.class_mask == 0 {
            return false;
        }
        product
            .items
            .iter()
            .all(|item| !(viewer.owned)(item.item_id) && (viewer.item_allowed)(item.item_id))
    }

    /// LegionCore `BattlepayManager::WriteProduct`.
    fn product_packet(
        &self,
        product: &BattlePayProductLikeCpp,
        viewer: &ProductListViewerLikeCpp<'_>,
        disable_buy: bool,
    ) -> BattlePayProduct {
        let pack = product.items.len() > 1;
        BattlePayProduct {
            product_id: product.product_id,
            product_type: product.product_type,
            flags: product.flags,
            items: product
                .items
                .iter()
                .map(|item| BattlePayProductItem {
                    id: item.id,
                    // The client shows one tooltip only: packs send no item id.
                    item_id: if pack { 0 } else { item.item_id },
                    quantity: item.quantity,
                    // LegionCore disables the buy button through HasPet.
                    has_pet: (viewer.owned)(item.item_id) || disable_buy,
                    pet_result: Some(item.pet_result),
                    display_info: self.display_info_like_cpp(item.display_info_id, viewer.locale),
                    ..BattlePayProductItem::default()
                })
                .collect(),
            display_info: self.display_info_like_cpp(product.display_info_id, viewer.locale),
            ..BattlePayProduct::default()
        }
    }

    /// LegionCore `BattlepayManager::SendProductList` for an available shop.
    pub(crate) fn product_list_like_cpp(
        &self,
        viewer: &ProductListViewerLikeCpp<'_>,
    ) -> BattlePayGetProductListResponse {
        let mut response = BattlePayGetProductListResponse {
            result: PRODUCT_LIST_AVAILABLE_LIKE_CPP,
            currency_id: viewer.currency_id,
            ..BattlePayGetProductListResponse::default()
        };
        for group in self
            .groups
            .iter()
            .filter(|group| viewer.group_visible(group))
        {
            response.product_groups.push(BattlePayProductGroup {
                group_id: group.group_id,
                icon_file_data_id: group.icon_file_data_id,
                display_type: group.display_type,
                ordering: group.ordering,
                flags: group.flags,
                name: self.group_name(group, viewer.locale),
                ..BattlePayProductGroup::default()
            });
        }
        for entry in &self.shop_entries {
            let Some(group) = self
                .groups
                .iter()
                .find(|group| group.group_id == entry.group_id)
            else {
                continue;
            };
            if !viewer.group_visible(group) {
                continue;
            }
            response.shop_entries.push(BattlePayShopEntry {
                entry_id: entry.entry_id,
                group_id: entry.group_id,
                product_id: entry.product_id,
                ordering: entry.ordering,
                vas_service_type: entry.flags,
                store_delivery_type: entry.banner_type,
                display_info: self.display_info_like_cpp(entry.display_info_id, viewer.locale),
            });
        }
        for product in self.products.values() {
            if !self.product_visible_like_cpp(product, viewer) {
                continue;
            }
            // A product without a shop entry is still sent (the client looks some
            // products up by id), with a zero balance for the price check.
            let mut token_balance = 0;
            if let Some(group) = self.group_for_product(product.product_id) {
                if !viewer.group_visible(group) {
                    continue;
                }
                token_balance = viewer.balance(group.token_type);
            }
            let display_info = self.display_info_like_cpp(product.display_info_id, viewer.locale);
            let hide_price = display_info
                .as_ref()
                .and_then(|info| info.flags)
                .is_some_and(|flags| flags & DISPLAY_FLAG_HIDE_PRICE_LIKE_CPP != 0);
            let enough_tokens = viewer.web_checkout
                || token_balance >= fixed_point_to_tokens_like_cpp(product.current_price);
            response.product_infos.push(BattlePayProductInfo {
                product_id: product.product_id,
                normal_price_fixed_point: product.normal_price,
                current_price_fixed_point: product.current_price,
                product_ids: vec![product.product_id],
                unk1: PRODUCT_INFO_UNK1_LIKE_CPP,
                choice_type: product.choice_type & 0x7F,
                display_info,
                ..BattlePayProductInfo::default()
            });
            response.products.push(self.product_packet(
                product,
                viewer,
                hide_price && !enough_tokens,
            ));
        }
        response
    }
}
