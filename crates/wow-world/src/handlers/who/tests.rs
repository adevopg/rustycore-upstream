//! CMSG_WHO regressions against `MiscHandler.cpp:85-236`.

use super::*;
use std::sync::Arc;
use wow_constants::ServerOpcodes;
use wow_packet::WorldPacket;
use wow_packet::packets::who::WhoRequest;

use crate::session::directory::{
    PlayerDirectoryIdentityLikeCpp, PlayerDirectoryPlacementLikeCpp, PlayerRegistration,
    PlayerRegistry, PlayerSessionRegistrationLikeCpp,
};

const ALLIANCE_HUMAN: u8 = 1;
const HORDE_ORC: u8 = 2;
const CLASS_WARRIOR: u8 = 1;
const CLASS_MAGE: u8 = 8;

fn registration_like_cpp(
    guid: ObjectGuid,
    name: &str,
    race: u8,
    class: u8,
    level: u8,
    send_tx: flume::Sender<Vec<u8>>,
) -> PlayerSessionRegistrationLikeCpp {
    let (command_tx, _command_rx) = flume::bounded(1);
    PlayerSessionRegistrationLikeCpp {
        identity: PlayerDirectoryIdentityLikeCpp {
            player_name: name.to_owned(),
            account_id: guid.counter() as u32,
            battlenet_account_id: 0,
            recruiter_id: 0,
            race,
            class,
            sex: 0,
            active_expansion: 2,
        },
        placement: PlayerDirectoryPlacementLikeCpp {
            map_id: 0,
            instance_id: 0,
            position: wow_core::Position::ZERO,
            is_in_world: true,
            level,
            is_alive: true,
        },
        active_loot_rolls: Vec::new(),
        realm_send_tx: send_tx.clone(),
        send_tx,
        command_tx,
        session_phase_tx: crate::session::directory::detached_session_phase_rail_like_cpp(),
        durable_creature_runtime_commands_like_cpp: Default::default(),
        client_visible_guids_like_cpp: Default::default(),
        client_visible_transports_like_cpp: Default::default(),
        advanced_combat_logging_enabled_like_cpp: Default::default(),
        visibility_refresh_pending_like_cpp: Default::default(),
    }
}

fn register_online_like_cpp(
    registry: &PlayerRegistry,
    counter: i64,
    name: &str,
    race: u8,
    class: u8,
    level: u8,
) -> ObjectGuid {
    let guid = ObjectGuid::create_player(0, counter);
    let (send_tx, _send_rx) = flume::bounded(8);
    registry.register_or_replace(
        guid,
        registration_like_cpp(guid, name, race, class, level, send_tx),
        Default::default(),
    );
    guid
}

fn configure_canonical_like_cpp(
    registry: &PlayerRegistry,
    guid: ObjectGuid,
    configure: impl FnOnce(&mut wow_entities::Player),
) {
    let canonical = registry
        .fixture_canonical_map_manager_like_cpp()
        .expect("canonical player fixture manager");
    let mut manager = canonical.lock().unwrap();
    let map = manager.create_world_map(0, 0).map_mut();
    configure(map.get_typed_player_mut(guid).expect("canonical player"));
}

fn make_session(security: u8) -> (WorldSession, flume::Receiver<Vec<u8>>, Arc<PlayerRegistry>) {
    let (_pkt_tx, pkt_rx) = flume::bounded(8);
    let (send_tx, send_rx) = flume::bounded(8);
    let mut session = WorldSession::new(
        1,
        "WhoTest".into(),
        security,
        2,
        9,
        54261,
        vec![0; 40],
        "enUS".into(),
        pkt_rx,
        send_tx.clone(),
    );
    let my_guid = ObjectGuid::create_player(0, 1);
    session.set_player_guid(Some(my_guid));
    session.set_loaded_player_identity_like_cpp(0, ALLIANCE_HUMAN, CLASS_WARRIOR, 80, 0);
    let registry = Arc::new(PlayerRegistry::with_canonical_player_fixtures_like_cpp());
    registry.register_or_replace(
        my_guid,
        registration_like_cpp(
            my_guid,
            "Whotest",
            ALLIANCE_HUMAN,
            CLASS_WARRIOR,
            80,
            send_tx,
        ),
        Default::default(),
    );
    session.set_player_registry(Arc::clone(&registry));
    (session, send_rx, registry)
}

