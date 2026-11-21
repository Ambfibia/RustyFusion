use super::{group, mission};
use crate::{
    chunk::{InstanceID, TickMode},
    entity::{Entity, EntityID, Player, NPC},
    mission::Task,
    net::{
        packet::{Packet, PacketID::*, *},
        ClientMap, ClientMessage, ClientMetadata, ClientType, FFClient,
    },
    state::ShardServerState,
    tabledata::{tdata_get, tdata_init},
    Position,
};
use std::collections::HashMap;

const NPC_ID: i32 = 2_000_000_941;
const TARGET_ID: i32 = 2_000_000_942;
const BASE: Position = Position {
    x: 200_000,
    y: 200_000,
    z: 100,
};

fn fixture() -> (
    ShardServerState,
    HashMap<usize, FFClient>,
    tokio::sync::mpsc::UnboundedReceiver<ClientMessage>,
) {
    tdata_init().unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            "127.0.0.1:39441".parse().unwrap(),
            Some(ClientType::GameClient {
                account_id: 1,
                serial_key: 1,
                pc_id: Some(1),
            }),
        ),
    );
    let mut state = ShardServerState::default();
    let mut player = Player::new(1, 0);
    player.set_player_id(1);
    player.set_level(36).unwrap();
    player.instance_id = InstanceID {
        map_num: 143,
        ..Default::default()
    };
    player.set_position(BASE);
    player.set_client(client.clone());
    let previous: Task = tdata_get().get_task_definition(5208).unwrap().into();
    player.mission_journal.start_task(previous, 36).unwrap();
    player.mission_journal.complete_task(5208).unwrap();
    let chunk = player.get_chunk_coords();
    let eid = state.entity_map.track(Box::new(player), TickMode::Always);
    state.entity_map.update(eid, Some(chunk), false);
    for (id, ty, offset) in [(NPC_ID, 2567, 0), (TARGET_ID, 1588, 2000)] {
        let npc = NPC::new(
            id,
            ty,
            Position {
                x: BASE.x + offset,
                ..BASE
            },
            0,
            state.get_player(1).unwrap().instance_id,
        )
        .unwrap();
        let chunk = npc.get_chunk_coords();
        let eid = state.entity_map.track(Box::new(npc), TickMode::Never);
        state.entity_map.update(eid, Some(chunk), false);
    }
    (state, HashMap::from([(1, client)]), rx)
}

fn begin(state: &mut ShardServerState, clients: &HashMap<usize, FFClient>, task: i32) {
    mission::task_start(
        Packet::new(
            P_CL2FE_REQ_PC_TASK_START,
            &sP_CL2FE_REQ_PC_TASK_START {
                iTaskNum: task,
                iNPC_ID: 0,
                iEscortNPC_ID: NPC_ID,
            },
        )
        .unwrap(),
        &clients[&1],
        state,
    )
    .unwrap();
}

fn end(
    state: &mut ShardServerState,
    clients: &HashMap<usize, FFClient>,
    task: i32,
    target: i32,
) -> crate::error::FFResult<()> {
    mission::task_end(
        Packet::new(
            P_CL2FE_REQ_PC_TASK_END,
            &sP_CL2FE_REQ_PC_TASK_END {
                iTaskNum: task,
                iNPC_ID: target,
                iEscortNPC_ID: NPC_ID,
                iBox1Choice: 0,
                iBox2Choice: 0,
            },
        )
        .unwrap(),
        &ClientMap::new(1, clients),
        state,
    )
}

fn move_player(state: &mut ShardServerState, pos: Position) {
    let player = state.get_player_mut(1).unwrap();
    player.set_position(pos);
    let chunk = player.get_chunk_coords();
    state
        .entity_map
        .update(EntityID::Player(1), Some(chunk), false);
}

