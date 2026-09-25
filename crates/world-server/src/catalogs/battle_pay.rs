//! Composition boundary for the in-game shop (LegionCore
//! `BattlePayDataStoreMgr::Initialize` + the `Bpay.*` / `Browser.*` settings).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, bail};
use tracing::{info, warn};
use wow_database::{
    CharacterDatabase, LoginDatabase, MariaDbBattlePayAccountPersistenceAdapterLikeCpp,
    MariaDbBattlePayCatalogPersistenceAdapterLikeCpp,
    MariaDbBattlePayCharacterServicePersistenceAdapterLikeCpp,
    MariaDbBattlePayDeliveryPersistenceAdapterLikeCpp,
    MariaDbBattlePayDistributionPersistenceAdapterLikeCpp, WorldDatabase,
};
use wow_persistence::{BattlePayCatalogLoadOutcomeLikeCpp, BattlePayCatalogPersistencePortLikeCpp};
use wow_world::battle_pay::{
    BattlePayCatalogLikeCpp, BattlePayConfigLikeCpp, BattlePayServiceLikeCpp,
};

/// `worldserver.conf` keys (no C++ world-config registry rows: LegionCore-only).
fn battle_pay_config_like_cpp(store_enabled_for_players: bool) -> BattlePayConfigLikeCpp {
    let defaults = BattlePayConfigLikeCpp::default();
    BattlePayConfigLikeCpp {
        enabled: wow_config::get_value_default("Bpay.Enabled", defaults.enabled),
        store_enabled_for_players,
        web_checkout: wow_config::get_value_default("Bpay.WebCheckout", defaults.web_checkout),
        currency_code: wow_config::get_string_default("Bpay.Currency", &defaults.currency_code),
        wallet_name: wow_config::get_string_default("Bpay.WalletName", &defaults.wallet_name),
        browser_enabled: wow_config::get_value_default("Browser.Enabled", defaults.browser_enabled),
        token_lifetime_secs: wow_config::get_value_default(
            "Browser.TokenLifetime",
            defaults.token_lifetime_secs,
        ),
        boost_money: wow_config::get_value_default("Bpay.Boost.Money", defaults.boost_money),
        characters_per_realm: wow_config::get_value_default(
            "CharactersPerRealm",
            defaults.characters_per_realm,
        ),
    }
}

/// `CharacterLoadout.db2` + `CharacterLoadoutItem.db2` as `(class, purpose)` ->
/// item ids (LegionCore `DB2Manager::GetItemLoadOutIdsBy`, lowest loadout id first;
/// the WotLK Classic boost purposes carry one loadout per class and all races).
fn boost_loadouts_like_cpp(data_dir: &str, locale: &str) -> HashMap<(u8, i32), Vec<u32>> {
    let (loadouts, items) = match (
        wow_data::character_progression::CharacterLoadoutStore::load(data_dir, locale),
        wow_data::character_progression::CharacterLoadoutItemStore::load(data_dir, locale),
    ) {
        (Ok(loadouts), Ok(items)) => (loadouts, items),
        (Err(error), _) | (_, Err(error)) => {
            warn!("BattlePay boost loadouts not loaded: {error:#}");
            return HashMap::new();
        }
    };
    let mut by_loadout: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for item in items.iter() {
        by_loadout
            .entry(item.character_loadout_id)
            .or_default()
            .push((item.id, item.item_id));
    }
    let mut ordered: Vec<_> = loadouts.iter().collect();
    ordered.sort_by_key(|loadout| loadout.id);
    let mut table: HashMap<(u8, i32), Vec<u32>> = HashMap::new();
    for loadout in ordered {
        let Ok(class) = u8::try_from(loadout.chr_class_id) else {
            continue;
        };
        let Some(mut entries) = by_loadout.get(&loadout.id).cloned() else {
            continue;
        };
        entries.sort_unstable();
        table
            .entry((class, loadout.purpose))
            .or_insert_with(|| entries.into_iter().map(|(_, item)| item).collect());
    }
    table
}

pub(crate) async fn load_battle_pay_catalog_like_cpp(
    persistence: &dyn BattlePayCatalogPersistencePortLikeCpp,
    item_exists: impl Fn(u32) -> bool,
) -> Result<BattlePayCatalogLikeCpp> {
    match persistence.load_rows_like_cpp().await {
        BattlePayCatalogLoadOutcomeLikeCpp::Loaded(rows) => {
            let (catalog, report) = BattlePayCatalogLikeCpp::from_rows_like_cpp(rows, item_exists);
            if report.skipped_products + report.skipped_items + report.skipped_locales > 0 {
                warn!(
                    products = report.skipped_products,
                    items = report.skipped_items,
                    locales = report.skipped_locales,
                    "Skipped invalid BattlePay catalog rows"
                );
            }
            Ok(catalog)
        }
        BattlePayCatalogLoadOutcomeLikeCpp::Failed { reason } => bail!(reason),
    }
}

