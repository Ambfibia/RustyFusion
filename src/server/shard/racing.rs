use std::time::SystemTime;

use crate::{
    database::{db_get, DbImpl as _},
    defines::*,
    entity::Entity,
    enums::{ItemLocation, ItemType, RacingMode, RewardCategory, RewardType},
    error::*,
    item::Item,
    net::{
        packet::{PacketID::*, *},
        ClientMap,
    },
    racing::{RaceResult, RaceState},
    state::ShardServerState,
    tabledata::tdata_get,
    util,
};

const ERROR_CODE_RACE_GENERIC: i32 = 1;

/// Resolves the Infected Zone a race agent NPC belongs to.
/// Returns `(ep_id, map_num)`.
fn get_ep_for_npc(npc_id: i32, state: &ShardServerState) -> FFResult<(i32, u32)> {
    let npc = state.get_npc(npc_id)?;
    let map_num = npc.instance_id.map_num;
    let map_data = tdata_get().get_map_data(map_num)?;
    let ep_id = map_data.ep_id.ok_or_else(|| {
        FFError::build(
            Severity::Warning,
            format!("Map {} is not an Infected Zone", map_num),
        )
    })?;
    Ok((ep_id as i32, map_num))
}

pub fn race_start(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_EP_RACE_START = pkt.get()?;
    let start_npc_id = pkt.iStartEcomID;
    let mode = pkt.iEPRaceMode;
    let ticket_slot = pkt.iEPTicketItemSlotNum;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    (|| {
        let mode: RacingMode = mode.try_into()?;
        let (ep_id, map_num) = get_ep_for_npc(start_npc_id, state)?;
        let race_data = tdata_get().get_race_data(ep_id)?;

        let player = state.get_player(pc_id)?;
        if player.instance_id.map_num != map_num {
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "{} tried to start a race in map {} from map {}",
                    player, map_num, player.instance_id.map_num
                ),
            ));
        }

        // The ticket slot is advisory; the client keeps the ticket. We record
        // it so a future implementation can consume it without a protocol change.
        let ticket_slot = if (0..SIZEOF_INVEN_SLOT as i32).contains(&ticket_slot) {
            Some(ticket_slot as usize)
        } else {
            None
        };

        let time_limit = race_data.get_time_limit();
        state.ongoing_races.insert(
            pc_id,
            RaceState::new(ep_id, map_num, mode, start_npc_id, ticket_slot),
        );

        let resp = sP_FE2CL_REP_EP_RACE_START_SUCC {
            iStartTick: unused!(), // the client ignores this and starts its own clock
            iLimitTime: time_limit.as_secs() as i32,
        };
        client.send_packet(P_FE2CL_REP_EP_RACE_START_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_EP_RACE_START_FAIL {
            iErrorCode: ERROR_CODE_RACE_GENERIC,
        };
        client.send_packet(P_FE2CL_REP_EP_RACE_START_FAIL, &resp);
    })
}

pub fn race_get_ring(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_EP_GET_RING = pkt.get()?;
    let ring_id = pkt.iRingLID;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    (|| {
        let race = state.ongoing_races.get_mut(&pc_id).ok_or_else(|| {
            FFError::build(
                Severity::Warning,
                format!("Player {} isn't in a race", pc_id),
            )
        })?;
        let num_rings = race.collect_ring(ring_id)?;

        let resp = sP_FE2CL_REP_EP_GET_RING_SUCC {
            iRingLID: ring_id,
            iRingCount_Get: num_rings as i32,
        };
        client.send_packet(P_FE2CL_REP_EP_GET_RING_SUCC, &resp);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_EP_GET_RING_FAIL {
            iErrorCode: ERROR_CODE_RACE_GENERIC,
        };
        client.send_packet(P_FE2CL_REP_EP_GET_RING_FAIL, &resp);
    })
}

pub fn race_cancel(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let _pkt: &sP_CL2FE_REQ_EP_RACE_CANCEL = pkt.get()?;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    (|| {
        if state.ongoing_races.remove(&pc_id).is_none() {
            return Err(FFError::build(
                Severity::Warning,
                format!("Player {} isn't in a race", pc_id),
            ));
        }

        let resp = sP_FE2CL_REP_EP_RACE_CANCEL_SUCC { iTemp: unused!() };
        client.send_packet(P_FE2CL_REP_EP_RACE_CANCEL_SUCC, &resp);

        // This packet doubles as "I ran out of time". In that case the client
        // has already frozen the player waiting for the server to move them,
        // so we always relocate to avoid a softlock.
        let player = state.get_player_mut(pc_id)?;
        let pos = player.get_position();
        let map_num = player.instance_id.map_num;
        let respawn_pos = tdata_get()
            .get_nearest_respawn_point(pos, map_num)
            .unwrap_or(pos);
        player.set_position(respawn_pos);
        let entity_id = player.get_id();
        let chunk = player.get_chunk_coords();
        state.entity_map.update(entity_id, Some(chunk), true);

        let goto_pkt = sP_FE2CL_REP_PC_GOTO_SUCC {
            iX: respawn_pos.x,
            iY: respawn_pos.y,
            iZ: respawn_pos.z,
        };
        client.send_packet(P_FE2CL_REP_PC_GOTO_SUCC, &goto_pkt);
        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_EP_RACE_CANCEL_FAIL {
            iErrorCode: ERROR_CODE_RACE_GENERIC,
        };
        client.send_packet(P_FE2CL_REP_EP_RACE_CANCEL_FAIL, &resp);
    })
}

