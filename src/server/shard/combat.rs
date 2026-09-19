use std::time::{Duration, SystemTime};

use crate::{
    defines::*,
    entity::{Combatant, Entity, EntityID, Projectile, ProjectileKind},
    enums::{SkillTargetType, TargetType, WeaponTargetMode},
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

#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[repr(packed(4))]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct sTargetPcId {
    pub iPC_ID: i32,
}
impl FFPacket for sTargetPcId {}

const MAX_TARGETS: usize = 3;
const BATTERY_BASE_COST: u32 = 6;

pub fn pc_attack_npcs(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    let mut reader = PacketReader::new(&pkt);
    let pkt: &sP_CL2FE_REQ_PC_ATTACK_NPCs = reader.get_struct()?;
    let target_count = pkt.iNPCCnt as usize;
    if target_count == 0 {
        return Ok(());
    }

    let mut target_ids = Vec::with_capacity(MAX_TARGETS);
    let mut weapon_boosts_needed = 0;
    for i in 0..target_count {
        // TODO stricter anti-cheat.
        // validate target count, range, attack cooldown, etc against weapon stats
        if i >= MAX_TARGETS {
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "Player {} tried to attack {} NPCs (max {})",
                    pc_id, pkt.iNPCCnt, MAX_TARGETS
                ),
            ));
        }
        let npc_id = reader.get_struct::<sTargetNpcId>()?.iNPC_ID;
        let npc = match state.get_npc(npc_id) {
            Ok(npc) => npc,
            Err(e) => {
                log_error(e);
                continue;
            }
        };
        weapon_boosts_needed += BATTERY_BASE_COST + npc.get_level() as u32;
        target_ids.push(npc.get_id());
    }

    // consume weapon boosts
    let player = state.get_player_mut(pc_id)?;
    let weapon_boosts = player.get_weapon_boosts();
    let charged = if weapon_boosts >= weapon_boosts_needed {
        player.set_weapon_boosts(weapon_boosts - weapon_boosts_needed);
        true
    } else {
        player.set_weapon_boosts(0);
        false
    };

    // attack handler
    skills::do_basic_attack(
        player.get_id(),
        &target_ids,
        charged,
        AttackContext::default(),
        state,
    )?;

    Ok(())
}

pub fn pc_attack_pcs(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    let mut reader = PacketReader::new(&pkt);
    let pkt: &sP_CL2FE_REQ_PC_ATTACK_CHARs = reader.get_struct()?;
    let target_count = pkt.iTargetCnt as usize;
    if target_count == 0 {
        return Ok(());
    }

    let player = state.get_player(pc_id)?;
    let mut target_ids = Vec::with_capacity(MAX_TARGETS);
    let mut weapon_boosts_needed = 0;
    for i in 0..target_count {
        // TODO see above
        if i >= MAX_TARGETS {
            log(
                Severity::Warning,
                &format!(
                    "{} tried to attack {} PCs (max {})",
                    player, pkt.iTargetCnt, MAX_TARGETS
                ),
            );
            break;
        }

        let target_pc_id = reader.get_struct::<sTargetPcId>()?.iPC_ID;
        if target_pc_id == pc_id {
            log(
                Severity::Warning,
                &format!("{} tried to attack themselves", player),
            );
            continue;
        }

        let target_player = match state.get_player(target_pc_id) {
            Ok(player) => player,
            Err(e) => {
                log_error(e);
                continue;
            }
        };

        weapon_boosts_needed += BATTERY_BASE_COST + target_player.get_level() as u32;
        target_ids.push(target_player.get_id());
    }

    // consume weapon boosts
    let player = state.get_player_mut(pc_id)?;
    let weapon_boosts = player.get_weapon_boosts();
    let charged = if weapon_boosts >= weapon_boosts_needed {
        player.set_weapon_boosts(weapon_boosts - weapon_boosts_needed);
        true
    } else {
        player.set_weapon_boosts(0);
        false
    };

    // attack handler
    skills::do_basic_attack(
        player.get_id(),
        &target_ids,
        charged,
        AttackContext::default(),
        state,
    )?;

    Ok(())
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

    let mut reader = PacketReader::new(&pkt);
    let pkt: &sP_CL2FE_REQ_NANO_SKILL_USE = reader.get_struct()?;
    let pkt_copy = *pkt;
    let target_count = pkt.iTargetCnt as usize;

    // Group skills (group heal, group recall, group phoenix) come in with no
    // targets at all: the client expects the server to resolve the group.
    if skill.target_type == TargetType::CasterPC {
        let mut target_ids = vec![EntityID::Player(pc_id)];
        if let Some(group) = group_id.and_then(|group_id| state.groups.get(&group_id)) {
            target_ids = group
                .get_member_ids()
                .iter()
                .copied()
                .filter(|id| matches!(id, EntityID::Player(_)))
                .collect();
        }
        return finish_nano_skill(&pkt_copy, &target_ids, skill, skill_level, clients, state);
    }

    if target_count == 0 {
        return Ok(());
    }

    let mut target_ids = Vec::with_capacity(MAX_TARGETS);
    for i in 0..target_count {
        if i >= MAX_TARGETS {
            log(
                Severity::Warning,
                &format!(
                    "{} tried to use a nano skill on {} targets (max {})",
                    player, pkt.iTargetCnt, MAX_TARGETS
                ),
            );
            break;
        }

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

        // validate against targeting type
        let valid = match skill.targeting_type {
            SkillTargetType::None => {
                return Err(FFError::build(
                    Severity::Warning,
                    format!(
                        "{} tried to use a nano skill with no targeting type",
                        player
                    ),
                ));
            }
            _ => placeholder!(true), // TODO validate for each targeting type
        };

        if !valid {
            log(
                Severity::Warning,
                &format!(
                    "{} tried to use a nano skill on an invalid target {:?} for targeting type {:?}",
                    player, target_id, skill.targeting_type
                ),
            );
            continue;
        }

        target_ids.push(target_id);
    }

    finish_nano_skill(&pkt_copy, &target_ids, skill, skill_level, clients, state)
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

    let results = skills::do_skill(
        EntityID::Player(pc_id),
        target_ids,
        skill,
        skill_level,
        state,
    )?;

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

    let target_cnt = results.len() as i32;
    let skill_id = nano.selected_skill.unwrap();
    let nano_stamina = nano.get_stamina();
    let nano_deactive = nano_stamina == 0;
    let nano_id = nano.get_id();
    let skill_type = skill.skill_type as i32;

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
const EXPLOSION_RADIUS: u32 = 500;
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
    let pkt: &sP_CL2FE_REQ_PC_ROCKET_STYLE_READY = pkt.get()?;
    let pc_id = clients.get_sender().get_player_id()?;
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
    let pkt: &sP_CL2FE_REQ_PC_GRENADE_STYLE_READY = pkt.get()?;
    let pc_id = clients.get_sender().get_player_id()?;
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
    let pkt: &sP_CL2FE_REQ_PC_ROCKET_STYLE_FIRE = pkt.get()?;
    let from = Position::new(pkt.iX, pkt.iY, pkt.iZ);
    let to = Position::new(pkt.iToX, pkt.iToY, pkt.iToZ);
    fire_projectile(WeaponTargetMode::Rocket, Some(from), to, clients, state)
}

