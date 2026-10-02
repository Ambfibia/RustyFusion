use std::{
    cmp::min,
    time::{Duration, SystemTime},
};

use rand::random;

use crate::{
    config::config_get,
    defines::*,
    entity::{Combatant, Entity, EntityID},
    enums::*,
    error::*,
    helpers,
    item::Item,
    net::{
        packet::{PacketID::*, *},
        ClientMap, FFClient,
    },
    skills,
    state::ShardServerState,
    tabledata::tdata_get,
    util,
};

pub fn item_move(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_ITEM_MOVE = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player_mut(pc_id)?;

    let location_from = movable_location(pkt.eFrom)?;
    let location_to = movable_location(pkt.eTo)?;
    // make sure the client can't reach past the end of the bank
    validate_bank_slot(location_from, pkt.iFromSlotNum)?;
    validate_bank_slot(location_to, pkt.iToSlotNum)?;
    let slot_from = slot_index(pkt.iFromSlotNum)?;
    let slot_to = slot_index(pkt.iToSlotNum)?;
    if player.trade_id.is_some() {
        return Err(FFError::build(
            Severity::Warning,
            format!("Player {} tried to move an item while trading", pc_id),
        ));
    }

    // Work on copies and validate the whole outcome before touching the live
    // slots, so a rejected move can never leave either slot cleared.
    let mut item_from = *player.get_item(location_from, slot_from)?;
    let mut item_to = *player.get_item(location_to, slot_to)?;
    let moved = (location_from, slot_from) != (location_to, slot_to);
    if moved {
        Item::transfer_items(&mut item_from, &mut item_to)?;
        validate_equip_slot(location_from, slot_from, &item_from)?;
        validate_equip_slot(location_to, slot_to, &item_to)?;
        player.set_item(location_from, slot_from, item_from)?;
        player.set_item(location_to, slot_to, item_to)?;
    }

    // Each (location, slot, item) triple is that slot's post-move content.
    let resp = sP_FE2CL_PC_ITEM_MOVE_SUCC {
        eFrom: pkt.eFrom,
        iFromSlotNum: pkt.iFromSlotNum,
        FromSlotItem: item_from.into_proto(),
        eTo: pkt.eTo,
        iToSlotNum: pkt.iToSlotNum,
        ToSlotItem: item_to.into_proto(),
    };

    client.send_packet(P_FE2CL_PC_ITEM_MOVE_SUCC, &resp);

    if !moved {
        return Ok(());
    }

    // EQUIP_CHANGE carries what the equip slot holds now (OpenFusion sends
    // the item that landed in the slot, never the one that left it).
    let entity_id = player.get_id();
    let mut equip_changes = Vec::with_capacity(2);
    if location_from == ItemLocation::Equip {
        equip_changes.push((pkt.iFromSlotNum, item_from));
    }
    if location_to == ItemLocation::Equip {
        equip_changes.push((pkt.iToSlotNum, item_to));
    }
    for (slot_num, item) in equip_changes {
        state.entity_map.for_each_around(entity_id, |c| {
            let pkt = sP_FE2CL_PC_EQUIP_CHANGE {
                iPC_ID: pc_id,
                iEquipSlotNum: slot_num,
                EquipSlotItem: item.into_proto(),
            };

            c.send_packet(P_FE2CL_PC_EQUIP_CHANGE, &pkt);
        });
    }

    // dismount vehicle
    let player = state.get_player_mut(pc_id).unwrap();
    if ((location_from == ItemLocation::Equip && slot_from == EQUIP_SLOT_VEHICLE as usize)
        || (location_to == ItemLocation::Equip && slot_to == EQUIP_SLOT_VEHICLE as usize))
        && player.vehicle_speed.is_some()
    {
        player.vehicle_speed = None;
        helpers::broadcast_state(pc_id, player.get_state_bit_flag(), state);
        let pkt = sP_FE2CL_PC_VEHICLE_OFF_SUCC::default();
        clients
            .get_sender()
            .send_packet(P_FE2CL_PC_VEHICLE_OFF_SUCC, &pkt);
    }

    Ok(())
}

/// Item moves only address slot-indexed storage; quest items have no slots.
fn movable_location(raw: i32) -> FFResult<ItemLocation> {
    match raw.try_into()? {
        ItemLocation::QInven => Err(FFError::build(
            Severity::Warning,
            "Quest inventory items can't be moved by slot".to_owned(),
        )),
        location => Ok(location),
    }
}

fn slot_index(slot_num: i32) -> FFResult<usize> {
    usize::try_from(slot_num)
        .map_err(|_| FFError::build(Severity::Warning, format!("Bad slot number: {slot_num}")))
}

