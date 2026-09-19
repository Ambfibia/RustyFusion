use std::collections::HashMap;

use crate::{
    chunk::{ChunkCoords, InstanceID, TickMode},
    defines::*,
    entity::{Entity, Player},
    enums::{ItemLocation, ItemType},
    item::Item,
    net::{
        packet::{Packet, PacketID::*, *},
        ClientMap, ClientMessage, ClientMetadata, ClientType, FFClient,
    },
    state::ShardServerState,
    tabledata::tdata_init,
};

use super::item::item_move;

type Rx = tokio::sync::mpsc::UnboundedReceiver<ClientMessage>;

fn client(pc_id: i32) -> (FFClient, Rx) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            format!("127.0.0.1:{}", 39100 + pc_id).parse().unwrap(),
            Some(ClientType::GameClient {
                account_id: pc_id as i64,
                serial_key: pc_id as i64,
                pc_id: Some(pc_id),
            }),
        ),
    );
    (client, rx)
}

/// Owner (PC 1) and an observer (PC 2) standing in the same chunk.
fn fixture() -> (ShardServerState, HashMap<usize, FFClient>, Rx, Rx) {
    tdata_init().unwrap();
    let mut state = ShardServerState::default();
    let mut clients = HashMap::new();
    let mut rxs = Vec::new();
    for pc_id in [1, 2] {
        let (c, rx) = client(pc_id);
        let mut player = Player::new(pc_id as i64, 0);
        player.set_player_id(pc_id);
        player.set_client(c.clone());
        let chunk = ChunkCoords::from_pos_inst(player.get_position(), InstanceID::default());
        let id = state.entity_map.track(Box::new(player), TickMode::Always);
        state.entity_map.update(id, Some(chunk), false);
        clients.insert(pc_id as usize, c);
        rxs.push(rx);
    }
    let observer = rxs.pop().unwrap();
    let owner = rxs.pop().unwrap();
    (state, clients, owner, observer)
}

fn shirt(id: i16) -> Item {
    Item::new(ItemType::UpperBody, id)
}

fn stack(qty: u16) -> Item {
    // General item 1 stacks to 20 in the shipped xdt.
    let mut item = Item::new(ItemType::General, 1);
    item.quantity = qty;
    item
}

fn put(state: &mut ShardServerState, loc: ItemLocation, slot: usize, item: Option<Item>) {
    state
        .get_player_mut(1)
        .unwrap()
        .set_item(loc, slot, item)
        .unwrap();
}

fn get(state: &ShardServerState, loc: ItemLocation, slot: usize) -> Option<Item> {
    *state.get_player(1).unwrap().get_item(loc, slot).unwrap()
}

fn mv(
    state: &mut ShardServerState,
    clients: &HashMap<usize, FFClient>,
    from: (i32, i32),
    to: (i32, i32),
) -> bool {
    let pkt = Packet::new(
        P_CL2FE_REQ_ITEM_MOVE,
        &sP_CL2FE_REQ_ITEM_MOVE {
            eFrom: from.0,
            iFromSlotNum: from.1,
            eTo: to.0,
            iToSlotNum: to.1,
        },
    )
    .unwrap();
    item_move(pkt, &ClientMap::new(1, clients), state).is_ok()
}

fn drain(rx: &mut Rx) -> Vec<Packet> {
    let mut out = Vec::new();
    while let Ok(ClientMessage::SendPacket(p)) = rx.try_recv() {
        out.push(p);
    }
    out
}

fn equip_changes(pkts: &[Packet]) -> Vec<(i32, i32, Option<Item>)> {
    pkts.iter()
        .filter(|p| p.id() == P_FE2CL_PC_EQUIP_CHANGE)
        .map(|p| {
            let e = p.get::<sP_FE2CL_PC_EQUIP_CHANGE>().unwrap();
            let item: Option<Item> = Option::<Item>::try_from_proto({ e.EquipSlotItem }).unwrap();
            ({ e.iPC_ID }, { e.iEquipSlotNum }, item)
        })
        .collect()
}

/// Applies the owner's reply stream the way FFOneClient does: ITEM_MOVE_SUCC
/// post-state, then own-PC EQUIP_CHANGE post-state.
fn client_view(
    mut equip: [Option<Item>; 9],
    mut inven: [Option<Item>; 50],
    pkts: &[Packet],
) -> ([Option<Item>; 9], [Option<Item>; 50]) {
    for p in pkts {
        let mut write = |loc: i32, slot: i32, item: sItemBase| {
            let item = Option::<Item>::try_from_proto(item).unwrap();
            match loc {
                0 => equip[slot as usize] = item,
                1 => inven[slot as usize] = item,
                _ => {}
            }
        };
        if p.id() == P_FE2CL_PC_ITEM_MOVE_SUCC {
            let s = p.get::<sP_FE2CL_PC_ITEM_MOVE_SUCC>().unwrap();
            write(s.eFrom, s.iFromSlotNum, s.FromSlotItem);
            write(s.eTo, s.iToSlotNum, s.ToSlotItem);
        } else if p.id() == P_FE2CL_PC_EQUIP_CHANGE {
            let e = p.get::<sP_FE2CL_PC_EQUIP_CHANGE>().unwrap();
            if e.iPC_ID == 1 {
                write(0, e.iEquipSlotNum, e.EquipSlotItem);
            }
        }
    }
    (equip, inven)
}

