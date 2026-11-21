//! Packet round trip against isolated TableData, with no database or listening socket.
use crate::{
    chunk::{EntityMap, TickMode},
    entity::{Entity, Player, NPC},
    enums::{ItemLocation, ItemType},
    item::Item,
    net::{
        packet::{Packet, PacketID::*, *},
        ClientMap, ClientMessage, ClientMetadata, ClientType, FFClient,
    },
    state::ShardServerState,
    tabledata::{tdata_get, tdata_init},
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};
const CHILD: &str = "RUSTYFUSION_INSTANT_TEST_ROOT";
const NAME: &str =
    "server::shard::instant_mission_tests::one_click_hand_in_is_atomic_and_replay_safe";

#[test]
fn one_click_hand_in_is_atomic_and_replay_safe() {
    if let Ok(root) = std::env::var(CHILD) {
        run_child(PathBuf::from(root));
        return;
    }
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = base
        .join("target/instant-tests")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    for name in [
        "xdt.json",
        "worldnames.json",
        "paths.json",
        "NPCs.json",
        "mobs.json",
        "gruntwork.json",
        "eggs.json",
        "drops.json",
    ] {
        std::fs::copy(base.join("tabledata").join(name), root.join(name)).unwrap();
    }
    let mut doc: Value =
        serde_json::from_slice(&std::fs::read(root.join("xdt.json")).unwrap()).unwrap();
    let table = &mut doc["m_pMissionTable"];
    let reward_id = table["m_pRewardData"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["m_iMissionRewardID"].as_i64())
        .max()
        .unwrap()
        + 1;
    let mut reward = table["m_pRewardData"][1].clone();
    reward["m_iMissionRewardID"] = json!(reward_id);
    reward["m_iCash"] = json!(100);
    reward["m_iFusionMatter"] = json!(0);
    reward["m_iMissionRewardItemID"] = json!([1, 1]);
    reward["m_iMissionRewarItemType"] = json!([1, 1]);
    table["m_pRewardData"].as_array_mut().unwrap().push(reward);
    let rows = table["m_pMissionData"].as_array_mut().unwrap();
    let task_id = rows
        .iter()
        .filter_map(|r| r["m_iHTaskID"].as_i64())
        .max()
        .unwrap()
        + 1;
    let mission = (1..=1024)
        .find(|id| {
            !rows
                .iter()
                .any(|r| r["m_iHMissionID"].as_i64() == Some(*id))
        })
        .unwrap();
    let mut task = rows
        .iter()
        .find(|r| r["m_iHTaskID"] == 451)
        .unwrap()
        .clone();
    for key in [
        "m_iCSTReqMission",
        "m_iCSTRReqNano",
        "m_iCSTItemID",
        "m_iCSTItemNumNeeded",
        "m_iCSUEnemyID",
        "m_iCSUNumToKill",
        "m_iCSUItemID",
        "m_iCSUItemNumNeeded",
        "m_iSTItemID",
        "m_iSTItemNumNeeded",
        "m_iSUItem",
        "m_iSUInstancename",
        "m_iHBarkerTextID",
    ] {
        for value in task[key].as_array_mut().unwrap() {
            *value = json!(0);
        }
    }
    for key in [
        "m_iCTRReqLvMin",
        "m_iCTRReqLvMax",
        "m_iCSTReqGuide",
        "m_iCSTTrigger",
        "m_iSTGrantTimer",
        "m_iCSUCheckTimer",
        "m_iSTNanoID",
        "m_iSUOutgoingTask",
        "m_iFOutgoingTask",
        "m_iRepeatflag",
        "m_iCSUDEFNPCID",
        "m_iRequireInstanceID",
        "_ffoneRequiredBarker",
        "_ffoneRequiredBarkerNpc",
        "_ffoneRequiredBarkerSeconds",
    ] {
        task[key] = json!(0);
    }
    for (key, value) in [
        ("m_iHTaskID", task_id),
        ("m_iHMissionID", mission),
        ("m_iHMissionType", 3),
        ("m_iHTaskType", 7),
        ("m_iHNPCID", 870),
        ("m_iHTerminatorNPCID", 870),
        ("m_iSUReward", reward_id),
    ] {
        task[key] = json!(value);
    }
    task["_ffoneInventory"] = json!({"min_taros":20,"min_fm":15,"required":[{"item_type":7,"item_id":1,"count":1}],"before_finish":[{"operation":"remove","target":"inventory","item":{"item_type":7,"item_id":1,"count":1}}]});
    rows.push(task);
    std::fs::write(root.join("xdt.json"), serde_json::to_vec(&doc).unwrap()).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
        .env(CHILD, &root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
fn run_child(root: PathBuf) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/instant-tests")
        .canonicalize()
        .unwrap();
    assert!(root.canonicalize().unwrap().starts_with(base));
    crate::config::use_isolated_test_tables(&root);
    tdata_init().unwrap();
    let doc: Value =
        serde_json::from_slice(&std::fs::read(root.join("xdt.json")).unwrap()).unwrap();
    let task = doc["m_pMissionTable"]["m_pMissionData"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["m_iHTaskType"] == 7)
        .unwrap();
    let task_id = task["m_iHTaskID"].as_i64().unwrap() as i32;
    let mission = task["m_iHMissionID"].as_i64().unwrap() as i32;
    assert_eq!(
        tdata_get().get_task_definition(task_id).unwrap().task_type,
        crate::enums::TaskType::SelfCompleting
    );
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            "127.0.0.1:39021".parse().unwrap(),
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
    player.set_taros(20);
    player.set_fusion_matter(15);
    let position = player.get_position();
    let instance = player.get_instance_id();
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
    state.entity_map.track(
        Box::new(NPC::new(900_001, 870, position, 0, instance).unwrap()),
        TickMode::Always,
    );
    let clients = HashMap::from([(1, client)]);
    let request = || {
        Packet::new(
            P_CL2FE_REQ_PC_TASK_END,
            &sP_CL2FE_REQ_PC_TASK_END {
                iTaskNum: task_id,
                iNPC_ID: 900_001,
                iBox1Choice: 0,
                iBox2Choice: 0,
                iEscortNPC_ID: 0,
            },
        )
        .unwrap()
    };
    while rx.try_recv().is_ok() {}
    assert!(super::mission::task_end(request(), &ClientMap::new(1, &clients), &mut state).is_err());
    assert!(!state
        .get_player(1)
        .unwrap()
        .mission_journal
        .get_current_tasks()
        .iter()
        .any(|t| t.get_task_id() == task_id));
    state
        .get_player_mut(1)
        .unwrap()
        .set_item(
            ItemLocation::Inven,
            0,
            Some(Item::new(ItemType::General, 1)),
        )
        .unwrap();
    state.get_npc_mut(900_001).unwrap().interactable = false;
    assert!(super::mission::task_end(request(), &ClientMap::new(1, &clients), &mut state).is_err());
    state.get_npc_mut(900_001).unwrap().interactable = true;
    state.get_player_mut(1).unwrap().set_taros(19);
    assert!(super::mission::task_end(request(), &ClientMap::new(1, &clients), &mut state).is_err());
    state.get_player_mut(1).unwrap().set_taros(20);
    let size = crate::defines::SIZEOF_INVEN_SLOT as usize;
    for slot in 1..size {
        state
            .get_player_mut(1)
            .unwrap()
            .set_item(
                ItemLocation::Inven,
                slot,
                Some(Item::new(ItemType::UpperBody, 1)),
            )
            .unwrap();
    }
    let before = inventory(&state);
    assert!(super::mission::task_end(request(), &ClientMap::new(1, &clients), &mut state).is_err());
    assert_eq!(inventory(&state), before);
    assert_eq!(state.get_player(1).unwrap().get_taros(), 20);
    assert!(!state
        .get_player(1)
        .unwrap()
        .mission_journal
        .get_current_tasks()
        .iter()
        .any(|t| t.get_task_id() == task_id));
    while rx.try_recv().is_ok() {}
    state
        .get_player_mut(1)
        .unwrap()
        .set_item(ItemLocation::Inven, 1, None)
        .unwrap();
    super::mission::task_end(request(), &ClientMap::new(1, &clients), &mut state).unwrap();
    assert_eq!(state.get_player(1).unwrap().get_taros(), 120);
    assert!(state
        .get_player(1)
        .unwrap()
        .mission_journal
        .is_mission_completed(mission)
        .unwrap());
    assert_eq!(state.get_player(1).unwrap().get_quest_item_count(1), 0);
    let mut end = false;
    while let Ok(ClientMessage::SendPacket(packet)) = rx.try_recv() {
        if packet.id() == P_FE2CL_REP_PC_TASK_START_SUCC {
            assert_ne!(
                packet
                    .get::<sP_FE2CL_REP_PC_TASK_START_SUCC>()
                    .unwrap()
                    .iTaskNum,
                task_id
            );
        }
        end |= packet.id() == P_FE2CL_REP_PC_TASK_END_SUCC;
    }
    assert!(end);
    assert!(super::mission::task_end(request(), &ClientMap::new(1, &clients), &mut state).is_err());
    assert_eq!(state.get_player(1).unwrap().get_taros(), 120);
}
fn inventory(state: &ShardServerState) -> Vec<Option<Item>> {
    (0..crate::defines::SIZEOF_INVEN_SLOT as usize)
        .map(|i| {
            *state
                .get_player(1)
                .unwrap()
                .get_item(ItemLocation::Inven, i)
                .unwrap()
        })
        .collect()
}
