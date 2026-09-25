//! Handler-level scenarios of `HandleCharRaceOrFactionChangeCallback` with a
//! recording persistence port (no database).

use std::sync::{Arc, Mutex};

use wow_constants::ServerOpcodes;
use wow_core::{ObjectGuid, Position};
use wow_data::{
    MapEntry, MapStore, PlayerCreateInfoRowLikeCpp, PlayerCreateInfoStoreLikeCpp,
    PlayerCreatePositionLikeCpp,
};
use wow_packet::WorldPacket;
use wow_packet::packets::character::{CharRaceOrFactionChange, ChrCustomizationChoice};
use wow_persistence::{
    CharacterAdministrationLoadOutcomeLikeCpp as LoadOutcome,
    CharacterAdministrationMutationOutcomeLikeCpp as MutationOutcome,
    CharacterAdministrationPersistencePortLikeCpp, CharacterCreatePersistenceRequestLikeCpp,
    CharacterCustomizationPersistenceLikeCpp, CharacterCustomizeCandidateLikeCpp,
    CharacterRaceOrFactionChangeCandidateLikeCpp, CharacterRaceOrFactionChangeCommitLikeCpp,
    CharacterRenameCandidateLikeCpp, PersistenceFutureLikeCpp,
};

use crate::WorldSession;
use crate::character_race_faction_change::RaceFactionChangeCatalogLikeCpp;

#[derive(Default)]
struct PortFixture {
    candidate: Mutex<Option<CharacterRaceOrFactionChangeCandidateLikeCpp>>,
    name_in_use: bool,
    standings: Vec<(u32, i32)>,
    commits: Mutex<Vec<CharacterRaceOrFactionChangeCommitLikeCpp>>,
}

