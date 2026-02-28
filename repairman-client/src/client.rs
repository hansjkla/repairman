use std::{
    collections::HashMap,  fs::{self, File}, io::{self, Write}, path::Path
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt}, net::*, sync::mpsc, task
};


use blake2::Blake2s256;
use digest::Digest;
use file_hashing::get_hash_file;
use flate2::write::DeflateDecoder;

use repairman_common::*;


pub async fn start_communication(server: &str, origin_path: &str) -> std::io::Result<()> {
    
    let mut stream = tokio::net::TcpStream::connect(format!("{server}:6767")).await?;

    request_hashes(&mut stream).await?;

    let response = async_parse_request(&mut stream).await?;

    if response.get_type() != &RequestType::GiveHashes {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Response isn't file hashes."));
    }

    let mut file_list: HashMap<u32, HashedFile> = HashMap::new();
    let mut reader = tokio::io::BufReader::new(&mut stream);
    let mut buffer = Vec::new();

    let file_count = reader.read_u32().await?;
    println!("file_count: {file_count}");

    for _ in 0..file_count {
        let id = reader.read_u32().await?;

        buffer.clear();
        let n = reader.read_until(b'\0', &mut buffer).await?;

        if buffer.last() == Some(&b'\0') {
            buffer.pop();
        }


        let path = match URL_SAFE_NO_PAD.decode(&buffer[..(n - 1)]) {
            Ok(v) => {
                match str::from_utf8(&v) {
                    Ok(v) => v.to_string(),
                    Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Error getting file path: {e}"))),
                }
            },
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("Error decoding file path: {e}"))),
        };

        let mut hash_buffer = vec![0u8; 64];
        reader.read_exact(&mut hash_buffer).await?;
        let hash = match str::from_utf8(&hash_buffer) {
            Ok(f) => f,
            Err(err) => {
                eprintln!("Error passing a file request to the unpacking task: {}", err);
                continue;
            },
        };

        file_list.insert(id, HashedFile::new(path.as_str(), hash));
    }

    println!("Stating checking...");
    let mut loop_iter = 0;

    loop {
        let checked_files = match check_files(Path::new(origin_path), &file_list) {
            Some(v) => v,
            None => return Err(io::Error::new(io::ErrorKind::InvalidData, "Error checking the files against hashes.")),
        };

        for (id, state) in &checked_files {
            println!("{}  {}", id, state);
        }
        println!(" ");
        
        let to_download_total = request_files(&mut stream, &checked_files).await?;

        if to_download_total == 0 {
            break;
        } else if loop_iter == 3 {
            eprintln!("Still incorrect files, after third download attempt, exiting.");
            break;
        }

        if !Path::new(origin_path).exists() {
            fs::create_dir(origin_path)?;
        }

        let (tx, mut rx) = mpsc::channel::<Body>(100);

        let origin = origin_path.to_string();

        let unpacker_handle = task::spawn_blocking(move || {
            let result: io::Result<()> = (|| {
                let mut current_decoder: Option<DeflateDecoder<fs::File>> = None;

                while let Some(body) = rx.blocking_recv() {
                    match body {
                        Body::StartFile(name) => {
                            let path = Path::new(&origin).join(name);

                            if let Some(parent) = path.parent() {
                                fs::create_dir_all(parent)?;
                            }

                            let file = File::create(path)?;
                            current_decoder = Some(DeflateDecoder::new(file));
                        },

                        Body::Content(cont) => {
                            if let Some(ref mut decoder) = current_decoder {
                                decoder.write_all(&cont)?;
                            }
                        },

                        Body::FileDone => {
                            if let Some(decode) = current_decoder.take() {
                                decode.finish()?;
                            }
                        },
                    }
                }
                Ok(())
            })();

            if let Err(err) = result {
                eprintln!("Error unpacking: {err}");
            }
        });


        for _ in 0..to_download_total  {
            let response = async_parse_request(&mut stream).await?;

            if response.get_type() != &RequestType::GiveFiles {
                continue;
            }

            // let mut file_name_buffer = vec![0u8; *response.get_body_size()];

            // stream.read_exact(&mut file_name_buffer).await?;

            let current_id = stream.read_u32().await?;

            let file_path = match file_list.get(&current_id) {
                Some(p) => p,
                None => {
                    eprintln!("An invalid ID was send from the server, skipping file.");
                    continue;
                },
            };

            let name = Body::StartFile(file_path.get_path().to_string());

            match tx.send(name).await {
                Ok(_) => (),
                Err(err) => eprintln!("Error passing a file request to the unpacking task: {}", err),
            };


            loop {
                let response = async_parse_request(&mut stream).await?;

                match response.get_type() {
                    RequestType::EndFile => break,
                    RequestType::Chunk => {
                        let to_read = *response.get_body_size();
                        let mut buffer = vec![0u8; to_read];
                        stream.read_exact(&mut buffer).await?;
                        let to_send = Body::Content(buffer);
                        tx.send(to_send).await.map_err(|err| {
                            io::Error::new(io::ErrorKind::InvalidData, err.to_string())
                        })?;
                    },
                    _ => {
                        eprintln!("Didn't recieve a right response.");
                        break;
                    },
                }
            }

            tx.send(Body::FileDone).await.map_err(|err| {
                io::Error::new(io::ErrorKind::InvalidData, err.to_string())
            })?;
        }

        drop(tx);

        unpacker_handle.await?;

        loop_iter += 1;
    }

    let disconnect_header = create_header(RequestVersion::ZEROpOne, RequestType::Disconnect, 0);
    stream.write_all(&disconnect_header).await?;

    Ok(())
}

enum Body {
    StartFile(String),
    Content(Vec<u8>),
    FileDone,
}

async fn request_hashes(stream: &mut TcpStream) -> io::Result<()> {
    let header = create_header(RequestVersion::ZEROpOne, RequestType::GetHashes, 0);

    stream.write_all(&header).await?;

    Ok(())
}


fn check_files(path: &Path, files: &HashMap<u32, HashedFile>) -> Option<Vec<(u32, FileState)>> {
    if !path.exists() {
        let list:Vec<(u32, FileState)> = files.par_iter().map(|(id, _)| {
            (*id, FileState::Missing)
        }).collect();

        return Some(list);
    }

    if path.is_dir() {
        let list2: Vec<(u32, FileState)> = files.par_iter().map(|(id, file)| { 
            let full_path = path.join(file.get_path());

            if !full_path.exists() {
                return (*id, FileState::Missing);
            }

            let mut hasher = Blake2s256::new();

            let file_hash = match get_hash_file(&full_path, &mut hasher) {
                Ok(r) => r,
                Err(_) => return (*id, FileState::Missing),
            };

            if file_hash == file.get_hash() {
                (*id, FileState::Present)
            } else {
                (*id, FileState::Corrupted)
            }
         }).collect();

        if !list2.is_empty() {
            return Some(list2);
        }
    }

    None
}

async fn request_files(stream: &mut TcpStream, checked_files: &[(u32, FileState)]) -> std::io::Result<usize> {
    let mut missing_amount: usize = 0;

    let body: Vec<u8> = checked_files.iter()
        .filter(|(_, state)| {
            if *state != FileState::Present {
                missing_amount += 1;
                return true;
            }
            false
        })
        .flat_map(|(id, _)| {
            id.to_be_bytes()
        })
    .collect();

    if body.is_empty() {
        return Ok(missing_amount);
    }

    let body_size = body.len() as u32;
    let header = create_header(RequestVersion::ZEROpOne, RequestType::GetFiles, body_size);

    stream.write_all(&header).await?;
    stream.write_all(&body).await?;

    Ok(missing_amount)
}