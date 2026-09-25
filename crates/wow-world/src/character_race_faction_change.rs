//! Pure rules of C++ `WorldSession::HandleCharRaceOrFactionChangeCallback`
//! (`Handlers/CharacterHandler.cpp:2030-2558`, TDB343.24081) and the immutable
//! process catalog it reads (`sObjectMgr` faction-change maps, `sChrRacesStore`,
//! the TaxiNodes masks of `DB2Manager::LoadStores`, `sCharTitlesStore.MaskID`,
//! `ObjectMgr::GetQuestTemplates()` race masks, `sWorld` switches).
//!
//! The session handler (`handlers/character/race_faction_change.rs`) owns the
//! packet, the persistence port and the group runtime; this module has no I/O.

use std::collections::HashMap;
use std::sync::Arc;

use wow_data::progression_rewards::FactionStore;
use wow_data::{FactionChangePairKindLikeCpp, FactionChangeStoreLikeCpp};
use wow_persistence::CharacterRaceOrFactionChangeCandidateLikeCpp;

pub(crate) const AT_LOGIN_RESURRECT_LIKE_CPP: u16 = 0x100;
pub(crate) const AT_LOGIN_CHANGE_FACTION_LIKE_CPP: u16 = 0x040;
pub(crate) const AT_LOGIN_CHANGE_RACE_LIKE_CPP: u16 = 0x080;
/// C++ `PLAYER_EXTRA_HAS_RACE_CHANGED` (`Player.h:525`).
pub(crate) const PLAYER_EXTRA_HAS_RACE_CHANGED_LIKE_CPP: u16 = 0x0200;
const CLASS_DEATH_KNIGHT_LIKE_CPP: u8 = 6;

/// C++ `TeamId` (`SharedDefines.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeamIdLikeCpp {
    Alliance,
    Horde,
    Neutral,
}

/// Immutable race/faction change data, built once at startup.
#[derive(Clone, Default)]
pub struct RaceFactionChangeCatalogLikeCpp {
    pub faction_change: Arc<FactionChangeStoreLikeCpp>,
    /// `ChrRacesEntry::Alliance` by race id (0 Alliance, 1 Horde, 2 Neutral).
    pub race_alliance: HashMap<u8, i8>,
    /// `sHordeTaxiNodesMask` / `sAllianceTaxiNodesMask` (`TaxiMask::value_type = uint8`).
    pub horde_taxi_mask: Vec<u8>,
    pub alliance_taxi_mask: Vec<u8>,
    /// `CharTitlesEntry::MaskID` by title id.
    pub title_mask_ids: HashMap<u32, u32>,
    /// `(quest id, AllowableRaces)` of every quest whose mask is not `uint64(-1)`,
    /// ascending quest id.
    pub race_restricted_quests: Vec<(u32, u64)>,
    /// `sFactionStore`, for `ObjectMgr::GetBaseReputationOf`.
    pub factions: Option<Arc<FactionStore>>,
    /// `sObjectMgr->IsReservedName` store.
    pub reserved_names: Option<Arc<wow_data::ReservedNameStoreLikeCpp>>,
    /// `CONFIG_CHARACTER_CREATING_DISABLED_RACEMASK`.
    pub disabled_race_mask: u64,
    /// `CONFIG_PREVENT_RENAME_CUSTOMIZATION`.
    pub prevent_rename_customization: bool,
    /// `CONFIG_ALLOW_TWO_SIDE_INTERACTION_GUILD`.
    pub allow_two_side_interaction_guild: bool,
    /// `CONFIG_ALLOW_TWO_SIDE_INTERACTION_GROUP`.
    pub allow_two_side_interaction_group: bool,
}

impl RaceFactionChangeCatalogLikeCpp {
    /// C++ `Player::TeamIdForRace` (`TeamId(ChrRacesEntry::Alliance)`, Neutral when
    /// the race is not in `ChrRaces.db2`).
    pub(crate) fn team_id_for_race_like_cpp(&self, race: u8) -> TeamIdLikeCpp {
        match self.race_alliance.get(&race) {
            Some(0) => TeamIdLikeCpp::Alliance,
            Some(1) => TeamIdLikeCpp::Horde,
            _ => TeamIdLikeCpp::Neutral,
        }
    }
}