/// Load the catalog and build the process-owned shop service.
///
/// A disabled shop (`Bpay.Enabled = 0`) tolerates a failed catalog read so the
/// realm still starts; an enabled one treats it as fatal like other catalogs.
pub(crate) async fn load_service_like_cpp(
    world_db: &Arc<WorldDatabase>,
    login_db: &Arc<LoginDatabase>,
    char_db: &Arc<CharacterDatabase>,
    item_store: &wow_data::ItemStore,
    world_configs: &wow_config::WorldConfigSet,
    data_dir: &str,
    locale: &str,
) -> Result<Arc<BattlePayServiceLikeCpp>> {
    let config = battle_pay_config_like_cpp(crate::bootstrap::world_config_bool(
        world_configs,
        "CONFIG_FEATURE_SYSTEM_BPAY_STORE_ENABLED",
        false,
    ));
    let persistence = MariaDbBattlePayCatalogPersistenceAdapterLikeCpp::new(Arc::clone(world_db));
    let catalog =
        match load_battle_pay_catalog_like_cpp(&persistence, |item| item_store.get(item).is_some())
            .await
        {
            Ok(catalog) => catalog,
            Err(error) if !config.enabled => {
                warn!("BattlePay catalog not loaded (shop disabled): {error:#}");
                BattlePayCatalogLikeCpp::default()
            }
            Err(error) => return Err(error.context("Failed to load the BattlePay catalog")),
        };
    info!(
        enabled = config.enabled,
        web_checkout = config.web_checkout,
        currency = %config.currency_code,
        "Loaded {} BattlePay products in {} groups ({} shop entries)",
        catalog.product_count(),
        catalog.group_count(),
        catalog.shop_entry_count()
    );
    let boost_loadouts = boost_loadouts_like_cpp(data_dir, locale);
    info!(
        "Loaded {} BattlePay boost loadouts (class, purpose)",
        boost_loadouts.len()
    );
    Ok(Arc::new(
        BattlePayServiceLikeCpp::new(
            config,
            Arc::new(catalog),
            Arc::new(MariaDbBattlePayAccountPersistenceAdapterLikeCpp::new(
                Arc::clone(login_db),
            )),
            Arc::new(MariaDbBattlePayDeliveryPersistenceAdapterLikeCpp::new(
                Arc::clone(char_db),
            )),
        )
        .with_character_services(
            Arc::new(MariaDbBattlePayDistributionPersistenceAdapterLikeCpp::new(
                Arc::clone(login_db),
            )),
            Arc::new(
                MariaDbBattlePayCharacterServicePersistenceAdapterLikeCpp::new(Arc::clone(char_db)),
            ),
        )
        .with_boost_loadouts(boost_loadouts),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wow_persistence::{
        BattlePayCatalogRowsLikeCpp, BattlePayProductItemRowLikeCpp, BattlePayProductRowLikeCpp,
        PersistenceFutureLikeCpp,
    };

    struct FixedPort(BattlePayCatalogLoadOutcomeLikeCpp);

    impl BattlePayCatalogPersistencePortLikeCpp for FixedPort {
        fn load_rows_like_cpp(
            &self,
        ) -> PersistenceFutureLikeCpp<'_, BattlePayCatalogLoadOutcomeLikeCpp> {
            Box::pin(async move { self.0.clone() })
        }
    }

    #[tokio::test]
    async fn unknown_items_are_dropped_and_failures_propagate() {
        let rows = BattlePayCatalogRowsLikeCpp {
            products: vec![BattlePayProductRowLikeCpp {
                product_id: 1,
                website_type: 3,
                ..Default::default()
            }],
            product_items: vec![
                BattlePayProductItemRowLikeCpp {
                    id: 1,
                    product_id: 1,
                    item_id: 43516,
                    quantity: 1,
                    ..Default::default()
                },
                BattlePayProductItemRowLikeCpp {
                    id: 2,
                    product_id: 1,
                    item_id: 99_999_999,
                    quantity: 1,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let catalog = load_battle_pay_catalog_like_cpp(
            &FixedPort(BattlePayCatalogLoadOutcomeLikeCpp::Loaded(rows)),
            |item| item == 43516,
        )
        .await
        .unwrap();
        assert_eq!(catalog.product_count(), 1);

        let error = load_battle_pay_catalog_like_cpp(
            &FixedPort(BattlePayCatalogLoadOutcomeLikeCpp::Failed {
                reason: "no battlepay_product".into(),
            }),
            |_| true,
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "no battlepay_product");
    }
}
