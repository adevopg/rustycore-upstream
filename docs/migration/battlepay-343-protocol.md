# BattlePay (in-game shop) protocol for client 3.4.3.54261

Status: protocol specification and evidence record for the `battlepay` branch.
Rust codec: `crates/wow-packet/src/packets/battlepay.rs`. Written 2026-09-25.

## 1. Evidence sources and how they rank

| # | Source | Role for 54261 |
|---|--------|----------------|
| A | Client `WowClassic.exe` 3.4.3.54261, decrypted image read from the running Wine process (`/proc/<pid>/mem`, image base `0x140000000`, mapping `140000000-142cb0000 r-xp /memfd:wine-mapping`, `.rdata` at `0x14260d000`, `.data` at `0x142cb8000`). `.pdata` (function bounds) taken from the on-disk file (unencrypted). Disassembly with capstone 4.0.2. | **Primary.** Every layout below cites the JAM reader/writer address. |
| B | WowPacketParser (TrinityCore/WowPacketParser master, fetched 2026-09-25) | **No parser for this build.** `WowPacketParserModule.V3_4_0_45166/Parsers/BattlePayHandler.cs` declares all BattlePay opcodes only for `ClientVersionBuild.V3_4_4_59817+` and the body is `packet.ReadToEnd()` ("No BattlePay support, do not ask"). Build 54261 maps to defining build `V3_4_0_45166` (`ClientVersion.cs` `GetVersionDefiningBuild`, case `V3_4_3_54261`), whose fallback chain (`FallbackVersionDefiningBuild`) is `V3_4_0_45166 -> V2_5_1_38707 -> V9_0_1_36216 -> V1_13_2_31446 (classic origin) -> V8_0_1_27101 -> V7_0_3_22248 -> V6_0_2_19033`. `Handler.cs` walks that chain and takes the first module declaring the opcode: for every BattlePay SMSG this is `V7_0_3_22248/Parsers/BattlePayHandler.cs`, again `ReadToEnd()`. Only a few CMSG (`CMSG_BATTLE_PAY_START_PURCHASE`, `..._CONFIRM_PURCHASE_RESPONSE`, `..._DISTRIBUTION_ASSIGN_TO_TARGET`, `..._GET_PRODUCT_LIST`, `..._GET_PURCHASE_LIST`) resolve to the real WoD parsers in `V6_0_2_19033/Parsers/BattlePayHandler.cs`. Those WoD layouts are cited as lineage, never as authority. |
| C | TrinityCore 3.4.3 opcode table (via WPP `Enums/Version/V3_4_3_51666/Opcodes.cs`, identical numbers to RustyCore `crates/wow-constants/src/opcodes/{client,server}.rs`) | Opcode numbers only; TrinityCore 3.4.3 has no BattlePay handlers. |
| D | LegionCore 7.3.5 (`/home/inna/legioncore-ref/src/server/game/Server/Packets/BattlePayPackets.{h,cpp}`, `Handlers/BattlePayHandler.cpp`, `tools/bnet-shop/README.md`, `tools/re-opcodes/*`) | Comparison baseline ("Reader in Wow-64.exe 7.3.5.26972"); field names for fields whose position is unchanged. |
| E | Client UI Lua/XML extracted from the CASC install (`Interface_TBC/AddOns/Blizzard_StoreUI/*`, `Interface/AddOns/Blizzard_StoreUI/Blizzard_SimpleCheckout.lua`, ...; dumped to the session scratch dir `bp/ui/`, not committed) | Semantics of fields/enums/events, checkout frame flow. Section 5. |

Confidence levels used below:

- **verified-in-client**: transcribed from the 54261 JAM reader (SMSG) or writer (CMSG); names may still be inferred (flagged).
- **handler-verified**: SMSG mapped in-situ (raw bytes, see 2.1); the field offsets come from the client handler that consumes them.
- **inferred**: layout or name taken from 7.3.5/WoD because the client does not read the field or its meaning is not observable.

### 1.1 Client RE method (reproducible)

1. Dump `0x140000000..0x1447e0000` from `/proc/<pid>/mem` (75 MB). `.text` is plaintext from `0x1400d0000` on; `0x140001000-0x1400c0000` stays high-entropy in memory (loader/anti-tamper area, irrelevant here).
2. CDataStore helpers: `GetInt8 0x1402c9990`, `GetInt16 0x1402c99e0`, `GetInt32 0x1402c9a30`, `GetInt64 0x1402c9a80`, `GetBytes(ptr,n) 0x1402c9fe0`, `GetDataInSitu 0x1402c9f90`, packed-GUID reader `0x141550b20`; bit reader struct `{ds, curbyte, bitpos}` with `ReadBits6 0x140720900`, `ReadBits7 0x1407209c0`, `ReadBits24 0x1406aaec0`; writers `PutOpcode(u16) 0x1402ca3d0`, `PutInt8 0x1402ca320`, `PutInt32 0x1402ca480`, `PutInt64 0x1402ca690`, `PutFloat 0x1402ca530`, `PutGuid128 0x141550bf0`, `PutData 0x1402cab90`, `WriteBits{3,4,5,6,7} 0x140758a20/ae0/ba0/c60/d20`, `FlushBits 0x1406f5d20`. Bit streams are MSB-first exactly like TrinityCore `ByteBuffer::WriteBits`.
3. SMSG dispatch: four jump-table switches; the shop opcodes are in the switch at `0x140727dc2` (`add eax,-0x256c; cmp eax,0x340; table RVA 0x73651c`, i.e. opcodes `0x256c..0x28ac`). Each case does `ctor_reader(storage, conn, ..., ds)` then calls a handler slot (`mov rax,[rip+slot]; call rax`). Case addresses used: `0x2775->0x140731082`, `0x2776->0x1407310c4`, `0x2777->0x140731106`, `0x2778->0x14073113f`, `0x2779->0x140731179`, `0x277a->0x1407311b2`, `0x277b->0x1407311ec`, `0x277c->0x14073122e`, `0x277d->0x140731268`, `0x277e->0x1407312a2`, `0x2783->0x1407313d4`, `0x2784->0x14073140e`, `0x2786->0x140731482`, `0x2787->0x1407314c4`, `0x2788->0x1407314fe`, `0x2818->0x1407336ce`, `0x281e->0x14073383a`, `0x2824->0x1407339e2`, `0x2888->0x140735c25`, `0x264c->0x14072cc58`, `0x286b->0x140735523`, `0x286c->0x140735565`, `0x27bf->0x1407321a4`, `0x27c0->0x1407321de`, `0x27c1->0x140732220`, `0x27c3->0x140732294`, `0x27f3->0x140732dec`, `0x27f4->0x140732e2e`, `0x27f5->0x140732e68`.
4. Handler slots are written at start-up by registrar thunks (`lea rax,[fn]; mov [slot],rax`); the handler addresses below come from those writers (the slots are zero in the dump because the client sat at the login screen).
5. CMSG: every outgoing JAM class has a 4-slot vtable `[dtor, SerializeBody, Write, GetInfo]` (vtables ICF-folded in `.rdata 0x1426d7c20..0x1426e0b00`); `Write` emits `PutOpcode(imm16)` first. All 699 `PutOpcode` call sites were enumerated: 624 carry an immediate opcode, all of which exist in RustyCore's `ClientOpcodes` except `0x31ba`, `0x371a` (section 3.13) and `0x3a63`; there is **no `CMSG_GENERATE_SSO_TOKEN`-like single-byte message** in this client (section 4).

