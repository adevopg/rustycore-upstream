//! Presence channels (LegionCore `BuildAccountPresence` / `BuildGameAccountPresence`
//! / `FillStatus` / `FillZoneAndLevel`) and the `PresenceService` methods.
//!
//! Field ids live in `presence_fields.rs`. Every incoming `Query` key and
//! `Update` operation is logged at debug level so a live 3.4.3 capture can pin
//! the ASSUMED ids down.

use std::collections::BTreeMap;

use tracing::debug;
use wow_proto::bgs::protocol::account::v1::AccountId;
use wow_proto::bgs::protocol::presence::v1::field_operation::OperationType;
use wow_proto::bgs::protocol::presence::v1::{
    BatchSubscribeResponse, Field, FieldKey, FieldOperation, PresenceState, QueryResponse,
    SubscribeNotification, SubscribeResult,
};
use wow_proto::bgs::protocol::{EntityId, Variant};
use wow_proto::{service_hash, status};

use super::manager::{
    BnetFriendsMgr, BnetFriendsStateLikeCpp, OutgoingLikeCpp, PRESENCE_LISTENER_ON_SUBSCRIBE,
    deliver_like_cpp,
};
use super::presence_fields::*;
use super::session_port::{BnetAgentLikeCpp, BnetGameAccountPresenceSnapshotLikeCpp};
use super::{
    BnetEntityKindLikeCpp, account_entity_id_like_cpp, entity_kind_like_cpp,
    game_account_entity_id_like_cpp,
};

/// Presence of one Battle.net account: its game accounts plus whatever the
/// client itself `Update`d on the account entity (away / busy / rich text).
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct AccountPresenceLikeCpp {
    pub game_accounts: BTreeMap<u32, GameAccountPresenceLikeCpp>,
    pub self_fields: BTreeMap<FieldKeyLikeCpp, Variant>,
    /// Unix time the last session closed, while no session is open.
    pub last_online: u64,
}

/// Presence of one game account on this realm.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct GameAccountPresenceLikeCpp {
    pub online: bool,
    pub last_online: u64,
    pub character: Option<BnetGameAccountPresenceSnapshotLikeCpp>,
    pub self_fields: BTreeMap<FieldKeyLikeCpp, Variant>,
}

pub(crate) fn variant_bool(value: bool) -> Variant {
    Variant {
        bool_value: Some(value),
        ..Default::default()
    }
}

pub(crate) fn variant_int(value: i64) -> Variant {
    Variant {
        int_value: Some(value),
        ..Default::default()
    }
}

pub(crate) fn variant_string(value: impl Into<String>) -> Variant {
    Variant {
        string_value: Some(value.into()),
        ..Default::default()
    }
}

pub(crate) fn variant_fourcc(value: &str) -> Variant {
    Variant {
        fourcc_value: Some(value.to_owned()),
        ..Default::default()
    }
}

pub(crate) fn variant_entity(value: EntityId) -> Variant {
    Variant {
        entity_id_value: Some(value),
        ..Default::default()
    }
}

pub(crate) fn set_op(key: FieldKeyLikeCpp, value: Variant) -> FieldOperation {
    FieldOperation {
        field: Field {
            key: key.to_proto(),
            value,
        },
        operation: Some(OperationType::Set as i32),
    }
}

pub(crate) fn clear_op(key: FieldKeyLikeCpp) -> FieldOperation {
    FieldOperation {
        field: Field {
            key: key.to_proto(),
            value: Variant::default(),
        },
        operation: Some(OperationType::Clear as i32),
    }
}

/// LegionCore `BuildAccountPresence`.
pub(crate) fn account_presence_state_like_cpp(
    state: &BnetFriendsStateLikeCpp,
    account_id: u32,
) -> PresenceState {
    let mut ops = Vec::new();
    let battle_tag = state.battle_tag_of(account_id);
    if !battle_tag.is_empty() {
        ops.push(set_op(
            FieldKeyLikeCpp::account(ACCOUNT_FIELD_BATTLE_TAG_LIKE_CPP),
            variant_string(battle_tag),
        ));
    }
    let presence = state.presence.get(&account_id);
    let online_game_accounts: Vec<u32> = presence
        .map(|presence| {
            presence
                .game_accounts
                .iter()
                .filter(|(_, game_account)| game_account.online)
                .map(|(id, _)| *id)
                .collect()
        })
        .unwrap_or_default();
    for (index, game_account_id) in online_game_accounts.iter().enumerate() {
        ops.push(set_op(
            FieldKeyLikeCpp::account(ACCOUNT_FIELD_GAME_ACCOUNT_LIKE_CPP)
                .with_unique_id(index as u64),
            variant_entity(game_account_entity_id_like_cpp(*game_account_id)),
        ));
    }
    if online_game_accounts.is_empty() {
        if let Some(last_online) = presence.map(|presence| presence.last_online) {
            if last_online != 0 {
                ops.push(set_op(
                    FieldKeyLikeCpp::account(ACCOUNT_FIELD_LAST_ONLINE_LIKE_CPP),
                    variant_int(last_online as i64),
                ));
            }
        }
    }
    if let Some(presence) = presence {
        for (key, value) in &presence.self_fields {
            ops.push(set_op(*key, value.clone()));
        }
    }
    PresenceState {
        entity_id: Some(account_entity_id_like_cpp(account_id)),
        field_operation: ops,
    }
}

