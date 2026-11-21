//! Authored E.G.G. types and loading tip validation for the published TableData.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const TABLE: &str = "ffone_egg_tips";
pub const SCHEMA: &str = "ffone.egg-tips.v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct EggDefinition {
    pub id: i32,
    pub name_en: String,
    pub name_ru: String,
    pub model_type: i32,
    pub effect_id: i32,
    pub drop_crate_id: i16,
    pub nano_potions: u32,
    pub weapon_boosts: u32,
    pub regen: u32,
    pub duration: u32,
    pub texture: String,
    pub text_en: String,
    pub text_ru: String,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct LoadingTip {
    pub id: i32,
    pub priority: u8,
    pub text_en: String,
    pub text_ru: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EggTipsCatalog {
    pub schema: String,
    #[serde(rename = "EggsInitialized", default)]
    pub eggs_initialized: bool,
    #[serde(rename = "EggTypes", default)]
    pub eggs: Vec<EggDefinition>,
    #[serde(rename = "LoadingTips", default)]
    pub tips: Vec<LoadingTip>,
}
impl Default for EggTipsCatalog {
    fn default() -> Self {
        Self {
            schema: SCHEMA.into(),
            eggs_initialized: false,
            eggs: vec![],
            tips: vec![],
        }
    }
}
impl EggTipsCatalog {
    pub fn from_value(value: &Value) -> Result<Self, String> {
        let catalog: Self = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        catalog.validate()?;
        Ok(catalog)
    }
    pub fn from_server_root(root: &serde_json::Map<String, Value>) -> Result<Self, String> {
        if let Some(value) = root.get(TABLE) {
            return Self::from_value(value);
        }
        let mut matches = root
            .get("_ffone")
            .and_then(|v| v["tables"].as_array())
            .into_iter()
            .flatten()
            .filter_map(|t| t["value"].get(TABLE));
        let catalog = matches
            .next()
            .map(Self::from_value)
            .transpose()?
            .unwrap_or_default();
        if matches.next().is_some() {
            return Err("Duplicate egg/tip catalog".into());
        }
        Ok(catalog)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err("Unsupported egg/tip catalog schema".into());
        }
        let mut ids = BTreeSet::new();
        for egg in &self.eggs {
            if egg.id < 0 || !ids.insert(egg.id) {
                return Err(format!("Invalid or duplicate egg ID {}", egg.id));
            }
            if egg.model_type < 0
                || !(0..=i16::MAX as i32).contains(&egg.effect_id)
                || egg.drop_crate_id < 0
            {
                return Err(format!("Invalid egg model/effect/loot ID {}", egg.id));
            }
            if egg.nano_potions > 9999 || egg.weapon_boosts > 9999 {
                return Err(format!("Egg {} battery quantity exceeds 9999", egg.id));
            }
            if !egg.texture.is_empty() && !safe_asset_path(&egg.texture) {
                return Err(format!(
                    "Egg {} texture must be a relative native image path",
                    egg.id
                ));
            }
        }
        ids.clear();
        for tip in &self.tips {
            if tip.id <= 0
                || !ids.insert(tip.id)
                || tip.priority > 100
                || tip.text_en.trim().is_empty()
                || tip.text_ru.trim().is_empty()
            {
                return Err(format!("Invalid loading tip {}: unique positive ID, priority 0–100 and EN/RU text required", tip.id));
            }
        }
        Ok(())
    }
}
pub fn safe_asset_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':'])
        && !path.starts_with('/')
        && path.split('/').all(|p| !matches!(p, "" | "." | ".."))
        && [".png", ".jpg", ".jpeg", ".webp", ".ktx2"]
            .iter()
            .any(|ext| path.to_lowercase().ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_server_root_and_nested_published_catalog() {
        let catalog = EggTipsCatalog {
            tips: vec![LoadingTip {
                id: 1,
                priority: 100,
                text_en: "Tip".into(),
                text_ru: "Совет".into(),
            }],
            ..EggTipsCatalog::default()
        };
        let value = serde_json::to_value(&catalog).unwrap();
        let root = serde_json::json!({ (TABLE): value.clone() });
        assert_eq!(
            EggTipsCatalog::from_server_root(root.as_object().unwrap()).unwrap(),
            catalog
        );

        let table = serde_json::json!({ "value": { (TABLE): value } });
        let root = serde_json::json!({ "_ffone": { "tables": [table.clone()] } });
        assert_eq!(
            EggTipsCatalog::from_server_root(root.as_object().unwrap()).unwrap(),
            catalog
        );

        let root = serde_json::json!({ "_ffone": { "tables": [table.clone(), table] } });
        assert!(EggTipsCatalog::from_server_root(root.as_object().unwrap()).is_err());
    }

    #[test]
    fn native_texture_paths_cannot_escape_asset_root() {
        assert!(safe_asset_path("characters/shinies/custom.png"));
        for path in [
            "../egg.png",
            "C:/egg.png",
            "/egg.png",
            "a\\egg.png",
            "a/../egg.png",
            "a//egg.png",
        ] {
            assert!(!safe_asset_path(path));
        }
    }
}
