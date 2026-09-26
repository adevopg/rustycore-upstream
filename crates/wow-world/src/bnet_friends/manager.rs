//! In-memory Battle.net friends manager (LegionCore `Battlenet::FriendsMgr`).
//!
//! State: every account identity, friend link and pending invitation loaded at
//! startup (`LoadFromDB`), the registered world sessions of each Battle.net
//! account (`OnSessionOpened/Closed`) and the presence of each game account
//! (`OnPlayerLogin/Logout/LevelChanged/ZoneChanged/StatusChanged`). Every
//! `FriendsService` mutation persists first and mutates memory only after the
//! Login DB reported `Applied`; notifications go out after the lock is released.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use prost::Message;
use tracing::{debug, info, warn};
use wow_packet::ServerPacket;
use wow_packet::packets::battlenet::BattlenetNotification;
use wow_persistence::{
    BnetAccountIdentityLikeCpp, BnetAccountLookupLikeCpp, BnetFriendInvitationRowLikeCpp,
    BnetFriendsPersistencePortLikeCpp, PersistenceOutcomeLikeCpp,
};
use wow_proto::bgs::protocol::friends::v1::{
    Friend, FriendNotification, FriendOfFriend, InvitationNotification, ReceivedInvitation,
    SentInvitation, SentInvitationAddedNotification, SentInvitationRemovedNotification,
    SubscribeResponse, UpdateFriendStateNotification,
};
use wow_proto::bgs::protocol::presence::v1::{PresenceState, StateChangedNotification};
use wow_proto::bgs::protocol::{
    Attribute, EntityId, Identity, InvitationRemovedReason, Role, Variant,
};
use wow_proto::{service_hash, status};

use super::presence::{
    AccountPresenceLikeCpp, GameAccountPresenceLikeCpp, account_presence_state_like_cpp,
    game_account_presence_state_like_cpp,
};
use super::presence_fields::{
    FRIEND_NOTE_ATTRIBUTE_NAME_LIKE_CPP, FRIEND_NOTE_MAX_LENGTH_LIKE_CPP, MAX_FRIENDS_LIKE_CPP,
    MAX_RECEIVED_INVITATIONS_LIKE_CPP, MAX_SENT_INVITATIONS_LIKE_CPP, PROGRAM_WOW_LIKE_CPP,
    ROLE_BATTLE_TAG_FRIEND_LIKE_CPP, ROLE_BATTLE_TAG_FRIEND_NAME_LIKE_CPP,
    ROLE_REAL_ID_FRIEND_LIKE_CPP, ROLE_REAL_ID_FRIEND_NAME_LIKE_CPP,
};
use super::session_port::{BnetAgentLikeCpp, BnetFriendsSessionLikeCpp};
use super::{BnetEntityKindLikeCpp, account_entity_id_like_cpp, entity_kind_like_cpp};

/// `FriendsListener` method ids (`friends_service.pb.cc` `SendRequest` calls).
pub(crate) const FRIENDS_LISTENER_ON_FRIEND_ADDED: u32 = 1;
pub(crate) const FRIENDS_LISTENER_ON_FRIEND_REMOVED: u32 = 2;
pub(crate) const FRIENDS_LISTENER_ON_RECEIVED_INVITATION_ADDED: u32 = 3;
pub(crate) const FRIENDS_LISTENER_ON_RECEIVED_INVITATION_REMOVED: u32 = 4;
pub(crate) const FRIENDS_LISTENER_ON_SENT_INVITATION_ADDED: u32 = 5;
pub(crate) const FRIENDS_LISTENER_ON_SENT_INVITATION_REMOVED: u32 = 6;
pub(crate) const FRIENDS_LISTENER_ON_UPDATE_FRIEND_STATE: u32 = 7;
/// `PresenceListener` method ids (`presence_listener.pb.cc`).
pub(crate) const PRESENCE_LISTENER_ON_SUBSCRIBE: u32 = 1;
pub(crate) const PRESENCE_LISTENER_ON_STATE_CHANGED: u32 = 2;

/// `SubscribeResponse.max_*` (LegionCore `FriendsMgr` limits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BnetFriendsLimitsLikeCpp {
    pub max_friends: u32,
    pub max_received_invitations: u32,
    pub max_sent_invitations: u32,
}

impl Default for BnetFriendsLimitsLikeCpp {
    fn default() -> Self {
        Self {
            max_friends: MAX_FRIENDS_LIKE_CPP,
            max_received_invitations: MAX_RECEIVED_INVITATIONS_LIKE_CPP,
            max_sent_invitations: MAX_SENT_INVITATIONS_LIKE_CPP,
        }
    }
}

/// One direction of a friendship as the owner sees it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct FriendLinkLikeCpp {
    pub note: String,
    /// The attribute name the client used for the note (echoed back), else
    /// [`FRIEND_NOTE_ATTRIBUTE_NAME_LIKE_CPP`].
    pub note_attribute: Option<String>,
    pub role: u32,
    pub creation_time: u64,
}

/// A registered world session of a Battle.net account.
pub(crate) struct SessionEndpointLikeCpp {
    pub game_account_id: u32,
    pub sender: flume::Sender<Vec<u8>>,
    /// C++ `WorldSession::_battlenetRequestToken`.
    pub next_token: u32,
    pub friends_subscribed: bool,
    pub presence_subscriptions: Vec<EntityId>,
}

