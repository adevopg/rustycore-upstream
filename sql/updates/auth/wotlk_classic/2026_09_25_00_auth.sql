-- In-game shop (BattlePay) account-side tables, ported from LegionCore 7.3.5 to
-- WotLK Classic 3.4.3 (see docs/operations/battlepay-db.md).
--
-- Source: LegionCore auth updates 0002_battlepay_cleanup, 0003_battlepay_rework,
-- 2026_09_04_00_browser_url_map, 2026_09_04_01_battlepay_purchase,
-- 2026_09_05_00_battlepay_distribution, 2026_09_08_00_wow_token,
-- 2026_09_14_02_battlepay_purchase_vas, 2026_09_14_04_battlepay_revocacion,
-- 2026_09_14_05_battlepay_purchase_revoke_applied and
-- 2026_09_23_00_subastas_dinero_real (only the battlepay_purchase columns), i.e. the
-- final column set used by LoginDatabase.cpp (LOGIN_*_BPAY_*, LOGIN_*_ACCOUNT_TOKEN*,
-- LOGIN_SEL_BROWSER_URL_MAP, LOGIN_INS_BNET_WEB_TOKEN, LOGIN_*_ACCOUNT_GAME_TIME,
-- LOGIN_*_ACCOUNT_WOW_TOKEN) and by tools/bnet-shop.
--
-- Column names, types, defaults and key names are LegionCore's final state so that
-- tools/bnet-shop works unchanged against this database. Only the table charset is
-- normalised to utf8mb4_unicode_ci (LegionCore mixed latin1_swedish_ci and
-- utf8mb4_general_ci). Idempotent.

