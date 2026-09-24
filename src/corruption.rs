//! NPC corruption attacks: the rock-paper-scissors special move, ported from
//! OpenFusion `MobAI::useAbilities` and `dealCorruption`.
//!
//! A mob whose table entry has an `m_iCorruptionType` rolls for it each time it
//! is about to attack. On a hit it broadcasts `NPC_SKILL_CORRUPTION_READY` with
//! the style that beats its target's active nano, stops and can't be hurt
//! while it winds up, then resolves `NPC_SKILL_CORRUPTION_HIT` against whatever
//! nano the target has out by then. The AI script owns the timing; this module
//! decides and applies.

use rand::Rng;

use crate::{
    defines::*,
    entity::{Combatant, Entity as _, EntityID, PendingCorruption},
    enums::{BuffID, CharType, CombatStyle, SkillTargetType, SkillType, TargetType},
    error::*,
    net::packet::{PacketID::*, *},
    skills::{self, Skill, SkillResult},
    state::ShardServerState,
    tabledata::tdata_get,
};

/// Time between READY and HIT (OpenFusion `nextAttack = currTime + 1800`).
pub const CORRUPTION_WINDUP_SECONDS: f64 = 1.8;
/// Pause after the HIT before the mob acts again (`currTime + 1000`).
pub const CORRUPTION_COOLDOWN_SECONDS: f64 = 1.0;

/// Nano stamina a won exchange gives back, and the cap it's clamped to.
const WIN_STAMINA: i16 = 45;
const WIN_STAMINA_CAP: i16 = 150;
/// Nano stamina a lost exchange takes.
const LOSE_STAMINA: i16 = 90;

/// Winning turns the attack back on the mob as a 200-power damage "nano
/// skill" (OpenFusion builds the same throwaway `SkillData`).
static COUNTER_SKILL: Skill = Skill {
    skill_type: SkillType::Damage,
    targeting_type: SkillTargetType::Target,
    target_type: TargetType::HostileNPCs,
    passive: false,
    range: 0,
    cast_range: 0,
    target_count: 1,
    cooldown: std::time::Duration::ZERO,
    values_a: [200; 4],
    values_b: [None; 4],
    values_c: [None; 4],
    costs: [0; 4],
    durations: [None; 4],
};

#[cfg(test)]
thread_local! {
    /// Pins the next corruption rolls, so tests don't depend on the 5% chance.
    pub(crate) static FORCED_ROLL: std::cell::Cell<Option<i32>> =
        const { std::cell::Cell::new(None) };
}

/// OpenFusion `Rand::rand(2000) * 1000`: table probabilities are out of
/// 2,000,000, so the usual weight of 100,000 is a 5% chance per attack.
fn roll() -> i32 {
    #[cfg(test)]
    if let Some(roll) = FORCED_ROLL.with(|r| r.get()) {
        return roll;
    }
    rand::thread_rng().gen_range(0..2000) * 1000
}

/// The style that beats `style`.
pub fn counter_style(style: CombatStyle) -> CombatStyle {
    match style {
        CombatStyle::Adaptium => CombatStyle::Cosmix,
        CombatStyle::Blastons => CombatStyle::Adaptium,
        CombatStyle::Cosmix => CombatStyle::Blastons,
    }
}

/// The style a mob winds up against a target: whatever beats the target's
/// active nano, or a random one when there's no nano out.
pub fn windup_style(target_nano: Option<CombatStyle>) -> CombatStyle {
    match target_nano {
        Some(style) => counter_style(style),
        None => match rand::thread_rng().gen_range(0..3) {
            0 => CombatStyle::Adaptium,
            1 => CombatStyle::Blastons,
            _ => CombatStyle::Cosmix,
        },
    }
}

/// How a corruption exchange went, from the player's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorruptionOutcome {
    /// No nano out: the full hit lands, reported as a tie.
    NoNano,
    /// Same style: nothing happens.
    Tie,
    /// The nano beats the mob's style: stamina back and a counter-hit.
    Win,
    /// The mob's style beats the nano: full hit and a stamina drain.
    Lose,
}

pub fn corruption_outcome(mob: CombatStyle, nano: Option<CombatStyle>) -> CorruptionOutcome {
    match nano {
        None => CorruptionOutcome::NoNano,
        Some(nano) if nano == mob => CorruptionOutcome::Tie,
        Some(nano) if nano == counter_style(mob) => CorruptionOutcome::Win,
        Some(_) => CorruptionOutcome::Lose,
    }
}

/// OpenFusion scales the skill's first value by a player's max HP at the
/// mob's level, `PC_MAXHEALTH(level) = 925 + 75 * level`.
pub fn corruption_damage(skill: &Skill, npc_level: i16) -> i32 {
    skill.values_a[0] * (925 + 75 * npc_level as i32) / 1500
}

