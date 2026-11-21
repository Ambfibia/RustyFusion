//! Isolated native branches, private route ownership, reset and reward replay.
use crate::{
    chunk::{EntityMap, TickMode},
    entity::{Entity, Player, NPC},
    mission::Task,
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
const ENV: &str = "RUSTYFUSION_STAGE_TEST_ROOT";
const NAME: &str =
    "server::shard::mission_stage_tests::mission_stage_branch_routes_are_private_and_resettable";
#[test]
fn mission_stage_branch_routes_are_private_and_resettable() {
    if let Ok(root) = std::env::var(ENV) {
        run(PathBuf::from(root));
        return;
    }
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = base
        .join("target/mission-stage-tests")
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
    let speaker = doc["m_pNpcTable"]["m_pNpcData"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|npc| npc["m_iNpcNumber"] == 870)
        .unwrap();
    speaker["_ffoneNpcSpeech"] = json!({"use_click":true,"use_background":true,"click":[{"id":1000777,"en":"Native click","ru":"При нажатии","conditions":{"inventory":{"min_taros":50}}}],"background":[{"id":1000778,"en":"Native background","ru":"Фоновая","conditions":{"min_level":2}}]});
    let npc_doc: Value =
        serde_json::from_slice(&std::fs::read(root.join("NPCs.json")).unwrap()).unwrap();
    let mut ids: Vec<_> = npc_doc["NPCs"]
        .as_object()
        .unwrap()
        .keys()
        .filter_map(|id| id.parse::<i32>().ok().and_then(|id| id.checked_add(1)))
        .collect();
    ids.sort_unstable();
    let mut paths: Value =
        serde_json::from_slice(&std::fs::read(root.join("paths.json")).unwrap()).unwrap();
    paths["npc"]["999999"] = json!({"aNPCTypes":[],"aNPCIDs":[],"iBaseSpeed":300,"bLoop":false,"aPoints":[{"iX":0,"iY":0,"iZ":0,"iStopTicks":0},{"iX":100,"iY":0,"iZ":0,"iStopTicks":0}]});
    std::fs::write(root.join("paths.json"), serde_json::to_vec(&paths).unwrap()).unwrap();
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
    reward["m_iMissionRewardItemID"] = json!([]);
    reward["m_iMissionRewarItemType"] = json!([]);
    table["m_pRewardData"].as_array_mut().unwrap().push(reward);
    let rows = table["m_pMissionData"].as_array_mut().unwrap();
    let mut task = rows
        .iter()
        .find(|r| r["m_iHTaskID"] == 451)
        .unwrap()
        .clone();
    let id = rows
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
        for v in task[key].as_array_mut().unwrap() {
            *v = json!(0);
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
    task["m_iHTaskID"] = json!(id);
    task["m_iHMissionID"] = json!(mission);
    task["m_iHMissionType"] = json!(3);
    task["m_iHTaskType"] = json!(1);
    task["m_iHNPCID"] = json!(2555);
    task["m_iHTerminatorNPCID"] = json!(2555);
    task["m_iSUOutgoingTask"] = json!(id + 1);
    task["m_iSUReward"] = json!(reward_id);
    task["_ffoneMissionStage"] = json!({"completions":[{"npc":2555,"next_task":id+1},{"npc":870,"next_task":id+2,"conditions":{"inventory":{"min_taros":100}}},{"npc":707,"next_task":0,"conditions":{"min_level":4}},{"npc":708,"next_task":0}],"before":[{"npc_id":ids[0],"route":"999999","finish":"replace","replacement_npc":708}],"visibility":[{"npc_ids":[ids[1]],"visible":false,"conditions":{"inventory":{"min_taros":50}}}]});
    let template = task.clone();
    rows.push(task);
    for i in 1..=2 {
        let mut next = template.clone();
        next.as_object_mut().unwrap().remove("_ffoneMissionStage");
        next["m_iHTaskID"] = json!(id + i);
        next["m_iSUOutgoingTask"] = json!(0);
        next["m_iHNPCID"] = json!(0);
        next["m_iSUReward"] = json!(0);
        rows.push(next);
    }
    std::fs::write(root.join("xdt.json"), serde_json::to_vec(&doc).unwrap()).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", NAME, "--nocapture", "--test-threads=1"])
        .env(ENV, &root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn run(root: PathBuf) {
    assert!(root.canonicalize().unwrap().starts_with(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/mission-stage-tests")
            .canonicalize()
            .unwrap()
    ));
    crate::config::use_isolated_test_tables(&root);
    tdata_init().unwrap();
    let doc: Value =
        serde_json::from_slice(&std::fs::read(root.join("xdt.json")).unwrap()).unwrap();
    let row = doc["m_pMissionTable"]["m_pMissionData"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r.get("_ffoneMissionStage").is_some())
        .unwrap();
    let id = row["m_iHTaskID"].as_i64().unwrap() as i32;
    let definition = tdata_get().get_task_definition(id).unwrap();
    let source = definition.stage.before[0].npc_id;
    let second = definition.stage.visibility[0].npc_ids[0];
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            "127.0.0.1:39031".parse().unwrap(),
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
    for (n, ty) in [2555, 870, 707, 708].into_iter().enumerate() {
        state.entity_map.track(
            Box::new(NPC::new(900001 + n as i32, ty, position, 0, instance).unwrap()),
            TickMode::Always,
        );
    }
    for (id, authored, ty) in [(900005, source, 707), (900006, second, 708)] {
        let mut npc = NPC::new(id, ty, position, 0, instance).unwrap();
        npc.authored_id = Some(authored);
        state.entity_map.track(Box::new(npc), TickMode::Always);
    }
    let clients = HashMap::from([(1, client)]);
    super::mission::start_requested_task(
        &sP_CL2FE_REQ_PC_TASK_START {
            iTaskNum: id,
            iNPC_ID: 900001,
            iEscortNPC_ID: 0,
        },
        clients.get(&1).unwrap(),
        &mut state,
        false,
    )
    .unwrap();
    let actor_id = state.get_player(1).unwrap().native_stage.actors[0]
        .runtime_id
        .unwrap();
    assert!(!state
        .get_npc(900005)
        .unwrap()
        .ordinary_visible(state.get_player(1).unwrap()));
    let mut other = Player::new(2, 0);
    other.set_player_id(2);
    assert!(state.get_npc(900005).unwrap().visible_to(&other));
    assert!(!state.get_npc(actor_id).unwrap().visible_to(&other));
    assert!(state
        .get_npc(actor_id)
        .unwrap()
        .visible_to(state.get_player(1).unwrap()));
    for _ in 0..30 {
        crate::mission_stage::tick_actor(&mut state, actor_id);
        if state.get_player(1).unwrap().native_stage.actors[0].done {
            break;
        }
    }
    assert!(state.get_player(1).unwrap().native_stage.actors[0].done);
    assert_eq!(state.get_npc(actor_id).unwrap().ty, 708);
    assert_eq!(state.get_npc(900005).unwrap().get_position(), position);
    let end = || {
        Packet::new(
            P_CL2FE_REQ_PC_TASK_END,
            &sP_CL2FE_REQ_PC_TASK_END {
                iTaskNum: id,
                iNPC_ID: 900002,
                iBox1Choice: 0,
                iBox2Choice: 0,
                iEscortNPC_ID: 0,
            },
        )
        .unwrap()
    };
    assert!(super::mission::task_end(end(), &ClientMap::new(1, &clients), &mut state).is_err());
    assert_eq!(state.get_player(1).unwrap().get_taros(), 0);
    state.get_player_mut(1).unwrap().set_taros(100);
    crate::mission_stage::tick_player(&mut state, 1);
    assert!(!state
        .get_npc(900006)
        .unwrap()
        .visible_to(state.get_player(1).unwrap()));
    super::mission::task_end(end(), &ClientMap::new(1, &clients), &mut state).unwrap();
    assert_eq!(state.get_player(1).unwrap().get_taros(), 200);
    let p = state.get_player(1).unwrap();
    assert!(p
        .mission_journal
        .check_completed_previous_task(tdata_get().get_task_definition(id + 2).unwrap()));
    assert!(!p
        .mission_journal
        .check_completed_previous_task(tdata_get().get_task_definition(id + 1).unwrap()));
    let mut acknowledged = false;
    while let Ok(ClientMessage::SendPacket(packet)) = rx.try_recv() {
        if packet.id() == P_FE2CL_COMPLETION_BRANCH {
            let ack = packet.get::<NativeCompletionBranch>().unwrap();
            let (variant, next) = (ack.variant, ack.next_task);
            assert_eq!((variant, next), (2, id + 2));
            acknowledged = true;
        }
    }
    assert!(acknowledged);
    assert!(super::mission::task_end(end(), &ClientMap::new(1, &clients), &mut state).is_err());
    assert_eq!(state.get_player(1).unwrap().get_taros(), 200);
    let saved = state
        .get_player(1)
        .unwrap()
        .native_stage
        .serialize()
        .unwrap();
    let restored = crate::mission_stage::PlayerStage::load(&saved).unwrap();
    assert_eq!(restored.branches[&id], 1);
    assert!(restored.actors[0].runtime_id.is_none());
    assert!(restored.actors[0].done);
    #[cfg(feature = "sqlite")]
    {
        let loaded = tokio::runtime::Runtime::new().unwrap().block_on(
            crate::database::native_stage_persistence_tests::round_trip(
                state.get_player(1).unwrap(),
            ),
        );
        assert!(loaded
            .mission_journal
            .check_completed_previous_task(tdata_get().get_task_definition(id + 2).unwrap()));
        assert!(!loaded
            .mission_journal
            .check_completed_previous_task(tdata_get().get_task_definition(id + 1).unwrap()));
        assert!(loaded.native_stage.actors[0].done);
        assert!(loaded.native_stage.actors[0].runtime_id.is_none());
    }
    state.get_player_mut(1).unwrap().perms = 10;
    assert!(state
        .get_npc(900006)
        .unwrap()
        .visible_to(state.get_player(1).unwrap()));
    state.get_player_mut(1).unwrap().user_view = true;
    assert!(!state
        .get_npc(900006)
        .unwrap()
        .visible_to(state.get_player(1).unwrap()));
    crate::mission_stage::reset(&mut state, 1, definition.mission_id);
    assert!(state
        .get_npc(900005)
        .unwrap()
        .visible_to(state.get_player(1).unwrap()));
    assert!(state
        .get_npc(900006)
        .unwrap()
        .visible_to(state.get_player(1).unwrap()));
    assert!(state
        .get_player(1)
        .unwrap()
        .native_stage
        .branches
        .is_empty());
    let _: Task = definition.into();
    let click = |flag| {
        Packet::new(
            P_CL2FE_REQ_NPC_INTERACTION,
            &sP_CL2FE_REQ_NPC_INTERACTION {
                iNPC_ID: 900002,
                bFlag: flag,
            },
        )
        .unwrap()
    };
    state.get_player_mut(1).unwrap().set_taros(0);
    super::npc::npc_interaction(click(1), clients.get(&1).unwrap(), &mut state).unwrap();
    assert!(speech_replies(&mut rx).is_empty());
    super::npc::npc_interaction(click(0), clients.get(&1).unwrap(), &mut state).unwrap();
    state.get_player_mut(1).unwrap().set_taros(100);
    super::npc::npc_interaction(click(1), clients.get(&1).unwrap(), &mut state).unwrap();
    assert_eq!(speech_replies(&mut rx), vec![(0, 1000777)]);
    super::npc::npc_interaction(click(0), clients.get(&1).unwrap(), &mut state).unwrap();
    super::npc::npc_interaction(click(1), clients.get(&1).unwrap(), &mut state).unwrap();
    assert!(speech_replies(&mut rx).is_empty());
    let background = || {
        Packet::new(
            P_CL2FE_REQ_AMBIENT_BARK,
            &NativeAmbientBark {
                npc_id: 900002,
                set: 0,
                line: 1,
            },
        )
        .unwrap()
    };
    state.get_player_mut(1).unwrap().ambient_attempts.clear();
    super::npc::ambient_bark(background(), clients.get(&1).unwrap(), &mut state).unwrap();
    assert!(speech_replies(&mut rx).is_empty());
    state.get_player_mut(1).unwrap().set_level(2).unwrap();
    super::npc::ambient_bark(background(), clients.get(&1).unwrap(), &mut state).unwrap();
    assert_eq!(speech_replies(&mut rx), vec![(1, 1000778)]);
}
fn speech_replies(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ClientMessage>) -> Vec<(i32, i32)> {
    let mut replies = Vec::new();
    while let Ok(ClientMessage::SendPacket(packet)) = rx.try_recv() {
        if packet.id() == P_FE2CL_NPC_SPEECH {
            let ack = packet.get::<NativeNpcSpeech>().unwrap();
            replies.push((ack.kind, ack.line));
        }
    }
    replies
}