/// `RACEMASK_ALLIANCE` / `RACEMASK_HORDE` (`RaceMask.h`, TDB343.24081).
pub(crate) fn team_race_mask_like_cpp(team: TeamIdLikeCpp) -> u64 {
    const ALLIANCE: [u8; 13] = [1, 3, 4, 7, 11, 22, 25, 29, 30, 32, 34, 37, 52];
    const PLAYABLE: [u8; 27] = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 22, 24, 25, 26, 27, 28, 29, 30, 31, 32, 34, 35, 36, 37,
        52, 70,
    ];
    let mask = |races: &[u8]| {
        races.iter().fold(0_u64, |mask, race| {
            mask | crate::reputation::mgr::player_race_mask_like_cpp(*race).unwrap_or(0)
        })
    };
    let alliance = mask(&ALLIANCE);
    match team {
        TeamIdLikeCpp::Alliance => alliance,
        // RACEMASK_ALL_PLAYABLE & ~(RACEMASK_NEUTRAL | RACEMASK_ALLIANCE)
        TeamIdLikeCpp::Horde => mask(&PLAYABLE) & !(mask(&[24]) | alliance),
        TeamIdLikeCpp::Neutral => mask(&[24]),
    }
}

/// C++ `normalizePlayerName`: first letter upper case, the rest lower case;
/// `None` for an empty name (`CHAR_NAME_NO_NAME`).
pub(crate) fn normalize_player_name_like_cpp(name: &str) -> Option<String> {
    let mut chars = name.chars();
    let first = chars.next()?;
    Some(
        first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
    )
}

/// Race language of the `switch (factionChangeInfo->RaceID)`; `Ok(None)` for the
/// races C++ skips (Orc, Human, Mag'har), `Err(())` for its `default:` error.
pub(crate) fn race_language_like_cpp(race: u8) -> Result<Option<u16>, ()> {
    Ok(Some(match race {
        1 | 2 | 36 => return Ok(None),
        3 | 34 => 111,
        11 | 30 => 759,
        7 => 313,
        4 => 113,
        22 => 791,
        5 => 673,
        6 | 28 => 115,
        8 => 315,
        10 | 29 => 137,
        9 => 792,
        27 => 2464,
        _ => return Err(()),
    }))
}

/// Languages inserted after `CHAR_DEL_CHAR_SKILL_LANGUAGES`: faction language
/// (Orcish 109 / Common 98) then the race language.
pub(crate) fn languages_like_cpp(new_race: u8, new_team: TeamIdLikeCpp) -> Result<Vec<u16>, ()> {
    let mut languages = vec![if new_team == TeamIdLikeCpp::Horde {
        109
    } else {
        98
    }];
    languages.extend(race_language_like_cpp(new_race)?);
    Ok(languages)
}

/// `taximaskstream` of C++: every mask byte of the new team (plus the Death Knight
/// node 315 in byte 39) as a decimal followed by a space.
pub(crate) fn taximask_like_cpp(
    catalog: &RaceFactionChangeCatalogLikeCpp,
    new_team: TeamIdLikeCpp,
    class: u8,
) -> String {
    let mask = if new_team == TeamIdLikeCpp::Horde {
        &catalog.horde_taxi_mask
    } else {
        &catalog.alliance_taxi_mask
    };
    mask.iter()
        .enumerate()
        .map(|(index, byte)| {
            let death_knight_extra_node = if class != CLASS_DEATH_KNIGHT_LIKE_CPP || index != 39 {
                0
            } else {
                4
            };
            format!("{} ", u32::from(byte | death_knight_extra_node))
        })
        .collect()
}

