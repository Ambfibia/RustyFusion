//! /level and /whois driven through the FreeChat packet that FFOneClient
//! sends for server slash commands.

use std::collections::{HashMap, HashSet};

use crate::{
    chunk::{ChunkCoords, EntityMap, InstanceID, TickMode},
    database::{open_test_sqlite, test_suite, DbImpl},
    entity::{Combatant, Entity, EntityID, Player, NPC},
    net::{
        packet::{Packet, PacketID::*, *},
        ClientMap, ClientMessage, ClientMetadata, ClientType, FFClient,
    },
    server::shard::chat::send_freechat_message,
    state::ShardServerState,
    tabledata::tdata_init,
    util, Position,
};

const PLAYER_POS: Position = Position {
    x: 200_000,
    y: 200_000,
    z: 0,
};

#[test]
fn quest_commands_start_a_mission_and_clear_only_its_completed_flag() {
    let mut f=fixture(50,1);
    let id=1;
    f.state.get_player_mut(1).unwrap().mission_journal.set_mission_completed(id).unwrap();
    f.state.get_player_mut(1).unwrap().mission_journal.set_mission_completed(2).unwrap();
    f.chat("/deletequest 1");
    let journal=&f.state.get_player(1).unwrap().mission_journal;
    assert!(!journal.is_mission_completed(1).unwrap());assert!(journal.is_mission_completed(2).unwrap());
    assert_eq!(system_messages(&drain(&mut f.rx)),["Quest 1 removed from completed missions."]);
    f.chat("/startquest 1");
    let out=drain(&mut f.rx);
    assert!(out.iter().any(|p|p.id()==P_FE2CL_REP_PC_TASK_START_SUCC),"{:?}",system_messages(&out));
    assert!(f.state.get_player(1).unwrap().mission_journal.get_current_tasks().iter().any(|t|t.get_mission_def().mission_id==1));
    f.chat("/startquest 1");assert_eq!(system_messages(&drain(&mut f.rx)),["Quest 1 is already active."]);
}

#[test]
fn quest_commands_reject_access_invalid_ids_and_extra_arguments() {
    let mut f=fixture(51,1);f.chat("/startquest 1");assert_eq!(system_messages(&drain(&mut f.rx)),["You don't have access to that command!"]);
    assert!(f.state.get_player(1).unwrap().mission_journal.get_current_tasks().is_empty());
    let mut f=fixture(50,1);
    for command in ["/startquest 0","/startquest 1 extra","/deletequest -1"] {f.chat(command);assert!(system_messages(&drain(&mut f.rx))[0].starts_with("Usage:"));}
    f.chat("/startquest 999999");assert_eq!(system_messages(&drain(&mut f.rx)),["Unknown mission ID: 999999"]);
}

fn place(state: &mut ShardServerState, entity: Box<dyn Entity>) {
    let id = entity.get_id();
    let coords = ChunkCoords::from_pos_inst(entity.get_position(), InstanceID::default());
    state.entity_map.track(entity, TickMode::Never);
    state.entity_map.update(id, Some(coords), false);
}

fn player(
    pc_id: i32,
    perms: i16,
    level: i16,
) -> (
    Player,
    FFClient,
    tokio::sync::mpsc::UnboundedReceiver<ClientMessage>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let client = FFClient::new(
        tx,
        ClientMetadata::new(
            "127.0.0.1:39005".parse().unwrap(),
            Some(ClientType::GameClient {
                account_id: pc_id as i64,
                serial_key: 1,
                pc_id: Some(pc_id),
            }),
        ),
    );
    let mut player = Player::new(pc_id as i64, 0);
    player.set_player_id(pc_id);
    player.perms = perms;
    player.set_level(level).unwrap();
    player.set_position(PLAYER_POS);
    player.set_client(client.clone());
    (player, client, rx)
}

struct Fixture {
    state: ShardServerState,
    clients: HashMap<usize, FFClient>,
    rx: tokio::sync::mpsc::UnboundedReceiver<ClientMessage>,
    observer_rx: tokio::sync::mpsc::UnboundedReceiver<ClientMessage>,
}

fn fixture(perms: i16, level: i16) -> Fixture {
    tdata_init().unwrap();
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
    let (sender, client, rx) = player(1, perms, level);
    let (mut observer, observer_client, observer_rx) = player(2, 99, 1);
    // A player standing closer than any NPC must not be picked by /whois.
    observer.set_position(Position {
        x: PLAYER_POS.x + 5,
        ..PLAYER_POS
    });
    place(&mut state, Box::new(sender));
    place(&mut state, Box::new(observer));
    Fixture {
        state,
        clients: HashMap::from([(1, client), (2, observer_client)]),
        rx,
        observer_rx,
    }
}

