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
