//! Recall registration through the production handler with real 0104 rows.
use super::nano::recall_register;
use crate::{
    chunk::{InstanceID, TickMode},
    entity::{Combatant, Entity, EntityID, Player, NPC},
    net::{packet::{Packet, PacketBuilder, PacketID::*, *}, ClientMessage, ClientMetadata, ClientType, FFClient},
    state::ShardServerState,
    tabledata::{tdata_get, tdata_init},
    Position,
};

type Rx = tokio::sync::mpsc::UnboundedReceiver<ClientMessage>;
fn fixture() -> (ShardServerState, FFClient, Rx) {
    tdata_init().unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(tx, ClientMetadata::new("127.0.0.1:39014".parse().unwrap(), Some(ClientType::GameClient { account_id: 1, serial_key: 1, pc_id: Some(1) })));
    let mut player = Player::new(1, 0);
    player.set_player_id(1); player.set_client(client.clone());
    player.instance_id = InstanceID { map_num: 75, ..Default::default() };
    let pos = Position::new(130715, 223861, -16066);
    player.set_position(pos);
    let mut state = ShardServerState::default();
    state.entity_map = crate::chunk::EntityMap::default();
    state.entity_map.track(Box::new(NPC::new(569, 1386, pos, 0, player.instance_id).unwrap()), TickMode::Always);
    state.entity_map.track(Box::new(NPC::new(570, 1386, Position { x: pos.x + 50, ..pos }, 0, player.instance_id).unwrap()), TickMode::Always);
    state.entity_map.track(Box::new(player), TickMode::Always);
    (state, client, rx)
}
fn request(npc: i32) -> Packet {
    Packet::new(P_CL2FE_REQ_REGIST_RXCOM, &sP_CL2FE_REQ_REGIST_RXCOM { iNPCID: npc }).unwrap()
}
fn reply(rx: &mut Rx, id: PacketID) -> Packet {
    std::iter::from_fn(|| rx.try_recv().ok()).find_map(|m| match m {
        ClientMessage::SendPacket(p) if p.id() == id => Some(p), _ => None,
    }).expect("expected Recall response")
}

#[test]
fn recall_ack_selects_npc_identity_and_registration_replaces_only_recall_target() {
    let (mut state, client, mut rx) = fixture();
    let pre = state.get_player(1).unwrap().get_pre_warp().clone();
    for id in [569, 569, 570] {
        recall_register(request(id), &client, &mut state).unwrap();
        let response = reply(&mut rx, P_FE2CL_REP_REGIST_RXCOM);
        assert_eq!(response.read_bytes().len(), 20);
        let p = *response.get::<sP_FE2CL_REP_REGIST_RXCOM>().unwrap();
        let point = state.get_player(1).unwrap().get_recall_point().unwrap();
        assert_eq!({p.iMapNum}, id); // not map 75 or type 1386
        assert_eq!(({p.iX}, {p.iY}, {p.iZ}), (point.position.x, point.position.y, point.position.z));
        assert_eq!(point.npc_id, id);
        let after = state.get_player(1).unwrap().get_pre_warp();
        assert_eq!((after.instance_id, after.position), (pre.instance_id, pre.position));
    }
}

