use crate::{
    defines::*,
    entity::{Entity, EntityID},
    error::*,
    helpers::send_system_message,
    monitor::monitor_event_to_packet,
    net::{
        packet::{PacketID::*, *},
        ClientMap,
    },
    state::ShardServerState,
    util,
};

/// FFOneClient forwards unrecognized `/` commands as FreeChat, matching the
/// local OpenFusion `CMD_PREFIX`. `!` remains accepted for existing users.
const CUSTOM_COMMAND_PREFIX: char = '/';
const CUSTOM_COMMAND_PREFIXES: [char; 2] = [CUSTOM_COMMAND_PREFIX, '!'];

#[path = "chat_redeem.rs"]
mod redeem;
#[path="chat_quest_commands.rs"]
mod quests;

pub async fn send_freechat_message(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_SEND_FREECHAT_MESSAGE = pkt.get()?;

    (async {
        let msg = util::parse_utf16(&pkt.szFreeChat)?;
        if let Some(cmdstr) = msg.strip_prefix(CUSTOM_COMMAND_PREFIXES) {
            let tokens = cmdstr.split_whitespace().collect::<Vec<_>>();
            if !tokens.is_empty() {
                return commands::handle_custom_command(tokens, clients, state).await;
            }
        }

        let client = clients.get_sender();
        let pc_id = client.get_player_id()?;
        let player = state.get_player(pc_id)?;
        if player.freechat_muted {
            return Err(FFError::build_dc(
                Severity::Warning,
                "Muted player sent freechat packet".to_string(),
            ));
        }

        let msg = helpers::process_freechat_message(msg);
        if msg.trim().is_empty() {
            return Ok(());
        }

        let resp = sP_FE2CL_REP_SEND_FREECHAT_MESSAGE_SUCC {
            iPC_ID: pc_id,
            szFreeChat: util::encode_utf16(&msg).unwrap(),
            iEmoteCode: pkt.iEmoteCode,
        };

        log(Severity::Info, &format!("{}: \"{}\"", player, msg));

        if let Some(login_server) = clients.get_login_server() {
            let chat_event = ffmonitor::ChatEvent {
                kind: ffmonitor::ChatKind::FreeChat,
                from: player.to_string(),
                to: None,
                message: msg,
            };

            let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();

            login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
        } else {
            log(
                Severity::Warning,
                "No login server to send monitor chat event",
            );
        }

        state
            .entity_map
            .for_each_around(EntityID::Player(pc_id), |client| {
                client.send_packet(P_FE2CL_REP_SEND_FREECHAT_MESSAGE_SUCC, &resp);
            });
        Ok(())
    })
    .await
    .catch_fail(|| {
        let client = clients.get_sender();
        let resp = sP_FE2CL_REP_SEND_FREECHAT_MESSAGE_FAIL {
            iErrorCode: unused!(),
            szFreeChat: pkt.szFreeChat,
            iEmoteCode: pkt.iEmoteCode,
        };
        client.send_packet(P_FE2CL_REP_SEND_FREECHAT_MESSAGE_FAIL, &resp);
    })
}

pub fn send_menuchat_message(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_SEND_MENUCHAT_MESSAGE = pkt.get()?;

    (|| {
        let client = clients.get_sender();
        let pc_id = client.get_player_id()?;
        let player = state.get_player(pc_id)?;

        let msg = util::parse_utf16(&pkt.szFreeChat)?;
        if !helpers::validate_menuchat_message(&msg) {
            return Err(FFError::build(
                Severity::Warning,
                format!("Invalid menuchat message\n\t{}: '{}'", player, msg),
            ));
        }

        log(Severity::Info, &format!("{}: '{}'", player, msg));

        if let Some(login_server) = clients.get_login_server() {
            let chat_event = ffmonitor::ChatEvent {
                kind: ffmonitor::ChatKind::MenuChat,
                from: player.to_string(),
                to: None,
                message: msg,
            };
            let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();
            login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
        } else {
            log(
                Severity::Warning,
                "No login server to send monitor chat event",
            );
        }

        let resp = sP_FE2CL_REP_SEND_MENUCHAT_MESSAGE_SUCC {
            iPC_ID: pc_id,
            szFreeChat: pkt.szFreeChat,
            iEmoteCode: pkt.iEmoteCode,
        };
        state
            .entity_map
            .for_each_around(EntityID::Player(pc_id), |client| {
                client.send_packet(P_FE2CL_REP_SEND_MENUCHAT_MESSAGE_SUCC, &resp);
            });
        Ok(())
    })()
    .catch_fail(|| {
        let client = clients.get_sender();
        let resp = sP_FE2CL_REP_SEND_MENUCHAT_MESSAGE_FAIL {
            iErrorCode: unused!(),
            szFreeChat: pkt.szFreeChat,
            iEmoteCode: pkt.iEmoteCode,
        };
        client.send_packet(P_FE2CL_REP_SEND_MENUCHAT_MESSAGE_FAIL, &resp);
    })
}

