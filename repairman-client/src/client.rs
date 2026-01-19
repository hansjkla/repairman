use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path, sync::Arc,
};

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt}, net::*, sync::mpsc, task
};


use blake2::Blake2s256;
use digest::Digest;
use file_hashing::get_hash_file;
use flate2::write::ZlibDecoder;

use repairman_common::*;


pub async fn start_communication(server: &str, origin_path: &str) -> std::io::Result<()> {
    let mut stream = tokio::net::TcpStream::connect(format!("{server}:6767")).await?;
    let mut file_list = Vec::new();

    request_hashes(&mut stream).await?;

    let response = async_parse_request(&mut stream).await?;

    if response.get_type() != &RequestType::GiveHashes {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Response isn't file hashes."));
    }

    let mut body = vec![0u8; response.get_body_size().unwrap()];

    stream.read_exact(&mut body).await?;

    let body = match str::from_utf8(&body) {
        Ok(b) => b,
        Err(_) => return Err(io::Error::new(io::ErrorKind::InvalidData, "Couldn't turn response body into string.")),
    };

    let lines = body.lines();

    for line in lines {
        let mut part = line.split(' ');
            
        let path = match part.next() {
            Some(p) => p,
            None => return Err(io::Error::new(io::ErrorKind::InvalidData, "Responses body contains invalid path.")),
        };

        let hash = match part.next() {
            Some(h) => h,
            None => return Err(io::Error::new(io::ErrorKind::InvalidData, "Responses body contains invalid hash.")),
        };

        file_list.push(HashedFile::new(path, hash));
    }

    let checked_files = match check_files(Path::new(origin_path), &file_list) {
        Some(v) => v,
        None => return Err(io::Error::new(io::ErrorKind::InvalidData, "Error checking the files against hashes.")),
    };

    for file in &checked_files {
        println!("{}  {}", file.0.get_path(), file.1);
    }
    
    request_files(&mut stream, &checked_files).await?;

    let to_download_total:Vec<&(&HashedFile, FileState)> = checked_files.par_iter()
        .filter(|f| {
            if f.1 != FileState::Present {
                return true;
            }
            false
        }).collect();

    if !Path::new(origin_path).exists() {
        fs::create_dir(origin_path)?;
    }

    let (tx, mut rx) = mpsc::channel::<Arc<Body>>(100);

    let origin = Arc::new(origin_path.to_string());

    let unpacker_handle = task::spawn_blocking(move || {
        let result: io::Result<()> = (|| {
            let mut current_decoder: Option<ZlibDecoder<fs::File>> = None;

            while let Some(body) = rx.blocking_recv() {
                match body.as_ref() {
                    Body::StartFile(name, _size) => {
                        let path = Path::new(origin.as_ref()).join(name);

                        if let Some(parent) = path.parent() {
                            fs::create_dir_all(parent)?;
                        }

                        let file = File::create_new(path)?;
                        current_decoder = Some(ZlibDecoder::new(file));
                    },

                    Body::Content(cont) => {
                        if let Some(ref mut decoder) = current_decoder {
                            decoder.write_all(cont)?;
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


    for _ in 0..to_download_total.len()  {
        let response = async_parse_request(&mut stream).await?;

        if response.get_type() != &RequestType::GiveFiles {
            continue;
        }

        let mut file_name_buffer = vec![0u8; response.get_file_name_size().unwrap()];

        stream.read_exact(&mut file_name_buffer).await?;

        let file_name = match String::from_utf8(file_name_buffer) {
            Ok(f) => f,
            Err(err) => {
                eprintln!("Error passing a file request to the unpacking task: {}", err);
                continue;
            },
        };

        let name = Arc::new(Body::StartFile(file_name, response.get_body_size().unwrap()));

        match tx.send(Arc::clone(&name)).await {
            Ok(_) => (),
            Err(err) => eprintln!("Error passing a file request to the unpacking task: {}", err),
        };

        let mut send_count = response.get_body_size().unwrap() - response.get_file_name_size().unwrap();
        while send_count > 0 {
            let to_read = std::cmp::min(send_count, 8192);
            let mut buffer = vec![0u8; to_read];
            stream.read_exact(&mut buffer).await?;
            let to_send = Arc::new(Body::Content(buffer));
            tx.send(Arc::clone(&to_send)).await.map_err(|err| {
                io::Error::new(io::ErrorKind::InvalidData, err.to_string())
            })?;
            send_count -= to_read;
        }

        let end_msg = Arc::new(Body::FileDone);
        tx.send(end_msg).await.map_err(|err| {
            io::Error::new(io::ErrorKind::InvalidData, err.to_string())
        })?;
    }

    drop(tx);

    unpacker_handle.await?;

    Ok(())
}

enum Body {
    StartFile(String, usize),
    Content(Vec<u8>),
    FileDone,
}

async fn request_hashes(stream: &mut TcpStream) -> std::io::Result<()> {
    let header = create_header(RequestVersion::ZEROpOne, RequestType::GetHashes, 0, 0);

    stream.write_all(&header).await?;

    Ok(())
}


fn check_files<'a>(path: &Path, files: &'a [HashedFile]) -> Option<Vec<(&'a HashedFile, FileState)>> {
    if !path.exists() {
        let list:Vec<(&HashedFile, FileState)> = files.par_iter().map(|f| {
            (f, FileState::Missing)
        }).collect();

        return Some(list);
    }

    if path.is_dir() {
        let list2: Vec<(&HashedFile, FileState)> = files.par_iter().map(|entry| { 
            let full_path = path.join(entry.get_path());

            if !full_path.exists() {
                return (entry, FileState::Missing);
            }

            let mut hasher = Blake2s256::new();

            let file_hash = match get_hash_file(&full_path, &mut hasher) {
                Ok(r) => r,
                Err(_) => return (entry, FileState::Missing),
            };

            if file_hash == entry.get_hash() {
                (entry, FileState::Present)
            } else {
                (entry, FileState::Corrupted)
            }
         }).collect();

        if !list2.is_empty() {
            return Some(list2);
        }
    }

    None
}

async fn request_files(stream: &mut TcpStream, checked_files: &[(&HashedFile, FileState)]) -> std::io::Result<()> {
    let body: String = checked_files.par_iter()
        .filter(|f| {
            if f.1 != FileState::Present {
                return true;
            }
            false
        })
        .map(|f| {
            format!("{}\n", f.0.get_path())
        })
        .collect();

    let body_size = body.len() as u32;
    let header = create_header(RequestVersion::ZEROpOne, RequestType::GetFiles, 0, body_size);

    stream.write_all(&header).await?;
    stream.write_all(body.as_bytes()).await?;

    Ok(())
}