#[test]
fn recall_rejections_preserve_last_registration_and_return_fail() {
    for case in ["missing", "wrong_type", "far", "map", "copy", "hidden", "disabled", "dead", "overworld", "trailing", "short"] {
        let (mut state, client, mut rx) = fixture();
        recall_register(request(569), &client, &mut state).unwrap();
        reply(&mut rx, P_FE2CL_REP_REGIST_RXCOM);
        let accepted = state.get_player(1).unwrap().get_recall_point().unwrap();
        let mut req = request(570);
        match case {
            "missing" => req = request(999999),
            "wrong_type" => state.get_npc_mut(570).unwrap().ty = 651,
            "far" => state.get_npc_mut(570).unwrap().set_position(Position::new(500000, 0, 0)),
            "map" => state.get_npc_mut(570).unwrap().instance_id.map_num = 76,
            "copy" => state.get_npc_mut(570).unwrap().instance_id.instance_num = Some(123),
            "hidden" => state.get_npc_mut(570).unwrap().hidden = true,
            "disabled" => state.get_npc_mut(570).unwrap().interactable = false,
            "dead" => { state.get_player_mut(1).unwrap().take_damage(i32::MAX, None); },
            "overworld" => {
                state.get_player_mut(1).unwrap().instance_id.map_num = 0;
                state.get_npc_mut(570).unwrap().instance_id.map_num = 0;
            },
            "trailing" => req = PacketBuilder::new(P_CL2FE_REQ_REGIST_RXCOM).with(&sP_CL2FE_REQ_REGIST_RXCOM { iNPCID: 570 }).with(&sP_CL2FE_REQ_REGIST_RXCOM { iNPCID: 569 }).build().unwrap(),
            "short" => req = PacketBuilder::new(P_CL2FE_REQ_REGIST_RXCOM).build().unwrap(),
            _ => unreachable!(),
        }
        assert!(recall_register(req, &client, &mut state).is_err(), "{case}");
        let failure = reply(&mut rx, P_FE2CL_REP_REGIST_RXCOM_FAIL);
        assert_eq!(failure.read_bytes().len(), 8);
        // Direct instance edits are test fixtures; returning restores the original state.
        state.get_player_mut(1).unwrap().instance_id = accepted.instance_id;
        assert_eq!(state.get_player(1).unwrap().get_recall_point().unwrap().npc_id, 569, "{case}");
    }
}

#[test]
fn recall_skill_uses_registered_point_and_expires_on_instance_change() {
    let (mut state, client, mut rx) = fixture();
    let skill = tdata_get().get_skill(25).unwrap();
    let origin = state.get_player(1).unwrap().get_position();
    let moved = Position { x: origin.x + 400, ..origin };
    assert!(crate::skills::do_skill(EntityID::Player(1), &[EntityID::Player(1)], skill, 0, &mut state).is_err());
    recall_register(request(569), &client, &mut state).unwrap();
    reply(&mut rx, P_FE2CL_REP_REGIST_RXCOM);
    state.get_player_mut(1).unwrap().set_position(moved);
    crate::skills::do_skill(EntityID::Player(1), &[EntityID::Player(1)], skill, 0, &mut state).unwrap();
    assert_eq!(state.get_player(1).unwrap().get_position(), origin);
    let warp = reply(&mut rx, P_FE2CL_REP_PC_GOTO_SUCC);
    assert_eq!(warp.read_bytes().len(), 16);
    let player = state.get_player_mut(1).unwrap();
    let same = player.instance_id;
    player.set_instance_id(InstanceID { instance_num: Some(2), ..same });
    player.set_instance_id(same);
    assert!(player.get_recall_point().is_none());
    assert!(crate::skills::do_skill(EntityID::Player(1), &[EntityID::Player(1)], skill, 0, &mut state).is_err());
    recall_register(request(569), &client, &mut state).unwrap();
    state.get_player_mut(1).unwrap().clear_recall_point();
    assert!(state.get_player(1).unwrap().get_recall_point().is_none());
}

