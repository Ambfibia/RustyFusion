//! Nano crowd control on mobs against OpenFusion `Abilities::getCSTBFromST`,
//! `handleSkillDamageNDebuff`, `Mob::takeDamage` and `MobAI::combatStep`.
//!
//! Skill 1 (tune 1) is a single-target Stun, 3s at level 0; skill 28 is a
//! single-target Sleep, 8s at level 0. NPC 59 is a level-2 mob running `mob`.

use super::*;
use crate::{
    chunk::{ChunkCoords, TickMode},
    entity::{Player, NPC},
    net::{packet::Packet, ClientMessage, ClientMetadata, ClientType, FFClient},
    scripting::{scripting_get, scripting_init},
    tabledata::{tdata_get, tdata_init},
};

type Rx = tokio::sync::mpsc::UnboundedReceiver<ClientMessage>;

const PC_ID: i32 = 1;
const MOB_TYPE: i32 = 59;
const STUN_SKILL: i16 = 1;
const SLEEP_SKILL: i16 = 28;

fn fixture(npc_id: i32) -> (ShardServerState, Rx) {
    tdata_init().unwrap();
    let mut state = ShardServerState::default();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            "127.0.0.1:39202".parse().unwrap(),
            Some(ClientType::GameClient {
                account_id: 1,
                serial_key: 1,
                pc_id: Some(PC_ID),
            }),
        ),
    );
    let mut player = Player::new(1, 0);
    player.set_player_id(PC_ID);
    player.set_client(client);
    player.set_level(36).unwrap();
    let pos = player.get_position();
    let chunk = ChunkCoords::from_pos_inst(pos, InstanceID::default());
    let id = state.entity_map.track(Box::new(player), TickMode::Always);
    state.entity_map.update(id, Some(chunk), false);

    let npc = NPC::new(npc_id, MOB_TYPE, pos, 0, InstanceID::default()).unwrap();
    let id = state.entity_map.track(Box::new(npc), TickMode::Always);
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

fn count(pkts: &[Packet], id: PacketID) -> usize {
    pkts.iter().filter(|p| p.id() == id).count()
}

fn cast(state: &mut ShardServerState, npc_id: i32, skill_id: i16) -> SkillResult {
    let skill = tdata_get().get_skill(skill_id).unwrap();
    let mut results = do_skill(
        EntityID::Player(PC_ID),
        &[EntityID::NPC(npc_id)],
        skill,
        0,
        state,
    )
    .unwrap();
    assert_eq!(results.len(), 1);
    results.remove(0)
}

fn debuff_result(result: SkillResult) -> sSkillResult_Damage_N_Debuff {
    match result {
        SkillResult::DamageAndDebuff(r) => r,
        _ => panic!("expected a Damage_N_Debuff result"),
    }
}

fn short_buff(ms: u64) -> BuffInstance {
    BuffInstance::new(
        BuffType::Nano,
        1,
        None,
        None,
        Some(Duration::from_millis(ms)),
    )
}

fn tick(state: &mut ShardServerState, npc_id: i32, times: usize) {
    for _ in 0..times {
        NPC::tick(state, npc_id);
    }
}

fn with_ai(state: &mut ShardServerState, npc_id: i32) {
    let _ = scripting_init();
    scripting_get().lock().remove_npc(npc_id);
    let npc = state.get_npc_mut(npc_id).unwrap();
    npc.ai = Some("mob".to_string());
    npc.target_id = Some(EntityID::Player(PC_ID));
}

#[test]
fn stun_nano_stuns_the_mob_for_the_table_duration() {
    let npc_id = 2_000_000_101;
    let (mut state, _rx) = fixture(npc_id);
    let skill = tdata_get().get_skill(STUN_SKILL).unwrap();
    assert_eq!(skill.skill_type, SkillType::Stun);
    assert_eq!(skill.get_buff_id(), Some(BuffID::Stun));

    let r = debuff_result(cast(&mut state, npc_id, STUN_SKILL));
    assert_eq!({ r.bProtected }, 0);
    // OpenFusion reports the duration (in seconds) as the damage number
    assert_eq!({ r.iDamage }, 3);
    // the result already carries the stun bit, like getCompositeCondition()
    assert_ne!({ r.iConditionBitFlag } as u32 & CSB_BIT_STUN, 0);

    let npc = state.get_npc(npc_id).unwrap();
    assert!(npc.has_buff(BuffID::Stun, None));
    assert!(!npc.has_buff(BuffID::Sleep, None), "stun is not sleep");
    // casting a CC nano takes aggro
    assert_eq!(npc.target_id, Some(EntityID::Player(PC_ID)));
}

