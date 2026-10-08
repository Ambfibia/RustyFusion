//! Administrative mission actions share ordinary task initialization and grants.
use super::*;
use std::{future::Future,pin::Pin};
use crate::{tabledata::tdata_get,net::FFClient,entity::Combatant};

fn argument(tokens:&[&str])->Option<i32> {
    (tokens.len()==2).then(||tokens[1].parse::<i32>().ok()).flatten().filter(|id|*id>0)
}
pub(super) fn start<'a>(tokens:Vec<&'a str>,clients:&'a ClientMap<'a>,state:&'a mut ShardServerState)->Pin<Box<dyn Future<Output=FFResult<()>>+Send+'a>> {
    Box::pin(async move {run(&tokens,clients.get_sender(),state,false)})
}
pub(super) fn delete<'a>(tokens:Vec<&'a str>,clients:&'a ClientMap<'a>,state:&'a mut ShardServerState)->Pin<Box<dyn Future<Output=FFResult<()>>+Send+'a>> {
    Box::pin(async move {run(&tokens,clients.get_sender(),state,true)})
}
fn run(tokens:&[&str],client:&FFClient,state:&mut ShardServerState,delete:bool)->FFResult<()> {
    let pc_id=client.get_player_id()?;
    let player=state.get_player(pc_id)?;
    if player.perms>CN_ACCOUNT_LEVEL__DEVELOPER as i16 {return send_system_message(client,"You don't have access to that command!");}
    let Some(id)=argument(tokens) else {return send_system_message(client,&format!("Usage: /{} <mission ID>",tokens[0]));};
    let Ok(mission)=tdata_get().get_mission_definition(id) else {return send_system_message(client,&format!("Unknown mission ID: {id}"));};
    if delete {
        state.get_player_mut(pc_id)?.mission_journal.clear_mission_completed(id)?;
        return send_system_message(client,&format!("Quest {id} removed from completed missions."));
    }
    if player.mission_journal.is_mission_completed(id)? {return send_system_message(client,&format!("Quest {id} is completed. Use /deletequest {id} first."));}
    if player.mission_journal.get_current_tasks().iter().any(|t|t.get_mission_def().mission_id==id) {return send_system_message(client,&format!("Quest {id} is already active."));}
    let task=tdata_get().get_task_definition(mission.first_task_id)?;
    // Escort objectives still need a real entity in this player's instance.
    let escort=if let Some(ty)=task.obj_escort_npc_type {
        state.entity_map.get_npc_ids().filter_map(|id|state.get_npc(id).ok().map(|n|(id,n))).filter(|(_,n)|n.ty==ty && n.instance_id==player.instance_id && !n.is_dead()).min_by_key(|(_,n)|n.get_position().distance_to(&player.get_position())).map(|(id,_)|id)
    } else {Some(0)};
    let Some(escort)=escort else {return send_system_message(client,"The mission needs its escort NPC in the current instance.");};
    let request=sP_CL2FE_REQ_PC_TASK_START {iTaskNum:mission.first_task_id,iNPC_ID:0,iEscortNPC_ID:escort};
    match super::super::mission::start_requested_task(&request,client,state,true) {
        Ok(())=>send_system_message(client,&format!("Quest {id} started.")),
        Err(error)=>send_system_message(client,&format!("Unable to start quest {id}: {}",error.get_msg())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn strict_mission_argument() {
        assert_eq!(argument(&["startquest","42"]),Some(42));
        for tokens in [vec!["startquest"],vec!["deletequest","0"],vec!["startquest","-1"],vec!["startquest","2","extra"],vec!["startquest","x"]] {assert_eq!(argument(&tokens),None);}
    }
}
