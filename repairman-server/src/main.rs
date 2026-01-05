use std::{env, path::Path};

use server::run_server;

use hashed_files::par_hash;

mod hashed_files;
mod server;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 2 {
        eprintln!("Too many arguments passed.");
        return;
    } else if args.len() < 2 {
        eprintln!("Not enough arguments passed. The path is missing.");
        return;
    }


    let list = match par_hash(Path::new(&args[1])) {
        Ok(w) => w,
        Err(err) => {
            eprintln!("Error getting file hashes: {}", err);
            return;
        },
    };

    for item in &list {
        println!("{}", item);
    }

    run_server(&list, "127.0.0.1:6767").unwrap();
}
