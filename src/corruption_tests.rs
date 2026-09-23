//! NPC corruption attacks against OpenFusion `MobAI::useAbilities` and
//! `dealCorruption`.
//!
//! NPC 75 (Motor Raptor, level 4) carries corruption skill 118 at weight
//! 100,000; NPC 59 (level 2) has none. Skill 118's first value is 240, so a
//! full hit is 240 * PC_MAXHEALTH(4) / 1500 = 240 * 1225 / 1500 = 196.
//! Nanos 2, 1 and 3 are Adaptium, Blastons and Cosmix.

use std::time::Duration;

use super::*;
use crate::{
    chunk::{ChunkCoords, InstanceID, TickMode},
    entity::{Player, NPC},
    net::{
        packet::{Packet, PacketReader},
        ClientMessage, ClientMetadata, ClientType, FFClient,
    },
    scripting::{scripting_get, scripting_init},
    skills::BuffInstance,
    enums::BuffType,
    tabledata::tdata_init,
};

type Rx = tokio::sync::mpsc::UnboundedReceiver<ClientMessage>;

const PC_ID: i32 = 1;
const CORRUPT_MOB: i32 = 75;
const PLAIN_MOB: i32 = 59;
const SKILL: i16 = 118;
const FULL_HIT: i32 = 196;
/// Carry slots 0..3 hold an Adaptium, a Blastons and a Cosmix nano.
const NANOS: [(i16, CombatStyle); 3] = [
    (2, CombatStyle::Adaptium),
    (1, CombatStyle::Blastons),
    (3, CombatStyle::Cosmix),
];

fn slot_of(style: CombatStyle) -> usize {
    NANOS.iter().position(|(_, s)| *s == style).unwrap()
}

fn fixture(npc_id: i32, mob_type: i32) -> (ShardServerState, Rx) {
    tdata_init().unwrap();
    let mut state = ShardServerState::default();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            "127.0.0.1:39203".parse().unwrap(),
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
    player.set_level(10).unwrap();
    let max_hp = player.get_max_hp();
    player.set_hp(max_hp);
    for (slot, (nano_id, _)) in NANOS.iter().enumerate() {
        let skill = tdata_get().get_nano_stats(*nano_id).unwrap().skills[0];
        player.unlock_nano(*nano_id).unwrap().tune(Some(skill));
        player.change_nano(slot, Some(*nano_id)).unwrap();
    }
    let pos = player.get_position();
    let chunk = ChunkCoords::from_pos_inst(pos, InstanceID::default());
    let id = state.entity_map.track(Box::new(player), TickMode::Always);
    state.entity_map.update(id, Some(chunk), false);

    let mut npc = NPC::new(npc_id, mob_type, pos, 0, InstanceID::default()).unwrap();
    npc.target_id = Some(EntityID::Player(PC_ID));
    let id = state.entity_map.track(Box::new(npc), TickMode::Always);
    state.entity_map.update(id, Some(chunk), false);
    (state, rx)
}

fn use_nano(state: &mut ShardServerState, style: Option<CombatStyle>) {
    let player = state.get_player_mut(PC_ID).unwrap();
    match style {
        Some(style) => {
            player.activate_nano(slot_of(style)).unwrap();
        }
        None => player.deactivate_nano(),
    }
}

fn set_stamina(state: &mut ShardServerState, stamina: i16) {
    let player = state.get_player_mut(PC_ID).unwrap();
    player.get_active_nano_mut().unwrap().set_stamina(stamina);
}

fn with_roll<T>(roll: i32, f: impl FnOnce() -> T) -> T {
    FORCED_ROLL.with(|r| r.set(Some(roll)));
    let out = f();
    FORCED_ROLL.with(|r| r.set(None));
    out
}