pub fn pc_grenade_style_fire(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_GRENADE_STYLE_FIRE = pkt.get()?;
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

    let player = state.get_player_mut(pc_id)?;
    let charged = player.consume_weapon_boosts(weapon_boosts_needed);
    let start_pos = from.unwrap_or_else(|| player.get_position());

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
        expires: SystemTime::now() + flight_time + Duration::from_secs(10),
    };

    // stale projectiles would otherwise hold their slots forever
    player.expire_projectiles(SystemTime::now());
    let bullet_id = player.add_projectile(projectile).ok_or_else(|| {
        FFError::build(
            Severity::Warning,
            format!("Player {} has too many projectiles in flight", pc_id),
        )
    })?;
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
pub fn pc_projectile_hit(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    let mut reader = PacketReader::new(&pkt);
    let pkt: &sP_CL2FE_REQ_PC_ROCKET_STYLE_HIT = reader.get_struct()?;
    let bullet_id = pkt.iBulletID;
    let target_count = pkt.iTargetCnt as usize;
    let hit_pos = Position::new(pkt.iX, pkt.iY, pkt.iZ);

    let player = state.get_player_mut(pc_id)?;
    let projectile = player.remove_projectile(bullet_id).ok_or_else(|| {
        FFError::build(
            Severity::Warning,
            format!("Player {} has no projectile {} in flight", pc_id, bullet_id),
        )
    })?;

    if target_count == 0 {
        // nothing was hit; the bullet slot is already freed
        return Ok(());
    }
    if target_count > MAX_PROJECTILE_TARGETS {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "Player {} reported {} projectile targets (max {})",
                pc_id, target_count, MAX_PROJECTILE_TARGETS
            ),
        ));
    }

    let instance_id = state.get_player(pc_id)?.instance_id;
    let mut claimed_ids = Vec::with_capacity(target_count);
    for _ in 0..target_count {
        let npc_id = reader.get_struct::<sProjectileTarget>()?.iID;
        // only mobs can be caught in a blast
        if state.get_npc(npc_id).is_err() {
            continue;
        }
        claimed_ids.push(EntityID::NPC(npc_id));
    }

    // the client decides what the blast touched, so re-check it against the
    // impact point before anything takes damage
    let target_ids = state.entity_map.filter_ids_in_proximity(
        hit_pos,
        instance_id,
        &claimed_ids,
        EXPLOSION_RADIUS,
    );
    if target_ids.is_empty() {
        return Ok(());
    }

    skills::do_basic_attack(
        EntityID::Player(pc_id),
        &target_ids,
        projectile.charged,
        AttackContext {
            power: Some((projectile.single_power, projectile.multi_power)),
            projectile: Some((bullet_id, projectile.into())),
        },
        state,
    )
}
