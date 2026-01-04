use std::{
    fs,
    io::{self, Write},
    net::{TcpListener, TcpStream},
};

use flate2::Compression;
use flate2::write::ZlibEncoder;

use repairman_common::*;

pub fn run_server(files: &[HashedFile], addr: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr)?;

    for stream in listener.incoming() {
        let stream = stream?;

        handle_connection(stream, files).unwrap_or_else(|err| {
            eprintln!("Error handeling a connection: {err}");
        });
    }

    Ok(())
}

fn handle_connection(mut stream: TcpStream, files: &[HashedFile]) -> std::io::Result<()> {
    loop {
        let request = parse_request(&stream)?;

        match request.get_type() {
            RequestType::GetHashes => {
                let mut body = String::new();
                for file in files {
                    body.push_str(format!("{} {}\n", file.get_path(), file.get_hash()).as_str());
                }

                let body_size = body.len() as u32;
                let header = create_header(RequestVersion::ZEROpOne, RequestType::GiveHashes, 0, body_size);

                stream.write_all(&header)?;
                stream.write_all(body.as_bytes())?;
            },
            RequestType::GetFiles => {
                let files = request.get_body().as_ref().unwrap();
                let files = match str::from_utf8(files) {
                    Ok(f) => f,
                    Err(_) => return Err(io::Error::new(io::ErrorKind::InvalidData, "Couldn't convert body to string.")),
                };
                let files: Vec<&str> = files.lines().collect();

                for file in files {
                    let mut body = file.as_bytes().to_vec();

                    let file_name_len = body.len() as u32;

                    let compressed_content = compress_file(file)?;

                    body.extend_from_slice(&compressed_content);

                    let body_size = body.len() as u32;

                    let header = create_header(RequestVersion::ZEROpOne, RequestType::GiveFiles, file_name_len, body_size);

                    stream.write_all(&header)?;
                    stream.write_all(&body)?;
                }

                break;
            },
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Got an invalid request type.")),
        }
    }

    Ok(())
}

fn compress_file(path: &str) -> io::Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());

    let content = fs::read_to_string(path)?;

    encoder.write_all(content.as_bytes())?;
    

    let result = encoder.finish()?;

    Ok(result)
}