fn any_request_like_cpp() -> WhoRequestPkt {
    WhoRequestPkt {
        request: WhoRequest {
            min_level: 0,
            max_level: 100,
            race_filter: -1,
            class_filter: -1,
            ..WhoRequest::default()
        },
        request_id: 0x77,
        origin: 1,
        is_from_addon: false,
        areas: Vec::new(),
    }
}

/// Parse SMSG_WHO back into `(request_id, names, area_ids, is_gm)`.
fn parse_response_like_cpp(bytes: &[u8]) -> (u32, Vec<(String, i32, bool)>) {
    assert_eq!(
        u16::from_le_bytes([bytes[0], bytes[1]]),
        ServerOpcodes::Who as u16
    );
    let mut body = WorldPacket::from_bytes(&bytes[2..]);
    let request_id = body.read_uint32().unwrap();
    let count = body.read_bits(6).unwrap() as usize;
    body.reset_bits();
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        body.read_bit().unwrap();
        let name_len = body.read_bits(6).unwrap() as usize;
        for _ in 0..5 {
            assert_eq!(body.read_bits(7).unwrap(), 0);
        }
        body.read_packed_guid().unwrap();
        body.read_packed_guid().unwrap();
        body.read_packed_guid().unwrap();
        body.read_uint64().unwrap();
        body.read_uint32().unwrap();
        body.read_bytes(5).unwrap();
        let name = body.read_string(name_len).unwrap();
        body.read_packed_guid().unwrap();
        body.read_uint32().unwrap();
        let area = body.read_int32().unwrap();
        let guild_len = body.read_bits(7).unwrap() as usize;
        let is_gm = body.read_bit().unwrap();
        body.reset_bits();
        body.read_string(guild_len).unwrap();
        entries.push((name, area, is_gm));
    }
    assert!(body.is_empty());
    (request_id, entries)
}

async fn who_names(
    session: &mut WorldSession,
    send_rx: &flume::Receiver<Vec<u8>>,
    request: WhoRequestPkt,
) -> Vec<String> {
    session.handle_who(request).await;
    let bytes = send_rx.try_recv().expect("SMSG_WHO");
    let (_, entries) = parse_response_like_cpp(&bytes);
    let mut names: Vec<String> = entries.into_iter().map(|entry| entry.0).collect();
    names.sort();
    names
}

fn snapshot_like_cpp(
    guid: ObjectGuid,
    name: &str,
    race: u8,
    class: u8,
    level: u8,
) -> PlayerPresenceSnapshotLikeCpp {
    let registry = PlayerRegistry::with_canonical_player_fixtures_like_cpp();
    let (send_tx, _rx) = flume::bounded(1);
    registry.register_or_replace(
        guid,
        registration_like_cpp(guid, name, race, class, level, send_tx),
        Default::default(),
    );
    registry
        .presence_snapshot_like_cpp(guid)
        .expect("presence snapshot")
}

fn filter_like_cpp() -> WhoFilterLikeCpp {
    WhoFilterLikeCpp::from_request_like_cpp(&any_request_like_cpp())
}

fn observer_like_cpp(security: u8) -> WhoObserverLikeCpp {
    WhoObserverLikeCpp {
        guid: ObjectGuid::create_player(0, 1),
        race: ALLIANCE_HUMAN,
        security,
    }
}

// ── registration ─────────────────────────────────────────────────────────────

#[test]
fn who_dispatch_metadata_matches_cpp_opcodes_row() {
    let entry = inventory::iter::<PacketHandlerEntry>
        .into_iter()
        .find(|entry| entry.opcode == ClientOpcodes::Who)
        .expect("Who handler entry");

    assert_eq!(entry.status, SessionStatus::LoggedIn);
    assert_eq!(entry.processing, PacketProcessing::ThreadSafe);
    assert_eq!(entry.handler_name, "handle_who");
}

// ── race mask ────────────────────────────────────────────────────────────────

#[test]
fn race_bits_follow_cpp_get_race_bit_table() {
    assert_eq!(race_bit_like_cpp(1), Some(0));
    assert_eq!(race_bit_like_cpp(11), Some(10));
    assert_eq!(race_bit_like_cpp(22), Some(21));
    assert_eq!(race_bit_like_cpp(32), Some(31));
    assert_eq!(race_bit_like_cpp(34), Some(11));
    assert_eq!(race_bit_like_cpp(35), Some(12));
    assert_eq!(race_bit_like_cpp(36), Some(13));
    assert_eq!(race_bit_like_cpp(37), Some(14));
    assert_eq!(race_bit_like_cpp(70), Some(15));
    assert_eq!(race_bit_like_cpp(52), Some(16));
    assert_eq!(race_bit_like_cpp(0), None);
    assert_eq!(race_bit_like_cpp(23), None);
}

