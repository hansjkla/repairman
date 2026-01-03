use client::start_communication;


mod client;

fn main() {
    start_communication("127.0.0.1:6767").unwrap_or_else(|err| { eprintln!("{err}") });
}
