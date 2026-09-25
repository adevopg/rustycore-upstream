//! Paid character transfer (VAS `CharacterTransfer` / `FactionTransfer`).
//!
//! Port of LegionCore `HandleBattlePayStartVasPurchase` checks
//! (`BattlePayHandler.cpp:1397-1470`), `HandleBattlePayValidateBnetVasTransfer`
//! (1292-1352) and the same-realm branch of `CompleteVasCharacterTransfer` (98):
//! the character's `account` column moves to the destination game account in
//! one Character DB transaction with its guild membership removal, the
//! faction-change flag of the faction bundle and the delivery receipt. Only the
//! same realm is a destination: this realm's characters DB holds the character,
//! and LegionCore's cross-realm path (PlayerDump export into
//! `auth.vas_character_transfer` + import by the target realm) has no RustyCore
//! PlayerDump to build on (documented in the protocol doc, section 9.5).

use tracing::{info, warn};
use wow_core::ObjectGuid;
use wow_core::guid::HighGuid;
use wow_packet::packets::battlepay::{
    BattlePayStartVasPurchase, VasCheckTransferOk, VasCheckTransferOkResponse,
    VasTransferGameAccount,
};
use wow_persistence::{
    BattlePayCharacterRowLikeCpp, BattlePayCharacterTransferLikeCpp,
    BattlePayDeliveryReceiptLikeCpp, PersistenceOutcomeLikeCpp,
};

use super::constants::*;
use super::flow::{
    BattlePayIdentityLikeCpp, BattlePaySessionLikeCpp, DeliveryOutcomeLikeCpp, PaidOrderLikeCpp,
    mark_delivered, receipt_exists,
};
use super::service::{BattlePayServiceLikeCpp, VasTransferTargetLikeCpp};
use super::vas::{player_guid_like_cpp, send_vas_complete};

/// Result of the Battle.net account validation (0 = found, 1 = not usable).
const TRANSFER_VALIDATION_OK_LIKE_CPP: u32 = 0;
const TRANSFER_VALIDATION_FAILED_LIKE_CPP: u32 = 1;

/// Destination checks of LegionCore `HandleBattlePayStartVasPurchase` and its
/// callback (target account belongs to the named Battle.net account).
pub(crate) async fn check_transfer(
    service: &BattlePayServiceLikeCpp,
    identity: &BattlePayIdentityLikeCpp,
    request: &BattlePayStartVasPurchase,
    row: &BattlePayCharacterRowLikeCpp,
) -> Result<VasTransferTargetLikeCpp, (u32, &'static str)> {
    let in_world = identity
        .player
        .is_some_and(|player| player.guid.counter() as u64 == row.guid);
    if row.online || in_world {
        return Err((vas_error::CHAR_LOCKED, "the character is online"));
    }
    if row.guild_id != 0 && row.guild_leader_guid == row.guid {
        return Err((
            vas_error::CANNOT_MOVE_GUILD_MASTER,
            "the character leads its guild",
        ));
    }
    if request.destination_realm_address != 0
        && request.destination_realm_address != identity.virtual_realm_address
    {
        return Err((
            vas_error::INELIGIBLE_TARGET_REALM,
            "only same-realm transfers are delivered",
        ));
    }
    let to_account = if request.wow_account_guid.is_empty() {
        identity.account_id
    } else {
        request.wow_account_guid.counter() as u32
    };
    let to_bnet = if request.bnet_account_guid.is_empty() {
        identity.battlenet_account_id
    } else {
        request.bnet_account_guid.counter() as u32
    };
    if to_account == 0 || to_account == identity.account_id {
        return Err((
            vas_error::INVALID_DESTINATION_ACCOUNT,
            "the destination is the same account and realm",
        ));
    }
    match service
        .distributions
        .load_account_battlenet_like_cpp(to_account)
        .await
    {
        Ok(Some(bnet)) if bnet == to_bnet && bnet != 0 => {}
        Ok(_) => {
            return Err((
                vas_error::INVALID_DESTINATION_ACCOUNT,
                "the account is not of that Battle.net account",
            ));
        }
        Err(error) => {
            warn!(%error, "BattlePay: transfer target lookup failed");
            return Err((
                vas_error::INVALID_DESTINATION_ACCOUNT,
                "target lookup failed",
            ));
        }
    }
    match service
        .characters
        .load_account_characters_like_cpp(to_account)
        .await
    {
        Ok(rows) if rows.len() < service.config.characters_per_realm => {}
        Ok(_) => {
            return Err((
                vas_error::MAX_CHARACTERS_ON_SERVER,
                "the destination account is full",
            ));
        }
        Err(error) => {
            warn!(%error, "BattlePay: transfer target capacity lookup failed");
            return Err((
                vas_error::MAX_CHARACTERS_ON_SERVER,
                "capacity lookup failed",
            ));
        }
    }
    Ok(VasTransferTargetLikeCpp {
        account_id: to_account,
        battlenet_account_id: to_bnet,
        realm_id: identity.realm_id,
    })
}