pub fn send_group_freechat_message(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_SEND_ALL_GROUP_FREECHAT_MESSAGE = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;

    let msg = util::parse_utf16(&pkt.szFreeChat)?;
    if player.freechat_muted {
        return Err(FFError::build_dc(
            Severity::Warning,
            "Muted player sent freechat packet".to_string(),
        ));
    }

    let msg = helpers::process_freechat_message(msg);
    if msg.trim().is_empty() {
        return Ok(());
    }

    let pkt = sP_FE2CL_REP_SEND_ALL_GROUP_FREECHAT_MESSAGE_SUCC {
        iSendPCID: pc_id,
        szFreeChat: util::encode_utf16(&msg).unwrap(),
        iEmoteCode: pkt.iEmoteCode,
    };

    log(
        Severity::Info,
        &format!("{} (to group): \"{}\"", player, msg),
    );

    if let Some(login_server) = clients.get_login_server() {
        let chat_event = ffmonitor::ChatEvent {
            kind: ffmonitor::ChatKind::GroupChat,
            from: player.to_string(),
            to: None,
            message: msg,
        };
        let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();
        login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
    } else {
        log(
            Severity::Warning,
            "No login server to send monitor chat event",
        );
    }

    if let Some(group_id) = player.group_id {
        let group = state.groups.get(&group_id).unwrap();
        for eid in group.get_member_ids() {
            let entity = state.entity_map.get_entity_raw(*eid).unwrap();
            if let Some(client) = entity.get_client() {
                client.send_packet(P_FE2CL_REP_SEND_ALL_GROUP_FREECHAT_MESSAGE_SUCC, &pkt);
            }
        }
    }

    Ok(())
}

pub fn send_group_menuchat_message(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_SEND_ALL_GROUP_MENUCHAT_MESSAGE = pkt.get()?;
    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;

    let msg = util::parse_utf16(&pkt.szFreeChat)?;
    if !helpers::validate_menuchat_message(&msg) {
        return Err(FFError::build(
            Severity::Warning,
            format!("Invalid menuchat message\n\t{}: '{}'", player, msg),
        ));
    }

    log(Severity::Info, &format!("{} (to group): '{}'", player, msg));

    if let Some(login_server) = clients.get_login_server() {
        let chat_event = ffmonitor::ChatEvent {
            kind: ffmonitor::ChatKind::GroupMenuChat,
            from: player.to_string(),
            to: None,
            message: msg,
        };
        let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();
        login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
    } else {
        log(
            Severity::Warning,
            "No login server to send monitor chat event",
        );
    }

    let pkt = sP_FE2CL_REP_SEND_ALL_GROUP_MENUCHAT_MESSAGE_SUCC {
        iSendPCID: pc_id,
        szFreeChat: pkt.szFreeChat,
        iEmoteCode: pkt.iEmoteCode,
    };
    if let Some(group_id) = player.group_id {
        let group = state.groups.get(&group_id).unwrap();
        for eid in group.get_member_ids() {
            let entity = state.entity_map.get_entity_raw(*eid).unwrap();
            if let Some(client) = entity.get_client() {
                client.send_packet(P_FE2CL_REP_SEND_ALL_GROUP_MENUCHAT_MESSAGE_SUCC, &pkt);
            }
        }
    }

    Ok(())
}

pub fn send_buddy_freechat_message(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_SEND_BUDDY_FREECHAT_MESSAGE = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;

    let buddy_uid = pkt.iBuddyPCUID;

    if !player.is_buddies_with(buddy_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "{} tried to send freechat to non-buddy UID {}",
                player, buddy_uid
            ),
        ));
    }

    if is_blocked_either_way(pc_id, buddy_uid, state) {
        return Err(FFError::build(
            Severity::Info,
            format!(
                "{} tried to send freechat to blocked UID {}",
                pc_id, buddy_uid
            ),
        ));
    }

    let msg = helpers::process_freechat_message(util::parse_utf16(&pkt.szFreeChat)?);
    let reencoded_msg = util::encode_utf16(&msg).unwrap();

    let response_pkt = sP_FE2CL_REP_SEND_BUDDY_FREECHAT_MESSAGE_SUCC {
        iFromPCUID: player.get_uid(),
        iToPCUID: pkt.iBuddyPCUID,
        szFreeChat: reencoded_msg,
        iEmoteCode: pkt.iEmoteCode,
    };

    if let Some(buddy) = state.get_player_by_uid(buddy_uid) {
        log(
            Severity::Info,
            &format!("{} (to {}): \"{}\"", player, buddy, msg),
        );

        if let Some(login_server) = clients.get_login_server() {
            let chat_event = ffmonitor::ChatEvent {
                kind: ffmonitor::ChatKind::BuddyChat,
                from: player.to_string(),
                to: Some(buddy.to_string()),
                message: msg,
            };
            let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();
            login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
        } else {
            log(
                Severity::Warning,
                "No login server to send monitor chat event",
            );
        }

        if let Some(buddy_client) = buddy.get_client() {
            buddy_client.send_packet(P_FE2CL_REP_SEND_BUDDY_FREECHAT_MESSAGE_SUCC, &response_pkt);
        }

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_SEND_BUDDY_FREECHAT_MESSAGE_SUCC, &response_pkt);

        return Ok(());
    }

    if let Some(login_server) = clients.get_login_server() {
        let cross_shard_pkt = sP_FE2LS_REQ_SEND_BUDDY_FREECHAT {
            iFromPCUID: player.get_uid(),
            iToPCUID: pkt.iBuddyPCUID,
            szFreeChat: reencoded_msg,
            iEmoteCode: pkt.iEmoteCode,
        };

        login_server.send_packet(P_FE2LS_REQ_SEND_BUDDY_FREECHAT, &cross_shard_pkt);
    } else {
        return Err(FFError::build(
            Severity::Warning,
            "No login server found to forward buddy freechat message".to_string(),
        ));
    }

    Ok(())
}

