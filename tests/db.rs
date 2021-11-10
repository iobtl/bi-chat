use std::path::Path;

use bi_chat::{
    self,
    db::{spawn_db, DBMessage},
};

use rusqlite::Connection;
use tokio::sync::mpsc;

#[test]
// Mainly tests that we can perform a proper insertion into the DB
fn test_db_single_insert() {
    let db_path = Path::new("./test_single.db");
    if db_path.exists() {
        std::fs::remove_file(db_path).unwrap();
    }
    let (db_tx, db_rx) = mpsc::unbounded_channel();

    let db_handle = std::thread::spawn(move || spawn_db(db_path, db_rx));

    let user_id = 1;
    let room_name = String::from("TestRoom");
    let message = String::from("Hello there");
    let chat_message = DBMessage::new(user_id, &room_name, &message);
    db_tx
        .send(chat_message)
        .expect("Failed to send message to Receiver!");

    drop(db_tx);

    db_handle.join().unwrap().unwrap();

    // Establish another connection to check if rows are properly inserted
    let conn = Connection::open(&db_path).expect("Unable to establish connection to DB.");
    let mut stmt = conn
        .prepare("SELECT user_id, room_name, message FROM chat_messages")
        .expect("Failed preparing SQL statement.");

    let returned_msg = stmt
        .query_map([], |row| {
            Ok(DBMessage {
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

    std::fs::remove_file(db_path).unwrap();
}

#[test]
// Mainly tests that DB can handle multiple  requests
fn test_db_multiple_inserts() {
    const TOTAL_ROWS: usize = 1_000_000;

    let db_path = Path::new("./test_multiple.db");
    if db_path.exists() {
        std::fs::remove_file(db_path).unwrap();
    }
    let (db_tx, db_rx) = tokio::sync::mpsc::unbounded_channel();

    let db_handle = std::thread::spawn(move || spawn_db(db_path, db_rx));

    let user_id = 1;
    let room_name = String::from("TestRoom");
    let message = String::from("Hello there");

    for _ in 0..TOTAL_ROWS {
        let tx = db_tx.clone();
        tx.send(DBMessage::new(user_id, &room_name, &message))
            .expect("Receiver disconnected!");
    }

    drop(db_tx);

    db_handle.join().unwrap().unwrap();

    // Establish another connection to check if rows are properly inserted
    let conn = Connection::open(&db_path).expect("Unable to establish connection to DB.");
    let mut stmt = conn
        .prepare("SELECT user_id, room_name, message FROM chat_messages")
        .unwrap();

    let rows = stmt
        .query_map([], |row| {
            Ok(DBMessage {
                user_id: row.get(0).expect("user_id not found!"),
                room_name: row.get(1).expect("room_name not found!"),
                message: row.get(2).expect("message not found!"),
            })
        })
        .expect("Query failed")
        .map(|row| row.unwrap())
        .collect::<Vec<DBMessage>>();

    assert_eq!(rows.len(), TOTAL_ROWS);

    std::fs::remove_file(db_path).unwrap();
}

#[test]
// Mainly tests that DB can handle many requests at once -- also for channel bottleneck
fn test_db_parallel_inserts() {
    use rayon::prelude::*;

    const TOTAL_ROWS: usize = 1_000_000;

    let db_path = Path::new("./test_parallel.db");
    if db_path.exists() {
        std::fs::remove_file(db_path).unwrap();
    }
    let (db_tx, db_rx) = tokio::sync::mpsc::unbounded_channel();

    let db_handle = std::thread::spawn(move || spawn_db(db_path, db_rx));

    let user_id = 1;
    let room_name = String::from("TestRoom");
    let message = String::from("Hello there");

    // Simulate many requests at once
    (0..TOTAL_ROWS).into_par_iter().for_each(|_| {
        let tx = db_tx.clone();
        tx.send(DBMessage::new(user_id, &room_name, &message))
            .expect("Receiver disconnected!");
    });

    drop(db_tx);

    db_handle.join().unwrap().unwrap();

    // Establish another connection to check if rows are properly inserted
    let conn = Connection::open(&db_path).expect("Unable to establish connection to DB.");
    let mut stmt = conn
        .prepare("SELECT user_id, room_name, message FROM chat_messages")
        .unwrap();

    let rows = stmt
        .query_map([], |row| {
            Ok(DBMessage {
                user_id: row.get(0).expect("user_id not found!"),
                room_name: row.get(1).expect("room_name not found!"),
                message: row.get(2).expect("message not found!"),
            })
        })
        .expect("Query failed")
        .map(|row| row.unwrap())
        .collect::<Vec<DBMessage>>();

    assert_eq!(rows.len(), TOTAL_ROWS);

    std::fs::remove_file(db_path).unwrap();
}
