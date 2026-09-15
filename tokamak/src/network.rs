use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::thread::{self, ThreadId};
use std::time::Duration;

use openssl::ssl::{SslConnector, SslMethod, SslStream};
use thiserror::Error;
use url::Url;

const MAX_HEADERS: usize = 64 * 1024;
const MAX_BODY: usize = 250 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const SOCKET_BUFFER: usize = 16 * 1024;

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("network IO failed: {0}")]
    Io(#[from] io::Error),
    #[error("TLS failed: {0}")]
    Tls(String),
    #[error("HTTP response is invalid: {0}")]
    InvalidResponse(String),
    #[error("socket {0} is not open")]
    MissingSocket(u64),
    #[error("redirect limit exceeded")]
    RedirectLimit,
}

pub(crate) struct FetchRequest {
    pub(crate) url: String,
    pub(crate) method: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
    pub(crate) redirect: String,
}

pub(crate) struct FetchResponse {
    pub(crate) url: String,
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
    pub(crate) redirected: bool,
}

pub(crate) fn fetch(request: FetchRequest) -> Result<FetchResponse, Error> {
    let mut request = request;
    for redirect_count in 0..=20 {
        let response = send_once(&request)?;
        let Some(location) = redirect_location(&response) else {
            return Ok(FetchResponse {
                redirected: redirect_count > 0,
                ..response
            });
        };
        match request.redirect.as_str() {
            "error" => return Err(Error::InvalidResponse("redirect was rejected".to_owned())),
            "manual" => {
                return Ok(FetchResponse {
                    redirected: false,
                    ..response
                });
            }
            _ => {}
        }
        if redirect_count == 20 {
            return Err(Error::RedirectLimit);
        }
        request = redirected_request(request, response.status, &location)?;
    }
    Err(Error::RedirectLimit)
}

fn redirect_location(response: &FetchResponse) -> Option<String> {
    matches!(response.status, 301 | 302 | 303 | 307 | 308)
        .then(|| response.headers.get("location").cloned())
        .flatten()
}

fn redirected_request(
    mut request: FetchRequest,
    status: u16,
    location: &str,
) -> Result<FetchRequest, Error> {
    let current = Url::parse(&request.url).map_err(|error| Error::InvalidUrl(error.to_string()))?;
    request.url = current
        .join(location)
        .map_err(|error| Error::InvalidUrl(error.to_string()))?
        .to_string();
    if matches!(status, 301..=303) && request.method != "GET" && request.method != "HEAD" {
        request.method = "GET".to_owned();
        request.body.clear();
        request.headers.remove("content-type");
    }
    Ok(request)
}

fn send_once(request: &FetchRequest) -> Result<FetchResponse, Error> {
    let url = Url::parse(&request.url).map_err(|error| Error::InvalidUrl(error.to_string()))?;
    let host = url
        .host_str()
        .ok_or_else(|| Error::InvalidUrl("host is missing".to_owned()))?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(Error::InvalidUrl(format!("unsupported scheme '{scheme}'")));
    }
    let port = url
        .port_or_known_default()
        .ok_or_else(|| Error::InvalidUrl("port is missing".to_owned()))?;
    let stream = connect(host, port, scheme == "https")?;
    write_request(stream, request, &url, host, port)
}

fn connect(host: &str, port: u16, secure: bool) -> Result<Connection, Error> {
    let address = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| Error::InvalidUrl("host has no addresses".to_owned()))?;
    let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(READ_TIMEOUT))?;
    if !secure {
        return Ok(Connection::Plain(stream));
    }
    let connector = SslConnector::builder(SslMethod::tls())
        .map_err(|error| Error::Tls(error.to_string()))?
        .build();
    connector
        .connect(host, stream)
        .map(Connection::Tls)
        .map_err(|error| Error::Tls(error.to_string()))
}