### 1.2 In-situ messages

Several small SMSG are not parsed field by field: the ctor (`0x1406b55f0`, `0x1406b8bb0`, `0x1406b86a0`, `0x1406b8d10`) calls `GetDataInSitu(ds, size - pos)` and stores a pointer to the raw remaining bytes; the handler then reads fixed offsets. For those the layout is "whatever the handler reads" (handler-verified) plus 7.3.5 for unread trailing fields.

## 2. Server -> client packets

Notation: `u8/u16/u32/u64` little-endian, `guid` = TrinityCore packed 128-bit GUID (`write_packed_guid`), `bits(n)` = n-bit big-endian bit field, `flush` = byte-align, `str(L)` = L raw bytes (no terminator) whose length was sent as a bit field earlier. Struct offsets in brackets are the client object offsets (useful to re-check the disassembly).

### 2.1 Shared structures

#### DisplayInfo (`JamBattlepayDisplayCard`, reader `0x140709370`) — verified-in-client

```
bits: HasCreatureDisplayID(1) HasFileDataID(1) Name1Len(10) Name2Len(10) Name3Len(13)
      Name4Len(13) Name5Len(13) HasFlags(1) HasUnk1(1) HasUnk2(1) HasUnk3(1)
      Name6Len(13) Name7Len(12)                      -> flush (exactly 12 bytes)
u32 VisualCount   [+0x53f4 area]      u32 Unk4 [+0x3450]   u32 Unk5 [+0x53f8]   u32 Unk6 [+0x53fc]
[u32 CreatureDisplayID +0x0]  [u32 FileDataID +0x8]
str Name1 [+0x10] str Name2 [+0x211] str Name3 [+0x412] str Name4 [+0x1413] str Name5 [+0x2414]
[u32 Flags +0x3418] [u32 Unk1 +0x3420] [u32 Unk2 +0x3428] [u32 Unk3 +0x3430]
str Name6 [+0x3454] str Name7 [+0x4455]
Visual[VisualCount]: bits NameLen(10) flush; u32 DisplayId; u32 VisualId; u32 Unk; str Name  (stride 0x210)
```

Bit decode proof (byte b1..b12 read with `GetInt8`): `Name1 = (b1&0x3f)<<4 | b2>>4`, `Name2 = (b2&0xf)<<6 | b3>>2`, `Name3 = (b3&3)<<11 | b4<<3 | b5>>5`, `Name4 = (b5&0x1f)<<8 | b6`, `Name5 = b7<<5 | b8>>3`, flags bits = `b8 bit2, bit1, bit0`, `HasUnk3 = b9 bit7`, `Name6 = (b9&0x7f)<<6 | b10>>2`, `Name7 = (b10&3)<<10 | b11<<2 | b12>>6`.
Diff vs 7.3.5 (`BattlePayPackets.cpp operator<<(ProductDisplayInfo)`): +Name5/Name6/Name7, +Unk4/Unk5/Unk6 after VisualCount, +Unk in Visual. Lua mapping (section 5): Name1 is what the UI shows as `sharedData.name`, Name3 as `description` (7.3.5 DB usage); the exact name->Lua-field map for Name2/4/5/6/7 was not proven (open question).

#### ProductItem (inside Product, loop at `0x140708d31`) — verified-in-client

```
u32 ID; u8 UnkByte; u32 ItemID; u32 Quantity; u32 UnkInt1; u32 UnkInt2
bits: HasPet(1) HasPetResult(1) HasDisplayInfo(1) [PetResult(4) if HasPetResult]  flush
[DisplayInfo]
```
Same as 7.3.5.

#### Product (`JamBattlePayProduct`, reader `0x140708b10`) — verified-in-client

```
u32 ProductID; u8 Type; u32 Flags; u32 Unk1; u32 DisplayId; u32 ItemId; u32 Unk4; u32 Unk5;
u32 Unk6; u32 Unk7; u32 Unk8; u32 Unk9
bits: UnkStringLen(8) UnkBit(1) HasUnkBits(1) ItemCount(7) HasDisplayInfo(1) [UnkBits(4)]  flush
ProductItem[ItemCount]
str UnkString            <- written AFTER the items (client GetBytes at 0x140708e77)
[DisplayInfo]
```
Diff vs 7.3.5: +Unk6..Unk9; 7.3.5 never emitted the UnkString bytes (length always 0).

#### ProductInfo (reader `0x140709000`, first array of the product list) — verified-in-client

```
u32 ProductID; u64 NormalPriceFixedPoint; u64 CurrentPriceFixedPoint
u32 ProductIDCount; u32 Unk1 [+0x34]; u32 Unk2 [+0x38]; u32 UnkIntCount; u32 Unk3 [+0x5460]
u32 ProductIDs[ProductIDCount]; u32 UnkInts[UnkIntCount]
bits: ChoiceType(7) HasDisplayInfo(1)  flush
[DisplayInfo]
```
Diff vs 7.3.5: +Unk2, +Unk3. Prices are fixed point: Lua splits into `currentDollars = v/100`, `currentCents = v%100` (section 5).

#### ProductGroup (inline loop `0x1406f8880`) — verified-in-client

```
u32 GroupID; u32 IconFileDataID; u8 DisplayType; u32 Ordering; u32 Flags; u32 Unk [+0x124]
u8 NameLen (i.e. bits(8)); bits(24) DescField   (4 bytes total, DescField MSB first)
str Name
Desc: DescField 0 or 1 -> nothing; DescField >= 2 -> DescField bytes whose LAST byte must be 0
      (client 0x14155f220 checks ptr[len-1]==0; string = DescField-1 chars)
```
Diff vs 7.3.5: +Unk. 7.3.5 wrote `len+1` in 24 bits but no NUL: at 54261 the NUL is required.