pub fn send_buddy_menuchat_message(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pkt: &sP_CL2FE_REQ_SEND_BUDDY_MENUCHAT_MESSAGE = pkt.get()?;

    let pc_id = client.get_player_id()?;
    let player = state.get_player(pc_id)?;

    let buddy_uid = pkt.iBuddyPCUID;

    if !player.is_buddies_with(buddy_uid) {
        return Err(FFError::build(
            Severity::Warning,
            format!(
                "{} tried to send menuchat to non-buddy UID {}",
                player, buddy_uid
            ),
        ));
    }

    if is_blocked_either_way(pc_id, buddy_uid, state) {
        return Err(FFError::build(
            Severity::Info,
            format!(
                "{} tried to send menuchat to blocked UID {}",
                pc_id, buddy_uid
            ),
        ));
    }

    let msg = util::parse_utf16(&pkt.szFreeChat)?;
    if !helpers::validate_menuchat_message(&msg) {
        return Err(FFError::build(
            Severity::Warning,
            format!("Invalid menuchat message\n\t{}: '{}'", player, msg),
        ));
    }

    let response_pkt = sP_FE2CL_REP_SEND_BUDDY_MENUCHAT_MESSAGE_SUCC {
        iFromPCUID: player.get_uid(),
        iToPCUID: pkt.iBuddyPCUID,
        szFreeChat: pkt.szFreeChat,
        iEmoteCode: pkt.iEmoteCode,
    };

    if let Some(buddy) = state.get_player_by_uid(buddy_uid) {
        log(
            Severity::Info,
            &format!("{} (to {}): '{}'", player, buddy, msg),
        );

        if let Some(login_server) = clients.get_login_server() {
            let chat_event = ffmonitor::ChatEvent {
                kind: ffmonitor::ChatKind::BuddyMenuChat,
                from: player.to_string(),
                to: Some(buddy.to_string()),
                message: msg,
            };
            let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Chat(chat_event)).unwrap();
            login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
        } else {
            log(
                Severity::Warning,
                "No login server to send monitor chat event",
            );
        }

        if let Some(buddy_client) = buddy.get_client() {
            buddy_client.send_packet(P_FE2CL_REP_SEND_BUDDY_MENUCHAT_MESSAGE_SUCC, &response_pkt);
        }

        clients
            .get_sender()
            .send_packet(P_FE2CL_REP_SEND_BUDDY_MENUCHAT_MESSAGE_SUCC, &response_pkt);

        return Ok(());
    }

    if let Some(login_server) = clients.get_login_server() {
        let cross_shard_pkt = sP_FE2LS_REQ_SEND_BUDDY_MENUCHAT {
            iFromPCUID: player.get_uid(),
            iToPCUID: pkt.iBuddyPCUID,
            szFreeChat: pkt.szFreeChat,
            iEmoteCode: pkt.iEmoteCode,
        };

        login_server.send_packet(P_FE2LS_REQ_SEND_BUDDY_MENUCHAT, &cross_shard_pkt);
    } else {
        return Err(FFError::build(
            Severity::Warning,
            "No login server found to forward buddy menuchat message".to_string(),
        ));
    }

    Ok(())
}

pub fn pc_avatar_emotes_chat(
    pkt: Packet,
    clients: &ClientMap,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pkt: &sP_CL2FE_REQ_PC_AVATAR_EMOTES_CHAT = pkt.get()?;

    let resp = sP_FE2CL_REP_PC_AVATAR_EMOTES_CHAT {
        iID_From: pkt.iID_From,
        iEmoteCode: pkt.iEmoteCode,
    };

    state
        .entity_map
        .for_each_around(EntityID::Player(pc_id), |client| {
            client.send_packet(P_FE2CL_REP_PC_AVATAR_EMOTES_CHAT, &resp);
        });

    Ok(())
}

/// True if either party has blocked the other. Blocks are one-sided on the
/// client, but a conversation needs both ends, so we check both.
fn is_blocked_either_way(pc_id: i32, other_uid: i64, state: &ShardServerState) -> bool {
    let Ok(player) = state.get_player(pc_id) else {
        return false;
    };
    if player.has_blocked(other_uid) {
        return true;
    }
    state
        .get_player_by_uid(other_uid)
        .is_some_and(|other| other.has_blocked(player.get_uid()))
}

