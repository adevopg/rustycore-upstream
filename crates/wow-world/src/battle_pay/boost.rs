//! Character boosts: distributions, assignment from character select and the
//! boost applied at the character's next login.
//!
//! LegionCore anchors: `AddDistribution`, `LoadDistributions`, `WriteDistribution`,
//! `SendDistributionList`, `SendBattlePayDistribution`,
//! `AssignDistributionToCharacter` + callback (`BattlePayMgr.cpp:943-1200`),
//! `HandleCharacterBoostLoginCallback` (`BattlePayHandler.cpp:231`) and
//! `CharacterService::ApplyBoost` / `GetBoostLoadoutItems`. 54261 client evidence
//! (`docs/migration/battlepay-343-protocol.md`, section 9.4):
//! `C_CharacterServices.AssignUpgradeDistribution` sends CMSG 0x36cb with
//! `ProductChoice = faction << 24 | specID` for the first available (status 1, not
//! revoked) distribution whose product has Type 1 and the requested boost type
//! (`0x14169aaa0` -> `0x142583430`); the boost types, levels and loadouts are the
//! WotLK Classic `CharacterServiceInfo`/`CharacterLoadout` rows (`constants.rs`).

use tracing::{info, warn};
use wow_core::{ObjectGuid, ObjectGuidGenerator};
use wow_packet::packets::battlepay::{
    BattlePayDistributionAssignToTarget, BattlePayDistributionObject, BattlePayDistributionUpdate,
    BattlePayGetDistributionListResponse, BattlePayStartDistributionAssignToTargetResponse,
    CharacterUpgradeComplete, CharacterUpgradeManualUnrevokeRequest,
    CharacterUpgradeManualUnrevokeResult, CharacterUpgradeStarted,
};
use wow_persistence::{
    BATTLE_PAY_DISTRIBUTION_STATUS_ASSIGNED_LIKE_CPP,
    BATTLE_PAY_DISTRIBUTION_STATUS_AVAILABLE_LIKE_CPP,
    BATTLE_PAY_DISTRIBUTION_STATUS_FINISHED_LIKE_CPP, BattlePayBoostCompletionLikeCpp,
    BattlePayDeliveryReceiptLikeCpp, BattlePayDistributionAssignLikeCpp,
    BattlePayDistributionGrantLikeCpp, BattlePayDistributionRowLikeCpp, PersistenceOutcomeLikeCpp,
};

use super::catalog::BattlePayProductLikeCpp;
use super::constants::*;
use super::flow::{
    BattlePayIdentityLikeCpp, BattlePaySessionLikeCpp, DeliveryOutcomeLikeCpp, PaidOrderLikeCpp,
};
use super::product_kind::ProductKindLikeCpp;
use super::service::BattlePayServiceLikeCpp;
use super::vas::player_guid_like_cpp;

/// LegionCore `Battlepay::Error::PurchaseDenied`: any non-zero result makes the
/// client fire PRODUCT_ASSIGN_TO_TARGET_FAILED (handler `0x141a49fd0`).
const ASSIGN_DENIED_LIKE_CPP: u32 = 1;
/// CHARACTER_UPGRADE_UNREVOKE_RESULT failure (LegionCore `UnrevokeResult{1}`).
const UNREVOKE_FAILED_LIKE_CPP: u32 = 1;
/// Hearthstone: kept out of the boost gear when the character already has one
/// (LegionCore `ApplyBoost`).
const HEARTHSTONE_ITEM_LIKE_CPP: u32 = 6948;

fn boost_of(product: &BattlePayProductLikeCpp) -> Option<BoostDefinitionLikeCpp> {
    match product.kind_like_cpp() {
        ProductKindLikeCpp::Boost(boost) => Some(boost),
        _ => None,
    }
}

/// LegionCore `BattlepayManager::WriteDistribution`.
fn distribution_object(
    service: &BattlePayServiceLikeCpp,
    identity: &BattlePayIdentityLikeCpp,
    row: &BattlePayDistributionRowLikeCpp,
) -> Option<BattlePayDistributionObject> {
    let product = service.catalog.product(row.product_id)?;
    let target_player = if row.character_guid == 0 {
        ObjectGuid::EMPTY
    } else {
        player_guid_like_cpp(identity.realm_id, row.character_guid)
    };
    let target_realm = if target_player.is_empty() {
        0
    } else {
        identity.virtual_realm_address
    };
    Some(BattlePayDistributionObject {
        distribution_id: row.id,
        status: u32::from(row.status),
        product_id: row.product_id,
        target_player,
        target_virtual_realm: target_realm,
        target_native_realm: target_realm,
        product: Some(
            service
                .catalog
                .distribution_product_like_cpp(product, identity.locale),
        ),
        revoked: row.revoked,
        ..BattlePayDistributionObject::default()
    })
}

