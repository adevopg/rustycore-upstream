// Copyright (c) 2026 alseif0x
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! In-game shop (BattlePay) opcode registrations.
//!
//! C++ 3.4.3 only registers `CMSG_BATTLE_PAY_GET_PRODUCT_LIST` (Authed,
//! ThreadUnsafe); every other shop opcode is `Handle_NULL` there. The shop works
//! from the character screen and in game, so every registration is `Authed`
//! (`ThreadUnsafe`, like LegionCore `Opcodes.cpp:246-264`); `UpdateVasPurchaseStates`
//! keeps the C++ `PROCESS_INPLACE`. The VAS character/realm lists, service status,
//! transfer validation, VAS purchase, boost assignment and unrevoke opcodes are
//! LegionCore's VAS handlers on their 54261 opcodes. The logic lives in
//! `crate::battle_pay`.

use tracing::warn;
use wow_constants::ClientOpcodes;
use wow_handler::{PacketProcessing, SessionStatus};
use wow_packet::ClientPacket;
use wow_packet::packets::battlepay::{
    BattlePayAckFailedResponse, BattlePayCancelOpenCheckout, BattlePayConfirmPurchaseResponse,
    BattlePayDistributionAssignToTarget, BattlePayOpenCheckout, BattlePayPurchaseSubmitted,
    BattlePayRequestPriceInfo, BattlePayStartPurchase, BattlePayStartVasPurchase,
    CharacterUpgradeManualUnrevokeRequest, GetVasAccountCharacterList,
    GetVasTransferTargetRealmList, VasCheckTransferOk,
};

use crate::battle_pay;
use crate::session::registry::PacketHandlerEntry;

