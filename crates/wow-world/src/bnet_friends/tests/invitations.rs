//! Invitation lifecycle, friend removal, notes and the subscribe response.

use wow_proto::bgs::protocol::friends::v1::{
    FriendNotification, InvitationNotification, SentInvitationAddedNotification,
    SentInvitationRemovedNotification, UpdateFriendStateNotification,
};
use wow_proto::bgs::protocol::presence::v1::StateChangedNotification;
use wow_proto::bgs::protocol::{Attribute, InvitationRemovedReason, Variant};
use wow_proto::{service_hash, status};

use super::*;
use crate::bnet_friends::manager::*;

async fn open(mgr: &BnetFriendsMgr, session: &FakeSession) {
    mgr.on_session_opened_like_cpp(session).await;
    session.drain();
}

fn battle_tag(tag: &str) -> BnetAccountLookupLikeCpp {
    BnetAccountLookupLikeCpp::BattleTag(tag.to_owned())
}

#[tokio::test]
async fn send_invitation_persists_the_row_and_notifies_both_sides() {
    let (mgr, persistence) = manager_with(fixture_load()).await;
    let alpha = FakeSession::new(1, 11);
    let beta = FakeSession::new(2, 22);
    open(&mgr, &alpha).await;
    open(&mgr, &beta).await;

    mgr.send_invitation_like_cpp(alpha.agent(), battle_tag("beta#0002"), "hi".to_owned())
        .await
        .unwrap();

    assert_eq!(
        persistence.calls(),
        [format!("insert_invitation(1 1->2 created={NOW})")]
    );
    let received = beta.drain();
    let added = find(
        &received,
        service_hash::FRIENDS_LISTENER,
        FRIENDS_LISTENER_ON_RECEIVED_INVITATION_ADDED,
    );
    assert_eq!(added.len(), 1);
    let notification: InvitationNotification = added[0].decode();
    assert_eq!(notification.invitation.id, 1);
    assert_eq!(
        notification.invitation.inviter_name.as_deref(),
        Some("Alpha#0001")
    );
    assert_eq!(
        notification.invitation.invitee_name.as_deref(),
        Some("Beta#0002")
    );
    assert_eq!(
        notification.invitation.inviter_identity.account_id,
        Some(account_entity_id_like_cpp(1))
    );
    assert_eq!(notification.account_id, Some(account_entity_id_like_cpp(2)));
    assert_eq!(
        added[0].token, 1,
        "token 0 was the session's own registration presence push"
    );

    let sent = alpha.drain();
    let sent_added = find(
        &sent,
        service_hash::FRIENDS_LISTENER,
        FRIENDS_LISTENER_ON_SENT_INVITATION_ADDED,
    );
    assert_eq!(sent_added.len(), 1);
    let notification: SentInvitationAddedNotification = sent_added[0].decode();
    assert_eq!(
        notification.invitation.unwrap().target_name.as_deref(),
        Some("Beta#0002")
    );
    assert!(mgr.lock().invitations.contains_key(&1));
}

#[tokio::test]
async fn send_invitation_refusals_match_the_battlenet_statuses() {
    let (mgr, persistence) = manager_with(fixture_load()).await;
    let alpha = FakeSession::new(1, 11);
    open(&mgr, &alpha).await;
    let agent = alpha.agent();

    assert_eq!(
        mgr.send_invitation_like_cpp(agent, battle_tag("Alpha#0001"), String::new())
            .await,
        Err(status::ERROR_INVALID_ARGS),
        "self"
    );
    assert_eq!(
        mgr.send_invitation_like_cpp(agent, battle_tag("Nobody#9999"), String::new())
            .await,
        Err(status::ERROR_NOT_EXISTS)
    );
    assert!(
        persistence
            .calls()
            .iter()
            .any(|call| call.starts_with("find_account(BattleTag"))
    );

    mgr.send_invitation_like_cpp(agent, battle_tag("Beta#0002"), String::new())
        .await
        .unwrap();
    assert_eq!(
        mgr.send_invitation_like_cpp(agent, battle_tag("Beta#0002"), String::new())
            .await,
        Err(status::ERROR_FRIENDS_INVITATION_ALREADY_EXISTS)
    );
    let beta = BnetAgentLikeCpp {
        account_id: 2,
        game_account_id: 22,
    };
    assert_eq!(
        mgr.send_invitation_like_cpp(
            beta,
            BnetAccountLookupLikeCpp::Email("ALPHA@example.test".to_owned()),
            String::new()
        )
        .await,
        Err(status::ERROR_FRIENDS_INVITATION_ALREADY_EXISTS),
        "reverse duplicate"
    );

    mgr.accept_invitation_like_cpp(beta, 1).await.unwrap();
    assert_eq!(
        mgr.send_invitation_like_cpp(agent, BnetAccountLookupLikeCpp::Id(2), String::new())
            .await,
        Err(status::ERROR_FRIENDS_FRIENDSHIP_ALREADY_EXISTS)
    );

    persistence
        .fail_writes
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        mgr.send_invitation_like_cpp(agent, battle_tag("Gamma#0003"), String::new())
            .await,
        Err(status::ERROR_INTERNAL)
    );
    assert!(
        mgr.lock().invitations.is_empty(),
        "not persisted -> not kept"
    );
}

