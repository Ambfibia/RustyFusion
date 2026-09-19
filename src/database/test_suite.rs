use std::sync::Once;
use std::time::{Duration, SystemTime};

use crate::config::Config;
use crate::database::{Database, DbImpl};
use crate::defines::DB_VERSION;
use crate::entity::Player;

#[macro_export]
macro_rules! for_each_db_test {
    ($macro:ident) => {
        $macro!(test_meta);
        $macro!(test_account_crud);
        $macro!(test_player_init_load);
        $macro!(test_player_save_reload);
        $macro!(test_save_players_batch);
        $macro!(test_player_appearance);
        $macro!(test_player_selected);
        $macro!(test_player_delete);
        $macro!(test_email_round_trip);
        $macro!(test_race_results);
    };
}

pub fn ensure_init() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        crate::tabledata::tdata_init().expect("tdata_init failed");
    });
}

pub fn build_config(db_path: &str) -> Config {
    let toml = format!(
        "[general]\ndb_path = \"{}\"\n",
        db_path.replace('\\', "\\\\")
    );
    Config::from_str(&toml).expect("test config parse")
}

fn make_player(uid: i64, slot_num: usize) -> Player {
    let mut p = Player::new(uid, slot_num);
    p.first_name = format!("Test{}", uid);
    p.last_name = "Player".to_string();
    p
}

// Tests //

pub async fn test_meta<D: DbImpl>(db: &Database<D>) {
    let v = db.get_db_version().await.expect("get_db_version");
    assert_eq!(v, DB_VERSION, "db version should equal compiled DB_VERSION");
}

pub async fn test_account_crud<D: DbImpl>(db: &Database<D>) {
    // create + lookup
    let acc = db
        .create_account("cake", "$2b$10$hash")
        .await
        .expect("create_account");
    assert_eq!(acc.username, "cake");
    assert!(acc.id > 0);

    let found = db
        .find_account_from_username("cake")
        .await
        .expect("find_account_from_username")
        .expect("account exists");
    assert_eq!(found.id, acc.id);
    assert_eq!(found.password_hashed, "$2b$10$hash");

    // missing
    let missing = db
        .find_account_from_username("nobody")
        .await
        .expect("find_account_from_username (missing)");
    assert!(missing.is_none());

    // change level
    db.change_account_level(acc.id, 50)
        .await
        .expect("change_account_level");
    let after = db
        .find_account_from_username("cake")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.account_level, 50);

    // ban + unban
    let until = SystemTime::now() + Duration::from_secs(3600);
    db.ban_account(acc.id, until, "spamming")
        .await
        .expect("ban_account");
    let banned = db
        .find_account_from_username("cake")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(banned.ban_reason, "spamming");
    assert!(
        banned.banned_until > SystemTime::now(),
        "banned_until should be in the future"
    );

    db.unban_account(acc.id).await.expect("unban_account");
    let unbanned = db
        .find_account_from_username("cake")
        .await
        .unwrap()
        .unwrap();
    assert!(
        unbanned.banned_until <= SystemTime::now(),
        "banned_until should be cleared"
    );
}

pub async fn test_player_init_load<D: DbImpl>(db: &Database<D>) {
    let acc = db.create_account("dong", "h").await.unwrap();
    let uid: i64 = 1001;
    let player = make_player(uid, 1);

    db.init_player(acc.id, &player).await.expect("init_player");

    // load by UID
    let loaded = db
        .load_player(acc.id, uid)
        .await
        .expect("load_player")
        .expect("player exists");
    assert_eq!(loaded.get_uid(), uid);
    assert_eq!(loaded.get_slot_num(), 1);
    assert_eq!(loaded.first_name, "Test1001");

    // load all players for account
    let all = db.load_players(acc.id).await.expect("load_players");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].get_uid(), uid);

    // find_account_from_player
    let acc2 = db
        .find_account_from_player(uid)
        .await
        .expect("find_account_from_player")
        .expect("account exists");
    assert_eq!(acc2.id, acc.id);
}

