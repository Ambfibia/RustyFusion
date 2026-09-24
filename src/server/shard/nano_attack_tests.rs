//! Real table rows through the production packet handler, without a database.
use super::combat::nano_skill_use;
use crate::{
    chunk::TickMode,
    entity::{Combatant, Entity, EntityID, Player, NPC},
    net::{
        packet::{Packet, PacketBuilder, PacketID::*, PacketReader, *},
        ClientMap, ClientMessage, ClientMetadata, ClientType, FFClient,
    },
    state::ShardServerState,
    tabledata::tdata_init,
};
use std::collections::HashMap;

type Rx = tokio::sync::mpsc::UnboundedReceiver<ClientMessage>;
fn fixture(nano: i16, skill: i16) -> (ShardServerState, HashMap<usize, FFClient>, Rx) {
    tdata_init().unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            "127.0.0.1:39010".parse().unwrap(),
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
    player.unlock_nano(nano).unwrap();
    player.tune_nano(nano, Some(skill)).unwrap();
    player.change_nano(0, Some(nano)).unwrap();
    player.activate_nano(0).unwrap();
    let mut state = ShardServerState::default();
    state.entity_map = crate::chunk::EntityMap::default();
    for id in 10..13 {
        let npc = NPC::new(id, 59, player.get_position(), 0, player.instance_id).unwrap();
        state.entity_map.track(Box::new(npc), TickMode::Always);
    }
    state.entity_map.track(Box::new(player), TickMode::Always);
    (state, HashMap::from([(1, client)]), rx)
}
fn request(ids: &[i32]) -> Packet {
    let mut b = PacketBuilder::new(P_CL2FE_REQ_NANO_SKILL_USE).with(&sP_CL2FE_REQ_NANO_SKILL_USE {
        iBulletID: -1,
        iArg1: ids.first().copied().unwrap_or(0),
        iArg2: 0,
        iArg3: 0,
        iTargetCnt: ids.len() as i32,
    });
    for id in ids {
        b.push(&Target { id: *id });
    }
    b.build().unwrap()
}
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct Target {
    id: i32,
}
impl FFPacket for Target {}
fn stamina(state: &ShardServerState) -> i16 {
    state
        .get_player(1)
        .unwrap()
        .get_active_nano()
        .unwrap()
        .get_stamina()
}
fn hp(state: &ShardServerState, id: i32) -> i32 {
    state.get_npc(id).unwrap().get_hp()
}

#[test]
fn single_and_area_return_authoritative_hp_and_one_cost() {
    for (nano, skill, ids, cost, damage) in [
        (7, 19, vec![10], 26, 200),
        (6, 21, vec![10, 11, 12], 39, 130),
    ] {
        let (mut state, clients, mut rx) = fixture(nano, skill);
        nano_skill_use(request(&ids), &ClientMap::new(1, &clients), &mut state).unwrap();
        assert_eq!(stamina(&state), 150 - cost);
        for id in &ids {
            assert_eq!(hp(&state, *id), 350 - damage);
        }
        let packet = std::iter::from_fn(|| rx.try_recv().ok())
            .find_map(|m| match m {
                ClientMessage::SendPacket(p) if p.id() == P_FE2CL_NANO_SKILL_USE_SUCC => Some(p),
                _ => None,
            })
            .unwrap();
        let mut r = PacketReader::new(&packet);
        let p = *r.get_struct::<sP_FE2CL_NANO_SKILL_USE_SUCC>().unwrap();
        assert_eq!({ p.iTargetCnt }, ids.len() as i32);
        assert_eq!({ p.iNanoStamina }, 150 - cost);
        for id in ids {
            let result = *r.get_struct::<sSkillResult_Damage>().unwrap();
            assert_eq!(
                ({ result.iID }, { result.iHP }, { result.iDamage }),
                (id, hp(&state, id), damage)
            );
        }
    }
}

#[test]
fn missing_dead_far_and_duplicate_targets_do_not_spend_or_damage() {
    for case in [
        "missing",
        "dead",
        "far",
        "duplicate",
        "empty",
        "friendly",
        "instance",
        "area",
        "too_many",
    ] {
        let (mut state, clients, mut rx) = fixture(6, 21);
        let ids = match case {
            "missing" => vec![99999],
            "dead" => {
                state
                    .get_combatant_mut(EntityID::NPC(10))
                    .unwrap()
                    .take_damage(350, None);
                vec![10]
            }
            "far" => {
                let target = state.get_combatant_mut(EntityID::NPC(10)).unwrap();
                let mut pos = target.get_position();
                pos.x += 100_000;
                target.set_position(pos);
                vec![10]
            }
            "duplicate" => vec![10, 10],
            "friendly" => {
                state.get_npc_mut(10).unwrap().ty = 637;
                vec![10]
            }
            "instance" => {
                state.get_npc_mut(10).unwrap().instance_id.channel_num = 2;
                vec![10]
            }
            "area" => {
                let target = state.get_npc_mut(11).unwrap();
                let mut pos = target.get_position();
                pos.x += 1_000;
                target.set_position(pos);
                vec![10, 11]
            }
            "too_many" => vec![10, 11, 12, 99999],
            _ => vec![],
        };
        let before = hp(&state, 10);
        let _ = nano_skill_use(request(&ids), &ClientMap::new(1, &clients), &mut state);
        assert_eq!(
            stamina(&state),
            150,
            "{case}: rejected requests must not consume stamina"
        );
        assert_eq!(hp(&state, 10), before, "{case}");
        assert!(!std::iter::from_fn(|| rx.try_recv().ok()).any(|m| matches!(m, ClientMessage::SendPacket(p) if p.id() == P_FE2CL_NANO_SKILL_USE_SUCC)), "{case}: no empty success");
    }
}

