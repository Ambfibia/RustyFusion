use super::*;
use std::collections::HashMap;

use crate::{
    chunk::{ChunkCoords, InstanceID, TickMode},
    database::{open_test_sqlite, DbImpl},
    entity::{Entity, Player, NPC},
    enums::ItemType,
    item::Item,
    net::{ClientMessage, ClientMetadata, ClientType},
    tabledata::tdata_init,
    Position,
};

type Rx = tokio::sync::mpsc::UnboundedReceiver<ClientMessage>;
const BARBER_ID: i32 = 900_347;

fn place(state: &mut ShardServerState, entity: Box<dyn Entity>) {
    let id = entity.get_id();
    let chunk = ChunkCoords::from_pos_inst(entity.get_position(), InstanceID::default());
    state.entity_map.track(entity, TickMode::Always);
    state.entity_map.update(id, Some(chunk), false);
}

fn fixture() -> (ShardServerState, HashMap<usize, FFClient>, Rx, Rx) {
    tdata_init().unwrap();
    let mut state = ShardServerState::default();
    let mut clients = HashMap::new();
    let mut receivers = Vec::new();
    let pos = Position {
        x: 200_000,
        y: 200_000,
        z: 0,
    };
    for id in [1, 2] {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let client = FFClient::new(
            tx,
            ClientMetadata::new(
                format!("127.0.0.1:{}", 39200 + id).parse().unwrap(),
                Some(ClientType::GameClient {
                    account_id: id as i64,
                    serial_key: id as i64,
                    pc_id: Some(id),
                }),
            ),
        );
        let mut player = Player::new(id as i64, 0);
        player.set_player_id(id);
        player.set_position(pos);
        player.set_client(client.clone());
        player.style = Some(PlayerStyle::default());
        player.set_taros(1_000);
        place(&mut state, Box::new(player));
        clients.insert(id as usize, client);
        receivers.push(rx);
    }
    place(
        &mut state,
        Box::new(NPC::new(BARBER_ID, 3470, pos, 0, InstanceID::default()).unwrap()),
    );
    (state, clients, receivers.remove(0), receivers.remove(0))
}

fn drain(rx: &mut Rx) -> Vec<Packet> {
    std::iter::from_fn(|| match rx.try_recv().ok()? {
        ClientMessage::SendPacket(packet) => Some(packet),
        _ => None,
    })
    .collect()
}

fn quote(
    state: &mut ShardServerState,
    clients: &HashMap<usize, FFClient>,
    rx: &mut Rx,
) -> BarberPrices {
    open(
        Packet::new(
            P_CL2FE_REQ_PC_BARBER_OPEN,
            &BarberOpenRequest { npc_id: BARBER_ID },
        )
        .unwrap(),
        clients.get(&1).unwrap(),
        state,
    )
    .unwrap();
    let packets = drain(rx);
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].id(), P_FE2CL_REP_PC_BARBER_OPEN_SUCC);
    *packets[0].get::<BarberPrices>().unwrap()
}

fn submit(
    state: &mut ShardServerState,
    clients: &HashMap<usize, FFClient>,
    style: sPCStyle,
) -> FFResult<()> {
    confirm(
        Packet::new(
            P_CL2FE_REQ_PC_BARBER_CONFIRM,
            &BarberConfirmRequest { style },
        )
        .unwrap(),
        &ClientMap::new(1, clients),
        state,
    )
}

fn gendered_shirt() -> Item {
    let id = (1..=i16::MAX)
        .find(|id| {
            tdata_get()
                .get_item_stats(*id, ItemType::UpperBody)
                .is_ok_and(|stats| stats.gender == Some(1))
        })
        .expect("the shipped item table must have a male-only shirt");
    Item::new(ItemType::UpperBody, id)
}

#[test]
fn quote_confirm_replay_and_observer_are_consistent() {
    let (mut state, clients, mut owner, mut observer) = fixture();
    let prices = quote(&mut state, &clients, &mut owner);
    assert_eq!(prices.change_gender, 1);
    let shirt = gendered_shirt();
    state
        .get_player_mut(1)
        .unwrap()
        .set_item(ItemLocation::Equip, 1, Some(shirt))
        .unwrap();
    let mut style = state.get_player(1).unwrap().get_style();
    style.iGender = 2;
    style.iFaceStyle = 6;
    style.iHairStyle = 25;
    let expected_cost = cost(
        PlayerStyle::default(),
        PlayerStyle::try_from(style).unwrap(),
        prices,
    )
    .unwrap();
    submit(&mut state, &clients, style).unwrap();
    let reply = drain(&mut owner);
    let answer = reply
        .iter()
        .find(|p| p.id() == P_FE2CL_REP_PC_BARBER_CONFIRM)
        .unwrap()
        .get::<BarberReply>()
        .unwrap();
    assert_eq!(answer.error, 0);
    assert_eq!(answer.taros, (1_000 - expected_cost) as i32);
    assert_eq!(answer.slots[0], 0);
    let player = state.get_player(1).unwrap();
    assert_eq!(player.get_taros(), 1_000 - expected_cost);
    assert_eq!(player.style.unwrap().gender, 2);
    assert_eq!(*player.get_item(ItemLocation::Equip, 1).unwrap(), None);
    assert_eq!(
        *player.get_item(ItemLocation::Inven, 0).unwrap(),
        Some(shirt)
    );
    let seen = drain(&mut observer);
    assert!(seen.iter().any(|p| p.id() == P_FE2CL_PC_EQUIP_CHANGE));
    assert!(seen.iter().any(|p| p.id() == P_FE2CL_PC_CHANGE_STYLE));
    assert!(submit(&mut state, &clients, style).is_err());
    assert_eq!(
        drain(&mut owner)
            .iter()
            .find(|p| p.id() == P_FE2CL_REP_PC_BARBER_CONFIRM)
            .unwrap()
            .get::<BarberReply>()
            .unwrap()
            .error,
        2
    );
    assert_eq!(
        state.get_player(1).unwrap().get_taros(),
        1_000 - expected_cost
    );
}

