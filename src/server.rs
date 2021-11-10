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
    user::{add_user_to_room, Rooms, User},
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
                let (user_tx, user_rx) = mpsc::unbounded_channel();

                let new_user = User {
                    user_id,
                    chat_room,
                    user_tx,
                    db_tx,
                };

                // Establish new connection
                tokio::task::spawn(async move {
                    add_user_to_room(&new_user, &rooms).await;
                    new_user.listen(socket, user_rx, rooms).await
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
