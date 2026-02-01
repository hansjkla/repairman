#[derive(PartialEq, Eq, Debug, Hash)]
pub struct HashedFile {
    path: String,
    hash: String,
}

impl std::fmt::Display for HashedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "name: {}, hash: {}", self.path, self.hash)
    }
}

impl HashedFile {
    pub fn new(path: &str, hash: &str) -> HashedFile {
        HashedFile {
            path: path.to_string(),
            hash: hash.to_string(),
        }
    }

    pub fn get_path(&self) -> &str {
        &self.path
    }

    pub fn get_hash(&self) -> &str {
        &self.hash
    }
}

#[derive(PartialEq)]
pub enum RequestVersion {
    ZEROpOne,
}

impl core::fmt::Display for RequestVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestVersion::ZEROpOne => write!(f, "0.1"),
        } 
    }
}

#[derive(PartialEq)]
pub enum RequestType {
    GetHashes,
    GetFiles,
    GiveHashes,
    GiveFiles,
    Chunk,
    EndFile,
    Disconnect,
}

impl core::fmt::Display for RequestType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestType::GetFiles => write!(f, "Get Files"),
            RequestType::GetHashes => write!(f, "Get Hashes"),
            RequestType::GiveHashes => write!(f, "Give Hashes"),
            RequestType::GiveFiles => write!(f, "Give Files"),
            RequestType::Chunk => write!(f, "Chunk"),
            RequestType::EndFile => write!(f, "End File"),
            RequestType::Disconnect => write!(f, "Disconnect"),
        }
    }
}

pub struct Request {
    version: RequestVersion,
    request_type: RequestType,
    file_name_size: usize,
    body_size: usize,
}

impl Request {
    pub fn new(version: RequestVersion, request_type: RequestType,
            file_name_size: usize, body_size: usize) -> Request {
        Request { version, request_type, file_name_size, body_size }
    }

    pub fn get_version(&self) -> &RequestVersion {
        &self.version
    }

    pub fn get_type(&self) -> &RequestType {
        &self.request_type
    }

    pub fn get_file_name_size(&self) -> &usize {
        &self.file_name_size
    }

    pub fn get_body_size(&self) -> &usize {
        &self.body_size
    }
}

impl core::fmt::Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Version: {}\nType: {}\nFile name size: {}\nBody size: {}", self.get_version(), self.get_type(), self.get_file_name_size(), self.get_body_size())
    }
}

#[derive(PartialEq)]
pub enum FileState {
    Present,
    Missing,
    Corrupted,
}

impl core::fmt::Display for FileState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileState::Corrupted => write!(f, "Corrupted"),
            FileState::Missing => write!(f, "Missing"),
            FileState::Present => write!(f, "Present"),
        }
    }
}

pub fn create_header(version: RequestVersion, reqeuest_type: RequestType, file_name_size: u32, body_size: u32) -> [u8; 64] {
    let mut buffer = [0u8; 64];

    let mut header_text = String::from("repairman ");
    
    match version {
        RequestVersion::ZEROpOne => header_text.push_str("0.1 "),
    }

    match reqeuest_type {
        RequestType::GetHashes => header_text.push_str("GET-HASHES"),
        RequestType::GetFiles => header_text.push_str("GET-FILES"),
        RequestType::GiveHashes => header_text.push_str("GIVE-HASHES"),
        RequestType::GiveFiles => header_text.push_str("GIVE-FILES"),
        RequestType::Chunk => header_text.push_str("CHUNK"),
        RequestType::EndFile => header_text.push_str("END-FILE"),
        RequestType::Disconnect => header_text.push_str("DISCONNECT"),
    }

    let bytes = header_text.as_bytes();

    let len = bytes.len().min(56);
    buffer[..len].copy_from_slice(&bytes[..len]);

    buffer[56..60].copy_from_slice(&file_name_size.to_be_bytes());
    buffer[60..64].copy_from_slice(&body_size.to_be_bytes());

    buffer
}

pub async fn async_parse_request(stream: &mut tokio::net::TcpStream) -> std::io::Result<Request> {
    use tokio::io::AsyncReadExt;

    let mut header = [0u8; 64];
    stream.read_exact(&mut header).await?;

    let body_size = u32::from_be_bytes(header[60..64].try_into().map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "Couldn't read out body size from header."))?) as usize;
    let file_name_size = u32::from_be_bytes(header[56..60].try_into().map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "Couldn't read out file name size from header."))?) as usize;

    let request_line = String::from_utf8_lossy(&header[0..56]);
    let request_line = request_line.trim_matches(char::from(0));

    let mut sperate = request_line.split(" ");

    if let Some(name) = sperate.next() {
        if name != "repairman" {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Protocol name invalid."));
        }
    } else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Header is empty."));
    }

    let version = match sperate.next() {
        Some("0.1") => RequestVersion::ZEROpOne,
        _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Version in header is wrong.")),
    };

    let request_type = match sperate.next() {
        Some(t) => {
            match t {
                "GIVE-HASHES" => RequestType::GiveHashes,
                "GIVE-FILES" => RequestType::GiveFiles,
                "GET-HASHES" => return Ok(Request::new(version, RequestType::GetHashes, 0, 0)),
                "GET-FILES" => RequestType::GetFiles,
                "CHUNK" => RequestType::Chunk,
                "END-FILE" => RequestType::EndFile,
                "DISCONNECT" => RequestType::Disconnect,
                _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid request type was recieved.")),
            }
        }
        None => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Header incomplete, request type wasn't recieved.")),
    };

    if request_type == RequestType::GiveHashes {
        return Ok(Request::new(version, request_type, 0, body_size));
    }

    Ok(Request::new(version, request_type, file_name_size, body_size))
}
