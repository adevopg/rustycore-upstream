# BattlePay (in-game shop) database schema

> Scope: the world and auth tables behind the 3.4.3 in-game shop, ported from
> LegionCore 7.3.5 as RustyCore migrations. Packet/handler behaviour is documented
> separately in `docs/migration/battlepay-343-protocol.md`.
> Source of truth for the schema: `/home/inna/legioncore-ref` (final effective schema
> after all `sql/updates/{world,auth}/*battlepay*` updates), the C++ loaders in
> `src/server/game/Globals/BattlePayData.cpp`, the prepared statements in
> `src/common/Database/Implementation/LoginDatabase.cpp` and the web shop
> `tools/bnet-shop/shop-server.mjs`.

## Migrations

| Database | Version | File | Manifest sha256 |
|---|---|---|---|
| world | `core:2026.09.25.00` | `sql/updates/world/wotlk_classic/2026_09_25_00_world.sql` | `670c0d36…5ec8a9` |
| auth | `core:2026.09.25.00` | `sql/updates/auth/wotlk_classic/2026_09_25_00_auth.sql` | `10277954…ecc7db` |
| characters | `core:2026.09.25.00` | `sql/updates/characters/wotlk_classic/2026_09_25_00_characters.sql` | `f1a652a0…f901ea` |

Both are idempotent (`CREATE TABLE IF NOT EXISTS` + `INSERT IGNORE`) and carry an
`adopt_query` in `database/migrations/manifest.toml` that recognises an already
created schema by its column set and primary keys, so an installation prepared by
hand is imported into `rustycore_schema_history` without re-running the DDL.

Apply them like any other RustyCore migration (backup first, see
[db-bootstrap.md](db-bootstrap.md)):

```bash
./target/release/rustycore-db status --config worldserver.conf
./target/release/rustycore-db migrate --dry-run --config worldserver.conf
./target/release/rustycore-db migrate --config worldserver.conf
./target/release/rustycore-db validate --config worldserver.conf
```

`status` and `migrate --dry-run` exit 3 while the migrations are pending; `migrate`
and `validate` exit 0 once applied. Server binaries embed the manifest at build time
(`wow_database::migration::bundled_manifest`), so `world-server` and `bnet-server`
must be rebuilt after the manifest changes or they refuse to open listeners with
"applied migration core:2026.09.25.00 is absent from the manifest".

## World database: catalog

All tables keep LegionCore's column names. Types are normalised to current MariaDB
syntax; every table is InnoDB `utf8mb4_unicode_ci` (LegionCore mixed MyISAM and
`utf8mb4_general_ci`, which changes nothing for readers).

### `battlepay_product` — the catalog

| Column | Type | Meaning |
|---|---|---|
| `ProductID` | int unsigned PK | Product id sent to the client. Some ids are hardcoded in the client UI (LegionCore had to use 189/239 for character transfer); check the 3.4.3 `Blizzard_StoreUI` Lua before reusing Blizzard ids. |
| `NormalPriceFixedPoint`, `CurrentPriceFixedPoint` | decimal(12,2) unsigned | Currency units with cents (`10.99`). The server multiplies by 10000 for the client fixed point, or rounds up to whole tokens in token mode. Despite the name they are **not** stored as fixed point. |
| `Type` | tinyint unsigned | 0 item product, 1 character service/boost, 2 WoW Token, 3 game time, 10 product choice (LegionCore `PRODUCT_TYPE_*`). |
| `ChoiceType` | tinyint unsigned | Client decorator selector: 5 WoW Token, 7 name change, 8 faction change, 9 appearance change, 10 race change, 15 character transfer, 16 faction transfer, 12/13 expansion. |
| `Flags` | int unsigned | Product flags sent as-is. |
| `DisplayInfoID` | int unsigned | `battlepay_display_info.DisplayInfoId` of the card. |
| `ScriptName` | varchar(64) | Server delivery script name (`battlepay_service_level90` in LegionCore). Empty for plain item delivery. |
| `ClassMask` | int unsigned | Class restriction mask, 0 = all. |
| `WebsiteType` | tinyint unsigned NULL | LegionCore `Battlepay::WebsiteType` (3 Item, 21 ItemMount, 29 CharacterBoost, 30 BattlePet, 31 GameTime, …); rows with a value ≥ 32 are skipped by the loader. |
| `GameTimeDays` | smallint unsigned | Days added to `auth.account_game_time` for `WebsiteType` 31 products. |

