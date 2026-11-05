use crate::{
    model::{Media, Progress},
    providers::{self, Capabilities},
};
use reqwest::{header, redirect::Policy, Response, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::sync::watch;

const MAX_BYTES: u64 = 128 * 1024 * 1024 * 1024;
const PART: &str = ".direct-source.part";
const RESUME: &str = ".direct-source.resume.json";
const COMPLETED: &str = ".direct-source.completed.json";
const PREFIX_LIMIT: usize = 4096;

#[derive(Clone, Copy)]
enum Network {
    Public,
    #[cfg(test)]
    Fixture(SocketAddr),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Resume {
    url_hash: String,
    etag: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Completed {
    url_hash: String,
    filename: String,
    bytes: u64,
    sha256: String,
}

struct MediaType {
    extension: &'static str,
    video: bool,
}

fn parsed_url(input: &str, network: Network) -> Result<url::Url, String> {
    if input.len() > 8192 || input.chars().any(char::is_control) {
        return Err("The direct URL exceeds its size limit or contains control characters.".into());
    }
    #[cfg(test)]
    if let Network::Fixture(address) = network {
        let parsed = url::Url::parse(input).map_err(|_| "Invalid fixture URL.")?;
        if parsed.scheme() == "http"
            && parsed.host_str() == Some("fixture.invalid")
            && parsed.port() == Some(address.port())
            && parsed.username().is_empty()
            && parsed.password().is_none()
        {
            reject_manifest_path(&parsed)?;
            return Ok(parsed);
        }
    }
    let _ = network;
    let normalized = crate::model::validate_url(input)?;
    let parsed = url::Url::parse(&normalized).map_err(|_| "Invalid direct URL.")?;
    reject_manifest_path(&parsed)?;
    Ok(parsed)
}

fn reject_manifest_path(url: &url::Url) -> Result<(), String> {
    let path = url.path().to_ascii_lowercase().replace("%2e", ".");
    if [".m3u8", ".m3u", ".mpd", ".ism", ".isml", ".smil", ".pls"]
        .iter()
        .any(|extension| path.ends_with(extension))
    {
        return Err(
            "Remote HLS/DASH and playlist manifests are not supported by the safe direct provider."
                .into(),
        );
    }
    Ok(())
}

async fn cancellation(mut cancel: watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

async fn addresses(url: &url::Url, network: Network) -> Result<Vec<SocketAddr>, String> {
    #[cfg(test)]
    if let Network::Fixture(address) = network {
        if url.host_str() == Some("fixture.invalid") {
            return Ok(vec![address]);
        }
    }
    let _ = network;
    let port = url
        .port_or_known_default()
        .ok_or("Unsupported direct URL scheme.")?;
    let addresses = match url.host().ok_or("The direct URL has no host.")? {
        url::Host::Ipv4(ip) => vec![SocketAddr::new(ip.into(), port)],
        url::Host::Ipv6(ip) => vec![SocketAddr::new(ip.into(), port)],
        url::Host::Domain(host) => tokio::time::timeout(
            Duration::from_secs(8),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| "Direct source DNS lookup timed out.")?
        .map_err(|_| "Direct source hostname could not be resolved.")?
        .collect(),
    };
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !providers::is_public_ip(address.ip()))
    {
        return Err("Local or reserved direct network destinations are not supported.".into());
    }
    Ok(addresses)
}

async fn fetch(
    input: &str,
    range: Option<&str>,
    etag: Option<&str>,
    inspect: bool,
    cancel: watch::Receiver<bool>,
    network: Network,
) -> Result<Response, String> {
    let operation = async {
        let mut url = parsed_url(input, network)?;
        for hop in 0..=5 {
            let approved = addresses(&url, network).await?;
            let host = url.host_str().ok_or("The direct URL has no host.")?;
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .read_timeout(Duration::from_secs(20))
                .timeout(Duration::from_secs(if inspect { 30 } else { 24 * 60 * 60 }))
                .resolve_to_addrs(host, &approved)
                .user_agent(concat!(
                    env!("CARGO_PKG_NAME"),
                    "/",
                    env!("CARGO_PKG_VERSION")
                ))
                .build()
                .map_err(|_| "Could not prepare the direct HTTP client.")?;
            let mut request = client
                .get(url.clone())
                .header(header::ACCEPT_ENCODING, "identity");
            if let Some(range) = range.filter(|_| etag.is_none() || hop == 0) {
                request = request.header(header::RANGE, range);
            }
            if let Some(etag) = etag.filter(|_| hop == 0) {
                request = request.header(header::IF_RANGE, etag);
            }
            let response = request
                .send()
                .await
                .map_err(|_| "The direct HTTP request failed. Check the connection and retry.")?;
            if response.status().is_redirection() {
                if hop == 5 {
                    return Err("The direct source exceeded the five-redirect limit.".into());
                }
                let location = response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or("The source returned an invalid redirect.")?;
                let redirected = url
                    .join(location)
                    .map_err(|_| "The source returned an invalid redirect URL.")?;
                if url.scheme() == "https" && redirected.scheme() == "http" {
                    return Err("An HTTPS direct source cannot redirect to plaintext HTTP.".into());
                }
                url = parsed_url(redirected.as_str(), network)?;
                continue;
            }
            return Ok(response);
        }
        Err("The direct source exceeded its redirect limit.".into())
    };
    tokio::select! { _ = cancellation(cancel) => Err(crate::engine::CANCELLED.into()), result = operation => result }
}

fn content_length(response: &Response) -> Result<Option<u64>, String> {
    if response.headers().contains_key(header::TRANSFER_ENCODING)
        && response.headers().contains_key(header::CONTENT_LENGTH)
    {
        return Err("The source returned contradictory HTTP length headers.".into());
    }
    if response
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|value| value != "identity")
    {
        return Err("The direct source ignored the request for uncompressed transfer data.".into());
    }
    let mut length = None;
    for value in response.headers().get_all(header::CONTENT_LENGTH) {
        let value = value
            .to_str()
            .ok()
            .and_then(|value| digits(value).ok())
            .ok_or("The direct source returned an invalid Content-Length.")?;
        if length.is_some_and(|old| old != value) {
            return Err("The source returned contradictory HTTP lengths.".into());
        }
        if value > MAX_BYTES {
            return Err("The direct file exceeds the 128 GiB limit.".into());
        }
        length = Some(value);
    }
    Ok(length)
}

fn digits(value: &str) -> Result<u64, String> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("Invalid HTTP range number.".into());
    }
    value
        .parse()
        .map_err(|_| "HTTP range number exceeds its limit.".into())
}

