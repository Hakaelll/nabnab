use crate::{
    file_organization as files, image_assets as images, library::LibraryItem,
    portable_collections as portable, queue, workspaces::Workspace, AppState,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

#[derive(Default)]
pub struct PreviewState {
    images: Mutex<HashMap<String, ImageApproval>>,
}
struct ImageApproval {
    ids: Vec<String>,
    options: String,
    hashes: Vec<String>,
    expires: u64,
}

pub(crate) fn context(state: &AppState) -> Result<(PathBuf, Vec<LibraryItem>), String> {
    let guard = state.repository.lock().unwrap();
    let repo = guard.as_ref().ok_or("Open nabnab first.")?;
    Ok((repo.root().to_owned(), repo.data.library.clone()))
}
pub(crate) fn selected(
    library: &[LibraryItem],
    ids: &[String],
    limit: usize,
) -> Result<Vec<LibraryItem>, String> {
    if ids.is_empty() || ids.len() > limit || ids.iter().collect::<HashSet<_>>().len() != ids.len()
    {
        return Err(format!(
            "Choose 1–{limit} different library files in the intended order."
        ));
    }
    ids.iter()
        .map(|id| {
            let item = library
                .iter()
                .find(|i| &i.id == id)
                .cloned()
                .ok_or("A selected file is no longer in the library.")?;
            crate::library::authorized_path(&item.path)?;
            Ok(item)
        })
        .collect()
}
async fn fingerprints(
    items: &[LibraryItem],
    cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<Vec<String>, String> {
    let mut hashes = Vec::with_capacity(items.len());
    for item in items {
        hashes.push(queue::digest(item.path.clone(), cancel.clone()).await?);
    }
    Ok(hashes)
}
#[tauri::command]
pub async fn image_assets_inspect(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    stitch: bool,
    request_id: u64,
) -> Result<images::Inspection, String> {
    let (root, library) = context(&state)?;
    let inputs = selected(&library, &ids, 256)?;
    let engines = crate::dependencies::engine_root(&app)?;
    let workspace = Workspace::new(&root)?;
    let cancel = crate::file_sources::begin(&state, request_id)?;
    let result = images::inspect(
        &engines,
        &inputs,
        &workspace.path,
        stitch,
        cancel,
        request_id,
    )
    .await;
    crate::file_sources::finish(&state, request_id);
    result
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePreview {
    #[serde(flatten)]
    preview: images::Preview,
    preview_id: String,
}
#[tauri::command]
pub async fn image_assets_preview(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    options: images::Options,
    request_id: u64,
) -> Result<ImagePreview, String> {
    images::validate_options(&options, ids.len())?;
    let (root, library) = context(&state)?;
    let inputs = selected(&library, &ids, 256)?;
    let engines = crate::dependencies::engine_root(&app)?;
    let workspace = Workspace::new(&root)?;
    let cancel = crate::file_sources::begin(&state, request_id)?;
    let result = async {
        let hashes = fingerprints(&inputs, cancel.clone()).await?;
        let preview = images::preview(
            &engines,
            &inputs,
            &workspace.path,
            &options,
            cancel.clone(),
            request_id,
        )
        .await?;
        if fingerprints(&inputs, cancel.clone()).await? != hashes {
            return Err(
                "An image changed while its preview was generated. Review it again.".into(),
            );
        }
        if *cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        let options = serde_json::to_string(&options).map_err(|e| e.to_string())?;
        let preview_id = format!(
            "{:x}",
            Sha256::digest(
                format!("{ids:?}:{hashes:?}:{options}:{}", workspace.path.display()).as_bytes()
            )
        );
        let mut approvals = state.file_previews.images.lock().unwrap();
        approvals.retain(|_, a| a.expires > queue::now());
        if approvals.len() >= 32 {
            approvals.clear();
        }
        approvals.insert(
            preview_id.clone(),
            ImageApproval {
                ids,
                options,
                hashes,
                expires: queue::now() + 600_000,
            },
        );
        Ok(ImagePreview {
            preview,
            preview_id,
        })
    }
    .await;
    crate::file_sources::finish(&state, request_id);
    result
}
#[tauri::command]
pub async fn queue_image_assets(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    options: images::Options,
    preview_id: String,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    images::validate_options(&options, ids.len())?;
    crate::dependencies::engine_root(&app)?;
    let (_, library) = context(&state)?;
    let inputs = selected(&library, &ids, 256)?;
    let (_sender, cancel) = tokio::sync::watch::channel(false);
    let hashes = fingerprints(&inputs, cancel).await?;
    let serialized = serde_json::to_string(&options).map_err(|e| e.to_string())?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let previous: Vec<_> = repo
        .data
        .jobs
        .iter()
        .filter(|j| j.client_request == client_request)
        .cloned()
        .collect();
    if !previous.is_empty() {
        return Ok(previous);
    }
    let current = selected(&repo.data.library, &ids, 256)?;
    if current.iter().zip(&inputs).any(|(a, b)| a.path != b.path) {
        return Err("A selected image moved. Review it again.".into());
    }
    let mut approvals = state.file_previews.images.lock().unwrap();
    let approval = approvals
        .get(&preview_id)
        .ok_or("Generate an image preview before adding this operation to the queue.")?;
    if approval.ids != ids
        || approval.options != serialized
        || approval.hashes != hashes
        || approval.expires <= queue::now()
    {
        return Err(
            "The image preview expired or no longer matches these files and options.".into(),
        );
    }
    let title = format!("Images · {}", options.name());
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::ImageAssets {
                library_ids: ids,
                options,
                input_hashes: hashes,
            },
            title,
        )],
        &client_request,
    )?;
    approvals.remove(&preview_id);
    state.runtime.notify.notify_one();
    Ok(jobs)
}
#[tauri::command]
pub fn cancel_image_assets_preview(state: State<Arc<AppState>>, request_id: u64) {
    crate::file_sources::cancel_file_source_inspection(state, request_id);
}

