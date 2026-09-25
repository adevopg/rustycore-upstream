-- In-game shop (BattlePay): demo catalog of the character services, the character
-- transfer and the WotLK Classic character boosts (see docs/operations/battlepay-db.md
-- and docs/migration/battlepay-343-protocol.md, section 9).
--
-- Group 22 is the store's WOW_SERVICES_CATEGORY_ID (Blizzard_StoreUISecure.lua:318);
-- IngameOnly = 0 lists it on the character selection store too (LegionCore
-- ProductFilter without a player lists services and boosts).
--
-- Products (LegionCore Battlepay::WebsiteType / ProductChoiceTypeVas):
--   20 Name change            WebsiteType 5  (Rename),        ChoiceType 7  (VAS NameChange)
--   21 Appearance change      WebsiteType 22 (Customization), ChoiceType 9  (VAS AppearanceChange)
--   22 Faction change         WebsiteType 9  (Faction),       ChoiceType 8  (VAS FactionChange)
--   23 Race change            WebsiteType 10 (Race),          ChoiceType 10 (VAS RaceChange)
--   24 Restore deleted char.  WebsiteType 15 (DeletedCharacter), no VAS flow
--   25 Level 70 boost         WebsiteType 29 (CharacterBoost), Type 1, ScriptName "..._70"
--   26 Level 80 boost         WebsiteType 29 (CharacterBoost), Type 1, ScriptName "..._80"
--  189 Character transfer     ChoiceType 15 (VAS CharacterTransfer)
--  239 Transfer + faction     ChoiceType 16 (VAS FactionTransfer)
-- 189 and 239 are the client's hard-coded CHARACTER_TRANSFER_PRODUCT_ID and
-- CHARACTER_TRANSFER_FACTION_BUNDLE_PRODUCT_ID (Blizzard_StoreUISecure.lua:321-322).
-- The shop entry Flags carry Enum.VasServiceType (NameChange 0, FactionChange 1,
-- AppearanceChange 2, RaceChange 3, CharacterTransfer 4, FactionTransfer 5; values
-- read from the 54261 client's enum registration).
-- Icons are the CharacterServiceInfo.db2 (54261) icon FileDataIDs of each service:
-- 1126584 name, 1126583 faction, 1126585 race, 1530081 transfer, 342402 boost to 70,
-- 254652 boost to 80; the appearance change reuses the race change icon.
--
-- Idempotent: INSERT IGNORE on fixed ids.

INSERT IGNORE INTO `battlepay_product_group` (`GroupID`, `Name`, `IconFileDataID`, `DisplayType`, `Ordering`, `Flags`, `TokenType`, `IngameOnly`, `OwnsTokensOnly`) VALUES
(22, 'Services', 1126584, 0, 4, 0, 1, 0, 0);

INSERT IGNORE INTO `battlepay_product_group_locales` (`GroupID`, `Locale`, `Name`) VALUES
(22, 6, 'Servicios');

INSERT IGNORE INTO `battlepay_display_info` (`DisplayInfoId`, `CreatureDisplayInfoID`, `FileDataID`, `Flags`, `Name1`, `Name2`, `Name3`, `Name4`) VALUES
(20, 0, 1126584, 0, 'Name Change',               'Character service', 'Change the name of one of your characters. The new name is chosen on the character selection screen.', ''),
(21, 0, 1126585, 0, 'Appearance Change',         'Character service', 'Change the appearance of one of your characters on the character selection screen.', ''),
(22, 0, 1126583, 0, 'Faction Change',            'Character service', 'Change the faction of one of your characters (Horde to Alliance or Alliance to Horde) on the character selection screen.', ''),
(23, 0, 1126585, 0, 'Race Change',               'Character service', 'Change the race of one of your characters within its faction on the character selection screen.', ''),
(24, 0, 1126584, 0, 'Restore Deleted Character', 'Account service',   'Clears the waiting time of the character restore, so a deleted character can be restored now from the character selection screen.', ''),
(25, 0, 342402,  0, 'Level 70 Character Boost',  'Character boost',   'Raise one of your characters to level 70 with a set of gear. Choose the character on the character selection screen.', ''),
(26, 0, 254652,  0, 'Level 80 Character Boost',  'Character boost',   'Raise one of your characters to level 80 with a set of gear. Choose the character on the character selection screen.', ''),
(27, 0, 1530081, 0, 'Character Transfer',        'Character service', 'Move one of your characters to another game account of this realm.', ''),
(28, 0, 1530081, 0, 'Character Transfer + Faction Change', 'Character service', 'Move one of your characters to another game account of this realm and change its faction.', '');

INSERT IGNORE INTO `battlepay_display_info_locales` (`Id`, `Locale`, `Name1`, `Name2`, `Name3`, `Name4`) VALUES
(20, 6, 'Cambio de nombre',              'Servicio de personaje', 'Cambia el nombre de uno de tus personajes. El nombre nuevo se elige en la pantalla de selección de personaje.', ''),
(21, 6, 'Cambio de apariencia',          'Servicio de personaje', 'Cambia la apariencia de uno de tus personajes en la pantalla de selección de personaje.', ''),
(22, 6, 'Cambio de facción',             'Servicio de personaje', 'Cambia la facción de uno de tus personajes (de la Horda a la Alianza o de la Alianza a la Horda) en la pantalla de selección de personaje.', ''),
(23, 6, 'Cambio de raza',                'Servicio de personaje', 'Cambia la raza de uno de tus personajes dentro de su facción en la pantalla de selección de personaje.', ''),
(24, 6, 'Recuperar personaje borrado',   'Servicio de cuenta',    'Elimina el tiempo de espera de la recuperación de personajes: puedes recuperar ya un personaje borrado desde la pantalla de selección de personaje.', ''),
(25, 6, 'Subida de personaje al nivel 70', 'Subida de personaje', 'Sube uno de tus personajes al nivel 70 con un equipo completo. Elige el personaje en la pantalla de selección de personaje.', ''),
(26, 6, 'Subida de personaje al nivel 80', 'Subida de personaje', 'Sube uno de tus personajes al nivel 80 con un equipo completo. Elige el personaje en la pantalla de selección de personaje.', ''),
(27, 6, 'Transferencia de personaje',    'Servicio de personaje', 'Mueve uno de tus personajes a otra cuenta de juego de este reino.', ''),
(28, 6, 'Transferencia de personaje y cambio de facción', 'Servicio de personaje', 'Mueve uno de tus personajes a otra cuenta de juego de este reino y cambia su facción.', '');

INSERT IGNORE INTO `battlepay_product` (`ProductID`, `NormalPriceFixedPoint`, `CurrentPriceFixedPoint`, `Type`, `ChoiceType`, `Flags`, `DisplayInfoID`, `ScriptName`, `ClassMask`, `WebsiteType`, `GameTimeDays`) VALUES
(20,  10.00, 10.00, 0, 7,  0, 20, '',                   0, 5,  0),
(21,  10.00, 10.00, 0, 9,  0, 21, '',                   0, 22, 0),
(22,  25.00, 25.00, 0, 8,  0, 22, '',                   0, 9,  0),
(23,  20.00, 20.00, 0, 10, 0, 23, '',                   0, 10, 0),
(24,   5.00,  5.00, 0, 0,  0, 24, '',                   0, 15, 0),
(25,  40.00, 40.00, 1, 0,  0, 25, 'battlepay_boost_70', 0, 29, 0),
(26,  60.00, 60.00, 1, 0,  0, 26, 'battlepay_boost_80', 0, 29, 0),
(189, 20.00, 20.00, 0, 15, 0, 27, '',                   0, 12, 0),
(239, 35.00, 35.00, 0, 16, 0, 28, '',                   0, 12, 0);

INSERT IGNORE INTO `battlepay_shop_entry` (`EntryID`, `GroupID`, `ProductID`, `Ordering`, `Flags`, `BannerType`, `DisplayInfoID`) VALUES
(20, 22, 25,  1, 0, 0, 0),
(21, 22, 26,  2, 0, 0, 0),
(22, 22, 20,  3, 0, 0, 0),
(23, 22, 21,  4, 2, 0, 0),
(24, 22, 22,  5, 1, 0, 0),
(25, 22, 23,  6, 3, 0, 0),
(26, 22, 189, 7, 4, 0, 0),
(27, 22, 239, 8, 5, 0, 0),
(28, 22, 24,  9, 0, 0, 0);
