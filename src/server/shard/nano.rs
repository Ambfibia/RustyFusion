use crate::{
    defines::*,
    entity::{Combatant, Entity, EntityID, Player},
    enums::*,
    error::*,
    item::Item,
    net::{
        packet::{PacketID::*, *},
        ClientMap, FFClient,
    },
    state::ShardServerState,
    tabledata::tdata_get,
};

/// Staff may use nano station actions anywhere; everyone else must stand at one.
fn at_nano_station(state: &ShardServerState, player: &Player) -> bool {
    player.perms as u32 <= CN_ACCOUNT_LEVEL__DEVELOPER
        || !state
            .entity_map
            .find_npcs(|npc| {
                npc.ty == TYPE_NANO_MACHINE
                    && npc.get_position().distance_to(&player.get_position()) <= RANGE_INTERACT
                    && npc.instance_id == player.instance_id
            })
            .is_empty()
}

pub fn nano_equip(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_NANO_EQUIP = pkt.get()?;

    let player = state.get_player(pc_id)?;
    if !at_nano_station(state, player) {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} tried to equip a nano without a nano station", player),
        ));
    }

    let player = state.get_player_mut(pc_id)?;
    player.change_nano(pkt.iNanoSlotNum as usize, Some(pkt.iNanoID))?;

    let deactivate = player.get_active_nano_slot() == Some(pkt.iNanoSlotNum as usize);
    let resp = sP_FE2CL_REP_NANO_EQUIP_SUCC {
        iNanoID: pkt.iNanoID,
        iNanoSlotNum: pkt.iNanoSlotNum,
        bNanoDeactive: if deactivate { 1 } else { 0 },
    };

    if deactivate {
        player.deactivate_nano();
        let bcast = sP_FE2CL_NANO_ACTIVE {
            iPC_ID: pc_id,
            Nano: None.into_proto(),
            iConditionBitFlag: player.get_condition_bit_flag(),
            eCSTB___Add: 0,
        };

        state
            .entity_map
            .for_each_around(EntityID::Player(pc_id), |c| {
                c.send_packet(P_FE2CL_NANO_ACTIVE, &bcast);
            });
    }

    client.send_packet(P_FE2CL_REP_NANO_EQUIP_SUCC, &resp);
    Ok(())
}

pub fn nano_unequip(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_NANO_UNEQUIP = pkt.get()?;

    let player = state.get_player_mut(pc_id)?;
    player.change_nano(pkt.iNanoSlotNum as usize, None)?;

    let deactivate = player.get_active_nano_slot() == Some(pkt.iNanoSlotNum as usize);
    let resp = sP_FE2CL_REP_NANO_UNEQUIP_SUCC {
        iNanoSlotNum: pkt.iNanoSlotNum,
        bNanoDeactive: if deactivate { 1 } else { 0 },
    };

    if deactivate {
        player.deactivate_nano();
        let bcast = sP_FE2CL_NANO_ACTIVE {
            iPC_ID: pc_id,
            Nano: None.into_proto(),
            iConditionBitFlag: player.get_condition_bit_flag(),
            eCSTB___Add: 0,
        };

        state
            .entity_map
            .for_each_around(EntityID::Player(pc_id), |c| {
                c.send_packet(P_FE2CL_NANO_ACTIVE, &bcast);
            });
    }

    client.send_packet(P_FE2CL_REP_NANO_UNEQUIP_SUCC, &resp);
    Ok(())
}

pub fn nano_active(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_NANO_ACTIVE = pkt.get()?;

    let player = state.get_player_mut(pc_id)?;
    let buff_applied = if pkt.iNanoSlotNum == NANO_SLOT_NONE {
        player.deactivate_nano();
        false
    } else {
        player.activate_nano(pkt.iNanoSlotNum as usize)?
    };

    let resp = sP_FE2CL_REP_NANO_ACTIVE_SUCC {
        iActiveNanoSlotNum: pkt.iNanoSlotNum,
        eCSTB___Add: buff_applied as i32,
    };

    let bcast = sP_FE2CL_NANO_ACTIVE {
        iPC_ID: pc_id,
        Nano: player.get_active_nano().into_proto(),
        iConditionBitFlag: player.get_condition_bit_flag(),
        eCSTB___Add: buff_applied as i32,
    };

    state
        .entity_map
        .for_each_around(EntityID::Player(pc_id), |c| {
            c.send_packet(P_FE2CL_NANO_ACTIVE, &bcast);
        });

    client.send_packet(P_FE2CL_REP_NANO_ACTIVE_SUCC, &resp);
    Ok(())
}

pub fn charge_nano_stamina(client: &FFClient, state: &mut ShardServerState) -> FFResult<()> {
    let pc_id = client.get_player_id()?;
    let player = state.get_player_mut(pc_id)?;
    let available_potions = player.get_nano_potions();
    if available_potions == 0 {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "{} tried to charge nano stamina without any potions",
                player
            ),
        ));
    }

    let Some(active_nano) = player.get_active_nano_mut() else {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "{} tried to charge nano stamina without an active nano",
                player
            ),
        ));
    };

    let nano_id = active_nano.get_id();
    let current_stamina = active_nano.get_stamina();
    let depleted_stamina = (NANO_STAMINA_MAX - current_stamina) as u32;
    if depleted_stamina == 0 {
        return Ok(()); // nothing to do
    }

    let potions_to_use = depleted_stamina.min(available_potions);

    // heal nano
    let new_stamina = current_stamina + potions_to_use as i16;
    active_nano.set_stamina(new_stamina);

    // consume from player
    let player = state.get_player_mut(pc_id)?; // reborrow
    let new_available_potions = player.set_nano_potions(available_potions - potions_to_use);

    let resp = sP_FE2CL_REP_CHARGE_NANO_STAMINA {
        iBatteryN: new_available_potions as i32,
        iNanoID: nano_id,
        iNanoStamina: new_stamina,
    };

    client.send_packet(P_FE2CL_REP_CHARGE_NANO_STAMINA, &resp);
    Ok(())
}

