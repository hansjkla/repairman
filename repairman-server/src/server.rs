use std::{
    io::{self, Write, Read}, path::Path, sync::Arc
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt}, net::{TcpListener, TcpStream}, fs,
};

use flate2::{Compression, write::DeflateEncoder};

use blake2::Blake2s256;
use digest::Digest;
use file_hashing::get_hash_file;

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

    let mut use_cache = false;

    match cache {
        Some(path) => {
            let path = Path::new(&path);
            if path.exists() {
                parse_cache(path, files)?;
            } else {
                create_cache(path, files)?;
            }
        },
        // None => use_cache = true,
        None => use_cache = false,
    }

    loop {
        let (stream, _) = listener.accept().await?;

        let hashes_clone = Arc::clone(&hashes);

        if !use_cache {
            tokio::spawn(async move {
                handle_connection(stream, hashes_clone).await.unwrap_or_else(|err| {
                    eprintln!("Error handeling a connection: {err}");
                });
            });
        }
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


fn parse_cache(path: &Path, files: &[HashedFile]) -> io::Result<()> {
    let inventory_file = path.join(Path::new("inventory.compmeta"));

    if !inventory_file.exists() {
        return create_cache(path, files);
    }

    let inventory_content = std::fs::read_to_string(inventory_file)?;
    let mut inv_list: Vec<(HashedFile, String)> = Vec::new();

    for line in inventory_content.lines() {
        let mut parts = line.split("\0");

        let current_path = parts.next();
        let file_hash = parts.next();
        let compressed_file_hash = parts.next();

        if current_path.is_none() || file_hash.is_none() || compressed_file_hash.is_none() {
            continue;
        }

        let current_path = current_path.unwrap();
        let file_hash = file_hash.unwrap();
        let compressed_file_hash = compressed_file_hash.unwrap().trim();

        inv_list.push((HashedFile::new(current_path, file_hash), compressed_file_hash.to_string()));
    }

    let mut buffer = vec![0u8; 8192];

    for file in files {
        let mut current_file_exist = false;
        let mut current_compress_hash = String::new();

        for (entry, hash) in &inv_list {
            let mut str_to_cmp = path.join("files").join(file.get_path());
            str_to_cmp.set_extension("comp");

            let cmp_hashedfile = HashedFile::new(str_to_cmp.to_str().unwrap(), file.get_hash());

            if &cmp_hashedfile == entry {
                current_file_exist = true;
                current_compress_hash = hash.to_string();
                break;
            }
        }

        let mut current_compressed_file = Path::new(path).join("files").join(file.get_path());
        current_compressed_file.set_extension("comp");


        if current_file_exist && current_compressed_file.exists() {
            let mut hasher = Blake2s256::new();

            let new_compressed_file_hash = get_hash_file(&current_compressed_file, &mut hasher)
                .map_err(|e| io::Error::new(e.kind(), format!("Failed to hash {:?}: {}", &current_compressed_file, e)))?;

            if current_compress_hash.as_str() != new_compressed_file_hash.as_str() {
                let mut file_handle = std::fs::File::open(file.get_path())?;
                let compressed_file = std::fs::File::create(&current_compressed_file)?;
                let mut encoder = DeflateEncoder::new(compressed_file, Compression::fast());

                loop {
                    let n = file_handle.read(&mut buffer)?;
                    if n == 0 { break; };

                    encoder.write_all(&buffer[..n])?;
                }

                encoder.finish()?;
            }
        } else {
            let mut file_handle = std::fs::File::open(file.get_path())?;
            let compressed_file = std::fs::File::create(&current_compressed_file)?;
            let mut encoder = DeflateEncoder::new(compressed_file, Compression::fast());

            loop {
                let n = file_handle.read(&mut buffer)?;
                if n == 0 { break; };

                encoder.write_all(&buffer[..n])?;
            }

            encoder.finish()?;
        }
    }

    let mut meta_file_handle = std::fs::File::create(path.join(Path::new("inventory.compmeta")))?;

    for file in files {
        let mut path = Path::new(path).join("files").join(file.get_path());
        path.set_extension("comp");

        let path = match path.to_str() {
            Some(p) => p,
            None => break,
        };

        let mut hasher = Blake2s256::new();

        let current_comp_hash = get_hash_file(path, &mut hasher)
            .map_err(|e| io::Error::new(e.kind(), format!("Failed to hash {:?}: {}", &path, e)))?;

        let line = format!("{}\0{}\0{}\n", path, file.get_hash(), current_comp_hash);

        meta_file_handle.write_all(line.as_bytes())?;
    }

    Ok(())
}

fn create_cache(path: &Path, files: &[HashedFile]) -> io::Result<()> {
    std::fs::create_dir_all(path)?;

    let mut buffer = vec![0u8; 8192];
    let mut meta_file_handle = std::fs::File::create(path.join(Path::new("inventory.compmeta")))?;

    for file in files {
        let mut file_handle = std::fs::File::open(file.get_path())?;
        let mut path = path.join(Path::new("files")).join(file.get_path());

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        path.set_extension("comp");

        let compressed_file = std::fs::File::create(&path)?;
        let mut encoder = DeflateEncoder::new(compressed_file, Compression::fast());

        loop {
            let n = file_handle.read(&mut buffer)?;
            if n == 0 { break; };

            encoder.write_all(&buffer[..n])?;
        }

        encoder.finish()?;

        let mut hasher = Blake2s256::new();

        let compressed_file_hash = get_hash_file(&path, &mut hasher)
            .map_err(|e| io::Error::new(e.kind(), format!("Failed to hash {:?}: {}", &path, e)))?;

        let line = format!("{}\0{}\0{}\n", path.to_str().unwrap(), file.get_hash(), compressed_file_hash);

        meta_file_handle.write_all(line.as_bytes())?;
    }

    Ok(())
}
