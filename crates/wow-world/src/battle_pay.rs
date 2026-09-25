// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! In-game shop (BattlePay) for client 3.4.3.54261.
//!
//! Port of the LegionCore 7.3.5 shop (`/home/inna/legioncore-ref`:
//! `BattlePay/BattlePayMgr.{h,cpp}`, `BattlePay/CharacterService.cpp`,
//! `Globals/BattlePayData.cpp`, `Handlers/BattlePayHandler.cpp`,
//! `Player::ChangeTokenCount`) onto the 3.4.3 packet layouts reverse-engineered in
//! `docs/migration/battlepay-343-protocol.md` (codec: `wow_packet::packets::battlepay`).
//! Two purchase modes, selected by `Bpay.WebCheckout`: a token wallet
//! (`auth.account_tokens`) confirmed in game, or a real-money checkout opened in the
//! in-game browser with an SSO token. Products: items, the character services
//! (name, appearance, faction, race; restore deleted character), same-realm
//! character transfers and WotLK Classic character boosts.
//!
//! Scope and deliberate departures are listed in the protocol document, sections 8-9.

mod boost;
mod catalog;
mod constants;
mod flow;
mod product_kind;
mod service;
mod session_port;
mod transfer;
mod vas;
mod web;

pub use catalog::{BattlePayCatalogLikeCpp, BattlePayCatalogLoadReportLikeCpp};
pub use constants::BattlePayConfigLikeCpp;
pub use service::BattlePayServiceLikeCpp;

pub(crate) use constants::PRODUCT_DELIVERY_DELAY_SECS_LIKE_CPP;

/// Shop packets of the session-init burst (LegionCore `SendDisplayPromo`); sent right
/// after the character-select init packets.
pub async fn send_session_init_packets_like_cpp(
    session: &crate::session::WorldSession,
    service: &BattlePayServiceLikeCpp,
) {
    flow::send_session_init(session, service).await;
}

/// LegionCore `CharacterHandler.cpp:1031`: after a character entered the world,
/// apply its assigned character boost.
pub(crate) async fn after_player_login_like_cpp(
    session: &mut crate::session::WorldSession,
    service: &BattlePayServiceLikeCpp,
    item_guid_generator: &wow_core::ObjectGuidGenerator,
) {
    boost::apply_pending_boost(session, service, item_guid_generator).await;
}

pub(crate) use boost::{
    handle_character_upgrade_manual_unrevoke_request, handle_distribution_assign_to_target,
};
pub(crate) use flow::{
    handle_ack_failed_response, handle_confirm_purchase_response, handle_get_product_list,
    handle_get_purchase_list, handle_request_price_info, handle_start_purchase,
};
pub(crate) use transfer::handle_vas_check_transfer_ok;
pub(crate) use vas::{
    handle_get_vas_account_character_list, handle_get_vas_transfer_target_realm_list,
    handle_start_vas_purchase, handle_update_vas_purchase_states, handle_vas_get_service_status,
};
pub(crate) use web::{
    handle_cancel_open_checkout, handle_open_checkout, handle_purchase_submitted,
};

#[cfg(test)]
#[path = "battle_pay/tests/mod.rs"]
mod tests;