pub fn nano_tune(pkt: Packet, client: &FFClient, state: &mut ShardServerState) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_NANO_TUNE = pkt.get()?;
    let pc_id = client.get_player_id()?;
    (|| {
        // The client sends the row's tune number; the tuning row names the
        // skill it grants (OpenFusion Nanos.cpp setNanoSkill).
        let tuning = tdata_get().get_nano_tuning(pkt.iTuneID)?;
        if !tdata_get()
            .get_nano_stats(pkt.iNanoID)?
            .tunes
            .contains(&pkt.iTuneID)
        {
            return Err(FFError::build(
                Severity::Warning,
                format!("Tune {} doesn't belong to nano {}", pkt.iTuneID, pkt.iNanoID),
            ));
        }
        let skill_id = tuning.skill_id;

        let player = state.get_player(pc_id)?;
        let current_skill = player
            .get_nano(pkt.iNanoID)
            .ok_or(FFError::build(
                Severity::Warning,
                format!("Player does not have nano {}", pkt.iNanoID),
            ))?
            .selected_skill;
        // also absorbs a repeated request, which would otherwise pay again
        if current_skill == Some(skill_id) {
            return Err(FFError::build(
                Severity::Warning,
                format!("Nano {} already has skill {}", pkt.iNanoID, skill_id),
            ));
        }
        // the first tuning is free; changing a skill is a nano station service
        let free = current_skill.is_none();
        if !free && !at_nano_station(state, player) {
            return Err(FFError::build(
                Severity::Warning,
                format!("{} tried to retune a nano without a nano station", player),
            ));
        }

        // check for + consume tuning items
        let mut item_slots = [-1; 10];
        let mut items = [None.into_proto(); 10];
        let mut quantity_left = tuning.req_item_quantity;

        let mut player_working = player.clone();
        if !free {
            for (i, slot_num) in pkt.aiNeedItemSlotNum.iter().enumerate() {
                if quantity_left == 0 {
                    break;
                }

                let slot = player_working.get_item_mut(ItemLocation::Inven, *slot_num as usize)?;
                if slot.is_some_and(|stack| {
                    stack.ty == ItemType::General && stack.id == tuning.req_item_id
                }) {
                    let removed = Item::split_items(slot, quantity_left);
                    quantity_left -= removed.unwrap().quantity;
                    item_slots[i] = *slot_num;
                    items[i] = (*slot).into_proto();
                }
            }

            if quantity_left != 0 {
                return Err(FFError::build(
                    Severity::Warning,
                    format!(
                        "Not enough items to tune nano ({} < {})",
                        tuning.req_item_quantity - quantity_left,
                        tuning.req_item_quantity
                    ),
                ));
            }

            // consume FM; the price follows the player's level, not the tune
            let fm_cost = tdata_get()
                .get_player_stats(player_working.get_level())?
                .req_fm_nano_tune;
            if player_working.get_fusion_matter() < fm_cost {
                return Err(FFError::build(
                    Severity::Warning,
                    format!(
                        "Not enough fusion matter to tune nano {} ({} < {})",
                        pkt.iNanoID,
                        player_working.get_fusion_matter(),
                        fm_cost
                    ),
                ));
            }
            player_working.set_fusion_matter(player_working.get_fusion_matter() - fm_cost);
        }

        // Recall a nano whose skill changes, or its old passive buff would
        // outlive it (OpenFusion unsummons it before retuning).
        let recalled = player_working
            .get_active_nano()
            .is_some_and(|nano| nano.get_id() == pkt.iNanoID);
        if recalled {
            player_working.deactivate_nano();
        }
        player_working.tune_nano(pkt.iNanoID, Some(skill_id))?;
        let player = state.get_player_mut(pc_id)?;
        *player = player_working; // commit changes

        if recalled {
            let condition_bit_flag = player.get_condition_bit_flag();
            client.send_packet(
                P_FE2CL_REP_NANO_ACTIVE_SUCC,
                &sP_FE2CL_REP_NANO_ACTIVE_SUCC {
                    iActiveNanoSlotNum: NANO_SLOT_NONE,
                    eCSTB___Add: 0,
                },
            );
            let bcast = sP_FE2CL_NANO_ACTIVE {
                iPC_ID: pc_id,
                Nano: None.into_proto(),
                iConditionBitFlag: condition_bit_flag,
                eCSTB___Add: 0,
            };
            state
                .entity_map
                .for_each_around(EntityID::Player(pc_id), |c| {
                    c.send_packet(P_FE2CL_NANO_ACTIVE, &bcast);
                });
        }

        let player = state.get_player(pc_id)?;
        let resp = sP_FE2CL_REP_NANO_TUNE_SUCC {
            iNanoID: pkt.iNanoID,
            iSkillID: skill_id,
            iPC_FusionMatter: player.get_fusion_matter() as i32,
            aiItemSlotNum: item_slots,
            aItem: items,
        };

        client.send_packet(P_FE2CL_REP_NANO_TUNE_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_NANO_TUNE_FAIL {
            iPC_ID: pc_id,
            iErrorCode: unused!(),
        };

        client.send_packet(P_FE2CL_REP_NANO_TUNE_FAIL, &resp);
    })
}