/// Mirrors OpenFusion's itemMoveHandler: slots 0-6 take their own item type,
/// the secondary hand slot takes weapons and the vehicle slot takes vehicles.
fn validate_equip_slot(location: ItemLocation, slot_num: usize, item: &Option<Item>) -> FFResult<()> {
    let Some(item) = item else {
        return Ok(());
    };
    if location != ItemLocation::Equip {
        return Ok(());
    }
    let fits = match slot_num as u32 {
        EQUIP_SLOT_HAND_EX => item.ty == ItemType::Hand,
        EQUIP_SLOT_VEHICLE => item.ty == ItemType::Vehicle,
        slot => item.ty as u32 == slot && slot <= EQUIP_SLOT_END,
    };
    if !fits {
        return Err(FFError::build(
            Severity::Warning,
            format!("Item {:?} can't be equipped in slot {}", item, slot_num),
        ));
    }
    Ok(())
}

/// Rejects bank slots that don't exist.
fn validate_bank_slot(location: ItemLocation, slot_num: i32) -> FFResult<()> {
    if location != ItemLocation::Bank {
        return Ok(());
    }
    if slot_num < 0 || slot_num as u32 >= SIZEOF_BANK_SLOT {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "Bank slot {} is out of range (max {})",
                slot_num, SIZEOF_BANK_SLOT
            ),
        ));
    }
    Ok(())
}

pub fn item_delete(pkt: Packet, client: &FFClient, state: &mut ShardServerState) -> FFResult<()> {
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_PC_ITEM_DELETE = pkt.get()?;
    let player = state.get_player_mut(pc_id)?;
    let location = pkt.eIL.try_into()?;
    if location != ItemLocation::Inven {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "Tried to delete item from invalid inventory: {:?}",
                location
            ),
        ));
    }

    player.set_item(location, pkt.iSlotNum as usize, None)?;

    let resp = sP_FE2CL_REP_PC_ITEM_DELETE_SUCC {
        eIL: pkt.eIL,
        iSlotNum: pkt.iSlotNum,
    };
    client.send_packet(P_FE2CL_REP_PC_ITEM_DELETE_SUCC, &resp);
    Ok(())
}

