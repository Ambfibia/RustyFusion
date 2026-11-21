use crate::{
    chunk::InstanceID,
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
    state::ShardServerState,
    tabledata::tdata_get,
    util,
};

pub fn regist_transportation_location(
    pkt: Packet,
    client: &FFClient,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_REGIST_TRANSPORTATION_LOCATION = pkt.get()?;
    (|| {
            let pc_id = client.get_player_id()?;
            let npc = state.get_npc(pkt.iNPC_ID)?;
            let npc_type = npc.ty;
            state
                .entity_map
                .validate_proximity(&[EntityID::Player(pc_id), npc.get_id()], RANGE_INTERACT)?;

            let player = state.get_player_mut(pc_id)?;
            let transport_type: TransportationType = pkt.eTT.try_into()?;
            match transport_type {
                TransportationType::Warp => {
                    let location_data = tdata_get().get_scamper_data(pkt.iLocationID)?;
                    if location_data.npc_type != npc_type {
                        return Err(FFError::build(
                            Severity::Warning,
                            format!(
                                "Player {} tried to register a S.C.A.M.P.E.R. location with the wrong NPC type",
                                player.get_player_id()
                            ),
                        ));
                    }
                    player.unlock_scamper_location(pkt.iLocationID)?;
                }
                TransportationType::Wyvern => {
                    let location_data = tdata_get().get_skyway_data(pkt.iLocationID)?;
                    if location_data.npc_type != npc_type {
                        return Err(FFError::build(
                            Severity::Warning,
                            format!(
                                "Player {} tried to register a Skyway location with the wrong NPC type",
                                player.get_player_id()
                            ),
                        ));
                    }
                    player.unlock_skyway_location(pkt.iLocationID)?;
                }
                other => {
                    return Err(FFError::build(
                        Severity::Warning,
                        format!("Can't register transportation type {:?}", other),
                    ));
                }
            }

            let resp = sP_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_SUCC {
                eTT: pkt.eTT,
                iLocationID: pkt.iLocationID,
                iWarpLocationFlag: player.flags.scamper_flags.get_chunk(0).unwrap(),
                aWyvernLocationFlag: player.flags.skyway_flags.to_array().unwrap(),
            };

            client.send_packet(P_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_SUCC, &resp);
            Ok(())
        })().catch_fail(
        || {
            let resp = sP_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_FAIL {
                eTT: pkt.eTT,
                iLocationID: pkt.iLocationID,
                iErrorCode: unused!(),
            };

            client.send_packet(P_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_FAIL, &resp);
        }
    )
}

pub fn warp_use_transportation(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_WARP_USE_TRANSPORTATION = pkt.get()?;

    (|| {
        let client = clients.get_sender();
        let pc_id = client.get_player_id()?;

        let npc = state.get_npc(pkt.iNPC_ID)?;
        let npc_type = npc.ty;
        state
            .entity_map
            .validate_proximity(&[EntityID::Player(pc_id), npc.get_id()], RANGE_INTERACT)?;

        let player = state.get_player_mut(pc_id)?;
        let trip_id = pkt.iTransporationID;
        let trip = tdata_get().get_trip_data(trip_id)?;
        if player.get_taros() < trip.cost {
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "Player {} doesn't have enough taros to warp",
                    player.get_player_id()
                ),
            ));
        }

        let new_taros = player.get_taros() - trip.cost;
        match trip.transportation_type {
            TransportationType::Warp => {
                let src_data = tdata_get().get_scamper_data(trip.start_location)?;
                if !player
                    .is_scamper_location_unlocked(trip.start_location)
                    .unwrap()
                {
                    return Err(FFError::build(
                        Severity::Warning,
                        format!(
                            "Player {} tried to warp from an unregistered S.C.A.M.P.E.R. location",
                            player.get_player_id()
                        ),
                    ));
                }

                let dest_data = tdata_get().get_scamper_data(trip.end_location)?;
                if !player
                    .is_scamper_location_unlocked(trip.end_location)
                    .unwrap()
                {
                    return Err(FFError::build(
                        Severity::Warning,
                        format!(
                            "Player {} tried to warp to an unregistered S.C.A.M.P.E.R. location",
                            player.get_player_id()
                        ),
                    ));
                }

                if src_data.npc_type != npc_type {
                    return Err(FFError::build(
                        Severity::Warning,
                        format!(
                            "Player {} tried to warp with the wrong NPC type",
                            player.get_player_id()
                        ),
                    ));
                }

                player.set_taros(new_taros);
                player.set_position(dest_data.pos);
            }
            TransportationType::Wyvern => {
                let src_data = tdata_get().get_skyway_data(trip.start_location)?;
                if !player
                    .is_skyway_location_unlocked(trip.start_location)
                    .unwrap()
                {
                    return Err(FFError::build(
                        Severity::Warning,
                        format!(
                            "Player {} tried to warp from an unregistered Skyway location",
                            player.get_player_id()
                        ),
                    ));
                }

                if !player.is_skyway_location_unlocked(trip.end_location)? {
                    return Err(FFError::build(
                        Severity::Warning,
                        format!(
                            "Player {} tried to warp to an unregistered Skyway location",
                            player.get_player_id()
                        ),
                    ));
                }

                if src_data.npc_type != npc_type {
                    return Err(FFError::build(
                        Severity::Warning,
                        format!(
                            "Player {} tried to warp with the wrong NPC type",
                            player.get_player_id()
                        ),
                    ));
                }

                let path = tdata_get().get_skyway_path(trip.route_number)?;
                player.deactivate_nano();
                player.start_skyway_ride(trip, path);
                // we don't charge the player until the ride is done
            }
            other => {
                return Err(FFError::build(
                    Severity::Warning,
                    format!("Can't warp with transportation type {:?}", other),
                ));
            }
        }

        let new_pos = player.get_position();
        let resp = sP_FE2CL_REP_PC_WARP_USE_TRANSPORTATION_SUCC {
            eTT: trip.transportation_type as i32,
            iX: new_pos.x,
            iY: new_pos.y,
            iZ: new_pos.z,
            iCandy: new_taros as i32,
        };

        client.send_packet(P_FE2CL_REP_PC_WARP_USE_TRANSPORTATION_SUCC, &resp);

        if trip.transportation_type == TransportationType::Wyvern {
            helpers::broadcast_monkey(pc_id, RideType::Wyvern, state);
        }

        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_WARP_USE_TRANSPORTATION_FAIL {
            iTransportationID: pkt.iTransporationID,
            iErrorCode: unused!(),
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_PC_WARP_USE_TRANSPORTATION_FAIL, &resp);
    })
}

