// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! In-game shop (BattlePay) packet codecs for client build 3.4.3.54261.
//!
//! Evidence: `docs/migration/battlepay-343-protocol.md`. Every layout below was
//! transcribed from the JAM readers/writers of the running 54261 client
//! (`WowClassic.exe` image dumped from `/proc/<pid>/mem`; addresses in the doc
//! are image-relative to base `0x140000000`). TrinityCore 3.4.3 has no
//! BattlePay handlers and WowPacketParser only has a `ReadToEnd` stub for this
//! branch, so the client is the primary source; WoD/Legion layouts are cited in
//! the doc only as lineage.
//!
//! Field names marked `unk*` are read by the client but their meaning could
//! not be established; keep them zero unless the doc says otherwise.

use wow_constants::{ClientOpcodes, ServerOpcodes};
use wow_core::ObjectGuid;

use crate::{ClientPacket, PacketError, ServerPacket, WorldPacket, packets::item::ItemInstance};

/// Upper bound for client-provided array counts that we materialise.
pub const BATTLEPAY_MAX_CLIENT_ARRAY: usize = 64;

fn check_capacity(requested: usize) -> Result<(), PacketError> {
    if requested > BATTLEPAY_MAX_CLIENT_ARRAY {
        return Err(PacketError::InvalidArrayCapacity {
            requested,
            max: BATTLEPAY_MAX_CLIENT_ARRAY,
        });
    }
    Ok(())
}

/// Write a string whose length was already emitted as a bit field.
fn write_str_body(pkt: &mut WorldPacket, s: &str) {
    pkt.write_string(s);
}

// ── Shared sub-structures ─────────────────────────────────────────────

/// `JamBattlePayDisplayInfo` visual entry (client reader 0x140709370, loop at 0x140709830).
///
/// Wire: `bits(10) NameLen; flush; u32 DisplayId; u32 VisualId; u32 Unk; Name`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayVisual {
    pub display_id: u32,
    pub visual_id: u32,
    /// New in 3.4.3 (7.3.5 had only DisplayId/VisualId).
    pub unk: u32,
    /// Max 1023 bytes (10-bit length).
    pub name: String,
}

impl BattlePayVisual {
    pub fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.name.len() < (1 << 10));
        pkt.write_bits(self.name.len() as u32, 10);
        pkt.flush_bits();
        pkt.write_uint32(self.display_id);
        pkt.write_uint32(self.visual_id);
        pkt.write_uint32(self.unk);
        write_str_body(pkt, &self.name);
    }
}

/// `JamBattlepayDisplayCard` / display info (client reader 0x140709370).
///
/// Bit header (12 bytes after flush): HasCreatureDisplayID(1), HasFileDataID(1),
/// Name1(10), Name2(10), Name3(13), Name4(13), Name5(13), HasFlags(1), HasUnk1(1),
/// HasUnk2(1), HasUnk3(1), Name6(13), Name7(12). Then u32 VisualCount, u32 Unk4,
/// u32 Unk5, u32 Unk6, optional CreatureDisplayID/FileDataID, Name1..Name5,
/// optional Flags/Unk1/Unk2/Unk3, Name6, Name7, visuals.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDisplayInfo {
    pub creature_display_id: Option<u32>,
    pub file_data_id: Option<u32>,
    /// 10-bit length. Lua `sharedData.name`.
    pub name1: String,
    /// 10-bit length.
    pub name2: String,
    /// 13-bit length. Lua `sharedData.description` (bullets separated by `$bullet`).
    pub name3: String,
    /// 13-bit length.
    pub name4: String,
    /// 13-bit length (new in 3.4.3).
    pub name5: String,
    /// 13-bit length (new in 3.4.3).
    pub name6: String,
    /// 12-bit length (new in 3.4.3).
    pub name7: String,
    /// Lua `sharedData.flags` (Enum.BattlepayDisplayFlag bitmask).
    pub flags: Option<u32>,
    pub unk1: Option<u32>,
    pub unk2: Option<u32>,
    pub unk3: Option<u32>,
    /// Three plain u32 following the visual count (new in 3.4.3).
    pub unk4: u32,
    pub unk5: u32,
    pub unk6: u32,
    pub visuals: Vec<BattlePayVisual>,
}

impl BattlePayDisplayInfo {
    pub fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.name1.len() < (1 << 10));
        debug_assert!(self.name2.len() < (1 << 10));
        debug_assert!(self.name3.len() < (1 << 13));
        debug_assert!(self.name4.len() < (1 << 13));
        debug_assert!(self.name5.len() < (1 << 13));
        debug_assert!(self.name6.len() < (1 << 13));
        debug_assert!(self.name7.len() < (1 << 12));

        pkt.write_bit(self.creature_display_id.is_some());
        pkt.write_bit(self.file_data_id.is_some());
        pkt.write_bits(self.name1.len() as u32, 10);
        pkt.write_bits(self.name2.len() as u32, 10);
        pkt.write_bits(self.name3.len() as u32, 13);
        pkt.write_bits(self.name4.len() as u32, 13);
        pkt.write_bits(self.name5.len() as u32, 13);
        pkt.write_bit(self.flags.is_some());
        pkt.write_bit(self.unk1.is_some());
        pkt.write_bit(self.unk2.is_some());
        pkt.write_bit(self.unk3.is_some());
        pkt.write_bits(self.name6.len() as u32, 13);
        pkt.write_bits(self.name7.len() as u32, 12);
        pkt.flush_bits();

        pkt.write_uint32(self.visuals.len() as u32);
        pkt.write_uint32(self.unk4);
        pkt.write_uint32(self.unk5);
        pkt.write_uint32(self.unk6);
        if let Some(v) = self.creature_display_id {
            pkt.write_uint32(v);
        }
        if let Some(v) = self.file_data_id {
            pkt.write_uint32(v);
        }
        write_str_body(pkt, &self.name1);
        write_str_body(pkt, &self.name2);
        write_str_body(pkt, &self.name3);
        write_str_body(pkt, &self.name4);
        write_str_body(pkt, &self.name5);
        if let Some(v) = self.flags {
            pkt.write_uint32(v);
        }
        if let Some(v) = self.unk1 {
            pkt.write_uint32(v);
        }
        if let Some(v) = self.unk2 {
            pkt.write_uint32(v);
        }
        if let Some(v) = self.unk3 {
            pkt.write_uint32(v);
        }
        write_str_body(pkt, &self.name6);
        write_str_body(pkt, &self.name7);
        for visual in &self.visuals {
            visual.write(pkt);
        }
    }
}

