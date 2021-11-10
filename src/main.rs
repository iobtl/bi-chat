use bi_chat::server;
use std::path::PathBuf;
use structopt::StructOpt;

#[derive(StructOpt)]
#[structopt(name = "bi_chat", about = "A simple chat server backend.")]
struct Opt {
    #[structopt(default_value = "./main.db", parse(from_os_str))]
    db_path: PathBuf,
}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();
    server::run(3030, opt.db_path).await;
}
