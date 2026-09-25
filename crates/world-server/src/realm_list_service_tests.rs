use super::*;
use wow_database::SqlParam;
use wow_proto::realm_list_json::decompress_json_like_cpp;

fn row(realm_id: u32, region: u8, battlegroup: u8) -> RealmListRawRowLikeCpp {
    RealmListRawRowLikeCpp {
        realm_id,
        name: format!("Realm {realm_id}"),
        address: "127.0.0.1".to_owned(),
        local_address: "127.0.0.1".to_owned(),
        port: 8085,
        icon: 1,
        flag: 0,
        timezone: 8,
        allowed_security_level: 0,
        population: 0.0,
        build: 54261,
        region,
        battlegroup,
    }
}

fn snapshot(rows: Vec<RealmListRawRowLikeCpp>) -> RealmListSnapshotLikeCpp {
    let mut snapshot = RealmListSnapshotLikeCpp::default();
    for row in rows {
        let entry = realm_list_entry_from_row_like_cpp(row);
        snapshot
            .sub_regions
            .insert(entry.id.sub_region_address_like_cpp());
        snapshot.realms.insert(entry.id, entry);
    }
    snapshot
}

const BUILDS: [RealmBuildInfoLikeCpp; 1] = [RealmBuildInfoLikeCpp {
    major_version: 3,
    minor_version: 4,
    bugfix_version: 3,
    build: 54261,
}];

#[test]
fn realm_list_updates_filter_sub_region_and_fill_cpp_entry() {
    let mut offline = row(2, 1, 1);
    offline.flag = REALM_FLAG_OFFLINE_LIKE_CPP;
    offline.population = 3.0;
    let mut old_build = row(3, 1, 1);
    old_build.build = 12340;
    old_build.population = 2.7;
    let snapshot = snapshot(vec![row(1, 1, 1), offline, old_build, row(4, 1, 2)]);

    let updates = realm_list_updates_like_cpp(&snapshot, &BUILDS, 54261, "1-1-0");
    let realms: Vec<_> = updates.updates.iter().map(|state| &state.update).collect();
    assert_eq!(
        realms.iter().map(|r| r.cfg_realms_id).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(updates.updates.iter().all(|state| !state.deleting));

    let online = realms[0];
    assert_eq!(online.wow_realm_address, 0x0101_0001);
    assert_eq!(online.cfg_timezones_id, 1);
    assert_eq!(online.population_state, 1, "max(population, 1)");
    assert_eq!(online.cfg_categories_id, 8);
    assert_eq!(online.flags, 0);
    assert_eq!(online.cfg_configs_id, 2, "ConfigIdByType[PVP]");
    assert_eq!(online.cfg_languages_id, 1);
    assert_eq!(
        (
            online.version.version_major,
            online.version.version_minor,
            online.version.version_revision,
            online.version.version_build
        ),
        (3, 4, 3, 54261)
    );

    assert_eq!(realms[1].population_state, 0, "offline realm");
    assert_eq!(u32::from(REALM_FLAG_OFFLINE_LIKE_CPP), realms[1].flags);

    let mismatch = realms[2];
    assert_eq!(
        mismatch.flags,
        u32::from(REALM_FLAG_VERSION_MISMATCH_LIKE_CPP)
    );
    assert_eq!(mismatch.population_state, 2);
    assert_eq!(
        (
            mismatch.version.version_major,
            mismatch.version.version_minor,
            mismatch.version.version_revision,
            mismatch.version.version_build
        ),
        (6, 2, 4, 12340),
        "C++ fallback version when build_info has no row"
    );
}

#[test]
fn realm_list_blob_is_cpp_prefixed_compressed_json() {
    let service = snapshot(vec![row(1, 1, 1)]);
    let json = realm_list_json::serialize_prefixed_like_cpp(
        realm_list_json::REALM_LIST_UPDATES_PREFIX,
        &realm_list_updates_like_cpp(&service, &BUILDS, 54261, "1-1-0"),
    );
    assert!(json.starts_with(
        "JSONRealmListUpdates:{\"updates\":[{\"update\":{\"wowRealmAddress\":16842753,"
    ));
    assert!(json.ends_with("\"deleting\":false}]}"));
    let blob = realm_list_json::compress_json_like_cpp(&json);
    assert_eq!(
        decompress_json_like_cpp(&blob).as_deref(),
        Some(json.as_str())
    );
}

#[test]
fn join_realm_target_gates_match_cpp() {
    let mut offline = row(2, 1, 1);
    offline.flag = REALM_FLAG_OFFLINE_LIKE_CPP;
    let snapshot = snapshot(vec![row(1, 1, 1), offline]);

    assert_eq!(
        join_realm_target_like_cpp(&snapshot, 0x0101_0009, 54261),
        Err(status::ERROR_UTIL_SERVER_UNKNOWN_REALM)
    );
    assert_eq!(
        join_realm_target_like_cpp(&snapshot, 0x0101_0002, 54261),
        Err(status::ERROR_USER_SERVER_NOT_PERMITTED_ON_REALM)
    );
    assert_eq!(
        join_realm_target_like_cpp(&snapshot, 0x0101_0001, 12340),
        Err(status::ERROR_USER_SERVER_NOT_PERMITTED_ON_REALM)
    );
    // RealmHandle lookup ignores region/site like C++ RealmHandle::operator<.
    let target = join_realm_target_like_cpp(&snapshot, 0x0505_0001, 54261).unwrap();
    assert_eq!(target.realm_id, 1);
    assert_eq!(target.port, 8085);
}

#[test]
fn server_addresses_blob_matches_cpp_json() {
    let blob = realm_server_addresses_blob_like_cpp([192, 168, 1, 5], 8085);
    assert_eq!(
        decompress_json_like_cpp(&blob).unwrap(),
        "JSONRealmListServerIPAddresses:{\"families\":[{\"family\":1,\"addresses\":[{\"ip\":\"192.168.1.5\",\"port\":8085}]}]}"
    );
}

#[test]
fn join_realm_login_info_statement_binds_cpp_params_in_order() {
    let request = RealmJoinRequestLikeCpp {
        realm_address: 0x0101_0001,
        build: 54261,
        client_address: "10.0.0.2".to_owned(),
        client_secret: [0x11; 32],
        locale: 6,
        os: "Wn64".to_owned(),
        timezone_offset_minutes: -60,
        account_name: "1#1".to_owned(),
    };
    let stmt = join_realm_login_info_statement_like_cpp(&request, "10.0.0.2", &[0x22; 32]);
    assert_eq!(
        stmt.sql(),
        LoginStatements::UPD_BNET_GAME_ACCOUNT_LOGIN_INFO.sql()
    );
    let mut key = vec![0x11; 32];
    key.extend_from_slice(&[0x22; 32]);
    assert_eq!(
        stmt.params(),
        [
            SqlParam::Bytes(key),
            SqlParam::String("10.0.0.2".to_owned()),
            SqlParam::U8(6),
            SqlParam::String("Wn64".to_owned()),
            SqlParam::I16(-60),
            SqlParam::String("1#1".to_owned()),
        ]
    );
}