#[test]
fn freedom_makes_the_target_immune() {
    let npc_id = 2_000_000_102;
    let (mut state, _rx) = fixture(npc_id);
    state
        .get_npc_mut(npc_id)
        .unwrap()
        .apply_buff(BuffID::Freedom, short_buff(60_000), None);

    let r = debuff_result(cast(&mut state, npc_id, STUN_SKILL));
    assert_eq!({ r.bProtected }, 1);
    assert_eq!({ r.iDamage }, 0);
    assert!(!state.get_npc(npc_id).unwrap().has_buff(BuffID::Stun, None));
}

#[test]
fn damage_wakes_sleep_but_not_stun() {
    let npc_id = 2_000_000_103;
    let (mut state, mut rx) = fixture(npc_id);
    // stun first: the stun nano's own zero-damage aggro hit would wake a
    // sleeping mob, exactly like OpenFusion
    cast(&mut state, npc_id, STUN_SKILL);
    cast(&mut state, npc_id, SLEEP_SKILL);
    tick(&mut state, npc_id, 1);
    let flags = state.get_npc(npc_id).unwrap().get_condition_bit_flag() as u32;
    assert_eq!(
        flags & (CSB_BIT_MEZ | CSB_BIT_STUN),
        CSB_BIT_MEZ | CSB_BIT_STUN
    );
    drain(&mut rx);

    state
        .get_npc_mut(npc_id)
        .unwrap()
        .take_damage(1, Some(EntityID::Player(PC_ID)));
    tick(&mut state, npc_id, 1);

    let npc = state.get_npc(npc_id).unwrap();
    assert!(
        !npc.has_buff(BuffID::Sleep, None),
        "a hit wakes a sleeping mob"
    );
    assert!(
        npc.has_buff(BuffID::Stun, None),
        "a hit does not end a stun"
    );
    let flags = npc.get_condition_bit_flag() as u32;
    assert_eq!(flags & (CSB_BIT_MEZ | CSB_BIT_STUN), CSB_BIT_STUN);
    assert_eq!(count(&drain(&mut rx), P_FE2CL_CHAR_TIME_BUFF_TIME_OUT), 1);
}

#[test]
fn recasting_sleep_on_a_sleeping_mob_puts_it_back_to_sleep() {
    let npc_id = 2_000_000_104;
    let (mut state, _rx) = fixture(npc_id);
    cast(&mut state, npc_id, SLEEP_SKILL);
    tick(&mut state, npc_id, 1);
    // the nano's own zero-damage aggro hit wakes, then the debuff reapplies
    cast(&mut state, npc_id, SLEEP_SKILL);
    tick(&mut state, npc_id, 1);
    assert!(state.get_npc(npc_id).unwrap().has_buff(BuffID::Sleep, None));
}

#[test]
fn stunning_a_sleeping_mob_wakes_it() {
    let npc_id = 2_000_000_108;
    let (mut state, _rx) = fixture(npc_id);
    cast(&mut state, npc_id, SLEEP_SKILL);
    tick(&mut state, npc_id, 1);
    cast(&mut state, npc_id, STUN_SKILL);
    tick(&mut state, npc_id, 1);
    let npc = state.get_npc(npc_id).unwrap();
    assert!(!npc.has_buff(BuffID::Sleep, None));
    assert!(npc.has_buff(BuffID::Stun, None));
}

#[test]
fn retreat_and_respawn_reset_clears_crowd_control() {
    let npc_id = 2_000_000_105;
    let (mut state, mut rx) = fixture(npc_id);
    cast(&mut state, npc_id, STUN_SKILL);
    cast(&mut state, npc_id, SLEEP_SKILL);
    tick(&mut state, npc_id, 1);
    drain(&mut rx);

    // retreat/respawn go through reset()
    state.get_npc_mut(npc_id).unwrap().reset();
    tick(&mut state, npc_id, 1);
    let npc = state.get_npc(npc_id).unwrap();
    assert!(!npc.has_buff(BuffID::Stun, None));
    assert!(!npc.has_buff(BuffID::Sleep, None));
    assert_eq!(npc.get_condition_bit_flag(), 0);
    assert_eq!(count(&drain(&mut rx), P_FE2CL_CHAR_TIME_BUFF_TIME_OUT), 1);
}