pub async fn test_player_save_reload<D: DbImpl>(db: &Database<D>) {
    let acc = db.create_account("cpunch", "h").await.unwrap();
    let uid: i64 = 2002;
    let player = make_player(uid, 2);
    db.init_player(acc.id, &player).await.unwrap();

    // mutate then save
    let mut p = db.load_player(acc.id, uid).await.unwrap().unwrap();
    p.set_taros(12345);
    db.save_player(&p).await.expect("save_player");

    let reloaded = db.load_player(acc.id, uid).await.unwrap().unwrap();
    assert_eq!(
        reloaded.get_taros(),
        12345,
        "taros should persist across save/load"
    );
}

pub async fn test_save_players_batch<D: DbImpl>(db: &Database<D>) {
    let acc = db.create_account("sane", "h").await.unwrap();
    let mut players = Vec::new();
    for i in 0..3 {
        let p = make_player(3000 + i as i64, i);
        db.init_player(acc.id, &p).await.unwrap();
        players.push(p);
    }
    // load + mutate + batch-save
    let mut loaded: Vec<Player> = Vec::new();
    for p in &players {
        loaded.push(db.load_player(acc.id, p.get_uid()).await.unwrap().unwrap());
    }
    for (i, p) in loaded.iter_mut().enumerate() {
        p.set_taros(100 * (i as u32 + 1));
    }
    let refs: Vec<&Player> = loaded.iter().collect();
    db.save_players(&refs).await.expect("save_players");

    for (i, p) in players.iter().enumerate() {
        let r = db.load_player(acc.id, p.get_uid()).await.unwrap().unwrap();
        assert_eq!(r.get_taros(), 100 * (i as u32 + 1));
    }
}

pub async fn test_player_appearance<D: DbImpl>(db: &Database<D>) {
    use crate::entity::PlayerStyle;
    let acc = db.create_account("kevman", "h").await.unwrap();
    let uid: i64 = 4004;
    let p = make_player(uid, 0);
    db.init_player(acc.id, &p).await.unwrap();

    let mut p = db.load_player(acc.id, uid).await.unwrap().unwrap();
    p.style = Some(PlayerStyle {
        gender: 1,
        face_style: 2,
        hair_style: 3,
        hair_color: 4,
        skin_color: 5,
        eye_color: 6,
        height: 7,
        body: 8,
    });
    db.update_player_appearance(&p)
        .await
        .expect("update_player_appearance");

    let r = db.load_player(acc.id, uid).await.unwrap().unwrap();
    let s = r
        .style
        .expect("style should be set after appearance update");
    assert_eq!(s.gender, 1);
    assert_eq!(s.face_style, 2);
    assert_eq!(s.body, 8);
}

pub async fn test_player_selected<D: DbImpl>(db: &Database<D>) {
    let acc = db.create_account("finn", "h").await.unwrap();
    let uid: i64 = 5005;
    let p = make_player(uid, 3);
    db.init_player(acc.id, &p).await.unwrap();

    db.update_selected_player(acc.id, 3)
        .await
        .expect("update_selected_player");

    let after = db
        .find_account_from_username("finn")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.selected_slot, 3);
}

pub async fn test_player_delete<D: DbImpl>(db: &Database<D>) {
    let acc = db.create_account("jade", "h").await.unwrap();
    let uid: i64 = 6006;
    let p = make_player(uid, 0);
    db.init_player(acc.id, &p).await.unwrap();
    assert!(db.load_player(acc.id, uid).await.unwrap().is_some());

    db.delete_player(uid).await.expect("delete_player");

    let after = db.load_player(acc.id, uid).await.unwrap();
    assert!(after.is_none(), "player should be gone after delete");
}

