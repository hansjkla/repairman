use std::path::Path;

use hashed_files::get_file_hashes;
use server::run_server;

mod hashed_files;
mod server;

fn main() {
    let list = match get_file_hashes(Path::new("src")) {
        Ok(w) => w,
        Err(err) => {
            eprintln!("Error getting file hashes: {}", err);
            return;
        },
    };

    for item in &list {
        println!("{}", item);
    }

    run_server(&list).unwrap();
}
