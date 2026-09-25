//! Composition of the immutable data `HandleCharRaceOrFactionChangeCallback` reads
//! from `sObjectMgr`, `sDB2Manager` and `sWorld` (TDB343.24081).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use tracing::{info, warn};
use wow_config::WorldConfigSet;
use wow_data::wdc4::Wdc4Reader;
use wow_world::character_race_faction_change::RaceFactionChangeCatalogLikeCpp;

/// `TaxiNodeFlags::ShowOnAllianceMap` / `ShowOnHordeMap` (`DBCEnums.h:2098`).
const TAXI_NODE_FLAG_ALLIANCE_LIKE_CPP: i32 = 0x1;
const TAXI_NODE_FLAG_HORDE_LIKE_CPP: i32 = 0x2;
/// `TaxiNodesEntry::IsPartOfTaxiNetwork` whitelist.
const TAXI_NETWORK_HIDDEN_NODES_LIKE_CPP: [u32; 7] = [1985, 1986, 1987, 2627, 2628, 2732, 2835];
/// TaxiNodes.db2 / CharTitles.db2 field indices of the 54261 files
/// (`TaxiNodesLoadInfo` Flags, `CharTitlesLoadInfo` MaskID).
const TAXI_NODES_FLAGS_FIELD: usize = 8;
const CHAR_TITLES_MASK_ID_FIELD: usize = 2;

fn open_db2(data_dir: &str, locale: &str, file: &str) -> Option<Wdc4Reader> {
    let path = Path::new(data_dir).join("dbc").join(locale).join(file);
    match Wdc4Reader::open(&path) {
        Ok(reader) => Some(reader),
        Err(error) => {
            warn!(
                "race/faction change: cannot open {}: {error:#}",
                path.display()
            );
            None
        }
    }
}

/// `sHordeTaxiNodesMask` / `sAllianceTaxiNodesMask` of `DB2Manager::LoadStores`
/// (`TaxiMask` size = `((GetNumRows() - 1) / 64 + 1) * 8` bytes).
fn taxi_masks_like_cpp(reader: &Wdc4Reader) -> (Vec<u8>, Vec<u8>) {
    let nodes: Vec<(u32, i32)> = reader
        .iter_records()
        .map(|(id, idx)| (id, reader.get_field_i32(idx, TAXI_NODES_FLAGS_FIELD)))
        .collect();
    let num_rows = nodes.iter().map(|(id, _)| id + 1).max().unwrap_or(0);
    let size = if num_rows == 0 {
        0
    } else {
        (((num_rows - 1) / 64 + 1) * 8) as usize
    };
    let (mut horde, mut alliance) = (vec![0_u8; size], vec![0_u8; size]);
    for (id, flags) in nodes {
        let in_network = flags & (TAXI_NODE_FLAG_ALLIANCE_LIKE_CPP | TAXI_NODE_FLAG_HORDE_LIKE_CPP)
            != 0
            || TAXI_NETWORK_HIDDEN_NODES_LIKE_CPP.contains(&id);
        if !in_network || id == 0 {
            continue;
        }
        let field = ((id - 1) / 8) as usize;
        let submask = 1_u8 << ((id - 1) % 8);
        if flags & TAXI_NODE_FLAG_HORDE_LIKE_CPP != 0 {
            horde[field] |= submask;
        }
        if flags & TAXI_NODE_FLAG_ALLIANCE_LIKE_CPP != 0 {
            alliance[field] |= submask;
        }
    }
    (horde, alliance)
}

pub(crate) struct RaceFactionChangeInputsLikeCpp<'a> {
    pub faction_change: Arc<wow_data::FactionChangeStoreLikeCpp>,
    pub chr_races: &'a wow_data::character_progression::ChrRacesStore,
    pub quests: &'a wow_data::quest::QuestStore,
    pub factions: &'a Arc<wow_data::progression_rewards::FactionStore>,
    pub reserved_names: &'a Arc<wow_data::ReservedNameStoreLikeCpp>,
    pub data_dir: &'a str,
    pub locale: &'a str,
    pub world_configs: &'a WorldConfigSet,
}

pub(crate) fn build_catalog_like_cpp(
    inputs: RaceFactionChangeInputsLikeCpp<'_>,
) -> Arc<RaceFactionChangeCatalogLikeCpp> {
    let (horde_taxi_mask, alliance_taxi_mask) =
        open_db2(inputs.data_dir, inputs.locale, "TaxiNodes.db2")
            .map(|reader| taxi_masks_like_cpp(&reader))
            .unwrap_or_default();
    let title_mask_ids: HashMap<u32, u32> =
        open_db2(inputs.data_dir, inputs.locale, "CharTitles.db2")
            .map(|reader| {
                reader
                    .iter_records()
                    .map(|(id, idx)| (id, reader.get_field_u32(idx, CHAR_TITLES_MASK_ID_FIELD)))
                    .collect()
            })
            .unwrap_or_default();
    let mut race_restricted_quests: Vec<(u32, u64)> = inputs
        .quests
        .quests_like_cpp()
        .filter(|quest| quest.allowable_races != u64::MAX)
        .map(|quest| (quest.id, quest.allowable_races))
        .collect();
    race_restricted_quests.sort_unstable();
    let configs = inputs.world_configs;
    let catalog = RaceFactionChangeCatalogLikeCpp {
        faction_change: inputs.faction_change,
        race_alliance: inputs
            .chr_races
            .iter()
            .map(|race| (race.id as u8, race.alliance))
            .collect(),
        horde_taxi_mask,
        alliance_taxi_mask,
        title_mask_ids,
        race_restricted_quests,
        factions: Some(Arc::clone(inputs.factions)),
        reserved_names: Some(Arc::clone(inputs.reserved_names)),
        disabled_race_mask: configs
            .get_int64("CONFIG_CHARACTER_CREATING_DISABLED_RACEMASK")
            .unwrap_or(0),
        prevent_rename_customization: configs
            .get_bool("CONFIG_PREVENT_RENAME_CUSTOMIZATION")
            .unwrap_or(false),
        allow_two_side_interaction_guild: configs
            .get_bool("CONFIG_ALLOW_TWO_SIDE_INTERACTION_GUILD")
            .unwrap_or(false),
        allow_two_side_interaction_group: configs
            .get_bool("CONFIG_ALLOW_TWO_SIDE_INTERACTION_GROUP")
            .unwrap_or(false),
    };
    info!(
        "Race/faction change: {} race teams, {}-byte taxi masks, {} title masks, {} race-restricted quests",
        catalog.race_alliance.len(),
        catalog.horde_taxi_mask.len(),
        catalog.title_mask_ids.len(),
        catalog.race_restricted_quests.len()
    );
    Arc::new(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_taxi_nodes_build_disjoint_nonempty_team_masks_when_data_is_present() {
        let Some(reader) = open_db2("/home/inna/rustycore-run/Data", "esES", "TaxiNodes.db2")
        else {
            return;
        };
        let (horde, alliance) = taxi_masks_like_cpp(&reader);
        assert_eq!(horde.len() % 8, 0);
        assert_eq!(horde.len(), alliance.len());
        assert!(horde.iter().any(|byte| *byte != 0));
        // Stormwind (node 2) is Alliance only.
        assert_ne!(alliance[0] & 0b10, 0);
        assert_eq!(horde[0] & 0b10, 0);
    }
}
