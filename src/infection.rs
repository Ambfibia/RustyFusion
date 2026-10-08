//! Client-reported poisoned ground, with shard-owned damage and Nano protection.
use crate::{
    entity::{Combatant, EntityID},
    enums::{BuffID, BuffType, CharType},
    error::{FFError, FFResult, Severity},
    net::{
        packet::{PacketID::*, *},
        ClientMap,
    },
    skills::BuffInstance,
    state::ShardServerState,
};

pub fn on_off(packet: Packet, clients: &ClientMap, state: &mut ShardServerState) -> FFResult<()> {
    let flag = packet.get::<sP_CL2FE_DOT_DAMAGE_ONOFF>()?.iFlag;
    if !matches!(flag, 0 | 1) {
        return Err(FFError::build(
            Severity::Warning,
            "Invalid infection flag".into(),
        ));
    }
    let pc_id = clients.get_sender().get_player_id()?;
    let player = state.get_player_mut(pc_id)?;
    if flag == 1 && !player.has_buff(BuffID::Infection, Some(BuffType::LandEffect)) {
        player.apply_buff(
            BuffID::Infection,
            BuffInstance::new(BuffType::LandEffect, 0, None, None, None),
            None,
        );
    } else if flag == 0 {
        player.remove_buff(BuffID::Infection, Some(BuffType::LandEffect));
    }
    Ok(())
}

/// OpenFusion Combat::dealGooDamage: 15% max HP, or Protected (-2) and
/// three Nano stamina. Egg protection never consumes the active Nano.
pub fn tick(pc_id: i32, state: &mut ShardServerState) -> FFResult<()> {
    let player = state.get_player_mut(pc_id)?;
    if player.is_dead()
        || !player.has_buff(BuffID::Infection, None)
        || player.invulnerable
        || player.has_buff(BuffID::Invulnerable, None)
    {
        return Ok(());
    }
    let protected = player.has_buff(BuffID::ProtectInfection, None);
    let nano_protection = protected
        && !player.has_buff(BuffID::ProtectInfection, Some(BuffType::Shiny))
        && (player.has_buff(BuffID::ProtectInfection, Some(BuffType::Nano))
            || player.has_buff(BuffID::ProtectInfection, Some(BuffType::GroupNano)));
    let damage = if protected {
        -2
    } else {
        player.take_damage(player.get_max_hp() * 3 / 20, None)
    };
    if nano_protection {
        if let Some(nano) = player.get_active_nano_mut() {
            nano.set_stamina((nano.get_stamina() - 3).max(0));
        }
    }
    let stamina = player
        .get_active_nano()
        .map_or(0, |nano| nano.get_stamina());
    let deactivated = player.get_active_nano().is_some() && stamina <= 0;
    if deactivated {
        player.deactivate_nano();
    }
    let condition = player.get_condition_bit_flag();
    let packet = PacketBuilder::new(P_FE2CL_CHAR_TIME_BUFF_TIME_TICK)
        .with(&sP_FE2CL_CHAR_TIME_BUFF_TIME_TICK {
            eCT: CharType::Player as i32,
            iID: pc_id,
            iTB_ID: BuffID::Infection as i16,
        })
        .with(&sSkillResult_DotDamage {
            eCT: CharType::Player as i32,
            iID: pc_id,
            bProtected: protected as i32,
            iDamage: damage,
            iHP: player.get_hp(),
            iStamina: stamina,
            bNanoDeactive: deactivated as i32,
            iConditionBitFlag: condition,
        })
        .build()?;
    state
        .entity_map
        .for_each_around(EntityID::Player(pc_id), |client| {
            client.send_payload(packet.clone());
            if deactivated {
                client.send_packet(
                    P_FE2CL_NANO_ACTIVE,
                    &sP_FE2CL_NANO_ACTIVE {
                        iPC_ID: pc_id,
                        Nano: Default::default(),
                        iConditionBitFlag: condition,
                        eCSTB___Add: 0,
                    },
                );
            }
        });
    Ok(())
}

#[cfg(test)]
mod tests;
