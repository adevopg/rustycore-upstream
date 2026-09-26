//! Battle.net friends manager tests: invitation lifecycle, friend removal,
//! subscribe contents and presence delivery against fake persistence/sessions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use prost::Message;
use wow_constants::ServerOpcodes;
use wow_packet::WorldPacket;
use wow_persistence::{
    BnetAccountIdentityLikeCpp, BnetAccountLookupLikeCpp, BnetFriendInvitationRowLikeCpp,
    BnetFriendLinkRowLikeCpp, BnetFriendsLoadLikeCpp, BnetFriendsPersistencePortLikeCpp,
    PersistenceFutureLikeCpp, PersistenceOutcomeLikeCpp,
};

use super::*;

mod invitations;
mod presence;

pub(super) const NOW: u64 = 1_700_000_000;

#[derive(Default)]
pub(super) struct FakePersistence {
    pub load: Mutex<BnetFriendsLoadLikeCpp>,
    pub calls: Mutex<Vec<String>>,
    pub fail_writes: AtomicBool,
    pub unknown_writes: AtomicBool,
}

impl FakePersistence {
    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn write_outcome(&self) -> PersistenceOutcomeLikeCpp {
        if self.unknown_writes.load(Ordering::SeqCst) {
            PersistenceOutcomeLikeCpp::Unknown {
                reason: "commit lost".to_owned(),
            }
        } else if self.fail_writes.load(Ordering::SeqCst) {
            PersistenceOutcomeLikeCpp::Failed {
                reason: "refused".to_owned(),
            }
        } else {
            PersistenceOutcomeLikeCpp::Applied { rows: 1 }
        }
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl BnetFriendsPersistencePortLikeCpp for FakePersistence {
    fn load_all_like_cpp(
        &self,
    ) -> PersistenceFutureLikeCpp<'_, Result<BnetFriendsLoadLikeCpp, String>> {
        let load = self.load.lock().unwrap().clone();
        Box::pin(async move { Ok(load) })
    }

    fn find_account_like_cpp(
        &self,
        lookup: BnetAccountLookupLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, Result<Option<BnetAccountIdentityLikeCpp>, String>> {
        self.record(format!("find_account({lookup:?})"));
        let found = self
            .load
            .lock()
            .unwrap()
            .accounts
            .iter()
            .find(|identity| match &lookup {
                BnetAccountLookupLikeCpp::Id(id) => identity.account_id == *id,
                BnetAccountLookupLikeCpp::BattleTag(tag) => {
                    identity.battle_tag.eq_ignore_ascii_case(tag)
                }
                BnetAccountLookupLikeCpp::Email(email) => {
                    identity.email.eq_ignore_ascii_case(email)
                }
            })
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn insert_invitation_like_cpp(
        &self,
        invitation: BnetFriendInvitationRowLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        self.record(format!(
            "insert_invitation({} {}->{} created={})",
            invitation.id, invitation.inviter_id, invitation.invitee_id, invitation.created
        ));
        let outcome = self.write_outcome();
        Box::pin(async move { outcome })
    }

    fn delete_invitation_like_cpp(
        &self,
        invitation_id: u64,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        self.record(format!("delete_invitation({invitation_id})"));
        let outcome = self.write_outcome();
        Box::pin(async move { outcome })
    }

    fn accept_invitation_like_cpp(
        &self,
        invitation: BnetFriendInvitationRowLikeCpp,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        self.record(format!(
            "accept_invitation({} {}->{})",
            invitation.id, invitation.inviter_id, invitation.invitee_id
        ));
        let outcome = self.write_outcome();
        Box::pin(async move { outcome })
    }

    fn delete_friendship_like_cpp(
        &self,
        account_id: u32,
        friend_id: u32,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        self.record(format!("delete_friendship({account_id},{friend_id})"));
        let outcome = self.write_outcome();
        Box::pin(async move { outcome })
    }

    fn update_friend_note_like_cpp(
        &self,
        account_id: u32,
        friend_id: u32,
        note: String,
    ) -> PersistenceFutureLikeCpp<'_, PersistenceOutcomeLikeCpp> {
        self.record(format!(
            "update_friend_note({account_id},{friend_id},{note:?})"
        ));
        let outcome = self.write_outcome();
        Box::pin(async move { outcome })
    }
}

pub(super) fn identity(
    account_id: u32,
    battle_tag: &str,
    email: &str,
) -> BnetAccountIdentityLikeCpp {
    BnetAccountIdentityLikeCpp {
        account_id,
        email: email.to_owned(),
        battle_tag: battle_tag.to_owned(),
    }
}

pub(super) fn link(account_id: u32, friend_id: u32, note: &str) -> BnetFriendLinkRowLikeCpp {
    BnetFriendLinkRowLikeCpp {
        account_id,
        friend_id,
        note: note.to_owned(),
        role: 1,
    }
}

pub(super) fn invitation_row(
    id: u64,
    inviter_id: u32,
    invitee_id: u32,
) -> BnetFriendInvitationRowLikeCpp {
    BnetFriendInvitationRowLikeCpp {
        id,
        inviter_id,
        invitee_id,
        message: String::new(),
        created: NOW - 60,
        role: 1,
    }
}

/// Accounts 1 Alpha#0001, 2 Beta#0002, 3 Gamma#0003 (no links, no invitations).
pub(super) fn fixture_load() -> BnetFriendsLoadLikeCpp {
    BnetFriendsLoadLikeCpp {
        accounts: vec![
            identity(1, "Alpha#0001", "alpha@example.test"),
            identity(2, "Beta#0002", "beta@example.test"),
            identity(3, "Gamma#0003", "gamma@example.test"),
        ],
        links: vec![],
        invitations: vec![],
    }
}

pub(super) async fn manager_with(
    load: BnetFriendsLoadLikeCpp,
) -> (BnetFriendsMgr, Arc<FakePersistence>) {
    let persistence = Arc::new(FakePersistence {
        load: Mutex::new(load),
        ..Default::default()
    });
    let mgr = BnetFriendsMgr::new(Arc::clone(&persistence) as _).with_clock(|| NOW);
    mgr.load_from_db_like_cpp().await.unwrap();
    (mgr, persistence)
}

pub(super) struct FakeSession {
    pub account_id: u32,
    pub game_account_id: u32,
    pub sender: flume::Sender<Vec<u8>>,
    pub receiver: flume::Receiver<Vec<u8>>,
    pub snapshot: Option<BnetGameAccountPresenceSnapshotLikeCpp>,
}

impl FakeSession {
    pub fn new(account_id: u32, game_account_id: u32) -> Self {
        let (sender, receiver) = flume::unbounded();
        Self {
            account_id,
            game_account_id,
            sender,
            receiver,
            snapshot: None,
        }
    }

    pub fn agent(&self) -> BnetAgentLikeCpp {
        BnetAgentLikeCpp {
            account_id: self.account_id,
            game_account_id: self.game_account_id,
        }
    }

    pub fn drain(&self) -> Vec<Notification> {
        let mut notifications = Vec::new();
        while let Ok(bytes) = self.receiver.try_recv() {
            notifications.push(Notification::parse(&bytes));
        }
        notifications
    }

    pub fn with_character(mut self, name: &str, level: u8, zone_id: u32) -> Self {
        self.snapshot = Some(BnetGameAccountPresenceSnapshotLikeCpp {
            character_name: name.to_owned(),
            level,
            class: 8,
            race: 4,
            faction: 1,
            zone_id,
            realm_name: "RustyCore".to_owned(),
            realm_address: 0x0101_0001,
            afk: false,
            dnd: false,
        });
        self
    }
}

impl BnetFriendsSessionLikeCpp for FakeSession {
    fn battlenet_account_id_like_cpp(&self) -> u32 {
        self.account_id
    }

    fn game_account_id_like_cpp(&self) -> u32 {
        self.game_account_id
    }

    fn packet_sender_like_cpp(&self) -> flume::Sender<Vec<u8>> {
        self.sender.clone()
    }

    fn game_account_presence_like_cpp(&self) -> Option<BnetGameAccountPresenceSnapshotLikeCpp> {
        self.snapshot.clone()
    }
}

/// One decoded `SMSG_BATTLENET_NOTIFICATION`.
#[derive(Debug, Clone)]
pub(super) struct Notification {
    pub service_hash: u32,
    pub method_id: u32,
    pub token: u32,
    pub data: Vec<u8>,
}

impl Notification {
    pub fn parse(bytes: &[u8]) -> Self {
        let mut pkt = WorldPacket::from_bytes(bytes);
        assert_eq!(
            pkt.read_uint16().unwrap(),
            ServerOpcodes::BattlenetNotification as u16
        );
        let method_type = pkt.read_uint64().unwrap();
        assert_eq!(pkt.read_int64().unwrap(), 1, "C++ ObjectId is always 1");
        let token = pkt.read_uint32().unwrap();
        let size = pkt.read_uint32().unwrap() as usize;
        let data = pkt.read_bytes(size).unwrap();
        Self {
            service_hash: (method_type >> 32) as u32,
            method_id: method_type as u32,
            token,
            data,
        }
    }

    pub fn decode<M: Message + Default>(&self) -> M {
        M::decode(self.data.as_slice()).expect("notification payload decodes")
    }
}

pub(super) fn find<'a>(
    notifications: &'a [Notification],
    service_hash: u32,
    method_id: u32,
) -> Vec<&'a Notification> {
    notifications
        .iter()
        .filter(|n| n.service_hash == service_hash && n.method_id == method_id)
        .collect()
}

