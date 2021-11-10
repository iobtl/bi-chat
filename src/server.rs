use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

use tokio::sync::{
    broadcast,
    mpsc::{self},
};
use warp::{ws::Ws, Filter};

use crate::{
    db::spawn_db,
    routes,
    shutdown::Shutdown,
    user::{Rooms, User},
};

const MAIN_DB_PATH: &str = "./main.db";

static NEXT_USER_ID: AtomicUsize = AtomicUsize::new(1);

pub async fn run(port: u16) {
    // Broadcast channel for sending a shutdown message to all active connections
    let (notify_shutdown, _) = broadcast::channel(1);
    let (shutdown_complete_tx, mut shutdown_complete_rx) = mpsc::channel(1);
    let shutdown_listener = notify_shutdown.subscribe();
    let db_shutdown_complete_tx = shutdown_complete_tx.clone();

    // Spawning of a dedicated thread to handle DB writes
    let (db_tx, db_rx) = mpsc::unbounded_channel();
    let db_path = Path::new(MAIN_DB_PATH);
    let db_handler = std::thread::spawn(move || {
        spawn_db(
            db_path,
            db_rx,
            Shutdown::new(shutdown_listener, db_shutdown_complete_tx),
        )
    });

    // Defining stateful data + DB channel
    let rooms = Rooms::default();
    let rooms = warp::any().map(move || rooms.clone());
    // A DB channel transmission handle/sender should be passed to each connection
    let db_tx = warp::any().map(move || db_tx.clone());

    let chat = routes::chat()
        .and(db_tx)
        .and(rooms)
        .map(|ws: Ws, chat_room, db_tx, rooms| {
            // let shutdown_listener = notify_shutdown.subscribe();
            // let shutdown_complete_tx = shutdown_complete_tx.clone();
            ws.on_upgrade(move |socket| async {
                let user_id = NEXT_USER_ID.fetch_add(1, Ordering::Relaxed);

                // Create unbounded channel to handle buffering and consuming of messages
                let (tx, rx) = mpsc::unbounded_channel();

                let new_user = User {
                    user_id,
                    chat_room,
                    tx,
                    db_tx,
                };

                // Establish new connection
                tokio::spawn(async move {
                    if let Err(e) = new_user.listen(socket, rx, rooms).await {
                        eprintln!(
                            "Failed to establish connection for user {} to room {}: {}",
                            &new_user.user_id, &new_user.chat_room, e
                        );
                    }
                });
            })
        });

    let index = routes::index();

    let routes = index.or(chat);

    let shutdown = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Unable to bind ctrl-c signal handler");
    };
    let server = warp::serve(routes).run(([127, 0, 0, 1], port));

    tokio::select! {
        _ = server => {}
        _ = shutdown => {
            eprintln!("Shutting down");

            // Closes broadcast channel, sending shutdown message to all connections
            drop(notify_shutdown);

            // At this point, each connection should be terminating, dropping their
            // shutdown_complete `Senders`
            // When all connections have terminated, the channel closes and `recv()`
            // returns `None`.
            drop(shutdown_complete_tx);

            eprintln!("Waiting for processes to finish");
            let _ = shutdown_complete_rx.recv().await;
            eprintln!("Done");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::INDEX_HTML;
    use futures::future;
    use warp::test;

    #[tokio::test]
    async fn test_html_connection() {
        let index = routes::index();

        let response = test::request().reply(&index).await;

        assert_eq!(response.status(), 200);
        assert_eq!(response.body(), INDEX_HTML);
    }

    #[tokio::test]
    async fn test_ws_connection() {
        let chat = routes::chat().map(|ws: Ws, _| ws.on_upgrade(|_| future::ready(())));

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
        let chat = routes::chat().map(|ws: Ws, _| ws.on_upgrade(|_| future::ready(())));

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