/// Rolls for a corruption attack and, on a hit, starts the windup. Only mobs
/// with a corruption skill that are fighting a live player can roll. Returns
/// whether a windup started.
pub fn try_begin(npc_id: i32, state: &mut ShardServerState) -> FFResult<bool> {
    let npc = state.get_npc(npc_id)?;
    if npc.corruption.is_some() || npc.is_dead() {
        return Ok(false);
    }
    let Some(corruption) = tdata_get().get_npc_stats(npc.ty)?.corruption else {
        return Ok(false);
    };
    let Some(EntityID::Player(pc_id)) = npc.target_id else {
        return Ok(false);
    };
    if roll() >= corruption.prob {
        return Ok(false);
    }

    let Ok(player) = state.get_player(pc_id) else {
        return Ok(false);
    };
    if player.is_dead() {
        return Ok(false);
    }
    let target_pos = player.get_position();
    let style = windup_style(Combatant::get_style(player));

    let npc = state.get_npc_mut(npc_id)?;
    // a mob stands still while it casts
    npc.path = None;
    npc.corruption = Some(PendingCorruption {
        skill_id: corruption.skill_id,
        style,
        target_pc_id: pc_id,
    });

    let pkt = sP_FE2CL_NPC_SKILL_CORRUPTION_READY {
        iNPC_ID: npc_id,
        iSkillID: corruption.skill_id,
        iStyle: style as i16,
        iValue1: target_pos.x,
        iValue2: target_pos.y,
        iValue3: target_pos.z,
    };
    state.entity_map.for_each_around(EntityID::NPC(npc_id), |c| {
        c.send_packet(P_FE2CL_NPC_SKILL_CORRUPTION_READY, &pkt);
    });
    Ok(true)
}

/// Whether a windup can no longer land: the mob died, got stunned or put to
/// sleep, dropped its target, or the target died, left, or ran out of the
/// mob's combat range (all of which end OpenFusion's combat step first).
pub fn is_interrupted(npc_id: i32, state: &ShardServerState) -> bool {
    let Ok(npc) = state.get_npc(npc_id) else {
        return true;
    };
    let Some(pending) = npc.corruption else {
        return true;
    };
    if npc.is_dead()
        || npc.retreating
        || npc.has_buff(BuffID::Stun, None)
        || npc.has_buff(BuffID::Sleep, None)
        || npc.target_id != Some(EntityID::Player(pending.target_pc_id))
    {
        return true;
    }
    let Ok(player) = state.get_player(pending.target_pc_id) else {
        return true;
    };
    let Ok(stats) = tdata_get().get_npc_stats(npc.ty) else {
        return true;
    };
    player.is_dead()
        || player.get_chunk_coords().i != npc.get_chunk_coords().i
        || player.get_position().distance_to(&npc.spawn_position) > stats.combat_range
}

/// Ends a windup: lands the HIT, or cancels it if it was interrupted.
/// Returns whether the HIT landed.
pub fn finish(npc_id: i32, state: &mut ShardServerState) -> FFResult<bool> {
    let interrupted = is_interrupted(npc_id, state);
    let Some(pending) = state.get_npc_mut(npc_id)?.corruption.take() else {
        return Ok(false);
    };
    if interrupted {
        send_cancel(npc_id, state);
        return Ok(false);
    }
    if let Err(e) = resolve(npc_id, pending, state) {
        send_cancel(npc_id, state);
        return Err(e);
    }
    Ok(true)
}

/// Drops any windup in progress and tells the clients to clear its effect.
pub fn cancel(npc_id: i32, state: &mut ShardServerState) -> FFResult<()> {
    if state.get_npc_mut(npc_id)?.corruption.take().is_some() {
        send_cancel(npc_id, state);
    }
    Ok(())
}

fn send_cancel(npc_id: i32, state: &mut ShardServerState) {
    let pkt = sP_FE2CL_NPC_SKILL_CANCEL { iNPC_ID: npc_id };
    state.entity_map.for_each_around(EntityID::NPC(npc_id), |c| {
        c.send_packet(P_FE2CL_NPC_SKILL_CANCEL, &pkt);
    });
}

