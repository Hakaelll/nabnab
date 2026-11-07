#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod batch;
mod file_sources;
mod personal_servers;
mod web_archive;
mod portable_zip;
mod file_organization;
mod image_assets;
mod workspaces;
mod screen_capture;
mod automation;
mod automation_wire;
mod automation_credentials;
mod automation_pipe;
mod capture_operations;
mod capture_shortcut;
mod voice;
mod voice_sources;
mod voice_operations;
mod portable_collections;
mod file_operations;
mod document_edits;
mod document_operations;
mod ocr;
mod ocr_component;
mod public_sources;
mod cloud_storage;
mod browser;
mod captions;
mod captions_editor;
mod media_edits;
mod components;
mod component_archives;
mod courses;
mod transcription;
mod dependencies;
mod direct;
mod engine;
mod failure;
mod source_exports;
mod danmaku;
mod advanced_files;
mod palettes;
mod audio_preview;
mod galleries;
mod gallery_sources;
mod gallery_auth;
mod gallery_exports;
mod documents;
mod telegram;
mod live;
mod federated;
mod federated_auth;
mod library;
mod library_features;
mod audio_library;
mod delimited;
mod local_media;
mod media;
mod model;
mod options;
#[cfg(test)]
mod platform_checks;
mod podcasts;
mod preferences;
mod providers;
mod queue;
mod scheduling;
mod sessions;
#[cfg(test)]
mod smoke;
mod sources;
mod store;
mod support;
mod tools;
mod torrents;
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
    headless: bool,
    automation_deadlines: Mutex<std::collections::HashMap<String,(u64,std::time::Instant)>>,
    automation_pipe: Mutex<Option<automation_pipe::Server>>,
    capture: capture_operations::HostState,
    document_previews: document_operations::PreviewState,
    file_previews: file_operations::PreviewState,
    file_sources: file_sources::BrowserState,
    personal_servers: personal_servers::BrowserState,
    audio_playlist_browser: audio_library::playlist_import::Browser,
    repository: Mutex<Option<store::Repository>>,
    runtime: queue::Runtime,
    metadata: Mutex<Option<(u64, watch::Sender<bool>)>>,
    latest_metadata: AtomicU64,
    bridge: browser::Bridge,
    galleries: Mutex<std::collections::HashMap<String, galleries::Gallery>>,
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
    presets: Vec<options::Preset>,
    rules: Vec<options::Rule>,
    drafts: Vec<batch::Draft>,
    sources: Vec<sources::FollowedSource>,
    smart_collections: Vec<library_features::SmartCollection>,
    courses: Vec<courses::Course>,
    audio_playlists: Vec<audio_library::Playlist>,
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
        presets: repo.data.presets.clone(),
        rules: repo.data.rules.clone(),
        drafts: repo.data.drafts.clone(),
        sources: repo.data.sources.clone(),
        smart_collections: repo.data.smart_collections.clone(),
        courses: repo.data.courses.clone(),
        audio_playlists: repo.data.audio_playlists.clone(),
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
            .or_else(||std::env::var_os("NABNAB_DATA_DIR").map(PathBuf::from).filter(|p|p.is_absolute()))
    }
    #[cfg(not(debug_assertions))]
    {
        std::env::var_os("NABNAB_DATA_DIR").map(PathBuf::from).filter(|p|p.is_absolute())
    }
}

