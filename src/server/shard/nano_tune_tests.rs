//! T04: nano station tuning resolves the client's tune number to its skill,
//! charges `AvatarGrowth` FM plus the tune's items for a retune, and never
//! commits half of a transaction.

use std::collections::{HashMap, HashSet};

use crate::{
    chunk::{EntityMap, TickMode},
    defines::*,
    entity::{Combatant, Player},
    enums::*,
    item::Item,
    net::{
        packet::{Packet, PacketID::*, *},
        ClientMessage, ClientMetadata, ClientType, FFClient,
    },
    server::shard::nano::nano_tune,
    state::ShardServerState,
    tabledata::{tdata_get, tdata_init},
};

// Nano 38 (real rows): tune 201 -> skill 13, tune 202 -> skill 4 (passive);
// both cost 5 of general item 1.
const NANO: i16 = 38;
const TUNE_ACTIVE: i16 = 201;
const TUNE_PASSIVE: i16 = 202;
const REQ_ITEM: i16 = 1;

fn fixture(perms: i16) -> (
    ShardServerState,
    FFClient,
    tokio::sync::mpsc::UnboundedReceiver<ClientMessage>,
) {
    tdata_init().unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            "127.0.0.1:39014".parse().unwrap(),
            Some(ClientType::GameClient {
                account_id: 1,
                serial_key: 1,
                pc_id: Some(1),
            }),
        ),
    );
    let mut player = Player::new(1, 0);
    player.set_player_id(1);
    player.perms = perms;
    player.set_client(client.clone());
    player.unlock_nano(NANO).unwrap();
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
    (state, client, rx)
}

fn tune(nano_id: i16, tune_id: i16, slots: [i32; 10]) -> Packet {
    Packet::new(
        P_CL2FE_REQ_NANO_TUNE,
        &sP_CL2FE_REQ_NANO_TUNE {
            iNanoID: nano_id,
            iTuneID: tune_id,
            aiNeedItemSlotNum: slots,
        },
    )
    .unwrap()
}

fn replies(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ClientMessage>) -> Vec<Packet> {
    let mut out = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let ClientMessage::SendPacket(pkt) = msg {
            out.push(pkt);
        }
    }
    out
}

fn tune_cost(player: &Player) -> u32 {
    tdata_get()
        .get_player_stats(player.get_level())
        .unwrap()
        .req_fm_nano_tune
}

/// Tunes for free, then funds exactly one retune: `extra_fm` above the price
/// and `items` general item 1 split 3 + rest across slots 0 and 1.
fn tuned_and_funded(
    state: &mut ShardServerState,
    client: &FFClient,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<ClientMessage>,
    extra_fm: i64,
    items: u16,
) -> u32 {
    nano_tune(tune(NANO, TUNE_ACTIVE, [-1; 10]), client, state).unwrap();
    replies(rx);
    let player = state.get_player_mut(1).unwrap();
    let fm = (tune_cost(player) as i64 + extra_fm) as u32;
    player.set_fusion_matter(fm);
    for (slot, quantity) in [(0, items.min(3)), (1, items.saturating_sub(3))] {
        let stack = (quantity > 0).then(|| {
            let mut item = Item::new(ItemType::General, REQ_ITEM);
            item.quantity = quantity;
            item
        });
        player.set_item(ItemLocation::Inven, slot, stack).unwrap();
    }
    replies(rx);
    fm
}

fn inven_quantity(state: &ShardServerState, slot: usize) -> u16 {
    let mut player = state.get_player(1).unwrap().clone();
    player
        .get_item_mut(ItemLocation::Inven, slot)
        .unwrap()
        .map_or(0, |item| item.quantity)
}

fn skill(state: &ShardServerState) -> Option<i16> {
    state.get_player(1).unwrap().get_nano(NANO).unwrap().selected_skill
}

const PAY_SLOTS: [i32; 10] = [0, 1, -1, -1, -1, -1, -1, -1, -1, -1];