pub async fn race_end(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_EP_RACE_END = pkt.get()?;
    let end_npc_id = pkt.iEndEcomID;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    let result = (async {
        let (ep_id, _) = get_ep_for_npc(end_npc_id, state)?;
        let race = state.ongoing_races.get(&pc_id).ok_or_else(|| {
            FFError::build(
                Severity::Warning,
                format!("Player {} isn't in a race", pc_id),
            )
        })?;
        if race.ep_id != ep_id {
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "Player {} started a race in EP {} but finished in EP {}",
                    pc_id, race.ep_id, ep_id
                ),
            ));
        }

        let race_data = tdata_get().get_race_data(ep_id)?;
        let elapsed = race.get_elapsed();
        let num_rings = race.get_num_rings();
        let mode = race.mode;

        let cap_score = config_score_capped();
        let (score, base_fm) = race_data.score_run(num_rings, elapsed, cap_score);
        let rank = race_data.get_rank(score);

        let this_result = RaceResult {
            ep_id,
            pc_uid: state.get_player(pc_id)?.get_uid(),
            score,
            num_pods: num_rings as i32,
            time_s: elapsed.as_secs() as i32,
            timestamp: util::get_timestamp_sec(SystemTime::now()),
        };

        // Practice runs don't go on the leaderboard, matching the client's
        // own labeling of the two modes.
        let db = db_get();
        if mode == RacingMode::Record {
            db.save_race_result(&this_result).await?;
        }

        let best = db
            .load_best_race_result(ep_id, this_result.pc_uid)
            .await?
            .unwrap_or(this_result);
        let best_rank = race_data.get_rank(best.score);

        // fusion matter reward, scaled by the racing reward rate
        let player = state.get_player_mut(pc_id)?;
        let fm_rate = player
            .reward_data
            .get_reward_rate(RewardType::FusionMatter, RewardCategory::Racing as usize)
            .unwrap_or(1.0);
        let fm_reward = (base_fm as f32 * fm_rate) as u32;
        let fusion_matter = player.set_fusion_matter(player.get_fusion_matter() + fm_reward);

        // rank reward crate, if the player has room for it
        let mut reward_item = sItemReward::default();
        if let Some(crate_id) = race_data.get_reward_crate_id(rank) {
            match player.find_free_slot(ItemLocation::Inven) {
                Ok(slot_num) => {
                    let item = Item::new(ItemType::Chest, crate_id);
                    player.set_item(ItemLocation::Inven, slot_num, Some(item))?;
                    reward_item = sItemReward {
                        sItem: Some(item).into_proto(),
                        eIL: ItemLocation::Inven as i32,
                        iSlotNum: slot_num as i32,
                    };
                }
                Err(e) => log_error(e),
            }
        }

        state.ongoing_races.remove(&pc_id);

        let resp = sP_FE2CL_REP_EP_RACE_END_SUCC {
            iEPRaceMode: mode as i32,
            iEPRaceTime: this_result.time_s,
            iEPRingCnt: this_result.num_pods,
            iEPScore: this_result.score,
            iEPRank: rank as i32,
            iEPRewardFM: fm_reward as i32,
            iEPTopScore: best.score,
            iEPTopRank: best_rank as i32,
            iEPTopTime: best.time_s,
            iEPTopRingCount: best.num_pods,
            iFusionMatter: fusion_matter as i32,
            RewardItem: reward_item,
            iFatigue: 50,
            iFatigue_Level: 1,
        };
        client.send_packet(P_FE2CL_REP_EP_RACE_END_SUCC, &resp);
        Ok(())
    })
    .await;

    result.catch_fail(|| {
        state.ongoing_races.remove(&pc_id);
        let resp = sP_FE2CL_REP_EP_RACE_END_FAIL {
            iErrorCode: ERROR_CODE_RACE_GENERIC,
        };
        client.send_packet(P_FE2CL_REP_EP_RACE_END_FAIL, &resp);
    })
}

fn config_score_capped() -> bool {
    crate::config::config_get().shard.iz_race_score_capped.get()
}
