//! Server-owned code rewards: persist the claim and inventory before publishing.
use super::*;
use crate::{
    database::{db_get, Database, DbImpl},
    enums::ItemLocation,
    item::Item,
    tabledata::tdata_get,
};
use std::{future::Future, pin::Pin};

pub(super) fn cmd_redeem<'a>(
    tokens: Vec<&'a str>,
    clients: &'a ClientMap<'a>,
    state: &'a mut ShardServerState,
) -> Pin<Box<dyn Future<Output = FFResult<()>> + Send + 'a>> {
    Box::pin(async move {
        let client = clients.get_sender();
        let Some(raw) = tokens.get(1) else {
            return send_system_message(client, "/redeem: No code specified");
        };
        if tokens.len() != 2 {
            return send_system_message(client, "Usage: /redeem <code>");
        }
        if raw.len() > 256 {
            return send_system_message(client, "/redeem: Code too long");
        }
        let code = raw.to_ascii_lowercase();
        let Some(rewards) = tdata_get().code_rewards(&code)? else {
            return send_system_message(client, "/redeem: Unknown code");
        };
        let player = state.get_player_mut(client.get_player_id()?)?;
        redeem_rewards(db_get(), client, player, &code, rewards).await
    })
}

async fn redeem_rewards<D: DbImpl>(
    db: &Database<D>,
    client: &crate::net::FFClient,
    player: &mut crate::entity::Player,
    code: &str,
    rewards: Vec<Item>,
) -> FFResult<()> {
    let already_redeemed = match db.is_code_redeemed(player.get_uid(), code).await {
        Ok(redeemed) => redeemed,
        Err(error) => {
            log_error(error);
            return send_system_message(
                client,
                "/redeem: Could not save code rewards; try again later",
            );
        }
    };
    if already_redeemed {
        return send_system_message(client, "/redeem: You have already redeemed this code item");
    }
    if player.trade_id.is_some() {
        return send_system_message(client, "/redeem: Finish trading before redeeming a code");
    }
    let Some((staged, slots)) = stage_rewards(player, &rewards)? else {
        return send_system_message(client, "/redeem: Not enough space in inventory");
    };
    // A unique (character, code) claim and all items share a DB transaction.
    match db.redeem_code(&staged, code).await {
        Ok(false) => {
            return send_system_message(client, "/redeem: You have already redeemed this code item")
        }
        Err(error) => {
            log_error(error);
            return send_system_message(
                client,
                "/redeem: Could not save code rewards; try again later",
            );
        }
        Ok(true) => {}
    }
    *player = staged;
    for (slot, item) in slots.into_iter().zip(rewards) {
        client.send_packet(
            P_FE2CL_REP_PC_GIVE_ITEM_SUCC,
            &sP_FE2CL_REP_PC_GIVE_ITEM_SUCC {
                eIL: ItemLocation::Inven as i32,
                iSlotNum: slot as i32,
                Item: Some(item).into_proto(),
            },
        );
    }
    send_system_message(client, "You have redeemed code items")
}