#[tauri::command]
pub async fn file_analysis(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    options: files::AnalyzeOptions,
    request_id: u64,
) -> Result<files::Analysis, String> {
    let (root, library) = context(&state)?;
    selected(&library, &ids, 200)?;
    let ffmpeg = if matches!(options, files::AnalyzeOptions::Similar { .. }) {
        crate::dependencies::engine_root(&app)?.join("ffmpeg.exe")
    } else {
        PathBuf::new()
    };
    let workspace = Workspace::new(&root)?;
    let cancel = crate::file_sources::begin(&state, request_id)?;
    let result = files::analyze(
        &ffmpeg,
        &workspace.path,
        &library,
        &ids,
        &options,
        cancel,
        request_id,
        |_| {},
    )
    .await;
    crate::file_sources::finish(&state, request_id);
    result
}
#[tauri::command]
pub fn queue_file_analysis(
    state: State<Arc<AppState>>,
    ids: Vec<String>,
    options: files::AnalyzeOptions,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    selected(&repo.data.library, &ids, 200)?;
    if matches!(options, files::AnalyzeOptions::Similar { max_distance } if max_distance > 16) {
        return Err("Choose a similarity distance of 0–16 bits.".into());
    }
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::FileAnalysis {
                library_ids: ids,
                options,
            },
            "File analysis".into(),
        )],
        &client_request,
    )?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}
#[tauri::command]
pub async fn file_rename_preview(
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    options: files::RenameOptions,
    request_id: u64,
) -> Result<files::RenamePlan, String> {
    let (root, library) = context(&state)?;
    selected(&library, &ids, 200)?;
    let cancel = crate::file_sources::begin(&state, request_id)?;
    let result = tokio::task::spawn_blocking(move || {
        files::plan_rename(&root, &library, &ids, &options, cancel)
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|v| v);
    crate::file_sources::finish(&state, request_id);
    result
}
#[tauri::command]
pub fn queue_file_rename(
    state: State<Arc<AppState>>,
    plan_id: String,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    queue::validate_request_id(&client_request)?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let previous: Vec<_> = repo
        .data
        .jobs
        .iter()
        .filter(|j| j.client_request == client_request)
        .cloned()
        .collect();
    if !previous.is_empty() {
        return Ok(previous);
    }
    files::approve_rename(repo, &plan_id)?;
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(queue::Source::FileRename { plan_id }, "Rename files".into())],
        &client_request,
    )?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}
