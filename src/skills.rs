use std::{
    collections::{hash_map::Entry, HashMap},
    time::{Duration, Instant},
};

use rand::Rng;

use crate::{
    chunk::InstanceID,
    defines::*,
    entity::{Combatant, Entity as _, EntityID},
    enums::{
        BuffID, BuffType, CharType, CombatStyle, SkillTargetType, SkillType, TargetType,
        TimeBuffUpdate,
    },
    error::*,
    net::packet::{PacketID::*, *},
    state::ShardServerState,
    Position,
};

pub enum SkillResult {
    Damage(sSkillResult_Damage),
    DotDamage(sSkillResult_DotDamage),
    HealHP(sSkillResult_Heal_HP),
    HealStamina(sSkillResult_Heal_Stamina),
    StaminaSelf(sSkillResult_Stamina_Self),
    DamageAndDebuff(sSkillResult_Damage_N_Debuff),
    Buff(sSkillResult_Buff),
    BatteryDrain(sSkillResult_BatteryDrain),
    DamageAndMove(sSkillResult_Damage_N_Move),
    Move(sSkillResult_Move),
    Resurrect(sSkillResult_Resurrect),
}

#[derive(Debug)]
pub struct Skill {
    pub skill_type: SkillType,
    pub targeting_type: SkillTargetType,
    pub target_type: TargetType,
    pub passive: bool,
    pub range: u32,
    pub values_a: [i32; SKILL_LEVEL_MAX + 1],
    pub values_b: [Option<i32>; SKILL_LEVEL_MAX + 1],
    pub values_c: [Option<i32>; SKILL_LEVEL_MAX + 1],
    pub costs: [i16; SKILL_LEVEL_MAX + 1],
    pub durations: [Option<Duration>; SKILL_LEVEL_MAX + 1],
}
impl Skill {
    pub fn get_buff_id(&self) -> Option<BuffID> {
        let buff_id = match self.skill_type {
            SkillType::Run => BuffID::UpMoveSpeed,
            SkillType::Jump => BuffID::UpJumpHeight,
            SkillType::Stealth => BuffID::UpStealth,
            SkillType::Phoenix => BuffID::Phoenix,
            SkillType::ProtectBattery => BuffID::ProtectBattery,
            SkillType::ProtectInfection => BuffID::ProtectInfection,
            SkillType::Snare => BuffID::DnMoveSpeed,
            SkillType::Sleep => BuffID::Sleep,
            SkillType::MiniMapEnemy => BuffID::MiniMapEnemy,
            SkillType::MiniMapTreasure => BuffID::MiniMapTreasure,
            SkillType::RewardBlob => BuffID::RewardBlob,
            SkillType::RewardCash => BuffID::RewardCash,
            SkillType::InfectionDamage => BuffID::Infection,
            SkillType::Freedom => BuffID::Freedom,
            SkillType::BoundingBall => BuffID::BoundingBall,
            SkillType::Invulnerable => BuffID::Invulnerable,
            SkillType::BuffHeal => BuffID::Heal,
            SkillType::NanoStimPak => BuffID::StimPakSlot1,
            _ => return None,
        };

        Some(buff_id)
    }

    /// Like [`Skill::make_buff_instance`], but doesn't require the skill type
    /// itself to map to a buff. Needed for E.G.G.s and gumballs, which pick
    /// the buff slot themselves.
    pub fn make_buff_instance_forced(&self, ty: BuffType, level: usize) -> FFResult<BuffInstance> {
        if level > SKILL_LEVEL_MAX {
            return Err(FFError::build(
                Severity::Warning,
                format!("Skill level {} is above max of {}", level, SKILL_LEVEL_MAX),
            ));
        }

        let duration = if self.passive {
            None
        } else {
            self.durations[level]
        };
        Ok(BuffInstance::new(
            ty,
            self.values_a[level],
            self.values_b[level],
            self.values_c[level],
            duration,
        ))
    }

    pub fn make_buff_instance(&self, ty: BuffType, level: usize) -> FFResult<BuffInstance> {
        if level > SKILL_LEVEL_MAX {
            return Err(FFError::build(
                Severity::Warning,
                format!("Skill level {} is above max of {}", level, SKILL_LEVEL_MAX),
            ));
        }

        if self.get_buff_id().is_none() {
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "Skill type {:?} does not have an associated buff",
                    self.skill_type
                ),
            ));
        }

        let value = self.values_a[level];
        let sub_value = self.values_b[level];
        let special_value = self.values_c[level];
        let duration = if self.passive {
            None
        } else {
            self.durations[level]
        };

        let buff = BuffInstance::new(ty, value, sub_value, special_value, duration);
        Ok(buff)
    }
}

#[derive(Debug)]
pub enum BuffUpdate {
    Added(BuffID, BuffType, sTimeBuff),
    Changed(BuffID, BuffType, sTimeBuff),
    Removed(BuffID),
}
impl From<BuffUpdate> for sP_FE2CL_PC_BUFF_UPDATE {
    fn from(update: BuffUpdate) -> Self {
        match update {
            BuffUpdate::Added(buff_id, source, time_buff) => Self {
                eTBU: TimeBuffUpdate::Add as i32,
                eTBT: source as i32,
                eCSTB: buff_id as i32,
                TimeBuff: time_buff,
                iConditionBitFlag: 0, // set by caller based on active buffs
            },
            BuffUpdate::Changed(buff_id, source, time_buff) => Self {
                eTBU: TimeBuffUpdate::Change as i32,
                eTBT: source as i32,
                eCSTB: buff_id as i32,
                TimeBuff: time_buff,
                iConditionBitFlag: 0, // set by caller based on active buffs
            },
            BuffUpdate::Removed(buff_id) => Self {
                eTBU: TimeBuffUpdate::Del as i32,
                eTBT: unused!(),
                eCSTB: buff_id as i32,
                TimeBuff: unused!(),
                iConditionBitFlag: 0, // set by caller based on active buffs
            },
        }
    }
}