mod helpers {
    pub fn validate_menuchat_message(_msg: &str) -> bool {
        // TODO validate
        true
    }

    pub fn process_freechat_message(msg: String) -> String {
        crate::helpers::sanitize_text(&msg, false)
    }
}

mod commands {
    use std::{collections::HashMap, future::Future, pin::Pin, sync::OnceLock, time::SystemTime};

    use crate::{
        chunk::TickMode,
        database::{db_get, DbImpl as _},
        entity::Combatant,
        enums::CombatantTeam,
        scripting::scripting_get,
    };

    use super::*;

    struct Command {
        description: &'static str,
        handler: CommandHandler,
    }

    static AVAILABLE_COMMANDS: OnceLock<HashMap<&'static str, Command>> = OnceLock::new();
    type CommandHandler = for<'a> fn(
        Vec<&'a str>,
        &'a ClientMap<'a>,
        &'a mut ShardServerState,
    )
        -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>>;

    fn init_commands() -> HashMap<&'static str, Command> {
        #[rustfmt::skip]
        let commands: [(&'static str, &'static str, CommandHandler); 21] = [
            ("about", "Show information about the server", cmd_about),
            ("level", "Change your character's level", cmd_level),
            ("levelx", "Change your character's level", cmd_level), // for Academy
            ("whois", "Describe the nearest NPC", cmd_whois),
            ("ban_a", "Ban an account", cmd_ban),
            ("ban_i", "Ban a player and their account", cmd_ban),
            ("unban", "Unban an account", cmd_unban),
            ("followme", "Make the nearest NPC start following you", cmd_followme),
            ("unfollowme", "Stop the nearest NPC from following you", cmd_unfollowme),
            ("changeai", "Change the AI script of an NPC", cmd_changeai),
            ("queryai", "Query the AI script of an NPC", cmd_queryai),
            ("reload", "Reload all Lua scripts", cmd_reload),
            ("perms", "View or change a player's permissions level", cmd_perms),
            ("ping", "View your current ping to the server", cmd_ping),
            ("refresh", "Reinsert the player into the current chunk", cmd_refresh),
            ("registerall", "Register all transportation locations", cmd_registerall),
            ("unregisterall", "Unregister all transportation locations", cmd_unregisterall),
            ("help", "Show this help message", cmd_help),
            ("startquest", "Start a mission by its mission ID", quests::start),
            ("deletequest", "Remove a mission from completed missions", quests::delete),
            ("redeem", "Redeem a code item", redeem::cmd_redeem),
        ];

        commands
            .into_iter()
            .map(|(name, description, handler)| {
                (
                    name,
                    Command {
                        description,
                        handler,
                    },
                )
            })
            .collect()
    }

    fn parse_pc_id(token: &str) -> Result<Option<i32>, ()> {
        if token == "." {
            return Ok(None);
        }
        token.parse::<i32>().map_err(|_| ()).map(Some)
    }

    pub async fn handle_custom_command<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> FFResult<()> {
        let cmds = AVAILABLE_COMMANDS.get_or_init(init_commands);

        let cmd_name = tokens[0];
        if let Some(cmd) = cmds.get(cmd_name) {
            (cmd.handler)(tokens, clients, state).await
        } else {
            send_system_message(
                clients.get_sender(),
                &format!(
                    "Unknown command {}{}\nUse {}help for a list of available commands",
                    CUSTOM_COMMAND_PREFIX, cmd_name, CUSTOM_COMMAND_PREFIX
                ),
            )
        }
    }

    fn cmd_about<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        _state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            send_system_message(
                clients.get_sender(),
                &format!(
                    "RustyFusion by ycc\n\
                Library version: {}\n\
                Protocol version: {}\n\
                Database version: {}",
                    LIB_VERSION, PROTOCOL_VERSION, DB_VERSION,
                ),
            )
        })
    }

    // OpenFusion registers /level, /levelx and /whois for accountLevel <= 50.
    const NO_ACCESS: &str = "You don't have access to that command!";

    fn cmd_level<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player_mut(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__DEVELOPER as i16 {
                return send_system_message(client, NO_ACCESS);
            }

            let usage = format!(
                "Usage: {}{} <level 1-{}>",
                CUSTOM_COMMAND_PREFIX, tokens[0], PC_LEVEL_MAX
            );
            let Some(arg) = tokens.get(1) else {
                return send_system_message(
                    client,
                    &format!(
                        "{}{}: no level specified\n{}",
                        CUSTOM_COMMAND_PREFIX, tokens[0], usage
                    ),
                );
            };
            if tokens.len() > 2 {
                return send_system_message(client, &usage);
            }
            // OpenFusion echoes out-of-range levels to accountLevel <= 30
            // without storing them; reject instead so client and server agree.
            let level = match arg.parse::<i16>() {
                Ok(level) if (1..=PC_LEVEL_MAX as i16).contains(&level) => level,
                Ok(_) => {
                    return send_system_message(
                        client,
                        &format!("Level out of range [1, {}]", PC_LEVEL_MAX),
                    )
                }
                Err(_) => {
                    return send_system_message(
                        client,
                        &format!("Invalid level: {}\n{}", arg, usage),
                    )
                }
            };

            let old_level = player.get_level();
            // Max HP/FM limits are read from the level's stats row; Nanos are
            // a separate contract and are not granted here.
            let new_level = player.set_level(level)?;
            log(
                Severity::Info,
                &format!(
                    "{} changed level {} -> {} via command",
                    player, old_level, new_level
                ),
            );

            let resp = sP_FE2CL_REP_PC_CHANGE_LEVEL {
                iPC_ID: pc_id,
                iPC_Level: new_level,
            };
            // Includes the sender, who is always in its own view.
            state
                .entity_map
                .for_each_around(EntityID::Player(pc_id), |c| {
                    c.send_packet(P_FE2CL_REP_PC_CHANGE_LEVEL, &resp)
                });
            send_system_message(
                client,
                &format!("Level changed from {} to {}", old_level, new_level),
            )
        })
    }

    fn cmd_whois<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__DEVELOPER as i16 {
                return send_system_message(client, NO_ACCESS);
            }

            // Like OpenFusion's NPCManager::getNearestNPC: every NPC in the
            // player's view, by 3D distance, with no interaction-range cap.
            let player_pos = player.get_position();
            let nearest = state
                .entity_map
                .get_around_entity(EntityID::Player(pc_id))
                .into_iter()
                .filter_map(|eid| match eid {
                    EntityID::NPC(npc_id) => state.get_npc(npc_id).ok(),
                    _ => None,
                })
                .min_by_key(|npc| (player_pos.distance_to(&npc.get_position()), npc.id));
            let Some(npc) = nearest else {
                return send_system_message(client, "[WHOIS] No NPCs found nearby");
            };

            let pos = npc.get_position();
            let chunk = npc.get_chunk_coords();
            let lines = [
                format!("ID: {}", npc.id),
                format!("Type: {}", npc.ty),
                format!("Name: {}", npc.get_name()),
                format!("HP: {}", npc.get_hp()),
                "EntityType: NPC".to_string(),
                format!("X: {}", pos.x),
                format!("Y: {}", pos.y),
                format!("Z: {}", pos.z),
                format!("Angle: {}", npc.get_rotation()),
                format!("Chunk: {{{}, {}}}", chunk.x, chunk.y),
                format!("MapNum: {}", chunk.i.map_num),
                format!(
                    "Instance: {}",
                    chunk
                        .i
                        .instance_num
                        .map_or("None".to_string(), |i| i.to_string())
                ),
                format!("Channel: {}", chunk.i.channel_num),
                format!("Distance: {}", player_pos.distance_to(&pos)),
            ];
            for line in lines {
                send_system_message(client, &format!("[WHOIS] {}", line))?;
            }
            Ok(())
        })
    }

    fn cmd_ban<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let is_ban_i = tokens[0] == "ban_i";
            if tokens.len() < 3 {
                return send_system_message(
                    client,
                    &if is_ban_i {
                        format!(
                            "Usage: {}ban_i <pc_id> <duration> [reason]\n\
                    Duration example: 1d3h5m42s (no spaces!)",
                            CUSTOM_COMMAND_PREFIX
                        )
                    } else {
                        format!(
                            "Usage: {}ban_a <account_id> <duration> [reason]\n\
                    Duration example: 1d3h5m42s (no spaces!)",
                            CUSTOM_COMMAND_PREFIX
                        )
                    },
                );
            }

            let own_pc_id = client.get_player_id()?;
            let player = state.get_player(own_pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to ban players");
            }

            let pc_id = match parse_pc_id(tokens[1]) {
                Ok(Some(pc_id)) => pc_id,
                Ok(None) => own_pc_id,
                Err(_) => return send_system_message(client, "Invalid player ID"),
            };

            if own_pc_id == pc_id {
                return send_system_message(client, "You cannot ban yourself");
            }

            let db = db_get();

            let acc_id = if is_ban_i {
                let Ok(player) = state.get_player(pc_id) else {
                    return send_system_message(client, &format!("Player {} not found", pc_id));
                };

                let pc_uid = player.get_uid();
                let Some(acc) = db.find_account_from_player(pc_uid).await? else {
                    return send_system_message(
                        client,
                        &format!("Account not found for player {}", pc_uid),
                    );
                };

                acc.id
            } else {
                let Ok(acc_id) = tokens[1].parse::<i64>() else {
                    return send_system_message(client, "Invalid account ID");
                };

                acc_id
            };

            let Ok(duration) = util::get_duration_from_shorthand(tokens[2]) else {
                return send_system_message(client, "Invalid duration");
            };

            if duration.is_zero() {
                return send_system_message(client, "Duration must be non-zero");
            }

            let banned_until = SystemTime::now() + duration;
            let ban_reason = if tokens.len() > 3 {
                tokens[3..].join(" ")
            } else {
                "No reason given".to_string()
            };

            match db.ban_account(acc_id, banned_until, &ban_reason).await {
                Ok(()) => {
                    let ban_msg = format!(
                        "Account {} banned for {}\n\
                    Reason: {}",
                        acc_id,
                        util::format_duration(duration),
                        ban_reason,
                    );
                    log_if_failed(send_system_message(client, &ban_msg));
                    log(
                        Severity::Info,
                        &format!("{}\nBanned by: {}", ban_msg, player),
                    );
                }
                Err(e) => {
                    return send_system_message(client, &format!("Failed to ban: {}", e.get_msg()));
                }
            }

            if is_ban_i {
                let banned_player = state.get_player(pc_id)?;
                let banned_client = banned_player.get_client().unwrap();
                let pkt = sP_FE2CL_REP_PC_EXIT_SUCC {
                    iID: pc_id,
                    iExitCode: EXIT_CODE_REQ_BY_GM as i32,
                };

                banned_client.send_packet(P_FE2CL_REP_PC_EXIT_SUCC, &pkt);
                banned_client.disconnect();

                let client = clients.get_sender();
                log_if_failed(send_system_message(
                    client,
                    &format!("{} kicked", banned_player),
                ));
            }

            Ok(())
        })
    }

    fn cmd_unban<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            if tokens.len() < 2 {
                return send_system_message(
                    client,
                    &format!("Usage: {}unban <account_id>", CUSTOM_COMMAND_PREFIX),
                );
            }

            let player = state.get_player(client.get_player_id()?)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to unban players");
            }

            let Ok(acc_id) = tokens[1].parse::<i64>() else {
                return send_system_message(client, "Invalid account ID");
            };

            let db = db_get();
            match db.unban_account(acc_id).await {
                Ok(()) => {
                    let unban_msg = format!("Account {} unbanned", acc_id);
                    log(
                        Severity::Info,
                        &format!("{}\nUnbanned by: {}", unban_msg, player),
                    );
                    send_system_message(client, &unban_msg)
                }
                Err(e) => send_system_message(client, &format!("Failed to unban: {}", e.get_msg())),
            }
        })
    }

    fn cmd_queryai<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to change AI");
            }

            let npc_id = if tokens.len() < 2 {
                let player_pos = player.get_position();
                let candidates = state.entity_map.get_around_entity(EntityID::Player(pc_id));
                let mut closest_npc_id = None;
                let mut closest_distance = u32::MAX;
                for eid in candidates {
                    if let EntityID::NPC(npcid) = eid {
                        let npc = state.get_npc(npcid).unwrap();
                        let npc_pos = npc.get_position();
                        let distance = player_pos.distance_to(&npc_pos);
                        if distance < RANGE_INTERACT && distance < closest_distance {
                            closest_npc_id = Some(npc.id);
                            closest_distance = distance;
                        }
                    }
                }
                closest_npc_id
            } else {
                match tokens[1].parse::<i32>() {
                    Ok(id) => Some(id),
                    Err(_) => return send_system_message(client, "Invalid NPC ID"),
                }
            };

            if let Some(npc_id) = npc_id {
                let npc = state.get_npc(npc_id).unwrap();
                match npc.ai.as_ref() {
                    Some(ai_script_name) => send_system_message(
                        client,
                        &format!("NPC {} has AI script: {}", npc, ai_script_name),
                    ),
                    None => send_system_message(
                        client,
                        &format!("NPC {} has no AI script assigned", npc),
                    ),
                }
            } else {
                send_system_message(client, "No NPCs nearby")
            }
        })
    }

    fn cmd_changeai<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to change AI");
            }

            let ai_script_name = if tokens.len() < 2 {
                return send_system_message(
                    client,
                    &format!("Usage: {}changeai <ai_script_name> [tick_mode]\nai_script_name: 'none' to clear\ntick_mode: when_loaded (default) | always", CUSTOM_COMMAND_PREFIX));
            } else {
                tokens[1]
            };

            let tick_mode = if tokens.len() < 3 {
                TickMode::WhenLoaded
            } else {
                match tokens[2].to_lowercase().as_str() {
                    "when_loaded" => TickMode::WhenLoaded,
                    "always" => TickMode::Always,
                    _ => {
                        return send_system_message(
                            client,
                            "Invalid tick mode. Valid options: when_loaded | always",
                        );
                    }
                }
            };

            let player_pos = player.get_position();
            let candidates = state.entity_map.get_around_entity(EntityID::Player(pc_id));
            let mut closest_npc_id = None;
            let mut closest_distance = u32::MAX;
            for eid in candidates {
                if let EntityID::NPC(npcid) = eid {
                    let npc = state.get_npc(npcid).unwrap();
                    let npc_pos = npc.get_position();
                    let distance = player_pos.distance_to(&npc_pos);
                    if distance < RANGE_INTERACT && distance < closest_distance {
                        closest_npc_id = Some(npc.id);
                        closest_distance = distance;
                    }
                }
            }

            if let Some(npc_id) = closest_npc_id {
                let npc = state.get_npc_mut(npc_id).unwrap();
                send_system_message(
                    client,
                    &format!("Changing AI of {} to {}", npc, ai_script_name),
                )?;

                npc.ai = match ai_script_name.to_lowercase().as_str() {
                    "none" => None,
                    other => Some(other.to_string()),
                };

                state
                    .entity_map
                    .set_tick(EntityID::NPC(npc_id), tick_mode)
                    .unwrap();

                Ok(())
            } else {
                send_system_message(client, "No NPCs nearby")
            }
        })
    }

    fn cmd_reload<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to reload scripts");
            }

            let mut scripting = scripting_get().lock();
            scripting.reload()?;
            send_system_message(client, "Scripts reloaded")
        })
    }

    fn cmd_followme<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to move NPCs");
            }

            let player_pos = player.get_position();
            let candidates = state.entity_map.get_around_entity(EntityID::Player(pc_id));
            let mut closest_npc_id = None;
            let mut closest_distance = u32::MAX;
            for eid in candidates {
                if let EntityID::NPC(npcid) = eid {
                    let npc = state.get_npc(npcid).unwrap();
                    let npc_pos = npc.get_position();
                    let distance = player_pos.distance_to(&npc_pos);
                    if distance < RANGE_INTERACT && distance < closest_distance {
                        closest_npc_id = Some(npc.id);
                        closest_distance = distance;
                    }
                }
            }

            if let Some(npc_id) = closest_npc_id {
                let npc = state.get_npc_mut(npc_id).unwrap();
                npc.set_follow(EntityID::Player(pc_id));
                match &npc.ai {
                    None => {
                        npc.ai = Some("follow".to_string());
                        npc.team = CombatantTeam::Friendly;
                        npc.reset();
                        log_if_failed(send_system_message(
                            client,
                            &format!("{} is now following you", npc),
                        ));

                        state
                            .entity_map
                            .set_tick(EntityID::NPC(npc_id), TickMode::WhenLoaded)
                            .unwrap();
                    }
                    Some(ai_script) => {
                        log_if_failed(send_system_message(
                            client,
                            &format!(
                                "{} is unable to follow you (already running AI script: {})",
                                npc, ai_script
                            ),
                        ));
                    }
                }
                Ok(())
            } else {
                send_system_message(client, "No NPCs nearby")
            }
        })
    }

    fn cmd_unfollowme<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
                return send_system_message(client, "You do not have permission to move NPCs");
            }
            let player_pos = player.get_position();

            let candidates = state.entity_map.get_around_entity(EntityID::Player(pc_id));
            let mut closest_npc_id = None;
            let mut closest_distance = u32::MAX;
            for eid in candidates {
                if let EntityID::NPC(npcid) = eid {
                    let npc = state.get_npc(npcid).unwrap();
                    let npc_pos = npc.get_position();
                    let distance = player_pos.distance_to(&npc_pos);
                    if distance < RANGE_INTERACT && distance < closest_distance {
                        closest_npc_id = Some(npc.id);
                        closest_distance = distance;
                    }
                }
            }

            if let Some(npc_id) = closest_npc_id {
                let npc = state.get_npc_mut(npc_id).unwrap();
                if npc.loose_follow == Some(EntityID::Player(pc_id)) {
                    npc.loose_follow = None;
                    npc.ai = None;
                    log_if_failed(send_system_message(
                        client,
                        &format!("{} is no longer following you", npc),
                    ));

                    state
                        .entity_map
                        .set_tick(EntityID::NPC(npc_id), TickMode::Never)
                        .unwrap();

                    Ok(())
                } else {
                    send_system_message(client, &format!("{} is not following you!", npc))
                }
            } else {
                send_system_message(client, "No NPCs nearby")
            }
        })
    }

    fn cmd_perms<'a>(
        tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            if tokens.len() < 2 {
                return send_system_message(
                    client,
                    &format!(
                        "Usage: {}perms <pc_id> [new_level] [\"save\"]\n\
                Use . for pc_id to select yourself\n\
                Leave new_level empty to view the current level\n\
                Add \"save\" to save the new level to the account",
                        CUSTOM_COMMAND_PREFIX
                    ),
                );
            }

            let pc_id = client.get_player_id()?;
            let player = state.get_player(pc_id)?;
            let own_perms = player.perms;

            let target_pc_id = match parse_pc_id(tokens[1]) {
                Ok(Some(pc_id)) => pc_id,
                Ok(None) => pc_id,
                Err(_) => return send_system_message(client, "Invalid player ID"),
            };

            let Ok(target_player) = state.get_player_mut(target_pc_id) else {
                return send_system_message(client, &format!("Player {} not found", target_pc_id));
            };
            let target_perms = target_player.perms;
            let target_uid = target_player.get_uid();

            if tokens.len() < 3 {
                return send_system_message(
                    client,
                    &format!("{} has permissions level {}", target_player, target_perms),
                );
            }

            let Ok(new_perms) = tokens[2].parse::<i16>() else {
                return send_system_message(client, "Invalid permissions level");
            };

            if !(1..=99).contains(&new_perms) {
                return send_system_message(client, "Permissions level out of range [1, 99]");
            }

            if new_perms <= own_perms {
                return send_system_message(
                    client,
                    &format!(
                        "Can only grant weaker permissions than your own (> {})",
                        own_perms
                    ),
                );
            }

            if pc_id != target_pc_id && target_perms <= own_perms {
                return send_system_message(
                client, &format!(
                    "Can only change the permissions of a player with weaker ones than your own (> {})",
                    own_perms
                ),
            );
            }

            target_player.perms = new_perms;
            log_if_failed(send_system_message(
                client,
                &format!(
                    "Permissions level changed to {} for {}",
                    new_perms, target_player
                ),
            ));

            if tokens.get(3).is_some_and(|arg| *arg == "save") {
                let db = db_get();
                let saved = async {
                    let acc =
                        db.find_account_from_player(target_uid)
                            .await?
                            .ok_or(FFError::build(
                                Severity::Warning,
                                format!("Account not found for player with UID {}", target_uid),
                            ))?;
                    db.change_account_level(acc.id, new_perms as i32).await
                }
                .await;
                match saved {
                    Ok(()) => log_if_failed(send_system_message(
                        client,
                        "Permissions level saved to account!",
                    )),
                    Err(e) => log_if_failed(send_system_message(
                        client,
                        &format!("Failed to save permissions level: {}", e.get_msg()),
                    )),
                };
            }
            Ok(())
        })
    }

    fn cmd_ping<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        _state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let client = clients.get_sender();
            let meta = client.meta.read();
            match meta.ping_ms {
                Some(ref ping) => {
                    let ms = ping.load(std::sync::atomic::Ordering::Relaxed);
                    drop(meta);
                    send_system_message(client, &format!("Your ping is {} ms", ms))
                }
                None => {
                    drop(meta);
                    send_system_message(client, "Ping not available")
                }
            }
        })
    }

    fn cmd_refresh<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let pc_id = clients.get_sender().get_player_id()?;
            let player = state.get_player(pc_id)?;
            let chunk_coords = player.get_chunk_coords();

            // remove from chunk completely; pulls all entities out of view
            state.entity_map.update(EntityID::Player(pc_id), None, true);

            // re-add to chunk; pushes all entities back into view
            state
                .entity_map
                .update(EntityID::Player(pc_id), Some(chunk_coords), true);

            Ok(())
        })
    }

    fn cmd_registerall<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let pc_id = clients.get_sender().get_player_id()?;
            let player = state.get_player_mut(pc_id)?;

            player.flags.scamper_flags.set_all_chunks(i32::MAX);
            player.flags.skyway_flags.set_all_chunks(i64::MAX);

            let pkt = sP_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_SUCC {
                eTT: unused!(),
                iLocationID: unused!(),
                iWarpLocationFlag: player.flags.scamper_flags.get_chunk(0).unwrap(),
                aWyvernLocationFlag: player.flags.skyway_flags.to_array().unwrap(),
            };

            player
                .get_client()
                .unwrap()
                .send_packet(P_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_SUCC, &pkt);

            send_system_message(
                clients.get_sender(),
                "All warp locations have been registered.",
            )
        })
    }

    fn cmd_unregisterall<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            let pc_id = clients.get_sender().get_player_id()?;
            let player = state.get_player_mut(pc_id)?;

            player.flags.scamper_flags.set_all_chunks(0);
            player.flags.skyway_flags.set_all_chunks(0);

            let pkt = sP_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_SUCC {
                eTT: unused!(),
                iLocationID: unused!(),
                iWarpLocationFlag: player.flags.scamper_flags.get_chunk(0).unwrap(),
                aWyvernLocationFlag: player.flags.skyway_flags.to_array().unwrap(),
            };

            player
                .get_client()
                .unwrap()
                .send_packet(P_FE2CL_REP_PC_REGIST_TRANSPORTATION_LOCATION_SUCC, &pkt);

            send_system_message(
                clients.get_sender(),
                "All warp locations have been unregistered.",
            )
        })
    }

    fn cmd_help<'a>(
        _tokens: Vec<&'a str>,
        clients: &'a ClientMap<'a>,
        _state: &'a mut ShardServerState,
    ) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
        Box::pin(async move {
            // MOTD holds 512 UTF-16 units. One entry per packet avoids overflow
            // and fits the chat history's individual visible rows.
            send_system_message(clients.get_sender(), "Available commands")?;
            let mut entries: Vec<_> = AVAILABLE_COMMANDS.get().unwrap().iter().collect();
            entries.sort_unstable_by_key(|(name, _)| **name);
            for (cmd_name, cmd) in entries {
                send_system_message(clients.get_sender(), &format!(
                    "{}{}: {}", CUSTOM_COMMAND_PREFIX, cmd_name, cmd.description
                ))?;
            }
            Ok(())
        })
    }
}