/// Item inside a `JamBattlePayProduct` (client reader 0x140708b10, loop at 0x140708d31).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayProductItem {
    pub id: u32,
    pub unk_byte: u8,
    pub item_id: u32,
    pub quantity: u32,
    pub unk1: u32,
    pub unk2: u32,
    pub has_pet: bool,
    /// 4-bit value.
    pub pet_result: Option<u8>,
    pub display_info: Option<BattlePayDisplayInfo>,
}

impl BattlePayProductItem {
    pub fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.id);
        pkt.write_uint8(self.unk_byte);
        pkt.write_uint32(self.item_id);
        pkt.write_uint32(self.quantity);
        pkt.write_uint32(self.unk1);
        pkt.write_uint32(self.unk2);
        pkt.write_bit(self.has_pet);
        pkt.write_bit(self.pet_result.is_some());
        pkt.write_bit(self.display_info.is_some());
        if let Some(pr) = self.pet_result {
            pkt.write_bits(u32::from(pr), 4);
        }
        pkt.flush_bits();
        if let Some(d) = &self.display_info {
            d.write(pkt);
        }
    }
}

/// `JamBattlePayProduct` (client reader 0x140708b10).
///
/// Wire: u32 ProductID; u8 Type; u32 Flags; u32 Unk1; u32 DisplayId; u32 ItemId;
/// u32 Unk4..Unk9 (six u32); bits: UnkStringLen(8), UnkBit(1), HasUnkBits(1),
/// ItemCount(7), HasDisplayInfo(1), [UnkBits(4)]; flush; items; UnkString; [DisplayInfo].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayProduct {
    pub product_id: u32,
    pub product_type: u8,
    pub flags: u32,
    pub unk1: u32,
    pub display_id: u32,
    pub item_id: u32,
    pub unk4: u32,
    pub unk5: u32,
    /// Unk6..Unk9 are new in 3.4.3 (7.3.5 stopped at Unk5).
    pub unk6: u32,
    pub unk7: u32,
    pub unk8: u32,
    pub unk9: u32,
    /// 8-bit length; written after the items.
    pub unk_string: String,
    pub unk_bit: bool,
    /// 4-bit value.
    pub unk_bits: Option<u8>,
    /// Max 127 (7-bit count).
    pub items: Vec<BattlePayProductItem>,
    pub display_info: Option<BattlePayDisplayInfo>,
}

impl BattlePayProduct {
    pub fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.unk_string.len() < (1 << 8));
        debug_assert!(self.items.len() < (1 << 7));
        pkt.write_uint32(self.product_id);
        pkt.write_uint8(self.product_type);
        pkt.write_uint32(self.flags);
        pkt.write_uint32(self.unk1);
        pkt.write_uint32(self.display_id);
        pkt.write_uint32(self.item_id);
        pkt.write_uint32(self.unk4);
        pkt.write_uint32(self.unk5);
        pkt.write_uint32(self.unk6);
        pkt.write_uint32(self.unk7);
        pkt.write_uint32(self.unk8);
        pkt.write_uint32(self.unk9);
        pkt.write_bits(self.unk_string.len() as u32, 8);
        pkt.write_bit(self.unk_bit);
        pkt.write_bit(self.unk_bits.is_some());
        pkt.write_bits(self.items.len() as u32, 7);
        pkt.write_bit(self.display_info.is_some());
        if let Some(b) = self.unk_bits {
            pkt.write_bits(u32::from(b), 4);
        }
        pkt.flush_bits();
        for item in &self.items {
            item.write(pkt);
        }
        write_str_body(pkt, &self.unk_string);
        if let Some(d) = &self.display_info {
            d.write(pkt);
        }
    }
}

/// Product price/choice info (first array of the product list; client reader 0x140709000).
///
/// Wire: u32 ProductID; u64 NormalPriceFixedPoint; u64 CurrentPriceFixedPoint;
/// u32 ProductIDCount; u32 Unk1; u32 Unk2; u32 UnkIntCount; u32 Unk3; u32 ProductIDs[];
/// u32 UnkInts[]; bits: ChoiceType(7), HasDisplayInfo(1); flush; [DisplayInfo].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayProductInfo {
    pub product_id: u32,
    /// Lua `sharedData.normalDollars/normalCents` = value / 100, value % 100.
    pub normal_price_fixed_point: u64,
    pub current_price_fixed_point: u64,
    pub product_ids: Vec<u32>,
    pub unk1: u32,
    /// New in 3.4.3.
    pub unk2: u32,
    pub unk_ints: Vec<u32>,
    /// New in 3.4.3.
    pub unk3: u32,
    /// 7-bit value.
    pub choice_type: u8,
    pub display_info: Option<BattlePayDisplayInfo>,
}

impl BattlePayProductInfo {
    pub fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.product_id);
        pkt.write_uint64(self.normal_price_fixed_point);
        pkt.write_uint64(self.current_price_fixed_point);
        pkt.write_uint32(self.product_ids.len() as u32);
        pkt.write_uint32(self.unk1);
        pkt.write_uint32(self.unk2);
        pkt.write_uint32(self.unk_ints.len() as u32);
        pkt.write_uint32(self.unk3);
        for id in &self.product_ids {
            pkt.write_uint32(*id);
        }
        for v in &self.unk_ints {
            pkt.write_uint32(*v);
        }
        pkt.write_bits(u32::from(self.choice_type), 7);
        pkt.write_bit(self.display_info.is_some());
        pkt.flush_bits();
        if let Some(d) = &self.display_info {
            d.write(pkt);
        }
    }
}

/// `JamBattlePayProductGroup` (inline in the product-list reader, loop at 0x1406f8880).
///
/// Wire: u32 GroupID; u32 IconFileDataID; u8 DisplayType; u32 Ordering; u32 Flags;
/// u32 Unk; bits(8) NameLen; bits(24) DescriptionField; (4 bytes total) Name;
/// Description (only when DescriptionField >= 2: DescriptionField bytes, NUL-terminated).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayProductGroup {
    pub group_id: u32,
    pub icon_file_data_id: u32,
    /// Lua `displayType` (Enum.BattlepayGroupDisplayType).
    pub display_type: u8,
    pub ordering: u32,
    /// Lua `flags` (Enum.BattlepayProductGroupFlag).
    pub flags: u32,
    /// New in 3.4.3.
    pub unk: u32,
    /// Lua `groupName`, max 255 bytes.
    pub name: String,
    /// Lua `disabledTooltip` (7.3.5 "IsAvailableDescription"); max 2^24-3 bytes.
    pub is_available_description: String,
}

