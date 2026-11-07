use crate::{dependencies, engine, failure, library, AppState};
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use tauri::AppHandle;
use tokio::sync::{watch, Semaphore};

static THUMBNAILS: OnceLock<Semaphore> = OnceLock::new();
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
    name.ends_with(".jpg")
        && name.len() < 180
        && name.bytes().all(|b| {
            b.is_ascii_hexdigit() || b == b'-' || b == b'.' || b == b'j' || b == b'p' || b == b'g'
        })
}
pub fn thumbnail_path(root: &Path, item: &library::LibraryItem) -> Result<PathBuf, String> {
    let metadata = fs::metadata(&item.path)
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
    Ok(root.join(format!("{}-{}-{stamp}.jpg", item.id, metadata.len())))
}
#[derive(Serialize)]
pub struct CacheStatus {
    pub bytes: u64,
    pub files: usize,
}
fn files(root: &Path) -> Result<Vec<(PathBuf, fs::Metadata)>, String> {
    if !root.exists() {
        return Ok(vec![]);
    }
    let root = library::authorized_path(root)?;
    if fs::read(root.join(".nabnab-cache-v1")).ok().as_deref() != Some(b"nabnab thumbnails v1") {
        return Err(
            "The thumbnail cache ownership marker is missing. No files were removed.".into(),
        );
    }
    let mut result = vec![];
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if cache_file(&name) {
            let path = library::authorized_path(&entry.path())?;
            let meta = fs::metadata(&path).map_err(|e| e.to_string())?;
            if meta.is_file() {
                result.push((path, meta));
            }
        }
    }
    Ok(result)
}
pub fn cache_status(state: &AppState) -> Result<CacheStatus, String> {
    let entries = files(&cache_root(state)?)?;
    Ok(CacheStatus {
        bytes: entries.iter().map(|(_, m)| m.len()).sum(),
        files: entries.len(),
    })
}
pub async fn clear_cache(state: &AppState) -> Result<CacheStatus, String> {
    let _permit = slots().acquire().await.map_err(|e| e.to_string())?;
    let entries = files(&cache_root(state)?)?;
    for (path, _) in entries {
        fs::remove_file(path).map_err(|e| format!("Could not remove a thumbnail: {e}"))?;
    }
    cache_status(state)
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
    if item.kind == "audio" && !item.details.audio.as_ref().is_some_and(|tags| tags.embedded_cover) {
        return Err("This audio file has no indexed embedded cover.".into());
    }
    let input = library::authorized_path(&item.path)?;
    let root = cache_root(state)?;
    let _permit = slots().acquire().await.map_err(|e| e.to_string())?;
    if !root.exists() {
        fs::create_dir(&root).map_err(|e| e.to_string())?;
        fs::write(root.join(".nabnab-cache-v1"), b"nabnab thumbnails v1")
            .map_err(|e| e.to_string())?;
    }
    let _ = files(&root)?;
    let output = thumbnail_path(&root, &item)?;
    if !output.is_file() {
        let mut entries = files(&root)?;
        entries.sort_by_key(|(_, m)| m.modified().ok());
        let mut total = entries.iter().map(|(_, m)| m.len()).sum::<u64>();
        let mut count = entries.len();
        for (path, meta) in entries {
            if count < 512 && total < 128 * 1024 * 1024 {
                break;
            }
            fs::remove_file(path).map_err(|e| e.to_string())?;
            total = total.saturating_sub(meta.len());
            count -= 1;
        }
        let engines = dependencies::engine_root(app)?;
        let (_sender, cancel) = watch::channel(false);
        let args=["-hide_banner","-v","error","-nostdin","-n","-protocol_whitelist","file","-format_whitelist","mov,matroska,webm,avi,mp3,aac,ogg,wav,flac,asf,mpeg,mpegts,image2,jpeg_pipe,png_pipe,webp_pipe,bmp_pipe,gif","-i"].into_iter().map(str::to_owned).chain([input.to_string_lossy().into_owned(),"-vf".into(),"scale=320:180:force_original_aspect_ratio=decrease".into(),"-frames:v".into(),"1".into(),"-q:v".into(),"5".into(),output.to_string_lossy().into_owned()]).collect();
        if let Err(e) = engine::tool(&engines, "ffmpeg.exe", args, cancel).await {
            let _ = fs::remove_file(&output);
            return Err(failure::sanitize(&e));
        }
        if !output.is_file() {
            return Err("The thumbnail could not be generated.".into());
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
        for name in [
            "../abc.jpg",
            "cookies.txt",
            "active.json",
            "file.exe",
            "thing.jpg",
        ] {
            assert!(!cache_file(name), "{name}");
        }
    }
}