pub fn warp_use_npc(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_WARP_USE_NPC = pkt.get()?;

    (|| {
        let item_remaining = do_warp(
            clients,
            state,
            Some(pkt.iNPC_ID),
            pkt.iWarpID,
            pkt.eIL1,
            pkt.iItemSlot1 as usize,
            pkt.eIL2,
            pkt.iItemSlot2 as usize,
        )?;

        let client = clients.get_sender();
        let player = state.get_player(client.get_player_id().unwrap()).unwrap();
        let pos = player.get_position();
        let taros_left = player.get_taros();
        let resp = sP_FE2CL_REP_PC_WARP_USE_NPC_SUCC {
            iX: pos.x,
            iY: pos.y,
            iZ: pos.z,
            eIL: pkt.eIL2,
            iItemSlotNum: pkt.iItemSlot2,
            Item: item_remaining.into_proto(),
            iCandy: taros_left as i32,
        };

        client.send_packet(P_FE2CL_REP_PC_WARP_USE_NPC_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_WARP_USE_NPC_FAIL {
            iErrorCode: unused!(),
        };
        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_PC_WARP_USE_NPC_FAIL, &resp);
    })
}

pub fn time_to_go_warp(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_TIME_TO_GO_WARP = pkt.get()?;

    (|| {
        let player = state.get_player(clients.get_sender().get_player_id()?)?;
        if player.is_future_done() {
            return Err(FFError::build(
                Severity::Warning,
                format!("Player {} is in the past", player.get_player_id()),
            ));
        }

        let item_remaining = do_warp(
            clients,
            state,
            None,
            ID_TIME_MACHINE_WARP,
            pkt.eIL1,
            pkt.iItemSlot1 as usize,
            pkt.eIL2,
            pkt.iItemSlot2 as usize,
        )?;

        let client = clients.get_sender();
        let player = state.get_player(client.get_player_id().unwrap()).unwrap();
        let pos = player.get_position();
        let taros_left = player.get_taros();
        let resp = sP_FE2CL_REP_PC_WARP_USE_NPC_SUCC {
            iX: pos.x,
            iY: pos.y,
            iZ: pos.z,
            eIL: pkt.eIL2,
            iItemSlotNum: pkt.iItemSlot2,
            Item: item_remaining.into_proto(),
            iCandy: taros_left as i32,
        };

        client.send_packet(P_FE2CL_REP_PC_WARP_USE_NPC_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_WARP_USE_NPC_FAIL {
            iErrorCode: unused!(),
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_PC_WARP_USE_NPC_FAIL, &resp);
    })
}

#[allow(clippy::too_many_arguments)]
fn do_warp(
    clients: &ClientMap,
    state: &mut ShardServerState,
    npc_id: Option<i32>,
    warp_id: i32,
    req_item_location_ord: i32,
    req_item_slot: usize,
    req_item_consume_location_ord: i32,
    req_item_consume_slot: usize,
) -> FFResult<Option<Item>> {
    let warp_data = tdata_get().get_warp_data(warp_id)?;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    let mut to_past = false;

    if let Some(npc_id) = npc_id {
        let npc = state.get_npc(npc_id)?;
        if npc.ty != warp_data.npc_type {
            return Err(FFError::build(
                Severity::Warning,
                format!("Player {} tried to warp with the wrong NPC type", pc_id),
            ));
        }
        to_past = npc.ty == TYPE_TIME_MACHINE;

        // proximity check.
        // for some reason, the time machine NPC's range is HUGE
        // but we don't need to check anyway since it's a special case
        if npc.ty != TYPE_TIME_MACHINE {
            state
                .entity_map
                .validate_proximity(&[EntityID::Player(pc_id), npc.get_id()], RANGE_INTERACT)?;
        }
    }

    let player = state.get_player(pc_id)?;

    if player.get_taros() < warp_data.cost {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "Player {} doesn't have enough taros to warp",
                player.get_player_id()
            ),
        ));
    }

    if player.get_level() < warp_data.min_level {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "Player {} isn't a high enough level to warp ({} < {})",
                player.get_player_id(),
                player.get_level(),
                warp_data.min_level
            ),
        ));
    }

    if let Some((_, task_id)) = warp_data.req_task {
        if !player.mission_journal.get_current_task_ids().contains(&task_id) {
            return Err(FFError::build(
                Severity::Warning,
                "Required warp quest stage is not active".into(),
            ));
        }
    }
    if let Some(mission_id) = warp_data.req_completed_mission {
        if !player.mission_journal.is_mission_completed(mission_id)? {
            return Err(FFError::build(
                Severity::Warning,
                "Required warp quest is not completed".into(),
            ));
        }
    }
    if let Some((item_type, item_id)) = warp_data.req_item {
        let count = if item_type == ItemType::Quest {
            player.get_quest_item_count(item_id)
        } else {
            let item = player
                .get_item(req_item_location_ord.try_into()?, req_item_slot)?
                .as_ref();
            if !item.is_some_and(|item| item.ty == item_type && item.id == item_id) {
                return Err(FFError::build(
                    Severity::Warning,
                    "Required warp item slot does not match".into(),
                ));
            }
            if warp_data.req_item_count == 0 {
                1
            } else {
                let slots = player.find_items(ItemLocation::Inven, |item| {
                    item.ty == item_type && item.id == item_id
                });
                let mut count = 0;
                for slot in slots {
                    if let Some(item) = player.get_item(ItemLocation::Inven, slot)? {
                        count += usize::from(item.quantity.max(1));
                    }
                }
                count
            }
        };
        if !warp_item_quantity_allowed(warp_data.req_item_count, count) {
            return Err(FFError::build(
                Severity::Warning,
                "Not enough required warp items".into(),
            ));
        }
    }

    // group proximity check
    let position = player.get_position();
    if warp_data.is_group_warp {
        if let Some(group_id) = player.group_id {
            let group = state.groups.get(&group_id).unwrap();
            for member_id in group.get_member_ids() {
                if let EntityID::Player(member_pc_id) = *member_id {
                    if member_pc_id == pc_id {
                        continue;
                    }

                    let member = state.get_player(member_pc_id).unwrap();
                    if member.instance_id != player.instance_id || member.get_position().distance_to(&position) > RANGE_GROUP_WARP {
                        return Err(FFError::build(
                            Severity::Warning,
                            format!(
                                "Player {} tried to group warp with a group member too far away",
                                pc_id
                            ),
                        ));
                    }
                }
            }
        }
    }

    // good to warp
    let player = state.get_player_mut(pc_id)?;

    let mut item_consumed = None;
    if let Some((item_type, item_id)) = warp_data.req_item_consumed {
        let item = player.get_item_mut(
            req_item_consume_location_ord.try_into()?,
            req_item_consume_slot,
        )?;
        if !item
            .as_mut()
            .is_some_and(|item| item.ty == item_type && item.id == item_id)
        {
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "Player {} doesn't have the required consumed item ({:?}) to warp",
                    player.get_player_id(),
                    warp_data.req_item_consumed
                ),
            ));
        }
        Item::split_items(item, 1); // consume item
        item_consumed = *item;
    }
    player.set_taros(player.get_taros() - warp_data.cost);

    let mut pc_ids_to_warp = vec![pc_id];
    if warp_data.is_group_warp {
        if let Some(group_id) = player.group_id {
            let group = state.groups.get(&group_id).unwrap();
            for member_id in group.get_member_ids() {
                if let EntityID::Player(member_pc_id) = *member_id {
                    if member_pc_id == pc_id {
                        continue;
                    }
                    pc_ids_to_warp.push(member_pc_id);
                }
            }
        }
    }

    let instance_num = if warp_data.is_instance {
        Some(util::rand_range_inclusive(1, u32::MAX))
    } else {
        None
    };

    for warping_pc_id in pc_ids_to_warp {
        // A committed portal warp ends the run, including group passengers.
        // Do this after all warp validation, so a failed warp keeps the race.
        // The warp success is the authoritative client lifecycle edge; sending
        // a race-cancel reply here would be an uncorrelated transaction.
        state.ongoing_races.remove(&warping_pc_id);
        let player = state.get_player_mut(warping_pc_id)?;
        let client = player.get_client().unwrap();
        player.clear_recall_point();
        player.set_pre_warp();
        player.set_position(warp_data.pos);
        let instance_id = InstanceID {
            channel_num: player.instance_id.channel_num,
            map_num: warp_data.map_num,
            instance_num,
        };
        player.instance_id = instance_id;

        // force vehicle dismount
        player.vehicle_speed = None;

        if to_past {
            player.set_future_done();

            // remove all active tasks
            for task_id in player.mission_journal.get_current_task_ids() {
                let task = player.mission_journal.remove_task(task_id).unwrap();
                for item_id in &task.get_task_def().delete_qitems {
                    let qitem_slot = player.set_quest_item_count(*item_id, 0).unwrap();
                    // client doesn't automatically delete qitems clientside
                    let pkt = sP_FE2CL_REP_PC_ITEM_DELETE_SUCC {
                        eIL: ItemLocation::QInven as i32,
                        iSlotNum: qitem_slot as i32,
                    };
                    client.send_packet(P_FE2CL_REP_PC_ITEM_DELETE_SUCC, &pkt);
                }

                let resp = sP_FE2CL_REP_PC_TASK_STOP_SUCC { iTaskNum: task_id };
                client.send_packet(P_FE2CL_REP_PC_TASK_STOP_SUCC, &resp);
            }
        }

        // generic NPC warp packet for players that are warping along.
        // caller should reply with the correct packet for the initiator
        if warping_pc_id != pc_id {
            let resp = sP_FE2CL_REP_PC_WARP_USE_NPC_SUCC {
                iX: player.get_position().x,
                iY: player.get_position().y,
                iZ: player.get_position().z,
                eIL: ItemLocation::end(),
                iItemSlotNum: unused!(),
                Item: None.into_proto(),
                iCandy: player.get_taros() as i32,
            };
            client.send_packet(P_FE2CL_REP_PC_WARP_USE_NPC_SUCC, &resp);
        }

        helpers::broadcast_state(warping_pc_id, player.get_state_bit_flag(), state);

        // we remove the player from the chunk here and wait for PC_LOADING_COMPLETE to put them back.
        // it needs to be done this way or the client will miss the PC/NPC_ENTER packets.
        state.entity_map.update(EntityID::Player(warping_pc_id), None, true);
    }

    Ok(item_consumed)
}