#[test]
fn eduardo_goto_trigger_starts_outgoing_escort_before_interaction_range() {
    let (mut state, clients, mut rx) = fixture();
    let player = state.get_player_mut(1).unwrap();
    player.mission_journal.remove_task(5208).unwrap();
    player.set_level(3).unwrap();
    player.instance_id.map_num = 11;
    player.unlock_nano(3).unwrap();
    player
        .mission_journal
        .start_task(tdata_get().get_task_definition(575).unwrap().into(), 3)
        .unwrap();
    let instance = player.instance_id;
    for (id, ty, offset) in [(NPC_ID, 1007, 0), (TARGET_ID, 1114, -40)] {
        state.entity_map.untrack(EntityID::NPC(id));
        let npc = NPC::new(
            id,
            ty,
            Position {
                x: BASE.x + offset,
                ..BASE
            },
            0,
            instance,
        )
        .unwrap();
        let chunk = npc.get_chunk_coords();
        let eid = state.entity_map.track(Box::new(npc), TickMode::Never);
        state.entity_map.update(eid, Some(chunk), false);
    }
    let sight = tdata_get().get_npc_stats(1114).unwrap().sight_range;
    assert_eq!(sight, 1500);
    move_player(
        &mut state,
        Position {
            x: BASE.x + 1400,
            ..BASE
        },
    );
    // The real trigger is within its 1500-unit sight radius, but Eduardo is
    // still outside RANGE_INTERACT (800). END and outgoing START both succeed.
    end(&mut state, &clients, 575, TARGET_ID).unwrap();
    // Trigger range remains bounded; an outgoing chain is not permission to
    // start following an NPC from anywhere in the instance.
    move_player(
        &mut state,
        Position {
            x: BASE.x + 2000,
            ..BASE
        },
    );
    assert!(mission::task_start(
        Packet::new(
            P_CL2FE_REQ_PC_TASK_START,
            &sP_CL2FE_REQ_PC_TASK_START {
                iTaskNum: 576,
                iNPC_ID: 0,
                iEscortNPC_ID: NPC_ID,
            }
        )
        .unwrap(),
        &clients[&1],
        &mut state,
    )
    .is_err());
    assert!(state.get_npc(NPC_ID).unwrap().mission_escort.is_none());
    while rx.try_recv().is_ok() {}
    move_player(
        &mut state,
        Position {
            x: BASE.x + 1400,
            ..BASE
        },
    );
    begin(&mut state, &clients, 576);
    assert!(state
        .get_player(1)
        .unwrap()
        .mission_journal
        .get_current_tasks()
        .iter()
        .any(|task| task.get_task_id() == 576 && !task.completed && !task.failed));
    assert_eq!(
        state
            .get_npc(NPC_ID)
            .unwrap()
            .mission_escort
            .as_ref()
            .unwrap()
            .task_id,
        576
    );
    assert!(!std::iter::from_fn(|| rx.try_recv().ok())
        .any(|msg| matches!(msg, ClientMessage::SendPacket(p) if p.id() == P_FE2CL_REP_PC_TASK_START_FAIL)));
}

