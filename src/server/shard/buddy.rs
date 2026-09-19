use crate::{
    chunk::InstanceID,
    database::{db_get, DbImpl as _},
    defines::*,
    entity::{BuddyListEntry, Entity, EntityID, Player, PlayerSearchQuery},
    error::{codes::BuddyWarpErr, *},
    net::{
        packet::{PacketID::*, *},
        ClientMap,
    },
    state::ShardServerState,
    util,
};

use std::time::SystemTime;

const ERROR_CODE_BUDDY_DENY: i32 = 6;

pub fn get_buddy_state(clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;

    let mut req = sP_FE2LS_REQ_GET_BUDDY_STATE {
        iPC_UID: player.get_uid(),
        aBuddyUID: [0; 50],
    };

    let buddy_info = player.get_all_buddy_info();
    for (i, buddy_uid) in buddy_info.iter().map(|info| info.pc_uid).enumerate() {
        req.aBuddyUID[i] = buddy_uid;
    }

    if let Some(login_server) = clients.get_login_server() {
        login_server.send_packet(P_FE2LS_REQ_GET_BUDDY_STATE, &req);
        Ok(())
    } else {
        Err(FFError::build(
            Severity::Warning,
            "No login server connected".to_string(),
        ))
    }
}

pub fn request_make_buddy(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_REQUEST_MAKE_BUDDY = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let buddy_id = pkt.iBuddyID;
    let buddy_uid = pkt.iBuddyPCUID;

    state.entity_map.validate_proximity(
        &[EntityID::Player(pc_id), EntityID::Player(buddy_id)],
        RANGE_INTERACT,
    )?;

    let player = state.get_player(pc_id)?;
    if player.is_buddies_with(buddy_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} is already buddies with player {}", player, buddy_uid),
        ));
    }

    if player.get_num_buddies() >= SIZEOF_BUDDYLIST_SLOT as usize {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} has too many buddies", player),
        ));
    }

    let player_uid = player.get_uid();
    let player_has_blocked_buddy = player.has_blocked(buddy_uid);

    let req_pkt = sP_FE2CL_REP_REQUEST_MAKE_BUDDY_SUCC_TO_ACCEPTER {
        iRequestID: pc_id,
        iBuddyID: buddy_id,
        szFirstName: util::encode_utf16(&player.first_name).unwrap(),
        szLastName: util::encode_utf16(&player.last_name).unwrap(),
    };

    let buddy = state.get_player(buddy_id)?;
    if buddy.get_uid() != buddy_uid {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "Buddy UID mismatch (client: {}, server: {})",
                buddy_uid,
                buddy.get_uid()
            ),
        ));
    }

    if buddy.get_num_buddies() >= SIZEOF_BUDDYLIST_SLOT as usize
        || buddy.has_blocked(player_uid)
        || player_has_blocked_buddy
    {
        // instant deny
        let deny_pkt = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL {
            iBuddyID: buddy_id,
            iBuddyPCUID: buddy_uid,
            iErrorCode: ERROR_CODE_BUDDY_DENY,
        };

        client.send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL, &deny_pkt);
        return Ok(());
    }

    let buddy_client = buddy.get_client().unwrap();
    buddy_client.send_packet(P_FE2CL_REP_REQUEST_MAKE_BUDDY_SUCC_TO_ACCEPTER, &req_pkt);

    let player = state.get_player_mut(pc_id).unwrap();
    player.buddy_offered_to = Some(buddy_uid);

    Ok(())
}

