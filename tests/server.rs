use std::time::Duration;

use bi_chat::server;
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::test]
async fn same_room_users() {
    tokio::task::spawn(async move {
        server::run(3030).await;
    });

    let uri = "ws://localhost:3030/chat/room1";

    let res = tokio::try_join!(connect_async(uri), connect_async(uri));

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
}