#[test]
fn free_tuning_resolves_tune_number_to_skill() {
    // tune number == skill (Buttercup's first row) and tune number != skill
    let (mut state, client, mut rx) = fixture(30);
    state.get_player_mut(1).unwrap().unlock_nano(1).unwrap();
    for (nano, tune_id, skill_id) in [(1, 1, 1), (NANO, TUNE_ACTIVE, 13)] {
        let fm = state.get_player(1).unwrap().get_fusion_matter();
        nano_tune(tune(nano, tune_id, [-1; 10]), &client, &mut state).unwrap();
        let reply = replies(&mut rx).pop().unwrap();
        assert_eq!(reply.id(), P_FE2CL_REP_NANO_TUNE_SUCC);
        let succ: &sP_FE2CL_REP_NANO_TUNE_SUCC = reply.get().unwrap();
        assert_eq!({ succ.iSkillID }, skill_id);
        assert_eq!({ succ.aiItemSlotNum }, [-1; 10]);
        let player = state.get_player(1).unwrap();
        assert_eq!(player.get_nano(nano).unwrap().selected_skill, Some(skill_id));
        assert_eq!(player.get_fusion_matter(), fm, "first tuning is free");
    }
}

#[test]
fn paid_retune_charges_growth_fm_and_items() {
    let (mut state, client, mut rx) = fixture(30);
    tuned_and_funded(&mut state, &client, &mut rx, 7, 6);

    nano_tune(tune(NANO, TUNE_PASSIVE, PAY_SLOTS), &client, &mut state).unwrap();
    let reply = replies(&mut rx).pop().unwrap();
    assert_eq!(reply.id(), P_FE2CL_REP_NANO_TUNE_SUCC);
    let succ: &sP_FE2CL_REP_NANO_TUNE_SUCC = reply.get().unwrap();
    assert_eq!({ succ.iSkillID }, 4);
    assert_eq!({ succ.iPC_FusionMatter }, 7);
    assert_eq!(&{ succ.aiItemSlotNum }[..3], &[0, 1, -1]);
    assert_eq!({ succ.aItem[0].iID }, 0, "slot 0 emptied");
    assert_eq!(({ succ.aItem[1].iID }, { succ.aItem[1].iOpt }), (REQ_ITEM, 1));

    assert_eq!(skill(&state), Some(4));
    assert_eq!(state.get_player(1).unwrap().get_fusion_matter(), 7);
    assert_eq!((inven_quantity(&state, 0), inven_quantity(&state, 1)), (0, 1));
}

#[test]
fn failed_retune_keeps_skill_fm_and_items() {
    // (fm above price, items owned, item type)
    let cases = [
        (-1, 5, ItemType::General), // one FM short
        (0, 4, ItemType::General),  // one item short
        (0, 5, ItemType::Chest),    // right ID, wrong item table
    ];
    for (extra_fm, items, ty) in cases {
        let (mut state, client, mut rx) = fixture(30);
        let fm = tuned_and_funded(&mut state, &client, &mut rx, extra_fm, items);
        if ty != ItemType::General {
            let player = state.get_player_mut(1).unwrap();
            for slot in [0, 1] {
                let slot = player.get_item_mut(ItemLocation::Inven, slot).unwrap();
                if let Some(item) = slot {
                    item.ty = ty;
                }
            }
        }

        let _ = nano_tune(tune(NANO, TUNE_PASSIVE, PAY_SLOTS), &client, &mut state);
        let sent = replies(&mut rx);
        assert_eq!(sent.len(), 1, "case {extra_fm}/{items}/{ty:?}");
        assert_eq!(sent[0].id(), P_FE2CL_REP_NANO_TUNE_FAIL);
        assert_eq!(skill(&state), Some(13));
        assert_eq!(state.get_player(1).unwrap().get_fusion_matter(), fm);
        assert_eq!(
            inven_quantity(&state, 0) + inven_quantity(&state, 1),
            items,
            "case {extra_fm}/{items}/{ty:?}"
        );
    }
}