#[tauri::command]
pub fn file_rename_history(
    state: State<Arc<AppState>>,
) -> Result<Vec<files::RenameJournal>, String> {
    let guard = state.repository.lock().unwrap();
    files::rename_history(guard.as_ref().ok_or("Open nabnab first.")?)
}
#[tauri::command]
pub fn queue_file_undo(
    state: State<Arc<AppState>>,
    operation_id: String,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    if !files::rename_history(repo)?
        .iter()
        .any(|j| j.id == operation_id)
    {
        return Err("Choose a recorded rename operation.".into());
    }
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::FileUndo { operation_id },
            "Undo rename".into(),
        )],
        &client_request,
    )?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}
#[tauri::command]
pub fn file_organize_review(
    state: State<Arc<AppState>>,
    ids: Vec<String>,
    collection: String,
) -> Result<(), String> {
    let mut guard = state.repository.lock().unwrap();
    files::organize(
        guard.as_mut().ok_or("Open nabnab first.")?,
        &ids,
        &collection,
    )
}
#[tauri::command]
pub fn cancel_file_operation(state: State<Arc<AppState>>, request_id: u64) {
    crate::file_sources::cancel_file_source_inspection(state, request_id);
}

#[tauri::command]
pub async fn portable_preview_export(
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    options: portable::ExportOptions,
    request_id: u64,
) -> Result<portable::ExportPreview, String> {
    let (root, library) = context(&state)?;
    let inputs = selected(&library, &ids, 2000)?;
    let cancel = crate::file_sources::begin(&state, request_id)?;
    let result = portable::preview_export(&root, &inputs, &options, cancel).await;
    crate::file_sources::finish(&state, request_id);
    result
}
#[tauri::command]
pub fn queue_portable_export(
    state: State<Arc<AppState>>,
    ids: Vec<String>,
    options: portable::ExportOptions,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    selected(&repo.data.library, &ids, 2000)?;
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::PortableExport {
                library_ids: ids,
                options,
            },
            "Export portable collection".into(),
        )],
        &client_request,
    )?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}
#[tauri::command]
pub async fn portable_choose_import(
    app: AppHandle,
    folder: bool,
) -> Result<Option<String>, String> {
    let selected = tauri::async_runtime::spawn_blocking(move || {
        if folder {
            app.dialog().file().blocking_pick_folder()
        } else {
            app.dialog()
                .file()
                .add_filter("nabnab portable collection", &["zip"])
                .blocking_pick_file()
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    let path = crate::library::authorized_path(
        &selected
            .into_path()
            .map_err(|_| "Choose a local collection.")?,
    )?;
    Ok(Some(path.to_string_lossy().into_owned()))
}
#[tauri::command]
pub async fn portable_preview_import(
    state: State<'_, Arc<AppState>>,
    path: String,
    request_id: u64,
) -> Result<portable::ImportPreview, String> {
    let path = crate::library::authorized_path(&PathBuf::from(path))?;
    let (root, library) = context(&state)?;
    let target = {
        let guard = state.repository.lock().unwrap();
        guard
            .as_ref()
            .ok_or("Open nabnab first.")?
            .data
            .settings
            .folder
            .clone()
    };
    let cancel = crate::file_sources::begin(&state, request_id)?;
    let result = portable::preview_import(&root, &path, &target, &library, cancel).await;
    crate::file_sources::finish(&state, request_id);
    result
}
#[tauri::command]
pub fn queue_portable_import(
    state: State<Arc<AppState>>,
    plan_id: String,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    portable::validate_import_plan(repo.root(), &plan_id, &repo.data.settings.folder)?;
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::PortableImport { plan_id },
            "Import portable collection".into(),
        )],
        &client_request,
    )?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}
#[tauri::command]
pub fn cancel_portable_preview(state: State<Arc<AppState>>, request_id: u64) {
    crate::file_sources::cancel_file_source_inspection(state, request_id);
}