pub fn item_combination(
    pkt: Packet,
    client: &FFClient,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_ITEM_COMBINATION = pkt.get()?;
    (|| {
        let player = state.get_player_mut(client.get_player_id()?)?;
        if pkt.iCostumeItemSlot==pkt.iStatItemSlot || pkt.iCashItemSlot1!=0 || pkt.iCashItemSlot2!=0 || player.trade_id.is_some() {
            return Err(FFError::build(Severity::Warning,"Invalid Croc Pot slots or active trade".to_owned()));
        }

        let looks_item = player
            .get_item(ItemLocation::Inven, pkt.iCostumeItemSlot as usize)?
            .as_ref()
            .ok_or(FFError::build(
                Severity::Warning,
                format!("Costume item (slot {}) empty", pkt.iCostumeItemSlot),
            ))?;
        let looks_item_stats = looks_item.get_stats()?;
        let looks_item_rarity = looks_item_stats.rarity.ok_or(FFError::build(
            Severity::Warning,
            format!("Costume item has no rarity: {:?}", looks_item),
        ))?;

        let stats_item = player
            .get_item(ItemLocation::Inven, pkt.iStatItemSlot as usize)?
            .as_ref()
            .ok_or(FFError::build(
                Severity::Warning,
                format!("Stats item (slot {}) empty", pkt.iStatItemSlot),
            ))?;
        if looks_item.ty!=stats_item.ty || !(0..=3).contains(&(looks_item.ty as i32)) {
            return Err(FFError::build(Severity::Warning,"Croc Pot requires matching equipment types".to_owned()));
        }
        let stats_item_stats = stats_item.get_stats()?;
        let stats_item_rarity = stats_item_stats.rarity.ok_or(FFError::build(
            Severity::Warning,
            format!("Stats item has no rarity: {:?}", stats_item),
        ))?;

        let level_gap = (looks_item_stats.required_level - stats_item_stats.required_level).abs();
        let rarity_gap = (looks_item_rarity - stats_item_rarity).unsigned_abs();
        if rarity_gap > 3 {
            return Err(FFError::build(
                Severity::Warning,
                format!("Rarity gap {} larger than 3", rarity_gap),
            ));
        }

        let crocpot_data = tdata_get().get_crocpot_data(level_gap)?;
        let cost = looks_item_stats.buy_price.checked_mul(crocpot_data.price_multiplier_looks)
            .and_then(|looks| stats_item_stats.buy_price.checked_mul(crocpot_data.price_multiplier_stats).and_then(|stats|looks.checked_add(stats)))
            .ok_or_else(||FFError::build(Severity::Warning,"Croc Pot cost overflow".to_owned()))?;
        if player.get_taros() < cost {
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "Not enough taros to perform combination ({} < {})",
                    player.get_taros(),
                    cost
                ),
            ));
        }
        let taros_left = player.set_taros(player.get_taros() - cost);

        let looks_item = player
            .set_item(ItemLocation::Inven, pkt.iCostumeItemSlot as usize, None)
            .unwrap()
            .unwrap();
        let stats_item = player
            .set_item(ItemLocation::Inven, pkt.iStatItemSlot as usize, None)
            .unwrap()
            .unwrap();

        let success_chance =
            crocpot_data.base_chance * crocpot_data.rarity_diff_multipliers[rarity_gap as usize];
        let roll: f32 = random();
        let succeeded = roll < success_chance;
        if succeeded {
            // set the appearance of the stats item
            let combined_item = looks_item.combine_stats(&stats_item);

            // put it back (where the looks item came from, since that's what the client expects)
            player
                .set_item(
                    ItemLocation::Inven,
                    pkt.iCostumeItemSlot as usize,
                    Some(combined_item),
                )
                .unwrap();
        } else {
            // put the items back
            player
                .set_item(
                    ItemLocation::Inven,
                    pkt.iCostumeItemSlot as usize,
                    Some(looks_item),
                )
                .unwrap();
            player
                .set_item(
                    ItemLocation::Inven,
                    pkt.iStatItemSlot as usize,
                    Some(stats_item),
                )
                .unwrap();
        }

        let resp = sP_FE2CL_REP_PC_ITEM_COMBINATION_SUCC {
            iNewItemSlot: pkt.iCostumeItemSlot,
            sNewItem: (*player.get_item(ItemLocation::Inven, pkt.iCostumeItemSlot as usize)?).into_proto(),
            iStatItemSlot: pkt.iStatItemSlot,
            iCashItemSlot1: pkt.iCashItemSlot1,
            iCashItemSlot2: pkt.iCashItemSlot2,
            iCandy: taros_left as i32,
            iSuccessFlag: if succeeded { 1 } else { 0 },
        };

        client.send_packet(P_FE2CL_REP_PC_ITEM_COMBINATION_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_ITEM_COMBINATION_FAIL {
            iErrorCode: unused!(),
            iCostumeItemSlot: pkt.iCostumeItemSlot,
            iStatItemSlot: pkt.iStatItemSlot,
            iCashItemSlot1: pkt.iCashItemSlot1,
            iCashItemSlot2: pkt.iCashItemSlot2,
        };

        client.send_packet(P_FE2CL_REP_PC_ITEM_COMBINATION_FAIL, &resp);
    })
}

