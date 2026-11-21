//! Mission-owned delivery escorts, independent of social group membership and
//! the NPC table's idle AI. Defence missions continue to use authored routes.
use std::{collections::VecDeque, time::{Duration, Instant}};

use crate::{
    ai::make_for_npc,
    chunk::TickMode,
    defines::SHARD_TICKS_PER_SECOND,
    entity::{Combatant, Entity, EntityID, NPC, Player},
    enums::CombatantTeam,
    path::Path,
    skills::{do_basic_attack, AttackContext},
    tabledata::tdata_get,
    state::ShardServerState,
    Position,
};

#[derive(Debug, Clone)]
pub struct Escort {
    pub player_id: i32,
    pub task_id: i32,
    origin: Position,
    previous_path: Option<Path>,
    previous_ai: Option<String>,
    trail: VecDeque<Position>,
    last_player: Position,
    pub(crate) fights: bool,
    next_attack: Instant,
}

pub fn start(state: &mut ShardServerState, npc_id: i32, player_id: i32, task_id: i32) {
    let player_pos = state.get_player(player_id).unwrap().get_position();
    let npc = state.get_npc_mut(npc_id).unwrap();
    npc.mission_escort = Some(Escort {
        player_id,
        task_id,
        origin: npc.get_position(),
        previous_path: npc.path.take(),
        previous_ai: npc.ai.take(),
        trail: VecDeque::from([player_pos]),
        last_player: player_pos,
        fights: tdata_get().get_task_definition(task_id).is_ok_and(|task| task.escort_fights),
        next_attack: Instant::now(),
    });
    state
        .entity_map
        .set_tick(EntityID::NPC(npc_id), TickMode::Always)
        .unwrap();
}

pub fn release(state: &mut ShardServerState, npc_id: i32, restore_origin: bool) {
    let npc = state.get_npc_mut(npc_id).unwrap();
    let Some(escort) = npc.mission_escort.take() else {
        return;
    };
    npc.path = escort.previous_path;
    npc.ai = escort.previous_ai;
    if restore_origin {
        npc.set_position(escort.origin);
    }
    let chunk = npc.get_chunk_coords();
    let (_, tick_mode) = make_for_npc(npc, false);
    state
        .entity_map
        .set_tick(EntityID::NPC(npc_id), tick_mode)
        .unwrap();
    // Publish restoration through normal chunk enter/exit and movement packets.
    state
        .entity_map
        .update(EntityID::NPC(npc_id), Some(chunk), true);
    if restore_origin {
        let mut path = Path::new_single(escort.origin, 800);
        path.start();
        NPC::tick_movement_along_path(npc_id, &mut path, state);
    }
}

/// Returns true while this tick belongs to an escort (also on release).
pub fn tick(state: &mut ShardServerState, npc_id: i32) -> bool {
    let npc = state.get_npc(npc_id).unwrap();
    let Some(escort) = npc.mission_escort.as_ref() else {
        return false;
    };
    let player = state.get_player(escort.player_id).ok();
    let active = player.is_some_and(|player| {
        player.instance_id == npc.instance_id
            && player
                .mission_journal
                .get_current_tasks()
                .iter()
                .any(|task| task.get_task_id() == escort.task_id && !task.failed && !task.completed)
    });
    if !active || npc.is_dead() {
        release(state, npc_id, true);
        return true;
    }
    let player = player.unwrap();
    let player_pos = player.get_position();
    if let Some((target_id, delay)) = escort_target(state, npc, player, escort.fights) {
        let now = Instant::now();
        if now >= escort.next_attack {
            state.get_npc_mut(npc_id).unwrap().mission_escort.as_mut().unwrap().next_attack = now + delay;
            let _ = do_basic_attack(EntityID::NPC(npc_id), &[target_id], false, AttackContext::default(), state);
        }
        return true;
    }
    let npc = state.get_npc_mut(npc_id).unwrap();
    let pos = npc.get_position();
    let escort = npc.mission_escort.as_mut().unwrap();
    let distance = escort.last_player.distance_to(&player_pos);
    // Match the reference's abandonment guard; never chase a teleport across
    // the map, or accumulate an unbounded trail behind a stuck NPC.
    if distance > 5000 || escort.trail.len() >= 512 {
        release(state, npc_id, true);
        return true;
    }
    if distance >= 75 {
        escort.trail.push_back(player_pos);
        escort.last_player = player_pos;
    }
    while escort
        .trail
        .front()
        .is_some_and(|point| pos.distance_to(point) < 2)
    {
        escort.trail.pop_front();
    }
    let Some(target) = escort.trail.front().copied() else {
        return true;
    };
    let distance = pos.distance_to(&target);
    let travel = if escort.trail.len() == 1 {
        distance.saturating_sub(200)
    } else {
        distance
    }
    .min(800 / SHARD_TICKS_PER_SECOND as u32);
    if travel > 0 {
        let (next, _) = pos.interpolate(&target, travel as f32);
        let mut path = Path::new_single(next, (travel as usize * SHARD_TICKS_PER_SECOND) as i32);
        path.start();
        NPC::tick_movement_along_path(npc_id, &mut path, state);
    }
    true
}

fn escort_target(state: &ShardServerState, npc: &NPC, leader: &Player, fights: bool) -> Option<(EntityID, Duration)> {
    if !fights || npc.get_position().distance_to(&leader.get_position()) > 2500 { return None; }
    let EntityID::NPC(id) = leader.get_target().or(npc.last_attacked_by)? else { return None; };
    let target = state.get_npc(id).ok()?;
    let stats = tdata_get().get_npc_stats(npc.ty).ok()?;
    (target.instance_id == npc.instance_id && !target.hidden && !target.is_dead() &&
        target.get_team() == CombatantTeam::Mob && npc.get_position().distance_to(&target.get_position()) <= stats.attack_range + stats.radius)
        .then(|| (EntityID::NPC(id), Duration::from_millis(100 * stats.delay_time.max(1))))
}
