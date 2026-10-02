use super::*;
use crate::{
    chunk::{ChunkCoords, InstanceID, TickMode},
    entity::Player,
    item::Item,
    net::{ClientMessage, ClientMetadata, ClientType, FFClient},
    tabledata::{tdata_get, tdata_init},
    Position,
};
use std::collections::HashMap;
type Rx = tokio::sync::mpsc::UnboundedReceiver<ClientMessage>;
fn fixture() -> (ShardServerState, HashMap<usize, FFClient>, Rx, Rx) {
    tdata_init().unwrap();
    let mut state = ShardServerState::default();
    let mut clients = HashMap::new();
    let mut receivers = Vec::new();
    let pos = Position {
        x: 200_000,
        y: 200_000,
        z: 0,
    };
    for id in [1, 2] {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let client = FFClient::new(
            tx,
            ClientMetadata::new(
                format!("127.0.0.1:{}", 39390 + id).parse().unwrap(),
                Some(ClientType::GameClient {
                    account_id: id as i64,
                    serial_key: id as i64,
                    pc_id: Some(id),
                }),
            ),
        );
        let mut player = Player::new(id as i64, 0);
        player.set_player_id(id);
        player.set_position(pos);
        player.set_client(client.clone());
        player.set_taros(1000);
        let entity = Box::new(player);
        let eid = entity.get_id();
        state.entity_map.track(entity, TickMode::Always);
        state.entity_map.update(
            eid,
            Some(ChunkCoords::from_pos_inst(pos, InstanceID::default())),
            false,
        );
        clients.insert(id as usize, client);
        receivers.push(rx);
    }
    (state, clients, receivers.remove(0), receivers.remove(0))
}
fn drain(rx: &mut Rx) -> Vec<Packet> {
    std::iter::from_fn(|| match rx.try_recv().ok()? {
        ClientMessage::SendPacket(p) => Some(p),
        _ => None,
    })
    .collect()
}
fn offer(
    state: &mut ShardServerState,
    clients: &HashMap<usize, FFClient>,
    to: i32,
) -> FFResult<()> {
    trade_offer(
        Packet::new(
            P_CL2FE_REQ_PC_TRADE_OFFER,
            &sP_CL2FE_REQ_PC_TRADE_OFFER {
                iID_Request: 1,
                iID_From: 1,
                iID_To: to,
            },
        )
        .unwrap(),
        &ClientMap::new(1, clients),
        state,
    )
}
fn accept(state: &mut ShardServerState, clients: &HashMap<usize, FFClient>) -> FFResult<()> {
    trade_offer_accept(
        Packet::new(
            P_CL2FE_REQ_PC_TRADE_OFFER_ACCEPT,
            &sP_CL2FE_REQ_PC_TRADE_OFFER_ACCEPT {
                iID_Request: 2,
                iID_From: 1,
                iID_To: 2,
            },
        )
        .unwrap(),
        &ClientMap::new(2, clients),
        state,
    )
}
#[test]
fn self_offer_and_replayed_accept_cannot_corrupt_trade() {
    let (mut state, clients, mut a, mut b) = fixture();
    assert!(offer(&mut state, &clients, 1).is_err());
    assert!(state.ongoing_trades.is_empty());
    drain(&mut a);
    offer(&mut state, &clients, 2).unwrap();
    assert_eq!(drain(&mut b)[0].id(), P_FE2CL_REP_PC_TRADE_OFFER);
    accept(&mut state, &clients).unwrap();
    assert_eq!(drain(&mut a)[0].id(), P_FE2CL_REP_PC_TRADE_OFFER_SUCC);
    assert_eq!(drain(&mut b)[0].id(), P_FE2CL_REP_PC_TRADE_OFFER_SUCC);
    let id = state.get_player(1).unwrap().trade_id;
    assert!(accept(&mut state, &clients).is_err());
    assert_eq!(state.get_player(1).unwrap().trade_id, id);
    assert_eq!(state.get_player(2).unwrap().trade_id, id);
    assert_eq!(state.ongoing_trades.len(), 1);
}
#[test]
fn decline_cancel_and_late_accept_leave_inventory_intact() {
    let (mut state, clients, mut a, mut b) = fixture();
    offer(&mut state, &clients, 2).unwrap();
    drain(&mut b);
    trade_offer_refusal(
        Packet::new(
            P_CL2FE_REQ_PC_TRADE_OFFER_REFUSAL,
            &sP_CL2FE_REQ_PC_TRADE_OFFER_REFUSAL {
                iID_Request: 2,
                iID_From: 1,
                iID_To: 2,
            },
        )
        .unwrap(),
        &ClientMap::new(2, &clients),
        &mut state,
    )
    .unwrap();
    assert_eq!(drain(&mut a)[0].id(), P_FE2CL_REP_PC_TRADE_OFFER_REFUSAL);
    assert!(accept(&mut state, &clients).is_err());
    drain(&mut b);
    offer(&mut state, &clients, 2).unwrap();
    drain(&mut b);
    trade_offer_cancel(
        Packet::new(
            P_CL2FE_REQ_PC_TRADE_OFFER_CANCEL,
            &sP_CL2FE_REQ_PC_TRADE_OFFER_CANCEL {
                iID_Request: 1,
                iID_From: 1,
                iID_To: 2,
            },
        )
        .unwrap(),
        &ClientMap::new(1, &clients),
        &mut state,
    )
    .unwrap();
    assert_eq!(drain(&mut a)[0].id(), P_FE2CL_REP_PC_TRADE_OFFER_CANCEL);
    assert_eq!(drain(&mut b)[0].id(), P_FE2CL_REP_PC_TRADE_OFFER_CANCEL);
    assert!(accept(&mut state, &clients).is_err());
    assert!(state.ongoing_trades.is_empty());
    assert_eq!(state.get_player(1).unwrap().get_taros(), 1000);
}
fn item() -> Item {
    let id = (1..=i16::MAX)
        .find(|id| {
            tdata_get()
                .get_item_stats(*id, ItemType::General)
                .is_ok_and(|s| s.tradeable)
        })
        .unwrap();
    let mut item = Item::new(ItemType::General, id);
    item.quantity = 5;
    item
}
#[test]
fn excessive_and_forged_registration_are_rejected_without_partial_offer() {
    let (mut state, clients, mut a, mut b) = fixture();
    let item = item();
    state
        .get_player_mut(1)
        .unwrap()
        .set_item(ItemLocation::Inven, 0, Some(item))
        .unwrap();
    offer(&mut state, &clients, 2).unwrap();
    accept(&mut state, &clients).unwrap();
    drain(&mut a);
    drain(&mut b);
    for quantity in [6, 65536, i32::MAX] {
        let req = sP_CL2FE_REQ_PC_TRADE_ITEM_REGISTER {
            iID_Request: 1,
            iID_From: 1,
            iID_To: 2,
            Item: sItemTrade {
                iType: item.ty as i16,
                iID: item.id,
                iOpt: quantity,
                iInvenNum: 0,
                iSlotNum: 0,
            },
        };
        assert!(trade_item_register(
            Packet::new(P_CL2FE_REQ_PC_TRADE_ITEM_REGISTER, &req).unwrap(),
            &ClientMap::new(1, &clients),
            &mut state
        )
        .is_err());
        assert_eq!(
            drain(&mut a)[0].id(),
            P_FE2CL_REP_PC_TRADE_ITEM_REGISTER_FAIL
        );
        assert!(drain(&mut b).is_empty());
        assert_eq!(
            *state
                .get_player(1)
                .unwrap()
                .get_item(ItemLocation::Inven, 0)
                .unwrap(),
            Some(item)
        );
    }
    let id = state.get_player(1).unwrap().trade_id.unwrap();
    let trade = state.ongoing_trades.get_mut(&id).unwrap();
    assert_eq!(trade.add_item_checked(1, 0, 0, 5, 5).unwrap(), 5);
    assert!(trade.add_item_checked(1, 1, 0, 1, 5).is_err());
    assert_eq!(trade.remove_item(1, 0).unwrap(), (0, 0));
}
#[test]
fn accepted_cancel_and_missing_context_are_safe() {
    let (mut state, clients, mut a, mut b) = fixture();
    offer(&mut state, &clients, 2).unwrap();
    accept(&mut state, &clients).unwrap();
    drain(&mut a);
    drain(&mut b);
    let request = Packet::new(
        P_CL2FE_REQ_PC_TRADE_CONFIRM_CANCEL,
        &sP_CL2FE_REQ_PC_TRADE_CONFIRM_CANCEL {
            iID_Request: 1,
            iID_From: 1,
            iID_To: 2,
        },
    )
    .unwrap();
    trade_confirm_cancel(request.clone(), &ClientMap::new(1, &clients), &mut state).unwrap();
    assert_eq!(drain(&mut b)[0].id(), P_FE2CL_REP_PC_TRADE_CONFIRM_CANCEL);
    assert!(state.ongoing_trades.is_empty());
    assert!(state.get_player(1).unwrap().trade_id.is_none());
    assert!(state.get_player(2).unwrap().trade_id.is_none());
    assert!(trade_confirm_cancel(request, &ClientMap::new(1, &clients), &mut state).is_err());
}