fn content_range(value: &str) -> Result<(u64, u64, Option<u64>), String> {
    let (range, total) = value
        .strip_prefix("bytes ")
        .ok_or("Invalid Content-Range unit.")?
        .split_once('/')
        .ok_or("Invalid Content-Range.")?;
    let (start, end) = range
        .split_once('-')
        .ok_or("Invalid Content-Range bounds.")?;
    let start = digits(start)?;
    let end = digits(end)?;
    let total = if total == "*" {
        None
    } else {
        Some(digits(total)?)
    };
    if start > end
        || end >= MAX_BYTES
        || total.is_some_and(|total| total > MAX_BYTES || total <= end)
    {
        return Err("The source returned an invalid or excessive Content-Range.".into());
    }
    Ok((start, end, total))
}

fn range_header(response: &Response) -> Result<(u64, u64, Option<u64>), String> {
    let value = response
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or("A partial response omitted Content-Range.")?;
    content_range(value)
}

fn media_type(response: &Response) -> Result<MediaType, String> {
    let mime = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let (extension, video) = match mime.as_str() {
        "video/mp4" => ("mp4", true), "video/webm" => ("webm", true), "video/quicktime" => ("mov", true),
        "video/x-matroska" => ("mkv", true), "video/x-msvideo" => ("avi", true), "video/mpeg" => ("mpg", true), "video/mp2t" => ("ts", true),
        "audio/mpeg" | "audio/mp3" => ("mp3", false), "audio/mp4" | "audio/x-m4a" => ("m4a", false),
        "audio/ogg" | "application/ogg" => ("ogg", false), "audio/opus" => ("opus", false),
        "audio/wav" | "audio/x-wav" | "audio/wave" => ("wav", false), "audio/flac" | "audio/x-flac" => ("flac", false),
        "audio/aac" | "audio/aacp" => ("aac", false), "audio/webm" => ("webm", false),
        "application/octet-stream" => {
            let extension = response.url().path().rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            match extension.as_str() {
                "mp4" => ("mp4", true), "m4v" => ("m4v", true), "webm" => ("webm", true), "mkv" => ("mkv", true), "mov" => ("mov", true), "avi" => ("avi", true), "mpg" | "mpeg" => ("mpg", true), "ts" => ("ts", true),
                "mp3" => ("mp3", false), "m4a" => ("m4a", false), "ogg" => ("ogg", false), "opus" => ("opus", false), "wav" => ("wav", false), "flac" => ("flac", false), "aac" => ("aac", false),
                _ => return Err("Binary direct responses require a supported media extension.".into()),
            }
        }
        _ => return Err("The direct source is not a supported audio/video file. Images, HTML and HLS/DASH playlists are not supported here.".into()),
    };
    Ok(MediaType { extension, video })
}