/// A serialized `SMSG_BATTLENET_NOTIFICATION` bound for one session, built
/// under the lock and delivered after it.
pub(crate) struct OutgoingLikeCpp {
    sender: flume::Sender<Vec<u8>>,
    bytes: Vec<u8>,
}

pub(crate) fn deliver_like_cpp(outgoing: Vec<OutgoingLikeCpp>) {
    for packet in outgoing {
        if packet.sender.send(packet.bytes).is_err() {
            debug!("Battle.net notification dropped: session send channel closed");
        }
    }
}

#[derive(Default)]
pub(crate) struct BnetFriendsStateLikeCpp {
    pub accounts: HashMap<u32, BnetAccountIdentityLikeCpp>,
    /// account -> friend -> link (two entries per friendship).
    pub friends: HashMap<u32, BTreeMap<u32, FriendLinkLikeCpp>>,
    pub invitations: BTreeMap<u64, BnetFriendInvitationRowLikeCpp>,
    /// Battle.net account -> its registered world sessions.
    pub sessions: HashMap<u32, Vec<SessionEndpointLikeCpp>>,
    pub presence: HashMap<u32, AccountPresenceLikeCpp>,
    pub next_invitation_id: u64,
}

impl BnetFriendsStateLikeCpp {
    pub(crate) fn battle_tag_of(&self, account_id: u32) -> String {
        self.accounts
            .get(&account_id)
            .map(|identity| identity.battle_tag.clone())
            .unwrap_or_default()
    }

    pub(crate) fn are_friends(&self, account_id: u32, other: u32) -> bool {
        self.friends
            .get(&account_id)
            .is_some_and(|links| links.contains_key(&other))
    }

    pub(crate) fn friend_count(&self, account_id: u32) -> usize {
        self.friends.get(&account_id).map_or(0, BTreeMap::len)
    }

    pub(crate) fn friend_ids(&self, account_id: u32) -> Vec<u32> {
        self.friends
            .get(&account_id)
            .map(|links| links.keys().copied().collect())
            .unwrap_or_default()
    }

    pub(crate) fn is_online(&self, account_id: u32) -> bool {
        self.sessions
            .get(&account_id)
            .is_some_and(|endpoints| !endpoints.is_empty())
    }

    pub(crate) fn endpoint_mut(
        &mut self,
        agent: BnetAgentLikeCpp,
    ) -> Option<&mut SessionEndpointLikeCpp> {
        self.sessions
            .get_mut(&agent.account_id)?
            .iter_mut()
            .find(|endpoint| endpoint.game_account_id == agent.game_account_id)
    }

    pub(crate) fn friend_proto(&self, account_id: u32, friend_id: u32) -> Friend {
        let link = self
            .friends
            .get(&account_id)
            .and_then(|links| links.get(&friend_id))
            .cloned()
            .unwrap_or_default();
        let mut attribute = Vec::new();
        if !link.note.is_empty() {
            attribute.push(Attribute {
                name: link
                    .note_attribute
                    .clone()
                    .unwrap_or_else(|| FRIEND_NOTE_ATTRIBUTE_NAME_LIKE_CPP.to_owned()),
                value: Variant {
                    string_value: Some(link.note.clone()),
                    ..Default::default()
                },
            });
        }
        Friend {
            account_id: account_entity_id_like_cpp(friend_id),
            attribute,
            role: vec![link.role],
            privileges: None,
            attributes_epoch: None,
            creation_time: Some(link.creation_time),
        }
    }

    pub(crate) fn received_invitation_proto(
        &self,
        invitation: &BnetFriendInvitationRowLikeCpp,
    ) -> ReceivedInvitation {
        ReceivedInvitation {
            id: invitation.id,
            inviter_identity: Identity {
                account_id: Some(account_entity_id_like_cpp(invitation.inviter_id)),
                game_account_id: None,
            },
            invitee_identity: Identity {
                account_id: Some(account_entity_id_like_cpp(invitation.invitee_id)),
                game_account_id: None,
            },
            inviter_name: Some(self.battle_tag_of(invitation.inviter_id)),
            invitee_name: Some(self.battle_tag_of(invitation.invitee_id)),
            creation_time: Some(invitation.created),
            program: Some(PROGRAM_WOW_LIKE_CPP),
        }
    }

    pub(crate) fn sent_invitation_proto(
        &self,
        invitation: &BnetFriendInvitationRowLikeCpp,
    ) -> SentInvitation {
        SentInvitation {
            id: Some(invitation.id),
            target_name: Some(self.battle_tag_of(invitation.invitee_id)),
            role: Some(invitation.role),
            attribute: vec![],
            creation_time: Some(invitation.created),
            program: Some(PROGRAM_WOW_LIKE_CPP),
        }
    }