const WOW_CHARACTER_FIELDS_LIKE_CPP: [u32; 10] = [
    WOW_FIELD_CHARACTER_NAME_LIKE_CPP,
    WOW_FIELD_REALM_NAME_LIKE_CPP,
    WOW_FIELD_REALM_ADDRESS_LIKE_CPP,
    WOW_FIELD_FACTION_LIKE_CPP,
    WOW_FIELD_RACE_LIKE_CPP,
    WOW_FIELD_CLASS_LIKE_CPP,
    WOW_FIELD_LEVEL_LIKE_CPP,
    WOW_FIELD_ZONE_ID_LIKE_CPP,
    WOW_FIELD_AFK_LIKE_CPP,
    WOW_FIELD_DND_LIKE_CPP,
];

/// LegionCore `FillStatus` + `FillZoneAndLevel`: the WoW rich-presence fields.
fn character_ops_like_cpp(
    character: &BnetGameAccountPresenceSnapshotLikeCpp,
) -> Vec<FieldOperation> {
    vec![
        set_op(
            FieldKeyLikeCpp::wow(WOW_FIELD_CHARACTER_NAME_LIKE_CPP),
            variant_string(character.character_name.clone()),
        ),
        set_op(
            FieldKeyLikeCpp::wow(WOW_FIELD_REALM_NAME_LIKE_CPP),
            variant_string(character.realm_name.clone()),
        ),
        set_op(
            FieldKeyLikeCpp::wow(WOW_FIELD_REALM_ADDRESS_LIKE_CPP),
            variant_int(i64::from(character.realm_address)),
        ),
        set_op(
            FieldKeyLikeCpp::wow(WOW_FIELD_FACTION_LIKE_CPP),
            variant_int(character.faction),
        ),
        set_op(
            FieldKeyLikeCpp::wow(WOW_FIELD_RACE_LIKE_CPP),
            variant_int(i64::from(character.race)),
        ),
        set_op(
            FieldKeyLikeCpp::wow(WOW_FIELD_CLASS_LIKE_CPP),
            variant_int(i64::from(character.class)),
        ),
        set_op(
            FieldKeyLikeCpp::wow(WOW_FIELD_LEVEL_LIKE_CPP),
            variant_int(i64::from(character.level)),
        ),
        set_op(
            FieldKeyLikeCpp::wow(WOW_FIELD_ZONE_ID_LIKE_CPP),
            variant_int(i64::from(character.zone_id)),
        ),
        set_op(
            FieldKeyLikeCpp::wow(WOW_FIELD_AFK_LIKE_CPP),
            variant_bool(character.afk),
        ),
        set_op(
            FieldKeyLikeCpp::wow(WOW_FIELD_DND_LIKE_CPP),
            variant_bool(character.dnd),
        ),
    ]
}

/// LegionCore `BuildGameAccountPresence`.
pub(crate) fn game_account_presence_state_like_cpp(
    state: &BnetFriendsStateLikeCpp,
    account_id: u32,
    game_account_id: u32,
) -> PresenceState {
    let game_account = state
        .presence
        .get(&account_id)
        .and_then(|presence| presence.game_accounts.get(&game_account_id))
        .cloned()
        .unwrap_or_default();
    let mut ops = vec![
        set_op(
            FieldKeyLikeCpp::game_account(GAME_ACCOUNT_FIELD_ONLINE_LIKE_CPP),
            variant_bool(game_account.online),
        ),
        set_op(
            FieldKeyLikeCpp::game_account(GAME_ACCOUNT_FIELD_PROGRAM_LIKE_CPP),
            variant_fourcc("WoW"),
        ),
        set_op(
            FieldKeyLikeCpp::game_account(GAME_ACCOUNT_FIELD_NAME_LIKE_CPP),
            variant_string(state.battle_tag_of(account_id)),
        ),
        set_op(
            FieldKeyLikeCpp::game_account(GAME_ACCOUNT_FIELD_ACCOUNT_ID_LIKE_CPP),
            variant_entity(account_entity_id_like_cpp(account_id)),
        ),
    ];
    if !game_account.online && game_account.last_online != 0 {
        ops.push(set_op(
            FieldKeyLikeCpp::game_account(GAME_ACCOUNT_FIELD_LAST_ONLINE_LIKE_CPP),
            variant_int(game_account.last_online as i64),
        ));
    }
    match &game_account.character {
        Some(character) => ops.extend(character_ops_like_cpp(character)),
        None => ops.extend(
            WOW_CHARACTER_FIELDS_LIKE_CPP
                .iter()
                .map(|field| clear_op(FieldKeyLikeCpp::wow(*field))),
        ),
    }
    for (key, value) in &game_account.self_fields {
        ops.push(set_op(*key, value.clone()));
    }
    PresenceState {
        entity_id: Some(game_account_entity_id_like_cpp(game_account_id)),
        field_operation: ops,
    }
}

