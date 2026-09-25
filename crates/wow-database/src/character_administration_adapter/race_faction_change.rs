//! C++ `WorldSession::HandleCharRaceOrFactionChangeCallback` transaction
//! (`Handlers/CharacterHandler.cpp:2138-2551`, TDB343.24081), statement for
//! statement and in the same order.
//!
//! Deliberate departures, all forced by RustyCore state:
//! - `CHAR_INS_PLAYER_HOMEBIND` has seven placeholders and C++ binds six (the
//!   orientation stays unbound, i.e. NULL, which a strict-mode MariaDB rejects);
//!   the orientation of the C++ `WorldLocation` (0) is bound explicitly.
//! - RustyCore's `CHAR_UPD_CHARACTER_POSITION` also stores `instance_id`; the new
//!   position is an open-world capital, so it is 0.
//! - There is no `GuildMgr` runtime: `Guild::DeleteMember` is reproduced on the
//!   rows (new leader = lowest rank id, ties by guid; a sole leader disbands the
//!   guild, whose separate C++ transaction is folded into this one).

use wow_persistence::{
    CharacterGuildRemovalLikeCpp, CharacterRaceOrFactionChangeCandidateLikeCpp,
    CharacterRaceOrFactionChangeCommitLikeCpp,
};

use crate::result::SqlResult;
use crate::{CharStatements, PreparedStatement, SqlTransaction};

/// C++ `AT_LOGIN_RESURRECT` (`Player::OfflineResurrect`).
const AT_LOGIN_RESURRECT_LIKE_CPP: u16 = 0x100;

fn stmt(statement: CharStatements) -> PreparedStatement {
    PreparedStatement::for_statement(statement)
}

fn guid_stmt(statement: CharStatements, guid: u64) -> PreparedStatement {
    let mut prepared = stmt(statement);
    prepared.set_u64(0, guid);
    prepared
}

/// `Guild::DeleteMember(trans, guid, false, false, true)` on rows.
fn append_guild_removal_like_cpp(
    transaction: &mut SqlTransaction,
    guid: u64,
    removal: CharacterGuildRemovalLikeCpp,
    new_leader: Option<u64>,
) {
    let guild_id = removal.guild_id;
    if removal.is_leader {
        let Some(new_leader) = new_leader else {
            // `Guild::Disband`: members, guild, ranks, bank tabs, bank items (and the
            // `Item::DeleteFromDB` rows of every stored item), rights, logs.
            for statement in [
                CharStatements::DEL_GUILD_MEMBERS,
                CharStatements::DEL_GUILD,
                CharStatements::DEL_GUILD_RANKS,
                CharStatements::DEL_GUILD_BANK_TABS,
                CharStatements::DEL_GUILD_BANK_ITEM_INSTANCE_GEMS,
                CharStatements::DEL_GUILD_BANK_ITEM_INSTANCE_TRANSMOG,
                CharStatements::DEL_GUILD_BANK_ITEM_GIFTS,
                CharStatements::DEL_GUILD_BANK_ITEM_INSTANCES,
                CharStatements::DEL_GUILD_BANK_ITEMS,
                CharStatements::DEL_GUILD_BANK_RIGHTS,
                CharStatements::DEL_GUILD_BANK_EVENTLOGS,
                CharStatements::DEL_GUILD_EVENTLOGS,
            ] {
                transaction.append(guid_stmt(statement, guild_id));
            }
            return;
        };
        // `_SetLeader`: `Member::ChangeRank(GuildMaster)` then `CHAR_UPD_GUILD_LEADER`.
        let mut rank = stmt(CharStatements::UPD_GUILD_MEMBER_RANK);
        rank.set_u8(0, 0);
        rank.set_u64(1, new_leader);
        transaction.append(rank);
        let mut leader = stmt(CharStatements::UPD_GUILD_LEADER);
        leader.set_u64(0, new_leader);
        leader.set_u64(1, guild_id);
        transaction.append(leader);
    }
    transaction.append(guid_stmt(CharStatements::DEL_GUILD_MEMBER, guid));
}