/// `SMSG_BATTLE_PAY_GET_DISTRIBUTION_LIST_RESPONSE` (LegionCore
/// `LoadDistributions` + `SendDistributionList`). Distributions of other realms
/// and of unknown products are skipped like LegionCore's loader.
///
/// The 54261 store keeps its "Loading" alert until `C_StoreSecure.HasPurchaseList()`,
/// `HasProductList()` and `HasDistributionList()` are all true
/// (`Blizzard_StoreUISecure.lua` `StoreFrame_UpdateActivePanel`), so the client must
/// receive this list even when it is empty or unreadable.
pub(crate) async fn send_distribution_list<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
) {
    let identity = session.battle_pay_identity();
    let rows = match service
        .distributions
        .load_distributions_like_cpp(identity.account_id)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            warn!(account = identity.account_id, %error, "BattlePay: distributions unavailable");
            Vec::new()
        }
    };
    let distribution_objects = rows
        .iter()
        .filter(|row| row.realm_id == 0 || row.realm_id == identity.realm_id)
        .filter_map(|row| distribution_object(service, &identity, row))
        .take((1 << 11) - 1)
        .collect();
    session.send_battle_pay_packet(&BattlePayGetDistributionListResponse {
        result: error::OK,
        distribution_objects,
    });
}

fn send_distribution_update<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    identity: &BattlePayIdentityLikeCpp,
    row: &BattlePayDistributionRowLikeCpp,
) {
    if let Some(distribution_object) = distribution_object(service, identity, row) {
        session.send_battle_pay_packet(&BattlePayDistributionUpdate {
            distribution_object,
        });
    }
}

/// LegionCore `ProcessDelivery` `WebsiteType::CharacterBoost`: the paid order
/// becomes an available distribution (`AddDistribution`), committed with the
/// order's `Paid -> Delivered` transition.
pub(crate) async fn deliver_boost<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    order: &PaidOrderLikeCpp,
    product: &BattlePayProductLikeCpp,
) -> DeliveryOutcomeLikeCpp {
    let identity = session.battle_pay_identity();
    let distribution_id = service.next_distribution_id_like_cpp();
    match service
        .distributions
        .grant_distribution_like_cpp(BattlePayDistributionGrantLikeCpp {
            distribution_id,
            external_id: order.external_id.clone(),
            web_order_id: order.web_order_id.clone(),
        })
        .await
    {
        PersistenceOutcomeLikeCpp::Applied { .. } => {}
        outcome => {
            warn!(order = %order.external_id, ?outcome, "BattlePay: boost distribution did not commit");
            return DeliveryOutcomeLikeCpp::Deferred("boost distribution did not commit");
        }
    }
    send_distribution_update(
        session,
        service,
        &identity,
        &BattlePayDistributionRowLikeCpp {
            id: distribution_id,
            product_id: product.product_id,
            status: BATTLE_PAY_DISTRIBUTION_STATUS_AVAILABLE_LIKE_CPP,
            ..BattlePayDistributionRowLikeCpp::default()
        },
    );
    info!(
        account = identity.account_id,
        product = product.product_id,
        distribution = distribution_id,
        order = %order.external_id,
        "BattlePay: character boost delivered as a distribution"
    );
    DeliveryOutcomeLikeCpp::Delivered
}

fn send_assign_result<S: BattlePaySessionLikeCpp>(
    session: &S,
    identity: &BattlePayIdentityLikeCpp,
    distribution_id: u64,
    result: u32,
    why: &str,
) {
    if result != error::OK {
        info!(
            account = identity.account_id,
            distribution = distribution_id,
            "BattlePay: boost assignment refused: {why}"
        );
    }
    session.send_battle_pay_packet(&BattlePayStartDistributionAssignToTargetResponse {
        distribution_id,
        result,
        unk: 0,
    });
}