#[test]
fn invalid_value_funds_room_and_distance_preserve_the_entire_state() {
    let (mut state, clients, mut owner, mut observer) = fixture();
    let shirt = gendered_shirt();
    state
        .get_player_mut(1)
        .unwrap()
        .set_item(ItemLocation::Equip, 1, Some(shirt))
        .unwrap();
    let original = state.get_player(1).unwrap().get_style();
    let mut draft = original;
    draft.iGender = 2;
    draft.iFaceStyle = 6;
    draft.iHairStyle = 25;
    let check = |state: &ShardServerState| {
        let p = state.get_player(1).unwrap();
        assert_eq!(p.style.unwrap().gender, 1);
        assert_eq!(*p.get_item(ItemLocation::Equip, 1).unwrap(), Some(shirt));
        assert_eq!(p.get_taros(), 1_000);
    };
    quote(&mut state, &clients, &mut owner);
    let mut bad = draft;
    bad.iHairStyle = 127;
    assert!(submit(&mut state, &clients, bad).is_err());
    assert_eq!(drain(&mut owner)[0].get::<BarberReply>().unwrap().error, 2);
    check(&state);
    state.get_player_mut(1).unwrap().set_taros(0);
    assert!(submit(&mut state, &clients, draft).is_err());
    assert_eq!(drain(&mut owner)[0].get::<BarberReply>().unwrap().error, 2);
    assert_eq!(state.get_player(1).unwrap().get_taros(), 0);
    state.get_player_mut(1).unwrap().set_taros(1_000);
    for slot in 0..SIZEOF_INVEN_SLOT as usize {
        state
            .get_player_mut(1)
            .unwrap()
            .set_item(
                ItemLocation::Inven,
                slot,
                Some(Item::new(ItemType::General, 1)),
            )
            .unwrap();
    }
    assert!(submit(&mut state, &clients, draft).is_err());
    assert_eq!(drain(&mut owner)[0].get::<BarberReply>().unwrap().error, 1);
    check(&state);
    let player = state.get_player_mut(1).unwrap();
    player.set_item(ItemLocation::Inven, 0, None).unwrap();
    player.set_position(Position {
        x: 300_000,
        y: 200_000,
        z: 0,
    });
    assert!(submit(&mut state, &clients, draft).is_err());
    assert_eq!(drain(&mut owner)[0].get::<BarberReply>().unwrap().error, 2);
    check(&state);
    assert!(drain(&mut observer).is_empty());
}

#[tokio::test]
async fn confirmed_style_money_and_clothes_survive_reload() {
    let (mut state, clients, mut owner, _observer) = fixture();
    let db_path = format!("target/t14/barber-{}.db", uuid::Uuid::new_v4());
    std::fs::create_dir_all("target/t14").unwrap();
    let db = open_test_sqlite(&db_path).await;
    let account = db
        .create_account("barber_reload", "unused-test-hash")
        .await
        .unwrap();
    db.init_player(account.id, state.get_player(1).unwrap())
        .await
        .unwrap();

    let shirt = gendered_shirt();
    state
        .get_player_mut(1)
        .unwrap()
        .set_item(ItemLocation::Equip, 1, Some(shirt))
        .unwrap();
    let prices = quote(&mut state, &clients, &mut owner);
    let mut style = state.get_player(1).unwrap().get_style();
    style.iGender = 2;
    style.iFaceStyle = 6;
    style.iHairStyle = 25;
    let expected_cost = cost(
        PlayerStyle::default(),
        PlayerStyle::try_from(style).unwrap(),
        prices,
    )
    .unwrap();
    submit(&mut state, &clients, style).unwrap();
    assert_eq!(drain(&mut owner)[0].get::<BarberReply>().unwrap().error, 0);
    db.save_player(state.get_player(1).unwrap()).await.unwrap();
    let loaded = db.load_player(account.id, 1).await.unwrap().unwrap();
    assert_eq!(loaded.style.unwrap().gender, 2);
    assert_eq!(loaded.get_taros(), 1_000 - expected_cost);
    assert_eq!(*loaded.get_item(ItemLocation::Equip, 1).unwrap(), None);
    assert_eq!(
        *loaded.get_item(ItemLocation::Inven, 0).unwrap(),
        Some(shirt)
    );
}
