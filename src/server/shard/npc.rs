use std::time::{Duration, Instant};

use rand::seq::IteratorRandom;

use crate::{
    entity::{EntityID, Entity},
    error::*,
    net::{
        packet::{PacketID::*, *},
        ClientMap, FFClient, PACKET_BODY_SIZE,
    },
    state::ShardServerState,
    tabledata::tdata_get,
};

pub fn npc_interaction(
    pkt: Packet,
    client: &FFClient,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_NPC_INTERACTION = pkt.get()?;
    let pc_id = client.get_player_id()?;
    let npc_id = pkt.iNPC_ID;

    let npc = state.get_npc_mut(npc_id)?;
    if pkt.bFlag == 0 {
        if !npc.interacting_pcs.remove(&pc_id) {
            log(
                Severity::Warning,
                &format!(
                    "Player {} tried to stop interacting with NPC {} when not interacting",
                    pc_id, npc_id
                ),
            );
        }
    } else if !npc.interacting_pcs.insert(pc_id) {
        log(
            Severity::Warning,
            &format!(
                "Player {} tried to start interacting with NPC {} when already interacting",
                pc_id, npc_id
            ),
        );
    }

    Ok(())
}

pub fn npc_bark(pkt: Packet, client: &FFClient, state: &mut ShardServerState) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_BARKER = pkt.get()?;
    let task_id = pkt.iMissionTaskID;
    let pc_id = client.get_player_id()?;

    let task_def = tdata_get().get_task_definition(task_id)?;
    let barks = &task_def.barks;
    if barks.is_empty() {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "Player requested barker for task with no barks ({})",
                task_id
            ),
        ));
    }

    // we ignore pkt.iNPC_ID because the client always picks the first NPC in its container.
    // instead, we pick at random from the nearby NPCS that have a compatible barker type
    // for this mission. thanks @FinnHornhoover for finding this workaround (OpenFusion #266)
    let nearby_npc_ids = state.entity_map.get_around_entity(EntityID::Player(pc_id));
    let mut compatible_barks = Vec::with_capacity(nearby_npc_ids.len());
    for npc_id in nearby_npc_ids {
        if let EntityID::NPC(npc_id) = npc_id {
            let npc = state.get_npc(npc_id).unwrap();
            let npc_stats = tdata_get().get_npc_stats(npc.ty).unwrap();
            if npc_stats.bark_type.is_some_and(|bt| bt <= barks.len()) {
                compatible_barks.push((npc_id, npc_stats.bark_type.unwrap() - 1));
            }
        }
    }

    let chosen_bark = compatible_barks.iter().choose(&mut rand::thread_rng());
    if let Some(&(npc_id, bark_idx)) = chosen_bark {
        let bark_id = barks[bark_idx];
        let pkt = sP_FE2CL_REP_BARKER {
            iNPC_ID: npc_id,
            iMissionStringID: bark_id,
        };
        client.send_packet(P_FE2CL_REP_BARKER, &pkt);
    }

    Ok(())
}

/// How often a single client may ask for the NPC types around it.
const PRESENT_NPC_TYPES_INTERVAL: Duration = Duration::from_secs(1);

/// One trailing entry of [`sP_FE2CL_REP_PRESENT_NPC_TYPES`].
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[repr(packed(4))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct sNPCTypeEntry {
    pub iNPCType: i32,
}
impl FFPacket for sNPCTypeEntry {}

/// Answers the client's request for every NPC type currently present in its
/// instance, so it can prefetch their assets.
///
/// We don't keep a per-client history of what's already been sent, so every
/// answer is a full snapshot, with the clear flag set on the first packet.
pub fn present_npc_types(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let _pkt: &sP_CL2FE_REQ_PRESENT_NPC_TYPES = pkt.get()?;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    let now = Instant::now();
    let player = state.get_player_mut(pc_id)?;
    if player
        .last_npc_type_sync
        .is_some_and(|last| now.duration_since(last) < PRESENT_NPC_TYPES_INTERVAL)
    {
        return Ok(());
    }
    player.last_npc_type_sync = Some(now);
    let instance_id = player.instance_id;

    let npc_ids = state
        .entity_map
        .find_npcs(|npc| npc.instance_id == instance_id);
    let mut npc_types: Vec<i32> = npc_ids
        .into_iter()
        .filter_map(|npc_id| state.get_npc(npc_id).ok().map(|npc| npc.ty))
        .collect();
    npc_types.sort_unstable();
    npc_types.dedup();

    // how many type IDs fit alongside the header in one packet
    // FFOne's 4096-byte limit for this packet includes both the length and ID.
    let max_per_packet = (PACKET_BODY_SIZE - size_of::<u32>()
        - size_of::<sP_FE2CL_REP_PRESENT_NPC_TYPES>())
        / size_of::<sNPCTypeEntry>();

    let mut clear = true;
    let mut offset = 0;
    loop {
        let count = max_per_packet.min(npc_types.len() - offset);
        let mut builder = PacketBuilder::new(P_FE2CL_REP_PRESENT_NPC_TYPES).with(
            &sP_FE2CL_REP_PRESENT_NPC_TYPES {
                bClear: clear as i32,
                iCnt: count as i32,
            },
        );
        for npc_type in &npc_types[offset..offset + count] {
            builder.push(&sNPCTypeEntry {
                iNPCType: *npc_type,
            });
        }
        client.send_payload(builder.build()?);

        offset += count;
        clear = false;
        if offset >= npc_types.len() {
            break;
        }
    }

    // The type list cannot express moved/added JSON placements. Publish a complete,
    // instance-scoped friendly-NPC snapshot using bounded ordered chunks.
    let mut entries: Vec<_> = state.entity_map.find_npcs(|npc| npc.instance_id == instance_id
        && npc.team != crate::enums::CombatantTeam::Mob).into_iter()
        .filter_map(|id| state.get_npc(id).ok().map(|npc| {
            let pos=npc.get_position();
            NpcMapSnapshotEntry { id:npc.id, npc_type:npc.ty, x:pos.x, y:pos.y, z:pos.z }
        })).collect();
    entries.sort_by_key(|entry|entry.id);
    let chunks=entries.len().div_ceil(203).max(1);
    for index in 0..chunks {
        let start=(index*203).min(entries.len());let end=((index+1)*203).min(entries.len());
        let header=NpcMapSnapshotHeader { flags: i32::from(index==0) | (i32::from(index+1==chunks)<<1), count:(end-start) as i32 };
        let mut builder=PacketBuilder::new(P_FE2CL_NPC_MAP_SNAPSHOT).with(&header);
        for entry in &entries[start..end] {builder.push(entry);}
        client.send_payload(builder.build()?);
    }

    Ok(())
}
