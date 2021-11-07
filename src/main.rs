pub mod db;
pub mod html;

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use db::{spawn_db, ChatMessage};
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use html::INDEX_HTML;
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    RwLock,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::{
    ws::{Message, WebSocket, Ws},
    Filter,
};

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);
// static NEXT_ROOM_ID: AtomicUsize = AtomicUsize::new(1);

type Users = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;
type Rooms = Arc<RwLock<HashMap<String, Users>>>;

// Move filters to another module
fn chat() -> impl Filter<Extract = (Ws, String), Error = warp::Rejection> + Copy {
    warp::path("chat")
        .and(warp::ws())
        .and(warp::path::param::<String>())
    // .map(|ws: Ws, rooms, chat_room| {
    //     ws.on_upgrade(move |socket| user_connected(socket, rooms, chat_room))
    // })
}

fn index(
) -> impl Filter<Extract = (warp::reply::Html<&'static str>,), Error = warp::Rejection> + Copy {
    warp::path::end().map(|| warp::reply::html(INDEX_HTML))
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Spawning of a dedicated task to handle DB writes
    let (db_tx, db_rx): (UnboundedSender<ChatMessage>, UnboundedReceiver<ChatMessage>) =
        mpsc::unbounded_channel();
    let mut db_rx = UnboundedReceiverStream::new(db_rx);
    let db_manager = spawn_db(db_rx).await;

    // We maintain the hierarchy that each `Room` contains `Users`
    // Any message that is sent to a `Room` should be broadcast to all `Users`.
    let rooms = Rooms::default();
    let rooms = warp::any().map(move || rooms.clone());

    let db_tx = warp::any().map(move || db_tx.clone());

    let chat = chat()
        .and(rooms)
        .and(db_tx)
        .map(|ws: Ws, chat_room, rooms, db_tx| {
            // Handle Ws message size here
            ws.on_upgrade(move |socket| user_connected(socket, chat_room, rooms, db_tx))
        });
    // let chat = chat(rooms);

    let index = index();

    let routes = index.or(chat);

    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;

    Ok(())
}

async fn user_connected(
    ws: WebSocket,
    chat_room: String,
    rooms: Rooms,
    db_tx: UnboundedSender<ChatMessage>,
) {
    let user_id = NEXT_USER_ID.fetch_add(1, Ordering::Relaxed);
    // let room_id = NEXT_ROOM_ID.fetch_add(1, Ordering::Relaxed);
    println!("Joining room: {}", &chat_room);

    let (mut user_ws_tx, mut user_ws_rx) = ws.split();

    // Create unbounded channel to handle buffering and consuming of messages
    let (tx, rx) = mpsc::unbounded_channel();
    let mut rx = UnboundedReceiverStream::new(rx);

    // Dedicated thread to listen and buffer incoming messages using Tokio channels
    // Then feeds into WS sink -> WS stream (to be consumed and displayed)
    tokio::task::spawn(async move {
        while let Some(message) = rx.next().await {
            user_ws_tx
                .send(message)
                .unwrap_or_else(|e| {
                    eprintln!("WebSocket send error: {}", e);
                })
                .await;
        }
    });

    // TODO: relook this part -- calling await with lock held!!!
    let users = rooms
        // Need write
        .write()
        .await
        .get(&chat_room)
        .map(|u| u.clone())
        .unwrap_or_else(|| Users::default());
    users.write().await.insert(user_id, tx);
    rooms.write().await.insert(chat_room.clone(), users.clone());

    // "Broadcasting" message sent by this user to all other users in the same room
    while let Some(result) = user_ws_rx.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("Websocket error(uid={}): {}", user_id, e);
                break;
            }
        };
        user_message(user_id, msg, &rooms, &chat_room, db_tx.clone())
            .await
            .unwrap();
    }

    // TODO: is the lock on rooms held until here?? bad
    user_disconnected(user_id, &users).await;
}

// Decouple from DB stuff -- look at channels
async fn user_message(
    user_id: usize,
    msg: Message,
    rooms: &Rooms,
    chat_room: &str,
    db_tx: UnboundedSender<ChatMessage>,
) -> Result<(), anyhow::Error> {
    let msg = if let Ok(s) = msg.to_str() {
        s
    } else {
        return Ok(());
    };

    let new_msg = format!("<User#{}>: {}", user_id, msg);
    db_tx.send(ChatMessage::new(
        user_id,
        chat_room.to_string(),
        new_msg.clone(),
    ))?;

    let users = rooms
        .read()
        .await
        .get(chat_room)
        .map(|u| u.clone())
        .unwrap_or_else(|| Users::default());
    // TODO: possible bottleneck? Blocks adding of a user to a room. Possible tradeoffs?
    for (&uid, tx) in users.read().await.iter() {
        if user_id != uid {
            if let Err(_disconnected) = tx.send(Message::text(&new_msg)) {}
        }
    }

    Ok(())
}

async fn user_disconnected(user_id: usize, users: &Users) {
    eprintln!("User disconnected: {}", user_id);

    users.write().await.remove(&user_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future;
    use warp::test;

    #[tokio::test]
    async fn test_html_connection() -> Result<(), warp::Rejection> {
        let index = index();

        let value = test::request().filter(&index).await?;
        // assert_eq!(String::from(value), INDEX_HTML);

        Ok(())
    }

    // #[tokio::test]
    // async fn test_ws_connection() {
    //     let chat = chat().map(|ws: Ws, _| ws.on_upgrade(|_| future::ready(())));

    //     test::ws()
    //         .path("/chat")
    //         .handshake(chat)
    //         .await
    //         .expect("Handshake failed");
    // }
}