// A zero authoring count preserves the legacy one-item presence requirement.
fn warp_item_quantity_allowed(required: u16, available: usize) -> bool {
    available >= usize::from(required.max(1))
}

#[cfg(test)]
mod warp_condition_tests {
    use super::*;
    #[test]
    fn group_warp_detaches_each_passenger_and_rejects_other_instances() {
        use crate::{chunk::{ChunkCoords, TickMode}, entity::{Group, Player}, net::{ClientMetadata, ClientType}, tabledata::tdata_init};
        use std::collections::HashMap;
        tdata_init().unwrap();
        let mut state = ShardServerState::default();
        let mut connections = HashMap::new();
        let mut receivers = Vec::new();
        let group_id = uuid::Uuid::new_v4();
        let mut group = Group::new(EntityID::Player(1)); group.add_member(EntityID::Player(2)).unwrap();
        state.groups.insert(group_id, group);
        for id in [1, 2] {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel(); receivers.push(rx);
            let client = FFClient::new(tx, ClientMetadata::new(format!("127.0.0.1:{}", 39300 + id).parse().unwrap(),
                Some(ClientType::GameClient { account_id: id as i64, serial_key: id as i64, pc_id: Some(id) })));
            let mut player = Player::new(id as i64, 0); player.set_player_id(id); player.set_client(client.clone()); player.group_id = Some(group_id);
            let chunk = ChunkCoords::from_pos_inst(player.get_position(), player.instance_id);
            state.entity_map.track(Box::new(player), TickMode::Always);
            state.entity_map.update(EntityID::Player(id), Some(chunk), false); connections.insert(id as usize, client);
        }
        let clients = ClientMap::new(1, &connections);
        state.get_player_mut(2).unwrap().instance_id.map_num = 1;
        for id in [1, 2] {
            let player = state.get_player_mut(id).unwrap();
            player.register_recall_point(100 + id, player.get_position());
        }
        assert!(do_warp(&clients, &mut state, None, 4, 4, 0, 4, 0).is_err());
        for id in [1, 2] { assert!(state.get_player(id).unwrap().get_recall_point().is_some(), "rejected group warp preserves passenger {id}'s Recall registration"); }
        assert!(state.entity_map.get_entity_chunk(EntityID::Player(1)).is_some());
        assert!(state.entity_map.get_entity_chunk(EntityID::Player(2)).is_some());
        state.get_player_mut(2).unwrap().instance_id = state.get_player(1).unwrap().instance_id;
        let player = state.get_player_mut(2).unwrap();
        player.register_recall_point(102, player.get_position());
        do_warp(&clients, &mut state, None, 4, 4, 0, 4, 0).unwrap();
        assert_eq!(state.get_player(1).unwrap().instance_id, state.get_player(2).unwrap().instance_id);
        for id in [1, 2] {
            assert!(state.entity_map.get_entity_chunk(EntityID::Player(id)).is_none(), "passenger {id} waits for loading complete");
            assert!(state.get_player(id).unwrap().get_recall_point().is_none(), "committed group warp clears passenger {id}'s Recall registration");
        }
    }
    #[test]
    fn warp_count_preserves_legacy_presence_and_checks_exact_threshold() {
        assert!(!warp_item_quantity_allowed(0,0));
        assert!(warp_item_quantity_allowed(0,1));
        assert!(!warp_item_quantity_allowed(3,2));
        assert!(warp_item_quantity_allowed(3,3));
        assert!(warp_item_quantity_allowed(3,4));
    }

