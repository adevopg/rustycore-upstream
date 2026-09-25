use std::sync::{Arc, Mutex};

use prost::Message;
use wow_constants::{ClientOpcodes, ServerOpcodes};
use wow_handler::{PacketProcessing, SessionStatus};
use wow_packet::WorldPacket;
use wow_packet::packets::battlenet::{BattlenetRequest, MethodCall};
use wow_proto::bgs::protocol::game_utilities::v1::{
    ClientRequest, ClientResponse, GetAllValuesForAttributeRequest,
    GetAllValuesForAttributeResponse,
};
use wow_proto::bgs::protocol::{Attribute, Variant};
use wow_proto::realm_list_json::decompress_json_like_cpp;
use wow_proto::{service_hash, status};

use super::*;
use crate::session::registry::PacketHandlerEntry;

const REALM_LIST_BLOB: &[u8] = b"compressed-realm-list";

#[derive(Default)]
struct FakeRealmListLikeCpp {
    join_requests: Mutex<Vec<RealmJoinRequestLikeCpp>>,
    realm_list_calls: Mutex<Vec<(u32, String)>>,
    join_status: Option<u32>,
}

impl WorldserverRealmListPortLikeCpp for FakeRealmListLikeCpp {
    fn sub_regions_like_cpp(&self) -> Vec<String> {
        vec!["1-1-0".to_owned(), "1-2-0".to_owned()]
    }

    fn current_realm_build_like_cpp(&self) -> u32 {
        54261
    }

    fn get_realm_list_like_cpp(&self, build: u32, sub_region: &str) -> Vec<u8> {
        self.realm_list_calls
            .lock()
            .unwrap()
            .push((build, sub_region.to_owned()));
        REALM_LIST_BLOB.to_vec()
    }

    fn join_realm_like_cpp(&self, request: RealmJoinRequestLikeCpp) -> RealmJoinFutureLikeCpp<'_> {
        self.join_requests.lock().unwrap().push(request);
        let result = match self.join_status {
            Some(status) => Err(status),
            None => Ok(RealmJoinGrantLikeCpp {
                server_addresses: b"addresses".to_vec(),
                join_secret: [0x42; 32],
            }),
        };
        Box::pin(async move { result })
    }
}

fn make_session() -> (WorldSession, flume::Receiver<Vec<u8>>) {
    let (_pkt_tx, pkt_rx) = flume::bounded::<WorldPacket>(8);
    let (send_tx, send_rx) = flume::bounded::<Vec<u8>>(8);
    (
        WorldSession::new(
            1,
            "1#1".to_string(),
            0,
            0,
            0,
            0,
            vec![],
            "esES".to_string(),
            pkt_rx,
            send_tx,
        ),
        send_rx,
    )
}

fn session_with_realm_list(
    realm_list: Arc<FakeRealmListLikeCpp>,
) -> (WorldSession, flume::Receiver<Vec<u8>>) {
    let (mut session, send_rx) = make_session();
    session.set_worldserver_realm_list_like_cpp(realm_list);
    (session, send_rx)
}

struct SentResponse {
    status: u32,
    method_type: u64,
    token: u32,
    data: Vec<u8>,
}

fn read_response(send_rx: &flume::Receiver<Vec<u8>>) -> SentResponse {
    let bytes = send_rx.try_recv().expect("BattlenetResponse sent");
    let mut pkt = WorldPacket::from_bytes(&bytes);
    assert_eq!(
        pkt.read_uint16().unwrap(),
        ServerOpcodes::BattlenetResponse as u16
    );
    let status = pkt.read_uint32().unwrap();
    let method_type = pkt.read_uint64().unwrap();
    assert_eq!(pkt.read_int64().unwrap(), 1, "C++ ObjectId is always 1");
    let token = pkt.read_uint32().unwrap();
    let size = pkt.read_uint32().unwrap() as usize;
    let data = pkt.read_bytes(size).unwrap();
    SentResponse {
        status,
        method_type,
        token,
        data,
    }
}

fn request(service: u32, method_id: u32, token: u32, data: Vec<u8>) -> BattlenetRequest {
    BattlenetRequest {
        method: MethodCall::from_parts(service, method_id, token),
        data,
    }
}

