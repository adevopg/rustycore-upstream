//! Presence: build from the session, push to online friends only, subscribe /
//! query / update through `PresenceService`.

use wow_proto::bgs::protocol::Variant;
use wow_proto::bgs::protocol::presence::v1::field_operation::OperationType;
use wow_proto::bgs::protocol::presence::v1::{
    FieldOperation, PresenceState, StateChangedNotification, SubscribeNotification,
};
use wow_proto::{service_hash, status};

use super::*;
use crate::bnet_friends::manager::*;
use crate::bnet_friends::presence::{set_op, variant_bool};
use crate::bnet_friends::presence_fields::*;

fn field<'a>(state: &'a PresenceState, key: FieldKeyLikeCpp) -> Option<&'a FieldOperation> {
    state
        .field_operation
        .iter()
        .find(|op| FieldKeyLikeCpp::from_proto(&op.field.key) == key)
}

/// Accounts 1 and 2 are friends, 3 is a stranger; every account has a session.
async fn three_sessions() -> (BnetFriendsMgr, FakeSession, FakeSession, FakeSession) {
    let mut load = fixture_load();
    load.links = vec![link(1, 2, ""), link(2, 1, "")];
    let (mgr, _persistence) = manager_with(load).await;
    let alpha = FakeSession::new(1, 11).with_character("Alphachar", 42, 1537);
    let beta = FakeSession::new(2, 22);
    let gamma = FakeSession::new(3, 33);
    for session in [&alpha, &beta, &gamma] {
        mgr.on_session_opened_like_cpp(session).await;
    }
    for session in [&alpha, &beta, &gamma] {
        session.drain();
    }
    (mgr, alpha, beta, gamma)
}

#[tokio::test]
async fn player_login_pushes_account_and_game_account_presence_to_online_friends_only() {
    let (mgr, alpha, beta, gamma) = three_sessions().await;

    mgr.on_player_login_like_cpp(&alpha);

    assert!(gamma.drain().is_empty(), "strangers get nothing");
    let seen = beta.drain();
    let changed = find(
        &seen,
        service_hash::PRESENCE_LISTENER,
        PRESENCE_LISTENER_ON_STATE_CHANGED,
    );
    assert_eq!(changed.len(), 1);
    let notification: StateChangedNotification = changed[0].decode();
    assert_eq!(notification.subscriber_id.unwrap().id, 2);
    assert_eq!(notification.subscriber_program, Some(PROGRAM_WOW_LIKE_CPP));
    assert_eq!(notification.state.len(), 2);

    let account = &notification.state[0];
    assert_eq!(account.entity_id, Some(account_entity_id_like_cpp(1)));
    assert_eq!(
        field(
            account,
            FieldKeyLikeCpp::account(ACCOUNT_FIELD_BATTLE_TAG_LIKE_CPP)
        )
        .unwrap()
        .field
        .value
        .string_value
        .as_deref(),
        Some("Alpha#0001")
    );
    let game_accounts = field(
        account,
        FieldKeyLikeCpp::account(ACCOUNT_FIELD_GAME_ACCOUNT_LIKE_CPP),
    )
    .unwrap();
    assert_eq!(
        game_accounts.field.value.entity_id_value,
        Some(game_account_entity_id_like_cpp(11))
    );
    assert!(
        field(
            account,
            FieldKeyLikeCpp::account(ACCOUNT_FIELD_LAST_ONLINE_LIKE_CPP)
        )
        .is_none()
    );

    let game_account = &notification.state[1];
    assert_eq!(
        game_account.entity_id,
        Some(game_account_entity_id_like_cpp(11))
    );
    let value = |key| field(game_account, key).unwrap().field.value.clone();
    assert_eq!(
        value(FieldKeyLikeCpp::game_account(
            GAME_ACCOUNT_FIELD_ONLINE_LIKE_CPP
        ))
        .bool_value,
        Some(true)
    );
    assert_eq!(
        value(FieldKeyLikeCpp::game_account(
            GAME_ACCOUNT_FIELD_PROGRAM_LIKE_CPP
        ))
        .fourcc_value
        .as_deref(),
        Some("WoW")
    );
    assert_eq!(
        value(FieldKeyLikeCpp::game_account(
            GAME_ACCOUNT_FIELD_NAME_LIKE_CPP
        ))
        .string_value
        .as_deref(),
        Some("Alpha#0001")
    );
    assert_eq!(
        value(FieldKeyLikeCpp::game_account(
            GAME_ACCOUNT_FIELD_ACCOUNT_ID_LIKE_CPP
        ))
        .entity_id_value,
        Some(account_entity_id_like_cpp(1))
    );
    assert_eq!(
        value(FieldKeyLikeCpp::wow(WOW_FIELD_CHARACTER_NAME_LIKE_CPP))
            .string_value
            .as_deref(),
        Some("Alphachar")
    );
    assert_eq!(
        value(FieldKeyLikeCpp::wow(WOW_FIELD_LEVEL_LIKE_CPP)).int_value,
        Some(42)
    );
    assert_eq!(
        value(FieldKeyLikeCpp::wow(WOW_FIELD_ZONE_ID_LIKE_CPP)).int_value,
        Some(1537)
    );
    assert_eq!(
        value(FieldKeyLikeCpp::wow(WOW_FIELD_REALM_NAME_LIKE_CPP))
            .string_value
            .as_deref(),
        Some("RustyCore")
    );
    assert_eq!(
        value(FieldKeyLikeCpp::wow(WOW_FIELD_FACTION_LIKE_CPP)).int_value,
        Some(1)
    );
    assert_eq!(
        value(FieldKeyLikeCpp::wow(WOW_FIELD_AFK_LIKE_CPP)).bool_value,
        Some(false)
    );
    assert!(
        game_account
            .field_operation
            .iter()
            .all(|op| op.operation() == OperationType::Set)
    );

    // The account's own session sees its presence too.
    assert_eq!(
        find(
            &alpha.drain(),
            service_hash::PRESENCE_LISTENER,
            PRESENCE_LISTENER_ON_STATE_CHANGED
        )
        .len(),
        1
    );
}

