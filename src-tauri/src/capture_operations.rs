use crate::{capture_shortcut::Shortcut, queue, screen_capture as capture, AppState};
use serde::Serialize;
use std::{path::PathBuf, sync::{Arc, Mutex}, time::Duration};
use tauri::{AppHandle, Emitter, State};

#[derive(Default)]
pub struct HostState {
    controller: Mutex<Option<capture::Controller>>,
    shortcut: Mutex<Option<(String, Shortcut)>>,
    shortcut_reason: Mutex<Option<(String, String)>>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all="camelCase")]
pub struct Status {
    #[serde(flatten)] state: capture::State,
    shortcut_label: Option<String>,
    shortcut_reason: Option<String>,
}
pub(crate) fn active(state: &AppState) -> bool {
    state.capture.controller.lock().unwrap().as_ref().is_some_and(|controller| matches!(controller.status().phase.as_str(), "starting"|"recording"|"replay"|"stopping"))
}
fn require_enabled(app: &AppHandle) -> Result<(), String> {
    if !crate::components::root(app)?.join("capture.enabled").is_file() {
        return Err("Enable Screen capture in Settings → Components before selecting or capturing a source.".into());
    }
    Ok(())
}
fn controller(app: &AppHandle, state: &AppState) -> Result<capture::Controller, String> {
    let (root, _) = crate::file_operations::context(state)?;
    let mut stored = state.capture.controller.lock().unwrap();
    if stored.is_none() { *stored = Some(capture::Controller::new(root, crate::dependencies::engine_root(app)?)?); }
    stored.as_ref().cloned().ok_or_else(|| "The capture controller is unavailable.".into())
}
fn destination(state: &AppState) -> Result<PathBuf, String> {
    let guard = state.repository.lock().unwrap();
    Ok(guard.as_ref().ok_or("Open nabnab first.")?.data.settings.folder.clone())
}
fn status(state: &AppState) -> Status {
    let current = state.capture.controller.lock().unwrap().as_ref().map(|controller| controller.status()).unwrap_or_default();
    let shortcut_label=state.capture.shortcut.lock().unwrap().as_ref().filter(|(id,_)|current.session_id.as_ref()==Some(id)).map(|(_,shortcut)|shortcut.label.clone());
    let shortcut_reason=state.capture.shortcut_reason.lock().unwrap().as_ref().filter(|(id,_)|current.session_id.as_ref()==Some(id)).map(|(_,reason)|reason.clone());
    Status { state: current, shortcut_label, shortcut_reason }
}
fn stop_shortcut(state: &AppState, session: Option<&str>) {
    let shortcut = {let mut stored=state.capture.shortcut.lock().unwrap();if session.is_none_or(|id|stored.as_ref().is_some_and(|(current,_)|current==id)){stored.take()}else{None}};
    drop(shortcut);
    let mut reason=state.capture.shortcut_reason.lock().unwrap();if session.is_none_or(|id|reason.as_ref().is_some_and(|(current,_)|current==id)){*reason=None;}
}
pub(crate) fn enqueue_ready(app: &AppHandle, state: &Arc<AppState>, ready: &capture::ReadyCapture) -> Result<Vec<queue::Job>, String> {
    if state.runtime.updating.load(std::sync::atomic::Ordering::SeqCst) { return Err("nabnab is closing or updating. The capture remains available for recovery.".into()); }
    let (root, _) = crate::file_operations::context(state)?;
    let previous = {
        let guard = state.repository.lock().unwrap();
        guard.as_ref().ok_or("Open nabnab first.")?.data.jobs.iter().filter(|job| matches!(&job.source, queue::Source::Capture {capture_id} if capture_id == &ready.id)).cloned().collect::<Vec<_>>()
    };
    if !previous.is_empty() {
        if previous.iter().any(|job|matches!(job.state,queue::JobState::Error|queue::JobState::Cancelled)) {let journal=capture::readjournal(&root,&ready.id)?;if !matches!(journal.phase.as_str(),"ready"|"interrupted"){return Err("The capture cache is no longer available for retry.".into());}}
        for job in &previous { if matches!(job.state, queue::JobState::Error|queue::JobState::Cancelled) { queue::action(state, std::slice::from_ref(&job.id), "retry")?; } }
        let _ = app.emit("queue-changed", ());
        let guard = state.repository.lock().unwrap();
        return Ok(guard.as_ref().ok_or("Open nabnab first.")?.data.jobs.iter().filter(|job| previous.iter().any(|old| old.id == job.id)).cloned().collect());
    }
    let journal = capture::readjournal(&root, &ready.id)?;
    if !matches!(journal.phase.as_str(), "ready"|"interrupted") { return Err("This capture has already been published or is not ready for finalization.".into()); }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let label = if journal.interrupted { if repo.data.settings.language == "es" { "Recuperar captura parcial" } else { "Recover interrupted capture" } } else if repo.data.settings.language == "es" { "Captura" } else { "Capture" };
    let jobs = queue::enqueue_tasks_at(repo, vec![(queue::Source::Capture {capture_id: ready.id.clone()}, format!("{label} · {}", journal.source_label))], &format!("capture:{}", ready.id), journal.destination)?;
    drop(guard);
    state.runtime.notify.notify_one();
    let _ = app.emit("queue-changed", ());
    Ok(jobs)
}
fn start_shortcut(app: AppHandle, state: Arc<AppState>, controller: capture::Controller, session: String) {
    stop_shortcut(&state,None);
    let handle = app.clone();
    let shared = state.clone();
    let callback_session=session.clone();
    match Shortcut::start(move || {
        let handle = handle.clone();
        let shared = shared.clone();
        let controller = controller.clone();
        let expected=callback_session.clone();
        tauri::async_runtime::spawn(async move {
            let current = controller.status();
            if current.phase != "replay" || current.session_id.as_deref()!=Some(&expected) { return; }
            let Some(id) = current.session_id else { return; };
            match controller.save_replay(&id).await.and_then(|ready| enqueue_ready(&handle, &shared, &ready).map(|_| ())) {
                Ok(()) => { let _ = handle.emit("capture-saved", ()); }
                Err(error) => { let _ = handle.emit("capture-error", error); }
            }
        });
    }) {
        Ok(shortcut) => *state.capture.shortcut.lock().unwrap() = Some((session,shortcut)),
        Err(error) => *state.capture.shortcut_reason.lock().unwrap() = Some((session,error)),
    }
}
fn monitor(app: AppHandle, state: Arc<AppState>, controller: capture::Controller, session: String) {
    tauri::async_runtime::spawn(async move {
        loop {
            let current = controller.status();
            let _ = app.emit("capture-update", status(&state));
            if current.session_id.as_deref() != Some(&session) { stop_shortcut(&state,Some(&session)); break; }
            if !matches!(current.phase.as_str(), "starting"|"recording"|"replay"|"stopping") {
                stop_shortcut(&state,Some(&session));
                if let Some(ready) = current.ready_capture.filter(|ready| !ready.interrupted) {
                    if let Err(error) = enqueue_ready(&app, &state, &ready) { let _ = app.emit("capture-error", error); }
                }
                let _ = app.emit("capture-update", status(&state));
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
}
#[tauri::command]
pub fn screen_capture_status(state: State<Arc<AppState>>) -> Status { status(&state) }
#[tauri::command]
pub async fn screen_capture_sources(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<capture::Sources, String> {
    if let Err(reason) = require_enabled(&app) { return Ok(capture::Sources { available: false, reason: Some(reason), sources: vec![], audio: vec![] }); }
    controller(&app, &state)?.sources().await
}
#[tauri::command]
pub async fn screen_capture_preview(app: AppHandle, state: State<'_, Arc<AppState>>, source_id: String, region: Option<capture::Region>, request_id: u64) -> Result<capture::Preview, String> {
    require_enabled(&app)?;
    let controller = controller(&app, &state)?;
    let cancel = crate::file_sources::begin(&state, request_id)?;
    let result = controller.preview(&source_id, region, cancel).await;
    crate::file_sources::finish(&state, request_id);
    result
}
#[tauri::command]
pub fn cancel_screen_capture_preview(state: State<Arc<AppState>>, request_id: u64) { crate::file_sources::cancel_file_source_inspection(state, request_id); }
#[tauri::command]
pub async fn screen_capture_start(app: AppHandle, state: State<'_, Arc<AppState>>, preview_id: String, options: capture::Options) -> Result<Status, String> {
    require_enabled(&app)?;
    let controller = controller(&app, &state)?;
    let replay = options.mode == capture::Mode::Replay;
    let current = controller.start(&preview_id, options, destination(&state)?).await?;
    if replay { if let Some(session)=current.session_id.as_ref(){start_shortcut(app.clone(), state.inner().clone(), controller.clone(),session.clone());} }
    if let Some(session) = current.session_id { monitor(app.clone(), state.inner().clone(), controller, session); }
    let current = status(&state);
    let _ = app.emit("capture-update", &current);
    Ok(current)
}
#[tauri::command]
pub async fn screen_capture_stop(app: AppHandle, state: State<'_, Arc<AppState>>, session_id: String) -> Result<Status, String> {
    let current = controller(&app, &state)?.stop(&session_id).await?;
    stop_shortcut(&state,Some(&session_id));
    if let Some(ready) = current.ready_capture.filter(|ready| !ready.interrupted) { enqueue_ready(&app, state.inner(), &ready)?; }
    let result = status(&state);
    let _ = app.emit("capture-update", &result);
    Ok(result)
}
#[tauri::command]
pub async fn screen_capture_save_replay(app: AppHandle, state: State<'_, Arc<AppState>>, session_id: String) -> Result<capture::ReadyCapture, String> {
    let ready = controller(&app, &state)?.save_replay(&session_id).await?;
    enqueue_ready(&app, state.inner(), &ready)?;
    Ok(ready)
}
#[tauri::command]
pub async fn screen_capture_screenshot(app: AppHandle, state: State<'_, Arc<AppState>>, preview_id: String) -> Result<capture::ReadyCapture, String> {
    require_enabled(&app)?;
    let ready = controller(&app, &state)?.screenshot(&preview_id, destination(&state)?).await?;
    enqueue_ready(&app, state.inner(), &ready)?;
    Ok(ready)
}
#[tauri::command]
pub fn screen_capture_recoverable(state: State<Arc<AppState>>) -> Result<Vec<capture::ReadyCapture>, String> {
    let (root, _) = crate::file_operations::context(&state)?;
    let candidates = capture::recover(&root)?;
    let guard = state.repository.lock().unwrap();
    let repo = guard.as_ref().ok_or("Open nabnab first.")?;
    Ok(candidates.into_iter().filter(|ready| !repo.data.jobs.iter().any(|job| matches!(&job.source, queue::Source::Capture {capture_id} if capture_id == &ready.id) && !matches!(job.state, queue::JobState::Error|queue::JobState::Cancelled))).collect())
}
#[tauri::command]
pub fn queue_screen_capture_recovery(app: AppHandle, state: State<Arc<AppState>>, capture_id: String, client_request: String) -> Result<Vec<queue::Job>, String> {
    queue::validate_request_id(&client_request)?;
    let (root, _) = crate::file_operations::context(&state)?;
    let ready = capture::recover(&root)?.into_iter().find(|ready| ready.id == capture_id).ok_or("Choose a stopped recoverable capture.")?;
    enqueue_ready(&app, state.inner(), &ready)
}
pub(crate) fn shutdown(state: &AppState) {
    stop_shortcut(state,None);
    let controller = state.capture.controller.lock().unwrap().clone();
    if let Some(controller) = controller { tauri::async_runtime::block_on(controller.shutdown()); }
}