### `battlepay_product_item` — deliverables

`ID` PK, `ProductID`, `ItemID` (item entry, validated against the item store at load),
`Quantity`, `DisplayID` (optional `battlepay_display_info` id for the item's own card,
0 = product card; rows pointing to an unknown display are skipped), `PetResult`
(7.3.5 battle-pet result code, keep 0). Extra index `idx_product (ProductID)`.

### `battlepay_product_group` — tabs

`GroupID` PK, `Name`, `IconFileDataID`, `DisplayType` (0 default, 1 splash,
2 double wide), `Ordering`, `Flags` (`BattlepayProductGroupFlag`: 0x01
HideOwnedProducts, 0x02 EnabledForTrial, 0x04 DisableOwnedProducts, 0x08
EnabledForVeteran, 0x10 HideForNonveterans), `TokenType` (wallet from
`battlepay_tokens`), `IngameOnly` (1 = hidden on the character-selection store),
`OwnsTokensOnly` (1 = hidden from accounts without tokens of that type).

LegionCore found that the 7.3.5 client opens some tabs by fixed id (22 services,
30 WoW Token, 33 games, 37 game time). Whether 3.4.3 has the same constants is a
protocol question; the seed uses LegionCore's generic ids (1 mounts, 9 toys, 11 bags).

### `battlepay_shop_entry` — placement

`EntryID` PK, `GroupID`, `ProductID`, `Ordering` (signed), `Flags` (VAS service
type for services), `BannerType` (`StoreDeliveryType`: 0 item, 2 service),
`DisplayInfoID` (card override, keep 0: LegionCore never shipped a non-zero value
and hit a client crash while testing one). Extra index `idx_group (GroupID)`.

### `battlepay_display_info` — card texts

`DisplayInfoId` PK, `CreatureDisplayInfoID` (legacy card model), `FileDataID`
(icon `FileDataID` from `ManifestInterfaceData.db2`, global across modern clients
including 3.4.3), `Flags` (0x8 hides the price), `Name1` title, `Name2` subtitle,
`Name3` description, `Name4` unused.

### `battlepay_display_info_locales` and `battlepay_product_group_locales`

Translations keyed by `(Id|GroupID, Locale)`. `Locale` is the numeric
`LocaleConstant`: enUS 0, koKR 1, frFR 2, deDE 3, zhCN 4, zhTW 5, esES 6, esMX 7,
ruRU 8, ptBR 10, itIT 11. The numeric form is a compatibility requirement: the web
shop joins `battlepay_display_info_locales.Locale = <number>`.

Note for the Rust loader: LegionCore's C++ read this column as a string and passed it
through `GetLocaleByName`, which does not understand `"6"` and silently fell back to
enUS; the effective LegionCore behaviour was therefore wrong for the game while the
web used the numbers correctly. RustyCore must map the number directly.

### `battlepay_display_info_visuals` — card models

`(DisplayInfoId, DisplayId)` PK, `VisualId`, `ProductName`. `DisplayId` is a
`CreatureDisplayInfo.db2` id and `VisualId` a `UiModelScene.db2` id. The 3.4.3 client
data only contains scenes 4, 6, 7, 10 and 595; scene 4 is what `Mount.db2` assigns to
mounts. LegionCore's 7.3.5 "store mount scene" 72 does not exist in 3.4.3.

