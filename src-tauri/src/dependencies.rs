use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};

const MANIFEST: &str = include_str!("../../scripts/engine-manifest.json");
const EXECUTABLES: [&str; 4] = ["yt-dlp.exe", "ffmpeg.exe", "ffprobe.exe", "deno.exe"];
const MAX_ASSET_BYTES: usize = 200 * 1024 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub name: String,
    pub version: Option<String>,
    pub path: Option<String>,
    pub size: u64,
    pub status: String,
    pub source: String,
}

#[derive(Clone, Deserialize)]
struct Pin {
    name: String,
    version: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    build: String,
}
struct Pins {
    yt_dlp: Pin,
    deno: Pin,
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|v| v.is_ascii_hexdigit())
}

fn pins(raw: &str) -> Result<Pins, String> {
    let entries: Vec<Pin> =
        serde_json::from_str(raw).map_err(|_| "The bundled engine manifest is invalid.")?;
    if entries.len() != 3 {
        return Err("The bundled engine manifest has unexpected components.".into());
    }
    let mut names = HashSet::new();
    for entry in &entries {
        if !names.insert(&entry.name)
            || !entry
                .version
                .bytes()
                .all(|v| v.is_ascii_digit() || v == b'.')
            || entry.version.split('.').count() != 3
            || entry.version.split('.').any(str::is_empty)
        {
            return Err(
                "The bundled engine manifest has invalid versions or duplicate components.".into(),
            );
        }
        let expected = match entry.name.as_str() {
            "yt-dlp" if entry.filename == "yt-dlp.exe" => format!(
                "https://github.com/yt-dlp/yt-dlp/releases/download/{}/yt-dlp.exe", entry.version
            ),
            "deno" if entry.filename == "deno.zip" => format!(
                "https://github.com/denoland/deno/releases/download/v{}/deno-x86_64-pc-windows-msvc.zip", entry.version
            ),
            "ffmpeg" if entry.build == "vcpkg" => continue,
            _ => return Err("The bundled engine manifest names an untrusted component.".into()),
        };
        if entry.url != expected || !valid_hash(&entry.sha256) {
            return Err(
                "The bundled engine manifest has an untrusted source or invalid hash.".into(),
            );
        }
    }
    Ok(Pins {
        yt_dlp: entries
            .iter()
            .find(|entry| entry.name == "yt-dlp")
            .ok_or("Missing yt-dlp pin.")?
            .clone(),
        deno: entries
            .into_iter()
            .find(|entry| entry.name == "deno")
            .ok_or("Missing Deno pin.")?,
    })
}

#[derive(Default, Serialize, Deserialize)]
struct Active {
    current: String,
    previous: String,
}

fn managed_root(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(root) = crate::test_data_dir() {
        return Ok(root.join("managed-engines"));
    }
    app.path()
        .app_local_data_dir()
        .map(|root| root.join("managed-engines"))
        .map_err(|e| e.to_string())
}

pub(crate) fn bundled_root(app: &AppHandle) -> Result<PathBuf, String> {
    let resource = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("engines");
    if resource.join("ffmpeg.exe").is_file() && resource.join("ffprobe.exe").is_file() {
        return Ok(resource);
    }
    #[cfg(debug_assertions)]
    {
        let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("engines");
        if development.join("ffmpeg.exe").is_file() && development.join("ffprobe.exe").is_file() {
            return Ok(development);
        }
    }
    Ok(resource)
}

fn read_active(root: &Path) -> Active {
    fs::read(root.join("active.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn valid_set_name(name: &str) -> bool {
    name.starts_with("set-")
        && name.len() <= 80
        && name
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'-' | b'_'))
}

fn set_path(root: &Path, name: &str) -> Option<PathBuf> {
    if !valid_set_name(name) {
        return None;
    }
    let path = root.join(name);
    if !EXECUTABLES.iter().all(|name| path.join(name).is_file()) {
        return None;
    }
    crate::library::authorized_path(&path).ok()
}

