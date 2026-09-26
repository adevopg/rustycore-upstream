//! `CMSG_BATTLENET_REQUEST` method tables of `FriendsService`, `PresenceService`
//! and `UserManagerService` (the generated `CallServerMethod` switches of
//! `friends_service.pb.cc` / `presence_service.pb.cc` / `user_manager_service.pb.cc`),
//! bound to [`BnetFriendsMgr`].

use prost::Message;
use tracing::debug;
use wow_persistence::BnetAccountLookupLikeCpp;
use wow_proto::bgs::protocol::friends::v1 as friends;
use wow_proto::bgs::protocol::presence::v1 as presence;
use wow_proto::bgs::protocol::user_manager::v1 as user_manager;
use wow_proto::bgs::protocol::{InvitationRemovedReason, NoData};
use wow_proto::status;

use super::manager::{BnetFriendsMgr, deliver_like_cpp};
use super::session_port::BnetAgentLikeCpp;
use super::{BnetEntityKindLikeCpp, entity_kind_like_cpp};
use crate::bnet_services::{
    BattlenetReplyLikeCpp, continuation_like_cpp, unimplemented_method_like_cpp,
};

fn decode_like_cpp<M: Message + Default>(
    method: &str,
    data: &[u8],
) -> Result<M, BattlenetReplyLikeCpp> {
    match M::decode(data) {
        Ok(request) => {
            debug!("{method}: {request:?}");
            Ok(request)
        }
        Err(_) => {
            debug!("Failed to parse request for {method}");
            Err(BattlenetReplyLikeCpp::Status(
                status::ERROR_RPC_MALFORMED_REQUEST,
            ))
        }
    }
}

fn no_data_like_cpp(result: Result<(), u32>) -> BattlenetReplyLikeCpp {
    continuation_like_cpp(result.map(|()| NoData {}))
}

/// `FriendInvitationParams` -> how the invitee is looked up.
pub(crate) fn invitation_lookup_like_cpp(
    request: &friends::SendInvitationRequest,
) -> Result<BnetAccountLookupLikeCpp, u32> {
    if let Some(params) = &request.params.friend_params {
        if let Some(battle_tag) = params
            .target_battle_tag
            .as_deref()
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
        {
            return Ok(BnetAccountLookupLikeCpp::BattleTag(battle_tag.to_owned()));
        }
        if let Some(email) = params
            .target_email
            .as_deref()
            .map(str::trim)
            .filter(|email| !email.is_empty())
        {
            return Ok(BnetAccountLookupLikeCpp::Email(email.to_owned()));
        }
    }
    match entity_kind_like_cpp(&request.target_id) {
        Some(BnetEntityKindLikeCpp::Account(account_id)) if account_id != 0 => {
            Ok(BnetAccountLookupLikeCpp::Id(account_id))
        }
        _ => Err(status::ERROR_INVALID_ARGS),
    }
}

