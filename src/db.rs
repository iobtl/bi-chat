use std::path::Path;

use rusqlite::{params, Connection, DropBehavior};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::shutdown::Shutdown;

pub type DbTx = UnboundedSender<DBMessage>;
pub type DbRx = UnboundedReceiver<DBMessage>;

#[derive(Debug)]
pub struct DBMessage {
    pub user_id: usize,
    pub room_name: String,
    pub message: String,
}

impl DBMessage {
    pub fn new(user_id: usize, room_name: &str, message: &str) -> Self {
        DBMessage {
            user_id,
            room_name: String::from(room_name),
            message: String::from(message),
        }
    }
}

pub fn spawn_db(
    db_path: &Path,
    mut db_rx: DbRx,
    mut shutdown: Shutdown,
) -> Result<(), rusqlite::Error> {
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

    let insert_query =
        "INSERT INTO chat_messages (user_id, room_name, message) VALUES (?1, ?2, ?3)";
    let mut tx = conn.transaction()?;
    tx.set_drop_behavior(DropBehavior::Commit);

    let mut stmt = tx.prepare_cached(insert_query)?;

    // While shutdown signal not received, keep listening for messages.
    while !shutdown.is_shutdown() {
        // Update shutdown state
        shutdown.listen();
        // If shutdown signal has been received, finish processing remaining
        // messages.
        // Else, continue listening for messages on `db_rx`.
        if shutdown.is_shutdown() {
            while let Ok(msg) = db_rx.try_recv() {
                stmt.execute(params![msg.user_id, msg.room_name, msg.message])?;
            }

            break;
        } else if let Ok(msg) = db_rx.try_recv() {
            stmt.execute(params![msg.user_id, msg.room_name, msg.message])?;
        }
    }

    eprintln!("Shutdown signal received: closing DB connection");
    drop(stmt);
    tx.commit()?;
    conn.close().expect("Failed to close DB connection");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::{broadcast, mpsc};

    #[test]
    fn test_db_connection() {
        let (_, db_rx) = mpsc::unbounded_channel();
        let (notify_shutdown, _) = broadcast::channel(1);
        let (shutdown_complete_tx, _) = mpsc::channel(1);

        let shutdown_listener = notify_shutdown.subscribe();

        let db_path = Path::new("./test.db");
        let db_conn = std::thread::spawn(move || {
            spawn_db(
                db_path,
                db_rx,
                Shutdown::new(shutdown_listener, shutdown_complete_tx),
            )
        });

        // Shutdown the connection and return
        drop(notify_shutdown);

        // Get return value from DB handle to ensure no errors during DB
        // operations.
        db_conn.join().unwrap().unwrap();

        std::fs::remove_file(db_path).unwrap();
    }
}