#[test]
fn race_mask_all_ones_has_every_valid_race_and_rejects_invalid_ones() {
    assert!(race_mask_has_race_like_cpp(-1, HORDE_ORC));
    assert!(race_mask_has_race_like_cpp(1 << 1, HORDE_ORC));
    assert!(!race_mask_has_race_like_cpp(1 << 0, HORDE_ORC));
    assert!(!race_mask_has_race_like_cpp(-1, 23));
}

// ── filter derivation ────────────────────────────────────────────────────────

#[test]
fn filter_widens_max_level_at_or_above_cpp_max_level_and_lowers_strings() {
    let mut packet = any_request_like_cpp();
    packet.request.max_level = 123;
    packet.request.name = "JaInA".into();
    packet.request.guild = "Kirin TOR".into();
    packet.request.words = vec!["MaGe".into()];
    packet.areas = vec![1519];

    let filter = WhoFilterLikeCpp::from_request_like_cpp(&packet);

    assert_eq!(filter.max_level, 255);
    assert_eq!(filter.name, "jaina");
    assert_eq!(filter.guild, "kirin tor");
    assert_eq!(filter.words, vec!["mage".to_owned()]);
    assert_eq!(filter.areas, vec![1519]);
}

#[test]
fn filter_keeps_max_level_below_cpp_max_level() {
    let mut packet = any_request_like_cpp();
    packet.request.max_level = 122;
    assert_eq!(
        WhoFilterLikeCpp::from_request_like_cpp(&packet).max_level,
        122
    );
}

// ── per-row gates ────────────────────────────────────────────────────────────

