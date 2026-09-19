use std::collections::{HashMap, HashSet};

use crate::{
    chunk::{EntityMap, TickMode},
    entity::{Combatant, Player},
    net::{
        packet::{Packet, PacketID::*, *},
        ClientMap, ClientMessage, ClientMetadata, ClientType, FFClient,
    },
    server::shard::gm::gm_pc_give_nano,
    state::ShardServerState,
    tabledata::{tdata_get, tdata_init},
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
            "127.0.0.1:39003".parse().unwrap(),
            Some(ClientType::GameClient {
                account_id: 1,
                serial_key: 1,
                pc_id: Some(1),
            }),
        ),
    );
    let mut player = Player::new(1, 0);
    player.set_player_id(1);
    player.perms = 30;
    player.set_level(5).unwrap();
    player.set_fusion_matter(123);
    player.set_client(client.clone());
    let mut state = ShardServerState {
        shard_id: None,
        login_server_conn_id: None,
        login_data: HashMap::new(),
        entity_map: EntityMap::default(),
        buyback_lists: HashMap::new(),
        ongoing_trades: HashMap::new(),
        groups: HashMap::new(),
        player_uid_to_id: HashMap::new(),
        pending_entering_uids: HashSet::new(),
        pending_buff_effects: Vec::new(),
        ongoing_races: HashMap::new(),
    };
    state.entity_map.track(Box::new(player), TickMode::Always);
    (state, HashMap::from([(1, client)]), rx)
}

#[test]
fn gm_nano_is_ownership_not_level_and_repeat_is_unchanged() {
    let (mut state, clients, mut rx) = fixture();
    let request = || {
        Packet::new(
            P_CL2FE_REQ_PC_GIVE_NANO,
            &sP_CL2FE_REQ_PC_GIVE_NANO { iNanoID: 46 },
        )
        .unwrap()
    };
    gm_pc_give_nano(request(), &ClientMap::new(1, &clients), &mut state).unwrap();
    let player = state.get_player(1).unwrap();
    assert_eq!(player.get_level(), 5);
    assert_eq!(player.get_fusion_matter(), 123);
    assert!(player.get_nano(46).is_some());
    let ClientMessage::SendPacket(reply) = rx.try_recv().unwrap() else {
        panic!("missing reply")
    };
    assert_eq!(reply.id(), P_FE2CL_REP_PC_NANO_CREATE_SUCC);
    assert_eq!(
        {
            reply
                .get::<sP_FE2CL_REP_PC_NANO_CREATE_SUCC>()
                .unwrap()
                .iPC_Level
        },
        5
    );
    assert!(gm_pc_give_nano(request(), &ClientMap::new(1, &clients), &mut state).is_err());
    assert_eq!(state.get_player(1).unwrap().get_level(), 5);
    assert_eq!(state.get_player(1).unwrap().get_fusion_matter(), 123);
    let ClientMessage::SendPacket(reply) = rx.try_recv().unwrap() else {
        panic!("missing reply")
    };
    assert_eq!(reply.id(), P_FE2CL_REP_PC_NANO_CREATE_FAIL);
    assert_eq!(
        {
            reply
                .get::<sP_FE2CL_REP_PC_NANO_CREATE_FAIL>()
                .unwrap()
                .iErrorCode
        },
        2
    );
}

#[test]
fn gm_nano_50_obeys_loaded_content_without_progression() {
    let (mut state, clients, mut rx) = fixture();
    let known = tdata_get().get_nano_stats(50).is_ok();
    let request = Packet::new(
        P_CL2FE_REQ_PC_GIVE_NANO,
        &sP_CL2FE_REQ_PC_GIVE_NANO { iNanoID: 50 },
    )
    .unwrap();
    assert_eq!(
        gm_pc_give_nano(request, &ClientMap::new(1, &clients), &mut state).is_ok(),
        known
    );
    let player = state.get_player(1).unwrap();
    assert_eq!(player.get_level(), 5);
    assert_eq!(player.get_fusion_matter(), 123);
    assert_eq!(player.get_nano(50).is_some(), known);
    let ClientMessage::SendPacket(reply) = rx.try_recv().unwrap() else {
        panic!("missing reply")
    };
    assert_eq!(
        reply.id(),
        if known {
            P_FE2CL_REP_PC_NANO_CREATE_SUCC
        } else {
            P_FE2CL_REP_PC_NANO_CREATE_FAIL
        }
    );
    if !known {
        assert_eq!(
            {
                reply
                    .get::<sP_FE2CL_REP_PC_NANO_CREATE_FAIL>()
                    .unwrap()
                    .iErrorCode
            },
            1
        );
    }
}

