//! SQLx-free contract of C++ `WorldSession::HandleCharRaceOrFactionChangeCallback`
//! (`Handlers/CharacterHandler.cpp:2030-2558`, TDB343.24081).
//!
//! `wow-world` validates the request and computes every converted id (achievements,
//! items, quests, spells, reputations, titles, taxi mask) from the process catalogs;
//! the adapter turns this owned request into the single Character DB transaction C++
//! builds, in the same statement order.

use crate::CharacterCustomizationPersistenceLikeCpp;

/// The `CharacterCache` fields and `CHAR_SEL_CHAR_RACE_OR_FACTION_CHANGE_INFOS`
/// columns the callback reads before building its transaction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CharacterRaceOrFactionChangeCandidateLikeCpp {
    pub name: String,
    pub race: u8,
    pub class: u8,
    pub level: u8,
    pub sex: u8,
    /// `characters.at_login`.
    pub at_login_flags: u16,
    /// `characters.knownTitles`.
    pub known_titles: String,
    /// `group_member.guid` (the group's db store id), 0 when not grouped.
    pub group_id: u32,
    /// `guild_member.guildid`, 0 when guildless.
    pub guild_id: u64,
    /// `guild.leaderguid` of that guild (0 without guild).
    pub guild_leader_guid: u64,
}

/// C++ `Guild::DeleteMember(trans, guid, false, false, true)` for an offline member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterGuildRemovalLikeCpp {
    pub guild_id: u64,
    /// The character leads the guild: `_SetLeader` on the lowest-rank member, or
    /// `Disband` when it is the only member.
    pub is_leader: bool,
}

/// Faction-only part of the transaction (`if (factionChangeInfo->FactionChange)`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CharacterFactionChangeCommitLikeCpp {
    /// `CHAR_UPD_CHAR_TAXIMASK` value, only when `level > 7`.
    pub taximask: Option<String>,
    pub guild: Option<CharacterGuildRemovalLikeCpp>,
    /// `CHAR_DEL_CHAR_SOCIAL_BY_GUID` / `_BY_FRIEND` (no two-side-add-friend permission).
    pub delete_social: bool,
    /// Homebind and position: `(map, zone, x, y, z)`.
    pub homebind: (u16, u16, f32, f32, f32),
    /// `(new achievement, old achievement)` per `FactionChangeAchievements` pair.
    pub achievements: Vec<(u32, u32)>,
    /// `(old item, new item)` per `FactionChangeItems*` pair of the new team.
    pub items: Vec<(u32, u32)>,
    /// `(new quest, old quest)` per `FactionChangeQuests` pair.
    pub quests: Vec<(u32, u32)>,
    /// Quests whose `AllowableRaces` exclude the new team.
    pub disabled_quests: Vec<u32>,
    /// `(new spell, old spell)` per `FactionChangeSpells` pair.
    pub spells: Vec<(u32, u32)>,
    /// `(new faction, new standing, old faction)` for every pair with an old row.
    pub reputations: Vec<(u32, i32, u32)>,
    /// Final `knownTitles` when the title loop reached the update.
    pub known_titles: Option<String>,
}

/// Everything C++ appends to the race/faction change transaction.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CharacterRaceOrFactionChangeCommitLikeCpp {
    pub guid: u64,
    pub name: String,
    /// `(atLoginFlags | AT_LOGIN_RESURRECT) & ~usedLoginFlag`.
    pub at_login_flags: u16,
    pub customizations: Vec<CharacterCustomizationPersistenceLikeCpp>,
    pub race: u8,
    /// `PLAYER_EXTRA_HAS_RACE_CHANGED`.
    pub extra_flags: u16,
    /// Identity cache only (C++ `UpdateCharacterData`); not written to `characters`.
    pub sex: u8,
    /// Languages after `CHAR_DEL_CHAR_SKILL_LANGUAGES`, only when the race changed.
    pub languages: Option<Vec<u16>>,
    /// Present only for a faction change with a changed race.
    pub faction: Option<CharacterFactionChangeCommitLikeCpp>,
}