impl BattlePayProductGroup {
    pub fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.name.len() < (1 << 8));
        pkt.write_uint32(self.group_id);
        pkt.write_uint32(self.icon_file_data_id);
        pkt.write_uint8(self.display_type);
        pkt.write_uint32(self.ordering);
        pkt.write_uint32(self.flags);
        pkt.write_uint32(self.unk);
        pkt.write_bits(self.name.len() as u32, 8);
        // The client reads the 24-bit field, consumes nothing for 0/1 and otherwise
        // consumes that many bytes requiring the last one to be NUL.
        let desc_field = if self.is_available_description.is_empty() {
            0
        } else {
            self.is_available_description.len() as u32 + 1
        };
        pkt.write_bits(desc_field, 24);
        pkt.flush_bits();
        write_str_body(pkt, &self.name);
        if !self.is_available_description.is_empty() {
            pkt.write_cstring(&self.is_available_description);
        }
    }
}

/// `JamBattlePayShopEntry` (inline in the product-list reader, loop at 0x1406f89c0).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayShopEntry {
    pub entry_id: u32,
    pub group_id: u32,
    pub product_id: u32,
    pub ordering: i32,
    /// Lua `sharedData.vasServiceType`.
    pub vas_service_type: u32,
    /// Lua `bannerType`? (7.3.5 name "StoreDeliveryType").
    pub store_delivery_type: u8,
    pub display_info: Option<BattlePayDisplayInfo>,
}

impl BattlePayShopEntry {
    pub fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.entry_id);
        pkt.write_uint32(self.group_id);
        pkt.write_uint32(self.product_id);
        pkt.write_int32(self.ordering);
        pkt.write_uint32(self.vas_service_type);
        pkt.write_uint8(self.store_delivery_type);
        pkt.write_bit(self.display_info.is_some());
        pkt.flush_bits();
        if let Some(d) = &self.display_info {
            d.write(pkt);
        }
    }
}

/// `JamBattlePayPurchase` (client reader 0x1407091e0).
///
/// Wire: u64 PurchaseID; u32 Status; u32 ResultCode; u32 ProductID; u64 Unk1; u64 Unk2;
/// u64 Unk3; bits(8) WalletNameLen; flush; WalletName.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayPurchase {
    pub purchase_id: u64,
    pub status: u32,
    pub result_code: u32,
    pub product_id: u32,
    /// 7.3.5 had u64,u64,u32 here; 3.4.3 reads three u64.
    pub unk1: u64,
    pub unk2: u64,
    pub unk3: u64,
    /// Max 255 bytes.
    pub wallet_name: String,
}

impl BattlePayPurchase {
    pub fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.wallet_name.len() < (1 << 8));
        pkt.write_uint64(self.purchase_id);
        pkt.write_uint32(self.status);
        pkt.write_uint32(self.result_code);
        pkt.write_uint32(self.product_id);
        pkt.write_uint64(self.unk1);
        pkt.write_uint64(self.unk2);
        pkt.write_uint64(self.unk3);
        pkt.write_bits(self.wallet_name.len() as u32, 8);
        pkt.flush_bits();
        write_str_body(pkt, &self.wallet_name);
    }
}

/// `JamBattlePayDistributionObject` (client reader 0x140708ec0).
///
/// Wire: u64 DistributionID; u32 Status; u32 ProductID; packed guid TargetPlayer;
/// packed guid UnkGuid; u32 TargetVirtualRealm; u32 TargetNativeRealm; u64 PurchaseID;
/// u32 Unk; bits: HasProduct(1), Revoked(1); flush; [Product].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDistributionObject {
    pub distribution_id: u64,
    pub status: u32,
    pub product_id: u32,
    pub target_player: ObjectGuid,
    /// New in 3.4.3 (second GUID right after TargetPlayer).
    pub unk_guid: ObjectGuid,
    pub target_virtual_realm: u32,
    pub target_native_realm: u32,
    pub purchase_id: u64,
    /// New in 3.4.3.
    pub unk: u32,
    pub product: Option<BattlePayProduct>,
    pub revoked: bool,
}

impl BattlePayDistributionObject {
    pub fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint64(self.distribution_id);
        pkt.write_uint32(self.status);
        pkt.write_uint32(self.product_id);
        pkt.write_packed_guid(&self.target_player);
        pkt.write_packed_guid(&self.unk_guid);
        pkt.write_uint32(self.target_virtual_realm);
        pkt.write_uint32(self.target_native_realm);
        pkt.write_uint64(self.purchase_id);
        pkt.write_uint32(self.unk);
        pkt.write_bit(self.product.is_some());
        pkt.write_bit(self.revoked);
        pkt.flush_bits();
        if let Some(p) = &self.product {
            p.write(pkt);
        }
    }
}

/// VAS purchase state entry (client reader 0x14071f6b0).
///
/// Wire: packed guid PlayerGuid; u32 ProductID; u32 State; u64 PurchaseID;
/// bits(2) ErrorCount; flush; u32 Errors[].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VasPurchase {
    pub player_guid: ObjectGuid,
    pub product_id: u32,
    /// Enum.VasPurchaseProgress.
    pub state: u32,
    pub purchase_id: u64,
    /// Max 3 (2-bit count); Enum.VasError values.
    pub errors: Vec<u32>,
}

impl VasPurchase {
    pub fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.errors.len() < 4);
        pkt.write_packed_guid(&self.player_guid);
        pkt.write_uint32(self.product_id);
        pkt.write_uint32(self.state);
        pkt.write_uint64(self.purchase_id);
        pkt.write_bits(self.errors.len() as u32, 2);
        pkt.flush_bits();
        for e in &self.errors {
            pkt.write_uint32(*e);
        }
    }
}

/// `JamWowEntitlement` (client reader 0x14071ad50).
///
/// Wire: u32 Unk1; u64 Unk2; u64 Unk3; u32 Unk4; bit UnkBit; flush.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WowEntitlement {
    pub unk1: u32,
    pub unk2: u64,
    pub unk3: u64,
    pub unk4: u32,
    pub unk_bit: bool,
}

impl WowEntitlement {
    pub fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.unk1);
        pkt.write_uint64(self.unk2);
        pkt.write_uint64(self.unk3);
        pkt.write_uint32(self.unk4);
        pkt.write_bit(self.unk_bit);
        pkt.flush_bits();
    }
}

// ── Server packets ────────────────────────────────────────────────────