#[test]
fn stunned_mob_stops_running_and_does_not_attack_then_resumes() {
    let npc_id = 2_000_000_106;
    let (mut state, mut rx) = fixture(npc_id);
    with_ai(&mut state, npc_id);

    // mid-run: a path is in flight when the stun lands
    let far = {
        let npc = state.get_npc_mut(npc_id).unwrap();
        let mut far = npc.get_position();
        far.x += 5000;
        npc.move_towards(far, None);
        assert!(npc.path.is_some());
        far
    };
    let start = state.get_npc(npc_id).unwrap().get_position();
    state.get_npc_mut(npc_id).unwrap().apply_buff(
        BuffID::Stun,
        short_buff(600),
        Some(EntityID::Player(PC_ID)),
    );

    tick(&mut state, npc_id, 4);
    let npc = state.get_npc(npc_id).unwrap();
    assert!(npc.path.is_none(), "a stunned mob drops its path");
    assert_eq!(npc.get_position(), start, "a stunned mob does not move");
    assert_ne!(npc.get_position(), far);
    assert_eq!(
        npc.target_id,
        Some(EntityID::Player(PC_ID)),
        "stun keeps aggro"
    );
    let pkts = drain(&mut rx);
    assert_eq!(
        count(&pkts, P_FE2CL_NPC_ATTACK_PCs),
        0,
        "no attacks while stunned"
    );
    assert_eq!(count(&pkts, P_FE2CL_NPC_MOVE), 0);

    std::thread::sleep(Duration::from_millis(650));
    // the stun times out on the next tick and the mob attacks again
    tick(&mut state, npc_id, 2);
    let npc = state.get_npc(npc_id).unwrap();
    assert!(!npc.has_buff(BuffID::Stun, None));
    assert_eq!(npc.get_condition_bit_flag() as u32 & CSB_BIT_STUN, 0);
    let pkts = drain(&mut rx);
    assert!(count(&pkts, P_FE2CL_CHAR_TIME_BUFF_TIME_OUT) >= 1);
    assert_eq!(
        count(&pkts, P_FE2CL_NPC_ATTACK_PCs),
        1,
        "attacks resume after stun"
    );

    scripting_get().lock().remove_npc(npc_id);
}

#[test]
fn sleeping_mob_does_not_attack_until_woken() {
    let npc_id = 2_000_000_107;
    let (mut state, mut rx) = fixture(npc_id);
    with_ai(&mut state, npc_id);
    cast(&mut state, npc_id, SLEEP_SKILL);

    tick(&mut state, npc_id, 4);
    assert_eq!(count(&drain(&mut rx), P_FE2CL_NPC_ATTACK_PCs), 0);

    state
        .get_npc_mut(npc_id)
        .unwrap()
        .take_damage(1, Some(EntityID::Player(PC_ID)));
    tick(&mut state, npc_id, 1);
    assert_eq!(count(&drain(&mut rx), P_FE2CL_NPC_ATTACK_PCs), 1);

    scripting_get().lock().remove_npc(npc_id);
}

#[test]
fn a_dead_mob_keeps_no_crowd_control() {
    let npc_id = 2_000_000_109;
    let (mut state, mut rx) = fixture(npc_id);
    with_ai(&mut state, npc_id);
    cast(&mut state, npc_id, STUN_SKILL);
    tick(&mut state, npc_id, 1);
    assert!(state.get_npc(npc_id).unwrap().has_buff(BuffID::Stun, None));
    drain(&mut rx);

    let npc = state.get_npc_mut(npc_id).unwrap();
    let hp = npc.get_hp();
    npc.take_damage(hp, Some(EntityID::Player(PC_ID)));
    // death handling runs, then the emptied stack is dropped and reported
    tick(&mut state, npc_id, 2);
    let npc = state.get_npc(npc_id).unwrap();
    assert!(npc.is_dead());
    assert!(!npc.has_buff(BuffID::Stun, None));
    assert_eq!(npc.get_condition_bit_flag(), 0);
    assert!(count(&drain(&mut rx), P_FE2CL_CHAR_TIME_BUFF_TIME_OUT) >= 1);

    scripting_get().lock().remove_npc(npc_id);
}
