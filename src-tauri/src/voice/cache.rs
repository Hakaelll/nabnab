use super::{cancelled, hash, wave};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, Metadata, OpenOptions},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};
use tokio::sync::{watch, RwLock};
pub(super) static WRITE_LOCK: Mutex<()> = Mutex::new(());
pub(super) static IN_USE: RwLock<()> = RwLock::const_new(());
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const CACHE_FILES: usize = 8192;
const CACHE_BYTES: u64 = 512 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Receipt {
    version: u8,
    key: String,
    sha256: String,
    bytes: u64,
    samples: u64,
}
pub(super) struct Cached {
    pub path: PathBuf,
    pub wave: wave::Wave,
}
pub(super) fn directory(root: &Path, name: &str) -> Result<PathBuf, String> {
    crate::library::authorized_path(&crate::sessions::secure_module_directory(root, name)?)
}
pub(super) fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CacheStatus {
    bytes: u64,
    files: usize,
    max_bytes: u64,
    max_files: usize,
}

fn managed_file(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return false;
    };
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("wav" | "json") => valid_id(stem),
        Some("pending") => stem
            .strip_prefix("voice-")
            .and_then(|suffix| suffix.split_once('-'))
            .is_some_and(|(stamp, sequence)| {
                [stamp, sequence].iter().all(|field| !field.is_empty() && field.bytes().all(|b| b.is_ascii_digit()))
            }),
        _ => false,
    }
}

fn cache_files(root: &Path) -> Result<Vec<(PathBuf, Metadata)>, String> {
    let path = root.join("voice-cache");
    let path = match fs::symlink_metadata(&path) {
        Ok(_) => crate::library::authorized_path(&path)?,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut files = Vec::new();
    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if managed_file(&path) {
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_file() && !crate::library::is_link(&meta) => files.push((path, meta)),
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
    }
    Ok(files)
}

pub(crate) fn cache_status(root: &Path) -> Result<CacheStatus, String> {
    let files = cache_files(root)?;
    Ok(CacheStatus { bytes: files.iter().map(|(_, meta)| meta.len()).sum(), files: files.len(),
        max_bytes: CACHE_BYTES, max_files: CACHE_FILES })
}

fn remove(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not clean the voice cache: {error}")),
    }
}

pub(crate) fn clear_cache(root: &Path) -> Result<CacheStatus, String> {
    let _guard = IN_USE.try_write().map_err(|_| "Voice audio is in use. Let the preview or job finish, or pause the job before clearing the cache.")?;
    for (path, _) in cache_files(root)? {
        remove(&path)?;
    }
    cache_status(root)
}

pub(crate) fn trim_cache(root: &Path) -> Result<(), String> {
    trim(root, CACHE_FILES, CACHE_BYTES)
}