/// SMSG_BATTLE_PAY_GET_PRODUCT_LIST_RESPONSE 0x2775 (client reader 0x1406f84c0).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayGetProductListResponse {
    pub result: u32,
    /// `C_StoreSecure.GetCurrencyID()`; 1 USD, 2 KRW(?), 3 EUR... see doc.
    pub currency_id: u32,
    pub product_infos: Vec<BattlePayProductInfo>,
    pub products: Vec<BattlePayProduct>,
    pub product_groups: Vec<BattlePayProductGroup>,
    pub shop_entries: Vec<BattlePayShopEntry>,
}

impl ServerPacket for BattlePayGetProductListResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayGetProductListResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.result);
        pkt.write_uint32(self.currency_id);
        pkt.write_uint32(self.product_infos.len() as u32);
        pkt.write_uint32(self.products.len() as u32);
        pkt.write_uint32(self.product_groups.len() as u32);
        pkt.write_uint32(self.shop_entries.len() as u32);
        for v in &self.product_infos {
            v.write(pkt);
        }
        for v in &self.products {
            v.write(pkt);
        }
        for v in &self.product_groups {
            v.write(pkt);
        }
        for v in &self.shop_entries {
            v.write(pkt);
        }
    }
}

/// SMSG_BATTLE_PAY_GET_PURCHASE_LIST_RESPONSE 0x2776 (client ctor-reader 0x1406b8f90).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayGetPurchaseListResponse {
    pub result: u32,
    pub purchases: Vec<BattlePayPurchase>,
}

impl ServerPacket for BattlePayGetPurchaseListResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayGetPurchaseListResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.result);
        pkt.write_uint32(self.purchases.len() as u32);
        for p in &self.purchases {
            p.write(pkt);
        }
    }
}

/// SMSG_BATTLE_PAY_PURCHASE_UPDATE 0x2786 (client ctor-reader 0x1406b9060).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayPurchaseUpdate {
    pub purchases: Vec<BattlePayPurchase>,
}

impl ServerPacket for BattlePayPurchaseUpdate {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayPurchaseUpdate;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.purchases.len() as u32);
        for p in &self.purchases {
            p.write(pkt);
        }
    }
}

/// SMSG_BATTLE_PAY_GET_DISTRIBUTION_LIST_RESPONSE 0x2777 (client reader 0x1406f8290).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayGetDistributionListResponse {
    pub result: u32,
    /// Max 2047 (11-bit count).
    pub distribution_objects: Vec<BattlePayDistributionObject>,
}

impl ServerPacket for BattlePayGetDistributionListResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayGetDistributionListResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.distribution_objects.len() < (1 << 11));
        pkt.write_uint32(self.result);
        pkt.write_bits(self.distribution_objects.len() as u32, 11);
        pkt.flush_bits();
        for o in &self.distribution_objects {
            o.write(pkt);
        }
    }
}

/// SMSG_BATTLE_PAY_DISTRIBUTION_UPDATE 0x2779 (client ctor 0x1406b8e10 -> reader 0x140708ec0).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDistributionUpdate {
    pub distribution_object: BattlePayDistributionObject,
}

impl ServerPacket for BattlePayDistributionUpdate {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayDistributionUpdate;

    fn write(&self, pkt: &mut WorldPacket) {
        self.distribution_object.write(pkt);
    }
}

/// SMSG_BATTLE_PAY_DISTRIBUTION_UNREVOKED 0x2778 (client ctor-reader 0x1406b8d70).
///
/// Wire verified (u32, packed guid, u32); field meanings inferred.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDistributionUnrevoked {
    pub unk1: u32,
    pub character_guid: ObjectGuid,
    pub unk2: u32,
}

impl ServerPacket for BattlePayDistributionUnrevoked {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayDistributionUnrevoked;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.unk1);
        pkt.write_packed_guid(&self.character_guid);
        pkt.write_uint32(self.unk2);
    }
}

/// SMSG_BATTLE_PAY_DELIVERY_STARTED 0x277a.
///
/// In-situ POD in the client whose handler is a no-op stub (ignored). Layout from
/// WoD/7.3.5 (u64 DistributionID); not verifiable in the client.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDeliveryStarted {
    pub distribution_id: u64,
}

impl ServerPacket for BattlePayDeliveryStarted {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayDeliveryStarted;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint64(self.distribution_id);
    }
}

/// SMSG_BATTLE_PAY_DELIVERY_ENDED 0x277b (client reader 0x1406f8110).
///
/// Wire: u64 DistributionID; u32 ItemCount; ItemInstance[] (JamItemInstance, stride 0x78).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDeliveryEnded {
    pub distribution_id: u64,
    pub items: Vec<ItemInstance>,
}

impl ServerPacket for BattlePayDeliveryEnded {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayDeliveryEnded;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint64(self.distribution_id);
        pkt.write_uint32(self.items.len() as u32);
        for item in &self.items {
            item.write(pkt);
        }
    }
}

/// SMSG_BATTLE_PAY_MOUNT_DELIVERED 0x277c (in-situ; handler 0x141a49ec0 reads one u32,
/// looks the product up and fires STORE_REFRESH).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayMountDelivered {
    pub product_id: u32,
}

impl ServerPacket for BattlePayMountDelivered {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayMountDelivered;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.product_id);
    }
}

/// SMSG_BATTLE_PAY_BATTLE_PET_DELIVERED 0x277d (client ctor-reader 0x1406b8b50).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayBattlePetDelivered {
    pub display_id: u32,
    pub battle_pet_guid: ObjectGuid,
}

impl ServerPacket for BattlePayBattlePetDelivered {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayBattlePetDelivered;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.display_id);
        pkt.write_packed_guid(&self.battle_pet_guid);
    }
}

/// SMSG_BATTLE_PAY_COLLECTION_ITEM_DELIVERED 0x277e.
///
/// In-situ POD whose client handler is a no-op stub (ignored); u32 payload inferred.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayCollectionItemDelivered {
    pub unk: u32,
}

impl ServerPacket for BattlePayCollectionItemDelivered {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayCollectionItemDelivered;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.unk);
    }
}

/// SMSG_BATTLE_PAY_START_PURCHASE_RESPONSE 0x2783 (in-situ; handler 0x141a4b4e0).
///
/// Handler: ignores the packet unless ClientToken (@12) equals the pending token;
/// PurchaseID (@0) != 0 -> purchase accepted, else PurchaseResult (@8) is mapped to a
/// store error and STORE_ORDER_INITIATION_FAILED fires.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayStartPurchaseResponse {
    pub purchase_id: u64,
    pub purchase_result: u32,
    pub client_token: u32,
}

impl ServerPacket for BattlePayStartPurchaseResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayStartPurchaseResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint64(self.purchase_id);
        pkt.write_uint32(self.purchase_result);
        pkt.write_uint32(self.client_token);
    }
}