#[test]
fn repeated_cast_cannot_damage_or_charge_twice() {
    let (mut state, clients, _) = fixture(6, 21);
    let clients = ClientMap::new(1, &clients);
    nano_skill_use(request(&[10]), &clients, &mut state).unwrap();
    let _ = nano_skill_use(request(&[10]), &clients, &mut state);
    assert_eq!(stamina(&state), 111);
    assert_eq!(hp(&state, 10), 220);
}

#[test]
fn switching_nanos_preserves_cooldown_and_expiry_allows_reuse() {
    let (mut state, clients, _) = fixture(6, 21);
    let clients = ClientMap::new(1, &clients);
    nano_skill_use(request(&[10]), &clients, &mut state).unwrap();
    let player = state.get_player_mut(1).unwrap();
    player.unlock_nano(7).unwrap();
    player.tune_nano(7, Some(19)).unwrap();
    player.change_nano(1, Some(7)).unwrap();
    player.activate_nano(1).unwrap();
    nano_skill_use(request(&[11]), &clients, &mut state).unwrap();
    let player = state.get_player_mut(1).unwrap();
    player.activate_nano(0).unwrap();
    assert!(nano_skill_use(request(&[10]), &clients, &mut state).is_err());
    assert_eq!(stamina(&state), 111);
    state
        .get_player_mut(1)
        .unwrap()
        .get_active_nano_mut()
        .unwrap()
        .start_skill_cooldown(std::time::Duration::ZERO);
    nano_skill_use(request(&[10]), &clients, &mut state).unwrap();
    assert_eq!((stamina(&state), hp(&state, 10)), (72, 90));
}

#[test]
fn insufficient_stamina_and_exact_depletion_are_authoritative() {
    let (mut state, clients, _) = fixture(7, 19);
    let clients = ClientMap::new(1, &clients);
    state
        .get_player_mut(1)
        .unwrap()
        .get_active_nano_mut()
        .unwrap()
        .set_stamina(25);
    assert!(nano_skill_use(request(&[10]), &clients, &mut state).is_err());
    assert_eq!((stamina(&state), hp(&state, 10)), (25, 350));
    state
        .get_player_mut(1)
        .unwrap()
        .get_active_nano_mut()
        .unwrap()
        .set_stamina(26);
    nano_skill_use(request(&[10]), &clients, &mut state).unwrap();
    assert_eq!(hp(&state, 10), 150);
    let player = state.get_player(1).unwrap();
    assert!(player.get_active_nano().is_none());
    assert_eq!(player.get_nano(7).unwrap().get_stamina(), 0);
    assert!(nano_skill_use(request(&[11]), &clients, &mut state).is_err());
    assert_eq!(hp(&state, 11), 350);
}

#[test]
fn malformed_tail_is_rejected_before_any_mutation() {
    let (mut state, clients, _) = fixture(7, 19);
    let clients = ClientMap::new(1, &clients);
    for count in [-1, 1, 10000] {
        let packet = Packet::new(
            P_CL2FE_REQ_NANO_SKILL_USE,
            &sP_CL2FE_REQ_NANO_SKILL_USE {
                iBulletID: -1,
                iArg1: 10,
                iArg2: 0,
                iArg3: 0,
                iTargetCnt: count,
            },
        )
        .unwrap();
        assert!(nano_skill_use(packet, &clients, &mut state).is_err());
        assert_eq!((stamina(&state), hp(&state, 10)), (150, 350));
    }
}

#[test]
fn target_body_extent_and_area_boundary_match_native_selection() {
    // Mob 59 is radius 60, height 220: native targeting uses extent 110.
    // Skill 21 reaches 1600 + extent, splash reaches 300 + extent.
    let (mut state, clients, _) = fixture(6, 21);
    let clients = ClientMap::new(1, &clients);
    let origin = state.get_player(1).unwrap().get_position();
    let mut focus = origin;
    focus.x += 1709;
    state.get_npc_mut(10).unwrap().set_position(focus);
    let mut edge = focus;
    edge.x += 410;
    state.get_npc_mut(11).unwrap().set_position(edge);
    assert!(nano_skill_use(request(&[10, 11]), &clients, &mut state).is_err());
    assert_eq!((stamina(&state), hp(&state, 10)), (150, 350));
    edge.x -= 1;
    state.get_npc_mut(11).unwrap().set_position(edge);
    nano_skill_use(request(&[10, 11]), &clients, &mut state).unwrap();
    assert_eq!(
        (stamina(&state), hp(&state, 10), hp(&state, 11)),
        (111, 220, 220)
    );
}