pub async fn test_email_round_trip<D: DbImpl>(db: &Database<D>) {
    use crate::email::Email;
    use crate::enums::ItemType;
    use crate::item::Item;

    let acc = db.create_account("mailman", "h").await.unwrap();
    let sender_uid: i64 = 7007;
    let recipient_uid: i64 = 7008;
    let sender = make_player(sender_uid, 0);
    let recipient = make_player(recipient_uid, 1);
    db.init_player(acc.id, &sender).await.unwrap();
    db.init_player(acc.id, &recipient).await.unwrap();

    // nothing in the inbox yet
    assert_eq!(
        db.get_unread_email_count(recipient_uid).await.unwrap(),
        0,
        "a fresh inbox should be empty"
    );

    let mut email = Email::new(&sender, recipient_uid);
    email.subject = "Hey".to_string();
    email.body = "Line one\nLine two".to_string();
    email.taros = 500;
    let mut attachment = Item::new(ItemType::General, 119);
    attachment.quantity = 3;
    email.attachments[0] = Some(attachment);

    let msg_index = db.send_email(&email).await.expect("send_email");
    assert_eq!(msg_index, 1, "the first email should get index 1");
    assert_eq!(
        db.get_unread_email_count(recipient_uid).await.unwrap(),
        1,
        "the new email should be unread"
    );

    // the sender's own inbox is untouched
    assert_eq!(db.get_unread_email_count(sender_uid).await.unwrap(), 0);

    let loaded = db
        .load_email(recipient_uid, msg_index)
        .await
        .unwrap()
        .expect("email exists");
    assert_eq!(loaded.subject, "Hey");
    assert_eq!(loaded.body, "Line one\nLine two");
    assert_eq!(loaded.taros, 500);
    assert_eq!(loaded.sender_uid, sender_uid);
    assert!(!loaded.read);
    let stored_attachment = loaded.attachments[0].expect("attachment should round-trip");
    assert_eq!(stored_attachment.id, 119);
    assert_eq!(stored_attachment.quantity, 3);

    // the page listing should show it too
    let page = db.load_emails(recipient_uid, 1).await.unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].msg_index, msg_index);
    assert!(page[0].has_attachments());

    // claim the goodies and mark it read
    let mut claimed = loaded;
    claimed.read = true;
    claimed.taros = 0;
    claimed.attachments[0] = None;
    db.update_email(&claimed).await.expect("update_email");

    let after = db
        .load_email(recipient_uid, msg_index)
        .await
        .unwrap()
        .unwrap();
    assert!(after.read, "read flag should persist");
    assert_eq!(after.taros, 0);
    assert!(
        after.attachments.iter().all(|item| item.is_none()),
        "claimed attachments should be gone"
    );
    assert!(
        !after.has_attachments(),
        "the paperclip should clear once everything is claimed"
    );
    assert_eq!(db.get_unread_email_count(recipient_uid).await.unwrap(), 0);

    // indices keep counting up
    let second_index = db.send_email(&email).await.unwrap();
    assert_eq!(second_index, 2);

    db.delete_emails(recipient_uid, &[msg_index as i64, 0, 0, 0, 0])
        .await
        .expect("delete_emails");
    assert!(db
        .load_email(recipient_uid, msg_index)
        .await
        .unwrap()
        .is_none());
    assert!(
        db.load_email(recipient_uid, second_index)
            .await
            .unwrap()
            .is_some(),
        "deleting one email shouldn't touch the others"
    );
}

pub async fn test_race_results<D: DbImpl>(db: &Database<D>) {
    use crate::racing::RaceResult;

    let acc = db.create_account("racer", "h").await.unwrap();
    let uid: i64 = 8008;
    let p = make_player(uid, 0);
    db.init_player(acc.id, &p).await.unwrap();

    const EP_ID: i32 = 1;
    assert!(
        db.load_best_race_result(EP_ID, uid)
            .await
            .unwrap()
            .is_none(),
        "no runs yet"
    );

    let slower = RaceResult {
        ep_id: EP_ID,
        pc_uid: uid,
        score: 100,
        num_pods: 5,
        time_s: 90,
        timestamp: 1000,
    };
    let faster = RaceResult {
        score: 400,
        num_pods: 12,
        time_s: 60,
        timestamp: 2000,
        ..slower
    };
    db.save_race_result(&slower)
        .await
        .expect("save_race_result");
    db.save_race_result(&faster)
        .await
        .expect("save_race_result");

    let best = db
        .load_best_race_result(EP_ID, uid)
        .await
        .unwrap()
        .expect("a best run exists");
    assert_eq!(best.score, 400, "the best run should win on score");
    assert_eq!(best.num_pods, 12);
    assert_eq!(best.time_s, 60);

    // results are scoped per zone
    assert!(
        db.load_best_race_result(EP_ID + 1, uid)
            .await
            .unwrap()
            .is_none(),
        "another zone shouldn't see this run"
    );
}
