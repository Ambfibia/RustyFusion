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

    let e = gm_npc_summon(summon(i32::MAX), &ClientMap::new(1, &clients), &mut state).unwrap_err();
    assert_eq!(e.get_msg(), "NPC stats for type 2147483647 don't exist");
    assert_eq!(state.entity_map.get_npc_ids().count(), 1);
}

#[test]
fn unstable_nano_accepts_only_its_own_tunes() {
    let (mut state, clients, mut rx) = fixture();
    state.get_player_mut(1).unwrap().unlock_nano(39).unwrap();
    let tune = |tune_id: i16| {
        Packet::new(
            P_CL2FE_REQ_NANO_TUNE,
            &sP_CL2FE_REQ_NANO_TUNE {
                iNanoID: 39,
                iTuneID: tune_id,
                aiNeedItemSlotNum: [-1; 10],
            },
        )
        .unwrap()
    };
    let client = clients.get(&1).unwrap();

    // first tuning is free; all three Unstable tunes grant skill 287
    nano_tune(tune(291), client, &mut state).unwrap();
    let reply = next_reply(&mut rx);
    assert_eq!(reply.id(), P_FE2CL_REP_NANO_TUNE_SUCC);
    assert_eq!({ reply.get::<sP_FE2CL_REP_NANO_TUNE_SUCC>().unwrap().iSkillID }, 287);

    // Ben/Ghostfreak/Upgrade's tune 285 and Coop's former tune 210 are foreign to 39
    for foreign in [285, 210, 288] {
        let _ = nano_tune(tune(foreign), client, &mut state);
        assert_eq!(next_reply(&mut rx).id(), P_FE2CL_REP_NANO_TUNE_FAIL, "tune {foreign}");
    }
    let player = state.get_player(1).unwrap();
    assert_eq!(player.get_nano(39).unwrap().selected_skill, Some(287));
}

#[test]
fn shared_skill_does_not_admit_another_nanos_tune() {
    // Ben (64) tune 285 and Coop (63) tune 210 both grant skill 210; the tune
    // number decides costs, so 64 must not be tuned through 210.
    let (mut state, clients, mut rx) = fixture();
    state.get_player_mut(1).unwrap().unlock_nano(64).unwrap();
    let tune = |tune_id: i16| {
        Packet::new(
            P_CL2FE_REQ_NANO_TUNE,
            &sP_CL2FE_REQ_NANO_TUNE {
                iNanoID: 64,
                iTuneID: tune_id,
                aiNeedItemSlotNum: [-1; 10],
            },
        )
        .unwrap()
    };
    let client = clients.get(&1).unwrap();

    let _ = nano_tune(tune(210), client, &mut state);
    assert_eq!(next_reply(&mut rx).id(), P_FE2CL_REP_NANO_TUNE_FAIL);
    assert_eq!(state.get_player(1).unwrap().get_nano(64).unwrap().selected_skill, None);

    nano_tune(tune(285), client, &mut state).unwrap();
    let reply = next_reply(&mut rx);
    assert_eq!(reply.id(), P_FE2CL_REP_NANO_TUNE_SUCC);
    assert_eq!({ reply.get::<sP_FE2CL_REP_NANO_TUNE_SUCC>().unwrap().iSkillID }, 210);
}

#[test]
fn hidden_placement_is_admin_only_and_native_appearance_preserves_enter_wire(){
    use crate::entity::{Entity,NPC};
    let (mut state,clients,mut rx)=fixture();
    let mut npc=NPC::new(91,707,crate::Position::default(),0,crate::chunk::InstanceID::default()).unwrap();
    npc.hidden=true;npc.map_icon=false;npc.scale=2.;
    let mut ordinary=Player::new(2,0);ordinary.perms=100;
    assert!(!npc.visible_to(&ordinary));assert!(npc.visible_to(state.get_player(1).unwrap()));
    npc.send_enter(&clients[&1]);assert_eq!(next_reply(&mut rx).id(),P_FE2CL_NPC_ENTER);
    let settings=next_reply(&mut rx);assert_eq!(settings.id(),P_FE2CL_NPC_EDITOR_SETTINGS);let wire=*settings.get::<NpcEditorSettings>().unwrap();assert_eq!((wire.id,wire.flags,wire.scale),(91,1,2.));assert_eq!(std::mem::size_of::<NpcEditorSettings>(),12);
    state.entity_map.track(Box::new(npc),TickMode::Always);state.get_player_mut(1).unwrap().perms=100;
    let interact=||Packet::new(P_CL2FE_REQ_NPC_INTERACTION,&sP_CL2FE_REQ_NPC_INTERACTION {iNPC_ID:91,bFlag:1}).unwrap();
    assert!(super::npc::npc_interaction(interact(),&clients[&1],&mut state).is_err());
    state.get_player_mut(1).unwrap().perms=30;super::npc::npc_interaction(interact(),&clients[&1],&mut state).unwrap();
}
#[test]
fn ambient_bark_is_server_approved_and_timed_requirement_checks_the_actual_speaker(){
    use crate::entity::{Entity,NPC};use crate::tabledata::tdata_get;
    let (mut state,clients,mut rx)=fixture();
    let (npc_type,set)=(1..4000).find_map(|ty|tdata_get().get_npc_ambient(ty).ok().and_then(|sets|sets.iter().find(|s|s.conditions.allows(state.get_player(1).unwrap())&&s.lines.iter().any(|s|!s.trim().is_empty()))).map(|set|(ty,set.set))).unwrap();
    let player=state.get_player(1).unwrap();let npc=NPC::new(92,npc_type,player.get_position(),0,player.get_instance_id()).unwrap();
    state.entity_map.track(Box::new(npc),TickMode::Always);
    let request=|set|Packet::new(P_CL2FE_REQ_AMBIENT_BARK,&NativeAmbientBark {npc_id:92,set,line:0}).unwrap();
    super::npc::ambient_bark(request(i32::MAX),&clients[&1],&mut state).unwrap();assert!(rx.try_recv().is_err());
    super::npc::ambient_bark(request(set),&clients[&1],&mut state).unwrap();let response=next_reply(&mut rx);assert_eq!(response.id(),P_FE2CL_AMBIENT_BARK);let bark=*response.get::<NativeAmbientBark>().unwrap();
    assert_eq!(bark.set,set);assert!((0..4).contains(&bark.line));
    let mut rule=crate::barker::RequiredBark {line:set*4+bark.line,npc:npc_type,seconds:10};assert!(rule.allows(state.get_player(1).unwrap()));
    rule.npc+=1;assert!(!rule.allows(state.get_player(1).unwrap()));rule.npc-=1;
    state.get_player_mut(1).unwrap().recent_barks.insert((rule.npc,rule.line),std::time::Instant::now()-std::time::Duration::from_secs(11));assert!(!rule.allows(state.get_player(1).unwrap()));
    super::npc::ambient_bark(request(set),&clients[&1],&mut state).unwrap();assert!(rx.try_recv().is_err());
}
