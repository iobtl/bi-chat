use bi_chat::server;

#[tokio::main]
async fn main() {
    tokio::task::spawn_blocking(|| {
        server::run(3030);
    })
    .await
    .expect("Main task failed")
}