    /// C++ `WorldSession::SendBattlenetRequest` to every session of `account_id`.
    pub(crate) fn notify_account<M: Message>(
        &mut self,
        account_id: u32,
        listener_hash: u32,
        method_id: u32,
        message: &M,
    ) -> Vec<OutgoingLikeCpp> {
        let data = message.encode_to_vec();
        self.sessions
            .get_mut(&account_id)
            .map(|endpoints| {
                endpoints
                    .iter_mut()
                    .map(|endpoint| endpoint.notification(listener_hash, method_id, data.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// One notification to one registered session (the request's own).
    pub(crate) fn notify_endpoint<M: Message>(
        &mut self,
        agent: BnetAgentLikeCpp,
        listener_hash: u32,
        method_id: u32,
        message: &M,
    ) -> Vec<OutgoingLikeCpp> {
        let data = message.encode_to_vec();
        self.endpoint_mut(agent)
            .map(|endpoint| vec![endpoint.notification(listener_hash, method_id, data)])
            .unwrap_or_default()
    }

    /// `PresenceListener.OnStateChanged(states)` to every session of
    /// `recipient` (`subscriber_id` = the recipient's account).
    pub(crate) fn notify_presence(
        &mut self,
        recipient: u32,
        states: &[PresenceState],
    ) -> Vec<OutgoingLikeCpp> {
        if states.is_empty() {
            return Vec::new();
        }
        let notification = StateChangedNotification {
            subscriber_id: Some(wow_proto::bgs::protocol::account::v1::AccountId { id: recipient }),
            state: states.to_vec(),
            subscriber_program: Some(PROGRAM_WOW_LIKE_CPP),
        };
        self.notify_account(
            recipient,
            service_hash::PRESENCE_LISTENER,
            PRESENCE_LISTENER_ON_STATE_CHANGED,
            &notification,
        )
    }

    /// LegionCore `SendPresenceToFriends`: `states` of `account_id` to every
    /// *online* friend and to the account's own sessions.
    pub(crate) fn broadcast_presence(
        &mut self,
        account_id: u32,
        states: &[PresenceState],
    ) -> Vec<OutgoingLikeCpp> {
        let mut recipients = self.friend_ids(account_id);
        recipients.push(account_id);
        let mut outgoing = Vec::new();
        for recipient in recipients {
            if self.is_online(recipient) {
                outgoing.extend(self.notify_presence(recipient, states));
            }
        }
        outgoing
    }

    /// LegionCore `SendFullPresenceOf(account)`: the account channel plus every
    /// known game account channel.
    pub(crate) fn full_presence_of(&self, account_id: u32) -> Vec<PresenceState> {
        let mut states = vec![account_presence_state_like_cpp(self, account_id)];
        if let Some(presence) = self.presence.get(&account_id) {
            for game_account_id in presence.game_accounts.keys() {
                states.push(game_account_presence_state_like_cpp(
                    self,
                    account_id,
                    *game_account_id,
                ));
            }
        }
        states
    }

    /// Account + game account channel states after a game account changed.
    pub(crate) fn changed_presence_of(
        &self,
        account_id: u32,
        game_account_id: u32,
    ) -> Vec<PresenceState> {
        vec![
            account_presence_state_like_cpp(self, account_id),
            game_account_presence_state_like_cpp(self, account_id, game_account_id),
        ]
    }

    fn game_account_presence_mut(
        &mut self,
        account_id: u32,
        game_account_id: u32,
    ) -> &mut GameAccountPresenceLikeCpp {
        self.presence
            .entry(account_id)
            .or_default()
            .game_accounts
            .entry(game_account_id)
            .or_default()
    }
}

impl SessionEndpointLikeCpp {
    pub(crate) fn notification(
        &mut self,
        listener_hash: u32,
        method_id: u32,
        data: Vec<u8>,
    ) -> OutgoingLikeCpp {
        let token = self.next_token;
        self.next_token = self.next_token.wrapping_add(1);
        OutgoingLikeCpp {
            sender: self.sender.clone(),
            bytes: BattlenetNotification::request(listener_hash, method_id, token, data).to_bytes(),
        }
    }
}

fn unix_now_like_cpp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The friend-note attribute of an `UpdateFriendState` request: any string
/// attribute whose name contains `note` (see `presence_fields.rs`).
pub(crate) fn note_attribute_like_cpp(attributes: &[Attribute]) -> Option<(&str, &str)> {
    attributes.iter().find_map(|attribute| {
        attribute
            .name
            .to_ascii_lowercase()
            .contains("note")
            .then(|| {
                attribute
                    .value
                    .string_value
                    .as_deref()
                    .map(|note| (attribute.name.as_str(), note))
            })
            .flatten()
    })
}

/// The worldserver Battle.net friends manager (one per process).
pub struct BnetFriendsMgr {
    persistence: Arc<dyn BnetFriendsPersistencePortLikeCpp>,
    limits: BnetFriendsLimitsLikeCpp,
    pub(crate) state: Mutex<BnetFriendsStateLikeCpp>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl BnetFriendsMgr {
    pub fn new(persistence: Arc<dyn BnetFriendsPersistencePortLikeCpp>) -> Self {
        Self {
            persistence,
            limits: BnetFriendsLimitsLikeCpp::default(),
            state: Mutex::new(BnetFriendsStateLikeCpp {
                next_invitation_id: 1,
                ..Default::default()
            }),
            clock: Arc::new(unix_now_like_cpp),
        }
    }

    pub fn with_limits(mut self, limits: BnetFriendsLimitsLikeCpp) -> Self {
        self.limits = limits;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_clock(mut self, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    pub fn limits(&self) -> BnetFriendsLimitsLikeCpp {
        self.limits
    }

    pub(crate) fn now(&self) -> u64 {
        (self.clock)()
    }

    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, BnetFriendsStateLikeCpp> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// LegionCore `FriendsMgr::LoadFromDB`. Returns `(links, invitations)`.
    pub async fn load_from_db_like_cpp(&self) -> Result<(usize, usize), String> {
        let started = std::time::Instant::now();
        let loaded = self.persistence.load_all_like_cpp().await?;
        let mut state = self.lock();
        state.accounts = loaded
            .accounts
            .into_iter()
            .map(|identity| (identity.account_id, identity))
            .collect();
        state.friends.clear();
        for link in &loaded.links {
            state.friends.entry(link.account_id).or_default().insert(
                link.friend_id,
                FriendLinkLikeCpp {
                    note: link.note.clone(),
                    note_attribute: None,
                    role: link.role,
                    creation_time: 0,
                },
            );
        }
        state.invitations = loaded
            .invitations
            .iter()
            .map(|invitation| (invitation.id, invitation.clone()))
            .collect();
        state.next_invitation_id = state
            .invitations
            .keys()
            .next_back()
            .map_or(1, |max| max + 1);
        let links = loaded.links.len();
        let invitations = loaded.invitations.len();
        drop(state);
        info!(
            ">> Loaded {links} Battle.net friend links and {invitations} pending invitations in {} ms",
            started.elapsed().as_millis()
        );
        Ok((links, invitations))
    }

    /// Identity of `account_id`, fetched from the Login DB when the startup load
    /// did not see it (account created since) and cached.
    async fn ensure_identity_like_cpp(
        &self,
        account_id: u32,
    ) -> Option<BnetAccountIdentityLikeCpp> {
        if let Some(identity) = self.lock().accounts.get(&account_id).cloned() {
            return Some(identity);
        }
        match self
            .persistence
            .find_account_like_cpp(BnetAccountLookupLikeCpp::Id(account_id))
            .await
        {
            Ok(Some(identity)) => {
                self.lock().accounts.insert(account_id, identity.clone());
                Some(identity)
            }
            Ok(None) => None,
            Err(error) => {
                warn!("Battle.net friends: account {account_id} lookup failed: {error}");
                None
            }
        }
    }

    /// LegionCore `FriendsMgr::OnSessionOpened`: register the session's packet
    /// channel and mark its game account online (no character yet).
    pub async fn on_session_opened_like_cpp(&self, session: &impl BnetFriendsSessionLikeCpp) {
        let agent = session.bnet_agent_like_cpp();
        if agent.account_id == 0 {
            return;
        }
        self.ensure_identity_like_cpp(agent.account_id).await;
        let outgoing = self.register_session_like_cpp(agent, session.packet_sender_like_cpp());
        deliver_like_cpp(outgoing);
    }

    /// Synchronous half of [`Self::on_session_opened_like_cpp`], also used by the
    /// request dispatcher so a session whose login hooks were not wired still
    /// receives its own notifications. Idempotent per game account.
    pub(crate) fn register_session_like_cpp(
        &self,
        agent: BnetAgentLikeCpp,
        sender: flume::Sender<Vec<u8>>,
    ) -> Vec<OutgoingLikeCpp> {
        let mut state = self.lock();
        let endpoints = state.sessions.entry(agent.account_id).or_default();
        if let Some(endpoint) = endpoints
            .iter_mut()
            .find(|endpoint| endpoint.game_account_id == agent.game_account_id)
        {
            endpoint.sender = sender;
            return Vec::new();
        }
        endpoints.push(SessionEndpointLikeCpp {
            game_account_id: agent.game_account_id,
            sender,
            next_token: 0,
            friends_subscribed: false,
            presence_subscriptions: Vec::new(),
        });
        let game_account = state.game_account_presence_mut(agent.account_id, agent.game_account_id);
        game_account.online = true;
        game_account.character = None;
        let friends = state.friend_count(agent.account_id);
        let invitations = state
            .invitations
            .values()
            .filter(|invitation| {
                invitation.inviter_id == agent.account_id
                    || invitation.invitee_id == agent.account_id
            })
            .count();
        debug!(
            "Battle.net friends: session registered for battlenet account {} ({friends} friends, {invitations} invitations)",
            agent.account_id
        );
        let states = state.changed_presence_of(agent.account_id, agent.game_account_id);
        state.broadcast_presence(agent.account_id, &states)
    }

    /// LegionCore `FriendsMgr::OnSessionClosed`: the game account goes offline
    /// and its friends learn `last_online`.
    pub fn on_session_closed_like_cpp(&self, session: &impl BnetFriendsSessionLikeCpp) {
        let agent = session.bnet_agent_like_cpp();
        let mut state = self.lock();
        let Some(endpoints) = state.sessions.get_mut(&agent.account_id) else {
            return;
        };
        endpoints.retain(|endpoint| endpoint.game_account_id != agent.game_account_id);
        if endpoints.is_empty() {
            state.sessions.remove(&agent.account_id);
        }
        let now = self.now();
        let game_account = state.game_account_presence_mut(agent.account_id, agent.game_account_id);
        game_account.online = false;
        game_account.character = None;
        game_account.last_online = now;
        if !state.is_online(agent.account_id) {
            state
                .presence
                .entry(agent.account_id)
                .or_default()
                .last_online = now;
        }
        let states = state.changed_presence_of(agent.account_id, agent.game_account_id);
        let outgoing = state.broadcast_presence(agent.account_id, &states);
        drop(state);
        deliver_like_cpp(outgoing);
    }

    /// LegionCore `FriendsMgr::OnPlayerLogin`.
    pub fn on_player_login_like_cpp(&self, session: &impl BnetFriendsSessionLikeCpp) {
        self.refresh_game_account_presence_like_cpp(session);
    }

    /// LegionCore `FriendsMgr::OnPlayerLogout`: back at character select.
    pub fn on_player_logout_like_cpp(&self, session: &impl BnetFriendsSessionLikeCpp) {
        let agent = session.bnet_agent_like_cpp();
        let mut state = self.lock();
        state
            .game_account_presence_mut(agent.account_id, agent.game_account_id)
            .character = None;
        let states = state.changed_presence_of(agent.account_id, agent.game_account_id);
        let outgoing = state.broadcast_presence(agent.account_id, &states);
        drop(state);
        deliver_like_cpp(outgoing);
    }

    /// LegionCore `FriendsMgr::OnPlayerLevelChanged`.
    pub fn on_player_level_changed_like_cpp(&self, session: &impl BnetFriendsSessionLikeCpp) {
        self.refresh_game_account_presence_like_cpp(session);
    }

    /// LegionCore `FriendsMgr::OnPlayerZoneChanged`.
    pub fn on_player_zone_changed_like_cpp(&self, session: &impl BnetFriendsSessionLikeCpp) {
        self.refresh_game_account_presence_like_cpp(session);
    }

    /// LegionCore `FriendsMgr::OnPlayerStatusChanged` (AFK / DND).
    pub fn on_player_status_changed_like_cpp(&self, session: &impl BnetFriendsSessionLikeCpp) {
        self.refresh_game_account_presence_like_cpp(session);
    }

    /// LegionCore `FriendsMgr::UpdatePresence`: rebuild this game account's
    /// presence from the session and push it to online friends.
    pub fn refresh_game_account_presence_like_cpp(&self, session: &impl BnetFriendsSessionLikeCpp) {
        let agent = session.bnet_agent_like_cpp();
        if agent.account_id == 0 {
            return;
        }
        let snapshot = session.game_account_presence_like_cpp();
        let mut state = self.lock();
        let game_account = state.game_account_presence_mut(agent.account_id, agent.game_account_id);
        game_account.online = true;
        game_account.character = snapshot;
        let states = state.changed_presence_of(agent.account_id, agent.game_account_id);
        let outgoing = state.broadcast_presence(agent.account_id, &states);
        drop(state);
        deliver_like_cpp(outgoing);
    }

    // ---- FriendsService ---------------------------------------------------

    /// `FriendsService.Subscribe`.
    pub fn subscribe_like_cpp(&self, agent: BnetAgentLikeCpp) -> Result<SubscribeResponse, u32> {
        let mut state = self.lock();
        if let Some(endpoint) = state.endpoint_mut(agent) {
            endpoint.friends_subscribed = true;
        }
        let friends = state
            .friend_ids(agent.account_id)
            .into_iter()
            .map(|friend_id| state.friend_proto(agent.account_id, friend_id))
            .collect();
        let received_invitations = state
            .invitations
            .values()
            .filter(|invitation| invitation.invitee_id == agent.account_id)
            .map(|invitation| state.received_invitation_proto(invitation))
            .collect();
        let sent_invitations = state
            .invitations
            .values()
            .filter(|invitation| invitation.inviter_id == agent.account_id)
            .map(|invitation| state.sent_invitation_proto(invitation))
            .collect();
        Ok(SubscribeResponse {
            max_friends: Some(self.limits.max_friends),
            max_received_invitations: Some(self.limits.max_received_invitations),
            max_sent_invitations: Some(self.limits.max_sent_invitations),
            role: vec![
                Role {
                    id: ROLE_BATTLE_TAG_FRIEND_LIKE_CPP,
                    name: ROLE_BATTLE_TAG_FRIEND_NAME_LIKE_CPP.to_owned(),
                    ..Default::default()
                },
                Role {
                    id: ROLE_REAL_ID_FRIEND_LIKE_CPP,
                    name: ROLE_REAL_ID_FRIEND_NAME_LIKE_CPP.to_owned(),
                    ..Default::default()
                },
            ],
            friends,
            received_invitations,
            sent_invitations,
        })
    }

    /// `FriendsService.Unsubscribe`.
    pub fn unsubscribe_like_cpp(&self, agent: BnetAgentLikeCpp) {
        if let Some(endpoint) = self.lock().endpoint_mut(agent) {
            endpoint.friends_subscribed = false;
        }
    }

    /// Resolve a `SendInvitation` target (cache first, then the Login DB).
    async fn resolve_invitation_target_like_cpp(
        &self,
        lookup: BnetAccountLookupLikeCpp,
    ) -> Result<u32, u32> {
        let cached = {
            let state = self.lock();
            state
                .accounts
                .values()
                .find(|identity| match &lookup {
                    BnetAccountLookupLikeCpp::Id(id) => identity.account_id == *id,
                    BnetAccountLookupLikeCpp::BattleTag(tag) => {
                        !identity.battle_tag.is_empty()
                            && identity.battle_tag.eq_ignore_ascii_case(tag)
                    }
                    BnetAccountLookupLikeCpp::Email(email) => {
                        identity.email.eq_ignore_ascii_case(email)
                    }
                })
                .map(|identity| identity.account_id)
        };
        if let Some(account_id) = cached {
            return Ok(account_id);
        }
        match self.persistence.find_account_like_cpp(lookup).await {
            Ok(Some(identity)) => {
                let account_id = identity.account_id;
                self.lock().accounts.insert(account_id, identity);
                Ok(account_id)
            }
            Ok(None) => Err(status::ERROR_NOT_EXISTS),
            Err(error) => {
                warn!("Battle.net friends: invitation target lookup failed: {error}");
                Err(status::ERROR_INTERNAL)
            }
        }
    }

    /// `FriendsService.SendInvitation` by BattleTag, e-mail or account entity.
    pub async fn send_invitation_like_cpp(
        &self,
        agent: BnetAgentLikeCpp,
        lookup: BnetAccountLookupLikeCpp,
        message: String,
    ) -> Result<(), u32> {
        let invitee_id = self.resolve_invitation_target_like_cpp(lookup).await?;
        let invitation = {
            let mut state = self.lock();
            if invitee_id == agent.account_id {
                return Err(status::ERROR_INVALID_ARGS);
            }
            if state.are_friends(agent.account_id, invitee_id) {
                return Err(status::ERROR_FRIENDS_FRIENDSHIP_ALREADY_EXISTS);
            }
            if state.invitations.values().any(|invitation| {
                (invitation.inviter_id == agent.account_id && invitation.invitee_id == invitee_id)
                    || (invitation.inviter_id == invitee_id
                        && invitation.invitee_id == agent.account_id)
            }) {
                return Err(status::ERROR_FRIENDS_INVITATION_ALREADY_EXISTS);
            }
            let sent = state
                .invitations
                .values()
                .filter(|invitation| invitation.inviter_id == agent.account_id)
                .count();
            if sent >= self.limits.max_sent_invitations as usize {
                return Err(status::ERROR_FRIENDS_TOO_MANY_SENT_INVITATIONS);
            }
            let received = state
                .invitations
                .values()
                .filter(|invitation| invitation.invitee_id == invitee_id)
                .count();
            if received >= self.limits.max_received_invitations as usize {
                return Err(status::ERROR_FRIENDS_TOO_MANY_RECEIVED_INVITATIONS);
            }
            if state.friend_count(agent.account_id) >= self.limits.max_friends as usize {
                return Err(status::ERROR_FRIENDS_INVITER_AT_MAX_FRIENDS);
            }
            if state.friend_count(invitee_id) >= self.limits.max_friends as usize {
                return Err(status::ERROR_FRIENDS_INVITEE_AT_MAX_FRIENDS);
            }
            let id = state.next_invitation_id;
            state.next_invitation_id += 1;
            let invitation = BnetFriendInvitationRowLikeCpp {
                id,
                inviter_id: agent.account_id,
                invitee_id,
                message,
                created: self.now(),
                role: ROLE_BATTLE_TAG_FRIEND_LIKE_CPP,
            };
            state.invitations.insert(id, invitation.clone());
            invitation
        };

        let outcome = self
            .persistence
            .insert_invitation_like_cpp(invitation.clone())
            .await;
        if !outcome.is_applied() {
            warn!(
                "Battle.net friends: invitation {} -> {} not persisted: {outcome:?}",
                invitation.inviter_id, invitation.invitee_id
            );
            self.lock().invitations.remove(&invitation.id);
            return Err(status::ERROR_INTERNAL);
        }

        let mut state = self.lock();
        let received = InvitationNotification {
            invitation: state.received_invitation_proto(&invitation),
            reason: None,
            account_id: Some(account_entity_id_like_cpp(invitation.invitee_id)),
        };
        let mut outgoing = state.notify_account(
            invitation.invitee_id,
            service_hash::FRIENDS_LISTENER,
            FRIENDS_LISTENER_ON_RECEIVED_INVITATION_ADDED,
            &received,
        );
        let sent = SentInvitationAddedNotification {
            account_id: Some(account_entity_id_like_cpp(invitation.inviter_id)),
            invitation: Some(state.sent_invitation_proto(&invitation)),
        };
        outgoing.extend(state.notify_account(
            invitation.inviter_id,
            service_hash::FRIENDS_LISTENER,
            FRIENDS_LISTENER_ON_SENT_INVITATION_ADDED,
            &sent,
        ));
        drop(state);
        deliver_like_cpp(outgoing);
        Ok(())
    }

    /// `FriendsService.AcceptInvitation` (the invitee accepts).
    pub async fn accept_invitation_like_cpp(
        &self,
        agent: BnetAgentLikeCpp,
        invitation_id: u64,
    ) -> Result<(), u32> {
        let invitation = {
            let state = self.lock();
            match state.invitations.get(&invitation_id) {
                Some(invitation) if invitation.invitee_id == agent.account_id => invitation.clone(),
                _ => return Err(status::ERROR_FRIENDS_INVALID_INVITATION),
            }
        };
        let outcome = self
            .persistence
            .accept_invitation_like_cpp(invitation.clone())
            .await;
        if !outcome.is_applied() {
            warn!(
                "Battle.net friends: accept of invitation {invitation_id} not persisted: {outcome:?}"
            );
            return Err(status::ERROR_INTERNAL);
        }

        let now = self.now();
        let mut state = self.lock();
        state.invitations.remove(&invitation_id);
        let link = FriendLinkLikeCpp {
            note: String::new(),
            note_attribute: None,
            role: invitation.role,
            creation_time: now,
        };
        state
            .friends
            .entry(invitation.inviter_id)
            .or_default()
            .insert(invitation.invitee_id, link.clone());
        state
            .friends
            .entry(invitation.invitee_id)
            .or_default()
            .insert(invitation.inviter_id, link);

        let mut outgoing = self.invitation_removed_notifications_like_cpp(
            &mut state,
            &invitation,
            InvitationRemovedReason::Accepted,
        );
        for (owner, friend) in [
            (invitation.invitee_id, invitation.inviter_id),
            (invitation.inviter_id, invitation.invitee_id),
        ] {
            let added = FriendNotification {
                target: state.friend_proto(owner, friend),
                account_id: Some(account_entity_id_like_cpp(owner)),
            };
            outgoing.extend(state.notify_account(
                owner,
                service_hash::FRIENDS_LISTENER,
                FRIENDS_LISTENER_ON_FRIEND_ADDED,
                &added,
            ));
            // LegionCore: full presence of each new friend to the other.
            let presence = state.full_presence_of(friend);
            outgoing.extend(state.notify_presence(owner, &presence));
        }
        drop(state);
        deliver_like_cpp(outgoing);
        Ok(())
    }

    fn invitation_removed_notifications_like_cpp(
        &self,
        state: &mut BnetFriendsStateLikeCpp,
        invitation: &BnetFriendInvitationRowLikeCpp,
        reason: InvitationRemovedReason,
    ) -> Vec<OutgoingLikeCpp> {
        let removed = InvitationNotification {
            invitation: state.received_invitation_proto(invitation),
            reason: Some(reason as u32),
            account_id: Some(account_entity_id_like_cpp(invitation.invitee_id)),
        };
        let mut outgoing = state.notify_account(
            invitation.invitee_id,
            service_hash::FRIENDS_LISTENER,
            FRIENDS_LISTENER_ON_RECEIVED_INVITATION_REMOVED,
            &removed,
        );
        let sent_removed = SentInvitationRemovedNotification {
            account_id: Some(account_entity_id_like_cpp(invitation.inviter_id)),
            invitation_id: Some(invitation.id),
            reason: Some(reason as u32),
        };
        outgoing.extend(state.notify_account(
            invitation.inviter_id,
            service_hash::FRIENDS_LISTENER,
            FRIENDS_LISTENER_ON_SENT_INVITATION_REMOVED,
            &sent_removed,
        ));
        outgoing
    }

    /// `DeclineInvitation` / `IgnoreInvitation` (invitee) and `RevokeInvitation`
    /// (inviter): the invitation row is deleted and both sides are told why.
    pub async fn remove_invitation_like_cpp(
        &self,
        agent: BnetAgentLikeCpp,
        invitation_id: u64,
        reason: InvitationRemovedReason,
    ) -> Result<(), u32> {
        let invitation = {
            let state = self.lock();
            let Some(invitation) = state.invitations.get(&invitation_id) else {
                return Err(status::ERROR_FRIENDS_INVALID_INVITATION);
            };
            let allowed = match reason {
                InvitationRemovedReason::Revoked => invitation.inviter_id == agent.account_id,
                _ => invitation.invitee_id == agent.account_id,
            };
            if !allowed {
                return Err(status::ERROR_FRIENDS_INVALID_INVITATION);
            }
            invitation.clone()
        };
        let outcome = self
            .persistence
            .delete_invitation_like_cpp(invitation_id)
            .await;
        if matches!(outcome, PersistenceOutcomeLikeCpp::Unknown { .. }) {
            warn!("Battle.net friends: invitation {invitation_id} delete outcome unknown");
            return Err(status::ERROR_INTERNAL);
        }
        if let PersistenceOutcomeLikeCpp::Failed { reason } = &outcome {
            warn!("Battle.net friends: invitation {invitation_id} delete failed: {reason}");
            return Err(status::ERROR_INTERNAL);
        }
        let mut state = self.lock();
        state.invitations.remove(&invitation_id);
        let outgoing =
            self.invitation_removed_notifications_like_cpp(&mut state, &invitation, reason);
        drop(state);
        deliver_like_cpp(outgoing);
        Ok(())
    }

    /// `FriendsService.RemoveFriend`: both rows, both sides notified.
    pub async fn remove_friend_like_cpp(
        &self,
        agent: BnetAgentLikeCpp,
        target: &EntityId,
    ) -> Result<(), u32> {
        let Some(BnetEntityKindLikeCpp::Account(friend_id)) = entity_kind_like_cpp(target) else {
            return Err(status::ERROR_INVALID_ARGS);
        };
        if !self.lock().are_friends(agent.account_id, friend_id) {
            return Err(status::ERROR_FRIENDS_FRIENDSHIP_DOES_NOT_EXIST);
        }
        let outcome = self
            .persistence
            .delete_friendship_like_cpp(agent.account_id, friend_id)
            .await;
        if !outcome.is_applied() {
            warn!(
                "Battle.net friends: removal {} -> {friend_id} not persisted: {outcome:?}",
                agent.account_id
            );
            return Err(status::ERROR_INTERNAL);
        }
        let mut state = self.lock();
        let mut outgoing = Vec::new();
        for (owner, friend) in [(agent.account_id, friend_id), (friend_id, agent.account_id)] {
            let removed = FriendNotification {
                target: state.friend_proto(owner, friend),
                account_id: Some(account_entity_id_like_cpp(owner)),
            };
            outgoing.extend(state.notify_account(
                owner,
                service_hash::FRIENDS_LISTENER,
                FRIENDS_LISTENER_ON_FRIEND_REMOVED,
                &removed,
            ));
            if let Some(links) = state.friends.get_mut(&owner) {
                links.remove(&friend);
            }
        }
        drop(state);
        deliver_like_cpp(outgoing);
        Ok(())
    }

    /// `FriendsService.ViewFriends`: the friends of one of the agent's friends
    /// (or of the agent), as `FriendOfFriend`.
    pub fn view_friends_like_cpp(
        &self,
        agent: BnetAgentLikeCpp,
        target: &EntityId,
    ) -> Result<Vec<FriendOfFriend>, u32> {
        let Some(BnetEntityKindLikeCpp::Account(target_id)) = entity_kind_like_cpp(target) else {
            return Err(status::ERROR_INVALID_ARGS);
        };
        let state = self.lock();
        if target_id != agent.account_id && !state.are_friends(agent.account_id, target_id) {
            return Ok(Vec::new());
        }
        Ok(state
            .friends
            .get(&target_id)
            .map(|links| {
                links
                    .iter()
                    .map(|(friend_id, link)| FriendOfFriend {
                        account_id: Some(account_entity_id_like_cpp(*friend_id)),
                        role: vec![link.role],
                        privileges: None,
                        full_name: None,
                        battle_tag: Some(state.battle_tag_of(*friend_id)),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// `FriendsService.UpdateFriendState`: the note attribute is persisted and
    /// echoed back through `OnUpdateFriendState`; other attributes are logged.
    pub async fn update_friend_state_like_cpp(
        &self,
        agent: BnetAgentLikeCpp,
        target: &EntityId,
        attributes: &[Attribute],
    ) -> Result<(), u32> {
        let Some(BnetEntityKindLikeCpp::Account(friend_id)) = entity_kind_like_cpp(target) else {
            return Err(status::ERROR_INVALID_ARGS);
        };
        if !self.lock().are_friends(agent.account_id, friend_id) {
            return Err(status::ERROR_FRIENDS_FRIENDSHIP_DOES_NOT_EXIST);
        }
        for attribute in attributes {
            debug!(
                "FriendsService.UpdateFriendState account {} friend {friend_id}: attribute {:?} = {:?}",
                agent.account_id, attribute.name, attribute.value
            );
        }
        let Some((attribute_name, note)) = note_attribute_like_cpp(attributes) else {
            debug!(
                "FriendsService.UpdateFriendState: no note attribute among {} attributes (unhandled)",
                attributes.len()
            );
            return Ok(());
        };
        if note.chars().count() > FRIEND_NOTE_MAX_LENGTH_LIKE_CPP {
            return Err(status::ERROR_FRIENDS_NOTE_MAX_SIZE_EXCEEDED);
        }
        let outcome = self
            .persistence
            .update_friend_note_like_cpp(agent.account_id, friend_id, note.to_owned())
            .await;
        if !outcome.is_applied() {
            warn!(
                "Battle.net friends: note of {} -> {friend_id} not persisted: {outcome:?}",
                agent.account_id
            );
            return Err(status::ERROR_FRIENDS_UPDATE_FRIEND_STATE_FAILED);
        }
        let mut state = self.lock();
        if let Some(link) = state
            .friends
            .get_mut(&agent.account_id)
            .and_then(|links| links.get_mut(&friend_id))
        {
            link.note = note.to_owned();
            link.note_attribute = Some(attribute_name.to_owned());
        }
        let notification = UpdateFriendStateNotification {
            changed_friend: state.friend_proto(agent.account_id, friend_id),
            account_id: Some(account_entity_id_like_cpp(agent.account_id)),
        };
        let outgoing = state.notify_account(
            agent.account_id,
            service_hash::FRIENDS_LISTENER,
            FRIENDS_LISTENER_ON_UPDATE_FRIEND_STATE,
            &notification,
        );
        drop(state);
        deliver_like_cpp(outgoing);
        Ok(())
    }
}
