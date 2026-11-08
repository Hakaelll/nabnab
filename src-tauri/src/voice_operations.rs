use crate::{queue, voice, voice_sources, AppState};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::watch;

pub(crate) fn require_enabled(app: &AppHandle) -> Result<(), String> {
    if !crate::components::root(app)?.join("voice.enabled").is_file() {
        return Err("Enable Local voice in Settings → Components before preparing or generating narration.".into());
    }
    Ok(())
}
#[derive(Serialize)]
pub struct Status { #[serde(flatten)] status: voice::VoiceStatus, enabled: bool }
#[tauri::command]
pub async fn voice_status(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<Status, String> {
    let (root, _) = crate::file_operations::context(&state)?;
    let (_sender, cancel) = watch::channel(false);
    let status = voice::status(&root, cancel).await?;
    Ok(Status { status, enabled: crate::components::root(&app)?.join("voice.enabled").is_file() })
}
#[tauri::command]
pub async fn voice_prepare(app: AppHandle, state: State<'_, Arc<AppState>>, source: voice_sources::Input, settings: voice::VoiceSettings, request_id: u64) -> Result<voice::VoicePlan, String> {
    require_enabled(&app)?;
    voice::validate_settings(&settings)?;
    let (root, library) = crate::file_operations::context(&state)?;
    let cancel = crate::file_sources::begin(&state, request_id)?;
    let result = async {
        let source = voice_sources::resolve(&app, &root, &library, source, cancel.clone(), request_id).await?;
        voice::prepare(&root, &source.text, &source.source_version, voice::Origin { title: source.title, library_id: source.library_id, kind: source.kind }, settings, cancel).await
    }.await;
    crate::file_sources::finish(&state, request_id);
    result
}
#[tauri::command]
pub async fn voice_sample(app: AppHandle, state: State<'_, Arc<AppState>>, plan_id: String, request_id: u64) -> Result<voice::Sample, String> {
    require_enabled(&app)?;
    let (root, _) = crate::file_operations::context(&state)?;
    let cancel = crate::file_sources::begin(&state, request_id)?;
    let result = voice::sample(&root, &plan_id, cancel, request_id, |_| {}).await;
    crate::file_sources::finish(&state, request_id);
    result
}
#[tauri::command]
pub async fn voice_plan_status(state: State<'_, Arc<AppState>>, plan_id: String) -> Result<voice::PlanStatus, String> {
    let (root, _) = crate::file_operations::context(&state)?;
    tokio::task::spawn_blocking(move || voice::plan_status(&root, &plan_id)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn voice_segment_audio(state: State<'_, Arc<AppState>>, plan_id: String, index: usize) -> Result<Vec<u8>, String> {
    let (root, _) = crate::file_operations::context(&state)?;
    tokio::task::spawn_blocking(move || voice::segment_audio(&root, &plan_id, index)).await.map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn queue_voice(app: AppHandle, state: State<'_, Arc<AppState>>, plan_id: String, client_request: String) -> Result<Vec<queue::Job>, String> {
    require_enabled(&app)?;
    let (root, _) = crate::file_operations::context(&state)?;
    let selected = plan_id.clone();
    let plan = tokio::task::spawn_blocking(move || voice::read_plan(&root, &selected)).await.map_err(|e| e.to_string())??;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let title = format!("{} · {}", if repo.data.settings.language == "es" { "Voz" } else { "Voice" }, plan.origin.title);
    let jobs = queue::enqueue_tasks(repo, vec![(queue::Source::Voice { plan_id }, title)], &client_request)?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}
#[tauri::command]
pub fn voice_pause_job(state: State<Arc<AppState>>, id: String) -> Result<(), String> {
    {
        let guard = state.repository.lock().unwrap();
        let repo = guard.as_ref().ok_or("Open nabnab first.")?;
        if !repo.data.jobs.iter().any(|job| job.id == id && matches!(job.source, queue::Source::Voice {..})) {
            return Err("Choose a narration job to pause.".into());
        }
    }
    queue::action(&state, &[id], "pause")
}
#[tauri::command]
pub fn cancel_voice_request(state: State<Arc<AppState>>, request_id: u64) {
    crate::file_sources::cancel_file_source_inspection(state, request_id);
}
