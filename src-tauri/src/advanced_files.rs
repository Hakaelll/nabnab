use crate::model::Progress;
use reqwest::{header, redirect::Policy, Response, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tauri::AppHandle;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    process::Command,
    sync::{watch, Semaphore},
    task::JoinSet,
};

const MAX_BYTES: u64 = 128 * 1024 * 1024 * 1024;
const STATE: &str = ".advanced-http.json";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileProbe {
    pub filename: String,
    pub host: String,
    pub size: Option<u64>,
    pub supports_ranges: bool,
}
#[derive(Clone)]
struct Remote {
    url: String,
    probe: FileProbe,
    etag: Option<String>,
    allowed_origin: String,
    network: Network,
}
#[derive(Clone, Copy)]
enum Network {
    Public,
    #[cfg(test)]
    Fixture(SocketAddr),
}
#[derive(Serialize, Deserialize)]
struct Resume {
    source_hash: String,
    filename: String,
    size: Option<u64>,
    etag: Option<String>,
    checksum: Option<String>,
    completed: bool,
}

pub fn validate_checksum(value: Option<&str>) -> Result<Option<String>, String> {
    value.map(|value| if value.len()==64 && value.bytes().all(|byte|byte.is_ascii_hexdigit()) {Ok(value.to_ascii_lowercase())}else{Err("Enter exactly 64 hexadecimal characters for the expected SHA-256, or leave it blank.".into())}).transpose()
}
fn validate_connections(value: u8) -> Result<(), String> {
    if !(1..=16).contains(&value) {
        return Err("Choose between 1 and 16 HTTP connections.".into());
    }
    Ok(())
}
fn parsed(input: &str) -> Result<url::Url, String> {
    if input.len() > 8192 || input.chars().any(char::is_control) {
        return Err("The file URL exceeds its limit or contains control characters.".into());
    }
    url::Url::parse(&crate::model::validate_url(input)?).map_err(|_| "Invalid file URL.".into())
}
async fn public_addresses(url: &url::Url, network: Network) -> Result<Vec<SocketAddr>, String> {
    #[cfg(test)]
    if let Network::Fixture(address) = network {
        if address.ip().is_loopback()
            && url.scheme() == "http"
            && url.host_str() == Some("fixture.example.org")
            && url.port() == Some(address.port())
        {
            return Ok(vec![address]);
        }
    }
    let _ = network;
    let port = url
        .port_or_known_default()
        .ok_or("Unsupported file URL scheme.")?;
    let addresses: Vec<_> = match url.host().ok_or("The file URL has no host.")? {
        url::Host::Ipv4(ip) => vec![SocketAddr::new(ip.into(), port)],
        url::Host::Ipv6(ip) => vec![SocketAddr::new(ip.into(), port)],
        url::Host::Domain(host) => tokio::time::timeout(
            Duration::from_secs(8),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| "File source DNS lookup timed out.")?
        .map_err(|_| "The file source hostname could not be resolved.")?
        .collect(),
    };
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !crate::providers::is_public_ip(address.ip()))
    {
        return Err("Local or reserved file source addresses are not supported.".into());
    }
    Ok(addresses)
}
async fn fetched(
    input: &str,
    range: Option<&str>,
    etag: Option<&str>,
    allowed_origin: Option<&str>,
    cancel: watch::Receiver<bool>,
    network: Network,
) -> Result<(url::Url, Response), String> {
    let work = async {
        let mut url = parsed(input)?;
        for hop in 0..=5 {
            if allowed_origin.is_some_and(|origin| url.origin().ascii_serialization() != origin) {
                return Err("The file source changed its redirect origin. Retry the job to inspect it again.".into());
            }
            let addresses = public_addresses(&url, network).await?;
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(24 * 60 * 60))
                .resolve_to_addrs(
                    url.host_str().ok_or("The file URL has no host.")?,
                    &addresses,
                )
                .user_agent(concat!("nabnab/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|_| "Could not prepare the file HTTP client.")?;
            let mut request = client
                .get(url.clone())
                .header(header::ACCEPT_ENCODING, "identity");
            if let Some(range) = range {
                request = request.header(header::RANGE, range);
            }
            if let Some(etag) = etag {
                request = request.header(header::IF_MATCH, etag);
            }
            let response = request
                .send()
                .await
                .map_err(|_| "The file HTTP request failed. Check the connection and retry.")?;
            if response.status().is_redirection() {
                if hop == 5 {
                    return Err("The file source exceeded its five-redirect limit.".into());
                }
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or("The file source returned an invalid redirect.")?;
                let next = url
                    .join(location)
                    .map_err(|_| "The file source returned an invalid redirect URL.")?;
                if url.scheme() == "https" && next.scheme() == "http" {
                    return Err("HTTPS file sources cannot redirect to HTTP.".into());
                }
                url = parsed(next.as_str())?;
                continue;
            }
            if response
                .headers()
                .get(header::CONTENT_ENCODING)
                .is_some_and(|value| value.as_bytes() != b"identity")
            {
                return Err("The file server ignored identity encoding. A byte-exact download cannot be verified.".into());
            }
            return Ok((url, response));
        }
        Err("The file source exceeded its redirect limit.".into())
    };
    tokio::select! {result=work=>result,_=cancelled(cancel)=>Err(crate::engine::CANCELLED.into())}
}
async fn cancelled(mut cancel: watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
fn content_range(value: &str) -> Option<(u64, u64, u64)> {
    let range = value.strip_prefix("bytes ")?;
    let (range, total) = range.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end, total) = (
        start.parse::<u64>().ok()?,
        end.parse::<u64>().ok()?,
        total.parse::<u64>().ok()?,
    );
    (start <= end && end < total && total <= MAX_BYTES).then_some((start, end, total))
}
fn strong_etag(value: &str) -> bool {
    value.len() >= 2
        && value.len() <= 512
        && value.starts_with('"')
        && value.ends_with('"')
        && value.as_bytes()[1..value.len() - 1]
            .iter()
            .all(|byte| *byte == 0x21 || (0x23..=0x7e).contains(byte))
}
fn percent_decoded(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] == b'%' {
            let part = bytes.get(cursor + 1..cursor + 3)?;
            let part = std::str::from_utf8(part).ok()?;
            result.push(u8::from_str_radix(part, 16).ok()?);
            cursor += 3;
        } else {
            result.push(bytes[cursor]);
            cursor += 1;
        }
    }
    String::from_utf8(result).ok()
}
fn safe_filename(value: &str) -> String {
    if let Some((stem, extension)) = value.rsplit_once('.') {
        if !extension.is_empty()
            && extension.len() <= 16
            && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        {
            let stem: String = crate::options::safe_segment(stem)
                .chars()
                .take(100)
                .collect();
            return format!("{stem}.{extension}");
        }
    }
    crate::options::safe_segment(value)
}
fn filename(url: &url::Url, disposition: Option<&str>) -> String {
    let mut ordinary = None;
    if let Some(disposition) = disposition.filter(|value| value.len() <= 4096) {
        for part in disposition.split(';').skip(1) {
            let Some((key, value)) = part.trim().split_once('=') else {
                continue;
            };
            if key.eq_ignore_ascii_case("filename*") {
                if let Some(value) = value
                    .strip_prefix("UTF-8''")
                    .or_else(|| value.strip_prefix("utf-8''"))
                {
                    if let Some(value) = percent_decoded(value) {
                        return safe_filename(&value);
                    }
                }
            } else if key.eq_ignore_ascii_case("filename") {
                ordinary = Some(value.trim_matches('"').to_owned());
            }
        }
    }
    let proposed = ordinary
        .or_else(|| {
            url.path_segments()?
                .next_back()
                .filter(|name| !name.is_empty())
                .and_then(percent_decoded)
        })
        .unwrap_or_else(|| "download.bin".into());
    safe_filename(&proposed)
}
fn owned_file(path: &Path) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("Could not inspect the file job's owned data.".into()),
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("File job data cannot be a reparse point.".into());
        }
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("File job data must be a regular owned file.".into());
    }
    Ok(true)
}
fn probe_response(
    url: &url::Url,
    status: StatusCode,
    headers: &header::HeaderMap,
) -> Result<FileProbe, String> {
    let range = headers
        .get(header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(content_range);
    let length = headers
        .get(header::CONTENT_LENGTH)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .ok_or("The file server returned an invalid Content-Length.")
        })
        .transpose()?;
    if status != StatusCode::OK && status != StatusCode::PARTIAL_CONTENT {
        return Err(format!(
            "File inspection returned HTTP {}.",
            status.as_u16()
        ));
    }
    if status == StatusCode::PARTIAL_CONTENT
        && (!range.is_some_and(|(start, end, _)| start == 0 && end == 0)
            || length.is_some_and(|length| length != 1))
    {
        return Err("The server returned an invalid response to the byte-range probe.".into());
    }
    let size = if status == StatusCode::PARTIAL_CONTENT {
        range.map(|(_, _, total)| total)
    } else {
        length
    };
    if size.is_some_and(|size| size > MAX_BYTES) {
        return Err("This file exceeds the 128 GiB download limit.".into());
    }
    Ok(FileProbe {
        filename: filename(
            url,
            headers
                .get(header::CONTENT_DISPOSITION)
                .and_then(|value| value.to_str().ok()),
        ),
        host: url.host_str().unwrap_or_default().into(),
        size,
        supports_ranges: status == StatusCode::PARTIAL_CONTENT,
    })
}
async fn probe(input: &str, cancel: watch::Receiver<bool>) -> Result<Remote, String> {
    let (url, response) = tokio::time::timeout(
        Duration::from_secs(30),
        fetched(
            input,
            Some("bytes=0-0"),
            None,
            None,
            cancel,
            Network::Public,
        ),
    )
    .await
    .map_err(|_| "File inspection timed out.")??;
    let info = probe_response(&url, response.status(), response.headers())?;
    let etag = response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|value| strong_etag(value))
        .map(str::to_owned);
    Ok(Remote {
        allowed_origin: url.origin().ascii_serialization(),
        url: url.to_string(),
        probe: info,
        etag,
        network: Network::Public,
    })
}
#[tauri::command]
pub async fn inspect_advanced_file(url: String) -> Result<FileProbe, String> {
    let (_sender, cancel) = watch::channel(false);
    Ok(probe(&url, cancel).await?.probe)
}

