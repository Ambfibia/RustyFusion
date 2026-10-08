use super::*;
use crate::{
    chunk::{ChunkCoords, InstanceID, TickMode},
    entity::{Entity, Player},
    net::{ClientMessage, ClientMetadata, ClientType, FFClient},
    tabledata::tdata_init,
};
use std::{collections::HashMap, time::SystemTime};

fn fixture() -> (
    ShardServerState,
    HashMap<usize, FFClient>,
    tokio::sync::mpsc::UnboundedReceiver<ClientMessage>,
) {
    tdata_init().unwrap();
    let mut state = ShardServerState::default();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            "127.0.0.1:39207".parse().unwrap(),
            Some(ClientType::GameClient {
                account_id: 1,
                serial_key: 1,
                pc_id: Some(1),
            }),
        ),
    );
    let mut player = Player::new(1, 0);
    player.set_player_id(1);
    player.set_client(client.clone());
    player.set_level(4).unwrap();
    player.set_hp(player.get_max_hp());
    let chunk = ChunkCoords::from_pos_inst(player.get_position(), InstanceID::default());
    let id = state.entity_map.track(Box::new(player), TickMode::Always);
    state.entity_map.update(id, Some(chunk), false);
    (state, HashMap::from([(1, client)]), rx)
}

fn toggle(
    state: &mut ShardServerState,
    clients: &HashMap<usize, FFClient>,
    flag: i32,
) -> FFResult<()> {
    let packet = PacketBuilder::new(P_CL2FE_DOT_DAMAGE_ONOFF)
        .with(&sP_CL2FE_DOT_DAMAGE_ONOFF { iFlag: flag })
        .build()?;
    on_off(packet, &ClientMap::new(1, clients), state)
}

fn results(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<ClientMessage>,
) -> Vec<sSkillResult_DotDamage> {
    let mut found = Vec::new();
    while let Ok(message) = rx.try_recv() {
        if let ClientMessage::SendPacket(packet) = message {
            if packet.id() == P_FE2CL_CHAR_TIME_BUFF_TIME_TICK {
                let mut reader = PacketReader::new(&packet);
                let prefix = *reader
                    .get_struct::<sP_FE2CL_CHAR_TIME_BUFF_TIME_TICK>()
                    .unwrap();
                assert_eq!(prefix.iTB_ID, BuffID::Infection as i16);
                found.push(*reader.get_struct::<sSkillResult_DotDamage>().unwrap());
            }
        }
    }
    found
}

#[test]
fn enter_ticks_once_without_regen_and_exit_stops_damage() {
    let (mut state, clients, mut rx) = fixture();
    toggle(&mut state, &clients, 1).unwrap();
    toggle(&mut state, &clients, 1).unwrap();
    let max = state.get_player(1).unwrap().get_max_hp();
    Player::tick(&mut state, 1, &SystemTime::now());
    assert_eq!(state.pending_buff_effects.len(), 1);
    state.pending_buff_effects.clear();
    tick(1, &mut state).unwrap();
    Player::tick(&mut state, 1, &SystemTime::now());
    assert!(
        state.pending_buff_effects.is_empty(),
        "two-second tick must not stack or repeat per frame"
    );
    assert_eq!(state.get_player(1).unwrap().get_hp(), max - max * 3 / 20);
    let hits = results(&mut rx);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].iDamage, max * 3 / 20);
    assert_eq!(hits[0].iHP, max - max * 3 / 20);
    toggle(&mut state, &clients, 0).unwrap();
    let hp = state.get_player(1).unwrap().get_hp();
    tick(1, &mut state).unwrap();
    assert_eq!(state.get_player(1).unwrap().get_hp(), hp);
    assert!(results(&mut rx).is_empty());
}

#[test]
fn protection_consumes_nano_and_exhaustion_removes_its_protection() {
    let (mut state, clients, mut rx) = fixture();
    let player = state.get_player_mut(1).unwrap();
    player.unlock_nano(5).unwrap().tune(Some(14));
    player.change_nano(0, Some(5)).unwrap();
    player.activate_nano(0).unwrap();
    player.get_active_nano_mut().unwrap().set_stamina(3);
    toggle(&mut state, &clients, 1).unwrap();
    let hp = state.get_player(1).unwrap().get_hp();
    tick(1, &mut state).unwrap();
    let hits = results(&mut rx);
    assert_eq!(hits[0].iDamage, -2);
    assert_eq!(hits[0].bProtected, 1);
    assert_eq!(hits[0].iStamina, 0);
    assert_eq!(hits[0].bNanoDeactive, 1);
    assert_eq!(state.get_player(1).unwrap().get_hp(), hp);
    assert!(state.get_player(1).unwrap().get_active_nano().is_none());
    assert!(!state
        .get_player(1)
        .unwrap()
        .has_buff(BuffID::ProtectInfection, None));
    tick(1, &mut state).unwrap();
    assert!(state.get_player(1).unwrap().get_hp() < hp);
}

#[test]
fn egg_protection_and_invulnerability_and_invalid_toggle_preserve_state() {
    let (mut state, clients, mut rx) = fixture();
    assert!(toggle(&mut state, &clients, 2).is_err());
    assert!(!state
        .get_player(1)
        .unwrap()
        .has_buff(BuffID::Infection, None));
    let player = state.get_player_mut(1).unwrap();
    player.apply_buff(
        BuffID::ProtectInfection,
        BuffInstance::new(BuffType::Shiny, 0, None, None, None),
        None,
    );
    toggle(&mut state, &clients, 1).unwrap();
    tick(1, &mut state).unwrap();
    let hits = results(&mut rx);
    assert_eq!(hits[0].iDamage, -2);
    assert_eq!(hits[0].bNanoDeactive, 0);
    state.get_player_mut(1).unwrap().invulnerable = true;
    tick(1, &mut state).unwrap();
    assert!(results(&mut rx).is_empty());
}

#[test]
fn land_toggle_owns_only_infection_and_preserves_other_sources() {
    let (mut state, clients, _rx) = fixture();
    let player = state.get_player_mut(1).unwrap();
    player.apply_buff(
        BuffID::Heal,
        BuffInstance::new(BuffType::LandEffect, 0, None, None, None),
        None,
    );
    player.apply_buff(
        BuffID::Infection,
        BuffInstance::new(BuffType::Nano, 0, None, None, None),
        None,
    );
    assert!(!player.has_buff(BuffID::Infection, Some(BuffType::LandEffect)));
    toggle(&mut state, &clients, 1).unwrap();
    toggle(&mut state, &clients, 0).unwrap();
    let player = state.get_player(1).unwrap();
    assert!(!player.has_buff(BuffID::Infection, Some(BuffType::LandEffect)));
    assert!(player.has_buff(BuffID::Infection, Some(BuffType::Nano)));
    assert!(player.has_buff(BuffID::Heal, Some(BuffType::LandEffect)));
}
