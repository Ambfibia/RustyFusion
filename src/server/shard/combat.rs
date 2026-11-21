use std::time::{Duration, SystemTime};

use crate::{
    defines::*,
    entity::{Combatant, Entity, EntityID, Projectile, ProjectileKind},
    enums::{BuffID, CombatantTeam, SkillTargetType, SkillType, TargetType, WeaponTargetMode},
    error::*,
    net::{
        packet::{PacketID::*, *},
        ClientMap,
    },
    skills::{self, AttackContext, Skill, SkillResult},
    state::ShardServerState,
    Position,
};

#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[repr(packed(4))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct sTargetNpcId {
    pub iNPC_ID: i32,
}
impl FFPacket for sTargetNpcId {}

#[allow(non_camel_case_types, non_snake_case)]
#[repr(C, packed(4))]
#[derive(Debug, Copy, Clone)]
struct sTargetPcId { iPC_ID: i32 }
impl FFPacket for sTargetPcId {}

const MAX_TARGETS: usize = 3;

const BATTERY_BASE_COST: u32 = 6;

fn invalid_weapon(message: &str) -> FFError { FFError::build(Severity::Warning, message.into()) }

fn distance(a: Position, b: Position) -> f64 {
    let d = [f64::from(a.x) - f64::from(b.x), f64::from(a.y) - f64::from(b.y), f64::from(a.z) - f64::from(b.z)];
    (d[0]*d[0] + d[1]*d[1] + d[2]*d[2]).sqrt()
}

fn weapon_rules(player: &crate::entity::Player) -> FFResult<(Option<WeaponTargetMode>, u32, usize, Duration)> {
    if let Some(item) = player.get_equipped()[EQUIP_SLOT_HAND as usize] {
        let stats = item.get_stats()?;
        Ok((stats.target_mode, stats.attack_range, stats.target_count, stats.attack_delay))
    } else { Ok((Some(WeaponTargetMode::Hand), 200, 1, Duration::from_millis(1000))) }
}

fn weapon_available(player: &crate::entity::Player, delay: Duration, now: SystemTime) -> FFResult<()> {
    if player.is_dead() || player.vehicle_speed.is_some() || player.skyway_ride.is_some()
        || [BuffID::Stun, BuffID::Sleep, BuffID::KnockDown].iter().any(|id| player.has_buff(*id, None))
        || player.last_weapon_attack.is_some_and(|at| now.duration_since(at).unwrap_or_default() < delay) {
        return Err(invalid_weapon("Weapon is not available"));
    }
    Ok(())
}