pub fn download_args(
    relay: &str,
    staging: &Path,
    info: &FileProbe,
    connections: u8,
    resume: bool,
) -> Result<Vec<String>, String> {
    validate_connections(connections)?;
    let url = url::Url::parse(relay).map_err(|_| "Invalid internal file relay.")?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || url.username() != ""
        || url.password().is_some()
    {
        return Err("File transfers require the authenticated internal loopback relay.".into());
    }
    let connections = if info.supports_ranges { connections } else { 1 };
    Ok(vec![
        "--no-conf=true".into(),
        "--no-netrc=true".into(),
        "--enable-rpc=false".into(),
        "--follow-torrent=false".into(),
        "--follow-metalink=false".into(),
        "--metalink-enable-unique-protocol=false".into(),
        "--http-proxy=".into(),
        "--https-proxy=".into(),
        "--all-proxy=".into(),
        "--no-proxy=*".into(),
        "--enable-color=false".into(),
        "--console-log-level=warn".into(),
        "--summary-interval=1".into(),
        "--human-readable=false".into(),
        "--truncate-console-readout=false".into(),
        "--auto-save-interval=1".into(),
        "--allow-overwrite=false".into(),
        "--auto-file-renaming=false".into(),
        "--content-disposition=false".into(),
        "--file-allocation=none".into(),
        "--max-tries=2".into(),
        "--retry-wait=2".into(),
        "--connect-timeout=10".into(),
        "--timeout=30".into(),
        "--use-head=true".into(),
        format!("--continue={resume}"),
        format!("--max-connection-per-server={connections}"),
        format!("--split={connections}"),
        "--min-split-size=20M".into(),
        format!("--dir={}", staging.display()),
        format!("--out={}", info.filename),
        "--".into(),
        relay.into(),
    ])
}
fn nonce() -> Result<String, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Security::Cryptography::{
            BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        };
        let mut bytes = [0u8; 32];
        if unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        } < 0
        {
            return Err("Could not secure the local file relay.".into());
        }
        return Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect());
    }
    #[cfg(not(windows))]
    {
        let mut bytes = [0u8; 32];
        fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut bytes))
            .map_err(|_| "Could not secure the local file relay.")?;
        Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}