pub(super) fn stage_rewards(
    player: &crate::entity::Player,
    rewards: &[Item],
) -> FFResult<Option<(crate::entity::Player, Vec<usize>)>> {
    if player.trade_id.is_some() {
        return Err(FFError::build(
            Severity::Warning,
            "Cannot redeem while trading".into(),
        ));
    }
    if player.get_free_slots(ItemLocation::Inven) < rewards.len() {
        return Ok(None);
    }
    let mut staged = player.clone();
    let mut slots = Vec::new();
    for item in rewards {
        let slot = staged.find_free_slot(ItemLocation::Inven)?;
        staged.set_item(ItemLocation::Inven, slot, Some(*item))?;
        slots.push(slot);
    }
    Ok(Some((staged, slots)))
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use crate::{
        database::{open_test_sqlite, test_suite},
        entity::Player,
        enums::ItemType,
        net::{ClientMessage, ClientMetadata, FFClient},
    };

    fn messages(rx: &mut tokio::sync::mpsc::UnboundedReceiver<ClientMessage>) -> Vec<Packet> {
        std::iter::from_fn(|| match rx.try_recv().ok()? {
            ClientMessage::SendPacket(packet) => Some(packet),
            _ => None,
        })
        .collect()
    }

    #[tokio::test]
    async fn redeem_atomic_rewards_repeat_full_inventory_and_relogin() {
        test_suite::ensure_init();
        std::fs::create_dir_all("target/bug56").unwrap();
        let path = format!("target/bug56/redeem-{}.db", uuid::Uuid::new_v4());
        let db = open_test_sqlite(&path).await;
        let account = db.create_account("bug56", "unused").await.unwrap();
        let mut player = Player::new(56001, 1);
        db.init_player(account.id, &player).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let client = FFClient::new(
            tx,
            ClientMetadata::new("127.0.0.1:39056".parse().unwrap(), None),
        );
        let reward = Item::new(ItemType::General, 119);
        let free = player.get_free_slots(ItemLocation::Inven);
        redeem_rewards(&db, &client, &mut player, "test", vec![reward, reward])
            .await
            .unwrap();
        let packets = messages(&mut rx);
        for (slot, packet) in packets
            .iter()
            .filter(|p| p.id() == P_FE2CL_REP_PC_GIVE_ITEM_SUCC)
            .enumerate()
        {
            let item = packet.get::<sP_FE2CL_REP_PC_GIVE_ITEM_SUCC>().unwrap();
            assert_eq!({ item.iSlotNum }, slot as i32);
            assert_eq!({ item.Item.iID }, 119);
            assert_eq!({ item.Item.iType }, ItemType::General as i16);
        }
        assert_eq!(
            packets
                .iter()
                .filter(|p| p.id() == P_FE2CL_REP_PC_GIVE_ITEM_SUCC)
                .count(),
            2
        );
        assert_eq!(player.get_free_slots(ItemLocation::Inven), free - 2);
        let mut loaded = db
            .load_player(account.id, player.get_uid())
            .await
            .unwrap()
            .unwrap();
        loaded.set_player_id(1);
        assert_eq!(loaded.get_free_slots(ItemLocation::Inven), free - 2);
        // Even a stale candidate cannot overwrite the first award.
        let stale = Player::new(player.get_uid(), 1);
        assert!(!db.redeem_code(&stale, "test").await.unwrap());
        assert_eq!(
            db.load_player(account.id, player.get_uid())
                .await
                .unwrap()
                .unwrap()
                .get_free_slots(ItemLocation::Inven),
            free - 2
        );
        drop(db);
        let db = open_test_sqlite(&path).await;
        redeem_rewards(&db, &client, &mut loaded, "test", vec![reward])
            .await
            .unwrap();
        let out = messages(&mut rx);
        assert_eq!(out.len(), 1);
        let text =
            util::parse_utf16(&{ out[0].get::<sP_FE2CL_PC_MOTD_LOGIN>().unwrap().szSystemMsg })
                .unwrap();
        assert!(text.contains("already redeemed"));
        // No partial rewards or claim when the entire batch cannot fit.
        while let Ok(slot) = loaded.find_free_slot(ItemLocation::Inven) {
            loaded
                .set_item(ItemLocation::Inven, slot, Some(reward))
                .unwrap();
        }
        redeem_rewards(&db, &client, &mut loaded, "full", vec![reward])
            .await
            .unwrap();
        assert!(!db.is_code_redeemed(loaded.get_uid(), "full").await.unwrap());
        assert_eq!(messages(&mut rx).len(), 1);
        loaded.trade_id = None;
        loaded.set_item(ItemLocation::Inven, 0, None).unwrap();
        let sql = deadpool_sqlite::rusqlite::Connection::open(&path).unwrap();
        sql.execute_batch("CREATE TRIGGER fail_award BEFORE INSERT ON inventory BEGIN SELECT RAISE(ABORT, 'test award failure'); END;").unwrap();
        redeem_rewards(&db, &client, &mut loaded, "db-failure", vec![reward])
            .await
            .unwrap();
        assert!(loaded.get_item(ItemLocation::Inven, 0).unwrap().is_none());
        assert!(!db
            .is_code_redeemed(loaded.get_uid(), "db-failure")
            .await
            .unwrap());
        let failure = messages(&mut rx);
        assert_eq!(failure.len(), 1);
        assert_eq!(failure[0].id(), P_FE2CL_PC_MOTD_LOGIN);
        assert_eq!(
            db.load_player(account.id, player.get_uid())
                .await
                .unwrap()
                .unwrap()
                .get_free_slots(ItemLocation::Inven),
            free - 2
        );
        sql.execute_batch("DROP TRIGGER fail_award;").unwrap();
        redeem_rewards(&db, &client, &mut loaded, "db-failure", vec![reward])
            .await
            .unwrap();
        assert!(db
            .is_code_redeemed(loaded.get_uid(), "db-failure")
            .await
            .unwrap());
        assert_eq!(
            messages(&mut rx)
                .iter()
                .filter(|p| p.id() == P_FE2CL_REP_PC_GIVE_ITEM_SUCC)
                .count(),
            1
        );
        loaded.set_item(ItemLocation::Inven, 0, None).unwrap();
        loaded.trade_id = Some(uuid::Uuid::new_v4());
        redeem_rewards(&db, &client, &mut loaded, "trade", vec![reward])
            .await
            .unwrap();
        assert!(!db
            .is_code_redeemed(loaded.get_uid(), "trade")
            .await
            .unwrap());
        assert_eq!(messages(&mut rx).len(), 1);
    }
}