#[tokio::test]
async fn logout_clears_the_character_and_session_close_marks_the_game_account_offline() {
    let (mgr, alpha, beta, _gamma) = three_sessions().await;
    mgr.on_player_login_like_cpp(&alpha);
    beta.drain();

    mgr.on_player_logout_like_cpp(&alpha);
    let notification: StateChangedNotification = find(
        &beta.drain(),
        service_hash::PRESENCE_LISTENER,
        PRESENCE_LISTENER_ON_STATE_CHANGED,
    )[0]
    .decode();
    let game_account = &notification.state[1];
    let name = field(
        game_account,
        FieldKeyLikeCpp::wow(WOW_FIELD_CHARACTER_NAME_LIKE_CPP),
    )
    .unwrap();
    assert_eq!(name.operation(), OperationType::Clear);
    assert_eq!(
        field(
            game_account,
            FieldKeyLikeCpp::game_account(GAME_ACCOUNT_FIELD_ONLINE_LIKE_CPP)
        )
        .unwrap()
        .field
        .value
        .bool_value,
        Some(true)
    );

    mgr.on_session_closed_like_cpp(&alpha);
    let notification: StateChangedNotification = find(
        &beta.drain(),
        service_hash::PRESENCE_LISTENER,
        PRESENCE_LISTENER_ON_STATE_CHANGED,
    )[0]
    .decode();
    let account = &notification.state[0];
    assert!(
        field(
            account,
            FieldKeyLikeCpp::account(ACCOUNT_FIELD_GAME_ACCOUNT_LIKE_CPP)
        )
        .is_none()
    );
    assert_eq!(
        field(
            account,
            FieldKeyLikeCpp::account(ACCOUNT_FIELD_LAST_ONLINE_LIKE_CPP)
        )
        .unwrap()
        .field
        .value
        .int_value,
        Some(NOW as i64)
    );
    let game_account = &notification.state[1];
    assert_eq!(
        field(
            game_account,
            FieldKeyLikeCpp::game_account(GAME_ACCOUNT_FIELD_ONLINE_LIKE_CPP)
        )
        .unwrap()
        .field
        .value
        .bool_value,
        Some(false)
    );
    assert_eq!(
        field(
            game_account,
            FieldKeyLikeCpp::game_account(GAME_ACCOUNT_FIELD_LAST_ONLINE_LIKE_CPP)
        )
        .unwrap()
        .field
        .value
        .int_value,
        Some(NOW as i64)
    );
    assert!(!mgr.lock().is_online(1));

    // Closed session: nothing more is delivered to it, friends keep working.
    alpha.drain();
    mgr.on_player_login_like_cpp(&beta);
    assert!(alpha.drain().is_empty());
}