/// SMSG_BATTLE_PAY_START_DISTRIBUTION_ASSIGN_TO_TARGET_RESPONSE 0x2784 (in-situ;
/// handler 0x141a49fd0 only checks the u32 at offset 8: non-zero = error).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayStartDistributionAssignToTargetResponse {
    pub distribution_id: u64,
    pub result: u32,
    pub unk: u32,
}

impl ServerPacket for BattlePayStartDistributionAssignToTargetResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayStartDistributionAssignToTargetResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint64(self.distribution_id);
        pkt.write_uint32(self.result);
        pkt.write_uint32(self.unk);
    }
}

/// SMSG_BATTLE_PAY_CONFIRM_PURCHASE 0x2787 (in-situ; handler 0x141a4b1a0 reads
/// PurchaseID @0 and ServerToken @8, then fires STORE_CONFIRM_PURCHASE).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayConfirmPurchase {
    pub purchase_id: u64,
    pub server_token: u32,
}

impl ServerPacket for BattlePayConfirmPurchase {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayConfirmPurchase;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint64(self.purchase_id);
        pkt.write_uint32(self.server_token);
    }
}

/// SMSG_BATTLE_PAY_ACK_FAILED 0x2788 (in-situ; handler 0x141a4b130 reads ServerToken @8,
/// Status @12, Result @16; Result 60/63 take a special branch, otherwise
/// STORE_PURCHASE_ERROR fires and the UI acks with CMSG_BATTLE_PAY_ACK_FAILED_RESPONSE).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayAckFailed {
    pub purchase_id: u64,
    pub server_token: u32,
    pub status: u32,
    pub result: u32,
}

impl ServerPacket for BattlePayAckFailed {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayAckFailed;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint64(self.purchase_id);
        pkt.write_uint32(self.server_token);
        pkt.write_uint32(self.status);
        pkt.write_uint32(self.result);
    }
}

/// SMSG_BATTLE_PAY_VALIDATE_PURCHASE_RESPONSE 0x2818 (client ctor-reader 0x1406b91a0;
/// its handler slot is a no-op stub, i.e. the client ignores the content).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayValidatePurchaseResponse {
    pub unk1: u32,
    pub unk2: u32,
    pub unk3: u64,
    pub unk4: u64,
    pub unk_bit: bool,
}

impl ServerPacket for BattlePayValidatePurchaseResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayValidatePurchaseResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.unk1);
        pkt.write_uint32(self.unk2);
        pkt.write_uint64(self.unk3);
        pkt.write_uint64(self.unk4);
        pkt.write_bit(self.unk_bit);
        pkt.flush_bits();
    }
}

/// SMSG_GENERATE_SSO_TOKEN_RESPONSE 0x281e (client ctor-reader 0x1406c0900, handler 0x14148bd60).
///
/// `kind` must echo the u32 the client sent in CMSG_BATTLE_PAY_OPEN_CHECKOUT (the
/// handler uses it as the key of its pending-request map). `result == 0` delivers the
/// token; the checkout window then loads `<login/sso URL>` + `[?|&]token=<token>`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GenerateSsoTokenResponse {
    pub kind: u32,
    pub result: u32,
    /// Two u64 handed to the callback together with the token (meaning unknown).
    pub unk1: u64,
    pub unk2: u64,
    /// Max 127 bytes (7-bit length).
    pub token: String,
}

impl ServerPacket for GenerateSsoTokenResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::GenerateSsoTokenResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.token.len() < (1 << 7));
        pkt.write_uint32(self.kind);
        pkt.write_uint32(self.result);
        pkt.write_uint64(self.unk1);
        pkt.write_uint64(self.unk2);
        pkt.write_bits(self.token.len() as u32, 7);
        pkt.flush_bits();
        write_str_body(pkt, &self.token);
    }
}

/// SMSG_BATTLE_PAY_START_CHECKOUT 0x2824 (read inline in the dispatcher case 0x1407339e2,
/// handler 0x141a49fa0).
///
/// Wire: u32 ProductID; u32 GameServiceRegionID; u64 GameAccountID; bits(6) Len1;
/// bits(7) Len2; bit Subscription; flush; Str1; Str2. The two strings feed the
/// checkout `purchaseRequest` JSON (keys serverValidationSignature /
/// externalTransactionId); their order follows 7.3.5 and is inferred.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayStartCheckout {
    pub product_id: u32,
    pub game_service_region_id: u32,
    pub game_account_id: u64,
    /// 6-bit length (max 63).
    pub server_validation_signature: String,
    /// 7-bit length; client buffer holds 88 bytes.
    pub external_transaction_id: String,
    pub subscription: bool,
}

impl ServerPacket for BattlePayStartCheckout {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayStartCheckout;

    fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.server_validation_signature.len() < (1 << 6));
        debug_assert!(self.external_transaction_id.len() < 89);
        pkt.write_uint32(self.product_id);
        pkt.write_uint32(self.game_service_region_id);
        pkt.write_uint64(self.game_account_id);
        pkt.write_bits(self.server_validation_signature.len() as u32, 6);
        pkt.write_bits(self.external_transaction_id.len() as u32, 7);
        pkt.write_bit(self.subscription);
        pkt.flush_bits();
        write_str_body(pkt, &self.server_validation_signature);
        write_str_body(pkt, &self.external_transaction_id);
    }
}

/// SMSG_BATTLE_PAY_DISTRIBUTION_ASSIGN_VAS_RESPONSE 0x2888 (in-situ; ctor 0x1406b8d10
/// pre-zeroes a {u64, u32} default before mapping the payload; handler is a virtual
/// listener, layout inferred).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDistributionAssignVasResponse {
    pub distribution_id: u64,
    pub result: u32,
}

impl ServerPacket for BattlePayDistributionAssignVasResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::BattlePayDistributionAssignVasResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint64(self.distribution_id);
        pkt.write_uint32(self.result);
    }
}

/// SMSG_DISPLAY_PROMOTION 0x264c (in-situ; handler 0x1416a7920 reads one u32, -1 special).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DisplayPromotion {
    pub promotion_id: u32,
}

impl ServerPacket for DisplayPromotion {
    const OPCODE: ServerOpcodes = ServerOpcodes::DisplayPromotion;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.promotion_id);
    }
}

/// SMSG_SYNC_WOW_ENTITLEMENTS 0x286b (client ctor-reader 0x1406cbe30).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncWowEntitlements {
    pub entitlements: Vec<WowEntitlement>,
    pub products: Vec<BattlePayProduct>,
}