/// `CMSG_BATTLE_PAY_DISTRIBUTION_ASSIGN_TO_TARGET` 0x36cb (LegionCore
/// `HandleBattlePayDistributionAssign` -> `AssignDistributionToCharacter`).
pub(crate) async fn handle_distribution_assign_to_target<S: BattlePaySessionLikeCpp>(
    session: &S,
    service: &BattlePayServiceLikeCpp,
    request: BattlePayDistributionAssignToTarget,
) {
    let identity = session.battle_pay_identity();
    if !service.config.is_available_for_like_cpp(identity.security) {
        return;
    }
    let distribution_id = request.distribution_id;
    let deny = |why: &str| {
        send_assign_result(
            session,
            &identity,
            distribution_id,
            ASSIGN_DENIED_LIKE_CPP,
            why,
        )
    };
    let rows = match service
        .distributions
        .load_distributions_like_cpp(identity.account_id)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            warn!(%error, "BattlePay: distributions unavailable");
            return deny("distributions unavailable");
        }
    };
    let Some(row) = rows.into_iter().find(|row| row.id == distribution_id) else {
        return deny("unknown distribution");
    };
    if row.revoked || row.status != BATTLE_PAY_DISTRIBUTION_STATUS_AVAILABLE_LIKE_CPP {
        return deny("distribution not available");
    }
    let Some(boost) = service.catalog.product(row.product_id).and_then(boost_of) else {
        return deny("the distribution is not a character boost");
    };
    let character_guid = request.target_character.counter() as u64;
    let character = match service
        .characters
        .load_character_like_cpp(character_guid)
        .await
    {
        Ok(Some(character)) if character.account_id == identity.account_id => character,
        Ok(_) => return deny("the character is not of this account"),
        Err(error) => {
            warn!(%error, "BattlePay: boost target lookup failed");
            return deny("character lookup failed");
        }
    };
    let in_world = identity
        .player
        .is_some_and(|player| player.guid.counter() as u64 == character_guid);
    if character.online || in_world {
        return deny("the character is online");
    }
    if character.level >= boost.level {
        return deny("the character is not below the boost level");
    }
    if service
        .boost_loadout_like_cpp(character.class, boost.loadout_purpose)
        .is_none()
    {
        // No CharacterLoadout for this class and boost (e.g. death knights for the
        // level 58 boost): the client restricts the class the same way.
        return deny("no boost loadout for the class");
    }
    let assign = BattlePayDistributionAssignLikeCpp {
        distribution_id,
        account_id: identity.account_id,
        realm_id: identity.realm_id,
        character_guid,
        // 54261 ProductChoice = faction << 24 | specID; Wrath Classic sends spec 0.
        specialization_id: (request.product_choice & 0xFFFF) as u16,
        choice_id: 0,
    };
    match service
        .distributions
        .assign_distribution_like_cpp(assign)
        .await
    {
        PersistenceOutcomeLikeCpp::Applied { rows: 1 } => {}
        outcome => {
            warn!(?outcome, "BattlePay: boost assignment not recorded");
            return deny("distribution assignment did not apply");
        }
    }
    match service
        .characters
        .queue_character_boost_like_cpp(
            character_guid,
            identity.account_id,
            boost.level,
            at_login::CHARACTER_BOOST,
        )
        .await
    {
        PersistenceOutcomeLikeCpp::Applied { rows: 1 } => {}
        outcome => {
            warn!(
                ?outcome,
                "BattlePay: boost not queued on the character; assignment reverted"
            );
            let revert = service
                .distributions
                .unassign_distribution_like_cpp(distribution_id, identity.account_id)
                .await;
            if !revert.is_applied() {
                warn!(
                    ?revert,
                    distribution = distribution_id,
                    "BattlePay: boost assignment revert failed"
                );
            }
            return deny("the character could not be queued");
        }
    }
    info!(
        account = identity.account_id,
        distribution = distribution_id,
        character = character_guid,
        level = boost.level,
        "BattlePay: character boost assigned"
    );
    // LegionCore callback order: response, (UPGRADE_QUEUED: no 54261 opcode),
    // CHARACTER_UPGRADE_STARTED, distribution update.
    send_assign_result(session, &identity, distribution_id, error::OK, "");
    session.send_battle_pay_packet(&CharacterUpgradeStarted {
        character_guid: request.target_character,
    });
    send_distribution_update(
        session,
        service,
        &identity,
        &BattlePayDistributionRowLikeCpp {
            status: BATTLE_PAY_DISTRIBUTION_STATUS_ASSIGNED_LIKE_CPP,
            character_guid,
            realm_id: identity.realm_id,
            ..row
        },
    );
}

/// `CMSG_CHARACTER_UPGRADE_MANUAL_UNREVOKE_REQUEST` 0x36cc: boost revocation
/// (LegionCore `BattlePayRevocation`) is not ported, so no character is ever
/// revoked and there is nothing to unrevoke.
pub(crate) fn handle_character_upgrade_manual_unrevoke_request<S: BattlePaySessionLikeCpp>(
    session: &S,
    _request: CharacterUpgradeManualUnrevokeRequest,
) {
    session.send_battle_pay_packet(&CharacterUpgradeManualUnrevokeResult {
        result: UNREVOKE_FAILED_LIKE_CPP,
    });
}