#[tokio::test]
async fn presence_subscribe_answers_with_the_current_state_of_visible_entities() {
    let (mgr, alpha, beta, gamma) = three_sessions().await;
    mgr.on_player_login_like_cpp(&alpha);
    alpha.drain();
    beta.drain();

    mgr.presence_subscribe_like_cpp(beta.agent(), &account_entity_id_like_cpp(1), &[])
        .unwrap();
    let seen = beta.drain();
    let subscribed = find(
        &seen,
        service_hash::PRESENCE_LISTENER,
        PRESENCE_LISTENER_ON_SUBSCRIBE,
    );
    assert_eq!(subscribed.len(), 1);
    let notification: SubscribeNotification = subscribed[0].decode();
    assert_eq!(notification.subscriber_id.unwrap().id, 2);
    assert_eq!(notification.state.len(), 1);
    assert_eq!(
        notification.state[0].entity_id,
        Some(account_entity_id_like_cpp(1))
    );
    assert!(
        alpha.drain().is_empty(),
        "OnSubscribe goes to the subscriber only"
    );

    // A friend's game account entity is visible; a stranger's is not.
    mgr.presence_subscribe_like_cpp(beta.agent(), &game_account_entity_id_like_cpp(11), &[])
        .unwrap();
    let notification: SubscribeNotification = find(
        &beta.drain(),
        service_hash::PRESENCE_LISTENER,
        PRESENCE_LISTENER_ON_SUBSCRIBE,
    )[0]
    .decode();
    assert_eq!(
        notification.state[0].entity_id,
        Some(game_account_entity_id_like_cpp(11))
    );
    mgr.presence_subscribe_like_cpp(gamma.agent(), &account_entity_id_like_cpp(1), &[])
        .unwrap();
    assert!(gamma.drain().is_empty());
    assert_eq!(
        mgr.presence_subscribe_like_cpp(gamma.agent(), &EntityId { high: 9, low: 9 }, &[]),
        Err(status::ERROR_INVALID_ARGS)
    );
    {
        let mut state = mgr.lock();
        let endpoint = state.endpoint_mut(beta.agent()).unwrap();
        assert_eq!(endpoint.presence_subscriptions.len(), 2);
    }
    mgr.presence_unsubscribe_like_cpp(beta.agent(), &account_entity_id_like_cpp(1));
    assert_eq!(
        mgr.lock()
            .endpoint_mut(beta.agent())
            .unwrap()
            .presence_subscriptions
            .len(),
        1
    );

    let batch = mgr.presence_batch_subscribe_like_cpp(
        beta.agent(),
        &[account_entity_id_like_cpp(1), EntityId { high: 9, low: 9 }],
        &[],
    );
    assert_eq!(batch.subscribe_failed.len(), 1);
    assert_eq!(
        batch.subscribe_failed[0].result,
        Some(status::ERROR_INVALID_ARGS)
    );
}

#[tokio::test]
async fn presence_query_returns_the_selected_set_fields() {
    let (mgr, alpha, beta, gamma) = three_sessions().await;
    mgr.on_player_login_like_cpp(&alpha);

    let all = mgr
        .presence_query_like_cpp(beta.agent(), &game_account_entity_id_like_cpp(11), &[])
        .unwrap();
    assert!(all.field.len() >= 14);
    let only_name = mgr
        .presence_query_like_cpp(
            beta.agent(),
            &game_account_entity_id_like_cpp(11),
            &[FieldKeyLikeCpp::wow(WOW_FIELD_CHARACTER_NAME_LIKE_CPP).to_proto()],
        )
        .unwrap();
    assert_eq!(only_name.field.len(), 1);
    assert_eq!(
        only_name.field[0].value.string_value.as_deref(),
        Some("Alphachar")
    );
    let stranger = mgr
        .presence_query_like_cpp(gamma.agent(), &account_entity_id_like_cpp(1), &[])
        .unwrap();
    assert!(stranger.field.is_empty());
}

