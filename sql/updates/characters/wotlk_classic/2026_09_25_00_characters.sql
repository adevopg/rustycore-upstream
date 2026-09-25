-- In-game shop (BattlePay) delivery receipts. RustyCore-only table (no LegionCore
-- equivalent): LegionCore delivered a paid order by flagging
-- auth.battlepay_purchase as delivered and then adding the items in memory, so a
-- crash between the two lost the purchase and a lost flag delivered it twice.
-- The wallet debit / web payment lives in the auth database and the items in this
-- one, and no portable transaction spans both pools. The realm therefore commits
-- this receipt in the same Character DB transaction as the delivered item rows and
-- only then marks the order delivered (auth.battlepay_purchase status 1 -> 2). A
-- replayed delivery (crash, lost status update, duplicate client notification)
-- finds the receipt and only re-marks the order: every paid order is delivered at
-- most once per realm and is never lost. external_id is the order's
-- auth.battlepay_purchase.external_id (unique there).
CREATE TABLE IF NOT EXISTS `character_battlepay_delivery` (
  `external_id` varchar(40) NOT NULL COMMENT 'auth.battlepay_purchase.external_id',
  `account` int unsigned NOT NULL COMMENT 'game account id',
  `guid` bigint unsigned NOT NULL COMMENT 'character that received the items',
  `product_id` int unsigned NOT NULL COMMENT 'world.battlepay_product.ProductID',
  `delivered` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (`external_id`),
  KEY `idx_guid` (`guid`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci COMMENT='BattlePay orders already delivered into this realm';