fn prepare_task(state: &mut ShardServerState, task_id: i32, level: i16, fm: u32) {
    use crate::mission::Task;
    let def = tdata_get().get_task_definition(task_id).unwrap();
    let player = state.get_player_mut(1).unwrap();
    player.set_level(level).unwrap();
    // Avoid invoking auto-start while constructing a completion fixture.
    let mut task: Task = def.into();
    for count in task.remaining_enemy_defeats.values_mut() {
        *count = 0;
    }
    assert!(player.mission_journal.start_task(task, level).unwrap());
    player.set_fusion_matter(fm);
    for (&id, &count) in &def.obj_qitems {
        player.set_quest_item_count(id, count).unwrap();
    }
    assert!(def.obj_npc_type.is_none(), "fixture needs an NPC");
}

fn end_task(
    state: &mut ShardServerState,
    clients: &HashMap<usize, FFClient>,
    task_id: i32,
) -> crate::error::FFResult<()> {
    super::mission::task_end(
        Packet::new(
            P_CL2FE_REQ_PC_TASK_END,
            &sP_CL2FE_REQ_PC_TASK_END {
                iTaskNum: task_id,
                iNPC_ID: 0,
                iBox1Choice: 0,
                iBox2Choice: 0,
                iEscortNPC_ID: 0,
            },
        )
        .unwrap(),
        &ClientMap::new(1, clients),
        state,
    )
}

#[test]
fn growth_cost_and_level_are_once_even_when_nano_was_given_by_gm() {
    for already_owned in [false, true] {
        let (mut state, clients, _) = fixture();
        let cost = tdata_get().get_player_stats(5).unwrap().req_fm_nano_create;
        prepare_task(&mut state, 833, 5, cost + 17);
        if already_owned {
            state
                .get_player_mut(1)
                .unwrap()
                .unlock_nano(6)
                .unwrap()
                .tune(Some(7));
        }
        end_task(&mut state, &clients, 833).unwrap();
        let player = state.get_player(1).unwrap();
        assert_eq!(player.get_level(), 6);
        assert_eq!(player.get_fusion_matter(), 17);
        assert!(player.get_nano(6).is_some());
        if already_owned {
            assert_eq!(player.get_nano(6).unwrap().selected_skill, Some(7));
        }
        assert!(player.mission_journal.is_mission_completed(538).unwrap());
        assert!(end_task(&mut state, &clients, 833).is_err());
        let player = state.get_player_mut(1).unwrap();
        assert_eq!(player.get_level(), 6);
        assert_eq!(player.get_fusion_matter(), 17);
        player.set_fusion_matter(cost + 17);
        assert!(!player
            .mission_journal
            .get_current_tasks()
            .iter()
            .any(|t| t.get_task_def().mission_id == 538));
    }
}

#[test]
fn insufficient_growth_fm_retains_task_items_and_reward() {
    let (mut state, clients, _) = fixture();
    prepare_task(&mut state, 833, 5, 1);
    assert!(end_task(&mut state, &clients, 833).is_err());
    let player = state.get_player(1).unwrap();
    assert_eq!((player.get_level(), player.get_fusion_matter()), (5, 1));
    assert!(player.get_nano(6).is_none());
    assert!(!player.mission_journal.is_mission_completed(538).unwrap());
    assert!(!player.mission_journal.get_current_tasks()[0].completed);
}

#[test]
fn growth_with_surplus_fm_starts_the_next_tier_not_the_completed_mission() {
    let (mut state, clients, _) = fixture();
    let cost = tdata_get().get_player_stats(5).unwrap().req_fm_nano_create;
    let next = tdata_get().get_player_stats(6).unwrap();
    prepare_task(&mut state, 833, 5, cost + next.req_fm_nano_create);
    end_task(&mut state, &clients, 833).unwrap();
    let player = state.get_player(1).unwrap();
    assert_eq!(player.get_level(), 6);
    assert_eq!(player.get_fusion_matter(), next.req_fm_nano_create);
    let tasks = player.mission_journal.get_current_tasks();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].get_task_id(), next.nano_quest_task_id.unwrap());
    assert_ne!(tasks[0].get_task_def().mission_id, 538);
}

