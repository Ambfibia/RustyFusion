use crate::{
    entity::{Entity, EntityID, Group},
    error::*,
    helpers,
    net::{
        packet::{PacketID::*, *},
        ClientMap,
    },
    state::ShardServerState,
};
use uuid::Uuid;

#[cfg(test)]
mod invite_contract_tests {
    use super::*;
    use crate::{
        chunk::{ChunkCoords, InstanceID, TickMode},
        entity::Player,
        net::{ClientMetadata, ClientType, FFClient},
        tabledata::tdata_init,
    };
    use std::collections::HashMap;

    #[test]
    fn rejected_targets_do_not_reserve_an_invitation_and_refusal_allows_retry() {
        tdata_init().unwrap();
        let mut state = ShardServerState::default();
        let mut connections = HashMap::new();
        let mut receivers = Vec::new();
        for id in [1, 2] {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            receivers.push(rx);
            let client = FFClient::new(tx, ClientMetadata::new(
                format!("127.0.0.1:{}", 39200 + id).parse().unwrap(),
                Some(ClientType::GameClient { account_id: id as i64, serial_key: id as i64, pc_id: Some(id) }),
            ));
            let mut player = Player::new(id as i64, 0);
            player.set_player_id(id); player.set_client(client.clone());
            let chunk = ChunkCoords::from_pos_inst(player.get_position(), InstanceID::default());
            let entity = state.entity_map.track(Box::new(player), TickMode::Always);
            state.entity_map.update(entity, Some(chunk), false);
            connections.insert(id as usize, client);
        }
        for target in [i32::MAX, 1] {
            let packet = Packet::new(P_CL2FE_REQ_PC_GROUP_INVITE, &sP_CL2FE_REQ_PC_GROUP_INVITE { iID_To: target }).unwrap();
            assert!(pc_group_invite(packet, &ClientMap::new(1, &connections), &mut state).is_err());
            assert_eq!(state.get_player(1).unwrap().group_offered_to, None);
        }
        for _ in 0..2 {
            let packet = Packet::new(P_CL2FE_REQ_PC_GROUP_INVITE, &sP_CL2FE_REQ_PC_GROUP_INVITE { iID_To: 2 }).unwrap();
            pc_group_invite(packet, &ClientMap::new(1, &connections), &mut state).unwrap();
            assert_eq!(state.get_player(1).unwrap().group_offered_to, Some(2));
            let packet = Packet::new(P_CL2FE_REQ_PC_GROUP_INVITE_REFUSE, &sP_CL2FE_REQ_PC_GROUP_INVITE_REFUSE { iID_From: 1 }).unwrap();
            pc_group_invite_refuse(packet, &ClientMap::new(2, &connections), &mut state).unwrap();
            assert_eq!(state.get_player(1).unwrap().group_offered_to, None);
        }
    }
}

pub fn pc_group_invite(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_GROUP_INVITE = pkt.get()?;
    (|| {
        let client = clients.get_sender();
        let pc_id = client.get_player_id()?;
        let player = state.get_player_mut(pc_id)?;
        if player.group_offered_to.is_some() {
            return Err(FFError::build(
                Severity::Debug,
                format!("{} is already offering a group invite", player),
            ));
        }

        let target_pc_id = pkt.iID_To;
        if target_pc_id == pc_id {
            return Err(FFError::build(Severity::Warning, "Cannot invite self".to_string()));
        }
        let target_player = state.get_player(target_pc_id)?;
        if target_player.group_id.is_some() {
            return Err(FFError::build(
                Severity::Debug,
                format!("{} is already in a group", target_player),
            ));
        }

        let target_client = target_player.get_client().unwrap();
        let pkt = sP_FE2CL_PC_GROUP_INVITE { iHostID: pc_id };
        target_client.send_packet(P_FE2CL_PC_GROUP_INVITE, &pkt);
        state.get_player_mut(pc_id)?.group_offered_to = Some(target_pc_id);
        Ok(())
    })()
    .catch_fail(|| {
        let pkt = sP_FE2CL_PC_GROUP_INVITE_FAIL {
            iErrorCode: unused!(),
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_PC_GROUP_INVITE_FAIL, &pkt);
    })
}

