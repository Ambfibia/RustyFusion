use std::time::SystemTime;

use crate::{
    database::{db_get, DbImpl as _},
    defines::*,
    email::{get_email_cost, Email, EMAIL_ITEM_SLOTS},
    entity::Entity,
    enums::ItemLocation,
    error::*,
    helpers,
    item::Item,
    monitor::monitor_event_to_packet,
    net::{
        packet::{PacketID::*, *},
        ClientMap, FFClient,
    },
    state::ShardServerState,
    util,
};

/// Error code the client shows for "you can't send goodies across time".
const ERROR_CODE_EMAIL_CROSS_TIME: i32 = 9;
/// Generic "couldn't do that" error code.
const ERROR_CODE_EMAIL_GENERIC: i32 = 1;

/// Tells the client how many unread emails are waiting. Also used as a
/// server-initiated "you've got mail" nudge when an email lands.
pub fn notify_new_email(client: &FFClient, num_unread: i32) {
    let resp = sP_FE2CL_REP_PC_NEW_EMAIL {
        iNewEmailCnt: num_unread,
    };
    client.send_packet(P_FE2CL_REP_PC_NEW_EMAIL, &resp);
}

pub async fn email_update_check(
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let client = clients.get_sender();
    let pc_uid = state.get_player(client.get_player_id()?)?.get_uid();
    let num_unread = db_get().get_unread_email_count(pc_uid).await?;
    notify_new_email(client, num_unread);
    Ok(())
}

pub async fn email_receive_page_list(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_RECV_EMAIL_PAGE_LIST = pkt.get()?;
    let page_num = pkt.iPageNum;
    let client = clients.get_sender();
    let pc_uid = state.get_player(client.get_player_id()?)?.get_uid();

    (async {
        let emails = db_get().load_emails(pc_uid, page_num as i32).await?;
        let mut resp = sP_FE2CL_REP_PC_RECV_EMAIL_PAGE_LIST_SUCC {
            iPageNum: page_num,
            aEmailInfo: Default::default(),
        };
        for (idx, email) in emails.iter().enumerate() {
            resp.aEmailInfo[idx] = email.to_email_info()?;
        }
        client.send_packet(P_FE2CL_REP_PC_RECV_EMAIL_PAGE_LIST_SUCC, &resp);
        Ok(())
    })
    .await
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_RECV_EMAIL_PAGE_LIST_FAIL {
            iPageNum: page_num,
            iErrorCode: ERROR_CODE_EMAIL_GENERIC,
        };
        client.send_packet(P_FE2CL_REP_PC_RECV_EMAIL_PAGE_LIST_FAIL, &resp);
    })
}

pub async fn email_read(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_READ_EMAIL = pkt.get()?;
    let msg_index = pkt.iEmailIndex;
    let client = clients.get_sender();
    let pc_uid = state.get_player(client.get_player_id()?)?.get_uid();

    (async {
        let mut email = get_email(pc_uid, msg_index).await?;

        // mark as read. we do this before replying so a dropped reply
        // doesn't leave the email stuck as unread forever.
        if !email.read {
            email.read = true;
            db_get().update_email(&email).await?;
        }

        let mut resp = sP_FE2CL_REP_PC_READ_EMAIL_SUCC {
            iEmailIndex: msg_index,
            szContent: util::encode_utf16(&email.body)?,
            aItem: Default::default(),
            iCash: email.taros as i32,
        };
        for (idx, item) in email.attachments.iter().enumerate() {
            resp.aItem[idx] = (*item).into_proto();
        }
        client.send_packet(P_FE2CL_REP_PC_READ_EMAIL_SUCC, &resp);
        Ok(())
    })
    .await
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_READ_EMAIL_FAIL {
            iEmailIndex: msg_index,
            iErrorCode: ERROR_CODE_EMAIL_GENERIC,
        };
        client.send_packet(P_FE2CL_REP_PC_READ_EMAIL_FAIL, &resp);
    })
}

