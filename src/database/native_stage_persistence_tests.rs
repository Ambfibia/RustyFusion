//! Shared by the isolated authored-stage packet fixture.
use super::{sqlite::SqliteDatabase, test_suite, Database, DbImpl};
use crate::entity::Player;
pub(crate) async fn round_trip(player: &Player) -> Player {
    std::fs::create_dir_all("target/mission-stage-tests").unwrap();
    let path = format!(
        "target/mission-stage-tests/state-{}.db",
        uuid::Uuid::new_v4()
    );
    let cfg = test_suite::build_config(&path);
    let db = Database::new(SqliteDatabase::connect(&cfg.general).await.unwrap());
    let account = db
        .create_account("native_stage", "unused-test-hash")
        .await
        .unwrap();
    db.init_player(account.id, player).await.unwrap();
    db.save_player(player).await.unwrap();
    drop(db);
    let reopened = Database::new(SqliteDatabase::connect(&cfg.general).await.unwrap());
    reopened
        .load_player(account.id, player.get_uid())
        .await
        .unwrap()
        .unwrap()
}