/// The `SET` fields of a channel state that `filter` selects (`Query`).
pub(crate) fn fields_of_state_like_cpp(
    state: &PresenceState,
    filter: &[FieldKeyLikeCpp],
) -> Vec<Field> {
    state
        .field_operation
        .iter()
        .filter(|op| op.operation() == OperationType::Set)
        .filter(|op| FieldKeyLikeCpp::from_proto(&op.field.key).selected_by(filter))
        .map(|op| op.field.clone())
        .collect()
}

fn describe_keys_like_cpp(keys: &[FieldKey]) -> String {
    keys.iter()
        .map(|key| FieldKeyLikeCpp::from_proto(key).describe())
        .collect::<Vec<_>>()
        .join(", ")
}

impl BnetFriendsStateLikeCpp {
    /// Which account owns `game_account_id` (only known once it registered).
    pub(crate) fn owner_of_game_account(&self, game_account_id: u32) -> Option<u32> {
        self.presence
            .iter()
            .find(|(_, presence)| presence.game_accounts.contains_key(&game_account_id))
            .map(|(account_id, _)| *account_id)
    }

    /// Whether `agent` may see `entity` (itself, its game accounts, its friends
    /// and their game accounts).
    fn entity_visible_to(&self, agent: u32, entity: &EntityId) -> Option<u32> {
        match entity_kind_like_cpp(entity)? {
            BnetEntityKindLikeCpp::Account(account_id) => {
                (account_id == agent || self.are_friends(agent, account_id)).then_some(account_id)
            }
            BnetEntityKindLikeCpp::GameAccount(game_account_id) => {
                let owner = self.owner_of_game_account(game_account_id)?;
                (owner == agent || self.are_friends(agent, owner)).then_some(owner)
            }
        }
    }

    /// The channel state of one entity.
    fn entity_state(&self, entity: &EntityId) -> Option<PresenceState> {
        match entity_kind_like_cpp(entity)? {
            BnetEntityKindLikeCpp::Account(account_id) => {
                Some(account_presence_state_like_cpp(self, account_id))
            }
            BnetEntityKindLikeCpp::GameAccount(game_account_id) => {
                let owner = self.owner_of_game_account(game_account_id)?;
                Some(game_account_presence_state_like_cpp(
                    self,
                    owner,
                    game_account_id,
                ))
            }
        }
    }

    /// `PresenceListener.OnSubscribe(states)` to the requesting session only.
    fn notify_subscribe(
        &mut self,
        agent: BnetAgentLikeCpp,
        states: Vec<PresenceState>,
    ) -> Vec<OutgoingLikeCpp> {
        let notification = SubscribeNotification {
            subscriber_id: Some(AccountId {
                id: agent.account_id,
            }),
            state: states,
            subscriber_program: Some(PROGRAM_WOW_LIKE_CPP),
        };
        self.notify_endpoint(
            agent,
            service_hash::PRESENCE_LISTENER,
            PRESENCE_LISTENER_ON_SUBSCRIBE,
            &notification,
        )
    }
}

impl BnetFriendsMgr {
    /// `PresenceService.Subscribe` (one entity of a `BatchSubscribe`): the
    /// subscription is recorded and the entity's current channel state answers
    /// through `PresenceListener.OnSubscribe`. Entities the agent may not see
    /// are accepted silently (no state).
    pub fn presence_subscribe_like_cpp(
        &self,
        agent: BnetAgentLikeCpp,
        entity: &EntityId,
        keys: &[FieldKey],
    ) -> Result<(), u32> {
        debug!(
            "PresenceService.Subscribe account {} entity {:#X}/{} keys [{}]",
            agent.account_id,
            entity.high,
            entity.low,
            describe_keys_like_cpp(keys)
        );
        if entity_kind_like_cpp(entity).is_none() {
            return Err(status::ERROR_INVALID_ARGS);
        }
        let mut state = self.lock();
        if let Some(endpoint) = state.endpoint_mut(agent) {
            if !endpoint.presence_subscriptions.contains(entity) {
                endpoint.presence_subscriptions.push(*entity);
            }
        }
        let outgoing = match state.entity_visible_to(agent.account_id, entity) {
            Some(_) => {
                let states = state.entity_state(entity).into_iter().collect();
                state.notify_subscribe(agent, states)
            }
            None => Vec::new(),
        };
        drop(state);
        deliver_like_cpp(outgoing);
        Ok(())
    }

