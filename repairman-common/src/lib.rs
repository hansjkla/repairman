use std::io;

pub struct FileToSendInfo {
    path: String,
    is_empty: bool,
}

impl FileToSendInfo {
    pub fn new(path: &str, is_empty: bool) -> FileToSendInfo {
        FileToSendInfo { path: path.to_string(), is_empty }
    }

    pub fn get_path(&self) -> &str {
        &self.path
    }

    pub fn is_empty(&self) -> bool {
        self.is_empty
    }
}

#[derive(PartialEq, Eq, Debug, Hash)]
pub struct HashedFile {
    path: String,
    hash: String,
    is_empty: bool,
}

impl std::fmt::Display for HashedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "name: {}, hash: {}, is_empty: {}", self.path, self.hash, self.is_empty)
    }
}

impl HashedFile {
    pub fn new(path: &str, hash: &str, is_empty: bool) -> HashedFile {
        HashedFile {
            path: path.to_string(),
            hash: hash.to_string(),
            is_empty,
        }
    }

    pub fn get_path(&self) -> &str {
        &self.path
    }

    pub fn get_hash(&self) -> &str {
        &self.hash
    }

    pub fn is_empty(&self) -> bool {
        self.is_empty
    }
}

#[derive(PartialEq)]
pub enum RequestVersion {
    ZEROpOne,
    ZEROpTwo,
    ZEROpThree,
    ZEROpFour,
}

impl core::fmt::Display for RequestVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestVersion::ZEROpOne => write!(f, "0.1"),
            RequestVersion::ZEROpTwo => write!(f, "0.2"),
            RequestVersion::ZEROpThree => write!(f, "0.3"),
            RequestVersion::ZEROpFour => write!(f, "0.4"),
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
    EmptyFile,
}

impl core::fmt::Display for RequestType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestType::GetFiles => write!(f, "Get Files"),
            RequestType::GetHashes => write!(f, "Get Hashes"),
            RequestType::GiveHashes => write!(f, "Give Hashes"),
            RequestType::GiveFiles => write!(f, "Give Files"),
            RequestType::Chunk => write!(f, "Chunk"),
            RequestType::EmptyFile => write!(f, "Empty File"),
            RequestType::EndFile => write!(f, "End File"),
            RequestType::Disconnect => write!(f, "Disconnect"),
        }
    }
}

pub struct Request {
    version: RequestVersion,
    request_type: RequestType,
    body_size: usize,
}

impl Request {
    pub fn new(version: RequestVersion, request_type: RequestType, body_size: usize) -> Request {
        Request { version, request_type, body_size }
    }

    pub fn get_version(&self) -> &RequestVersion {
        &self.version
    }

    pub fn get_type(&self) -> &RequestType {
        &self.request_type
    }

    pub fn get_body_size(&self) -> &usize {
        &self.body_size
    }
}

impl core::fmt::Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Version: {}\nType: {}\nBody size: {}", self.get_version(), self.get_type(), self.get_body_size())
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

pub fn create_header(version: RequestVersion, reqeuest_type: RequestType, body_size: u32) -> [u8; 16] {
    let mut buffer = [0u8; 16];

    buffer[..4].copy_from_slice("rpmn".as_bytes());
    
    match version {
        RequestVersion::ZEROpOne   => buffer[4] = 1,
        RequestVersion::ZEROpTwo   => buffer[4] = 2,
        RequestVersion::ZEROpThree => buffer[4] = 3,
        RequestVersion::ZEROpFour  => buffer[4] = 4,
    }

    match reqeuest_type {
        RequestType::GetHashes  => buffer[5] = 1,
        RequestType::GetFiles   => buffer[5] = 2,
        RequestType::GiveHashes => buffer[5] = 3,
        RequestType::GiveFiles  => buffer[5] = 4,
        RequestType::Chunk      => buffer[5] = 5,
        RequestType::EndFile    => buffer[5] = 6,
        RequestType::Disconnect => buffer[5] = 7,
        RequestType::EmptyFile  => buffer[5] = 8,
    }

    buffer[6..10].copy_from_slice(&body_size.to_be_bytes());

    buffer
}

pub async fn async_parse_request(stream: &mut tokio::net::TcpStream) -> std::io::Result<Request> {
    use tokio::io::AsyncReadExt;

    let mut header = [0u8; 16];
    stream.read_exact(&mut header).await?;

    if header[..4] != *"rpmn".as_bytes() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "The first four bytes of a header aren't 'rpmn'."));
    }

    let version = u8::from_be(header[4]);
    let version = match version {
        1 => RequestVersion::ZEROpOne,
        2 => RequestVersion::ZEROpTwo,
        3 => RequestVersion::ZEROpThree,
        4 => RequestVersion::ZEROpFour,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Couldn't read a valid request version from header.")),
    };

    let request_type = u8::from_be(header[5]);
    let request_type = match request_type {
        1 => RequestType::GetHashes,
        2 => RequestType::GetFiles,
        3 => RequestType::GiveHashes,
        4 => RequestType::GiveFiles,
        5 => RequestType::Chunk,
        6 => RequestType::EndFile,
        7 => RequestType::Disconnect,
        8 => RequestType::EmptyFile,
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Couldn't read a valid request type from header.")),
    };

    let body_size = u32::from_be_bytes(header[6..10].try_into()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Couldn't read a valid body size from header: {}", e)))?) as usize;

    Ok(Request::new(version, request_type, body_size))
}
