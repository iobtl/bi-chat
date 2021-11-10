use std::path::PathBuf;

use bi_chat::server;
use futures::{FutureExt, SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
async fn same_room_users() {
    const PORT: u16 = 3030;

    let db_path = PathBuf::from("./main_same_room.db");
    let spawn_db_path = db_path.clone();
    tokio::task::spawn(async move {
        server::run(PORT, spawn_db_path).await;
    });

    let uri = format!("ws://localhost:{}/chat/room1", PORT);

    let res = tokio::try_join!(connect_async(&uri), connect_async(&uri));

    let (mut stream1, mut stream2) = match res {
        Ok(((stream1, _), (stream2, _))) => (stream1, stream2),
        Err(_) => panic!("Unable to connect to WS uri: {}", uri),
    };

    let msg_text = String::from("Hello from the other side");
    let msg = Message::Text(msg_text.clone());
    stream1
        .send(msg.clone())
        .await
        .expect("Unable to send message");

    let received_msg = stream2.next().await.expect("No value found!").unwrap();
    let received_msg_text = received_msg.into_text().unwrap();
    let extracted_msg_text = received_msg_text.split(":").last().unwrap().trim();

    assert_eq!(msg_text, extracted_msg_text);

    std::fs::remove_file(&db_path).expect(&format!(
        "Failed to remove test db file: {}",
        &db_path.to_str().unwrap()
    ));
}

#[tokio::test]
// Tests that users in different rooms do not receive messages from each other.
async fn different_room_users() {
    const PORT: u16 = 3031;

    let db_path = PathBuf::from("./main_different_room.db");
    let spawn_db_path = db_path.clone();
    tokio::task::spawn(async move {
        server::run(PORT, spawn_db_path).await;
    });

    let uri1 = format!("ws://localhost:{}/chat/room1", PORT);
    let uri2 = format!("ws://localhost:{}/chat/room2", PORT);

    let res = tokio::try_join!(connect_async(&uri1), connect_async(&uri2));

    let (mut stream1, mut stream2) = match res {
        Ok(((stream1, _), (stream2, _))) => (stream1, stream2),
        Err(_) => panic!("Unable to establish WS connection"),
    };

    let msg_text1 = String::from("Hello from the other side");
    let msg1 = Message::Text(msg_text1.clone());
    stream1
        .send(msg1.clone())
        .await
        .expect("Unable to send message");

    let msg_text2 = String::from("Hello from the other side");
    let msg2 = Message::Text(msg_text2.clone());
    stream2
        .send(msg2.clone())
        .await
        .expect("Unable to send message");

    assert!(stream1.next().now_or_never().is_none());
    assert!(stream2.next().now_or_never().is_none());

    std::fs::remove_file(&db_path).expect(&format!(
        "Failed to remove test db file: {}",
        &db_path.to_str().unwrap()
    ));
}
