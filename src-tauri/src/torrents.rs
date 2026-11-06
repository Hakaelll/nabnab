use crate::model::Progress;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::watch,
};

const MAX_METADATA: usize = 16 * 1024 * 1024;
const MAX_FILES: usize = 50_000;
static INSPECTION: OnceLock<Mutex<Option<watch::Sender<bool>>>> = OnceLock::new();

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentStatus {
    pub enabled: bool,
    pub installed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentSelection {
    pub reference: String,
    pub manifest: TorrentManifest,
}

#[tauri::command]
pub fn torrent_status(app: AppHandle) -> Result<TorrentStatus, String> {
    let root = crate::components::root(&app)?;
    Ok(TorrentStatus {
        enabled: root.join("torrent.enabled").is_file(),
        installed: root.join("aria2c.exe").is_file(),
    })
}

#[tauri::command]
pub fn set_torrent_enabled(app: AppHandle, enabled: bool) -> Result<TorrentStatus, String> {
    let root = crate::components::root(&app)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    if enabled {
        if !root.join("aria2c.exe").is_file() {
            return Err(
                "Install the optional Files/BitTorrent component before activating P2P.".into(),
            );
        }
        fs::write(
            root.join("torrent.enabled"),
            b"P2P exchange explicitly enabled",
        )
        .map_err(|e| e.to_string())?;
    } else {
        if root.join("torrent.enabled").exists() {
            fs::remove_file(root.join("torrent.enabled")).map_err(|e| e.to_string())?;
        }
        if let Some(sender) = INSPECTION.get().and_then(|lock| lock.lock().ok()?.clone()) {
            let _ = sender.send(true);
        }
    }
    torrent_status(app)
}

fn metadata_root(app: &AppHandle) -> Result<PathBuf, String> {
    let root = crate::components::root(app)?.join("torrent-metadata");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    crate::library::authorized_path(&root)
}

fn retain_metadata(app: &AppHandle, path: &Path) -> Result<TorrentSelection, String> {
    let bytes = read_metadata(path)?;
    let manifest = parse_manifest(&bytes)?;
    let reference = format!("{:x}", Sha256::digest(&bytes));
    let retained = metadata_root(app)?.join(format!("{reference}.torrent"));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&retained)
    {
        Ok(mut file) => {
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            resolve_reference(app, &reference)?;
        }
        Err(error) => return Err(error.to_string()),
    }
    Ok(TorrentSelection {
        reference,
        manifest,
    })
}

pub fn resolve_reference(app: &AppHandle, reference: &str) -> Result<PathBuf, String> {
    if reference.len() != 64
        || !reference
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        return Err("Invalid retained torrent reference.".into());
    }
    let root = metadata_root(app)?;
    let path = crate::library::authorized_path(&root.join(format!("{reference}.torrent")))?;
    if !path.starts_with(&root)
        || fs::metadata(&path).map_err(|e| e.to_string())?.len() > MAX_METADATA as u64
    {
        return Err("The retained torrent metadata is invalid.".into());
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    if format!("{:x}", Sha256::digest(bytes)) != reference {
        return Err(
            "Retained torrent metadata failed its integrity check. Open the torrent again.".into(),
        );
    }
    Ok(path)
}