fn validate_weapon_targets(pc_id: i32, targets: &[EntityID], range: u32, count: usize, state: &ShardServerState) -> FFResult<u32> {
    let player = state.get_player(pc_id)?;
    if targets.is_empty() || targets.len() > count || targets.len() > MAX_PROJECTILE_TARGETS {
        return Err(invalid_weapon("Invalid weapon target count"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut cost = 0;
    for id in targets {
        let target = state.get_combatant(*id)?;
        if !seen.insert(*id) || *id == EntityID::Player(pc_id) || target.is_dead()
            || target.get_chunk_coords().i != player.instance_id || !target.visible_to(player) {
            return Err(invalid_weapon("Invalid weapon target"));
        }
        let extent = match id {
            EntityID::NPC(npc_id) => {
                if target.get_team() != CombatantTeam::Mob { return Err(invalid_weapon("Friendly NPC cannot be attacked")); }
                let stats = crate::tabledata::tdata_get().get_npc_stats(state.get_npc(*npc_id)?.ty)?;
                f64::from(stats.radius).max(f64::from(stats.height)*0.5)
            },
            EntityID::Player(_) => 100.0,
            _ => return Err(invalid_weapon("Unsupported weapon target")),
        };
        if distance(player.get_position(), target.get_position()) > f64::from(range) + extent {
            return Err(invalid_weapon("Weapon target is out of range"));
        }
        cost += BATTERY_BASE_COST + target.get_level().max(0) as u32;
    }
    Ok(cost)
}

pub fn pc_attack_npcs(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let pc_id = clients.get_sender().get_player_id()?;
    let request = *pkt.get::<sP_CL2FE_REQ_PC_ATTACK_NPCs>()?;
    let count = request.iNPCCnt as usize;
    if count == 0 || count > MAX_PROJECTILE_TARGETS || pkt.read_bytes().len() != 8 + count*4 {
        return Err(invalid_weapon("Malformed NPC attack targets"));
    }
    let mut reader = PacketReader::new(&pkt); reader.get_struct::<sP_CL2FE_REQ_PC_ATTACK_NPCs>()?;
    let mut targets = Vec::with_capacity(count);
    for _ in 0..count { targets.push(EntityID::NPC(reader.get_struct::<sTargetNpcId>()?.iNPC_ID)); }
    commit_weapon_attack(pc_id, &targets, false, state)
}

pub fn pc_attack_pcs(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let pc_id = clients.get_sender().get_player_id()?;
    // This native packet is the GM combat variant in local OpenFusion, not an
    // entitlement for ordinary accounts to damage other players.
    if state.get_player(pc_id)?.perms as u32 > CN_ACCOUNT_LEVEL__GM { return Err(invalid_weapon("GM character attack permission required")); }
    let request = *pkt.get::<sP_CL2FE_REQ_PC_ATTACK_CHARs>()?;
    let count = request.iTargetCnt as usize;
    if count == 0 || count > MAX_PROJECTILE_TARGETS || pkt.read_bytes().len() != 8 + count*8 {
        return Err(invalid_weapon("Malformed character attack targets"));
    }
    let mut reader = PacketReader::new(&pkt); reader.get_struct::<sP_CL2FE_REQ_PC_ATTACK_CHARs>()?;
    let mut targets = Vec::with_capacity(count);
    for _ in 0..count {
        let target = *reader.get_struct::<sCharacterTarget>()?;
        targets.push(match target.kind { 1 => EntityID::Player(target.id), 4 => EntityID::NPC(target.id), _ => return Err(invalid_weapon("Invalid character attack namespace")) });
    }
    commit_weapon_attack(pc_id, &targets, true, state)
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct sCharacterTarget { id: i32, kind: i32 }
impl FFPacket for sCharacterTarget {}

fn commit_weapon_attack(pc_id: i32, targets: &[EntityID], chars: bool, state: &mut ShardServerState) -> FFResult<()> {
    let player = state.get_player(pc_id)?;
    let (mode, range, count, delay) = weapon_rules(player)?;
    if matches!(mode, Some(WeaponTargetMode::Rocket | WeaponTargetMode::Grenade)) { return Err(invalid_weapon("Projectile weapon requires fire/hit transaction")); }
    let now = SystemTime::now(); weapon_available(player, delay, now)?;
    let cost = validate_weapon_targets(pc_id, targets, range, count, state)?;
    let player = state.get_player_mut(pc_id)?;
    let charged = player.consume_weapon_boosts(cost); player.last_weapon_attack = Some(now);
    skills::do_basic_attack(EntityID::Player(pc_id), targets, charged, AttackContext { char_targets: chars, ..Default::default() }, state)
}

pub fn nano_skill_use(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;
    let Some(nano) = player.get_active_nano() else {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "{} tried to use a nano skill without an active nano",
                player
            ),
        ));
    };

    // 0 normally, top tier when the nano is boosted by a gumball
    let skill_level = player.get_nano_skill_level();

    let Some(skill) = nano.get_skill() else {
        return Err(FFError::build(
            Severity::Warning,
            format!("{} tried to use a skill from a nano with no skill", player),
        ));
    };

    let skill_cost = skill.costs[skill_level];
    if player.is_dead() || player.skyway_ride.is_some() || player.vehicle_speed.is_some()
        || skill.passive || !nano.skill_is_ready()
        || [BuffID::Stun, BuffID::Sleep, BuffID::KnockDown].iter().any(|id| player.has_buff(*id, None)) {
        return Err(FFError::build(
            Severity::Warning,
            "Nano skill is not available".to_owned(),
        ));
    }
    let nano_stamina = nano.get_stamina();
    if nano_stamina < skill_cost {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "{} tried to use a nano skill without enough stamina ({} needed, {} available)",
                player, skill_cost, nano_stamina
            ),
        ));
    }

    let group_id = player.get_group_id();
    let caster_instance = player.instance_id;
    let request_len = pkt.read_bytes().len();

    let mut reader = PacketReader::new(&pkt);
    let pkt: &sP_CL2FE_REQ_NANO_SKILL_USE = reader.get_struct()?;
    let pkt_copy = *pkt;
    let target_count = pkt.iTargetCnt as usize;
    // Reject the complete frame before effects, including truncated/trailing tails.
    if target_count > MAX_TARGETS
        || request_len != 4 + size_of::<sP_CL2FE_REQ_NANO_SKILL_USE>() + target_count * 4
    {
        return Err(FFError::build(
            Severity::Warning,
            "Invalid Nano skill target count".to_owned(),
        ));
    }

    // The accepted single-Recall rows use m_iEffectTarget=Self even where
    // m_iTargetType says NPC. Like OpenFusion's matchTargets, resolve self
    // before reading ordinary targets; the original client sends none.
    if skill.skill_type == SkillType::Recall && skill.targeting_type == SkillTargetType::SelfTarget {
        return finish_nano_skill(&pkt_copy, &[EntityID::Player(pc_id)], skill, skill_level, clients, state);
    }

    // Group skills (group heal, group recall, group phoenix) come in with no
    // targets at all: the client expects the server to resolve the group.
    if skill.target_type == TargetType::CasterPC {
        let mut target_ids = vec![EntityID::Player(pc_id)];
        if let Some(group) = group_id.and_then(|group_id| state.groups.get(&group_id)) {
            target_ids = group
                .get_member_ids()
                .iter()
                .copied()
                .filter(|id| match id {
                    EntityID::Player(id) => state.get_player(*id).is_ok_and(|member| member.instance_id == caster_instance),
                    _ => false,
                })
                .collect();
        }
        return finish_nano_skill(&pkt_copy, &target_ids, skill, skill_level, clients, state);
    }

    if target_count == 0 {
        return Ok(());
    }

    let mut target_ids = Vec::with_capacity(MAX_TARGETS);
    for _ in 0..target_count {
        let target_id = match skill.target_type {
            TargetType::HostileNPCs => {
                let target_npc_id = reader.get_struct::<sTargetNpcId>()?.iNPC_ID;
                EntityID::NPC(target_npc_id)
            }
            TargetType::FriendlyPCs => {
                let target_pc_id = reader.get_struct::<sTargetPcId>()?.iPC_ID;
                EntityID::Player(target_pc_id)
            }
            TargetType::CasterPC => player.get_id(),
        };

        if skill.targeting_type == SkillTargetType::None {
            return Err(FFError::build(
                Severity::Warning,
                "Nano skill has no targeting type".to_owned(),
            ));
        }

        target_ids.push(target_id);
    }

    if skill.target_type == TargetType::HostileNPCs {
        validate_nano_attack_targets(player, &pkt_copy, &target_ids, skill, state)?;
    }
    finish_nano_skill(&pkt_copy, &target_ids, skill, skill_level, clients, state)
}

