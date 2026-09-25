//! What a BattlePay product delivers (LegionCore `BattlepayManager::ProcessDelivery`
//! `WebsiteType` switch, `BattlePayMgr.cpp:273-446`, plus the VAS choice types of
//! `HandleBattlePayStartVasPurchase`, `BattlePayHandler.cpp:1397`).

use super::catalog::BattlePayProductLikeCpp;
use super::constants::*;

/// Character services delivered as an at-login flag (LegionCore
/// `CharacterService::{SetRename,Customize,ChangeFaction,ChangeRace}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CharacterServiceLikeCpp {
    Rename,
    Customize,
    ChangeFaction,
    ChangeRace,
}

impl CharacterServiceLikeCpp {
    pub(crate) fn at_login_flag_like_cpp(self) -> u16 {
        match self {
            Self::Rename => at_login::RENAME,
            Self::Customize => at_login::CUSTOMIZE,
            Self::ChangeFaction => at_login::CHANGE_FACTION,
            Self::ChangeRace => at_login::CHANGE_RACE,
        }
    }

    /// `Enum.VasError` the store shows for a character that already owes this
    /// (or a conflicting) change: the character list only exposes one pending
    /// customize/race/faction service at a time (`CharacterPackets.cpp:96-103`).
    pub(crate) fn already_flagged_error_like_cpp(self, at_login_flags: u16) -> Option<u32> {
        let appearance = at_login::CUSTOMIZE | at_login::CHANGE_FACTION | at_login::CHANGE_RACE;
        match self {
            Self::Rename if at_login_flags & at_login::RENAME != 0 => {
                Some(vas_error::ALREADY_RENAME_FLAGGED)
            }
            Self::Customize if at_login_flags & appearance != 0 => {
                Some(vas_error::CUSTOMIZE_ALREADY_REQUESTED)
            }
            Self::ChangeFaction | Self::ChangeRace if at_login_flags & appearance != 0 => {
                Some(vas_error::CHARACTER_HAS_VAS_PENDING)
            }
            _ => None,
        }
    }

    fn from_website_type_like_cpp(website_type: u8) -> Option<Self> {
        Some(match website_type {
            WEBSITE_TYPE_RENAME_LIKE_CPP => Self::Rename,
            WEBSITE_TYPE_FACTION_LIKE_CPP => Self::ChangeFaction,
            WEBSITE_TYPE_RACE_LIKE_CPP => Self::ChangeRace,
            WEBSITE_TYPE_CUSTOMIZATION_LIKE_CPP => Self::Customize,
            _ => return None,
        })
    }

    fn from_choice_type_like_cpp(choice_type: u8) -> Option<Self> {
        Some(match choice_type {
            CHOICE_TYPE_VAS_NAME_CHANGE_LIKE_CPP => Self::Rename,
            CHOICE_TYPE_VAS_FACTION_CHANGE_LIKE_CPP => Self::ChangeFaction,
            CHOICE_TYPE_VAS_APPEARANCE_CHANGE_LIKE_CPP => Self::Customize,
            CHOICE_TYPE_VAS_RACE_CHANGE_LIKE_CPP => Self::ChangeRace,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProductKindLikeCpp {
    /// `WebsiteType::Item` / `ItemMount`: items into the bags of the player in the world.
    Items,
    /// At-login flag on the chosen character.
    Service(CharacterServiceLikeCpp),
    /// Same-realm move to another game account (ChoiceType 15/16).
    Transfer { faction_change: bool },
    /// `WebsiteType::CharacterBoost`: a distribution assigned from character select.
    Boost(BoostDefinitionLikeCpp),
    /// `WebsiteType::DeletedCharacter`: the undelete cooldown is cleared.
    RestoreDeletedCharacter,
    /// Everything else (WoW Token, game time, pets, script products...).
    Unsupported,
}

impl ProductKindLikeCpp {
    pub(crate) fn needs_player_in_world(self) -> bool {
        matches!(self, Self::Items)
    }
}

impl BattlePayProductLikeCpp {
    pub(crate) fn kind_like_cpp(&self) -> ProductKindLikeCpp {
        if self.product_type == PRODUCT_TYPE_WOW_TOKEN_LIKE_CPP {
            return ProductKindLikeCpp::Unsupported;
        }
        match self.choice_type {
            CHOICE_TYPE_VAS_CHARACTER_TRANSFER_LIKE_CPP => {
                return ProductKindLikeCpp::Transfer {
                    faction_change: false,
                };
            }
            CHOICE_TYPE_VAS_FACTION_TRANSFER_LIKE_CPP => {
                return ProductKindLikeCpp::Transfer {
                    faction_change: true,
                };
            }
            _ => {}
        }
        if let Some(service) =
            CharacterServiceLikeCpp::from_website_type_like_cpp(self.website_type)
                .or_else(|| CharacterServiceLikeCpp::from_choice_type_like_cpp(self.choice_type))
        {
            return ProductKindLikeCpp::Service(service);
        }
        match self.website_type {
            WEBSITE_TYPE_CHARACTER_BOOST_LIKE_CPP => {
                ProductKindLikeCpp::Boost(boost_definition_for_script_like_cpp(&self.script_name))
            }
            WEBSITE_TYPE_DELETED_CHARACTER_LIKE_CPP => ProductKindLikeCpp::RestoreDeletedCharacter,
            WEBSITE_TYPE_ITEM_LIKE_CPP | WEBSITE_TYPE_ITEM_MOUNT_LIKE_CPP
                if self.script_name.is_empty() && !self.items.is_empty() =>
            {
                ProductKindLikeCpp::Items
            }
            _ => ProductKindLikeCpp::Unsupported,
        }
    }

    /// The 54261 store runs its VAS character flow for these choice types.
    pub(crate) fn is_vas_like_cpp(&self) -> bool {
        matches!(
            self.choice_type,
            CHOICE_TYPE_VAS_NAME_CHANGE_LIKE_CPP
                | CHOICE_TYPE_VAS_FACTION_CHANGE_LIKE_CPP
                | CHOICE_TYPE_VAS_APPEARANCE_CHANGE_LIKE_CPP
                | CHOICE_TYPE_VAS_RACE_CHANGE_LIKE_CPP
                | CHOICE_TYPE_VAS_CHARACTER_TRANSFER_LIKE_CPP
                | CHOICE_TYPE_VAS_FACTION_TRANSFER_LIKE_CPP
        )
    }

    /// LegionCore `ProductFilter` without a player (`BattlePayMgr.cpp:481-545`):
    /// only services, boosts, pets, game time, premade/premium characters and
    /// mounts are listed at character select (game time, pets and premade
    /// characters are not ported).
    pub(crate) fn listed_at_glue_like_cpp(&self) -> bool {
        match self.kind_like_cpp() {
            ProductKindLikeCpp::Items => self.website_type == WEBSITE_TYPE_ITEM_MOUNT_LIKE_CPP,
            ProductKindLikeCpp::Unsupported => false,
            _ => true,
        }
    }
}