pub fn item_chest_open(
    pkt: Packet,
    client: &FFClient,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_ITEM_CHEST_OPEN = pkt.get()?;
    (|| {
        let player = state.get_player_mut(client.get_player_id()?)?;
        let location: ItemLocation = pkt.eIL.try_into()?;
        if location != ItemLocation::Inven {
            return Err(FFError::build(
                Severity::Warning,
                format!("C.R.A.T.E. not in main inventory: {:?}", location),
            ));
        }

        let chest = player
            .set_item(location, pkt.iSlotNum as usize, None)?
            .ok_or(FFError::build(
                Severity::Warning,
                format!("C.R.A.T.E. in empty slot: {}", pkt.iSlotNum),
            ))?;

        if chest.ty != ItemType::Chest {
            return Err(FFError::build(
                Severity::Warning,
                format!("Item is not a C.R.A.T.E.: {:?}", chest),
            ));
        }

        let reward_item = tdata_get()
            .get_item_from_crate(chest.id, player.get_style().iGender as i32)
            .unwrap_or_else(|e| {
                // If for some reason we can't find a valid drop for the crate,
                // give the player a random gumball instead.
                // This idea was taken from OpenFusion <3
                log_error(e);
                util::get_random_gumball()
            });

        player.set_item(location, pkt.iSlotNum as usize, Some(reward_item))?;

        let reward_pkt = PacketBuilder::new(P_FE2CL_REP_REWARD_ITEM)
            .with(&sP_FE2CL_REP_REWARD_ITEM {
                m_iCandy: player.get_taros() as i32,
                m_iFusionMatter: player.get_fusion_matter() as i32,
                m_iBatteryN: player.get_nano_potions() as i32,
                m_iBatteryW: player.get_weapon_boosts() as i32,
                iItemCnt: 1,
                iFatigue: 100,
                iFatigue_Level: 1,
                iNPC_TypeID: unused!(),
                iTaskID: unused!(),
            })
            .with(&sItemReward {
                sItem: Some(reward_item).into_proto(),
                eIL: location as i32,
                iSlotNum: pkt.iSlotNum,
            })
            .build()?;

        client.send_payload(reward_pkt);

        let resp = sP_FE2CL_REP_ITEM_CHEST_OPEN_SUCC {
            iSlotNum: pkt.iSlotNum,
        };

        client.send_packet(P_FE2CL_REP_ITEM_CHEST_OPEN_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_ITEM_CHEST_OPEN_FAIL {
            iSlotNum: pkt.iSlotNum,
            iErrorCode: unused!(),
        };
        client.send_packet(P_FE2CL_REP_ITEM_CHEST_OPEN_FAIL, &resp);
    })
}

pub fn vendor_start(pkt: Packet, client: &FFClient, state: &mut ShardServerState) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_VENDOR_START = pkt.get()?;
    (|| {
        validate_vendor(client, state, pkt.iNPC_ID, pkt.iVendorID)?;
        let resp = sP_FE2CL_REP_PC_VENDOR_START_SUCC {
            iNPC_ID: pkt.iNPC_ID,
            iVendorID: pkt.iVendorID,
        };

        client.send_packet(P_FE2CL_REP_PC_VENDOR_START_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_VENDOR_START_FAIL {
            iErrorCode: unused!(),
        };

        client.send_packet(P_FE2CL_REP_PC_VENDOR_START_FAIL, &resp);
    })
}

pub fn vendor_table_update(pkt: Packet, client: &FFClient) -> FFResult<()> {
    (|| {
        let pkt: &sP_CL2FE_REQ_PC_VENDOR_TABLE_UPDATE = pkt.get()?;

        let vendor_data = tdata_get().get_vendor_data(pkt.iVendorID)?;
        let resp = sP_FE2CL_REP_PC_VENDOR_TABLE_UPDATE_SUCC {
            item: vendor_data.as_arr()?,
        };

        client.send_packet(P_FE2CL_REP_PC_VENDOR_TABLE_UPDATE_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_VENDOR_TABLE_UPDATE_FAIL {
            iErrorCode: unused!(),
        };

        client.send_packet(P_FE2CL_REP_PC_VENDOR_TABLE_UPDATE_FAIL, &resp);
    })
}

pub fn vendor_item_buy(
    pkt: Packet,
    client: &FFClient,
    state: &mut ShardServerState,
    time: SystemTime,
) -> FFResult<()> {
    (|| {
        let pkt: &sP_CL2FE_REQ_PC_VENDOR_ITEM_BUY = pkt.get()?;

        validate_vendor(client, state, pkt.iNPC_ID, pkt.iVendorID)?;

        // sanitize the item
        let item: Option<Item> = pkt.Item.try_into_proto()?;
        let mut item = item.ok_or(FFError::build(
            Severity::Warning,
            "Tried to buy nothing".to_string(),
        ))?;

        if item.ty == ItemType::Vehicle {
            // set expiration date
            let duration_min = config_get().shard.vehicle_duration.get();
            let duration_sec = Duration::from_secs(duration_min * 60);
            let expires = time + duration_sec;
            item.set_expiry_time(expires);
        }

        let vendor_data = tdata_get().get_vendor_data(pkt.iVendorID)?;
        if !vendor_data.has_item(item.id, item.ty) {
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "Vendor {} doesn't sell item ({}, {:?})",
                    pkt.iVendorID, item.id, item.ty
                ),
            ));
        }

        let stats = item.get_stats()?;
        let price = stats.buy_price * item.quantity as u32;
        let player = state.get_player_mut(client.get_player_id()?)?;
        if !guide_shop_purchase_allowed(
            pkt.iVendorID,
            stats.mentor,
            stats.required_level,
            player.get_guide(),
            player.get_level(),
        ) {
            return Err(FFError::build(
                Severity::Warning,
                format!("Guide item ({}, {:?}) is unavailable to this player", item.id, item.ty),
            ));
        }
        if player.get_taros() < price {
            Err(FFError::build(
                Severity::Warning,
                format!(
                    "Not enough taros to buy item ({} < {})",
                    player.get_taros(),
                    price
                ),
            ))
        } else {
            player.set_item(ItemLocation::Inven, pkt.iInvenSlotNum as usize, Some(item))?;
            player.set_taros(player.get_taros() - price);

            let resp = sP_FE2CL_REP_PC_VENDOR_ITEM_BUY_SUCC {
                iCandy: player.get_taros() as i32,
                iInvenSlotNum: pkt.iInvenSlotNum,
                Item: Some(item).into_proto(),
            };

            client.send_packet(P_FE2CL_REP_PC_VENDOR_ITEM_BUY_SUCC, &resp);
            Ok(())
        }
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_VENDOR_ITEM_BUY_FAIL {
            iErrorCode: unused!(),
        };
        client.send_packet(P_FE2CL_REP_PC_VENDOR_ITEM_BUY_FAIL, &resp);
    })
}