#[tokio::test]
async fn load_from_db_indexes_links_invitations_and_the_next_invitation_id() {
    let mut load = fixture_load();
    load.links = vec![link(1, 2, "buddy"), link(2, 1, "")];
    load.invitations = vec![invitation_row(7, 3, 1), invitation_row(12, 1, 2)];
    let (mgr, _persistence) = manager_with(load).await;
    let state = mgr.lock();
    assert!(state.are_friends(1, 2));
    assert!(state.are_friends(2, 1));
    assert!(!state.are_friends(1, 3));
    assert_eq!(state.friends[&1][&2].note, "buddy");
    assert_eq!(state.invitations.len(), 2);
    assert_eq!(state.next_invitation_id, 13);
    assert_eq!(state.battle_tag_of(3), "Gamma#0003");
    assert_eq!(state.battle_tag_of(99), "");
}

#[test]
fn entity_ids_follow_the_logon_result_conventions() {
    let account = account_entity_id_like_cpp(7);
    assert_eq!(account.high, 0x0100_0000_0000_0000);
    assert_eq!(account.low, 7);
    assert_eq!(
        entity_kind_like_cpp(&account),
        Some(BnetEntityKindLikeCpp::Account(7))
    );
    let game_account = game_account_entity_id_like_cpp(9);
    assert_eq!(game_account.high, 0x0200_0002_0057_6F57);
    assert_eq!(
        entity_kind_like_cpp(&game_account),
        Some(BnetEntityKindLikeCpp::GameAccount(9))
    );
    assert_eq!(entity_kind_like_cpp(&EntityId { high: 5, low: 1 }), None);
}

#[test]
fn field_key_selection_and_description() {
    use presence_fields::FieldKeyLikeCpp;
    let battle_tag = FieldKeyLikeCpp::account(4);
    assert_eq!(battle_tag.describe(), "BN/1/4");
    assert_eq!(
        FieldKeyLikeCpp::wow(1).with_unique_id(3).describe(),
        "WoW/2/1#3"
    );
    assert!(battle_tag.selected_by(&[]));
    assert!(battle_tag.selected_by(&[FieldKeyLikeCpp::account(4)]));
    assert!(!battle_tag.selected_by(&[FieldKeyLikeCpp::account(6)]));
    let entry = FieldKeyLikeCpp::account(3).with_unique_id(2);
    assert!(entry.selected_by(&[FieldKeyLikeCpp::account(3)]));
    assert!(!entry.selected_by(&[FieldKeyLikeCpp::account(3).with_unique_id(1)]));
    let proto = entry.to_proto();
    assert_eq!(proto.unique_id, Some(2));
    assert_eq!(FieldKeyLikeCpp::from_proto(&proto), entry);
    assert_eq!(FieldKeyLikeCpp::account(4).to_proto().unique_id, None);
}
