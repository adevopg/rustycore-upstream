//! MariaDB adapter for LegionCore `BattlePayDataStoreMgr::Initialize`
//! (`src/server/game/Globals/BattlePayData.cpp`): every BattlePay world table in
//! the C++ load order. Validation (unknown items/display infos, website types)
//! stays in the typed store built by world-server.

use std::sync::Arc;

use wow_persistence::{
    BattlePayCatalogLoadOutcomeLikeCpp, BattlePayCatalogPersistencePortLikeCpp,
    BattlePayCatalogRowsLikeCpp, BattlePayDisplayInfoLocaleRowLikeCpp,
    BattlePayDisplayInfoRowLikeCpp, BattlePayDisplayInfoVisualRowLikeCpp,
    BattlePayProductGroupLocaleRowLikeCpp, BattlePayProductGroupRowLikeCpp,
    BattlePayProductItemRowLikeCpp, BattlePayProductRowLikeCpp, BattlePayShopEntryRowLikeCpp,
    BattlePayTokenTypeRowLikeCpp, PersistenceFutureLikeCpp,
};

use crate::battle_pay_adapter::{column_i64_like_cpp, column_u64_like_cpp};
use crate::result::SqlResult;
use crate::{WorldDatabase, WorldStatements};

pub struct MariaDbBattlePayCatalogPersistenceAdapterLikeCpp {
    world_db: Arc<WorldDatabase>,
}

impl MariaDbBattlePayCatalogPersistenceAdapterLikeCpp {
    pub fn new(world_db: Arc<WorldDatabase>) -> Self {
        Self { world_db }
    }

    async fn rows<T>(
        &self,
        statement: WorldStatements,
        read: impl Fn(&SqlResult) -> T,
    ) -> anyhow::Result<Vec<T>> {
        let mut result = self
            .world_db
            .query(&self.world_db.prepare(statement))
            .await?;
        let mut rows = Vec::with_capacity(result.count());
        if result.is_empty() {
            return Ok(rows);
        }
        loop {
            rows.push(read(&result));
            if !result.next_row() {
                break;
            }
        }
        Ok(rows)
    }

