use std::path::Path;

use server::run_server;

use hashed_files::par_hash;
use clap::{Parser};

mod hashed_files;
mod server;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    path: String,

    #[arg(short, long, default_value_t = 6767)]
    port: u16,

    #[arg(short, long, default_value_t = String::from("0.0.0.0"))]
    address: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    println!("path: {}, address: {}, port: {}", args.path, args.address, args.port);

    let list = match par_hash(Path::new(&args.path)) {
        Ok(w) => w,
        Err(err) => {
            eprintln!("Error getting file hashes: {}", err);
            return;
        },
    };

    for item in &list {
        println!("{}", item);
    }

    match run_server(&list, &format!("{}:{}", args.address, args.port)).await {
        Ok(_) => (),
        Err(e) => {
            eprintln!("{e}");
            return;
        },
    };
}
