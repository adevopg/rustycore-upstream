// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Selected `SpellAuraInterruptFlags` bits (`Spells/SpellDefines.h:76-100`).

/// C++ `SpellAuraInterruptFlags::LeaveWorld` (`SpellDefines.h:92`). The save runs
/// before the Player is removed from the world, so
/// `SpellMgr::LoadSpellInfoCustomAttributes` (`SpellMgr.cpp:3604-3605`) marks
/// every aura carrying it as one `_SaveAuras` must skip.
pub const LEAVE_WORLD_LIKE_CPP: u32 = 0x0008_0000;