impl Fixture {
    fn chat(&mut self, text: &str) {
        let pkt = Packet::new(
            P_CL2FE_REQ_SEND_FREECHAT_MESSAGE,
            &sP_CL2FE_REQ_SEND_FREECHAT_MESSAGE {
                szFreeChat: util::encode_utf16(text).unwrap(),
                iEmoteCode: 0,
            },
        )
        .unwrap();
        let clients = ClientMap::new(1, &self.clients);
        futures::executor::block_on(send_freechat_message(pkt, &clients, &mut self.state)).unwrap();
    }

    fn level(&self) -> i16 {
        self.state.get_player(1).unwrap().get_level()
    }
}

fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ClientMessage>) -> Vec<Packet> {
    std::iter::from_fn(|| match rx.try_recv().ok()? {
        ClientMessage::SendPacket(pkt) => Some(pkt),
        _ => None,
    })
    .collect()
}

fn system_messages(packets: &[Packet]) -> Vec<String> {
    packets
        .iter()
        .filter(|p| p.id() == P_FE2CL_PC_MOTD_LOGIN)
        .map(|p| {
            util::parse_utf16(&{ p.get::<sP_FE2CL_PC_MOTD_LOGIN>().unwrap().szSystemMsg }).unwrap()
        })
        .collect()
}

fn level_broadcasts(packets: &[Packet]) -> Vec<(i32, i16)> {
    packets
        .iter()
        .filter(|p| p.id() == P_FE2CL_REP_PC_CHANGE_LEVEL)
        .map(|p| {
            let r = p.get::<sP_FE2CL_REP_PC_CHANGE_LEVEL>().unwrap();
            (r.iPC_ID, r.iPC_Level)
        })
        .collect()
}

#[test]
fn slash_level_changes_level_and_notifies_self_and_viewers() {
    let mut f = fixture(50, 5);
    f.chat("/level 12");
    assert_eq!(f.level(), 12);
    let own = drain(&mut f.rx);
    assert_eq!(level_broadcasts(&own), [(1, 12)]);
    assert_eq!(system_messages(&own), ["Level changed from 5 to 12"]);
    assert!(own
        .iter()
        .all(|p| p.id() != P_FE2CL_REP_SEND_FREECHAT_MESSAGE_SUCC));
    let seen = drain(&mut f.observer_rx);
    assert_eq!(level_broadcasts(&seen), [(1, 12)]);
    assert!(seen
        .iter()
        .all(|p| p.id() != P_FE2CL_REP_SEND_FREECHAT_MESSAGE_SUCC));

    // Academy alias and the legacy `!` prefix share the handler.
    f.chat("/levelx 36");
    assert_eq!(f.level(), 36);
    f.chat("!level 1");
    assert_eq!(f.level(), 1);
}

#[test]
fn level_rejects_access_arguments_and_bounds_without_changes() {
    let mut f = fixture(51, 5);
    f.chat("/level 10");
    assert_eq!(f.level(), 5);
    let out = drain(&mut f.rx);
    assert!(level_broadcasts(&out).is_empty());
    assert_eq!(
        system_messages(&out),
        ["You don't have access to that command!"]
    );

    let mut f = fixture(30, 5);
    for (cmd, expected) in [
        (
            "/level",
            "/level: no level specified\nUsage: /level <level 1-36>",
        ),
        (
            "/level abc",
            "Invalid level: abc\nUsage: /level <level 1-36>",
        ),
        ("/level 10 20", "Usage: /level <level 1-36>"),
        ("/level 0", "Level out of range [1, 36]"),
        ("/level 37", "Level out of range [1, 36]"),
        ("/level -3", "Level out of range [1, 36]"),
        (
            "/level 99999",
            "Invalid level: 99999\nUsage: /level <level 1-36>",
        ),
    ] {
        f.chat(cmd);
        assert_eq!(f.level(), 5, "{cmd}");
        let out = drain(&mut f.rx);
        assert!(level_broadcasts(&out).is_empty(), "{cmd}");
        assert_eq!(system_messages(&out), [expected], "{cmd}");
        assert!(drain(&mut f.observer_rx).is_empty(), "{cmd}");
    }
    for bound in [1, 36] {
        f.chat(&format!("/level {bound}"));
        assert_eq!(f.level(), bound);
        assert_eq!(level_broadcasts(&drain(&mut f.rx)), [(1, bound)]);
    }
}

#[tokio::test]
async fn level_set_by_command_survives_save_and_relogin() {
    test_suite::ensure_init();
    std::fs::create_dir_all("target/t05").unwrap();
    let path = format!("target/t05/level-{}.db", uuid::Uuid::new_v4());
    let db = open_test_sqlite(&path).await;
    let account = db
        .create_account("t05_level", "unused-test-hash")
        .await
        .unwrap();

    let mut f = fixture(50, 5);
    let uid = f.state.get_player(1).unwrap().get_uid();
    db.init_player(account.id, f.state.get_player(1).unwrap())
        .await
        .unwrap();
    f.chat("/level 17");
    // The shard saves the live player on logout; reopen the database for the
    // next login instead of reading from the in-memory state.
    db.save_player(f.state.get_player(1).unwrap())
        .await
        .unwrap();
    drop(db);
    let db = open_test_sqlite(&path).await;
    let loaded = db.load_player(account.id, uid).await.unwrap().unwrap();
    assert_eq!(loaded.get_level(), 17);
}

