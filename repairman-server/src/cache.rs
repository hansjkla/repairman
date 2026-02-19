use std::{
    cell::RefCell, collections::HashMap, fs, io::{self, BufRead, BufReader, Read, Write}, path::{Path, PathBuf}
};

use rayon::iter::{IntoParallelRefIterator, ParallelIterator};

use flate2::{Compression, write::DeflateEncoder};
use blake2::Blake2s256;
use digest::Digest;
use file_hashing::get_hash_file;

use repairman_common::*;

pub fn parse_cache(path: &Path, files: &[HashedFile]) -> io::Result<HashMap<String, String>> {
    let inventory_file = path.join(Path::new("inventory.compmeta"));

    if !inventory_file.exists() {
        return create_cache(path, files);
    }

    let mut inv_map: HashMap<HashedFile, String> = HashMap::new();

    let meta_file_handle = fs::File::open(inventory_file)?;
    let meta_file_handle = BufReader::new(meta_file_handle);
    let mut segments = meta_file_handle.split(b'\0').map(|seg| {
        seg.and_then(|bytes| {
            String::from_utf8(bytes).map_err(|_e| io::Error::new(io::ErrorKind::InvalidData, "Unable to read out segment of cache metadata file."))
        })
    });

    while let Some(path_res) = segments.next() {
        let path = path_res?;

        let origin_file_hash = segments.next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Unable to trun a origin file hash into a string."))??;

        let compressed_file_hash = segments.next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Unable to trun a compressed file hash into a string."))??;

        inv_map.insert(HashedFile::new(&path, &origin_file_hash), compressed_file_hash);
    }

    let mut buffer = vec![0u8; 8192];
    let mut cache_was_invalid = false;
    let mut paths_map = HashMap::with_capacity(files.len());

    for file in files {
        let mut file_has_to_be_redone = false;
        
        let path_to_cmp = path.join("files").join(file.get_path());
        let mut os_file_path = path_to_cmp.into_os_string();
        os_file_path.push(".comp");

        let path_to_cmp = PathBuf::from(os_file_path);


        let compressed_file_exists = path_to_cmp.exists();

        let path_to_cmp = path_to_cmp.to_str().unwrap();

        paths_map.insert(file.get_path().to_string(), path_to_cmp.to_string());

        let hashedfile_to_cmp = HashedFile::new(path_to_cmp, file.get_hash());

        if let Some((hashedfile, compressed_hash)) = inv_map.get_key_value(&hashedfile_to_cmp) &&
                    hashedfile.get_hash() == file.get_hash() &&
                    compressed_file_exists {

            let mut hasher = Blake2s256::new();

            let actual_compressed_files_hash = get_hash_file(path_to_cmp, &mut hasher)
                .map_err(|e| io::Error::new(e.kind(), format!("Failed to hash {:?}: {}", path_to_cmp, e)))?;

            if actual_compressed_files_hash.as_str() != compressed_hash.as_str() {
                file_has_to_be_redone = true;
            }
        } else {
            file_has_to_be_redone = true;
        }

        if file_has_to_be_redone {
            cache_was_invalid = true;

            let mut origin_file_handle = fs::File::open(file.get_path())?;
            let compressed_file_handle = fs::File::create(path_to_cmp)?;
            let mut encoder = DeflateEncoder::new(compressed_file_handle, Compression::fast());

            loop {
                let n = origin_file_handle.read(&mut buffer)?;
                if n == 0 { break; };

                encoder.write_all(&buffer[..n])?;
            }

            encoder.finish()?;
        }
    }

    if cache_was_invalid {
        println!("Cache was invalid, redoing the metadata file.");
        let mut metadata = String::with_capacity(264 * files.len());

        let results: Vec<io::Result<String>> = files.par_iter().map(|f| {
            let path = path.join("files").join(f.get_path());
            let mut os_file_path = path.into_os_string();
            os_file_path.push(".comp");

            let path = PathBuf::from(os_file_path);

            let path = path.to_str().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Unable to turn path into a string, for creation of metadata file."))?;

            let mut hasher = Blake2s256::new();

            let current_comp_hash = get_hash_file(path, &mut hasher)
                .map_err(|e| io::Error::new(e.kind(), format!("Failed to hash {:?}: {}", &path, e)))?;

            Ok(format!("{}\0{}\0{}\0", path, f.get_hash(), current_comp_hash))
        }).collect();

        for line in results {
            let line = line?;
            metadata.push_str(&line);
        }

        fs::write(path.join(Path::new("inventory.compmeta")), metadata)?;
    }

    Ok(paths_map)
}

thread_local! {
    static THEAD_BUFFER: RefCell<Vec<u8>> = RefCell::new(vec![0u8; 8192]);
}

pub fn create_cache(path: &Path, files: &[HashedFile]) -> io::Result<HashMap<String, String>> {
    fs::create_dir_all(path)?;

    let cache_parts: Vec<io::Result<ChachePart>> = files.par_iter().map(|f| {
        let mut file_handle = fs::File::open(f.get_path())?;
        let path = path.join(Path::new("files")).join(f.get_path());

        if let Some(parent) = path.parent() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }

        let mut os_file_path = path.into_os_string();
        os_file_path.push(".comp");

        let path = PathBuf::from(os_file_path);

        let compressed_file = fs::File::create(&path)?;
        let mut encoder = DeflateEncoder::new(compressed_file, Compression::fast());

        THEAD_BUFFER.with(|buffer| -> io::Result<()> {
            let mut buffer = buffer.borrow_mut();
            loop {
                let n = file_handle.read(&mut buffer)?;
                if n == 0 { break; };

                encoder.write_all(&buffer[..n])?;
            }
            Ok(())
        })?;

        encoder.finish()?;

        let mut hasher = Blake2s256::new();

        let compressed_file_hash = get_hash_file(&path, &mut hasher)
            .map_err(|e| io::Error::new(e.kind(), format!("Failed to hash {:?}: {}", &path, e)))?;

        let path = match path.to_str() {
            Some(p) => p,
            None => return Err(io::Error::new(io::ErrorKind::AddrInUse, "")),
        };

        Ok(ChachePart::new(format!("{}\0{}\0{}\0", path, f.get_hash(), compressed_file_hash), f.get_path().to_string(), path.to_string()))
    }).collect();

    let mut metadata = String::with_capacity(264 * cache_parts.len());
    let mut paths_map = HashMap::with_capacity(cache_parts.len());

    for part in cache_parts {
        let part = part?;
        metadata.push_str(&part.compmeta_line);
        paths_map.insert(part.uncompressed_path, part.compressed_path);
    }

    fs::write(path.join(Path::new("inventory.compmeta")), metadata)?;

    Ok(paths_map)
}

struct ChachePart {
    compmeta_line: String,
    uncompressed_path: String,
    compressed_path: String,
}

impl ChachePart {
    fn new(compmeta_line: String,
    uncompressed_path: String,
    compressed_path: String,) -> ChachePart {
        ChachePart { compmeta_line, uncompressed_path, compressed_path }
    }
}