/// `CMSG_VAS_CHECK_TRANSFER_OK` 0x3713 (LegionCore
/// `HandleBattlePayValidateBnetVasTransfer`): the store's "other Battle.net
/// account" field, answered with that account's game accounts.
pub(crate) async fn handle_vas_check_transfer_ok<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    request: VasCheckTransferOk,
) {
    let identity = session.battle_pay_identity();
    if !service.config.is_available_for_like_cpp(identity.security) {
        return;
    }
    let mut response = VasCheckTransferOkResponse {
        client_token: request.client_token,
        result: TRANSFER_VALIDATION_FAILED_LIKE_CPP,
        ..VasCheckTransferOkResponse::default()
    };
    let email = request.bnet_account_name.trim().to_owned();
    if email.is_empty() || email.len() > 320 || !email.contains('@') {
        session.send_battle_pay_packet(&response);
        return;
    }
    match service
        .distributions
        .load_bnet_game_accounts_like_cpp(email)
        .await
    {
        // The own Battle.net account is not a "different" destination: the other
        // drop-down moves between game accounts of the same Battle.net account.
        Ok(Some(found))
            if found.battlenet_account_id != identity.battlenet_account_id
                && !found.game_accounts.is_empty() =>
        {
            response.result = TRANSFER_VALIDATION_OK_LIKE_CPP;
            response.bnet_account_guid = ObjectGuid::create_global(
                HighGuid::BNetAccount,
                0,
                i64::from(found.battlenet_account_id),
            );
            response.game_accounts = found
                .game_accounts
                .into_iter()
                .map(|(account, name)| VasTransferGameAccount {
                    guid: ObjectGuid::create_global(HighGuid::WowAccount, 0, i64::from(account)),
                    name,
                })
                .collect();
        }
        Ok(_) => {}
        Err(error) => warn!(%error, "BattlePay: Battle.net transfer validation failed"),
    }
    session.send_battle_pay_packet(&response);
}

/// LegionCore `CompleteVasCharacterTransfer`, same-realm branch.
pub(crate) async fn deliver_transfer<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    order: &PaidOrderLikeCpp,
    faction_change: bool,
) -> DeliveryOutcomeLikeCpp {
    let identity = session.battle_pay_identity();
    let Some(target) = order.transfer else {
        return DeliveryOutcomeLikeCpp::Deferred("the order names no transfer target");
    };
    if order.character_guid == 0 {
        return DeliveryOutcomeLikeCpp::Deferred("the order names no character");
    }
    if target.realm_id != identity.realm_id {
        return DeliveryOutcomeLikeCpp::Deferred("cross-realm transfers are not delivered");
    }
    let already_delivered = match receipt_exists(service, order).await {
        Ok(exists) => exists,
        Err(outcome) => return outcome,
    };
    let name = service
        .characters
        .load_character_like_cpp(order.character_guid)
        .await
        .ok()
        .flatten()
        .map(|row| row.name)
        .unwrap_or_default();
    if !already_delivered {
        let receipt = BattlePayDeliveryReceiptLikeCpp {
            external_id: order.external_id.clone(),
            account_id: identity.account_id,
            character_guid: order.character_guid,
            product_id: order.product_id,
        };
        let transfer = BattlePayCharacterTransferLikeCpp {
            character_guid: order.character_guid,
            from_account_id: identity.account_id,
            to_account_id: target.account_id,
            add_at_login_flags: if faction_change {
                at_login::CHANGE_FACTION
            } else {
                0
            },
        };
        match service
            .characters
            .persist_transfer_delivery_like_cpp(receipt, transfer)
            .await
        {
            PersistenceOutcomeLikeCpp::Applied { .. } => {}
            outcome => {
                warn!(order = %order.external_id, ?outcome, "BattlePay: transfer did not commit");
                return DeliveryOutcomeLikeCpp::Deferred("transfer did not commit");
            }
        }
    }
    mark_delivered(service, order).await;
    let character = player_guid_like_cpp(identity.realm_id, order.character_guid);
    send_vas_complete(session, order.product_id, character, &name);
    info!(
        account = identity.account_id,
        to_account = target.account_id,
        character = order.character_guid,
        faction_change,
        order = %order.external_id,
        already_delivered,
        "BattlePay: character transferred"
    );
    if already_delivered {
        DeliveryOutcomeLikeCpp::AlreadyDelivered
    } else {
        DeliveryOutcomeLikeCpp::Delivered
    }
}
