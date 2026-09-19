//! T02: accepted XDT identities reach the packet handlers (NPC summon, Nano tune).

use std::collections::{HashMap, HashSet};

use crate::{
    chunk::{EntityMap, TickMode},
    entity::Player,
    net::{
        packet::{Packet, PacketID::*, *},
        ClientMap, ClientMessage, ClientMetadata, ClientType, FFClient,
    },
    server::shard::{gm::gm_npc_summon, nano::nano_tune},
    state::ShardServerState,
    tabledata::tdata_init,
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
            "127.0.0.1:39004".parse().unwrap(),
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

fn next_reply(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ClientMessage>) -> Packet {
    let ClientMessage::SendPacket(reply) = rx.try_recv().unwrap() else {
        panic!("missing reply")
    };
    reply
}

#[test]
fn summon_accepts_barber_3470_and_rejects_unknown_type() {
    let (mut state, clients, _rx) = fixture();
    let summon = |ty: i32| {
        Packet::new(
            P_CL2FE_REQ_NPC_SUMMON,
            &sP_CL2FE_REQ_NPC_SUMMON {
                iNPCType: ty,
                iNPCCnt: 1,
            },
        )
        .unwrap()
    };

    gm_npc_summon(summon(3470), &ClientMap::new(1, &clients), &mut state).unwrap();
    let npc_ids: Vec<i32> = state.entity_map.get_npc_ids().collect();
    assert_eq!(npc_ids.len(), 1);
    assert_eq!(state.get_npc(npc_ids[0]).unwrap().ty, 3470);

    let e = gm_npc_summon(summon(3490), &ClientMap::new(1, &clients), &mut state).unwrap_err();
    assert_eq!(e.get_msg(), "NPC stats for type 3490 don't exist");
    assert_eq!(state.entity_map.get_npc_ids().count(), 1);
}

#[test]
fn unstable_nano_accepts_only_its_own_tunes() {
    let (mut state, clients, mut rx) = fixture();
    state.get_player_mut(1).unwrap().unlock_nano(41).unwrap();
    let tune = |tune_id: i16| {
        Packet::new(
            P_CL2FE_REQ_NANO_TUNE,
            &sP_CL2FE_REQ_NANO_TUNE {
                iNanoID: 41,
                iTuneID: tune_id,
                aiNeedItemSlotNum: [-1; 10],
            },
        )
        .unwrap()
    };
    let client = clients.get(&1).unwrap();

    // first tuning is free; all three Unstable tunes grant skill 122
    nano_tune(tune(288), client, &mut state).unwrap();
    let reply = next_reply(&mut rx);
    assert_eq!(reply.id(), P_FE2CL_REP_NANO_TUNE_SUCC);
    assert_eq!({ reply.get::<sP_FE2CL_REP_NANO_TUNE_SUCC>().unwrap().iSkillID }, 122);

    // Ben/Ghostfreak/Upgrade's tune 285 and Coop's former tune 210 are foreign to 41
    for foreign in [285, 210] {
        let _ = nano_tune(tune(foreign), client, &mut state);
        assert_eq!(next_reply(&mut rx).id(), P_FE2CL_REP_NANO_TUNE_FAIL, "tune {foreign}");
    }
    let player = state.get_player(1).unwrap();
    assert_eq!(player.get_nano(41).unwrap().selected_skill, Some(122));
}

#[test]
fn shared_skill_does_not_admit_another_nanos_tune() {
    // Ben (68) tune 285 and Coop (67) tune 210 both grant skill 210; the tune
    // number decides costs, so 68 must not be tuned through 210.
    let (mut state, clients, mut rx) = fixture();
    state.get_player_mut(1).unwrap().unlock_nano(68).unwrap();
    let tune = |tune_id: i16| {
        Packet::new(
            P_CL2FE_REQ_NANO_TUNE,
            &sP_CL2FE_REQ_NANO_TUNE {
                iNanoID: 68,
                iTuneID: tune_id,
                aiNeedItemSlotNum: [-1; 10],
            },
        )
        .unwrap()
    };
    let client = clients.get(&1).unwrap();

    let _ = nano_tune(tune(210), client, &mut state);
    assert_eq!(next_reply(&mut rx).id(), P_FE2CL_REP_NANO_TUNE_FAIL);
    assert_eq!(state.get_player(1).unwrap().get_nano(68).unwrap().selected_skill, None);

    nano_tune(tune(285), client, &mut state).unwrap();
    let reply = next_reply(&mut rx);
    assert_eq!(reply.id(), P_FE2CL_REP_NANO_TUNE_SUCC);
    assert_eq!({ reply.get::<sP_FE2CL_REP_NANO_TUNE_SUCC>().unwrap().iSkillID }, 210);
}