### `battlepay_tokens` — wallets (token mode)

`tokenType` PK, `name`, `loginMessage` (NULL = none), `listIfNone`. Seed: type 1
"Battle Coins". Balances live in `auth.account_tokens`.

### `battlepay_product_web` — web storefront extension

`ProductID` PK, `Category`, `Featured`, `ImageUrl`, `Icon` (wowhead icon name),
`Visible`, `SortOrder`. Read only by `tools/bnet-shop`; a product without a row is
sold in game but not listed on the web. Names, descriptions and prices are never
duplicated here.

### Not ported

- `trinity_string` entries 14090–14096 / 20000 / 20062 (LegionCore shop messages):
  RustyCore server strings are owned by the protocol/handler work; the ids are free in
  `TDB 343.24081` if that work wants them.
- `command` row for `.reload battlepay`.
- Tables named `battlepay_product_addon` / `battlepay_currency` do not exist in
  LegionCore (they belong to other cores' schemas; the user's earlier Rust port used a
  different, FirestormCore-style layout: `battlepay_displayinfo`, `battlepay_group`,
  `battlepay_shop`, `battlepay_productinfo`, `battlepay_item`, `battlepay_addon`,
  `battlepay_visual`). That layout is intentionally not carried over.

## Auth database: accounts, orders, sessions

