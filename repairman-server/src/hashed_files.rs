use std::{
    fs, io, path::{Path, PathBuf}
};

use blake2::Blake2s256;
use digest::Digest;
use file_hashing::get_hash_file;

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use repairman_common::*;


pub fn par_hash(path: &Path) -> io::Result<Vec<HashedFile>> {
    let files = get_files(path)?;

    files.par_iter().map(|f| {
            let mut hasher = Blake2s256::new();

            let result_bytes = get_hash_file(f, &mut hasher)
                .map_err(|e| io::Error::new(e.kind(), format!("Failed to hash {:?}: {}", f, e)))?;

            let path_str = f.to_str()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid UTF-8 path"))?;

            Ok(HashedFile::new(path_str, &result_bytes))
    }).collect()
}

fn get_files(origin_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut list = Vec::new();

    if origin_dir.is_dir() {
        walkdir(origin_dir, &mut list)?;
    } else if origin_dir.exists() {
        return Ok(vec![origin_dir.to_path_buf()]);
    } else {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("Path does not exist: {:?}", origin_dir)));
    }

    if list.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Directory is empty"));
    }

    Ok(list)
}

fn walkdir(dir: &Path, buffer: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?.path();

        if entry.is_dir() {
            walkdir(&entry, buffer)?;
        } else {
            buffer.push(entry);
        }
    }
    Ok(())
}
