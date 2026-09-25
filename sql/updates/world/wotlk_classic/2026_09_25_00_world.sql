-- In-game shop (BattlePay) product catalog, ported from LegionCore 7.3.5 to
-- WotLK Classic 3.4.3 (see docs/operations/battlepay-db.md for the table-by-table
-- mapping, the deviations and the bnet-shop compatibility contract).
--
-- Source schema: LegionCore sql/base/LegionCore_world_2020_04_25 battlepay_* tables plus
-- the world updates 0014_battlepay_rework, 0015_battlepay_group_fixes,
-- 2026_09_04_10_battlepay_price_decimal, 2026_09_12_02_battlepay_product_web and
-- 2026_09_14_05_battlepay_suscripcion, i.e. the columns read by
-- src/server/game/Globals/BattlePayData.cpp (LoadDisplayInfos, LoadDisplayInfoVisuals,
-- LoadProduct, LoadProductGroups, LoadShopEntires, LoadProductGroupLocales,
-- LoadDisplayInfoLocales, LoadTokenTypes) and by tools/bnet-shop/shop-server.mjs.
--
-- Column names are LegionCore's. Types are normalised to current MariaDB syntax and
-- every table is InnoDB utf8mb4_unicode_ci (LegionCore mixed MyISAM/utf8mb4_general_ci).
-- Locale columns hold the numeric LocaleConstant (esES = 6) because bnet-shop joins
-- `battlepay_display_info_locales.Locale = <number>`.
--
-- Idempotent: CREATE TABLE IF NOT EXISTS plus INSERT IGNORE seed rows.

