//! Optional native mission branches and per-player NPC actions.
use super::mission_inventory::MissionInventory;
use serde::{Deserialize, Serialize};
pub const FIELD: &str = "_ffoneMissionStage";
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Conditions {
    pub min_level: u16,
    /// Zero means unlimited.
    pub max_level: u16,
    pub completed_missions: Vec<i32>,
    pub uncompleted_missions: Vec<i32>,
    pub inventory: MissionInventory,
}
impl Conditions {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_level > 0 && self.max_level < self.min_level {
            return Err("Maximum level must be at least minimum".into());
        }
        if self
            .completed_missions
            .iter()
            .chain(&self.uncompleted_missions)
            .any(|id| !(1..=1024).contains(id))
            || self
                .completed_missions
                .iter()
                .any(|id| self.uncompleted_missions.contains(id))
        {
            return Err("Invalid or conflicting mission conditions".into());
        }
        self.inventory.validate()?;
        if !self.inventory.start.is_empty()
            || !self.inventory.before_finish.is_empty()
            || !self.inventory.after_finish.is_empty()
        {
            return Err("Conditions cannot change inventory".into());
        }
        Ok(())
    }
    pub fn allows(
        &self,
        level: i32,
        mut completed: impl FnMut(i32) -> bool,
        inventory: impl FnOnce(&MissionInventory) -> bool,
    ) -> bool {
        level >= i32::from(self.min_level)
            && (self.max_level == 0 || level <= i32::from(self.max_level))
            && self.completed_missions.iter().all(|id| completed(*id))
            && self.uncompleted_missions.iter().all(|id| !completed(*id))
            && inventory(&self.inventory)
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Completion {
    pub npc: i32,
    pub next_task: i32,
    pub conditions: Conditions,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteFinish {
    Hide,
    #[default]
    Stay,
    Replace,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NpcRoute {
    /// Exact placement/network ID, preserving distinct actors of the same type.
    pub npc_id: i32,
    pub route: String,
    pub finish: RouteFinish,
    pub replacement_npc: i32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NpcVisibility {
    pub npc_ids: Vec<i32>,
    pub visible: bool,
    pub conditions: Conditions,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MissionStage {
    /// Empty retains the original terminator and outgoing task fields.
    pub completions: Vec<Completion>,
    pub before: Vec<NpcRoute>,
    pub after: Vec<NpcRoute>,
    pub visibility: Vec<NpcVisibility>,
}
impl MissionStage {
    pub fn from_row(row: &serde_json::Value) -> Result<Self, String> {
        let value = row
            .get(FIELD)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let stage: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
        stage.validate()?;
        Ok(stage)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.completions.len() > 4 {
            return Err("A stage supports at most four completion variants".into());
        }
        for (index, c) in self.completions.iter().enumerate() {
            if c.npc <= 0
                || c.next_task < 0
                || self.completions[..index].iter().any(|old| old.npc == c.npc)
            {
                return Err(
                    "Each completion variant needs a distinct NPC and a nonnegative next task"
                        .into(),
                );
            }
            c.conditions.validate()?;
        }
        if self.before.len() > 64 || self.after.len() > 64 || self.visibility.len() > 64 {
            return Err("Too many NPC actions".into());
        }
        for route in self.before.iter().chain(&self.after) {
            if route.npc_id <= 0
                || route.route.is_empty()
                || route.route.len() > 255
                || route.route.contains(['/', '\\', ':'])
                || route.replacement_npc < 0
                || (route.finish == RouteFinish::Replace && route.replacement_npc <= 0)
            {
                return Err("Choose an NPC placement, an authored route and a replacement NPC when required".into());
            }
        }
        for rule in &self.visibility {
            if rule.npc_ids.is_empty()
                || rule.npc_ids.len() > 128
                || rule.npc_ids.iter().any(|id| *id <= 0)
            {
                return Err("Choose at least one NPC placement for visibility".into());
            }
            rule.conditions.validate()?;
        }
        Ok(())
    }
    pub fn completion(&self, npc_type: i32) -> Option<(usize, &Completion)> {
        self.completions
            .iter()
            .enumerate()
            .find(|(_, c)| c.npc == npc_type)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn branch_conditions_are_independent_and_reject_ambiguous_npcs() {
        let mut stage = MissionStage {
            completions: vec![
                Completion {
                    npc: 7,
                    next_task: 12,
                    ..Default::default()
                },
                Completion {
                    npc: 8,
                    next_task: 13,
                    conditions: Conditions {
                        min_level: 5,
                        ..Default::default()
                    },
                },
            ],
            ..Default::default()
        };
        stage.validate().unwrap();
        assert_eq!(stage.completion(8).unwrap().1.next_task, 13);
        assert!(stage.completions[0]
            .conditions
            .allows(1, |_| false, |_| true));
        assert!(!stage.completions[1]
            .conditions
            .allows(1, |_| false, |_| true));
        stage.completions[1].npc = 7;
        assert!(stage.validate().is_err());
    }
    #[test]
    fn no_extensions_preserve_native_rows() {
        assert_eq!(
            MissionStage::from_row(&serde_json::json!({"m_iHTerminatorNPCID":7})).unwrap(),
            MissionStage::default()
        );
    }
}