#[derive(Debug)]
pub enum BuffEffect {
    HealEntity {
        target: EntityID,
        amount: i32,
    },
    DamageEntity {
        target: EntityID,
        source: Option<EntityID>,
        damage: i32,
    },
    /// Damage expressed as a percentage of the target's max HP. Used by
    /// Bounding Ball, which drains a fixed fraction rather than a flat amount.
    DrainEntity {
        target: EntityID,
        source: Option<EntityID>,
        percent_max_hp: i32,
    },
}

#[derive(Debug, Clone)]
pub struct BuffInstance {
    ty: BuffType,
    value: i32,
    _sub_value: Option<i32>,
    _special_value: Option<i32>,
    onset: Instant,
    expires: Option<Instant>,
    source: Option<EntityID>,
}
/// How often a buff that does something over time (rather than just setting a
/// condition flag) actually does it.
const BUFF_EFFECT_INTERVAL: Duration = Duration::from_secs(2);
/// Fraction (in percent of max HP) Bounding Ball drains per effect interval.
const BOUNDING_BALL_DRAIN_PERCENT: i32 = 2;
impl BuffInstance {
    pub fn new(
        ty: BuffType,
        value: i32,
        sub_value: Option<i32>,
        special_value: Option<i32>,
        duration: Option<Duration>,
    ) -> Self {
        let expires = duration.map(|d| Instant::now() + d);
        Self {
            ty,
            value,
            _sub_value: sub_value,
            _special_value: special_value,
            expires,
            onset: Instant::now(),
            source: None,
        }
    }

    fn is_expired(&self) -> bool {
        if let Some(expires) = self.expires {
            Instant::now() >= expires
        } else {
            false
        }
    }

    pub fn set_source(&mut self, source: EntityID) {
        self.source = Some(source);
    }
}

#[derive(Debug, Clone)]
struct BuffStack {
    buffs: Vec<BuffInstance>,
    applied: bool,
    changed: bool,
    remove: bool,
    last_effect: Option<Instant>,
}
impl BuffStack {
    fn new(first_stack: BuffInstance) -> Self {
        Self {
            buffs: vec![first_stack],
            applied: false,
            changed: false,
            remove: false,
            last_effect: None,
        }
    }

    fn add_stack(&mut self, buff: BuffInstance) {
        self.buffs.push(buff);
        self.changed = true;
    }

    fn remove_stacks(&mut self, buff_type: Option<BuffType>) {
        if let Some(buff_type) = buff_type {
            self.buffs.retain(|b| b.ty != buff_type);
        } else {
            self.buffs.clear();
        }
        self.changed = true;
    }

    fn has_stack(&self, buff_type: BuffType) -> bool {
        self.buffs.iter().any(|b| b.ty == buff_type)
    }

    fn tick(
        &mut self,
        buff_id: BuffID,
        target: EntityID,
        updates: &mut Vec<BuffUpdate>,
        effects: &mut Vec<BuffEffect>,
    ) {
        if !self.buffs.is_empty() {
            if !self.applied {
                self.on_stack_apply(buff_id, target);
                self.applied = true;
                self.changed = false;
                updates.push(BuffUpdate::Added(
                    buff_id,
                    self.get_dominant_type(),
                    (&*self).into(),
                ));
            }

            if self.changed {
                self.on_stack_change(buff_id, target);
                self.changed = false;
                updates.push(BuffUpdate::Changed(
                    buff_id,
                    self.get_dominant_type(),
                    (&*self).into(),
                ));
            }

            self.on_stack_tick(buff_id, target);
            self.tick_effects(buff_id, target, effects);
            self.buffs.retain(|buff| !buff.is_expired());
        }

        if self.buffs.is_empty() {
            self.on_stack_remove(buff_id, target);
            self.remove = true;
            updates.push(BuffUpdate::Removed(buff_id));
        }
    }

    /// Applies whatever this buff does over time, at most once per
    /// [`BUFF_EFFECT_INTERVAL`]. Most buffs are purely a condition flag that
    /// the client acts on, so they do nothing here.
    fn tick_effects(&mut self, buff_id: BuffID, target: EntityID, effects: &mut Vec<BuffEffect>) {
        let effect = match buff_id {
            BuffID::BoundingBall => BuffEffect::DrainEntity {
                target,
                source: self.get_last_source(),
                percent_max_hp: BOUNDING_BALL_DRAIN_PERCENT,
            },
            _ => return,
        };

        let now = Instant::now();
        if self
            .last_effect
            .is_some_and(|last| now.duration_since(last) < BUFF_EFFECT_INTERVAL)
        {
            return;
        }
        self.last_effect = Some(now);
        effects.push(effect);
    }

