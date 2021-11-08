use std::path::Path;

use bi_chat::{
    self,
    db::{spawn_db, ChatMessage},
};

use rusqlite::Connection;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;

#[tokio::test]
// Mainly tests that we can perform a proper insertion into the DB
async fn test_db_single_insert() {
    let (db_tx, db_rx): (UnboundedSender<ChatMessage>, UnboundedReceiver<ChatMessage>) =
        mpsc::unbounded_channel();
    let db_rx = UnboundedReceiverStream::new(db_rx);
    let db_path = Path::new("./test_single.db");
    if db_path.exists() {
        std::fs::remove_file(&db_path).unwrap();
    }

    let db_conn = spawn_db(&Path::new("./test_single.db"), db_rx);

    let user_id = 1;
    let room_name = String::from("TestRoom");
    let message = String::from("Hello there");
    let chat_message = ChatMessage::new(user_id, room_name.clone(), message.clone());
    db_tx
        .send(chat_message)
        .expect("Failed to send message to Receiver!");

    drop(db_tx);

    db_conn.await.expect("Error joining threads").unwrap();

    // Establish another connection to check if rows are properly inserted
    let conn = Connection::open(&db_path).expect("Unable to establish connection to DB.");
    let mut stmt = conn
        .prepare("SELECT user_id, room_name, message FROM chat_messages")
        .expect("Failed preparing SQL statement.");

    let returned_msg = stmt
        .query_map([], |row| {
            Ok(ChatMessage {
                user_id: row.get(0).expect("user_id not found!"),
                room_name: row.get(1).expect("room_name not found!"),
                message: row.get(2).expect("message not found!"),
            })
        })
        .expect("Query failed")
        .next()
        .expect("No message returned");

    assert!(returned_msg.is_ok());

    let returned_msg = returned_msg.unwrap();
    assert_eq!(returned_msg.user_id, user_id);
    assert_eq!(returned_msg.room_name, room_name);
    assert_eq!(returned_msg.message, message);
}