#### ShopEntry (inline loop `0x1406f89c0`) — verified-in-client

```
u32 EntryID; u32 GroupID; u32 ProductID; i32 Ordering; u32 VasServiceType; u8 StoreDeliveryType
bit HasDisplayInfo  flush
[DisplayInfo]
```
Identical to 7.3.5.

#### Purchase (`JamBattlePayPurchase`, reader `0x1407091e0`) — verified-in-client

```
u64 PurchaseID; u32 Status; u32 ResultCode; u32 ProductID; u64 Unk1; u64 Unk2; u64 Unk3
bits(8) WalletNameLen flush; str WalletName
```
Diff vs 7.3.5 (`u64 UnkLong, u64 UnkLong2, u32 UnkInt`): third unknown is a u64 now.

#### DistributionObject (`JamBattlePayDistributionObject`, reader `0x140708ec0`) — verified-in-client

```
u64 DistributionID; u32 Status; u32 ProductID; guid TargetPlayer [+0x10]; guid UnkGuid [+0x20]
u32 TargetVirtualRealm; u32 TargetNativeRealm; u64 PurchaseID; u32 Unk [+0x55ac]
bits: HasProduct(1) Revoked(1)  flush
[Product]
```
Diff vs 7.3.5: +UnkGuid after TargetPlayer, +Unk before the bits.

#### VasPurchase (reader `0x14071f6b0`) — verified-in-client

```
guid PlayerGuid; u32 ProductID [+0x10]; u32 State [+0x30]; u64 PurchaseID [+0x38]
bits(2) ErrorCount flush; u32 Errors[ErrorCount]
```
Same as 7.3.5 `VasPurchaseData`.

#### WowEntitlement (`JamWowEntitlement`, reader `0x14071ad50`) — verified-in-client (names unknown)

```
u32 Unk1; u64 Unk2; u64 Unk3; u32 Unk4; bit UnkBit flush
```

### 2.2 Packets