#[test]
fn matching_target_passes_every_gate() {
    let target = snapshot_like_cpp(
        ObjectGuid::create_player(0, 2),
        "Jaina",
        ALLIANCE_HUMAN,
        CLASS_MAGE,
        80,
    );
    assert!(who_target_matches_like_cpp(
        &filter_like_cpp(),
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
}

#[test]
fn enemy_faction_is_hidden_without_two_side_who_list() {
    let target = snapshot_like_cpp(
        ObjectGuid::create_player(0, 2),
        "Thrall",
        HORDE_ORC,
        CLASS_WARRIOR,
        80,
    );
    assert!(!who_target_matches_like_cpp(
        &filter_like_cpp(),
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
}

#[test]
fn level_range_is_inclusive_on_both_ends() {
    let target = snapshot_like_cpp(
        ObjectGuid::create_player(0, 2),
        "Jaina",
        ALLIANCE_HUMAN,
        CLASS_MAGE,
        40,
    );
    let mut filter = filter_like_cpp();
    filter.min_level = 40;
    filter.max_level = 40;
    assert!(who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
    filter.min_level = 41;
    assert!(!who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
    filter.min_level = 0;
    filter.max_level = 39;
    assert!(!who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
}

#[test]
fn class_mask_filters_and_negative_mask_means_any_class() {
    let target = snapshot_like_cpp(
        ObjectGuid::create_player(0, 2),
        "Jaina",
        ALLIANCE_HUMAN,
        CLASS_MAGE,
        80,
    );
    let mut filter = filter_like_cpp();
    filter.class_filter = 1 << CLASS_MAGE;
    assert!(who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
    filter.class_filter = 1 << CLASS_WARRIOR;
    assert!(!who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
    filter.class_filter = -1;
    assert!(who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
}

#[test]
fn race_mask_filters_same_faction_races() {
    // Dwarf (3) is Alliance like the observer, so only the race mask decides.
    let target = snapshot_like_cpp(
        ObjectGuid::create_player(0, 2),
        "Muradin",
        3,
        CLASS_WARRIOR,
        80,
    );
    let mut filter = filter_like_cpp();
    filter.race_filter = 1 << 2;
    assert!(who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
    filter.race_filter = 1 << 0;
    assert!(!who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
}

#[test]
fn name_and_words_match_case_insensitive_substrings() {
    let target = snapshot_like_cpp(
        ObjectGuid::create_player(0, 2),
        "Jainaproudmoore",
        ALLIANCE_HUMAN,
        CLASS_MAGE,
        80,
    );
    let mut filter = filter_like_cpp();
    filter.name = "proud".into();
    assert!(who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
    filter.name = "thrall".into();
    assert!(!who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));

    filter.name.clear();
    filter.words = vec![String::new(), "moore".into()];
    assert!(who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
    filter.words = vec!["orgrimmar".into()];
    assert!(!who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
    // An all-empty word list (client sends empty words) hides everyone, as in C++.
    filter.words = vec![String::new()];
    assert!(!who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
}

#[test]
fn guild_filter_matches_only_a_supplied_guild_name() {
    let target = snapshot_like_cpp(
        ObjectGuid::create_player(0, 2),
        "Jaina",
        ALLIANCE_HUMAN,
        CLASS_MAGE,
        80,
    );
    let mut filter = filter_like_cpp();
    filter.guild = "kirin".into();
    assert!(who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        "kirin tor"
    ));
    // Represented gap: rows carry no guild name, so a guild filter matches nobody.
    assert!(!who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
}

#[test]
fn gm_invisible_target_is_hidden_from_players_but_shown_to_higher_gms_and_to_itself() {
    let mut target = snapshot_like_cpp(
        ObjectGuid::create_player(0, 2),
        "Gamemaster",
        ALLIANCE_HUMAN,
        CLASS_MAGE,
        80,
    );
    target.is_gm_visible = false;
    let filter = filter_like_cpp();

    assert!(!who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(0),
        &target,
        1,
        ""
    ));
    assert!(who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(1),
        &target,
        1,
        ""
    ));
    assert!(!who_target_matches_like_cpp(
        &filter,
        &observer_like_cpp(1),
        &target,
        2,
        ""
    ));

    let self_observer = WhoObserverLikeCpp {
        guid: target.guid,
        race: ALLIANCE_HUMAN,
        security: 0,
    };
    assert!(who_target_matches_like_cpp(
        &filter,
        &self_observer,
        &target,
        1,
        ""
    ));
}

#[test]
fn target_above_gm_level_in_who_list_is_hidden() {
    let target = snapshot_like_cpp(
        ObjectGuid::create_player(0, 2),
        "Console",
        ALLIANCE_HUMAN,
        CLASS_MAGE,
        80,
    );
    assert!(!who_target_matches_like_cpp(
        &filter_like_cpp(),
        &observer_like_cpp(3),
        &target,
        GM_LEVEL_IN_WHO_LIST_LIKE_CPP + 1,
        ""
    ));
}

#[test]
fn loading_player_is_not_in_the_who_list() {
    let mut target = snapshot_like_cpp(
        ObjectGuid::create_player(0, 2),
        "Jaina",
        ALLIANCE_HUMAN,
        CLASS_MAGE,
        80,
    );
    target.is_in_world = false;
    assert!(!who_target_matches_like_cpp(
        &filter_like_cpp(),
        &observer_like_cpp(0),
        &target,
        0,
        ""
    ));
}

#[test]
fn who_entry_projects_player_guid_lookup_data_and_gm_bit() {
    let mut target = snapshot_like_cpp(
        ObjectGuid::create_player(0, 2),
        "Jaina",
        ALLIANCE_HUMAN,
        CLASS_MAGE,
        80,
    );
    target.zone_id = 4395;
    target.is_game_master = true;
    let entry = who_entry_like_cpp(&target, 0x01000001);

    assert_eq!(entry.player_data.name, "Jaina");
    assert_eq!(entry.player_data.guid_actual, target.guid);
    assert_eq!(entry.player_data.level, 80);
    assert_eq!(entry.player_data.class, CLASS_MAGE);
    assert_eq!(entry.player_data.virtual_realm_address, 0x01000001);
    assert_eq!(
        entry.player_data.account_id,
        ObjectGuid::new((HighGuid::WowAccount as i64) << 58, 2)
    );
    assert_eq!(entry.area_id, 4395);
    assert!(entry.is_gm);
    assert_eq!(entry.guild_guid, ObjectGuid::EMPTY);
    assert!(entry.guild_name.is_empty());
}

// ── handler ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn who_response_echoes_request_id_and_lists_same_faction_players_including_self() {
    let (mut session, send_rx, registry) = make_session(0);
    register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    register_online_like_cpp(&registry, 3, "Thrall", HORDE_ORC, CLASS_WARRIOR, 80);

    session.handle_who(any_request_like_cpp()).await;

    let bytes = send_rx.try_recv().expect("SMSG_WHO");
    let (request_id, entries) = parse_response_like_cpp(&bytes);
    assert_eq!(request_id, 0x77);
    let mut names: Vec<String> = entries.into_iter().map(|entry| entry.0).collect();
    names.sort();
    assert_eq!(names, vec!["Jaina".to_owned(), "Whotest".to_owned()]);
    assert!(send_rx.try_recv().is_err());
}

#[tokio::test]
async fn who_zone_filter_uses_the_canonical_player_zone() {
    let (mut session, send_rx, registry) = make_session(0);
    let jaina = register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    configure_canonical_like_cpp(&registry, jaina, |player| player.set_zone_id_like_cpp(4395));
    let mut request = any_request_like_cpp();
    request.areas = vec![4395];

    let names = who_names(&mut session, &send_rx, request).await;

    assert_eq!(names, vec!["Jaina".to_owned()]);
}

#[tokio::test]
async fn who_more_than_ten_areas_is_dropped_like_cpp() {
    let (mut session, send_rx, _registry) = make_session(0);
    let mut request = any_request_like_cpp();
    request.areas = vec![1; 11];

    session.handle_who(request).await;

    assert!(send_rx.try_recv().is_err());
}

#[tokio::test]
async fn who_more_than_four_words_is_dropped_like_cpp() {
    let (mut session, send_rx, _registry) = make_session(0);
    let mut request = any_request_like_cpp();
    request.request.words = vec!["a".into(); 5];

    session.handle_who(request).await;

    assert!(send_rx.try_recv().is_err());
}

#[tokio::test]
async fn who_caps_at_cpp_max_who_default() {
    let (mut session, send_rx, registry) = make_session(0);
    for counter in 2..=60 {
        register_online_like_cpp(
            &registry,
            counter,
            &format!("Player{counter}"),
            ALLIANCE_HUMAN,
            CLASS_WARRIOR,
            80,
        );
    }

    session.handle_who(any_request_like_cpp()).await;

    let bytes = send_rx.try_recv().expect("SMSG_WHO");
    let (_, entries) = parse_response_like_cpp(&bytes);
    assert_eq!(entries.len(), MAX_WHO_LIKE_CPP);
}

#[tokio::test]
async fn who_hides_gm_mode_flag_only_in_the_is_gm_bit_not_from_the_list() {
    let (mut session, send_rx, registry) = make_session(0);
    let gm = register_online_like_cpp(&registry, 2, "Gamemaster", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    configure_canonical_like_cpp(&registry, gm, |player| {
        player.set_game_master_like_cpp(true)
    });

    session.handle_who(any_request_like_cpp()).await;

    let bytes = send_rx.try_recv().expect("SMSG_WHO");
    let (_, entries) = parse_response_like_cpp(&bytes);
    let gm_entry = entries
        .iter()
        .find(|entry| entry.0 == "Gamemaster")
        .expect("GM-mode player stays listed; only IsGM is set");
    assert!(gm_entry.2);
}

#[tokio::test]
async fn who_with_name_filter_returns_only_matching_players() {
    let (mut session, send_rx, registry) = make_session(0);
    register_online_like_cpp(&registry, 2, "Jaina", ALLIANCE_HUMAN, CLASS_MAGE, 80);
    register_online_like_cpp(&registry, 3, "Anduin", ALLIANCE_HUMAN, CLASS_WARRIOR, 80);
    let mut request = any_request_like_cpp();
    request.request.name = "JAI".into();

    let names = who_names(&mut session, &send_rx, request).await;

    assert_eq!(names, vec!["Jaina".to_owned()]);
}

#[tokio::test]
async fn who_without_player_is_ignored() {
    let (mut session, send_rx, _registry) = make_session(0);
    session.set_player_guid(None);

    session.handle_who(any_request_like_cpp()).await;

    assert!(send_rx.try_recv().is_err());
}

#[test]
fn registration_token_is_carried_by_the_presence_snapshot() {
    let registry = PlayerRegistry::with_canonical_player_fixtures_like_cpp();
    let guid = ObjectGuid::create_player(0, 9);
    let (send_tx, _rx) = flume::bounded(1);
    let registration: PlayerRegistration = registry.register_or_replace(
        guid,
        registration_like_cpp(guid, "Nine", ALLIANCE_HUMAN, CLASS_MAGE, 1, send_tx),
        Default::default(),
    );
    let snapshot = registry.presence_snapshot_like_cpp(guid).expect("snapshot");
    assert_eq!(snapshot.registration, registration);
    assert!(
        registry
            .presence_snapshots_like_cpp()
            .iter()
            .any(|s| s.guid == guid)
    );
}