pub fn vendor_item_sell(
    pkt: Packet,
    client: &FFClient,
    state: &mut ShardServerState,
) -> FFResult<()> {
    (|| {
        let pkt: &sP_CL2FE_REQ_PC_VENDOR_ITEM_SELL = pkt.get()?;
        let pc_id = client.get_player_id()?;
        let player = state.get_player_mut(pc_id)?;

        let item = player
            .get_item(ItemLocation::Inven, pkt.iInvenSlotNum as usize)?
            .ok_or(FFError::build(
                Severity::Warning,
                format!("Tried to sell what's in empty slot {}", pkt.iInvenSlotNum),
            ))?;
        let stats = item.get_stats()?;

        if !stats.sellable {
            return Err(FFError::build(
                Severity::Warning,
                format!("Item not sellable: {:?}", item),
            ));
        }

        let mut remaining_item =
            player.set_item(ItemLocation::Inven, pkt.iInvenSlotNum as usize, None)?;
        let quantity = pkt.iItemCnt as u16;
        let item = Item::split_items(&mut remaining_item, quantity);
        player
            .set_item(
                ItemLocation::Inven,
                pkt.iInvenSlotNum as usize,
                remaining_item,
            )
            .unwrap();

        let sell_price = stats.sell_price * quantity as u32;
        let new_taros = player.set_taros(player.get_taros() + sell_price);
        let buyback_list = state.buyback_lists.entry(pc_id).or_default();
        buyback_list.push(item.unwrap());

        let resp = sP_FE2CL_REP_PC_VENDOR_ITEM_SELL_SUCC {
            iCandy: new_taros as i32,
            iInvenSlotNum: pkt.iInvenSlotNum,
            Item: item.into_proto(),
            ItemStay: remaining_item.into_proto(),
        };

        client.send_packet(P_FE2CL_REP_PC_VENDOR_ITEM_SELL_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_VENDOR_ITEM_SELL_FAIL {
            iErrorCode: unused!(),
        };

        client.send_packet(P_FE2CL_REP_PC_VENDOR_ITEM_SELL_FAIL, &resp);
    })
}

pub fn vendor_item_restore_buy(
    pkt: Packet,
    client: &FFClient,
    state: &mut ShardServerState,
) -> FFResult<()> {
    (|| {
        let pc_id = client.get_player_id()?;
        let pkt: &sP_CL2FE_REQ_PC_VENDOR_ITEM_RESTORE_BUY = pkt.get()?;
        validate_vendor(client, state, pkt.iNPC_ID, pkt.iVendorID)?;

        let item: Option<Item> = pkt.Item.try_into_proto()?;
        let item: Item = item.ok_or(FFError::build(
            Severity::Warning,
            format!("Bad item for buyback {:?}", pkt.Item),
        ))?;
        let buyback_list = state.buyback_lists.get_mut(&pc_id).ok_or(FFError::build(
            Severity::Warning,
            format!("Player {} has not sold any items", pc_id),
        ))?;

        let mut found_idx = None;
        for (i, list_item) in buyback_list.iter().enumerate() {
            if *list_item == item {
                found_idx = Some(i);
                break;
            }
        }
        let found_idx = found_idx.ok_or(FFError::build(
            Severity::Warning,
            format!(
                "Player tried to buyback an item they didn't sell: {:?}",
                item
            ),
        ))?;

        let item = buyback_list.remove(found_idx);
        let cost = item.get_stats()?.sell_price * item.quantity as u32; // sell price is cost for buyback
        let player = state.get_player_mut(pc_id)?;

        if player.get_taros() < cost {
            Err(FFError::build(
                Severity::Warning,
                format!(
                    "Not enough taros to buyback item ({} < {})",
                    player.get_taros(),
                    cost
                ),
            ))
        } else {
            player.set_item(ItemLocation::Inven, pkt.iInvenSlotNum as usize, Some(item))?;
            let new_taros = player.set_taros(player.get_taros() - cost);

            let resp = sP_FE2CL_REP_PC_VENDOR_ITEM_RESTORE_BUY_SUCC {
                iCandy: new_taros as i32,
                iInvenSlotNum: pkt.iInvenSlotNum,
                Item: Some(item).into_proto(),
            };

            client.send_packet(P_FE2CL_REP_PC_VENDOR_ITEM_RESTORE_BUY_SUCC, &resp);
            Ok(())
        }
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_VENDOR_ITEM_RESTORE_BUY_FAIL {
            iErrorCode: unused!(),
        };

        client.send_packet(P_FE2CL_REP_PC_VENDOR_ITEM_RESTORE_BUY_FAIL, &resp);
    })
}

pub fn vendor_battery_buy(
    pkt: Packet,
    client: &FFClient,
    state: &mut ShardServerState,
) -> FFResult<()> {
    const BATTERY_TYPE_BOOST: i16 = 3;
    const BATTERY_TYPE_POTION: i16 = 4;

    (|| {
        let pkt: &sP_CL2FE_REQ_PC_VENDOR_BATTERY_BUY = pkt.get()?;
        validate_vendor(client, state, pkt.iNPC_ID, pkt.iVendorID)?;

        let battery_type = pkt.Item.iID;
        let mut quantity = pkt.Item.iOpt as u32 * 100;

        let player = state.get_player_mut(client.get_player_id()?)?;
        match battery_type {
            BATTERY_TYPE_BOOST => {
                quantity = min(player.get_weapon_boosts() + quantity, PC_BATTERY_MAX)
                    - player.get_weapon_boosts();
            }
            BATTERY_TYPE_POTION => {
                quantity = min(player.get_nano_potions() + quantity, PC_BATTERY_MAX)
                    - player.get_nano_potions();
            }
            other => {
                return Err(FFError::build(
                    Severity::Warning,
                    format!("Bad battery type: {}", other),
                ));
            }
        }

        let cost = quantity;
        if player.get_taros() < cost {
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "Not enough taros to buyback item ({} < {})",
                    player.get_taros(),
                    cost
                ),
            ));
        }

        match battery_type {
            BATTERY_TYPE_BOOST => {
                player.set_weapon_boosts(player.get_weapon_boosts() + quantity);
            }
            BATTERY_TYPE_POTION => {
                player.set_nano_potions(player.get_nano_potions() + quantity);
            }
            other => {
                return Err(FFError::build(
                    Severity::Warning,
                    format!("Bad battery type: {}", other),
                ));
            }
        }
        let taros_new = player.set_taros(player.get_taros() - cost);

        let resp = sP_FE2CL_REP_PC_VENDOR_BATTERY_BUY_SUCC {
            iCandy: taros_new as i32,
            iBatteryW: player.get_weapon_boosts() as i32,
            iBatteryN: player.get_nano_potions() as i32,
        };

        client.send_packet(P_FE2CL_REP_PC_VENDOR_BATTERY_BUY_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_VENDOR_BATTERY_BUY_FAIL {
            iErrorCode: unused!(),
        };
        client.send_packet(P_FE2CL_REP_PC_VENDOR_BATTERY_BUY_FAIL, &resp);
    })
}

