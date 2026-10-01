// Copyright (c) 2026 alseif0x
// RustyCore — WoW WotLK 3.4.3 server in Rust
// Based on TrinityCore protocol research (https://github.com/TrinityCore/TrinityCore)
// Licensed under GPL v3 — https://www.gnu.org/licenses/gpl-3.0.html

//! Selected `Targets` ids and predicates from C++ `SpellImplicitTargetInfo`.

pub const TARGET_DEST_HOME: u32 = 9;
pub const TARGET_DEST_DB: u32 = 17;
pub const TARGET_DEST_NEARBY_ENTRY: u32 = 46;
pub const TARGET_DEST_NEARBY_ENTRY_2: u32 = 107;
pub const TARGET_DEST_NEARBY_ENTRY_OR_DB: u32 = 142;

/// Every `Targets` id whose selection category in C++
/// `SpellImplicitTargetInfo::_data` (`Spells/SpellInfo.cpp:242-396`) is
/// `TARGET_SELECT_CATEGORY_AREA` or `TARGET_SELECT_CATEGORY_CONE` — the two
/// categories `IsArea` accepts. The cone ones are 24, 54, 59, 60, 104, 108, 109,
/// 110, 128, 129, 130 and 136; the rest are area.
const AREA_OR_CONE_TARGETS_LIKE_CPP: [u32; 36] = [
    7, 8, 15, 16, 20, 24, 30, 31, 33, 34, 37, 51, 52, 54, 56, 59, 60, 61, 93, 104, 105, 108, 109,
    110, 115, 116, 118, 119, 120, 122, 123, 128, 129, 130, 136, 151,
];

/// C++ `SpellImplicitTargetInfo::IsArea` (`Spells/SpellInfo.cpp:75-78`).
pub fn is_area_implicit_target_like_cpp(target: u32) -> bool {
    AREA_OR_CONE_TARGETS_LIKE_CPP.contains(&target)
}
