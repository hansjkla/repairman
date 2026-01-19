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
}

impl core::fmt::Display for RequestType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestType::GetFiles => write!(f, "Get Files"),
            RequestType::GetHashes => write!(f, "Get Hashes"),
            RequestType::GiveHashes => write!(f, "Give Hashes"),
            RequestType::GiveFiles => write!(f, "Give Hashes"),
        }
    }
}

pub struct Request {
    version: RequestVersion,
    request_type: RequestType,
    file_name_size: Option<usize>,
    body: Option<Vec<u8>>,
}

impl Request {
    pub fn new(version: RequestVersion, request_type: RequestType,
            file_name_size: Option<usize>, body: Option<Vec<u8>>) -> Request {
        Request { version, request_type, file_name_size, body }
    }

    pub fn get_version(&self) -> &RequestVersion {
        &self.version
    }

    pub fn get_type(&self) -> &RequestType {
        &self.request_type
    }

    pub fn get_file_name_size(&self) -> &Option<usize> {
        &self.file_name_size
    }

    pub fn get_body(&self) -> &Option<Vec<u8>> {
        &self.body
    }
}

impl core::fmt::Display for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(body) = &self.body {
            let body =  String::from_utf8(body.clone()).unwrap();
            return write!(f, "Version: {}\nType: {}\nBody: {}", self.version, self.request_type, body);
        }

        write!(f, "Version: {}\nType: {}\nBody: NO-BODY", self.version, self.request_type)
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
    }

    let bytes = header_text.as_bytes();

    let len = bytes.len().min(56);
    buffer[..len].copy_from_slice(&bytes[..len]);

    buffer[56..60].copy_from_slice(&file_name_size.to_be_bytes());
    buffer[60..64].copy_from_slice(&body_size.to_be_bytes());

    buffer
}

pub fn parse_request(mut stream: &std::net::TcpStream) -> std::io::Result<Request> {
    use std::io::Read;

    let mut header = [0u8; 64];
    stream.read_exact(&mut header)?;

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
                "GET-HASHES" => return Ok(Request::new(version, RequestType::GetHashes, None, None)),
                "GET-FILES" => RequestType::GetFiles,
                _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid request type was recieved.")),
            }
        }
        None => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Header incomplete, request type wasn't recieved.")),
    };

    let mut body = vec![0u8; body_size];
    stream.read_exact(&mut body)?;

    if request_type == RequestType::GiveHashes {
        return Ok(Request::new(version, request_type, None, Some(body)));
    }

    Ok(Request::new(version, request_type, Some(file_name_size), Some(body)))
}

pub struct NewRequest {
    version: RequestVersion,
    request_type: RequestType,
    file_name_size: Option<usize>,
    body_size: Option<usize>,
}

impl NewRequest {
    pub fn new(version: RequestVersion, request_type: RequestType,
            file_name_size: Option<usize>, body_size: Option<usize>) -> NewRequest {
        NewRequest { version, request_type, file_name_size, body_size }
    }

    pub fn get_version(&self) -> &RequestVersion {
        &self.version
    }

    pub fn get_type(&self) -> &RequestType {
        &self.request_type
    }

    pub fn get_file_name_size(&self) -> &Option<usize> {
        &self.file_name_size
    }

    pub fn get_body_size(&self) -> &Option<usize> {
        &self.body_size
    }
}


pub async fn async_parse_request(stream: &mut tokio::net::TcpStream) -> std::io::Result<NewRequest> {
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
                "GET-HASHES" => return Ok(NewRequest::new(version, RequestType::GetHashes, None, None)),
                "GET-FILES" => RequestType::GetFiles,
                _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid request type was recieved.")),
            }
        }
        None => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Header incomplete, request type wasn't recieved.")),
    };

    if request_type == RequestType::GiveHashes {
        return Ok(NewRequest::new(version, request_type, None, Some(body_size)));
    }

    Ok(NewRequest::new(version, request_type, Some(file_name_size), Some(body_size)))
}