/// Title conversion loop of C++, including its Horde-branch quirk (both branches
/// test the Horde title's `MaskID`; the Horde branch re-sets that same bit). Returns
/// the final `knownTitles` when at least one pair reached the update statements.
pub(crate) fn convert_titles_like_cpp(
    catalog: &RaceFactionChangeCatalogLikeCpp,
    known_titles: &str,
    new_team: TeamIdLikeCpp,
) -> Option<String> {
    if known_titles.is_empty() {
        return None;
    }
    let mut titles: Vec<u32> = known_titles
        .split(' ')
        .filter(|token| !token.is_empty())
        .map(|token| token.parse().unwrap_or(0))
        .collect();
    let mut touched = false;
    for (alliance_title, horde_title) in catalog
        .faction_change
        .pairs_like_cpp(FactionChangePairKindLikeCpp::Title)
    {
        // `sCharTitlesStore.AssertEntry`: the pairs were validated against the store.
        let (Some(&alliance_mask), Some(&horde_mask)) = (
            catalog.title_mask_ids.get(&alliance_title),
            catalog.title_mask_ids.get(&horde_title),
        ) else {
            continue;
        };
        let index = (horde_mask / 32) as usize;
        if index >= titles.len() {
            continue;
        }
        let old_flag = 1_u32 << (horde_mask % 32);
        let (new_mask, new_flag) = if new_team == TeamIdLikeCpp::Alliance {
            (alliance_mask, 1_u32 << (alliance_mask % 32))
        } else {
            (horde_mask, 1_u32 << (horde_mask % 32))
        };
        if titles[index] & old_flag != 0 {
            titles[index] &= !old_flag;
            if let Some(slot) = titles.get_mut((new_mask / 32) as usize) {
                *slot |= new_flag;
            }
        }
        touched = true;
    }
    touched.then(|| titles.iter().map(|mask| format!("{mask} ")).collect())
}

/// `(new, old)` ids of a pair table for the new team (`newTeamId == TEAM_ALLIANCE ?
/// alliance : horde` first).
pub(crate) fn team_pairs_like_cpp(
    catalog: &RaceFactionChangeCatalogLikeCpp,
    kind: FactionChangePairKindLikeCpp,
    new_team: TeamIdLikeCpp,
) -> Vec<(u32, u32)> {
    catalog
        .faction_change
        .pairs_like_cpp(kind)
        .into_iter()
        .map(|(alliance, horde)| {
            if new_team == TeamIdLikeCpp::Alliance {
                (alliance, horde)
            } else {
                (horde, alliance)
            }
        })
        .collect()
}

/// Quests of the old faction: `AllowableRaces != uint64(-1)` and no bit of the
/// new team's race mask.
pub(crate) fn disabled_quests_like_cpp(
    catalog: &RaceFactionChangeCatalogLikeCpp,
    new_team: TeamIdLikeCpp,
) -> Vec<u32> {
    let new_race_mask = team_race_mask_like_cpp(new_team);
    catalog
        .race_restricted_quests
        .iter()
        .filter(|(_, allowable)| *allowable != u64::MAX && allowable & new_race_mask == 0)
        .map(|(quest_id, _)| *quest_id)
        .collect()
}

/// C++ `ObjectMgr::GetBaseReputationOf` (0 for an unknown faction).
pub(crate) fn base_reputation_of_like_cpp(
    catalog: &RaceFactionChangeCatalogLikeCpp,
    faction_id: u32,
    race: u8,
    class: u8,
) -> i32 {
    catalog
        .factions
        .as_ref()
        .and_then(|store| store.get(faction_id))
        .map_or(0, |entry| {
            crate::reputation::mgr::base_reputation_like_cpp(entry, race, class)
        })
}

/// The `if (!HasPermission(RBAC_PERM_SKIP_CHECK_CHARACTER_CREATION_RACEMASK))`
/// race mask test.
pub(crate) fn race_disabled_like_cpp(catalog: &RaceFactionChangeCatalogLikeCpp, race: u8) -> bool {
    crate::reputation::mgr::player_race_mask_like_cpp(race)
        .is_some_and(|mask| catalog.disabled_race_mask & mask != 0)
}

/// Home bind of the new team: `(map, zone, x, y, z)`.
pub(crate) fn capital_homebind_like_cpp(new_team: TeamIdLikeCpp) -> (u16, u16, f32, f32, f32) {
    if new_team == TeamIdLikeCpp::Alliance {
        (0, 1519, -8867.68, 673.373, 97.9034)
    } else {
        (1, 1637, 1633.33, -4439.11, 15.7588)
    }
}

/// Whether the guild removal applies (`!CONFIG_ALLOW_TWO_SIDE_INTERACTION_GUILD`
/// and the character is in a guild).
pub(crate) fn guild_removal_like_cpp(
    catalog: &RaceFactionChangeCatalogLikeCpp,
    candidate: &CharacterRaceOrFactionChangeCandidateLikeCpp,
    guid: u64,
) -> Option<wow_persistence::CharacterGuildRemovalLikeCpp> {
    (!catalog.allow_two_side_interaction_guild && candidate.guild_id != 0).then_some(
        wow_persistence::CharacterGuildRemovalLikeCpp {
            guild_id: candidate.guild_id,
            is_leader: candidate.guild_leader_guid == guid,
        },
    )
}

