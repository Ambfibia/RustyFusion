use super::*;
use crate::{chunk::TickMode, net::{ClientMessage, ClientMetadata, ClientType}, tabledata::tdata_init};

#[test]
fn quest_stack_update_and_consumption_reuse_existing_stack_after_an_earlier_hole() {
    tdata_init().unwrap();
    let mut player = Player::new(1, 0);
    assert_eq!(player.set_quest_item_count(43, 1).unwrap(), 0);
    assert_eq!(player.set_quest_item_count(68, 1).unwrap(), 1);
    player.set_quest_item_count(43, 0).unwrap();
    assert_eq!(player.set_quest_item_count(68, 2).unwrap(), 1);
    assert_eq!(player.get_quest_item_count(68), 2);
    assert_eq!(player.inventory.quest.iter().flatten().filter(|(id, _)| *id == 68).count(), 1);
    player.set_quest_item_count(68, 0).unwrap();
    assert_eq!(player.get_quest_item_count(68), 0);
}

#[test]
fn timed_failure_uses_its_own_item_deltas_and_exact_reward_trailer_once() {
    tdata_init().unwrap();
    // The original task 214 fails by taking both 68 and 69; its success
    // removes only 68. This is a real asymmetric row, not a synthetic task.
    let definition = tdata_get().get_task_definition(214).unwrap();
    assert_eq!(definition.fail_qitems.len(), 2);
    assert_eq!(definition.succ_qitems.len(), 1);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(tx, ClientMetadata::new("127.0.0.1:39542".parse().unwrap(),
        Some(ClientType::GameClient { account_id: 1, serial_key: 1, pc_id: Some(1) })));
    let mut player = Player::new(1, 0); player.set_player_id(1); player.set_level(36).unwrap(); player.set_hp(1000); player.set_client(client);
    player.set_quest_item_count(68, 1).unwrap(); player.set_quest_item_count(69, 1).unwrap();
    let mut task: Task = definition.into(); task.fail_time = Some(SystemTime::UNIX_EPOCH);
    player.mission_journal.start_editor_task(task, 36).unwrap();
    let mut state = ShardServerState::default();
    let chunk = player.get_chunk_coords(); let eid = state.entity_map.track(Box::new(player), TickMode::Always);
    state.entity_map.update(eid, Some(chunk), false);
    while rx.try_recv().is_ok() {}
    Player::tick_missions(&mut state, 1, &SystemTime::now());
    let player = state.get_player(1).unwrap();
    assert_eq!(player.get_quest_item_count(68), 0);
    assert_eq!(player.get_quest_item_count(69), 0);
    let packets: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).filter_map(|msg| if let ClientMessage::SendPacket(packet) = msg { Some(packet) } else { None }).collect();
    let reward = packets.iter().find(|packet| packet.id() == P_FE2CL_REP_REWARD_ITEM).unwrap();
    let mut reader = PacketReader::new(reward);
    let header: &sP_FE2CL_REP_REWARD_ITEM = reader.get_struct().unwrap();
    let count = header.iItemCnt; assert_eq!(count, 2);
    let mut slots = Vec::new();
    for _ in 0..count { let item: &sItemReward = reader.get_struct().unwrap(); let id = item.sItem.iID; let quantity = item.sItem.iOpt; let slot = item.iSlotNum; slots.push(slot); assert_eq!((id, quantity), (0, 0)); }
    slots.sort(); assert_eq!(slots, [0, 1]);
    Player::tick_missions(&mut state, 1, &SystemTime::now());
    assert!(!std::iter::from_fn(|| rx.try_recv().ok()).any(|msg| matches!(msg, ClientMessage::SendPacket(packet) if packet.id() == P_FE2CL_REP_REWARD_ITEM)));
}

#[test]
fn original_grant_only_timer_stages_receive_authoritative_deadlines() {
    tdata_init().unwrap();
    for (id, seconds) in [(5063, 120), (5064, 240), (5144, 60), (5176, 30)] {
        let definition = tdata_get().get_task_definition(id).unwrap();
        assert_eq!(definition.obj_time_limit.unwrap().as_secs(), seconds);
        let before = SystemTime::now(); let task: Task = definition.into();
        assert!(task.fail_time.unwrap().duration_since(before).unwrap().as_secs() >= seconds);
    }
}