/// C++ `FriendsService::CallServerMethod`.
pub(crate) async fn call_friends_service_method_like_cpp(
    mgr: &BnetFriendsMgr,
    agent: BnetAgentLikeCpp,
    sender: flume::Sender<Vec<u8>>,
    method_id: u32,
    data: &[u8],
) -> BattlenetReplyLikeCpp {
    deliver_like_cpp(mgr.register_session_like_cpp(agent, sender));
    match method_id & 0x3FFF_FFFF {
        1 => match decode_like_cpp::<friends::SubscribeRequest>("FriendsService.Subscribe", data) {
            Ok(_) => continuation_like_cpp(mgr.subscribe_like_cpp(agent)),
            Err(reply) => reply,
        },
        2 => match decode_like_cpp::<friends::SendInvitationRequest>(
            "FriendsService.SendInvitation",
            data,
        ) {
            Ok(request) => match invitation_lookup_like_cpp(&request) {
                Ok(lookup) => no_data_like_cpp(
                    mgr.send_invitation_like_cpp(
                        agent,
                        lookup,
                        request.params.invitation_message.unwrap_or_default(),
                    )
                    .await,
                ),
                Err(status) => BattlenetReplyLikeCpp::Status(status),
            },
            Err(reply) => reply,
        },
        3 => match decode_like_cpp::<friends::AcceptInvitationRequest>(
            "FriendsService.AcceptInvitation",
            data,
        ) {
            Ok(request) => no_data_like_cpp(
                mgr.accept_invitation_like_cpp(agent, request.invitation_id)
                    .await,
            ),
            Err(reply) => reply,
        },
        4 => match decode_like_cpp::<friends::RevokeInvitationRequest>(
            "FriendsService.RevokeInvitation",
            data,
        ) {
            Ok(request) => no_data_like_cpp(
                mgr.remove_invitation_like_cpp(
                    agent,
                    request.invitation_id.unwrap_or_default(),
                    InvitationRemovedReason::Revoked,
                )
                .await,
            ),
            Err(reply) => reply,
        },
        5 => match decode_like_cpp::<friends::DeclineInvitationRequest>(
            "FriendsService.DeclineInvitation",
            data,
        ) {
            Ok(request) => no_data_like_cpp(
                mgr.remove_invitation_like_cpp(
                    agent,
                    request.invitation_id,
                    InvitationRemovedReason::Declined,
                )
                .await,
            ),
            Err(reply) => reply,
        },
        6 => match decode_like_cpp::<friends::IgnoreInvitationRequest>(
            "FriendsService.IgnoreInvitation",
            data,
        ) {
            Ok(request) => no_data_like_cpp(
                mgr.remove_invitation_like_cpp(
                    agent,
                    request.invitation_id,
                    InvitationRemovedReason::Ignored,
                )
                .await,
            ),
            Err(reply) => reply,
        },
        8 => match decode_like_cpp::<friends::RemoveFriendRequest>(
            "FriendsService.RemoveFriend",
            data,
        ) {
            Ok(request) => {
                no_data_like_cpp(mgr.remove_friend_like_cpp(agent, &request.target_id).await)
            }
            Err(reply) => reply,
        },
        9 => {
            match decode_like_cpp::<friends::ViewFriendsRequest>("FriendsService.ViewFriends", data)
            {
                Ok(request) => continuation_like_cpp(
                    mgr.view_friends_like_cpp(agent, &request.target_id)
                        .map(|friends| friends::ViewFriendsResponse { friends }),
                ),
                Err(reply) => reply,
            }
        }
        10 => match decode_like_cpp::<friends::UpdateFriendStateRequest>(
            "FriendsService.UpdateFriendState",
            data,
        ) {
            Ok(request) => no_data_like_cpp(
                mgr.update_friend_state_like_cpp(agent, &request.target_id, &request.attribute)
                    .await,
            ),
            Err(reply) => reply,
        },
        11 => {
            match decode_like_cpp::<friends::UnsubscribeRequest>("FriendsService.Unsubscribe", data)
            {
                Ok(_) => {
                    mgr.unsubscribe_like_cpp(agent);
                    no_data_like_cpp(Ok(()))
                }
                Err(reply) => reply,
            }
        }
        12 => unimplemented_method_like_cpp::<friends::RevokeAllInvitationsRequest>(
            "FriendsService.RevokeAllInvitations",
            data,
        ),
        13 => unimplemented_method_like_cpp::<friends::GetFriendListRequest>(
            "FriendsService.GetFriendList",
            data,
        ),
        14 => unimplemented_method_like_cpp::<friends::CreateFriendshipRequest>(
            "FriendsService.CreateFriendship",
            data,
        ),
        _ => {
            debug!("Client tried to call invalid FriendsService method {method_id}");
            BattlenetReplyLikeCpp::Status(status::ERROR_RPC_INVALID_METHOD)
        }
    }
}

