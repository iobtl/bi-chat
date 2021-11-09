use std::path::Path;

use rusqlite::{params, Connection};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub type DbTx = UnboundedSender<DBMessage>;
pub type DbRx = UnboundedReceiver<DBMessage>;

#[derive(Debug)]
pub struct DBMessage {
    pub user_id: usize,
    pub room_name: String,
    pub message: String,
}

impl DBMessage {
    pub fn new(user_id: usize, room_name: String, message: String) -> Self {
        DBMessage {
            user_id,
            room_name,
            message,
        }
    }
}

pub fn spawn_db(db_path: &'static Path, mut db_rx: DbRx) -> Result<(), rusqlite::Error> {
    let mut conn =
        Connection::open(db_path).expect("Unable to establish connection to DB. Exiting");

    conn.execute(
        "CREATE TABLE IF NOT EXISTS chat_messages (
                message_id INTEGER PRIMARY KEY AUTOINCREMENT NOT NULL,
                user_id INTEGER,
                room_name TEXT NOT NULL,
                message TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP NOT NULL
            )",
        [],
    )?;

    let tx = conn.transaction()?;
    {
        let insert_query =
            "INSERT INTO chat_messages (user_id, room_name, message) VALUES (?1, ?2, ?3)";
        let mut stmt = tx.prepare_cached(insert_query)?;
        while let Some(msg) = db_rx.blocking_recv() {
            // TODO: Batch inserts as an improvement?
            stmt.execute(params![msg.user_id, msg.room_name, msg.message])?;
        }
    }

    tx.commit()?;

    Ok(())
}