pub async fn email_receive_taros(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_RECV_EMAIL_CANDY = pkt.get()?;
    let msg_index = pkt.iEmailIndex;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pc_uid = state.get_player(pc_id)?.get_uid();

    (async {
        let mut email = get_email(pc_uid, msg_index).await?;
        if email.taros == 0 {
            return Err(FFError::build(
                Severity::Warning,
                format!("Email {} has no taros attached", msg_index),
            ));
        }

        let player = state.get_player_mut(pc_id)?;
        let new_taros = (player.get_taros() as u64 + email.taros as u64).min(PC_CANDY_MAX as u64);
        let taros_taken = new_taros - player.get_taros() as u64;
        if taros_taken == 0 {
            return Err(FFError::build(
                Severity::Warning,
                format!("Player {} is at the taro cap", pc_id),
            ));
        }
        player.set_taros(new_taros as u32);

        // only clear what actually fit; the rest stays in the mailbox
        email.taros -= taros_taken as u32;
        db_get().update_email(&email).await?;

        let resp = sP_FE2CL_REP_PC_RECV_EMAIL_CANDY_SUCC {
            iEmailIndex: msg_index,
            iCandy: new_taros as i32,
        };
        client.send_packet(P_FE2CL_REP_PC_RECV_EMAIL_CANDY_SUCC, &resp);
        Ok(())
    })
    .await
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_RECV_EMAIL_CANDY_FAIL {
            iEmailIndex: msg_index,
            iErrorCode: ERROR_CODE_EMAIL_GENERIC,
        };
        client.send_packet(P_FE2CL_REP_PC_RECV_EMAIL_CANDY_FAIL, &resp);
    })
}

pub async fn email_receive_item(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_RECV_EMAIL_ITEM = pkt.get()?;
    let msg_index = pkt.iEmailIndex;
    let email_slot = pkt.iEmailItemSlot;
    let inven_slot = pkt.iSlotNum;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pc_uid = state.get_player(pc_id)?.get_uid();

    (async {
        if inven_slot < 0 || inven_slot >= SIZEOF_INVEN_SLOT as i32 {
            return Err(FFError::build(
                Severity::Warning,
                format!("Bad inventory slot {}", inven_slot),
            ));
        }

        let mut email = get_email(pc_uid, msg_index).await?;
        let item = email.take_attachment(email_slot as usize)?.ok_or_else(|| {
            FFError::build(
                Severity::Warning,
                format!("Email {} has nothing in slot {}", msg_index, email_slot),
            )
        })?;

        let player = state.get_player_mut(pc_id)?;
        if player
            .get_item(ItemLocation::Inven, inven_slot as usize)?
            .is_some()
        {
            return Err(FFError::build(
                Severity::Warning,
                format!("Inventory slot {} is occupied", inven_slot),
            ));
        }
        player.set_item(ItemLocation::Inven, inven_slot as usize, Some(item))?;
        db_get().update_email(&email).await?;

        let resp = sP_FE2CL_REP_PC_RECV_EMAIL_ITEM_SUCC {
            iEmailIndex: msg_index,
            iSlotNum: inven_slot,
            iEmailItemSlot: email_slot,
        };
        client.send_packet(P_FE2CL_REP_PC_RECV_EMAIL_ITEM_SUCC, &resp);
        send_give_item(client, inven_slot, Some(item));
        Ok(())
    })
    .await
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_RECV_EMAIL_ITEM_FAIL {
            iEmailIndex: msg_index,
            iSlotNum: inven_slot,
            iEmailItemSlot: email_slot,
            iErrorCode: ERROR_CODE_EMAIL_GENERIC,
        };
        client.send_packet(P_FE2CL_REP_PC_RECV_EMAIL_ITEM_FAIL, &resp);
    })
}