pub fn find_name_make_buddy(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_PC_FIND_NAME_MAKE_BUDDY = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    let pc_uid = player.get_uid();
    if player.get_num_buddies() >= SIZEOF_BUDDYLIST_SLOT as usize {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} has too many buddies", player),
        ));
    }

    let first_name = util::parse_utf16(&pkt.szFirstName)?;
    let last_name = util::parse_utf16(&pkt.szLastName)?;

    let search = PlayerSearchQuery::ByName(first_name, last_name);
    let res = search.execute(state);
    if res.is_none() {
        // TODO cross-shard
        return Ok(());
    }
    let buddy_id = res.unwrap();

    let buddy = state.get_player(buddy_id).unwrap();
    let buddy_uid = buddy.get_uid();
    if buddy.is_buddies_with(pc_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} is already buddies with player {}", player, buddy_id),
        ));
    }

    if buddy.get_num_buddies() >= SIZEOF_BUDDYLIST_SLOT as usize
        || buddy.has_blocked(pc_uid)
        || state
            .get_player(pc_id)
            .is_ok_and(|player| player.has_blocked(buddy_uid))
    {
        // instant deny
        let deny_pkt = sP_FE2CL_REP_PC_FIND_NAME_MAKE_BUDDY_FAIL {
            iErrorCode: ERROR_CODE_BUDDY_DENY,
            szFirstName: pkt.szFirstName,
            szLastName: pkt.szLastName,
        };

        let client = clients.get_sender();
        client.send_packet(P_FE2CL_REP_PC_FIND_NAME_MAKE_BUDDY_FAIL, &deny_pkt);
        return Ok(());
    }

    let buddy_client = buddy.get_client().unwrap();
    let player = state.get_player_mut(pc_id).unwrap();
    player.buddy_offered_to = Some(buddy_uid);
    let req_pkt = sP_FE2CL_REP_PC_FIND_NAME_MAKE_BUDDY_SUCC {
        szFirstName: util::encode_utf16(&player.first_name).unwrap(),
        szLastName: util::encode_utf16(&player.last_name).unwrap(),
        iPCUID: pc_uid,
        iNameCheckFlag: player.flags.name_check as i8,
    };
    buddy_client.send_packet(P_FE2CL_REP_PC_FIND_NAME_MAKE_BUDDY_SUCC, &req_pkt);
    Ok(())
}

pub fn accept_make_buddy(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_ACCEPT_MAKE_BUDDY = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    let player_buddy_info = BuddyListEntry::new(player);
    let pc_uid = state.get_player(pc_id)?.get_uid();
    let buddy_id = pkt.iBuddyID;
    let accepted = pkt.iAcceptFlag == 1;

    let buddy = state.get_player_mut(buddy_id)?;
    let buddy_uid = buddy.get_uid();
    if buddy.buddy_offered_to != Some(pc_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} did not send buddy request to player {}", buddy, pc_id),
        ));
    }

    buddy.buddy_offered_to = None;

    (|| {
        let buddy = state.get_player_mut(buddy_id).unwrap(); // re-borrow
        if !accepted {
            // this failure will be caught and the deny packet will be sent
            return Err(FFError::build(
                Severity::Debug,
                format!("{} denied buddy request from player {}", buddy, pc_id),
            ));
        }

        // player -> buddy
        let pkt_buddy = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC {
            iBuddySlot: buddy.add_buddy(player_buddy_info.clone())? as i8,
            BuddyInfo: player_buddy_info.into(),
        };

        buddy
            .get_client()
            .unwrap()
            .send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC, &pkt_buddy);

        // buddy -> player
        let buddy_buddy_info = BuddyListEntry::new(buddy);
        let player = state.get_player_mut(pc_id).unwrap();
        let pkt_player = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC {
            iBuddySlot: player.add_buddy(buddy_buddy_info.clone())? as i8,
            BuddyInfo: buddy_buddy_info.into(),
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC, &pkt_player);

        Ok(())
    })()
    .catch_fail(|| {
        let player = state.get_player_mut(pc_id).unwrap();
        let _ = player.remove_buddy(buddy_uid);

        let buddy = state.get_player_mut(buddy_id).unwrap();
        let _ = buddy.remove_buddy(pc_uid);

        // we send the deny packet to the buddy in case of failure
        let deny_pkt = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL {
            iBuddyID: pc_id,
            iBuddyPCUID: pc_uid,
            iErrorCode: ERROR_CODE_BUDDY_DENY,
        };

        let buddy_client = buddy.get_client().unwrap();
        buddy_client.send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL, &deny_pkt);
    })
}

