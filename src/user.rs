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
use warp::ws::{Message, WebSocket, Ws};

use crate::{
    db::{DBMessage, DbTx},
    shutdown::Shutdown,
};

pub type Users = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;
pub type Rooms = Arc<RwLock<HashMap<String, Users>>>;

pub type UserTx = UnboundedSender<Message>;
pub type UserRx = UnboundedReceiver<Message>;

pub struct User {
    pub user_id: usize,

    pub chat_room: String,

    pub tx: UserTx,

    pub db_tx: DbTx,
}

impl User {
    pub async fn listen(
        &self,
        ws: WebSocket,
        mut rx: UserRx,
        rooms: Rooms,
    ) -> Result<(), anyhow::Error> {
        println!("Joining room: {}", &self.chat_room);

        let (mut user_ws_tx, mut user_ws_rx) = ws.split();

        add_user_to_room(self, &rooms).await;

        // Dedicated thread to listen and buffer incoming messages using Tokio channels
        // Then feeds into WS sink -> WS stream (to be consumed and displayed)
        tokio::task::spawn(async move {
            while let Some(message) = rx.recv().await {
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
                    eprintln!("Websocket error(uid={}): {}", self.user_id, e);
                    break;
                }
            };

            match user_message(self, msg, &rooms).await {
                Ok(_) => (),
                Err(e) => eprintln!("Failed to send user message: {}", e),
            }
        }

        user_disconnected(self, &rooms).await;

        Ok(())
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

async fn user_message(user: &User, msg: Message, rooms: &Rooms) -> Result<(), anyhow::Error> {
    let msg = if let Ok(s) = msg.to_str() {
        s
    } else {
        return Ok(());
    };

    let new_msg = format!("<User#{}>: {}", user.user_id, msg);
    user.db_tx
        .send(DBMessage::new(user.user_id, &user.chat_room, msg))?;

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

async fn user_disconnected(user: &User, rooms: &Rooms) {
    eprintln!("User disconnected: {}", user.user_id);

    remove_user_from_room(user, rooms).await;
}
