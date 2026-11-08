use crate::{dependencies, engine, failure, library, AppState};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use tauri::AppHandle;
use tokio::sync::{watch, Semaphore};

static THUMBNAILS: OnceLock<Semaphore> = OnceLock::new();
const CACHE_FILES: usize = 512;
const CACHE_BYTES: u64 = 128 * 1024 * 1024;
const CACHE_MARKER: &str = ".nabnab-cache-v1";
const CACHE_SIGNATURE: &[u8] = b"nabnab thumbnails v1";

fn slots() -> &'static Semaphore {
    THUMBNAILS.get_or_init(|| Semaphore::new(1))
}
fn cache_root(state: &AppState) -> Result<PathBuf, String> {
    let guard = state.repository.lock().unwrap();
    Ok(guard
        .as_ref()
        .ok_or("Open the app first.")?
        .root()
        .join("thumbnail-cache"))
}
fn cache_file(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".jpg").filter(|_| name.len() < 180) else {
        return false;
    };
    let mut fields = stem.rsplit('-');
    let last = fields.next().unwrap_or_default();
    let decimal = |value: &str| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
    if !decimal(last) {
        if last.len() != 16 || !last.bytes().all(|b| b.is_ascii_hexdigit())
            || !fields.next().is_some_and(decimal)
        {
            return false;
        }
    }
    fields.next().is_some_and(decimal)
        && fields.clone().next().is_some()
        && fields.all(|field| !field.is_empty() && field.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub fn thumbnail_path(root: &Path, item: &library::LibraryItem) -> Result<PathBuf, String> {
    let selected = item.details.audio_cover.as_deref().map(library::authorized_path).transpose()?;
    let input = selected.as_deref().unwrap_or(&item.path);
    let metadata = fs::metadata(input)
        .map_err(|_| "The file was not found. Locate it in the library.")?;
    let stamp = metadata
        .modified()
        .ok()
        .and_then(|v| v.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|v| v.as_nanos())
        .unwrap_or(0);
    if !item.id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err("Invalid media identifier.".into());
    }
    let cover = item.details.audio_cover.as_ref().map(|path| {
        let hash = format!("{:x}", Sha256::digest(path.to_string_lossy().as_bytes()));
        format!("-{}", &hash[..16])
    }).unwrap_or_default();
    Ok(root.join(format!("{}-{}-{stamp}{cover}.jpg", item.id, metadata.len())))
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStatus {
    pub bytes: u64,
    pub files: usize,
    pub max_bytes: u64,
    pub max_files: usize,
}
fn checked_cache(root: &Path) -> Result<Option<PathBuf>, String> {
    match fs::symlink_metadata(root) {
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    let root = library::authorized_path(root)?;
    let marker = library::authorized_path(&root.join(CACHE_MARKER)).ok();
    if marker
        .filter(|path| fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() == CACHE_SIGNATURE.len() as u64))
        .and_then(|path| fs::read(path).ok())
        .as_deref() != Some(CACHE_SIGNATURE)
    {
        return Err("The thumbnail cache ownership marker could not be verified.".into());
    }
    Ok(Some(root))
}

fn files(root: &Path) -> Result<Vec<(PathBuf, fs::Metadata)>, String> {
    let mut result = vec![];
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if cache_file(&entry.file_name().to_string_lossy()) {
            match fs::symlink_metadata(entry.path()) {
                Ok(meta) if meta.is_file() && !library::is_link(&meta) => result.push((entry.path(), meta)),
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        }
    }
    Ok(result)
}
pub fn cache_status(state: &AppState) -> Result<CacheStatus, String> {
    let entries = match checked_cache(&cache_root(state)?)? {
        Some(root) => files(&root)?,
        None => Vec::new(),
    };
    Ok(CacheStatus {
        bytes: entries.iter().map(|(_, m)| m.len()).sum(),
        files: entries.len(),
        max_bytes: CACHE_BYTES,
        max_files: CACHE_FILES,
    })
}
pub async fn clear_cache(state: &AppState) -> Result<CacheStatus, String> {
    let _permit = slots().acquire().await.map_err(|e| e.to_string())?;
    if let Some(root) = checked_cache(&cache_root(state)?)? {
        for (path, _) in files(&root)? {
            remove_thumbnail(&path).map_err(|e| format!("Could not remove a thumbnail: {e}"))?;
        }
    }
    cache_status(state)
}

fn remove_thumbnail(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

pub fn trim_thumbnail_cache(root: &Path) -> Result<(), String> {
    if let Some(root) = checked_cache(&root.join("thumbnail-cache"))? {
        trim(&root, None, CACHE_FILES, CACHE_BYTES)?;
    }
    Ok(())
}

fn trim(root: &Path, keep: Option<&Path>, max_files: usize, max_bytes: u64) -> Result<(), String> {
    let mut entries = files(root)?;
    let mut count = entries.len();
    let mut total = entries.iter().map(|(_, meta)| meta.len()).sum::<u64>();
    if count <= max_files && total <= max_bytes {
        return Ok(());
    }
    entries.sort_unstable_by(|(a_path, a), (b_path, b)| {
        a.modified().ok().cmp(&b.modified().ok()).then_with(|| a_path.cmp(b_path))
    });
    for (path, meta) in entries {
        if keep == Some(path.as_path()) || remove_thumbnail(&path).is_err() {
            continue;
        }
        count -= 1;
        total -= meta.len();
        if count <= max_files && total <= max_bytes {
            return Ok(());
        }
    }
    Err("The thumbnail cache could not be brought within its limits.".into())
}

pub async fn thumbnail(app: &AppHandle, state: &Arc<AppState>, id: &str) -> Result<String, String> {
    let item = {
        let guard = state.repository.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Open the app first.")?
            .data
            .library
            .iter()
            .find(|i| i.id == id)
            .cloned()
            .ok_or("Library item not found.")?
    };
    if item.kind == "audio" && item.details.audio_cover.is_none()
        && !item.details.audio.as_ref().is_some_and(|tags| tags.embedded_cover) {
        return Err("This audio file has no indexed embedded cover.".into());
    }
    let input = library::authorized_path(item.details.audio_cover.as_deref().unwrap_or(&item.path))?;
    let root = cache_root(state)?;
    let _permit = slots().acquire().await.map_err(|e| e.to_string())?;
    let cache = match checked_cache(&root)? {
        Some(cache) => cache,
        None => {
            library::authorized_path(root.parent().ok_or("Invalid thumbnail cache path.")?)?;
            fs::create_dir(&root).map_err(|e| e.to_string())?;
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join(CACHE_MARKER))
                .and_then(|mut marker| marker.write_all(CACHE_SIGNATURE))
                .map_err(|e| e.to_string())?;
            checked_cache(&root)?.ok_or("The thumbnail cache is unavailable.")?
        }
    };
    let output = thumbnail_path(&root, &item)?;
    let exists = match fs::symlink_metadata(&output) {
        Ok(meta) if meta.is_file() && !library::is_link(&meta) => {
            if meta.len() == 0 {
                remove_thumbnail(&output).map_err(|e| e.to_string())?;
            }
            meta.len() != 0
        }
        Ok(_) => return Err("The thumbnail path is not a regular cache file.".into()),
        Err(e) if e.kind() == ErrorKind::NotFound => false,
        Err(e) => return Err(e.to_string()),
    };
    if !exists {
        let engines = dependencies::engine_root(app)?;
        let (_sender, cancel) = watch::channel(false);
        let args=["-hide_banner","-v","error","-nostdin","-n","-protocol_whitelist","file","-format_whitelist","mov,matroska,webm,avi,mp3,aac,ogg,wav,flac,asf,mpeg,mpegts,image2,jpeg_pipe,png_pipe,webp_pipe,bmp_pipe,gif","-i"].into_iter().map(str::to_owned).chain([input.to_string_lossy().into_owned(),"-vf".into(),"scale=320:180:force_original_aspect_ratio=decrease".into(),"-frames:v".into(),"1".into(),"-q:v".into(),"5".into(),output.to_string_lossy().into_owned()]).collect();
        if let Err(e) = engine::tool(&engines, "ffmpeg.exe", args, cancel).await {
            let _ = fs::remove_file(&output);
            return Err(failure::sanitize(&e));
        }
        let size = fs::metadata(&output).map(|meta| meta.len()).unwrap_or(0);
        if size == 0 || size > CACHE_BYTES {
            let _ = remove_thumbnail(&output);
            return Err("The thumbnail could not be generated.".into());
        }
        let published = cache.join(output.file_name().ok_or("Invalid thumbnail path.")?);
        if let Err(error) = trim(&cache, Some(&published), CACHE_FILES, CACHE_BYTES) {
            let _ = remove_thumbnail(&published);
            return Err(error);
        }
    }
    Ok(format!("http://nabmedia.localhost/thumbnail/{id}"))
}
pub fn diagnostics(
    state: &AppState,
    engines: Vec<dependencies::EngineStatus>,
) -> Result<Vec<u8>, String> {
    let guard = state.repository.lock().unwrap();
    let repo = guard.as_ref().ok_or("Open the app first.")?;
    let jobs:Vec<_>=repo.data.jobs.iter().map(|j|serde_json::json!({"state":j.state,"attempt":j.attempt,"error":j.error.as_ref().map(|e|serde_json::json!({"code":e.code,"technical":failure::sanitize(&e.technical)}))})).collect();
    let engines:Vec<_>=engines.into_iter().map(|e|serde_json::json!({"name":e.name,"version":e.version,"status":e.status,"source":e.source,"bytes":e.size})).collect();
    serde_json::to_vec_pretty(&serde_json::json!({"product":"nabnab","version":env!("CARGO_PKG_VERSION"),"os":std::env::consts::OS,"engines":engines,"jobs":jobs,"libraryCount":repo.data.library.len(),"collectionCount":repo.data.collections.len()})).map_err(|e|e.to_string())
}
pub fn write_diagnostics(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Choose a new diagnostic filename: {e}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_only_accepts_generated_image_names() {
        assert!(cache_file("abc-12-345.jpg"));
        assert!(cache_file("abc-12-345-a0123456789bcdef.jpg"));
        for name in [
            "../abc.jpg",
            "cookies.txt",
            "active.json",
            "file.exe",
            "thing.jpg",
            "dead.jpg",
            "abc-12.jpg",
        ] {
            assert!(!cache_file(name), "{name}");
        }
    }

    #[test]
    fn trimming_uses_the_byte_budget_and_preserves_the_new_thumbnail() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("nabnab-cache-{}-{stamp}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let oldest = root.join("abc-4-1.jpg");
        let recent = root.join("abc-4-2.jpg");
        let published = root.join("abc-4-3.jpg");
        for (index, path) in [&published, &oldest, &recent].into_iter().enumerate() {
            fs::write(path, b"test").unwrap();
            fs::File::options().write(true).open(path).unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(index as u64))
                .unwrap();
        }
        fs::write(root.join("dead.jpg"), b"unrelated").unwrap();
        assert!(checked_cache(&root).is_err());
        assert!(oldest.is_file());
        fs::write(root.join(CACHE_MARKER), CACHE_SIGNATURE).unwrap();
        let checked = checked_cache(&root).unwrap().unwrap();
        trim(&checked, Some(&checked.join("abc-4-3.jpg")), 3, 8).unwrap();
        assert!(!oldest.exists());
        assert!(recent.is_file() && published.is_file());
        assert_eq!(fs::read(root.join("dead.jpg")).unwrap(), b"unrelated");
        assert_eq!(fs::read(root.join(CACHE_MARKER)).unwrap(), CACHE_SIGNATURE);
        fs::remove_dir_all(root).unwrap();
    }
}