pub fn find_name_accept_buddy(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_PC_FIND_NAME_ACCEPT_BUDDY = pkt.get()?;

    let accepted = pkt.iAcceptFlag == 1;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    let player_buddy_info = BuddyListEntry::new(player);
    let pc_uid = player.get_uid();
    if player.get_num_buddies() >= SIZEOF_BUDDYLIST_SLOT as usize {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} has too many buddies", player),
        ));
    }

    let buddy_uid = pkt.iBuddyPCUID;
    let search = PlayerSearchQuery::ByUID(buddy_uid);
    let res = search.execute(state);
    if res.is_none() {
        // TODO cross-shard
        return Ok(());
    }
    let buddy_id = res.unwrap();

    let buddy = state.get_player_mut(buddy_id)?;
    if buddy.buddy_offered_to != Some(pc_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} did not send buddy request to player {}", buddy, pc_id),
        ));
    }
    buddy.buddy_offered_to = None;

    (|| {
        let buddy = state.get_player_mut(buddy_id).unwrap(); // re-borrow
        if !accepted {
            // this failure will be caught and the deny packet will be sent
            return Err(FFError::build(
                Severity::Debug,
                format!("{} denied buddy request from player {}", buddy, pc_id),
            ));
        }

        // player -> buddy
        let pkt_buddy = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC {
            iBuddySlot: buddy.add_buddy(player_buddy_info.clone())? as i8,
            BuddyInfo: player_buddy_info.into(),
        };

        buddy
            .get_client()
            .unwrap()
            .send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC, &pkt_buddy);

        // buddy -> player
        let buddy_buddy_info = BuddyListEntry::new(buddy);
        let player = state.get_player_mut(pc_id).unwrap();
        let pkt_player = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC {
            iBuddySlot: player.add_buddy(buddy_buddy_info.clone())? as i8,
            BuddyInfo: buddy_buddy_info.into(),
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_SUCC, &pkt_player);

        Ok(())
    })()
    .catch_fail(|| {
        let player = state.get_player_mut(pc_id).unwrap();
        let _ = player.remove_buddy(buddy_uid);

        let buddy = state.get_player_mut(buddy_id).unwrap();
        let _ = buddy.remove_buddy(pc_uid);

        // we send the deny packet to the buddy in case of failure
        let deny_pkt = sP_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL {
            iBuddyID: pc_id,
            iBuddyPCUID: pc_uid,
            iErrorCode: ERROR_CODE_BUDDY_DENY,
        };

        let buddy_client = buddy.get_client().unwrap();
        buddy_client.send_packet(P_FE2CL_REP_ACCEPT_MAKE_BUDDY_FAIL, &deny_pkt);
    })
}

