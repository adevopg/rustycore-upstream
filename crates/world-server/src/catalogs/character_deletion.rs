//! Composition boundary for character deletion/undelete (TrinityCore 3.4.3
//! `World.cpp:1450-1453` `CharDelete.*` and `World.cpp:1598-1599`
//! `FeatureSystem.CharacterUndelete.*`; the Enabled switch itself is read into
//! `SupportFeaturePolicyLikeCpp`).

use std::sync::Arc;

use wow_config::WorldConfigSet;
use wow_database::{
    CharacterDatabase, CharacterIdentityCacheLikeCpp, LoginDatabase,
    MariaDbCharacterUndeletePersistenceAdapterLikeCpp,
};
use wow_world::character_undelete::{
    CharacterDeletionConfigLikeCpp, CharacterDeletionServiceLikeCpp,
};

use crate::bootstrap::world_config_u32;

pub(crate) fn service_like_cpp(
    char_db: &Arc<CharacterDatabase>,
    login_db: &Arc<LoginDatabase>,
    identity_cache: &Arc<CharacterIdentityCacheLikeCpp>,
    configs: &WorldConfigSet,
) -> Arc<CharacterDeletionServiceLikeCpp> {
    let defaults = CharacterDeletionConfigLikeCpp::default();
    let config = CharacterDeletionConfigLikeCpp {
        delete_method: world_config_u32(
            configs,
            "CONFIG_CHARDELETE_METHOD",
            defaults.delete_method,
        ),
        min_level: world_config_u32(configs, "CONFIG_CHARDELETE_MIN_LEVEL", defaults.min_level),
        death_knight_min_level: world_config_u32(
            configs,
            "CONFIG_CHARDELETE_DEATH_KNIGHT_MIN_LEVEL",
            defaults.death_knight_min_level,
        ),
        demon_hunter_min_level: world_config_u32(
            configs,
            "CONFIG_CHARDELETE_DEMON_HUNTER_MIN_LEVEL",
            defaults.demon_hunter_min_level,
        ),
        undelete_cooldown_secs: world_config_u32(
            configs,
            "CONFIG_FEATURE_SYSTEM_CHARACTER_UNDELETE_COOLDOWN",
            defaults.undelete_cooldown_secs,
        ),
    };
    tracing::info!(
        "Character deletion: CharDelete.Method {} (min level {}, DK {}, DH {}), undelete cooldown {}s",
        config.delete_method,
        config.min_level,
        config.death_knight_min_level,
        config.demon_hunter_min_level,
        config.undelete_cooldown_secs
    );
    Arc::new(CharacterDeletionServiceLikeCpp::new(
        config,
        Arc::new(MariaDbCharacterUndeletePersistenceAdapterLikeCpp::new(
            Arc::clone(char_db),
            Arc::clone(login_db),
            Arc::clone(identity_cache),
        )),
    ))
}