/// The whole transaction. `new_guild_leader` is the `SEL_GUILD_NEW_LEADER_CANDIDATE`
/// answer, read only when the character leads its guild.
pub(crate) fn race_or_faction_change_transaction_like_cpp(
    request: &CharacterRaceOrFactionChangeCommitLikeCpp,
    new_guild_leader: Option<u64>,
) -> SqlTransaction {
    let guid = request.guid;
    let mut transaction = SqlTransaction::new();

    // Player::OfflineResurrect -> Corpse::DeleteFromDB + CHAR_UPD_ADD_AT_LOGIN_FLAG.
    transaction.append(guid_stmt(CharStatements::DEL_CORPSE, guid));
    transaction.append(guid_stmt(CharStatements::DEL_CORPSE_PHASES, guid));
    transaction.append(guid_stmt(CharStatements::DEL_CORPSE_CUSTOMIZATIONS, guid));
    let mut resurrect = stmt(CharStatements::UPD_ADD_AT_LOGIN_FLAG);
    resurrect.set_u16(0, AT_LOGIN_RESURRECT_LIKE_CPP);
    resurrect.set_u64(1, guid);
    transaction.append(resurrect);

    // Name change and at-login flags.
    let mut name = stmt(CharStatements::UPD_CHAR_NAME_AT_LOGIN);
    name.set_string(0, &request.name);
    name.set_u16(1, request.at_login_flags);
    name.set_u64(2, guid);
    transaction.append(name);
    transaction.append(guid_stmt(CharStatements::DEL_CHAR_DECLINED_NAME, guid));

    // Player::SaveCustomizations.
    transaction.append(guid_stmt(
        CharStatements::DEL_CHARACTER_CUSTOMIZATIONS,
        guid,
    ));
    for customization in &request.customizations {
        let mut insert = stmt(CharStatements::INS_CHAR_CUSTOMIZATION);
        insert.set_u64(0, guid);
        insert.set_i32(1, customization.option_id);
        insert.set_i32(2, customization.choice_id);
        transaction.append(insert);
    }

    // Race change.
    let mut race = stmt(CharStatements::UPD_CHAR_RACE);
    race.set_u8(0, request.race);
    race.set_u16(1, request.extra_flags);
    race.set_u64(2, guid);
    transaction.append(race);

    let Some(languages) = request.languages.as_ref() else {
        return transaction;
    };
    transaction.append(guid_stmt(CharStatements::DEL_CHAR_SKILL_LANGUAGES, guid));
    for language in languages {
        let mut insert = stmt(CharStatements::INS_CHAR_SKILL_LANGUAGE);
        insert.set_u64(0, guid);
        insert.set_u16(1, *language);
        transaction.append(insert);
    }

    let Some(faction) = request.faction.as_ref() else {
        return transaction;
    };
    transaction.append(guid_stmt(CharStatements::UPD_CHAR_TAXI_PATH, guid));
    if let Some(taximask) = faction.taximask.as_ref() {
        let mut mask = stmt(CharStatements::UPD_CHAR_TAXIMASK);
        mask.set_string(0, taximask);
        mask.set_u64(1, guid);
        transaction.append(mask);
    }
    if let Some(guild) = faction.guild {
        append_guild_removal_like_cpp(&mut transaction, guid, guild, new_guild_leader);
    }
    if faction.delete_social {
        transaction.append(guid_stmt(CharStatements::DEL_CHAR_SOCIAL_BY_GUID, guid));
        transaction.append(guid_stmt(CharStatements::DEL_CHAR_SOCIAL_BY_FRIEND, guid));
    }

    let (map_id, zone_id, x, y, z) = faction.homebind;
    transaction.append(guid_stmt(CharStatements::DEL_PLAYER_HOMEBIND, guid));
    let mut homebind = stmt(CharStatements::INS_PLAYER_HOMEBIND);
    homebind.set_u64(0, guid);
    homebind.set_u16(1, map_id);
    homebind.set_u16(2, zone_id);
    homebind.set_f32(3, x);
    homebind.set_f32(4, y);
    homebind.set_f32(5, z);
    homebind.set_f32(6, 0.0);
    transaction.append(homebind);
    // Player::SavePositionInDB.
    let mut position = stmt(CharStatements::UPD_CHARACTER_POSITION);
    position.set_f32(0, x);
    position.set_f32(1, y);
    position.set_f32(2, z);
    position.set_f32(3, 0.0);
    position.set_u16(4, map_id);
    position.set_u32(5, 0);
    position.set_u16(6, zone_id);
    position.set_u64(7, guid);
    transaction.append(position);

    for &(new_achievement, old_achievement) in &faction.achievements {
        let mut delete = stmt(CharStatements::DEL_CHAR_ACHIEVEMENT_BY_ACHIEVEMENT);
        delete.set_u16(0, new_achievement as u16);
        delete.set_u64(1, guid);
        transaction.append(delete);
        let mut update = stmt(CharStatements::UPD_CHAR_ACHIEVEMENT);
        update.set_u32(0, u32::from(new_achievement as u16));
        update.set_u32(1, u32::from(old_achievement as u16));
        update.set_u64(2, guid);
        transaction.append(update);
    }

    for &(old_item, new_item) in &faction.items {
        let mut update = stmt(CharStatements::UPD_CHAR_INVENTORY_FACTION_CHANGE);
        update.set_u32(0, new_item);
        update.set_u32(1, old_item);
        update.set_u64(2, guid);
        transaction.append(update);
    }

    transaction.append(guid_stmt(CharStatements::DEL_CHAR_QUESTSTATUS, guid));
    for &(new_quest, old_quest) in &faction.quests {
        let mut delete = stmt(CharStatements::DEL_CHAR_QUESTSTATUS_REWARDED_BY_QUEST);
        delete.set_u64(0, guid);
        delete.set_u32(1, new_quest);
        transaction.append(delete);
        let mut update = stmt(CharStatements::UPD_CHAR_QUESTSTATUS_REWARDED_FACTION_CHANGE);
        update.set_u32(0, new_quest);
        update.set_u32(1, old_quest);
        update.set_u64(2, guid);
        transaction.append(update);
    }
    transaction.append(guid_stmt(
        CharStatements::UPD_CHAR_QUESTSTATUS_REWARDED_ACTIVE,
        guid,
    ));
    for &quest in &faction.disabled_quests {
        let mut update = stmt(CharStatements::UPD_CHAR_QUESTSTATUS_REWARDED_ACTIVE_BY_QUEST);
        update.set_u64(0, guid);
        update.set_u32(1, quest);
        transaction.append(update);
    }

    for &(new_spell, old_spell) in &faction.spells {
        let mut delete = stmt(CharStatements::DEL_CHAR_SPELL_BY_SPELL);
        delete.set_u32(0, new_spell);
        delete.set_u64(1, guid);
        transaction.append(delete);
        let mut update = stmt(CharStatements::UPD_CHAR_SPELL_FACTION_CHANGE);
        update.set_u32(0, new_spell);
        update.set_u32(1, old_spell);
        update.set_u64(2, guid);
        transaction.append(update);
    }

    for &(new_faction, new_standing, old_faction) in &faction.reputations {
        let mut delete = stmt(CharStatements::DEL_CHAR_REP_BY_FACTION);
        delete.set_u32(0, new_faction);
        delete.set_u64(1, guid);
        transaction.append(delete);
        let mut update = stmt(CharStatements::UPD_CHAR_REP_FACTION_CHANGE);
        update.set_u16(0, new_faction as u16);
        update.set_i32(1, new_standing);
        update.set_u16(2, old_faction as u16);
        update.set_u64(3, guid);
        transaction.append(update);
    }

    if let Some(known_titles) = faction.known_titles.as_ref() {
        let mut update = stmt(CharStatements::UPD_CHAR_TITLES_FACTION_CHANGE);
        update.set_string(0, known_titles);
        update.set_u64(1, guid);
        transaction.append(update);
        transaction.append(guid_stmt(
            CharStatements::RES_CHAR_TITLES_FACTION_CHANGE,
            guid,
        ));
    }
    transaction
}