Column names, types, defaults, key names and column order are LegionCore's final
state. The only deviation is the table charset (`utf8mb4_unicode_ci` instead of
LegionCore's `latin1_swedish_ci`/`utf8mb4_general_ci`), which does not affect any
query used by the core or the web shop.

### `battlepay_purchase` — web checkout orders

| Column | Notes |
|---|---|
| `id` | bigint PK auto-increment |
| `external_id` (unique `uk_external`), `signature` | `externalTransactionId` / `serverValidationSignature` given to the client in `SMSG_BATTLE_PAY_START_CHECKOUT` and forwarded to the web |
| `battlenet_account`, `account`, `realm`, `character_guid` | buyer identity |
| `product_id` | `world.battlepay_product.ProductID` |
| `rmah_auction`, `rmah_realm` | LegionCore real-money auction house link; RustyCore keeps 0. Present so the web's `SELECT *`/inserts stay identical |
| `price`, `currency` | decimal(12,2), ISO 4217 (`EUR`) |
| `status` | 0 created, 1 paid (web), 2 delivered (world), 3 failed/cancelled, 4 revoked |
| `web_order_id`, `payment_ref`, `ip` | set by the web |
| `created`, `paid`, `delivered`, `revoked` | timestamps of each transition |
| `vas_target_account`, `vas_target_bnet_account`, `vas_target_realm` | destination of a character transfer order |
| `revoke_applied` | 1 once the realm applied a refund (game time) |

Indexes: `idx_account_status (account, status)`, `idx_created (created)`.

### `battlepay_distribution` — character services owned by an account

`id` PK (DistributionID handed to the client), `account`, `battlenet_account`,
`product_id`, `purchase_id` (0 = granted by other means), `status` (1 available,
2 assigned, 4 finished), `revoked`, `realm`, `character_guid`, `specialization_id`,
`choice_id`, `created`, `assigned`, `finished`, `revoke_applied`. Indexes
`idx_account_status`, `idx_character`. This is the WotLK Classic level-70 boost model:
a paid boost becomes a distribution the player assigns at character selection.

### `battlenet_account_web_token` — SSO tokens for the in-game browser

`token` PK (64 hex), `battlenet_account`, `account`, `realm`, `character_guid`,
`program`, `kind` (0 web credentials, 1 SSO token), `ip`, `created`, `expires`.
Written by the world server on `GenerateWebCredentials`/`GenerateSSOToken`, expired
rows deleted with `DELETE … WHERE expires < NOW()`.

### `browser_url_map` — host rewrites for the in-game browser

`host` PK, `target`, `comment`. Served by bnet-server as JSON on
`GET /bnetserver/browser/urlmap/`. Seeded with LegionCore's three example rows
(`www.battle.net`, `nydus.battle.net`, `*.battle.net`) pointing at the placeholder
`https://shop.example.invalid:8095`; **edit these rows** to the real shop host. The
LegionCore Twitter callback row is not seeded (no Twitter integration in 3.4.3).

### `account_tokens` and `account_donate_token_log` — token wallets

`account_tokens (account_id, tokenType) PK, amount bigint` is the balance read at
session start (`SELECT tokenType, amount FROM account_tokens WHERE account_id = ?`)
and changed with `INSERT … ON DUPLICATE KEY UPDATE amount = amount + ?`
(`Player::ChangeTokenCount`). Every change is logged in `account_donate_token_log`
(`id`, `time`, `accountId`, `realmId`, `characterId`, `change`, `tokenType`,
`buyType` 0 shop / 1 vendor currency / 2 vendor item, `productId`).

### `account_game_time` and `account_wow_token`

Prepaid game time per account (`id` = account id, `expire_time`, `tokens_redeemed`,
`last_redeem`) and account-level WoW Tokens (`id` distribution id, `account`, `type`
0 auctionable / 1 redeemable, `created`). Delivery targets for game-time and WoW
Token products; both features are optional in RustyCore. The per-realm
`characters.wow_token_auction` table is auction-house scope and is not part of this
migration.

### Not ported (auth)

`account_raf_reward`, `rmah_payout`, `twitter_*`, `account.balans/first_ip/referer`
changes (LegionCore-specific features outside the shop).

## Characters database: delivery receipts

`character_battlepay_delivery (external_id PK, account, guid, product_id, delivered)` is
RustyCore-only. The realm commits it in the same transaction as the delivered item rows,
before marking `auth.battlepay_purchase` delivered, so a paid order (web or token wallet)
is delivered at most once per realm and is never lost; see
`docs/migration/battlepay-343-protocol.md` section 8.3. Token-wallet purchases are also
recorded in `auth.battlepay_purchase` (`currency = 'TOK'`, `payment_ref = 'tokens:<type>'`,
inserted with status 1), which bnet-shop never selects (it looks orders up by
`external_id` + `signature`).

## Seed catalog

The world migration seeds a three-product demo so the shop renders something once the
handlers exist. Every id was verified against the 3.4.3 (54261) client DB2 files in
`DataDir/dbc/esES` and the local databases:

| Product | Group | Item | Evidence | Price |
|---|---|---|---|---|
| 1 Big Blizzard Bear | 1 Mounts | 43516 | `ItemSparse.db2` record; `hotfixes.mount` 243 (spell 58983); `MountXDisplay.db2` → CreatureDisplayInfo 27567; icon FDID 298586 `ability_mount_bigblizzardbear` | 15.00 |
| 2 Super Simian Sphere | 9 Toys | 37254 | `ItemSparse.db2` record and `hotfixes.item_sparse`; icon FDID 133868 `INV_Misc_EngGizmos_10` | 5.00 |
| 3 "Gigantique" Bag | 11 Bags | 38082 | `ItemSparse.db2` record; sold in `world.npc_vendor`; icon FDID 133639 `INV_Misc_Bag_10` | 8.00 |

Group icons: 132261 `Ability_Mount_RidingHorse`, 134511 `INV_Misc_Toy_07`, 133639
`INV_Misc_Bag_10`. All products have enUS texts in `battlepay_display_info`, esES
texts (`Locale` 6) in `battlepay_display_info_locales`, group names in
`battlepay_product_group_locales`, a `battlepay_product_web` row (categories
`monturas`, `juguetes`, `bolsas`) and use wallet type 1. The mount has one visual
(display 27567, scene 4). All groups are `IngameOnly = 1`.

## Adding a product

1. `battlepay_display_info`: insert the card (title, subtitle, description, icon
   `FileDataID`). Add `battlepay_display_info_locales` rows per locale number and,
   for a model, `battlepay_display_info_visuals` (`DisplayId` from
   `CreatureDisplayInfo.db2`, `VisualId` 4 for mounts).
2. `battlepay_product`: insert the product with the price in currency units, `Type`
   0 for item delivery, `WebsiteType` 3 (item) or 21 (mount), `DisplayInfoID` from
   step 1.
3. `battlepay_product_item`: one row per delivered item (`ItemID` must exist in the
   item store or the loader drops it).
4. `battlepay_shop_entry`: place the product in a `battlepay_product_group`.
5. Optional `battlepay_product_web` row for the web storefront.
6. Reload the catalog (server reload command, once available) or restart the world
   server.

Character services use `Type` 1, a `ScriptName` and `WebsiteType` 29; the delivery
creates a `battlepay_distribution` row instead of items.

## bnet-shop compatibility contract

`tools/bnet-shop` (LegionCore) runs unchanged against these databases. It touches
exactly:

| Table | Reads | Writes |
|---|---|---|
| `auth.battlenet_account_web_token` | `token, battlenet_account, account, kind, expires` (`support.mjs` also `realm, character_guid`) | — |
| `auth.battlepay_purchase` | `SELECT *` by `external_id + signature`; `status`, `payment_ref`, `created`, `paid`, `account`, `product_id`, `price`, `currency`, `external_id`, `web_order_id` | `payment_ref`; `status`/`paid`/`payment_ref`/`web_order_id` on payment; `status` on failure; `status`/`revoked`/`payment_ref` on refund |
| `auth.battlepay_distribution` | — | `revoked = 1 WHERE purchase_id = ?` on refund |
| `world.battlepay_product` | `ProductID`, `CurrentPriceFixedPoint` | — |
| `world.battlepay_product_web` | `ProductID`, `Visible`, `Category`, `Featured`, `ImageUrl`, `Icon`, `SortOrder` | — |
| `world.battlepay_display_info` | `DisplayInfoId`, `Name1`, `Name3` | — |
| `world.battlepay_display_info_locales` | `Id`, `Locale` (numeric), `Name1`, `Name3` | — |

Status values it relies on: 0 Created, 1 Paid, 2 Delivered, 3 Failed, 4 Revoked.
Its database names come from `SHOP_DB_AUTH`/`SHOP_DB_WORLD`/`SHOP_DB_CHARACTERS`
(defaults `auth`/`world`/`characters`); on this host the characters database is named
`rc_characters`, which only matters for `support.mjs` (tickets), not for the shop.
The server side of the contract (what RustyCore must do with a paid order, the SSO
token generation and the checkout packets) is in the protocol document.

## Deviations from LegionCore, summarised

- Charset/engine normalised (InnoDB, `utf8mb4_unicode_ci`).
- `battlepay_display_info` and `battlepay_display_info_visuals` use a real primary
  key; visuals key is `(DisplayInfoId, DisplayId)` so one card can carry several
  models, matching the C++ loader's vector.
- `battlepay_product_group_locales`: `Locale` is numeric and part of the primary key
  (LegionCore had `text` and a `GroupID`-only key, which cannot hold two locales).
- `battlepay_product.ScriptName` is `varchar(64)` instead of `text`.
- Extra secondary indexes: `battlepay_product_item.idx_product`,
  `battlepay_shop_entry.idx_group`.
- Seeds differ (3.4.3 ids; LegionCore's 7.3.5 boosts, WoW Token, game time, RAF
  rewards and character transfer products are not seeded). Column order of
  `battlepay_purchase` follows LegionCore's ALTER history exactly.
- Nothing else: every column LegionCore reads or writes exists with the same name and
  type, including 7.3.5-only ones (`PetResult`, `GameTimeDays`, `rmah_*`, `vas_*`).
