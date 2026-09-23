use crate::{
    chunk::TickMode,
    entity::{Combatant, Entity, EntityID, NPC},
    enums::CombatantTeam,
    error::{log_if_failed, FFResult},
    net::packet::{PacketID::*, *},
    skills::{self, SkillResult},
    state::ShardServerState,
    tabledata::tdata_get,
};

/// Lord Fuse fights in three bodies. The first two summon the next stage (and
/// one of his arms) when they go down; the last one is just a mob.
pub const NPC_TYPE_FUSE_STAGE_1: i32 = 2466;
pub const NPC_TYPE_FUSE_STAGE_2: i32 = 2467;

/// The heal a mob casts on itself once it is back home after a retreat
/// (OpenFusion `MobAI::onRoamStart`).
pub const RETURN_HOME_HEAL_SKILL: i16 = 110;

fn get_script_name(npc: &NPC) -> Option<&'static str> {
    let is_combatant = npc.as_combatant().is_some();
    if !is_combatant {
        return None;
    }

    if matches!(npc.ty, NPC_TYPE_FUSE_STAGE_1 | NPC_TYPE_FUSE_STAGE_2) {
        return Some("lord_fuse");
    }

    let stats = tdata_get().get_npc_stats(npc.ty).unwrap();
    match stats.team {
        CombatantTeam::Friendly => Some("friendly_combatant"),
        CombatantTeam::Mob => {
            if npc.tight_follow.is_some() {
                Some("mob_pack_member")
            } else {
                Some("mob")
            }
        }
        _ => None,
    }
}

pub fn make_for_npc(npc: &NPC, force: bool) -> (Option<String>, TickMode) {
    let stats = tdata_get().get_npc_stats(npc.ty).unwrap();
    if !force && npc.path.is_none() && stats.ai_type == 0 {
        return (None, TickMode::Never);
    }

    let behavior_name = get_script_name(npc);
    let tick_mode = if npc.path.is_some() {
        TickMode::Always
    } else {
        TickMode::WhenLoaded
    };

    (behavior_name.map(|name| name.to_string()), tick_mode)
}

/// Resets a mob that has made it back to its spawn point and tells everyone
/// watching. The reset alone only heals the mob server-side, which would leave
/// clients that saw the fight drawing the old HP bar; OpenFusion instead casts
/// a ReturnHomeHeal on the mob, whose NPC_SKILL_HIT carries the new HP
/// (`MobAI::onRoamStart`). Clients that come into range later get the healed
/// HP in NPC_ENTER as usual.
pub fn return_home(npc_id: i32, state: &mut ShardServerState) -> FFResult<()> {
    let npc = state.get_npc_mut(npc_id)?;
    npc.reset();
    let pos = npc.get_position();

    let npc_eid = EntityID::NPC(npc_id);
    let skill = tdata_get().get_skill(RETURN_HOME_HEAL_SKILL)?;
    let results = skills::do_skill(npc_eid, &[npc_eid], skill, 0, state)?;
    let heals: Vec<_> = results
        .into_iter()
        .filter_map(|result| match result {
            SkillResult::HealHP(heal) => Some(heal),
            _ => None,
        })
        .collect();

    let mut builder = PacketBuilder::new(P_FE2CL_NPC_SKILL_HIT).with(&sP_FE2CL_NPC_SKILL_HIT {
        iNPC_ID: npc_id,
        iSkillID: RETURN_HOME_HEAL_SKILL,
        iValue1: pos.x,
        iValue2: pos.y,
        iValue3: pos.z,
        eST: skill.skill_type as i32,
        iTargetCnt: heals.len() as i32,
    });
    for heal in &heals {
        builder.push(heal);
    }
    if let Some(pkt) = log_if_failed(builder.build()) {
        state
            .entity_map
            .for_each_around(npc_eid, |c| c.send_payload(pkt.clone()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "ai_tests.rs"]
mod tests;