#[tokio::test]
async fn invitation_limits_are_enforced() {
    let (mgr, _persistence) = manager_with(fixture_load()).await;
    let mgr = BnetFriendsMgr::new(Arc::new(FakePersistence {
        load: Mutex::new(fixture_load()),
        ..Default::default()
    }))
    .with_limits(BnetFriendsLimitsLikeCpp {
        max_friends: 1,
        max_received_invitations: 1,
        max_sent_invitations: 1,
    })
    .with_clock(|| NOW);
    mgr.load_from_db_like_cpp().await.unwrap();
    let alpha = BnetAgentLikeCpp {
        account_id: 1,
        game_account_id: 11,
    };
    let gamma = BnetAgentLikeCpp {
        account_id: 3,
        game_account_id: 33,
    };
    mgr.send_invitation_like_cpp(alpha, battle_tag("Beta#0002"), String::new())
        .await
        .unwrap();
    assert_eq!(
        mgr.send_invitation_like_cpp(alpha, battle_tag("Gamma#0003"), String::new())
            .await,
        Err(status::ERROR_FRIENDS_TOO_MANY_SENT_INVITATIONS)
    );
    assert_eq!(
        mgr.send_invitation_like_cpp(gamma, battle_tag("Beta#0002"), String::new())
            .await,
        Err(status::ERROR_FRIENDS_TOO_MANY_RECEIVED_INVITATIONS)
    );
    mgr.accept_invitation_like_cpp(
        BnetAgentLikeCpp {
            account_id: 2,
            game_account_id: 22,
        },
        1,
    )
    .await
    .unwrap();
    assert_eq!(
        mgr.send_invitation_like_cpp(alpha, battle_tag("Gamma#0003"), String::new())
            .await,
        Err(status::ERROR_FRIENDS_INVITER_AT_MAX_FRIENDS)
    );
    assert_eq!(
        mgr.send_invitation_like_cpp(gamma, battle_tag("Beta#0002"), String::new())
            .await,
        Err(status::ERROR_FRIENDS_INVITEE_AT_MAX_FRIENDS)
    );
}