pub fn engine_root(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(root) = managed_root(app) {
        let active = read_active(&root);
        if active.current != "bundled" {
            if let Some(path) =
                set_path(&root, &active.current).or_else(|| set_path(&root, &active.previous))
            {
                return Ok(path);
            }
        }
    }
    let bundled = bundled_root(app)?;
    if bundled.is_dir() {
        return Ok(bundled);
    }
    Err("The media engines are missing. Open Settings to repair them, or repair nabnab using its installer.".into())
}

fn origin(name: &str) -> String {
    match name {
        "yt-dlp" => "Official yt-dlp release at github.com/yt-dlp/yt-dlp".into(),
        "deno" => "Official Deno release at github.com/denoland/deno".into(),
        _ => "Bundled FFmpeg, compiled from pinned sources. See licenses/SOURCES.md.".into(),
    }
}

async fn version(path: &Path, name: &str) -> Result<String, String> {
    let flag = if matches!(name, "ffmpeg" | "ffprobe") {
        "-version"
    } else {
        "--version"
    };
    let (_sender, cancel) = tokio::sync::watch::channel(false);
    let raw = tokio::time::timeout(
        Duration::from_secs(10),
        crate::engine::run_process(path.to_owned(), vec![flag.into()], cancel, 0, true, |_| {}),
    )
    .await
    .map_err(|_| "The engine did not answer its version check.")??;
    let line = raw.lines().next().unwrap_or_default().trim();
    if line.is_empty() || line.len() > 512 {
        return Err("The engine returned an invalid version.".into());
    }
    if name == "deno" {
        let valid = line
            .strip_prefix("deno ")
            .and_then(|v| v.split_whitespace().next())
            .and_then(|v| {
                let mut parts = v.split('.');
                Some((
                    parts.next()?.parse::<u32>().ok()?,
                    parts.next()?.parse::<u32>().ok()?,
                ))
            })
            .is_some_and(|(major, minor)| major > 2 || (major == 2 && minor >= 3));
        if !valid {
            return Err("Deno 2.3 or newer is required by yt-dlp.".into());
        }
    }
    Ok(line.to_owned())
}

pub async fn status(app: &AppHandle) -> Vec<EngineStatus> {
    let root = engine_root(app).ok();
    let mut statuses = Vec::new();
    for (name, executable) in [
        ("yt-dlp", "yt-dlp.exe"),
        ("ffmpeg", "ffmpeg.exe"),
        ("ffprobe", "ffprobe.exe"),
        ("deno", "deno.exe"),
    ] {
        let path = root
            .as_ref()
            .map(|root| {
                if name == "yt-dlp" {
                    crate::updater::executable(app, root).unwrap_or_else(|_| root.join(executable))
                } else {
                    root.join(executable)
                }
            })
            .filter(|path| path.is_file());
        let checked = match &path {
            Some(path) => version(path, name).await.ok(),
            None => None,
        };
        statuses.push(EngineStatus {
            name: name.into(),
            status: if path.is_none() {
                "missing"
            } else if checked.is_some() {
                "installed"
            } else {
                "broken"
            }
            .into(),
            version: checked,
            size: path
                .as_ref()
                .and_then(|p| fs::metadata(p).ok())
                .map_or(0, |m| m.len()),
            path: path.map(|p| p.to_string_lossy().into_owned()),
            source: origin(name),
        });
    }
    statuses
}

fn safe_resource(name: &str) -> bool {
    if let Some(license) = name.strip_prefix("licenses/") {
        return !license.is_empty()
            && !license.contains(['/', '\\'])
            && license.ends_with(".txt")
            && license
                .bytes()
                .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'-' | b'_' | b'.'));
    }
    !name.contains(['/', '\\'])
        && (EXECUTABLES.contains(&name)
            || matches!(name, "versions.json" | "ffmpeg-build.json")
            || (name.ends_with(".dll")
                && name
                    .bytes()
                    .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'-' | b'_' | b'.'))))
}

fn generated_set_name(name: &str) -> bool {
    valid_set_name(name) && name.strip_prefix("set-")
        .and_then(|rest| rest.split_once('-'))
        .is_some_and(|(pid, stamp)| pid.parse::<u32>().is_ok() && stamp.parse::<u128>().is_ok())
}

