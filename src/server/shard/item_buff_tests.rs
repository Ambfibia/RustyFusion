use super::*;
use std::collections::HashMap;
use crate::{chunk::{ChunkCoords, InstanceID, TickMode}, entity::Player, net::{ClientMessage, ClientMetadata, ClientType}, tabledata::tdata_init};
type Rx = tokio::sync::mpsc::UnboundedReceiver<ClientMessage>;
fn fixture() -> (ShardServerState, HashMap<usize, FFClient>, Rx, Rx) {
    tdata_init().unwrap(); let mut state = ShardServerState::default(); state.entity_map = crate::chunk::EntityMap::default();
    let mut clients = HashMap::new(); let mut receivers = Vec::new();
    for id in [1, 2] {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel(); let client = FFClient::new(tx, ClientMetadata::new(format!("127.0.0.1:{}", 39920 + id).parse().unwrap(), Some(ClientType::GameClient { account_id: id as i64, serial_key: id as i64, pc_id: Some(id) })));
        let mut p = Player::new(id as i64, 0); p.set_player_id(id); p.set_client(client.clone());
        let chunk = ChunkCoords::from_pos_inst(p.get_position(), InstanceID::default());
        let eid = state.entity_map.track(Box::new(p), TickMode::Always); state.entity_map.update(eid, Some(chunk), false);
        clients.insert(id as usize, client); receivers.push(rx);
    }
    let observer = receivers.pop().unwrap(); let owner = receivers.pop().unwrap(); (state, clients, owner, observer)
}
fn request() -> Packet { Packet::new(P_CL2FE_REQ_ITEM_USE, &sP_CL2FE_REQ_ITEM_USE { eIL: ItemLocation::Inven as i32, iSlotNum: 0, iNanoSlot: 0 }).unwrap() }
fn take(rx: &mut Rx, kind: PacketID) -> Packet { std::iter::from_fn(|| rx.try_recv().ok()).find_map(|m| match m { ClientMessage::SendPacket(p) if p.id() == kind => Some(p), _ => None }).unwrap() }
#[test]
fn all_accepted_subtype_four_known_effect_items_consume_once_and_obey_authored_cooldown() {
    for id in [122, 124, 125, 126, 127, 137, 138, 146, 147, 148, 150, 151, 152] {
        let (mut state, clients, mut owner, mut observer) = fixture();
        let p = state.get_player_mut(1).unwrap(); p.take_damage(200, None); p.set_item(ItemLocation::Inven, 0, Some(Item::new(ItemType::General, id))).unwrap();
        item_use(request(), &ClientMap::new(1, &clients), &mut state).unwrap();
        let receipt = take(&mut owner, P_FE2CL_REP_PC_ITEM_USE_SUCC); let header = *receipt.get::<sP_FE2CL_REP_PC_ITEM_USE_SUCC>().unwrap();
        let stats = tdata_get().get_item_stats(id, ItemType::General).unwrap();
        assert_eq!({header.iSkillID}, stats.linked_skill.unwrap()); assert_eq!({header.iTargetCnt}, 1);
        assert_eq!(receipt.read_bytes().len(), 4 + 36 + 16); assert!(state.get_player(1).unwrap().get_item(ItemLocation::Inven, 0).unwrap().is_none());
        let broadcast = take(&mut observer, P_FE2CL_PC_ITEM_USE); assert_eq!(broadcast.read_bytes().len(), 4 + 16 + 16);
        state.get_player_mut(1).unwrap().set_item(ItemLocation::Inven, 0, Some(Item::new(ItemType::General, id))).unwrap();
        assert!(item_use(request(), &ClientMap::new(1, &clients), &mut state).is_err());
        assert!(state.get_player(1).unwrap().get_item(ItemLocation::Inven, 0).unwrap().is_some());
    }
}
#[test]
fn unsupported_capacity_token_and_nanocom_items_are_not_consumed() {
    for id in [1, 130, 134, 135, 136, 167, 168] {
        let (mut state, clients, _owner, _observer) = fixture();
        state.get_player_mut(1).unwrap().set_item(ItemLocation::Inven, 0, Some(Item::new(ItemType::General, id))).unwrap();
        assert!(item_use(request(), &ClientMap::new(1, &clients), &mut state).is_err(), "item{id}");
        assert_eq!(state.get_player(1).unwrap().get_item(ItemLocation::Inven, 0).unwrap().unwrap().id, id);
    }
}
#[test]
fn wildcard_gumballs_keep_linked_skill_id_duration_and_complete_broadcast() {
    for (id, skill_id) in [(123, 191), (145, 197), (149, 197)] {
        let (mut state, clients, mut owner, mut observer) = fixture();
        let p = state.get_player_mut(1).unwrap(); p.unlock_nano(1).unwrap(); p.change_nano(0, Some(1)).unwrap();
        p.set_item(ItemLocation::Inven, 0, Some(Item::new(ItemType::General, id))).unwrap();
        item_use(request(), &ClientMap::new(1, &clients), &mut state).unwrap();
        let receipt = take(&mut owner, P_FE2CL_REP_PC_ITEM_USE_SUCC); let header = *receipt.get::<sP_FE2CL_REP_PC_ITEM_USE_SUCC>().unwrap();
        assert_eq!({header.iSkillID}, skill_id); assert!(state.get_player(1).unwrap().has_buff(BuffID::StimPakSlot1, Some(BuffType::CashItem)));
        let broadcast = take(&mut observer, P_FE2CL_PC_ITEM_USE); assert_eq!(broadcast.read_bytes().len(), 4 + 16 + 16);
    }
}

#[test]
fn wrong_gumball_flavor_does_not_consume_or_start_cooldown() {
    let (mut state, clients, mut owner, _observer) = fixture();
    let p = state.get_player_mut(1).unwrap(); p.unlock_nano(1).unwrap(); p.change_nano(0, Some(1)).unwrap();
    assert_eq!(tdata_get().get_nano_stats(1).unwrap().style, CombatStyle::Blastons);
    p.set_item(ItemLocation::Inven, 0, Some(Item::new(ItemType::General, 119))).unwrap();
    assert!(item_use(request(), &ClientMap::new(1, &clients), &mut state).is_err());
    assert_eq!(state.get_player(1).unwrap().get_item(ItemLocation::Inven, 0).unwrap().unwrap().id, 119);
    assert!(!state.get_player(1).unwrap().has_buff(BuffID::StimPakSlot1, None));
    state.get_player_mut(1).unwrap().set_item(ItemLocation::Inven, 0, Some(Item::new(ItemType::General, 120))).unwrap();
    item_use(request(), &ClientMap::new(1, &clients), &mut state).unwrap();
    take(&mut owner, P_FE2CL_REP_PC_ITEM_USE_SUCC);
}