#[tokio::test]
async fn accept_creates_both_directions_and_notifies_friends_invitations_and_presence() {
    let mut load = fixture_load();
    load.invitations = vec![invitation_row(5, 1, 2)];
    let (mgr, persistence) = manager_with(load).await;
    let alpha = FakeSession::new(1, 11).with_character("Alphachar", 60, 1519);
    let beta = FakeSession::new(2, 22);
    open(&mgr, &alpha).await;
    open(&mgr, &beta).await;
    mgr.on_player_login_like_cpp(&alpha);
    alpha.drain();

    assert_eq!(
        mgr.accept_invitation_like_cpp(alpha.agent(), 5).await,
        Err(status::ERROR_FRIENDS_INVALID_INVITATION),
        "only the invitee accepts"
    );
    assert_eq!(
        mgr.accept_invitation_like_cpp(beta.agent(), 99).await,
        Err(status::ERROR_FRIENDS_INVALID_INVITATION)
    );
    mgr.accept_invitation_like_cpp(beta.agent(), 5)
        .await
        .unwrap();

    assert_eq!(persistence.calls(), ["accept_invitation(5 1->2)"]);
    {
        let state = mgr.lock();
        assert!(state.invitations.is_empty());
        assert!(state.are_friends(1, 2) && state.are_friends(2, 1));
        assert_eq!(state.friends[&1][&2].creation_time, NOW);
    }

    let beta_seen = beta.drain();
    let removed: InvitationNotification = find(
        &beta_seen,
        service_hash::FRIENDS_LISTENER,
        FRIENDS_LISTENER_ON_RECEIVED_INVITATION_REMOVED,
    )[0]
    .decode();
    assert_eq!(
        removed.reason,
        Some(InvitationRemovedReason::Accepted as u32)
    );
    assert_eq!(removed.invitation.id, 5);
    let added: FriendNotification = find(
        &beta_seen,
        service_hash::FRIENDS_LISTENER,
        FRIENDS_LISTENER_ON_FRIEND_ADDED,
    )[0]
    .decode();
    assert_eq!(added.target.account_id, account_entity_id_like_cpp(1));
    assert_eq!(added.target.role, vec![1]);
    assert_eq!(added.account_id, Some(account_entity_id_like_cpp(2)));
    // Full presence of the new friend (account + game account channels).
    let presence: StateChangedNotification = find(
        &beta_seen,
        service_hash::PRESENCE_LISTENER,
        PRESENCE_LISTENER_ON_STATE_CHANGED,
    )[0]
    .decode();
    assert_eq!(presence.subscriber_id.unwrap().id, 2);
    let entities: Vec<_> = presence
        .state
        .iter()
        .map(|s| s.entity_id.unwrap())
        .collect();
    assert_eq!(
        entities,
        [
            account_entity_id_like_cpp(1),
            game_account_entity_id_like_cpp(11)
        ]
    );
    assert!(
        presence.state[1]
            .field_operation
            .iter()
            .any(|op| { op.field.value.string_value.as_deref() == Some("Alphachar") })
    );

    let alpha_seen = alpha.drain();
    let sent_removed: SentInvitationRemovedNotification = find(
        &alpha_seen,
        service_hash::FRIENDS_LISTENER,
        FRIENDS_LISTENER_ON_SENT_INVITATION_REMOVED,
    )[0]
    .decode();
    assert_eq!(sent_removed.invitation_id, Some(5));
    assert_eq!(sent_removed.reason, Some(0));
    let added: FriendNotification = find(
        &alpha_seen,
        service_hash::FRIENDS_LISTENER,
        FRIENDS_LISTENER_ON_FRIEND_ADDED,
    )[0]
    .decode();
    assert_eq!(added.target.account_id, account_entity_id_like_cpp(2));
    assert_eq!(
        find(
            &alpha_seen,
            service_hash::PRESENCE_LISTENER,
            PRESENCE_LISTENER_ON_STATE_CHANGED
        )
        .len(),
        1
    );
}

#[tokio::test]
async fn accept_keeps_memory_unchanged_when_the_transaction_did_not_apply() {
    let mut load = fixture_load();
    load.invitations = vec![invitation_row(5, 1, 2)];
    let (mgr, persistence) = manager_with(load).await;
    persistence
        .unknown_writes
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let beta = BnetAgentLikeCpp {
        account_id: 2,
        game_account_id: 22,
    };
    assert_eq!(
        mgr.accept_invitation_like_cpp(beta, 5).await,
        Err(status::ERROR_INTERNAL)
    );
    let state = mgr.lock();
    assert!(state.invitations.contains_key(&5));
    assert!(!state.are_friends(1, 2));
}