pub fn streetstall_cancel(client: &FFClient) -> FFResult<()> {
    // streetstalls are scrapped, but the client still sends this packet
    // if the UI is brought up with /Store and then closed. it gets
    // softlocked if we don't respond
    let resp = sP_FE2CL_PC_STREETSTALL_REP_CANCEL_SUCC {
        iPCCharState: unused!(),
    };
    client.send_packet(P_FE2CL_PC_STREETSTALL_REP_CANCEL_SUCC, &resp);
    Ok(())
}

fn guide_shop_purchase_allowed(
    vendor_id: i32,
    item_mentor: Option<i16>,
    required_level: i16,
    player_guide: PlayerGuide,
    player_level: i16,
) -> bool {
    !(643..=650).contains(&vendor_id)
        || (item_mentor == Some(player_guide as i16) && player_level >= required_level)
}

#[cfg(test)]
mod guide_shop_tests {
    use super::*;

    #[test]
    fn paired_guide_shops_accept_only_their_guide_and_required_level() {
        for vendor in [643, 644] {
            assert!(guide_shop_purchase_allowed(vendor, Some(1), 6, PlayerGuide::Edd, 6));
            assert!(!guide_shop_purchase_allowed(vendor, Some(1), 6, PlayerGuide::Ben, 20));
            assert!(!guide_shop_purchase_allowed(vendor, Some(1), 6, PlayerGuide::Edd, 5));
            assert!(!guide_shop_purchase_allowed(vendor, None, 1, PlayerGuide::Edd, 20));
        }
        assert!(guide_shop_purchase_allowed(651, None, 1, PlayerGuide::Edd, 1));
    }
}