    fn get_last_source(&self) -> Option<EntityID> {
        self.buffs.last().and_then(|buff| buff.source)
    }

    fn get_max_value(&self) -> i32 {
        self.buffs.iter().map(|b| b.value).max().unwrap_or(0)
    }

    fn get_dominant_type(&self) -> BuffType {
        self.buffs
            .iter()
            .max_by_key(|b| b.value)
            .map(|b| b.ty)
            .unwrap_or(BuffType::Nano)
    }

    fn get_expires(&self) -> Option<Instant> {
        // if any instance doesn't expire, then the whole buff doesn't expire.
        // otherwise, the buff expires when the last instance expires.
        if self.buffs.iter().any(|b| b.expires.is_none()) {
            None
        } else {
            self.buffs.iter().map(|b| b.expires.unwrap()).max()
        }
    }

    fn get_onset(&self) -> Instant {
        self.buffs
            .iter()
            .map(|b| b.onset)
            .min()
            .unwrap_or_else(Instant::now)
    }

    fn get_duration(&self) -> Option<Duration> {
        let onset = self.get_onset();
        self.get_expires()
            .map(|expires| expires.duration_since(onset))
    }

    fn on_stack_apply(&mut self, buff_id: BuffID, target: EntityID) {
        // do stuff
        log(
            Severity::Debug,
            &format!("Buff {:?} applied to {:?}", buff_id, target),
        );
    }

    fn on_stack_change(&mut self, buff_id: BuffID, target: EntityID) {
        // do stuff
        log(
            Severity::Debug,
            &format!("Buff {:?} changed on {:?}", buff_id, target),
        );
    }

    fn on_stack_remove(&mut self, buff_id: BuffID, target: EntityID) {
        // do stuff
        log(
            Severity::Debug,
            &format!("Buff {:?} removed from {:?}", buff_id, target),
        );
    }