fn read_or_warn<T: ClientPacket>(
    opcode: ClientOpcodes,
    pkt: &mut wow_packet::WorldPacket,
) -> Option<T> {
    match T::read(pkt) {
        Ok(packet) => Some(packet),
        Err(error) => {
            warn!("Failed to read {opcode:?}: {error}");
            None
        }
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::BattlePayGetProductList,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_battle_pay_get_product_list",
        handler: |session, catalogs, _pkt| {
            Box::pin(async move {
                battle_pay::handle_get_product_list(
                    session,
                    &catalogs.battle_pay,
                    &catalogs.id_generators.item,
                )
                .await;
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::BattlePayGetPurchaseList,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_battle_pay_get_purchase_list",
        handler: |session, _catalogs, _pkt| {
            Box::pin(async move { battle_pay::handle_get_purchase_list(session) })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::UpdateVasPurchaseStates,
        status: SessionStatus::Authed,
        processing: PacketProcessing::Inplace,
        handler_name: "handle_update_vas_purchase_states",
        handler: |session, catalogs, _pkt| {
            Box::pin(async move {
                battle_pay::handle_update_vas_purchase_states(session, &catalogs.battle_pay)
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::BattlePayStartPurchase,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_battle_pay_start_purchase",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) = read_or_warn::<BattlePayStartPurchase>(
                    ClientOpcodes::BattlePayStartPurchase,
                    &mut pkt,
                ) {
                    battle_pay::handle_start_purchase(session, &catalogs.battle_pay, request).await;
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::BattlePayConfirmPurchaseResponse,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_battle_pay_confirm_purchase_response",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) = read_or_warn::<BattlePayConfirmPurchaseResponse>(
                    ClientOpcodes::BattlePayConfirmPurchaseResponse,
                    &mut pkt,
                ) {
                    battle_pay::handle_confirm_purchase_response(
                        session,
                        &catalogs.battle_pay,
                        &catalogs.id_generators.item,
                        request,
                    )
                    .await;
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::BattlePayAckFailedResponse,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_battle_pay_ack_failed_response",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) = read_or_warn::<BattlePayAckFailedResponse>(
                    ClientOpcodes::BattlePayAckFailedResponse,
                    &mut pkt,
                ) {
                    battle_pay::handle_ack_failed_response(session, &catalogs.battle_pay, request);
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::BattlePayOpenCheckout,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_battle_pay_open_checkout",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) = read_or_warn::<BattlePayOpenCheckout>(
                    ClientOpcodes::BattlePayOpenCheckout,
                    &mut pkt,
                ) {
                    battle_pay::handle_open_checkout(session, &catalogs.battle_pay, request).await;
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::BattlePayPurchaseSubmitted,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_battle_pay_purchase_submitted",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) = read_or_warn::<BattlePayPurchaseSubmitted>(
                    ClientOpcodes::BattlePayPurchaseSubmitted,
                    &mut pkt,
                ) {
                    battle_pay::handle_purchase_submitted(
                        session,
                        &catalogs.battle_pay,
                        &catalogs.id_generators.item,
                        request,
                    )
                    .await;
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::BattlePayCancelOpenCheckout,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_battle_pay_cancel_open_checkout",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) = read_or_warn::<BattlePayCancelOpenCheckout>(
                    ClientOpcodes::BattlePayCancelOpenCheckout,
                    &mut pkt,
                ) {
                    battle_pay::handle_cancel_open_checkout(
                        session,
                        &catalogs.battle_pay,
                        &catalogs.id_generators.item,
                        request,
                    )
                    .await;
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::BattlePayRequestPriceInfo,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_battle_pay_request_price_info",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if read_or_warn::<BattlePayRequestPriceInfo>(
                    ClientOpcodes::BattlePayRequestPriceInfo,
                    &mut pkt,
                )
                .is_some()
                {
                    battle_pay::handle_request_price_info(session, &catalogs.battle_pay).await;
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::GetVasAccountCharacterList,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_get_vas_account_character_list",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) = read_or_warn::<GetVasAccountCharacterList>(
                    ClientOpcodes::GetVasAccountCharacterList,
                    &mut pkt,
                ) {
                    battle_pay::handle_get_vas_account_character_list(
                        session,
                        &catalogs.battle_pay,
                        request,
                    )
                    .await;
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::GetVasTransferTargetRealmList,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_get_vas_transfer_target_realm_list",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) = read_or_warn::<GetVasTransferTargetRealmList>(
                    ClientOpcodes::GetVasTransferTargetRealmList,
                    &mut pkt,
                ) {
                    battle_pay::handle_get_vas_transfer_target_realm_list(
                        session,
                        &catalogs.battle_pay,
                        request,
                    );
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::VasGetServiceStatus,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_vas_get_service_status",
        handler: |session, _catalogs, _pkt| {
            Box::pin(async move { battle_pay::handle_vas_get_service_status(session) })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::VasCheckTransferOk,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_vas_check_transfer_ok",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) =
                    read_or_warn::<VasCheckTransferOk>(ClientOpcodes::VasCheckTransferOk, &mut pkt)
                {
                    battle_pay::handle_vas_check_transfer_ok(session, &catalogs.battle_pay, request)
                        .await;
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::BattlePayStartVasPurchase,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_battle_pay_start_vas_purchase",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) = read_or_warn::<BattlePayStartVasPurchase>(
                    ClientOpcodes::BattlePayStartVasPurchase,
                    &mut pkt,
                ) {
                    battle_pay::handle_start_vas_purchase(session, &catalogs.battle_pay, request)
                        .await;
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::BattlePayDistributionAssignToTarget,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_battle_pay_distribution_assign_to_target",
        handler: |session, catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) = read_or_warn::<BattlePayDistributionAssignToTarget>(
                    ClientOpcodes::BattlePayDistributionAssignToTarget,
                    &mut pkt,
                ) {
                    battle_pay::handle_distribution_assign_to_target(
                        session,
                        &catalogs.battle_pay,
                        request,
                    )
                    .await;
                }
            })
        },
    }
}

inventory::submit! {
    PacketHandlerEntry {
        opcode: ClientOpcodes::CharacterUpgradeManualUnrevokeRequest,
        status: SessionStatus::Authed,
        processing: PacketProcessing::ThreadUnsafe,
        handler_name: "handle_character_upgrade_manual_unrevoke_request",
        handler: |session, _catalogs, mut pkt| {
            Box::pin(async move {
                if let Some(request) = read_or_warn::<CharacterUpgradeManualUnrevokeRequest>(
                    ClientOpcodes::CharacterUpgradeManualUnrevokeRequest,
                    &mut pkt,
                ) {
                    battle_pay::handle_character_upgrade_manual_unrevoke_request(session, request);
                }
            })
        },
    }
}