pub fn pc_buddy_warp(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_PC_BUDDY_WARP = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    let player_uid = player.get_player_uid();
    let player_is_on_skyway = player.get_skyway_ride().is_some();
    let player_payzone_flag = player.get_payzone_flag();
    let player_is_warp_on_cooldown = player.is_warp_on_cooldown();
    let buddy_uid = pkt.iBuddyPCUID;

    let invalid_warp = |msg: String, error_code: i32| -> FFResult<()> {
        let response = sP_FE2CL_REP_PC_BUDDY_WARP_FAIL {
            iBuddyPCUID: buddy_uid,
            iErrorCode: error_code,
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_PC_BUDDY_WARP_FAIL, &response);

        Err(FFError::build(Severity::Info, msg))
    };

    if !player.is_buddies_with(buddy_uid) {
        return invalid_warp(
            format!(
                "Buddy {} is not buddies with player {}",
                player_uid, buddy_uid
            ),
            BuddyWarpErr::CantWarpToLocation as i32,
        );
    }

    if player_is_on_skyway {
        return invalid_warp(
            format!("Player {} is currently on a skyway ride", player_uid),
            BuddyWarpErr::CantWarpToLocation as i32,
        );
    }

    if player_is_warp_on_cooldown {
        return invalid_warp(
            format!("Player {}'s buddy warp is on cooldown", player_uid),
            BuddyWarpErr::RechargeNotComplete as i32,
        );
    }

    let search = PlayerSearchQuery::ByUID(buddy_uid);
    let res = search.execute(state);
    if res.is_none() {
        let login_server = match clients.get_login_server() {
            Some(ls) => ls,
            None => {
                return Err(FFError::build(
                    Severity::Warning,
                    "No login server connected for cross-shard buddy warp".to_string(),
                ));
            }
        };

        let req_pkt = sP_FE2LS_REQ_BUDDY_WARP {
            iPCPayzoneFlag: player_payzone_flag as i8,
            iFromPCUID: player_uid,
            iBuddyPCUID: buddy_uid,
        };

        login_server.send_packet(P_FE2LS_REQ_BUDDY_WARP, &req_pkt);
        return Ok(());
    }

    let buddy_id = res.unwrap();

    let buddy = state.get_player_mut(buddy_id)?;
    let buddy_is_on_skyway = buddy.get_skyway_ride().is_some();
    let buddy_payzone_flag = buddy.get_payzone_flag();
    let buddy_instance_id = buddy.get_instance_id();
    let buddy_position = buddy.get_position();

    if buddy_is_on_skyway {
        return invalid_warp(
            format!("Buddy {} is currently on a skyway ride", buddy_uid),
            BuddyWarpErr::CantWarpToLocation as i32,
        );
    }

    if player_payzone_flag != buddy_payzone_flag {
        return invalid_warp(
            format!("Buddy {} is in a different payzone state", buddy_uid,),
            BuddyWarpErr::CantWarpToLocation as i32,
        );
    }

    if buddy_instance_id.map_num != ID_OVERWORLD {
        return invalid_warp(
            format!("Buddy {} is not in the overworld instance", buddy_uid,),
            BuddyWarpErr::CantWarpToLocation as i32,
        );
    }

    {
        let player = state.get_player_mut(pc_id).unwrap();
        player.set_position(buddy_position);
        player.set_instance_id(InstanceID {
            map_num: buddy_instance_id.map_num,
            channel_num: buddy_instance_id.channel_num,
            instance_num: None,
        });
        player.buddy_warp_available_at =
            Some(util::get_timestamp_sec(SystemTime::now()) + BUDDYWARP_INTERVAL);

        state.entity_map.update(EntityID::Player(pc_id), None, true);

        // this packet in client code seems to just leave group
        let same_shard_succ_pkt = sP_FE2CL_REP_PC_BUDDY_WARP_SAME_SHARD_SUCC::default();

        // this packet in client code loads the new position
        let goto_succ_pkt = sP_FE2CL_REP_PC_GOTO_SUCC {
            iX: buddy_position.x,
            iY: buddy_position.y,
            iZ: buddy_position.z,
        };

        let client = clients.get_sender();

        client.send_packet(
            P_FE2CL_REP_PC_BUDDY_WARP_SAME_SHARD_SUCC,
            &same_shard_succ_pkt,
        );

        client.send_packet(P_FE2CL_REP_PC_GOTO_SUCC, &goto_succ_pkt);

        Ok(())
    }
    .catch_fail(|| {
        let response = sP_FE2CL_REP_PC_BUDDY_WARP_FAIL {
            iBuddyPCUID: buddy_uid,
            iErrorCode: BuddyWarpErr::CantWarpToLocation as i32,
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_PC_BUDDY_WARP_FAIL, &response);
    })
}

/// Blocks an existing buddy. The client keeps them in the list with the
/// blocked flag set; the friendship itself is dissolved on both sides.
pub async fn set_buddy_block(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_SET_BUDDY_BLOCK = pkt.get()?;
    let buddy_uid = pkt.iBuddyPCUID;
    let slot_num = pkt.iBuddySlot;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    (|| {
        let player = state.get_player_mut(pc_id)?;
        let pc_uid = player.get_uid();
        validate_buddy_slot(player, slot_num, buddy_uid)?;
        player.block_player(buddy_uid)?;

        let resp = sP_FE2CL_REP_SET_BUDDY_BLOCK_SUCC {
            iBuddyPCUID: buddy_uid,
            iBuddySlot: slot_num,
        };
        client.send_packet(P_FE2CL_REP_SET_BUDDY_BLOCK_SUCC, &resp);

        // the block is one-sided, but the friendship isn't: drop us from
        // their list too so they don't keep a dangling buddy entry
        remove_from_other_side(buddy_uid, pc_uid, state);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_SET_BUDDY_BLOCK_FAIL {
            iBuddyPCUID: buddy_uid,
            iErrorCode: ERROR_CODE_BUDDY_DENY,
        };
        client.send_packet(P_FE2CL_REP_SET_BUDDY_BLOCK_FAIL, &resp);
    })
}

