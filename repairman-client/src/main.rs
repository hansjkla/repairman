use std::env;

use client::start_communication;


mod client;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 3 {
        eprintln!("Too many arguments passed.");
        return;
    } else if args.len() < 3 {
        eprintln!("Not enough arguments passed.");
        return;
    }

    let result = start_communication(&args[1], &args[2]).await;
    result.unwrap_or_else(|err| { eprintln!("{err}") });
}