-- Host -> target rewrites for the in-game browser, served by bnet-server on
-- GET /bnetserver/browser/urlmap/ as {"host":"target",...}. `host` is an exact host,
-- `*.suffix` (longest wins) or `*`; `target` is scheme://host[:port][/prefix].
-- Replace shop.example.invalid with the real web shop host.
CREATE TABLE IF NOT EXISTS `browser_url_map` (
  `host` varchar(255) NOT NULL,
  `target` varchar(512) NOT NULL,
  `comment` varchar(255) NOT NULL DEFAULT '',
  PRIMARY KEY (`host`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci COMMENT='In-game browser host rewrites';

INSERT IGNORE INTO `browser_url_map` (`host`, `target`, `comment`) VALUES
('www.battle.net',   'https://shop.example.invalid:8095',       'shop / support (placeholder host, edit me)'),
('nydus.battle.net', 'https://shop.example.invalid:8095/nydus', 'checkout (placeholder host, edit me)'),
('*.battle.net',     'https://shop.example.invalid:8095',       'everything else (placeholder host, edit me)');

-- Single sign-on tokens handed to the client by the world server
-- (AuthenticationService.GenerateWebCredentials kind 0 / GenerateSSOToken kind 1);
-- the web validates the token against this table and matches `account` with the order.
CREATE TABLE IF NOT EXISTS `battlenet_account_web_token` (
  `token` varchar(64) NOT NULL,
  `battlenet_account` int unsigned NOT NULL,
  `account` int unsigned NOT NULL COMMENT 'game account id',
  `realm` int unsigned NOT NULL DEFAULT 0,
  `character_guid` bigint unsigned NOT NULL DEFAULT 0,
  `program` int unsigned NOT NULL DEFAULT 0 COMMENT 'FourCC from the request (WoW = 0x576F57)',
  `kind` tinyint unsigned NOT NULL DEFAULT 0 COMMENT '0 = web credentials, 1 = sso token',
  `ip` varchar(45) NOT NULL DEFAULT '',
  `created` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  `expires` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (`token`),
  KEY `idx_bnet` (`battlenet_account`),
  KEY `idx_expires` (`expires`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci COMMENT='In-game browser SSO tokens';

-- Shop orders paid through the web checkout. status: 0 created, 1 paid (set by the
-- web), 2 delivered, 3 failed/cancelled, 4 revoked (refund/chargeback, set by the web
-- or `.battlepay revoke`). external_id/signature are the externalTransactionId and
-- serverValidationSignature handed to the client in SMSG_BATTLE_PAY_START_CHECKOUT.
-- vas_target_* keep the destination of a character transfer until the web confirms
-- the payment. rmah_* mark an order that pays a LegionCore real-money auction instead
-- of delivering a product; RustyCore keeps them at 0 (kept for bnet-shop compatibility).
CREATE TABLE IF NOT EXISTS `battlepay_purchase` (
  `id` bigint unsigned NOT NULL AUTO_INCREMENT,
  `external_id` varchar(40) NOT NULL COMMENT 'externalTransactionId handed to the client/web',
  `signature` varchar(40) NOT NULL COMMENT 'serverValidationSignature handed to the client/web',
  `battlenet_account` int unsigned NOT NULL,
  `account` int unsigned NOT NULL COMMENT 'game account id',
  `realm` int unsigned NOT NULL DEFAULT 0,
  `character_guid` bigint unsigned NOT NULL DEFAULT 0,
  `product_id` int unsigned NOT NULL,
  `rmah_auction` bigint unsigned NOT NULL DEFAULT 0 COMMENT '0 = normal shop order; otherwise the real-money auction this order pays (unused by RustyCore)',
  `rmah_realm` int unsigned NOT NULL DEFAULT 0 COMMENT 'realm of that auction (unused by RustyCore)',
  `price` decimal(12,2) NOT NULL,
  `currency` varchar(3) NOT NULL DEFAULT 'EUR',
  `status` tinyint unsigned NOT NULL DEFAULT 0 COMMENT '0 created, 1 paid (set by the web), 2 delivered, 3 failed/cancelled, 4 revoked',
  `web_order_id` varchar(64) NOT NULL DEFAULT '' COMMENT 'GlobalOrderId reported by the web',
  `payment_ref` varchar(128) NOT NULL DEFAULT '' COMMENT 'payment provider reference, set by the web',
  `ip` varchar(45) NOT NULL DEFAULT '',
  `created` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  `paid` timestamp NULL DEFAULT NULL,
  `delivered` timestamp NULL DEFAULT NULL,
  `vas_target_account` int unsigned NOT NULL DEFAULT 0 COMMENT 'destination game account (0 = not a VAS order)',
  `vas_target_bnet_account` int unsigned NOT NULL DEFAULT 0,
  `vas_target_realm` int unsigned NOT NULL DEFAULT 0 COMMENT 'destination realmlist.id',
  `revoked` timestamp NULL DEFAULT NULL COMMENT 'refund/chargeback detected',
  `revoke_applied` tinyint unsigned NOT NULL DEFAULT 0 COMMENT '1 = the refund was already applied to the account (game time)',
  PRIMARY KEY (`id`),
  UNIQUE KEY `uk_external` (`external_id`),
  KEY `idx_account_status` (`account`, `status`),
  KEY `idx_created` (`created`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci COMMENT='In-game shop orders paid through the web checkout';

-- Character services owned by an account (character boost tokens shown on the
-- character selection screen). status: 1 available, 2 assigned (waiting for the
-- character to log in), 4 finished. revoked/revoke_applied drive the refund flow.
CREATE TABLE IF NOT EXISTS `battlepay_distribution` (
  `id` bigint unsigned NOT NULL COMMENT 'DistributionID handed to the client',
  `account` int unsigned NOT NULL COMMENT 'game account id',
  `battlenet_account` int unsigned NOT NULL DEFAULT 0,
  `product_id` int unsigned NOT NULL,
  `purchase_id` bigint unsigned NOT NULL DEFAULT 0 COMMENT 'battlepay_purchase.id that granted it (0 = other source)',
  `status` tinyint unsigned NOT NULL DEFAULT 1,
  `revoked` tinyint unsigned NOT NULL DEFAULT 0,
  `realm` int unsigned NOT NULL DEFAULT 0,
  `character_guid` bigint unsigned NOT NULL DEFAULT 0,
  `specialization_id` smallint unsigned NOT NULL DEFAULT 0,
  `choice_id` smallint unsigned NOT NULL DEFAULT 0,
  `created` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  `assigned` timestamp NULL DEFAULT NULL,
  `finished` timestamp NULL DEFAULT NULL,
  `revoke_applied` tinyint unsigned NOT NULL DEFAULT 0 COMMENT 'the realm already applied (1) or lifted (0) the character lock',
  PRIMARY KEY (`id`),
  KEY `idx_account_status` (`account`, `status`),
  KEY `idx_character` (`character_guid`, `status`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci COMMENT='Character services owned by an account, applied from the character selection screen';

-- Token wallets (token mode, Bpay.WebCheckout = 0): balance per game account and
-- token type (world.battlepay_tokens.tokenType). Read at world session start
-- (LOGIN_SEL_ACCOUNT_TOKENS) and changed with INSERT ... ON DUPLICATE KEY UPDATE
-- amount = amount + ? (LOGIN_INS_OR_UPD_TOKEN).
CREATE TABLE IF NOT EXISTS `account_tokens` (
  `account_id` int unsigned NOT NULL,
  `tokenType` tinyint unsigned NOT NULL,
  `amount` bigint NOT NULL DEFAULT 0,
  PRIMARY KEY (`account_id`, `tokenType`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Audit log of every token balance change. buyType: 0 BattlePayShop,
-- 1 VendorBuyCurrency, 2 VendorBuyItem (Battlepay::BattlepayCustomType).
CREATE TABLE IF NOT EXISTS `account_donate_token_log` (
  `id` int unsigned NOT NULL AUTO_INCREMENT,
  `time` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  `accountId` int unsigned NOT NULL,
  `realmId` int unsigned NOT NULL,
  `characterId` bigint unsigned NOT NULL,
  `change` bigint NOT NULL DEFAULT 0,
  `tokenType` tinyint unsigned NOT NULL,
  `buyType` tinyint unsigned NOT NULL,
  `productId` int unsigned NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- Prepaid game time per game account (game time products, WebsiteType 31, add
-- GameTimeDays here). A missing row means never initialised.
CREATE TABLE IF NOT EXISTS `account_game_time` (
  `id` int unsigned NOT NULL COMMENT 'account.id',
  `expire_time` int unsigned NOT NULL DEFAULT 0 COMMENT 'unix time the paid game time runs out',
  `tokens_redeemed` int unsigned NOT NULL DEFAULT 0,
  `last_redeem` int unsigned NOT NULL DEFAULT 0,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

-- WoW Tokens owned at account level ("distributions" for the client). type 0 =
-- auctionable token bought in the shop and not yet handed to a character, 1 =
-- redeemable token bought for gold and not redeemed yet.
CREATE TABLE IF NOT EXISTS `account_wow_token` (
  `id` bigint unsigned NOT NULL COMMENT 'distribution id sent to the client',
  `account` int unsigned NOT NULL COMMENT 'account.id',
  `type` tinyint unsigned NOT NULL DEFAULT 0 COMMENT '0 auctionable (pending delivery), 1 redeemable',
  `created` int unsigned NOT NULL DEFAULT 0,
  PRIMARY KEY (`id`),
  KEY `idx_account` (`account`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
