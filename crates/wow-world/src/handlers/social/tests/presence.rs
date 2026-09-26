//! `SMSG_FRIEND_STATUS` presence regressions against `SocialMgr.cpp:200-288`,
//! `CharacterHandler.cpp:1224`, `WorldSession.cpp:651` and `Player.cpp:3953-3966`.

use super::*;
use crate::session::directory::PlayerPresenceSnapshotLikeCpp;

fn snapshot_like_cpp(
    counter: i64,
    name: &str,
    race: u8,
    class: u8,
    level: u8,
) -> PlayerPresenceSnapshotLikeCpp {
    let registry = PlayerRegistry::with_canonical_player_fixtures_like_cpp();
    let (guid, _rx) = register_online_like_cpp(&registry, counter, name, race, class, level);
    registry
        .presence_snapshot_like_cpp(guid)
        .expect("presence snapshot")
}

fn observer_like_cpp(security: u8) -> SocialObserverLikeCpp {
    SocialObserverLikeCpp {
        guid: ObjectGuid::create_player(0, SELF_COUNTER),
        race: ALLIANCE_HUMAN,
        security,
        account_id: 1,
        recruiter_id: 0,
    }
}

// ── SocialMgr::GetFriendInfo ──────────────────────────────────────────────────

#[test]
fn friend_info_offline_target_is_all_zero_and_drops_the_note() {
    let info = friend_info_like_cpp(&observer_like_cpp(0), None, "raid");
    assert_eq!(info, FriendInfoLikeCpp::default());
}