fn server_view(state: &ShardServerState) -> ([Option<Item>; 9], [Option<Item>; 50]) {
    let p = state.get_player(1).unwrap();
    let mut inven = [None; 50];
    for (i, slot) in inven.iter_mut().enumerate() {
        *slot = *p.get_item(ItemLocation::Inven, i).unwrap();
    }
    (*p.get_equipped(), inven)
}

const EQ: i32 = ItemLocation::Equip as i32;
const INV: i32 = ItemLocation::Inven as i32;
const UB: i32 = EQUIP_SLOT_UPPERBODY as i32;

#[test]
fn single_item_equips_on_first_move_and_client_matches_server() {
    let (mut state, clients, mut owner, mut observer) = fixture();
    put(&mut state, ItemLocation::Equip, UB as usize, None);
    put(&mut state, ItemLocation::Inven, 3, Some(shirt(2)));
    let before = server_view(&state);

    assert!(mv(&mut state, &clients, (INV, 3), (EQ, UB)));
    assert_eq!(
        get(&state, ItemLocation::Equip, UB as usize),
        Some(shirt(2))
    );
    assert_eq!(get(&state, ItemLocation::Inven, 3), None);

    let pkts = drain(&mut owner);
    assert_eq!(client_view(before.0, before.1, &pkts), server_view(&state));
    assert_eq!(equip_changes(&pkts), vec![(1, UB, Some(shirt(2)))]);
    assert_eq!(
        equip_changes(&drain(&mut observer)),
        vec![(1, UB, Some(shirt(2)))]
    );
}

#[test]
fn unequip_and_swap_report_final_slot_contents() {
    let (mut state, clients, mut owner, mut observer) = fixture();
    put(&mut state, ItemLocation::Equip, UB as usize, Some(shirt(2)));
    put(&mut state, ItemLocation::Inven, 0, Some(shirt(3)));

    // swap equipped shirt with inventory shirt, initiated from the equip side
    let before = server_view(&state);
    assert!(mv(&mut state, &clients, (EQ, UB), (INV, 0)));
    assert_eq!(
        get(&state, ItemLocation::Equip, UB as usize),
        Some(shirt(3))
    );
    assert_eq!(get(&state, ItemLocation::Inven, 0), Some(shirt(2)));
    let pkts = drain(&mut owner);
    assert_eq!(client_view(before.0, before.1, &pkts), server_view(&state));
    assert_eq!(equip_changes(&pkts), vec![(1, UB, Some(shirt(3)))]);
    assert_eq!(
        equip_changes(&drain(&mut observer)),
        vec![(1, UB, Some(shirt(3)))]
    );

    // plain unequip into an empty inventory slot
    let before = server_view(&state);
    assert!(mv(&mut state, &clients, (EQ, UB), (INV, 7)));
    assert_eq!(get(&state, ItemLocation::Equip, UB as usize), None);
    assert_eq!(get(&state, ItemLocation::Inven, 7), Some(shirt(3)));
    let pkts = drain(&mut owner);
    assert_eq!(client_view(before.0, before.1, &pkts), server_view(&state));
    assert_eq!(equip_changes(&pkts), vec![(1, UB, None)]);
}

#[test]
fn invalid_target_index_does_not_lose_source_item() {
    let (mut state, clients, mut owner, _observer) = fixture();
    put(&mut state, ItemLocation::Inven, 3, Some(shirt(2)));
    let before = server_view(&state);
    for to in [
        (EQ, 9),
        (EQ, -1),
        (INV, 50),
        (INV, -1),
        (3, SIZEOF_BANK_SLOT as i32),
    ] {
        assert!(!mv(&mut state, &clients, (INV, 3), to), "{to:?}");
        assert_eq!(server_view(&state), before, "{to:?}");
    }
    // bad source index / unknown or quest locations are rejected without panicking
    for from in [(INV, -1), (INV, 50), (2, 0), (4, 0), (-1, 0)] {
        assert!(!mv(&mut state, &clients, from, (EQ, UB)), "{from:?}");
        assert_eq!(server_view(&state), before, "{from:?}");
    }
    assert!(drain(&mut owner).is_empty());
}