#[test]
fn escort_group_start_zero_speed_follow_chain_and_delivery_round_trip() {
    let (mut state, clients, mut rx) = fixture();
    let stats = tdata_get().get_npc_stats(2567).unwrap();
    assert_eq!(
        (stats.ai_type, stats.walk_speed, stats.run_speed),
        (0, 0, 0)
    );
    group::npc_group_invite(
        Packet::new(
            P_CL2FE_REQ_NPC_GROUP_INVITE,
            &sP_CL2FE_REQ_NPC_GROUP_INVITE { iNPC_ID: NPC_ID },
        )
        .unwrap(),
        &ClientMap::new(1, &clients),
        &mut state,
    )
    .unwrap();
    // Membership alone must leave this otherwise stationary NPC in place.
    NPC::tick(&mut state, NPC_ID);
    assert_eq!(state.get_npc(NPC_ID).unwrap().get_position(), BASE);
    begin(&mut state, &clients, 5209);
    assert!(state.get_npc(NPC_ID).unwrap().mission_escort.is_some());
    move_player(
        &mut state,
        Position {
            x: BASE.x + 1200,
            ..BASE
        },
    );
    for _ in 0..20 {
        NPC::tick(&mut state, NPC_ID);
    }
    let pos = state.get_npc(NPC_ID).unwrap().get_position();
    assert!(pos.x > BASE.x + 800);
    assert!(pos.distance_to(&state.get_player(1).unwrap().get_position()) <= 250);
    assert!(std::iter::from_fn(|| rx.try_recv().ok())
        .any(|msg| matches!(msg, ClientMessage::SendPacket(p) if p.id() == P_FE2CL_NPC_MOVE)));
    // Complete the kill objective using the journal's normal kill update.
    state
        .get_player_mut(1)
        .unwrap()
        .mission_journal
        .mark_enemy_defeated(344);
    end(&mut state, &clients, 5209, 0).unwrap();
    begin(&mut state, &clients, 5211);
    move_player(
        &mut state,
        Position {
            x: BASE.x + 2000,
            ..BASE
        },
    );
    assert!(
        end(&mut state, &clients, 5211, TARGET_ID).is_err(),
        "lagging escort must block completion"
    );
    for _ in 0..20 {
        NPC::tick(&mut state, NPC_ID);
    }
    end(&mut state, &clients, 5211, TARGET_ID).unwrap();
    assert!(state.get_npc(NPC_ID).unwrap().mission_escort.is_none());
    assert!(state
        .get_player(1)
        .unwrap()
        .mission_journal
        .get_current_tasks()
        .iter()
        .any(|task| task.get_task_id() == 5211 && task.completed));
}

#[test]
fn completed_escort_cannot_fail_while_waiting_for_outgoing_task_start() {
    let (mut state, clients, mut rx) = fixture();
    state.get_player_mut(1).unwrap().set_hp(1000);
    begin(&mut state, &clients, 5209);
    state.get_player_mut(1).unwrap().mission_journal.mark_enemy_defeated(344);
    end(&mut state, &clients, 5209, 0).unwrap();
    assert!(state.get_npc(NPC_ID).unwrap().mission_escort.is_none());
    // Successful completion releases its escort before the client can send
    // TASK_START for the next stage. A world tick in that gap must preserve
    // the committed success rather than publish EscortFailed for the release.
    while rx.try_recv().is_ok() {}
    Player::tick(&mut state, 1, &std::time::SystemTime::now());
    let task = state.get_player(1).unwrap().mission_journal.get_current_tasks()
        .into_iter().find(|task| task.get_task_id() == 5209).unwrap();
    assert!(task.completed);
    assert!(!task.failed, "successful escort completion must not be reclassified as failure");
    assert!(!std::iter::from_fn(|| rx.try_recv().ok())
        .any(|msg| matches!(msg, ClientMessage::SendPacket(packet) if packet.id() == P_FE2CL_REP_PC_TASK_END_FAIL)));
    begin(&mut state, &clients, 5211);
}

#[test]
fn failed_escort_publishes_its_terminal_failure_once() {
    let (mut state, clients, mut rx) = fixture();
    state.get_player_mut(1).unwrap().set_hp(1000);
    begin(&mut state, &clients, 5209);
    crate::escort::release(&mut state, NPC_ID, true);
    while rx.try_recv().is_ok() {}
    Player::tick(&mut state, 1, &std::time::SystemTime::now());
    let failures = std::iter::from_fn(|| rx.try_recv().ok())
        .filter(|msg| matches!(msg, ClientMessage::SendPacket(packet) if packet.id() == P_FE2CL_REP_PC_TASK_END_FAIL)).count();
    assert_eq!(failures, 1);
    Player::tick(&mut state, 1, &std::time::SystemTime::now());
    assert!(!std::iter::from_fn(|| rx.try_recv().ok())
        .any(|msg| matches!(msg, ClientMessage::SendPacket(packet) if packet.id() == P_FE2CL_REP_PC_TASK_END_FAIL)));
    let task = state.get_player(1).unwrap().mission_journal.get_current_tasks()
        .into_iter().find(|task| task.get_task_id() == 5209).unwrap();
    assert!(task.failed);
    assert!(!task.completed);
}

