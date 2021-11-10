use bi_chat::server;

#[tokio::main]
async fn main() {
    server::run(3030).await;
}
