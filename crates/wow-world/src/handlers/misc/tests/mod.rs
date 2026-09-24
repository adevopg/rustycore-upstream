//! Behaviour tests for the miscellaneous capability handlers.
//!
//! The shared entries/packets/session fixtures now live in
//! [`crate::handlers::test_support`]; each scenario module below still
//! exercises the production module next to it through `use super::*;`.

use crate::handlers::test_support::*;

mod account_data;
mod arena;
mod auction;
mod battle_pet;
mod calendar;
mod chat;
mod client_state;
mod collections;
mod corpse;
mod gameobject;
mod guild;
mod instance;
mod lfg;
mod player;
mod pvp;
mod reputation;
mod support;
mod trade;
mod travel;