pub async fn email_receive_item_all(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_RECV_EMAIL_ITEM_ALL = pkt.get()?;
    let msg_index = pkt.iEmailIndex;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;
    let pc_uid = state.get_player(pc_id)?.get_uid();

    (async {
        let mut email = get_email(pc_uid, msg_index).await?;
        let num_attachments = email.attachments.iter().filter(|i| i.is_some()).count();
        if num_attachments == 0 {
            return Err(FFError::build(
                Severity::Warning,
                format!("Email {} has no attachments", msg_index),
            ));
        }

        let player = state.get_player(pc_id)?;
        if player.get_free_slots(ItemLocation::Inven) < num_attachments {
            return Err(FFError::build(
                Severity::Warning,
                format!("Player {} has no room for {} items", pc_id, num_attachments),
            ));
        }

        // all-or-nothing: we checked for room above, so nothing is lost partway
        let mut given = Vec::with_capacity(num_attachments);
        for slot_num in 1..=EMAIL_ITEM_SLOTS {
            let Some(item) = email.take_attachment(slot_num)? else {
                continue;
            };
            let player = state.get_player_mut(pc_id)?;
            let inven_slot = player.find_free_slot(ItemLocation::Inven)?;
            player.set_item(ItemLocation::Inven, inven_slot, Some(item))?;
            given.push((inven_slot as i32, item));
        }
        db_get().update_email(&email).await?;

        let resp = sP_FE2CL_REP_PC_RECV_EMAIL_ITEM_ALL_SUCC {
            iEmailIndex: msg_index,
        };
        client.send_packet(P_FE2CL_REP_PC_RECV_EMAIL_ITEM_ALL_SUCC, &resp);
        for (inven_slot, item) in given {
            send_give_item(client, inven_slot, Some(item));
        }
        Ok(())
    })
    .await
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_RECV_EMAIL_ITEM_ALL_FAIL {
            iEmailIndex: msg_index,
            iErrorCode: ERROR_CODE_EMAIL_GENERIC,
        };
        client.send_packet(P_FE2CL_REP_PC_RECV_EMAIL_ITEM_ALL_FAIL, &resp);
    })
}

pub async fn email_delete(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: &sP_CL2FE_REQ_PC_DELETE_EMAIL = pkt.get()?;
    let msg_indices = pkt.iEmailIndexArray;
    let client = clients.get_sender();
    let pc_uid = state.get_player(client.get_player_id()?)?.get_uid();

    (async {
        db_get().delete_emails(pc_uid, &msg_indices).await?;
        let resp = sP_FE2CL_REP_PC_DELETE_EMAIL_SUCC {
            iEmailIndexArray: msg_indices,
        };
        client.send_packet(P_FE2CL_REP_PC_DELETE_EMAIL_SUCC, &resp);
        Ok(())
    })
    .await
    .catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_DELETE_EMAIL_FAIL {
            iEmailIndexArray: msg_indices,
            iErrorCode: ERROR_CODE_EMAIL_GENERIC,
        };
        client.send_packet(P_FE2CL_REP_PC_DELETE_EMAIL_FAIL, &resp);
    })
}