#[tokio::test]
async fn decline_revoke_and_ignore_delete_the_invitation_with_their_reason() {
    for (reason, by_inviter) in [
        (InvitationRemovedReason::Declined, false),
        (InvitationRemovedReason::Revoked, true),
        (InvitationRemovedReason::Ignored, false),
    ] {
        let mut load = fixture_load();
        load.invitations = vec![invitation_row(5, 1, 2)];
        let (mgr, persistence) = manager_with(load).await;
        let alpha = FakeSession::new(1, 11);
        let beta = FakeSession::new(2, 22);
        open(&mgr, &alpha).await;
        open(&mgr, &beta).await;
        let (allowed, forbidden) = if by_inviter {
            (alpha.agent(), beta.agent())
        } else {
            (beta.agent(), alpha.agent())
        };
        assert_eq!(
            mgr.remove_invitation_like_cpp(forbidden, 5, reason).await,
            Err(status::ERROR_FRIENDS_INVALID_INVITATION),
            "{reason:?}"
        );
        mgr.remove_invitation_like_cpp(allowed, 5, reason)
            .await
            .unwrap();
        assert_eq!(persistence.calls(), ["delete_invitation(5)"]);
        assert!(mgr.lock().invitations.is_empty());
        assert!(!mgr.lock().are_friends(1, 2));

        let removed: InvitationNotification = find(
            &beta.drain(),
            service_hash::FRIENDS_LISTENER,
            FRIENDS_LISTENER_ON_RECEIVED_INVITATION_REMOVED,
        )[0]
        .decode();
        assert_eq!(removed.reason, Some(reason as u32));
        let sent_removed: SentInvitationRemovedNotification = find(
            &alpha.drain(),
            service_hash::FRIENDS_LISTENER,
            FRIENDS_LISTENER_ON_SENT_INVITATION_REMOVED,
        )[0]
        .decode();
        assert_eq!(sent_removed.reason, Some(reason as u32));
    }
}

#[tokio::test]
async fn remove_friend_deletes_both_directions_and_notifies_both_sides() {
    let mut load = fixture_load();
    load.links = vec![link(1, 2, ""), link(2, 1, "")];
    let (mgr, persistence) = manager_with(load).await;
    let alpha = FakeSession::new(1, 11);
    let beta = FakeSession::new(2, 22);
    open(&mgr, &alpha).await;
    open(&mgr, &beta).await;

    assert_eq!(
        mgr.remove_friend_like_cpp(alpha.agent(), &account_entity_id_like_cpp(3))
            .await,
        Err(status::ERROR_FRIENDS_FRIENDSHIP_DOES_NOT_EXIST)
    );
    assert_eq!(
        mgr.remove_friend_like_cpp(alpha.agent(), &game_account_entity_id_like_cpp(22))
            .await,
        Err(status::ERROR_INVALID_ARGS)
    );
    mgr.remove_friend_like_cpp(alpha.agent(), &account_entity_id_like_cpp(2))
        .await
        .unwrap();

    assert_eq!(persistence.calls(), ["delete_friendship(1,2)"]);
    assert!(!mgr.lock().are_friends(1, 2));
    assert!(!mgr.lock().are_friends(2, 1));
    let removed: FriendNotification = find(
        &alpha.drain(),
        service_hash::FRIENDS_LISTENER,
        FRIENDS_LISTENER_ON_FRIEND_REMOVED,
    )[0]
    .decode();
    assert_eq!(removed.target.account_id, account_entity_id_like_cpp(2));
    let removed: FriendNotification = find(
        &beta.drain(),
        service_hash::FRIENDS_LISTENER,
        FRIENDS_LISTENER_ON_FRIEND_REMOVED,
    )[0]
    .decode();
    assert_eq!(removed.target.account_id, account_entity_id_like_cpp(1));
    assert_eq!(removed.account_id, Some(account_entity_id_like_cpp(2)));
}

