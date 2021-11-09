use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use futures::{SinkExt, StreamExt, TryFutureExt};
use tokio::sync::{
    mpsc::{self, UnboundedReceiver, UnboundedSender},
    RwLock,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::ws::{Message, WebSocket};

use crate::db::{DBMessage, DbTx};

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);

pub type Users = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;
pub type Rooms = Arc<RwLock<HashMap<String, Users>>>;

pub type UserTx = UnboundedSender<Message>;
pub type UserRx = UnboundedReceiver<Message>;

struct User {
    user_id: usize,
    chat_room: String,
    tx: UserTx,
    db_tx: DbTx,
}

impl User {
    pub fn new(user_id: usize, chat_room: String, tx: UserTx, db_tx: DbTx) -> Self {
        User {
            user_id,
            chat_room,
            tx,
            db_tx,
        }
    }
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

pub async fn user_connected(ws: WebSocket, chat_room: String, db_tx: DbTx, rooms: Rooms) {
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
    user.db_tx.send(DBMessage::new(
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