/// Blocks someone who isn't on the buddy list yet. They get added to the list
/// in the blocked state, which is how the client models it.
pub async fn set_pc_block(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_SET_PC_BLOCK = pkt.get()?;
    let block_id = pkt.iBlock_ID;
    let block_uid = pkt.iBlock_PCUID;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    (async {
        let player = state.get_player(pc_id)?;
        if player.get_uid() == block_uid {
            return Err(FFError::build(
                Severity::Warning,
                format!("Player {} tried to block themselves", pc_id),
            ));
        }
        if player.get_num_buddies() >= SIZEOF_BUDDYLIST_SLOT as usize {
            return Err(FFError::build(
                Severity::Warning,
                format!("{} has no free buddy list slots to block with", player),
            ));
        }

        // resolve the entry without holding a shared borrow of the state
        // across the DB await
        let cached = state.get_player_by_uid(block_uid).map(BuddyListEntry::new);
        let entry = match cached {
            Some(entry) => entry,
            None => lookup_buddy_entry(block_uid).await?,
        };
        let player = state.get_player_mut(pc_id)?;
        let slot_num = player.add_blocked_player(entry)?;

        let resp = sP_FE2CL_REP_SET_PC_BLOCK_SUCC {
            iBlock_ID: block_id,
            iBlock_PCUID: block_uid,
            iBuddySlot: slot_num as i8,
        };
        client.send_packet(P_FE2CL_REP_SET_PC_BLOCK_SUCC, &resp);
        Ok(())
    })
    .await
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_SET_PC_BLOCK_FAIL {
            iBlock_ID: block_id,
            iBlock_PCUID: block_uid,
            iErrorCode: ERROR_CODE_BUDDY_DENY,
        };
        client.send_packet(P_FE2CL_REP_SET_PC_BLOCK_FAIL, &resp);
    })
}

/// Removes a buddy list entry. The client uses this for un-buddying *and*
/// for unblocking, so we handle both.
pub async fn remove_buddy(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_REMOVE_BUDDY = pkt.get()?;
    let buddy_uid = pkt.iBuddyPCUID;
    let slot_num = pkt.iBuddySlot;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    (|| {
        let player = state.get_player_mut(pc_id)?;
        let pc_uid = player.get_uid();
        validate_buddy_slot(player, slot_num, buddy_uid)?;
        let was_blocked = player.has_blocked(buddy_uid);
        player.remove_buddy(buddy_uid)?;

        let resp = sP_FE2CL_REP_REMOVE_BUDDY_SUCC {
            iBuddyPCUID: buddy_uid,
            iBuddySlot: slot_num,
        };
        client.send_packet(P_FE2CL_REP_REMOVE_BUDDY_SUCC, &resp);

        // unblocking doesn't touch the other player; un-buddying does
        if !was_blocked {
            remove_from_other_side(buddy_uid, pc_uid, state);
        }
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_REMOVE_BUDDY_FAIL {
            iBuddyPCUID: buddy_uid,
            iErrorCode: ERROR_CODE_BUDDY_DENY,
        };
        client.send_packet(P_FE2CL_REP_REMOVE_BUDDY_FAIL, &resp);
    })
}