impl ServerPacket for SyncWowEntitlements {
    const OPCODE: ServerOpcodes = ServerOpcodes::SyncWowEntitlements;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.entitlements.len() as u32);
        pkt.write_uint32(self.products.len() as u32);
        for e in &self.entitlements {
            e.write(pkt);
        }
        for p in &self.products {
            p.write(pkt);
        }
    }
}

/// SMSG_WOW_ENTITLEMENT_NOTIFICATION 0x286c (client ctor-reader 0x1406ce1f0).
///
/// Wire: bits(3) Unk; flush; WowEntitlement; BattlePayProduct.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WowEntitlementNotification {
    /// 3-bit value.
    pub unk: u8,
    pub entitlement: WowEntitlement,
    pub product: BattlePayProduct,
}

impl ServerPacket for WowEntitlementNotification {
    const OPCODE: ServerOpcodes = ServerOpcodes::WowEntitlementNotification;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_bits(u32::from(self.unk), 3);
        pkt.flush_bits();
        self.entitlement.write(pkt);
        self.product.write(pkt);
    }
}

/// SMSG_CHARACTER_UPGRADE_STARTED 0x27bf (client ctor-reader 0x1406bb6a0).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CharacterUpgradeStarted {
    pub character_guid: ObjectGuid,
}

impl ServerPacket for CharacterUpgradeStarted {
    const OPCODE: ServerOpcodes = ServerOpcodes::CharacterUpgradeStarted;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_packed_guid(&self.character_guid);
    }
}

/// SMSG_CHARACTER_UPGRADE_ABORTED 0x27c1 (same ctor-reader as Started).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CharacterUpgradeAborted {
    pub character_guid: ObjectGuid,
}

impl ServerPacket for CharacterUpgradeAborted {
    const OPCODE: ServerOpcodes = ServerOpcodes::CharacterUpgradeAborted;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_packed_guid(&self.character_guid);
    }
}

/// SMSG_CHARACTER_UPGRADE_COMPLETE 0x27c0 (client ctor-reader 0x1406bb6e0).
///
/// Wire: packed guid; u32 Count; u32 Values[]; bit Unk; flush.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CharacterUpgradeComplete {
    pub character_guid: ObjectGuid,
    pub values: Vec<u32>,
    pub unk_bit: bool,
}

impl ServerPacket for CharacterUpgradeComplete {
    const OPCODE: ServerOpcodes = ServerOpcodes::CharacterUpgradeComplete;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_packed_guid(&self.character_guid);
        pkt.write_uint32(self.values.len() as u32);
        for v in &self.values {
            pkt.write_uint32(*v);
        }
        pkt.write_bit(self.unk_bit);
        pkt.flush_bits();
    }
}

/// SMSG_CHARACTER_UPGRADE_MANUAL_UNREVOKE_RESULT 0x27c3 (in-situ; handler 0x14169c0d0
/// forwards the u32 to CHARACTER_UPGRADE_UNREVOKE_RESULT(result)).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CharacterUpgradeManualUnrevokeResult {
    pub result: u32,
}

impl ServerPacket for CharacterUpgradeManualUnrevokeResult {
    const OPCODE: ServerOpcodes = ServerOpcodes::CharacterUpgradeManualUnrevokeResult;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.result);
    }
}

/// SMSG_VAS_PURCHASE_STATE_UPDATE 0x27f3 (client ctor-reader 0x1406cd7b0).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VasPurchaseStateUpdate {
    pub unk: u32,
    pub purchase: VasPurchase,
}

impl ServerPacket for VasPurchaseStateUpdate {
    const OPCODE: ServerOpcodes = ServerOpcodes::VasPurchaseStateUpdate;

    fn write(&self, pkt: &mut WorldPacket) {
        pkt.write_uint32(self.unk);
        self.purchase.write(pkt);
    }
}

/// SMSG_VAS_PURCHASE_COMPLETE 0x27f4 (client ctor-reader 0x1406cd660).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VasPurchaseComplete {
    pub product_id: u32,
    pub result: u32,
    pub character_guid: ObjectGuid,
    pub glue_guid: ObjectGuid,
    pub unk: u32,
    pub handled_guid: ObjectGuid,
    /// 6-bit length.
    pub character_name: String,
}

impl ServerPacket for VasPurchaseComplete {
    const OPCODE: ServerOpcodes = ServerOpcodes::VasPurchaseComplete;

    fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.character_name.len() < (1 << 6));
        pkt.write_uint32(self.product_id);
        pkt.write_uint32(self.result);
        pkt.write_packed_guid(&self.character_guid);
        pkt.write_packed_guid(&self.glue_guid);
        pkt.write_uint32(self.unk);
        pkt.write_packed_guid(&self.handled_guid);
        pkt.write_bits(self.character_name.len() as u32, 6);
        pkt.flush_bits();
        write_str_body(pkt, &self.character_name);
    }
}

/// SMSG_ENUM_VAS_PURCHASE_STATES_RESPONSE 0x27f5 (client ctor-reader 0x1406bf210).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnumVasPurchaseStatesResponse {
    /// Max 63 (6-bit count).
    pub purchases: Vec<VasPurchase>,
}

impl ServerPacket for EnumVasPurchaseStatesResponse {
    const OPCODE: ServerOpcodes = ServerOpcodes::EnumVasPurchaseStatesResponse;

    fn write(&self, pkt: &mut WorldPacket) {
        debug_assert!(self.purchases.len() < (1 << 6));
        pkt.write_bits(self.purchases.len() as u32, 6);
        pkt.flush_bits();
        for p in &self.purchases {
            p.write(pkt);
        }
    }
}

// ── Client packets ────────────────────────────────────────────────────

/// CMSG_BATTLE_PAY_GET_PRODUCT_LIST 0x36c4 (client Write 0x1407647f0: opcode only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BattlePayGetProductList;

impl ClientPacket for BattlePayGetProductList {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayGetProductList;

    fn read(_pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self)
    }
}

/// CMSG_BATTLE_PAY_GET_PURCHASE_LIST 0x36c5 (client Write 0x140764810: opcode only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BattlePayGetPurchaseList;

impl ClientPacket for BattlePayGetPurchaseList {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayGetPurchaseList;

    fn read(_pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self)
    }
}

/// CMSG_UPDATE_VAS_PURCHASE_STATES 0x36fb (client Write 0x140766e40: opcode only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpdateVasPurchaseStates;

impl ClientPacket for UpdateVasPurchaseStates {
    const OPCODE: ClientOpcodes = ClientOpcodes::UpdateVasPurchaseStates;