#[test]
fn non_growth_nano_keeps_level_and_fm_and_respects_level_requirement() {
    for level in [27, 28] {
        let (mut state, clients, _) = fixture();
        prepare_task(&mut state, 5237, level, 123);
        assert_eq!(end_task(&mut state, &clients, 5237).is_ok(), level == 28);
        let player = state.get_player(1).unwrap();
        assert_eq!(
            (player.get_level(), player.get_fusion_matter()),
            (level, 123)
        );
        assert_eq!(player.get_nano(43).is_some(), level == 28);
        assert_eq!(
            player.mission_journal.is_mission_completed(843).unwrap(),
            level == 28
        );
    }
}

#[test]
fn unknown_nano_cannot_mutate_ownership() {
    let (mut state, _, _) = fixture();
    let player = state.get_player_mut(1).unwrap();
    assert!(player.unlock_nano(i16::MAX).is_err());
    assert!(player.get_nano(i16::MAX).is_none());
}

#[test]
fn completed_growth_mission_does_not_restart_on_fm() {
    let (mut state, _, mut rx) = fixture();
    let player = state.get_player_mut(1).unwrap();
    let stats = tdata_get().get_player_stats(5).unwrap();
    let task = tdata_get()
        .get_task_definition(stats.nano_quest_task_id.unwrap())
        .unwrap();
    player
        .mission_journal
        .set_mission_completed(task.mission_id)
        .unwrap();
    player.set_fusion_matter(stats.req_fm_nano_create + 1);
    assert!(!player.mission_journal.has_nano_mission());
    assert!(
        rx.try_recv().is_err(),
        "must not advertise a rejected start"
    );
    assert!(!player.mission_journal.start_task(task.into(), 5).unwrap());
}

#[test]
fn nano_step_requires_items_and_repeated_end_does_not_consume_again() {
    let (mut state, clients, _) = fixture();
    prepare_task(&mut state, 832, 5, 123);
    state
        .get_player_mut(1)
        .unwrap()
        .set_quest_item_count(222, 0)
        .unwrap();
    assert!(end_task(&mut state, &clients, 832).is_err());
    assert!(
        !state
            .get_player(1)
            .unwrap()
            .mission_journal
            .get_current_tasks()[0]
            .completed
    );
    state
        .get_player_mut(1)
        .unwrap()
        .set_quest_item_count(222, 1)
        .unwrap();
    end_task(&mut state, &clients, 832).unwrap();
    assert_eq!(state.get_player(1).unwrap().get_quest_item_count(222), 0);
    // Reacquiring an item after completing the step must not let a duplicate
    // TASK_END consume it or issue the step's reward again.
    state
        .get_player_mut(1)
        .unwrap()
        .set_quest_item_count(222, 1)
        .unwrap();
    assert!(end_task(&mut state, &clients, 832).is_err());
    let player = state.get_player(1).unwrap();
    assert_eq!(player.get_quest_item_count(222), 1);
    assert_eq!((player.get_level(), player.get_fusion_matter()), (5, 123));
    assert!(player.get_nano(6).is_none());
}

#[test]
fn repeated_active_nano_start_does_not_replace_progress() {
    let (mut state, clients, mut rx) = fixture();
    prepare_task(&mut state, 832, 5, 123);
    let request = Packet::new(
        P_CL2FE_REQ_PC_TASK_START,
        &sP_CL2FE_REQ_PC_TASK_START {
            iTaskNum: 832,
            iNPC_ID: 0,
            iEscortNPC_ID: 0,
        },
    )
    .unwrap();
    super::mission::task_start(request, clients.get(&1).unwrap(), &mut state).unwrap();
    let player = state.get_player(1).unwrap();
    assert_eq!(player.get_quest_item_count(222), 1);
    assert_eq!(player.mission_journal.get_current_tasks().len(), 1);
    assert_eq!((player.get_level(), player.get_fusion_matter()), (5, 123));
    assert!(rx.try_recv().is_err());
}