fn write_request(
    mut stream: Connection,
    request: &FetchRequest,
    url: &Url,
    host: &str,
    port: u16,
) -> Result<FetchResponse, Error> {
    let path = request_target(url);
    write!(stream, "{} {path} HTTP/1.1\r\n", request.method)?;
    for (name, value) in &request.headers {
        if is_hop_by_hop(name)
            || matches!(name.as_str(), "host" | "content-length" | "connection")
        {
            continue;
        }
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(stream, "Host: {}\r\nConnection: close\r\n", host_header(url, host, port))?;
    write!(stream, "Content-Length: {}\r\n\r\n", request.body.len())?;
    stream.write_all(&request.body)?;
    stream.flush()?;
    read_response(&mut stream, &request.method, url.to_string())
}

fn request_target(url: &Url) -> String {
    let mut target = if url.path().is_empty() {
        "/".to_owned()
    } else {
        url.path().to_owned()
    };
    if let Some(query) = url.query() {
        target.push('?');
        target.push_str(query);
    }
    target
}

fn host_header(url: &Url, host: &str, port: u16) -> String {
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    if url.port_or_known_default() == Some(port) && url.port().is_none() {
        host
    } else {
        format!("{host}:{port}")
    }
}

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn read_response(
    stream: &mut Connection,
    method: &str,
    url: String,
) -> Result<FetchResponse, Error> {
    loop {
        let header_block = read_header_block(stream)?;
        let (status, status_text, headers) = parse_headers(&header_block)?;
        if (100..200).contains(&status) && status != 101 {
            continue;
        }
        let body = read_body(stream, method, status, &headers)?;
        return Ok(FetchResponse {
            url,
            status,
            status_text,
            headers,
            body,
            redirected: false,
        });
    }
}

fn read_header_block(stream: &mut impl Read) -> Result<Vec<u8>, Error> {
    let mut data = Vec::new();
    loop {
        let mut byte = [0; 1];
        stream.read_exact(&mut byte)?;
        data.push(byte[0]);
        if data.len() > MAX_HEADERS {
            return Err(Error::InvalidResponse("headers exceed the limit".to_owned()));
        }
        if data.ends_with(b"\r\n\r\n") {
            return Ok(data);
        }
    }
}

fn parse_headers(data: &[u8]) -> Result<(u16, String, BTreeMap<String, String>), Error> {
    let text = std::str::from_utf8(data)
        .map_err(|_| Error::InvalidResponse("headers are not UTF-8".to_owned()))?;
    let mut lines = text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| Error::InvalidResponse("status line is missing".to_owned()))?;
    let mut status_parts = status_line.splitn(3, ' ');
    let version = status_parts.next();
    if version != Some("HTTP/1.1") && version != Some("HTTP/1.0") {
        return Err(Error::InvalidResponse("HTTP version is invalid".to_owned()));
    }
    let status = status_parts
        .next()
        .ok_or_else(|| Error::InvalidResponse("status is missing".to_owned()))?
        .parse::<u16>()
        .map_err(|_| Error::InvalidResponse("status is invalid".to_owned()))?;
    let status_text = status_parts.next().unwrap_or_default().to_owned();
    let mut headers = BTreeMap::new();
    for line in lines.take_while(|line| !line.is_empty()) {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| Error::InvalidResponse("header is invalid".to_owned()))?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();
        headers
            .entry(name)
            .and_modify(|current: &mut String| {
                current.push_str(", ");
                current.push_str(&value);
            })
            .or_insert(value);
    }
    Ok((status, status_text, headers))
}

fn read_body(
    stream: &mut impl Read,
    method: &str,
    status: u16,
    headers: &BTreeMap<String, String>,
) -> Result<Vec<u8>, Error> {
    if method.eq_ignore_ascii_case("HEAD")
        || (100..200).contains(&status)
        || matches!(status, 204 | 304)
    {
        return Ok(Vec::new());
    }
    if headers
        .get("transfer-encoding")
        .is_some_and(|value| value.split(',').any(|item| item.trim() == "chunked"))
    {
        return read_chunked_body(stream);
    }
    if let Some(length) = headers.get("content-length") {
        let length = length
            .parse::<usize>()
            .map_err(|_| Error::InvalidResponse("content length is invalid".to_owned()))?;
        if length > MAX_BODY {
            return Err(Error::InvalidResponse("body exceeds the limit".to_owned()));
        }
        let mut body = vec![0; length];
        stream.read_exact(&mut body)?;
        return Ok(body);
    }
    let mut body = Vec::new();
    stream.take((MAX_BODY + 1) as u64).read_to_end(&mut body)?;
    if body.len() > MAX_BODY {
        return Err(Error::InvalidResponse("body exceeds the limit".to_owned()));
    }
    Ok(body)
}

