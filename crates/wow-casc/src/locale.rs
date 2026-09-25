//! CASC locale bits (`CascLib` `CASC_LOCALE_*`) and the `TrinityCore` `LocaleConstant`
//! mapping used by the extractors (`WowLocaleToCascLocaleBit`).

pub const NONE: u32 = 0x0000_0000;
pub const UNKNOWN1: u32 = 0x0000_0001;
pub const ENUS: u32 = 0x0000_0002;
pub const KOKR: u32 = 0x0000_0004;
pub const RESERVED: u32 = 0x0000_0008;
pub const FRFR: u32 = 0x0000_0010;
pub const DEDE: u32 = 0x0000_0020;
pub const ZHCN: u32 = 0x0000_0040;
pub const ESES: u32 = 0x0000_0080;
pub const ZHTW: u32 = 0x0000_0100;
pub const ENGB: u32 = 0x0000_0200;
pub const ENCN: u32 = 0x0000_0400;
pub const ENTW: u32 = 0x0000_0800;
pub const ESMX: u32 = 0x0000_1000;
pub const RURU: u32 = 0x0000_2000;
pub const PTBR: u32 = 0x0000_4000;
pub const ITIT: u32 = 0x0000_8000;
pub const PTPT: u32 = 0x0001_0000;
pub const ALL: u32 = 0xFFFF_FFFF;

/// `TrinityCore` `LocaleConstant` names, indexed by locale id (`enUS` = 0 .. `itIT` = 11).
pub const TC_LOCALE_NAMES: [&str; 12] = [
    "enUS", "koKR", "frFR", "deDE", "zhCN", "zhTW", "esES", "esMX", "ruRU", "none", "ptBR", "itIT",
];

/// `TrinityCore` `WowLocaleToCascLocaleBit` (`src/common/Common.cpp`), indexed by
/// `LocaleConstant`: the CASC locale *bit index* (`CascLocaleBit`), not a mask.
/// `LOCALE_none` (9) maps to `CascLocaleBit::None` (0) and is never extracted.
pub const TC_LOCALE_TO_CASC_BIT: [u8; 12] = [1, 2, 4, 5, 6, 8, 7, 12, 13, 0, 14, 15];

/// CASC locale mask for a `TrinityCore` `LocaleConstant` (`1 << WowLocaleToCascLocaleBit[l]`).
pub const fn tc_locale_mask(tc_locale: usize) -> u32 {
    1 << TC_LOCALE_TO_CASC_BIT[tc_locale]
}
