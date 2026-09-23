//! Mob idle roaming and the return-home reset, against OpenFusion
//! `MobAI::roamingStep`, `retreatStep` and `onRoamStart`.
//!
//! NPC 59 is a level-2 mob (ai type 2, idle range 1000, sight 400, combat
//! range 4000, walk 300, run 500). NPC 3174 (Epmetaltriangle) has an idle
//! range but a walk speed of zero, so it must stay put, as must Lord Fuse
//! (2466, idle range 0). Skill 110 is the ReturnHomeHeal a mob casts on
//! itself once it is back at its spawn point.

use super::*;
use crate::{
    chunk::{ChunkCoords, InstanceID},
    entity::{Combatant, EntityID, Player},
    enums::{CharType, SkillType},
    net::{
        packet::{Packet, PacketID, PacketReader},
        ClientMessage, ClientMetadata, ClientType, FFClient,
    },
    scripting::{scripting_get, scripting_init},
    state::ShardServerState,
    tabledata::tdata_init,
    Position,
};

type Rx = tokio::sync::mpsc::UnboundedReceiver<ClientMessage>;

const PC_ID: i32 = 1;
const ROAMING_MOB: i32 = 59;
const IMMOBILE_MOB: i32 = 3174;
const BASE: Position = Position {
    x: 200_000,
    y: 200_000,
    z: 0,
};

fn client(port: u16, pc_id: i32) -> (FFClient, Rx) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            format!("127.0.0.1:{}", port).parse().unwrap(),
            Some(ClientType::GameClient {
                account_id: pc_id as i64,
                serial_key: pc_id as i64,
                pc_id: Some(pc_id),
            }),
        ),
    );
    (client, rx)
}

fn add_player(state: &mut ShardServerState, pc_id: i32, pos: Position, broadcast: bool) -> Rx {
    let (client, rx) = client(39300 + pc_id as u16, pc_id);
    let mut player = Player::new(pc_id as i64, 0);
    player.set_player_id(pc_id);
    player.set_client(client);
    player.set_level(36).unwrap();
    player.set_position(pos);
    let chunk = ChunkCoords::from_pos_inst(pos, InstanceID::default());
    let id = state.entity_map.track(Box::new(player), TickMode::Always);
    state.entity_map.update(id, Some(chunk), broadcast);
    rx
}

/// A mob at `BASE` running the script `make_for_npc` picks for it, watched
/// by a player standing `pc_offset` away.
fn fixture(npc_id: i32, npc_type: i32, pc_offset: i32) -> (ShardServerState, Rx) {
    tdata_init().unwrap();
    let _ = scripting_init();
    scripting_get().lock().remove_npc(npc_id);

    let mut state = ShardServerState::default();
    let rx = add_player(
        &mut state,
        PC_ID,
        Position {
            x: BASE.x + pc_offset,
            ..BASE
        },
        false,
    );

    let mut npc = NPC::new(npc_id, npc_type, BASE, 0, InstanceID::default()).unwrap();
    let (ai, tick_mode) = make_for_npc(&npc, false);
    npc.ai = ai;
    let chunk = npc.get_chunk_coords();
    let id = state.entity_map.track(Box::new(npc), tick_mode);
    state.entity_map.update(id, Some(chunk), false);
    (state, rx)
}

fn drain(rx: &mut Rx) -> Vec<Packet> {
    let mut out = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let ClientMessage::SendPacket(p) = msg {
            out.push(p);
        }
    }
    out
}

fn only(pkts: &[Packet], id: PacketID) -> Vec<&Packet> {
    pkts.iter().filter(|p| p.id() == id).collect()
}

fn tick(state: &mut ShardServerState, npc_id: i32) {
    NPC::tick(state, npc_id);
}

fn move_npc(state: &mut ShardServerState, npc_id: i32, pos: Position) {
    let npc = state.get_npc_mut(npc_id).unwrap();
    npc.set_position(pos);
    let chunk = npc.get_chunk_coords();
    state
        .entity_map
        .update(EntityID::NPC(npc_id), Some(chunk), false);
}