#[test]
fn wrong_slot_type_is_rejected_both_directions() {
    let (mut state, clients, _owner, _observer) = fixture();
    put(&mut state, ItemLocation::Inven, 0, Some(shirt(2)));
    put(&mut state, ItemLocation::Inven, 1, Some(stack(5)));
    put(
        &mut state,
        ItemLocation::Inven,
        2,
        Some(Item::new(ItemType::Vehicle, 1)),
    );
    put(
        &mut state,
        ItemLocation::Equip,
        EQUIP_SLOT_HAND as usize,
        Some(Item::new(ItemType::Hand, 1)),
    );
    let before = server_view(&state);

    assert!(!mv(
        &mut state,
        &clients,
        (INV, 0),
        (EQ, EQUIP_SLOT_FOOT as i32)
    ));
    assert!(!mv(&mut state, &clients, (INV, 1), (EQ, UB)));
    assert!(!mv(&mut state, &clients, (INV, 2), (EQ, UB)));
    assert!(!mv(
        &mut state,
        &clients,
        (INV, 0),
        (EQ, EQUIP_SLOT_VEHICLE as i32)
    ));
    // swap initiated from the equip side would push a shirt into the hand slot
    assert!(!mv(
        &mut state,
        &clients,
        (EQ, EQUIP_SLOT_HAND as i32),
        (INV, 0)
    ));
    // equip -> equip across different slot types
    assert!(!mv(
        &mut state,
        &clients,
        (EQ, EQUIP_SLOT_HAND as i32),
        (EQ, UB)
    ));
    assert_eq!(server_view(&state), before);

    // vehicle and weapon slots accept their own types
    assert!(mv(
        &mut state,
        &clients,
        (INV, 2),
        (EQ, EQUIP_SLOT_VEHICLE as i32)
    ));
    put(
        &mut state,
        ItemLocation::Inven,
        4,
        Some(Item::new(ItemType::Hand, 2)),
    );
    assert!(mv(
        &mut state,
        &clients,
        (INV, 4),
        (EQ, EQUIP_SLOT_HAND as i32)
    ));
    assert_eq!(
        get(&state, ItemLocation::Equip, EQUIP_SLOT_HAND as usize),
        Some(Item::new(ItemType::Hand, 2))
    );
    assert_eq!(
        get(&state, ItemLocation::Inven, 4),
        Some(Item::new(ItemType::Hand, 1))
    );
}

#[test]
fn same_slot_and_repeated_requests_neither_lose_nor_duplicate() {
    let (mut state, clients, mut owner, _observer) = fixture();
    put(&mut state, ItemLocation::Inven, 3, Some(shirt(2)));
    put(&mut state, ItemLocation::Inven, 5, Some(stack(7)));
    let before = server_view(&state);
    assert!(mv(&mut state, &clients, (INV, 3), (INV, 3)));
    assert!(mv(&mut state, &clients, (INV, 5), (INV, 5)));
    assert_eq!(server_view(&state), before);
    let pkts = drain(&mut owner);
    assert_eq!(client_view(before.0, before.1, &pkts), before);

    assert!(mv(&mut state, &clients, (INV, 3), (EQ, UB)));
    let after = server_view(&state);
    // the client resending the same request moves an empty slot: no change
    assert!(mv(&mut state, &clients, (INV, 3), (EQ, UB)));
    assert_eq!(server_view(&state), after);
    let pkts = drain(&mut owner);
    assert_eq!(client_view(before.0, before.1, &pkts), after);
}

#[test]
fn stacking_preserves_total_quantity() {
    let (mut state, clients, mut owner, _observer) = fixture();
    put(&mut state, ItemLocation::Inven, 0, Some(stack(15)));
    put(&mut state, ItemLocation::Inven, 1, Some(stack(12)));
    let before = server_view(&state);
    assert!(mv(&mut state, &clients, (INV, 0), (INV, 1)));
    assert_eq!(get(&state, ItemLocation::Inven, 1), Some(stack(20)));
    assert_eq!(get(&state, ItemLocation::Inven, 0), Some(stack(7)));
    assert_eq!(
        client_view(before.0, before.1, &drain(&mut owner)),
        server_view(&state)
    );
}

#[test]
fn trading_player_cannot_move_and_keeps_items() {
    let (mut state, clients, mut owner, _observer) = fixture();
    put(&mut state, ItemLocation::Inven, 3, Some(shirt(2)));
    put(&mut state, ItemLocation::Equip, UB as usize, Some(shirt(3)));
    state.get_player_mut(1).unwrap().trade_id = Some(uuid::Uuid::new_v4());
    let before = server_view(&state);
    assert!(!mv(&mut state, &clients, (INV, 3), (EQ, UB)));
    assert!(!mv(&mut state, &clients, (INV, 9), (EQ, UB)));
    assert_eq!(server_view(&state), before);
    assert!(drain(&mut owner).is_empty());
}

#[test]
fn bank_moves_and_expiry_are_preserved() {
    let (mut state, clients, _owner, _observer) = fixture();
    let mut timed = shirt(2);
    timed.set_expiry_time(
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(2_000_000_000),
    );
    put(&mut state, ItemLocation::Inven, 0, Some(timed));
    assert!(mv(&mut state, &clients, (INV, 0), (3, 4)));
    assert_eq!(get(&state, ItemLocation::Bank, 4), Some(timed));
    assert!(mv(&mut state, &clients, (3, 4), (EQ, UB)));
    assert_eq!(get(&state, ItemLocation::Equip, UB as usize), Some(timed));
    assert_eq!(get(&state, ItemLocation::Bank, 4), None);
}