fn read_chunked_body(stream: &mut impl Read) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    loop {
        let line = read_line(stream)?;
        let size = line
            .split(';')
            .next()
            .unwrap_or_default()
            .trim();
        let size = usize::from_str_radix(size, 16)
            .map_err(|_| Error::InvalidResponse("chunk size is invalid".to_owned()))?;
        if size == 0 {
            loop {
                if read_line(stream)?.is_empty() {
                    return Ok(body);
                }
            }
        }
        if body.len().saturating_add(size) > MAX_BODY {
            return Err(Error::InvalidResponse("body exceeds the limit".to_owned()));
        }
        let start = body.len();
        body.resize(start + size, 0);
        stream.read_exact(&mut body[start..])?;
        let mut terminator = [0; 2];
        stream.read_exact(&mut terminator)?;
        if terminator != *b"\r\n" {
            return Err(Error::InvalidResponse("chunk is not terminated".to_owned()));
        }
    }
}

fn read_line(stream: &mut impl Read) -> Result<String, Error> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0; 1];
        stream.read_exact(&mut byte)?;
        line.push(byte[0]);
        if line.len() > MAX_HEADERS {
            return Err(Error::InvalidResponse("line exceeds the limit".to_owned()));
        }
        if line.ends_with(b"\r\n") {
            line.truncate(line.len() - 2);
            return String::from_utf8(line)
                .map_err(|_| Error::InvalidResponse("line is not UTF-8".to_owned()));
        }
    }
}

enum Connection {
    Plain(TcpStream),
    Tls(SslStream<TcpStream>),
}

impl Read for Connection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buffer),
            Self::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for Connection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buffer),
            Self::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

type SocketStore = HashMap<ThreadId, HashMap<u64, Connection>>;
static SOCKETS: OnceLock<Mutex<SocketStore>> = OnceLock::new();

#[derive(Clone)]
pub(crate) struct CacheEntry {
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
    pub(crate) url: String,
    pub(crate) redirected: bool,
    pub(crate) response_type: String,
}

type CacheStore = HashMap<String, BTreeMap<String, CacheEntry>>;
static CACHES: OnceLock<Mutex<CacheStore>> = OnceLock::new();

fn sockets() -> &'static Mutex<SocketStore> {
    SOCKETS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn socket_connect(host: &str, port: u16, secure: bool) -> Result<u64, Error> {
    let connection = connect(host, port, secure)?;
    let mut store = sockets().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let entries = store.entry(thread::current().id()).or_default();
    let id = entries.keys().max().copied().unwrap_or(0).saturating_add(1);
    entries.insert(id, connection);
    Ok(id)
}

pub(crate) fn socket_start_tls(id: u64, host: &str) -> Result<(), Error> {
    let mut store = sockets().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let entries = store
        .get_mut(&thread::current().id())
        .ok_or(Error::MissingSocket(id))?;
    let connection = entries.remove(&id).ok_or(Error::MissingSocket(id))?;
    let Connection::Plain(stream) = connection else {
        return Err(Error::Tls("socket is already secure".to_owned()));
    };
    let connector = SslConnector::builder(SslMethod::tls())
        .map_err(|error| Error::Tls(error.to_string()))?
        .build();
    let connection = connector
        .connect(host, stream)
        .map(Connection::Tls)
        .map_err(|error| Error::Tls(error.to_string()))?;
    entries.insert(id, connection);
    Ok(())
}

pub(crate) fn socket_read(id: u64) -> Result<Option<Vec<u8>>, Error> {
    let mut store = sockets().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let connection = store
        .get_mut(&thread::current().id())
        .and_then(|entries| entries.get_mut(&id))
        .ok_or(Error::MissingSocket(id))?;
    let mut buffer = vec![0; SOCKET_BUFFER];
    let count = connection.read(&mut buffer)?;
    if count == 0 {
        Ok(None)
    } else {
        buffer.truncate(count);
        Ok(Some(buffer))
    }
}

