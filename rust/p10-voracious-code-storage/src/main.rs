use clap::Parser;
use tokio::net::TcpListener;

use tracing::info;

use p10_voracious_code_storage::run;

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
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    info!("start");

    let socket = TcpListener::bind(&format!("{}:{}", args.address, args.port)).await?;

    Ok(run(socket).await?)
}