fn open_repository(app: &AppHandle, state: &AppState) -> Result<store::Settings, String> {
    let mut guard = state.repository.lock().unwrap();
    if let Some(repo) = guard.as_ref() {
        return Ok(repo.data.settings.clone());
    }
    let root = match test_data_dir() {
        Some(path) => path,
        None => app.path().app_config_dir().map_err(|e| e.to_string())?,
    };
    let default = app.path().download_dir().map_err(|e| e.to_string())?;
    let repo = store::Repository::open(&root, &default)?;
    sessions::cleanup(&root)?;
    let settings = repo.data.settings.clone();
    *guard = Some(repo);
    Ok(settings)
}
fn start_queue(app: &AppHandle, state: &Arc<AppState>, processing_limit: usize) {
    if !state.runtime.started.load(Ordering::SeqCst) && processing_limit > 1 {
        state
            .runtime
            .processing
            .add_permits(processing_limit - 1);
    }
    queue::start(app.clone(), state.clone());
}
#[tauri::command]
async fn startup(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<Startup, String> {
    let settings = open_repository(&app, &state)?;
    preferences::apply(&app, &settings)?;
    start_queue(&app, state.inner(), settings.processing_limit);
    sources::start(app.clone(), state.inner().clone());
    if let Err(error)=automation::start(&app,state.inner()){let _=app.emit("app-warning",error);}
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
    if providers::course_page(&url){return Err("Open this course in Platforms → Courses to select authorized materials.".into());}
    let receiver = {
        let repository = state.repository.lock().unwrap();
        queue::check_cooldown(repository.as_ref().ok_or("Open the app first.")?, &url)?;
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
        let _metadata_permit = batch::metadata_permit(&url).await?;
        let url = providers::validate_remote(&url).await?;
        if providers::identify(&url)?.id == providers::ProviderId::Generic {
            match direct::inspect(&url, request_id, receiver.clone()).await {
                Ok(metadata)=>{if state.latest_metadata.load(Ordering::SeqCst)!=request_id{return Err(engine::CANCELLED.into());}return Ok(metadata);},
                Err(error) if direct::engine_fallback(&error)=>{},
                Err(error)=>return Err(error),
            }
        }
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
    {
        let mut active = state.metadata.lock().unwrap();
        if active.as_ref().is_some_and(|(id, _)| *id == request_id) {
            *active = None;
        }
    }
    result.map_err(|raw| {
        let mut failure = Failure::classify(&raw, "extraction");
        if failure.code == "rate_limited" {
            failure.retry_after = Some(queue::cooldown_seconds(failure.retry_after));
        }
        if let Err(error) = queue::record_metadata_failure(&state, &url, &failure) {
            failure.technical.push_str(&format!(
                "\nCould not persist the shared cooldown: {}",
                failure::sanitize(&error)
            ));
        }
        failure
    })
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
fn prepare_download(
    state: State<Arc<AppState>>,
    url: String,
    title: String,
    creator: String,
    preset_id: Option<String>,
) -> Result<options::PreparedDownload, String> {
    let content_id = options::content_identity(&url)?;
    let provider = providers::identify(&url)?;
    let context = options::Context {
        url: url.clone(),
        title: title.clone(),
        creator,
        content_id: content_id.rsplit(':').next().unwrap_or(&content_id).into(),
        platform: provider.name,
        content_type: "video".into(),
        ..Default::default()
    };
    let guard = state.repository.lock().unwrap();
    let repo = guard.as_ref().ok_or("Open the app first.")?;
    let options = options::resolve(repo, &context, preset_id.as_deref(), None)?;
    Ok(options::PreparedDownload {
        url,
        title,
        content_id,
        options,
        source_id: None,
        group_id: None,
    })
}
#[tauri::command]
fn queue_prepared(
    app: AppHandle,
    state: State<Arc<AppState>>,
    items: Vec<options::PreparedDownload>,
    request: String,
) -> Result<Vec<queue::Job>, String> {
    let jobs = {
        let mut guard = state.repository.lock().unwrap();
        queue::enqueue_prepared(
            guard.as_mut().ok_or("Open the app first.")?,
            items,
            &request,
        )?
    };
    for job in &jobs {
        let _ = app.emit("job-update", job);
    }
    state.runtime.notify.notify_one();
    Ok(jobs)
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
    subtitle_language: Option<String>,
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
            subtitle_language,
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
fn queue_torrent(app: AppHandle, state: State<Arc<AppState>>, reference: String, manifest_id: String, options: torrents::TorrentOptions, client_request: String) -> Result<Vec<queue::Job>, String> {
    components::executable(&app, "torrent")?;
    let path = torrents::resolve_reference(&app, &reference)?;
    let manifest = torrents::read_manifest(&path)?;
    if manifest.id != manifest_id { return Err("Torrent metadata changed. Inspect it again.".into()); }
    let mut guard = state.repository.lock().unwrap();
    let jobs = queue::enqueue_torrent(guard.as_mut().ok_or("Open nabnab first.")?, reference, &manifest, options, &client_request)?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}
#[tauri::command]
fn configure_jobs(
    state: State<Arc<AppState>>,
    ids: Vec<String>,
    priority: scheduling::Priority,
    schedule: scheduling::Schedule,
    order_action: Option<String>,
) -> Result<Snapshot, String> {
    schedule.validate()?;
    if order_action
        .as_deref()
        .is_some_and(|value| !matches!(value, "next" | "up" | "down"))
    {
        return Err("Unsupported queue order action.".into());
    }
    {
        let mut guard = state.repository.lock().unwrap();
        let repo = guard.as_mut().ok_or("Open the app first.")?;
        if ids.is_empty()
            || ids.iter().any(|id| {
                !repo.data.jobs.iter().any(|j| {
                    j.id == *id
                        && matches!(j.state, queue::JobState::Pending | queue::JobState::Paused)
                })
            })
        {
            return Err(
                "Select pending jobs. Active jobs keep their current priority and schedule.".into(),
            );
        }
        let previous = repo.data.jobs.clone();
        let minimum = repo
            .data
            .jobs
            .iter()
            .map(|j| j.manual_order)
            .min()
            .unwrap_or(0);
        for job in &mut repo.data.jobs {
            if ids.contains(&job.id) {
                job.priority = priority;
                job.schedule = schedule.clone();
                job.updated_at = queue::now();
                match order_action.as_deref() {
                    Some("next") => {
                        job.run_next = true;
                        job.manual_order = minimum.saturating_sub(1);
                    }
                    Some("up" | "down") => {}
                    Some(_) => unreachable!(),
                    None => {}
                }
            } else if order_action.as_deref() == Some("next") {
                job.run_next = false;
            }
        }
        if let Some(direction @ ("up" | "down")) = order_action.as_deref() {
            let mut ordered: Vec<_> = repo
                .data
                .jobs
                .iter()
                .enumerate()
                .filter(|(_, job)| {
                    matches!(
                        job.state,
                        queue::JobState::Pending | queue::JobState::Paused
                    ) && job.priority == priority
                        && serde_json::to_string(&job.schedule).ok()
                            == serde_json::to_string(&schedule).ok()
                })
                .map(|(index, _)| index)
                .collect();
            ordered.sort_by_key(|index| {
                (
                    repo.data.jobs[*index].manual_order,
                    repo.data.jobs[*index].created_at,
                )
            });
            for id in &ids {
                if let Some(index) = ordered.iter().position(|i| repo.data.jobs[*i].id == *id) {
                    let next = if direction == "up" {
                        index.checked_sub(1)
                    } else {
                        (index + 1 < ordered.len()).then_some(index + 1)
                    };
                    if let Some(next) = next {
                        ordered.swap(index, next);
                    }
                }
            }
            for (order, index) in ordered.into_iter().enumerate() {
                repo.data.jobs[index].manual_order = order as i64;
            }
        }
        if let Err(e) = repo.save() {
            repo.data.jobs = previous;
            return Err(e);
        }
    }
    state.runtime.notify.notify_one();
    snapshot(&state)
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
) -> Result<browser::Receipt, String> {
    if request.version != 1 {
        return Err("Unsupported bridge version.".into());
    }
    let state = app.state::<Arc<AppState>>();
    let request_id = format!("{}:{}", request.client_id, request.request_id);
    if request.intent != browser::BrowserIntent::Download {
        let text = if request.urls.is_empty() {
            request.url.clone()
        } else {
            request.urls.join("\n")
        };
        let draft = {
            let mut guard = state.repository.lock().unwrap();
            batch::create_for_request(
                guard.as_mut().ok_or("Open nabnab first.")?,
                &text,
                request.preset_id,
                &request_id,
                request.intent == browser::BrowserIntent::Save,
            )?
        };
        let _ = app.emit("browser-draft", &draft.id);
        return Ok(browser::Receipt {
            version: 1,
            request_id: request.request_id,
            accepted_job_ids: vec![],
            accepted_draft_id: Some(draft.id),
            intent: request.intent,
        });
    }
    let format = match request.profile.as_str() {
        "mp4" => Format::Mp4,
        "mp3" => Format::Mp3,
        "source" => Format::Source,
        "audio" => Format::AudioOriginal,
        _ => return Err("Unknown profile.".into()),
    };
    let jobs = {
        let mut guard = state.repository.lock().unwrap();
        let repo = guard.as_mut().ok_or("Open nabnab first.")?;
        if let Some(preset) = request.preset_id {
            let content_id = options::content_identity(&request.url)?;
            let context = options::Context {
                url: request.url.clone(),
                title: providers::identify(&request.url)?.name,
                content_id: content_id.rsplit(':').next().unwrap_or(&content_id).into(),
                ..Default::default()
            };
            let resolved = options::resolve(repo, &context, Some(&preset), None)?;
            queue::enqueue_prepared(
                repo,
                vec![options::PreparedDownload {
                    url: request.url,
                    title: context.title,
                    content_id,
                    options: resolved,
                    source_id: None,
                    group_id: None,
                }],
                &request_id,
            )?
        } else {
            queue::enqueue_download(
                repo,
                vec![request.url],
                vec![],
                format,
                None,
                None,
                None,
                &request_id,
            )?
        }
    };
    for job in &jobs {
        let _ = app.emit("job-update", job);
    }
    state.runtime.notify.notify_one();
    Ok(browser::Receipt {
        version: 1,
        request_id: request.request_id,
        accepted_job_ids: jobs.into_iter().map(|j| j.id).collect(),
        accepted_draft_id: None,
        intent: request.intent,
    })
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
                        "mp4", "mkv", "webm", "mov", "avi", "m4v", "mp3", "m4a", "m4b", "aac", "opus",
                        "ogg", "wav", "flac", "jpg", "jpeg", "png", "webp", "gif", "bmp",
                    ],
                )
                .add_filter("Documentos y texto", &["pdf", "epub", "txt", "md", "csv", "doc", "docx", "odt", "ppt", "pptx", "odp", "xls", "xlsx", "ods", "rtf"])
                .add_filter("Subtítulos", &["srt", "vtt"])
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
                .filter(|i| i.duration.is_none() && matches!(i.kind.as_str(), "video" | "audio"))
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
                            let chapters = metadata.details.chapters.clone();
                            let audio = metadata.details.audio.take();
                            metadata.details = item.details.clone();
                            if metadata.details.audio.is_none() {
                                metadata.details.audio = audio;
                            }
                            if metadata.details.chapters.is_empty() {
                                metadata.details.chapters = chapters;
                            }
                            metadata.content_hash = item.content_hash.clone();
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
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .collect();
    if lines.len() > 10_000 {
        return Err("The URL list contains more than 10000 entries. Split the file to keep drafts responsive.".into());
    }
    Ok(lines.into_iter().map(str::to_owned).collect())
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
        if !folder && !library::can_open(item) { return Err("This file type cannot be opened from nabnab. Use Show folder to review it.".into()); }
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
        || batch::metadata_busy()
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
    if std::env::args().skip(1).eq(["--automation-worker"]) {
        if let Err(error) = automation::run_worker(tauri::generate_context!()) {
            eprintln!("{error}");
            std::process::exit(3);
        }
        return;
    }
    let state = Arc::new(AppState::default());
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .register_uri_scheme_protocol("nabmedia", |context, request| {
            local_media::serve(context.app_handle(), request)
        })
        .invoke_handler(tauri::generate_handler![
            web_archive::archive_document,web_archive::archive_add_annotation,web_archive::archive_remove_annotation,web_archive::queue_article_export,
            capture_operations::screen_capture_status,capture_operations::screen_capture_sources,capture_operations::screen_capture_preview,capture_operations::cancel_screen_capture_preview,capture_operations::screen_capture_start,capture_operations::screen_capture_stop,capture_operations::screen_capture_save_replay,capture_operations::screen_capture_screenshot,capture_operations::screen_capture_recoverable,capture_operations::queue_screen_capture_recovery,
            voice_operations::voice_status,voice_operations::voice_prepare,voice_operations::voice_sample,voice_operations::voice_plan_status,voice_operations::voice_segment_audio,voice_operations::queue_voice,voice_operations::voice_pause_job,voice_operations::cancel_voice_request,
            file_operations::image_assets_inspect,file_operations::image_assets_preview,file_operations::queue_image_assets,file_operations::cancel_image_assets_preview,
            file_operations::file_analysis,file_operations::queue_file_analysis,file_operations::file_rename_preview,file_operations::queue_file_rename,file_operations::file_rename_history,file_operations::queue_file_undo,file_operations::file_organize_review,file_operations::cancel_file_operation,
            file_operations::portable_preview_export,file_operations::queue_portable_export,file_operations::portable_choose_import,file_operations::portable_preview_import,file_operations::queue_portable_import,file_operations::cancel_portable_preview,
            document_operations::document_edit_info,document_operations::document_edit_preview,document_operations::cancel_document_edit_preview,document_operations::queue_document_edit,
            document_operations::ocr_status,document_operations::ocr_models,document_operations::install_ocr_model,document_operations::queue_ocr,
            document_operations::ocr_get,document_operations::ocr_preview,document_operations::ocr_correct_page,document_operations::ocr_remove_generated,document_operations::ocr_search,document_operations::ocr_save_text,
            file_sources::public_source_inspect, file_sources::cancel_file_source_inspection,
            file_sources::queue_public_files, file_sources::cloud_status, file_sources::cloud_configure,
            file_sources::cloud_connect, file_sources::cloud_revoke, file_sources::cloud_browse, file_sources::cloud_preview_folder, file_sources::queue_cloud_files,
            personal_servers::personal_servers_status, personal_servers::personal_server_webdav, personal_servers::personal_server_choose_key,
            personal_servers::personal_server_nas, personal_servers::personal_server_probe_sftp, personal_servers::personal_server_trust_sftp,
            personal_servers::personal_server_remove, personal_servers::personal_server_browse, personal_servers::personal_server_index,
            personal_servers::personal_server_copy,
            transcription::remove_transcription_model, transcription::transcription_models, transcription::install_transcription_model,
            courses::members::save_course_token, courses::members::inspect_member_course, courses::inspect_course, courses::queue_course_assets, courses::list_courses, courses::import_local_course, courses::course_learned, courses::save_course_note,
            courses::browser::open_course_site, courses::browser::import_course_materials,
            queue_torrent, torrents::torrent_status, torrents::set_torrent_enabled, torrents::open_torrent, torrents::inspect_torrent_magnet, torrents::cancel_torrent_inspection,
            queue::modules::queue_advanced_file, queue::modules::queue_palette, queue::modules::queue_audio_preview, palettes::palette_preview, advanced_files::inspect_advanced_file,
            queue::modules::queue_source_export, queue::modules::queue_caption_edit, queue::modules::queue_media_edit, queue::modules::queue_document, queue::modules::queue_gallery_export, queue::modules::queue_telegram, queue::modules::queue_live, live::stop_recording,
            media_edits::media_edit_capabilities, documents::document_info, documents::document_page, documents::document_search, federated::inspect_federated, federated_auth::set_federated_token, federated_auth::federated_auth_status,
            telegram::telegram_status, telegram::telegram_connect, telegram::telegram_authorization, telegram::telegram_authorize, telegram::telegram_disconnect, telegram::telegram_chats, telegram::telegram_media, telegram::telegram_message_link,
            components::optional_components,
            components::install_component,
            components::enable_component, components::remove_component,
            gallery_auth::set_gallery_token, gallery_auth::gallery_auth_status, galleries::gallery_preview, galleries::inspect_gallery,
            galleries::queue_gallery,
            automation::automation_status,automation::automation_enable,automation::automation_choose_root,automation::automation_remove_root,automation::automation_save_client,automation::automation_revoke_client,automation::automation_approve_plan,
            startup,
            get_snapshot,
            inspect_media,
            cancel_job,
            queue_download,
            prepare_download,
            queue_prepared,
            queue_tool,
            job_action,
            configure_jobs,
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
            bridge_revoke,
            options::save_preset,
            options::delete_preset,
            options::save_rules,
            options::test_rule,
            batch::batch_create,
            batch::batch_save,
            batch::batch_inspect,
            batch::batch_cancel,
            batch::batch_enqueue,
            batch::batch_remove,
            batch::batch_chapters,
            scheduling::local_timezone,
            sources::add_source,
            sources::import_podcast_opml,
            sources::export_podcast_opml,
            sources::export_podcast_m3u,
            sources::check_source,
            sources::save_source,
            sources::source_item_action,
            sources::remove_source,
            library_features::library_search,
            library_features::library_playback,
            audio_library::audio_tags_edit, audio_library::audio_tags_read,
            audio_library::tag_copy::audio_tags_queue_copies,
            audio_library::audio_playlist_save, audio_library::audio_playlist_remove,
            audio_library::playlist_import::audio_playlist_choose, audio_library::playlist_import::audio_playlist_map,
            audio_library::audio_lyrics, audio_library::audio_lyrics_choose,
            audio_library::audio_cover_choose,
            library_features::library_favorite,
            library_features::library_bookmark,
            library_features::smart_collection_save,
            library_features::smart_collection_remove,
            library_features::subtitle_import,
            library_features::subtitle_export,
            library_features::subtitle_rebuild
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
                telegram::shutdown();
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
                capture_operations::shutdown(&state);
                state.automation_pipe.lock().unwrap().take();
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