    fn on_stack_tick(&mut self, buff_id: BuffID, target: EntityID) -> bool {
        // do stuff
        log(
            Severity::Debug,
            &format!("Buff {:?} ticked on {:?}", buff_id, target),
        );
        false
    }
}
impl From<&BuffStack> for sTimeBuff {
    fn from(stack: &BuffStack) -> Self {
        let now = Instant::now();
        Self {
            iTimeLimit: match stack.get_expires() {
                Some(expires) => expires.saturating_duration_since(now).as_millis() as u64,
                None => 0,
            },
            iTimeDuration: stack.get_duration().map_or(0, |d| d.as_millis() as u64),
            iTimeRepeat: unused!(),
            iValue: stack.get_max_value(),
            iConfirmNum: unused!(),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct BuffContainer {
    buff_stacks: HashMap<BuffID, BuffStack>,
}
impl BuffContainer {
    pub fn add_buff(
        &mut self,
        buff_id: BuffID,
        mut buff: BuffInstance,
        source: Option<EntityID>,
    ) -> bool {
        if let Some(source) = source {
            buff.set_source(source);
        }

        match self.buff_stacks.entry(buff_id) {
            Entry::Occupied(mut entry) => {
                entry.get_mut().add_stack(buff);
                false
            }
            Entry::Vacant(entry) => {
                entry.insert(BuffStack::new(buff));
                true
            }
        }
    }

    pub fn remove_buff(&mut self, buff_id: BuffID, buff_type: Option<BuffType>) -> bool {
        if let Some(stack) = self.buff_stacks.get_mut(&buff_id) {
            stack.remove_stacks(buff_type);
            true
        } else {
            false
        }
    }

    pub fn tick(&mut self, target: EntityID, effects: &mut Vec<BuffEffect>) -> Vec<BuffUpdate> {
        let mut updates = Vec::new();
        for (buff_id, buff_stack) in self.buff_stacks.iter_mut() {
            buff_stack.tick(*buff_id, target, &mut updates, effects);
        }

        self.buff_stacks.retain(|_, buff_stack| !buff_stack.remove);
        updates
    }

    pub fn has_buff(&self, buff_id: BuffID, buff_type: Option<BuffType>) -> bool {
        match buff_type {
            Some(buff_type) => self.buff_stacks.values().any(|s| s.has_stack(buff_type)),
            None => self.buff_stacks.contains_key(&buff_id),
        }
    }

    /// Forces every stack of a buff to expire after `duration` from now.
    /// E.G.G.s carry their own duration, which overrides the skill table's.
    pub fn set_buff_duration(&mut self, buff_id: BuffID, duration: Duration) -> bool {
        let Some(stack) = self.buff_stacks.get_mut(&buff_id) else {
            return false;
        };
        let expires = Instant::now() + duration;
        for buff in &mut stack.buffs {
            buff.expires = Some(expires);
        }
        stack.changed = true;
        true
    }

    pub fn get_buff_value(&self, buff_id: BuffID) -> Option<i32> {
        self.buff_stacks
            .get(&buff_id)
            .map(|stack| stack.get_max_value())
    }

    pub fn get_bit_flags(&self) -> i32 {
        let mut flags = 0;
        for (buff_id, buff_stack) in &self.buff_stacks {
            if buff_stack.applied {
                flags |= 1 << (*buff_id as i32 - 1);
            }
        }
        flags
    }
}

struct BasicAttack {
    power: i32,
    crit_chance: Option<f32>,
    attack_style: Option<CombatStyle>,
    charged: bool,
}

/// Optional tweaks to a basic attack. A plain melee/gun swing uses the
/// default; rockets and grenades use it to carry their own damage numbers and
/// to make the client render an explosion instead of a swing.
#[derive(Default, Clone, Copy)]
pub struct AttackContext {
    /// Overrides the attacker's own (single-target, multi-target) power.
    pub power: Option<(i32, i32)>,
    /// Bullet slot and descriptor, when this attack is a projectile detonating.
    pub projectile: Option<(i8, sPCBullet)>,
}

/// Everything a skill handler needs to know about whoever cast it. Snapshotted
/// up front because the caster and the target can be the same entity, and we
/// can only hold one mutable borrow of the entity map at a time.
#[derive(Debug, Clone, Copy)]
struct CasterInfo {
    id: EntityID,
    char_type: CharType,
    level: i16,
    max_hp: i32,
    /// Where a Recall skill sends people. Only players have one.
    recall: Option<(InstanceID, Position)>,
}

struct SkillCast {
    skill: &'static Skill,
    level: usize,
    caster: CasterInfo,
}

/// A skill effect that lands back on the caster rather than the target.
#[derive(Debug)]
enum CasterEffect {
    Heal(i32),
    Warp(InstanceID, Position),
}

pub fn do_basic_attack(
    attacker_id: EntityID,
    target_ids: &[EntityID],
    charged: bool,
    ctx: AttackContext,
    state: &mut ShardServerState,
) -> FFResult<()> {
    const CRIT_CHANCE: f32 = 0.05;

    if let EntityID::Player(pc_id) = attacker_id {
        let player = state.get_player_mut(pc_id)?;
        if let Some(eid) = target_ids.first() {
            // last_attacked_by is used by scripts as an indicator
            // of who the player is in combat with
            player.target = Some(*eid);
        }
    }

    let attacker = state.get_combatant(attacker_id)?;
    let mut attacker_client = attacker.get_client();

    let (single_power, multi_power) = ctx
        .power
        .unwrap_or((attacker.get_single_power(), attacker.get_multi_power()));
    let power = if target_ids.len() == 1 {
        single_power
    } else {
        multi_power
    };
    let basic_attack = BasicAttack {
        power,
        crit_chance: Some(CRIT_CHANCE),
        attack_style: attacker.get_style(),
        charged,
    };
    // OpenFusion Combat::npcAttackPc: a mob hits a player with its table power
    // alone, no weapon boost, no nano-style RPS and no difficulty term. A flat
    // power bonus overwhelms low-level players (level-2 power 198 became 648).
    let npc_vs_player_attack = match attacker_id {
        EntityID::NPC(npc_id) => Some(BasicAttack {
            power: state.get_npc(npc_id)?.get_table_power(),
            crit_chance: Some(CRIT_CHANCE),
            attack_style: None,
            charged: false,
        }),
        _ => None,
    };

    let mut pc_attack_results = Vec::new();
    let mut npc_attack_results = Vec::new();
    for target_id in target_ids {
        let target = match state.get_combatant_mut(*target_id) {
            Ok(target) => target,
            Err(e) => {
                log_error(e);
                continue;
            }
        };
        if target.is_dead() {
            log(
                Severity::Warning,
                &format!(
                    "{:?} tried to attack dead target {:?}",
                    attacker_id, target_id
                ),
            );
            continue;
        }
        let result = match (&npc_vs_player_attack, target_id) {
            (Some(attack), EntityID::Player(_)) => {
                handle_basic_attack(attacker_id, target, attack, Some(0))
            }
            _ => handle_basic_attack(attacker_id, target, &basic_attack, None),
        };
        match target_id {
            EntityID::Player(_) => pc_attack_results.push(result),
            EntityID::NPC(_) => npc_attack_results.push(result),
            _ => unreachable!(),
        }
    }

    let pc_count = pc_attack_results.len();
    let npc_count = npc_attack_results.len();
    if pc_count == 0 && npc_count == 0 {
        return Ok(());
    }

    let battery_w = if let EntityID::Player(pc_id) = attacker_id {
        Some(state.get_player(pc_id).unwrap().get_weapon_boosts() as i32)
    } else {
        None
    };

    // PC targets
    let pc_bcast = if pc_count > 0 {
        // attacker response (PC attackers only)
        if let (EntityID::Player(_), Some(bw), Some(client)) =
            (attacker_id, battery_w, attacker_client.as_mut())
        {
            let mut resp = PacketBuilder::new(P_FE2CL_PC_ATTACK_CHARs_SUCC).with(
                &sP_FE2CL_PC_ATTACK_CHARs_SUCC {
                    iBatteryW: bw,
                    iTargetCnt: pc_count as i32,
                },
            );

            for r in &pc_attack_results {
                resp.push(r);
            }

            if let Some(resp) = log_if_failed(resp.build()) {
                client.send_payload(resp)
            }
        }

        // broadcast
        let mut bcast = match attacker_id {
            EntityID::Player(pc_id) => {
                PacketBuilder::new(P_FE2CL_PC_ATTACK_CHARs).with(&sP_FE2CL_PC_ATTACK_CHARs {
                    iPC_ID: pc_id,
                    iTargetCnt: pc_count as i32,
                })
            }
            EntityID::NPC(npc_id) => {
                PacketBuilder::new(P_FE2CL_NPC_ATTACK_PCs).with(&sP_FE2CL_NPC_ATTACK_PCs {
                    iNPC_ID: npc_id,
                    iPCCnt: pc_count as i32,
                })
            }
            _ => unreachable!(),
        };

        for r in &pc_attack_results {
            bcast.push(r);
        }

        Some(bcast.build()?)
    } else {
        None
    };

    // NPC targets
    let npc_bcast = if npc_count > 0 {
        if let (EntityID::Player(pc_id), Some((bullet_id, bullet))) = (attacker_id, ctx.projectile)
        {
            // A detonating projectile uses one packet for both the shooter and
            // the onlookers. The client's rocket-hit packet doesn't render
            // properly, so grenade-hit is used for both kinds, same as
            // OpenFusion.
            let mut pkt = PacketBuilder::new(P_FE2CL_PC_GRENADE_STYLE_HIT).with(
                &sP_FE2CL_PC_GRENADE_STYLE_HIT {
                    iPC_ID: pc_id,
                    iBulletID: bullet_id,
                    Bullet: bullet,
                    iTargetCnt: npc_count as i32,
                },
            );
            for r in &npc_attack_results {
                pkt.push(r);
            }
            let pkt = pkt.build()?;
            if let Some(client) = attacker_client.as_mut() {
                client.send_payload(pkt.clone());
            }
            Some(pkt)
        } else {
            // attacker response (PC attackers only)
            if let (EntityID::Player(_), Some(bw), Some(client)) =
                (attacker_id, battery_w, attacker_client.as_mut())
            {
                let mut resp = PacketBuilder::new(P_FE2CL_PC_ATTACK_NPCs_SUCC).with(
                    &sP_FE2CL_PC_ATTACK_NPCs_SUCC {
                        iBatteryW: bw,
                        iNPCCnt: npc_count as i32,
                    },
                );

                for r in &npc_attack_results {
                    resp.push(r);
                }

                if let Some(resp) = log_if_failed(resp.build()) {
                    client.send_payload(resp);
                }
            }

            // broadcast
            let mut bcast = match attacker_id {
                EntityID::Player(pc_id) => {
                    PacketBuilder::new(P_FE2CL_PC_ATTACK_NPCs).with(&sP_FE2CL_PC_ATTACK_NPCs {
                        iPC_ID: pc_id,
                        iNPCCnt: npc_count as i32,
                    })
                }
                EntityID::NPC(npc_id) => {
                    PacketBuilder::new(P_FE2CL_NPC_ATTACK_CHARs).with(&sP_FE2CL_NPC_ATTACK_CHARs {
                        iNPC_ID: npc_id,
                        iTargetCnt: npc_count as i32,
                    })
                }
                _ => unreachable!(),
            };

            for r in &npc_attack_results {
                bcast.push(r);
            }

            Some(bcast.build()?)
        }
    } else {
        None
    };

    state.entity_map.for_each_around(attacker_id, |c| {
        if let Some(pkt) = &pc_bcast {
            c.send_payload(pkt.clone());
        }
        if let Some(pkt) = &npc_bcast {
            c.send_payload(pkt.clone());
        }
    });

    Ok(())
}

pub fn do_skill(
    caster_id: EntityID,
    target_ids: &[EntityID],
    skill: &'static Skill,
    level: usize,
    state: &mut ShardServerState,
) -> FFResult<Vec<SkillResult>> {
    if level > SKILL_LEVEL_MAX {
        return Err(FFError::build(
            Severity::Warning,
            format!("Skill level {} is above max of {}", level, SKILL_LEVEL_MAX),
        ));
    }

    let caster = state.get_combatant(caster_id)?;

    log(
        Severity::Debug,
        &format!(
            "{} used skill {:?} on targets {:?}",
            caster, skill.skill_type, target_ids
        ),
    );

    let recall = caster
        .as_any()
        .downcast_ref::<crate::entity::Player>()
        .map(|player| {
            let pre_warp = player.get_pre_warp();
            (pre_warp.instance_id, pre_warp.position)
        });

    let skill_cast = SkillCast {
        skill,
        level,
        caster: CasterInfo {
            id: caster_id,
            char_type: caster.get_char_type(),
            level: caster.get_level(),
            max_hp: caster.get_max_hp(),
            recall,
        },
    };

    let mut skill_results = Vec::new();
    let mut caster_effects = Vec::new();
    for target_id in target_ids {
        let target = match state.get_combatant_mut(*target_id) {
            Ok(target) => target,
            Err(e) => {
                log_error(e);
                continue;
            }
        };

        // Group resurrection is the one skill that's supposed to hit a dead
        // target. (Plain Phoenix is a buff that revives its holder later.)
        let revives = matches!(skill.skill_type, SkillType::PhoenixGroup);
        if target.is_dead() && !revives {
            log(
                Severity::Warning,
                &format!(
                    "{:?} tried to use skill {:?} on dead target {:?}",
                    caster_id, skill.skill_type, target_id
                ),
            );
            continue;
        }

        match handle_skill_cast(target, &skill_cast) {
            Ok((skill_result, caster_effect)) => {
                if let Some(skill_result) = skill_result {
                    skill_results.push(skill_result);
                }
                if let Some(caster_effect) = caster_effect {
                    caster_effects.push(caster_effect);
                }
            }
            Err(e) => log_error(e),
        }
    }

    // effects that land back on the caster, applied once the target loop is
    // done so we never need two mutable borrows at once
    for effect in caster_effects {
        match effect {
            CasterEffect::Heal(amount) => {
                if let Ok(caster) = state.get_combatant_mut(caster_id) {
                    caster.heal(amount);
                }
            }
            CasterEffect::Warp(instance_id, pos) => {
                if let EntityID::Player(pc_id) = caster_id {
                    if let Ok(player) = state.get_player_mut(pc_id) {
                        player.set_position(pos);
                        player.set_instance_id(instance_id);
                        let chunk = player.get_chunk_coords();
                        let client = player.get_client();
                        state.entity_map.update(caster_id, Some(chunk), true);
                        if let Some(client) = client {
                            let pkt = sP_FE2CL_REP_PC_GOTO_SUCC {
                                iX: pos.x,
                                iY: pos.y,
                                iZ: pos.z,
                            };
                            client.send_packet(P_FE2CL_REP_PC_GOTO_SUCC, &pkt);
                        }
                    }
                }
            }
        }
    }

    Ok(skill_results)
}

/// Applies a single skill's buff to one combatant, outside of the nano skill
/// pipeline. Used by gumballs, E.G.G.s and anything else that hands out a
/// timed effect without a caster/target relationship.
pub fn apply_skill_buff(
    target: &mut dyn Combatant,
    skill: &'static Skill,
    level: usize,
    buff_type: BuffType,
    buff_id_override: Option<BuffID>,
    source: Option<EntityID>,
) -> FFResult<BuffID> {
    let buff_id = buff_id_override
        .or_else(|| skill.get_buff_id())
        .ok_or_else(|| {
            FFError::build(
                Severity::Warning,
                format!("Skill type {:?} has no associated buff", skill.skill_type),
            )
        })?;
    let buff = skill.make_buff_instance_forced(buff_type, level)?;
    target.apply_buff(buff_id, buff, source);
    Ok(buff_id)
}

/// Random inputs to one damage roll, split out so tests can pin them.
#[derive(Debug, Clone, Copy)]
struct DamageRolls {
    /// Variance in whole percent, 0..40 (OpenFusion `Rand::rand(40)`).
    variance: i32,
    crit: bool,
}

impl DamageRolls {
    fn roll(crit_chance: Option<f32>) -> Self {
        let mut rng = rand::thread_rng();
        Self {
            variance: rng.gen_range(0..40),
            crit: crit_chance.is_some_and(|chance| rng.gen::<f32>() < chance),
        }
    }
}

/// OpenFusion `getDamage`, 1:1. `difficulty` is the level term that eats into
/// damage when defense outweighs power.
fn calculate_damage(
    attack: &BasicAttack,
    defense: i32,
    defense_style: Option<CombatStyle>,
    difficulty: i16,
    rolls: DamageRolls,
) -> (i32, bool) {
    let BasicAttack {
        power: attack,
        attack_style,
        charged,
        ..
    } = attack;

    // base damage + variability
    if attack + defense == 0 {
        // divide-by-0 check
        return (0, false);
    }
    let mut damage = attack * attack / (attack + defense);
    damage = std::cmp::max(
        10 + attack / 10,
        damage - (defense - attack / 6) * difficulty as i32 / 100,
    );
    damage = damage * (rolls.variance + 80) / 100;

    // rock-paper-scissors
    let rps = do_rps(attack_style, &defense_style);
    match rps {
        RpsResult::Win => {
            damage = damage * 5 / 4;
        }
        RpsResult::Lose => {
            damage = damage * 4 / 5;
        }
        RpsResult::Draw => {}
    };

    // boost
    if *charged {
        damage = damage * 5 / 4;
    }

    // crit
    if rolls.crit {
        damage *= 2;
    }

    (damage, rolls.crit)
}

/// `difficulty` overrides the default level term (the target's level).
fn handle_basic_attack(
    from: EntityID,
    to: &mut dyn Combatant,
    attack: &BasicAttack,
    difficulty: Option<i16>,
) -> sAttackResult {
    let defense = to.get_defense();
    let defense_style = to.get_style();
    let difficulty = difficulty.unwrap_or_else(|| to.get_level());
    let rolls = DamageRolls::roll(attack.crit_chance);
    let (damage, crit) = calculate_damage(attack, defense, defense_style, difficulty, rolls);
    let dealt = to.take_damage(damage, Some(from));

    let mut hit_flag = HF_BIT_NORMAL as i8;
    if crit {
        hit_flag |= HF_BIT_CRITICAL as i8;
    }

    sAttackResult {
        eCT: to.get_char_type() as i32,
        iID: match to.get_id() {
            EntityID::Player(id) => id,
            EntityID::NPC(id) => id,
            _ => unreachable!(),
        },
        bProtected: unused!(),
        iDamage: dealt,
        iHP: to.get_hp(),
        iHitFlag: hit_flag,
    }
}

fn handle_skill_cast(
    to: &mut dyn Combatant,
    cast: &SkillCast,
) -> FFResult<(Option<SkillResult>, Option<CasterEffect>)> {
    if cast.skill.passive {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "Passive skill {:?} was cast, should not happen",
                cast.skill.skill_type
            ),
        ));
    }

