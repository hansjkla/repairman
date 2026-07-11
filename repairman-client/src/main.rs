use clap::Parser;

use client::start_communication;


mod client;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    server: String,

    path: String,

    #[arg(short, long, default_value_t = 6767)]
    port: u16,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let result = start_communication(&args.server, &args.path, args.port).await;
    result.unwrap_or_else(|err| { eprintln!("{err}") });
}