    async fn load_like_cpp(&self) -> anyhow::Result<BattlePayCatalogRowsLikeCpp> {
        let u32_at = |r: &SqlResult, i: usize| column_u64_like_cpp(r, i) as u32;
        let u8_at = |r: &SqlResult, i: usize| column_u64_like_cpp(r, i) as u8;
        let names_at = |r: &SqlResult, first: usize| {
            [
                r.read_string(first),
                r.read_string(first + 1),
                r.read_string(first + 2),
                r.read_string(first + 3),
            ]
        };
        Ok(BattlePayCatalogRowsLikeCpp {
            display_infos: self
                .rows(WorldStatements::SEL_BATTLEPAY_DISPLAY_INFOS, |r| {
                    BattlePayDisplayInfoRowLikeCpp {
                        display_info_id: u32_at(r, 0),
                        creature_display_info_id: u32_at(r, 1),
                        file_data_id: u32_at(r, 2),
                        flags: u32_at(r, 3),
                        names: names_at(r, 4),
                    }
                })
                .await?,
            visuals: self
                .rows(WorldStatements::SEL_BATTLEPAY_DISPLAY_INFO_VISUALS, |r| {
                    BattlePayDisplayInfoVisualRowLikeCpp {
                        display_info_id: u32_at(r, 0),
                        display_id: u32_at(r, 1),
                        visual_id: u32_at(r, 2),
                        product_name: r.read_string(3),
                    }
                })
                .await?,
            products: self
                .rows(WorldStatements::SEL_BATTLEPAY_PRODUCTS, |r| {
                    BattlePayProductRowLikeCpp {
                        product_id: u32_at(r, 0),
                        normal_price_cents: column_u64_like_cpp(r, 1),
                        current_price_cents: column_u64_like_cpp(r, 2),
                        product_type: u8_at(r, 3),
                        website_type: u8_at(r, 4),
                        choice_type: u8_at(r, 5),
                        flags: u32_at(r, 6),
                        display_info_id: u32_at(r, 7),
                        class_mask: u32_at(r, 8),
                        script_name: r.read_string(9),
                        game_time_days: column_u64_like_cpp(r, 10) as u16,
                    }
                })
                .await?,
            product_items: self
                .rows(WorldStatements::SEL_BATTLEPAY_PRODUCT_ITEMS, |r| {
                    BattlePayProductItemRowLikeCpp {
                        id: u32_at(r, 0),
                        product_id: u32_at(r, 1),
                        item_id: u32_at(r, 2),
                        quantity: u32_at(r, 3),
                        display_info_id: u32_at(r, 4),
                        pet_result: u8_at(r, 5),
                    }
                })
                .await?,
            groups: self
                .rows(WorldStatements::SEL_BATTLEPAY_PRODUCT_GROUPS, |r| {
                    BattlePayProductGroupRowLikeCpp {
                        group_id: u32_at(r, 0),
                        name: r.read_string(1),
                        icon_file_data_id: u32_at(r, 2),
                        display_type: u8_at(r, 3),
                        ordering: u32_at(r, 4),
                        flags: u32_at(r, 5),
                        token_type: u8_at(r, 6),
                        ingame_only: u8_at(r, 7) != 0,
                        owns_tokens_only: u8_at(r, 8) != 0,
                    }
                })
                .await?,
            shop_entries: self
                .rows(WorldStatements::SEL_BATTLEPAY_SHOP_ENTRIES, |r| {
                    BattlePayShopEntryRowLikeCpp {
                        entry_id: u32_at(r, 0),
                        group_id: u32_at(r, 1),
                        product_id: u32_at(r, 2),
                        ordering: column_i64_like_cpp(r, 3) as i32,
                        flags: u32_at(r, 4),
                        banner_type: u8_at(r, 5),
                        display_info_id: u32_at(r, 6),
                    }
                })
                .await?,
            group_locales: self
                .rows(WorldStatements::SEL_BATTLEPAY_PRODUCT_GROUP_LOCALES, |r| {
                    BattlePayProductGroupLocaleRowLikeCpp {
                        group_id: u32_at(r, 0),
                        locale: u32_at(r, 1),
                        name: r.read_string(2),
                    }
                })
                .await?,
            display_info_locales: self
                .rows(WorldStatements::SEL_BATTLEPAY_DISPLAY_INFO_LOCALES, |r| {
                    BattlePayDisplayInfoLocaleRowLikeCpp {
                        display_info_id: u32_at(r, 0),
                        locale: u32_at(r, 1),
                        names: names_at(r, 2),
                    }
                })
                .await?,
            token_types: self
                .rows(WorldStatements::SEL_BATTLEPAY_TOKEN_TYPES, |r| {
                    BattlePayTokenTypeRowLikeCpp {
                        token_type: u8_at(r, 0),
                        name: r.read_string(1),
                        login_message: (!r.is_null(2)).then(|| r.read_string(2)),
                        list_if_none: u8_at(r, 3) != 0,
                    }
                })
                .await?,
        })
    }
}

impl BattlePayCatalogPersistencePortLikeCpp for MariaDbBattlePayCatalogPersistenceAdapterLikeCpp {
    fn load_rows_like_cpp(
        &self,
    ) -> PersistenceFutureLikeCpp<'_, BattlePayCatalogLoadOutcomeLikeCpp> {
        Box::pin(async move {
            match self.load_like_cpp().await {
                Ok(rows) => BattlePayCatalogLoadOutcomeLikeCpp::Loaded(rows),
                Err(error) => BattlePayCatalogLoadOutcomeLikeCpp::Failed {
                    reason: error.to_string(),
                },
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{StatementDef, WorldStatements};

    #[test]
    fn prices_are_read_as_whole_cents_and_nullable_columns_are_coalesced() {
        let products = WorldStatements::SEL_BATTLEPAY_PRODUCTS.sql();
        assert!(products.contains("CAST(ROUND(NormalPriceFixedPoint * 100) AS UNSIGNED)"));
        assert!(products.contains("COALESCE(WebsiteType, 0)"));
        assert!(
            WorldStatements::SEL_BATTLEPAY_DISPLAY_INFOS
                .sql()
                .contains("COALESCE(FileDataID, 0)")
        );
        assert!(
            WorldStatements::SEL_BATTLEPAY_PRODUCT_ITEMS
                .sql()
                .contains("COALESCE(DisplayID, 0)")
        );
    }
}
