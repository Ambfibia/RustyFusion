//! Authoritative native vendor conditions and source-specific loot pools.
use crate::{
    entity::{Combatant, Player},
    enums::ItemType,
    error::{FFError, FFResult, Severity},
    item::Item,
};
use crate::tabledata_contracts::economy::{
    EconomyCatalog, EconomyItem, ItemConditions, LootCategory, ECONOMY_TABLE,
};
use rand::Rng;
use serde_json::{Map, Value};

pub fn load(root: &Map<String, Value>) -> Result<EconomyCatalog, String> {
    let catalog: EconomyCatalog = root
        .get(ECONOMY_TABLE)
        .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
        .transpose()?
        .unwrap_or_default();
    catalog.validate()?;
    Ok(catalog)
}
pub fn allows(conditions: &ItemConditions, player: &Player) -> bool {
    conditions.allows(i32::from(player.get_level()), player.get_taros(), |id| {
        player
            .mission_journal
            .is_mission_completed(i32::from(id))
            .unwrap_or(false)
    })
}
pub fn eligible(item: &EconomyItem, player: &Player) -> bool {
    item.value > 0.
        && allows(&item.conditions, player)
        && item
            .gender
            .is_none_or(|gender| gender == player.get_style().iGender as i32)
}
fn native_item(item: &EconomyItem) -> FFResult<Item> {
    Ok(Item::new(item.item_type.try_into()?, item.item_id))
}
pub fn draw(
    catalog: &EconomyCatalog,
    category: LootCategory,
    id: i32,
    player: &Player,
) -> FFResult<Option<Vec<Item>>> {
    draw_rank(catalog, category, id, player, None)
}
pub fn draw_rank(
    catalog: &EconomyCatalog,
    category: LootCategory,
    id: i32,
    player: &Player,
    rank: Option<u8>,
) -> FFResult<Option<Vec<Item>>> {
    let source = catalog.loot.iter().find(|v| {
        v.id == id
            && if category.is_crate() {
                v.category.is_crate()
            } else {
                v.category == category
            }
    });
    let Some(source) = source else {
        return Ok(None);
    };
    let mut rng = rand::thread_rng();
    let pool: Vec<_> = source
        .items
        .iter()
        .filter(|i| {
            eligible(i, player)
                && rank.is_none_or(|r| i.race_ranks.is_empty() || i.race_ranks.contains(&r))
        })
        .collect();
    if category.is_crate() {
        let weight = |item: &EconomyItem| {
            item.gender_chances
                .map(|chances| {
                    let base = (chances[0] + chances[1]) * 0.5;
                    let gender = (player.get_style().iGender as usize)
                        .saturating_sub(1)
                        .min(1);
                    if base > 0. {
                        item.value / base * chances[gender]
                    } else {
                        item.value
                    }
                })
                .unwrap_or(item.value)
        };
        let total: f64 = pool.iter().map(|v| weight(v)).sum();
        if total <= 0. {
            return Err(FFError::build(
                Severity::Warning,
                "No eligible item in this crate".into(),
            ));
        }
        let mut roll = rng.gen_range(0.0..total);
        for item in &pool {
            let chance = weight(item);
            if roll < chance {
                return Ok(Some(vec![native_item(item)?]));
            }
            roll -= chance;
        }
        return Ok(Some(vec![native_item(pool.last().unwrap())?]));
    }
    let mut result = vec![];
    if !source.independent {
        let mut roll = rng.gen_range(0.0..100.);
        for item in pool {
            if roll < item.value {
                return Ok(Some(vec![native_item(item)?]));
            }
            roll -= item.value;
        }
        return Ok(Some(vec![]));
    }
    for item in pool {
        if rng.gen_range(0.0..100.) < item.value {
            result.push(native_item(item)?);
        }
    }
    Ok(Some(result))
}
pub fn give_items(player: &mut Player, items: &[Item]) -> FFResult<()> {
    let actions = items
        .iter()
        .map(
            |item| crate::tabledata_contracts::mission_inventory::InventoryAction {
                operation: crate::tabledata_contracts::mission_inventory::InventoryOperation::Add,
                target: crate::tabledata_contracts::mission_inventory::InventoryTarget::Inventory,
                item: crate::tabledata_contracts::mission_inventory::MissionItem {
                    item_type: item.ty as i16,
                    item_id: item.id,
                    count: u32::from(item.quantity),
                    ..Default::default()
                },
            },
        )
        .collect();
    let spec = crate::tabledata_contracts::mission_inventory::MissionInventory {
        start: actions,
        ..Default::default()
    };
    let plan = player.plan_mission_inventory(&spec, false, &[])?;
    player.commit_mission_inventory(plan, 0)
}
pub fn validate_items(
    catalog: &EconomyCatalog,
    exists: impl Fn(i16, ItemType) -> bool,
) -> Result<(), String> {
    for item in catalog
        .vendors
        .iter()
        .flat_map(|s| &s.items)
        .chain(catalog.loot.iter().flat_map(|s| &s.items))
    {
        let ty: ItemType = item
            .item_type
            .try_into()
            .map_err(|e: FFError| e.get_msg().to_owned())?;
        if !exists(item.item_id, ty) {
            return Err(format!(
                "Economy references missing item {}:{}",
                item.item_type, item.item_id
            ));
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{VendorData, VendorItem};
    fn item(value: f64) -> EconomyItem {
        EconomyItem {
            item_type: 0,
            item_id: 1,
            value,
            conditions: Default::default(),
            visible_unavailable: false,
            gender: None,
            race_ranks: vec![],
            gender_chances: None,
        }
    }
    #[test]
    fn unavailable_vendor_rows_can_be_visible_but_never_purchased() {
        let player = Player::default();
        let mut rule = item(123.);
        rule.conditions.min_level = 10;
        let mut hidden = VendorData::new(1);
        hidden.insert(VendorItem {
            sort_number: 0,
            ty: ItemType::Hand,
            id: 1,
            native: Some(rule.clone()),
        });
        let rows = hidden.as_arr_for(Some(&player)).unwrap();
        let id = rows[0].item.iID;
        assert_eq!(id, 0);
        assert!(hidden.price_for(1, ItemType::Hand, &player).is_err());
        rule.visible_unavailable = true;
        let mut visible = VendorData::new(1);
        visible.insert(VendorItem {
            sort_number: 0,
            ty: ItemType::Hand,
            id: 1,
            native: Some(rule),
        });
        let rows = visible.as_arr_for(Some(&player)).unwrap();
        let id = rows[0].item.iID;
        let price = rows[0].fBuyCost;
        assert_eq!(id, 1);
        assert_eq!(price, 123.);
        assert!(visible.price_for(1, ItemType::Hand, &player).is_err());
    }
    #[test]
    fn empty_or_ineligible_crate_pool_errors_without_a_fallback_reward() {
        let mut catalog = EconomyCatalog::default();
        let mut rule = item(100.);
        rule.conditions.min_level = 10;
        catalog
            .loot
            .push(crate::tabledata_contracts::economy::LootCatalog {
                category: LootCategory::Event,
                id: 32000,
                name_en: "Event".into(),
                name_ru: "Эвент".into(),
                level: 1,
                items: vec![rule],
                independent: false,
            });
        assert!(draw(&catalog, LootCategory::Mission, 32000, &Player::default()).is_err());
        assert!(draw(&catalog, LootCategory::Mob, 1, &Player::default())
            .unwrap()
            .is_none());
    }
    #[test]
    fn vendor_purchase_uses_the_same_filtered_twenty_row_page_as_the_catalog() {
        let player = Player::default();
        let mut vendor = VendorData::new(651);
        for id in 1..=20 {
            let mut rule = item(123.);
            rule.conditions.min_level = 10;
            vendor.insert(VendorItem { sort_number: id as i32, ty: ItemType::Hand, id, native: Some(rule) });
        }
        vendor.insert(VendorItem { sort_number: 21, ty: ItemType::Hand, id: 99, native: Some(item(321.)) });
        let rows = vendor.as_arr_for(Some(&player)).unwrap();
        let id = rows[0].item.iID;
        assert_eq!(id, 99);
        assert_eq!(vendor.price_for(99, ItemType::Hand, &player).unwrap(), 321);
        assert!(vendor.price_for(1, ItemType::Hand, &player).is_err());
    }
    #[test]
    fn race_rolls_only_the_awarded_rank_and_zero_chance_does_not_drop() {
        let mut catalog = EconomyCatalog::default();
        let mut first = item(100.);
        first.race_ranks = vec![0];
        catalog
            .loot
            .push(crate::tabledata_contracts::economy::LootCatalog {
                category: LootCategory::Race,
                id: 1,
                name_en: "Race".into(),
                name_ru: "Гонка".into(),
                level: 0,
                items: vec![first],
                independent: false,
            });
        assert_eq!(
            draw_rank(&catalog, LootCategory::Race, 1, &Player::default(), Some(0))
                .unwrap()
                .unwrap()
                .len(),
            1
        );
        assert!(
            draw_rank(&catalog, LootCategory::Race, 1, &Player::default(), Some(1))
                .unwrap()
                .unwrap()
                .is_empty()
        );
        catalog.loot[0].items[0].value = 0.;
        assert!(
            draw_rank(&catalog, LootCategory::Race, 1, &Player::default(), Some(0))
                .unwrap()
                .unwrap()
                .is_empty()
        );
    }
}
