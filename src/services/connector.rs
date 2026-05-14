use crate::storage::Storage;
use crate::types::ProjectType;

const CONNECTOR_MODRINTH_SLUG: &str = "connector";
const FORGIFIED_FABRIC_API_SLUG: &str = "forgified-fabric-api";

pub fn is_connector_installed(storage: &Storage) -> bool {
	storage.list(ProjectType::Mod).is_ok_and(|mods| {
		mods.iter().any(|m| {
			m.id == CONNECTOR_MODRINTH_SLUG
				|| m.id == FORGIFIED_FABRIC_API_SLUG
				|| m.name.to_lowercase().contains("sinytra connector")
				|| m.name.to_lowercase().contains("sinytra adapter")
		})
	})
}