/// C++ response codes of the callback (`SharedDefines.h` `ResponseCodes`).
pub(crate) const RESPONSE_SUCCESS_LIKE_CPP: u8 = 0;
pub(crate) const CHAR_CREATE_ERROR_LIKE_CPP: u8 = 25;
pub(crate) const CHAR_CREATE_NAME_IN_USE_LIKE_CPP: u8 = 27;
pub(crate) const CHAR_CREATE_RESTRICTED_RACECLASS_LIKE_CPP: u8 = 37;
pub(crate) const CHAR_CREATE_CHARACTER_SWAP_FACTION_LIKE_CPP: u8 = 42;
pub(crate) const CHAR_CREATE_CHARACTER_RACE_ONLY_LIKE_CPP: u8 = 43;
pub(crate) const CHAR_NAME_FAILURE_LIKE_CPP: u8 = 91;
pub(crate) const CHAR_NAME_NO_NAME_LIKE_CPP: u8 = 92;
pub(crate) const CHAR_NAME_RESERVED_LIKE_CPP: u8 = 98;

/// Outcome of the checks before C++ reads the name cache and builds the transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RaceFactionChangePlanLikeCpp {
    /// `normalizePlayerName` result.
    pub name: String,
    pub used_login_flag: u16,
    pub new_team: TeamIdLikeCpp,
}

/// The request fields the checks read.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RaceFactionChangeRequestLikeCpp<'a> {
    pub faction_change: bool,
    pub race: u8,
    pub name: &'a str,
}

/// C++ checks from `GetPlayerInfo` to `CheckPlayerName`, in C++ order.
/// `player_info_exists` is `sObjectMgr->GetPlayerInfo(newRace, class) != nullptr`;
/// `skip_rbac_checks` stands for `RBAC_PERM_SKIP_CHECK_CHARACTER_CREATION_RACEMASK`
/// and `..._RESERVEDNAME`.
pub(crate) fn plan_race_or_faction_change_like_cpp(
    catalog: &RaceFactionChangeCatalogLikeCpp,
    candidate: &CharacterRaceOrFactionChangeCandidateLikeCpp,
    request: RaceFactionChangeRequestLikeCpp<'_>,
    player_info_exists: bool,
    skip_rbac_checks: bool,
) -> Result<RaceFactionChangePlanLikeCpp, u8> {
    if !player_info_exists {
        return Err(CHAR_CREATE_ERROR_LIKE_CPP);
    }
    let used_login_flag = if request.faction_change {
        AT_LOGIN_CHANGE_FACTION_LIKE_CPP
    } else {
        AT_LOGIN_CHANGE_RACE_LIKE_CPP
    };
    if candidate.at_login_flags & used_login_flag == 0 {
        return Err(CHAR_CREATE_ERROR_LIKE_CPP);
    }
    let new_team = catalog.team_id_for_race_like_cpp(request.race);
    if new_team == TeamIdLikeCpp::Neutral {
        return Err(CHAR_CREATE_RESTRICTED_RACECLASS_LIKE_CPP);
    }
    if request.faction_change == (catalog.team_id_for_race_like_cpp(candidate.race) == new_team) {
        return Err(if request.faction_change {
            CHAR_CREATE_CHARACTER_SWAP_FACTION_LIKE_CPP
        } else {
            CHAR_CREATE_CHARACTER_RACE_ONLY_LIKE_CPP
        });
    }
    if !skip_rbac_checks && race_disabled_like_cpp(catalog, request.race) {
        return Err(CHAR_CREATE_ERROR_LIKE_CPP);
    }
    if catalog.prevent_rename_customization && request.name != candidate.name {
        return Err(CHAR_NAME_FAILURE_LIKE_CPP);
    }
    let Some(name) = normalize_player_name_like_cpp(request.name) else {
        return Err(CHAR_NAME_NO_NAME_LIKE_CPP);
    };
    let result =
        crate::handlers::character_rules::represented_character_rename_name_result_like_cpp(&name);
    if result != RESPONSE_SUCCESS_LIKE_CPP {
        return Err(result);
    }
    if !skip_rbac_checks
        && catalog
            .reserved_names
            .as_ref()
            .is_some_and(|names| names.is_reserved_name_like_cpp(&name))
    {
        return Err(CHAR_NAME_RESERVED_LIKE_CPP);
    }
    Ok(RaceFactionChangePlanLikeCpp {
        name,
        used_login_flag,
        new_team,
    })
}

