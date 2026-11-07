use crate::{
    automation_credentials as credentials,
    automation_wire::{ApiError, Envelope, Response},
    library, queue,
    store::Repository,
    tools::ToolOptions,
    AppState,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::watch;

pub const CAPABILITIES: [&str; 8] = [
    "media.inspect",
    "jobs.plan",
    "jobs.enqueue",
    "jobs.status",
    "jobs.cancel",
    "library.search",
    "library.get_item",
    "tools.run",
];
const MIN_BYTES: u64 = 1024 * 1024;
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CLIENTS: usize = 32;
const MAX_PLANS: usize = 256;

mod model;
pub use model::*;
mod requests;
pub use requests::request;

fn budget_valid(bytes: u64, seconds: u32) -> bool {
    (MIN_BYTES..=MAX_BYTES).contains(&bytes) && (5..=600).contains(&seconds)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn plan_digest(plan: &Plan) -> Result<String, String> {
    Ok(hash(&serde_json::to_vec(&json!({"id":plan.id,"client":plan.client_id,"operation":plan.operation,"source":plan.source,"inputHash":plan.input_hash,"rootId":plan.output_root_id,"path":plan.output_path,"collection":plan.collection,"budget":plan.budget,"expires":plan.expires_at})).map_err(|e|e.to_string())?))
}
pub fn validate(data: &Data) -> Result<(), String> {
    if data.clients.len() > MAX_CLIENTS || data.roots.len() > 32 || data.plans.len() > MAX_PLANS {
        return Err("Automation records exceed their limits.".into());
    }
    let mut ids = std::collections::HashSet::new();
    for client in &data.clients {
        if !credentials::valid_id(&client.id)
            || !ids.insert(client.id.clone())
            || client.name.is_empty()
            || client.name.chars().count() > 80
            || !budget_valid(client.grant.max_bytes, client.grant.max_seconds)
            || client.grant.capabilities.len() > 8
            || client
                .grant
                .capabilities
                .iter()
                .any(|c| !CAPABILITIES.contains(&c.as_str()))
            || client.grant.collections.len() > 200
            || client.grant.output_root_ids.len() > 32
        {
            return Err("Invalid automation client or permissions.".into());
        }
    }
    ids.clear();
    for root in &data.roots {
        if !credentials::valid_id(&root.id)
            || !ids.insert(root.id.clone())
            || !root.path.is_absolute()
            || root.label.len() > 1024
        {
            return Err("Invalid automation destination.".into());
        }
    }
    ids.clear();
    for plan in &data.plans {
        if !credentials::valid_id(&plan.id)
            || !ids.insert(plan.id.clone())
            || !credentials::valid_id(&plan.client_id)
            || !budget_valid(plan.budget.max_bytes, plan.budget.max_seconds)
            || plan.digest != plan_digest(plan)?
            || plan.expires_at <= plan.created_at
            || plan.expires_at - plan.created_at > 600_000
        {
            return Err("Invalid or modified automation approval.".into());
        }
        if !matches!(
            plan.source,
            queue::Source::Download { .. } | queue::Source::Transform { .. }
        ) {
            return Err("This operation is not exposed to automation.".into());
        }
        if let queue::Source::Download { url, url_ref, .. } = &plan.source {
            if queue::public_origin(url)? != *url
                || !url_ref
                    .as_deref()
                    .is_some_and(crate::sessions::valid_url_reference)
            {
                return Err(
                    "Automation download plans require a protected source reference.".into(),
                );
            }
        }
    }
    Ok(())
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    enabled: bool,
    clients: Vec<Client>,
    roots: Vec<OutputRoot>,
    plans: Vec<Value>,
    collections: Vec<String>,
    catalog: Value,
}
fn public_plan(plan: &Plan) -> Value {
    json!({"id":plan.id,"clientId":plan.client_id,"clientName":plan.client_name,"createdAt":plan.created_at,"expiresAt":plan.expires_at,"state":plan.state,"operation":plan.operation,"title":plan.title,"outputRootId":plan.output_root_id,"outputLabel":plan.output_label,"outputPath":plan.output_path,"collection":plan.collection,"budget":plan.budget,"digest":plan.digest,"jobId":plan.job_id})
}
fn status(repo: &Repository) -> Status {
    Status {
        enabled: repo.data.automation.enabled,
        clients: repo.data.automation.clients.clone(),
        roots: repo.data.automation.roots.clone(),
        plans: repo
            .data
            .automation
            .plans
            .iter()
            .rev()
            .take(100)
            .map(public_plan)
            .collect(),
        collections: repo.data.collections.clone(),
        catalog: json!([{"id":"extractAudio","label":"Extract audio","description":"One library file to MP3 or M4A."},{"id":"frame","label":"Save a frame","description":"One frame from a library video, up to 1920 pixels wide."}]),
    }
}
fn change(
    state: &AppState,
    edit: impl FnOnce(&mut Repository) -> Result<(), String>,
) -> Result<Status, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let previous = repo.data.automation.clone();
    let result = edit(repo)
        .and_then(|_| validate(&repo.data.automation))
        .and_then(|_| repo.save());
    if let Err(error) = result {
        repo.data.automation = previous;
        return Err(error);
    }
    Ok(status(repo))
}
fn updated(app: &AppHandle) {
    let _ = app.emit("automation-update", ());
}
pub fn start(app: &AppHandle, state: &Arc<AppState>) -> Result<(), String> {
    let root = {
        let guard = state.repository.lock().unwrap();
        let repo = guard.as_ref().ok_or("Open the app first.")?;
        if !repo.data.automation.enabled {
            return Ok(());
        }
        repo.root().to_owned()
    };
    let mut server = state.automation_pipe.lock().unwrap();
    if server.is_none() {
        *server = Some(crate::automation_pipe::start(app, state, &root)?);
    }
    Ok(())
}

pub fn run_worker(mut context: tauri::Context<tauri::Wry>) -> Result<(), String> {
    context.config_mut().app.windows.clear();
    let state = Arc::new(AppState {
        headless: true,
        ..AppState::default()
    });
    tauri::Builder::default()
        .setup(move |app| {
            let handle = app.handle().clone();
            let settings = crate::open_repository(&handle, &state)?;
            tauri::async_runtime::block_on(async { start(&handle, &state) })?;
            if state.automation_pipe.lock().unwrap().is_none() {
                return Err("Enable local automation in nabnab before starting its worker.".into());
            }
            crate::start_queue(&handle, &state, settings.processing_limit);
            let state = state.clone();
            // The CLI owns the write end. Its exit closes the lease without leaving a background worker.
            std::thread::spawn(move || {
                let _ = std::io::stdin().read(&mut [0u8; 1]);
                tauri::async_runtime::block_on(async {
                    use std::sync::atomic::Ordering;
                    state.runtime.updating.store(true, Ordering::SeqCst);
                    state.automation_pipe.lock().unwrap().take();
                    for cancel in state.runtime.active.lock().unwrap().values() {
                        let _ = cancel.send(true);
                    }
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(8), async {
                        while !state.runtime.active.lock().unwrap().is_empty() {
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                    })
                    .await;
                });
                handle.exit(0);
            });
            println!(
                "{}",
                json!({"version":1,"ok":true,"result":{"mode":"worker","state":"running"}})
            );
            Ok(())
        })
        .run(context)
        .map_err(|error| error.to_string())
}
#[tauri::command]
pub fn automation_status(state: State<Arc<AppState>>) -> Result<Status, String> {
    let guard = state.repository.lock().unwrap();
    Ok(status(guard.as_ref().ok_or("Open the app first.")?))
}
#[tauri::command]
pub async fn automation_enable(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    enabled: bool,
) -> Result<Status, String> {
    let result = change(&state, |repo| {
        repo.data.automation.enabled = enabled;
        Ok(())
    })?;
    if enabled {
        if let Err(error) = start(&app, &state) {
            let _ = change(&state, |repo| {
                repo.data.automation.enabled = false;
                Ok(())
            });
            return Err(error);
        }
    } else {
        state.automation_pipe.lock().unwrap().take();
        cancel_disallowed(&state);
    }
    updated(&app);
    Ok(result)
}
#[tauri::command]
pub async fn automation_choose_root(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Status, String> {
    let selected = app
        .dialog()
        .file()
        .set_title("Grant an automation output folder")
        .blocking_pick_folder();
    let selected = selected
        .map(|p| p.into_path().map_err(|e| e.to_string()))
        .transpose()?;
    let result = change(&state, |repo| {
        if let Some(path) = selected {
            let path = library::authorized_path(&path)?;
            if !path.is_dir() {
                return Err("Choose an existing local directory.".into());
            }
            if !repo.data.automation.roots.iter().any(|r| r.path == path) {
                if repo.data.automation.roots.len() >= 32 {
                    return Err("Remove a destination before granting another.".into());
                }
                let label = path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned();
                repo.data.automation.roots.push(OutputRoot {
                    id: credentials::random_id()?,
                    label,
                    path,
                });
            }
        }
        Ok(())
    })?;
    updated(&app);
    Ok(result)
}
#[tauri::command]
pub fn automation_remove_root(
    app: AppHandle,
    state: State<Arc<AppState>>,
    root_id: String,
) -> Result<Status, String> {
    let result = change(&state, |repo| {
        repo.data.automation.roots.retain(|r| r.id != root_id);
        for client in &mut repo.data.automation.clients {
            client.grant.output_root_ids.retain(|id| id != &root_id);
        }
        Ok(())
    })?;
    cancel_disallowed(&state);
    updated(&app);
    Ok(result)
}
#[tauri::command]
pub fn automation_save_client(
    app: AppHandle,
    state: State<Arc<AppState>>,
    client_id: String,
    grant: Grant,
) -> Result<Status, String> {
    let result = change(&state, |repo| {
        if grant
            .collections
            .iter()
            .any(|c| !repo.data.collections.contains(c))
            || grant
                .output_root_ids
                .iter()
                .any(|id| !repo.data.automation.roots.iter().any(|r| &r.id == id))
        {
            return Err("Select existing collections and granted destinations.".into());
        }
        let client = repo
            .data
            .automation
            .clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or("Unknown automation client.")?;
        if client.state == ClientState::Revoked {
            return Err("Connect a new client after revocation.".into());
        }
        client.grant = grant;
        client.state = ClientState::Authorized;
        Ok(())
    })?;
    cancel_disallowed(&state);
    updated(&app);
    Ok(result)
}
#[tauri::command]
pub fn automation_revoke_client(
    app: AppHandle,
    state: State<Arc<AppState>>,
    client_id: String,
) -> Result<Status, String> {
    let result = change(&state, |repo| {
        let client = repo
            .data
            .automation
            .clients
            .iter_mut()
            .find(|c| c.id == client_id)
            .ok_or("Unknown client.")?;
        client.state = ClientState::Revoked;
        for plan in &mut repo.data.automation.plans {
            if plan.client_id == client_id && plan.state != PlanState::Consumed {
                plan.state = PlanState::Denied;
            }
        }
        Ok(())
    })?;
    cancel_disallowed(&state);
    updated(&app);
    Ok(result)
}
#[tauri::command]
pub fn automation_approve_plan(
    app: AppHandle,
    state: State<Arc<AppState>>,
    plan_id: String,
    approve: bool,
) -> Result<Status, String> {
    let result = change(&state, |repo| {
        let index = repo
            .data
            .automation
            .plans
            .iter()
            .position(|p| p.id == plan_id)
            .ok_or("Unknown plan.")?;
        if repo.data.automation.plans[index].state != PlanState::Pending {
            return Err("This request has already been reviewed.".into());
        }
        if approve {
            grant_plan(repo, &repo.data.automation.plans[index])?;
        }
        repo.data.automation.plans[index].state = if approve {
            PlanState::Approved
        } else {
            PlanState::Denied
        };
        Ok(())
    })?;
    updated(&app);
    Ok(result)
}
fn allowed_item(client: &Client, item: &library::LibraryItem) -> bool {
    client
        .grant
        .collections
        .iter()
        .any(|c| item.collections.contains(c))
        && (client.grant.include_documents
            || !matches!(
                item.kind.as_str(),
                "document" | "article" | "subtitle" | "file"
            ))
}
fn current_client<'a>(
    repo: &'a Repository,
    id: &str,
    capability: &str,
) -> Result<&'a Client, ApiError> {
    if !repo.data.automation.enabled {
        return Err(api("disabled", "Automation is disabled."));
    }
    repo.data
        .automation
        .clients
        .iter()
        .find(|c| {
            c.id == id
                && c.state == ClientState::Authorized
                && c.grant.capabilities.iter().any(|v| v == capability)
        })
        .ok_or_else(|| {
            api(
                "permission_denied",
                "The client's current permissions do not allow this operation.",
            )
        })
}
fn grant_plan(repo: &Repository, plan: &Plan) -> Result<(), String> {
    if !repo.data.automation.enabled {
        return Err("Automation is disabled.".into());
    }
    let client = repo
        .data
        .automation
        .clients
        .iter()
        .find(|c| c.id == plan.client_id && c.state == ClientState::Authorized)
        .ok_or("The client's authorization has been revoked.")?;
    let can_plan = client.grant.capabilities.iter().any(|c| c == "jobs.plan")
        || (!matches!(plan.operation, Operation::Download { .. })
            && client.grant.capabilities.iter().any(|c| c == "tools.run"));
    if !can_plan
        || !client
            .grant
            .capabilities
            .iter()
            .any(|c| c == "jobs.enqueue")
        || !client.grant.collections.contains(&plan.collection)
        || !client.grant.output_root_ids.contains(&plan.output_root_id)
        || plan.budget.max_bytes > client.grant.max_bytes
        || plan.budget.max_seconds > client.grant.max_seconds
    {
        return Err("This plan exceeds the client's current permissions or budget.".into());
    }
    if plan.digest != plan_digest(plan)? || queue::now() >= plan.expires_at {
        return Err("This approval expired or its parameters changed. Prepare a new plan.".into());
    }
    let root = repo
        .data
        .automation
        .roots
        .iter()
        .find(|r| r.id == plan.output_root_id)
        .ok_or("The destination was revoked.")?;
    let current = library::authorized_path(&root.path)?;
    if current != plan.output_path || !current.is_dir() {
        return Err("The granted output directory changed.".into());
    }
    if let queue::Source::Transform { library_id, .. } = &plan.source {
        let item = repo
            .data
            .library
            .iter()
            .find(|i| &i.id == library_id && allowed_item(client, i))
            .ok_or("The selected library item is no longer authorized.")?;
        library::authorized_path(&item.path)?;
    }
    Ok(())
}
fn file_hash(path: &Path) -> Result<String, String> {
    let path = library::authorized_path(path)?;
    let mut file = fs::File::open(path).map_err(|_| "The selected input is unavailable.")?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 1024 * 1024 * 1024 {
        return Err("Automation inputs are limited to 1 GiB.".into());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut bytes = 0u64;
    loop {
        let length = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if length == 0 {
            break;
        }
        bytes += length as u64;
        if bytes > 1024 * 1024 * 1024 {
            return Err("Automation inputs are limited to 1 GiB.".into());
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn source_identity(input: &str) -> Result<String, String> {
    let url = url::Url::parse(input).map_err(|_| "Invalid source URL.")?;
    let file = url
        .path_segments()
        .and_then(|mut parts| parts.next_back())
        .filter(|part| {
            part.len() <= 120
                && part
                    .chars()
                    .all(|c| c.is_alphanumeric() || "-_.%".contains(c))
                && part.rsplit('.').next().is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "mp4"
                            | "webm"
                            | "mkv"
                            | "mov"
                            | "mp3"
                            | "m4a"
                            | "wav"
                            | "opus"
                            | "ogg"
                            | "flac"
                            | "png"
                            | "jpg"
                            | "jpeg"
                    )
                })
        });
    Ok(format!(
        "{}{}/#source-{}",
        url.origin().ascii_serialization(),
        file.map(|name| format!("/…/{name}")).unwrap_or_default(),
        &hash(input.as_bytes())[..16]
    ))
}
fn keep_plan(repo: &Repository, plan: &Plan) -> bool {
    plan.expires_at > queue::now()
        || (plan.state == PlanState::Consumed
            && repo.data.jobs.iter().any(|job| {
                plan.job_id.as_deref() == Some(job.id.as_str())
                    && !matches!(
                        job.state,
                        queue::JobState::Completed
                            | queue::JobState::Cancelled
                            | queue::JobState::Error
                    )
            }))
}
fn persist_plan(repo: &mut Repository, mut plan: Plan) -> Result<Plan, String> {
    if repo
        .data
        .automation
        .plans
        .iter()
        .any(|existing| existing.id == plan.id)
    {
        return Err("This plan identifier has already been saved.".into());
    }
    plan.digest = plan_digest(&plan)?;
    grant_plan(repo, &plan)?;
    let retained = repo
        .data
        .automation
        .plans
        .iter()
        .filter(|p| keep_plan(repo, p))
        .cloned()
        .collect::<Vec<_>>();
    if retained.len() >= MAX_PLANS {
        return Err("Review outstanding requests before preparing more plans.".into());
    }
    let reference = if let queue::Source::Download { url, url_ref, .. } = &mut plan.source {
        let reference =
            crate::sessions::seal_url(repo.root(), &format!("automation-{}", plan.id), url)?;
        *url = queue::public_origin(url)?;
        *url_ref = Some(reference.clone());
        Some(reference)
    } else {
        None
    };
    plan.digest = plan_digest(&plan)?;
    let previous = std::mem::replace(&mut repo.data.automation.plans, retained);
    repo.data.automation.plans.push(plan.clone());
    if let Err(error) = repo.save() {
        repo.data.automation.plans = previous;
        if let Some(reference) = reference {
            let _ = crate::sessions::remove_uncommitted_url(repo.root(), &reference);
        }
        return Err(error);
    }
    Ok(plan)
}
pub(crate) fn authorize_cancel(
    repo: &Repository,
    client_id: &str,
    ids: &[String],
) -> Result<(), String> {
    current_client(repo, client_id, "jobs.cancel")
        .map_err(|_| "This client's cancellation permission is no longer active.")?;
    if ids.iter().any(|id| {
        !repo.data.jobs.iter().any(|job| {
            &job.id == id
                && job
                    .automation
                    .as_ref()
                    .is_some_and(|a| a.client_id == client_id)
        })
    }) {
        return Err("This client does not own the selected job.".into());
    }
    Ok(())
}
fn api(code: &str, message: &str) -> ApiError {
    ApiError {
        code: code.into(),
        message: message.into(),
        retryable: false,
    }
}
fn invalid(message: &str) -> ApiError {
    api("invalid_request", message)
}
fn params<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, ApiError> {
    serde_json::from_value(value)
        .map_err(|_| invalid("Parameters do not match this operation's closed schema."))
}
fn item_view(item: &library::LibraryItem) -> Value {
    json!({"id":item.id,"name":item.name,"kind":item.kind,"size":item.size,"duration":item.duration,"width":item.width,"height":item.height})
}
fn job_view(job: &queue::Job) -> Value {
    json!({"id":job.id,"title":job.title,"state":job.state,"createdAt":job.created_at,"updatedAt":job.updated_at,"progress":job.progress,"error":job.error.as_ref().map(|e|json!({"code":e.code,"retryable":e.retryable})),"outputCount":job.outputs.len()})
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Id {
    plan_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JobId {
    job_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ItemId {
    item_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Search {
    #[serde(default)]
    query: String,
    limit: Option<u16>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct JobQuery {
    job_id: Option<String>,
}

fn authorize_current(state: &AppState, id: &str, capability: &str) -> Result<(), ApiError> {
    let guard = state.repository.lock().unwrap();
    let repo = guard
        .as_ref()
        .ok_or_else(|| api("owner_unavailable", "Open nabnab first."))?;
    if !repo.data.automation.enabled
        || !repo.data.automation.clients.iter().any(|c| {
            c.id == id
                && c.state == ClientState::Authorized
                && c.grant.capabilities.iter().any(|v| v == capability)
        })
    {
        return Err(api(
            "permission_denied",
            "The authorization changed during this request.",
        ));
    }
    Ok(())
}
fn prepare(state: &AppState, client: &Client, input: PlanInput) -> Result<Plan, String> {
    let guard = state.repository.lock().unwrap();
    let repo = guard.as_ref().ok_or("Open the app first.")?;
    if !budget_valid(input.budget.max_bytes, input.budget.max_seconds) {
        return Err("Choose an output budget of 1–512 MiB and 5–600 seconds.".into());
    }
    let output = repo
        .data
        .automation
        .roots
        .iter()
        .find(|r| r.id == input.output_root_id)
        .ok_or("Choose a granted output root ID.")?;
    let output_path = library::authorized_path(&output.path)?;
    let mut operation = input.operation;
    let (source, title) = match &mut operation {
        Operation::Download { url, format } => {
            if !matches!(
                format,
                crate::model::Format::Mp4
                    | crate::model::Format::Mp3
                    | crate::model::Format::M4a
                    | crate::model::Format::Opus
                    | crate::model::Format::Source
            ) {
                return Err("Choose MP4, MP3, M4A, Opus or source.".into());
            }
            let operational = crate::model::validate_url(url)?;
            if crate::providers::course_page(&operational) {
                return Err("Courses must be selected in the desktop app.".into());
            }
            *url = source_identity(&operational)?;
            (
                queue::Source::Download {
                    url: operational,
                    url_ref: None,
                    format: *format,
                    height: None,
                    audio_bitrate: None,
                    subtitle_language: None,
                },
                format!("Public {:?} download", format),
            )
        }
        Operation::ExtractAudio { library_id, format } => {
            if !matches!(format.as_str(), "mp3" | "m4a") {
                return Err("Choose MP3 or M4A.".into());
            }
            let item = repo
                .data
                .library
                .iter()
                .find(|i| {
                    &i.id == library_id
                        && allowed_item(client, i)
                        && matches!(i.kind.as_str(), "audio" | "video")
                })
                .ok_or("Select an authorized audio or video library ID.")?;
            (
                queue::Source::Transform {
                    library_id: library_id.clone(),
                    options: ToolOptions::ExtractAudio {
                        format: format.clone(),
                    },
                },
                item.name.clone(),
            )
        }
        Operation::Frame {
            library_id,
            at,
            width,
        } => {
            if !at.is_finite() || *at < 0.0 || !(64..=1920).contains(width) {
                return Err("Choose a nonnegative frame time and width of 64–1920 pixels.".into());
            }
            let item = repo
                .data
                .library
                .iter()
                .find(|i| &i.id == library_id && allowed_item(client, i) && i.kind == "video")
                .ok_or("Select an authorized video library ID.")?;
            if item.duration.is_some_and(|d| *at >= d) {
                return Err("The frame time must be inside the video.".into());
            }
            (
                queue::Source::Transform {
                    library_id: library_id.clone(),
                    options: ToolOptions::Frame {
                        at: *at,
                        width: *width,
                    },
                },
                item.name.clone(),
            )
        }
    };
    let created_at = queue::now();
    let mut plan = Plan {
        id: credentials::random_id()?,
        client_id: client.id.clone(),
        client_name: client.name.clone(),
        created_at,
        expires_at: created_at + 600_000,
        state: PlanState::Pending,
        operation,
        output_root_id: output.id.clone(),
        output_label: output.label.clone(),
        output_path,
        collection: input.collection,
        budget: input.budget,
        digest: String::new(),
        job_id: None,
        source,
        input_hash: None,
        title,
    };
    plan.digest = plan_digest(&plan)?;
    grant_plan(repo, &plan)?;
    if repo
        .data
        .automation
        .plans
        .iter()
        .filter(|p| keep_plan(repo, p))
        .count()
        >= MAX_PLANS
    {
        return Err("Review outstanding requests before preparing more plans.".into());
    }
    Ok(plan)
}
pub fn begin_run(state: &AppState, job: &queue::Job) -> Result<(), String> {
    let Some(auth) = &job.automation else {
        return Ok(());
    };
    let seconds = {
        let guard = state.repository.lock().unwrap();
        let repo = guard.as_ref().ok_or("Store unavailable.")?;
        job_allowed(repo, job)?;
        repo.data
            .automation
            .plans
            .iter()
            .find(|p| p.id == auth.plan_id)
            .ok_or("Automation approval is missing.")?
            .budget
            .max_seconds
    };
    state.automation_deadlines.lock().unwrap().insert(
        job.id.clone(),
        (
            job.run_id,
            std::time::Instant::now() + std::time::Duration::from_secs(seconds as u64),
        ),
    );
    Ok(())
}
pub fn time_allowed(state: &AppState, job: &queue::Job) -> Result<(), String> {
    if job.automation.is_none() {
        return Ok(());
    }
    let deadlines = state.automation_deadlines.lock().unwrap();
    if !deadlines.get(&job.id).is_some_and(|(run_id, deadline)| {
        *run_id == job.run_id && std::time::Instant::now() < *deadline
    }) {
        return Err("Automation runtime budget exceeded. Prepare a new plan to continue.".into());
    }
    Ok(())
}
pub fn end_run(state: &AppState, job: &queue::Job) {
    let mut deadlines = state.automation_deadlines.lock().unwrap();
    if deadlines
        .get(&job.id)
        .is_some_and(|(run_id, _)| *run_id == job.run_id)
    {
        deadlines.remove(&job.id);
    }
}
fn enqueue(state: &AppState, client_id: &str, plan_id: &str) -> Result<Value, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let index = repo
        .data
        .automation
        .plans
        .iter()
        .position(|p| p.id == plan_id && p.client_id == client_id)
        .ok_or("No plan belonging to this client has this ID.")?;
    let plan = repo.data.automation.plans[index].clone();
    if plan.state != PlanState::Approved {
        return Err("Review and approve this exact plan in Settings → Automation. Each approval can be consumed once.".into());
    }
    grant_plan(repo, &plan)?;
    let mut job = queue::Job::new(
        plan.source.clone(),
        plan.title.clone(),
        plan.output_path.clone(),
        format!("auto-{}", plan.id),
        None,
    );
    job.automation = Some(JobAuthorization {
        client_id: client_id.into(),
        plan_id: plan.id.clone(),
        digest: plan.digest.clone(),
    });
    job.associations.push(queue::OutputAssociation {
        source_id: None,
        collection: Some(plan.collection.clone()),
        tags: vec![],
    });
    let job_id = job.id.clone();
    let length = repo.data.jobs.len();
    repo.data.jobs.push(job);
    repo.data.automation.plans[index].state = PlanState::Consumed;
    repo.data.automation.plans[index].job_id = Some(job_id.clone());
    if let Err(error) = repo.save() {
        repo.data.jobs.truncate(length);
        repo.data.automation.plans[index] = plan;
        return Err(error);
    }
    Ok(json!({"jobId":job_id,"planId":plan_id,"state":"pending"}))
}
pub fn job_allowed(repo: &Repository, job: &queue::Job) -> Result<(), String> {
    let Some(authorization) = &job.automation else {
        return Ok(());
    };
    let plan = repo
        .data
        .automation
        .plans
        .iter()
        .find(|p| {
            p.id == authorization.plan_id
                && p.client_id == authorization.client_id
                && p.state == PlanState::Consumed
                && p.job_id.as_deref() == Some(job.id.as_str())
        })
        .ok_or("Automation approval is missing or does not belong to this job.")?;
    grant_plan(repo, plan)?;
    if authorization.digest != plan.digest
        || job.folder != plan.output_path
        || serde_json::to_value(&job.source).ok() != serde_json::to_value(&plan.source).ok()
    {
        return Err("Automation job parameters differ from the approved plan.".into());
    }
    Ok(())
}
pub fn verify_input(state: &AppState, job: &queue::Job) -> Result<(), String> {
    let input = {
        let guard = state.repository.lock().unwrap();
        let repo = guard.as_ref().ok_or("Store unavailable.")?;
        job_allowed(repo, job)?;
        let Some(authorization) = &job.automation else {
            return Ok(());
        };
        let plan = repo
            .data
            .automation
            .plans
            .iter()
            .find(|p| p.id == authorization.plan_id)
            .ok_or("The automation plan is missing.")?;
        match &plan.source {
            queue::Source::Transform { library_id, .. } => Some((
                repo.data
                    .library
                    .iter()
                    .find(|i| &i.id == library_id)
                    .ok_or("The selected input is unavailable.")?
                    .path
                    .clone(),
                plan.input_hash
                    .clone()
                    .ok_or("The approved input hash is missing.")?,
            )),
            _ => None,
        }
    };
    if let Some((path, expected)) = input {
        if file_hash(&path)? != expected {
            return Err("The approved input changed. Prepare and approve a new plan.".into());
        }
    }
    Ok(())
}
pub fn runtime_guard(
    state: &AppState,
    job: &queue::Job,
    staging: &Path,
    elapsed: std::time::Duration,
) -> Result<(), String> {
    let budget = {
        let guard = state.repository.lock().unwrap();
        let repo = guard.as_ref().ok_or("Store unavailable.")?;
        job_allowed(repo, job)?;
        let Some(auth) = &job.automation else {
            return Ok(());
        };
        repo.data
            .automation
            .plans
            .iter()
            .find(|p| p.id == auth.plan_id)
            .ok_or("The automation approval is missing.")?
            .budget
            .clone()
    };
    if elapsed.as_secs() >= budget.max_seconds as u64 {
        return Err(
            "Automation runtime budget exceeded. Prepare a new approval to continue.".into(),
        );
    }
    let mut total = 0u64;
    let mut pending = vec![staging.to_owned()];
    let mut entries = 0;
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            entries += 1;
            if entries > 4096 {
                return Err("Automation output file count exceeded its bound.".into());
            }
            let meta = entry.metadata().map_err(|e| e.to_string())?;
            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
            if total > budget.max_bytes {
                return Err(
                    "Automation disk budget exceeded. The output was not published.".into(),
                );
            }
        }
    }
    Ok(())
}
pub fn output_allowed(repo: &Repository, job: &queue::Job, bytes: u64) -> Result<(), String> {
    job_allowed(repo, job)?;
    if let Some(auth) = &job.automation {
        let plan = repo
            .data
            .automation
            .plans
            .iter()
            .find(|p| p.id == auth.plan_id)
            .ok_or("Automation approval is missing.")?;
        if bytes > plan.budget.max_bytes {
            return Err("Automation output exceeds the approved byte budget.".into());
        }
    }
    Ok(())
}
fn cancel_disallowed(state: &AppState) {
    let ids = {
        let guard = state.repository.lock().unwrap();
        let Some(repo) = guard.as_ref() else {
            return;
        };
        repo.data
            .jobs
            .iter()
            .filter(|j| {
                j.automation.is_some()
                    && !matches!(
                        j.state,
                        queue::JobState::Completed
                            | queue::JobState::Cancelled
                            | queue::JobState::Error
                    )
                    && job_allowed(repo, j).is_err()
            })
            .map(|j| j.id.clone())
            .collect::<Vec<_>>()
    };
    if !ids.is_empty() {
        let _ = queue::action(state, &ids, "cancel");
        state.runtime.notify.notify_one();
    }
}

#[cfg(test)]
pub(crate) mod tests;