/// LegionCore `CharacterHandler.cpp:1031` + `HandleCharacterBoostLoginCallback`:
/// apply the assigned boost when the boosted character enters the world. The
/// level was set when the boost was assigned (`CHAR_UPD_CHARACTER_BOOST_QUEUED`),
/// so the character already loaded at the boost level; this grants the class
/// loadout of `CharacterLoadout.db2` and the boost money, clears
/// `AT_LOGIN_CHARACTER_BOOST` and finishes the distribution.
pub(crate) async fn apply_pending_boost<S: BattlePaySessionLikeCpp>(
    session: &mut S,
    service: &BattlePayServiceLikeCpp,
    item_guid_generator: &ObjectGuidGenerator,
) {
    let identity = session.battle_pay_identity();
    let Some(player) = identity.player else {
        return;
    };
    let flags = session.battle_pay_player_at_login_flags();
    if flags & at_login::CHARACTER_BOOST == 0 {
        return;
    }
    let character_guid = player.guid.counter() as u64;
    let pending = match service
        .distributions
        .load_pending_distribution_like_cpp(character_guid, identity.realm_id)
        .await
    {
        Ok(pending) => pending,
        Err(error) => {
            warn!(%error, "BattlePay: pending boost unavailable; retried at the next login");
            return;
        }
    };
    let boost = pending.as_ref().and_then(|row| {
        service
            .catalog
            .product(row.product_id)
            .and_then(boost_of)
            .map(|boost| (row.clone(), boost))
    });
    let Some((row, boost)) = boost else {
        // LegionCore `RemoveAtLoginFlag(AT_LOGIN_CHARACTER_BOOST, true)`; no row ->
        // log and return. The player's next save persists the cleared flag.
        warn!(
            character = character_guid,
            "BattlePay: boost flag without an assigned boost; flag cleared"
        );
        session.battle_pay_set_player_at_login_flags(flags & !at_login::CHARACTER_BOOST);
        return;
    };
    let items: Vec<(u32, u32)> = service
        .boost_loadout_like_cpp(player.class, boost.loadout_purpose)
        .unwrap_or_default()
        .iter()
        .filter(|item| {
            **item != HEARTHSTONE_ITEM_LIKE_CPP
                || session.battle_pay_item_count(HEARTHSTONE_ITEM_LIKE_CPP) == 0
        })
        .map(|item| (*item, 1))
        .collect();
    if !session.battle_pay_can_store(&items) {
        // LegionCore mails what does not fit; RustyCore has no server mail
        // sender, so the boost waits for free bag space (next login).
        warn!(
            character = character_guid,
            items = items.len(),
            "BattlePay: boost gear does not fit the bags; retried at the next login"
        );
        return;
    }
    let Some(inventory) = session
        .battle_pay_grant_items(item_guid_generator, &items)
        .await
    else {
        session.battle_pay_quarantine("BattlePay boost gear delivery diverged; relog required");
        return;
    };
    let receipt = BattlePayDeliveryReceiptLikeCpp {
        external_id: format!("boost:{}", row.id),
        account_id: identity.account_id,
        character_guid,
        product_id: row.product_id,
    };
    let completion = BattlePayBoostCompletionLikeCpp {
        character_guid,
        remove_at_login_flags: at_login::CHARACTER_BOOST,
        money: service.config.boost_money,
    };
    match service
        .characters
        .persist_boost_completion_like_cpp(receipt, completion, inventory)
        .await
    {
        PersistenceOutcomeLikeCpp::Applied { .. } => {}
        outcome => {
            warn!(?outcome, "BattlePay: boost completion did not commit");
            session.battle_pay_quarantine("BattlePay boost did not commit; relog required");
            return;
        }
    }
    session.battle_pay_add_player_money(service.config.boost_money);
    session.battle_pay_set_player_at_login_flags(flags & !at_login::CHARACTER_BOOST);
    let finished = service
        .distributions
        .finish_distribution_like_cpp(row.id)
        .await;
    if !finished.is_applied() {
        warn!(
            ?finished,
            distribution = row.id,
            "BattlePay: boost applied but the distribution was not finished"
        );
    }
    info!(
        account = identity.account_id,
        character = character_guid,
        distribution = row.id,
        level = boost.level,
        items = items.len(),
        "BattlePay: character boost applied"
    );
    session.send_battle_pay_packet(&CharacterUpgradeComplete {
        character_guid: player.guid,
        values: Vec::new(),
        unk_bit: false,
    });
    send_distribution_update(
        session,
        service,
        &identity,
        &BattlePayDistributionRowLikeCpp {
            status: BATTLE_PAY_DISTRIBUTION_STATUS_FINISHED_LIKE_CPP,
            ..row
        },
    );
}
