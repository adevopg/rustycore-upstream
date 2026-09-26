-- Battle.net (BattleTag) friends served by the worldserver, ported from the
-- LegionCore 7.3.5 `Battlenet::FriendsMgr` auth schema to WotLK Classic 3.4.3.
--
-- TrinityCore 3.4.3 registers `friends.v1.FriendsService` / `presence.v1.PresenceService`
-- as ERROR_RPC_NOT_IMPLEMENTED stubs and has no BattleTag column; the 3.4.3 client
-- only shows the BattleTag friends panel when `LogonResult.battle_tag` is set
-- (`FriendsFrame_Shared.lua` / `BNGetInfo()`).
--
-- Tables (LegionCore statements LOGIN_*_BNET_FRIEND*):
--   battlenet_accounts.battle_tag             the account's `Name#1234`
--   battlenet_account_friends                 two rows per friendship
--   battlenet_account_friend_invitations      pending invitations
--
-- Idempotent; MySQL 8 / 9 and MariaDB compatible (the guarded ALTER uses a prepared
-- statement because `ADD COLUMN IF NOT EXISTS` is MariaDB-only).

SET @rustycore_bnet_friends_add_battle_tag = (
  SELECT IF(
    (SELECT COUNT(*) FROM information_schema.columns
      WHERE table_schema = DATABASE() AND table_name = 'battlenet_accounts' AND column_name = 'battle_tag') = 0,
    'ALTER TABLE `battlenet_accounts` ADD COLUMN `battle_tag` VARCHAR(32) NULL DEFAULT NULL AFTER `email`',
    'SELECT 1'
  )
);
PREPARE rustycore_bnet_friends_stmt FROM @rustycore_bnet_friends_add_battle_tag;
EXECUTE rustycore_bnet_friends_stmt;
DEALLOCATE PREPARE rustycore_bnet_friends_stmt;

-- Default BattleTag: the e-mail local part plus a zero-padded discriminator
-- derived from the account id (`inna@inna.cl` id 7 -> `inna#0007`). Operators may
-- overwrite it; the value is only read, never regenerated, by the servers.
UPDATE `battlenet_accounts`
  SET `battle_tag` = CONCAT(SUBSTRING_INDEX(`email`, '@', 1), '#', LPAD(`id`, 4, '0'))
  WHERE `battle_tag` IS NULL;

CREATE TABLE IF NOT EXISTS `battlenet_account_friends` (
  `account_id` int unsigned NOT NULL COMMENT 'battlenet_accounts.id',
  `friend_id` int unsigned NOT NULL COMMENT 'battlenet_accounts.id of the friend',
  `note` varchar(127) NOT NULL DEFAULT '' COMMENT 'note account_id keeps about friend_id',
  `role` int unsigned NOT NULL DEFAULT 1 COMMENT '1 = BattleTag friend',
  PRIMARY KEY (`account_id`, `friend_id`),
  KEY `idx_friend` (`friend_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci COMMENT='Battle.net friend links (two rows per friendship)';

CREATE TABLE IF NOT EXISTS `battlenet_account_friend_invitations` (
  `id` bigint unsigned NOT NULL,
  `inviter_id` int unsigned NOT NULL COMMENT 'battlenet_accounts.id',
  `invitee_id` int unsigned NOT NULL COMMENT 'battlenet_accounts.id',
  `message` varchar(255) NOT NULL DEFAULT '',
  `created` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  `role` int unsigned NOT NULL DEFAULT 1,
  PRIMARY KEY (`id`),
  KEY `idx_inviter` (`inviter_id`),
  KEY `idx_invitee` (`invitee_id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci COMMENT='Pending Battle.net friend invitations';
