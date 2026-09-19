use std::time::SystemTime;

use crate::{
    defines::*,
    entity::{Combatant, Entity, EntityID},
    enums::{BuffType, ItemLocation, ItemType, SkillType},
    error::*,
    item::Item,
    net::{
        packet::{PacketID::*, *},
        ClientMap,
    },
    skills,
    state::ShardServerState,
    tabledata::tdata_get,
};

/// Picks up an E.G.G. The client only shows the buff icon when we reply with a
/// skill ID, so the reply is skipped entirely for eggs that have no effect
/// (crate-only eggs), same as OpenFusion.
pub fn shiny_pickup(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_SHINY_PICKUP = pkt.get()?;
    let egg_id = pkt.iShinyID;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    (|| {
        let egg = state.get_egg(egg_id)?;
        if !egg.is_live() {
            return Err(FFError::build(
                Severity::Warning,
                format!("E.G.G. {} has already been picked up", egg_id),
            ));
        }

        let egg_type = egg.get_type();
        let egg_pos = egg.get_position();
        let egg_instance = egg.get_instance_id();
        let summoned = egg.is_summoned();
        let stats = tdata_get().get_egg_stats(egg_type)?;
        let crate_id = stats.crate_id;
        let effect_id = stats.effect_id;
        let effect_duration = stats.effect_duration;
        let respawn_time = stats.respawn_time;

        let player = state.get_player(pc_id)?;
        if player.instance_id != egg_instance
            || player.get_position().distance_to(&egg_pos) > RANGE_INTERACT
        {
            return Err(FFError::build(
                Severity::Warning,
                format!("{} tried to pick up an E.G.G. out of reach", player),
            ));
        }

        // take the egg out of the world first, so a double-click can't
        // collect it twice
        let egg = state.get_egg_mut(egg_id)?;
        egg.pick_up(SystemTime::now() + respawn_time);
        state.entity_map.update(EntityID::Egg(egg_id), None, true);
        if summoned {
            // a GM-summoned E.G.G. is one-shot; it never comes back
            state.entity_map.mark_for_cleanup(EntityID::Egg(egg_id));
        }

        if let Some(effect_id) = effect_id {
            log_if_failed(apply_egg_effect(
                pc_id,
                egg_id,
                effect_id as i16,
                effect_duration_secs(effect_duration),
                state,
            ));
        }

        if let Some(crate_id) = crate_id {
            log_if_failed(give_egg_crate(pc_id, crate_id, state));
        }

        Ok(())
    })()
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_SHINY_PICKUP_FAIL::default();
        client.send_packet(P_FE2CL_REP_SHINY_PICKUP_FAIL, &resp);
    })
}

fn effect_duration_secs(duration: std::time::Duration) -> u64 {
    duration.as_secs()
}

