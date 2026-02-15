use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc
};


use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    fs,
};

use flate2::{Compression, write::DeflateEncoder};

use crate::cache::*;
use repairman_common::*;

pub async fn run_server(files: &[HashedFile], addr: &str, cache: Option<String>) -> std::io::Result<()> {
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

    if let Some(ref path) = cache {
        let path = Path::new(&path);
        if path.exists() {
            parse_cache(path, files)?;
        } else {
            create_cache(path, files)?;
        }

        println!("Caching done...\nListening now");
    }

    let arc_cache = Arc::new(cache);

    loop {
        let (stream, _) = listener.accept().await?;

        let hashes_clone = Arc::clone(&hashes);
        let cache_clone = Arc::clone(&arc_cache);

        
        tokio::spawn(async move {
            handle_connection(stream, hashes_clone, cache_clone).await.unwrap_or_else(|err| {
                eprintln!("Error handeling a connection: {err}");
            });
        });
    }

    // Ok(())
}

async fn handle_connection(mut stream: TcpStream, hashes: Arc<Vec<u8>>, cache_path: Arc<Option<String>>) -> std::io::Result<()> {
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

                let mut buffer = vec![0u8; 32768];
                let mut compression_buffer = Vec::new();

                for file in files.lines() {
                    let file_name_len = file.len() as u32;

                    let header = create_header(RequestVersion::ZEROpOne, RequestType::GiveFiles, file_name_len, 0);

                    stream.write_all(&header).await?;
                    stream.write_all(file.as_bytes()).await?;

                    if let Some(path) = cache_path.as_ref() {
                        let mut path = Path::new(path).join("files").join(file).into_os_string();
                        path.push(".comp");
                        let path = PathBuf::from(path);

                        let mut file_handle = fs::File::open(path).await?;

                        loop {
                            let n = file_handle.read(&mut buffer).await?;
                            if n == 0 { break; }

                            let header = create_header(RequestVersion::ZEROpOne, RequestType::Chunk, 0, n as u32);
                            stream.write_all(&header).await?;
                            stream.write_all(&buffer[..n]).await?;
                        }

                        let end_header = create_header(RequestVersion::ZEROpOne, RequestType::EndFile, 0, 0);
                        stream.write_all(&end_header).await?;

                    } else {
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
                }
            },

            RequestType::Disconnect => break,
            
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Got an invalid request type.")),
        }
    }

    Ok(())
}