#[tauri::command]
pub async fn open_torrent(app: AppHandle) -> Result<Option<TorrentSelection>, String> {
    let picker = app.clone();
    let selected = tauri::async_runtime::spawn_blocking(move || {
        picker
            .dialog()
            .file()
            .add_filter("BitTorrent metadata", &["torrent"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = selected
        .into_path()
        .map_err(|_| "Choose a local torrent metadata file.")?;
    retain_metadata(&app, &path).map(Some)
}

#[tauri::command]
pub fn cancel_torrent_inspection() {
    if let Some(sender) = INSPECTION.get().and_then(|lock| lock.lock().ok()?.clone()) {
        let _ = sender.send(true);
    }
}

#[tauri::command]
pub async fn inspect_torrent_magnet(
    app: AppHandle,
    magnet: String,
) -> Result<TorrentSelection, String> {
    let executable = crate::components::executable(&app, "torrent")?;
    let status = torrent_status(app.clone())?;
    if !status.enabled {
        return Err("Explicitly activate P2P before retrieving magnet metadata.".into());
    }
    let (sender, receiver) = watch::channel(false);
    {
        let mut guard = INSPECTION
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| "Torrent inspection state is unavailable.")?;
        if guard.is_some() {
            return Err("Wait for the current magnet metadata lookup or cancel it first.".into());
        }
        *guard = Some(sender);
    }
    let result = async {
        let staging = metadata_root(&app)?.join(format!(
            "magnet-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "The system clock is invalid.")?
                .as_nanos()
        ));
        fs::create_dir(&staging).map_err(|e| e.to_string())?;
        let (path, _) = inspect_magnet(
            executable
                .parent()
                .ok_or("aria2 component path is invalid.")?,
            &staging,
            &magnet,
            true,
            receiver,
        )
        .await?;
        retain_metadata(&app, &path)
    }
    .await;
    if let Ok(mut guard) = INSPECTION.get_or_init(|| Mutex::new(None)).lock() {
        *guard = None;
    }
    result
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentFile {
    pub index: u32,
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentManifest {
    pub id: String,
    pub name: String,
    pub files: Vec<TorrentFile>,
    pub total_size: u64,
    pub private: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Seeding {
    #[default]
    Stop,
    Minutes {
        minutes: u32,
    },
    Ratio {
        ratio: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorrentOptions {
    pub selected: Vec<u32>,
    pub download_limit_kib: u32,
    pub upload_limit_kib: u32,
    #[serde(default)]
    pub seeding: Seeding,
}

#[derive(Debug)]
enum Bencode<'a> {
    Integer(i64),
    Bytes(&'a [u8]),
    List(Vec<Bencode<'a>>),
    Dictionary(BTreeMap<&'a [u8], Bencode<'a>>),
}

impl<'a> Bencode<'a> {
    fn get(&self, key: &[u8]) -> Option<&Self> {
        match self {
            Self::Dictionary(values) => values.get(key),
            _ => None,
        }
    }
    fn bytes(&self) -> Option<&'a [u8]> {
        match self {
            Self::Bytes(v) => Some(v),
            _ => None,
        }
    }
    fn integer(&self) -> Option<i64> {
        match self {
            Self::Integer(v) => Some(*v),
            _ => None,
        }
    }
    fn list(&self) -> Option<&[Self]> {
        match self {
            Self::List(v) => Some(v),
            _ => None,
        }
    }
    fn text(&self) -> Result<&str, String> {
        std::str::from_utf8(self.bytes().ok_or("Torrent text is not a byte string.")?)
            .map_err(|_| "Only torrents with valid UTF-8 filenames are supported.".into())
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    cursor: usize,
    nodes: usize,
    info: Option<(usize, usize)>,
}

impl<'a> Parser<'a> {
    fn value(&mut self, depth: usize) -> Result<Bencode<'a>, String> {
        self.nodes += 1;
        if depth > 32 || self.nodes > 500_000 {
            return Err("Torrent metadata is too complex.".into());
        }
        let first = *self
            .bytes
            .get(self.cursor)
            .ok_or("Truncated torrent metadata.")?;
        self.cursor += 1;
        match first {
            b'i' => {
                let start = self.cursor;
                let end = self.bytes[start..]
                    .iter()
                    .position(|c| *c == b'e')
                    .map(|v| start + v)
                    .ok_or("Unterminated torrent integer.")?;
                let raw = std::str::from_utf8(&self.bytes[start..end])
                    .map_err(|_| "Invalid torrent integer.")?;
                if raw.is_empty()
                    || raw == "-0"
                    || (raw.starts_with('0') && raw.len() > 1)
                    || raw.starts_with("-0")
                    || !raw
                        .trim_start_matches('-')
                        .bytes()
                        .all(|c| c.is_ascii_digit())
                {
                    return Err("Invalid torrent integer.".into());
                }
                self.cursor = end + 1;
                Ok(Bencode::Integer(
                    raw.parse()
                        .map_err(|_| "Torrent integer is out of range.")?,
                ))
            }
            b'0'..=b'9' => {
                let start = self.cursor - 1;
                let end = self.bytes[start..]
                    .iter()
                    .position(|c| *c == b':')
                    .map(|v| start + v)
                    .ok_or("Missing torrent string length.")?;
                let raw = std::str::from_utf8(&self.bytes[start..end])
                    .map_err(|_| "Invalid torrent string length.")?;
                if !raw.bytes().all(|c| c.is_ascii_digit())
                    || (raw.len() > 1 && raw.starts_with('0'))
                {
                    return Err("Invalid torrent string length.".into());
                }
                let size: usize = raw.parse().map_err(|_| "Torrent string is too large.")?;
                let end = (end + 1)
                    .checked_add(size)
                    .ok_or("Torrent string is too large.")?;
                let bytes = self
                    .bytes
                    .get(self.cursor + raw.len()..end)
                    .ok_or("Truncated torrent string.")?;
                self.cursor = end;
                Ok(Bencode::Bytes(bytes))
            }
            b'l' => {
                let mut list = Vec::new();
                while self.bytes.get(self.cursor) != Some(&b'e') {
                    list.push(self.value(depth + 1)?);
                }
                self.cursor += 1;
                Ok(Bencode::List(list))
            }
            b'd' => {
                let mut values = BTreeMap::new();
                let mut previous: Option<&[u8]> = None;
                while self.bytes.get(self.cursor) != Some(&b'e') {
                    let key = self
                        .value(depth + 1)?
                        .bytes()
                        .ok_or("Torrent dictionary key is invalid.")?;
                    if previous.is_some_and(|last| last >= key) {
                        return Err(
                            "Torrent dictionary keys are duplicated or out of order.".into()
                        );
                    }
                    previous = Some(key);
                    let start = self.cursor;
                    let value = self.value(depth + 1)?;
                    if depth == 0 && key == b"info" {
                        self.info = Some((start, self.cursor));
                    }
                    values.insert(key, value);
                }
                self.cursor += 1;
                Ok(Bencode::Dictionary(values))
            }
            _ => Err("Invalid bencoded torrent metadata.".into()),
        }
    }
}

fn safe_component(value: &str) -> Result<&str, String> {
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if value.is_empty()
        || value.len() > 240
        || matches!(value, "." | "..")
        || value.ends_with([' ', '.'])
        || value
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        || matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err("Torrent contains a filename that is unsafe on Windows.".into());
    }
    Ok(value)
}

fn unsigned(value: Option<&Bencode<'_>>, name: &str) -> Result<u64, String> {
    value
        .and_then(Bencode::integer)
        .and_then(|v| u64::try_from(v).ok())
        .ok_or_else(|| format!("The torrent has an invalid {name}."))
}

pub fn parse_manifest(bytes: &[u8]) -> Result<TorrentManifest, String> {
    if bytes.is_empty() || bytes.len() > MAX_METADATA {
        return Err("Choose a torrent metadata file up to 16 MiB.".into());
    }
    let mut parser = Parser {
        bytes,
        cursor: 0,
        nodes: 0,
        info: None,
    };
    let root = parser.value(0)?;
    if parser.cursor != bytes.len() {
        return Err("Torrent metadata has trailing data.".into());
    }
    if root.get(b"url-list").is_some() || root.get(b"httpseeds").is_some() {
        return Err("Web-seeded torrents are not supported by this module. Use a torrent with peer and tracker sources only.".into());
    }
    if let Some(announce) = root.get(b"announce") {
        validate_tracker(announce.text()?)?;
    }
    if let Some(tiers) = root.get(b"announce-list") {
        for tier in tiers
            .list()
            .ok_or("Torrent tracker tiers must be a list.")?
        {
            for tracker in tier.list().ok_or("Torrent tracker tier is invalid.")? {
                validate_tracker(tracker.text()?)?;
            }
        }
    }
    let info = root
        .get(b"info")
        .ok_or("Torrent metadata has no info dictionary.")?;
    if info.get(b"meta version").is_some() {
        return Err("BitTorrent v2 and hybrid torrents are not supported by this aria2 module. Use a v1 torrent.".into());
    }
    let name = safe_component(
        info.get(b"name.utf-8")
            .or_else(|| info.get(b"name"))
            .ok_or("Torrent has no name.")?
            .text()?,
    )?
    .to_owned();
    let piece_length = unsigned(info.get(b"piece length"), "piece length")?;
    let pieces = info
        .get(b"pieces")
        .and_then(Bencode::bytes)
        .ok_or("Torrent has no v1 piece hashes.")?;
    if piece_length == 0 || pieces.is_empty() || pieces.len() % 20 != 0 {
        return Err("Torrent piece hashes are invalid.".into());
    }
    let mut files = Vec::new();
    if let Some(entries) = info.get(b"files") {
        let entries = entries.list().ok_or("Torrent files must be a list.")?;
        if entries.is_empty() || entries.len() > MAX_FILES || info.get(b"length").is_some() {
            return Err("Torrent has an invalid or excessive file list.".into());
        }
        let mut names = std::collections::HashSet::new();
        for (index, entry) in entries.iter().enumerate() {
            if entry
                .get(b"attr")
                .and_then(Bencode::bytes)
                .is_some_and(|v| v.contains(&b'l'))
                || entry.get(b"symlink path").is_some()
            {
                return Err("Torrent symbolic links are not supported.".into());
            }
            let size = unsigned(entry.get(b"length"), "file size")?;
            let parts = entry
                .get(b"path.utf-8")
                .or_else(|| entry.get(b"path"))
                .and_then(Bencode::list)
                .ok_or("Torrent file path is invalid.")?;
            if parts.is_empty() || parts.len() > 32 {
                return Err("Torrent file path is too deep or empty.".into());
            }
            let mut path = name.clone();
            for part in parts {
                path.push('/');
                path.push_str(safe_component(part.text()?)?);
            }
            if path.len() > 2000 || !names.insert(path.to_lowercase()) {
                return Err("Torrent contains duplicate or excessive file paths.".into());
            }
            files.push(TorrentFile {
                index: index as u32 + 1,
                path,
                size,
            });
        }
    } else {
        files.push(TorrentFile {
            index: 1,
            path: name.clone(),
            size: unsigned(info.get(b"length"), "file size")?,
        });
    }
    let total_size = files
        .iter()
        .try_fold(0u64, |total, file| total.checked_add(file.size))
        .ok_or("Torrent total size overflows.")?;
    if total_size == 0 || total_size.div_ceil(piece_length) != (pieces.len() / 20) as u64 {
        return Err("Torrent piece hashes do not match its file sizes.".into());
    }
    let (start, end) = parser.info.ok_or("Torrent info dictionary is invalid.")?;
    let private = match info.get(b"private") {
        None | Some(Bencode::Integer(0)) => false,
        Some(Bencode::Integer(1)) => true,
        _ => return Err("Torrent private flag must be 0 or 1.".into()),
    };
    Ok(TorrentManifest {
        id: format!("torrent-{:x}", Sha256::digest(&bytes[start..end])),
        name,
        files,
        total_size,
        private,
    })
}

fn read_metadata(path: &Path) -> Result<Vec<u8>, String> {
    let path = crate::library::authorized_path(path)?;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_METADATA as u64 {
        return Err("Choose a regular torrent metadata file up to 16 MiB.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_METADATA as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_METADATA {
        return Err("Torrent metadata exceeded its 16 MiB limit.".into());
    }
    Ok(bytes)
}

pub fn read_manifest(path: &Path) -> Result<TorrentManifest, String> {
    parse_manifest(&read_metadata(path)?)
}

pub fn validate_magnet(input: &str) -> Result<String, String> {
    if input.len() > 16_384 || input.chars().any(char::is_control) {
        return Err("The magnet link is too long or contains control characters.".into());
    }
    let url = url::Url::parse(input.trim()).map_err(|_| "Paste a complete magnet link.")?;
    if url.scheme() != "magnet" || url.host_str().is_some() || !url.path().is_empty() {
        return Err("Paste a magnet:?xt=urn:btih:… link.".into());
    }
    let mut hashes = 0;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "xt" => {
                let hash = value.strip_prefix("urn:btih:").ok_or("Only BitTorrent v1 btih magnet links are supported.")?;
                if !((hash.len() == 40 && hash.bytes().all(|c| c.is_ascii_hexdigit())) || (hash.len() == 32 && hash.bytes().all(|c| c.is_ascii_alphabetic() || (b'2'..=b'7').contains(&c)))) {
                    return Err("The magnet info hash is invalid.".into());
                }
                hashes += 1;
            }
            "tr" => { validate_tracker(&value)?; }
            "dn" => { if value.len() > 512 { return Err("The magnet display name is too long.".into()); } }
            // Disallow web seeds, exact sources and extension parameters that could fetch other URLs.
            _ => return Err("This magnet contains unsupported parameters. Use btih, tracker and display-name fields only.".into()),
        }
    }
    if hashes != 1 {
        return Err("The magnet link must contain exactly one v1 info hash.".into());
    }
    Ok(url.to_string())
}

fn validate_tracker(input: &str) -> Result<(), String> {
    let url = url::Url::parse(input).map_err(|_| "A torrent tracker URL is invalid.")?;
    if !matches!(url.scheme(), "http" | "https" | "udp")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(
            "Only public HTTP, HTTPS and UDP trackers without credentials are supported.".into(),
        );
    }
    let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
    crate::model::validate_url(&format!("https://{authority}/"))
        .map(|_| ())
        .map_err(|_| "Local or reserved torrent trackers are not supported.".into())
}

fn base_args(staging: &Path) -> Vec<String> {
    [
        "--no-conf=true",
        "--enable-rpc=false",
        "--enable-color=false",
        "--console-log-level=warn",
        "--summary-interval=1",
        "--human-readable=false",
        "--truncate-console-readout=false",
        "--show-console-readout=true",
        "--auto-save-interval=1",
        "--continue=true",
        "--allow-overwrite=false",
        "--auto-file-renaming=false",
        "--file-allocation=none",
        "--check-integrity=true",
        "--bt-enable-lpd=false",
        "--bt-max-peers=55",
        "--bt-tracker-connect-timeout=15",
        "--bt-tracker-timeout=30",
        "--follow-torrent=false",
        "--follow-metalink=false",
        "--bt-remove-unselected-file=false",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([
        format!("--dir={}", staging.display()),
        format!("--dht-file-path={}", staging.join(".dht.dat").display()),
        format!("--dht-file-path6={}", staging.join(".dht6.dat").display()),
    ])
    .collect()
}

pub fn download_args(
    torrent: &Path,
    staging: &Path,
    manifest: &TorrentManifest,
    options: &TorrentOptions,
) -> Result<Vec<String>, String> {
    if options.selected.is_empty()
        || options.selected.len() > manifest.files.len()
        || options
            .selected
            .iter()
            .any(|index| *index == 0 || *index as usize > manifest.files.len())
    {
        return Err("Select at least one file from this torrent.".into());
    }
    let mut selected = options.selected.clone();
    selected.sort_unstable();
    selected.dedup();
    let mut args = base_args(staging);
    args.extend([
        format!(
            "--select-file={}",
            selected
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        format!("--max-download-limit={}K", options.download_limit_kib),
        format!("--max-upload-limit={}K", options.upload_limit_kib),
    ]);
    match options.seeding {
        Seeding::Stop => args.push("--seed-time=0".into()),
        Seeding::Minutes { minutes } if (1..=1440).contains(&minutes) => {
            args.extend([format!("--seed-time={minutes}"), "--seed-ratio=0.0".into()])
        }
        Seeding::Ratio { ratio } if ratio.is_finite() && (0.1..=20.0).contains(&ratio) => {
            args.extend([format!("--seed-ratio={ratio}"), "--seed-time=1440".into()])
        }
        _ => {
            return Err(
                "Choose a seeding time between 1 and 1440 minutes or a ratio between 0.1 and 20."
                    .into(),
            )
        }
    }
    args.extend(["--".into(), torrent.to_string_lossy().into_owned()]);
    Ok(args)
}

fn progress(line: &str, request_id: u64) -> Option<Progress> {
    let line = line.trim();
    let start = line.find("[#")?;
    let mut fields = line[start..].split_ascii_whitespace();
    fields.next()?;
    let transfer = fields.next()?;
    if transfer.starts_with("SEED(") {
        return Some(Progress {
            request_id,
            phase: "seeding".into(),
            percent: None,
            speed: None,
            downloaded: None,
            total: None,
        });
    }
    let (downloaded, rest) = transfer.split_once('/')?;
    let total = rest
        .split('(')
        .next()?
        .trim_end_matches('B')
        .parse::<f64>()
        .ok()?;
    let downloaded = downloaded.trim_end_matches('B').parse::<f64>().ok()?;
    if !total.is_finite() || !downloaded.is_finite() || total < 0.0 || downloaded < 0.0 {
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
        percent: (total > 0.0).then(|| (downloaded / total * 100.0).min(100.0)),
        speed,
        downloaded: Some(downloaded),
        total: (total > 0.0).then_some(total),
    })
}

async fn run_process(
    executable: &Path,
    args: Vec<String>,
    mut cancel: watch::Receiver<bool>,
    request_id: u64,
    timeout: Duration,
    emit: impl Fn(Progress) + Clone + Send + Sync + 'static,
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
    let mut child = command.spawn().map_err(|_| {
        "Could not start aria2. Install the optional Files/BitTorrent component in Settings."
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not read aria2 progress.")?;
    let stderr = child.stderr.take().ok_or("Could not read aria2 errors.")?;
    let errors = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut failed = false;
        while let Ok(Some(line)) = lines.next_line().await {
            failed |= line.contains("errorCode=");
        }
        failed
    });
    let output = async {
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines
            .next_line()
            .await
            .map_err(|_| "Could not read aria2 progress.")?
        {
            if let Some(update) = progress(&line, request_id) {
                emit(update);
            }
        }
        let status = child.wait().await.map_err(|_| "Could not finish aria2.")?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("aria2 stopped with exit code {}. Partial files are kept; check peers, trackers and the selected destination before retrying.", status.code().map(|n| n.to_string()).unwrap_or_else(|| "unknown".into())))
        }
    };
    let outcome = tokio::select! {
        _ = cancel.changed() => Err(crate::engine::CANCELLED.into()),
        _ = tokio::time::sleep(timeout) => Err("The torrent operation reached its time limit. Partial files are kept.".into()),
        result = output => result,
    };
    if outcome.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    errors.abort();
    outcome
}

/// P2P activation is checked at execution too, so disabling the module prevents pending jobs from starting.
pub async fn run(
    engine_root: &Path,
    torrent: &Path,
    staging: &Path,
    options: &TorrentOptions,
    p2p_enabled: bool,
    cancel: watch::Receiver<bool>,
    request_id: u64,
    emit: impl Fn(Progress) + Clone + Send + Sync + 'static,
) -> Result<Vec<PathBuf>, String> {
    if !p2p_enabled {
        return Err("Activate BitTorrent explicitly before connecting to peers. Downloading also uploads pieces to other peers.".into());
    }
    let torrent = crate::library::authorized_path(torrent)?;
    let manifest = read_manifest(&torrent)?;
    let staging = crate::library::authorized_path(staging)?;
    let args = download_args(&torrent, &staging, &manifest, options)?;
    emit(Progress {
        request_id,
        phase: "verifying".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: Some(manifest.total_size as f64),
    });
    run_process(
        &engine_root.join("aria2c.exe"),
        args,
        cancel.clone(),
        request_id,
        Duration::from_secs(48 * 60 * 60),
        emit,
    )
    .await?;
    let mut outputs = Vec::new();
    for file in manifest
        .files
        .iter()
        .filter(|file| options.selected.contains(&file.index))
    {
        if *cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        let path = crate::library::authorized_path(&staging.join(&file.path))?;
        if !path.starts_with(&staging)
            || !fs::metadata(&path).map_err(|e| e.to_string())?.is_file()
            || fs::metadata(&path).map_err(|e| e.to_string())?.len() != file.size
        {
            return Err("A selected torrent output is missing or has an unexpected size. Partial files are kept.".into());
        }
        outputs.push(path);
    }
    Ok(outputs)
}

pub async fn inspect_magnet(
    engine_root: &Path,
    staging: &Path,
    magnet: &str,
    p2p_enabled: bool,
    cancel: watch::Receiver<bool>,
) -> Result<(PathBuf, TorrentManifest), String> {
    if !p2p_enabled {
        return Err(
            "Activate BitTorrent before retrieving metadata from peers. This is a P2P connection."
                .into(),
        );
    }
    let magnet = validate_magnet(magnet)?;
    let staging = crate::library::authorized_path(staging)?;
    if fs::read_dir(&staging)
        .map_err(|e| e.to_string())?
        .next()
        .is_some()
    {
        return Err("Retrieve magnet metadata in a new, empty job folder.".into());
    }
    let mut args = base_args(&staging);
    args.extend([
        "--bt-metadata-only=true".into(),
        "--bt-save-metadata=true".into(),
        "--seed-time=0".into(),
        "--".into(),
        magnet,
    ]);
    run_process(
        &engine_root.join("aria2c.exe"),
        args,
        cancel,
        0,
        Duration::from_secs(120),
        |_| {},
    )
    .await?;
    let mut metadata = fs::read_dir(&staging)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|v| v == "torrent"));
    let path = metadata
        .next()
        .ok_or("Peers did not return usable torrent metadata.")?;
    if metadata.next().is_some() {
        return Err("The magnet returned ambiguous torrent metadata.".into());
    }
    let manifest = read_manifest(&path)?;
    Ok((path, manifest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(name: &str) -> Vec<u8> {
        let mut bytes = format!(
            "d4:infod6:lengthi4e4:name{}:{}12:piece lengthi4e6:pieces20:",
            name.len(),
            name
        )
        .into_bytes();
        bytes.extend([0u8; 20]);
        bytes.extend(b"ee");
        bytes
    }
    #[test]
    fn manifest_lists_files_without_connecting_and_rejects_unsafe_paths() {
        let manifest = parse_manifest(&single("movie.mp4")).unwrap();
        assert_eq!(manifest.name, "movie.mp4");
        assert_eq!(manifest.files[0].path, "movie.mp4");
        assert_eq!(manifest.files[0].size, 4);
        assert_eq!(manifest.total_size, 4);
        for name in [
            "../evil",
            "CON.txt",
            "NUL",
            "C:evil",
            "trailing.",
            "evil\\file",
        ] {
            assert!(parse_manifest(&single(name)).is_err(), "{name}");
        }
        assert!(parse_manifest(b"d4:infod4:name1:xeeJUNK").is_err());
        assert!(parse_manifest(b"d4:infod6:lengthi04e4:name1:xee").is_err());
    }
    #[test]
    fn multi_file_paths_retain_torrent_order_and_cannot_escape() {
        let mut bytes = b"d4:infod5:filesld6:lengthi2e4:pathl5:a.mp4eed6:lengthi2e4:pathl5:b.mp4eee4:name3:set12:piece lengthi4e6:pieces20:".to_vec();
        bytes.extend([0u8; 20]);
        bytes.extend(b"ee");
        let manifest = parse_manifest(&bytes).unwrap();
        assert_eq!(
            manifest
                .files
                .iter()
                .map(|file| (&*file.path, file.index))
                .collect::<Vec<_>>(),
            vec![("set/a.mp4", 1), ("set/b.mp4", 2)]
        );
    }
    #[test]
    fn magnet_accepts_v1_only_and_never_adds_external_sources() {
        let valid = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789&dn=Fixture";
        assert!(validate_magnet(valid).is_ok());
        assert!(validate_magnet(&format!("{valid}&ws=https://127.0.0.1/file")).is_err());
        assert!(validate_magnet(&format!("{valid}&tr=http://127.0.0.1/announce")).is_err());
        assert!(validate_magnet("magnet:?xt=urn:btmh:1220abcdef").is_err());
        assert!(validate_tracker("udp://tracker.example.org:80/announce").is_ok());
    }
    #[test]
    fn arguments_disable_rpc_and_post_download_seeding_by_default() {
        let manifest = parse_manifest(&single("fixture.mp4")).unwrap();
        let options = TorrentOptions {
            selected: vec![1],
            download_limit_kib: 2048,
            upload_limit_kib: 128,
            seeding: Seeding::Stop,
        };
        let args = download_args(
            Path::new("C:/fixture.torrent"),
            Path::new("C:/staging"),
            &manifest,
            &options,
        )
        .unwrap();
        for expected in [
            "--no-conf=true",
            "--enable-rpc=false",
            "--bt-enable-lpd=false",
            "--seed-time=0",
            "--max-upload-limit=128K",
            "--select-file=1",
        ] {
            assert!(args.iter().any(|arg| arg == expected), "{expected}");
        }
        assert!(!args.iter().any(|arg| arg.starts_with("--bt-tracker=")));
        let mut invalid = options.clone();
        invalid.selected = vec![2];
        assert!(download_args(
            Path::new("fixture"),
            Path::new("staging"),
            &manifest,
            &invalid
        )
        .is_err());
        invalid.selected = vec![1];
        invalid.seeding = Seeding::Ratio { ratio: f64::NAN };
        assert!(download_args(
            Path::new("fixture"),
            Path::new("staging"),
            &manifest,
            &invalid
        )
        .is_err());
    }
    #[test]
    fn progress_reports_real_byte_counts_without_an_eta() {
        let update = progress("[#2089b0 400B/1000B(40%) CN:1 DL:115B ETA:4s]", 42).unwrap();
        assert_eq!(update.request_id, 42);
        assert_eq!(update.percent, Some(40.0));
        assert_eq!(update.speed, Some(115.0));
        assert_eq!(update.total, Some(1000.0));
        assert!(progress("arbitrary remote log", 1).is_none());
        assert_eq!(
            progress("[#2089b0 SEED(0.8) CN:1 UL:100B(800B)]", 42)
                .unwrap()
                .phase,
            "seeding"
        );
    }
}
