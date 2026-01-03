use std::{
    path::Path,
    io,
};

use blake2::Blake2s256;
use digest::Digest;
use file_hashing::get_hash_file;

use repairman_common::*;


pub fn get_file_hashes(dir: &Path) -> Result<Vec<HashedFile>, io::Error> {
    let mut buffer: Vec<HashedFile> = Vec::new();

    if dir.is_dir() {
        search_and_hash(dir, &mut buffer)?;
    } else {
        return Err(
            io::Error::new(io::ErrorKind::NotADirectory,
                "The path specified is not a dir.")
            );
    }

    Ok(buffer)
}


fn search_and_hash(current_dir: &Path, buffer: &mut Vec<HashedFile>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current_dir)? {
        let path = entry?.path();

        let path_str = path.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Path contains invalid UTF-8")
        })?;

        if path.is_dir() {
            search_and_hash(Path::new(&path), buffer)?;
        } else {
            let mut hasher = Blake2s256::new();
            let result_bytes = get_hash_file(&path, &mut hasher)?;

            buffer.push(HashedFile::new(path_str, &result_bytes));
        }
    }

    Ok(())
}