#[test]
fn recall_nano_wire_request_resolves_self_charges_once_and_rejects_missing_or_expired_point() {
    use std::collections::HashMap;
    use crate::net::ClientMap;
    let (mut state, client, mut rx) = fixture();
    let player = state.get_player_mut(1).unwrap();
    // Real Nano 9 owns accepted tune 25; no table override or administrator bypass.
    player.unlock_nano(9).unwrap(); player.tune_nano(9, Some(25)).unwrap();
    player.change_nano(0, Some(9)).unwrap(); player.activate_nano(0).unwrap();
    let clients = HashMap::from([(1usize, client.clone())]);
    let skill_request = || Packet::new(P_CL2FE_REQ_NANO_SKILL_USE, &sP_CL2FE_REQ_NANO_SKILL_USE {
        iBulletID: -1, iArg1: 0, iArg2: 0, iArg3: 0, iTargetCnt: 0,
    }).unwrap();
    let stamina = |state: &ShardServerState| state.get_player(1).unwrap().get_active_nano().unwrap().get_stamina();
    assert!(super::combat::nano_skill_use(skill_request(), &ClientMap::new(1, &clients), &mut state).is_err());
    assert_eq!(stamina(&state), 150);
    recall_register(request(569), &client, &mut state).unwrap();
    reply(&mut rx, P_FE2CL_REP_REGIST_RXCOM);
    let registered = state.get_player(1).unwrap().get_position();
    let moved = Position { x: registered.x + 400, ..registered };
    state.get_player_mut(1).unwrap().set_position(moved);
    super::combat::nano_skill_use(skill_request(), &ClientMap::new(1, &clients), &mut state).unwrap();
    assert_eq!(state.get_player(1).unwrap().get_position(), registered);
    assert_eq!(stamina(&state), 70); // exact authored cost 80, once
    reply(&mut rx, P_FE2CL_REP_PC_GOTO_SUCC);
    assert!(!state.get_player(1).unwrap().get_active_nano().unwrap().skill_is_ready());
    state.get_player_mut(1).unwrap().set_position(moved);
    assert!(super::combat::nano_skill_use(skill_request(), &ClientMap::new(1, &clients), &mut state).is_err());
    assert_eq!(state.get_player(1).unwrap().get_position(), moved);
    assert_eq!(stamina(&state), 70);
    state.get_player_mut(1).unwrap().clear_recall_point();
    assert!(crate::skills::do_skill(EntityID::Player(1), &[EntityID::Player(1)], tdata_get().get_skill(25).unwrap(), 0, &mut state).is_err());
    assert_eq!(stamina(&state), 70);
}

#[test]
fn unstable_nano_randomly_warps_within_each_public_and_private_map() {
    use crate::net::ClientMap;
    use std::collections::{HashMap, HashSet};
    tdata_init().unwrap();
    // Include single-entrance maps, paired instances using the same authored geometry,
    // and a public world. Start each sequence at its actual entrance.
    for warp_id in [2, 24, 28, 1, 18, 26, 251, 273, 274, 275, 276, 291, 298, 302, 305, 307, 309] {
        let warp = tdata_get().get_warp_data(warp_id).unwrap();
        let map = warp.map_num;
        let copy = (map != 0).then_some(map + 900);
        let instance = InstanceID { channel_num: 1, map_num: map, instance_num: copy };
        let mut seen = HashSet::new();
        let (mut state, client, mut rx) = fixture();
        let player = state.get_player_mut(1).unwrap();
        player.set_instance_id(instance);
        player.set_position(warp.pos);
        player.unlock_nano(39).unwrap();
        player.tune_nano(39, Some(287)).unwrap();
        player.change_nano(0, Some(39)).unwrap();
        player.activate_nano(0).unwrap();
        let clients = HashMap::from([(1usize, client.clone())]);
        let casts = if matches!(map, 0 | 12 | 14) {24} else {4};
        for _ in 0..casts {
            let player = state.get_player_mut(1).unwrap();
            let start = player.get_position();
            // Isolate destination selection from the separately tested stamina and real-time lock.
            let nano = player.get_active_nano_mut().unwrap();
            nano.set_stamina(150);
            nano.start_skill_cooldown(std::time::Duration::ZERO);
            let request = Packet::new(P_CL2FE_REQ_NANO_SKILL_USE, &sP_CL2FE_REQ_NANO_SKILL_USE {
                iBulletID: -1, iArg1: 0, iArg2: 0, iArg3: 0, iTargetCnt: 0,
            }).unwrap();
            super::combat::nano_skill_use(request, &ClientMap::new(1, &clients), &mut state).unwrap();
            let end = state.get_player(1).unwrap();
            assert_eq!(end.get_instance_id(), instance);
            let point = end.get_position();
            assert!(start.distance_to(&point) >= 150);
            let warp = reply(&mut rx, P_FE2CL_REP_PC_GOTO_SUCC);
            let result = *warp.get::<sP_FE2CL_REP_PC_GOTO_SUCC>().unwrap();
            assert_eq!(({result.iX}, {result.iY}, {result.iZ}), (point.x, point.y, point.z));
            assert_eq!(end.get_active_nano().unwrap().get_stamina(), 70);
            seen.insert((point.x, point.y, point.z));
        }
        assert!(seen.len() >= 2, "map {map} requires distinct same-map landing choices {seen:?}");
        println!("map {map}, copy {copy:?}: {} distinct authoritative destinations in {casts} successive 0104 casts", seen.len());
    }
}
