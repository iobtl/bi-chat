use bi_chat::db;
use bi_chat::html::INDEX_HTML;

use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use db::{spawn_db, ChatMessage};
use futures_util::{SinkExt, StreamExt, TryFutureExt};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::RwLock;
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::{
    ws::{Message, WebSocket, Ws},
    Filter,
};

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);
const MAIN_DB_PATH: &str = "./main.db";

type Users = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;
type Rooms = Arc<RwLock<HashMap<String, Users>>>;

// Move filters to another module
fn chat() -> impl Filter<Extract = (Ws, String), Error = warp::Rejection> + Copy {
    warp::path("chat")
        .and(warp::ws())
        .and(warp::path::param::<String>())
}

fn index(
) -> impl Filter<Extract = (warp::reply::Html<&'static str>,), Error = warp::Rejection> + Copy {
    warp::path::end().map(|| warp::reply::html(INDEX_HTML))
}

struct User {
    user_id: usize,
    chat_room: String,
    tx: UnboundedSender<Message>,
    db_tx: UnboundedSender<ChatMessage>,
}

impl User {
    pub fn new(
        user_id: usize,
        chat_room: String,
        tx: UnboundedSender<Message>,
        db_tx: UnboundedSender<ChatMessage>,
    ) -> Self {
        User {
            user_id,
            chat_room,
            tx,
            db_tx,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Spawning of a dedicated task to handle DB writes
    let (db_tx, db_rx) = mpsc::unbounded_channel();
    let db_path = Path::new(MAIN_DB_PATH);
    let db_handle = std::thread::spawn(move || spawn_db(db_path, db_rx));

    // We maintain the hierarchy that each `Room` contains `Users`
    // Any message that is sent to a `Room` should be broadcast to all `Users`.
    let rooms = Rooms::default();
    let rooms = warp::any().map(move || rooms.clone());

    let db_tx = warp::any().map(move || db_tx.clone());

    let chat = chat()
        .and(db_tx)
        .and(rooms)
        .map(|ws: Ws, chat_room, db_tx, rooms| {
            ws.on_upgrade(move |socket| user_connected(socket, chat_room, db_tx, rooms))
        });

    let index = index();

    let routes = index.or(chat);

    println!("Serving!");
    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;

    db_handle.join().expect("Failed to join on DB thread")?;

    Ok(())
}

async fn add_user_to_room(new_user: &User, rooms: &Rooms) {
    let mut room = rooms.write().await;
    let users = room
        .entry(new_user.chat_room.clone())
        .or_insert(Users::default());
    users
        .write()
        .await
        .insert(new_user.user_id, new_user.tx.clone());
}

async fn remove_user_from_room(user: &User, rooms: &Rooms) {
    let mut room = rooms.write().await;
    let users = room
        .entry(user.chat_room.clone())
        .or_insert(Users::default());

    users.write().await.remove(&user.user_id);

    // Cleans up empty room
    if users.read().await.is_empty() {
        rooms.write().await.remove(&user.chat_room);
    }
}

async fn user_connected(
    ws: WebSocket,
    chat_room: String,
    db_tx: UnboundedSender<ChatMessage>,
    rooms: Rooms,
) {
    println!("Joining room: {}", &chat_room);

    let (mut user_ws_tx, mut user_ws_rx) = ws.split();

    // Create unbounded channel to handle buffering and consuming of messages
    let (tx, rx) = mpsc::unbounded_channel();
    let mut rx = UnboundedReceiverStream::new(rx);

    let user_id = NEXT_USER_ID.fetch_add(1, Ordering::Relaxed);
    let new_user = User::new(user_id, chat_room, tx, db_tx);
    add_user_to_room(&new_user, &rooms).await;

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

    // "Broadcasting" message sent by this user to all other users in the same room
    while let Some(result) = user_ws_rx.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("Websocket error(uid={}): {}", user_id, e);
                break;
            }
        };

        match user_message(&new_user, msg, &rooms).await {
            Ok(_) => (),
            Err(e) => eprintln!("Failed to send user message: {}", e),
        }
    }

    user_disconnected(new_user, &rooms).await;
}

// TODO: Tidy up parameters, maybe UserMessage struct or something
async fn user_message(user: &User, msg: Message, rooms: &Rooms) -> Result<(), anyhow::Error> {
    let msg = if let Ok(s) = msg.to_str() {
        s
    } else {
        return Ok(());
    };

    let new_msg = format!("<User#{}>: {}", user.user_id, msg);
    user.db_tx.send(ChatMessage::new(
        user.user_id,
        String::from(&user.chat_room),
        new_msg.clone(),
    ))?;

    let users = rooms
        .read()
        .await
        .get(&user.chat_room)
        .map(|u| u.clone())
        .unwrap_or_else(|| Users::default());
    for (&uid, tx) in users.read().await.iter() {
        if user.user_id != uid {
            // This will only fail if the receiving user has already disconnected -- just skip over
            if let Err(_disconnected) = tx.send(Message::text(&new_msg)) {}
        }
    }

    Ok(())
}

async fn user_disconnected(user: User, rooms: &Rooms) {
    eprintln!("User disconnected: {}", user.user_id);

    remove_user_from_room(&user, rooms).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::future;
    use warp::test;

    #[tokio::test]
    async fn test_html_connection() {
        let index = index();

        let response = test::request().reply(&index).await;

        assert_eq!(response.status(), 200);
        assert_eq!(response.body(), INDEX_HTML);
    }

    #[tokio::test]
    async fn test_ws_connection() {
        let chat = chat().map(|ws: Ws, _| ws.on_upgrade(|_| future::ready(())));

        test::ws()
            .path("/chat/room1")
            .handshake(chat)
            .await
            .expect("Handshake failed");

        test::ws()
            .path("/chat/room10")
            .handshake(chat)
            .await
            .expect("Handshake failed");
    }

    #[tokio::test]
    #[should_panic]
    async fn test_ws_connection_panics() {
        let chat = chat().map(|ws: Ws, _| ws.on_upgrade(|_| future::ready(())));

        // Should panic, since no room specified -- default should be 'public'
        test::ws()
            .path("/chat")
            .handshake(chat)
            .await
            .expect("Handshake failed");
    }

    #[test]
    fn test_db_connection() {
        let (_, db_rx) = mpsc::unbounded_channel();
        let db_path = Path::new("./test.db");
        let db_conn = std::thread::spawn(move || spawn_db(db_path, db_rx));

        // Sender is dropped immediately above, hence this should return without any errors
        db_conn.join().unwrap().unwrap();

        std::fs::remove_file(db_path).unwrap();
    }
}