/// Decode `SEL_CHAR_RACE_OR_FACTION_CHANGE_CACHE` + `..._INFOS` rows.
pub(crate) fn candidate_from_rows_like_cpp(
    cache: &SqlResult,
    infos: &SqlResult,
) -> CharacterRaceOrFactionChangeCandidateLikeCpp {
    use crate::battle_pay_adapter::column_u64_like_cpp as column;
    CharacterRaceOrFactionChangeCandidateLikeCpp {
        name: cache.read_string(0),
        race: column(cache, 1) as u8,
        class: column(cache, 2) as u8,
        level: column(cache, 3) as u8,
        sex: column(cache, 4) as u8,
        guild_id: column(cache, 5),
        guild_leader_guid: column(cache, 6),
        at_login_flags: column(infos, 0) as u16,
        known_titles: infos.read_string(1),
        group_id: column(infos, 2) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StatementDef;
    use wow_persistence::{
        CharacterCustomizationPersistenceLikeCpp, CharacterFactionChangeCommitLikeCpp,
    };

    fn race_only() -> CharacterRaceOrFactionChangeCommitLikeCpp {
        CharacterRaceOrFactionChangeCommitLikeCpp {
            guid: 7,
            name: "Newname".into(),
            at_login_flags: 0x100,
            customizations: vec![CharacterCustomizationPersistenceLikeCpp {
                option_id: 1,
                choice_id: 2,
            }],
            race: 3,
            extra_flags: 0x200,
            sex: 0,
            languages: Some(vec![98, 111]),
            faction: None,
        }
    }

    #[test]
    fn race_change_resurrects_renames_customizes_and_switches_languages_in_cpp_order() {
        let transaction = race_or_faction_change_transaction_like_cpp(&race_only(), None);
        assert_eq!(
            transaction.sqls_for_test(),
            vec![
                CharStatements::DEL_CORPSE.sql(),
                CharStatements::DEL_CORPSE_PHASES.sql(),
                CharStatements::DEL_CORPSE_CUSTOMIZATIONS.sql(),
                CharStatements::UPD_ADD_AT_LOGIN_FLAG.sql(),
                CharStatements::UPD_CHAR_NAME_AT_LOGIN.sql(),
                CharStatements::DEL_CHAR_DECLINED_NAME.sql(),
                CharStatements::DEL_CHARACTER_CUSTOMIZATIONS.sql(),
                CharStatements::INS_CHAR_CUSTOMIZATION.sql(),
                CharStatements::UPD_CHAR_RACE.sql(),
                CharStatements::DEL_CHAR_SKILL_LANGUAGES.sql(),
                CharStatements::INS_CHAR_SKILL_LANGUAGE.sql(),
                CharStatements::INS_CHAR_SKILL_LANGUAGE.sql(),
            ]
        );
    }

    #[test]
    fn same_race_skips_languages_and_team_conversion() {
        let mut request = race_only();
        request.languages = None;
        request.faction = Some(CharacterFactionChangeCommitLikeCpp::default());
        let transaction = race_or_faction_change_transaction_like_cpp(&request, None);
        assert_eq!(transaction.len(), 9);
    }

    #[test]
    fn faction_change_converts_team_data_and_moves_to_the_new_capital() {
        let mut request = race_only();
        request.faction = Some(CharacterFactionChangeCommitLikeCpp {
            taximask: Some("1 2 ".into()),
            guild: Some(CharacterGuildRemovalLikeCpp {
                guild_id: 5,
                is_leader: false,
            }),
            delete_social: true,
            homebind: (1, 1637, 1633.33, -4439.11, 15.7588),
            achievements: vec![(10, 11)],
            items: vec![(20, 21)],
            quests: vec![(30, 31)],
            disabled_quests: vec![40],
            spells: vec![(50, 51)],
            reputations: vec![(60, 100, 61)],
            known_titles: Some("0 0 ".into()),
        });
        let transaction = race_or_faction_change_transaction_like_cpp(&request, None);
        let sqls = transaction.sqls_for_test();
        let tail: Vec<&str> = sqls[12..].to_vec();
        assert_eq!(
            tail,
            vec![
                CharStatements::UPD_CHAR_TAXI_PATH.sql(),
                CharStatements::UPD_CHAR_TAXIMASK.sql(),
                CharStatements::DEL_GUILD_MEMBER.sql(),
                CharStatements::DEL_CHAR_SOCIAL_BY_GUID.sql(),
                CharStatements::DEL_CHAR_SOCIAL_BY_FRIEND.sql(),
                CharStatements::DEL_PLAYER_HOMEBIND.sql(),
                CharStatements::INS_PLAYER_HOMEBIND.sql(),
                CharStatements::UPD_CHARACTER_POSITION.sql(),
                CharStatements::DEL_CHAR_ACHIEVEMENT_BY_ACHIEVEMENT.sql(),
                CharStatements::UPD_CHAR_ACHIEVEMENT.sql(),
                CharStatements::UPD_CHAR_INVENTORY_FACTION_CHANGE.sql(),
                CharStatements::DEL_CHAR_QUESTSTATUS.sql(),
                CharStatements::DEL_CHAR_QUESTSTATUS_REWARDED_BY_QUEST.sql(),
                CharStatements::UPD_CHAR_QUESTSTATUS_REWARDED_FACTION_CHANGE.sql(),
                CharStatements::UPD_CHAR_QUESTSTATUS_REWARDED_ACTIVE.sql(),
                CharStatements::UPD_CHAR_QUESTSTATUS_REWARDED_ACTIVE_BY_QUEST.sql(),
                CharStatements::DEL_CHAR_SPELL_BY_SPELL.sql(),
                CharStatements::UPD_CHAR_SPELL_FACTION_CHANGE.sql(),
                CharStatements::DEL_CHAR_REP_BY_FACTION.sql(),
                CharStatements::UPD_CHAR_REP_FACTION_CHANGE.sql(),
                CharStatements::UPD_CHAR_TITLES_FACTION_CHANGE.sql(),
                CharStatements::RES_CHAR_TITLES_FACTION_CHANGE.sql(),
            ]
        );
    }

    #[test]
    fn guild_leader_hands_over_or_disbands_like_guild_delete_member() {
        let mut transaction = SqlTransaction::new();
        let removal = CharacterGuildRemovalLikeCpp {
            guild_id: 5,
            is_leader: true,
        };
        append_guild_removal_like_cpp(&mut transaction, 7, removal, Some(9));
        assert_eq!(
            transaction.sqls_for_test(),
            vec![
                CharStatements::UPD_GUILD_MEMBER_RANK.sql(),
                CharStatements::UPD_GUILD_LEADER.sql(),
                CharStatements::DEL_GUILD_MEMBER.sql(),
            ]
        );
        let mut transaction = SqlTransaction::new();
        append_guild_removal_like_cpp(&mut transaction, 7, removal, None);
        let sqls = transaction.sqls_for_test();
        assert_eq!(sqls.len(), 12);
        assert_eq!(sqls[0], CharStatements::DEL_GUILD_MEMBERS.sql());
        assert_eq!(sqls[1], CharStatements::DEL_GUILD.sql());
        assert!(
            sqls.iter()
                .position(|sql| *sql == CharStatements::DEL_GUILD_BANK_ITEM_INSTANCES.sql())
                < sqls
                    .iter()
                    .position(|sql| *sql == CharStatements::DEL_GUILD_BANK_ITEMS.sql())
        );
    }
}
