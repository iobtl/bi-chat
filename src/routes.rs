use warp::{ws::Ws, Filter};

use crate::html::INDEX_HTML;

pub fn chat() -> impl Filter<Extract = (Ws, String), Error = warp::Rejection> + Copy {
    warp::path("chat")
        .and(warp::ws())
        .and(warp::path::param::<String>())
}

pub fn index(
) -> impl Filter<Extract = (warp::reply::Html<&'static str>,), Error = warp::Rejection> + Copy {
    warp::path::end().map(|| warp::reply::html(INDEX_HTML))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{html::INDEX_HTML, routes};
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
}
