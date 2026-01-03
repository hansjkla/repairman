use std::{
    fs,
    io::{self, Write},
    net::TcpStream,
    path::Path,
};


use blake2::Blake2s256;
use digest::Digest;
use file_hashing::get_hash_file;
use flate2::write::ZlibDecoder;

use repairman_common::*;


pub fn start_communication(server: &str) -> std::io::Result<()> {
    let stream = TcpStream::connect(server)?;
    let mut file_list = Vec::new();

    request_hashes(&stream)?;

    let response = match parse_request(&stream) {
        Ok(r) => r,
        Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Couldn't parse GET-HASHES response.")),
    };

    if response.get_type() != &RequestType::GiveHashes {
        return Err(io::Error::new(io::ErrorKind::Other, "Response isn't file hashes."));
    }


    if let Some(body) = response.get_body() {
        let body = match str::from_utf8(body) {
            Ok(b) => b,
            Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Couldn't turn response body into string.")),
        };

        let lines = body.lines();

        for line in lines {
            let mut part = line.split(' ');
            
            let path = match part.next() {
                Some(p) => p,
                None => return Err(io::Error::new(io::ErrorKind::Other, "Responses body contains invalid path.")),
            };

            let hash = match part.next() {
                Some(h) => h,
                None => return Err(io::Error::new(io::ErrorKind::Other, "Responses body contains invalid hash.")),
            };

            file_list.push(HashedFile::new(path, hash));
        }
    }

    let checked_files = match check_files(&Path::new("test"), &file_list) {
        Some(v) => v,
        None => return Err(io::Error::new(io::ErrorKind::Other, "Error checking the files against hashes.")),
    };

    for file in &checked_files {
        println!("{}  {}", file.0.get_path(), file.1);
    }
    
    request_files(&stream, &checked_files)?;

    let mut recieved_files: Vec<Request> = Vec::with_capacity(checked_files.len());

    let to_download_total:Vec<&(&HashedFile, FileState)> = checked_files.iter()
        .filter(|f| {
            if f.1 != FileState::Present {
                return true;
            }
            false
        }).collect();

    for _ in 0..to_download_total.len()  {
        let response = match parse_request(&stream) {
            Ok(r) => r,
            Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Couldn't parse GET-FILES response.")),
        };

        recieved_files.push(response);
    }

    println!("{}", recieved_files.len());

    for file in recieved_files {
        if file.get_type() != &RequestType::GiveFiles {
            continue;
        }

        let body = file.get_body().as_ref().unwrap();

        let (file_name, compressed_file) = body.split_at(file.get_file_name_size().unwrap());

        let file_name = match String::from_utf8(file_name.to_vec()) {
            Ok(s) => s,
            Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Couldn't convert name from file request body to string.")),
        };

        println!("File: {file_name}");

        let mut writer = Vec::new();
        let mut z = ZlibDecoder::new(writer);
        z.write_all(compressed_file)?;
        writer = z.finish()?;
        let return_string = String::from_utf8(writer).expect("String parsing error");

        let path = Path::new("test");

        let path = path.join(file_name);
        
        fs::write(path, return_string)?;
    }
    

    Ok(())
}

fn request_hashes(mut stream: &TcpStream) -> std::io::Result<()> {
    let header = create_header(RequestVersion::ZEROpOne, RequestType::GetHashes, 0, 0);

    stream.write_all(&header)?;

    Ok(())
}


fn check_files<'a>(path: &Path, files: &'a [HashedFile]) -> Option<Vec<(&'a HashedFile, FileState)>> {
    let mut list: Vec<(&HashedFile, FileState)> = Vec::with_capacity(files.len());

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

            println!("{}", entry.get_hash());
            println!("File hash: {}", file_hash);

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

fn request_files(mut stream: &TcpStream, checked_files: &[(&HashedFile, FileState)]) -> std::io::Result<()> {
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

    println!("body size: {}", body_size);

    stream.write_all(&header)?;
    stream.write_all(body.as_bytes())?;

    Ok(())
}