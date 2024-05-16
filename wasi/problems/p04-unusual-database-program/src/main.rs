use wasi_async::net::UdpSocket;

use clap::Parser;

use tracing::info;

use p04_unusual_database_program::run;

#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "0.0.0.0")]
    address: String,

    #[arg(long, default_value_t = 10000)]
    port: u16,
}

fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    info!("start");

    let result = wasi_async_runtime::block_on(|reactor| async move {
        let socket = UdpSocket::bind(reactor, format!("{}:{}", args.address, args.port)).await?;

        run(socket).await
    });

    info!("done: {result:?}");

    Ok(result?)
}