/// `(new faction, old faction)` of every `FactionChangeReputation` pair; C++ reads
/// the old standing of each with `CHAR_SEL_CHAR_REP_BY_FACTION`.
pub(crate) fn reputation_pairs_like_cpp(
    catalog: &RaceFactionChangeCatalogLikeCpp,
    new_team: TeamIdLikeCpp,
) -> Vec<(u32, u32)> {
    team_pairs_like_cpp(catalog, FactionChangePairKindLikeCpp::Reputation, new_team)
}

/// Everything after the checks: the owned transaction request. `old_standings`
/// holds the `(old faction, standing)` rows that exist. `Err` is the C++ language
/// `default:` error, sent before anything is committed.
pub(crate) fn build_commit_like_cpp(
    catalog: &RaceFactionChangeCatalogLikeCpp,
    guid: u64,
    candidate: &CharacterRaceOrFactionChangeCandidateLikeCpp,
    change: RaceFactionChangeCommitInputLikeCpp,
    plan: &RaceFactionChangePlanLikeCpp,
    old_standings: &HashMap<u32, i32>,
) -> Result<wow_persistence::CharacterRaceOrFactionChangeCommitLikeCpp, u8> {
    let new_team = plan.new_team;
    let mut commit = wow_persistence::CharacterRaceOrFactionChangeCommitLikeCpp {
        guid,
        name: plan.name.clone(),
        at_login_flags: (candidate.at_login_flags | AT_LOGIN_RESURRECT_LIKE_CPP)
            & !plan.used_login_flag,
        customizations: change.customizations,
        race: change.race,
        extra_flags: PLAYER_EXTRA_HAS_RACE_CHANGED_LIKE_CPP,
        sex: change.sex,
        languages: None,
        faction: None,
    };
    if candidate.race == change.race {
        return Ok(commit);
    }
    commit.languages =
        Some(languages_like_cpp(change.race, new_team).map_err(|()| CHAR_CREATE_ERROR_LIKE_CPP)?);
    if !change.faction_change {
        return Ok(commit);
    }
    let reputations = reputation_pairs_like_cpp(catalog, new_team)
        .into_iter()
        .filter_map(|(new_faction, old_faction)| {
            let old_db = *old_standings.get(&old_faction)?;
            let old_base =
                base_reputation_of_like_cpp(catalog, old_faction, candidate.race, candidate.class);
            let new_base =
                base_reputation_of_like_cpp(catalog, new_faction, change.race, candidate.class);
            Some((new_faction, old_db + old_base - new_base, old_faction))
        })
        .collect();
    commit.faction = Some(wow_persistence::CharacterFactionChangeCommitLikeCpp {
        taximask: (candidate.level > 7)
            .then(|| taximask_like_cpp(catalog, new_team, candidate.class)),
        guild: guild_removal_like_cpp(catalog, candidate, guid),
        // RustyCore has no RBAC runtime: a player never holds
        // RBAC_PERM_TWO_SIDE_ADD_FRIEND (see handlers/social.rs AddFriend).
        delete_social: true,
        homebind: capital_homebind_like_cpp(new_team),
        achievements: team_pairs_like_cpp(
            catalog,
            FactionChangePairKindLikeCpp::Achievement,
            new_team,
        ),
        items: catalog
            .faction_change
            .item_conversion_like_cpp(new_team == TeamIdLikeCpp::Alliance),
        quests: team_pairs_like_cpp(catalog, FactionChangePairKindLikeCpp::Quest, new_team),
        disabled_quests: disabled_quests_like_cpp(catalog, new_team),
        spells: team_pairs_like_cpp(catalog, FactionChangePairKindLikeCpp::Spell, new_team),
        reputations,
        known_titles: convert_titles_like_cpp(catalog, &candidate.known_titles, new_team),
    });
    Ok(commit)
}

/// Request fields copied into the transaction.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RaceFactionChangeCommitInputLikeCpp {
    pub faction_change: bool,
    pub race: u8,
    pub sex: u8,
    pub customizations: Vec<wow_persistence::CharacterCustomizationPersistenceLikeCpp>,
}

#[cfg(test)]
#[path = "character_race_faction_change/tests.rs"]
mod tests;