    fn read(_pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self)
    }
}

/// CMSG_BATTLE_PAY_START_PURCHASE 0x36d3 (client Write 0x140764830).
///
/// Wire: u32 ClientToken; u32 ProductID; packed guid TargetCharacter; bits(6) Len1;
/// bits(12) Len2; bits(7) Len3; flush; Str1; Str2; Str3. String meanings inferred from
/// 7.3.5 CMSG_BATTLE_PAY_PURCHASE_PRODUCT (WowSytem 6 bits, PublicKey 12 bits); the
/// third string is new.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayStartPurchase {
    pub client_token: u32,
    pub product_id: u32,
    pub target_character: ObjectGuid,
    pub wow_system: String,
    pub public_key: String,
    pub unk_string: String,
}

impl ClientPacket for BattlePayStartPurchase {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayStartPurchase;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        let client_token = pkt.read_uint32()?;
        let product_id = pkt.read_uint32()?;
        let target_character = pkt.read_packed_guid()?;
        let len1 = pkt.read_bits(6)? as usize;
        let len2 = pkt.read_bits(12)? as usize;
        let len3 = pkt.read_bits(7)? as usize;
        let wow_system = pkt.read_string(len1)?;
        let public_key = pkt.read_string(len2)?;
        let unk_string = pkt.read_string(len3)?;
        Ok(Self {
            client_token,
            product_id,
            target_character,
            wow_system,
            public_key,
            unk_string,
        })
    }
}

/// CMSG_BATTLE_PAY_CONFIRM_PURCHASE_RESPONSE 0x36d4 (client Write 0x1407646e0).
///
/// Wire: bit ConfirmPurchase; flush; u32 ServerToken; u64 ClientCurrentPriceFixedPoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BattlePayConfirmPurchaseResponse {
    pub confirm_purchase: bool,
    pub server_token: u32,
    pub client_current_price_fixed_point: u64,
}

impl ClientPacket for BattlePayConfirmPurchaseResponse {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayConfirmPurchaseResponse;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        let confirm_purchase = pkt.read_bit()?;
        let server_token = pkt.read_uint32()?;
        let client_current_price_fixed_point = pkt.read_uint64()?;
        Ok(Self {
            confirm_purchase,
            server_token,
            client_current_price_fixed_point,
        })
    }
}

/// CMSG_BATTLE_PAY_ACK_FAILED_RESPONSE 0x36d5 (client Write 0x1407646a0: u32 ServerToken).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BattlePayAckFailedResponse {
    pub server_token: u32,
}

impl ClientPacket for BattlePayAckFailedResponse {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayAckFailedResponse;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self {
            server_token: pkt.read_uint32()?,
        })
    }
}

/// CMSG_BATTLE_PAY_DISTRIBUTION_ASSIGN_TO_TARGET 0x36cb (client Write 0x140764750).
///
/// Wire: u32 ClientToken; u64 DistributionID; packed guid TargetCharacter; u32 ProductChoice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BattlePayDistributionAssignToTarget {
    pub client_token: u32,
    pub distribution_id: u64,
    pub target_character: ObjectGuid,
    pub product_choice: u32,
}

impl ClientPacket for BattlePayDistributionAssignToTarget {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayDistributionAssignToTarget;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self {
            client_token: pkt.read_uint32()?,
            distribution_id: pkt.read_uint64()?,
            target_character: pkt.read_packed_guid()?,
            product_choice: pkt.read_uint32()?,
        })
    }
}

/// CMSG_BATTLE_PAY_OPEN_CHECKOUT 0x3714 (client Write 0x140766f30: one u32).
///
/// Sent by the browser controller (0x14148bf70) right after the client processed
/// SMSG_BATTLE_PAY_START_CHECKOUT: the u32 is a fresh client request id which the
/// server must echo as `kind` in SMSG_GENERATE_SSO_TOKEN_RESPONSE. This is the
/// 3.4.3 replacement of 7.3.5's CMSG_GENERATE_SSO_TOKEN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BattlePayOpenCheckout {
    pub request_id: u32,
}

impl ClientPacket for BattlePayOpenCheckout {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayOpenCheckout;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self {
            request_id: pkt.read_uint32()?,
        })
    }
}

/// CMSG_BATTLE_PAY_CANCEL_OPEN_CHECKOUT 0x371b (client Write 0x140765820).
///
/// Wire: bits(6) Len; bit Flag; flush; String (max 63). String meaning inferred
/// (7.3.5: ExternalTransactionID echo).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayCancelOpenCheckout {
    pub external_transaction_id: String,
    pub unk_bit: bool,
}

impl ClientPacket for BattlePayCancelOpenCheckout {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayCancelOpenCheckout;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        let len = pkt.read_bits(6)? as usize;
        let unk_bit = pkt.read_bit()?;
        let external_transaction_id = pkt.read_string(len)?;
        Ok(Self {
            external_transaction_id,
            unk_bit,
        })
    }
}

/// CMSG 0x371a (client Write 0x140766fe0; absent from the TrinityCore 3.4.3 opcode table).
///
/// Wire: bits(6) Len1; bits(7) Len2; bit Flag; flush; Str1 (max 40); Str2 (max 89).
/// Sent by the Store module (0x141a4b030). Its 7.3.5 analogue is
/// CMSG_BATTLE_PAY_PURCHASE_SUBMITTED (GlobalOrderID 6 bits, ExternalTransactionID 7 bits).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayPurchaseSubmitted {
    pub global_order_id: String,
    pub external_transaction_id: String,
    pub unk_bit: bool,
}

impl ClientPacket for BattlePayPurchaseSubmitted {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayPurchaseSubmitted;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        let len1 = pkt.read_bits(6)? as usize;
        let len2 = pkt.read_bits(7)? as usize;
        let unk_bit = pkt.read_bit()?;
        let global_order_id = pkt.read_string(len1)?;
        let external_transaction_id = pkt.read_string(len2)?;
        Ok(Self {
            global_order_id,
            external_transaction_id,
            unk_bit,
        })
    }
}

/// CMSG_BATTLE_PAY_REQUEST_PRICE_INFO 0x3710 (client Write 0x1407649f0: two u32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BattlePayRequestPriceInfo {
    pub client_token: u32,
    pub product_id: u32,
}

impl ClientPacket for BattlePayRequestPriceInfo {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayRequestPriceInfo;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self {
            client_token: pkt.read_uint32()?,
            product_id: pkt.read_uint32()?,
        })
    }
}

