//! Ordinary item requirements and atomic inventory actions for mission stages.
use serde::{Deserialize, Serialize};
pub const MISSION_INVENTORY_FIELD: &str = "_ffoneInventory";
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionItem {
    pub item_type: i16,
    pub item_id: i16,
    pub count: u32,
    #[serde(default)]
    pub allow_equipped: bool,
    #[serde(default)]
    pub require_equipped: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryOperation {
    Add,
    Remove,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryTarget {
    Inventory,
    Bank,
    Both,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryAction {
    pub operation: InventoryOperation,
    pub target: InventoryTarget,
    pub item: MissionItem,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MissionInventory {
    pub min_taros: u32,
    pub min_fm: u32,
    pub required: Vec<MissionItem>,
    pub start: Vec<InventoryAction>,
    pub before_finish: Vec<InventoryAction>,
    pub after_finish: Vec<InventoryAction>,
}
impl MissionItem {
    pub fn validate(&self) -> Result<(), String> {
        if !(0..=10).contains(&self.item_type)
            || self.item_id <= 0
            || self.count == 0
            || self.count > 1_000_000
        {
            return Err("Invalid mission inventory item or quantity".into());
        }
        Ok(())
    }
}
impl MissionInventory {
    pub fn items(&self) -> impl Iterator<Item = &MissionItem> {
        self.required.iter().chain(
            self.start
                .iter()
                .chain(&self.before_finish)
                .chain(&self.after_finish)
                .map(|action| &action.item),
        )
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.min_taros > i32::MAX as u32 || self.min_fm > i32::MAX as u32 {
            return Err("Mission currency requirement exceeds the server range".into());
        }
        for item in &self.required {
            item.validate()?;
        }
        for action in self
            .start
            .iter()
            .chain(&self.before_finish)
            .chain(&self.after_finish)
        {
            action.item.validate()?;
            if action.operation == InventoryOperation::Add && action.target == InventoryTarget::Both
            {
                return Err("Choose one destination for an added item".into());
            }
        }
        Ok(())
    }
    pub fn from_row(row: &serde_json::Value) -> Result<Self, String> {
        let value = row
            .get(MISSION_INVENTORY_FIELD)
            .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
            .transpose()?
            .unwrap_or_default();
        Self::validate(&value)?;
        if row["m_iHTaskType"].as_i64() == Some(7) {
            let giver = row["m_iHNPCID"].as_i64().unwrap_or(0);
            let terminator = row["m_iHTerminatorNPCID"].as_i64().unwrap_or(0);
            let branches = row
                .pointer("/_ffoneMissionStage/completions")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|v| !v.is_empty());
            if giver <= 0
                || (!branches && terminator > 0 && terminator != giver)
                || !value.start.is_empty()
                || row["m_iCSUDEFNPCID"].as_i64().unwrap_or(0) > 0
                || row["m_iCSUNumToKill"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|v| v.as_i64().unwrap_or(0) > 0)
                || row["m_iSTItemID"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .any(|(i, v)| {
                        v.as_i64().unwrap_or(0) > 0
                            && row["m_iSTItemNumNeeded"][i].as_i64().unwrap_or(0) != 0
                    })
            {
                return Err("Self-completing stages require one giver NPC and cannot grant start items, start inventory actions, kills or escorts".into());
            }
        }
        Ok(value)
    }
}