/// Validate the entire hostile target set before applying any damage/debuff.
/// Target/TargetArea are the accepted single and splash Nano attack branches.
fn validate_nano_attack_targets(
    player: &crate::entity::Player,
    pkt: &sP_CL2FE_REQ_NANO_SKILL_USE,
    ids: &[EntityID],
    skill: &Skill,
    state: &ShardServerState,
) -> FFResult<()> {
    let invalid = || FFError::build(Severity::Warning, "Invalid Nano attack targets".to_owned());
    if ids.is_empty() || ids.len() > skill.target_count {
        return Err(invalid());
    }
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        let target = state.get_combatant(*id)?;
        if !seen.insert(*id)
            || target.is_dead()
            || target.get_team() != CombatantTeam::Mob
            || target.get_chunk_coords().i != player.instance_id
        {
            return Err(invalid());
        }
    }
    if matches!(
        skill.targeting_type,
        SkillTargetType::Target | SkillTargetType::TargetArea
    ) {
        let focus = EntityID::NPC(pkt.iArg1);
        if !ids.contains(&focus)
            || (skill.targeting_type == SkillTargetType::Target && ids.len() != 1)
        {
            return Err(invalid());
        }
        let focus = state.get_combatant(focus)?;
        if !nano_target_in_range(player.get_position(), focus, skill.cast_range)? {
            return Err(invalid());
        }
        if skill.targeting_type == SkillTargetType::TargetArea {
            for id in ids {
                let target = state.get_combatant(*id)?;
                if !nano_target_in_range(focus.get_position(), target, skill.range)? {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}

fn nano_target_in_range(origin: Position, target: &dyn Combatant, range: u32) -> FFResult<bool> {
    let npc = target
        .as_any()
        .downcast_ref::<crate::entity::NPC>()
        .unwrap();
    let stats = crate::tabledata::tdata_get().get_npc_stats(npc.ty)?;
    // Native targeting includes the NPC body extent in protocol centiunits.
    let extent = f64::from(stats.radius).max(f64::from(stats.height) * 0.5);
    let pos = target.get_position();
    let delta = [
        f64::from(pos.x) - f64::from(origin.x),
        f64::from(pos.y) - f64::from(origin.y),
        f64::from(pos.z) - f64::from(origin.z),
    ];
    Ok(delta.into_iter().map(|v| v * v).sum::<f64>().sqrt() < f64::from(range) + extent)
}

/// Runs a nano skill against already-resolved targets and sends the
/// confirmation to the caster plus the broadcast to everyone nearby.
fn finish_nano_skill(
    pkt: &sP_CL2FE_REQ_NANO_SKILL_USE,
    target_ids: &[EntityID],
    skill: &'static Skill,
    skill_level: usize,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let skill_cost = skill.costs[skill_level];

    let self_recall = matches!(skill.skill_type, SkillType::Recall | SkillType::RecallGroup)
        && target_ids.contains(&EntityID::Player(pc_id));

    let results = skills::do_skill(
        EntityID::Player(pc_id),
        target_ids,
        skill,
        skill_level,
        state,
    )?;
    // 0104 has no Nano-use FAIL. An empty SUCC is invalid to the native client;
    // ordinary skills without an effect must not commit a resource cost.
    // A successful self Recall sends GOTO_SUCC instead of a Move trailer.
    // It still spends its authored cost and starts the cooldown once.
    // do_skill rejects a missing/expired registration before this point.
    let moves_caster = matches!(skill.skill_type, SkillType::RetroRocketSelf | SkillType::Dash)
        && target_ids.contains(&EntityID::Player(pc_id));
    if results.is_empty() && !self_recall && !moves_caster {
        return Ok(());
    }

    let nano = state
        .get_player_mut(pc_id)
        .unwrap()
        .get_active_nano_mut()
        .ok_or(FFError::build(
            Severity::Warning,
            format!("Player {} lost their nano mid-skill", pc_id),
        ))?;

    let nano_stamina = nano.get_stamina();
    nano.set_stamina(nano_stamina - skill_cost);
    nano.start_skill_cooldown(skill.cooldown);

    let target_cnt = results.len() as i32;
    let skill_id = nano.selected_skill.unwrap();
    let nano_stamina = nano.get_stamina();
    let nano_deactive = nano_stamina == 0;
    let nano_id = nano.get_id();
    let skill_type = skill.skill_type as i32;

    if results.is_empty() {
        if nano_deactive { state.get_player_mut(pc_id)?.deactivate_nano(); }
        return Ok(()); // no invalid empty NANO_SKILL_USE_SUCC
    }

    let mut succ_builder =
        PacketBuilder::new(P_FE2CL_NANO_SKILL_USE_SUCC).with(&sP_FE2CL_NANO_SKILL_USE_SUCC {
            iPC_ID: pc_id,
            iBulletID: pkt.iBulletID,
            iSkillID: skill_id,
            iArg1: pkt.iArg1,
            iArg2: pkt.iArg2,
            iArg3: pkt.iArg3,
            bNanoDeactive: nano_deactive as i32,
            iNanoID: nano_id,
            iNanoStamina: nano_stamina,
            eST: skill_type,
            iTargetCnt: target_cnt,
        });

    let mut bcast_builder =
        PacketBuilder::new(P_FE2CL_NANO_SKILL_USE).with(&sP_FE2CL_NANO_SKILL_USE {
            iPC_ID: pc_id,
            iBulletID: pkt.iBulletID,
            iSkillID: skill_id,
            iArg1: pkt.iArg1,
            iArg2: pkt.iArg2,
            iArg3: pkt.iArg3,
            bNanoDeactive: nano_deactive as i32,
            iNanoID: nano_id,
            iNanoStamina: nano_stamina,
            eST: skill_type,
            iTargetCnt: target_cnt,
        });

    for result in results {
        // These look identical, but since they have different concrete types, each arm
        // is a different push call with a different type parameter.
        match result {
            SkillResult::Damage(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::Leech(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::DotDamage(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::HealHP(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::HealStamina(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::StaminaSelf(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::DamageAndDebuff(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::Buff(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::BatteryDrain(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::DamageAndMove(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::Move(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
            SkillResult::Resurrect(sr) => {
                succ_builder.push(&sr);
                bcast_builder.push(&sr);
            }
        }
    }

    if let Some(pkt) = log_if_failed(succ_builder.build()) {
        client.send_payload(pkt);
    }

    if let Some(pkt) = log_if_failed(bcast_builder.build()) {
        state
            .entity_map
            .for_each_around(EntityID::Player(pc_id), |c| {
                c.send_payload(pkt.clone());
            });
    }

    if nano_deactive {
        state.get_player_mut(pc_id)?.deactivate_nano();
    }

    Ok(())
}

//
// Rockets and grenades.
//
// The client fires a projectile, we hand back a bullet slot, and the client
// reports back what the blast hit. The damage numbers are locked in at fire
// time so the payload can't change while the projectile is in the air, and the
// reported targets are re-checked against the blast radius before they take
// damage.
//

/// How far from the reported impact point a target can be and still be hit.
/// The client doesn't tell us the weapon's blast radius, so this is a
/// server-side sanity bound rather than an exact simulation.
/// Upper bound on how many entities one blast may be reported to have hit.
const MAX_PROJECTILE_TARGETS: usize = 32;

/// A projectile hit's trailing entry. The client sends 8 bytes per target,
/// with the entity ID in the low half.
#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[repr(packed(4))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct sProjectileTarget {
    pub iID: i32,
    pub _unused: i32,
}
impl FFPacket for sProjectileTarget {}

pub fn pc_rocket_style_ready(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    if pkt.read_bytes().len() != 4 + size_of::<sP_CL2FE_REQ_PC_ROCKET_STYLE_READY>() { return Err(invalid_weapon("Malformed rocket ready")); }
    let pkt: &sP_CL2FE_REQ_PC_ROCKET_STYLE_READY = pkt.get()?;
    let pc_id = clients.get_sender().get_player_id()?;
    let player = state.get_player(pc_id)?;
    let (mode, _, _, delay) = weapon_rules(player)?;
    if mode != Some(WeaponTargetMode::Rocket) || pkt.iSkillID != 0 { return Err(invalid_weapon("Rocket ready does not match equipped weapon")); }
    weapon_available(player, delay, SystemTime::now())?;
    let bcast = sP_FE2CL_PC_ROCKET_STYLE_READY {
        iPC_ID: pc_id,
        iSkillID: pkt.iSkillID,
    };
    state
        .entity_map
        .for_each_around(EntityID::Player(pc_id), |c| {
            c.send_packet(P_FE2CL_PC_ROCKET_STYLE_READY, &bcast);
        });
    Ok(())
}

pub fn pc_grenade_style_ready(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    if pkt.read_bytes().len() != 4 + size_of::<sP_CL2FE_REQ_PC_GRENADE_STYLE_READY>() { return Err(invalid_weapon("Malformed grenade ready")); }
    let pkt: &sP_CL2FE_REQ_PC_GRENADE_STYLE_READY = pkt.get()?;
    let pc_id = clients.get_sender().get_player_id()?;
    let player = state.get_player(pc_id)?;
    let (mode, _, _, delay) = weapon_rules(player)?;
    if mode != Some(WeaponTargetMode::Grenade) || pkt.iSkillID != 0 { return Err(invalid_weapon("Grenade ready does not match equipped weapon")); }
    weapon_available(player, delay, SystemTime::now())?;
    let bcast = sP_FE2CL_PC_GRENADE_STYLE_READY {
        iPC_ID: pc_id,
        iSkillID: pkt.iSkillID,
    };
    state
        .entity_map
        .for_each_around(EntityID::Player(pc_id), |c| {
            c.send_packet(P_FE2CL_PC_GRENADE_STYLE_READY, &bcast);
        });
    Ok(())
}

pub fn pc_rocket_style_fire(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    if pkt.read_bytes().len() != 4 + size_of::<sP_CL2FE_REQ_PC_ROCKET_STYLE_FIRE>() { return Err(invalid_weapon("Malformed rocket fire")); }
    let pkt: &sP_CL2FE_REQ_PC_ROCKET_STYLE_FIRE = pkt.get()?;
    if pkt.iSkillID != 0 { return Err(invalid_weapon("Unsupported rocket skill")); }
    let from = Position::new(pkt.iX, pkt.iY, pkt.iZ);
    let to = Position::new(pkt.iToX, pkt.iToY, pkt.iToZ);
    fire_projectile(WeaponTargetMode::Rocket, Some(from), to, clients, state)
}

pub fn pc_grenade_style_fire(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    if pkt.read_bytes().len() != 4 + size_of::<sP_CL2FE_REQ_PC_GRENADE_STYLE_FIRE>() { return Err(invalid_weapon("Malformed grenade fire")); }
    let pkt: &sP_CL2FE_REQ_PC_GRENADE_STYLE_FIRE = pkt.get()?;
    if pkt.iSkillID != 0 { return Err(invalid_weapon("Unsupported grenade skill")); }
    let to = Position::new(pkt.iToX, pkt.iToY, pkt.iToZ);
    fire_projectile(WeaponTargetMode::Grenade, None, to, clients, state)
}

fn fire_projectile(
    mode: WeaponTargetMode,
    from: Option<Position>,
    to: Position,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    let player = state.get_player(pc_id)?;
    let weapon = player.get_equipped()[EQUIP_SLOT_HAND as usize].ok_or_else(|| {
        FFError::build(
            Severity::Warning,
            format!("{} tried to fire a projectile bare-handed", player),
        )
    })?;
    let weapon_stats = weapon.get_stats()?;
    if weapon_stats.target_mode != Some(mode) {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "{} tried to fire a {:?} with a {:?} weapon",
                player, mode, weapon_stats.target_mode
            ),
        ));
    }

    let flight_time = weapon_stats.projectile_time.unwrap_or(Duration::ZERO);
    let weapon_boosts_needed = BATTERY_BASE_COST + weapon_stats.required_level.max(0) as u32;
    let now = SystemTime::now();
    weapon_available(player, weapon_stats.attack_delay, now)?;
    let start_pos = from.unwrap_or_else(|| player.get_position());
    if distance(start_pos, player.get_position()) > 250.0 || distance(player.get_position(), to) > f64::from(weapon_stats.attack_range) + 200.0 {
        return Err(invalid_weapon("Projectile launch or destination is out of weapon range"));
    }

    let player = state.get_player_mut(pc_id)?;
    player.expire_projectiles(now);
    if player.projectile_count() >= SIZEOF_PC_BULLET_SLOT as usize { return Err(invalid_weapon("No free projectile slot")); }
    let charged = player.consume_weapon_boosts(weapon_boosts_needed);

    let projectile = Projectile {
        kind: match mode {
            WeaponTargetMode::Grenade => ProjectileKind::Grenade,
            _ => ProjectileKind::Rocket(weapon.id),
        },
        single_power: player.get_single_power(),
        multi_power: player.get_multi_power(),
        charged,
        start_pos,
        end_pos: to,
        // give the client a generous grace period on top of the flight time
        // before we reclaim the bullet slot
        expires: now + flight_time + Duration::from_secs(10),
        instance_id: player.instance_id,
        effect_area: weapon_stats.effect_area,
        target_count: weapon_stats.target_count,
        travel_range: (f64::from(weapon_stats.attack_range) * flight_time.as_secs_f64()).ceil() as u32 + 250,
    };

    // stale projectiles would otherwise hold their slots forever
    player.expire_projectiles(SystemTime::now());
    let bullet_id = player.add_projectile(projectile).ok_or_else(|| {
        FFError::build(
            Severity::Warning,
            format!("Player {} has too many projectiles in flight", pc_id),
        )
    })?;
    player.last_weapon_attack = Some(now);
    let bullet: sPCBullet = projectile.into();
    let weapon_boosts = player.get_weapon_boosts() as i32;

    match mode {
        WeaponTargetMode::Grenade => {
            let resp = sP_FE2CL_REP_PC_GRENADE_STYLE_FIRE_SUCC {
                iSkillID: unused!(),
                iToX: to.x,
                iToY: to.y,
                iToZ: to.z,
                iBulletID: bullet_id,
                Bullet: bullet,
                iBatteryW: weapon_boosts,
                bNanoDeactive: unused!(),
                iNanoID: unused!(),
                iNanoStamina: unused!(),
            };
            client.send_packet(P_FE2CL_REP_PC_GRENADE_STYLE_FIRE_SUCC, &resp);

            let bcast = sP_FE2CL_PC_GRENADE_STYLE_FIRE {
                iPC_ID: pc_id,
                iToX: to.x,
                iToY: to.y,
                iToZ: to.z,
                iBulletID: bullet_id,
                Bullet: bullet,
                bNanoDeactive: unused!(),
            };
            state
                .entity_map
                .for_each_around(EntityID::Player(pc_id), |c| {
                    c.send_packet(P_FE2CL_PC_GRENADE_STYLE_FIRE, &bcast);
                });
        }
        _ => {
            let resp = sP_FE2CL_REP_PC_ROCKET_STYLE_FIRE_SUCC {
                iSkillID: unused!(),
                iX: start_pos.x,
                iY: start_pos.y,
                iZ: start_pos.z,
                iToX: to.x,
                iToY: to.y,
                iToZ: to.z,
                iBulletID: bullet_id,
                Bullet: bullet,
                iBatteryW: weapon_boosts,
                bNanoDeactive: unused!(),
                iNanoID: unused!(),
                iNanoStamina: unused!(),
            };
            client.send_packet(P_FE2CL_REP_PC_ROCKET_STYLE_FIRE_SUCC, &resp);

            let bcast = sP_FE2CL_PC_ROCKET_STYLE_FIRE {
                iPC_ID: pc_id,
                iX: start_pos.x,
                iY: start_pos.y,
                iZ: start_pos.z,
                iToX: to.x,
                iToY: to.y,
                iToZ: to.z,
                iBulletID: bullet_id,
                Bullet: bullet,
                bNanoDeactive: unused!(),
            };
            state
                .entity_map
                .for_each_around(EntityID::Player(pc_id), |c| {
                    c.send_packet(P_FE2CL_PC_ROCKET_STYLE_FIRE, &bcast);
                });
        }
    }

    Ok(())
}

/// Handles a projectile detonating. Rockets and grenades use the same request
/// body, so one handler covers both.
pub fn pc_projectile_hit(pkt: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let pc_id = clients.get_sender().get_player_id()?;
    let request = *pkt.get::<sP_CL2FE_REQ_PC_ROCKET_STYLE_HIT>()?;
    let count = request.iTargetCnt as usize;
    if count > MAX_PROJECTILE_TARGETS || pkt.read_bytes().len() != 4 + size_of::<sP_CL2FE_REQ_PC_ROCKET_STYLE_HIT>() + count*8 {
        return Err(invalid_weapon("Malformed projectile targets"));
    }
    let player = state.get_player_mut(pc_id)?;
    player.expire_projectiles(SystemTime::now());
    let projectile = player.get_projectile(request.iBulletID).ok_or_else(|| invalid_weapon("Projectile slot is not owned or has expired"))?;
    if projectile.instance_id != player.instance_id || count > projectile.target_count {
        return Err(invalid_weapon("Projectile instance or target count does not match its launch"));
    }
    let hit = Position::new(request.iX, request.iY, request.iZ);
    if distance(projectile.start_pos, hit) > f64::from(projectile.travel_range) {
        return Err(invalid_weapon("Projectile impact is beyond its flight envelope"));
    }
    if matches!(projectile.kind, ProjectileKind::Rocket(_)) {
        // A rocket can collide before its aimed endpoint, but cannot teleport
        // the explosion to an unrelated part of this instance.
        let a = projectile.start_pos; let b = projectile.end_pos;
        let d = [f64::from(b.x)-f64::from(a.x), f64::from(b.y)-f64::from(a.y), f64::from(b.z)-f64::from(a.z)];
        let h = [f64::from(hit.x)-f64::from(a.x), f64::from(hit.y)-f64::from(a.y), f64::from(hit.z)-f64::from(a.z)];
        let length = d.iter().map(|v|v*v).sum::<f64>();
        let t = if length > 0.0 { (h.iter().zip(d).map(|(h,d)|h*d).sum::<f64>()/length).clamp(0.0,1.0) } else { 0.0 };
        let off = h.iter().zip(d).map(|(h,d)|(h-t*d).powi(2)).sum::<f64>().sqrt();
        if off > 500.0 { return Err(invalid_weapon("Rocket impact is outside its launch path")); }
    }
    let mut reader = PacketReader::new(&pkt); reader.get_struct::<sP_CL2FE_REQ_PC_ROCKET_STYLE_HIT>()?;
    let mut targets = Vec::with_capacity(count); let mut seen = std::collections::HashSet::new();
    for _ in 0..count {
        let target = *reader.get_struct::<sProjectileTarget>()?;
        let npc = state.get_npc(target.iID)?;
        if target._unused != 0 || !seen.insert(target.iID) || npc.is_dead() || npc.get_team() != CombatantTeam::Mob
            || npc.instance_id != projectile.instance_id || !npc.visible_to(state.get_player(pc_id)?)
            || !nano_target_in_range(hit, npc, projectile.effect_area)? {
            return Err(invalid_weapon("Invalid projectile blast target"));
        }
        targets.push(EntityID::NPC(target.iID));
    }
    // Commit only after the complete hit frame and all claimed targets pass.
    state.get_player_mut(pc_id)?.remove_projectile(request.iBulletID);
    if targets.is_empty() { return Ok(()); }
    skills::do_basic_attack(EntityID::Player(pc_id), &targets, projectile.charged,
        AttackContext { power: Some((projectile.single_power, projectile.multi_power)),
            projectile: Some((request.iBulletID, projectile.into())), ..Default::default() }, state)
}
