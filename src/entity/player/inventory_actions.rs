//! Plan ordinary mission inventory changes against a copy; publish only after acceptance.
use super::*;
use crate::tabledata_contracts::mission_inventory::{
    InventoryAction, InventoryOperation, InventoryTarget, MissionInventory, MissionItem,
};
pub struct InventoryPlan {
    before: PlayerInventory,
    after: PlayerInventory,
}
fn full() -> FFError {
    FFError::build(Severity::Warning, "Mission item destination is full".into())
}
fn remove_from(slots: &mut [Option<Item>], item: &MissionItem, remaining: &mut u32) {
    for slot in slots {
        if *remaining == 0 {
            break;
        }
        if let Some(owned) = slot
            .as_mut()
            .filter(|i| i.id == item.item_id && i.ty as i16 == item.item_type)
        {
            // Combined equipment encodes its appearance with a zero low-word
            // quantity, but still represents one physical inventory item.
            let quantity = u32::from(owned.quantity).max(1);
            let taken = (*remaining).min(quantity);
            *remaining -= taken;
            if taken == quantity {
                *slot = None;
            } else {
                owned.quantity = (quantity - taken) as u16;
            }
        }
    }
}
fn add_to(slots: &mut [Option<Item>], mut item: Item, count: u32, max_stack: u16) -> FFResult<()> {
    let mut remaining = count;
    let max_stack = max_stack.max(1);
    for owned in slots.iter_mut().flatten() {
        let mut base = *owned;
        base.quantity = 1;
        item.quantity = 1;
        if base == item && owned.quantity < max_stack {
            let count = remaining.min(u32::from(max_stack - owned.quantity));
            owned.quantity += count as u16;
            remaining -= count;
        }
        if remaining == 0 {
            return Ok(());
        }
    }
    for slot in slots.iter_mut().filter(|s| s.is_none()) {
        let count = remaining.min(u32::from(max_stack));
        item.quantity = count as u16;
        *slot = Some(item);
        remaining -= count;
        if remaining == 0 {
            return Ok(());
        }
    }
    Err(full())
}
fn apply(inventory: &mut PlayerInventory, actions: &[InventoryAction]) -> FFResult<()> {
    for action in actions {
        let item = Item::new(action.item.item_type.try_into()?, action.item.item_id);
        match action.operation {
            InventoryOperation::Remove => {
                let mut count = action.item.count;
                if action.target != InventoryTarget::Bank {
                    remove_from(&mut inventory.main, &action.item, &mut count);
                }
                if action.target != InventoryTarget::Inventory {
                    for bank in &mut inventory.banks {
                        remove_from(bank, &action.item, &mut count);
                    }
                }
            }
            InventoryOperation::Add => {
                let slots = match action.target {
                    InventoryTarget::Inventory => &mut inventory.main[..],
                    InventoryTarget::Bank => &mut inventory.banks[0][..],
                    InventoryTarget::Both => {
                        return Err(FFError::build(
                            Severity::Warning,
                            "Ambiguous mission item destination".into(),
                        ))
                    }
                };
                add_to(
                    slots,
                    item,
                    action.item.count,
                    item.get_stats()?.max_stack_size,
                )?;
            }
        }
    }
    Ok(())
}
impl Player {
    pub fn mission_inventory_allowed(&self, requirements: &[MissionItem]) -> bool {
        requirements.iter().all(|r| {
            let equipped=self.inventory.equipped.iter().flatten().filter(|i|i.id==r.item_id&&i.ty as i16==r.item_type).map(|i|u32::from(i.quantity).max(1)).sum::<u32>();
            if r.require_equipped {return equipped>=r.count;}
            self.inventory
                .main
                .iter()
                .flatten()
                .filter(|i| i.id == r.item_id && i.ty as i16 == r.item_type)
                .map(|i| u32::from(i.quantity).max(1))
                .sum::<u32>() + if r.allow_equipped {equipped}else{0}
                >= r.count
        })
    }
    pub fn mission_requirements_allowed(&self,spec:&MissionInventory)->bool {
        u64::from(self.get_taros())>=u64::from(spec.min_taros) && u64::from(self.get_fusion_matter())>=u64::from(spec.min_fm) && self.mission_inventory_allowed(&spec.required)
    }
    pub fn plan_mission_inventory(
        &self,
        spec: &MissionInventory,
        finish: bool,
        rewards: &[Item],
    ) -> FFResult<InventoryPlan> {
        if self.trade_id.is_some() {
            return Err(FFError::build(
                Severity::Warning,
                "Cannot advance inventory actions while trading".into(),
            ));
        }
        let mut after = self.inventory.clone();
        if finish {
            apply(&mut after, &spec.before_finish)?;
            for reward in rewards {
                add_to(
                    &mut after.main,
                    *reward,
                    u32::from(reward.quantity),
                    reward.get_stats()?.max_stack_size,
                )?;
            }
            apply(&mut after, &spec.after_finish)?;
        } else {
            apply(&mut after, &spec.start)?;
        }
        Ok(InventoryPlan {
            before: self.inventory.clone(),
            after,
        })
    }
    pub fn commit_mission_inventory(&mut self, plan: InventoryPlan, task_id: i32) -> FFResult<()> {
        if self.inventory.main != plan.before.main || self.inventory.banks != plan.before.banks {
            return Err(FFError::build(
                Severity::Warning,
                "Mission inventory changed during transaction".into(),
            ));
        }
        let mut changes = vec![];
        for (location, before, after) in [
            (
                ItemLocation::Inven,
                &plan.before.main[..],
                &plan.after.main[..],
            ),
            (
                ItemLocation::Bank,
                &plan.before.banks[self.inventory.active_bank][..],
                &plan.after.banks[self.inventory.active_bank][..],
            ),
        ] {
            for (slot, (before, after)) in before.iter().zip(after).enumerate() {
                if before != after {
                    changes.push(sItemReward {
                        sItem: (*after).into_proto(),
                        eIL: location as i32,
                        iSlotNum: slot as i32,
                    });
                }
            }
        }
        let mut packets = vec![];
        for batch in changes.chunks(32) {
            let mut packet =
                PacketBuilder::new(P_FE2CL_REP_REWARD_ITEM).with(&sP_FE2CL_REP_REWARD_ITEM {
                    m_iCandy: self.get_taros() as i32,
                    m_iFusionMatter: self.get_fusion_matter() as i32,
                    m_iBatteryN: self.get_nano_potions() as i32,
                    m_iBatteryW: self.get_weapon_boosts() as i32,
                    iItemCnt: batch.len() as i8,
                    iFatigue: 100,
                    iFatigue_Level: 1,
                    iTaskID: task_id,
                    iNPC_TypeID: 0,
                });
            for change in batch {
                packet.push(change);
            }
            packets.push(packet.build()?);
        }
        self.inventory.main = plan.after.main;
        self.inventory.banks = plan.after.banks;
        if let Some(client) = self.get_client() {
            for packet in packets {
                client.send_payload(packet);
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_removal_is_noop_and_partial_stacks_are_retained() {
        let mut item = Item::new(ItemType::General, 1);
        item.quantity = 5;
        let mut slots = [Some(item), None];
        let mut count = 3;
        remove_from(
            &mut slots,
            &MissionItem {
                item_type: 7,
                item_id: 1,
                count: 3,
            ..Default::default()
        },
            &mut count,
        );
        assert_eq!(slots[0].unwrap().quantity, 2);
        assert_eq!(count, 0);
        let mut count = 10;
        remove_from(
            &mut slots,
            &MissionItem {
                item_type: 7,
                item_id: 99,
                count: 10,
            ..Default::default()
        },
            &mut count,
        );
        assert_eq!(slots[0].unwrap().quantity, 2);
        assert_eq!(count, 10);
    }
    #[test]
    fn additions_use_existing_stacks_and_fail_on_a_copy() {
        let mut item = Item::new(ItemType::General, 1);
        item.quantity = 9;
        let slots = [Some(item), None];
        let mut planned = slots;
        assert!(add_to(&mut planned, Item::new(ItemType::General, 1), 11, 10).is_ok());
        assert_eq!(planned[0].unwrap().quantity, 10);
        assert_eq!(planned[1].unwrap().quantity, 10);
        let mut rejected = slots;
        assert!(add_to(&mut rejected, Item::new(ItemType::General, 1), 12, 10).is_err());
        assert_eq!(slots[0].unwrap().quantity, 9);
        assert!(slots[1].is_none());
    }
    #[test]
    fn removal_from_bank_does_not_satisfy_inventory_requirement() {
        let mut player = Player::default();
        player.inventory.banks[4][0] = Some(Item::new(ItemType::General, 1));
        player.inventory.equipped[0] = Some(Item::new(ItemType::General, 1));
        player.inventory.quest[0] = Some((1, 10));
        let required = [MissionItem {
            item_type: 7,
            item_id: 1,
            count: 1,
            ..Default::default()
        }];
        assert!(!player.mission_inventory_allowed(&required));
        player.inventory.main[0] = Some(Item::new(ItemType::General, 1));
        assert!(player.mission_inventory_allowed(&required));
        let mut count = 1;
        for bank in &mut player.inventory.banks {
            remove_from(bank, &required[0], &mut count);
        }
        assert_eq!(count, 0);
        assert!(player.inventory.banks[4][0].is_none());
    }
    #[test]
    fn mission_equipment_flags_distinguish_allowed_and_required_equipment(){
        let mut player=Player::default();player.inventory.main[0]=Some(Item::new(ItemType::General,1));player.inventory.equipped[0]=Some(Item::new(ItemType::General,1));
        let mut item=MissionItem{item_type:7,item_id:1,count:2,..Default::default()};assert!(!player.mission_inventory_allowed(&[item.clone()]));
        item.allow_equipped=true;assert!(player.mission_inventory_allowed(&[item.clone()]));
        item.require_equipped=true;assert!(!player.mission_inventory_allowed(&[item.clone()]));item.count=1;assert!(player.mission_inventory_allowed(&[item.clone()]));
        player.inventory.equipped[0]=None;assert!(!player.mission_inventory_allowed(&[item]));
    }
    #[test]
    fn full_inventory_cannot_commit_bank_cleanup_or_advance_the_journal() {
        tdata_get_or_init();
        let mut player = Player::default();
        player
            .inventory
            .main
            .fill(Some(Item::new(ItemType::Hand, 1)));
        player.inventory.banks[4][0] = Some(Item::new(ItemType::General, 1));
        let before = player.inventory.clone();
        let journal = player.mission_journal.clone();
        let spec = MissionInventory {
            start: vec![
                InventoryAction {
                    operation: InventoryOperation::Remove,
                    target: InventoryTarget::Both,
                    item: MissionItem {
                        item_type: 7,
                        item_id: 1,
                        count: 1,
            ..Default::default()
        },
                },
                InventoryAction {
                    operation: InventoryOperation::Add,
                    target: InventoryTarget::Inventory,
                    item: MissionItem {
                        item_type: 0,
                        item_id: 2,
                        count: 1,
            ..Default::default()
        },
                },
            ],
            ..Default::default()
        };
        assert!(player.plan_mission_inventory(&spec, false, &[]).is_err());
        assert_eq!(player.inventory.main, before.main);
        assert_eq!(player.inventory.banks, before.banks);
        assert_eq!(
            format!("{journal:?}"),
            format!("{:?}", player.mission_journal)
        );
    }
    #[test]
    fn finish_actions_and_normal_rewards_share_one_capacity_plan() {
        tdata_get_or_init();
        let mut player = Player::default();
        player
            .inventory
            .main
            .fill(Some(Item::new(ItemType::Hand, 1)));
        player.inventory.banks[4][0] = Some(Item::new(ItemType::General, 1));
        let mut spec = MissionInventory {
            before_finish: vec![
                InventoryAction {
                    operation: InventoryOperation::Remove,
                    target: InventoryTarget::Inventory,
                    item: MissionItem {
                        item_type: 0,
                        item_id: 1,
                        count: 1,
            ..Default::default()
        },
                },
                InventoryAction {
                    operation: InventoryOperation::Remove,
                    target: InventoryTarget::Bank,
                    item: MissionItem {
                        item_type: 7,
                        item_id: 1,
                        count: 1,
            ..Default::default()
        },
                },
            ],
            after_finish: vec![InventoryAction {
                operation: InventoryOperation::Add,
                target: InventoryTarget::Inventory,
                item: MissionItem {
                    item_type: 0,
                    item_id: 3,
                    count: 1,
            ..Default::default()
        },
            }],
            ..Default::default()
        };
        let rewards = [Item::new(ItemType::Hand, 2)];
        assert!(player
            .plan_mission_inventory(&spec, true, &rewards)
            .is_err());
        assert!(player.inventory.banks[4][0].is_some());
        spec.before_finish[0].item.count = 2;
        let plan = player
            .plan_mission_inventory(&spec, true, &rewards)
            .unwrap();
        player.commit_mission_inventory(plan, 1).unwrap();
        assert!(player.inventory.banks[4][0].is_none());
        assert_eq!(player.inventory.main[0].unwrap().id, 2);
        assert_eq!(player.inventory.main[1].unwrap().id, 3);
    }
    fn tdata_get_or_init() {
        crate::tabledata::tdata_init().unwrap();
    }
    #[test]
    fn combined_equipment_counts_as_one_and_removes_exactly_one_copy() {
        let mut player = Player::default();
        player.inventory.main[0] =
            Some(Item::new(ItemType::Hand, 1).combine_stats(&Item::new(ItemType::Hand, 2)));
        player.inventory.main[1] = Some(Item::new(ItemType::Hand, 2));
        let item = MissionItem {
            item_type: 0,
            item_id: 2,
            count: 2,
            ..Default::default()
        };
        assert!(player.mission_inventory_allowed(&[item.clone()]));
        let spec = MissionInventory {
            start: vec![InventoryAction {
                operation: InventoryOperation::Remove,
                target: InventoryTarget::Inventory,
                item: MissionItem { count: 1, ..item },
            }],
            ..Default::default()
        };
        let plan = player.plan_mission_inventory(&spec, false, &[]).unwrap();
        player.commit_mission_inventory(plan, 1).unwrap();
        assert!(player.inventory.main[0].is_none());
        assert!(player.inventory.main[1].is_some());
    }
    #[test]
    fn ordinary_quest_table_items_use_main_inventory_without_changing_abstract_counters() {
        tdata_get_or_init();
        let mut player = Player::default();
        player.inventory.quest[0] = Some((1, 7));
        let spec = MissionInventory {
            start: vec![InventoryAction {
                operation: InventoryOperation::Add,
                target: InventoryTarget::Inventory,
                item: MissionItem {
                    item_type: 8,
                    item_id: 1,
                    count: 2,
            ..Default::default()
        },
            }],
            ..Default::default()
        };
        let plan = player.plan_mission_inventory(&spec, false, &[]).unwrap();
        player.commit_mission_inventory(plan, 1).unwrap();
        assert_eq!(player.inventory.main[0].unwrap().ty, ItemType::Quest);
        assert_eq!(player.inventory.main[0].unwrap().quantity, 2);
        assert_eq!(player.inventory.quest[0], Some((1, 7)));
        assert!(player.mission_inventory_allowed(&[MissionItem {
            item_type: 8,
            item_id: 1,
            count: 2,
            ..Default::default()
        }]));
    }
}