fn string_attribute(name: &str, value: &str) -> Attribute {
    Attribute {
        name: name.to_owned(),
        value: Variant {
            string_value: Some(value.to_owned()),
            ..Default::default()
        },
    }
}

fn uint_attribute(name: &str, value: u64) -> Attribute {
    Attribute {
        name: name.to_owned(),
        value: Variant {
            uint_value: Some(value),
            ..Default::default()
        },
    }
}

fn client_request(attributes: Vec<Attribute>) -> Vec<u8> {
    ClientRequest {
        attribute: attributes,
        ..Default::default()
    }
    .encode_to_vec()
}

fn blob<'a>(response: &'a ClientResponse, name: &str) -> &'a [u8] {
    response
        .attribute
        .iter()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| attribute.value.blob_value.as_deref())
        .unwrap_or_else(|| panic!("missing blob attribute {name}"))
}

#[test]
fn worldserver_service_hashes_match_cpp_dispatcher_registration() {
    assert_eq!(
        WORLDSERVER_SERVICE_HASHES_LIKE_CPP,
        [
            0x62DA_0891, // account.v1.AccountService
            0x0DEC_FC01, // authentication.v1.AuthenticationService
            0x94B9_4786, // club.v1.membership.ClubMembershipService
            0xE273_DE0E, // club.v1.ClubService
            0x6544_6991, // connection.v1.ConnectionService
            0xA3DD_B1BD, // friends.v1.FriendsService
            0x3FC1_274D, // game_utilities.v1.GameUtilitiesService
            0xFA07_96FF, // presence.v1.PresenceService
            0x7CAF_61C9, // report.v1.ReportService
            0x3A42_18FB, // report.v2.ReportService
            0xECBE_75BA, // resources.v1.ResourcesService
            0x3E19_268A, // user_manager.v1.UserManagerService
        ]
    );
    assert_eq!(status::ERROR_RPC_NOT_IMPLEMENTED, 3015);
}

#[test]
fn battlenet_request_handler_metadata_is_registered() {
    let entry = inventory::iter::<PacketHandlerEntry>
        .into_iter()
        .find(|entry| entry.opcode == ClientOpcodes::BattlenetRequest)
        .expect("BattlenetRequest handler entry");
    assert_eq!(entry.status, SessionStatus::Authed);
    assert_eq!(entry.processing, PacketProcessing::ThreadUnsafe);
    assert_eq!(entry.handler_name, "handle_battlenet_request");
}

#[tokio::test]
async fn get_all_values_for_attribute_returns_sub_regions_like_cpp() {
    let (mut session, send_rx) = session_with_realm_list(Arc::default());
    let data = GetAllValuesForAttributeRequest {
        attribute_key: Some("Command_RealmListRequest_v1_b9".to_owned()),
        ..Default::default()
    }
    .encode_to_vec();

    session
        .handle_battlenet_request(request(service_hash::GAME_UTILITIES_SERVICE, 10, 77, data))
        .await;

    let sent = read_response(&send_rx);
    assert_eq!(sent.status, status::OK);
    assert_eq!(
        sent.method_type,
        (u64::from(service_hash::GAME_UTILITIES_SERVICE) << 32) | 10
    );
    assert_eq!(sent.token, 77);
    let response = GetAllValuesForAttributeResponse::decode(sent.data.as_slice()).unwrap();
    let values: Vec<_> = response
        .attribute_value
        .iter()
        .map(|value| value.string_value.as_deref().unwrap())
        .collect();
    assert_eq!(values, ["1-1-0", "1-2-0"]);
}

#[tokio::test]
async fn get_all_values_for_other_attribute_is_not_implemented_like_cpp() {
    let (mut session, send_rx) = session_with_realm_list(Arc::default());
    let data = GetAllValuesForAttributeRequest {
        attribute_key: Some("Command_Other_v1".to_owned()),
        ..Default::default()
    }
    .encode_to_vec();

    session
        .handle_battlenet_request(request(service_hash::GAME_UTILITIES_SERVICE, 10, 1, data))
        .await;

    let sent = read_response(&send_rx);
    assert_eq!(sent.status, status::ERROR_RPC_NOT_IMPLEMENTED);
    assert!(sent.data.is_empty());
}