#[tokio::test]
async fn presence_update_from_the_client_is_stored_and_rebroadcast_to_friends() {
    let (mgr, alpha, beta, gamma) = three_sessions().await;
    let away = set_op(
        FieldKeyLikeCpp::account(ACCOUNT_FIELD_AWAY_LIKE_CPP),
        variant_bool(true),
    );

    assert_eq!(
        mgr.presence_update_like_cpp(
            alpha.agent(),
            &account_entity_id_like_cpp(2),
            &[away.clone()]
        ),
        Err(status::ERROR_DENIED),
        "only the agent's own entities"
    );
    mgr.presence_update_like_cpp(
        alpha.agent(),
        &account_entity_id_like_cpp(1),
        &[away.clone()],
    )
    .unwrap();

    let notification: StateChangedNotification = find(
        &beta.drain(),
        service_hash::PRESENCE_LISTENER,
        PRESENCE_LISTENER_ON_STATE_CHANGED,
    )[0]
    .decode();
    assert_eq!(notification.state.len(), 1);
    assert_eq!(
        notification.state[0].entity_id,
        Some(account_entity_id_like_cpp(1))
    );
    assert_eq!(notification.state[0].field_operation, vec![away.clone()]);
    assert!(gamma.drain().is_empty());

    // The stored field is part of the account state from now on, until cleared.
    let full = mgr
        .presence_query_like_cpp(beta.agent(), &account_entity_id_like_cpp(1), &[])
        .unwrap();
    assert!(
        full.field
            .iter()
            .any(|f| FieldKeyLikeCpp::from_proto(&f.key)
                == FieldKeyLikeCpp::account(ACCOUNT_FIELD_AWAY_LIKE_CPP))
    );
    let clear = FieldOperation {
        operation: Some(OperationType::Clear as i32),
        ..away
    };
    mgr.presence_update_like_cpp(alpha.agent(), &account_entity_id_like_cpp(1), &[clear])
        .unwrap();
    let full = mgr
        .presence_query_like_cpp(beta.agent(), &account_entity_id_like_cpp(1), &[])
        .unwrap();
    assert!(
        !full
            .field
            .iter()
            .any(|f| FieldKeyLikeCpp::from_proto(&f.key)
                == FieldKeyLikeCpp::account(ACCOUNT_FIELD_AWAY_LIKE_CPP))
    );

    // Game account entity of the agent's own session is writable as well.
    let custom = set_op(
        FieldKeyLikeCpp::wow(WOW_FIELD_DND_LIKE_CPP),
        Variant {
            bool_value: Some(true),
            ..Default::default()
        },
    );
    mgr.presence_update_like_cpp(
        alpha.agent(),
        &game_account_entity_id_like_cpp(11),
        &[custom],
    )
    .unwrap();
    assert_eq!(
        mgr.presence_update_like_cpp(alpha.agent(), &game_account_entity_id_like_cpp(22), &[]),
        Err(status::ERROR_DENIED)
    );
}

#[tokio::test]
async fn session_registration_is_idempotent_and_replaces_the_channel() {
    let (mgr, _persistence) = manager_with(fixture_load()).await;
    let first = FakeSession::new(1, 11);
    mgr.on_session_opened_like_cpp(&first).await;
    first.drain();
    let second = FakeSession::new(1, 11);
    mgr.on_session_opened_like_cpp(&second).await;
    assert_eq!(mgr.lock().sessions[&1].len(), 1);
    let other_game_account = FakeSession::new(1, 12);
    mgr.on_session_opened_like_cpp(&other_game_account).await;
    assert_eq!(mgr.lock().sessions[&1].len(), 2);
    // Tokens count per session like C++ `_battlenetRequestToken`.
    second.drain();
    mgr.presence_subscribe_like_cpp(second.agent(), &account_entity_id_like_cpp(1), &[])
        .unwrap();
    mgr.presence_subscribe_like_cpp(second.agent(), &account_entity_id_like_cpp(1), &[])
        .unwrap();
    let tokens: Vec<u32> = second.drain().iter().map(|n| n.token).collect();
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[1], tokens[0] + 1);
    assert!(
        first.drain().is_empty(),
        "replaced channel receives nothing"
    );
}
