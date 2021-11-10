use std::{collections::HashMap, sync::Arc};

use futures::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt, TryFutureExt,
};
use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        RwLock,
    },
    task::JoinHandle,
};
use warp::ws::{Message, WebSocket};

use crate::db::{DBMessage, DbTx};

pub type Users = Arc<RwLock<HashMap<usize, mpsc::UnboundedSender<Message>>>>;
pub type Rooms = Arc<RwLock<HashMap<String, Users>>>;

pub type UserTx = UnboundedSender<Message>;
pub type UserRx = UnboundedReceiver<Message>;

type UserWsTx = SplitSink<WebSocket, Message>;
type UserWsRx = SplitStream<WebSocket>;

pub struct User {
    pub user_id: usize,

    pub chat_room: String,

    pub user_tx: UserTx,

    pub db_tx: DbTx,
}

impl User {
    // Indefinitely listens for messages from a front-end on a WebSocket connection.
    pub async fn listen(&self, ws: WebSocket, rx: UserRx, rooms: Rooms) {
        println!("Joining room: {}", &self.chat_room);

        let (user_ws_tx, mut user_ws_rx) = ws.split();

        // Dedicated thread to listen and buffer incoming messages
        // Then feeds into WS sink -> WS stream (to be consumed and displayed)
        let accept_handler = self.accept_messages(rx, user_ws_tx).await;

        // Main loop: listens for incoming messages from other end of WebSocket
        // "Broadcasting" message sent by this `User` to all other `User`s in the same room
        while let Some(result) = user_ws_rx.next().await {
            let msg = match result {
                Ok(msg) => msg,
                Err(e) => {
                    eprintln!("Websocket error(uid={}): {}", self.user_id, e);
                    break;
                }
            };

            match self.send_message(msg, &rooms).await {
                Ok(_) => (),
                Err(e) => eprintln!("Failed to send user message: {}", e),
            }
        }

        // WebSocket connection terminated, `user_ws_rx` Stream should be closed.
        user_disconnected(&self, &rooms).await;
        accept_handler.abort();
    }

    // Spawn a background task for this `User` to listen to messages from
    // other `User`s.
    async fn accept_messages(&self, mut rx: UserRx, mut user_ws_tx: UserWsTx) -> JoinHandle<()> {
        tokio::task::spawn(async move {
            while let Some(message) = rx.recv().await {
                user_ws_tx
                    .send(message)
                    .unwrap_or_else(|e| {
                        eprintln!("WebSocket send error: {}", e);
                    })
                    .await;
            }
        })
    }

    // Fires off a message to other `User`s in the same room.
    async fn send_message(&self, msg: Message, rooms: &Rooms) -> Result<(), anyhow::Error> {
        let msg = if let Ok(s) = msg.to_str() {
            s
        } else {
            return Ok(());
        };

        let new_msg = format!("<User#{}>: {}", self.user_id, msg);

        // Passes message to DB receiver
        self.db_tx
            .send(DBMessage::new(self.user_id, &self.chat_room, msg))?;

        let users = rooms
            .read()
            .await
            .get(&self.chat_room)
            .cloned()
            .unwrap_or_else(Users::default);
        for (&uid, tx) in users.read().await.iter() {
            if self.user_id != uid {
                // This will only fail if the receiving user has already disconnected -- just skip over
                if let Err(_disconnected) = tx.send(Message::text(&new_msg)) {}
            }
        }

        Ok(())
    }
}

// Adds a `User` to a room, creating one if it does not exist.
pub async fn add_user_to_room(new_user: &User, rooms: &Rooms) {
    let mut room = rooms.write().await;
    let users = room
        .entry(new_user.chat_room.clone())
        .or_insert_with(Users::default);

    users
        .write()
        .await
        .insert(new_user.user_id, new_user.user_tx.clone());
}

// Removes a `User` from a room.
// The "room" is also cleaned up if there are no users remaining.
async fn remove_user_from_room(user: &User, rooms: &Rooms) {
    let mut room = rooms.write().await;
    let room_empty = {
        let mut users = room
            .entry(user.chat_room.clone())
            .or_insert_with(Users::default)
            .write()
            .await;

        users.remove(&user.user_id);

        // Extra check to see if room is empty
        users.is_empty()
    };

    // Cleans up room, if empty
    if room_empty {
        room.remove(&user.chat_room);
    }
}

// User has been disconnected from the WebSocket connection.
async fn user_disconnected(user: &User, rooms: &Rooms) {
    eprintln!("User disconnected: {}", user.user_id);

    remove_user_from_room(user, rooms).await;
}