/// Sends a buddy's appearance so the client can render their portrait.
pub fn get_buddy_style(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_GET_BUDDY_STYLE = pkt.get()?;
    let buddy_uid = pkt.iBuddyPCUID;
    let slot_num = pkt.iBuddySlot;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    (|| {
        let player = state.get_player(pc_id)?;
        let entry = player
            .get_buddy_at_slot(slot_num as usize)
            .filter(|entry| entry.pc_uid == buddy_uid)
            .ok_or_else(|| {
                FFError::build(
                    Severity::Warning,
                    format!("No buddy {} in slot {}", buddy_uid, slot_num),
                )
            })?;

        let style = sPCStyle {
            iPC_UID: entry.pc_uid,
            iNameCheck: entry.name_check as i8,
            szFirstName: util::encode_utf16(&entry.first_name)?,
            szLastName: util::encode_utf16(&entry.last_name)?,
            iGender: entry.style.gender,
            iFaceStyle: entry.style.face_style,
            iHairStyle: entry.style.hair_style,
            iHairColor: entry.style.hair_color,
            iSkinColor: entry.style.skin_color,
            iEyeColor: entry.style.eye_color,
            iHeight: entry.style.height,
            iBody: entry.style.body,
            iClass: unused!(),
        };

        // equipment is only known for buddies who are online on this shard;
        // the client tolerates an empty set for everyone else
        let mut equip: [sItemBase; SIZEOF_EQUIP_SLOT as usize] = Default::default();
        if let Some(buddy) = state.get_player_by_uid(buddy_uid) {
            for (i, item) in buddy.get_equipped().iter().enumerate() {
                equip[i] = (*item).into_proto();
            }
        }

        let resp = sP_FE2CL_REP_GET_BUDDY_STYLE_SUCC {
            iBuddyPCUID: buddy_uid,
            iBuddySlot: slot_num,
            sBuddyStyle: sBuddyStyleInfo {
                sBuddyStyle: style,
                aEquip: equip,
            },
        };
        client.send_packet(P_FE2CL_REP_GET_BUDDY_STYLE_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_GET_BUDDY_STYLE_FAIL {
            iBuddyPCUID: buddy_uid,
            iErrorCode: ERROR_CODE_BUDDY_DENY,
        };
        client.send_packet(P_FE2CL_REP_GET_BUDDY_STYLE_FAIL, &resp);
    })
}

fn validate_buddy_slot(player: &Player, slot_num: i8, buddy_uid: i64) -> FFResult<()> {
    if slot_num < 0 || slot_num as u32 >= SIZEOF_BUDDYLIST_SLOT {
        return Err(FFError::build(
            Severity::Warning,
            format!("Bad buddy slot {}", slot_num),
        ));
    }
    match player.get_buddy_at_slot(slot_num as usize) {
        Some(entry) if entry.pc_uid == buddy_uid => Ok(()),
        _ => Err(FFError::build(
            Severity::Warning,
            format!("Buddy {} is not in slot {}", buddy_uid, slot_num),
        )),
    }
}

/// Drops `pc_uid` from `other_uid`'s buddy list, if they're online here.
/// Their DB row follows on their next save.
fn remove_from_other_side(other_uid: i64, pc_uid: i64, state: &mut ShardServerState) {
    let Some(other_id) = PlayerSearchQuery::ByUID(other_uid).execute(state) else {
        return;
    };
    let Ok(other) = state.get_player_mut(other_id) else {
        return;
    };
    let Some(slot_num) = other.get_buddy_slot_num(pc_uid) else {
        return;
    };
    if other.has_blocked(pc_uid) {
        // their block on us outranks our un-buddying
        return;
    }
    if other.remove_buddy(pc_uid).is_err() {
        return;
    }
    if let Some(other_client) = other.get_client() {
        let resp = sP_FE2CL_REP_REMOVE_BUDDY_SUCC {
            iBuddyPCUID: pc_uid,
            iBuddySlot: slot_num as i8,
        };
        other_client.send_packet(P_FE2CL_REP_REMOVE_BUDDY_SUCC, &resp);
    }
}

/// Builds a buddy list entry for a player who isn't on this shard by loading
/// them from the database.
async fn lookup_buddy_entry(pc_uid: i64) -> FFResult<BuddyListEntry> {
    let db = db_get();
    let account = db
        .find_account_from_player(pc_uid)
        .await?
        .ok_or_else(|| FFError::build(Severity::Warning, format!("Player {} not found", pc_uid)))?;
    let player = db
        .load_player(account.id, pc_uid)
        .await?
        .ok_or_else(|| FFError::build(Severity::Warning, format!("Player {} not found", pc_uid)))?;
    Ok(BuddyListEntry::new(&player))
}
