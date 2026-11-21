use super::*;
use std::{future::Future, pin::Pin};
pub(super) fn command<'a>(
    tokens: Vec<&'a str>,
    clients: &'a ClientMap<'a>,
    state: &'a mut ShardServerState,
) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
    Box::pin(async move {
        let id = clients.get_sender().get_player_id()?;
        let player = state.get_player(id)?;
        if player.perms > CN_ACCOUNT_LEVEL__GM as i16 {
            return send_system_message(
                clients.get_sender(),
                "You don't have access to that command!",
            );
        }
        let enabled = match tokens.as_slice() {
            [_] => !player.user_view,
            [_, "on"] => true,
            [_, "off"] => false,
            _ => return send_system_message(clients.get_sender(), "Usage: /user [on|off]"),
        };
        let chunk = player.get_chunk_coords();
        state.entity_map.update(EntityID::Player(id), None, true);
        let player = state.get_player_mut(id)?;
        player.user_view = enabled;
        player.last_npc_type_sync = None;
        clients.get_sender().send_packet(
            P_FE2CL_USER_VIEW,
            &NativeUserView {
                enabled: i32::from(enabled),
            },
        );
        state
            .entity_map
            .update(EntityID::Player(id), Some(chunk), true);
        send_system_message(
            clients.get_sender(),
            if enabled {
                "Player view enabled."
            } else {
                "Player view disabled."
            },
        )
    })
}