#[derive(Debug)]
struct RelayRequest {
    head: bool,
    range: Option<String>,
}
fn relay_request(raw: &[u8], host: &str, path: &str) -> Result<RelayRequest, String> {
    if raw.len() > 8192 {
        return Err("Relay headers exceed their limit.".into());
    }
    let text = std::str::from_utf8(raw).map_err(|_| "Invalid relay request encoding.")?;
    let (header, tail) = text
        .split_once("\r\n\r\n")
        .ok_or("Incomplete relay headers.")?;
    if !tail.is_empty() {
        return Err("Relay request bodies are not accepted.".into());
    }
    let mut lines = header.split("\r\n");
    let mut first = lines.next().ok_or("Missing relay request.")?.split(' ');
    let method = first.next().ok_or("Missing relay method.")?;
    if !matches!(method, "GET" | "HEAD")
        || first.next() != Some(path)
        || first.next() != Some("HTTP/1.1")
        || first.next().is_some()
    {
        return Err("The relay accepts only authenticated GET or HEAD requests.".into());
    }
    let mut names = HashSet::new();
    let mut found_host = false;
    let mut range = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("Invalid relay header.")?;
        let name = name.to_ascii_lowercase();
        let value = value.trim();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || !names.insert(name.clone())
            || value.bytes().any(|byte| byte < 32 || byte == 127)
        {
            return Err("Invalid or duplicate relay header.".into());
        }
        match name.as_str() {
            "host" => {
                if value != host {
                    return Err("Invalid relay host.".into());
                }
                found_host = true;
            }
            "range" => {
                let parsed = parse_requested_range(value)?;
                let _ = parsed;
                range = Some(value.into());
            }
            "content-length" if value != "0" => {
                return Err("Relay request bodies are not accepted.".into())
            }
            "transfer-encoding" | "cookie" | "authorization" | "proxy-authorization" => {
                return Err("Credential or transfer headers are not accepted by the relay.".into())
            }
            _ => {}
        }
    }
    if !found_host {
        return Err("Missing relay host.".into());
    }
    Ok(RelayRequest {
        head: method == "HEAD",
        range,
    })
}
fn parse_requested_range(value: &str) -> Result<(u64, Option<u64>), String> {
    let value = value
        .strip_prefix("bytes=")
        .ok_or("Invalid relay byte range.")?;
    let (start, end) = value.split_once('-').ok_or("Invalid relay byte range.")?;
    let start = start
        .parse::<u64>()
        .map_err(|_| "Invalid relay byte range.")?;
    let end = if end.is_empty() {
        None
    } else {
        Some(
            end.parse::<u64>()
                .map_err(|_| "Invalid relay byte range.")?,
        )
    };
    if start >= MAX_BYTES || end.is_some_and(|end| end < start || end >= MAX_BYTES) {
        return Err("Relay byte range exceeds its bounds.".into());
    }
    Ok((start, end))
}
async fn reject(socket: &mut TcpStream) {
    let _ = socket
        .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .await;
}
async fn relay_connection(
    mut socket: TcpStream,
    host: String,
    path: String,
    remote: Remote,
    stop: watch::Receiver<bool>,
) -> Result<(), String> {
    let mut raw = Vec::new();
    let read = async {
        let mut buffer = [0u8; 1024];
        while !raw.windows(4).any(|part| part == b"\r\n\r\n") {
            let count = socket
                .read(&mut buffer)
                .await
                .map_err(|_| "Could not read file relay request.")?;
            if count == 0 || raw.len() + count > 8192 {
                return Err("File relay request is incomplete or too large.");
            }
            raw.extend_from_slice(&buffer[..count]);
        }
        Ok(())
    };
    if tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .ok()
        .and_then(Result::ok)
        .is_none()
    {
        reject(&mut socket).await;
        return Err("File relay headers timed out or were invalid.".into());
    }
    let mut request = match relay_request(&raw, &host, &path) {
        Ok(request) => request,
        Err(error) => {
            reject(&mut socket).await;
            return Err(error);
        }
    };
    if request.head {
        let mut headers=String::from("HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nCache-Control: no-store\r\n");
        if let Some(size) = remote.probe.size {
            headers.push_str(&format!("Content-Length: {size}\r\n"));
        }
        if remote.probe.supports_ranges {
            headers.push_str("Accept-Ranges: bytes\r\n");
        }
        headers.push_str("\r\n");
        socket
            .write_all(headers.as_bytes())
            .await
            .map_err(|_| "Could not write relay response.")?;
        return Ok(());
    }
    if !remote.probe.supports_ranges {
        if let Some(range) = &request.range {
            let (start, end) = parse_requested_range(range)?;
            if start != 0 || end.is_some_and(|end| remote.probe.size != Some(end.saturating_add(1)))
            {
                reject(&mut socket).await;
                return Err("This server does not support the requested byte range.".into());
            }
            request.range = None;
        }
    }
    let (_, mut response) = match fetched(
        &remote.url,
        request.range.as_deref(),
        remote.etag.as_deref(),
        Some(&remote.allowed_origin),
        stop.clone(),
        remote.network,
    )
    .await
    {
        Ok(value) => value,
        Err(error) => {
            reject(&mut socket).await;
            return Err(error);
        }
    };
    let expected = if let Some(range) = &request.range {
        let (start, end) = parse_requested_range(range)?;
        let actual = response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(content_range)
            .ok_or("The file server did not return a valid requested range.")?;
        let expected_end = end.unwrap_or(actual.2.saturating_sub(1));
        if response.status() != StatusCode::PARTIAL_CONTENT
            || actual.0 != start
            || actual.1 != expected_end
            || remote.probe.size != Some(actual.2)
        {
            reject(&mut socket).await;
            return Err(
                "The source changed its byte-range response. Partial files were kept.".into(),
            );
        }
        Some(actual.1 - actual.0 + 1)
    } else {
        if response.status() != StatusCode::OK {
            reject(&mut socket).await;
            return Err("The file source did not return a complete file.".into());
        }
        remote.probe.size.or(response.content_length())
    };
    if expected.is_some_and(|length| length > MAX_BYTES)
        || response
            .content_length()
            .zip(expected)
            .is_some_and(|(actual, expected)| actual != expected)
    {
        reject(&mut socket).await;
        return Err("The source changed its declared file size. Partial files were kept.".into());
    }
    if remote.etag.as_ref().is_some_and(|etag| {
        response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
            != Some(etag.as_str())
    }) {
        reject(&mut socket).await;
        return Err("The file source ETag changed. Partial files were kept.".into());
    }
    let mut headers=format!("HTTP/1.1 {}\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nCache-Control: no-store\r\n",if request.range.is_some(){"206 Partial Content"}else{"200 OK"});
    if let Some(size) = expected {
        headers.push_str(&format!("Content-Length: {size}\r\n"));
    } else {
        headers.push_str("Transfer-Encoding: chunked\r\n");
    }
    if let Some(range) = response
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
    {
        headers.push_str(&format!("Content-Range: {range}\r\n"));
    }
    if remote.probe.supports_ranges {
        headers.push_str("Accept-Ranges: bytes\r\n");
    }
    headers.push_str("\r\n");
    socket
        .write_all(headers.as_bytes())
        .await
        .map_err(|_| "Could not write file relay headers.")?;
    let stream = async {
        let mut received = 0u64;
        let mut pending = None;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The file transfer was interrupted.")?
        {
            received = received
                .checked_add(chunk.len() as u64)
                .ok_or("File byte counter overflowed.")?;
            if received > expected.unwrap_or(MAX_BYTES) {
                return Err("The file server sent more bytes than expected.".into());
            }
            if chunk.is_empty() {
                continue;
            }
            if expected.is_some() {
                // Keep the final byte until EOF and length validation; aria2 must not accept a truncated version.
                if let Some(byte) = pending.take() {
                    socket
                        .write_all(&[byte])
                        .await
                        .map_err(|_| "The file relay client disconnected.")?;
                }
                socket
                    .write_all(&chunk[..chunk.len() - 1])
                    .await
                    .map_err(|_| "The file relay client disconnected.")?;
                pending = chunk.last().copied();
            } else {
                socket
                    .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                    .await
                    .map_err(|_| "The file relay client disconnected.")?;
                socket
                    .write_all(&chunk)
                    .await
                    .map_err(|_| "The file relay client disconnected.")?;
                socket
                    .write_all(b"\r\n")
                    .await
                    .map_err(|_| "The file relay client disconnected.")?;
            }
        }
        if expected.is_some_and(|expected| received != expected) {
            return Err("The file server returned fewer bytes than expected.".into());
        }
        if let Some(byte) = pending {
            socket
                .write_all(&[byte])
                .await
                .map_err(|_| "The file relay client disconnected.")?;
        }
        if expected.is_none() {
            socket
                .write_all(b"0\r\n\r\n")
                .await
                .map_err(|_| "The file relay client disconnected.")?;
        }
        socket
            .shutdown()
            .await
            .map_err(|_| "Could not close the file relay response.")?;
        Ok(())
    };
    tokio::select! {result=stream=>result,_=cancelled(stop)=>Err(crate::engine::CANCELLED.into())}
}
async fn relay(listener: TcpListener, remote: Remote, token: String, stop: watch::Receiver<bool>) {
    let Ok(address) = listener.local_addr() else {
        return;
    };
    let host = address.to_string();
    let path = format!("/{token}/file");
    let slots = Arc::new(Semaphore::new(16));
    let mut tasks = JoinSet::new();
    loop {
        tokio::select! {_=cancelled(stop.clone())=>break,result=listener.accept()=>{let Ok((socket,peer))=result else{break;};if !peer.ip().is_loopback(){continue;}let Ok(permit)=slots.clone().try_acquire_owned()else{continue;};let (host,path,remote,stop)=(host.clone(),path.clone(),remote.clone(),stop.clone());tasks.spawn(async move{let _permit=permit;let _=relay_connection(socket,host,path,remote,stop).await;});},Some(_)=tasks.join_next(),if !tasks.is_empty()=>{}}
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

pub async fn verify_checksum(
    path: &Path,
    expected: Option<&str>,
    cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    let expected = validate_checksum(expected)?;
    let Some(expected) = expected else {
        return Ok(());
    };
    let path = path.to_owned();
    tauri::async_runtime::spawn_blocking(move||{let mut file=fs::File::open(path).map_err(|_|"Could not read the file for checksum verification.")?;let mut digest=Sha256::new();let mut bytes=[0u8;64*1024];loop{if *cancel.borrow(){return Err(crate::engine::CANCELLED.into());}let count=file.read(&mut bytes).map_err(|_|"Could not read the file for checksum verification.")?;if count==0{break;}digest.update(&bytes[..count]);}if format!("{:x}",digest.finalize())!=expected{return Err("SHA-256 mismatch. The file was not added to the library; its job data was kept for inspection.".into());}Ok(())}).await.map_err(|_|"Checksum verification stopped unexpectedly.")?
}
fn progress(line: &str, request_id: u64) -> Option<Progress> {
    let at = line.find("[#")?;
    let mut fields = line[at..].split_ascii_whitespace();
    fields.next()?;
    let (done, total) = fields.next()?.split_once('/')?;
    let done = done.trim_end_matches('B').parse::<f64>().ok()?;
    let total = total
        .split('(')
        .next()?
        .trim_end_matches('B')
        .parse::<f64>()
        .ok()?;
    if !done.is_finite() || !total.is_finite() || done < 0.0 || total < 0.0 {
        return None;
    }
    let speed = fields.find_map(|field| {
        field
            .strip_prefix("DL:")?
            .trim_end_matches(']')
            .trim_end_matches('B')
            .parse::<f64>()
            .ok()
    });
    Some(Progress {
        request_id,
        phase: "downloading".into(),
        percent: (total > 0.0).then(|| (done / total * 100.0).min(100.0)),
        speed,
        downloaded: Some(done),
        total: (total > 0.0).then_some(total),
    })
}
async fn transfer(
    executable: PathBuf,
    args: Vec<String>,
    mut cancel: watch::Receiver<bool>,
    request_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<(), String> {
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start the optional aria2 Files component.")?;
    #[cfg(windows)]
    let tree = crate::engine::ProcessTree::attach(&child)?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not read aria2 file progress.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Could not read aria2 file errors.")?;
    let errors = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(_)) = lines.next_line().await {}
    });
    let run = async {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|_| "Could not read aria2 file progress.")?
        {
            if let Some(update) = progress(&line, request_id) {
                emit(update);
            }
        }
        let status = child
            .wait()
            .await
            .map_err(|_| "Could not finish aria2 file transfer.")?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("aria2 file transfer stopped with exit code {}. Partial files were kept; check the source and retry.",status.code().map(|code|code.to_string()).unwrap_or_else(||"unknown".into())))
        }
    };
    let result = tokio::select! {result=run=>result,_=cancel.changed()=>Err(crate::engine::CANCELLED.into()),_=tokio::time::sleep(Duration::from_secs(24*60*60))=>Err("The HTTP file transfer reached its 24-hour limit. Partial files were kept.".into())};
    #[cfg(windows)]
    drop(tree);
    if result.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    errors.abort();
    result
}
pub async fn execute(
    app: &AppHandle,
    url: &str,
    staging: &Path,
    connections: u8,
    expected_sha256: Option<&str>,
    cancel: watch::Receiver<bool>,
    request_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    validate_connections(connections)?;
    let checksum = validate_checksum(expected_sha256)?;
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    let executable = crate::components::executable(app, "files")?;
    let remote = probe(url, cancel.clone()).await?;
    let staging = crate::library::authorized_path(staging)?;
    let output = staging.join(&remote.probe.filename);
    let control = staging.join(format!("{}.aria2", remote.probe.filename));
    let source_hash = format!("{:x}", Sha256::digest(url.as_bytes()));
    let state_path = staging.join(STATE);
    let has_state = owned_file(&state_path)?;
    let has_output = owned_file(&output)?;
    let has_control = owned_file(&control)?;
    if !has_state && (has_output || has_control) {
        return Err(
            "Existing file data has no matching ownership record. No file was overwritten.".into(),
        );
    }
    let resumable = remote.probe.supports_ranges && (remote.etag.is_some() || checksum.is_some());
    if has_state {
        let path = crate::library::authorized_path(&state_path)?;
        if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 4096 {
            return Err("The file resume record exceeds its limit.".into());
        }
        let state: Resume = serde_json::from_slice(
            &fs::read(path).map_err(|_| "Could not read the file resume record.")?,
        )
        .map_err(|_| "The file resume record is invalid.")?;
        if state.source_hash != source_hash
            || state.filename != remote.probe.filename
            || state.size != remote.probe.size
            || state.etag != remote.etag
            || state.checksum != checksum
        {
            return Err("The file source or expected checksum changed. Partial files were kept; prepare a new job to download this new version.".into());
        }
        if state.completed && has_output {
            let verified = crate::library::authorized_path(&output)?;
            if !fs::metadata(&verified)
                .map_err(|_| "Completed file is unavailable.")?
                .is_file()
                || state.size.is_some_and(|size| {
                    fs::metadata(&verified)
                        .map(|metadata| metadata.len() != size)
                        .unwrap_or(true)
                })
            {
                return Err("The completed file changed size. Nothing was published.".into());
            }
            verify_checksum(&output, checksum.as_deref(), cancel).await?;
            return crate::library::authorized_path(&output);
        }
        if has_output && !resumable {
            return Err("This server has no safe resume validator for the existing partial. Prepare a new job; the partial was kept.".into());
        }
    }
    if has_output {
        crate::library::authorized_path(&output)?;
    }
    if has_control {
        crate::library::authorized_path(&control)?;
    }
    let mut state = Resume {
        source_hash,
        filename: remote.probe.filename.clone(),
        size: remote.probe.size,
        etag: remote.etag.clone(),
        checksum: checksum.clone(),
        completed: false,
    };
    fs::write(
        &state_path,
        serde_json::to_vec(&state).map_err(|_| "Could not encode file resume state.")?,
    )
    .map_err(|_| "Could not write file resume state.")?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "Could not bind the local authenticated file relay.")?;
    let address = listener
        .local_addr()
        .map_err(|_| "Could not inspect the file relay address.")?;
    let token = nonce()?;
    let relay_url = format!("http://{address}/{token}/file");
    let mut args = download_args(&relay_url, &staging, &remote.probe, connections, resumable)?;
    if let Some(checksum) = &checksum {
        args.splice(
            0..0,
            [
                "--check-integrity=true".into(),
                format!("--checksum=sha-256={checksum}"),
            ],
        );
    }
    let (stop, receiver) = watch::channel(false);
    let relay_task = tokio::spawn(relay(listener, remote.clone(), token, receiver));
    let result = transfer(executable, args, cancel.clone(), request_id, emit.clone()).await;
    let _ = stop.send(true);
    let _ = relay_task.await;
    result?;
    let output = crate::library::authorized_path(&output)?;
    if !output.starts_with(&staging)
        || !fs::metadata(&output).map_err(|e| e.to_string())?.is_file()
        || remote.probe.size.is_some_and(|size| {
            fs::metadata(&output)
                .map(|metadata| metadata.len() != size)
                .unwrap_or(true)
        })
    {
        return Err("The downloaded file is missing or does not match its known size. Nothing was published.".into());
    }
    emit(Progress {
        request_id,
        phase: "verifying".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: remote.probe.size.map(|size| size as f64),
    });
    verify_checksum(&output, checksum.as_deref(), cancel).await?;
    state.completed = true;
    fs::write(
        state_path,
        serde_json::to_vec(&state).map_err(|_| "Could not encode completed file state.")?,
    )
    .map_err(|_| "Could not save completed file state.")?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_actual_partial_content_enables_connections() {
        let url = url::Url::parse("https://example.org/archive.zip").unwrap();
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT_RANGES,
            header::HeaderValue::from_static("bytes"),
        );
        headers.insert(
            header::CONTENT_LENGTH,
            header::HeaderValue::from_static("1048576"),
        );
        assert!(
            !probe_response(&url, StatusCode::OK, &headers)
                .unwrap()
                .supports_ranges
        );
        headers.insert(
            header::CONTENT_LENGTH,
            header::HeaderValue::from_static("1"),
        );
        headers.insert(
            header::CONTENT_RANGE,
            header::HeaderValue::from_static("bytes 0-0/1048576"),
        );
        let probe = probe_response(&url, StatusCode::PARTIAL_CONTENT, &headers).unwrap();
        assert!(probe.supports_ranges);
        assert_eq!(probe.size, Some(1048576));
        headers.insert(
            header::CONTENT_RANGE,
            header::HeaderValue::from_static("bytes 0-4/1048576"),
        );
        assert!(probe_response(&url, StatusCode::PARTIAL_CONTENT, &headers).is_err());
    }
    #[test]
    fn http_arguments_reach_only_the_authenticated_loopback_relay() {
        let mut info = FileProbe {
            filename: "archive.zip".into(),
            host: "example.org".into(),
            size: Some(42),
            supports_ranges: false,
        };
        let args = download_args(
            "http://127.0.0.1:49152/token/file",
            Path::new("C:/job"),
            &info,
            16,
            false,
        )
        .unwrap();
        assert!(args.iter().any(|arg| arg == "--split=1"));
        assert!(args.iter().any(|arg| arg == "--no-netrc=true"));
        assert!(!args.iter().any(|arg| arg.contains("example.org")));
        assert!(!args.iter().any(|arg| arg.starts_with("--bt-")));
        info.supports_ranges = true;
        assert!(download_args(
            "http://127.0.0.1:49152/token/file",
            Path::new("C:/job"),
            &info,
            8,
            true
        )
        .unwrap()
        .iter()
        .any(|arg| arg == "--split=8"));
        assert!(download_args(
            "https://example.org/file",
            Path::new("C:/job"),
            &info,
            8,
            true
        )
        .is_err());
    }
    #[test]
    fn relay_rejects_wrong_host_path_duplicate_headers_credentials_and_bodies() {
        let valid =
            b"GET /token/file HTTP/1.1\r\nHost: 127.0.0.1:49152\r\nRange: bytes=2-5\r\n\r\n";
        assert_eq!(
            relay_request(valid, "127.0.0.1:49152", "/token/file")
                .unwrap()
                .range
                .as_deref(),
            Some("bytes=2-5")
        );
        for raw in [
            "GET /wrong/file HTTP/1.1\r\nHost: 127.0.0.1:49152\r\n\r\n",
            "GET /token/file HTTP/1.1\r\nHost: attacker.example.org\r\n\r\n",
            "GET /token/file HTTP/1.1\r\nHost: 127.0.0.1:49152\r\nHost: 127.0.0.1:49152\r\n\r\n",
            "GET /token/file HTTP/1.1\r\nHost: 127.0.0.1:49152\r\nCookie: private\r\n\r\n",
            "GET /token/file HTTP/1.1\r\nHost: 127.0.0.1:49152\r\n\r\nbody",
        ] {
            assert!(relay_request(raw.as_bytes(), "127.0.0.1:49152", "/token/file").is_err());
        }
    }
    #[test]
    fn filenames_and_checksums_are_boundary_validated() {
        let url = url::Url::parse("https://example.org/original.zip?signature=secret").unwrap();
        assert_eq!(
            filename(&url, Some("attachment; filename*=UTF-8''my%20archive.zip")),
            "my archive.zip"
        );
        assert_eq!(filename(&url, None), "original.zip");
        assert_eq!(
            validate_checksum(Some(&"A".repeat(64))).unwrap(),
            Some("a".repeat(64))
        );
        assert!(validate_checksum(Some("unknown")).is_err());
        assert!(parsed("http://127.0.0.1/private").is_err());
        assert!(parsed("ftp://example.org/private").is_err());
    }
    async fn fixture_upstream(
        reply: Option<&'static [u8]>,
    ) -> (Remote, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!(
            "http://fixture.example.org:{}/archive.bin?signature=fixture-secret",
            address.port()
        );
        let remote = Remote {
            allowed_origin: format!("http://fixture.example.org:{}", address.port()),
            url,
            probe: FileProbe {
                filename: "archive.bin".into(),
                host: "fixture.example.org".into(),
                size: Some(6),
                supports_ranges: true,
            },
            etag: Some("\"fixture\"".into()),
            network: Network::Fixture(address),
        };
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let mut raw = Vec::new();
                let mut buffer = [0u8; 1024];
                while !raw.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buffer[..count]);
                }
                let raw = String::from_utf8(raw).unwrap();
                let lower = raw.to_ascii_lowercase();
                assert!(!lower.contains("\r\ncookie:"));
                assert!(!lower.contains("\r\nauthorization:"));
                if let Some(reply) = reply {
                    socket.write_all(reply).await.unwrap();
                    continue;
                }
                let range = raw.split("\r\n").find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("range"))
                        .map(|(_, value)| value.trim())
                });
                let (start, end) = range
                    .map(parse_requested_range)
                    .transpose()
                    .unwrap()
                    .map(|(start, end)| (start as usize, end.unwrap_or(5) as usize))
                    .unwrap_or((0, 5));
                let body = &b"abcdef"[start..=end];
                let headers = if range.is_some() {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/6\r\n"
                    )
                } else {
                    "HTTP/1.1 200 OK\r\n".into()
                };
                socket.write_all(format!("{headers}Content-Length: {}\r\nETag: \"fixture\"\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await.unwrap();
                socket.write_all(body).await.unwrap();
            }
        });
        (remote, task)
    }
    #[tokio::test]
    async fn authenticated_relay_streams_an_exact_range_without_forwarding_credentials() {
        let (remote, upstream) = fixture_upstream(None).await;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, receiver) = watch::channel(false);
        let task = tokio::spawn(relay(listener, remote, "fixture-token".into(), receiver));
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket.write_all(format!("GET /fixture-token/file HTTP/1.1\r\nHost: {address}\r\nRange: bytes=2-4\r\n\r\n").as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        socket.read_to_end(&mut bytes).await.unwrap();
        let response = String::from_utf8(bytes).unwrap();
        assert!(response.starts_with("HTTP/1.1 206 Partial Content\r\n"));
        assert!(response.contains("Content-Range: bytes 2-4/6\r\n"));
        assert!(response.ends_with("\r\n\r\ncde"));
        assert!(!response.contains("fixture-secret"));
        let mut rejected = TcpStream::connect(address).await.unwrap();
        rejected
            .write_all(format!("GET /wrong/file HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut denied = String::new();
        rejected.read_to_string(&mut denied).await.unwrap();
        assert!(denied.starts_with("HTTP/1.1 403 Forbidden"));
        stop.send(true).unwrap();
        task.await.unwrap();
        upstream.abort();
    }
    #[tokio::test]
    async fn expected_sha256_checks_real_bytes_and_rejects_mismatch_or_cancellation() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "advanced-checksum-{}-{}",
                std::process::id(),
                crate::queue::now()
            ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("fixture.bin");
        fs::write(&path, b"abc").unwrap();
        let (_sender, cancel) = watch::channel(false);
        verify_checksum(
            &path,
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            cancel.clone(),
        )
        .await
        .unwrap();
        assert!(verify_checksum(&path, Some(&"0".repeat(64)), cancel)
            .await
            .unwrap_err()
            .contains("SHA-256 mismatch"));
        let (_sender, cancel) = watch::channel(true);
        assert_eq!(
            verify_checksum(&path, Some(&"0".repeat(64)), cancel)
                .await
                .unwrap_err(),
            crate::engine::CANCELLED
        );
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
    #[tokio::test]
    async fn relay_never_completes_the_advertised_length_when_source_chunked_size_changes() {
        let (remote,upstream)=fixture_upstream(Some(b"HTTP/1.1 200 OK\r\nETag: \"fixture\"\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n7\r\nabcdefg\r\n0\r\n\r\n")).await;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, receiver) = watch::channel(false);
        let task = tokio::spawn(relay(listener, remote, "fixture-token".into(), receiver));
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket
            .write_all(
                format!("GET /fixture-token/file HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes(),
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        socket.read_to_end(&mut response).await.unwrap();
        let response = String::from_utf8(response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        assert!(headers.contains("Content-Length: 6"));
        assert!(
            body.len() < 6,
            "aria2 must receive an incomplete advertised body on an upstream size mismatch"
        );
        stop.send(true).unwrap();
        task.await.unwrap();
        upstream.abort();
    }
    #[tokio::test]
    #[ignore = "Requires a separately verified official aria2 binary via NABNAB_TEST_ARIA2_EXE; does not install it"]
    async fn real_aria2_downloads_a_fixture_through_the_authenticated_relay() {
        let executable = PathBuf::from(
            std::env::var_os("NABNAB_TEST_ARIA2_EXE")
                .expect("Set NABNAB_TEST_ARIA2_EXE to a verified official executable"),
        );
        assert!(executable.is_file());
        let (remote, upstream) = fixture_upstream(None).await;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "advanced-aria2-{}-{}",
                std::process::id(),
                crate::queue::now()
            ));
        fs::create_dir_all(&directory).unwrap();
        let args = download_args(
            &format!("http://{address}/fixture-token/file"),
            &directory,
            &remote.probe,
            4,
            true,
        )
        .unwrap();
        let (stop, receiver) = watch::channel(false);
        let task = tokio::spawn(relay(listener, remote, "fixture-token".into(), receiver));
        let (_sender, cancel) = watch::channel(false);
        let result = transfer(executable, args, cancel, 1, |_| {}).await;
        let _ = stop.send(true);
        task.await.unwrap();
        upstream.abort();
        result.unwrap();
        assert_eq!(fs::read(directory.join("archive.bin")).unwrap(), b"abcdef");
        fs::remove_file(directory.join("archive.bin")).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