/// Applies the E.G.G.'s skill to the player who picked it up.
///
/// Passive-drain skills become a timed Land Effect buff; active ones (the
/// damage and heal eggs) resolve immediately.
fn apply_egg_effect(
    pc_id: i32,
    egg_id: i32,
    skill_id: i16,
    duration_secs: u64,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let skill = tdata_get().get_skill(skill_id)?;
    let player_eid = EntityID::Player(pc_id);
    let egg_eid = EntityID::Egg(egg_id);

    let (result, buff_id) = if skill.passive {
        let player = state.get_player_mut(pc_id)?;
        // The E.G.G.'s own duration wins over whatever the skill table says.
        let buff_id =
            skills::apply_skill_buff(player, skill, 0, BuffType::LandEffect, None, Some(egg_eid))?;
        player.override_buff_duration(buff_id, std::time::Duration::from_secs(duration_secs));

        (
            Some(SkillResultPayload::Buff(sSkillResult_Buff {
                eCT: player.get_char_type() as i32,
                iID: pc_id,
                bProtected: unused!(),
                iConditionBitFlag: player.get_condition_bit_flag(),
            })),
            Some(buff_id),
        )
    } else {
        let player = state.get_player_mut(pc_id)?;
        let value = player.get_max_hp() * skill.values_a[0] / 1000;
        match skill.skill_type {
            SkillType::Damage => {
                let dealt = player.take_damage(value, Some(egg_eid));
                (
                    Some(SkillResultPayload::Damage(sSkillResult_Damage {
                        eCT: player.get_char_type() as i32,
                        iID: pc_id,
                        bProtected: (dealt <= 0) as i32,
                        iDamage: dealt,
                        iHP: player.get_hp(),
                    })),
                    None,
                )
            }
            SkillType::HealHP => {
                let healed = player.heal(value);
                (
                    Some(SkillResultPayload::HealHP(sSkillResult_Heal_HP {
                        eCT: player.get_char_type() as i32,
                        iID: pc_id,
                        iHealHP: healed,
                        iHP: player.get_hp(),
                    })),
                    None,
                )
            }
            other => {
                return Err(FFError::build(
                    Severity::Warning,
                    format!("E.G.G. skill type {:?} is not supported", other),
                ));
            }
        }
    };

    // tell the picker which buff icon to show
    let player = state.get_player(pc_id)?;
    if let Some(client) = player.get_client() {
        let resp = sP_FE2CL_REP_SHINY_PICKUP_SUCC {
            iSkillID: skill_id,
            // the client can work out the icon for most skills, but a damage
            // egg needs to be told explicitly
            eCSTB: buff_id.map_or(0, |id| id as i32),
        };
        client.send_packet(P_FE2CL_REP_SHINY_PICKUP_SUCC, &resp);
    }

    // and show the effect to everyone nearby
    if let Some(result) = result {
        let mut builder = PacketBuilder::new(P_FE2CL_NPC_SKILL_HIT).with(&sP_FE2CL_NPC_SKILL_HIT {
            iNPC_ID: egg_id,
            iSkillID: skill_id,
            iValue1: unused!(),
            iValue2: unused!(),
            iValue3: unused!(),
            eST: skill.skill_type as i32,
            iTargetCnt: 1,
        });
        match &result {
            SkillResultPayload::Buff(r) => builder.push(r),
            SkillResultPayload::Damage(r) => builder.push(r),
            SkillResultPayload::HealHP(r) => builder.push(r),
        }
        if let Some(pkt) = log_if_failed(builder.build()) {
            state.entity_map.for_each_around(player_eid, |c| {
                c.send_payload(pkt.clone());
            });
        }
    }

    Ok(())
}

enum SkillResultPayload {
    Buff(sSkillResult_Buff),
    Damage(sSkillResult_Damage),
    HealHP(sSkillResult_Heal_HP),
}

/// Drops the E.G.G.'s C.R.A.T.E. into the player's inventory, if there's room.
fn give_egg_crate(pc_id: i32, crate_id: i16, state: &mut ShardServerState) -> FFResult<()> {
    let player = state.get_player_mut(pc_id)?;
    let slot_num = player.find_free_slot(ItemLocation::Inven)?;
    let item = Item::new(ItemType::Chest, crate_id);
    player.set_item(ItemLocation::Inven, slot_num, Some(item))?;

    let Some(client) = player.get_client() else {
        return Ok(());
    };
    let pkt = PacketBuilder::new(P_FE2CL_REP_REWARD_ITEM)
        .with(&sP_FE2CL_REP_REWARD_ITEM {
            m_iCandy: player.get_taros() as i32,
            m_iFusionMatter: player.get_fusion_matter() as i32,
            m_iBatteryN: player.get_nano_potions() as i32,
            m_iBatteryW: player.get_weapon_boosts() as i32,
            iItemCnt: 1,
            iFatigue: 100,
            iFatigue_Level: 1,
            iNPC_TypeID: unused!(),
            iTaskID: unused!(),
        })
        .with(&sItemReward {
            sItem: Some(item).into_proto(),
            eIL: ItemLocation::Inven as i32,
            iSlotNum: slot_num as i32,
        })
        .build()?;
    client.send_payload(pkt);
    Ok(())
}