/// OpenFusion `dealCorruption` for its single target.
fn resolve(npc_id: i32, pending: PendingCorruption, state: &mut ShardServerState) -> FFResult<()> {
    let skill = tdata_get().get_skill(pending.skill_id)?;
    let npc_eid = EntityID::NPC(npc_id);
    let full_damage = corruption_damage(skill, state.get_npc(npc_id)?.get_level());

    let pc_id = pending.target_pc_id;
    let player = state.get_player_mut(pc_id)?;
    let outcome = corruption_outcome(pending.style, Combatant::get_style(&*player));
    let active_slot = player.get_active_nano_slot().map_or(-1, |slot| slot as i16);
    let mut nano_deactive = false;
    let (hit_flag, damage) = match outcome {
        CorruptionOutcome::NoNano => (HF_BIT_STYLE_TIE, full_damage),
        CorruptionOutcome::Tie => (HF_BIT_STYLE_TIE, 0),
        CorruptionOutcome::Win => {
            let nano = player.get_active_nano_mut().unwrap();
            let stamina = (nano.get_stamina() + WIN_STAMINA).min(WIN_STAMINA_CAP);
            nano.set_stamina(stamina);
            (HF_BIT_STYLE_WIN, 0)
        }
        CorruptionOutcome::Lose => {
            let nano = player.get_active_nano_mut().unwrap();
            let stamina = nano.get_stamina() - LOSE_STAMINA;
            nano_deactive = stamina < 0;
            nano.set_stamina(stamina);
            (HF_BIT_STYLE_LOSE, full_damage)
        }
    };
    let (nano_id, nano_stamina) = player
        .get_active_nano()
        .map_or((0, 0), |nano| (nano.get_id(), nano.get_stamina()));
    let dealt = player.take_damage(damage, Some(npc_eid));

    let result = sCAttackResult {
        eCT: CharType::Player as i32,
        iID: pc_id,
        bProtected: 0,
        iDamage: dealt,
        iHP: player.get_hp(),
        iHitFlag: hit_flag as i8,
        iActiveNanoSlotNum: active_slot,
        bNanoDeactive: nano_deactive as i32,
        iNanoID: nano_id,
        iNanoStamina: nano_stamina,
        iConditionBitFlag: player.get_condition_bit_flag(),
        eCSTB___Del: unused!(),
    };
    let target_pos = player.get_position();

    // OpenFusion fires the counter-hit before it sends the HIT itself
    if outcome == CorruptionOutcome::Win {
        log_if_failed(counter_hit(pc_id, npc_id, state));
    }

    let pkt = PacketBuilder::new(P_FE2CL_NPC_SKILL_CORRUPTION_HIT)
        .with(&sP_FE2CL_NPC_SKILL_CORRUPTION_HIT {
            iNPC_ID: npc_id,
            iSkillID: pending.skill_id,
            iStyle: pending.style as i16,
            iValue1: target_pos.x,
            iValue2: target_pos.y,
            iValue3: target_pos.z,
            iTargetCnt: 1,
        })
        .with(&result)
        .build()?;
    state.entity_map.for_each_around(npc_eid, |c| {
        c.send_payload(pkt.clone());
    });
    Ok(())
}

/// A won exchange fires the player's nano back at the mob, reported like a
/// nano skill so the client shows the damage.
fn counter_hit(pc_id: i32, npc_id: i32, state: &mut ShardServerState) -> FFResult<()> {
    let player = state.get_player(pc_id)?;
    let Some(nano) = player.get_active_nano() else {
        return Ok(());
    };
    // the client drops nano results without a skill ID, so an untuned nano
    // has nothing to show the hit with
    let Some(skill_id) = nano.selected_skill else {
        return Ok(());
    };
    let nano_id = nano.get_id();
    let nano_stamina = nano.get_stamina();
    let level = player.get_nano_skill_level();
    let client = player.get_client();

    let results = skills::do_skill(
        EntityID::Player(pc_id),
        &[EntityID::NPC(npc_id)],
        &COUNTER_SKILL,
        level,
        state,
    )?;
    let damage: Vec<_> = results
        .into_iter()
        .filter_map(|r| match r {
            SkillResult::Damage(sr) => Some(sr),
            _ => None,
        })
        .collect();
    if damage.is_empty() {
        return Ok(());
    }

    let header = sP_FE2CL_NANO_SKILL_USE_SUCC {
        iPC_ID: pc_id,
        iBulletID: 0,
        iSkillID: skill_id,
        iArg1: 0,
        iArg2: 0,
        iArg3: 0,
        bNanoDeactive: (nano_stamina <= 0) as i32,
        iNanoID: nano_id,
        iNanoStamina: nano_stamina,
        eST: SkillType::Damage as i32,
        iTargetCnt: damage.len() as i32,
    };
    let bcast_header = sP_FE2CL_NANO_SKILL_USE {
        iPC_ID: header.iPC_ID,
        iBulletID: header.iBulletID,
        iSkillID: header.iSkillID,
        iArg1: header.iArg1,
        iArg2: header.iArg2,
        iArg3: header.iArg3,
        bNanoDeactive: header.bNanoDeactive,
        iNanoID: header.iNanoID,
        iNanoStamina: header.iNanoStamina,
        eST: header.eST,
        iTargetCnt: header.iTargetCnt,
    };
    let mut succ = PacketBuilder::new(P_FE2CL_NANO_SKILL_USE_SUCC).with(&header);
    let mut bcast = PacketBuilder::new(P_FE2CL_NANO_SKILL_USE).with(&bcast_header);
    for sr in &damage {
        succ.push(sr);
        bcast.push(sr);
    }

    if let Some(client) = client {
        client.send_payload(succ.build()?);
    }
    let bcast = bcast.build()?;
    state
        .entity_map
        .for_each_around(EntityID::Player(pc_id), |c| {
            c.send_payload(bcast.clone());
        });
    Ok(())
}

#[cfg(test)]
#[path = "corruption_tests.rs"]
mod tests;