#[test]
fn whois_describes_nearest_npc_not_a_player() {
    let mut f = fixture(50, 5);
    f.chat("/whois");
    assert_eq!(
        system_messages(&drain(&mut f.rx)),
        ["[WHOIS] No NPCs found nearby"]
    );

    let far = NPC::new(
        900,
        3470,
        Position {
            x: PLAYER_POS.x + 3000,
            ..PLAYER_POS
        },
        0,
        InstanceID::default(),
    )
    .unwrap();
    let near = NPC::new(
        901,
        3471,
        Position {
            x: PLAYER_POS.x + 100,
            y: PLAYER_POS.y - 200,
            z: 50,
        },
        90,
        InstanceID::default(),
    )
    .unwrap();
    place(&mut f.state, Box::new(far));
    place(&mut f.state, Box::new(near));

    // Arguments are ignored: this is NPC diagnostics, not a user lookup.
    for cmd in ["/whois", "/whois SomePlayer"] {
        f.chat(cmd);
        let lines = system_messages(&drain(&mut f.rx));
        let npc = f.state.get_npc(901).unwrap();
        let chunk = npc.get_chunk_coords();
        assert_eq!(
            lines,
            [
                "[WHOIS] ID: 901".to_string(),
                "[WHOIS] Type: 3471".to_string(),
                format!("[WHOIS] Name: {}", npc.get_name()),
                format!("[WHOIS] HP: {}", npc.get_hp()),
                "[WHOIS] EntityType: NPC".to_string(),
                format!("[WHOIS] X: {}", PLAYER_POS.x + 100),
                format!("[WHOIS] Y: {}", PLAYER_POS.y - 200),
                "[WHOIS] Z: 50".to_string(),
                "[WHOIS] Angle: 90".to_string(),
                format!("[WHOIS] Chunk: {{{}, {}}}", chunk.x, chunk.y),
                format!("[WHOIS] MapNum: {}", InstanceID::default().map_num),
                "[WHOIS] Instance: None".to_string(),
                "[WHOIS] Channel: 1".to_string(),
                format!(
                    "[WHOIS] Distance: {}",
                    PLAYER_POS.distance_to(&npc.get_position())
                ),
            ],
            "{cmd}"
        );
        assert!(drain(&mut f.observer_rx).is_empty(), "{cmd}");
    }
    assert!(f
        .state
        .entity_map
        .get_around_entity(EntityID::Player(1))
        .contains(&EntityID::Player(2)));
}

#[test]
fn whois_requires_access_and_unknown_slash_commands_stay_out_of_chat() {
    let mut f = fixture(99, 5);
    f.chat("/whois");
    assert_eq!(
        system_messages(&drain(&mut f.rx)),
        ["You don't have access to that command!"]
    );

    f.chat("/nosuchcommand 1");
    let out = drain(&mut f.rx);
    assert_eq!(
        system_messages(&out),
        ["Unknown command /nosuchcommand\nUse /help for a list of available commands"]
    );
    assert!(drain(&mut f.observer_rx).is_empty());

    // Ordinary chat is still broadcast.
    f.chat("hello");
    assert!(drain(&mut f.observer_rx)
        .iter()
        .any(|p| p.id() == P_FE2CL_REP_SEND_FREECHAT_MESSAGE_SUCC));
}

#[test]
fn help_and_redeem_route_for_normal_muted_players_without_broadcast() {
    let mut f = fixture(99, 5);
    f.state.get_player_mut(1).unwrap().freechat_muted = true;
    for command in ["/help", "!help"] {
        f.chat(command);
        let out = drain(&mut f.rx);
        let lines = system_messages(&out);
        assert_eq!(lines.len(), 20);
        assert_eq!(lines[0], "Available commands");
        assert!(lines.iter().any(|s| s == "/redeem: Redeem a code item"));
        assert!(lines[1..].windows(2).all(|pair| pair[0] < pair[1]));
        // This was the original /help failure: one oversized UTF-16 packet.
        assert!(util::encode_utf16::<512>(&lines.join("\n")).is_err());
        assert_eq!(out.len(), 20);
    }
    for (command, expected) in [("/redeem", "/redeem: No code specified"),
        ("/redeem unknown", "/redeem: Unknown code"),
        ("/redeem a b", "Usage: /redeem <code>")] {
        f.chat(command);
        assert_eq!(system_messages(&drain(&mut f.rx)), [expected]);
    }
    assert!(drain(&mut f.observer_rx).is_empty());
}