#[test]
fn authored_self_success_and_failure_loops_restart_only_after_terminal_state() {
    for (id, failed) in [(5064, true), (5176, false)] {
        let (mut state, clients, mut rx) = fixture();
        let player = state.get_player_mut(1).unwrap();
        player.mission_journal.remove_task(5208).unwrap(); player.instance_id.map_num = 0;
        let definition = tdata_get().get_task_definition(id).unwrap();
        let mut task: Task = definition.into(); task.failed = failed; task.completed = !failed;
        player.mission_journal.start_editor_task(task, 36).unwrap();
        while rx.try_recv().is_ok() {}
        begin(&mut state, &clients, id);
        let task = state.get_player(1).unwrap().mission_journal.get_current_tasks().into_iter().find(|task| task.get_task_id() == id).unwrap();
        assert!(!task.failed && !task.completed); assert!(task.fail_time.is_some());
        assert_eq!(task.remaining_enemy_defeats, definition.obj_enemies);
        assert!(std::iter::from_fn(|| rx.try_recv().ok()).any(|msg| matches!(msg, ClientMessage::SendPacket(packet) if packet.id() == P_FE2CL_REP_PC_TASK_START_SUCC)));
        let enemy = *definition.obj_enemies.keys().next().unwrap();
        state.get_player_mut(1).unwrap().mission_journal.mark_enemy_defeated(enemy);
        let expected = state.get_player(1).unwrap().mission_journal.get_current_tasks().into_iter().find(|task| task.get_task_id() == id).unwrap().remaining_enemy_defeats;
        begin(&mut state, &clients, id);
        assert_eq!(state.get_player(1).unwrap().mission_journal.get_current_tasks().into_iter().find(|task| task.get_task_id() == id).unwrap().remaining_enemy_defeats, expected);
    }
}

#[test]
fn escort_cancellation_instance_departure_and_teleport_restore_idle_npc() {
    for mode in 0..3 {
        let (mut state, clients, _) = fixture();
        begin(&mut state, &clients, 5209);
        move_player(
            &mut state,
            Position {
                x: BASE.x + 1000,
                ..BASE
            },
        );
        for _ in 0..4 {
            NPC::tick(&mut state, NPC_ID);
        }
        assert_ne!(state.get_npc(NPC_ID).unwrap().get_position(), BASE);
        match mode {
            0 => {
                state
                    .get_player_mut(1)
                    .unwrap()
                    .mission_journal
                    .remove_task(5209)
                    .unwrap();
            }
            1 => {
                state.get_player_mut(1).unwrap().instance_id.map_num = 0;
            }
            _ => move_player(
                &mut state,
                Position {
                    x: BASE.x + 10_000,
                    ..BASE
                },
            ),
        }
        NPC::tick(&mut state, NPC_ID);
        let npc = state.get_npc(NPC_ID).unwrap();
        assert!(npc.mission_escort.is_none());
        assert!(npc.ai.is_none());
        assert!(npc.path.is_none());
        assert_eq!(npc.get_position(), BASE);
    }
}

#[test]
fn escort_delivery_and_defence_classification_matches_reference() {
    tdata_init().unwrap();
    for id in [576, 5209, 5211, 5229, 5236, 5237] {
        assert!(
            tdata_get()
                .get_task_definition(id)
                .unwrap()
                .escort_follows_player
        );
    }
    for id in [1753, 1038, 872] {
        assert!(
            !tdata_get()
                .get_task_definition(id)
                .unwrap()
                .escort_follows_player
        );
    }
}

