// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! In-game shop (BattlePay) for client 3.4.3.54261.
//!
//! Port of the LegionCore 7.3.5 shop (`/home/inna/legioncore-ref`:
//! `BattlePay/BattlePayMgr.{h,cpp}`, `Globals/BattlePayData.cpp`,
//! `Handlers/BattlePayHandler.cpp`, `Player::ChangeTokenCount`) onto the 3.4.3
//! packet layouts reverse-engineered in `docs/migration/battlepay-343-protocol.md`
//! (codec: `wow_packet::packets::battlepay`). Two purchase modes, selected by
//! `Bpay.WebCheckout`: a token wallet (`auth.account_tokens`) confirmed in game, or
//! a real-money checkout opened in the in-game browser with an SSO token.
//!
//! Scope and deliberate departures are listed in the protocol document, section 8.

mod catalog;
mod constants;
mod flow;
mod service;
mod session_port;

pub use catalog::{BattlePayCatalogLikeCpp, BattlePayCatalogLoadReportLikeCpp};
pub use constants::BattlePayConfigLikeCpp;
pub use service::BattlePayServiceLikeCpp;

pub(crate) use constants::PRODUCT_DELIVERY_DELAY_SECS_LIKE_CPP;

/// Shop packets of the session-init burst (LegionCore `SendDisplayPromo`); sent right
/// after the character-select init packets.
pub fn send_session_init_packets_like_cpp(
    session: &crate::session::WorldSession,
    service: &BattlePayServiceLikeCpp,
) {
    flow::send_session_init(session, service);
}
pub(crate) use flow::{
    handle_ack_failed_response, handle_cancel_open_checkout, handle_confirm_purchase_response,
    handle_get_product_list, handle_get_purchase_list, handle_open_checkout,
    handle_purchase_submitted, handle_request_price_info, handle_start_purchase,
    handle_update_vas_purchase_states,
};

#[cfg(test)]
#[path = "battle_pay/tests/mod.rs"]
mod tests;
