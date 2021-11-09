use bi_chat::db;
use bi_chat::routes;
use bi_chat::user::user_connected;
use bi_chat::user::Rooms;

use std::path::Path;

use db::spawn_db;
use tokio::sync::mpsc::{self};
use warp::{ws::Ws, Filter};

const MAIN_DB_PATH: &str = "./main.db";

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

    let chat = routes::chat()
        .and(db_tx)
        .and(rooms)
        .map(|ws: Ws, chat_room, db_tx, rooms| {
            ws.on_upgrade(move |socket| user_connected(socket, chat_room, db_tx, rooms))
        });

    let index = routes::index();

    let routes = index.or(chat);

    println!("Serving!");
    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;

    db_handle.join().expect("Failed to join on DB thread")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bi_chat::html::INDEX_HTML;
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
        let db_path = Path::new("./test.db");
        let db_conn = std::thread::spawn(move || spawn_db(db_path, db_rx));

        // Sender is dropped immediately above, hence this should return without any errors
        db_conn.join().unwrap().unwrap();

        std::fs::remove_file(db_path).unwrap();
    }
}
