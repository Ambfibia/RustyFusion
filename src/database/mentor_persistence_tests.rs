use super::{open_test_sqlite, test_suite, DbImpl};
use crate::{entity::Player, enums::PlayerGuide};
use deadpool_sqlite::rusqlite::Connection;

#[tokio::test]
async fn mentor_count_migrates_legacy_players_and_survives_reopen() {
    test_suite::ensure_init();
    std::fs::create_dir_all("target/mentor-persistence-tests").unwrap();
    let path = format!(
        "target/mentor-persistence-tests/{}.db",
        uuid::Uuid::new_v4()
    );
    let db = open_test_sqlite(&path).await;
    let account = db
        .create_account("mentor_persistence", "unused-test-hash")
        .await
        .unwrap();
    let mut chosen = Player::new(303, 0);
    chosen.first_name = "Chosen".into();
    let mut unchosen = Player::new(304, 1);
    unchosen.first_name = "Unchosen".into();
    db.init_player(account.id, &chosen).await.unwrap();
    db.init_player(account.id, &unchosen).await.unwrap();
    drop(db);

    // Recreate the pre-migration Players shape with two kinds of legacy rows.
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "ALTER TABLE Players DROP COLUMN MentorCount;
             UPDATE Players SET Level = 4, Mentor = 2, FusionMatter = 2000 WHERE PlayerID = 303;",
        )
        .unwrap();
    drop(connection);

    let db = open_test_sqlite(&path).await;
    let mut chosen = db.load_player(account.id, 303).await.unwrap().unwrap();
    let unchosen = db.load_player(account.id, 304).await.unwrap().unwrap();
    assert_eq!(
        (chosen.get_guide(), chosen.get_guide_count()),
        (PlayerGuide::Dexter, 1)
    );
    assert_eq!(
        (unchosen.get_guide(), unchosen.get_guide_count()),
        (PlayerGuide::Computress, 0)
    );
    assert_eq!(chosen.update_guide(PlayerGuide::Ben), 2);
    db.save_player(&chosen).await.unwrap();
    drop(db);

    let db = open_test_sqlite(&path).await;
    let roster = db.load_players(account.id).await.unwrap();
    let chosen = roster.iter().find(|row| row.get_uid() == 303).unwrap();
    assert_eq!(
        (chosen.get_guide(), chosen.get_guide_count()),
        (PlayerGuide::Ben, 2)
    );
    let load_data = chosen.get_load_data();
    let (mentor, count, matter) = (
        load_data.iMentor,
        load_data.iMentorCount,
        load_data.iFusionMatter,
    );
    assert_eq!((mentor, count, matter), (PlayerGuide::Ben as i16, 2, 2000));
    let unchosen = roster.iter().find(|row| row.get_uid() == 304).unwrap();
    assert_eq!(
        (unchosen.get_guide(), unchosen.get_guide_count()),
        (PlayerGuide::Computress, 0)
    );
}
