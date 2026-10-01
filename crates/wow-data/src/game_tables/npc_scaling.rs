// Copyright (c) 2026 alseif0x
// RustyCore - WoW WotLK 3.4.3 server in Rust
// Licensed under GPL v3

//! C++ `DataStores/GameTables.*` creature-scaling table.
//!
//! This module owns `NPCManaCostScaler.txt`, the table
//! `SpellEffectInfo::CalcValue` reads to scale a creature spell's value from the
//! spell's own level to the caster's (`Spells/SpellInfo.cpp:586-592`). Like every
//! other game table it is indexed by row position rather than by the explicit
//! `Level` column, matching `LoadGameTable`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::parse_float_like_cpp;

/// C++ `GtNpcManaCostScalerEntry` (`DataStores/GameTables.h:332-335`).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NpcManaCostScalerEntryLikeCpp {
    pub scaler: f32,
}

/// C++ `sNpcManaCostScalerGameTable`.
///
/// Row 0 is the default unused entry `LoadGameTable` inserts, so a level indexes
/// its own row directly.
#[derive(Debug, Clone, PartialEq)]
pub struct NpcManaCostScalerGameTableLikeCpp {
    rows: Vec<NpcManaCostScalerEntryLikeCpp>,
}

impl NpcManaCostScalerGameTableLikeCpp {
    /// C++ `LOAD_GT(sNpcManaCostScalerGameTable, "NPCManaCostScaler.txt")`
    /// (`GameTables.cpp:127`).
    pub const FILE_NAME: &'static str = "NPCManaCostScaler.txt";
    pub const VALUE_COLUMN_COUNT: usize = 1;

    pub fn load(data_dir: impl AsRef<Path>) -> Result<Self> {
        Self::load_from_path(data_dir.as_ref().join("gt").join(Self::FILE_NAME))
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .with_context(|| format!("GameTable file {} cannot be opened.", path.display()))?;
        Self::parse_like_cpp(&content, path)
    }

    pub fn from_scalers(scalers: impl IntoIterator<Item = f32>) -> Self {
        let mut rows = vec![NpcManaCostScalerEntryLikeCpp::default()];
        rows.extend(
            scalers
                .into_iter()
                .map(|scaler| NpcManaCostScalerEntryLikeCpp { scaler }),
        );
        Self { rows }
    }

    /// C++ `GameTable::GetRow(level)`: `nullptr` past the end of the table.
    pub fn row(&self, level: u32) -> Option<&NpcManaCostScalerEntryLikeCpp> {
        self.rows.get(usize::try_from(level).ok()?)
    }

    /// C++ `value *= casterScaler->Scaler / spellScaler->Scaler`
    /// (`SpellInfo.cpp:586-592`): the factor a creature caster applies to a
    /// spell's calculated value.
    ///
    /// `None` when either level has no row, which is C++'s
    /// `if (spellScaler && casterScaler)` guard leaving the value untouched. A
    /// zero spell-level scaler also returns `None` rather than dividing by zero;
    /// C++ would produce an infinity there, and no installed row is zero.
    pub fn creature_level_value_factor_like_cpp(
        &self,
        spell_level: u32,
        caster_level: u32,
    ) -> Option<f32> {
        let spell_scaler = self.row(spell_level)?.scaler;
        let caster_scaler = self.row(caster_level)?.scaler;
        (spell_scaler != 0.0).then(|| caster_scaler / spell_scaler)
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    fn parse_like_cpp(content: &str, path: &Path) -> Result<Self> {
        let mut lines = content.lines();
        let Some(headers) = lines.next() else {
            bail!("GameTable file {} is empty.", path.display());
        };

        let column_defs: Vec<&str> = headers
            .split('\t')
            .filter(|part| !part.is_empty())
            .collect();
        if column_defs.len().saturating_sub(1) != Self::VALUE_COLUMN_COUNT {
            bail!(
                "GameTable '{}' has different count of columns {} than expected by size of C++ structure ({}).",
                path.display(),
                column_defs.len().saturating_sub(1),
                Self::VALUE_COLUMN_COUNT
            );
        }

        let mut rows = vec![NpcManaCostScalerEntryLikeCpp::default()];
        for raw_line in lines {
            let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
            let mut values: Vec<&str> = line.split('\t').collect();
            if values.is_empty() || (values.len() == 1 && values[0].is_empty()) {
                break;
            }
            while values.len() > 1 && values.last().is_some_and(|value| value.is_empty()) {
                values.pop();
            }
            if values.len() <= 1 {
                break;
            }
            if values.len() != column_defs.len() {
                bail!("{} == {}", values.len(), column_defs.len());
            }
            rows.push(NpcManaCostScalerEntryLikeCpp {
                scaler: parse_float_like_cpp(values[1]),
            });
        }

        Ok(Self { rows })
    }
}
