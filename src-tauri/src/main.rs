#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod browser;
mod dependencies;
mod engine;
mod failure;
mod library;
mod local_media;
mod media;
mod model;
mod preferences;
mod providers;
mod queue;
mod sessions;
#[cfg(test)]
mod smoke;
mod store;
mod support;
mod tools;
mod updater;

use failure::Failure;
use model::{Format, Media};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tokio::sync::watch;

#[derive(Default)]
pub(crate) struct AppState {
    repository: Mutex<Option<store::Repository>>,
    runtime: queue::Runtime,
    metadata: Mutex<Option<(u64, watch::Sender<bool>)>>,
    latest_metadata: AtomicU64,
    bridge: browser::Bridge,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryView {
    #[serde(flatten)]
    item: library::LibraryItem,
    missing: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    settings: store::Settings,
    jobs: Vec<queue::Job>,
    library: Vec<LibraryView>,
    collections: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Startup {
    folder: PathBuf,
    engines_ready: bool,
    engine_error: Option<String>,
    #[serde(flatten)]
    snapshot: Snapshot,
}
fn snapshot(state: &AppState) -> Result<Snapshot, String> {
    let guard = state.repository.lock().unwrap();
    let repo = guard.as_ref().ok_or("The application data is not open.")?;
    Ok(Snapshot {
        settings: repo.data.settings.clone(),
        jobs: repo.data.jobs.clone(),
        library: repo
            .data
            .library
            .iter()
            .map(|item| LibraryView {
                missing: !item.path.is_file(),
                item: item.clone(),
            })
            .collect(),
        collections: repo.data.collections.clone(),
    })
}
fn config_root(state: &AppState) -> Result<PathBuf, String> {
    let guard = state.repository.lock().unwrap();
    Ok(guard
        .as_ref()
        .ok_or("Open the app first.")?
        .root()
        .to_owned())
}
pub(crate) fn test_data_dir() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        std::env::var_os("NABNAB_TEST_DATA_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

#[tauri::command]
async fn startup(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<Startup, String> {
    {
        let mut guard = state.repository.lock().unwrap();
        if guard.is_none() {
            let root = match test_data_dir() {
                Some(path) => path,
                None => app.path().app_config_dir().map_err(|e| e.to_string())?,
            };
            let default = app.path().download_dir().map_err(|e| e.to_string())?;
            let repo = store::Repository::open(&root, &default)?;
            sessions::cleanup(&root)?;
            *guard = Some(repo);
        }
    }
    let settings = snapshot(&state)?.settings;
    preferences::apply(&app, &settings)?;
    if !state.runtime.started.load(Ordering::SeqCst) && settings.processing_limit > 1 {
        state
            .runtime
            .processing
            .add_permits(settings.processing_limit - 1);
    }
    queue::start(app.clone(), state.inner().clone());
    if let Err(error) = browser::start(&app, &state.bridge) {
        let _ = app.emit("app-warning", error);
    }
    let error = engine::engine_dir(&app).err();
    Ok(Startup {
        folder: settings.folder,
        engines_ready: error.is_none(),
        engine_error: error,
        snapshot: snapshot(&state)?,
    })
}
#[tauri::command]
fn get_snapshot(state: State<Arc<AppState>>) -> Result<Snapshot, String> {
    snapshot(&state)
}
#[tauri::command]
async fn inspect_media(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    url: String,
    request_id: u64,
) -> Result<Media, Failure> {
    let url = model::validate_url(&url)?;
    let receiver = {
        let _repository = state.repository.lock().unwrap();
        if state.runtime.updating.load(Ordering::SeqCst) {
            return Err("Wait for the engine update to finish.".into());
        }
        let mut active = state.metadata.lock().unwrap();
        if request_id < state.latest_metadata.load(Ordering::SeqCst) {
            return Err(engine::CANCELLED.into());
        }
        state.latest_metadata.store(request_id, Ordering::SeqCst);
        if let Some((_, old)) = active.take() {
            let _ = old.send(true);
        }
        let (sender, receiver) = watch::channel(false);
        *active = Some((request_id, sender));
        receiver
    };
    let result = async {
        let url = providers::validate_remote(&url).await?;
        let root = engine::engine_dir(&app)?;
        let cookies = sessions::materialize(&config_root(&state)?, &url)?;
        let mut args = engine::inspect_args(&root, &url);
        if let Some(file) = &cookies {
            args.splice(
                0..0,
                ["--cookies".into(), file.path.to_string_lossy().into_owned()],
            );
        }
        let raw = engine::run_process(
            updater::executable(&app, &root)?,
            args,
            receiver,
            request_id,
            true,
            |_| {},
        )
        .await?;
        if state.latest_metadata.load(Ordering::SeqCst) != request_id {
            return Err(engine::CANCELLED.into());
        }
        model::parse_media(&raw, request_id)
    }
    .await;
    let mut active = state.metadata.lock().unwrap();
    if active.as_ref().is_some_and(|(id, _)| *id == request_id) {
        *active = None;
    }
    result.map_err(|e| Failure::classify(&e, "extraction"))
}
#[tauri::command]
fn cancel_job(state: State<Arc<AppState>>, kind: String, request_id: u64) -> Result<(), String> {
    if kind != "metadata" {
        return Err("Use the queue to cancel downloads.".into());
    }
    if let Some((id, sender)) = state.metadata.lock().unwrap().as_ref() {
        if *id == request_id {
            let _ = sender.send(true);
        }
    }
    Ok(())
}
#[tauri::command]
fn queue_download(
    app: AppHandle,
    state: State<Arc<AppState>>,
    urls: Vec<String>,
    titles: Vec<String>,
    format: Format,
    height: Option<u32>,
    audio_bitrate: Option<u16>,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    let jobs = {
        let mut guard = state.repository.lock().unwrap();
        queue::enqueue_download(
            guard.as_mut().ok_or("Open the app first.")?,
            urls,
            titles,
            format,
            height,
            audio_bitrate,
            &client_request,
        )?
    };
    for job in &jobs {
        let _ = app.emit("job-update", job);
    }
    state.runtime.notify.notify_one();
    Ok(jobs)
}
#[tauri::command]
fn queue_tool(
    app: AppHandle,
    state: State<Arc<AppState>>,
    ids: Vec<String>,
    options: tools::ToolOptions,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    let jobs = {
        let mut guard = state.repository.lock().unwrap();
        queue::enqueue_tool(
            guard.as_mut().ok_or("Open the app first.")?,
            ids,
            options,
            &client_request,
        )?
    };
    for job in &jobs {
        let _ = app.emit("job-update", job);
    }
    state.runtime.notify.notify_one();
    Ok(jobs)
}
#[tauri::command]
fn job_action(
    state: State<Arc<AppState>>,
    ids: Vec<String>,
    action: String,
) -> Result<Snapshot, String> {
    queue::action(&state, &ids, &action)?;
    snapshot(&state)
}
pub(crate) async fn accept_browser(
    app: AppHandle,
    request: browser::BrowserRequest,
) -> Result<Vec<String>, String> {
    if request.version != 1 {
        return Err("Unsupported bridge version.".into());
    }
    let format = match request.profile.as_str() {
        "mp4" => Format::Mp4,
        "mp3" => Format::Mp3,
        "source" => Format::Source,
        "audio" => Format::AudioOriginal,
        _ => return Err("Unknown profile.".into()),
    };
    let state = app.state::<Arc<AppState>>();
    let request_id = format!("{}:{}", request.client_id, request.request_id);
    let jobs = {
        let mut guard = state.repository.lock().unwrap();
        queue::enqueue_download(
            guard.as_mut().ok_or("Open nabnab first.")?,
            vec![request.url],
            vec![],
            format,
            None,
            None,
            &request_id,
        )?
    };
    for job in &jobs {
        let _ = app.emit("job-update", job);
    }
    state.runtime.notify.notify_one();
    Ok(jobs.into_iter().map(|j| j.id).collect())
}
#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: State<Arc<AppState>>,
    settings: store::Settings,
) -> Result<Snapshot, String> {
    settings.validate()?;
    if !settings.folder.is_dir() {
        return Err("Choose an existing destination folder.".into());
    }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    if settings.processing_limit != repo.data.settings.processing_limit
        && !state.runtime.active.lock().unwrap().is_empty()
    {
        return Err("Wait for active jobs before changing the processing limit.".into());
    }
    let old = std::mem::replace(&mut repo.data.settings, settings);
    if let Err(e) = preferences::apply(&app, &repo.data.settings).and_then(|_| repo.save()) {
        let _ = preferences::apply(&app, &old);
        repo.data.settings = old;
        return Err(e);
    }
    let next = repo.data.settings.processing_limit;
    if next > old.processing_limit {
        state
            .runtime
            .processing
            .add_permits(next - old.processing_limit);
    } else if next < old.processing_limit {
        state
            .runtime
            .processing
            .forget_permits(old.processing_limit - next);
    }
    drop(guard);
    state.runtime.notify.notify_one();
    snapshot(&state)
}
#[tauri::command]
async fn choose_folder(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Option<String>, String> {
    let current = snapshot(&state)?.settings.folder;
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_directory(current)
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let folder = selected
        .into_path()
        .map_err(|_| "Choose a local directory.")?;
    if !folder.is_absolute() || !folder.is_dir() {
        return Err("Choose an existing directory.".into());
    }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let previous = std::mem::replace(&mut repo.data.settings.folder, folder.clone());
    if let Err(e) = repo.save() {
        repo.data.settings.folder = previous;
        return Err(e);
    }
    Ok(Some(folder.to_string_lossy().into_owned()))
}
async fn pick_paths(app: AppHandle, folders: bool) -> Result<Vec<PathBuf>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dialog = app.dialog().file();
        let selected = if folders {
            dialog.blocking_pick_folders()
        } else {
            dialog
                .add_filter(
                    "Multimedia",
                    &[
                        "mp4", "mkv", "webm", "mov", "avi", "m4v", "mp3", "m4a", "aac", "opus",
                        "ogg", "wav", "flac", "jpg", "jpeg", "png", "webp", "gif", "bmp",
                    ],
                )
                .blocking_pick_files()
        };
        selected
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.into_path().map_err(|_| "Choose local files.".to_owned()))
            .collect()
    })
    .await
    .map_err(|e| e.to_string())?
}
async fn import_selected(
    app: &AppHandle,
    state: &Arc<AppState>,
    paths: Vec<PathBuf>,
) -> Result<Snapshot, String> {
    {
        let mut guard = state.repository.lock().unwrap();
        library::import_paths(guard.as_mut().ok_or("Open the app first.")?, &paths)?;
    }
    if let Ok(root) = dependencies::engine_root(app) {
        let items = {
            let guard = state.repository.lock().unwrap();
            guard
                .as_ref()
                .unwrap()
                .data
                .library
                .iter()
                .filter(|i| i.duration.is_none() && i.kind != "image")
                .take(200)
                .map(|i| (i.id.clone(), i.path.clone()))
                .collect::<Vec<_>>()
        };
        for (id, path) in items {
            let (_sender, cancel) = watch::channel(false);
            if let Ok(info) = media::probe(&root, &path, cancel).await {
                let mut guard = state.repository.lock().unwrap();
                if let Some(repo) = guard.as_mut() {
                    if let Some(item) = repo.data.library.iter_mut().find(|i| i.id == id) {
                        if let Ok(mut metadata) = library::index(
                            &path,
                            Some(&info),
                            item.source.clone(),
                            item.parent_id.clone(),
                        ) {
                            metadata.id = item.id.clone();
                            metadata.added_at = item.added_at;
                            metadata.collections = item.collections.clone();
                            metadata.tags = item.tags.clone();
                            *item = metadata;
                        }
                    }
                }
            }
        }
        let guard = state.repository.lock().unwrap();
        guard.as_ref().unwrap().save()?;
    }
    let _ = app.emit("library-update", ());
    snapshot(state)
}
#[tauri::command]
async fn import_media(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    folders: bool,
) -> Result<Snapshot, String> {
    let paths = pick_paths(app.clone(), folders).await?;
    import_selected(&app, state.inner(), paths).await
}
#[tauri::command]
async fn import_paths(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    paths: Vec<PathBuf>,
) -> Result<Snapshot, String> {
    if paths.len() > 200 {
        return Err("Import at most 200 chosen paths at once.".into());
    }
    import_selected(&app, state.inner(), paths).await
}
#[tauri::command]
async fn import_url_file(app: AppHandle, path: Option<PathBuf>) -> Result<Vec<String>, String> {
    let path = match path {
        Some(p) => p,
        None => {
            let picked = tauri::async_runtime::spawn_blocking(move || {
                app.dialog()
                    .file()
                    .add_filter("URL list", &["txt"])
                    .blocking_pick_file()
            })
            .await
            .map_err(|e| e.to_string())?;
            let Some(p) = picked else { return Ok(vec![]) };
            p.into_path().map_err(|_| "Choose a local text file.")?
        }
    };
    if !path.is_absolute()
        || path
            .extension()
            .is_none_or(|e| !e.eq_ignore_ascii_case("txt"))
    {
        return Err("Choose a local .txt file.".into());
    }
    let meta = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("Choose a regular text file.".into());
        }
    }
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > 1024 * 1024 {
        return Err("Choose a regular text file smaller than 1 MiB.".into());
    }
    let text = fs::read_to_string(path).map_err(|_| "Use a UTF-8 text file.")?;
    let lines: Vec<_> = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if lines.len() > 200 {
        return Err("The URL list contains more than 200 links.".into());
    }
    lines.into_iter().map(model::validate_url).collect()
}
#[tauri::command]
fn update_library(
    state: State<Arc<AppState>>,
    id: String,
    collections: Vec<String>,
    tags: Vec<String>,
) -> Result<Snapshot, String> {
    let mut guard = state.repository.lock().unwrap();
    library::update(
        guard.as_mut().ok_or("Open the app first.")?,
        &id,
        collections,
        tags,
    )?;
    drop(guard);
    snapshot(&state)
}
#[tauri::command]
fn remove_library(state: State<Arc<AppState>>, id: String) -> Result<Snapshot, String> {
    let mut guard = state.repository.lock().unwrap();
    library::remove(guard.as_mut().ok_or("Open the app first.")?, &id)?;
    drop(guard);
    snapshot(&state)
}
#[tauri::command]
async fn locate_media(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<Snapshot, String> {
    let paths = pick_paths(app, false).await?;
    if let Some(path) = paths.first() {
        let mut guard = state.repository.lock().unwrap();
        library::locate(guard.as_mut().ok_or("Open the app first.")?, &id, path)?;
    }
    snapshot(&state)
}
#[tauri::command]
fn open_media(state: State<Arc<AppState>>, id: String, folder: bool) -> Result<(), String> {
    let path = {
        let guard = state.repository.lock().unwrap();
        let repo = guard.as_ref().ok_or("Open the app first.")?;
        let item = repo
            .data
            .library
            .iter()
            .find(|i| i.id == id)
            .ok_or("File not found in the library.")?;
        library::authorized_path(&item.path)?
    };
    open_local(if folder {
        path.parent().ok_or("Folder not found.")?
    } else {
        &path
    })
}
fn open_local(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let file: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let operation: Vec<_> = "open\0".encode_utf16().collect();
        let result = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result as isize <= 32 {
            return Err(
                "Windows could not open the selected file. Choose a compatible system player."
                    .into(),
            );
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("System playback is available in the Windows build.".into())
    }
}
#[tauri::command]
async fn engine_status(app: AppHandle) -> Vec<dependencies::EngineStatus> {
    dependencies::status(&app).await
}
#[tauri::command]
async fn thumbnail(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<String, String> {
    support::thumbnail(&app, state.inner(), &id).await
}
#[tauri::command]
fn cache_status(state: State<Arc<AppState>>) -> Result<support::CacheStatus, String> {
    support::cache_status(&state)
}
#[tauri::command]
async fn clear_cache(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<support::CacheStatus, String> {
    let status = support::cache_status(&state)?;
    let dialog = app.clone();
    let message=format!("Remove {} cached thumbnails ({} bytes)? Downloads, sessions and active jobs are not included.",status.files,status.bytes);
    let accepted = tauri::async_runtime::spawn_blocking(move || {
        dialog
            .dialog()
            .message(message)
            .title("nabnab")
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::YesNo)
            .blocking_show()
    })
    .await
    .map_err(|e| e.to_string())?;
    if accepted {
        support::clear_cache(&state).await
    } else {
        Ok(status)
    }
}
#[tauri::command]
async fn export_diagnostics(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Option<String>, String> {
    let bytes = support::diagnostics(&state, dependencies::status(&app).await)?;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_file_name("nabnab-diagnostics.json")
            .add_filter("JSON", &["json"])
            .blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(path) = picked else { return Ok(None) };
    let path = path.into_path().map_err(|_| "Choose a local file.")?;
    support::write_diagnostics(&path, &bytes)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}
fn with_engine_update(state: &AppState) -> Result<(), String> {
    let guard = state.repository.lock().unwrap();
    if guard
        .as_ref()
        .is_some_and(|r| r.data.jobs.iter().any(|j| j.state.active()))
        || !state.runtime.active.lock().unwrap().is_empty()
        || state.metadata.lock().unwrap().is_some()
    {
        return Err("Wait for active jobs and link analysis before updating engines.".into());
    }
    state
        .runtime
        .updating
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map_err(|_| "An engine update is already running.")?;
    Ok(())
}
#[tauri::command]
async fn repair_engines(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let dialog = app.clone();
    let accepted=tauri::async_runtime::spawn_blocking(move||dialog.dialog().message("Download the pinned yt-dlp and Deno engines from their official releases, validate their hashes and prepare a managed engine set? FFmpeg is supplied by the installer.").title("nabnab").kind(MessageDialogKind::Info).buttons(MessageDialogButtons::YesNo).blocking_show()).await.map_err(|e|e.to_string())?;
    if !accepted {
        return Ok(());
    }
    with_engine_update(&state)?;
    let result = dependencies::repair(&app).await;
    state.runtime.updating.store(false, Ordering::SeqCst);
    state.runtime.notify.notify_one();
    result
}
#[tauri::command]
async fn engine_update(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    mode: String,
) -> Result<updater::Status, Failure> {
    if !matches!(mode.as_str(), "check" | "install" | "rollback") {
        return Err("Unknown engine operation.".into());
    }
    with_engine_update(&state)?;
    let result = async {
        let root = engine::engine_dir(&app)?;
        let data = updater::data_dir(&app)?;
        updater::update(&data, &root, &mode).await
    }
    .await;
    state.runtime.updating.store(false, Ordering::SeqCst);
    state.runtime.notify.notify_one();
    result.map_err(|e| Failure::classify(&e, "engine-update"))
}
#[tauri::command]
async fn rollback_engines(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    with_engine_update(&state)?;
    let result = dependencies::rollback(&app).await;
    state.runtime.updating.store(false, Ordering::SeqCst);
    state.runtime.notify.notify_one();
    result
}
#[tauri::command]
fn sessions_status(state: State<Arc<AppState>>) -> Result<Vec<sessions::SessionInfo>, String> {
    sessions::list(&config_root(&state)?)
}
#[tauri::command]
async fn import_session(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    site: String,
) -> Result<(), String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Netscape cookies", &["txt"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Some(file) = picked {
        sessions::import(
            &config_root(&state)?,
            &site,
            &file
                .into_path()
                .map_err(|_| "Choose a local cookie file.")?,
        )?;
    }
    Ok(())
}
#[tauri::command]
fn revoke_session(state: State<Arc<AppState>>, site: String) -> Result<(), String> {
    sessions::revoke(&config_root(&state)?, &site)
}
#[tauri::command]
fn bridge_status(state: State<Arc<AppState>>) -> browser::Status {
    browser::status(&state.bridge)
}
#[tauri::command]
fn bridge_pair(
    app: AppHandle,
    state: State<Arc<AppState>>,
    extension_id: String,
) -> Result<browser::Pairing, String> {
    browser::pair(&app, &state.bridge, &extension_id)
}
#[tauri::command]
fn bridge_revoke(
    app: AppHandle,
    state: State<Arc<AppState>>,
    extension_id: String,
) -> Result<(), String> {
    browser::revoke(&app, &state.bridge, &extension_id)
}

pub(crate) fn checked_output(staging: &Path, file: &str) -> Result<PathBuf, String> {
    let staging = fs::canonicalize(staging).map_err(|_| "Staging directory unavailable.")?;
    let output =
        fs::canonicalize(file).map_err(|_| "The engine did not produce the expected file.")?;
    if output.parent() != Some(staging.as_path()) || !output.is_file() {
        return Err("Output escaped the job directory.".into());
    }
    Ok(output)
}
pub(crate) fn publish(staging: &Path, file: &str, destination: &Path) -> Result<PathBuf, String> {
    let output = checked_output(staging, file)?;
    let target = destination.join(output.file_name().ok_or("Invalid output filename.")?);
    #[cfg(windows)]
    let published = {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = output.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(from.as_ptr(), to.as_ptr(), 0)
        } != 0
        {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    };
    #[cfg(not(windows))]
    let published = fs::hard_link(&output, &target);
    published.map_err(|e|if e.kind()==std::io::ErrorKind::AlreadyExists{"This file already exists. Choose another destination or rename the existing file. The verified result was kept.".into()}else{format!("Could not save the finished file. Check space and permissions: {e}")})?;
    Ok(target)
}
fn main() {
    let state = Arc::new(AppState::default());
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .register_uri_scheme_protocol("nabmedia", |context, request| {
            local_media::serve(context.app_handle(), request)
        })
        .invoke_handler(tauri::generate_handler![
            startup,
            get_snapshot,
            inspect_media,
            cancel_job,
            queue_download,
            queue_tool,
            job_action,
            save_settings,
            choose_folder,
            import_media,
            import_paths,
            import_url_file,
            update_library,
            remove_library,
            locate_media,
            open_media,
            thumbnail,
            cache_status,
            clear_cache,
            export_diagnostics,
            engine_status,
            repair_engines,
            rollback_engines,
            engine_update,
            sessions_status,
            import_session,
            revoke_session,
            bridge_status,
            bridge_pair,
            bridge_revoke
        ])
        .build(tauri::generate_context!())
        .expect("Could not start nabnab")
        .run(|app, event| match event {
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                let state = app.state::<Arc<AppState>>();
                let hide = {
                    let guard = state.repository.lock().unwrap();
                    guard.as_ref().is_some_and(|r| {
                        r.data.settings.tray_enabled && r.data.settings.close_to_tray
                    })
                };
                if hide && preferences::close_to_tray(app).is_ok() {
                    api.prevent_close();
                }
            }
            tauri::RunEvent::Exit => {
                let state = app.state::<Arc<AppState>>();
                {
                    let _guard = state.repository.lock().unwrap();
                    state.runtime.updating.store(true, Ordering::SeqCst);
                    for cancel in state.runtime.active.lock().unwrap().values() {
                        let _ = cancel.send(true);
                    }
                    if let Some((_, sender)) = state.metadata.lock().unwrap().as_ref() {
                        let _ = sender.send(true);
                    }
                }
                browser::stop(&state.bridge);
            }
            _ => {}
        });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_never_replaces_existing_media() {
        let root = std::env::temp_dir().join(format!(
            "nabnab-publish-{}-{}",
            std::process::id(),
            queue::now()
        ));
        let stage = root.join("stage");
        fs::create_dir_all(&stage).unwrap();
        fs::write(stage.join("test.mp4"), b"new").unwrap();
        fs::write(root.join("test.mp4"), b"original").unwrap();
        assert!(publish(&stage, &stage.join("test.mp4").to_string_lossy(), &root).is_err());
        assert_eq!(fs::read(root.join("test.mp4")).unwrap(), b"original");
        assert!(checked_output(&stage, &root.join("test.mp4").to_string_lossy()).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