pub fn pc_group_invite_refuse(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_PC_GROUP_INVITE_REFUSE = pkt.get()?;
    let pc_id = client.get_player_id()?;

    let host_pc_id = pkt.iID_From;
    let host_player = state.get_player_mut(host_pc_id)?;
    if host_player.group_offered_to != Some(pc_id) {
        return Err(FFError::build(
            Severity::Debug,
            format!("Group offer from {} expired", host_player),
        ));
    }

    let host_client = host_player.get_client().unwrap();
    let pkt = sP_FE2CL_PC_GROUP_INVITE_REFUSE { iID_To: pc_id };
    host_client.send_packet(P_FE2CL_PC_GROUP_INVITE_REFUSE, &pkt);
    host_player.group_offered_to = None;
    Ok(())
}

pub fn pc_group_join(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_GROUP_JOIN = pkt.get()?;
    (|| {
        let client = clients.get_sender();
        let pc_id = client.get_player_id()?;
        let player = state.get_player(pc_id)?;
        if player.group_id.is_some() {
            return Err(FFError::build(
                Severity::Debug,
                format!("{} tried to join a group while already in one", player),
            ));
        }

        let host_pc_id = pkt.iID_From;
        let host_player = state.get_player_mut(host_pc_id)?;
        if host_player.group_offered_to != Some(pc_id) {
            return Err(FFError::build(
                Severity::Debug,
                format!("Group offer from {} expired", host_player),
            ));
        }

        host_player.group_offered_to = None;

        let group_id = host_player.group_id.unwrap_or(Uuid::new_v4());
        let mut group = if host_player.group_id.is_some() {
            state.groups.get(&group_id).unwrap().clone()
        } else {
            log(Severity::Debug, &format!("Creating group {}", group_id));
            Group::new(EntityID::Player(host_pc_id))
        };
        group.add_member(EntityID::Player(pc_id))?;

        let (pc_group_data, npc_group_data) = group.get_member_data(state);
        let mut pkt =
            PacketBuilder::new(P_FE2CL_PC_GROUP_JOIN_SUCC).with(&sP_FE2CL_PC_GROUP_JOIN_SUCC {
                iID_NewMember: pc_id,
                iMemberPCCnt: pc_group_data.len() as i32,
                iMemberNPCCnt: npc_group_data.len() as i32,
            });

        for pc_data in &pc_group_data {
            pkt.push(pc_data);
        }
        for npc_data in &npc_group_data {
            pkt.push(npc_data);
        }

        if let Some(pkt) = log_if_failed(pkt.build()) {
            for eid in group.get_member_ids() {
                let entity = state.entity_map.get_entity_raw(*eid).unwrap();
                if let Some(client) = entity.get_client() {
                    client.send_payload(pkt.clone());
                }
            }
        }

        state.groups.insert(group_id, group);
        state.get_player_mut(host_pc_id).unwrap().group_id = Some(group_id);
        state.get_player_mut(pc_id).unwrap().group_id = Some(group_id);
        Ok(())
    })()
    .catch_fail(|| {
        let pkt = sP_FE2CL_PC_GROUP_JOIN_FAIL {
            iID: pkt.iID_From,
            iErrorCode: unused!(),
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_PC_GROUP_JOIN_FAIL, &pkt);
    })
}

pub fn pc_group_leave(clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    (|| {
        let client = clients.get_sender();
        let leaver_pc_id = client.get_player_id()?;
        let player = state.get_player_mut(leaver_pc_id)?;
        let group_id = player.group_id.take().ok_or_else(|| {
            FFError::build(
                Severity::Warning,
                format!("{} tried to leave a group while not in one", player),
            )
        })?;

        helpers::remove_group_member(EntityID::Player(leaver_pc_id), group_id, state)?;

        // leaver needs the leave success packet too, thx client
        let resp = sP_FE2CL_PC_GROUP_LEAVE_SUCC::default();
        clients
            .get_sender()
            .send_packet(P_FE2CL_PC_GROUP_LEAVE_SUCC, &resp);

        Ok(())
    })()
    .catch_fail(|| {
        let pkt = sP_FE2CL_PC_GROUP_LEAVE_FAIL {
            iID: unused!(),
            iErrorCode: unused!(),
        };

        clients
            .get_sender()
            .send_packet(P_FE2CL_PC_GROUP_LEAVE_FAIL, &pkt);
    })
}