    let from = cast.caster.id;
    let level = cast.level;
    let value_a = cast.skill.values_a[level];
    let target_id = entity_id_to_raw(to.get_id());
    let target_ct = to.get_char_type() as i32;

    let result = match cast.skill.skill_type {
        // Straight damage. The scaling factor is taken from OpenFusion so that
        // nano damage lands in the same ballpark on both servers.
        SkillType::Damage
        | SkillType::CorruptionAttack
        | SkillType::CorruptionAttackWin
        | SkillType::CorruptionAttackLose => {
            let scaling = if cast.caster.char_type == CharType::Player {
                cast.caster.max_hp.max(to.get_max_hp()) as f32 / 1000.0
            } else {
                cast.caster.max_hp as f32 / 1500.0
            };
            let damage = (value_a as f32 * scaling) as i32;
            let dealt = to.take_damage(damage, Some(from));

            (
                Some(SkillResult::Damage(sSkillResult_Damage {
                    eCT: target_ct,
                    iID: target_id,
                    bProtected: (dealt <= 0) as i32,
                    iDamage: dealt,
                    iHP: to.get_hp(),
                })),
                None,
            )
        }

        SkillType::HealHP | SkillType::ReturnHomeHeal => {
            let heal = (value_a as f32 * to.get_max_hp() as f32 / 1000.0) as i32;
            let healed = to.heal(heal);

            (
                Some(SkillResult::HealHP(sSkillResult_Heal_HP {
                    eCT: target_ct,
                    iID: target_id,
                    iHealHP: healed,
                    iHP: to.get_hp(),
                })),
                None,
            )
        }

        // Crowd control. Freedom blocks all of it.
        SkillType::KnockDown | SkillType::Sleep | SkillType::Snare | SkillType::Stun => {
            // zero damage, but enough to take aggro
            to.take_damage(0, Some(from));

            let blocked = to.has_buff(BuffID::Freedom, None);
            let mut duration_ms = 0;
            if !blocked {
                if let Some(buff_id) = cast.skill.get_buff_id() {
                    let buff = cast.skill.make_buff_instance(BuffType::Nano, level)?;
                    duration_ms = cast.skill.durations[level].map_or(0, |d| d.as_millis() as i32);
                    to.apply_buff(buff_id, buff, Some(from));
                }
            }

            (
                Some(SkillResult::DamageAndDebuff(sSkillResult_Damage_N_Debuff {
                    eCT: target_ct,
                    iID: target_id,
                    bProtected: blocked as i32,
                    // the client renders the debuff duration in this field
                    // rather than an actual damage number
                    iDamage: duration_ms / 1000,
                    iHP: to.get_hp(),
                    iStamina: get_nano_stamina(to),
                    bNanoDeactive: unused!(),
                    iConditionBitFlag: to.get_condition_bit_flag(),
                })),
                None,
            )
        }

        // Drain the target's HP and give a portion back to the caster.
        SkillType::BloodSucking => {
            let heal = value_a;
            let damage = heal * 2;
            let dealt = to.take_damage(damage, Some(from));

            (
                Some(SkillResult::Damage(sSkillResult_Damage {
                    eCT: target_ct,
                    iID: target_id,
                    bProtected: (dealt <= 0) as i32,
                    iDamage: dealt,
                    iHP: to.get_hp(),
                })),
                Some(CasterEffect::Heal(heal)),
            )
        }

        // Battery drain only means anything against players.
        SkillType::BatteryDrain => {
            let Some(player) = to.as_any_mut().downcast_mut::<crate::entity::Player>() else {
                return Ok((None, None));
            };

            let scaling = (18 + cast.caster.level as i32) as f32 / 36.0;
            let blocked = player.has_buff(BuffID::ProtectBattery, None);
            let (drain_w, drain_n) = if blocked {
                (0, 0)
            } else {
                let boosts = player.get_weapon_boosts();
                let potions = player.get_nano_potions();
                let drain_w = ((value_a as f32 * scaling) as u32).min(boosts);
                let drain_n = ((cast.skill.values_b[level].unwrap_or(0) as f32 * scaling) as u32)
                    .min(potions);
                player.set_weapon_boosts(boosts - drain_w);
                player.set_nano_potions(potions - drain_n);
                (drain_w, drain_n)
            };

            let stamina = player.get_active_nano().map_or(0, |n| n.get_stamina());
            (
                Some(SkillResult::BatteryDrain(sSkillResult_BatteryDrain {
                    eCT: target_ct,
                    iID: target_id,
                    bProtected: blocked as i32,
                    iDrainW: drain_w as i32,
                    iBatteryW: player.get_weapon_boosts() as i32,
                    iDrainN: drain_n as i32,
                    iBatteryN: player.get_nano_potions() as i32,
                    iStamina: stamina,
                    bNanoDeactive: (stamina <= 0) as i32,
                    iConditionBitFlag: player.get_condition_bit_flag(),
                })),
                None,
            )
        }

        // Group recall: everyone gets sent to the caster's recall point.
        SkillType::Recall | SkillType::RecallGroup => {
            let Some((instance_id, pos)) = cast.caster.recall else {
                return Ok((None, None));
            };

            if to.get_id() == from {
                // the client expects no trailing struct for the caster;
                // we warp them directly instead
                return Ok((None, Some(CasterEffect::Warp(instance_id, pos))));
            }

            (
                Some(SkillResult::Move(sSkillResult_Move {
                    eCT: target_ct,
                    iID: target_id,
                    iMapNum: instance_id.map_num as i32,
                    iMoveX: pos.x,
                    iMoveY: pos.y,
                    iMoveZ: pos.z,
                })),
                None,
            )
        }

        SkillType::PhoenixGroup => {
            // The client performs the actual revive; we just confirm it and
            // report the HP it should come back with.
            (
                Some(SkillResult::Resurrect(sSkillResult_Resurrect {
                    eCT: target_ct,
                    iID: target_id,
                    iRegenHP: to.get_hp(),
                })),
                None,
            )
        }

        SkillType::HealStamina => {
            let healed = heal_nano_stamina(to, value_a as i16);
            (
                Some(SkillResult::HealStamina(sSkillResult_Heal_Stamina {
                    eCT: target_ct,
                    iID: target_id,
                    iHealNanoStamina: healed,
                    Nano: get_nano_proto(to),
                })),
                None,
            )
        }

        // Trade HP for nano stamina.
        SkillType::StaminaSelf => {
            let hp_cost = (to.get_max_hp() * value_a) / 1000;
            let taken = to.take_damage(hp_cost, None);
            let healed =
                heal_nano_stamina(to, cast.skill.values_b[level].unwrap_or(value_a) as i16);
            (
                Some(SkillResult::StaminaSelf(sSkillResult_Stamina_Self {
                    eCT: target_ct,
                    iID: target_id,
                    iReduceHP: taken,
                    iHP: to.get_hp(),
                    iHealNanoStamina: healed,
                    Nano: get_nano_proto(to),
                })),
                None,
            )
        }

        // Retro rockets are fired by the weapon system, not here.
        SkillType::RetroRocketSelf => (None, None),

        other => {
            // everything else is a plain (de)buff: set the condition flag and
            // let the client apply its own effect
            let Some(buff_id) = cast.skill.get_buff_id() else {
                return Err(FFError::build(
                    Severity::Warning,
                    format!("Skill type {:?} is not implemented", other),
                ));
            };

            let applied = if to.has_buff(BuffID::Invulnerable, None) && is_debuff(other) {
                false
            } else {
                let buff_type = if to.get_id() == from {
                    BuffType::Nano
                } else {
                    BuffType::GroupNano
                };
                let buff = cast.skill.make_buff_instance(buff_type, level)?;
                to.apply_buff(buff_id, buff, Some(from));
                true
            };

            (
                Some(SkillResult::Buff(sSkillResult_Buff {
                    eCT: target_ct,
                    iID: target_id,
                    bProtected: !applied as i32,
                    iConditionBitFlag: to.get_condition_bit_flag(),
                })),
                None,
            )
        }
    };

