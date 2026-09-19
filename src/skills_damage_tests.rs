//! Hostile NPC -> player damage against OpenFusion `Combat::npcAttackPc`.
//!
//! Expected numbers are OpenFusion `getDamage` worked by hand with the shipped
//! xdt; its NPC power/protection/level/style/team columns are identical to
//! `../OpenFusion/tdata/xdt.json` for all 3490 NPCs.

use super::*;
use crate::{
    chunk::{ChunkCoords, TickMode},
    entity::{Player, NPC},
    net::{
        packet::{Packet, PacketReader},
        ClientMessage, ClientMetadata, ClientType, FFClient,
    },
    tabledata::tdata_init,
};

// npcAttackPc inputs: table power, no style (no RPS), no boost, difficulty 0.
fn mob_attack(power: i32) -> BasicAttack {
    BasicAttack {
        power,
        crit_chance: Some(0.05),
        attack_style: None,
        charged: false,
    }
}

// OpenFusion Items::setItemStats: 16 + level * 4 + armor.
fn pc_defense(level: i32, armor: i32) -> i32 {
    16 + level * 4 + armor
}

fn hit(power: i32, defense: i32, style: Option<CombatStyle>, variance: i32, crit: bool) -> i32 {
    let rolls = DamageRolls { variance, crit };
    calculate_damage(&mob_attack(power), defense, style, 0, rolls).0
}

#[test]
fn low_level_mob_matches_openfusion() {
    // NPC 59: level 2, m_iPower 198. Level-2 player, no armor: defense 24.
    // 198*198/222 = 176
    let def = pc_defense(2, 0);
    assert_eq!(def, 24);
    assert_eq!(hit(198, def, None, 0, false), 140);
    assert_eq!(hit(198, def, None, 20, false), 176);
    assert_eq!(hit(198, def, None, 39, false), 209);
    assert_eq!(hit(198, def, None, 39, true), 418);

    // The removed path: 450 + 198, old base defense 20, player level as
    // difficulty. 648*648/668 = 628, minus (20 - 108) * 2 / 100 = -1.
    let old = BasicAttack {
        power: 648,
        ..mob_attack(0)
    };
    let rolls = DamageRolls {
        variance: 20,
        crit: false,
    };
    assert_eq!(calculate_damage(&old, 20, None, 2, rolls).0, 629);
}

#[test]
fn mid_level_mob_with_armor_matches_openfusion() {
    // NPC 44: level 18, m_iPower 454. Level-18 player, 100 armor: 188.
    // 454*454/642 = 321
    let def = pc_defense(18, 100);
    assert_eq!(hit(454, def, None, 0, false), 256);
    assert_eq!(hit(454, def, None, 20, false), 321);
    assert_eq!(hit(454, def, None, 20, true), 642);
}

#[test]
fn high_level_mob_with_armor_matches_openfusion() {
    // NPC 1544: level 36, m_iPower 720. Level-36 player, 300 armor: 460.
    // 720*720/1180 = 439
    let def = pc_defense(36, 300);
    assert_eq!(hit(720, def, None, 20, false), 439);
    assert_eq!(hit(720, def, None, 39, false), 522);
    assert_eq!(hit(720, def, None, 39, true), 1044);
}

#[test]
fn mob_hit_ignores_player_nano_style() {
    // No nano and every nano style land the same hit.
    let def = pc_defense(18, 100);
    let base = hit(454, def, None, 20, false);
    for style in [
        CombatStyle::Adaptium,
        CombatStyle::Blastons,
        CombatStyle::Cosmix,
    ] {
        assert_eq!(hit(454, def, Some(style), 20, false), base);
    }
}

#[test]
fn floor_and_zero_guards() {
    // Heavily armored: 198^2/1198 = 32 beats the 10 + 198/10 = 29 floor,
    // and difficulty 0 never subtracts.
    assert_eq!(hit(198, 1000, None, 20, false), 32);
    assert_eq!(hit(0, 0, None, 20, true), 0);
}

#[test]
fn player_attacks_keep_their_contract() {
    // PC -> mob keeps mob level as difficulty, RPS and boost (pcAttackNpcs).
    // 900/183 = 4; max(13, 4 - (153 - 5) * 2 / 100) = 13;
    // Adaptium beats Blastons: 16; boost: 20.
    let pc = BasicAttack {
        power: 30,
        crit_chance: None,
        attack_style: Some(CombatStyle::Adaptium),
        charged: true,
    };
    let rolls = DamageRolls {
        variance: 20,
        crit: false,
    };
    let dmg = calculate_damage(&pc, 153, Some(CombatStyle::Blastons), 2, rolls).0;
    assert_eq!(dmg, 20);
}