#[tokio::test]
async fn realm_list_request_returns_realm_list_and_character_counts_like_cpp() {
    let realm_list = Arc::new(FakeRealmListLikeCpp::default());
    let (mut session, send_rx) = session_with_realm_list(Arc::clone(&realm_list));
    session.set_realm_character_counts_like_cpp([(0x0101_0001, 3), (0x0101_0002, 1)]);
    let data = client_request(vec![
        string_attribute("Command_RealmListRequest_v1_b9", "1-1-0"),
        string_attribute("Param_Unused", "x"),
    ]);

    session
        .handle_battlenet_request(request(service_hash::GAME_UTILITIES_SERVICE, 1, 9, data))
        .await;

    let sent = read_response(&send_rx);
    assert_eq!(sent.status, status::OK);
    assert_eq!(sent.token, 9);
    assert_eq!(
        realm_list.realm_list_calls.lock().unwrap().as_slice(),
        [(54261, "1-1-0".to_owned())]
    );
    let response = ClientResponse::decode(sent.data.as_slice()).unwrap();
    let names: Vec<_> = response.attribute.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Param_RealmList", "Param_CharacterCountList"]);
    assert_eq!(blob(&response, "Param_RealmList"), REALM_LIST_BLOB);
    assert_eq!(
        decompress_json_like_cpp(blob(&response, "Param_CharacterCountList")).unwrap(),
        concat!(
            "JSONRealmCharacterCountList:{\"counts\":[",
            "{\"wowRealmAddress\":16842753,\"count\":3},",
            "{\"wowRealmAddress\":16842754,\"count\":1}]}"
        )
    );
}

#[tokio::test]
async fn realm_join_request_calls_join_realm_with_session_inputs_like_cpp() {
    let realm_list = Arc::new(FakeRealmListLikeCpp::default());
    let (mut session, send_rx) = session_with_realm_list(Arc::clone(&realm_list));
    session.set_realm_list_secret_like_cpp([0x5A; 32]);
    session.set_remote_address_like_cpp(Some("192.168.1.20".to_owned()));
    session.set_client_os_and_timezone_like_cpp("Wn64".to_owned(), -120);
    let data = client_request(vec![
        uint_attribute("Param_RealmAddress", 0x0101_0002),
        string_attribute("Command_RealmJoinRequest_v1_b9", "1-1-0"),
    ]);

    session
        .handle_battlenet_request(request(service_hash::GAME_UTILITIES_SERVICE, 1, 4, data))
        .await;

    let sent = read_response(&send_rx);
    assert_eq!(sent.status, status::OK);
    assert_eq!(
        realm_list.join_requests.lock().unwrap().as_slice(),
        [RealmJoinRequestLikeCpp {
            realm_address: 0x0101_0002,
            build: 54261,
            client_address: "192.168.1.20".to_owned(),
            client_secret: [0x5A; 32],
            locale: 6,
            os: "Wn64".to_owned(),
            timezone_offset_minutes: -120,
            account_name: "1#1".to_owned(),
        }]
    );
    let response = ClientResponse::decode(sent.data.as_slice()).unwrap();
    let names: Vec<_> = response.attribute.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Param_RealmJoinTicket",
            "Param_ServerAddresses",
            "Param_JoinSecret"
        ]
    );
    assert_eq!(blob(&response, "Param_RealmJoinTicket"), b"1#1");
    assert_eq!(blob(&response, "Param_ServerAddresses"), b"addresses");
    assert_eq!(blob(&response, "Param_JoinSecret"), [0x42; 32]);
}