/// Every return-home heal the mob broadcast, as (header, result).
fn return_home_heals(
    pkts: &[Packet],
    npc_id: i32,
) -> Vec<(sP_FE2CL_NPC_SKILL_HIT, sSkillResult_Heal_HP)> {
    only(pkts, P_FE2CL_NPC_SKILL_HIT)
        .into_iter()
        .filter_map(|p| {
            let mut reader = PacketReader::new(p);
            let head = *reader.get_struct::<sP_FE2CL_NPC_SKILL_HIT>().unwrap();
            if head.iNPC_ID != npc_id || head.iSkillID != RETURN_HOME_HEAL_SKILL {
                return None;
            }
            let result = *reader.get_struct::<sSkillResult_Heal_HP>().unwrap();
            Some((head, result))
        })
        .collect()
}

#[test]
fn healing_a_mob_stops_at_max_hp() {
    tdata_init().unwrap();
    let mut npc = NPC::new(1, ROAMING_MOB, BASE, 0, InstanceID::default()).unwrap();
    let max_hp = npc.get_max_hp();
    npc.take_damage(100, None);
    assert_eq!(npc.heal(30), 30);
    assert_eq!(npc.get_hp(), max_hp - 70);
    assert_eq!(npc.heal(max_hp * 5), 70);
    assert_eq!(npc.get_hp(), max_hp);
    assert_eq!(npc.heal(10), 0);
    assert_eq!(npc.get_hp(), max_hp);
}

#[test]
fn ordinary_mobs_and_fusions_get_the_roaming_mob_script() {
    tdata_init().unwrap();
    // 59 is an ordinary mob, 1 is Fusion Eduardo
    for ty in [ROAMING_MOB, 1] {
        let npc = NPC::new(1, ty, BASE, 0, InstanceID::default()).unwrap();
        let stats = tdata_get().get_npc_stats(ty).unwrap();
        assert_eq!(stats.ai_type, 2);
        assert!(stats.idle_range > 0 && stats.walk_speed > 0);
        let (ai, tick_mode) = make_for_npc(&npc, false);
        assert_eq!(ai.as_deref(), Some("mob"), "type {ty}");
        assert!(matches!(tick_mode, TickMode::WhenLoaded));
    }
}

#[test]
fn idle_mob_roams_inside_its_idle_box() {
    let npc_id = 2_000_000_901;
    // the watcher is well outside sight range, so the mob stays idle
    let (mut state, mut rx) = fixture(npc_id, ROAMING_MOB, 3000);
    let half_box = tdata_get().get_npc_stats(ROAMING_MOB).unwrap().idle_range as i32 / 2;

    let mut moves = 0;
    let mut left_spawn = false;
    // two minutes of shard time
    for _ in 0..(120 * crate::defines::SHARD_TICKS_PER_SECOND) {
        tick(&mut state, npc_id);
        let npc = state.get_npc(npc_id).unwrap();
        let pos = npc.get_position();
        assert!(npc.target_id.is_none(), "an idle mob must not aggro");
        assert!(
            (pos.x - BASE.x).abs() <= half_box && (pos.y - BASE.y).abs() <= half_box,
            "roamed out of the idle box: {:?}",
            pos
        );
        assert_eq!(pos.z, BASE.z, "roaming never changes height");
        left_spawn |= pos != BASE;
        moves += only(&drain(&mut rx), P_FE2CL_NPC_MOVE).len();
    }
    assert!(left_spawn, "the mob never walked away from its spawn point");
    assert!(moves > 0, "the watching client never saw the mob walk");
}

#[test]
fn mobs_that_cannot_walk_do_not_roam() {
    for (npc_id, ty) in [
        (2_000_000_902, IMMOBILE_MOB),
        (2_000_000_903, NPC_TYPE_FUSE_STAGE_1),
    ] {
        let (mut state, mut rx) = fixture(npc_id, ty, 3000);
        for _ in 0..(60 * crate::defines::SHARD_TICKS_PER_SECOND) {
            tick(&mut state, npc_id);
        }
        let npc = state.get_npc(npc_id).unwrap();
        assert_eq!(npc.get_position(), BASE, "type {ty} wandered off");
        assert!(npc.path.is_none(), "type {ty} is stuck mid-walk");
        assert!(
            only(&drain(&mut rx), P_FE2CL_NPC_MOVE).is_empty(),
            "type {ty} broadcast movement"
        );
    }
}

