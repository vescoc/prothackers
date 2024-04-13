use clap::Parser;
use tokio::net::TcpListener;

use tracing::info;

use p05_mob_in_the_middle::{run, BOGUSCOIN};

#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "0.0.0.0")]
    address: String,

    #[arg(long, default_value_t = 10000)]
    port: u16,

    #[arg(long, default_value = "chat.protohackers.com")]
    chat_address: String,

    #[arg(long, default_value_t = 16963)]
    chat_port: u16,

    #[arg(long, default_value_t = BOGUSCOIN.to_string())]
    boguscoin: String,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    info!("start");

    let listener = TcpListener::bind(&format!("{}:{}", args.address, args.port)).await?;

    run(listener, args.chat_address, args.chat_port, args.boguscoin).await
}