fn disposable_set(path: &Path) -> std::io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || crate::library::is_link(&metadata) {
        return Ok(false);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { return Ok(false) };
        if crate::library::is_link(&metadata) {
            return Ok(false);
        }
        if name == "licenses" && metadata.is_dir() {
            for license in fs::read_dir(entry.path())? {
                let license = license?;
                let metadata = fs::symlink_metadata(license.path())?;
                if !metadata.is_file() || crate::library::is_link(&metadata)
                    || !license.file_name().to_str().is_some_and(|name| safe_resource(&format!("licenses/{name}")))
                {
                    return Ok(false);
                }
            }
        } else if !metadata.is_file() || !(safe_resource(name) || matches!(name, "checksums.json" | "deno.zip")) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn cleanup_managed_sets(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    crate::library::authorized_path(root)?;
    let lock = OpenOptions::new().write(true).create(true).truncate(false)
        .open(root.join("repair.lock")).map_err(|error| error.to_string())?;
    if lock.try_lock().is_err() {
        return Ok(());
    }
    let pointer = root.join("active.json");
    let active = match fs::symlink_metadata(&pointer) {
        Ok(metadata) if metadata.is_file() && !crate::library::is_link(&metadata) && metadata.len() <= 4096 => {
            let bytes = fs::read(pointer).map_err(|error| error.to_string())?;
            let Ok(active) = serde_json::from_slice::<Active>(&bytes) else { return Ok(()) };
            if [&active.current, &active.previous].iter().any(|name| *name != "bundled" && !valid_set_name(name)) {
                return Ok(());
            }
            active
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Active::default(),
        Ok(_) => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        if name.to_str().is_some_and(|name| {
            generated_set_name(name) && !name.eq_ignore_ascii_case(&active.current)
                && !name.eq_ignore_ascii_case(&active.previous)
        }) && disposable_set(&entry.path()).unwrap_or(false) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}

pub(crate) fn cleanup(app: &AppHandle) -> Result<(), String> {
    cleanup_managed_sets(&managed_root(app)?)
}

fn checksum_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|_| "A bundled engine resource is unreadable.")?;
    let mut buffer = [0u8; 64 * 1024];
    let mut digest = Sha256::new();
    loop {
        let size = file
            .read(&mut buffer)
            .map_err(|_| "A bundled engine resource is unreadable.")?;
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn copy_ffmpeg(source: &Path, destination: &Path) -> Result<BTreeMap<String, String>, String> {
    let raw = fs::read(source.join("checksums.json")).map_err(|_| {
        "Bundled FFmpeg verification data is missing. Repair nabnab using its installer."
    })?;
    if raw.len() > 1024 * 1024 {
        return Err("Bundled engine verification data is too large.".into());
    }
    let checksums: BTreeMap<String, String> =
        serde_json::from_slice(&raw).map_err(|_| "Bundled engine verification data is invalid.")?;
    for name in ["ffmpeg.exe", "ffprobe.exe"] {
        if !checksums.contains_key(name) {
            return Err(
                "Bundled FFmpeg/ffprobe is missing. Repair nabnab using its installer.".into(),
            );
        }
    }
    let mut copied = BTreeMap::new();
    let mut total = 0;
    for (name, expected) in checksums {
        if !safe_resource(&name) || !valid_hash(&expected) {
            return Err(
                "Bundled engine verification data contains an unsafe path or checksum.".into(),
            );
        }
        if matches!(name.as_str(), "yt-dlp.exe" | "deno.exe" | "versions.json") {
            continue;
        }
        let path = crate::library::authorized_path(&source.join(&name))?;
        let size = fs::metadata(&path)
            .map_err(|_| "A bundled engine resource is missing.")?
            .len();
        total += size;
        if total > 512 * 1024 * 1024 {
            return Err("Bundled engine resources exceed the expected size.".into());
        }
        if checksum_file(&path)? != expected.to_lowercase() {
            return Err(
                "A bundled FFmpeg resource failed verification. Repair nabnab using its installer."
                    .into(),
            );
        }
        let target = destination.join(&name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|_| "Could not stage engine resources.")?;
        }
        fs::copy(&path, &target).map_err(|_| {
            "Could not copy verified engine resources. Check free space and permissions."
        })?;
        if checksum_file(&target)? != expected.to_lowercase() {
            return Err("A staged engine resource failed verification.".into());
        }
        copied.insert(name, expected.to_lowercase());
    }
    Ok(copied)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("nabnab engine repair")
        .timeout(Duration::from_secs(180))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() >= 5
                || url.scheme() != "https"
                || !matches!(
                    url.host_str(),
                    Some(
                        "github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                attempt.error("Untrusted engine release redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "Could not prepare the official engine download connection.".into())
}

fn verify_asset(bytes: &[u8], pin: &Pin) -> Result<(), String> {
    if bytes.len() > MAX_ASSET_BYTES
        || format!("{:x}", Sha256::digest(bytes)) != pin.sha256.to_lowercase()
    {
        return Err(
            "The downloaded engine failed its pinned SHA-256 check. Existing engines were kept."
                .into(),
        );
    }
    Ok(())
}

async fn download(client: &reqwest::Client, pin: &Pin) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(&pin.url)
        .send()
        .await
        .map_err(|_| "Could not reach the official engine release. Check your connection.")?
        .error_for_status()
        .map_err(|_| "The official engine release is unavailable. Try again later.")?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ASSET_BYTES as u64)
    {
        return Err("The engine download exceeds the expected size limit.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The engine download was interrupted. Existing engines were kept.")?
    {
        if bytes.len() + chunk.len() > MAX_ASSET_BYTES {
            return Err("The engine download exceeds the expected size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    verify_asset(&bytes, pin)?;
    Ok(bytes)
}

#[cfg(windows)]
fn system_tar() -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = vec![0u16; 32_768];
    let size = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    if size == 0 || size >= buffer.len() {
        return Err("Windows could not locate its archive utility.".into());
    }
    let path = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..size])).join("tar.exe");
    if !path.is_file() {
        return Err("Windows tar.exe is missing. Repair nabnab using its installer.".into());
    }
    Ok(path)
}

#[cfg(not(windows))]
fn system_tar() -> Result<PathBuf, String> {
    Err("Online engine repair currently supports Windows x64 only.".into())
}

fn validate_deno_entries(raw: &str) -> Result<(), String> {
    let entries: Vec<_> = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if entries != ["deno.exe"] && entries != ["./deno.exe"] {
        return Err(
            "The official Deno archive has unexpected entries. Existing engines were kept.".into(),
        );
    }
    Ok(())
}

async fn unpack_deno(stage: &Path, bytes: &[u8]) -> Result<(), String> {
    let archive = stage.join("deno.zip");
    fs::write(&archive, bytes).map_err(|_| "Could not stage the verified Deno archive.")?;
    let tar = system_tar()?;
    let (_sender, cancel) = tokio::sync::watch::channel(false);
    let listing = crate::engine::run_process(
        tar.clone(),
        vec!["-tf".into(), archive.to_string_lossy().into_owned()],
        cancel.clone(),
        0,
        true,
        |_| {},
    )
    .await?;
    validate_deno_entries(&listing)?;
    crate::engine::run_process(
        tar,
        vec![
            "-xf".into(),
            archive.to_string_lossy().into_owned(),
            "-C".into(),
            stage.to_string_lossy().into_owned(),
        ],
        cancel,
        0,
        true,
        |_| {},
    )
    .await?;
    crate::library::authorized_path(&stage.join("deno.exe"))?;
    fs::remove_file(archive).map_err(|_| "Could not remove the temporary engine archive.")?;
    Ok(())
}

fn save_active(root: &Path, state: &Active) -> Result<(), String> {
    let temporary = root.join(format!("active-{}.new", nonce()));
    write_new(
        &temporary,
        &serde_json::to_vec(state).map_err(|_| "Could not serialize engine state.")?,
    )?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = root
            .join("active.json")
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                0x1 | 0x8,
            )
        } == 0
        {
            return Err(
                "Could not activate the repaired engines. Existing engines were kept.".into(),
            );
        }
    }
    #[cfg(not(windows))]
    fs::rename(&temporary, root.join("active.json"))
        .map_err(|_| "Could not activate repaired engines.")?;
    Ok(())
}