fn validate_vendor(
    client: &FFClient,
    state: &mut ShardServerState,
    npc_id: i32,
    vendor_id: i32,
) -> FFResult<()> {
    let pc_id = client.get_player_id()?;
    let npc = state.get_npc(npc_id)?;
    if npc.ty != vendor_id {
        return Err(FFError::build(
            Severity::Warning,
            format!("Vendor {} has type {} instead of {}", npc_id, npc.ty, vendor_id),
        ));
    }
    state
        .entity_map
        .validate_proximity(
            &[EntityID::Player(pc_id), EntityID::NPC(npc_id)],
            RANGE_INTERACT,
        )
        .map_err(|e| {
            e.with_parent(FFError::build(
                Severity::Warning,
                format!("Vendor {} not close enough", npc_id),
            ))
        })
}

//
// Usable general items.
//

/// Skill that backs a gumball's nano boost (`NanoStimPak`).
const SKILL_ID_NANO_STIMPAK: i16 = 144;

/// Uses a general item out of the main inventory. Right now that means
/// gumballs; Nanocom boosters need three extra equip slots the client doesn't
/// have, so they're politely rejected.
pub fn item_use(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_ITEM_USE = pkt.get()?;
    let slot_num = pkt.iSlotNum;
    let nano_slot = pkt.iNanoSlot;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    (|| {
        let location: ItemLocation = pkt.eIL.try_into()?;
        if location != ItemLocation::Inven {
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "Items can only be used from the inventory (got {:?})",
                    location
                ),
            ));
        }
        if slot_num < 0 || slot_num >= SIZEOF_INVEN_SLOT as i32 {
            return Err(FFError::build(
                Severity::Warning,
                format!("Bad inventory slot {}", slot_num),
            ));
        }
        let slot_num = slot_num as usize;

        let player = state.get_player(pc_id)?;
        let item = player
            .get_item(ItemLocation::Inven, slot_num)?
            .ok_or_else(|| {
                FFError::build(
                    Severity::Warning,
                    format!("No item in slot {} to use", slot_num),
                )
            })?;
        if item.ty != ItemType::General {
            return Err(FFError::build(
                Severity::Warning,
                format!("Item {:?} isn't usable", item),
            ));
        }

        match item.id {
            id if (ID_GUMBALL..ID_GUMBALL + 3).contains(&id) => {
                use_gumball(pc_id, slot_num, nano_slot, id, clients, state)
            }
            other => Err(FFError::build(
                Severity::Warning,
                format!("General item {} is not usable", other),
            )),
        }
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_ITEM_USE_FAIL {
            iErrorCode: unused!(),
        };
        client.send_packet(P_FE2CL_REP_PC_ITEM_USE_FAIL, &resp);
    })
}

/// Eats a gumball, boosting the nano in `nano_slot` for the skill table's
/// duration. The gumball's flavor has to match the nano's style.
fn use_gumball(
    pc_id: i32,
    slot_num: usize,
    nano_slot: i16,
    gumball_id: i16,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    if !(0..SIZEOF_NANO_CARRY_SLOT as i16).contains(&nano_slot) {
        return Err(FFError::build(
            Severity::Warning,
            format!("Bad nano slot {}", nano_slot),
        ));
    }

    let required_style = match gumball_id - ID_GUMBALL {
        0 => CombatStyle::Adaptium,
        1 => CombatStyle::Blastons,
        _ => CombatStyle::Cosmix,
    };

    let player = state.get_player(pc_id)?;
    let nano_id = player
        .get_equipped_nano_id(nano_slot as usize)
        .ok_or_else(|| {
            FFError::build(
                Severity::Warning,
                format!("No nano equipped in slot {}", nano_slot),
            )
        })?;
    let nano_style = tdata_get().get_nano_stats(nano_id)?.style;
    if nano_style != required_style {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "Gumball {} ({:?}) doesn't match nano {} ({:?})",
                gumball_id, required_style, nano_id, nano_style
            ),
        ));
    }

    let skill = tdata_get().get_skill(SKILL_ID_NANO_STIMPAK)?;
    let buff_id = match nano_slot {
        0 => BuffID::StimPakSlot1,
        1 => BuffID::StimPakSlot2,
        _ => BuffID::StimPakSlot3,
    };

    // consume one gumball
    let player = state.get_player_mut(pc_id)?;
    let slot = player.get_item_mut(ItemLocation::Inven, slot_num)?;
    Item::split_items(slot, 1).ok_or_else(|| {
        FFError::build(
            Severity::Warning,
            format!("Couldn't consume the gumball in slot {}", slot_num),
        )
    })?;
    let remaining = *player.get_item(ItemLocation::Inven, slot_num)?;

    let player_eid = player.get_id();
    skills::apply_skill_buff(
        player,
        skill,
        0,
        BuffType::CashItem,
        Some(buff_id),
        Some(player_eid),
    )?;

    let resp = PacketBuilder::new(P_FE2CL_REP_PC_ITEM_USE_SUCC)
        .with(&sP_FE2CL_REP_PC_ITEM_USE_SUCC {
            iPC_ID: pc_id,
            eIL: ItemLocation::Inven as i32,
            iSlotNum: slot_num as i32,
            RemainItem: remaining.into_proto(),
            iSkillID: SKILL_ID_NANO_STIMPAK,
            eST: skill.skill_type as i32,
            iTargetCnt: 1,
        })
        .with(&sSkillResult_Buff {
            eCT: player.get_char_type() as i32,
            iID: pc_id,
            bProtected: unused!(),
            iConditionBitFlag: player.get_condition_bit_flag(),
        })
        .build()?;
    clients.get_sender().send_payload(resp);

    // everyone nearby should see the nano light up
    let bcast = sP_FE2CL_PC_ITEM_USE {
        iPC_ID: pc_id,
        iSkillID: SKILL_ID_NANO_STIMPAK,
        eST: skill.skill_type as i32,
        iTargetCnt: 1,
    };
    state.entity_map.for_each_around(player_eid, |c| {
        c.send_packet(P_FE2CL_PC_ITEM_USE, &bcast);
    });

    Ok(())
}

