//! Ground-targeted NPC Mega attack (OpenFusion MobAI::useAbilities).
use crate::{
    corruption,
    entity::{Combatant, Entity as _, EntityID},
    enums::BuffID,
    error::*,
    net::packet::{PacketID::*, *},
    skills::{self, SkillResult},
    state::ShardServerState,
    tabledata::tdata_get,
    Position,
};

pub fn try_begin(npc_id: i32, state: &mut ShardServerState) -> FFResult<bool> {
    let npc = state.get_npc(npc_id)?;
    if npc.is_dead() || npc.corruption.is_some() || npc.eruption.is_some() {
        return Ok(false);
    }
    let stats = tdata_get().get_npc_stats(npc.ty)?;
    let Some(attack) = stats.eruption else {
        return Ok(false);
    };
    let Some(EntityID::Player(pc_id)) = npc.target_id else {
        return Ok(false);
    };
    // AI calls this after a failed corruption roll. Preserve the table's
    // unconditional Mega weight even on mobs that also carry corruption.
    let remaining = 2_000_000 - stats.corruption.map_or(0, |attack| attack.prob);
    if i64::from(corruption::roll()) * i64::from(remaining) >= i64::from(attack.prob) * 2_000_000 {
        return Ok(false);
    }
    let player = state.get_player(pc_id)?;
    if player.is_dead() {
        return Ok(false);
    }
    let position = player.get_position();
    let npc = state.get_npc_mut(npc_id)?;
    npc.path = None;
    npc.eruption = Some((attack.skill_id, position, pc_id));
    let ready = sP_FE2CL_NPC_SKILL_READY {
        iNPC_ID: npc_id,
        iSkillID: attack.skill_id,
        iValue1: position.x,
        iValue2: position.y,
        iValue3: position.z,
    };
    state
        .entity_map
        .for_each_around(EntityID::NPC(npc_id), |c| {
            c.send_packet(P_FE2CL_NPC_SKILL_READY, &ready);
        });
    Ok(true)
}

pub fn is_interrupted(npc_id: i32, state: &ShardServerState) -> bool {
    let Ok(npc) = state.get_npc(npc_id) else {
        return true;
    };
    let Some((_, _, pc_id)) = npc.eruption else {
        return true;
    };
    if npc.is_dead()
        || npc.retreating
        || npc.has_buff(BuffID::Stun, None)
        || npc.has_buff(BuffID::Sleep, None)
        || npc.target_id != Some(EntityID::Player(pc_id))
    {
        return true;
    }
    let Ok(player) = state.get_player(pc_id) else {
        return true;
    };
    let Ok(stats) = tdata_get().get_npc_stats(npc.ty) else {
        return true;
    };
    player.is_dead()
        || player.get_chunk_coords().i != npc.get_chunk_coords().i
        || player.get_position().distance_to(&npc.spawn_position) > stats.combat_range
}

pub fn cancel(npc_id: i32, state: &mut ShardServerState) -> FFResult<()> {
    if state.get_npc_mut(npc_id)?.eruption.take().is_some() {
        let packet = sP_FE2CL_NPC_SKILL_CANCEL { iNPC_ID: npc_id };
        state
            .entity_map
            .for_each_around(EntityID::NPC(npc_id), |c| {
                c.send_packet(P_FE2CL_NPC_SKILL_CANCEL, &packet);
            });
    }
    Ok(())
}

pub fn finish(npc_id: i32, state: &mut ShardServerState) -> FFResult<bool> {
    if is_interrupted(npc_id, state) {
        cancel(npc_id, state)?;
        return Ok(false);
    }
    let Some((skill_id, position, _)) = state.get_npc_mut(npc_id)?.eruption.take() else {
        return Ok(false);
    };
    let skill = tdata_get().get_skill(skill_id)?;
    let instance = state.get_npc(npc_id)?.get_chunk_coords().i;
    // The warning pins the ground point; moving away avoids the explosion.
    let targets: Vec<_> = state
        .entity_map
        .get_around_entity(EntityID::NPC(npc_id))
        .into_iter()
        .filter(|id| matches!(id, EntityID::Player(_)))
        .filter(|id| {
            state.get_combatant(*id).is_ok_and(|p| {
                let pos = p.get_position();
                let dx = f64::from(pos.x) - f64::from(position.x);
                let dy = f64::from(pos.y) - f64::from(position.y);
                !p.is_dead()
                    && p.get_chunk_coords().i == instance
                    && dx.hypot(dy) < f64::from(skill.range)
            })
        })
        .take(4)
        .collect();
    let results = skills::do_skill(EntityID::NPC(npc_id), &targets, skill, 0, state)?;
    broadcast_hit(
        npc_id,
        skill_id,
        position,
        skill.skill_type as i32,
        results,
        state,
    )?;
    Ok(true)
}

fn broadcast_hit(
    npc_id: i32,
    skill_id: i16,
    position: Position,
    skill_type: i32,
    results: Vec<SkillResult>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let mut packet = PacketBuilder::new(P_FE2CL_NPC_SKILL_HIT).with(&sP_FE2CL_NPC_SKILL_HIT {
        iNPC_ID: npc_id,
        iSkillID: skill_id,
        iValue1: position.x,
        iValue2: position.y,
        iValue3: position.z,
        eST: skill_type,
        iTargetCnt: results.len() as i32,
    });
    for result in results {
        match result {
            SkillResult::Damage(r) => packet.push(&r),
            SkillResult::DotDamage(r) => packet.push(&r),
            SkillResult::HealHP(r) => packet.push(&r),
            SkillResult::HealStamina(r) => packet.push(&r),
            SkillResult::StaminaSelf(r) => packet.push(&r),
            SkillResult::DamageAndDebuff(r) => packet.push(&r),
            SkillResult::Buff(r) => packet.push(&r),
            SkillResult::BatteryDrain(r) => packet.push(&r),
            SkillResult::DamageAndMove(r) => packet.push(&r),
            SkillResult::Move(r) => packet.push(&r),
            SkillResult::Resurrect(r) => packet.push(&r),
        };
    }
    let packet = packet.build()?;
    state
        .entity_map
        .for_each_around(EntityID::NPC(npc_id), |c| c.send_payload(packet.clone()));
    Ok(())
}
