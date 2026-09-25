use super::*;

fn catalog() -> RaceFactionChangeCatalogLikeCpp {
    let outcome = FactionChangeStoreLikeCpp::from_validated_rows_like_cpp(
        [],
        [],
        [],
        [],
        [wow_data::FactionChangePairRowLikeCpp {
            alliance_id: 10,
            horde_id: 20,
        }],
        |_| true,
        |_| true,
        |_| true,
        |_| true,
        |_| true,
    );
    RaceFactionChangeCatalogLikeCpp {
        faction_change: Arc::new(outcome.store),
        race_alliance: HashMap::from([(1, 0), (2, 1), (24, 2)]),
        horde_taxi_mask: vec![2; 41],
        alliance_taxi_mask: vec![1; 41],
        // Alliance title bit 5, Horde title bit 40.
        title_mask_ids: HashMap::from([(10, 5), (20, 40)]),
        race_restricted_quests: vec![(100, 1), (101, 2), (102, u64::MAX - 1)],
        ..RaceFactionChangeCatalogLikeCpp::default()
    }
}

#[test]
fn team_comes_from_chr_races_alliance_and_unknown_races_are_neutral() {
    let catalog = catalog();
    assert_eq!(
        catalog.team_id_for_race_like_cpp(1),
        TeamIdLikeCpp::Alliance
    );
    assert_eq!(catalog.team_id_for_race_like_cpp(2), TeamIdLikeCpp::Horde);
    assert_eq!(
        catalog.team_id_for_race_like_cpp(24),
        TeamIdLikeCpp::Neutral
    );
    assert_eq!(
        catalog.team_id_for_race_like_cpp(99),
        TeamIdLikeCpp::Neutral
    );
}

#[test]
fn race_masks_match_racemask_h() {
    // Human, Dwarf, NightElf, Gnome, Draenei, Worgen, ... (RaceMask.h)
    let alliance = team_race_mask_like_cpp(TeamIdLikeCpp::Alliance);
    let horde = team_race_mask_like_cpp(TeamIdLikeCpp::Horde);
    assert_eq!(alliance & horde, 0);
    assert_ne!(alliance & 1, 0); // Human
    assert_ne!(horde & 2, 0); // Orc
    assert_ne!(horde & (1 << 9), 0); // Blood Elf
    assert_ne!(alliance & (1 << 10), 0); // Draenei
    assert_eq!((alliance | horde) & (1 << 23), 0); // Pandaren (neutral)
}

#[test]
fn languages_follow_the_cpp_switch() {
    assert_eq!(languages_like_cpp(2, TeamIdLikeCpp::Horde), Ok(vec![109]));
    assert_eq!(languages_like_cpp(1, TeamIdLikeCpp::Alliance), Ok(vec![98]));
    assert_eq!(
        languages_like_cpp(10, TeamIdLikeCpp::Horde),
        Ok(vec![109, 137])
    );
    assert_eq!(
        languages_like_cpp(11, TeamIdLikeCpp::Alliance),
        Ok(vec![98, 759])
    );
    assert_eq!(languages_like_cpp(24, TeamIdLikeCpp::Neutral), Err(()));
}

#[test]
fn names_are_normalized_like_normalize_player_name() {
    assert_eq!(
        normalize_player_name_like_cpp("nEWNAME").as_deref(),
        Some("Newname")
    );
    assert_eq!(normalize_player_name_like_cpp(""), None);
}

#[test]
fn taximask_is_the_team_mask_plus_the_death_knight_node() {
    let catalog = catalog();
    let horde = taximask_like_cpp(&catalog, TeamIdLikeCpp::Horde, 1);
    assert!(horde.starts_with("2 2 "));
    assert_eq!(horde.split(' ').filter(|v| !v.is_empty()).count(), 41);
    let dk = taximask_like_cpp(&catalog, TeamIdLikeCpp::Alliance, 6);
    assert_eq!(dk.split(' ').nth(39), Some("5"));
    assert_eq!(dk.split(' ').nth(38), Some("1"));
}

#[test]
fn titles_move_the_horde_bit_to_the_alliance_bit() {
    let catalog = catalog();
    // Horde title bit 40 = index 1, bit 8.
    let converted = convert_titles_like_cpp(&catalog, "0 256 0 ", TeamIdLikeCpp::Alliance);
    assert_eq!(converted.as_deref(), Some("32 0 0 "));
    // C++ Horde branch re-tests and re-sets the Horde bit (quirk kept).
    let converted = convert_titles_like_cpp(&catalog, "32 256 ", TeamIdLikeCpp::Horde);
    assert_eq!(converted.as_deref(), Some("32 256 "));
    // Index outside the stored masks: C++ `continue`s before the update.
    assert_eq!(
        convert_titles_like_cpp(&catalog, "7 ", TeamIdLikeCpp::Alliance),
        None
    );
    assert_eq!(
        convert_titles_like_cpp(&catalog, "", TeamIdLikeCpp::Alliance),
        None
    );
}

#[test]
fn old_faction_quests_are_the_ones_without_a_new_team_race() {
    let catalog = catalog();
    assert_eq!(
        disabled_quests_like_cpp(&catalog, TeamIdLikeCpp::Horde),
        vec![100]
    );
    assert_eq!(
        disabled_quests_like_cpp(&catalog, TeamIdLikeCpp::Alliance),
        vec![101]
    );
}

#[test]
fn pairs_are_ordered_new_team_first() {
    let catalog = catalog();
    assert_eq!(
        team_pairs_like_cpp(
            &catalog,
            FactionChangePairKindLikeCpp::Title,
            TeamIdLikeCpp::Horde
        ),
        vec![(20, 10)]
    );
}

#[test]
fn guild_removal_respects_two_side_guild_and_marks_the_leader() {
    let mut catalog = catalog();
    let candidate = CharacterRaceOrFactionChangeCandidateLikeCpp {
        guild_id: 3,
        guild_leader_guid: 7,
        ..Default::default()
    };
    assert_eq!(
        guild_removal_like_cpp(&catalog, &candidate, 7),
        Some(wow_persistence::CharacterGuildRemovalLikeCpp {
            guild_id: 3,
            is_leader: true
        })
    );
    catalog.allow_two_side_interaction_guild = true;
    assert_eq!(guild_removal_like_cpp(&catalog, &candidate, 7), None);
}