type Rx = tokio::sync::mpsc::UnboundedReceiver<ClientMessage>;

const PC_ID: i32 = 1;
// Far above the IDs the default state spawns from tabledata.
const NPC_ID: i32 = 2_000_000_000;

fn fixture(level: i16, mob_type: i32) -> (ShardServerState, Rx) {
    tdata_init().unwrap();
    let mut state = ShardServerState::default();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            "127.0.0.1:39201".parse().unwrap(),
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
    player.set_level(level).unwrap();
    let pos = player.get_position();
    let chunk = ChunkCoords::from_pos_inst(pos, InstanceID::default());
    let id = state.entity_map.track(Box::new(player), TickMode::Always);
    state.entity_map.update(id, Some(chunk), false);

    let mut npc = NPC::new(NPC_ID, mob_type, pos, 0, InstanceID::default()).unwrap();
    npc.target_id = Some(EntityID::Player(PC_ID));
    let id = state.entity_map.track(Box::new(npc), TickMode::Always);
    state.entity_map.update(id, Some(chunk), false);
    (state, rx)
}

fn attack_packets(rx: &mut Rx) -> Vec<Packet> {
    let mut out = Vec::new();
    while let Ok(msg) = rx.try_recv() {
        if let ClientMessage::SendPacket(p) = msg {
            if p.id() == P_FE2CL_NPC_ATTACK_PCs {
                out.push(p);
            }
        }
    }
    out
}

/// Runs the real attack path and checks the NPC_ATTACK_PCs the client gets.
fn check_network_hits(level: i16, mob_type: i32, power: i32, defense: i32, lo: i32, hi: i32) {
    let (mut state, mut rx) = fixture(level, mob_type);
    let npc = state.get_npc(NPC_ID).unwrap();
    assert_eq!(npc.get_table_power(), power);
    // NPC-vs-NPC fights keep their separate bonus.
    assert_eq!(npc.get_single_power(), 450 + power);
    assert_eq!(state.get_player(PC_ID).unwrap().get_defense(), defense);

    let max_hp = state.get_player(PC_ID).unwrap().get_max_hp();
    let (mut normal, mut crits) = (0, 0);
    for _ in 0..500 {
        state.get_player_mut(PC_ID).unwrap().set_hp(max_hp);
        do_basic_attack(
            EntityID::NPC(NPC_ID),
            &[EntityID::Player(PC_ID)],
            false,
            AttackContext::default(),
            &mut state,
        )
        .unwrap();

        let pkts = attack_packets(&mut rx);
        assert_eq!(pkts.len(), 1);
        let mut reader = PacketReader::new(&pkts[0]);
        let head = reader.get_struct::<sP_FE2CL_NPC_ATTACK_PCs>().unwrap();
        assert_eq!({ head.iNPC_ID }, NPC_ID);
        assert_eq!({ head.iPCCnt }, 1);
        let res = reader.get_struct::<sAttackResult>().unwrap();

        let server_hp = state.get_player(PC_ID).unwrap().get_hp();
        assert_eq!({ res.iID }, PC_ID);
        assert_eq!({ res.iHP }, server_hp, "client HP must equal server HP");
        let dealt = { res.iDamage };
        assert_eq!(dealt, max_hp - server_hp);

        let crit = res.iHitFlag as u32 & HF_BIT_CRITICAL != 0;
        let (lo, hi) = if crit { (lo * 2, hi * 2) } else { (lo, hi) };
        assert!(
            dealt == max_hp || (lo..=hi).contains(&dealt),
            "damage {dealt} (crit {crit}) outside OpenFusion {lo}..={hi}, max HP {max_hp}"
        );
        if crit {
            crits += 1;
        } else {
            normal += 1;
        }
    }
    assert!(normal > 0 && crits > 0, "normal {normal}, crits {crits}");
}

#[test]
fn network_low_level_hp_matches_packet() {
    // Level 2 vs NPC 59: 140..=209 (the old bonus bottomed out at 503).
    check_network_hits(2, 59, 198, 24, 140, 209);
}

#[test]
fn network_mid_level_hp_matches_packet() {
    // Level 18, no armor (88) vs NPC 44: 454^2/542 = 380 -> 304..=452.
    check_network_hits(18, 44, 454, 88, 304, 452);
}

#[test]
fn network_high_level_hp_matches_packet() {
    // Level 36, no armor (160) vs NPC 1544: 720^2/880 = 589 -> 471..=700.
    check_network_hits(36, 1544, 720, 160, 471, 700);
}
