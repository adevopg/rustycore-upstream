//! Protobuf message types for the Battle.net RPC protocol.
//!
//! Generated from `.proto` definitions via `prost-build`. The proto files
//! follow the Blizzard BNet protocol structure used by WoW 3.4.3 clients.
//!
//! # Module Structure
//!
//! The generated code mirrors the protobuf package hierarchy:
//! - `bgs::protocol` — Core RPC types (Header, ProcessId, EntityId, Attribute, etc.)
//! - `bgs::protocol::authentication::v1` — Authentication messages
//! - `bgs::protocol::connection::v1` — Connection messages
//! - `bgs::protocol::challenge::v1` — Challenge messages
//! - `bgs::protocol::game_utilities::v1` — GameUtilities messages
//! - `bgs::protocol::account::v1` — Account messages
//! - `bgs::protocol::friends::v1` — Friends service/listener messages
//! - `bgs::protocol::presence::v1` — Presence service/listener messages
//! - `bgs::protocol::user_manager::v1` — UserManager (block list) messages
//!
//! `bgs::protocol` also carries `Invitation`, `InvitationParams`, `Role` and
//! `RoleState` (invitation_types.proto / role_types.proto).

// Include the generated protobuf code.
// prost-build generates one file per package, named by the package path.
pub mod bgs {
    pub mod protocol {
        // Core types: Header, ProcessId, NoData, EntityId, Attribute, Variant, etc.
        include!(concat!(env!("OUT_DIR"), "/bgs.protocol.rs"));

        pub mod authentication {
            pub mod v1 {
                include!(concat!(
                    env!("OUT_DIR"),
                    "/bgs.protocol.authentication.v1.rs"
                ));
            }
        }

        pub mod connection {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/bgs.protocol.connection.v1.rs"));
            }
        }

