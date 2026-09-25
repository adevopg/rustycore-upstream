//! Game table extraction: port of `ExtractGameTables` from
//! `src/tools/map_extractor/System.cpp` (text files copied to `gt/`).

use std::path::Path;

use crate::casc::Casc;
use crate::fsutil::{create_dir, open_and_extract};

/// `DB2FileInfo GameTables[]` (FileDataID, output name), in C++ order.
pub(crate) const GAME_TABLES: [(u32, &str); 25] = [
    (1_582_086, "ArtifactKnowledgeMultiplier.txt"),
    (1_391_662, "ArtifactLevelXP.txt"),
    (1_391_663, "BarberShopCostBase.txt"),
    (1_391_664, "BaseMp.txt"),
    (1_391_665, "BattlePetTypeDamageMod.txt"),
    (1_391_666, "BattlePetXP.txt"),
    (1_391_667, "ChallengeModeDamage.txt"),
    (1_391_668, "ChallengeModeHealth.txt"),
    (1_391_669, "CombatRatings.txt"),
    (1_391_670, "CombatRatingsMultByILvl.txt"),
    (1_391_671, "HonorLevel.txt"),
    (1_391_642, "HpPerSta.txt"),
    (1_391_643, "ItemSocketCostPerLevel.txt"),
    (1_391_651, "NPCManaCostScaler.txt"),
    (1_391_659, "SandboxScaling.txt"),
    (1_391_660, "SpellScaling.txt"),
    (2_200_979, "ShieldBlockRegular.txt"),
    (2_238_239, "OCTRegenMP.txt"),
    (2_238_240, "RegenMPPerSpt.txt"),
    (3_953_485, "OCTRegenHP.txt"),
    (3_953_486, "RegenHPPerSpt.txt"),
    (3_999_262, "ChanceToMeleeCrit.txt"),
    (3_999_263, "ChanceToMeleeCritBase.txt"),
    (3_999_264, "ChanceToSpellCritBase.txt"),
    (3_999_265, "ChanceToSpellCrit.txt"),
];

/// `ExtractGameTables`.
pub(crate) fn extract_game_tables(casc: &Casc, output_path: &Path) -> anyhow::Result<()> {
    println!("Extracting game tables...");

    let output_path = output_path.join("gt");
    create_dir(&output_path)?;

    println!("output path {}", output_path.display());

    let mut count = 0u32;
    for (file_data_id, name) in GAME_TABLES {
        match open_and_extract(casc, file_data_id, &output_path.join(name)) {
            Ok(true) => count += 1,
            Ok(false) => {}
            Err(error) => println!("Unable to open file {name} in the archive: {error}"),
        }
    }

    println!("Extracted {count} files\n");
    Ok(())
}
