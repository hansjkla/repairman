use std::{
    collections::HashMap, io::{self, Write}, path::Path, sync::Arc
};


use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    fs,
};

use flate2::{Compression, write::DeflateEncoder};

use crate::cache::*;
use repairman_common::*;

pub async fn run_server(base_path: String, files: HashMap<u32, HashedFile>, addr: &str, cache: Option<String>) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;

    // Create the GIVE-HASHES response to reuse, body contains "file_name hash" on sperated lines
    let mut body: Vec<u8> = Vec::with_capacity((70 + 128) * files.len());
    body.extend_from_slice(&(files.len() as u32).to_be_bytes());

    if files.len() == 1 {
        if let Some((id, file)) = files.iter().next() && file.get_path() == base_path {
            let file_path = match Path::new(&base_path).file_name() {
                Some(n) => n.to_string_lossy(),
                None => return Err(io::Error::new(io::ErrorKind::InvalidInput, "Recieved and invalid file path, couldn't extract file name.")),
            };

            let file_path_len = (file_path.len() as u16).to_be_bytes();

            body.extend_from_slice(&(*id).to_be_bytes());
            body.extend_from_slice(&file_path_len);
            body.extend_from_slice(file_path.as_bytes());
            body.extend_from_slice(file.get_hash().as_bytes());
        }
    } else {
        for (id, file) in &files {

            let componants: Vec<_> = Path::new(file.get_path())
                .strip_prefix(&base_path)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect();

            let file_path = componants.join("/");

            let file_path_len = (file_path.len() as u16).to_be_bytes();

            body.extend_from_slice(&(*id).to_be_bytes());
            body.extend_from_slice(&file_path_len);
            body.extend_from_slice(file_path.as_bytes());
            body.extend_from_slice(file.get_hash().as_bytes());
        }
    }


    let body_size = body.len() as u32;
    let header = create_header(RequestVersion::ZEROpOne, RequestType::GiveHashes, body_size);

    let mut hashes = Vec::with_capacity(body.len() + header.len());
    hashes.extend_from_slice(&header);
    hashes.extend_from_slice(&body);


    let hashes = Arc::new(hashes);

    // Check for cache option and create map of origin_path -> compressed file path
    let mut paths_map = HashMap::with_capacity(files.len());
    let mut cache_on = false;

    if let Some(ref path) = cache {
        let path = Path::new(&path);
        if path.exists() {
            paths_map = parse_cache(path, &files)?;
        } else {
            paths_map = create_cache(path, &files)?;
        }
        cache_on = true;

        println!("Caching done...\nListening now");
    } else {
        for (id, file) in files {
            paths_map.insert(id, file.get_path().to_string());
        }
    }

    let paths_map = Arc::new(paths_map);

    loop {
        let (stream, _) = listener.accept().await?;

        let hashes_clone = Arc::clone(&hashes);
        let clone_paths_map = Arc::clone(&paths_map);

        
        tokio::spawn(async move {
            handle_connection(stream, hashes_clone, cache_on,  clone_paths_map).await.unwrap_or_else(|err| {
                eprintln!("Error handeling a connection: {err}");
            });
        });
    }

    // Ok(())
}

async fn handle_connection(mut stream: TcpStream, hashes: Arc<Vec<u8>>, cache_on: bool, paths_map: Arc<HashMap<u32, String>>) -> std::io::Result<()> {
    loop {
        let request = async_parse_request(&mut stream).await?;

        match request.get_type() {
            RequestType::GetHashes => {
                stream.write_all(&hashes).await?;
            },

            RequestType::GetFiles => {
                let body_size = *request.get_body_size();
                if body_size > 4 * paths_map.len() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Client requested more IDs, than possible: {}", body_size / 4)));
                }

                let mut files = vec![0u8; body_size];
                stream.read_exact(&mut files).await?;

                let ids: Vec<u32> = files.chunks_exact(4).map(|c| {
                    u32::from_be_bytes(c.try_into().unwrap())
                }).collect();

                if !files.chunks_exact(4).remainder().is_empty() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Couldn't convert body to IDs."));
                }


                let mut buffer = vec![0u8; 32768];
                let mut compression_buffer = Vec::new();


                for id in ids {
                    let path = match paths_map.get(&id) {
                        Some(v) => v,
                        None => continue, // Or: return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid file requested by client."))
                    };

                    let header = create_header(RequestVersion::ZEROpOne, RequestType::GiveFiles, 4);

                    stream.write_all(&header).await?;
                    stream.write_u32(id).await?;

                    if cache_on {
                        let mut file_handle = fs::File::open(path).await?;

                        loop {
                            let n = file_handle.read(&mut buffer).await?;
                            if n == 0 { break; }

                            let header = create_header(RequestVersion::ZEROpOne, RequestType::Chunk, n as u32);
                            stream.write_all(&header).await?;
                            stream.write_all(&buffer[..n]).await?;
                        }

                        let end_header = create_header(RequestVersion::ZEROpOne, RequestType::EndFile, 0);
                        stream.write_all(&end_header).await?;

                    } else {
                        let mut file_handle = fs::File::open(path).await?;
                        let mut encoder = DeflateEncoder::new(&mut compression_buffer, Compression::fast());

                        loop {
                            let n = file_handle.read(&mut buffer).await?;
                            if n == 0 { break; }
                            
                            encoder.write_all(&buffer[..n])?;
                            let compressed_data = encoder.get_mut();

                            if !compressed_data.is_empty() {
                                let chunk_header = create_header(RequestVersion::ZEROpOne, RequestType::Chunk, compressed_data.len() as u32);
                                stream.write_all(&chunk_header).await?;
                                stream.write_all(compressed_data).await?;

                                compressed_data.clear();
                            }
                        }

                        let final_compressed_data = encoder.finish()?;
                        if !final_compressed_data.is_empty() {
                            let chunk_header = create_header(RequestVersion::ZEROpOne, RequestType::Chunk, final_compressed_data.len() as u32);
                            stream.write_all(&chunk_header).await?;
                            stream.write_all(final_compressed_data).await?;
                        }

                        let end_header = create_header(RequestVersion::ZEROpOne, RequestType::EndFile, 0);
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