    #[test]
    fn portal_commit_retires_race_but_rejected_warp_preserves_it() {
        use std::collections::{HashMap, HashSet};
        use crate::{
            chunk::{EntityMap, TickMode}, entity::Player,
            net::{ClientMetadata, ClientType}, racing::RaceState, tabledata::tdata_init,
        };
        tdata_init().unwrap();
        let warp_id = (1..10000).find(|id| tdata_get().get_warp_data(*id).is_ok_and(|warp| {
            warp.req_task.is_none() && warp.req_completed_mission.is_none()
                && warp.req_item.is_none() && warp.req_item_consumed.is_none()
        })).expect("published unconditional portal");
        let warp = tdata_get().get_warp_data(warp_id).unwrap();
        let destination = warp.pos;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let client = FFClient::new(tx, ClientMetadata::new(
            "127.0.0.1:39013".parse().unwrap(),
            Some(ClientType::GameClient { account_id: 1, serial_key: 1, pc_id: Some(1) }),
        ));
        let mut player = Player::new(1, 0);
        player.set_player_id(1);
        player.set_client(client.clone());
        player.set_level(warp.min_level.max(1)).unwrap();
        player.set_taros(warp.cost);
        let mut state = ShardServerState {
            shard_id: None, login_server_conn_id: None, login_data: HashMap::new(),
            entity_map: EntityMap::default(), buyback_lists: HashMap::new(),
            ongoing_trades: HashMap::new(), groups: HashMap::new(), player_uid_to_id: HashMap::new(),
            pending_entering_uids: HashSet::new(), pending_buff_effects: Vec::new(),
            ongoing_races: HashMap::from([(1, RaceState::new(1, 1, RacingMode::Record, 0, None))]),
        };
        state.entity_map.track(Box::new(player), TickMode::Always);
        let clients = HashMap::from([(1, client)]);
        let clients = ClientMap::new(1, &clients);
        assert!(do_warp(&clients, &mut state, None, -1, 0, 0, 0, 0).is_err());
        assert!(state.ongoing_races.contains_key(&1));
        do_warp(&clients, &mut state, None, warp_id, 0, 0, 0, 0).unwrap();
        assert!(!state.ongoing_races.contains_key(&1));
        assert_eq!(state.get_player(1).unwrap().get_position(), destination);
    }
}