fn begin(state: &mut ShardServerState, npc_id: i32) -> bool {
    with_roll(0, || try_begin(npc_id, state).unwrap())
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

fn ready(pkts: &[Packet]) -> sP_FE2CL_NPC_SKILL_CORRUPTION_READY {
    let found = only(pkts, P_FE2CL_NPC_SKILL_CORRUPTION_READY);
    assert_eq!(found.len(), 1, "exactly one READY");
    *found[0].get::<sP_FE2CL_NPC_SKILL_CORRUPTION_READY>().unwrap()
}

fn hit(pkts: &[Packet]) -> (sP_FE2CL_NPC_SKILL_CORRUPTION_HIT, sCAttackResult) {
    let found = only(pkts, P_FE2CL_NPC_SKILL_CORRUPTION_HIT);
    assert_eq!(found.len(), 1, "exactly one HIT");
    let mut reader = PacketReader::new(found[0]);
    let head = *reader.get_struct::<sP_FE2CL_NPC_SKILL_CORRUPTION_HIT>().unwrap();
    let result = *reader.get_struct::<sCAttackResult>().unwrap();
    (head, result)
}

/// Winds up against `windup_nano`, swaps to `hit_nano` during the windup (the
/// player's reaction), then lands the hit.
fn exchange(
    npc_id: i32,
    windup_nano: Option<CombatStyle>,
    hit_nano: Option<CombatStyle>,
    stamina: i16,
) -> (ShardServerState, Vec<Packet>, sP_FE2CL_NPC_SKILL_CORRUPTION_READY) {
    let (mut state, mut rx) = fixture(npc_id, CORRUPT_MOB);
    use_nano(&mut state, windup_nano);
    assert!(begin(&mut state, npc_id));
    let ready = ready(&drain(&mut rx));
    use_nano(&mut state, hit_nano);
    if hit_nano.is_some() {
        set_stamina(&mut state, stamina);
    }
    assert!(finish(npc_id, &mut state).unwrap());
    let pkts = drain(&mut rx);
    (state, pkts, ready)
}

fn pc_hp(state: &ShardServerState) -> i32 {
    state.get_player(PC_ID).unwrap().get_hp()
}

fn pc_stamina(state: &ShardServerState) -> i16 {
    let player = state.get_player(PC_ID).unwrap();
    player.get_active_nano().unwrap().get_stamina()
}

#[test]
fn rps_table_matches_openfusion() {
    // OpenFusion: the player wins when mobStyle - nanoStyle == 1 or
    // nanoStyle - mobStyle == 2, ties on equal styles, loses otherwise.
    let styles = [CombatStyle::Adaptium, CombatStyle::Blastons, CombatStyle::Cosmix];
    for mob in styles {
        assert_eq!(corruption_outcome(mob, None), CorruptionOutcome::NoNano);
        for nano in styles {
            let (m, n) = (mob as i32, nano as i32);
            let expected = if m == n {
                CorruptionOutcome::Tie
            } else if m - n == 1 || n - m == 2 {
                CorruptionOutcome::Win
            } else {
                CorruptionOutcome::Lose
            };
            assert_eq!(corruption_outcome(mob, Some(nano)), expected, "{mob:?} vs {nano:?}");
        }
        // the windup always picks what beats the nano out right now
        // (OpenFusion nanoStyle - 1, wrapping)
        let windup = windup_style(Some(mob)) as i32;
        assert_eq!(windup, (mob as i32 + 2) % 3);
        assert_eq!(
            corruption_outcome(windup_style(Some(mob)), Some(mob)),
            CorruptionOutcome::Lose
        );
    }
}

#[test]
fn table_contract_is_loaded() {
    tdata_init().unwrap();
    let corruption = tdata_get().get_npc_stats(CORRUPT_MOB).unwrap().corruption.unwrap();
    assert_eq!(corruption.skill_id, SKILL);
    assert_eq!(corruption.prob, 100_000);
    assert!(tdata_get().get_npc_stats(PLAIN_MOB).unwrap().corruption.is_none());
    let skill = tdata_get().get_skill(SKILL).unwrap();
    assert_eq!(skill.skill_type, SkillType::CorruptionAttack);
    assert_eq!(corruption_damage(skill, 4), FULL_HIT);
}

#[test]
fn roll_uses_the_table_weight() {
    let npc_id = 2_000_000_201;
    let (mut state, mut rx) = fixture(npc_id, CORRUPT_MOB);
    // weight 100,000 out of 2,000,000: rolls 0..=99,000 hit, 100,000 misses
    assert!(!with_roll(100_000, || try_begin(npc_id, &mut state).unwrap()));
    assert!(state.get_npc(npc_id).unwrap().corruption.is_none());
    assert!(drain(&mut rx).is_empty());
    assert!(with_roll(99_000, || try_begin(npc_id, &mut state).unwrap()));
    assert!(state.get_npc(npc_id).unwrap().corruption.is_some());

    // a mob without the contract never winds up, whatever the roll
    let npc_id = 2_000_000_202;
    let (mut state, mut rx) = fixture(npc_id, PLAIN_MOB);
    assert!(!begin(&mut state, npc_id));
    assert!(drain(&mut rx).is_empty());
}

#[test]
fn windup_broadcasts_ready_and_protects_the_mob() {
    let npc_id = 2_000_000_203;
    let (mut state, mut rx) = fixture(npc_id, CORRUPT_MOB);
    use_nano(&mut state, Some(CombatStyle::Adaptium));
    {
        let npc = state.get_npc_mut(npc_id).unwrap();
        let mut far = npc.get_position();
        far.x += 5000;
        npc.move_towards(far, None);
    }
    assert!(begin(&mut state, npc_id));

    let ready = ready(&drain(&mut rx));
    let pos = state.get_player(PC_ID).unwrap().get_position();
    assert_eq!({ ready.iNPC_ID }, npc_id);
    assert_eq!({ ready.iSkillID }, SKILL);
    // Cosmix beats the Adaptium nano the player has out
    assert_eq!({ ready.iStyle }, CombatStyle::Cosmix as i16);
    assert_eq!(({ ready.iValue1 }, { ready.iValue2 }, { ready.iValue3 }), (pos.x, pos.y, pos.z));

    let npc = state.get_npc_mut(npc_id).unwrap();
    assert!(npc.path.is_none(), "a casting mob stands still");
    let hp = npc.get_hp();
    assert_eq!(npc.take_damage(500, Some(EntityID::Player(PC_ID))), 0);
    assert_eq!(npc.get_hp(), hp, "a casting mob can't be hurt");
    // no second windup on top of the first
    assert!(!begin(&mut state, npc_id));
}

#[test]
fn staying_on_the_countered_nano_loses() {
    let (state, pkts, ready) = exchange(
        2_000_000_204,
        Some(CombatStyle::Adaptium),
        Some(CombatStyle::Adaptium),
        150,
    );
    let (head, r) = hit(&pkts);
    assert_eq!({ head.iStyle }, { ready.iStyle });
    assert_eq!({ head.iSkillID }, SKILL);
    assert_eq!({ head.iTargetCnt }, 1);
    assert_eq!({ r.iHitFlag } as u32, HF_BIT_STYLE_LOSE);
    assert_eq!({ r.iDamage }, FULL_HIT);
    assert_eq!({ r.iHP }, pc_hp(&state));
    assert_eq!(pc_hp(&state), state.get_player(PC_ID).unwrap().get_max_hp() - FULL_HIT);
    assert_eq!({ r.iNanoStamina }, 60);
    assert_eq!(pc_stamina(&state), 60);
    assert_eq!({ r.bNanoDeactive }, 0);
    assert_eq!({ r.iActiveNanoSlotNum }, 0);
    assert_eq!({ r.iNanoID }, 2);
    assert!(only(&pkts, P_FE2CL_NANO_SKILL_USE_SUCC).is_empty());
}

#[test]
fn losing_on_low_stamina_knocks_the_nano_out() {
    let (state, pkts, _) = exchange(
        2_000_000_205,
        Some(CombatStyle::Blastons),
        Some(CombatStyle::Blastons),
        89,
    );
    let (_, r) = hit(&pkts);
    assert_eq!({ r.iHitFlag } as u32, HF_BIT_STYLE_LOSE);
    assert_eq!({ r.iNanoStamina }, 0);
    assert_eq!({ r.bNanoDeactive }, 1);
    assert_eq!(pc_stamina(&state), 0);
}

#[test]
fn swapping_to_the_winning_nano_wins_and_hits_back() {
    let npc_id = 2_000_000_206;
    // windup is Cosmix against Adaptium; Blastons beats Cosmix
    let (state, pkts, ready) = exchange(
        npc_id,
        Some(CombatStyle::Adaptium),
        Some(CombatStyle::Blastons),
        100,
    );
    assert_eq!({ ready.iStyle }, CombatStyle::Cosmix as i16);
    let (_, r) = hit(&pkts);
    assert_eq!({ r.iHitFlag } as u32, HF_BIT_STYLE_WIN);
    assert_eq!({ r.iDamage }, 0);
    assert_eq!(pc_hp(&state), state.get_player(PC_ID).unwrap().get_max_hp());
    assert_eq!({ r.iNanoStamina }, 145);
    assert_eq!(pc_stamina(&state), 145);
    assert_eq!({ r.iActiveNanoSlotNum }, 1);

    // the 200-power counter-hit lands on the mob and is reported as a nano
    // damage skill, before the HIT itself
    let succ = only(&pkts, P_FE2CL_NANO_SKILL_USE_SUCC);
    assert_eq!(succ.len(), 1);
    let mut reader = PacketReader::new(succ[0]);
    let head = *reader.get_struct::<sP_FE2CL_NANO_SKILL_USE_SUCC>().unwrap();
    assert_eq!({ head.eST }, SkillType::Damage as i32);
    assert_eq!({ head.iNanoID }, 1);
    assert!({ head.iSkillID } > 0);
    assert_eq!({ head.iNanoStamina }, 145);
    let dmg = *reader.get_struct::<sSkillResult_Damage>().unwrap();
    let npc = state.get_npc(npc_id).unwrap();
    let pc_max = state.get_player(PC_ID).unwrap().get_max_hp();
    let expected = (200.0 * pc_max.max(npc.get_max_hp()) as f32 / 1000.0) as i32;
    assert_eq!({ dmg.iID }, npc_id);
    assert_eq!({ dmg.iDamage }, expected.min(npc.get_max_hp()));
    assert_eq!(npc.get_hp(), npc.get_max_hp() - { dmg.iDamage });
    let order: Vec<_> = pkts.iter().map(|p| p.id()).collect();
    let succ_at = order.iter().position(|id| *id == P_FE2CL_NANO_SKILL_USE_SUCC);
    let hit_at = order.iter().position(|id| *id == P_FE2CL_NPC_SKILL_CORRUPTION_HIT);
    assert!(succ_at < hit_at);

    // stamina gain caps at 150
    let (state, pkts, _) = exchange(
        2_000_000_207,
        Some(CombatStyle::Adaptium),
        Some(CombatStyle::Blastons),
        140,
    );
    assert_eq!({ hit(&pkts).1.iNanoStamina }, 150);
    assert_eq!(pc_stamina(&state), 150);
}

#[test]
fn matching_the_mob_style_is_a_neutral_tie() {
    // windup Cosmix against Adaptium; swap to Cosmix
    let (state, pkts, _) = exchange(
        2_000_000_208,
        Some(CombatStyle::Adaptium),
        Some(CombatStyle::Cosmix),
        120,
    );
    let (_, r) = hit(&pkts);
    assert_eq!({ r.iHitFlag } as u32, HF_BIT_STYLE_TIE);
    assert_eq!({ r.iDamage }, 0);
    assert_eq!({ r.iNanoStamina }, 120);
    assert_eq!(pc_stamina(&state), 120);
    assert_eq!(pc_hp(&state), state.get_player(PC_ID).unwrap().get_max_hp());
    assert!(only(&pkts, P_FE2CL_NANO_SKILL_USE_SUCC).is_empty());
}

#[test]
fn no_nano_takes_the_full_hit() {
    let (state, pkts, ready) = exchange(2_000_000_209, None, None, 0);
    assert!((0..=2).contains(&{ ready.iStyle }));
    let (_, r) = hit(&pkts);
    assert_eq!({ r.iHitFlag } as u32, HF_BIT_STYLE_TIE);
    assert_eq!({ r.iDamage }, FULL_HIT);
    assert_eq!({ r.iActiveNanoSlotNum }, -1);
    assert_eq!({ r.iNanoID }, 0);
    assert_eq!(pc_hp(&state), state.get_player(PC_ID).unwrap().get_max_hp() - FULL_HIT);
}

/// Starts a windup, applies `interrupt`, and checks the hit is cancelled:
/// CANCEL instead of HIT, the player untouched, the mob hittable again.
fn assert_cancelled(npc_id: i32, interrupt: impl FnOnce(&mut ShardServerState)) {
    let (mut state, mut rx) = fixture(npc_id, CORRUPT_MOB);
    use_nano(&mut state, Some(CombatStyle::Adaptium));
    assert!(begin(&mut state, npc_id));
    drain(&mut rx);
    let hp = pc_hp(&state);

    interrupt(&mut state);
    assert!(is_interrupted(npc_id, &state));
    assert!(!finish(npc_id, &mut state).unwrap());
    let pkts = drain(&mut rx);
    assert!(only(&pkts, P_FE2CL_NPC_SKILL_CORRUPTION_HIT).is_empty());
    assert!(state.get_npc(npc_id).unwrap().corruption.is_none());
    // a player who logged out has no client left to tell
    if let Ok(player) = state.get_player(PC_ID) {
        assert_eq!(only(&pkts, P_FE2CL_NPC_SKILL_CANCEL).len(), 1);
        if !player.is_dead() {
            assert_eq!(pc_hp(&state), hp);
            assert_eq!(pc_stamina(&state), 150);
        }
    }
}

fn short_buff() -> BuffInstance {
    BuffInstance::new(BuffType::Nano, 1, None, None, Some(Duration::from_secs(60)))
}

#[test]
fn death_cancels_the_windup() {
    assert_cancelled(2_000_000_210, |state| {
        let npc = state.get_npc_mut(2_000_000_210).unwrap();
        npc.corruption.take();
        npc.take_damage(1_000_000, Some(EntityID::Player(PC_ID)));
        npc.corruption = Some(PendingCorruption {
            skill_id: SKILL,
            style: CombatStyle::Cosmix,
            target_pc_id: PC_ID,
        });
    });
}

#[test]
fn stun_and_sleep_cancel_the_windup() {
    assert_cancelled(2_000_000_211, |state| {
        state
            .get_npc_mut(2_000_000_211)
            .unwrap()
            .apply_buff(BuffID::Stun, short_buff(), None);
    });
    assert_cancelled(2_000_000_212, |state| {
        state
            .get_npc_mut(2_000_000_212)
            .unwrap()
            .apply_buff(BuffID::Sleep, short_buff(), None);
    });
}

#[test]
fn losing_the_target_cancels_the_windup() {
    // target died
    assert_cancelled(2_000_000_213, |state| {
        state.get_player_mut(PC_ID).unwrap().set_hp(0);
    });
    // target ran out of the mob's combat range
    assert_cancelled(2_000_000_214, |state| {
        let range = tdata_get().get_npc_stats(CORRUPT_MOB).unwrap().combat_range as i32;
        let player = state.get_player_mut(PC_ID).unwrap();
        let mut pos = player.get_position();
        pos.x += range + 100;
        player.set_position(pos);
    });
    // target logged out
    assert_cancelled(2_000_000_215, |state| {
        // logout takes the player off the map, then drops it
        state.entity_map.update(EntityID::Player(PC_ID), None, false);
        state.entity_map.untrack(EntityID::Player(PC_ID));
    });
    // mob dropped its target
    assert_cancelled(2_000_000_216, |state| {
        state.get_npc_mut(2_000_000_216).unwrap().target_id = None;
    });
}

#[test]
fn reset_and_death_drop_the_windup() {
    let npc_id = 2_000_000_217;
    let (mut state, mut rx) = fixture(npc_id, CORRUPT_MOB);
    assert!(begin(&mut state, npc_id));
    state.get_npc_mut(npc_id).unwrap().reset();
    assert!(state.get_npc(npc_id).unwrap().corruption.is_none());

    // reset also dropped the target; pick it back up
    state.get_npc_mut(npc_id).unwrap().target_id = Some(EntityID::Player(PC_ID));
    assert!(begin(&mut state, npc_id));
    drain(&mut rx);
    cancel(npc_id, &mut state).unwrap();
    assert!(state.get_npc(npc_id).unwrap().corruption.is_none());
    assert_eq!(only(&drain(&mut rx), P_FE2CL_NPC_SKILL_CANCEL).len(), 1);
    // nothing pending: no packet
    cancel(npc_id, &mut state).unwrap();
    assert!(drain(&mut rx).is_empty());
}

fn tick(state: &mut ShardServerState, npc_id: i32, times: usize) {
    for _ in 0..times {
        NPC::tick(state, npc_id);
    }
}

#[test]
fn mob_ai_winds_up_then_hits_then_resumes_attacking() {
    let npc_id = 2_000_000_218;
    let (mut state, mut rx) = fixture(npc_id, CORRUPT_MOB);
    use_nano(&mut state, Some(CombatStyle::Adaptium));
    let _ = scripting_init();
    scripting_get().lock().remove_npc(npc_id);
    state.get_npc_mut(npc_id).unwrap().ai = Some("mob".to_string());

    // the roll hits on the first attack: READY, no basic attack
    with_roll(0, || tick(&mut state, npc_id, 1));
    let pkts = drain(&mut rx);
    assert_eq!({ ready(&pkts).iStyle }, CombatStyle::Cosmix as i16);
    assert!(only(&pkts, P_FE2CL_NPC_ATTACK_PCs).is_empty());

    // the player reacts by swapping to Blastons; nothing lands during the
    // 1.8s (15-tick) windup
    use_nano(&mut state, Some(CombatStyle::Blastons));
    with_roll(i32::MAX, || tick(&mut state, npc_id, 14));
    let pkts = drain(&mut rx);
    assert!(only(&pkts, P_FE2CL_NPC_SKILL_CORRUPTION_HIT).is_empty());
    assert!(only(&pkts, P_FE2CL_NPC_ATTACK_PCs).is_empty());
    assert!(state.get_npc(npc_id).unwrap().corruption.is_some());

    with_roll(i32::MAX, || tick(&mut state, npc_id, 1));
    let pkts = drain(&mut rx);
    assert_eq!({ hit(&pkts).1.iHitFlag } as u32, HF_BIT_STYLE_WIN);
    assert!(state.get_npc(npc_id).unwrap().corruption.is_none());

    // 1s (8-tick) cooldown, then ordinary attacks resume
    with_roll(i32::MAX, || tick(&mut state, npc_id, 8));
    assert!(only(&drain(&mut rx), P_FE2CL_NPC_ATTACK_PCs).is_empty());
    with_roll(i32::MAX, || tick(&mut state, npc_id, 2));
    let pkts = drain(&mut rx);
    assert_eq!(only(&pkts, P_FE2CL_NPC_ATTACK_PCs).len(), 1);
    assert!(only(&pkts, P_FE2CL_NPC_SKILL_CORRUPTION_READY).is_empty());

    scripting_get().lock().remove_npc(npc_id);
}

#[test]
fn mob_ai_cancels_when_stunned_mid_windup() {
    let npc_id = 2_000_000_219;
    let (mut state, mut rx) = fixture(npc_id, CORRUPT_MOB);
    use_nano(&mut state, Some(CombatStyle::Adaptium));
    let _ = scripting_init();
    scripting_get().lock().remove_npc(npc_id);
    state.get_npc_mut(npc_id).unwrap().ai = Some("mob".to_string());

    with_roll(0, || tick(&mut state, npc_id, 1));
    ready(&drain(&mut rx));
    with_roll(i32::MAX, || tick(&mut state, npc_id, 3));
    state
        .get_npc_mut(npc_id)
        .unwrap()
        .apply_buff(BuffID::Stun, short_buff(), None);
    with_roll(i32::MAX, || tick(&mut state, npc_id, 2));
    let pkts = drain(&mut rx);
    assert_eq!(only(&pkts, P_FE2CL_NPC_SKILL_CANCEL).len(), 1);
    assert!(only(&pkts, P_FE2CL_NPC_SKILL_CORRUPTION_HIT).is_empty());
    assert!(state.get_npc(npc_id).unwrap().corruption.is_none());
    assert_eq!(pc_hp(&state), state.get_player(PC_ID).unwrap().get_max_hp());

    scripting_get().lock().remove_npc(npc_id);
}