fn trim(root: &Path, max_files: usize, max_bytes: u64) -> Result<(), String> {
    let Ok(_guard) = IN_USE.try_write() else { return Ok(()); };
    let mut pairs = HashMap::<String, Vec<(PathBuf, Metadata)>>::new();
    for (path, meta) in cache_files(root)? {
        if path.extension().is_some_and(|extension| extension == "pending") {
            remove(&path)?;
        } else if let Some(key) = path.file_stem().and_then(|stem| stem.to_str()) {
            pairs.entry(key.to_owned()).or_default().push((path, meta));
        }
    }
    let mut entries = Vec::new();
    for (key, files) in pairs {
        if files.len() == 2 {
            let modified = files.iter().filter_map(|(_, meta)| meta.modified().ok()).max();
            let bytes: u64 = files.iter().map(|(_, meta)| meta.len()).sum();
            entries.push((modified, key, bytes, files));
        } else {
            for (path, _) in files {
                remove(&path)?;
            }
        }
    }
    let mut total: u64 = entries.iter().map(|(_, _, bytes, _)| bytes).sum();
    let mut count = entries.len() * 2;
    if total <= max_bytes && count <= max_files {
        return Ok(());
    }
    entries.sort_unstable_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    for (_, _, bytes, files) in entries {
        for (path, _) in files {
            remove(&path)?;
        }
        total -= bytes;
        count -= 2;
        if total <= max_bytes && count <= max_files {
            break;
        }
    }
    Ok(())
}
pub(super) fn read_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    max: u64,
) -> Result<T, String> {
    let path = crate::library::authorized_path(path)?;
    if !path.is_file() || fs::metadata(&path).map_err(|e| e.to_string())?.len() > max {
        return Err(
            "The local voice record exceeds its size limit or is not a regular file.".into(),
        );
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|_| "The local voice record is unreadable.".into())
}
pub(super) fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid voice record path.")?;
    let pending = parent.join(format!(
        "voice-{}-{}.pending",
        crate::queue::now(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|e| e.to_string())?;
    let result = file
        .write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string());
    drop(file);
    if let Err(error) = result {
        let _ = fs::remove_file(pending);
        return Err(error);
    }
    let result = crate::direct::move_completed(&pending, path);
    if result.is_err() {
        let _ = fs::remove_file(pending);
    }
    result
}
fn digest(path: &Path, cancel: &watch::Receiver<bool>) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut digest = sha2::Sha256::new();
    use sha2::Digest;
    let mut buffer = [0u8; 65536];
    loop {
        cancelled(cancel)?;
        let read = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
pub(super) fn cached(
    root: &Path,
    key: &str,
    verify_hash: bool,
    cancel: &watch::Receiver<bool>,
) -> Option<Cached> {
    let directory = directory(root, "voice-cache").ok()?;
    cached_at(&directory, key, verify_hash, cancel)
}
pub(super) fn cached_at(
    directory: &Path,
    key: &str,
    verify_hash: bool,
    cancel: &watch::Receiver<bool>,
) -> Option<Cached> {
    if !valid_id(key) {
        return None;
    }
    let path = directory.join(format!("{key}.wav"));
    let receipt: Receipt = read_json(&directory.join(format!("{key}.json")), 8192).ok()?;
    if receipt.version != 1 || receipt.key != key || !valid_id(&receipt.sha256) {
        return None;
    }
    let wave = wave::inspect(&path).ok()?;
    if wave.bytes != receipt.bytes || wave.length / 2 != receipt.samples {
        return None;
    }
    if verify_hash && digest(&path, cancel).ok()? != receipt.sha256 {
        return None;
    }
    Some(Cached { path, wave })
}
pub(super) fn publish(
    root: &Path,
    key: &str,
    source: &Path,
    cancel: &watch::Receiver<bool>,
) -> Result<(), String> {
    if !valid_id(key) {
        return Err("Invalid local voice segment identity.".into());
    }
    cancelled(cancel)?;
    let source = crate::library::authorized_path(source)?;
    let wave = wave::inspect(&source)?;
    let sha256 = digest(&source, cancel)?;
    let directory = directory(root, "voice-cache")?;
    let _guard = WRITE_LOCK.lock().unwrap();
    if cached_at(&directory, key, true, cancel).is_some() {
        return Ok(());
    }
    let output = directory.join(format!("{key}.wav"));
    let record = directory.join(format!("{key}.json"));
    for path in [&output, &record] {
        if fs::symlink_metadata(path).is_ok() {
            let owned = crate::library::authorized_path(path)?;
            if owned.parent() != Some(directory.as_path()) || !owned.is_file() {
                return Err("An invalid path occupies the managed voice cache.".into());
            }
            fs::remove_file(owned).map_err(|e| e.to_string())?;
        }
    }
    let bytes = fs::read(&source).map_err(|e| e.to_string())?;
    if bytes.len() as u64 != wave.bytes || hash(&bytes) != sha256 {
        return Err("A generated voice segment changed before it could be cached.".into());
    }
    write_new(&output, &bytes)?;
    let receipt = Receipt {
        version: 1,
        key: key.into(),
        sha256,
        bytes: wave.bytes,
        samples: wave.length / 2,
    };
    write_new(
        &record,
        &serde_json::to_vec(&receipt).map_err(|e| e.to_string())?,
    )?;
    Ok(())
}
pub(super) fn audio(
    root: &Path,
    key: &str,
    cancel: &watch::Receiver<bool>,
) -> Result<Vec<u8>, String> {
    cancelled(cancel)?;
    let cached=cached(root,key,true,cancel).ok_or("This voice segment is not complete or its cached audio changed. Resume the plan to generate it again.")?;
    let bytes = fs::read(&cached.path).map_err(|e| e.to_string())?;
    if bytes.len() as u64 != cached.wave.bytes {
        return Err("The voice segment changed while being read.".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn trims_oldest_complete_pair_and_abandoned_files_only() {
        let root = std::env::temp_dir().join(format!("nabnab-voice-trim-{}-{}", std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)));
        let directory = directory(&root, "voice-cache").unwrap();
        let old = "a".repeat(64);
        let recent = "b".repeat(64);
        let orphan = directory.join(format!("{}.wav", "c".repeat(64)));
        let pending = directory.join("voice-10-2.pending");
        let unrelated = directory.join("saved-export.wav");
        for (key, seconds) in [(&old, 1), (&recent, 2)] {
            for extension in ["wav", "json"] {
                let path = directory.join(format!("{key}.{extension}"));
                fs::write(&path, b"12345").unwrap();
                File::options().write(true).open(path).unwrap()
                    .set_modified(UNIX_EPOCH + Duration::from_secs(seconds)).unwrap();
            }
        }
        for path in [&orphan, &pending, &unrelated] {
            fs::write(path, b"keep only the unowned file").unwrap();
        }
        let plan = directory.parent().unwrap().join("draft.json");
        fs::write(&plan, b"keep this plan").unwrap();
        let original = cache_status(&root).unwrap();
        assert_eq!(original.files, 6);
        let active = IN_USE.try_read().unwrap();
        trim(&root, 4, 10).unwrap();
        assert!(orphan.exists() && pending.exists());
        assert_eq!(cache_status(&root).unwrap().files, 6);
        drop(active);
        trim(&root, 4, 10).unwrap();
        assert_eq!(cache_status(&root).unwrap().files, 2);
        assert_eq!(cache_status(&root).unwrap().bytes, 10);
        assert!(directory.join(format!("{recent}.wav")).is_file());
        assert!(directory.join(format!("{recent}.json")).is_file());
        assert!(!directory.join(format!("{old}.wav")).exists());
        assert!(!directory.join(format!("{old}.json")).exists());
        assert!(!orphan.exists() && !pending.exists());
        assert_eq!(fs::read(unrelated).unwrap(), b"keep only the unowned file");
        assert_eq!(fs::read(plan).unwrap(), b"keep this plan");
        trim(&root, 1, 10).unwrap();
        assert_eq!(cache_status(&root).unwrap().files, 0);
        fs::remove_dir_all(root).unwrap();
    }
}
