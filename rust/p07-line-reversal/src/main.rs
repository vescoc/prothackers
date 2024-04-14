use clap::Parser;
use tokio::net::UdpSocket;

use tracing::info;

use p07_line_reversal::{init_tracing_subscriber, run, DefaultSocketHandler};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "0.0.0.0")]
    address: String,

    #[arg(long, default_value_t = 10000)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    init_tracing_subscriber();

    let args = Args::parse();

    info!("start");

    let socket = UdpSocket::bind(&format!("{}:{}", args.address, args.port)).await?;

    run::<DefaultSocketHandler>(socket).await
}
