use std::{
    fs,
    io::{self, Write},
    path::Path, sync::Arc,
};

use tokio::{
    net::*,
    io::AsyncWriteExt,
    sync::mpsc,
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


    if let Some(body) = response.get_body() {
        let body = match str::from_utf8(body) {
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
    }

    let checked_files = match check_files(Path::new(origin_path), &file_list) {
        Some(v) => v,
        None => return Err(io::Error::new(io::ErrorKind::InvalidData, "Error checking the files against hashes.")),
    };

    for file in &checked_files {
        println!("{}  {}", file.0.get_path(), file.1);
    }
    
    request_files(&mut stream, &checked_files).await?;

    let to_download_total:Vec<&(&HashedFile, FileState)> = checked_files.iter()
        .filter(|f| {
            if f.1 != FileState::Present {
                return true;
            }
            false
        }).collect();

    if !Path::new(origin_path).exists() {
        fs::create_dir(origin_path)?;
    }

    let (tx, mut rx) = mpsc::channel::<Request>(100);

    let origin = Arc::new(origin_path.to_string());

    let unpacker_handle = tokio::spawn(async move {
        while let Some(file) = rx.recv().await {
            if file.get_type() != &RequestType::GiveFiles {
                continue;
            }

            let origin_path = Arc::clone(&origin);
            
            tokio::task::spawn_blocking(move || {
                let result: io::Result<()> = (|| {
                    let body = file.get_body().as_ref().unwrap();

                    let (file_name, compressed_file) = body.split_at(file.get_file_name_size().unwrap());

                    let file_name = match String::from_utf8(file_name.to_vec()) {
                        Ok(s) => s,
                        Err(_) => return Err(io::Error::new(io::ErrorKind::InvalidData, "Couldn't convert name from file request body to string.")),
                    };

                    let mut writer = Vec::new();
                    let mut z = ZlibDecoder::new(writer);
                    z.write_all(compressed_file)?;
                    writer = z.finish()?;

                    let path = Path::new(origin_path.as_str()).join(file_name);

                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent)?;
                    } else {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "Path of one object doesn't have a parent."));
                    }
                    
                    fs::write(&path, writer)?;

                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("Error unpacking a file: {e}");
                }
            });
        }
    });


    for _ in 0..to_download_total.len()  {
        let response = async_parse_request(&mut stream).await?;

        match tx.send(response).await {
            Ok(_) => (),
            Err(err) => eprintln!("Error passing a file request to the unpacking task: {}", err),
        }
    }

    drop(tx);

    unpacker_handle.await?;

    Ok(())
}

async fn request_hashes(stream: &mut TcpStream) -> std::io::Result<()> {
    let header = create_header(RequestVersion::ZEROpOne, RequestType::GetHashes, 0, 0);

    stream.write_all(&header).await?;

    Ok(())
}


fn check_files<'a>(path: &Path, files: &'a [HashedFile]) -> Option<Vec<(&'a HashedFile, FileState)>> {
    let mut list: Vec<(&HashedFile, FileState)> = Vec::with_capacity(files.len());

    if !path.exists() {
        let list:Vec<(&HashedFile, FileState)> = files.iter().map(|f| {
            (f, FileState::Missing)
        }).collect();

        return Some(list);
    }

    if path.is_dir() {
        for entry in files {
            let full_path = path.join(entry.get_path());

            if !full_path.exists() {
                list.push((entry, FileState::Missing));
                continue;
            }

            let mut hasher = Blake2s256::new();

            let file_hash = match get_hash_file(&full_path, &mut hasher) {
                Ok(r) => r,
                Err(_) => return None,
            };

            if file_hash == entry.get_hash() {
                list.push((entry, FileState::Present));
            } else {
                list.push((entry, FileState::Corrupted));
            }
        }

        if !list.is_empty() {
            return Some(list);
        }
    }

    None
}

async fn request_files(stream: &mut TcpStream, checked_files: &[(&HashedFile, FileState)]) -> std::io::Result<()> {
    let body: String = checked_files.iter()
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