| Opcode | Name (RustyCore) | Layout | Evidence / confidence |
|---|---|---|---|
| 0x2775 | BattlePayGetProductListResponse | `u32 Result; u32 CurrencyID; u32 nProductInfo; u32 nProduct; u32 nGroup; u32 nShopEntry; ProductInfo[]; Product[]; ProductGroup[]; ShopEntry[]` | ctor `0x1406b8f20` -> reader `0x1406f84c0`. verified-in-client. Same top-level order as 7.3.5. Handler slot `0x14309d7c0`. |
| 0x2776 | BattlePayGetPurchaseListResponse | `u32 Result; u32 Count; Purchase[Count]` | ctor-reader `0x1406b8f90` (+loop `0x1406b9020`). verified-in-client. |
| 0x2786 | BattlePayPurchaseUpdate | `u32 Count; Purchase[Count]` | ctor-reader `0x1406b9060`. verified-in-client. |
| 0x2777 | BattlePayGetDistributionListResponse | `u32 Result; bits(11) Count flush; DistributionObject[Count]` | ctor `0x1406b8ed0` -> reader `0x1406f8290` (count = `b1<<3 \| b2>>5`, element loop `0x1406f8490`). verified-in-client. Post-processed by `0x1406f3b00`, handled by `0x1406d9760` (direct call, no slot). |
| 0x2779 | BattlePayDistributionUpdate | `DistributionObject` | ctor `0x1406b8e10` -> `0x140708ec0`. verified-in-client. |
| 0x2778 | BattlePayDistributionUnrevoked | `u32 Unk1; guid CharacterGuid; u32 Unk2` | ctor-reader `0x1406b8d70`. Layout verified; names inferred. Slot `0x14309e000`, handler `0x141a49f00`. |
| 0x277a | BattlePayDeliveryStarted | `u64 DistributionID` | in-situ ctor `0x1406b55f0`; handler slot `0x14309e520` is written with the no-op stub `0x1402112e0` (`ret`), i.e. **the client ignores this packet**. Layout inferred from 7.3.5/WoD. |
| 0x277b | BattlePayDeliveryEnded | `u64 DistributionID; u32 Count; ItemInstance[Count]` | ctor `0x1406b8cb0` -> reader `0x1406f8110` (elements stride 0x78 read by the common `JamItemInstance` reader `0x1407134f0`). verified-in-client; ItemInstance body = RustyCore `packets::item::ItemInstance` (identity of the sub-reader inferred from the shared function). Handler `0x141a49720`. |
| 0x277c | BattlePayMountDelivered | `u32 ProductID` | in-situ; handler `0x141a49ec0` reads one u32, resolves the product (`0x141a44a00`) and fires STORE_REFRESH. handler-verified (name from 7.3.5). |
| 0x277d | BattlePayBattlePetDelivered | `u32 DisplayID; guid BattlePetGuid` | ctor-reader `0x1406b8b50`. verified-in-client. Handler `0x141a496d0`. |
| 0x277e | BattlePayCollectionItemDelivered | `u32 Unk` | in-situ; handler slot `0x14309e888` = stub `0x1402112e0`: **ignored by the client**. inferred. |
| 0x2783 | BattlePayStartPurchaseResponse | `u64 PurchaseID; u32 PurchaseResult; u32 ClientToken` | in-situ ctor `0x1406b8bb0`; handler `0x141a4b4e0`: ignores unless `ClientToken(@12)` == pending token; `PurchaseID(@0) != 0` -> stores it; else maps `PurchaseResult(@8)` via `0x141a4b8d0` and fires `STORE_ORDER_INITIATION_FAILED` (`0x14117c630`). handler-verified. (WoD WPP has ClientToken before PurchaseResult: not true here.) |
| 0x2784 | BattlePayStartDistributionAssignToTargetResponse | `u64 DistributionID; u32 Result; u32 Unk` | in-situ; handler `0x141a49fd0` only tests `u32 @8 != 0` (error path `0x14117ad50`). handler-verified for @8; rest inferred from 7.3.5. |
| 0x2787 | BattlePayConfirmPurchase | `u64 PurchaseID; u32 ServerToken` | in-situ; handler `0x141a4b1a0` reads @0 (compared with the stored PurchaseID) and @8 (stored as the token echoed by CMSG 0x36d4), then fires `STORE_CONFIRM_PURCHASE` (`0x14117c580`). handler-verified. The price shown by the UI comes from the product list (`GetConfirmationInfo`), not from this packet (WoD's `CurrentPriceFixedPoint` is gone). |
| 0x2788 | BattlePayAckFailed | `u64 PurchaseID; u32 ServerToken; u32 Status; u32 Result` | in-situ; handler `0x141a4b130` reads @8 (token, stored for CMSG 0x36d5), @16 (Result; values 60/63 take a branch that does not raise STORE_PURCHASE_ERROR), @12 (Status). handler-verified; identical to 7.3.5. |
| 0x2818 | BattlePayValidatePurchaseResponse | `u32 Unk1; u32 Unk2; u64 Unk3; u64 Unk4; bit UnkBit flush` | ctor-reader `0x1406b91a0`; handler slot `0x14309f010` = stub: **ignored**. Layout verified, names unknown. |
| 0x281e | GenerateSsoTokenResponse | `u32 Kind; u32 Result; u64 Unk1; u64 Unk2; bits(7) TokenLen flush; str Token` | ctor-reader `0x1406c0900` (`len = byte >> 1`, token buffer 128); handler `0x14148bd60` (browser module): `Kind` is the key of the pending-request map (fmix64 hash), `Result == 0` delivers `Token`+`Unk1`+`Unk2` to the callback. verified-in-client. See section 4. |
| 0x2824 | BattlePayStartCheckout | `u32 ProductID; u32 GameServiceRegionID; u64 GameAccountID; bits(6) Len1; bits(7) Len2; bit Subscription; flush; str1; str2` | read inline in the dispatcher case `0x1407339e2` (`ReadBits6`, `ReadBits7`, bit); handler `0x141a49fa0` passes `(ProductID, RegionID, GameAccountID, str1@+0x30, str2@+0x70, bool@+0xc9)` to `0x141a4c7c0`. Layout verified; str1 = ServerValidationSignature (max 63) and str2 = ExternalTransactionID (buffer 88) inferred from the 7.3.5 order and the checkout JSON keys. |
| 0x2888 | BattlePayDistributionAssignVasResponse | `u64 DistributionID; u32 Result` | in-situ ctor `0x1406b8d10` pre-zeroes `{u64,u32}`; handled by a virtual listener (`0x1406f3aa0`). inferred. |
| 0x264c | DisplayPromotion | `u32 PromotionID` | in-situ; handler `0x1416a7920` reads one u32 (`-1` = special). handler-verified. |
| 0x286b | SyncWowEntitlements | `u32 nEntitlements; u32 nProducts; WowEntitlement[]; Product[]` | ctor-reader `0x1406cbe30`. verified-in-client. |
| 0x286c | WowEntitlementNotification | `bits(3) Unk flush; WowEntitlement; Product` | ctor-reader `0x1406ce1f0` (`unk = byte >> 5`). verified-in-client. Lua event `ENTITLEMENT_DELIVERED` / `STORE_ENTITLEMENT_NOTIFICATION` (section 5). |
| 0x27bf / 0x27c1 | CharacterUpgradeStarted / Aborted | `guid CharacterGUID` | ctor-reader `0x1406bb6a0` (shared). verified-in-client. |
| 0x27c0 | CharacterUpgradeComplete | `guid CharacterGUID; u32 Count; u32 Values[Count]; bit Unk flush` | ctor-reader `0x1406bb6e0`. verified-in-client (names unknown). |
| 0x27c3 | CharacterUpgradeManualUnrevokeResult | `u32 Result` | in-situ; handler `0x14169c0d0` -> `CHARACTER_UPGRADE_UNREVOKE_RESULT(result)`. handler-verified. |
| 0x27f3 | VasPurchaseStateUpdate | `u32 Unk; VasPurchase` | ctor-reader `0x1406cd7b0`. verified-in-client. |
| 0x27f4 | VasPurchaseComplete | `u32 ProductID; u32 Result; guid CharacterGuid; guid GlueGuid; u32 Unk; guid HandledGuid; bits(6) NameLen flush; str CharacterName` | ctor-reader `0x1406cd660`. verified-in-client, identical to 7.3.5. |
| 0x27f5 | EnumVasPurchaseStatesResponse | `bits(6) Count flush; VasPurchase[Count]` | ctor-reader `0x1406bf210` (+loop `0x1406bf350`). verified-in-client. |

## 3. Client -> server packets (from the client Write functions)

| Opcode | Name | Layout | Evidence / confidence |
|---|---|---|---|
| 0x36c4 | BattlePayGetProductList | empty | Write `0x1407647f0`. verified. |
| 0x36c5 | BattlePayGetPurchaseList | empty | Write `0x140764810`. verified. |
| 0x36d3 | BattlePayStartPurchase | `u32 ClientToken; u32 ProductID; guid TargetCharacter; bits(6) Len1; bits(12) Len2; bits(7) Len3; flush; str1; str2; str3` | Write `0x140764830` (`WriteBits6`, inline 8+4 bits, `WriteBits7`, `FlushBits`, three `PutData`). Layout verified; str1/str2 named `WowSytem`/`PublicKey` after 7.3.5 `CMSG_BATTLE_PAY_PURCHASE_PRODUCT` (6/12 bits), str3 (buffer at +0x862) is new and unnamed. Lua trigger `C_StoreSecure.PurchaseProduct(productID)`. |
| 0x36d4 | BattlePayConfirmPurchaseResponse | `bit ConfirmPurchase; flush; u32 ServerToken; u64 ClientCurrentPriceFixedPoint` | Write `0x1407646e0`. verified (= WoD/7.3.5). Lua `PurchaseProductConfirm(confirm, dollars, cents)`. |
| 0x36d5 | BattlePayAckFailedResponse | `u32 ServerToken` | Write `0x1407646a0`. verified. Lua `AckFailure()`. |
| 0x36cb | BattlePayDistributionAssignToTarget | `u32 ClientToken; u64 DistributionID; guid TargetCharacter; u32 ProductChoice` | Write `0x140764750`. verified; names from WoD (7.3.5 had ProductID/spec/choice instead: not this build). |
| 0x3714 | BattlePayOpenCheckout | `u32 RequestID` | Write `0x140766f30`. verified. Sent by `0x14148bf70` (section 4): the value is a fresh client counter that must come back as `Kind` in 0x281e. |
| 0x371b | BattlePayCancelOpenCheckout | `bits(6) Len; bit Flag; flush; str (max 63)` | Write `0x140765820`. Layout verified; string = ExternalTransactionID echo inferred from 7.3.5 (which used 7 bits and no flag). |
| 0x371a | BattlePayPurchaseSubmitted (**added**, not in the TC table) | `bits(6) Len1; bits(7) Len2; bit Flag; flush; str1 (max 40); str2 (max 89)` | Write `0x140766fe0`; message built by Store fn `0x141a4b030`. Layout verified; name provisional after 7.3.5 `CMSG_BATTLE_PAY_PURCHASE_SUBMITTED` (GlobalOrderID 6 bits, ExternalTransactionID 7 bits). |
| 0x3710 | BattlePayRequestPriceInfo | `u32 ClientToken; u32 ProductID` | Write `0x1407649f0` (two u32 from a pointed struct). Layout verified; names from 7.3.5. |
| 0x36fa | BattlePayStartVasPurchase | `u32 ClientToken; u32 ProductID; guid CharacterGuid; u32 CurrentRealmAddress; u32 DestinationRealmAddress; guid WowAccountGuid; guid BnetAccountGuid; guid Unk; bits(6) L1; bits(7) L2; bits(7) L3; bits(6) L4; bits(12) L5; bit Flag; flush; str1..str5` | Write body `0x140756cb0`. Layout verified; names for the leading fields from 7.3.5; third guid, str2/str3 (7 bits), flag are new. Lua `PurchaseVASProduct(productID, guid, newName, destRealm, wowAccountGUID, bnetAccountGUID, factionChangeBundle)`. |
| 0x36fb | UpdateVasPurchaseStates | empty | Write `0x140766e40`. verified. |
| 0x3742 | BattlePayDistributionAssignVas | `u32; u64 DistributionID; guid; u32; u32; guid; guid; guid; u8; u8; u8; u32 n; {u32,u32}[n]; bits(6) L1; bits(7) L2; bits(7) L3; bit; bit; bit; flush; str1..str3` | Write body `0x1407533f0`. Layout verified; names unknown (Lua `C_CharacterServices.Assign{NameChange,RaceOrFactionChange,PCT}Distribution`). |
| 0x36cd | CharacterUpgradeStart | `guid CharacterGUID; u32 Unk` | Write `0x140769d10`. verified. |
| 0x36cc | CharacterUpgradeManualUnrevokeRequest | `guid CharacterGUID` | Write `0x140765c70`. verified. Lua `C_CharacterServices.RequestManualUnrevoke(guid)`. |

CMSG that do not exist in this client: nothing named `GenerateSSOToken`; the other unknown writers found were `0x31ba` (`u32,u32,guid`) and `0x3a63` (movement family), unrelated to the shop.

## 4. Checkout / SSO flow at 54261

Client-side chain (all addresses verified in the dump):

1. UI: `C_StoreSecure.PurchaseProduct(productID)` -> CMSG 0x36d3 (`ClientToken` = client counter). Server answers 0x2783 with the same token; `PurchaseID != 0` = accepted (UI state `WaitingOnConfirmation`).
2. Either the "wallet" path (0x2787 -> STORE_CONFIRM_PURCHASE -> Lua `PurchaseProductConfirm(true, dollars, cents)` -> CMSG 0x36d4 -> 0x2786/0x277a-0x277e/0x2776 delivery packets) or the **web checkout** path:
3. Server sends **0x2824 BattlePayStartCheckout**. Handler `0x141a49fa0 -> 0x141a4c7c0` keeps ProductID/RegionID/GameAccountID/strings/Subscription for the `purchaseRequest` JSON and calls the browser controller `0x14148bf70`, then fires the Lua event `STORE_OPEN_SIMPLE_CHECKOUT(checkoutID)` (`0x14117c520`).
4. `0x14148bf70`: `id = ++g_ssoRequestCounter`; inserts `{id, callback}` in the pending map (globals `0x142d07828/0x142d07830`); builds the JAM message with vtable `0x1426dc7a0` (= CMSG **0x3714 BattlePayOpenCheckout**) with its single u32 = `id`; sends it (`0x14156dc60`). **This is the SSO token request of 3.4.3** — there is no separate CMSG.
5. Server must answer **0x281e GenerateSsoTokenResponse** with `Kind = id` (the u32 from 0x3714), `Result = 0`, `Token` (<= 127 bytes), `Unk1/Unk2` (unknown; 0 accepted by the handler, they are only forwarded). Handler `0x14148bd60` looks the entry up by `Kind` and invokes the callback; a non-zero `Result` invokes it without a token.
6. Lua `SimpleCheckoutMixin` (`Interface/AddOns/Blizzard_StoreUI/Blizzard_SimpleCheckout.lua:30-43`) on `STORE_OPEN_SIMPLE_CHECKOUT(checkoutID)` calls the frame method `SimpleCheckout:OpenCheckout(checkoutID)` (or `CancelOpenCheckout()` if the store is hidden). The embedded browser (`bnl_checkout 5.3.4`, `CheckoutWindow.cpp`, strings at `0x14287d300..0x14287e260`) loads the region SSO URL (`PROD_CHECKOUT_URL`, e.g. `https://account.battle.net/login/sso?ref=https://battle.net/shop/blizzard-checkout/loading`; EU navbar `https://eu.battle.net/shop/blizzard-checkout/NavBar`) with the token appended: function `0x141f444f4` appends `'?'` or `'&'` (depending on whether the URL already has a query) + `"token="` + token (`0x141f44cd8`). The page receives the `purchaseRequest` JSON with keys `productId, purchaseType, gameServiceRegionId, gameAccountId, giftType, giftingData, currencyCode, routingKey, externalTransactionId, serverValidationSignature, skipUpsell, deviceId` (no `ssoToken` key: the token travels only in the URL). Browser->client events handled by the window: `windowCloseRequested, windowResizeRequested, purchaseCanceledBeforeSubmit, purchaseFailureBeforeSubmit, purchaseSubmitted, purchaseError, orderPending, orderFailure, orderComplete, navbarReload/Forward/Back`; response fields `globalOrderId, errorCodes`.
7. Closing the window -> `SIMPLE_CHECKOUT_CLOSED` (Lua) and CMSG **0x371b BattlePayCancelOpenCheckout** (Store fn `0x141a44be0`) when the web did not submit; a submitted order -> CMSG **0x371a** (Store fn `0x141a4b030`, two strings + flag). The client then expects the usual completion packets (0x2786 purchase update, delivery packets, 0x2776 refresh) exactly as in the wallet path; the UI shows "processing" until `STORE_PURCHASE_LIST_UPDATED`.
8. `ssoToken` semantics for the web side are outside this spec; the 7.3.5 LegionCore README (`tools/bnet-shop/README.md`, "Como dispara el SSO el cliente") describes the `login/sso?token=...` verification done by their web tier, which is the same contract this client expects.

Also present but not shop-specific: the client implements the Battle.net RPC `AuthenticationService.GenerateWebCredentials` (descriptor strings `bnet.protocol.authentication.AuthenticationClient/Server`, method table at `0x141f1a5d0` with service hash `0xDECFC01`, methods 1/7/8) which retail uses for web credentials; it is not on the checkout path above (no call chain from the checkout code into it was found) and needs no server support for the shop.

## 5. Client UI evidence (Lua)

Files were extracted from the CASC install (product `wow_classic`, legacy ROOT with name hashes) into the session scratch directory `bp/ui/` with a throwaway crate depending on `crates/wow-casc`; they are Blizzard property and are not committed. Paths below are relative to that directory; **S** = `Interface_TBC/AddOns/Blizzard_StoreUI/Blizzard_StoreUISecure.lua` (the Wrath TOC `Interface/AddOns/Blizzard_StoreUI/Blizzard_StoreUI_Wrath.toc:19` loads the TBC variant), **SC** = `Interface/AddOns/Blizzard_StoreUI/Blizzard_SimpleCheckout.lua`, **CU** = `Interface/SharedXML/SecureCurrencyUtil.lua`, **VE** = `Interface/SharedXML/VASErrorLookup.lua`, **CS** = `Interface/GlueXML/CharacterSelect.lua`.

Requests and events that map to packets:

- `C_StoreSecure.GetProductList()` (S:1403,1409,1420,1443; CS:485) -> 0x36c4; `GetPurchaseList()` (S:1269; CS:484) -> 0x36c5; `C_StoreGlue.UpdateVASPurchaseStates()` (CS:486) -> 0x36fb. All three are sent at character-list load (CS:484-489) and on `STORE_REFRESH`/`STORE_ENTITLEMENT_NOTIFICATION` (S:1408).
- `PurchaseProduct(productID)` (S:1946) -> 0x36d3; `PurchaseProductConfirm(confirm, dollars, cents)` (S:2164,2254,2266) -> 0x36d4; `AckFailure()` (S:1896) -> 0x36d5; `GetFailureInfo()` -> `errorID, internalErr` from the last 0x2788 (S:1313,1372); `GetConfirmationInfo()` -> `productID, walletName, _, _, currentDollars, currentCents` after 0x2787 (S:2187); `RequestPriceInfo`-style calls are not used by this UI.
- Events: `STORE_PRODUCTS_UPDATED` (0x2775), `STORE_PURCHASE_LIST_UPDATED` (0x2776/0x2786), `PRODUCT_DISTRIBUTIONS_UPDATED(isNewBoost)` (0x2777/0x2779), `STORE_CONFIRM_PURCHASE` (0x2787), `STORE_PURCHASE_ERROR` (0x2788, `needsAck=true`), `STORE_ORDER_INITIATION_FAILED(err, internalErr)` (0x2783 with `PurchaseID == 0`), `STORE_OPEN_SIMPLE_CHECKOUT(checkoutID)` (0x2824), `SIMPLE_CHECKOUT_CLOSED`, `STORE_REFRESH` (0x277c), `STORE_ENTITLEMENT_NOTIFICATION` (0x286c), `STORE_VAS_PURCHASE_ERROR/COMPLETE` (0x27f3/0x27f4), `CHARACTER_UPGRADE_UNREVOKE_RESULT(errorCode)` (0x27c3) — registrations S:1245-1263, 2052, 2311-2317; SC:26,55-57; CS:158-185.
- Checkout frame: `<Checkout name="SimpleCheckout" ...>` widget (`Blizzard_SimpleCheckout.xml:13`); `OpenCheckout(checkoutID) -> wasOpened`, `CancelOpenCheckout()`, `CloseCheckout()` (`Interface/AddOns/Blizzard_APIDocumentationGenerated/FrameAPISimpleCheckoutDocumentation.lua`); SC:30-43 opens on `STORE_OPEN_SIMPLE_CHECKOUT` only if `StoreFrame:IsShown()`, otherwise `CancelOpenCheckout()`; `OnHide` -> `CloseCheckout()` (SC:61-73). No Lua ever sees the token or the URL.

Fields the UI reads (used to pick which packet fields matter):

- `GetEntryInfo(entryID)`: `productID`, `alreadyOwned`, `bannerType` (0 featured/1 discount/2 new; S:312-314,640-648), `sharedData.{name, tooltip, description, currentDollars, currentCents, normalDollars, normalCents, flags, productDecorator, boostType, buyableHere, eligibility, overrideTextColor, overrideBackground, texture, overrideTexture, itemID, modelSceneID, cards[{creatureDisplayInfoID, modelSceneID}], deliverables[{name, owned}], vasServiceType, canChangeAccount, canChangeBNetAccount}` (S:516-806, 2337-2348, 2687-2721, 2973-3035). Prices are always split fixed-point (`(dollars*100)+cents`, S:516-524) and formatted with the server-provided `formatShort/formatLong` patterns (CU:139-157).
- `GetProductGroupInfo(groupID)`: `displayType` (Enum.BattlepayGroupDisplayType Splash/DoubleWide/grid, S:992-994), `flags` (DisableOwnedProducts/EnabledForTrial/EnabledForVeteran, S:1161-1193), `texture`, `groupName`, `disabledTooltip` (S:1170-1199) -> ProductGroup IconFileDataID / Name / IsAvailableDescription.
- `GetCurrencyInfo().sharedData`: `regionID` (1 US, 2 KR, 3 EU, 4 TW, 5 CN, 98 BETA), `formatShort`, `formatLong`, `licenseAcceptText`, `requireLicenseAccept`, `browseHasStar`, `hideBrowseNotice`, `hideConfirmationBrowseNotice` (CU:171-381) — selected by `CurrencyID` of 0x2775.
- Hard-coded IDs: `WOW_TOKEN_CATEGORY_ID = 30`, `WOW_SERVICES_CATEGORY_ID = 22`, `WOW_CLASSIC_DARK_PORTAL_PASS_CATEGORY_ID = 161`, `CHARACTER_TRANSFER_PRODUCT_ID = 189`, `CHARACTER_TRANSFER_FACTION_BUNDLE_PRODUCT_ID = 239`, `DARK_PORTAL_PASS_PRODUCT_ID = 680` (S:299-322, 863).

Status / error tables:

- Purchase error map keyed by `Enum.StoreError` (S:445-497; VE:332-388): `InvalidPaymentMethod, PaymentFailed, WrongCurrency, BattlepayDisabled, InsufficientBalance, Other, AlreadyOwned, ParentalControlsNoPurchase, PurchaseDenied, ConsumableTokenOwned, TooManyTokens, ItemUnavailable, ClientRestricted`; unknown codes fall back to `Other` (S:1727-1730). These are the values the client maps 0x2788 `Result` / 0x2783 `PurchaseResult` to (numeric enum values are not in the Lua; they must come from the client `Enum` tables — open question).
- VAS errors keyed by `Enum.VasError` (VE:167-330) and purchase progress `Enum.VasPurchaseProgress.{PaymentPending, ApplyingLicense, WaitingOnQueue, ProcessingFactionChange}` (CS:964-978) -> VasPurchase `State`/`Errors`.
- UI purchase state machine (S:28-44, 1738-1790): `WaitingOnConfirmation` after 0x36d3 until 0x2787/0x2824/0x2783-failure; `JustOrderedProduct` after `PurchaseProductConfirm(true)` until `STORE_PURCHASE_LIST_UPDATED`; 60 s VAS timeout (S:4117-4134).

## 6. Confidence table (per opcode)

| Level | Opcodes |
|---|---|
| verified-in-client | 0x2775, 0x2776, 0x2777, 0x2779, 0x277b, 0x277d, 0x281e, 0x2824 (names of the two strings inferred), 0x286b, 0x286c, 0x27bf, 0x27c0, 0x27c1, 0x27f3, 0x27f4, 0x27f5, 0x2778 (names inferred), 0x2818 (ignored by client); all CMSG in section 3 |
| handler-verified | 0x2783, 0x2787, 0x2788, 0x277c, 0x264c, 0x27c3, 0x2784 (only `@8`) |
| inferred | 0x277a, 0x277e (both ignored by the client), 0x2888 |

## 7. Open questions for the server-flow agent

1. Numeric values of `Enum.StoreError` / `Enum.VasError` / `Enum.PurchaseEligibility` / `Enum.BattlepayDisplayFlag` at 54261 are not in the dumped Lua (this build ships no `Blizzard_APIDocumentation` for the secure store namespaces). They live in the client's enum tables; 7.3.5 LegionCore `Battlepay::Error` values are the best current guess. Note the client special-cases 0x2788 `Result` 60 and 63.
2. Mapping of DisplayInfo `Name1..Name7` to the Lua `sharedData` fields (`name`, `tooltip`, `description`, ...) and of `Unk1..Unk6`/Product `Unk6..Unk9`/ProductInfo `Unk2/Unk3`; send zeros/empty until a capture proves otherwise.
3. `GenerateSsoTokenResponse.Unk1/Unk2`: forwarded to the checkout callback untouched; zero is accepted by the handler. Meaning unknown (expiry?).
4. CMSG 0x371a semantic name/trigger: the Store module sends it (`0x141a4b030`); by 7.3.5 analogy it is the "purchase submitted" report carrying `globalOrderId` and `externalTransactionId`. Handle idempotently.
5. 0x2884 (RustyCore `BattlePayStartDistributionAssignToTargetResponse` sibling numbers) was not in scope; 0x2784 is documented.
6. Live capture against this server is still required to confirm the string-order assumptions in 0x2824 and 0x36d3 (the client only shows what it reads, not what the fields mean).

## 8. Server implementation (RustyCore, `battlepay` branch)

Code: `crates/wow-world/src/battle_pay.rs` (+ `battle_pay/{constants,catalog,service,flow,session_port}.rs`),
handlers `crates/wow-world/src/handlers/misc/battle_pay.rs`, ports
`crates/wow-persistence/src/battle_pay.rs`, adapters
`crates/wow-database/src/{battle_pay_adapter.rs,catalogs/battle_pay_adapter.rs,web_token.rs}`,
composition `crates/world-server/src/catalogs/battle_pay.rs`. Port of LegionCore
`BattlePayMgr.cpp`, `BattlePayData.cpp`, `BattlePayHandler.cpp`, `Player::ChangeTokenCount`.

### 8.1 State and ownership

- Catalog: loaded once at startup (world DB, LegionCore loader order and validation:
  `WebsiteType >= 32` skipped, items unknown to `Item.db2` or pointing at an unknown
  display info skipped, locale rows keyed by the numeric `LocaleConstant` — LegionCore's
  `GetLocaleByName("6")` bug is not reproduced). Immutable `Arc` in
  `SessionHandlerCatalogsLikeCpp::battle_pay`. No `.reload battlepay` (no reload command
  system exists); restart the world server after catalog edits.
- The single open purchase per account (LegionCore `_actualTransaction`: ids, tokens,
  price, lock, web checkout keys/pending) lives in the process-owned
  `BattlePayServiceLikeCpp`, keyed by game account (the `WorldSession` field set is frozen).
- Wallet balances are read from `auth.account_tokens` when needed (product list,
  StartPurchase, and atomically inside the charge), never cached at login.

### 8.2 Opcodes

All registrations are `Authed` (usable at character select and in world);
`ThreadUnsafe` except `UpdateVasPurchaseStates` (`Inplace`, the C++ value). All SMSG
go on the realm connection (C++ `CONNECTION_TYPE_REALM`).

| CMSG | Behaviour |
|---|---|
| 0x36c4 GetProductList | `IsAvailable` = `Bpay.Enabled && (FeatureSystem.BpayStore.Enabled || security >= 1)`; else result 1. Product list, then delivery of this account's `Paid` orders of this realm (both modes). |
| 0x36c5 GetPurchaseList | empty 0x2776 (LegionCore). |
| 0x36fb UpdateVasPurchaseStates | empty 0x27f5 (no VAS). |
| 0x36d3 StartPurchase | LegionCore `MakePurchase` checks (character in world and = target, product, group, deliverable, wallet balance in token mode, bag space for all items together, not already owned); 0x2783 (+0x2786 Loading) then 0x2787 (token) or order insert + 0x2824 (web). |
| 0x36d4 ConfirmPurchaseResponse | lock, server token, confirm bit and price checked; bags/owned rechecked; charge; deliver; 0x277a, 0x277c (mount items), 0x277b, 0x2786 Finish. |
| 0x36d5 AckFailedResponse | releases the lock for the matching server token. |
| 0x3714 OpenCheckout | 0x281e `Kind = RequestID`; `Result 0` + kind-1 SSO token only with `Browser.Enabled`, web mode and a pending checkout, else `Result 1`. |
| 0x371a PurchaseSubmitted | must match the pending checkout; order `Paid` → deliver (web order id stored); `Created` → stays pending (submitted before paid); `Failed`/missing → 0x2788 PaymentFailed. |
| 0x371b CancelOpenCheckout | pending + unlocked only; `Paid` → deliver; else `Created → Failed`, 0x2786 with 0 or PaymentFailed (`payment_ref` ending `;failed`). |
| 0x3710 RequestPriceInfo | resends the product list. |

Result codes are LegionCore 7.3.5 `Battlepay::Error` (Denied 1, PaymentFailed 2,
Other 3, InsufficientBalance 28) and `UpdateStatus` (Loading 9, Finish 3):
TODO-verify against the 54261 enum tables (section 7, question 1). `ProductInfo.unk1 = 47`
and `CurrencyID` from LegionCore's currency enum (EUR = 4) are likewise unverified.

### 8.3 Durability (charge once, deliver once)

1. Token mode: one Login DB transaction = guarded debit
   (`UPDATE account_tokens ... AND amount >= ?`, must hit 1 row), `account_donate_token_log`
   row, and a `battlepay_purchase` row inserted already `Paid` (`currency 'TOK'`,
   `payment_ref 'tokens:<type>'`). An unknown COMMIT is resolved by reading that row.
   Web mode: the web tier moves the `Created` row to `Paid`.
2. Delivery: items (the quest-reward `StoreNewItem` projection, moved to
   `player/direct_item_grant.rs`) plus a `characters.character_battlepay_delivery`
   receipt keyed by `external_id` commit in one Character DB transaction; an unknown
   COMMIT is resolved by the receipt. A failed commit after the in-memory store kicks
   the session (same policy as quest rewards).
3. `battlepay_purchase` `Paid → Delivered` (`WHERE external_id = ? AND status = 1`).

Any interruption leaves a `Paid` row; the next product-list request delivers it, and an
existing receipt turns the retry into step 3 only. Recovery is limited to rows whose
`realm` is this realm (the receipt lives in this realm's Character DB) — a deliberate
departure from LegionCore, which delivered web orders on any realm.

### 8.4 Configuration (`worldserver.conf`, read with `wow_config`, no registry rows)

| Key | Default | Meaning |
|---|---|---|
| `Bpay.Enabled` | 0 | master switch; also FeatureSystemStatus `BpayStoreAvailable` (+ `BpayStoreProductDeliveryDelay` 180, LegionCore) in glue and in-world packets |
| `FeatureSystem.BpayStore.Enabled` | 0 | existing key: `BpayStoreEnabled` bit and shop access for non-GM accounts |
| `Bpay.WebCheckout` | 0 | 1 = real-money web checkout instead of the token wallet |
| `Bpay.Currency` | EUR | ISO code: client currency id and `battlepay_purchase.currency` |
| `Bpay.WalletName` | Donation points | `JamBattlePayPurchase.WalletName` |
| `Browser.Enabled` | 0 | allow SSO tokens for the checkout browser |
| `Browser.TokenLifetime` | 3600 | SSO token lifetime (s) |

### 8.5 Not ported (documented scope)

Delivered product types are LegionCore `WebsiteType` 3 (Item) and 21 (ItemMount, delivered
as its item; LegionCore had no delivery arm for it). Everything else is neither listed nor
sellable: character boosts/distributions (0x2777/0x2779, `DisplayPromotion` at auth),
VAS services and character transfer, class trials, game time, battle pets, WoW Token,
toys API, RMAH, gold, script products, and the LegionCore custom addon chat messages
(`NOVA_WOW_STORE_BALANCE`). No mail fallback exists: item products need the character in
the world with enough bag space, checked before charging. Race restrictions of items are
not filtered (only `AllowableClass` and learned-spell ownership).

### 8.6 Store "Loading" gate and currency (live test follow-up)

- `StoreFrame_UpdateActivePanel` (`Blizzard_StoreUISecure.lua:1738-1790`) keeps
  `BLIZZARD_STORE_LOADING` until `HasPurchaseList() and HasProductList() and
  HasDistributionList()`. RustyCore now sends, for an available shop,
  `SMSG_DISPLAY_PROMOTION` (0) + an empty `SMSG_BATTLE_PAY_GET_DISTRIBUTION_LIST_RESPONSE`
  (Result 0, 0 objects) right after the character-select init burst (LegionCore
  `InitializeSessionCallback` → `SendDisplayPromo`, after the tutorial flags), and the empty
  distribution list again after every product list. A disabled shop sends neither, so the
  login burst is unchanged.
- The next gates are `#GetStoreProductGroups() > 0` (at character select the seed groups are
  `IngameOnly = 1`, so the glue store shows "no items", as in LegionCore), free bag slots in
  game, and `SecureCurrencyUtil.GetActiveCurrencyInfo()`. Every card evaluates
  `bit.band(sharedData.flags, …)`, so display infos now always carry `Flags` (0 when unset).
- `C_StoreSecure.GetCurrencyInfo` (client `0x141a46770`) takes the `CurrencyID` of the last
  product list (global `0x1431f780c`), looks it up in `BattlepayCurrency.db2` (FileDataID
  5549327, store `0x142e3ac10`, meta name `BattlepayCurrency`) and builds
  `{regionID, formatShort, formatLong, licenseAcceptText, flags}` from the row. `regionID`
  (global `0x1431f7810`, computed by `0x141a45de0`) is 98 when the row code is `XTS`, else the
  first `Cfg_Regions` row 1..5 whose `RegionGroupMask` bit (`1 << (mask-1)`, field 3; rows 1-5
  have masks 1-5) is set in the currency row's 4-bit field 7. Dumped from the 54261 CASC
  (read-only, `wow-casc`): row 1 USD (ISO 840, field 7 = 833 → bit 1 → region 1 US),
  2 GBP (826), 3 KRW (410), **4 EUR (ISO 978, field 7 = 836 → bit 4 → region 3 EU)**,
  5 RUB (643), 6 COP, 7 PEN, 8 ARS, 9 CLP, 10 MXN, 11 BRL, 12 AUD, …. So the LegionCore ids
  used by `Bpay.Currency` (USD 1, GBP 2, KRW 3, EUR 4, RUB 5) are valid in 54261, and EUR
  resolves to `REGION_EU`, which `SecureCurrencyUtil.currencySpecific` defines. The currency
  does not depend on any other server packet.