    /// `PresenceService.BatchSubscribe`.
    pub fn presence_batch_subscribe_like_cpp(
        &self,
        agent: BnetAgentLikeCpp,
        entities: &[EntityId],
        keys: &[FieldKey],
    ) -> BatchSubscribeResponse {
        let mut response = BatchSubscribeResponse::default();
        for entity in entities {
            if let Err(result) = self.presence_subscribe_like_cpp(agent, entity, keys) {
                response.subscribe_failed.push(SubscribeResult {
                    entity_id: Some(*entity),
                    result: Some(result),
                });
            }
        }
        response
    }

    /// `PresenceService.Unsubscribe` / one entity of `BatchUnsubscribe`.
    pub fn presence_unsubscribe_like_cpp(&self, agent: BnetAgentLikeCpp, entity: &EntityId) {
        if let Some(endpoint) = self.lock().endpoint_mut(agent) {
            endpoint
                .presence_subscriptions
                .retain(|subscribed| subscribed != entity);
        }
    }

    /// `PresenceService.Query`: the selected `SET` fields of the entity's
    /// current state. The requested keys are the ids the client wants, logged
    /// so a capture can confirm the constants.
    pub fn presence_query_like_cpp(
        &self,
        agent: BnetAgentLikeCpp,
        entity: &EntityId,
        keys: &[FieldKey],
    ) -> Result<QueryResponse, u32> {
        debug!(
            "PresenceService.Query account {} entity {:#X}/{} keys [{}]",
            agent.account_id,
            entity.high,
            entity.low,
            describe_keys_like_cpp(keys)
        );
        if entity_kind_like_cpp(entity).is_none() {
            return Err(status::ERROR_INVALID_ARGS);
        }
        let filter: Vec<FieldKeyLikeCpp> = keys.iter().map(FieldKeyLikeCpp::from_proto).collect();
        let state = self.lock();
        let field = match state.entity_visible_to(agent.account_id, entity) {
            Some(_) => state
                .entity_state(entity)
                .map(|entity_state| fields_of_state_like_cpp(&entity_state, &filter))
                .unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(QueryResponse { field })
    }

    /// `PresenceService.Update`: the client's own AFK / DND / custom fields on
    /// its account or game account entity, stored and re-broadcast to friends.
    pub fn presence_update_like_cpp(
        &self,
        agent: BnetAgentLikeCpp,
        entity: &EntityId,
        operations: &[FieldOperation],
    ) -> Result<(), u32> {
        for op in operations {
            debug!(
                "PresenceService.Update account {} entity {:#X}/{} {:?} {} = {:?}",
                agent.account_id,
                entity.high,
                entity.low,
                op.operation(),
                FieldKeyLikeCpp::from_proto(&op.field.key).describe(),
                op.field.value
            );
        }
        let mut state = self.lock();
        let fields = match entity_kind_like_cpp(entity) {
            Some(BnetEntityKindLikeCpp::Account(account_id)) if account_id == agent.account_id => {
                &mut state.presence.entry(account_id).or_default().self_fields
            }
            Some(BnetEntityKindLikeCpp::GameAccount(game_account_id))
                if game_account_id == agent.game_account_id =>
            {
                &mut state
                    .presence
                    .entry(agent.account_id)
                    .or_default()
                    .game_accounts
                    .entry(game_account_id)
                    .or_default()
                    .self_fields
            }
            Some(_) => return Err(status::ERROR_DENIED),
            None => return Err(status::ERROR_INVALID_ARGS),
        };
        for op in operations {
            let key = FieldKeyLikeCpp::from_proto(&op.field.key);
            match op.operation() {
                OperationType::Set => {
                    fields.insert(key, op.field.value.clone());
                }
                OperationType::Clear => {
                    fields.remove(&key);
                }
            }
        }
        let changed = PresenceState {
            entity_id: Some(*entity),
            field_operation: operations.to_vec(),
        };
        let outgoing = state.broadcast_presence(agent.account_id, &[changed]);
        drop(state);
        deliver_like_cpp(outgoing);
        Ok(())
    }
}
