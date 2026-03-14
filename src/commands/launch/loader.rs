//! Modloader dependency checking. Warns the user when common required
//! companion mods (e.g. Fabric API, QSL) are missing from the modpack.

use crate::config::ModpackManifest;
use crate::output;
use crate::storage::Storage;
use crate::types::LoaderType;

/// Checks whether common required companion mods are present for the active
/// modloader and emits warnings if they're missing (e.g. Fabric API, QSL).
pub fn check_modloader_deps(
	storage: &Storage,
	modpack: &ModpackManifest,
) {
	let mods = storage
		.list(crate::types::ProjectType::Mod)
		.unwrap_or_default();
	let mod_ids: Vec<&str> = mods.iter().map(|m| m.id.as_str()).collect();

	match modpack.loader.loader_or_default() {
		LoaderType::Fabric => {
			let has_fabric_api = mod_ids
				.iter()
				.any(|id| matches!(*id, "fabric-api" | "fabric" | "fabricapi"));
			if !has_fabric_api && !mods.is_empty() {
				output::warning(
					"Fabric API is missing! Most Fabric mods require it.",
				);
				output::dim("  Run: yammm add fabric-api");
			}
		}
		LoaderType::Forge => {
			let has_forge_api = mod_ids
				.iter()
				.any(|id| matches!(*id, "forge" | "minecraftforge"));
			if !has_forge_api && !mods.is_empty() {
				output::warning(
					"Forge is present but no core Forge API mod detected. Some mods may not work.",
				);
			}
		}
		LoaderType::NeoForge => {
			let has_neoforge_api = mod_ids
				.iter()
				.any(|id| matches!(*id, "neoforge" | "neoforged"));
			if !has_neoforge_api && !mods.is_empty() {
				output::warning(
					"NeoForge is present but no core NeoForge API mod detected. Some mods may not work.",
				);
			}
		}
		LoaderType::Quilt => {
			let has_qsl = mod_ids.iter().any(|id| {
				matches!(*id, "qsl" | "quilted-fabric-api" | "qfapi")
			});
			if !has_qsl && !mods.is_empty() {
				output::warning(
					"Quilt Standard Libraries is missing! Most Quilt mods require it.",
				);
				output::dim("  Run: yammm add qsl");
			}
		}
	}
}