pub(crate) fn socket_write(id: u64, data: &[u8]) -> Result<(), Error> {
    let mut store = sockets().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let connection = store
        .get_mut(&thread::current().id())
        .and_then(|entries| entries.get_mut(&id))
        .ok_or(Error::MissingSocket(id))?;
    connection.write_all(data)?;
    connection.flush()?;
    Ok(())
}

pub(crate) fn socket_close(id: u64) {
    let mut store = sockets().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(entries) = store.get_mut(&thread::current().id()) {
        entries.remove(&id);
        if entries.is_empty() {
            store.remove(&thread::current().id());
        }
    }
}

pub(crate) fn cleanup_thread_sockets() {
    sockets()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&thread::current().id());
}

fn caches() -> &'static Mutex<CacheStore> {
    CACHES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_scope(path: &str, name: &str) -> String {
    format!("{path}\0{name}")
}

pub(crate) fn cache_match(path: &str, name: &str, key: &str) -> Option<CacheEntry> {
    caches()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&cache_scope(path, name))
        .and_then(|entries| entries.get(key).cloned())
}

pub(crate) fn cache_put(path: &str, name: &str, key: String, entry: CacheEntry) {
    caches()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(cache_scope(path, name))
        .or_default()
        .insert(key, entry);
}

pub(crate) fn cache_delete(path: &str, name: &str, key: &str) -> bool {
    let mut caches = caches()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let scope = cache_scope(path, name);
    let Some(entries) = caches.get_mut(&scope) else {
        return false;
    };
    let deleted = entries.remove(key).is_some();
    if entries.is_empty() {
        caches.remove(&scope);
    }
    deleted
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::{FetchRequest, fetch, parse_headers, read_body};

    #[test]
    fn parses_http_status_and_combines_headers() -> Result<(), Box<dyn std::error::Error>> {
        let (status, reason, headers) = parse_headers(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Test: one\r\nx-test: two\r\n\r\n",
        )?;
        assert_eq!(status, 200);
        assert_eq!(reason, "OK");
        assert_eq!(headers.get("content-type"), Some(&"text/plain".to_owned()));
        assert_eq!(headers.get("x-test"), Some(&"one, two".to_owned()));
        Ok(())
    }

    #[test]
    fn reads_chunked_http_bodies() -> Result<(), Box<dyn std::error::Error>> {
        let mut stream = Cursor::new(b"4\r\ntest\r\n0\r\nTrailer: value\r\n\r\n".to_vec());
        let body = read_body(
            &mut stream,
            "GET",
            200,
            &[("transfer-encoding".to_owned(), "chunked".to_owned())]
                .into_iter()
                .collect(),
        )?;
        assert_eq!(body, b"test");
        Ok(())
    }

    #[test]
    fn fetches_an_http_response_and_follows_redirects() -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let server = thread::spawn(move || -> Result<(), std::io::Error> {
            let (mut first, _) = listener.accept()?;
            read_request(&mut first)?;
            first.write_all(
                b"HTTP/1.1 302 Found\r\nLocation: /done\r\nContent-Length: 0\r\n\r\n",
            )?;
            first.flush()?;
            let (mut second, _) = listener.accept()?;
            read_request(&mut second)?;
            second.write_all(b"HTTP/1.1 200 OK\r\nX-Test: value\r\nContent-Length: 4\r\n\r\ndone")?;
            second.flush()
        });
        let response = fetch(FetchRequest {
            url: format!("http://127.0.0.1:{port}/start"),
            method: "GET".to_owned(),
            headers: std::collections::BTreeMap::new(),
            body: Vec::new(),
            redirect: "follow".to_owned(),
        })?;
        assert_eq!(response.status, 200);
        assert_eq!(response.url, format!("http://127.0.0.1:{port}/done"));
        assert!(response.redirected);
        assert_eq!(response.headers.get("x-test"), Some(&"value".to_owned()));
        assert_eq!(response.body, b"done");
        server.join().map_err(|_| "server thread panicked")??;
        Ok(())
    }

    fn read_request(stream: &mut impl Read) -> Result<(), std::io::Error> {
        let mut request = Vec::new();
        loop {
            let mut byte = [0; 1];
            stream.read_exact(&mut byte)?;
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                return Ok(());
            }
        }
    }
}