#[test]
fn escort_suspends_and_restores_existing_authored_route() {
    let (mut state, clients, _) = fixture();
    let target = Position {
        y: BASE.y + 3000,
        ..BASE
    };
    state.get_npc_mut(NPC_ID).unwrap().path = Some(crate::path::Path::new_single(target, 300));
    begin(&mut state, &clients, 5209);
    assert!(state.get_npc(NPC_ID).unwrap().path.is_none());
    move_player(
        &mut state,
        Position {
            x: BASE.x + 1000,
            ..BASE
        },
    );
    for _ in 0..4 {
        NPC::tick(&mut state, NPC_ID);
    }
    assert_eq!(state.get_npc(NPC_ID).unwrap().get_position().y, BASE.y);
    state
        .get_player_mut(1)
        .unwrap()
        .mission_journal
        .remove_task(5209)
        .unwrap();
    NPC::tick(&mut state, NPC_ID);
    let path = state.get_npc(NPC_ID).unwrap().path.as_ref().unwrap();
    assert_eq!(path.get_target_pos(), target);
    assert!(!path.is_started());
}


#[test]
fn eduardo_defends_the_leader_only_against_an_in_range_mob_and_stays_on_delivery() {
    use crate::entity::Combatant;
    let (mut state, _clients, mut rx) = fixture();
    let player = state.get_player_mut(1).unwrap();
    player.instance_id.map_num = 11;
    let instance = player.instance_id;
    player.mission_journal.remove_task(5208).unwrap();
    player.set_level(3).unwrap();
    player.unlock_nano(3).unwrap();
    assert!(player.mission_journal.start_task(tdata_get().get_task_definition(576).unwrap().into(), 3).unwrap());
    let player_chunk = player.get_chunk_coords();
    state.entity_map.update(EntityID::Player(1), Some(player_chunk), false);
    state.entity_map.untrack(EntityID::NPC(NPC_ID));
    const MOB: i32 = TARGET_ID + 1;
    for (id, ty, x) in [(NPC_ID, 1007, 0), (MOB, 59, 200)] {
        let npc = NPC::new(id, ty, Position { x: BASE.x+x, ..BASE }, 0, instance).unwrap();
        let chunk = npc.get_chunk_coords();
        let key = state.entity_map.track(Box::new(npc), TickMode::Never);
        state.entity_map.update(key, Some(chunk), false);
    }
    state.get_player_mut(1).unwrap().target = Some(EntityID::NPC(MOB));
    crate::escort::start(&mut state, NPC_ID, 1, 576);
    assert!(tdata_get().get_task_definition(576).unwrap().escort_fights, "authored AI 6");
    assert!(state.get_npc(NPC_ID).unwrap().mission_escort.as_ref().unwrap().fights);
    assert!(state.get_combatant(EntityID::NPC(NPC_ID)).is_ok(), "escort registered for combat");
    assert!(state.get_combatant(EntityID::NPC(MOB)).is_ok(), "hostile registered for combat");
    let hp = state.get_npc(MOB).unwrap().get_hp();
    assert!(hp>0 && !state.get_npc(MOB).unwrap().invulnerable, "live vulnerable mob hp={hp}");
    assert!(state.get_player(1).unwrap().mission_journal.get_current_tasks().iter().any(|t|t.get_task_id()==576 && !t.failed && !t.completed), "escort task active");
    assert_eq!(state.get_player(1).unwrap().get_position(), BASE);
    let _ = rx.try_recv();
    crate::escort::tick(&mut state, NPC_ID);
    let after = state.get_npc(MOB).unwrap().get_hp();
    assert!(after < hp);
    assert!(std::iter::from_fn(|| rx.try_recv().ok())
        .any(|m| matches!(m, ClientMessage::SendPacket(p) if p.id()==P_FE2CL_NPC_ATTACK_CHARs)));
    crate::escort::tick(&mut state, NPC_ID);
    assert_eq!(state.get_npc(MOB).unwrap().get_hp(), after, "authored attack delay");
    move_player(&mut state, Position { x: BASE.x+1200, ..BASE });
    state.get_player_mut(1).unwrap().target = None;
    for _ in 0..12 { crate::escort::tick(&mut state, NPC_ID); }
    let arrived=state.get_npc(NPC_ID).unwrap().get_position();
    assert!(arrived.x > BASE.x+500);
    crate::escort::release(&mut state, NPC_ID, false);
    assert_eq!(state.get_npc(NPC_ID).unwrap().get_position(), arrived);
}