-- Card texts and images. Name1 = title, Name2 = subtitle, Name3 = description,
-- Name4 unused. FileDataID is the icon FileDataID (ManifestInterfaceData.db2, global
-- across modern clients including 3.4.3). CreatureDisplayInfoID is the legacy card
-- model; the models actually shown come from battlepay_display_info_visuals.
CREATE TABLE IF NOT EXISTS `battlepay_display_info` (
  `DisplayInfoId` int unsigned NOT NULL AUTO_INCREMENT,
  `CreatureDisplayInfoID` int unsigned NOT NULL DEFAULT 0,
  `FileDataID` int unsigned DEFAULT NULL,
  `Flags` int unsigned NOT NULL DEFAULT 0,
  `Name1` varchar(1024) NOT NULL,
  `Name2` varchar(1024) NOT NULL DEFAULT '',
  `Name3` varchar(1024) NOT NULL DEFAULT '',
  `Name4` varchar(1024) NOT NULL DEFAULT '',
  PRIMARY KEY (`DisplayInfoId`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Translations of battlepay_display_info. Locale = LocaleConstant number
-- (enUS 0, koKR 1, frFR 2, deDE 3, zhCN 4, zhTW 5, esES 6, esMX 7, ruRU 8, ptBR 10, itIT 11).
CREATE TABLE IF NOT EXISTS `battlepay_display_info_locales` (
  `Id` mediumint unsigned NOT NULL DEFAULT 0,
  `Locale` mediumint unsigned NOT NULL DEFAULT 0,
  `Name1` varchar(1024) DEFAULT '',
  `Name2` varchar(1024) DEFAULT '',
  `Name3` varchar(1024) DEFAULT '',
  `Name4` varchar(1024) DEFAULT '',
  PRIMARY KEY (`Id`, `Locale`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Models rendered on a card: DisplayId = CreatureDisplayInfo.db2 id, VisualId =
-- UiModelScene.db2 id (3.4.3 data only has scenes 4, 6, 7, 10 and 595; 4 is the
-- scene Mount.db2 assigns to mounts). Several visuals may belong to one display info
-- (the loader collects a vector per DisplayInfoId), hence the composite key.
CREATE TABLE IF NOT EXISTS `battlepay_display_info_visuals` (
  `DisplayInfoId` int unsigned NOT NULL,
  `DisplayId` int unsigned NOT NULL DEFAULT 0,
  `VisualId` int unsigned NOT NULL DEFAULT 0,
  `ProductName` varchar(1024) NOT NULL DEFAULT '',
  PRIMARY KEY (`DisplayInfoId`, `DisplayId`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Shop tabs. DisplayType: 0 default, 1 splash, 2 double wide. Flags:
-- BattlepayProductGroupFlag (0x01 HideOwnedProducts, 0x02 EnabledForTrial,
-- 0x04 DisableOwnedProducts, 0x08 EnabledForVeteran, 0x10 HideForNonveterans).
-- TokenType selects the battlepay_tokens wallet used to pay for the group.
-- IngameOnly = 1 hides the group on the character selection (glue) store.
-- OwnsTokensOnly = 1 hides the group from accounts with no tokens of that type.
CREATE TABLE IF NOT EXISTS `battlepay_product_group` (
  `GroupID` int unsigned NOT NULL AUTO_INCREMENT,
  `Name` varchar(255) NOT NULL,
  `IconFileDataID` int unsigned NOT NULL DEFAULT 0,
  `DisplayType` tinyint unsigned NOT NULL DEFAULT 0,
  `Ordering` int unsigned NOT NULL DEFAULT 0,
  `Flags` int unsigned NOT NULL DEFAULT 0,
  `TokenType` tinyint unsigned NOT NULL DEFAULT 0,
  `IngameOnly` tinyint unsigned NOT NULL DEFAULT 1,
  `OwnsTokensOnly` tinyint unsigned NOT NULL DEFAULT 0,
  PRIMARY KEY (`GroupID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Translations of battlepay_product_group.Name, numeric Locale as above.
CREATE TABLE IF NOT EXISTS `battlepay_product_group_locales` (
  `GroupID` mediumint unsigned NOT NULL DEFAULT 0,
  `Locale` mediumint unsigned NOT NULL DEFAULT 0,
  `Name` varchar(255) NOT NULL DEFAULT '',
  PRIMARY KEY (`GroupID`, `Locale`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- The catalog. Prices are currency units with cents (10.99); the server converts
-- them to the client's fixed point (x10000), or rounds them up to whole tokens in
-- token mode. Type: 0 item product, 1 character boost/service, 2 WoW Token,
-- 3 game time, 10 product choice. ChoiceType is the client decorator selector
-- (5 WoW Token, 7-10/15/16 VAS services). WebsiteType is the LegionCore
-- Battlepay::WebsiteType (3 Item, 21 ItemMount, 29 CharacterBoost, 31 GameTime...).
-- ScriptName binds a server-side delivery script (battlepay_service_level70...).
-- GameTimeDays is delivered to auth.account_game_time for WebsiteType 31 products.
CREATE TABLE IF NOT EXISTS `battlepay_product` (
  `ProductID` int unsigned NOT NULL AUTO_INCREMENT,
  `NormalPriceFixedPoint` decimal(12,2) unsigned NOT NULL DEFAULT 0.00,
  `CurrentPriceFixedPoint` decimal(12,2) unsigned NOT NULL DEFAULT 0.00,
  `Type` tinyint unsigned NOT NULL DEFAULT 0,
  `ChoiceType` tinyint unsigned NOT NULL DEFAULT 0,
  `Flags` int unsigned NOT NULL DEFAULT 0,
  `DisplayInfoID` int unsigned NOT NULL DEFAULT 0,
  `ScriptName` varchar(64) NOT NULL DEFAULT '',
  `ClassMask` int unsigned NOT NULL DEFAULT 0,
  `WebsiteType` tinyint unsigned DEFAULT NULL,
  `GameTimeDays` smallint unsigned NOT NULL DEFAULT 0,
  PRIMARY KEY (`ProductID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Items delivered by a product. DisplayID is an optional battlepay_display_info id
-- for the item's own card (0 = use the product card); PetResult is the battle pet
-- result code (7.3.5 only, keep 0 on 3.4.3).
CREATE TABLE IF NOT EXISTS `battlepay_product_item` (
  `ID` int unsigned NOT NULL AUTO_INCREMENT,
  `ProductID` int unsigned NOT NULL,
  `ItemID` int unsigned NOT NULL,
  `Quantity` int unsigned NOT NULL DEFAULT 1,
  `DisplayID` int unsigned DEFAULT NULL,
  `PetResult` tinyint unsigned NOT NULL DEFAULT 0,
  PRIMARY KEY (`ID`),
  KEY `idx_product` (`ProductID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Placement of a product in a tab. Flags = VasServiceType for services,
-- BannerType = StoreDeliveryType (0 item, 2 service), DisplayInfoID overrides the
-- product card (0 = none; LegionCore never used a non-zero value in production).
CREATE TABLE IF NOT EXISTS `battlepay_shop_entry` (
  `EntryID` int unsigned NOT NULL AUTO_INCREMENT,
  `GroupID` int unsigned NOT NULL,
  `ProductID` int unsigned NOT NULL,
  `Ordering` int NOT NULL DEFAULT 0,
  `Flags` int unsigned NOT NULL DEFAULT 0,
  `BannerType` tinyint unsigned NOT NULL DEFAULT 0,
  `DisplayInfoID` int unsigned NOT NULL DEFAULT 0,
  PRIMARY KEY (`EntryID`),
  KEY `idx_group` (`GroupID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Wallets (token types) used in token mode. loginMessage, when not NULL, is shown at
-- login to accounts owning tokens of that type; listIfNone lists the wallet even at 0.
CREATE TABLE IF NOT EXISTS `battlepay_tokens` (
  `tokenType` tinyint unsigned NOT NULL,
  `name` varchar(255) NOT NULL DEFAULT '',
  `loginMessage` text DEFAULT NULL,
  `listIfNone` tinyint unsigned NOT NULL DEFAULT 1,
  PRIMARY KEY (`tokenType`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Extension row for the web storefront (tools/bnet-shop). Not a second catalog: names,
-- descriptions and prices come from battlepay_product / battlepay_display_info.
-- A product without a row here is sold in game but not listed on the web.
CREATE TABLE IF NOT EXISTS `battlepay_product_web` (
  `ProductID` int unsigned NOT NULL,
  `Category` varchar(32) NOT NULL DEFAULT '' COMMENT 'web tab: monturas, bolsas, servicios...',
  `Featured` tinyint(1) NOT NULL DEFAULT 0 COMMENT '1 = listed first',
  `ImageUrl` varchar(512) NOT NULL DEFAULT '' COMMENT 'web image; FileDataID is useless outside the client',
  `Icon` varchar(64) NOT NULL DEFAULT '' COMMENT 'icon name as published by wow.zamimg.com',
  `Visible` tinyint(1) NOT NULL DEFAULT 1 COMMENT '0 = sold in game, not advertised on the web',
  `SortOrder` int NOT NULL DEFAULT 0,
  PRIMARY KEY (`ProductID`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- ---------------------------------------------------------------------------
-- Demo catalog: one mount, one toy, one bag. Every id below was checked against the
-- 3.4.3 (54261) client data and the local databases:
--   43516 Big Blizzard Bear   ItemSparse.db2 record; mount 243 / spell 58983 in
--                             hotfixes.mount; CreatureDisplayInfo 27567 from MountXDisplay.db2
--   37254 Super Simian Sphere ItemSparse.db2 record; also in hotfixes.item_sparse
--   38082 "Gigantique" Bag    ItemSparse.db2 record; sold in world.npc_vendor
-- Icon FileDataIDs come from ManifestInterfaceData.db2 (Interface\ICONS\...).
-- ---------------------------------------------------------------------------

INSERT IGNORE INTO `battlepay_tokens` (`tokenType`, `name`, `loginMessage`, `listIfNone`) VALUES
(1, 'Battle Coins', NULL, 1);

-- Group ids follow LegionCore Battlepay::ProductGroups (1 Mount, 9 Toys, 11 Bags).
INSERT IGNORE INTO `battlepay_product_group` (`GroupID`, `Name`, `IconFileDataID`, `DisplayType`, `Ordering`, `Flags`, `TokenType`, `IngameOnly`, `OwnsTokensOnly`) VALUES
(1,  'Mounts', 132261, 0, 1, 0, 1, 1, 0),
(9,  'Toys',   134511, 0, 2, 0, 1, 1, 0),
(11, 'Bags',   133639, 0, 3, 0, 1, 1, 0);

INSERT IGNORE INTO `battlepay_product_group_locales` (`GroupID`, `Locale`, `Name`) VALUES
(1,  6, 'Monturas'),
(9,  6, 'Juguetes'),
(11, 6, 'Bolsas');

INSERT IGNORE INTO `battlepay_display_info` (`DisplayInfoId`, `CreatureDisplayInfoID`, `FileDataID`, `Flags`, `Name1`, `Name2`, `Name3`, `Name4`) VALUES
(1, 27567, 298586, 0, 'Big Blizzard Bear',   'Ground Mount', 'Summons and dismisses a rideable Big Blizzard Bear, complete with its murloc handler. Delivered to your character''s inventory or mailbox.', ''),
(2, 0,     133868, 0, 'Super Simian Sphere', 'Toy',          'Place yourself in a Super Simian Sphere and roll around Azeroth as a monkey in a ball. Delivered to your character''s inventory or mailbox.', ''),
(3, 0,     133639, 0, '"Gigantique" Bag',    '22-Slot Bag',  'A 22-slot bag from Haris Pilton. Delivered to your character''s inventory or mailbox.', '');

INSERT IGNORE INTO `battlepay_display_info_locales` (`Id`, `Locale`, `Name1`, `Name2`, `Name3`, `Name4`) VALUES
(1, 6, 'Gran oso de Blizzard',  'Montura terrestre',   'Invoca y despide un gran oso de Blizzard montable, con su cuidador múrloc incluido. Se entrega en el inventario o el buzón de tu personaje.', ''),
(2, 6, 'Superesfera simiesca',  'Juguete',             'Métete en una superesfera simiesca y rueda por Azeroth convertido en un mono. Se entrega en el inventario o el buzón de tu personaje.', ''),
(3, 6, 'Bolsa "Gigantique"',    'Bolsa de 22 espacios', 'Una bolsa de 22 espacios de Haris Pilton. Se entrega en el inventario o el buzón de tu personaje.', '');

INSERT IGNORE INTO `battlepay_display_info_visuals` (`DisplayInfoId`, `DisplayId`, `VisualId`, `ProductName`) VALUES
(1, 27567, 4, 'Big Blizzard Bear');

-- WebsiteType 21 = ItemMount, 3 = Item. Type 0 = plain item product.
INSERT IGNORE INTO `battlepay_product` (`ProductID`, `NormalPriceFixedPoint`, `CurrentPriceFixedPoint`, `Type`, `ChoiceType`, `Flags`, `DisplayInfoID`, `ScriptName`, `ClassMask`, `WebsiteType`, `GameTimeDays`) VALUES
(1, 15.00, 15.00, 0, 0, 0, 1, '', 0, 21, 0),
(2, 5.00,  5.00,  0, 0, 0, 2, '', 0, 3,  0),
(3, 8.00,  8.00,  0, 0, 0, 3, '', 0, 3,  0);

INSERT IGNORE INTO `battlepay_product_item` (`ID`, `ProductID`, `ItemID`, `Quantity`, `DisplayID`, `PetResult`) VALUES
(1, 1, 43516, 1, 0, 0),
(2, 2, 37254, 1, 0, 0),
(3, 3, 38082, 1, 0, 0);

INSERT IGNORE INTO `battlepay_shop_entry` (`EntryID`, `GroupID`, `ProductID`, `Ordering`, `Flags`, `BannerType`, `DisplayInfoID`) VALUES
(1, 1,  1, 1, 0, 0, 0),
(2, 9,  2, 1, 0, 0, 0),
(3, 11, 3, 1, 0, 0, 0);

INSERT IGNORE INTO `battlepay_product_web` (`ProductID`, `Category`, `Featured`, `ImageUrl`, `Icon`, `Visible`, `SortOrder`) VALUES
(1, 'monturas', 1, '', 'ability_mount_bigblizzardbear', 1, 10),
(2, 'juguetes', 0, '', 'inv_misc_enggizmos_10',         1, 20),
(3, 'bolsas',   0, '', 'inv_misc_bag_10',               1, 30);