        pub mod challenge {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/bgs.protocol.challenge.v1.rs"));
            }
        }

        pub mod game_utilities {
            pub mod v1 {
                include!(concat!(
                    env!("OUT_DIR"),
                    "/bgs.protocol.game_utilities.v1.rs"
                ));
            }
        }

        pub mod account {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/bgs.protocol.account.v1.rs"));
            }
        }

        pub mod friends {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/bgs.protocol.friends.v1.rs"));
            }
        }

        pub mod presence {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/bgs.protocol.presence.v1.rs"));
            }
        }

        pub mod user_manager {
            pub mod v1 {
                include!(concat!(env!("OUT_DIR"), "/bgs.protocol.user_manager.v1.rs"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Service hash constants (from C# OriginalHash enum)
// ---------------------------------------------------------------------------

/// Service hashes used for BNet RPC dispatch.
///
/// The `service_hash` field in `Header` identifies which service a request
/// targets. These are fixed values defined by the Blizzard BNet protocol.
pub mod realm_list_json;

pub mod service_hash {
    // Server-side services (server handles client requests)
    pub const AUTHENTICATION_SERVICE: u32 = 0x0DEC_FC01;
    pub const CONNECTION_SERVICE: u32 = 0x6544_6991;
    pub const ACCOUNT_SERVICE: u32 = 0x62DA_0891;
    pub const GAME_UTILITIES_SERVICE: u32 = 0x3FC1_274D;

    // Client-side listeners (server sends notifications to client)
    pub const AUTHENTICATION_LISTENER: u32 = 0x7124_0E35;
    pub const CHALLENGE_LISTENER: u32 = 0xBBDA_171F;
    pub const ACCOUNT_LISTENER: u32 = 0x54DF_DA17;

    // Other services (not currently used by BNet server)
    pub const FRIENDS_SERVICE: u32 = 0xA3DD_B1BD;
    pub const FRIENDS_LISTENER: u32 = 0x6F25_9A13;
    pub const PRESENCE_SERVICE: u32 = 0xFA07_96FF;
    pub const PRESENCE_LISTENER: u32 = 0x890A_B85F;
    pub const REPORT_SERVICE: u32 = 0x7CAF_61C9;
    pub const REPORT_SERVICE_V2: u32 = 0x3A42_18FB;
    pub const RESOURCES_SERVICE: u32 = 0xECBE_75BA;
    pub const USER_MANAGER_SERVICE: u32 = 0x3E19_268A;
    /// `bgs.protocol.club.v1.membership.ClubMembershipService` OriginalHash.
    pub const CLUB_MEMBERSHIP_SERVICE: u32 = 0x94B9_4786;
    /// `bgs.protocol.club.v1.ClubService` OriginalHash.
    pub const CLUB_SERVICE: u32 = 0xE273_DE0E;
    pub const USER_MANAGER_LISTENER: u32 = 0xBC87_2C22;
}

/// BNet RPC status codes.
pub mod status {
    pub const OK: u32 = 0;
    pub const ERROR_INTERNAL: u32 = 1;
    pub const ERROR_TIMED_OUT: u32 = 2;
    pub const ERROR_DENIED: u32 = 3;
    pub const ERROR_RPC_INVALID_METHOD: u32 = 0x0000_0BC3;
    pub const ERROR_RPC_MALFORMED_REQUEST: u32 = 0x0000_0BC5;
    pub const ERROR_RPC_NOT_IMPLEMENTED: u32 = 0x0000_0BC7;
    pub const ERROR_BAD_PROGRAM: u32 = 0x4D;
    pub const ERROR_BAD_LOCALE: u32 = 0x4E;
    pub const ERROR_BAD_PLATFORM: u32 = 0x4F;
    pub const ERROR_NO_GAME_ACCOUNT: u32 = 12;
    pub const ERROR_GAME_ACCOUNT_BANNED: u32 = 0x34;
    pub const ERROR_GAME_ACCOUNT_SUSPENDED: u32 = 0x35;
    pub const ERROR_UTIL_SERVER_UNKNOWN_REALM: u32 = 0x8000_0069;
    pub const ERROR_UTIL_SERVER_INVALID_IDENTITY_ARGS: u32 = 0x8000_006E;
    pub const ERROR_UTIL_SERVER_FAILED_TO_SERIALIZE_RESPONSE: u32 = 0x8000_0073;
    pub const ERROR_USER_SERVER_BAD_WOW_ACCOUNT: u32 = 0x8000_00D3;
    pub const ERROR_USER_SERVER_NOT_PERMITTED_ON_REALM: u32 = 0x8000_00E1;
    pub const ERROR_WOW_SERVICES_INVALID_JOIN_TICKET: u32 = 0x8000_012E;
    pub const ERROR_WOW_SERVICES_DENIED_REALM_LIST_TICKET: u32 = 0x8000_0132;
    pub const ERROR_WOW_SERVICES_GAME_ACCOUNT_LOCKED: u32 = 0x0002_0014;
    pub const ERROR_RISK_ACCOUNT_LOCKED: u32 = 0xA413;

    // TrinityCore `src/server/proto/BattlenetRpcErrorCodes.h`, friends / presence
    // / user manager ranges.
    pub const ERROR_NOT_EXISTS: u32 = 0x0000_0004;
    pub const ERROR_INVALID_ARGS: u32 = 0x0000_0007;
    pub const ERROR_TARGET_OFFLINE: u32 = 0x0000_002D;
    pub const ERROR_PRESENCE_INVALID_FIELD_ID: u32 = 0x0000_0FA0;
    pub const ERROR_PRESENCE_ALREADY_SUBSCRIBED: u32 = 0x0000_0FA2;
    pub const ERROR_FRIENDS_TOO_MANY_SENT_INVITATIONS: u32 = 0x0000_1389;
    pub const ERROR_FRIENDS_TOO_MANY_RECEIVED_INVITATIONS: u32 = 0x0000_138A;
    pub const ERROR_FRIENDS_FRIENDSHIP_ALREADY_EXISTS: u32 = 0x0000_138B;
    pub const ERROR_FRIENDS_FRIENDSHIP_DOES_NOT_EXIST: u32 = 0x0000_138C;
    pub const ERROR_FRIENDS_INVITATION_ALREADY_EXISTS: u32 = 0x0000_138D;
    pub const ERROR_FRIENDS_INVALID_INVITATION: u32 = 0x0000_138E;
    pub const ERROR_FRIENDS_ALREADY_SUBSCRIBED: u32 = 0x0000_138F;
    pub const ERROR_FRIENDS_NOT_SUBSCRIBED: u32 = 0x0000_1392;
    pub const ERROR_FRIENDS_NOTE_MAX_SIZE_EXCEEDED: u32 = 0x0000_1395;
    pub const ERROR_FRIENDS_UPDATE_FRIEND_STATE_FAILED: u32 = 0x0000_1396;
    pub const ERROR_FRIENDS_INVITEE_AT_MAX_FRIENDS: u32 = 0x0000_1397;
    pub const ERROR_FRIENDS_INVITER_AT_MAX_FRIENDS: u32 = 0x0000_1398;
    pub const ERROR_USER_MANAGER_CANNOT_BLOCK_SELF: u32 = 0x0000_1F42;
}

/// The special `service_id` value used for response messages.
pub const RESPONSE_SERVICE_ID: u32 = 0xFE;

// ---------------------------------------------------------------------------
// Re-exports for convenience
// ---------------------------------------------------------------------------

pub use bgs::protocol::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        use prost::Message;

        let header = Header {
            service_id: 0,
            method_id: Some(1),
            token: 42,
            service_hash: Some(service_hash::AUTHENTICATION_SERVICE),
            size: Some(0),
            ..Default::default()
        };

        let mut buf = Vec::new();
        header.encode(&mut buf).unwrap();

        let decoded = Header::decode(buf.as_slice()).unwrap();
        assert_eq!(decoded.service_id, 0);
        assert_eq!(decoded.method_id, Some(1));
        assert_eq!(decoded.token, 42);
        assert_eq!(
            decoded.service_hash,
            Some(service_hash::AUTHENTICATION_SERVICE)
        );
    }

    #[test]
    fn entity_id_roundtrip() {
        use prost::Message;

        let id = EntityId {
            high: 0x0100_0000_0000_0000,
            low: 1,
        };

        let mut buf = Vec::new();
        id.encode(&mut buf).unwrap();

        let decoded = EntityId::decode(buf.as_slice()).unwrap();
        assert_eq!(decoded.high, 0x0100_0000_0000_0000);
        assert_eq!(decoded.low, 1);
    }

    #[test]
    fn logon_request_encode() {
        use prost::Message;

        let req = authentication::v1::LogonRequest {
            program: Some("WoW".to_string()),
            platform: Some("Wn64".to_string()),
            locale: Some("enUS".to_string()),
            ..Default::default()
        };

        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();
        assert!(!buf.is_empty());

        let decoded = authentication::v1::LogonRequest::decode(buf.as_slice()).unwrap();
        assert_eq!(decoded.program.as_deref(), Some("WoW"));
        assert_eq!(decoded.platform.as_deref(), Some("Wn64"));
    }

    #[test]
    fn connect_request_encode() {
        use prost::Message;

        let req = connection::v1::ConnectRequest {
            client_id: Some(ProcessId {
                label: 1,
                epoch: 100,
            }),
            use_bindless_rpc: Some(true),
            ..Default::default()
        };

        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();

        let decoded = connection::v1::ConnectRequest::decode(buf.as_slice()).unwrap();
        assert_eq!(decoded.client_id.unwrap().label, 1);
        assert_eq!(decoded.use_bindless_rpc, Some(true));
    }

    #[test]
    fn client_request_with_attributes() {
        use prost::Message;

        let req = game_utilities::v1::ClientRequest {
            attribute: vec![Attribute {
                name: "Command_RealmListRequest_v1".to_string(),
                value: Variant {
                    string_value: Some("test".to_string()),
                    ..Default::default()
                },
            }],
            ..Default::default()
        };

        let mut buf = Vec::new();
        req.encode(&mut buf).unwrap();

        let decoded = game_utilities::v1::ClientRequest::decode(buf.as_slice()).unwrap();
        assert_eq!(decoded.attribute.len(), 1);
        assert_eq!(decoded.attribute[0].name, "Command_RealmListRequest_v1");
    }

    /// Wire layout must match TrinityCore friends_types.pb.h / friends_service.pb.h:
    /// InvitationParams.friend_params = 103 (extension inlined), FriendInvitationParams
    /// target_battle_tag = 2, SendInvitationRequest target_id = 2 / params = 3.
    #[test]
    fn send_invitation_request_wire_bytes_match_cpp_field_numbers() {
        use prost::Message;

        let request = friends::v1::SendInvitationRequest {
            agent_identity: None,
            target_id: EntityId { high: 0, low: 0 },
            params: InvitationParams {
                friend_params: Some(friends::v1::FriendInvitationParams {
                    target_battle_tag: Some("Ab#1".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        };
        assert_eq!(
            request.encode_to_vec(),
            vec![
                0x12, 0x12, // target_id (2), len 18
                0x09, 0, 0, 0, 0, 0, 0, 0, 0, // high (fixed64)
                0x11, 0, 0, 0, 0, 0, 0, 0, 0, // low (fixed64)
                0x1A, 0x09, // params (3), len 9
                0xBA, 0x06, 0x06, // friend_params (103 << 3 | 2 = 0x33A), len 6
                0x12, 0x04, b'A', b'b', b'#', b'1', // target_battle_tag (2)
            ]
        );
        let decoded =
            friends::v1::SendInvitationRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        assert_eq!(
            decoded
                .params
                .friend_params
                .unwrap()
                .target_battle_tag
                .as_deref(),
            Some("Ab#1")
        );
    }

    #[test]
    fn friends_types_round_trip_with_cpp_field_numbers() {
        use prost::Message;

        let response = friends::v1::SubscribeResponse {
            max_friends: Some(200),
            role: vec![Role {
                id: 1,
                name: "battle_tag_friend".to_owned(),
                ..Default::default()
            }],
            friends: vec![friends::v1::Friend {
                account_id: EntityId {
                    high: 0x0100_0000_0000_0000,
                    low: 7,
                },
                role: vec![1],
                creation_time: Some(1_700_000_000),
                ..Default::default()
            }],
            received_invitations: vec![friends::v1::ReceivedInvitation {
                id: 5,
                inviter_identity: Identity {
                    account_id: Some(EntityId {
                        high: 0x0100_0000_0000_0000,
                        low: 9,
                    }),
                    game_account_id: None,
                },
                invitee_identity: Identity::default(),
                inviter_name: Some("Some#1234".to_owned()),
                program: Some(0x0057_6F57),
                ..Default::default()
            }],
            sent_invitations: vec![friends::v1::SentInvitation {
                id: Some(6),
                target_name: Some("Other#1".to_owned()),
                role: Some(1),
                ..Default::default()
            }],
            ..Default::default()
        };
        let bytes = response.encode_to_vec();
        // SubscribeResponse.max_friends is field 1 (varint) and friends is field 5.
        assert_eq!(&bytes[..3], &[0x08, 0xC8, 0x01]);
        let decoded = friends::v1::SubscribeResponse::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded, response);
        assert_eq!(
            InvitationRemovedReason::Declined as u32,
            1,
            "invitation_types.pb.h INVITATION_REMOVED_REASON_DECLINED"
        );
        assert_eq!(InvitationRemovedReason::Revoked as u32, 2);
        assert_eq!(InvitationRemovedReason::Ignored as u32, 3);
    }

    /// presence_types.pb.h: FieldKey program=1/group=2/field=3/unique_id=4, Field
    /// key=1/value=2, FieldOperation field=1/operation=2, PresenceState
    /// entity_id=1/field_operation=2; presence_listener.pb.h
    /// StateChangedNotification subscriber_id=1 (AccountId.id fixed32) / state=2 /
    /// subscriber_program=3.
    #[test]
    fn presence_state_changed_wire_bytes_match_cpp_field_numbers() {
        use prost::Message;

        let notification = presence::v1::StateChangedNotification {
            subscriber_id: Some(account::v1::AccountId { id: 3 }),
            state: vec![presence::v1::PresenceState {
                entity_id: Some(EntityId {
                    high: 0x0100_0000_0000_0000,
                    low: 4,
                }),
                field_operation: vec![presence::v1::FieldOperation {
                    field: presence::v1::Field {
                        key: presence::v1::FieldKey {
                            program: 0x424E,
                            group: 2,
                            field: 1,
                            unique_id: None,
                        },
                        value: Variant {
                            bool_value: Some(true),
                            ..Default::default()
                        },
                    },
                    operation: Some(presence::v1::field_operation::OperationType::Clear as i32),
                }],
            }],
            subscriber_program: Some(0x0057_6F57),
        };
        let bytes = notification.encode_to_vec();
        assert_eq!(&bytes[..7], &[0x0A, 0x05, 0x0D, 0x03, 0x00, 0x00, 0x00]);
        assert_eq!(bytes[7], 0x12, "state is field 2");
        let decoded = presence::v1::StateChangedNotification::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded, notification);
        let op = &decoded.state[0].field_operation[0];
        assert_eq!(op.field.key.program, 0x424E);
        assert_eq!(
            op.operation(),
            presence::v1::field_operation::OperationType::Clear
        );
        // QueryRequest entity_id=1 / key=2; SubscribeRequest entity_id=2 / object_id=3.
        let query = presence::v1::QueryRequest::decode(
            &[
                0x0A, 0x12, 0x09, 1, 0, 0, 0, 0, 0, 0, 0, 0x11, 2, 0, 0, 0, 0, 0, 0, 0, 0x12, 0x06,
                0x08, 0x01, 0x10, 0x02, 0x18, 0x03,
            ][..],
        )
        .unwrap();
        assert_eq!(query.entity_id.low, 2);
        assert_eq!(query.key[0].field, 3);
    }

    #[test]
    fn user_manager_subscribe_response_round_trip() {
        use prost::Message;

        let empty = user_manager::v1::SubscribeResponse::default();
        assert!(empty.encode_to_vec().is_empty());
        let request = user_manager::v1::SubscribeRequest::decode(&[0x10, 0x01][..]).unwrap();
        assert_eq!(request.object_id, 1);
        let blocked = user_manager::v1::SubscribeResponse {
            blocked_players: vec![user_manager::v1::BlockedPlayer {
                account_id: EntityId { high: 1, low: 2 },
                battle_tag: Some("X#1".to_owned()),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            user_manager::v1::SubscribeResponse::decode(blocked.encode_to_vec().as_slice())
                .unwrap(),
            blocked
        );
    }

    #[test]
    fn service_hash_constants_are_correct() {
        // Verify against known values from C# source
        assert_eq!(service_hash::AUTHENTICATION_SERVICE, 0x0DEC_FC01);
        assert_eq!(service_hash::CONNECTION_SERVICE, 0x6544_6991);
        assert_eq!(service_hash::ACCOUNT_SERVICE, 0x62DA_0891);
        assert_eq!(service_hash::GAME_UTILITIES_SERVICE, 0x3FC1_274D);
        assert_eq!(service_hash::AUTHENTICATION_LISTENER, 0x7124_0E35);
        assert_eq!(service_hash::CHALLENGE_LISTENER, 0xBBDA_171F);
        // TrinityCore 3.4.3 proto/Client/*.pb.h `OriginalHash` values.
        assert_eq!(service_hash::FRIENDS_SERVICE, 0xA3DD_B1BD);
        assert_eq!(service_hash::FRIENDS_LISTENER, 0x6F25_9A13);
        assert_eq!(service_hash::PRESENCE_SERVICE, 0xFA07_96FF);
        assert_eq!(service_hash::PRESENCE_LISTENER, 0x890A_B85F);
        assert_eq!(service_hash::USER_MANAGER_SERVICE, 0x3E19_268A);
        assert_eq!(service_hash::USER_MANAGER_LISTENER, 0xBC87_2C22);
    }
}