pub async fn email_send(
    pkt: Packet,
    clients: &ClientMap<'_>,
    state: &mut ShardServerState,
) -> FFResult<()> {
    let pkt: sP_CL2FE_REQ_PC_SEND_EMAIL = *pkt.get()?;
    let recipient_uid = pkt.iTo_PCUID;
    let client = clients.get_sender();
    let pc_id = client.get_player_id()?;

    let mut error_code = ERROR_CODE_EMAIL_GENERIC;
    let result = (async {
        let db = db_get();
        let player = state.get_player(pc_id)?;
        if player.get_uid() == recipient_uid {
            return Err(FFError::build(
                Severity::Warning,
                format!("Player {} tried to email themselves", pc_id),
            ));
        }

        let taros_attached = if pkt.iCash < 0 {
            return Err(FFError::build(
                Severity::Warning,
                format!("Player {} tried to attach negative taros", pc_id),
            ));
        } else {
            pkt.iCash as u32
        };

        // pull the attachments off the sender, validating each against the
        // authoritative inventory as we go
        let mut attachments: [Option<Item>; EMAIL_ITEM_SLOTS] = [None; EMAIL_ITEM_SLOTS];
        let mut taken: Vec<(usize, Item)> = Vec::new();
        let mut num_attachments = 0;
        let mut seen_slots = Vec::new();
        for (idx, attachment) in pkt.aItem.iter().enumerate() {
            if attachment.ItemInven.iID == 0 {
                continue;
            }

            let slot_num = attachment.iSlotNum;
            if slot_num < 0 || slot_num >= SIZEOF_INVEN_SLOT as i32 {
                return Err(FFError::build(
                    Severity::Warning,
                    format!("Bad attachment slot {}", slot_num),
                ));
            }
            let slot_num = slot_num as usize;
            if seen_slots.contains(&slot_num) {
                return Err(FFError::build(
                    Severity::Warning,
                    format!("Attachment slot {} referenced twice", slot_num),
                ));
            }
            seen_slots.push(slot_num);

            let requested: Option<Item> = attachment.ItemInven.try_into_proto()?;
            let requested = requested.ok_or_else(|| {
                FFError::build(Severity::Warning, "Bad attachment item".to_string())
            })?;

            let player = state.get_player(pc_id)?;
            let real = player
                .get_item(ItemLocation::Inven, slot_num)?
                .ok_or_else(|| {
                    FFError::build(
                        Severity::Warning,
                        format!("Attachment slot {} is empty", slot_num),
                    )
                })?;
            if real.id != requested.id || real.ty != requested.ty {
                return Err(FFError::build(
                    Severity::Warning,
                    format!("Attachment slot {} doesn't hold that item", slot_num),
                ));
            }
            if !real.get_stats()?.tradeable {
                return Err(FFError::build(
                    Severity::Warning,
                    format!("Item in slot {} isn't tradeable", slot_num),
                ));
            }
            if requested.quantity == 0 || requested.quantity > real.quantity {
                return Err(FFError::build(
                    Severity::Warning,
                    format!("Bad attachment quantity for slot {}", slot_num),
                ));
            }

            let player = state.get_player_mut(pc_id)?;
            let slot = player.get_item_mut(ItemLocation::Inven, slot_num)?;
            let split = Item::split_items(slot, requested.quantity).ok_or_else(|| {
                FFError::build(
                    Severity::Warning,
                    format!("Couldn't split item in slot {}", slot_num),
                )
            })?;
            attachments[idx] = Some(split);
            taken.push((slot_num, split));
            num_attachments += 1;
        }

        // roll the sender's inventory back if anything below fails
        let refund = |state: &mut ShardServerState, taken: &[(usize, Item)]| {
            for (slot_num, item) in taken {
                let Ok(player) = state.get_player_mut(pc_id) else {
                    return;
                };
                if let Ok(slot) = player.get_item_mut(ItemLocation::Inven, *slot_num) {
                    let mut item = Some(*item);
                    log_if_failed(Item::transfer_items(&mut item, slot));
                }
            }
        };

        let cost = get_email_cost(taros_attached, num_attachments);
        let player = state.get_player(pc_id)?;
        if player.get_taros() < cost {
            refund(state, &taken);
            return Err(FFError::build(
                Severity::Warning,
                format!("Player {} can't afford the {} taro postage", pc_id, cost),
            ));
        }

        // The recipient has to exist, and goodies can't cross time periods.
        let recipient = match db.find_account_from_player(recipient_uid).await? {
            Some(account) => db.load_player(account.id, recipient_uid).await?,
            None => None,
        };
        let Some(recipient) = recipient else {
            refund(state, &taken);
            return Err(FFError::build(
                Severity::Warning,
                format!("Email recipient {} doesn't exist", recipient_uid),
            ));
        };

        let player = state.get_player(pc_id)?;
        if (taros_attached > 0 || num_attachments > 0)
            && player.get_payzone_flag() != recipient.get_payzone_flag()
        {
            refund(state, &taken);
            error_code = ERROR_CODE_EMAIL_CROSS_TIME;
            return Err(FFError::build(
                Severity::Warning,
                format!(
                    "Player {} tried to send attachments across time to {}",
                    pc_id, recipient_uid
                ),
            ));
        }

        let mut email = Email::new(player, recipient_uid);
        let subject = helpers::sanitize_text(&util::parse_utf16(&pkt.szSubject)?, false);
        email.subject = helpers::truncate_utf16(&subject, SIZEOF_EMAIL_SUBJECT_STRING as usize);
        let body = helpers::sanitize_text(&util::parse_utf16(&pkt.szContent)?, true);
        email.body = helpers::truncate_utf16(&body, SIZEOF_EMAIL_CONTENT_STRING as usize);
        email.taros = taros_attached;
        email.attachments = attachments;
        email.send_time = SystemTime::now();

        if let Err(e) = db.send_email(&email).await {
            refund(state, &taken);
            return Err(e);
        }

        // charge postage only once the email is safely stored
        let player = state.get_player_mut(pc_id)?;
        let new_taros = player.get_taros() - cost;
        player.set_taros(new_taros);

        let mut resp = sP_FE2CL_REP_PC_SEND_EMAIL_SUCC {
            iTo_PCUID: recipient_uid,
            iCandy: new_taros as i32,
            aItem: pkt.aItem,
        };
        // report the post-send state of every slot we touched
        for (idx, attachment) in resp.aItem.iter_mut().enumerate() {
            if attachments[idx].is_none() {
                *attachment = Default::default();
            }
        }
        client.send_packet(P_FE2CL_REP_PC_SEND_EMAIL_SUCC, &resp);

        // the taro counter in the GUI only refreshes off a set-value packet
        let taros_pkt = sP_FE2CL_GM_REP_PC_SET_VALUE {
            iPC_ID: pc_id,
            iSetValueType: CN_GM_SET_VALUE_TYPE__CANDY as i32,
            iSetValue: new_taros as i32,
        };
        client.send_packet(P_FE2CL_GM_REP_PC_SET_VALUE, &taros_pkt);

        // update the sender's inventory view for every slot we drew from
        for (slot_num, _) in &taken {
            let player = state.get_player(pc_id)?;
            let remaining = *player.get_item(ItemLocation::Inven, *slot_num)?;
            send_give_item(client, *slot_num as i32, remaining);
        }

        // monitor event
        if let Some(login_server) = clients.get_login_server() {
            let sender = state.get_player(pc_id)?;
            let email_event = ffmonitor::EmailEvent {
                from: sender.to_string(),
                to: format!("{} {}", recipient.first_name, recipient.last_name),
                subject: if email.subject.is_empty() {
                    None
                } else {
                    Some(email.subject.clone())
                },
                body: email.body.lines().map(|ln| ln.to_string()).collect(),
            };
            let monitor_pkt = monitor_event_to_packet(ffmonitor::Event::Email(email_event))?;
            login_server.send_packet(P_FE2LS_UPDATE_MONITOR, &monitor_pkt);
        } else {
            log(
                Severity::Warning,
                "No login server to send monitor email event",
            );
        }

        // ping the recipient if they happen to be on this shard.
        // the client handle is cloned out first so we don't hold a borrow of
        // the shard state across the DB await.
        let recipient_client = state
            .get_player_by_uid(recipient_uid)
            .and_then(|recipient| recipient.get_client());
        if let Some(recipient_client) = recipient_client {
            let num_unread = db.get_unread_email_count(recipient_uid).await.unwrap_or(1);
            notify_new_email(&recipient_client, num_unread);
        }

        Ok(())
    })
    .await;

    result.catch_fail(|| {
        let resp = sP_FE2CL_REP_PC_SEND_EMAIL_FAIL {
            iTo_PCUID: recipient_uid,
            iErrorCode: error_code,
        };
        client.send_packet(P_FE2CL_REP_PC_SEND_EMAIL_FAIL, &resp);
    })
}

async fn get_email(pc_uid: i64, msg_index: i64) -> FFResult<Email> {
    if msg_index <= 0 || msg_index > i32::MAX as i64 {
        return Err(FFError::build(
            Severity::Warning,
            format!("Bad email index {}", msg_index),
        ));
    }
    db_get()
        .load_email(pc_uid, msg_index as i32)
        .await?
        .ok_or_else(|| {
            FFError::build(
                Severity::Warning,
                format!("Email {} not found for player {}", msg_index, pc_uid),
            )
        })
}

fn send_give_item(client: &FFClient, slot_num: i32, item: Option<Item>) {
    let pkt = sP_FE2CL_REP_PC_GIVE_ITEM_SUCC {
        eIL: ItemLocation::Inven as i32,
        iSlotNum: slot_num,
        Item: item.into_proto(),
    };
    client.send_packet(P_FE2CL_REP_PC_GIVE_ITEM_SUCC, &pkt);
}