/// C++ `PresenceService::CallServerMethod`.
pub(crate) fn call_presence_service_method_like_cpp(
    mgr: &BnetFriendsMgr,
    agent: BnetAgentLikeCpp,
    sender: flume::Sender<Vec<u8>>,
    method_id: u32,
    data: &[u8],
) -> BattlenetReplyLikeCpp {
    deliver_like_cpp(mgr.register_session_like_cpp(agent, sender));
    match method_id & 0x3FFF_FFFF {
        1 => match decode_like_cpp::<presence::SubscribeRequest>("PresenceService.Subscribe", data)
        {
            Ok(request) => no_data_like_cpp(mgr.presence_subscribe_like_cpp(
                agent,
                &request.entity_id,
                &request.key,
            )),
            Err(reply) => reply,
        },
        2 => match decode_like_cpp::<presence::UnsubscribeRequest>(
            "PresenceService.Unsubscribe",
            data,
        ) {
            Ok(request) => {
                mgr.presence_unsubscribe_like_cpp(agent, &request.entity_id);
                no_data_like_cpp(Ok(()))
            }
            Err(reply) => reply,
        },
        3 => match decode_like_cpp::<presence::UpdateRequest>("PresenceService.Update", data) {
            Ok(request) => no_data_like_cpp(mgr.presence_update_like_cpp(
                agent,
                &request.entity_id,
                &request.field_operation,
            )),
            Err(reply) => reply,
        },
        4 => match decode_like_cpp::<presence::QueryRequest>("PresenceService.Query", data) {
            Ok(request) => continuation_like_cpp(mgr.presence_query_like_cpp(
                agent,
                &request.entity_id,
                &request.key,
            )),
            Err(reply) => reply,
        },
        8 => match decode_like_cpp::<presence::BatchSubscribeRequest>(
            "PresenceService.BatchSubscribe",
            data,
        ) {
            Ok(request) => continuation_like_cpp(Ok(mgr.presence_batch_subscribe_like_cpp(
                agent,
                &request.entity_id,
                &request.key,
            ))),
            Err(reply) => reply,
        },
        9 => match decode_like_cpp::<presence::BatchUnsubscribeRequest>(
            "PresenceService.BatchUnsubscribe",
            data,
        ) {
            Ok(request) => {
                for entity in &request.entity_id {
                    mgr.presence_unsubscribe_like_cpp(agent, entity);
                }
                no_data_like_cpp(Ok(()))
            }
            Err(reply) => reply,
        },
        _ => {
            debug!("Client tried to call invalid PresenceService method {method_id}");
            BattlenetReplyLikeCpp::Status(status::ERROR_RPC_INVALID_METHOD)
        }
    }
}

/// C++ `UserManagerService::CallServerMethod`: an empty block list; blocking
/// and recent players stay `ERROR_RPC_NOT_IMPLEMENTED` for now.
pub(crate) fn call_user_manager_service_method_like_cpp(
    mgr: &BnetFriendsMgr,
    agent: BnetAgentLikeCpp,
    sender: flume::Sender<Vec<u8>>,
    method_id: u32,
    data: &[u8],
) -> BattlenetReplyLikeCpp {
    deliver_like_cpp(mgr.register_session_like_cpp(agent, sender));
    match method_id & 0x3FFF_FFFF {
        1 => match decode_like_cpp::<user_manager::SubscribeRequest>(
            "UserManagerService.Subscribe",
            data,
        ) {
            Ok(_) => continuation_like_cpp(Ok(user_manager::SubscribeResponse::default())),
            Err(reply) => reply,
        },
        10 => unimplemented_method_like_cpp::<user_manager::AddRecentPlayersRequest>(
            "UserManagerService.AddRecentPlayers",
            data,
        ),
        11 => unimplemented_method_like_cpp::<user_manager::ClearRecentPlayersRequest>(
            "UserManagerService.ClearRecentPlayers",
            data,
        ),
        20 => unimplemented_method_like_cpp::<user_manager::BlockPlayerRequest>(
            "UserManagerService.BlockPlayer",
            data,
        ),
        21 => unimplemented_method_like_cpp::<user_manager::UnblockPlayerRequest>(
            "UserManagerService.UnblockPlayer",
            data,
        ),
        40 => unimplemented_method_like_cpp::<user_manager::BlockPlayerRequest>(
            "UserManagerService.BlockPlayerForSession",
            data,
        ),
        51 => match decode_like_cpp::<user_manager::UnsubscribeRequest>(
            "UserManagerService.Unsubscribe",
            data,
        ) {
            Ok(_) => no_data_like_cpp(Ok(())),
            Err(reply) => reply,
        },
        _ => {
            debug!("Client tried to call invalid UserManagerService method {method_id}");
            BattlenetReplyLikeCpp::Status(status::ERROR_RPC_INVALID_METHOD)
        }
    }
}