/// Pulls the mob (at half HP) past its combat range with the player right
/// next to it, and ticks until it is home and reset. Returns every packet
/// the watching player saw.
fn retreat(state: &mut ShardServerState, rx: &mut Rx, npc_id: i32) -> Vec<Packet> {
    retreat_with(state, rx, npc_id, &[])
}

/// `retreat`, ticking `others` alongside the mob.
fn retreat_with(
    state: &mut ShardServerState,
    rx: &mut Rx,
    npc_id: i32,
    others: &[i32],
) -> Vec<Packet> {
    let far = Position {
        x: BASE.x + 4500,
        ..BASE
    };
    move_npc(state, npc_id, far);
    {
        let npc = state.get_npc_mut(npc_id).unwrap();
        npc.target_id = Some(EntityID::Player(PC_ID));
        let half = npc.get_max_hp() / 2;
        npc.take_damage(half, Some(EntityID::Player(PC_ID)));
        assert!(npc.get_hp() < npc.get_max_hp());
    }
    drain(rx);

    let mut pkts = Vec::new();
    let mut retreated = false;
    for _ in 0..(20 * crate::defines::SHARD_TICKS_PER_SECOND) {
        tick(state, npc_id);
        for other in others {
            tick(state, *other);
        }
        pkts.extend(drain(rx));
        let npc = state.get_npc(npc_id).unwrap();
        retreated |= npc.retreating;
        if retreated && !npc.retreating {
            break;
        }
    }
    assert!(retreated, "the mob never retreated");
    pkts
}

#[test]
fn return_home_resyncs_hp_for_watching_and_new_clients() {
    let npc_id = 2_000_000_904;
    let (mut state, mut rx) = fixture(npc_id, ROAMING_MOB, 4500);
    let max_hp = state.get_npc(npc_id).unwrap().get_max_hp();

    let pkts = retreat(&mut state, &mut rx, npc_id);

    let npc = state.get_npc(npc_id).unwrap();
    assert_eq!(npc.get_hp(), max_hp, "the server did not heal the mob");
    assert!(!npc.retreating);
    assert!(npc.target_id.is_none());

    // a client that watched the whole fight is told the new HP
    let heals = return_home_heals(&pkts, npc_id);
    assert_eq!(heals.len(), 1, "expected exactly one return-home heal");
    let (head, result) = heals[0];
    assert_eq!({ head.eST }, SkillType::ReturnHomeHeal as i32);
    assert_eq!({ head.iTargetCnt }, 1);
    assert_eq!({ result.eCT }, CharType::Mob as i32);
    assert_eq!({ result.iID }, npc_id);
    assert_eq!({ result.iHP }, max_hp);

    // the retreat is plain movement, never a respawn
    assert!(only(&pkts, P_FE2CL_NPC_NEW).is_empty());
    assert!(only(&pkts, P_FE2CL_NPC_ENTER).len() <= 1);

    // and one that walks up afterwards gets the same HP in the spawn packet
    let mut late_rx = add_player(&mut state, 2, BASE, true);
    let enters = drain(&mut late_rx);
    let enter = only(&enters, P_FE2CL_NPC_ENTER)
        .into_iter()
        .map(|p| {
            *PacketReader::new(p)
                .get_struct::<sP_FE2CL_NPC_ENTER>()
                .unwrap()
        })
        .find(|e| e.NPCAppearanceData.iNPC_ID == npc_id)
        .expect("the new client was not shown the mob");
    assert_eq!({ enter.NPCAppearanceData.iHP }, max_hp);
}

#[test]
fn mob_fights_again_after_returning_home() {
    let npc_id = 2_000_000_905;
    let (mut state, mut rx) = fixture(npc_id, ROAMING_MOB, 4500);
    retreat(&mut state, &mut rx, npc_id);

    // the player walks right up to it again
    {
        let pc = state.get_player_mut(PC_ID).unwrap();
        pc.set_position(BASE);
    }
    let chunk = ChunkCoords::from_pos_inst(BASE, InstanceID::default());
    state
        .entity_map
        .update(EntityID::Player(PC_ID), Some(chunk), false);
    drain(&mut rx);

    let mut attacks = 0;
    for _ in 0..(5 * crate::defines::SHARD_TICKS_PER_SECOND) {
        tick(&mut state, npc_id);
        let pkts = drain(&mut rx);
        attacks += only(&pkts, P_FE2CL_NPC_ATTACK_PCs).len();
        assert!(only(&pkts, P_FE2CL_NPC_ENTER).is_empty());
    }
    let npc = state.get_npc(npc_id).unwrap();
    assert_eq!(npc.target_id, Some(EntityID::Player(PC_ID)));
    assert!(!npc.retreating);
    assert!(attacks > 0, "the mob never attacked again");
}