fn successful(response: &Response) -> Result<(), String> {
    if response.status() == StatusCode::OK || response.status() == StatusCode::PARTIAL_CONTENT {
        return Ok(());
    }
    let code = response.status().as_u16();
    if matches!(code, 401 | 403) {
        return Err(format!("HTTP Error {code}: the direct source requires authorized access. Sign in at the source or obtain a fresh authorized link."));
    }
    if code == 429 {
        let retry = response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| {
                digits(value).ok().or_else(|| {
                    httpdate::parse_http_date(value)
                        .ok()
                        .and_then(|date| date.duration_since(std::time::SystemTime::now()).ok())
                        .map(|wait| wait.as_secs() + 1)
                })
            })
            .unwrap_or(60)
            .clamp(1, 86400);
        return Err(format!(
            "HTTP Error 429: the direct server is limiting requests.\nRetry-After: {retry}"
        ));
    }
    Err(format!("HTTP Error {code}: the direct source rejected the request. Check access and retry when appropriate."))
}

pub async fn inspect(
    url: &str,
    request_id: u64,
    cancel: watch::Receiver<bool>,
) -> Result<Media, String> {
    inspect_with_network(url, request_id, cancel, Network::Public).await
}

async fn inspect_with_network(
    url: &str,
    request_id: u64,
    cancel: watch::Receiver<bool>,
    network: Network,
) -> Result<Media, String> {
    let response = fetch(url, Some("bytes=0-0"), None, true, cancel, network).await?;
    successful(&response)?;
    let kind = media_type(&response)?;
    let length = content_length(&response)?;
    if response.status() == StatusCode::PARTIAL_CONTENT {
        let (start, end, _) = range_header(&response)?;
        if start != 0 || end != 0 || length.is_some_and(|length| length != 1) {
            return Err("The direct source returned a contradictory inspection range.".into());
        }
    } else if response.headers().contains_key(header::CONTENT_RANGE) {
        return Err("A full response unexpectedly included Content-Range.".into());
    }
    let title = format!(
        "HTTP media · {}",
        response.url().host_str().unwrap_or("source")
    );
    Ok(Media {
        request_id,
        title,
        duration: None,
        thumbnail: None,
        uploader: response.url().host_str().map(str::to_owned),
        heights: Vec::new(),
        has_audio: true,
        has_video: kind.video,
        entries: Vec::new(),
        subtitle_languages: Vec::new(),
        capabilities: Capabilities {
            video: kind.video,
            audio: true,
            images: false,
            batches: false,
            subtitles: false,
            verified: false,
        },
    })
}

fn url_hash(url: &str) -> String {
    format!("{:x}", Sha256::digest(url.as_bytes()))
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
fn etag(response: &Response) -> Option<String> {
    response
        .headers()
        .get(header::ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|value| strong_etag(value))
        .map(str::to_owned)
}

fn regular(path: &Path) -> Result<Option<fs::Metadata>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("Could not inspect the owned direct temporary file.".into()),
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err("Direct temporary paths cannot be reparse points.".into());
        }
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Direct temporary paths must be regular owned files.".into());
    }
    Ok(Some(metadata))
}