#[test]
fn foreign_locked_unknown_and_repeated_requests_change_nothing() {
    let (mut state, client, mut rx) = fixture(30);
    let fm = tuned_and_funded(&mut state, &client, &mut rx, 0, 10);
    let requests = [
        tune(NANO, 1, PAY_SLOTS),            // Buttercup's tune
        tune(NANO, 0, PAY_SLOTS),            // empty row
        tune(NANO, 999, PAY_SLOTS),          // no such tune
        tune(1, 1, PAY_SLOTS),               // nano not owned
        tune(NANO, TUNE_ACTIVE, PAY_SLOTS),  // repeat of the current tune
        tune(NANO, TUNE_PASSIVE, [-1; 10]),  // paid retune without item slots
    ];
    for request in requests {
        let _ = nano_tune(request, &client, &mut state);
        assert_eq!(replies(&mut rx)[0].id(), P_FE2CL_REP_NANO_TUNE_FAIL);
    }
    assert_eq!(skill(&state), Some(13));
    assert!(state.get_player(1).unwrap().get_nano(1).is_none());
    assert_eq!(state.get_player(1).unwrap().get_fusion_matter(), fm);
    assert_eq!(inven_quantity(&state, 0) + inven_quantity(&state, 1), 10);

    // one paid retune, then its duplicate is rejected without a second charge
    nano_tune(tune(NANO, TUNE_PASSIVE, PAY_SLOTS), &client, &mut state).unwrap();
    assert_eq!(replies(&mut rx)[0].id(), P_FE2CL_REP_NANO_TUNE_SUCC);
    let _ = nano_tune(tune(NANO, TUNE_PASSIVE, PAY_SLOTS), &client, &mut state);
    assert_eq!(replies(&mut rx)[0].id(), P_FE2CL_REP_NANO_TUNE_FAIL);
    assert_eq!(skill(&state), Some(4));
    assert_eq!(state.get_player(1).unwrap().get_fusion_matter(), 0);
    assert_eq!(inven_quantity(&state, 0) + inven_quantity(&state, 1), 5);
}

#[test]
fn retune_requires_a_nano_station_but_first_tuning_does_not() {
    let (mut state, client, mut rx) = fixture(CN_ACCOUNT_LEVEL__USER as i16);
    let fm = tuned_and_funded(&mut state, &client, &mut rx, 0, 5);
    assert_eq!(skill(&state), Some(13), "free tuning away from a station");

    let _ = nano_tune(tune(NANO, TUNE_PASSIVE, PAY_SLOTS), &client, &mut state);
    assert_eq!(replies(&mut rx)[0].id(), P_FE2CL_REP_NANO_TUNE_FAIL);
    assert_eq!(skill(&state), Some(13));
    assert_eq!(state.get_player(1).unwrap().get_fusion_matter(), fm);
    assert_eq!(inven_quantity(&state, 0) + inven_quantity(&state, 1), 5);
}

#[test]
fn retuning_the_active_nano_recalls_it_and_drops_its_passive_buff() {
    let (mut state, client, mut rx) = fixture(30);
    nano_tune(tune(NANO, TUNE_PASSIVE, [-1; 10]), &client, &mut state).unwrap();
    let player = state.get_player_mut(1).unwrap();
    player.change_nano(0, Some(NANO)).unwrap();
    assert!(player.activate_nano(0).unwrap(), "skill 4 is passive");
    let buff = tdata_get().get_skill(4).unwrap().get_buff_id().unwrap();
    assert!(player.has_buff(buff, Some(BuffType::Nano)));
    player.set_fusion_matter(tune_cost(player));
    let mut item = Item::new(ItemType::General, REQ_ITEM);
    item.quantity = 5;
    player.set_item(ItemLocation::Inven, 0, Some(item)).unwrap();
    replies(&mut rx);

    nano_tune(tune(NANO, TUNE_ACTIVE, PAY_SLOTS), &client, &mut state).unwrap();
    let ids: Vec<_> = replies(&mut rx).iter().map(|p| p.id()).collect();
    assert_eq!(ids.first(), Some(&P_FE2CL_REP_NANO_ACTIVE_SUCC));
    assert_eq!(ids.last(), Some(&P_FE2CL_REP_NANO_TUNE_SUCC));
    let player = state.get_player(1).unwrap();
    assert_eq!(player.get_active_nano_slot(), None);
    // the emptied stack is dropped (and reported) on the next buff tick
    assert!(!player.has_buff(buff, Some(BuffType::Nano)), "old passive buff must not linger");
    assert_eq!(player.get_nano(NANO).unwrap().selected_skill, Some(13));
}