/// CMSG_BATTLE_PAY_START_VAS_PURCHASE 0x36fa (client Write body 0x140756cb0).
///
/// Wire: u32; u32; packed guid; u32; u32; packed guid; packed guid; packed guid;
/// bits(6) Len1; bits(7) Len2; bits(7) Len3; bits(6) Len4; bits(12) Len5; bit Flag;
/// flush; five strings. Names follow 7.3.5 where the position matches; the third
/// guid, two of the strings and the flag are new and unnamed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayStartVasPurchase {
    pub client_token: u32,
    pub product_id: u32,
    pub character_guid: ObjectGuid,
    pub current_realm_address: u32,
    pub destination_realm_address: u32,
    pub wow_account_guid: ObjectGuid,
    pub bnet_account_guid: ObjectGuid,
    pub unk_guid: ObjectGuid,
    pub new_character_name: String,
    pub unk_string1: String,
    pub unk_string2: String,
    pub platform: String,
    pub client_info: String,
    pub unk_bit: bool,
}

impl ClientPacket for BattlePayStartVasPurchase {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayStartVasPurchase;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        let client_token = pkt.read_uint32()?;
        let product_id = pkt.read_uint32()?;
        let character_guid = pkt.read_packed_guid()?;
        let current_realm_address = pkt.read_uint32()?;
        let destination_realm_address = pkt.read_uint32()?;
        let wow_account_guid = pkt.read_packed_guid()?;
        let bnet_account_guid = pkt.read_packed_guid()?;
        let unk_guid = pkt.read_packed_guid()?;
        let len1 = pkt.read_bits(6)? as usize;
        let len2 = pkt.read_bits(7)? as usize;
        let len3 = pkt.read_bits(7)? as usize;
        let len4 = pkt.read_bits(6)? as usize;
        let len5 = pkt.read_bits(12)? as usize;
        let unk_bit = pkt.read_bit()?;
        let new_character_name = pkt.read_string(len1)?;
        let unk_string1 = pkt.read_string(len2)?;
        let unk_string2 = pkt.read_string(len3)?;
        let platform = pkt.read_string(len4)?;
        let client_info = pkt.read_string(len5)?;
        Ok(Self {
            client_token,
            product_id,
            character_guid,
            current_realm_address,
            destination_realm_address,
            wow_account_guid,
            bnet_account_guid,
            unk_guid,
            new_character_name,
            unk_string1,
            unk_string2,
            platform,
            client_info,
            unk_bit,
        })
    }
}

/// CMSG_BATTLE_PAY_DISTRIBUTION_ASSIGN_VAS 0x3742 (client Write body 0x1407533f0).
///
/// Wire: u32; u64 DistributionID; packed guid; u32; u32; packed guid; packed guid;
/// packed guid; u8; u8; u8; u32 PairCount; {u32,u32}[]; bits(6) Len1; bits(7) Len2;
/// bits(7) Len3; bit; bit; bit; flush; three strings. Field meanings unknown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BattlePayDistributionAssignVas {
    pub unk1: u32,
    pub distribution_id: u64,
    pub target_character: ObjectGuid,
    pub unk2: u32,
    pub unk3: u32,
    pub unk_guid1: ObjectGuid,
    pub unk_guid2: ObjectGuid,
    pub unk_guid3: ObjectGuid,
    pub unk_byte1: u8,
    pub unk_byte2: u8,
    pub unk_byte3: u8,
    pub pairs: Vec<(u32, u32)>,
    pub unk_string1: String,
    pub unk_string2: String,
    pub unk_string3: String,
    pub unk_bit1: bool,
    pub unk_bit2: bool,
    pub unk_bit3: bool,
}

impl ClientPacket for BattlePayDistributionAssignVas {
    const OPCODE: ClientOpcodes = ClientOpcodes::BattlePayDistributionAssignVas;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        let unk1 = pkt.read_uint32()?;
        let distribution_id = pkt.read_uint64()?;
        let target_character = pkt.read_packed_guid()?;
        let unk2 = pkt.read_uint32()?;
        let unk3 = pkt.read_uint32()?;
        let unk_guid1 = pkt.read_packed_guid()?;
        let unk_guid2 = pkt.read_packed_guid()?;
        let unk_guid3 = pkt.read_packed_guid()?;
        let unk_byte1 = pkt.read_uint8()?;
        let unk_byte2 = pkt.read_uint8()?;
        let unk_byte3 = pkt.read_uint8()?;
        let count = pkt.read_uint32()? as usize;
        check_capacity(count)?;
        let mut pairs = Vec::with_capacity(count);
        for _ in 0..count {
            pairs.push((pkt.read_uint32()?, pkt.read_uint32()?));
        }
        let len1 = pkt.read_bits(6)? as usize;
        let len2 = pkt.read_bits(7)? as usize;
        let len3 = pkt.read_bits(7)? as usize;
        let unk_bit1 = pkt.read_bit()?;
        let unk_bit2 = pkt.read_bit()?;
        let unk_bit3 = pkt.read_bit()?;
        let unk_string1 = pkt.read_string(len1)?;
        let unk_string2 = pkt.read_string(len2)?;
        let unk_string3 = pkt.read_string(len3)?;
        Ok(Self {
            unk1,
            distribution_id,
            target_character,
            unk2,
            unk3,
            unk_guid1,
            unk_guid2,
            unk_guid3,
            unk_byte1,
            unk_byte2,
            unk_byte3,
            pairs,
            unk_string1,
            unk_string2,
            unk_string3,
            unk_bit1,
            unk_bit2,
            unk_bit3,
        })
    }
}

/// CMSG_CHARACTER_UPGRADE_START 0x36cd (client Write 0x140769d10: packed guid; u32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CharacterUpgradeStart {
    pub character_guid: ObjectGuid,
    pub unk: u32,
}

impl ClientPacket for CharacterUpgradeStart {
    const OPCODE: ClientOpcodes = ClientOpcodes::CharacterUpgradeStart;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self {
            character_guid: pkt.read_packed_guid()?,
            unk: pkt.read_uint32()?,
        })
    }
}

/// CMSG_CHARACTER_UPGRADE_MANUAL_UNREVOKE_REQUEST 0x36cc (client Write 0x140765c70: packed guid).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CharacterUpgradeManualUnrevokeRequest {
    pub character_guid: ObjectGuid,
}

impl ClientPacket for CharacterUpgradeManualUnrevokeRequest {
    const OPCODE: ClientOpcodes = ClientOpcodes::CharacterUpgradeManualUnrevokeRequest;

    fn read(pkt: &mut WorldPacket) -> Result<Self, PacketError> {
        Ok(Self {
            character_guid: pkt.read_packed_guid()?,
        })
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
}