    Ok(result)
}

/// True for skill types that hurt the target, which Invulnerable blocks.
fn is_debuff(skill_type: SkillType) -> bool {
    matches!(
        skill_type,
        SkillType::Snare
            | SkillType::Sleep
            | SkillType::Stun
            | SkillType::KnockDown
            | SkillType::KnockBack
            | SkillType::InfectionDamage
            | SkillType::WeaponSlow
            | SkillType::BoundingBall
    )
}

fn entity_id_to_raw(id: EntityID) -> i32 {
    match id {
        EntityID::Player(id) | EntityID::NPC(id) | EntityID::Slider(id) | EntityID::Egg(id) => id,
    }
}

fn get_nano_stamina(target: &dyn Combatant) -> i16 {
    target
        .as_any()
        .downcast_ref::<crate::entity::Player>()
        .and_then(|p| p.get_active_nano())
        .map_or(0, |n| n.get_stamina())
}

fn get_nano_proto(target: &dyn Combatant) -> sNano {
    target
        .as_any()
        .downcast_ref::<crate::entity::Player>()
        .and_then(|p| p.get_active_nano())
        .into_proto()
}

fn heal_nano_stamina(target: &mut dyn Combatant, amount: i16) -> i16 {
    let Some(player) = target.as_any_mut().downcast_mut::<crate::entity::Player>() else {
        return 0;
    };
    let Some(nano) = player.get_active_nano_mut() else {
        return 0;
    };
    let before = nano.get_stamina();
    nano.set_stamina(before + amount);
    nano.get_stamina() - before
}

enum RpsResult {
    Win,
    Lose,
    Draw,
}
fn do_rps(us: &Option<CombatStyle>, them: &Option<CombatStyle>) -> RpsResult {
    if us.is_none() || them.is_none() {
        return RpsResult::Draw;
    }

    let us = us.as_ref().unwrap();
    let them = them.as_ref().unwrap();
    match us {
        CombatStyle::Adaptium => match them {
            CombatStyle::Blastons => RpsResult::Win,
            CombatStyle::Cosmix => RpsResult::Lose,
            _ => RpsResult::Draw,
        },

        CombatStyle::Blastons => match them {
            CombatStyle::Cosmix => RpsResult::Win,
            CombatStyle::Adaptium => RpsResult::Lose,
            _ => RpsResult::Draw,
        },

        CombatStyle::Cosmix => match them {
            CombatStyle::Adaptium => RpsResult::Win,
            CombatStyle::Blastons => RpsResult::Lose,
            _ => RpsResult::Draw,
        },
    }
}

#[cfg(test)]
#[path = "skills_damage_tests.rs"]
mod damage_tests;