fn partial_file(path: &Path, offset: u64) -> Result<File, String> {
    let existing = regular(path)?.is_some();
    let mut options = OpenOptions::new();
    options.write(true);
    if existing {
        options.append(offset > 0).truncate(offset == 0);
    } else {
        options.create_new(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
            .share_mode(0);
    }
    let file = options
        .open(path)
        .map_err(|_| "Could not write direct media. Check space and folder permissions.")?;
    if file
        .metadata()
        .map_err(|_| "Could not inspect the direct temporary file.")?
        .len()
        != offset
    {
        return Err("The owned direct partial changed unexpectedly.".into());
    }
    Ok(file)
}

fn read_resume(staging: &Path, hash: &str) -> Result<Option<Resume>, String> {
    let path = staging.join(RESUME);
    let Some(metadata) = regular(&path)? else {
        return Ok(None);
    };
    if metadata.len() > 4096 {
        return Err("The owned resume state exceeds its limit.".into());
    }
    let state: Resume =
        serde_json::from_slice(&fs::read(path).map_err(|_| "Could not read direct resume state.")?)
            .map_err(|_| "The owned direct resume state is invalid.")?;
    if state.url_hash != hash || !strong_etag(&state.etag) {
        return Err(
            "The owned partial belongs to a different URL or has no valid strong ETag.".into(),
        );
    }
    Ok(Some(state))
}

fn save_resume(staging: &Path, hash: &str, tag: Option<&str>) -> Result<(), String> {
    let path = staging.join(RESUME);
    regular(&path)?;
    let Some(tag) = tag else {
        if path.exists() {
            fs::remove_file(path).map_err(|_| "Could not reset owned resume state.")?;
        }
        return Ok(());
    };
    write_receipt(
        staging,
        RESUME,
        &Resume {
            url_hash: hash.into(),
            etag: tag.into(),
        },
    )
}

fn write_receipt(staging: &Path, name: &str, state: &impl Serialize) -> Result<(), String> {
    let path = staging.join(name);
    regular(&path)?;
    let temp = staging.join(format!(
        "{name}-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|_| "Could not create owned resume state.")?;
    file.write_all(&serde_json::to_vec(state).map_err(|_| "Could not encode direct receipt.")?)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not save direct resume state.")?;
    drop(file);
    fs::rename(&temp, &path).map_err(|_| "Could not activate direct receipt.".into())
}

async fn file_hash(path: &Path, cancel: watch::Receiver<bool>) -> Result<String, String> {
    let metadata = regular(path)?.ok_or("The completed direct input is missing.")?;
    if metadata.len() > MAX_BYTES {
        return Err("The completed direct input exceeds its size limit.".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
            .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Could not verify the completed direct input.")?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        if *cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        let count = file
            .read(&mut buffer)
            .map_err(|_| "Could not verify the completed direct input.")?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > MAX_BYTES {
            return Err("The direct input changed while being verified.".into());
        }
        hash.update(&buffer[..count]);
        tokio::task::yield_now().await;
    }
    if bytes != metadata.len() {
        return Err("The direct input changed while being verified.".into());
    }
    Ok(format!("{:x}", hash.finalize()))
}

async fn completed_input(
    staging: &Path,
    hash: &str,
    cancel: watch::Receiver<bool>,
) -> Result<Option<PathBuf>, String> {
    let path = staging.join(COMPLETED);
    let Some(metadata) = regular(&path)? else {
        return Ok(None);
    };
    if metadata.len() > 4096 {
        return Err("The completed direct receipt exceeds its limit.".into());
    }
    let receipt: Completed = serde_json::from_slice(
        &fs::read(path).map_err(|_| "Could not read the completed direct receipt.")?,
    )
    .map_err(|_| "The completed direct receipt is invalid.")?;
    let extension = receipt
        .filename
        .strip_prefix("direct-source.")
        .ok_or("Invalid completed direct filename.")?;
    if receipt.url_hash != hash
        || receipt.bytes == 0
        || receipt.bytes > MAX_BYTES
        || receipt.sha256.len() != 64
        || !receipt.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        || ![
            "mp4", "m4v", "webm", "mkv", "mov", "avi", "mpg", "ts", "mp3", "m4a", "ogg", "opus",
            "wav", "flac", "aac",
        ]
        .contains(&extension)
    {
        return Err("The completed direct receipt does not match this immutable source.".into());
    }
    let output = staging.join(&receipt.filename);
    let source = if regular(&output)?.is_some() {
        output.clone()
    } else {
        staging.join(PART)
    };
    if regular(&source)?.is_none_or(|metadata| metadata.len() != receipt.bytes)
        || file_hash(&source, cancel).await? != receipt.sha256
    {
        return Err("The completed direct input was changed or is incomplete. It was not reused or overwritten.".into());
    }
    if source != output {
        move_completed(&source, &output)?;
    }
    Ok(Some(output))
}

fn prefix_ready(prefix: &[u8], complete: bool) -> Result<bool, String> {
    let mut value = prefix;
    if b"\xef\xbb\xbf".starts_with(value) && value.len() < 3 && !complete {
        return Ok(false);
    }
    if value.starts_with(b"\xef\xbb\xbf") {
        value = &value[3..];
    }
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    if value.is_empty() {
        return if complete {
            Err("The direct source contains no media data.".into())
        } else {
            Ok(false)
        };
    }
    if matches!(value[0], b'#' | b'<' | b'[') {
        return Err(
            "The direct body is a playlist, XML or HTML document, not a safe media input.".into(),
        );
    }
    let lower: Vec<u8> = value.iter().take(8).map(u8::to_ascii_lowercase).collect();
    if b"ffconcat".starts_with(&lower) && lower.len() < 8 && !complete {
        return Ok(false);
    }
    if lower.starts_with(b"ffconcat") {
        return Err("Remote concatenation manifests are not supported.".into());
    }
    Ok(true)
}

fn move_completed(part: &Path, output: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = part.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = output.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(), to.as_ptr(), 0)
        } == 0
        {
            return Err("The direct input already exists or could not be published. Existing files were preserved.".into());
        }
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(part, output).map_err(|_| "The direct input already exists or could not be published. Existing files were preserved.")?;
        fs::remove_file(part).map_err(|_| "Could not remove the completed owned partial.")?;
    }
    Ok(())
}

pub async fn download(
    url: &str,
    staging: &Path,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    download_with_network(url, staging, cancel, run_id, emit, Network::Public).await
}

#[cfg(test)]
pub(crate) async fn download_fixture(
    url: &str,
    staging: &Path,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
    address: SocketAddr,
) -> Result<PathBuf, String> {
    download_with_network(
        url,
        staging,
        cancel,
        run_id,
        emit,
        Network::Fixture(address),
    )
    .await
}

async fn download_with_network(
    url: &str,
    staging: &Path,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
    network: Network,
) -> Result<PathBuf, String> {
    parsed_url(url, network)?;
    let metadata = fs::symlink_metadata(staging)
        .map_err(|_| "The owned download staging folder is unavailable.")?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err("Direct staging cannot be a reparse point.".into());
        }
    }
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Direct staging must be an owned regular directory.".into());
    }
    let hash = url_hash(url);
    if regular(&staging.join(COMPLETED))?.is_some() {
        emit(Progress {
            request_id: run_id,
            phase: "verifying".into(),
            percent: None,
            speed: None,
            downloaded: None,
            total: None,
        });
        if let Some(output) = completed_input(staging, &hash, cancel.clone()).await? {
            return Ok(output);
        }
    }
    let part = staging.join(PART);
    let saved = read_resume(staging, &hash)?;
    let mut offset = regular(&part)?.map_or(0, |metadata| metadata.len());
    if offset > MAX_BYTES {
        return Err("The owned direct partial exceeds its size limit.".into());
    }
    if saved.is_none() {
        offset = 0;
    }
    let range = (offset > 0).then(|| format!("bytes={offset}-"));
    let mut response = fetch(
        url,
        range.as_deref(),
        saved
            .as_ref()
            .filter(|_| offset > 0)
            .map(|state| state.etag.as_str()),
        false,
        cancel.clone(),
        network,
    )
    .await?;
    if offset > 0 && response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
        offset = 0;
        response = fetch(url, None, None, false, cancel.clone(), network).await?;
    }
    successful(&response)?;
    let kind = media_type(&response)?;
    let length = content_length(&response)?;
    let tag = etag(&response);
    let total = if response.status() == StatusCode::PARTIAL_CONTENT {
        let (start, end, total) = range_header(&response)?;
        if offset == 0
            || start != offset
            || total != end.checked_add(1)
            || length.is_some_and(|length| length != end - start + 1)
            || saved.as_ref().map(|state| &state.etag) != tag.as_ref()
        {
            return Err("The resumed response has an invalid Content-Range or changed strong ETag. The partial was preserved.".into());
        }
        total
    } else {
        if response.headers().contains_key(header::CONTENT_RANGE) {
            return Err("A full response unexpectedly included Content-Range.".into());
        }
        offset = 0;
        length
    };
    let same_resource = response.url() == &parsed_url(url, network)?;
    if offset > 0 && !same_resource {
        return Err("The resumed resource changed its URL. The partial was preserved.".into());
    }
    // Strong validators belong to one resource URI, not a different redirect target.
    let mut prefix = Vec::new();
    let mut checked_prefix = offset > 0;
    if checked_prefix {
        let mut first = File::open(&part)
            .map_err(|_| "Could not inspect the resumed partial.")?
            .take(PREFIX_LIMIT as u64);
        first
            .read_to_end(&mut prefix)
            .map_err(|_| "Could not inspect the resumed partial.")?;
        prefix_ready(&prefix, true)?;
        prefix.clear();
    }
    let mut file = partial_file(&part, offset)?;
    // Truncate an ignored-Range partial before installing the replacement validator.
    save_resume(staging, &hash, tag.as_deref().filter(|_| same_resource))?;
    let start = Instant::now();
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let mut downloaded = offset;
    let report = |downloaded: u64| {
        emit(Progress {
            request_id: run_id,
            phase: "downloading".into(),
            percent: total
                .filter(|total| *total > 0)
                .map(|total| 100.0 * downloaded as f64 / total as f64),
            speed: (start.elapsed().as_secs_f64() > 0.001)
                .then(|| (downloaded - offset) as f64 / start.elapsed().as_secs_f64()),
            downloaded: Some(downloaded as f64),
            total: total.map(|total| total as f64),
        })
    };
    report(downloaded);
    loop {
        let chunk = tokio::select! { _ = cancellation(cancel.clone()) => return Err(crate::engine::CANCELLED.into()), chunk = response.chunk() => chunk.map_err(|_| "The direct transfer was interrupted. Its owned partial was kept for retry.")? };
        let Some(chunk) = chunk else {
            break;
        };
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .filter(|value| *value <= MAX_BYTES && total.is_none_or(|total| *value <= total))
            .ok_or("The direct response exceeded its declared size or 128 GiB limit.")?;
        if !checked_prefix {
            let take = (PREFIX_LIMIT - prefix.len()).min(chunk.len());
            prefix.extend_from_slice(&chunk[..take]);
            if !prefix_ready(&prefix, prefix.len() == PREFIX_LIMIT)? {
                continue;
            }
            file.write_all(&prefix)
                .and_then(|_| file.write_all(&chunk[take..]))
                .map_err(|_| "Could not write direct media. Check disk space and permissions.")?;
            prefix.clear();
            checked_prefix = true;
        } else {
            file.write_all(&chunk)
                .map_err(|_| "Could not write direct media. Check disk space and permissions.")?;
        }
        if last_emit.elapsed() >= Duration::from_millis(250) {
            report(downloaded);
            last_emit = Instant::now();
        }
    }
    if !checked_prefix {
        prefix_ready(&prefix, true)?;
        file.write_all(&prefix)
            .map_err(|_| "Could not finish the owned direct partial.")?;
    }
    if downloaded == 0 || total.is_some_and(|total| total != downloaded) {
        return Err("The direct file is empty or shorter than its declared size. Its partial was preserved.".into());
    }
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    file.sync_all()
        .map_err(|_| "Could not flush the completed direct file.")?;
    drop(file);
    let output = staging.join(format!("direct-source.{}", kind.extension));
    if regular(&output)?.is_some() {
        return Err("The direct input already exists. Existing files were preserved.".into());
    }
    report(downloaded);
    emit(Progress {
        request_id: run_id,
        phase: "verifying".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
    let sha256 = file_hash(&part, cancel).await?;
    write_receipt(
        staging,
        COMPLETED,
        &Completed {
            url_hash: hash,
            filename: output
                .file_name()
                .ok_or("Invalid direct output filename.")?
                .to_string_lossy()
                .into_owned(),
            bytes: downloaded,
            sha256,
        },
    )?;
    move_completed(&part, &output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    struct Reply {
        head: String,
        body: Vec<u8>,
        slow: bool,
    }
    fn reply(status: &str, headers: &[(&str, &str)], body: &[u8]) -> Reply {
        Reply {
            head: format!(
                "HTTP/1.1 {status}\r\nConnection: close\r\n{}\r\n",
                headers
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}\r\n"))
                    .collect::<String>()
            ),
            body: body.to_vec(),
            slow: false,
        }
    }
    struct Fixture {
        url: String,
        network: Network,
        requests: Arc<Mutex<Vec<String>>>,
        server: thread::JoinHandle<()>,
    }
    impl Fixture {
        fn start(replies: Vec<Reply>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            listener.set_nonblocking(true).unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let recorded = requests.clone();
            let server = thread::spawn(move || {
                for reply in replies {
                    let deadline = Instant::now() + Duration::from_secs(8);
                    let (mut socket, _) = loop {
                        match listener.accept() {
                            Ok(socket) => break socket,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                assert!(Instant::now() < deadline, "fixture request timed out");
                                thread::sleep(Duration::from_millis(5));
                            }
                            Err(error) => panic!("{error}"),
                        }
                    };
                    socket.set_nonblocking(false).unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        let count = socket.read(&mut buffer).unwrap();
                        if count == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..count]);
                        assert!(request.len() < 16 * 1024);
                    }
                    recorded
                        .lock()
                        .unwrap()
                        .push(String::from_utf8(request).unwrap());
                    socket.write_all(reply.head.as_bytes()).unwrap();
                    if reply.slow {
                        let _ = socket.write_all(&reply.body[..3]);
                        let _ = socket.flush();
                        thread::sleep(Duration::from_millis(300));
                        let _ = socket.write_all(&reply.body[3..]);
                    } else {
                        let _ = socket.write_all(&reply.body);
                    }
                }
            });
            Self {
                url: format!(
                    "http://fixture.invalid:{}/private-token/media.mp4?signature=secret",
                    address.port()
                ),
                network: Network::Fixture(address),
                requests,
                server,
            }
        }
        fn finish(self) -> Vec<String> {
            self.server.join().unwrap();
            self.requests.lock().unwrap().clone()
        }
    }
    fn stage() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "nabnab-direct-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }
    fn cleanup(path: &Path) {
        let root = fs::canonicalize(std::env::temp_dir()).unwrap();
        let path = fs::canonicalize(path).unwrap();
        assert_eq!(path.parent(), Some(root.as_path()));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("nabnab-direct-test-"));
        fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn direct_rejects_private_urls_and_remote_manifest_paths_before_requests() {
        for url in [
            "http://127.0.0.1/a.mp4",
            "http://2130706433/a.mp4",
            "http://[::1]/a.mp4",
            "http://10.0.0.1/a.mp4",
            "file:///C:/secret",
            "https://u:p@example.org/a.mp4",
            "https://example.org/a.m3u8",
            "https://example.org/a.mpd",
            "https://example.org/a%2em3u8",
        ] {
            let (_sender, cancel) = watch::channel(false);
            assert!(inspect(url, 1, cancel).await.is_err(), "{url}");
        }
        assert!(parsed_url(
            &format!("https://example.org/{}", "a".repeat(8192)),
            Network::Public
        )
        .is_err());
    }

    #[tokio::test]
    async fn direct_inspection_uses_range_get_and_does_not_expose_url_secrets() {
        let fixture = Fixture::start(vec![reply(
            "206 Partial Content",
            &[
                ("Content-Type", "video/mp4"),
                ("Content-Length", "1"),
                ("Content-Range", "bytes 0-0/100"),
            ],
            b"x",
        )]);
        let (_sender, cancel) = watch::channel(false);
        let media = inspect_with_network(&fixture.url, 8, cancel, fixture.network)
            .await
            .unwrap();
        assert_eq!(media.title, "HTTP media · fixture.invalid");
        assert_eq!(media.duration, None);
        assert!(media.has_video && media.has_audio);
        assert!(media.heights.is_empty());
        assert!(!media.capabilities.verified);
        let requests = fixture.finish();
        assert!(requests[0].starts_with("GET "));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("range: bytes=0-0"));
    }

    #[tokio::test]
    async fn direct_revalidates_redirect_destinations_and_rejects_manifest_mime() {
        let tripwire = TcpListener::bind("127.0.0.1:0").unwrap();
        tripwire.set_nonblocking(true).unwrap();
        let location = format!("http://{}/secret.mp4", tripwire.local_addr().unwrap());
        let fixture = Fixture::start(vec![reply(
            "302 Found",
            &[("Location", &location), ("Content-Length", "0")],
            b"",
        )]);
        let (_sender, cancel) = watch::channel(false);
        assert!(
            inspect_with_network(&fixture.url, 1, cancel, fixture.network)
                .await
                .unwrap_err()
                .contains("network")
        );
        assert_eq!(fixture.finish().len(), 1);
        assert_eq!(
            tripwire.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        for mime in [
            "application/vnd.apple.mpegurl",
            "application/x-mpegURL",
            "application/dash+xml",
            "text/html",
            "image/jpeg",
        ] {
            let fixture = Fixture::start(vec![reply(
                "200 OK",
                &[("Content-Type", mime), ("Content-Length", "0")],
                b"",
            )]);
            let (_sender, cancel) = watch::channel(false);
            assert!(
                inspect_with_network(&fixture.url, 1, cancel, fixture.network)
                    .await
                    .is_err()
            );
            fixture.finish();
        }
    }

    #[tokio::test]
    async fn direct_network_failure_resumes_only_with_matching_strong_etag_and_range() {
        let fixture = Fixture::start(vec![
            reply(
                "200 OK",
                &[
                    ("Content-Type", "video/mp4"),
                    ("Content-Length", "6"),
                    ("ETag", "\"stable\""),
                ],
                b"abc",
            ),
            reply(
                "206 Partial Content",
                &[
                    ("Content-Type", "video/mp4"),
                    ("Content-Length", "3"),
                    ("Content-Range", "bytes 3-5/6"),
                    ("ETag", "\"stable\""),
                ],
                b"def",
            ),
        ]);
        let path = stage();
        let (_sender, cancel) = watch::channel(false);
        assert!(download_with_network(
            &fixture.url,
            &path,
            cancel.clone(),
            1,
            |_| {},
            fixture.network
        )
        .await
        .is_err());
        assert_eq!(fs::read(path.join(PART)).unwrap(), b"abc");
        let receipt = fs::read_to_string(path.join(RESUME)).unwrap();
        assert!(!receipt.contains("secret") && !receipt.contains("http"));
        let output = download_with_network(
            &fixture.url,
            &path,
            cancel.clone(),
            2,
            |_| {},
            fixture.network,
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"abcdef");
        let url = fixture.url.clone();
        let network = fixture.network;
        let requests = fixture.finish();
        assert!(requests[1].to_ascii_lowercase().contains("range: bytes=3-"));
        assert!(requests[1]
            .to_ascii_lowercase()
            .contains("if-range: \"stable\""));
        // The fixture listener is gone: downstream retries reuse verified local input offline.
        assert_eq!(
            download_with_network(&url, &path, cancel.clone(), 3, |_| {}, network)
                .await
                .unwrap(),
            output
        );
        fs::write(&output, b"ABCDEF").unwrap();
        assert!(
            download_with_network(&url, &path, cancel, 4, |_| {}, network)
                .await
                .unwrap_err()
                .contains("changed")
        );
        assert_eq!(fs::read(output).unwrap(), b"ABCDEF");
        cleanup(&path);
    }

    #[tokio::test]
    async fn direct_ignored_range_restarts_instead_of_appending_and_wrong_range_preserves_partial()
    {
        for valid in [true, false] {
            let response = if valid {
                reply(
                    "200 OK",
                    &[
                        ("Content-Type", "video/mp4"),
                        ("Content-Length", "6"),
                        ("ETag", "\"new\""),
                    ],
                    b"uvwxyz",
                )
            } else {
                reply(
                    "206 Partial Content",
                    &[
                        ("Content-Type", "video/mp4"),
                        ("Content-Length", "3"),
                        ("Content-Range", "bytes 2-4/6"),
                        ("ETag", "\"old\""),
                    ],
                    b"def",
                )
            };
            let fixture = Fixture::start(vec![response]);
            let path = stage();
            fs::write(path.join(PART), b"abc").unwrap();
            save_resume(&path, &url_hash(&fixture.url), Some("\"old\"")).unwrap();
            let (_sender, cancel) = watch::channel(false);
            let result =
                download_with_network(&fixture.url, &path, cancel, 1, |_| {}, fixture.network)
                    .await;
            if valid {
                assert_eq!(fs::read(result.unwrap()).unwrap(), b"uvwxyz");
            } else {
                assert!(result.unwrap_err().contains("Content-Range"));
                assert_eq!(fs::read(path.join(PART)).unwrap(), b"abc");
            }
            fixture.finish();
            cleanup(&path);
        }
    }

    #[tokio::test]
    async fn direct_enforces_size_and_http_errors_and_never_overwrites_existing_input() {
        let length = (MAX_BYTES + 1).to_string();
        for response in [
            reply("413 Content Too Large", &[("Content-Length", "0")], b""),
            reply(
                "200 OK",
                &[("Content-Type", "video/mp4"), ("Content-Length", &length)],
                b"",
            ),
        ] {
            let fixture = Fixture::start(vec![response]);
            let path = stage();
            let (_sender, cancel) = watch::channel(false);
            assert!(
                download_with_network(&fixture.url, &path, cancel, 1, |_| {}, fixture.network)
                    .await
                    .is_err()
            );
            assert!(!path.join("direct-source.mp4").exists());
            fixture.finish();
            cleanup(&path);
        }
        let fixture = Fixture::start(vec![reply(
            "200 OK",
            &[("Content-Type", "video/mp4"), ("Content-Length", "3")],
            b"new",
        )]);
        let path = stage();
        fs::write(path.join("direct-source.mp4"), b"existing").unwrap();
        let (_sender, cancel) = watch::channel(false);
        assert!(
            download_with_network(&fixture.url, &path, cancel, 1, |_| {}, fixture.network)
                .await
                .is_err()
        );
        assert_eq!(
            fs::read(path.join("direct-source.mp4")).unwrap(),
            b"existing"
        );
        fixture.finish();
        cleanup(&path);
    }

    #[tokio::test]
    async fn direct_cancel_keeps_owned_partial_and_unknown_size_does_not_invent_progress() {
        let mut response = reply(
            "200 OK",
            &[
                ("Content-Type", "video/mp4"),
                ("Content-Length", "6"),
                ("ETag", "\"stable\""),
            ],
            b"abcdef",
        );
        response.slow = true;
        let fixture = Fixture::start(vec![response]);
        let path = stage();
        let (sender, cancel) = watch::channel(false);
        let result = download_with_network(
            &fixture.url,
            &path,
            cancel,
            1,
            move |progress| {
                if progress.downloaded == Some(3.0) {
                    let _ = sender.send(true);
                }
            },
            fixture.network,
        )
        .await;
        assert_eq!(result.unwrap_err(), crate::engine::CANCELLED);
        assert_eq!(fs::read(path.join(PART)).unwrap(), b"abc");
        fixture.finish();
        cleanup(&path);
        let fixture = Fixture::start(vec![reply(
            "200 OK",
            &[("Content-Type", "audio/mpeg")],
            b"ID3fixture",
        )]);
        let path = stage();
        let (_sender, cancel) = watch::channel(false);
        let progress = Arc::new(Mutex::new(Vec::new()));
        let observed = progress.clone();
        let output = download_with_network(
            &fixture.url,
            &path,
            cancel,
            2,
            move |event| observed.lock().unwrap().push(event),
            fixture.network,
        )
        .await
        .unwrap();
        assert_eq!(output.extension().unwrap(), "mp3");
        for event in progress.lock().unwrap().iter() {
            assert_eq!(event.total, None);
            assert_eq!(event.percent, None);
        }
        fixture.finish();
        cleanup(&path);
    }

    #[test]
    fn direct_prefix_guard_handles_split_bom_whitespace_and_manifests() {
        assert!(!prefix_ready(b"\xef", false).unwrap());
        assert!(!prefix_ready(b"\xef\xbb", false).unwrap());
        assert!(!prefix_ready(b"\xef\xbb\xbf \n", false).unwrap());
        for prefix in [
            b"#".as_slice(),
            b"\xef\xbb\xbf \n#EXTM3U",
            b" \t<MPD",
            b"[playlist]",
            b"ffconcat version 1.0",
        ] {
            assert!(prefix_ready(prefix, true).is_err());
        }
        assert!(!prefix_ready(b"ffc", false).unwrap());
        assert!(prefix_ready(b"\0\0\0\x18ftypisom", false).unwrap());
        for value in ["bytes 3-5/6", "bytes 0-0/*"] {
            assert!(content_range(value).is_ok());
        }
        for value in [
            "bytes 6-3/9",
            "bytes +1-2/3",
            "bytes 0-4/4",
            "bytes 0-1/2,4-5/6",
        ] {
            assert!(content_range(value).is_err());
        }
        assert!(!strong_etag("W/\"weak\""));
        assert!(!strong_etag("\"bad\n\""));
        assert!(strong_etag("\"stable\""));
    }
}