fn nonce() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "Could not stage engine data.")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not flush staged engine data. Check free space.".into())
}

struct Staging {
    root: PathBuf,
    path: PathBuf,
    keep: bool,
}
impl Drop for Staging {
    fn drop(&mut self) {
        if !self.keep
            && self.path.parent() == Some(self.root.as_path())
            && self
                .path
                .file_name()
                .and_then(|v| v.to_str())
                .is_some_and(valid_set_name)
            && crate::library::authorized_path(&self.path).is_ok()
        {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub async fn repair(app: &AppHandle) -> Result<(), String> {
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Err("Online engine repair currently supports Windows x64 only.".into());
    }
    let pins = pins(MANIFEST)?;
    let root = managed_root(app)?;
    fs::create_dir_all(&root).map_err(|_| "Could not create the managed engine folder.")?;
    crate::library::authorized_path(&root)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("repair.lock"))
        .map_err(|_| "Could not lock engine repair.")?;
    lock.try_lock()
        .map_err(|_| "Another engine repair is running.")?;
    let path = root.join(format!("set-{}", nonce()));
    fs::create_dir(&path).map_err(|_| "Could not create engine staging.")?;
    let mut staging = Staging {
        root: root.clone(),
        path: path.clone(),
        keep: false,
    };
    let mut checksums = copy_ffmpeg(&bundled_root(app)?, &path)?;
    let client = client()?;
    let yt_dlp = download(&client, &pins.yt_dlp).await?;
    if !yt_dlp.starts_with(b"MZ") {
        return Err("The verified yt-dlp asset is not a Windows executable.".into());
    }
    write_new(&path.join("yt-dlp.exe"), &yt_dlp)?;
    let deno = download(&client, &pins.deno).await?;
    unpack_deno(&path, &deno).await?;
    if version(&path.join("yt-dlp.exe"), "yt-dlp").await? != pins.yt_dlp.version
        || !version(&path.join("deno.exe"), "deno")
            .await?
            .starts_with(&format!("deno {} ", pins.deno.version))
    {
        return Err(
            "A repaired engine failed its pinned version check. Existing engines were kept.".into(),
        );
    }
    version(&path.join("ffmpeg.exe"), "ffmpeg").await?;
    version(&path.join("ffprobe.exe"), "ffprobe").await?;
    checksums.insert("yt-dlp.exe".into(), pins.yt_dlp.sha256);
    checksums.insert("deno.exe".into(), checksum_file(&path.join("deno.exe"))?);
    write_new(&path.join("versions.json"), MANIFEST.as_bytes())?;
    checksums.insert(
        "versions.json".into(),
        checksum_file(&path.join("versions.json"))?,
    );
    write_new(
        &path.join("checksums.json"),
        &serde_json::to_vec(&checksums)
            .map_err(|_| "Could not record engine verification data.")?,
    )?;
    let active = read_active(&root);
    save_active(
        &root,
        &Active {
            current: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            previous: if active.current.is_empty() {
                "bundled".into()
            } else {
                active.current
            },
        },
    )?;
    staging.keep = true;
    Ok(())
}

pub async fn rollback(app: &AppHandle) -> Result<(), String> {
    let root = managed_root(app)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("repair.lock"))
        .map_err(|_| "No managed engine rollback is available.")?;
    lock.try_lock()
        .map_err(|_| "Another engine operation is running.")?;
    let active = read_active(&root);
    let previous = if active.previous == "bundled" {
        let bundled = bundled_root(app)?;
        if !EXECUTABLES.iter().all(|name| bundled.join(name).is_file()) {
            return Err("The previous bundled engine set is unavailable. Repair nabnab using its installer.".into());
        }
        bundled
    } else {
        set_path(&root, &active.previous).ok_or("No previous managed engine set is available.")?
    };
    for name in ["yt-dlp", "ffmpeg", "ffprobe", "deno"] {
        version(&previous.join(format!("{name}.exe")), name).await?;
    }
    save_active(
        &root,
        &Active {
            current: active.previous,
            previous: active.current,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_manifest_restricts_sources_architecture_and_hashes() {
        let pinned = pins(MANIFEST).unwrap();
        assert_eq!(pinned.yt_dlp.version, "2026.10.26");
        assert_eq!(pinned.deno.version, "2.9.8");
        let mut altered: serde_json::Value = serde_json::from_str(MANIFEST).unwrap();
        altered[0]["url"] = serde_json::json!("https://github.com.attacker.net/yt-dlp.exe");
        assert!(pins(&altered.to_string()).is_err());
        altered = serde_json::from_str(MANIFEST).unwrap();
        altered[1]["sha256"] = serde_json::json!("not a digest");
        assert!(pins(&altered.to_string()).is_err());
        assert!(pins("[]").is_err());
    }

    #[test]
    fn asset_mismatch_fails_before_any_publication() {
        let mut pin = pins(MANIFEST).unwrap().yt_dlp;
        assert!(verify_asset(b"untrusted binary", &pin).is_err());
        pin.sha256 = format!("{:x}", Sha256::digest(b"fixture"));
        assert!(verify_asset(b"fixture", &pin).is_ok());
        assert!(verify_asset(b"changed fixture", &pin).is_err());
    }

    #[test]
    fn archive_and_resource_paths_cannot_escape_staging() {
        for listing in [
            "../deno.exe\n",
            "/deno.exe\n",
            "deno.exe\nextra.dll\n",
            "deno.exe\ndeno.exe\n",
            "C:\\deno.exe\n",
        ] {
            assert!(validate_deno_entries(listing).is_err());
        }
        assert!(validate_deno_entries("deno.exe\n").is_ok());
        for name in [
            "../ffmpeg.exe",
            "licenses/../../outside.txt",
            "C:\\outside.dll",
            "folder/name.dll",
            "evil.exe",
        ] {
            assert!(!safe_resource(name));
        }
        for name in [
            "ffmpeg.exe",
            "ffprobe.exe",
            "avcodec-63.dll",
            "licenses/ffmpeg.txt",
        ] {
            assert!(safe_resource(name));
        }
        assert!(!valid_set_name("set-../../outside"));
        assert!(!valid_set_name("../../set-owned"));
    }

    #[test]
    fn managed_cleanup_defers_to_repair_and_preserves_sets_with_an_invalid_pointer() {
        let root = std::env::temp_dir().join(format!("nabnab-engine-cleanup-{}", nonce()));
        let obsolete = root.join("set-100-1000");
        fs::create_dir_all(&obsolete).unwrap();
        fs::write(obsolete.join("deno.exe"), b"fixture").unwrap();
        let lock = OpenOptions::new().write(true).create_new(true).open(root.join("repair.lock")).unwrap();
        lock.try_lock().unwrap();
        cleanup_managed_sets(&root).unwrap();
        assert!(obsolete.is_dir());
        drop(lock);
        fs::write(root.join("active.json"), b"incomplete state").unwrap();
        cleanup_managed_sets(&root).unwrap();
        assert!(obsolete.is_dir());
        fs::write(root.join("active.json"), br#"{"current":"bundled","previous":"../set-100-1000"}"#).unwrap();
        cleanup_managed_sets(&root).unwrap();
        assert!(obsolete.is_dir());
        fs::write(root.join("active.json"), br#"{"current":"bundled","previous":"bundled"}"#).unwrap();
        cleanup_managed_sets(&root).unwrap();
        assert!(!obsolete.exists());
        cleanup_managed_sets(&root).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_ffmpeg_checksums_do_not_copy_an_unverified_file() {
        let root = std::env::temp_dir().join(format!("nabnab-engine-check-{}", nonce()));
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&target).unwrap();
        fs::write(source.join("ffmpeg.exe"), b"fixture").unwrap();
        fs::write(source.join("ffprobe.exe"), b"fixture").unwrap();
        fs::write(
            source.join("checksums.json"),
            serde_json::json!({"ffmpeg.exe":"0".repeat(64),"ffprobe.exe":"0".repeat(64)})
                .to_string(),
        )
        .unwrap();
        assert!(copy_ffmpeg(&source, &target).is_err());
        assert!(!target.join("ffmpeg.exe").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
