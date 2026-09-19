use super::{sqlite::SqliteDatabase, test_suite, Database, DbImpl};
use crate::{
    entity::{Combatant, Player},
    tabledata::tdata_get,
};

#[tokio::test]
async fn completed_nano_survives_reload_and_database_reopen() {
    test_suite::ensure_init();
    std::fs::create_dir_all("target/t03").unwrap();
    let path = format!("target/t03/nano-{}.db", uuid::Uuid::new_v4());
    let cfg = test_suite::build_config(&path);
    let db = Database::new(SqliteDatabase::connect(&cfg.general).await.unwrap());
    let account = db
        .create_account("nano_persistence", "unused-test-hash")
        .await
        .unwrap();
    let mut player = Player::new(303, 0);
    db.init_player(account.id, &player).await.unwrap();
    player.set_level(6).unwrap();
    player.set_fusion_matter(17);
    player.unlock_nano(6).unwrap();
    player.mission_journal.set_mission_completed(538).unwrap();
    db.save_player(&player).await.unwrap();
    let loaded = db.load_player(account.id, 303).await.unwrap().unwrap();
    assert_eq!((loaded.get_level(), loaded.get_fusion_matter()), (6, 17));
    assert!(loaded.get_nano(6).is_some());
    assert!(loaded.mission_journal.is_mission_completed(538).unwrap());
    drop(db);

    // A stale row from the old auto-start bug must not resurrect a completed
    // mission, even if a GM later returns the character to its previous level.
    let raw = deadpool_sqlite::rusqlite::Connection::open(&path).unwrap();
    raw.execute("UPDATE Players SET Level = 5 WHERE PlayerID = 303", [])
        .unwrap();
    raw.execute("INSERT INTO RunningQuests VALUES (303, 827, 0, 0, 0)", [])
        .unwrap();
    drop(raw);
    let reopened = Database::new(SqliteDatabase::connect(&cfg.general).await.unwrap());
    let mut loaded = reopened
        .load_player(account.id, 303)
        .await
        .unwrap()
        .unwrap();
    assert!(loaded.mission_journal.is_mission_completed(538).unwrap());
    assert!(!loaded.mission_journal.has_nano_mission());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    loaded.set_client(crate::net::FFClient::new(
        tx,
        crate::net::ClientMetadata::new("127.0.0.1:39003".parse().unwrap(), None),
    ));
    let stats = tdata_get().get_player_stats(5).unwrap();
    loaded.set_fusion_matter(stats.req_fm_nano_create + 10);
    assert!(!loaded.mission_journal.has_nano_mission());
    assert!(rx.try_recv().is_err());
    assert!(loaded.get_nano(6).is_some());
}

#[tokio::test]
async fn retuned_skill_survives_reload_when_tune_differs_from_skill() {
    test_suite::ensure_init();
    std::fs::create_dir_all("target/t04").unwrap();
    let path = format!("target/t04/nano-tune-{}.db", uuid::Uuid::new_v4());
    let cfg = test_suite::build_config(&path);
    let db = Database::new(SqliteDatabase::connect(&cfg.general).await.unwrap());
    let account = db
        .create_account("nano_tune_persistence", "unused-test-hash")
        .await
        .unwrap();
    let mut player = Player::new(304, 0);
    db.init_player(account.id, &player).await.unwrap();
    // Nano 38 tune 201 grants skill 13; the skill, not the tune, is stored
    player.unlock_nano(38).unwrap();
    player.tune_nano(38, Some(13)).unwrap();
    db.save_player(&player).await.unwrap();
    drop(db);

    let reopened = Database::new(SqliteDatabase::connect(&cfg.general).await.unwrap());
    let loaded = reopened
        .load_player(account.id, 304)
        .await
        .unwrap()
        .unwrap();
    let nano = loaded.get_nano(38).unwrap();
    assert_eq!(nano.selected_skill, Some(13));
    assert!(nano.get_skill().is_some(), "stored skill resolves for use");
}