//
// Banking.
//

/// Error code the client shows when a bank is locked behind a membership card.
const ERROR_CODE_BANK_NO_MEMBERSHIP: i32 = 2;

/// Which bank a banker NPC opens, and the membership card it requires.
/// The main bank (0) has no banker of its own and no card.
fn bank_for_npc_type(npc_type: i32) -> Option<(usize, i16)> {
    match npc_type {
        TYPE_GOLD_BANKER => Some((1, ID_GOLD_MEMBERSHIP_CARD)),
        TYPE_EMERALD_BANKER => Some((2, ID_EMERALD_MEMBERSHIP_CARD)),
        TYPE_RUBY_BANKER => Some((3, ID_RUBY_MEMBERSHIP_CARD)),
        TYPE_SAPPHIRE_BANKER => Some((4, ID_SAPPHIRE_MEMBERSHIP_CARD)),
        _ => None,
    }
}

pub fn bank_open(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_PC_BANK_OPEN = pkt.get()?;
    // which banker was talked to decides which bank opens
    let npc_id = pkt.iNPC_ID;

    (|| {
        // work out which bank was asked for
        let bank_num = if npc_id == 0 {
            0
        } else {
            let npc_type = state.get_npc(npc_id)?.ty;
            match bank_for_npc_type(npc_type) {
                Some((bank_num, card_id)) => {
                    let player = state.get_player(pc_id)?;
                    if !player.has_item_anywhere(ItemType::General, card_id) {
                        // a locked membership bank gets its own error code so
                        // the client explains why
                        let resp = sP_FE2CL_REP_PC_BANK_OPEN_FAIL {
                            iErrorCode: ERROR_CODE_BANK_NO_MEMBERSHIP,
                        };
                        client.send_packet(P_FE2CL_REP_PC_BANK_OPEN_FAIL, &resp);
                        return Ok(());
                    }
                    bank_num
                }
                // any other NPC just opens the ordinary bank
                None => 0,
            }
        };

        let player = state.get_player_mut(pc_id)?;
        player.set_active_bank(bank_num)?;

        let mut resp = sP_FE2CL_REP_PC_BANK_OPEN_SUCC {
            iExtraBank: bank_num as i32,
            ..Default::default()
        };
        for (slot_num, item) in resp.aBank.iter_mut().enumerate() {
            *item = (*player.get_bank_slot(bank_num, slot_num)?).into_proto();
        }
        client.send_packet(P_FE2CL_REP_PC_BANK_OPEN_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_BANK_OPEN_FAIL {
            iErrorCode: unused!(),
        };
        client.send_packet(P_FE2CL_REP_PC_BANK_OPEN_FAIL, &resp);
    })
}

pub fn bank_close(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let _pkt: &sP_CL2FE_REQ_PC_BANK_CLOSE = pkt.get()?;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    // fall back to the main bank so a later item move can't land in a
    // membership bank the player no longer has open
    state.get_player_mut(pc_id)?.set_active_bank(0)?;

    let resp = sP_FE2CL_REP_PC_BANK_CLOSE_SUCC { iPC_ID: pc_id };
    client.send_packet(P_FE2CL_REP_PC_BANK_CLOSE_SUCC, &resp);
    Ok(())
}
