//! Codec tests for `packets::battlepay` (moved out of the module file to keep it
//! under the physical source ceiling; contents unchanged).

use super::*;

fn payload(bytes: &[u8]) -> Vec<u8> {
    bytes[2..].to_vec()
}

fn le32(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

fn le64(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

/// Reads a packet the way the client does: opcode is not part of the payload.
fn client_read<T: ClientPacket>(body: &[u8]) -> Result<T, PacketError> {
    let mut pkt = WorldPacket::from_bytes(body);
    T::read(&mut pkt)
}

#[test]
fn display_info_bit_header_matches_client_decoder() {
    // Client reader 0x140709370: 12 header bytes, MSB first:
    // has_cdi(1) has_fdi(1) n1(10) n2(10) n3(13) n4(13) n5(13) hf(1) hu1(1) hu2(1) hu3(1) n6(13) n7(12)
    let info = BattlePayDisplayInfo {
        creature_display_id: Some(0x11111111),
        file_data_id: None,
        name1: "A".repeat(3),
        name2: "B".to_string(),
        name3: "C".repeat(2),
        name4: String::new(),
        name5: "E".to_string(),
        name6: String::new(),
        name7: "G".to_string(),
        flags: Some(0x22222222),
        unk1: None,
        unk2: None,
        unk3: Some(0x33333333),
        unk4: 4,
        unk5: 5,
        unk6: 6,
        visuals: vec![BattlePayVisual {
            display_id: 7,
            visual_id: 8,
            unk: 9,
            name: "vis".to_string(),
        }],
    };
    let mut pkt = WorldPacket::new_empty();
    info.write(&mut pkt);
    let bytes = pkt.into_data();

    // Re-derive the header with the client's exact bit arithmetic.
    let b = &bytes[..12];
    assert_eq!(b[0] >> 7, 1, "HasCreatureDisplayID = bit7 of byte 0");
    assert_eq!((b[0] >> 6) & 1, 0, "HasFileDataID = bit6 of byte 0");
    let n1 = ((u32::from(b[0]) & 0x3f) << 4) | (u32::from(b[1]) >> 4);
    let n2 = ((u32::from(b[1]) & 0xf) << 6) | (u32::from(b[2]) >> 2);
    let n3 = ((u32::from(b[2]) & 3) << 11) | (u32::from(b[3]) << 3) | (u32::from(b[4]) >> 5);
    let n4 = ((u32::from(b[4]) & 0x1f) << 8) | u32::from(b[5]);
    let n5 = (u32::from(b[6]) << 5) | (u32::from(b[7]) >> 3);
    let has_flags = (b[7] >> 2) & 1;
    let has_unk1 = (b[7] >> 1) & 1;
    let has_unk2 = b[7] & 1;
    let has_unk3 = b[8] >> 7;
    let n6 = ((u32::from(b[8]) & 0x7f) << 6) | (u32::from(b[9]) >> 2);
    let n7 = ((u32::from(b[9]) & 3) << 10) | (u32::from(b[10]) << 2) | (u32::from(b[11]) >> 6);
    assert_eq!((n1, n2, n3, n4, n5), (3, 1, 2, 0, 1));
    assert_eq!((has_flags, has_unk1, has_unk2, has_unk3), (1, 0, 0, 1));
    assert_eq!((n6, n7), (0, 1));

    let mut expected = Vec::new();
    expected.extend_from_slice(b);
    expected.extend_from_slice(&le32(1)); // visual count
    expected.extend_from_slice(&le32(4));
    expected.extend_from_slice(&le32(5));
    expected.extend_from_slice(&le32(6));
    expected.extend_from_slice(&le32(0x11111111)); // creature display id
    expected.extend_from_slice(b"AAA");
    expected.extend_from_slice(b"B");
    expected.extend_from_slice(b"CC");
    expected.extend_from_slice(b"E");
    expected.extend_from_slice(&le32(0x22222222)); // flags
    expected.extend_from_slice(&le32(0x33333333)); // unk3
    expected.extend_from_slice(b"G");
    // visual: bits(10) len=3 -> 0b00000000 11 -> bytes 0x00, 0xC0
    expected.extend_from_slice(&[0x00, 0xC0]);
    expected.extend_from_slice(&le32(7));
    expected.extend_from_slice(&le32(8));
    expected.extend_from_slice(&le32(9));
    expected.extend_from_slice(b"vis");
    assert_eq!(bytes, expected);
}

#[test]
fn product_group_writes_8_bit_name_and_24_bit_description_field() {
    let group = BattlePayProductGroup {
        group_id: 30,
        icon_file_data_id: 0xABCD,
        display_type: 2,
        ordering: 5,
        flags: 1,
        unk: 0,
        name: "Tok".to_string(),
        is_available_description: "no".to_string(),
    };
    let mut pkt = WorldPacket::new_empty();
    group.write(&mut pkt);
    let mut expected = Vec::new();
    expected.extend_from_slice(&le32(30));
    expected.extend_from_slice(&le32(0xABCD));
    expected.push(2);
    expected.extend_from_slice(&le32(5));
    expected.extend_from_slice(&le32(1));
    expected.extend_from_slice(&le32(0));
    expected.push(3); // name length byte
    expected.extend_from_slice(&[0, 0, 3]); // 24-bit field = len("no") + 1, MSB first
    expected.extend_from_slice(b"Tok");
    expected.extend_from_slice(b"no\0");
    assert_eq!(pkt.into_data(), expected);

    let empty = BattlePayProductGroup {
        is_available_description: String::new(),
        name: String::new(),
        ..group
    };
    let mut pkt = WorldPacket::new_empty();
    empty.write(&mut pkt);
    let bytes = pkt.into_data();
    assert_eq!(&bytes[21..25], &[0, 0, 0, 0]);
    assert_eq!(bytes.len(), 25);
}

#[test]
fn product_bit_block_matches_client_reader() {
    // Client reader 0x140708b10 byte layout after the 12 fixed fields:
    // byte0 = UnkString len; byte1 = UnkBit<<7 | HasUnkBits<<6 | count>>1;
    // byte2 = (count&1)<<7 | HasDisplay<<6 | UnkBits<<2
    let product = BattlePayProduct {
        product_id: 1,
        product_type: 2,
        flags: 3,
        unk1: 4,
        display_id: 5,
        item_id: 6,
        unk4: 7,
        unk5: 8,
        unk6: 9,
        unk7: 10,
        unk8: 11,
        unk9: 12,
        unk_string: "xy".to_string(),
        unk_bit: true,
        unk_bits: Some(0b1010),
        items: vec![BattlePayProductItem {
            id: 100,
            unk_byte: 1,
            item_id: 200,
            quantity: 3,
            unk1: 0,
            unk2: 0,
            has_pet: false,
            pet_result: Some(0b0110),
            display_info: None,
        }],
        display_info: None,
    };
    let mut pkt = WorldPacket::new_empty();
    product.write(&mut pkt);
    let bytes = pkt.into_data();
    let mut expected = Vec::new();
    expected.extend_from_slice(&le32(1));
    expected.push(2);
    for v in [3u32, 4, 5, 6, 7, 8, 9, 10, 11, 12] {
        expected.extend_from_slice(&le32(v));
    }
    expected.push(2); // unk_string len (8 bits)
    expected.push(0b1100_0000); // UnkBit=1, HasUnkBits=1, count high 6 bits = 0
    expected.push(0b1000_0000 | 0b1010 << 2); // count low bit=1, HasDisplay=0, UnkBits=1010
    // item
    expected.extend_from_slice(&le32(100));
    expected.push(1);
    expected.extend_from_slice(&le32(200));
    expected.extend_from_slice(&le32(3));
    expected.extend_from_slice(&le32(0));
    expected.extend_from_slice(&le32(0));
    expected.push(0b0100_0000 | 0b0110 << 1); // HasPet=0, HasPetResult=1, HasDisplay=0, PetResult
    expected.extend_from_slice(b"xy");
    assert_eq!(bytes, expected);
}

#[test]
fn product_list_response_byte_exact_minimal_shop() {
    let resp = BattlePayGetProductListResponse {
        result: 0,
        currency_id: 3,
        product_infos: vec![BattlePayProductInfo {
            product_id: 42,
            normal_price_fixed_point: 2500,
            current_price_fixed_point: 1999,
            product_ids: vec![42],
            unk1: 0,
            unk2: 0,
            unk_ints: vec![],
            unk3: 0,
            choice_type: 1,
            display_info: None,
        }],
        products: vec![],
        product_groups: vec![],
        shop_entries: vec![BattlePayShopEntry {
            entry_id: 7,
            group_id: 30,
            product_id: 42,
            ordering: -1,
            vas_service_type: 0,
            store_delivery_type: 0,
            display_info: None,
        }],
    };
    let bytes = resp.to_bytes();
    assert_eq!(
        u16::from_le_bytes([bytes[0], bytes[1]]),
        ServerOpcodes::BattlePayGetProductListResponse as u16
    );
    let mut expected = Vec::new();
    expected.extend_from_slice(&le32(0)); // result
    expected.extend_from_slice(&le32(3)); // currency
    expected.extend_from_slice(&le32(1)); // product infos
    expected.extend_from_slice(&le32(0)); // products
    expected.extend_from_slice(&le32(0)); // groups
    expected.extend_from_slice(&le32(1)); // shop entries
    // product info
    expected.extend_from_slice(&le32(42));
    expected.extend_from_slice(&le64(2500));
    expected.extend_from_slice(&le64(1999));
    expected.extend_from_slice(&le32(1)); // product id count
    expected.extend_from_slice(&le32(0)); // unk1
    expected.extend_from_slice(&le32(0)); // unk2
    expected.extend_from_slice(&le32(0)); // unk int count
    expected.extend_from_slice(&le32(0)); // unk3
    expected.extend_from_slice(&le32(42)); // product ids[0]
    expected.push(1 << 1); // choice type 1 (7 bits) + HasDisplay 0
    // shop entry
    expected.extend_from_slice(&le32(7));
    expected.extend_from_slice(&le32(30));
    expected.extend_from_slice(&le32(42));
    expected.extend_from_slice(&(-1i32).to_le_bytes());
    expected.extend_from_slice(&le32(0));
    expected.push(0);
    expected.push(0); // HasDisplayInfo bit
    assert_eq!(payload(&bytes), expected);
}

#[test]
fn purchase_list_and_update_share_the_purchase_layout() {
    let purchase = BattlePayPurchase {
        purchase_id: 0x0102_0304_0506_0708,
        status: 2,
        result_code: 0,
        product_id: 42,
        unk1: 0,
        unk2: 0,
        unk3: 0,
        wallet_name: "Wallet".to_string(),
    };
    let mut expected_purchase = Vec::new();
    expected_purchase.extend_from_slice(&le64(0x0102_0304_0506_0708));
    expected_purchase.extend_from_slice(&le32(2));
    expected_purchase.extend_from_slice(&le32(0));
    expected_purchase.extend_from_slice(&le32(42));
    expected_purchase.extend_from_slice(&[0; 24]);
    expected_purchase.push(6);
    expected_purchase.extend_from_slice(b"Wallet");

    let list = BattlePayGetPurchaseListResponse {
        result: 0,
        purchases: vec![purchase.clone()],
    }
    .to_bytes();
    let mut expected = Vec::new();
    expected.extend_from_slice(&le32(0));
    expected.extend_from_slice(&le32(1));
    expected.extend_from_slice(&expected_purchase);
    assert_eq!(payload(&list), expected);

    let update = BattlePayPurchaseUpdate {
        purchases: vec![purchase],
    }
    .to_bytes();
    let mut expected = Vec::new();
    expected.extend_from_slice(&le32(1));
    expected.extend_from_slice(&expected_purchase);
    assert_eq!(payload(&update), expected);
}

#[test]
fn distribution_list_uses_11_bit_count_and_object_layout() {
    let target = ObjectGuid::create_player(1, 7);
    let resp = BattlePayGetDistributionListResponse {
        result: 0,
        distribution_objects: vec![BattlePayDistributionObject {
            distribution_id: 9,
            status: 1,
            product_id: 42,
            target_player: target,
            unk_guid: ObjectGuid::EMPTY,
            target_virtual_realm: 0x0100_0001,
            target_native_realm: 0x0100_0001,
            purchase_id: 77,
            unk: 0,
            product: None,
            revoked: true,
        }],
    }
    .to_bytes();
    let body = payload(&resp);
    assert_eq!(&body[..4], &le32(0));
    // 11-bit count = 1 -> bytes 0x00, 0x20 (MSB first)
    assert_eq!(&body[4..6], &[0x00, 0x20]);
    let mut pkt = WorldPacket::from_bytes(&body[6..]);
    assert_eq!(pkt.read_uint64().unwrap(), 9);
    assert_eq!(pkt.read_uint32().unwrap(), 1);
    assert_eq!(pkt.read_uint32().unwrap(), 42);
    assert_eq!(pkt.read_packed_guid().unwrap(), target);
    assert_eq!(pkt.read_packed_guid().unwrap(), ObjectGuid::EMPTY);
    assert_eq!(pkt.read_uint32().unwrap(), 0x0100_0001);
    assert_eq!(pkt.read_uint32().unwrap(), 0x0100_0001);
    assert_eq!(pkt.read_uint64().unwrap(), 77);
    assert_eq!(pkt.read_uint32().unwrap(), 0);
    assert_eq!(
        pkt.read_uint8().unwrap(),
        0b0100_0000,
        "HasProduct=0, Revoked=1"
    );
    assert!(pkt.is_empty());
}

#[test]
fn checkout_and_sso_packets_are_byte_exact() {
    let checkout = BattlePayStartCheckout {
        product_id: 42,
        game_service_region_id: 3,
        game_account_id: 1234,
        server_validation_signature: "sig".to_string(),
        external_transaction_id: "ext-1".to_string(),
        subscription: false,
    }
    .to_bytes();
    let mut expected = Vec::new();
    expected.extend_from_slice(&le32(42));
    expected.extend_from_slice(&le32(3));
    expected.extend_from_slice(&le64(1234));
    // bits: 000011 0000101 0 -> 00001100 00101000
    expected.extend_from_slice(&[0b0000_1100, 0b0010_1000]);
    expected.extend_from_slice(b"sig");
    expected.extend_from_slice(b"ext-1");
    assert_eq!(payload(&checkout), expected);

    let sso = GenerateSsoTokenResponse {
        kind: 5,
        result: 0,
        unk1: 0,
        unk2: 0,
        token: "tok".to_string(),
    }
    .to_bytes();
    let mut expected = Vec::new();
    expected.extend_from_slice(&le32(5));
    expected.extend_from_slice(&le32(0));
    expected.extend_from_slice(&[0; 16]);
    expected.push(3 << 1); // 7-bit length, client reads byte >> 1
    expected.extend_from_slice(b"tok");
    assert_eq!(payload(&sso), expected);
}

#[test]
fn purchase_flow_pods_match_client_handler_offsets() {
    let start = BattlePayStartPurchaseResponse {
        purchase_id: 1,
        purchase_result: 0,
        client_token: 99,
    }
    .to_bytes();
    let body = payload(&start);
    assert_eq!(body.len(), 16);
    assert_eq!(&body[8..12], &le32(0), "PurchaseResult at +8");
    assert_eq!(&body[12..16], &le32(99), "ClientToken at +12");

    let confirm = BattlePayConfirmPurchase {
        purchase_id: 1,
        server_token: 7,
    }
    .to_bytes();
    assert_eq!(
        payload(&confirm),
        [le64(1).as_slice(), le32(7).as_slice()].concat()
    );

    let ack = BattlePayAckFailed {
        purchase_id: 1,
        server_token: 7,
        status: 2,
        result: 5,
    }
    .to_bytes();
    let body = payload(&ack);
    assert_eq!(&body[8..12], &le32(7), "ServerToken at +8");
    assert_eq!(&body[12..16], &le32(2), "Status at +12");
    assert_eq!(&body[16..20], &le32(5), "Result at +16");

    assert_eq!(
        payload(&DisplayPromotion { promotion_id: 3 }.to_bytes()),
        le32(3)
    );
    assert_eq!(
        payload(&BattlePayMountDelivered { product_id: 42 }.to_bytes()),
        le32(42)
    );
    assert_eq!(
        payload(&CharacterUpgradeManualUnrevokeResult { result: 1 }.to_bytes()),
        le32(1)
    );
}

#[test]
fn delivery_ended_writes_item_instances() {
    let bytes = BattlePayDeliveryEnded {
        distribution_id: 5,
        items: vec![ItemInstance {
            item_id: 19019,
            ..Default::default()
        }],
    }
    .to_bytes();
    let body = payload(&bytes);
    let mut pkt = WorldPacket::from_bytes(&body);
    assert_eq!(pkt.read_uint64().unwrap(), 5);
    assert_eq!(pkt.read_uint32().unwrap(), 1);
    assert_eq!(pkt.read_int32().unwrap(), 19019);
}

#[test]
fn entitlement_and_vas_packets_bit_fields() {
    let sync = SyncWowEntitlements {
        entitlements: vec![WowEntitlement {
            unk1: 1,
            unk2: 2,
            unk3: 3,
            unk4: 4,
            unk_bit: true,
        }],
        products: vec![],
    }
    .to_bytes();
    let mut expected = Vec::new();
    expected.extend_from_slice(&le32(1));
    expected.extend_from_slice(&le32(0));
    expected.extend_from_slice(&le32(1));
    expected.extend_from_slice(&le64(2));
    expected.extend_from_slice(&le64(3));
    expected.extend_from_slice(&le32(4));
    expected.push(0x80);
    assert_eq!(payload(&sync), expected);

    let notif = WowEntitlementNotification {
        unk: 0b101,
        entitlement: WowEntitlement::default(),
        product: BattlePayProduct::default(),
    }
    .to_bytes();
    assert_eq!(
        payload(&notif)[0],
        0b1010_0000,
        "3-bit value in the top bits"
    );

    let guid = ObjectGuid::create_player(1, 3);
    let vas = EnumVasPurchaseStatesResponse {
        purchases: vec![VasPurchase {
            player_guid: guid,
            product_id: 189,
            state: 2,
            purchase_id: 11,
            errors: vec![13, 14],
        }],
    }
    .to_bytes();
    let body = payload(&vas);
    assert_eq!(body[0], 1 << 2, "6-bit count, client reads byte >> 2");
    let mut pkt = WorldPacket::from_bytes(&body[1..]);
    assert_eq!(pkt.read_packed_guid().unwrap(), guid);
    assert_eq!(pkt.read_uint32().unwrap(), 189);
    assert_eq!(pkt.read_uint32().unwrap(), 2);
    assert_eq!(pkt.read_uint64().unwrap(), 11);
    assert_eq!(pkt.read_uint8().unwrap(), 2 << 6, "2-bit error count");
    assert_eq!(pkt.read_uint32().unwrap(), 13);
    assert_eq!(pkt.read_uint32().unwrap(), 14);
    assert!(pkt.is_empty());

    let complete = VasPurchaseComplete {
        product_id: 1,
        result: 0,
        character_guid: guid,
        glue_guid: guid,
        unk: 0,
        handled_guid: ObjectGuid::EMPTY,
        character_name: "Abc".to_string(),
    }
    .to_bytes();
    let body = payload(&complete);
    assert_eq!(body[body.len() - 4], 3 << 2, "6-bit name length");
    assert_eq!(&body[body.len() - 3..], b"Abc");
}

// ── CMSG reads, encoded exactly like the client's Write functions ──

#[test]
fn start_purchase_reads_client_write_layout() {
    let guid = ObjectGuid::create_player(1, 9);
    let mut w = WorldPacket::new_empty();
    w.write_uint32(0x1234); // ClientToken
    w.write_uint32(42); // ProductID
    w.write_packed_guid(&guid);
    w.write_bits(3, 6); // "win"
    w.write_bits(4, 12); // "pkey"
    w.write_bits(2, 7); // "ci"
    w.flush_bits();
    w.write_string("win");
    w.write_string("pkey");
    w.write_string("ci");
    let body = w.into_data();
    // 25 bits -> 4 bytes between the guid and the strings
    assert_eq!(body.len(), 8 + guid_len(&guid) + 4 + 3 + 4 + 2);

    let p: BattlePayStartPurchase = client_read(&body).unwrap();
    assert_eq!(p.client_token, 0x1234);
    assert_eq!(p.product_id, 42);
    assert_eq!(p.target_character, guid);
    assert_eq!(p.wow_system, "win");
    assert_eq!(p.public_key, "pkey");
    assert_eq!(p.unk_string, "ci");
}

fn guid_len(guid: &ObjectGuid) -> usize {
    let mut w = WorldPacket::new_empty();
    w.write_packed_guid(guid);
    w.size()
}

#[test]
fn confirm_purchase_response_reads_bit_then_token_then_price() {
    // Client Write 0x1407646e0: bit (flushed as one byte), u32, u64
    let mut body = vec![0x80];
    body.extend_from_slice(&le32(7));
    body.extend_from_slice(&le64(1999));
    let p: BattlePayConfirmPurchaseResponse = client_read(&body).unwrap();
    assert!(p.confirm_purchase);
    assert_eq!(p.server_token, 7);
    assert_eq!(p.client_current_price_fixed_point, 1999);

    let p: BattlePayAckFailedResponse = client_read(&le32(7)).unwrap();
    assert_eq!(p.server_token, 7);
    let p: BattlePayOpenCheckout = client_read(&le32(12)).unwrap();
    assert_eq!(p.request_id, 12);
    let p: BattlePayRequestPriceInfo = client_read(&[le32(1), le32(42)].concat()).unwrap();
    assert_eq!((p.client_token, p.product_id), (1, 42));
}

#[test]
fn distribution_assign_to_target_reads_wod_order() {
    let guid = ObjectGuid::create_player(1, 2);
    let mut w = WorldPacket::new_empty();
    w.write_uint32(5);
    w.write_uint64(0x99);
    w.write_packed_guid(&guid);
    w.write_uint32(1);
    let p: BattlePayDistributionAssignToTarget = client_read(&w.into_data()).unwrap();
    assert_eq!(p.client_token, 5);
    assert_eq!(p.distribution_id, 0x99);
    assert_eq!(p.target_character, guid);
    assert_eq!(p.product_choice, 1);
}

#[test]
fn cancel_open_checkout_and_purchase_submitted_string_bits() {
    let mut w = WorldPacket::new_empty();
    w.write_bits(5, 6);
    w.write_bit(true);
    w.flush_bits();
    w.write_string("ext-1");
    let p: BattlePayCancelOpenCheckout = client_read(&w.into_data()).unwrap();
    assert_eq!(p.external_transaction_id, "ext-1");
    assert!(p.unk_bit);

    let mut w = WorldPacket::new_empty();
    w.write_bits(3, 6);
    w.write_bits(5, 7);
    w.write_bit(false);
    w.flush_bits();
    w.write_string("ord");
    w.write_string("ext-1");
    let p: BattlePayPurchaseSubmitted = client_read(&w.into_data()).unwrap();
    assert_eq!(p.global_order_id, "ord");
    assert_eq!(p.external_transaction_id, "ext-1");
    assert!(!p.unk_bit);
}

#[test]
fn empty_and_guid_only_client_packets() {
    assert!(client_read::<BattlePayGetProductList>(&[]).is_ok());
    assert!(client_read::<BattlePayGetPurchaseList>(&[]).is_ok());
    assert!(client_read::<UpdateVasPurchaseStates>(&[]).is_ok());

    let guid = ObjectGuid::create_player(1, 4);
    let mut w = WorldPacket::new_empty();
    w.write_packed_guid(&guid);
    let p: CharacterUpgradeManualUnrevokeRequest = client_read(w.data()).unwrap();
    assert_eq!(p.character_guid, guid);
    w.write_uint32(3);
    let p: CharacterUpgradeStart = client_read(&w.into_data()).unwrap();
    assert_eq!((p.character_guid, p.unk), (guid, 3));
}

#[test]
fn vas_client_packets_round_trip_client_layout() {
    let g = ObjectGuid::create_player(1, 8);
    let mut w = WorldPacket::new_empty();
    w.write_uint32(1);
    w.write_uint32(189);
    w.write_packed_guid(&g);
    w.write_uint32(0x0100_0001);
    w.write_uint32(0);
    w.write_packed_guid(&g);
    w.write_packed_guid(&g);
    w.write_packed_guid(&ObjectGuid::EMPTY);
    w.write_bits(4, 6);
    w.write_bits(0, 7);
    w.write_bits(0, 7);
    w.write_bits(3, 6);
    w.write_bits(2, 12);
    w.write_bit(true);
    w.flush_bits();
    w.write_string("Name");
    w.write_string("win");
    w.write_string("{}");
    let p: BattlePayStartVasPurchase = client_read(&w.into_data()).unwrap();
    assert_eq!(p.product_id, 189);
    assert_eq!(p.character_guid, g);
    assert_eq!(p.new_character_name, "Name");
    assert_eq!(p.platform, "win");
    assert_eq!(p.client_info, "{}");
    assert!(p.unk_bit);

    let mut w = WorldPacket::new_empty();
    w.write_uint32(1);
    w.write_uint64(2);
    w.write_packed_guid(&g);
    w.write_uint32(3);
    w.write_uint32(4);
    w.write_packed_guid(&g);
    w.write_packed_guid(&g);
    w.write_packed_guid(&g);
    w.write_uint8(5);
    w.write_uint8(6);
    w.write_uint8(7);
    w.write_uint32(2);
    w.write_uint32(10);
    w.write_uint32(11);
    w.write_uint32(12);
    w.write_uint32(13);
    w.write_bits(1, 6);
    w.write_bits(1, 7);
    w.write_bits(1, 7);
    w.write_bit(true);
    w.write_bit(false);
    w.write_bit(true);
    w.flush_bits();
    w.write_string("a");
    w.write_string("b");
    w.write_string("c");
    let p: BattlePayDistributionAssignVas = client_read(&w.into_data()).unwrap();
    assert_eq!(p.distribution_id, 2);
    assert_eq!(p.pairs, vec![(10, 11), (12, 13)]);
    assert_eq!((p.unk_byte1, p.unk_byte2, p.unk_byte3), (5, 6, 7));
    assert_eq!(
        (
            p.unk_string1.as_str(),
            p.unk_string2.as_str(),
            p.unk_string3.as_str()
        ),
        ("a", "b", "c")
    );
    assert_eq!((p.unk_bit1, p.unk_bit2, p.unk_bit3), (true, false, true));
}

#[test]
fn assign_vas_rejects_oversized_pair_count() {
    let g = ObjectGuid::EMPTY;
    let mut w = WorldPacket::new_empty();
    w.write_uint32(1);
    w.write_uint64(2);
    w.write_packed_guid(&g);
    w.write_uint32(3);
    w.write_uint32(4);
    w.write_packed_guid(&g);
    w.write_packed_guid(&g);
    w.write_packed_guid(&g);
    w.write_uint8(0);
    w.write_uint8(0);
    w.write_uint8(0);
    w.write_uint32(0x7fff_ffff);
    assert!(matches!(
        client_read::<BattlePayDistributionAssignVas>(&w.into_data()),
        Err(PacketError::InvalidArrayCapacity { .. })
    ));
}