#[tokio::test]
async fn realm_join_failures_return_cpp_statuses() {
    let realm_list = Arc::new(FakeRealmListLikeCpp {
        join_status: Some(status::ERROR_USER_SERVER_NOT_PERMITTED_ON_REALM),
        ..Default::default()
    });
    let (mut session, send_rx) = session_with_realm_list(Arc::clone(&realm_list));

    session
        .handle_battlenet_request(request(
            service_hash::GAME_UTILITIES_SERVICE,
            1,
            1,
            client_request(vec![
                uint_attribute("Param_RealmAddress", 7),
                string_attribute("Command_RealmJoinRequest_v1_b9", ""),
            ]),
        ))
        .await;
    assert_eq!(
        read_response(&send_rx).status,
        status::ERROR_USER_SERVER_NOT_PERMITTED_ON_REALM
    );

    session
        .handle_battlenet_request(request(
            service_hash::GAME_UTILITIES_SERVICE,
            1,
            2,
            client_request(vec![string_attribute("Command_RealmJoinRequest_v1_b9", "")]),
        ))
        .await;
    assert_eq!(
        read_response(&send_rx).status,
        status::ERROR_WOW_SERVICES_INVALID_JOIN_TICKET
    );
    assert_eq!(realm_list.join_requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn process_client_request_command_errors_match_cpp() {
    let (mut session, send_rx) = session_with_realm_list(Arc::default());

    // No Command_* attribute.
    session
        .handle_battlenet_request(request(
            service_hash::GAME_UTILITIES_SERVICE,
            1,
            1,
            client_request(vec![string_attribute("Param_Foo", "x")]),
        ))
        .await;
    assert_eq!(
        read_response(&send_rx).status,
        status::ERROR_RPC_MALFORMED_REQUEST
    );

    // The worldserver does not register the bnetserver-only commands.
    session
        .handle_battlenet_request(request(
            service_hash::GAME_UTILITIES_SERVICE,
            1,
            2,
            client_request(vec![string_attribute(
                "Command_LastCharPlayedRequest_v1_b9",
                "1-1-0",
            )]),
        ))
        .await;
    assert_eq!(
        read_response(&send_rx).status,
        status::ERROR_RPC_NOT_IMPLEMENTED
    );

    // Unparseable protobuf payload.
    session
        .handle_battlenet_request(request(
            service_hash::GAME_UTILITIES_SERVICE,
            1,
            3,
            vec![0xFF, 0xFF, 0xFF],
        ))
        .await;
    assert_eq!(
        read_response(&send_rx).status,
        status::ERROR_RPC_MALFORMED_REQUEST
    );
}

#[tokio::test]
async fn game_utilities_method_table_matches_generated_cpp() {
    let (mut session, send_rx) = session_with_realm_list(Arc::default());

    // Method id 3 is not in the generated switch.
    session
        .handle_battlenet_request(request(service_hash::GAME_UTILITIES_SERVICE, 3, 1, vec![]))
        .await;
    assert_eq!(
        read_response(&send_rx).status,
        status::ERROR_RPC_INVALID_METHOD
    );

    // Registered but unimplemented methods answer NOT_IMPLEMENTED.
    for method_id in [2, 6, 7, 8, 11, 12] {
        session
            .handle_battlenet_request(request(
                service_hash::GAME_UTILITIES_SERVICE,
                method_id,
                method_id,
                vec![],
            ))
            .await;
        let sent = read_response(&send_rx);
        assert_eq!(
            sent.status,
            status::ERROR_RPC_NOT_IMPLEMENTED,
            "method {method_id}"
        );
        assert_eq!(sent.token, method_id);
    }

    // C++ masks the method id with 0x3FFFFFFF but echoes the original one.
    let data = GetAllValuesForAttributeRequest {
        attribute_key: Some("Command_RealmListRequest_v1".to_owned()),
        ..Default::default()
    }
    .encode_to_vec();
    session
        .handle_battlenet_request(request(
            service_hash::GAME_UTILITIES_SERVICE,
            0x4000_000A,
            5,
            data,
        ))
        .await;
    let sent = read_response(&send_rx);
    assert_eq!(sent.status, status::OK);
    assert_eq!(sent.method_type as u32, 0x4000_000A);
}

#[tokio::test]
async fn other_services_follow_cpp_dispatcher_registration() {
    let (mut session, send_rx) = make_session();

    session
        .handle_battlenet_request(request(service_hash::PRESENCE_SERVICE, 1, 8, vec![]))
        .await;
    let sent = read_response(&send_rx);
    assert_eq!(sent.status, status::ERROR_RPC_NOT_IMPLEMENTED);
    assert_eq!(sent.token, 8);

    // Unregistered service: C++ only logs, no response is sent.
    session
        .handle_battlenet_request(request(0xDEAD_BEEF, 1, 9, vec![]))
        .await;
    assert!(send_rx.try_recv().is_err());
}
