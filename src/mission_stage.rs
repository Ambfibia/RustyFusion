//! Server-owned authored conditions and private mission actors.
use crate::entity::Combatant;
use crate::{
    chunk::TickMode,
    entity::{Entity, EntityID, NPC},
    error::{FFError, FFResult, Severity},
    path::Path,
    state::ShardServerState,
    tabledata::tdata_get,
    Position,
};
use crate::tabledata_contracts::mission_stage::Conditions;
use crate::tabledata_contracts::mission_stage::{NpcRoute, NpcVisibility, RouteFinish};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
pub fn conditions_allowed(player: &crate::entity::Player, conditions: &Conditions) -> bool {
    conditions.allows(
        i32::from(player.get_level()),
        |id| {
            player
                .mission_journal
                .is_mission_completed(id)
                .unwrap_or(false)
        },
        |inventory| player.mission_requirements_allowed(inventory),
    )
}
pub fn resume_branches(client: &crate::net::FFClient, player: &crate::entity::Player) {
    use crate::net::packet::{sP_FE2CL_REP_PC_TASK_END_SUCC, NativeCompletionBranch, PacketID::*};
    for task in player
        .mission_journal
        .get_current_tasks()
        .iter()
        .filter(|t| t.completed)
    {
        if let Some(index) = task.completion_branch {
            if let Some(c) = task.get_task_def().stage.completions.get(index) {
                client.send_packet(
                    P_FE2CL_COMPLETION_BRANCH,
                    &NativeCompletionBranch {
                        task: task.get_task_id(),
                        variant: index as i32 + 1,
                        npc: c.npc,
                        next_task: c.next_task,
                    },
                );
                client.send_packet(
                    P_FE2CL_REP_PC_TASK_END_SUCC,
                    &sP_FE2CL_REP_PC_TASK_END_SUCC {
                        iTaskNum: task.get_task_id(),
                    },
                );
            }
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerStage {
    pub branches: BTreeMap<i32, usize>,
    pub actors: Vec<Actor>,
    pub visibility: Vec<Visibility>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Visibility {
    pub mission: i32,
    pub task: i32,
    pub rule: NpcVisibility,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Actor {
    pub mission: i32,
    pub task: i32,
    pub after: bool,
    pub index: usize,
    pub action: NpcRoute,
    pub position: Option<Position>,
    pub path: Option<Path>,
    pub done: bool,
    pub active: bool,
    #[serde(skip)]
    pub runtime_id: Option<i32>,
}
impl PlayerStage {
    pub fn serialize(&self) -> FFResult<String> {
        serde_json::to_string(self).map_err(|e| FFError::build(Severity::Warning, e.to_string()))
    }
    pub fn load(value: &str) -> FFResult<Self> {
        let stage: Self = serde_json::from_str(value)
            .map_err(|e| FFError::build(Severity::Warning, e.to_string()))?;
        if stage.actors.len() > 8192 || stage.visibility.len() > 8192 {
            return Err(FFError::build(
                Severity::Warning,
                "Too many saved mission NPC actions".into(),
            ));
        }
        Ok(stage)
    }
}
pub fn stage_started(state: &mut ShardServerState, pc: i32, task: i32, after: bool) {
    let def = tdata_get().get_task_definition(task).unwrap();
    let player = state.get_player_mut(pc).unwrap();
    let routes = if after {
        &def.stage.after
    } else {
        &def.stage.before
    };
    for (index, action) in routes.iter().enumerate() {
        if player
            .native_stage
            .actors
            .iter()
            .any(|a| a.active && a.task == task && a.after == after && a.index == index)
        {
            continue;
        }
        player.native_stage.actors.push(Actor {
            mission: def.mission_id,
            task,
            after,
            index,
            action: action.clone(),
            position: None,
            path: None,
            done: false,
            active: true,
            runtime_id: None,
        });
    }
    if !after
        && !player
            .native_stage
            .visibility
            .iter()
            .any(|v| v.task == task)
    {
        for rule in &def.stage.visibility {
            player.native_stage.visibility.push(Visibility {
                mission: def.mission_id,
                task,
                rule: rule.clone(),
            });
        }
    }
    tick_player(state, pc);
}
pub fn tick_player(state: &mut ShardServerState, pc: i32) {
    let Ok(player) = state.get_player(pc) else {
        return;
    };
    let instance = player.instance_id;
    let actors = player.native_stage.actors.clone();
    for (index, actor) in actors.iter().enumerate().filter(|(_, a)| a.active) {
        if actors[..index]
            .iter()
            .any(|old| old.active && old.action.npc_id == actor.action.npc_id && !old.done)
        {
            continue;
        }
        if let Some(id) = actor.runtime_id {
            if state
                .get_npc(id)
                .is_ok_and(|npc| npc.instance_id == instance)
            {
                continue;
            }
            if state.get_npc(id).is_ok() {
                state.entity_map.update(EntityID::NPC(id), None, true);
                state.entity_map.mark_for_cleanup(EntityID::NPC(id));
            }
            state.get_player_mut(pc).unwrap().native_stage.actors[index].runtime_id = None;
        }
        let source = state
            .entity_map
            .find_npcs(|npc| {
                npc.private_owner.is_none()
                    && npc.instance_id == instance
                    && (npc.authored_id == Some(actor.action.npc_id)
                        || (npc.authored_id.is_none() && npc.id == actor.action.npc_id))
            })
            .first()
            .and_then(|id| state.get_npc(*id).ok())
            .cloned();
        let Some(mut npc) = source else {
            continue;
        };
        if actor.done && actor.action.finish == RouteFinish::Hide {
            continue;
        }
        let mut prior_position = None;
        for (old_index, old) in actors[..index]
            .iter()
            .enumerate()
            .filter(|(_, old)| old.active && old.action.npc_id == actor.action.npc_id)
        {
            prior_position = old.position;
            if let Some(id) = old.runtime_id {
                if state.get_npc(id).is_ok() {
                    state.entity_map.update(EntityID::NPC(id), None, true);
                    state.entity_map.mark_for_cleanup(EntityID::NPC(id));
                }
            }
            let saved = &mut state.get_player_mut(pc).unwrap().native_stage.actors[old_index];
            saved.active = false;
            saved.runtime_id = None;
        }
        let id = state.entity_map.gen_next_npc_id();
        let position = actor
            .position
            .or(prior_position)
            .unwrap_or(npc.get_position());
        if actor.done && actor.action.finish == RouteFinish::Replace {
            let Ok(replacement) = NPC::new(
                id,
                actor.action.replacement_npc,
                position,
                npc.get_rotation(),
                instance,
            ) else {
                continue;
            };
            npc = replacement;
        }
        npc.id = id;
        npc.authored_id = None;
        npc.private_owner = Some(pc);
        npc.private_actor = Some(index);
        npc.hidden = false;
        npc.group_id = None;
        npc.ai = None;
        npc.authored_path = None;
        npc.mission_escort = None;
        npc.loose_follow = None;
        npc.tight_follow = None;
        npc.path = None;
        npc.set_position(position);
        npc.clear_buffs();
        npc.interacting_pcs.clear();
        let chunk = npc.get_chunk_coords();
        state.entity_map.track(Box::new(npc), TickMode::Always);
        state
            .entity_map
            .update(EntityID::NPC(id), Some(chunk), true);
        let saved = &mut state.get_player_mut(pc).unwrap().native_stage.actors[index];
        saved.runtime_id = Some(id);
        saved.position = Some(position);
        if !saved.done && saved.path.is_none() {
            saved.path = saved
                .action
                .route
                .parse()
                .ok()
                .and_then(|id| tdata_get().get_authored_npc_route(id));
        }
    }
    let player = state.get_player(pc).unwrap();
    let mut mask = HashMap::new();
    for visibility in &player.native_stage.visibility {
        if conditions_allowed(player, &visibility.rule.conditions) {
            for &id in &visibility.rule.npc_ids {
                mask.insert(id, visibility.rule.visible);
            }
        }
    }
    for actor in &player.native_stage.actors {
        if actor.active && (actor.runtime_id.is_some() || actor.done) {
            mask.insert(actor.action.npc_id, false);
        }
    }
    if mask != player.npc_visibility {
        let chunk = player.get_chunk_coords();
        state.entity_map.update(EntityID::Player(pc), None, true);
        let player = state.get_player_mut(pc).unwrap();
        player.npc_visibility = mask;
        player.last_npc_type_sync = None;
        if let Some(client) = player.get_client() {
            client.send_packet(
                crate::net::packet::PacketID::P_FE2CL_NPC_MAP_INVALIDATE,
                &crate::net::packet::NativeMapInvalidation { revision: 1 },
            );
        }
        state
            .entity_map
            .update(EntityID::Player(pc), Some(chunk), true);
    }
}
pub fn tick_actor(state: &mut ShardServerState, id: i32) -> bool {
    let npc = state.get_npc(id).unwrap();
    let (Some(pc), Some(index)) = (npc.private_owner, npc.private_actor) else {
        return false;
    };
    let Some(actor) = state
        .get_player(pc)
        .ok()
        .and_then(|p| p.native_stage.actors.get(index))
        .cloned()
        .filter(|a| a.active)
    else {
        state.entity_map.update(EntityID::NPC(id), None, true);
        state.entity_map.mark_for_cleanup(EntityID::NPC(id));
        return true;
    };
    if actor.done {
        return true;
    }
    let Some(mut path) = actor.path else {
        return true;
    };
    NPC::tick_movement_along_path(id, &mut path, state);
    let position = state.get_npc(id).unwrap().get_position();
    let done = path.is_done();
    let saved = &mut state.get_player_mut(pc).unwrap().native_stage.actors[index];
    saved.position = Some(position);
    saved.done = done;
    saved.path = (!done).then_some(path);
    if done {
        state.entity_map.update(EntityID::NPC(id), None, true);
        if actor.action.finish == RouteFinish::Hide {
            state.get_npc_mut(id).unwrap().hidden = true;
        }
        if actor.action.finish == RouteFinish::Replace {
            let old = state.get_npc(id).unwrap();
            if let Ok(mut replacement) = NPC::new(
                id,
                actor.action.replacement_npc,
                position,
                old.get_rotation(),
                old.instance_id,
            ) {
                replacement.private_owner = Some(pc);
                replacement.private_actor = Some(index);
                state.entity_map.untrack(EntityID::NPC(id));
                state
                    .entity_map
                    .track(Box::new(replacement), TickMode::Always);
            }
        }
        let chunk = state.get_npc(id).unwrap().get_chunk_coords();
        state
            .entity_map
            .update(EntityID::NPC(id), Some(chunk), true);
        let player = state.get_player_mut(pc).unwrap();
        player.last_npc_type_sync = None;
        if let Some(client) = player.get_client() {
            client.send_packet(
                crate::net::packet::PacketID::P_FE2CL_NPC_MAP_INVALIDATE,
                &crate::net::packet::NativeMapInvalidation { revision: 1 },
            );
        }
    }
    true
}
pub fn reset(state: &mut ShardServerState, pc: i32, mission: i32) {
    let actors = state.get_player(pc).unwrap().native_stage.actors.clone();
    for actor in actors.iter().filter(|a| a.mission == mission) {
        if let Some(id) = actor.runtime_id {
            if state.get_npc(id).is_ok() {
                state.entity_map.update(EntityID::NPC(id), None, true);
                state.entity_map.mark_for_cleanup(EntityID::NPC(id));
            }
        }
    }
    let player = state.get_player_mut(pc).unwrap();
    for actor in &mut player.native_stage.actors {
        if actor.mission == mission {
            actor.active = false;
            actor.runtime_id = None;
        }
    }
    player
        .native_stage
        .visibility
        .retain(|v| v.mission != mission);
    player.native_stage.branches.retain(|task, _| {
        tdata_get()
            .get_task_definition(*task)
            .is_ok_and(|d| d.mission_id != mission)
    });
    tick_player(state, pc);
}