pub fn npc_group_invite(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_NPC_GROUP_INVITE = pkt.get()?;
    (|| {
    state.entity_map.validate_proximity(&[EntityID::Player(pc_id), EntityID::NPC(pkt.iNPC_ID)], crate::defines::RANGE_INTERACT)?;
    let target = state.get_npc(pkt.iNPC_ID)?;
    let stats = crate::tabledata::tdata_get().get_npc_stats(target.ty)?;
    if target.mission_escort.as_ref().is_some_and(|escort| escort.player_id != pc_id) {
        return Err(FFError::build(Severity::Warning, "NPC escort belongs to another player".into()));
    }
    if stats.team != crate::enums::CombatantTeam::Friendly || !(stats.service_category == 6 || (stats.ai_type > 0 && stats.ai_type < 11)) || target.hidden || !target.interactable || target.private_owner.is_some() {
        return Err(FFError::build(Severity::Warning, "NPC cannot join a social group".into()));
    }
    let player = state.get_player_mut(pc_id)?;

    let group_id = player.group_id.unwrap_or(Uuid::new_v4());
    let mut group = if player.group_id.is_some() {
        state.groups.get(&group_id).unwrap().clone()
    } else {
        log(Severity::Debug, &format!("Creating group {}", group_id));
        Group::new(EntityID::Player(pc_id))
    };

    let target_npc_id = pkt.iNPC_ID;
    let target_npc = state.get_npc_mut(target_npc_id)?;
    if target_npc.group_id.is_some() {
        return Err(FFError::build(
            Severity::Warning,
            format!("NPC {} is already in a group", target_npc_id),
        ));
    }
    group.add_member(EntityID::NPC(target_npc_id))?;

    let (pc_group_data, npc_group_data) = group.get_member_data(state);
    let mut pkt = PacketBuilder::new(P_FE2CL_REP_NPC_GROUP_INVITE_SUCC).with(
        &sP_FE2CL_REP_NPC_GROUP_INVITE_SUCC {
            iPC_ID: unused!(),
            iNPC_ID: target_npc_id,
            iMemberPCCnt: pc_group_data.len() as i32,
            iMemberNPCCnt: npc_group_data.len() as i32,
        },
    );

    for pc_data in &pc_group_data {
        pkt.push(pc_data);
    }
    for npc_data in &npc_group_data {
        pkt.push(npc_data);
    }

    if let Some(pkt) = log_if_failed(pkt.build()) {
        for eid in group.get_member_ids() {
            let entity = state.entity_map.get_entity_raw(*eid).unwrap();
            if let Some(client) = entity.get_client() {
                client.send_payload(pkt.clone());
            }
        }
    }

    state.groups.insert(group_id, group);
    state.get_player_mut(pc_id).unwrap().group_id = Some(group_id);
    state.get_npc_mut(target_npc_id).unwrap().group_id = Some(group_id);
    Ok(())
    })().catch_fail(|| client.send_packet(P_FE2CL_REP_NPC_GROUP_INVITE_FAIL, &sP_FE2CL_REP_NPC_GROUP_INVITE_FAIL { iErrorCode: 1 }))
}

pub fn npc_group_kick(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_NPC_GROUP_KICK = pkt.get()?;
    (|| {
    let player = state.get_player(pc_id)?;

    let group_id = player.group_id.ok_or_else(|| {
        FFError::build(
            Severity::Warning,
            format!("{} tried to kick an NPC while not in a group", player),
        )
    })?;

    let target_npc_id = pkt.iNPC_ID;
    let target_npc = state.get_npc_mut(target_npc_id)?;
    if target_npc.group_id != Some(group_id) {
        return Err(FFError::build(
            Severity::Warning,
            format!("NPC {} is not in the group", target_npc_id),
        ));
    }

    target_npc.group_id = None;
    helpers::remove_group_member(target_npc.get_id(), group_id, state)
    })().catch_fail(|| client.send_packet(P_FE2CL_REP_NPC_GROUP_KICK_FAIL, &sP_FE2CL_REP_NPC_GROUP_KICK_FAIL { iErrorCode: 1 }))
}
