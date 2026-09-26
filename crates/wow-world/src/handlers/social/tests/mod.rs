//! Social handler regressions.
//!
//! Separated from social.rs under #685.

use super::*;
use num_traits::ToPrimitive;
use std::sync::{Arc, Mutex};
use wow_constants::ServerOpcodes;
use wow_persistence::{
    PersistenceFutureLikeCpp, SocialAddCandidateLoadOutcomeLikeCpp, SocialContactLoadRowLikeCpp,
    SocialPartyInviteLookupOutcomeLikeCpp, SocialPersistencePortLikeCpp,
    SocialRelationshipStateLikeCpp,
};

use crate::session::directory::{
    PlayerDirectoryIdentityLikeCpp, PlayerDirectoryPlacementLikeCpp, PlayerRegistry,
    PlayerSessionRegistrationLikeCpp,
};

struct RecordingSocialPort {
    contacts: SocialContactListLoadOutcomeLikeCpp,
    mutation: PersistenceOutcomeLikeCpp,
    candidate: SocialAddCandidateLoadOutcomeLikeCpp,
    listers: Result<Vec<u64>, String>,
    calls: Mutex<Vec<String>>,
}

impl SocialPersistencePortLikeCpp for RecordingSocialPort {
    fn load_contacts_like_cpp<'a>(
        &'a self,
        player_guid: i64,
        flags: u32,
    ) -> PersistenceFutureLikeCpp<'a, SocialContactListLoadOutcomeLikeCpp> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("load:{player_guid}:{flags}"));
        let outcome = self.contacts.clone();
        Box::pin(async move { outcome })
    }

    fn load_add_candidate_like_cpp<'a>(
        &'a self,
        _normalized_name: String,
        _kind: SocialRelationshipKindLikeCpp,
    ) -> PersistenceFutureLikeCpp<'a, SocialAddCandidateLoadOutcomeLikeCpp> {
        let outcome = self.candidate.clone();
        Box::pin(async move { outcome })
    }

    fn load_relationship_state_like_cpp<'a>(
        &'a self,
        _player_guid: i64,
        _target_guid: i64,
        _kind: SocialRelationshipKindLikeCpp,
    ) -> PersistenceFutureLikeCpp<'a, SocialRelationshipStateLikeCpp> {
        Box::pin(async {
            SocialRelationshipStateLikeCpp {
                already_present: false,
                relationship_count: 0,
            }
        })
    }

    fn party_invite_target_ignores_like_cpp<'a>(
        &'a self,
        _target_guid: i64,
        _inviter_guid: i64,
        _inviter_account_id: u32,
    ) -> PersistenceFutureLikeCpp<'a, SocialPartyInviteLookupOutcomeLikeCpp> {
        Box::pin(async { SocialPartyInviteLookupOutcomeLikeCpp::Resolved(false) })
    }

    fn party_invite_target_has_friend_like_cpp<'a>(
        &'a self,
        _target_guid: i64,
        _inviter_guid: i64,
    ) -> PersistenceFutureLikeCpp<'a, SocialPartyInviteLookupOutcomeLikeCpp> {
        Box::pin(async { SocialPartyInviteLookupOutcomeLikeCpp::Resolved(false) })
    }

    fn add_relationship_like_cpp<'a>(
        &'a self,
        _player_guid: i64,
        _target_guid: i64,
        _kind: SocialRelationshipKindLikeCpp,
        _note: String,
    ) -> PersistenceFutureLikeCpp<'a, PersistenceOutcomeLikeCpp> {
        let outcome = self.mutation.clone();
        Box::pin(async move { outcome })
    }

    fn remove_relationship_like_cpp<'a>(
        &'a self,
        player_guid: i64,
        target_guid: i64,
        kind: SocialRelationshipKindLikeCpp,
    ) -> PersistenceFutureLikeCpp<'a, PersistenceOutcomeLikeCpp> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("remove:{player_guid}:{target_guid}:{kind:?}"));
        let outcome = self.mutation.clone();
        Box::pin(async move { outcome })
    }

    fn set_contact_note_like_cpp<'a>(
        &'a self,
        _player_guid: i64,
        _target_guid: i64,
        _note: String,
    ) -> PersistenceFutureLikeCpp<'a, PersistenceOutcomeLikeCpp> {
        let outcome = self.mutation.clone();
        Box::pin(async move { outcome })
    }

    fn listers_of_like_cpp<'a>(
        &'a self,
        friend_guid: u64,
        flags: u32,
    ) -> PersistenceFutureLikeCpp<'a, Result<Vec<u64>, String>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("listers:{friend_guid}:{flags}"));
        let outcome = self.listers.clone();
        Box::pin(async move { outcome })
    }
}

fn recording_port(
    contacts: SocialContactListLoadOutcomeLikeCpp,
    mutation: PersistenceOutcomeLikeCpp,
) -> Arc<RecordingSocialPort> {
    Arc::new(RecordingSocialPort {
        contacts,
        mutation,
        candidate: SocialAddCandidateLoadOutcomeLikeCpp::NotFound,
        listers: Ok(Vec::new()),
        calls: Mutex::new(Vec::new()),
    })
}