#[test]
fn friend_info_online_same_team_target_reports_live_status_area_level_class_and_note() {
    let mut target = snapshot_like_cpp(2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    target.zone_id = 4395;

    let info = friend_info_like_cpp(&observer_like_cpp(0), Some(&target), "raid");

    assert_eq!(
        info,
        FriendInfoLikeCpp {
            status: FRIEND_STATUS_ONLINE_LIKE_CPP,
            area: 4395,
            level: 80,
            class: u32::from(CLASS_MAGE),
            note: "raid".into(),
        }
    );
}

#[test]
fn friend_info_dnd_wins_over_afk_and_neither_carries_raf() {
    let mut target = snapshot_like_cpp(2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    target.is_afk = true;
    assert_eq!(
        friend_info_like_cpp(&observer_like_cpp(0), Some(&target), "").status,
        FRIEND_STATUS_AFK_LIKE_CPP
    );
    target.is_dnd = true;
    assert_eq!(
        friend_info_like_cpp(&observer_like_cpp(0), Some(&target), "").status,
        FRIEND_STATUS_DND_LIKE_CPP
    );
}

#[test]
fn friend_info_sets_raf_bit_for_either_recruiter_relation() {
    let mut target = snapshot_like_cpp(2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    let mut observer = observer_like_cpp(0);

    target.recruiter_id = observer.account_id;
    assert_eq!(
        friend_info_like_cpp(&observer, Some(&target), "").status,
        FRIEND_STATUS_ONLINE_LIKE_CPP | FRIEND_STATUS_RAF_LIKE_CPP
    );

    target.recruiter_id = 0;
    observer.recruiter_id = target.account_id;
    assert_eq!(
        friend_info_like_cpp(&observer, Some(&target), "").status,
        FRIEND_STATUS_ONLINE_LIKE_CPP | FRIEND_STATUS_RAF_LIKE_CPP
    );
}

#[test]
fn friend_info_enemy_team_target_keeps_the_note_but_stays_offline() {
    let target = snapshot_like_cpp(2, "Thrall", HORDE_ORC, CLASS_WARRIOR, 80);

    let info = friend_info_like_cpp(&observer_like_cpp(0), Some(&target), "enemy");

    assert_eq!(
        info,
        FriendInfoLikeCpp {
            note: "enemy".into(),
            ..FriendInfoLikeCpp::default()
        }
    );
}

#[test]
fn friend_info_gm_invisible_target_is_offline_for_players_and_online_for_gms() {
    let mut target = snapshot_like_cpp(2, "Gamemaster", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    target.is_gm_visible = false;

    assert_eq!(
        friend_info_like_cpp(&observer_like_cpp(0), Some(&target), "").status,
        FRIEND_STATUS_OFFLINE_LIKE_CPP
    );
    assert_eq!(
        friend_info_like_cpp(&observer_like_cpp(1), Some(&target), "").status,
        FRIEND_STATUS_ONLINE_LIKE_CPP
    );
}

#[test]
fn is_visible_globally_for_follows_cpp_branches() {
    let mut target = snapshot_like_cpp(2, "Gamemaster", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    let observer = ObjectGuid::create_player(0, SELF_COUNTER);
    assert!(is_visible_globally_for_like_cpp(&target, 0, observer, 0));
    target.is_gm_visible = false;
    assert!(is_visible_globally_for_like_cpp(&target, 3, target.guid, 0));
    assert!(!is_visible_globally_for_like_cpp(&target, 1, observer, 0));
    assert!(is_visible_globally_for_like_cpp(&target, 1, observer, 1));
    assert!(!is_visible_globally_for_like_cpp(&target, 2, observer, 1));
}

// ── SocialMgr::BroadcastToFriendListers gates ─────────────────────────────────

#[test]
fn lister_gate_same_team_visible_player_receives() {
    let player = snapshot_like_cpp(1, "Socialtest", ALLIANCE_HUMAN, CLASS_WARRIOR, 80);
    let lister = snapshot_like_cpp(2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    assert!(friend_lister_receives_broadcast_like_cpp(
        &player, 0, &lister, 0
    ));
}

#[test]
fn lister_gate_enemy_team_lister_is_skipped() {
    let player = snapshot_like_cpp(1, "Socialtest", ALLIANCE_HUMAN, CLASS_WARRIOR, 80);
    let lister = snapshot_like_cpp(2, "Thrall", HORDE_ORC, CLASS_WARRIOR, 80);
    assert!(!friend_lister_receives_broadcast_like_cpp(
        &player, 0, &lister, 0
    ));
}

#[test]
fn lister_gate_gm_invisible_player_skips_player_listers_but_reaches_equal_or_higher_gms() {
    let mut player = snapshot_like_cpp(1, "Gamemaster", ALLIANCE_HUMAN, CLASS_WARRIOR, 80);
    player.is_gm_visible = false;
    let lister = snapshot_like_cpp(2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    assert!(!friend_lister_receives_broadcast_like_cpp(
        &player, 2, &lister, 0
    ));
    assert!(friend_lister_receives_broadcast_like_cpp(
        &player, 2, &lister, 2
    ));
    assert!(!friend_lister_receives_broadcast_like_cpp(
        &player, 2, &lister, 1
    ));
}

#[test]
fn lister_gate_player_above_gm_in_who_list_level_is_never_announced() {
    let player = snapshot_like_cpp(1, "Console", ALLIANCE_HUMAN, CLASS_WARRIOR, 80);
    let lister = snapshot_like_cpp(2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    assert!(!friend_lister_receives_broadcast_like_cpp(
        &player,
        GM_LEVEL_IN_WHO_LIST_LIKE_CPP + 1,
        &lister,
        0
    ));
}

// ── login / logout broadcast ──────────────────────────────────────────────────

#[tokio::test]
async fn login_broadcast_reaches_online_same_faction_listers_only() {
    let (mut session, my_rx, registry) = make_registered_session(0);
    let my_guid = session.player_guid().unwrap();
    configure_canonical_like_cpp(&registry, my_guid, |player| {
        player.set_zone_id_like_cpp(1519)
    });
    let (_ally, ally_rx) =
        register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    let (_enemy, enemy_rx) =
        register_online_like_cpp(&registry, 3, "Thrall", HORDE_ORC, CLASS_WARRIOR, 80);
    let port = recording_port_with_listers(Ok(vec![2, 3, 4]));
    session.set_social_persistence_port_like_cpp(port.clone());

    session
        .broadcast_friend_status_like_cpp(FriendsResult::Online)
        .await;

    let announced = parse_friend_status(&ally_rx.try_recv().expect("ally friend status"));
    assert_eq!(
        announced,
        ParsedFriendStatus {
            result: FriendsResult::Online as u8,
            guid: my_guid,
            status: FRIEND_STATUS_ONLINE_LIKE_CPP,
            area_id: 1519,
            level: 80,
            class_id: u32::from(CLASS_WARRIOR),
            notes: String::new(),
        }
    );
    assert!(ally_rx.try_recv().is_err(), "one packet per lister");
    assert!(
        enemy_rx.try_recv().is_err(),
        "enemy faction lister is gated"
    );
    assert!(
        my_rx.try_recv().is_err(),
        "broadcast never targets the announced player"
    );
    assert_eq!(port.calls.lock().unwrap().as_slice(), ["listers:1:1"]);
}

#[tokio::test]
async fn logout_broadcast_carries_offline_result_with_the_live_status_bits() {
    let (mut session, _my_rx, registry) = make_registered_session(0);
    let my_guid = session.player_guid().unwrap();
    configure_canonical_like_cpp(&registry, my_guid, |player| {
        player.set_player_flag(crate::session::PLAYER_FLAGS_AFK_LIKE_CPP);
    });
    let (_ally, ally_rx) =
        register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    session.set_social_persistence_port_like_cpp(recording_port_with_listers(Ok(vec![2])));

    session
        .broadcast_friend_status_like_cpp(FriendsResult::Offline)
        .await;

    let announced = parse_friend_status(&ally_rx.try_recv().expect("ally friend status"));
    assert_eq!(announced.result, FriendsResult::Offline as u8);
    assert_eq!(announced.guid, my_guid);
    // C++ `GetFriendInfo(_player, _player->GetGUID())` still finds the player online.
    assert_eq!(announced.status, FRIEND_STATUS_AFK_LIKE_CPP);
    assert_eq!(announced.level, 80);
}

#[tokio::test]
async fn broadcast_with_failed_reverse_lookup_sends_nothing() {
    let (mut session, _my_rx, registry) = make_registered_session(0);
    let (_ally, ally_rx) =
        register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    session.set_social_persistence_port_like_cpp(recording_port_with_listers(Err(
        "connection lost".into(),
    )));

    session
        .broadcast_friend_status_like_cpp(FriendsResult::Online)
        .await;

    assert!(ally_rx.try_recv().is_err());
}

#[tokio::test]
async fn broadcast_skips_offline_listers_and_gm_invisible_senders_for_player_listers() {
    let (mut session, _my_rx, registry) = make_registered_session(1);
    let (_ally, ally_rx) =
        register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    session.set_social_persistence_port_like_cpp(recording_port_with_listers(Ok(vec![2, 9])));
    // Offline lister 9 is simply not in the registry.
    session
        .broadcast_friend_status_like_cpp(FriendsResult::Online)
        .await;
    assert!(
        ally_rx.try_recv().is_ok(),
        "visible GM-account player is announced"
    );

    // A GM-invisible sender is represented by the pure gate (the entity keeps no
    // setter for PLAYER_EXTRA_GM_INVISIBLE); the handler-level path stays the same.
    let mut me = registry
        .presence_snapshot_like_cpp(session.player_guid().unwrap())
        .unwrap();
    me.is_gm_visible = false;
    let lister = registry
        .presence_snapshot_like_cpp(ObjectGuid::create_player(0, 2))
        .unwrap();
    assert!(!friend_lister_receives_broadcast_like_cpp(
        &me,
        session.security,
        &lister,
        0
    ));
    session.security = 0;
}

#[tokio::test]
async fn broadcast_without_player_or_port_sends_nothing() {
    let (mut session, _my_rx, registry) = make_registered_session(0);
    let (_ally, ally_rx) =
        register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);

    session
        .broadcast_friend_status_like_cpp(FriendsResult::Online)
        .await;
    assert!(
        ally_rx.try_recv().is_err(),
        "no persistence port → no listers"
    );

    session.set_social_persistence_port_like_cpp(recording_port_with_listers(Ok(vec![2])));
    session.set_player_guid(None);
    session
        .broadcast_friend_status_like_cpp(FriendsResult::Online)
        .await;
    assert!(
        ally_rx.try_recv().is_err(),
        "no player → nothing to announce"
    );
}

// ── character delete ──────────────────────────────────────────────────────────

#[tokio::test]
async fn deleted_character_notification_reaches_online_listers_with_zero_info() {
    let (session, _my_rx, registry) = make_registered_session(0);
    let (_ally, ally_rx) =
        register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    let deleted = ObjectGuid::create_player(0, 40);

    session.notify_listers_of_deleted_character_like_cpp(deleted, &[2, 9]);

    let removed = parse_friend_status(&ally_rx.try_recv().expect("lister friend status"));
    assert_eq!(
        removed,
        ParsedFriendStatus {
            result: FriendsResult::Removed as u8,
            guid: deleted,
            status: FRIEND_STATUS_OFFLINE_LIKE_CPP,
            area_id: 0,
            level: 0,
            class_id: 0,
            notes: String::new(),
        }
    );
    assert!(ally_rx.try_recv().is_err());
}

#[test]
fn deleted_character_notification_without_listers_is_silent() {
    let (session, _my_rx, registry) = make_registered_session(0);
    let (_ally, ally_rx) =
        register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);

    session.notify_listers_of_deleted_character_like_cpp(ObjectGuid::create_player(0, 40), &[]);

    assert!(ally_rx.try_recv().is_err());
}

// ── contact list / direct friend status through GetFriendInfo ─────────────────

#[tokio::test]
async fn contact_list_reports_live_status_for_online_friends_and_zeros_for_offline_ones() {
    let (mut session, my_rx, registry) = make_registered_session(0);
    let (online, _online_rx) =
        register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    configure_canonical_like_cpp(&registry, online, |player| {
        player.set_zone_id_like_cpp(4395)
    });
    let port = recording_port(
        SocialContactListLoadOutcomeLikeCpp::Loaded(vec![
            SocialContactLoadRowLikeCpp {
                friend_guid: 2,
                type_flags: 1,
                note: "online".into(),
                class_id: 1,
                level: 70,
                zone_id: 1519,
            },
            SocialContactLoadRowLikeCpp {
                friend_guid: 3,
                type_flags: 1,
                note: "offline".into(),
                class_id: 8,
                level: 60,
                zone_id: 1519,
            },
        ]),
        PersistenceOutcomeLikeCpp::Applied { rows: 1 },
    );
    session.set_social_persistence_port_like_cpp(port);

    session.send_contact_list_like_cpp(1).await;

    let bytes = my_rx.try_recv().expect("contact list");
    let mut body = wow_packet::WorldPacket::from_bytes(&bytes[2..]);
    assert_eq!(body.read_uint32().unwrap(), 1);
    assert_eq!(body.read_bits(8).unwrap(), 2);
    body.reset_bits();
    let mut contacts = Vec::new();
    for _ in 0..2 {
        let guid = body.read_packed_guid().unwrap();
        body.read_packed_guid().unwrap();
        body.read_uint32().unwrap();
        body.read_uint32().unwrap();
        body.read_uint32().unwrap();
        let status = body.read_uint8().unwrap();
        let area = body.read_int32().unwrap();
        let level = body.read_int32().unwrap();
        let class = body.read_uint32().unwrap();
        let note_len = body.read_bits(10).unwrap() as usize;
        body.read_bit().unwrap();
        body.reset_bits();
        let note = body.read_string(note_len).unwrap();
        contacts.push((guid, status, area, level, class, note));
    }
    assert!(body.is_empty());
    assert_eq!(
        contacts[0],
        (
            online,
            FRIEND_STATUS_ONLINE_LIKE_CPP,
            4395,
            80,
            u32::from(CLASS_MAGE),
            "online".to_owned()
        )
    );
    assert_eq!(
        contacts[1],
        (
            ObjectGuid::create_player(0, 3),
            FRIEND_STATUS_OFFLINE_LIKE_CPP,
            0,
            0,
            0,
            "offline".to_owned()
        )
    );
}

#[tokio::test]
async fn del_friend_status_reflects_an_online_target_like_cpp() {
    let (mut session, my_rx, registry) = make_registered_session(0);
    let (online, _online_rx) =
        register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    let port = recording_port(
        SocialContactListLoadOutcomeLikeCpp::Loaded(Vec::new()),
        PersistenceOutcomeLikeCpp::Applied { rows: 1 },
    );
    session.set_social_persistence_port_like_cpp(port.clone());

    session
        .handle_del_friend(DelFriend {
            player_guid: online,
            virtual_realm_address: 0,
        })
        .await;

    let removed = parse_friend_status(&my_rx.try_recv().expect("friend status"));
    assert_eq!(removed.result, FriendsResult::Removed as u8);
    assert_eq!(removed.guid, online);
    assert_eq!(removed.status, FRIEND_STATUS_ONLINE_LIKE_CPP);
    assert_eq!(removed.level, 80);
    assert!(removed.notes.is_empty());
    assert_eq!(port.calls.lock().unwrap().as_slice(), ["remove:1:2:Friend"]);
}

#[tokio::test]
async fn add_friend_online_visible_target_is_added_online_with_the_note() {
    let (mut session, my_rx, registry) = make_registered_session(0);
    let (online, _online_rx) =
        register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    session.set_social_persistence_port_like_cpp(recording_port_with_candidate(
        SocialAddCandidateLoadOutcomeLikeCpp::Found(wow_persistence::SocialAddCandidateLikeCpp {
            guid: 2,
            race: ALLIANCE_HUMAN,
            class_id: u32::from(CLASS_MAGE),
            level: 80,
            zone_id: 1519,
        }),
    ));

    session
        .handle_add_friend(AddFriend {
            name: "jaina".into(),
            notes: "raid".into(),
        })
        .await;

    let added = parse_friend_status(&my_rx.try_recv().expect("friend status"));
    assert_eq!(added.result, FriendsResult::AddedOnline as u8);
    assert_eq!(added.guid, online);
    assert_eq!(added.status, FRIEND_STATUS_ONLINE_LIKE_CPP);
    assert_eq!(added.class_id, u32::from(CLASS_MAGE));
    assert_eq!(added.notes, "raid");
}

#[tokio::test]
async fn add_friend_offline_target_is_added_offline_with_zero_info_and_no_note() {
    let (mut session, my_rx, _registry) = make_registered_session(0);
    session.set_social_persistence_port_like_cpp(recording_port_with_candidate(
        SocialAddCandidateLoadOutcomeLikeCpp::Found(wow_persistence::SocialAddCandidateLikeCpp {
            guid: 7,
            race: ALLIANCE_HUMAN,
            class_id: u32::from(CLASS_MAGE),
            level: 80,
            zone_id: 1519,
        }),
    ));

    session
        .handle_add_friend(AddFriend {
            name: "jaina".into(),
            notes: "raid".into(),
        })
        .await;

    let added = parse_friend_status(&my_rx.try_recv().expect("friend status"));
    assert_eq!(
        added,
        ParsedFriendStatus {
            result: FriendsResult::AddedOffline as u8,
            guid: ObjectGuid::create_player(0, 7),
            status: FRIEND_STATUS_OFFLINE_LIKE_CPP,
            area_id: 0,
            level: 0,
            class_id: 0,
            notes: String::new(),
        }
    );
}

#[tokio::test]
async fn add_friend_enemy_faction_replies_enemy_with_the_target_guid() {
    let (mut session, my_rx, _registry) = make_registered_session(0);
    session.set_social_persistence_port_like_cpp(recording_port_with_candidate(
        SocialAddCandidateLoadOutcomeLikeCpp::Found(wow_persistence::SocialAddCandidateLikeCpp {
            guid: 7,
            race: HORDE_ORC,
            class_id: 1,
            level: 80,
            zone_id: 1637,
        }),
    ));

    session
        .handle_add_friend(AddFriend {
            name: "thrall".into(),
            notes: String::new(),
        })
        .await;

    let reply = parse_friend_status(&my_rx.try_recv().expect("friend status"));
    assert_eq!(reply.result, FriendsResult::Enemy as u8);
    assert_eq!(reply.guid, ObjectGuid::create_player(0, 7));
}

#[test]
fn add_friend_and_del_friend_dispatch_metadata_match_cpp() {
    for (opcode, name) in [
        (ClientOpcodes::AddFriend, "handle_add_friend"),
        (ClientOpcodes::DelFriend, "handle_del_friend"),
        (ClientOpcodes::SendContactList, "handle_send_contact_list"),
    ] {
        let entry = inventory::iter::<PacketHandlerEntry>
            .into_iter()
            .find(|entry| entry.opcode == opcode)
            .expect("handler entry");
        assert_eq!(entry.status, SessionStatus::LoggedIn);
        assert_eq!(entry.processing, PacketProcessing::ThreadUnsafe);
        assert_eq!(entry.handler_name, name);
    }
}
