use std::{
    io::{self, Write}, sync::Arc
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt}, net::{TcpListener, TcpStream}, fs,
};

use flate2::{Compression, write::DeflateEncoder};

use repairman_common::*;

pub async fn run_server(files: &[HashedFile], addr: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;

    // Create the GIVE-HASHES response to reuse, body contains "file_name hash" on sperated lines
    let mut body = String::new();
    for file in files {
        body.push_str(format!("{} {}\n", file.get_path(), file.get_hash()).as_str());
    }

    let body_size = body.len() as u32;
    let header = create_header(RequestVersion::ZEROpOne, RequestType::GiveHashes, 0, body_size);

    let mut hashes = Vec::with_capacity(body.len() + header.len());
    hashes.extend_from_slice(&header);
    hashes.extend_from_slice(body.as_bytes());

    let hashes = Arc::new(hashes);


    loop {
        let (stream, _) = listener.accept().await?;

        let hashes_clone = Arc::clone(&hashes);

        tokio::spawn(async move {
            handle_connection(stream, hashes_clone).await.unwrap_or_else(|err| {
                eprintln!("Error handeling a connection: {err}");
            });
        });
    }

    // Ok(())
}

async fn handle_connection(mut stream: TcpStream, hashes: Arc<Vec<u8>>) -> std::io::Result<()> {
    loop {
        let request = async_parse_request(&mut stream).await?;

        match request.get_type() {
            RequestType::GetHashes => {
                stream.write_all(&hashes).await?;
            },

            RequestType::GetFiles => {
                let mut files = vec![0u8; *request.get_body_size()];
                stream.read_exact(&mut files).await?;
                let files = match str::from_utf8(&files) {
                    Ok(f) => f,
                    Err(_) => return Err(io::Error::new(io::ErrorKind::InvalidData, "Couldn't convert body to string.")),
                };

                let mut buffer = vec![0u8; 8192];
                let mut compression_buffer = Vec::new();

                for file in files.lines() {
                    let file_name_len = file.len() as u32;

                    let header = create_header(RequestVersion::ZEROpOne, RequestType::GiveFiles, file_name_len, 0);

                    stream.write_all(&header).await?;
                    stream.write_all(file.as_bytes()).await?;

                    let mut file_handle = fs::File::open(file).await?;
                    let mut encoder = DeflateEncoder::new(&mut compression_buffer, Compression::fast());

                    loop {
                        let n = file_handle.read(&mut buffer).await?;
                        if n == 0 { break; }
                        
                        encoder.write_all(&buffer[..n])?;
                        let compressed_data = encoder.get_mut();

                        if !compressed_data.is_empty() {
                            let chunk_header = create_header(RequestVersion::ZEROpOne, RequestType::Chunk, 0, compressed_data.len() as u32);
                            stream.write_all(&chunk_header).await?;
                            stream.write_all(compressed_data).await?;

                            compressed_data.clear();
                        }
                    }

                    let final_compressed_data = encoder.finish()?;
                    if !final_compressed_data.is_empty() {
                        let chunk_header = create_header(RequestVersion::ZEROpOne, RequestType::Chunk, 0, final_compressed_data.len() as u32);
                        stream.write_all(&chunk_header).await?;
                        stream.write_all(final_compressed_data).await?;
                    }

                    let end_header = create_header(RequestVersion::ZEROpOne, RequestType::EndFile, 0, 0);
                    stream.write_all(&end_header).await?;

                    compression_buffer.clear();
                }
            },

            RequestType::Disconnect => break,
            
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Got an invalid request type.")),
        }
    }

    Ok(())
}