/// Adds a pack member behind `leader_id`, the way tabledata spawns groups.
fn add_follower(state: &mut ShardServerState, leader_id: i32, follower_id: i32) -> Position {
    scripting_get().lock().remove_npc(follower_id);
    let offset = Position { x: 150, y: 0, z: 0 };
    let mut follower = NPC::new(
        follower_id,
        ROAMING_MOB,
        BASE + offset,
        0,
        InstanceID::default(),
    )
    .unwrap();
    follower.tight_follow = Some((EntityID::NPC(leader_id), offset));
    let (ai, tick_mode) = make_for_npc(&follower, false);
    assert_eq!(ai.as_deref(), Some("mob_pack_member"));
    follower.ai = ai;
    let chunk = follower.get_chunk_coords();
    let id = state.entity_map.track(Box::new(follower), tick_mode);
    state.entity_map.update(id, Some(chunk), false);
    offset
}

#[test]
fn pack_followers_walk_with_their_roaming_leader() {
    let leader_id = 2_000_000_908;
    let follower_id = 2_000_000_909;
    let (mut state, mut rx) = fixture(leader_id, ROAMING_MOB, 3000);
    let offset = add_follower(&mut state, leader_id, follower_id);

    let mut follower_moves = 0;
    for _ in 0..(120 * crate::defines::SHARD_TICKS_PER_SECOND) {
        tick(&mut state, leader_id);
        tick(&mut state, follower_id);
        follower_moves += only(&drain(&mut rx), P_FE2CL_NPC_MOVE)
            .into_iter()
            .filter(|p| {
                PacketReader::new(p)
                    .get_struct::<sP_FE2CL_NPC_MOVE>()
                    .unwrap()
                    .iNPC_ID
                    == follower_id
            })
            .count();
    }
    assert!(
        follower_moves > 0,
        "the follower never walked with its leader"
    );
    // and it keeps its place in the formation
    let leader = state.get_npc(leader_id).unwrap().get_position();
    let follower = state.get_npc(follower_id).unwrap().get_position();
    assert!(follower.distance_to(&(leader + offset)) <= 200);
}

#[test]
fn pack_followers_resync_hp_when_the_leader_gets_home() {
    let leader_id = 2_000_000_906;
    let follower_id = 2_000_000_907;
    let (mut state, mut rx) = fixture(leader_id, ROAMING_MOB, 4500);
    let offset = add_follower(&mut state, leader_id, follower_id);

    // the follower got hurt and dragged along behind the leader, but its own
    // target is gone, so only the leader's retreat can bring it home
    let far = Position {
        x: BASE.x + 4500,
        ..BASE
    };
    move_npc(&mut state, follower_id, far + offset);
    {
        let follower = state.get_npc_mut(follower_id).unwrap();
        let half = follower.get_max_hp() / 2;
        follower.take_damage(half, Some(EntityID::Player(PC_ID)));
        follower.target_id = None;
    }

    let mut pkts = retreat_with(&mut state, &mut rx, leader_id, &[follower_id]);
    // the follower notices the leader is home on its next tick
    tick(&mut state, follower_id);
    pkts.extend(drain(&mut rx));

    let follower = state.get_npc(follower_id).unwrap();
    let max_hp = follower.get_max_hp();
    assert_eq!(follower.get_hp(), max_hp);
    assert!(!follower.retreating);
    assert!(follower.target_id.is_none());
    let heals = return_home_heals(&pkts, follower_id);
    assert_eq!(
        heals.len(),
        1,
        "expected one return-home heal for the follower"
    );
    assert_eq!({ heals[0].1.iHP }, max_hp);
}
