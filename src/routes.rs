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