fn recording_port_with_listers(listers: Result<Vec<u64>, String>) -> Arc<RecordingSocialPort> {
    Arc::new(RecordingSocialPort {
        contacts: SocialContactListLoadOutcomeLikeCpp::Loaded(Vec::new()),
        mutation: PersistenceOutcomeLikeCpp::Applied { rows: 1 },
        candidate: SocialAddCandidateLoadOutcomeLikeCpp::NotFound,
        listers,
        calls: Mutex::new(Vec::new()),
    })
}

fn recording_port_with_candidate(
    candidate: SocialAddCandidateLoadOutcomeLikeCpp,
) -> Arc<RecordingSocialPort> {
    Arc::new(RecordingSocialPort {
        contacts: SocialContactListLoadOutcomeLikeCpp::Loaded(Vec::new()),
        mutation: PersistenceOutcomeLikeCpp::Applied { rows: 1 },
        candidate,
        listers: Ok(Vec::new()),
        calls: Mutex::new(Vec::new()),
    })
}

fn make_session() -> (WorldSession, flume::Receiver<Vec<u8>>) {
    let (_pkt_tx, pkt_rx) = flume::bounded(8);
    let (send_tx, send_rx) = flume::bounded(8);
    (
        WorldSession::new(
            1,
            "SocialTest".into(),
            0,
            2,
            9,
            54261,
            vec![0; 40],
            "enUS".into(),
            pkt_rx,
            send_tx,
        ),
        send_rx,
    )
}

fn opcode(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[0], bytes[1]])
}

// ── presence fixtures (C++ ObjectAccessor + canonical Player) ─────────────────

const ALLIANCE_HUMAN: u8 = 1;
const HORDE_ORC: u8 = 2;
const CLASS_WARRIOR: u8 = 1;
const CLASS_MAGE: u8 = 8;
const SELF_COUNTER: i64 = 1;

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

/// Register another connected player and hand back its realm receive channel.
fn register_online_like_cpp(
    registry: &PlayerRegistry,
    counter: i64,
    name: &str,
    race: u8,
    class: u8,
    level: u8,
) -> (ObjectGuid, flume::Receiver<Vec<u8>>) {
    let guid = ObjectGuid::create_player(0, counter);
    let (send_tx, send_rx) = flume::bounded(8);
    registry.register_or_replace(
        guid,
        registration_like_cpp(guid, name, race, class, level, send_tx),
        Default::default(),
    );
    (guid, send_rx)
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

/// A logged-in session that is itself registered in a canonical-fixture registry.
fn make_registered_session(
    security: u8,
) -> (WorldSession, flume::Receiver<Vec<u8>>, Arc<PlayerRegistry>) {
    let (_pkt_tx, pkt_rx) = flume::bounded(8);
    let (send_tx, send_rx) = flume::bounded(8);
    let mut session = WorldSession::new(
        1,
        "SocialTest".into(),
        security,
        2,
        9,
        54261,
        vec![0; 40],
        "enUS".into(),
        pkt_rx,
        send_tx.clone(),
    );
    let my_guid = ObjectGuid::create_player(0, SELF_COUNTER);
    session.set_player_guid(Some(my_guid));
    session.set_loaded_player_identity_like_cpp(0, ALLIANCE_HUMAN, CLASS_WARRIOR, 80, 0);
    let registry = Arc::new(PlayerRegistry::with_canonical_player_fixtures_like_cpp());
    registry.register_or_replace(
        my_guid,
        registration_like_cpp(
            my_guid,
            "Socialtest",
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

/// Parsed `SMSG_FRIEND_STATUS` (`SocialPackets.cpp:85-101`).
#[derive(Debug, PartialEq, Eq)]
struct ParsedFriendStatus {
    result: u8,
    guid: ObjectGuid,
    status: u8,
    area_id: i32,
    level: i32,
    class_id: u32,
    notes: String,
}

fn parse_friend_status(bytes: &[u8]) -> ParsedFriendStatus {
    assert_eq!(
        opcode(bytes),
        ServerOpcodes::FriendStatus.to_u16().expect("opcode")
    );
    let mut body = wow_packet::WorldPacket::from_bytes(&bytes[2..]);
    let result = body.read_uint8().unwrap();
    let guid = body.read_packed_guid().unwrap();
    let _account_guid = body.read_packed_guid().unwrap();
    let _vra = body.read_uint32().unwrap();
    let status = body.read_uint8().unwrap();
    let area_id = body.read_int32().unwrap();
    let level = body.read_int32().unwrap();
    let class_id = body.read_uint32().unwrap();
    let notes_len = body.read_bits(10).unwrap() as usize;
    let _mobile = body.read_bit().unwrap();
    body.reset_bits();
    let notes = body.read_string(notes_len).unwrap();
    assert!(body.is_empty());
    ParsedFriendStatus {
        result,
        guid,
        status,
        area_id,
        level,
        class_id,
        notes,
    }
}

mod presence;
mod scenarios;
