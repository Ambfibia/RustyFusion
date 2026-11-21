//! Native vendor listings and source-specific loot overrides in shared TableData.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const ECONOMY_SCHEMA: &str = "ffone.economy.v1";
pub const ECONOMY_TABLE: &str = "m_pNativeEconomy";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ItemConditions {
    pub min_level: u16,
    pub completed_mission: u16,
    pub uncompleted_mission: u16,
    pub min_taros: u32,
}
impl ItemConditions {
    pub fn validate(&self) -> Result<(), String> {
        if self.min_level > 32767
            || self.completed_mission > 2048
            || self.uncompleted_mission > 2048
        {
            return Err("Level or mission condition is out of range".into());
        }
        if self.completed_mission != 0 && self.completed_mission == self.uncompleted_mission {
            return Err("A mission cannot be both completed and uncompleted".into());
        }
        Ok(())
    }
    pub fn allows(&self, level: i32, taros: u32, completed: impl Fn(u16) -> bool) -> bool {
        level >= i32::from(self.min_level)
            && taros >= self.min_taros
            && (self.completed_mission == 0 || completed(self.completed_mission))
            && (self.uncompleted_mission == 0 || !completed(self.uncompleted_mission))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EconomyItem {
    pub item_type: i16,
    pub item_id: i16,
    /// Taros for a vendor; percentage/weight for loot (see LootCategory::is_crate).
    pub value: f64,
    #[serde(default)]
    pub conditions: ItemConditions,
    #[serde(default)]
    pub visible_unavailable: bool,
    /// Original item-set gender restriction, retained when projecting crate pools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gender: Option<i32>,
    /// Empty means any rank; existing race rewards retain their accepted rank slots.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub race_ranks: Vec<u8>,
    /// Projected original rarity/gender probabilities; edits scale these proportionally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gender_chances: Option<[f64; 2]>,
}
impl EconomyItem {
    pub fn validate(&self, vendor: bool) -> Result<(), String> {
        if !(0..=10).contains(&self.item_type)
            || self.item_id <= 0
            || !self.value.is_finite()
            || self.value < 0.
            || self.value > if vendor { i32::MAX as f64 } else { 100. }
            || (vendor && self.value.fract() != 0.)
        {
            return Err("Invalid item, price or chance".into());
        }
        if !vendor && self.conditions.min_taros != 0 {
            return Err("Loot does not support a Taros condition".into());
        }
        if self.gender.is_some_and(|g| !(1..=2).contains(&g)) {
            return Err("Invalid item gender".into());
        }
        if self.race_ranks.iter().any(|r| *r > 4) {
            return Err("Invalid race rank".into());
        }
        if self.gender_chances.is_some_and(|v| {
            v.iter()
                .any(|n| !n.is_finite() || !(0.0..=100.0).contains(n))
        }) {
            return Err("Invalid projected gender chances".into());
        }
        self.conditions.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LootCategory {
    Mob,
    Egg,
    Egger,
    Mission,
    Race,
    Event,
}
impl LootCategory {
    pub fn is_crate(self) -> bool {
        matches!(self, Self::Egger | Self::Mission | Self::Event)
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VendorCatalog {
    pub id: i32,
    pub icon: i32,
    pub items: Vec<EconomyItem>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LootCatalog {
    pub category: LootCategory,
    pub id: i32,
    pub name_en: String,
    pub name_ru: String,
    pub level: u16,
    pub items: Vec<EconomyItem>,
    #[serde(default)]
    pub independent: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EconomyCatalog {
    pub schema: String,
    pub vendors: Vec<VendorCatalog>,
    pub loot: Vec<LootCatalog>,
}
impl Default for EconomyCatalog {
    fn default() -> Self {
        Self {
            schema: ECONOMY_SCHEMA.into(),
            vendors: vec![],
            loot: vec![],
        }
    }
}
impl EconomyCatalog {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != ECONOMY_SCHEMA {
            return Err("Unsupported economy schema".into());
        }
        let mut vendors = BTreeSet::new();
        for v in &self.vendors {
            if v.id <= 0 || !vendors.insert(v.id) || v.icon < 0 {
                return Err("Invalid or duplicate vendor ID".into());
            }
            if v.items.len() > 20 {
                return Err("The 0104 vendor catalog supports 20 listings".into());
            }
            validate_items(&v.items, true)?;
        }
        let mut sources = BTreeSet::new();
        let mut crates = BTreeSet::new();
        for source in &self.loot {
            if source.id <= 0
                || !sources.insert((source.category, source.id))
                || (source.category.is_crate() && (!crates.insert(source.id) || source.id > 32767))
            {
                return Err("Invalid or duplicate loot source ID".into());
            }
            if source.level > 32767 {
                return Err("Invalid source level".into());
            }
            validate_items(&source.items, false)?;
            if !source.category.is_crate() && !source.independent {
                for rank in 0..5u8 {
                    let total: f64 = source
                        .items
                        .iter()
                        .filter(|i| {
                            source.category != LootCategory::Race
                                || i.race_ranks.is_empty()
                                || i.race_ranks.contains(&rank)
                        })
                        .map(|i| i.value)
                        .sum();
                    if total > 100. + 1e-8 {
                        return Err("Single-drop chances cannot total more than 100%".into());
                    }
                }
            }
        }
        Ok(())
    }
}
fn validate_items(items: &[EconomyItem], vendor: bool) -> Result<(), String> {
    let mut ids = BTreeSet::new();
    for item in items {
        item.validate(vendor)?;
        if !ids.insert((item.item_type, item.item_id)) {
            return Err("Duplicate item in a source".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conditions_are_conjunctive_and_zero_disables_them() {
        let c = ItemConditions {
            min_level: 10,
            completed_mission: 7,
            uncompleted_mission: 8,
            min_taros: 100,
        };
        assert!(c.allows(10, 100, |id| id == 7));
        assert!(!c.allows(9, 100, |id| id == 7));
        assert!(!c.allows(10, 99, |id| id == 7));
        assert!(!c.allows(10, 100, |_| true));
        assert!(!c.allows(10, 100, |_| false));
        assert!(ItemConditions::default().allows(1, 0, |_| false));
    }
    #[test]
    fn rejects_conflicting_conditions_and_duplicate_crate_categories() {
        assert!(ItemConditions {
            completed_mission: 7,
            uncompleted_mission: 7,
            ..Default::default()
        }
        .validate()
        .is_err());
        let source = LootCatalog {
            category: LootCategory::Mission,
            id: 1,
            name_en: "a".into(),
            name_ru: "а".into(),
            level: 1,
            items: vec![],
            independent: false,
        };
        let mut catalog = EconomyCatalog::default();
        catalog.loot.push(source.clone());
        catalog.loot.push(LootCatalog {
            category: LootCategory::Event,
            ..source
        });
        assert!(catalog.validate().is_err());
    }
}