impl CharacterAdministrationPersistencePortLikeCpp for PortFixture {
    fn find_character_name_like_cpp(
        &self,
        _name: &str,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<()>> {
        let in_use = self.name_in_use;
        Box::pin(async move {
            if in_use {
                LoadOutcome::Loaded(())
            } else {
                LoadOutcome::NotFound
            }
        })
    }

    fn load_account_character_count_like_cpp(
        &self,
        _account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<u64>> {
        unreachable!()
    }

    fn create_character_like_cpp(
        &self,
        _request: CharacterCreatePersistenceRequestLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        unreachable!()
    }

    fn delete_owned_character_like_cpp(
        &self,
        _guid: u64,
        _account_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        unreachable!()
    }

    fn load_rename_candidate_like_cpp(
        &self,
        _guid: u64,
        _new_name: &str,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<CharacterRenameCandidateLikeCpp>> {
        unreachable!()
    }

    fn commit_rename_like_cpp(
        &self,
        _guid: u64,
        _new_name: &str,
        _at_login_flags: u16,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        unreachable!()
    }

    fn load_customize_candidate_like_cpp(
        &self,
        _guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<CharacterCustomizeCandidateLikeCpp>> {
        unreachable!()
    }

    fn commit_customize_like_cpp(
        &self,
        _guid: u64,
        _name: &str,
        _at_login_flags: u16,
        _customizations: Vec<CharacterCustomizationPersistenceLikeCpp>,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        unreachable!()
    }

    fn load_race_or_faction_change_candidate_like_cpp(
        &self,
        _guid: u64,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<CharacterRaceOrFactionChangeCandidateLikeCpp>>
    {
        let candidate = self.candidate.lock().unwrap().clone();
        Box::pin(async move { candidate.map_or(LoadOutcome::NotFound, LoadOutcome::Loaded) })
    }

    fn load_reputation_standing_like_cpp(
        &self,
        _guid: u64,
        faction_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, LoadOutcome<i32>> {
        let standing = self
            .standings
            .iter()
            .find(|(faction, _)| *faction == faction_id)
            .map(|(_, standing)| *standing);
        Box::pin(async move { standing.map_or(LoadOutcome::NotFound, LoadOutcome::Loaded) })
    }

    fn commit_race_or_faction_change_like_cpp(
        &self,
        request: CharacterRaceOrFactionChangeCommitLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, MutationOutcome> {
        self.commits.lock().unwrap().push(request);
        Box::pin(async { MutationOutcome::Applied })
    }
}

fn session() -> (WorldSession, flume::Receiver<Vec<u8>>) {
    let (_packet_tx, packet_rx) = flume::bounded(1);
    let (send_tx, send_rx) = flume::bounded(8);
    let session = WorldSession::new(
        1,
        "FactionChange".into(),
        0,
        2,
        9,
        54261,
        vec![0; 40],
        "esES".into(),
        packet_rx,
        send_tx,
    );
    (session, send_rx)
}

fn create_info() -> PlayerCreateInfoStoreLikeCpp {
    let maps = MapStore::from_entries([0, 1].map(|id| MapEntry {
        id,
        instance_type: wow_data::map::MAP_COMMON,
        expansion_id: 0,
        parent_map_id: -1,
        cosmetic_parent_map_id: -1,
        flags1: 0,
        flags2: 0,
    }));
    let row = |race| PlayerCreateInfoRowLikeCpp {
        race,
        class: 1,
        create_position: PlayerCreatePositionLikeCpp {
            map_id: 0,
            position: Position::new(1.0, 2.0, 3.0, 0.0),
            transport_guid: None,
        },
        create_position_npe: None,
        npe_transport_template_valid: true,
    };
    PlayerCreateInfoStoreLikeCpp::from_rows_like_cpp(
        [row(1), row(2), row(3)],
        &maps,
        |_| true,
        |_| true,
        |_| true,
    )
}

fn catalog() -> RaceFactionChangeCatalogLikeCpp {
    let outcome = wow_data::FactionChangeStoreLikeCpp::from_validated_rows_like_cpp(
        [],
        [],
        [wow_data::FactionChangePairRowLikeCpp {
            alliance_id: 72,
            horde_id: 76,
        }],
        [],
        [],
        |_| true,
        |_| true,
        |_| true,
        |_| true,
        |_| true,
    );
    RaceFactionChangeCatalogLikeCpp {
        faction_change: Arc::new(outcome.store),
        race_alliance: [(1, 0), (2, 1), (3, 0)].into_iter().collect(),
        horde_taxi_mask: vec![2; 8],
        alliance_taxi_mask: vec![1; 8],
        ..RaceFactionChangeCatalogLikeCpp::default()
    }
}

fn human(at_login_flags: u16) -> CharacterRaceOrFactionChangeCandidateLikeCpp {
    CharacterRaceOrFactionChangeCandidateLikeCpp {
        name: "Oldname".into(),
        race: 1,
        class: 1,
        level: 20,
        sex: 0,
        at_login_flags,
        known_titles: String::new(),
        group_id: 0,
        guild_id: 0,
        guild_leader_guid: 0,
    }
}

fn request(guid: ObjectGuid, faction_change: bool, race: u8) -> CharRaceOrFactionChange {
    CharRaceOrFactionChange {
        faction_change,
        guid,
        sex_id: 1,
        race_id: race,
        initial_race_id: 1,
        name: "nEWNAME".into(),
        customizations: vec![ChrCustomizationChoice {
            option_id: 1,
            choice_id: 2,
        }],
    }
}

async fn run(
    port: Arc<PortFixture>,
    faction_change: bool,
    race: u8,
) -> (u8, WorldPacket, Arc<PortFixture>) {
    let (mut session, send_rx) = session();
    let guid = ObjectGuid::create_player(1, 42);
    session.set_legit_characters(vec![guid]);
    session.set_character_administration_persistence_port_like_cpp(port.clone());
    session
        .handle_char_race_or_faction_change_like_cpp(
            &catalog(),
            &create_info(),
            request(guid, faction_change, race),
        )
        .await;
    let mut pkt = WorldPacket::from_bytes(&send_rx.try_recv().expect("faction change result"));
    assert_eq!(
        pkt.server_opcode(),
        Some(ServerOpcodes::CharFactionChangeResult)
    );
    pkt.skip_opcode();
    let result = pkt.read_uint8().unwrap();
    assert_eq!(pkt.read_packed_guid().unwrap(), guid);
    (result, pkt, port)
}

fn port(candidate: CharacterRaceOrFactionChangeCandidateLikeCpp) -> Arc<PortFixture> {
    Arc::new(PortFixture {
        candidate: Mutex::new(Some(candidate)),
        standings: vec![(72, 1000)],
        ..PortFixture::default()
    })
}

#[tokio::test]
async fn non_owned_character_kicks_like_cpp() {
    let (mut session, send_rx) = session();
    session
        .handle_char_race_or_faction_change_like_cpp(
            &catalog(),
            &create_info(),
            request(ObjectGuid::create_player(1, 42), true, 2),
        )
        .await;
    assert_eq!(session.state(), crate::session::SessionState::Disconnecting);
    assert!(send_rx.try_recv().is_err());
}

#[tokio::test]
async fn missing_login_flag_and_team_mismatches_use_cpp_codes() {
    let (result, _, fixture) = run(port(human(0)), true, 2).await;
    assert_eq!(result, 25);
    assert!(fixture.commits.lock().unwrap().is_empty());
    // Faction change requested but the new race is on the same team.
    let (result, _, _) = run(port(human(0x40)), true, 3).await;
    assert_eq!(result, 42);
    // Race change requested but the new race is on the other team.
    let (result, _, _) = run(port(human(0x80)), false, 2).await;
    assert_eq!(result, 43);
    // No playercreateinfo for (race, class).
    let (result, _, _) = run(port(human(0x40)), true, 5).await;
    assert_eq!(result, 25);
}

#[tokio::test]
async fn name_in_use_by_another_character_is_refused() {
    let fixture = Arc::new(PortFixture {
        candidate: Mutex::new(Some(human(0x40))),
        name_in_use: true,
        ..PortFixture::default()
    });
    let (result, _, fixture) = run(fixture, true, 2).await;
    assert_eq!(result, 27);
    assert!(fixture.commits.lock().unwrap().is_empty());
}

#[tokio::test]
async fn faction_change_commits_the_cpp_transaction_and_sends_the_display() {
    let (result, mut pkt, fixture) = run(port(human(0x40 | 0x8)), true, 2).await;
    assert_eq!(result, 0);
    assert!(pkt.read_bit().unwrap());
    pkt.reset_bits();
    assert_eq!(pkt.read_bits(6).unwrap(), 7);
    assert_eq!(pkt.read_uint8().unwrap(), 1);
    assert_eq!(pkt.read_uint8().unwrap(), 2);
    assert_eq!(pkt.read_uint32().unwrap(), 1);
    assert_eq!(pkt.read_string(7).unwrap(), "Newname");

    let commits = fixture.commits.lock().unwrap();
    let commit = &commits[0];
    assert_eq!(commit.guid, 42);
    assert_eq!(commit.name, "Newname");
    // (at_login | RESURRECT) & ~CHANGE_FACTION keeps the customize flag.
    assert_eq!(commit.at_login_flags, 0x100 | 0x8);
    assert_eq!(commit.race, 2);
    assert_eq!(commit.extra_flags, 0x200);
    assert_eq!(commit.languages.as_deref(), Some(&[109][..]));
    let faction = commit.faction.as_ref().expect("team conversion");
    assert_eq!(faction.taximask.as_deref(), Some("2 2 2 2 2 2 2 2 "));
    assert_eq!(faction.homebind.0, 1);
    assert!(faction.delete_social);
    // Old Stormwind standing 1000 moves to Orgrimmar (no base reputation data).
    assert_eq!(faction.reputations, vec![(76, 1000, 72)]);
}

#[tokio::test]
async fn race_change_within_the_team_has_no_team_conversion() {
    let (result, _, fixture) = run(port(human(0x80)), false, 3).await;
    assert_eq!(result, 0);
    let commits = fixture.commits.lock().unwrap();
    assert_eq!(commits[0].languages.as_deref(), Some(&[98, 111][..]));
    assert!(commits[0].faction.is_none());
    assert_eq!(commits[0].at_login_flags, 0x100);
}