#[tokio::test]
async fn subscribe_response_lists_friends_invitations_limits_and_roles() {
    let mut load = fixture_load();
    load.links = vec![link(1, 2, "raid buddy"), link(2, 1, "")];
    load.invitations = vec![invitation_row(7, 3, 1), invitation_row(8, 1, 3)];
    let (mgr, _persistence) = manager_with(load).await;
    let alpha = FakeSession::new(1, 11);
    open(&mgr, &alpha).await;

    let response = mgr.subscribe_like_cpp(alpha.agent()).unwrap();
    assert_eq!(response.max_friends, Some(200));
    assert_eq!(response.max_received_invitations, Some(20));
    assert_eq!(response.max_sent_invitations, Some(20));
    assert_eq!(
        response
            .role
            .iter()
            .map(|role| (role.id, role.name.as_str()))
            .collect::<Vec<_>>(),
        [(1, "battle_tag_friend"), (2, "real_id_friend")]
    );
    assert_eq!(response.friends.len(), 1);
    let friend = &response.friends[0];
    assert_eq!(friend.account_id, account_entity_id_like_cpp(2));
    assert_eq!(friend.role, vec![1]);
    assert_eq!(friend.attribute.len(), 1);
    assert_eq!(friend.attribute[0].name, "friend_note");
    assert_eq!(
        friend.attribute[0].value.string_value.as_deref(),
        Some("raid buddy")
    );
    assert_eq!(response.received_invitations.len(), 1);
    assert_eq!(response.received_invitations[0].id, 7);
    assert_eq!(
        response.received_invitations[0].inviter_name.as_deref(),
        Some("Gamma#0003")
    );
    assert_eq!(response.received_invitations[0].program, Some(0x0057_6F57));
    assert_eq!(response.sent_invitations.len(), 1);
    assert_eq!(response.sent_invitations[0].id, Some(8));
    assert_eq!(
        response.sent_invitations[0].target_name.as_deref(),
        Some("Gamma#0003")
    );
    assert!(
        mgr.lock()
            .endpoint_mut(alpha.agent())
            .unwrap()
            .friends_subscribed
    );
    mgr.unsubscribe_like_cpp(alpha.agent());
    assert!(
        !mgr.lock()
            .endpoint_mut(alpha.agent())
            .unwrap()
            .friends_subscribed
    );

    // ViewFriends: a friend's friends; a stranger's list is empty.
    let of_beta = mgr
        .view_friends_like_cpp(alpha.agent(), &account_entity_id_like_cpp(2))
        .unwrap();
    assert_eq!(of_beta.len(), 1);
    assert_eq!(of_beta[0].battle_tag.as_deref(), Some("Alpha#0001"));
    assert!(
        mgr.view_friends_like_cpp(alpha.agent(), &account_entity_id_like_cpp(3))
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn update_friend_state_persists_the_note_and_echoes_the_attribute_name() {
    let mut load = fixture_load();
    load.links = vec![link(1, 2, ""), link(2, 1, "")];
    let (mgr, persistence) = manager_with(load).await;
    let alpha = FakeSession::new(1, 11);
    open(&mgr, &alpha).await;
    let note = |name: &str, text: &str| {
        vec![Attribute {
            name: name.to_owned(),
            value: Variant {
                string_value: Some(text.to_owned()),
                ..Default::default()
            },
        }]
    };

    assert_eq!(
        mgr.update_friend_state_like_cpp(
            alpha.agent(),
            &account_entity_id_like_cpp(3),
            &note("friend_note", "x")
        )
        .await,
        Err(status::ERROR_FRIENDS_FRIENDSHIP_DOES_NOT_EXIST)
    );
    assert_eq!(
        mgr.update_friend_state_like_cpp(
            alpha.agent(),
            &account_entity_id_like_cpp(2),
            &note("friend_note", &"x".repeat(128))
        )
        .await,
        Err(status::ERROR_FRIENDS_NOTE_MAX_SIZE_EXCEEDED)
    );
    // Unknown attributes are logged and acknowledged without a write.
    mgr.update_friend_state_like_cpp(
        alpha.agent(),
        &account_entity_id_like_cpp(2),
        &note("favorite", "1"),
    )
    .await
    .unwrap();
    assert!(persistence.calls().is_empty());

    mgr.update_friend_state_like_cpp(
        alpha.agent(),
        &account_entity_id_like_cpp(2),
        &note("Note", "healer"),
    )
    .await
    .unwrap();
    assert_eq!(persistence.calls(), ["update_friend_note(1,2,\"healer\")"]);
    let changed: UpdateFriendStateNotification = find(
        &alpha.drain(),
        service_hash::FRIENDS_LISTENER,
        FRIENDS_LISTENER_ON_UPDATE_FRIEND_STATE,
    )[0]
    .decode();
    assert_eq!(
        changed.changed_friend.account_id,
        account_entity_id_like_cpp(2)
    );
    assert_eq!(changed.changed_friend.attribute[0].name, "Note");
    assert_eq!(
        changed.changed_friend.attribute[0]
            .value
            .string_value
            .as_deref(),
        Some("healer")
    );
    assert_eq!(
        mgr.subscribe_like_cpp(alpha.agent()).unwrap().friends[0].attribute[0].name,
        "Note"
    );
}
