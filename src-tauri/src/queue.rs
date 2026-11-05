use crate::{
    engine,
    failure::Failure,
    library, media,
    model::{Format, Progress},
    providers,
    store::Repository,
    tools::ToolOptions,
    AppState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use tokio::sync::{watch, Notify, Semaphore};

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobState {
    Pending,
    Analyzing,
    Ready,
    Downloading,
    Processing,
    Verifying,
    Completed,
    Paused,
    Cancelled,
    Error,
}
impl JobState {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Analyzing | Self::Ready | Self::Downloading | Self::Processing | Self::Verifying
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Source {
    Download {
        url: String,
        #[serde(default)]
        url_ref: Option<String>,
        format: Format,
        height: Option<u32>,
        audio_bitrate: Option<u16>,
    },
    Transform {
        library_id: String,
        options: ToolOptions,
    },
}
impl Source {
    fn network(&self) -> bool {
        matches!(self, Self::Download { .. })
    }
    fn domain(&self) -> String {
        match self {
            Self::Download { url, .. } => providers::identify(url)
                .map(|p| {
                    if p.id == providers::ProviderId::Generic {
                        p.host
                    } else {
                        format!("{:?}", p.id)
                    }
                })
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobError {
    pub code: String,
    pub message: String,
    pub technical: String,
    pub retryable: bool,
    pub retry_after: Option<u64>,
}
impl From<Failure> for JobError {
    fn from(f: Failure) -> Self {
        Self {
            code: f.code.into(),
            message: f.message.into(),
            technical: f.technical,
            retryable: f.retryable,
            retry_after: f.retry_after,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub run_id: u64,
    pub client_request: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub state: JobState,
    pub source: Source,
    pub folder: PathBuf,
    pub provider: String,
    pub thumbnail: Option<String>,
    pub progress: Option<Progress>,
    pub outputs: Vec<PathBuf>,
    pub error: Option<JobError>,
    pub attempt: u32,
    pub group_id: Option<String>,
    #[serde(default)]
    pub next_retry_at: Option<u64>,
}
impl Job {
    pub fn protect_source(&mut self, root: &Path) -> Result<(), String> {
        if let Source::Download { url, url_ref, .. } = &mut self.source {
            let origin = public_origin(url)?;
            if url_ref.is_none() {
                *url_ref = Some(crate::sessions::seal_url(root, &self.id, url)?);
            }
            if !url_ref
                .as_deref()
                .is_some_and(crate::sessions::valid_url_reference)
            {
                return Err("The protected download reference is invalid.".into());
            }
            *url = origin;
        }
        Ok(())
    }

    pub fn validate_public_source(&self) -> Result<(), String> {
        if let Source::Download { url, url_ref, .. } = &self.source {
            if public_origin(url)? != *url
                || !url_ref
                    .as_deref()
                    .is_some_and(crate::sessions::valid_url_reference)
            {
                return Err(
                    "The job must contain a public origin and a protected download reference."
                        .into(),
                );
            }
        }
        Ok(())
    }

    pub fn recover(&mut self) {
        if self.state.active() || self.state == JobState::Pending {
            self.state = JobState::Paused;
            self.progress = None;
            self.next_retry_at = None;
            self.updated_at = now();
        }
    }
    fn new(
        source: Source,
        title: String,
        folder: PathBuf,
        client_request: String,
        group_id: Option<String>,
    ) -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let time = now();
        let provider = match &source {
            Source::Download { url, .. } => {
                providers::identify(url).map(|p| p.name).unwrap_or_default()
            }
            _ => "FFmpeg".into(),
        };
        Self {
            id: format!("{time}-{}", SEQUENCE.fetch_add(1, Ordering::SeqCst)),
            run_id: 0,
            client_request,
            title: title.chars().take(240).collect(),
            created_at: time,
            updated_at: time,
            state: JobState::Pending,
            source,
            folder,
            provider,
            thumbnail: None,
            progress: None,
            outputs: vec![],
            error: None,
            attempt: 0,
            group_id,
            next_retry_at: None,
        }
    }
}

pub struct Runtime {
    pub active: Mutex<HashMap<String, watch::Sender<bool>>>,
    pub started: AtomicBool,
    pub updating: AtomicBool,
    pub notify: Notify,
    pub processing: Arc<Semaphore>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            active: Mutex::new(HashMap::new()),
            started: AtomicBool::new(false),
            updating: AtomicBool::new(false),
            notify: Notify::new(),
            processing: Arc::new(Semaphore::new(1)),
        }
    }
}

pub fn validate_request_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 192
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_:.".contains(&c))
    {
        return Err("Invalid request identifier.".into());
    }
    Ok(())
}
fn public_origin(input: &str) -> Result<String, String> {
    let normalized = crate::model::validate_url(input)?;
    let parsed = url::Url::parse(&normalized).map_err(|_| "Invalid URL.")?;
    Ok(parsed.origin().ascii_serialization())
}

pub fn enqueue_download(
    repo: &mut Repository,
    urls: Vec<String>,
    titles: Vec<String>,
    format: Format,
    height: Option<u32>,
    audio_bitrate: Option<u16>,
    request: &str,
) -> Result<Vec<Job>, String> {
    validate_request_id(request)?;
    let previous: Vec<_> = repo
        .data
        .jobs
        .iter()
        .filter(|j| j.client_request == request)
        .cloned()
        .collect();
    if !previous.is_empty() {
        return Ok(previous);
    }
    if urls.is_empty() || urls.len() > 200 {
        return Err("Choose between 1 and 200 links.".into());
    }
    if height.is_some_and(|h| !format.is_video() || h == 0 || h > 16384) {
        return Err("Invalid output height.".into());
    }
    if audio_bitrate.is_some_and(|b| format != Format::Mp3 || !matches!(b, 128 | 192 | 256 | 320)) {
        return Err("Invalid MP3 bitrate.".into());
    }
    let group = (urls.len() > 1).then(|| request.to_owned());
    let jobs: Vec<_> = urls
        .into_iter()
        .enumerate()
        .map(|(i, url)| {
            if url.len() > 8192 {
                return Err("Use a download URL of at most 8192 bytes.".into());
            }
            crate::model::validate_url(&url)?;
            let title = titles
                .get(i)
                .filter(|s| !s.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| {
                    providers::identify(&url)
                        .map(|p| p.name)
                        .unwrap_or_else(|_| "Media".into())
                });
            Ok(Job::new(
                Source::Download {
                    url: url.trim().to_owned(),
                    url_ref: None,
                    format,
                    height,
                    audio_bitrate,
                },
                title,
                repo.data.settings.folder.clone(),
                request.into(),
                group.clone(),
            ))
        })
        .collect::<Result<_, String>>()?;
    let mut jobs = jobs;
    for job in &mut jobs {
        job.protect_source(repo.root())?;
    }
    let old_len = repo.data.jobs.len();
    repo.data.jobs.extend(jobs.clone());
    if let Err(error) = repo.save() {
        repo.data.jobs.truncate(old_len);
        return Err(error);
    }
    Ok(jobs)
}

pub fn enqueue_tool(
    repo: &mut Repository,
    ids: Vec<String>,
    options: ToolOptions,
    request: &str,
) -> Result<Vec<Job>, String> {
    validate_request_id(request)?;
    let previous: Vec<_> = repo
        .data
        .jobs
        .iter()
        .filter(|j| j.client_request == request)
        .cloned()
        .collect();
    if !previous.is_empty() {
        return Ok(previous);
    }
    if ids.is_empty() || ids.len() > 200 {
        return Err("Select between 1 and 200 files.".into());
    }
    let mut jobs = vec![];
    for id in ids {
        let item = repo
            .data
            .library
            .iter()
            .find(|i| i.id == id)
            .ok_or("Choose an imported library file.")?;
        library::authorized_path(&item.path)?;
        jobs.push(Job::new(
            Source::Transform {
                library_id: id,
                options: options.clone(),
            },
            item.name.clone(),
            repo.data.settings.folder.clone(),
            request.into(),
            Some(request.into()),
        ));
    }
    let old_len = repo.data.jobs.len();
    repo.data.jobs.extend(jobs.clone());
    if let Err(error) = repo.save() {
        repo.data.jobs.truncate(old_len);
        return Err(error);
    }
    Ok(jobs)
}

pub fn action(state: &AppState, ids: &[String], action: &str) -> Result<(), String> {
    if ids.len() > 1000 || !matches!(action, "cancel" | "retry" | "pause") {
        return Err("Invalid queue action.".into());
    }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let mut indices = Vec::new();
    for id in ids {
        let index = repo
            .data
            .jobs
            .iter()
            .position(|j| j.id == *id)
            .ok_or("Job not found.")?;
        if !indices.contains(&index) {
            indices.push(index);
        }
    }
    if action == "retry" {
        let active = state.runtime.active.lock().unwrap();
        if indices
            .iter()
            .any(|i| active.contains_key(&repo.data.jobs[*i].id))
        {
            return Err("Wait for the previous execution to stop and finish cleaning its files before retrying.".into());
        }
    }
    let previous = repo.data.jobs.clone();
    let mut cancellations = vec![];
    for index in indices {
        let job = &mut repo.data.jobs[index];
        match action {
            "cancel"
                if job.state.active()
                    || matches!(job.state, JobState::Pending | JobState::Paused) =>
            {
                job.state = JobState::Cancelled;
                cancellations.push(job.id.clone());
            }
            "pause" if job.state == JobState::Pending => job.state = JobState::Paused,
            "retry"
                if matches!(
                    job.state,
                    JobState::Error | JobState::Paused | JobState::Cancelled
                ) =>
            {
                job.state = JobState::Pending;
                job.error = None;
                job.attempt = 0;
                job.progress = None;
                job.next_retry_at = None;
            }
            _ => {}
        }
        job.updated_at = now();
    }
    if let Err(error) = repo.save() {
        repo.data.jobs = previous;
        return Err(error);
    }
    drop(guard);
    for id in cancellations {
        if let Some(cancel) = state.runtime.active.lock().unwrap().get(&id) {
            let _ = cancel.send(true);
        }
    }
    state.runtime.notify.notify_one();
    Ok(())
}

fn progress(app: &AppHandle, state: &AppState, id: &str, run_id: u64, update: Progress) {
    let mut guard = state.repository.lock().unwrap();
    if let Some(repo) = guard.as_mut() {
        if let Some(job) = repo
            .data
            .jobs
            .iter_mut()
            .find(|j| j.id == id && j.run_id == run_id && j.state.active())
        {
            job.state = match update.phase.as_str() {
                "processing" => JobState::Processing,
                "verifying" => JobState::Verifying,
                _ => JobState::Downloading,
            };
            job.progress = Some(update);
            job.updated_at = now();
            let _ = app.emit("job-update", job.clone());
        }
    }
}
fn phase(
    app: &AppHandle,
    state: &AppState,
    id: &str,
    run_id: u64,
    value: JobState,
) -> Result<(), String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Store unavailable.")?;
    let job = repo
        .data
        .jobs
        .iter_mut()
        .find(|j| j.id == id && j.run_id == run_id)
        .ok_or(engine::CANCELLED)?;
    if job.state == JobState::Cancelled {
        return Err(engine::CANCELLED.into());
    }
    job.state = value;
    job.updated_at = now();
    job.progress = None;
    let event = job.clone();
    repo.save()?;
    let _ = app.emit("job-update", event);
    Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Owner {
    schema_version: u32,
    job_id: String,
    #[serde(default)]
    run_id: u64,
}
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Publication {
    schema_version: u32,
    job_id: String,
    source_file: PathBuf,
    target: PathBuf,
    sha256: String,
    item: library::LibraryItem,
}
const OWNER: &str = "owner.json";
const JOURNAL: &str = "publication.json";

fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return meta.file_attributes() & 0x400 != 0;
    }
    #[cfg(not(windows))]
    false
}
fn stage_path(job: &Job) -> Result<(PathBuf, PathBuf), String> {
    if job.id.is_empty()
        || job.id.len() > 80
        || !job.id.bytes().all(|c| c.is_ascii_digit() || c == b'-')
    {
        return Err("Invalid job directory.".into());
    }
    let parent = library::authorized_path(&job.folder)
        .map_err(|_| "The destination folder is unavailable. Choose another folder.")?;
    if !parent.is_dir() {
        return Err("The destination is not a folder.".into());
    }
    Ok((parent.clone(), parent.join(format!(".nabnab-{}", job.id))))
}
fn read_owned_json<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Result<T, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("Could not read the job recovery record: {e}"))?;
    if is_link(&metadata) || !metadata.is_file() || metadata.len() > limit {
        return Err("The job recovery record is invalid.".into());
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    serde_json::from_reader(file).map_err(|_| "The job recovery record is invalid.".into())
}
fn owned_stage(job: &Job, staging: &Path) -> Result<PathBuf, String> {
    let (parent, expected) = stage_path(job)?;
    if staging != expected {
        return Err("The job directory does not belong to this job.".into());
    }
    let meta = fs::symlink_metadata(staging).map_err(|e| e.to_string())?;
    if is_link(&meta) || !meta.is_dir() {
        return Err("Job directory cannot be a symbolic link or reparse point.".into());
    }
    let canonical = fs::canonicalize(staging).map_err(|e| e.to_string())?;
    if canonical.parent() != Some(parent.as_path()) {
        return Err("Invalid job directory.".into());
    }
    let owner: Owner = read_owned_json(&canonical.join(OWNER), 4096)?;
    if owner.schema_version != 1 || owner.job_id != job.id {
        return Err(
            "The existing job directory is not owned by this job. Its files were left untouched."
                .into(),
        );
    }
    Ok(canonical)
}
fn stage(job: &Job) -> Result<PathBuf, String> {
    let (parent, staging) = stage_path(job)?;
    let fresh = match fs::create_dir(&staging) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let owned = owned_stage(job, &staging)?;
            let owner: Owner = read_owned_json(&owned.join(OWNER), 4096)?;
            let journaled = [JOURNAL, "publication.new"]
                .iter()
                .any(|name| fs::symlink_metadata(owned.join(name)).is_ok());
            if matches!(job.source, Source::Transform { .. })
                && owner.run_id != job.run_id
                && !journaled
            {
                let archived =
                    parent.join(format!(".nabnab-{}-interrupted-{}", job.id, owner.run_id));
                if fs::symlink_metadata(&archived).is_ok() {
                    return Err("This job's interrupted-results archive already exists. No files were replaced. Review its folder before retrying.".into());
                }
                #[cfg(windows)]
                {
                    use std::os::windows::ffi::OsStrExt;
                    let from: Vec<u16> = owned.as_os_str().encode_wide().chain(Some(0)).collect();
                    let to: Vec<u16> = archived.as_os_str().encode_wide().chain(Some(0)).collect();
                    if unsafe {
                        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                            from.as_ptr(),
                            to.as_ptr(),
                            0,
                        )
                    } == 0
                    {
                        return Err("Could not preserve the interrupted transform output. No files were replaced.".into());
                    }
                }
                #[cfg(not(windows))]
                fs::rename(&owned, &archived)
                    .map_err(|_| "Could not preserve the interrupted transform output.")?;
                fs::create_dir(&staging).map_err(|_| "The interrupted output was preserved, but a fresh job folder could not be created.")?;
                true
            } else {
                false
            }
        }
        Err(e) => {
            return Err(format!(
                "Could not write to the destination (check space and permissions): {e}"
            ))
        }
    };
    if fresh {
        let bytes = serde_json::to_vec(&Owner {
            schema_version: 1,
            job_id: job.id.clone(),
            run_id: job.run_id,
        })
        .map_err(|e| e.to_string())?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staging.join(OWNER))
            .map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    owned_stage(job, &staging)
}
fn cleanup(job: &Job, staging: &Path) -> Result<(), String> {
    let owned = owned_stage(job, staging)?;
    fs::remove_dir_all(owned)
        .map_err(|e| format!("Could not remove the job's temporary files: {e}"))
}
fn file_hash(file: &Path, cancel: &watch::Receiver<bool>) -> Result<String, String> {
    let mut file = fs::File::open(file).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        if *cancel.borrow() {
            return Err(engine::CANCELLED.into());
        }
        let count = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
async fn digest(file: PathBuf, cancel: watch::Receiver<bool>) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || file_hash(&file, &cancel))
        .await
        .map_err(|e| e.to_string())?
}
fn target_item(mut item: library::LibraryItem, target: PathBuf) -> library::LibraryItem {
    let key = target.to_string_lossy().into_owned();
    #[cfg(windows)]
    let key = key.to_lowercase();
    item.id = format!("{:x}", Sha256::digest(key.as_bytes()));
    item.name = target
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    item.path = target;
    item
}
fn validate_journal(job: &Job, staging: &Path, journal: &Publication) -> Result<(), String> {
    let stage = owned_stage(job, staging)?;
    let (parent, _) = stage_path(job)?;
    if journal.schema_version != 1
        || journal.job_id != job.id
        || journal.source_file.parent() != Some(stage.as_path())
        || journal.target.parent() != Some(parent.as_path())
        || journal.source_file.file_name() != journal.target.file_name()
        || journal.item.path != journal.target
        || journal.item.size == 0
        || journal.sha256.len() != 64
        || !journal.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("The publication recovery record does not match this job. Existing files were left untouched.".into());
    }
    Ok(())
}
fn write_journal(job: &Job, staging: &Path, journal: &Publication) -> Result<(), String> {
    validate_journal(job, staging, journal)?;
    let temporary = staging.join("publication.new");
    if let Ok(meta) = fs::symlink_metadata(&temporary) {
        if is_link(&meta) || !meta.is_file() {
            return Err("Invalid temporary publication record.".into());
        }
    }
    let mut file = fs::File::create(&temporary).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec(journal).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    let checked: Publication = read_owned_json(&temporary, 1024 * 1024)?;
    validate_journal(job, staging, &checked)?;
    fs::rename(temporary, staging.join(JOURNAL))
        .map_err(|e| format!("Could not record output publication: {e}"))
}
fn has_published_output(job: &Job, staging: &Path) -> Result<bool, String> {
    for name in [JOURNAL, "publication.new"] {
        let path = staging.join(name);
        if fs::symlink_metadata(&path).is_ok() {
            let journal: Publication = read_owned_json(&path, 1024 * 1024)?;
            validate_journal(job, staging, &journal)?;
            return Ok(fs::symlink_metadata(journal.target).is_ok());
        }
    }
    Ok(false)
}
fn publication_guard(
    state: &AppState,
    job: &Job,
    cancel: &watch::Receiver<bool>,
) -> Result<(), String> {
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let current = state.repository.lock().unwrap();
    let repo = current.as_ref().ok_or("Store unavailable.")?;
    checked_run(repo, job, cancel)
}
fn checked_run(repo: &Repository, job: &Job, cancel: &watch::Receiver<bool>) -> Result<(), String> {
    if *cancel.borrow()
        || !repo
            .data
            .jobs
            .iter()
            .any(|j| j.id == job.id && j.run_id == job.run_id && j.state.active())
    {
        return Err(engine::CANCELLED.into());
    }
    Ok(())
}
async fn recover_publication(
    state: &AppState,
    job: &Job,
    staging: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<Option<(PathBuf, library::LibraryItem)>, String> {
    let journal_path = if staging.join(JOURNAL).exists() {
        staging.join(JOURNAL)
    } else if staging.join("publication.new").exists() {
        staging.join("publication.new")
    } else {
        return Ok(None);
    };
    let journal: Publication = read_owned_json(&journal_path, 1024 * 1024)?;
    validate_journal(job, staging, &journal)?;
    let target_exists = fs::symlink_metadata(&journal.target).is_ok();
    let candidate = if target_exists {
        library::authorized_path(&journal.target)?
    } else {
        library::authorized_path(&journal.source_file).map_err(|_|"The recorded output is missing. Locate the file or create a new job; no existing file was replaced.")?
    };
    if fs::metadata(&candidate).map_err(|e| e.to_string())?.len() != journal.item.size
        || digest(candidate.clone(), cancel.clone()).await? != journal.sha256
    {
        return Err("This file already exists with different contents, or the recovery file changed. Choose another destination or create a new job. No file was replaced.".into());
    }
    let guard = state.repository.lock().unwrap();
    let repo = guard.as_ref().ok_or("Store unavailable.")?;
    checked_run(repo, job, &cancel)?;
    if target_exists {
        if candidate != journal.target {
            return Err(
                "The recovered output path changed. Existing files were left untouched.".into(),
            );
        }
        Ok(Some((journal.target, journal.item)))
    } else {
        let target = crate::publish(
            staging,
            &candidate.to_string_lossy(),
            journal
                .target
                .parent()
                .ok_or("Invalid output destination.")?,
        )?;
        Ok(Some((target, journal.item)))
    }
}

pub fn start(app: AppHandle, state: Arc<AppState>) {
    if state.runtime.started.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            if !state.runtime.updating.load(Ordering::SeqCst) {
                if let Some((job, receiver)) = reserve(&state) {
                    let handle = app.clone();
                    let shared = state.clone();
                    tauri::async_runtime::spawn(async move {
                        execute(handle, shared, job, receiver).await;
                    });
                    continue;
                }
            }
            tokio::select! { _=state.runtime.notify.notified()=>{}, _=tokio::time::sleep(std::time::Duration::from_millis(500))=>{} }
        }
    });
}

fn reserve(state: &AppState) -> Option<(Job, watch::Receiver<bool>)> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut()?;
    let mut active = state.runtime.active.lock().unwrap();
    if state.runtime.updating.load(Ordering::SeqCst) {
        return None;
    }
    let running: Vec<_> = repo
        .data
        .jobs
        .iter()
        .filter(|j| j.state.active() || active.contains_key(&j.id))
        .collect();
    let network = running.iter().filter(|j| j.source.network()).count();
    let local = running.iter().filter(|j| !j.source.network()).count();
    let next = repo.data.jobs.iter().position(|j| {
        j.state == JobState::Pending
            && !active.contains_key(&j.id)
            && j.next_retry_at.is_none_or(|time| time <= now())
            && if j.source.network() {
                network < repo.data.settings.network_limit
                    && !running
                        .iter()
                        .any(|a| a.source.network() && a.source.domain() == j.source.domain())
            } else {
                local < repo.data.settings.processing_limit
            }
    });
    let index = next?;
    let previous = repo.data.jobs[index].clone();
    let job = &mut repo.data.jobs[index];
    job.state = JobState::Analyzing;
    job.run_id = now().max(job.run_id + 1);
    job.attempt += 1;
    job.next_retry_at = None;
    job.updated_at = now();
    let selected = job.clone();
    if repo.save().is_err() {
        repo.data.jobs[index] = previous;
        return None;
    }
    let (sender, receiver) = watch::channel(false);
    active.insert(selected.id.clone(), sender);
    Some((selected, receiver))
}

async fn execute(app: AppHandle, state: Arc<AppState>, job: Job, cancel: watch::Receiver<bool>) {
    let initial = job.clone();
    let _ = app.emit("job-update", initial);
    let staging = stage(&job);
    let result = match &staging {
        Ok(staging) => run(&app, &state, &job, staging, cancel.clone()).await,
        Err(e) => Err(e.clone()),
    };
    let mut success = false;
    {
        let mut guard = state.repository.lock().unwrap();
        if let Some(repo) = guard.as_mut() {
            if let Some(index) = repo
                .data
                .jobs
                .iter()
                .position(|j| j.id == job.id && j.run_id == job.run_id)
            {
                let current = &mut repo.data.jobs[index];
                current.progress = None;
                current.updated_at = now();
                match result {
                    Ok((path, item)) => {
                        current.outputs = vec![path];
                        current.error = None;
                        current.state = JobState::Completed;
                        success = true;
                        if !repo.data.library.iter().any(|old| old.path == item.path) {
                            repo.data.library.push(item);
                        }
                    }
                    Err(ref e)
                        if e == engine::CANCELLED || current.state == JobState::Cancelled =>
                    {
                        current.state = JobState::Cancelled;
                        current.error = None;
                    }
                    Err(ref e) => {
                        let phase = if current.state == JobState::Verifying {
                            "verifying"
                        } else if current.state == JobState::Processing {
                            "processing"
                        } else {
                            "download"
                        };
                        let error = JobError::from(Failure::classify(e, phase));
                        if error.retryable
                            && current.attempt < 3
                            && current.state != JobState::Cancelled
                        {
                            let wait = error.retry_after.unwrap_or(2u64.pow(current.attempt) * 2)
                                * 1000
                                + current.run_id % 1000;
                            current.next_retry_at = Some(now().saturating_add(wait));
                            current.state = JobState::Pending;
                        } else if current.state != JobState::Cancelled {
                            current.state = JobState::Error;
                        }
                        current.error = Some(error);
                    }
                }
                let mut event = repo.data.jobs[index].clone();
                if let Err(e) = repo.save() {
                    event.state = JobState::Error;
                    event.error = Some(
                        Failure::classify(&format!("Could not save job state: {e}"), "persistence")
                            .into(),
                    );
                    repo.data.jobs[index] = event.clone();
                    success = false;
                }
                let _ = app.emit("job-update", event);
            }
        }
    }
    // Keep a publication journal after persistence failures so an explicit retry can adopt the final file.
    if let Ok(path) = staging {
        if success || (*cancel.borrow() && matches!(has_published_output(&job, &path), Ok(false))) {
            let _ = cleanup(&job, &path);
        }
    }
    if success {
        let _ = app.emit("library-update", ());
    }
    state.runtime.active.lock().unwrap().remove(&job.id);
    state.runtime.notify.notify_one();
}

async fn run(
    app: &AppHandle,
    state: &Arc<AppState>,
    job: &Job,
    staging: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<(PathBuf, library::LibraryItem), String> {
    publication_guard(state, job, &cancel)?;
    if let Some(recovered) = recover_publication(state, job, staging, cancel.clone()).await? {
        return Ok(recovered);
    }
    let root = if job.source.network() {
        engine::engine_dir(app)?
    } else {
        crate::dependencies::engine_root(app)?
    };
    let handle = app.clone();
    let shared = state.clone();
    let id = job.id.clone();
    let run_id = job.run_id;
    let emit = move |update| progress(&handle, &shared, &id, run_id, update);
    let (file, source, parent) = match &job.source {
        Source::Download {
            url_ref,
            format,
            height,
            audio_bitrate,
            ..
        } => {
            let cookie_root = {
                let guard = state.repository.lock().unwrap();
                guard
                    .as_ref()
                    .ok_or("Store unavailable.")?
                    .root()
                    .to_owned()
            };
            let operational = crate::sessions::resolve_url(
                &cookie_root,
                url_ref
                    .as_deref()
                    .ok_or("This download has no protected URL. Add its link again.")?,
            )?;
            let url = providers::validate_remote(operational.as_str()).await?;
            let cookies = crate::sessions::materialize(&cookie_root, &url)?;
            let mut args = engine::download_args(&root, *format, *height, staging, &url);
            if let Some(file) = &cookies {
                args.splice(
                    0..0,
                    ["--cookies".into(), file.path.to_string_lossy().into_owned()],
                );
            }
            if let Some(bitrate) = audio_bitrate {
                if let Some(i) = args.iter().position(|a| a == "--audio-quality") {
                    args[i + 1] = format!("{bitrate}K");
                }
            }
            let audio_permit = if !format.is_video() {
                Some(processing_permit(state, cancel.clone()).await?)
            } else {
                None
            };
            phase(app, state, &job.id, run_id, JobState::Downloading)?;
            let raw = engine::run_process(
                crate::updater::executable(app, &root)?,
                args,
                cancel.clone(),
                run_id,
                false,
                emit.clone(),
            )
            .await?;
            let file = crate::checked_output(staging, raw.trim())?;
            phase(app, state, &job.id, run_id, JobState::Processing)?;
            let _permit = if audio_permit.is_none() {
                Some(processing_permit(state, cancel.clone()).await?)
            } else {
                None
            };
            if *cancel.borrow() {
                return Err(engine::CANCELLED.into());
            }
            let file = media::prepare(&root, &file, *format, cancel.clone()).await?;
            (file, Some(url), None)
        }
        Source::Transform {
            library_id,
            options,
        } => {
            let input = {
                let guard = state.repository.lock().unwrap();
                library::authorized_path(
                    &guard
                        .as_ref()
                        .ok_or("Store unavailable.")?
                        .data
                        .library
                        .iter()
                        .find(|i| i.id == *library_id)
                        .ok_or("Library item not found.")?
                        .path,
                )?
            };
            let _permit = processing_permit(state, cancel.clone()).await?;
            if *cancel.borrow() {
                return Err(engine::CANCELLED.into());
            }
            phase(app, state, &job.id, run_id, JobState::Processing)?;
            let file = crate::tools::execute(
                &root,
                &input,
                staging,
                options,
                cancel.clone(),
                run_id,
                emit,
            )
            .await?;
            (file, None, Some(library_id.clone()))
        }
    };
    phase(app, state, &job.id, run_id, JobState::Verifying)?;
    let info = media::probe(&root, &file, cancel.clone()).await?;
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let file = crate::checked_output(staging, &file.to_string_lossy())?;
    let (destination, _) = stage_path(job)?;
    let target = destination.join(file.file_name().ok_or("Invalid output filename.")?);
    let item = target_item(
        library::index(&file, Some(&info), source, parent)?,
        target.clone(),
    );
    let hash = digest(file.clone(), cancel.clone()).await?;
    let journal = Publication {
        schema_version: 1,
        job_id: job.id.clone(),
        source_file: file.clone(),
        target: target.clone(),
        sha256: hash,
        item: item.clone(),
    };
    write_journal(job, staging, &journal)?;
    let guard = state.repository.lock().unwrap();
    let repo = guard.as_ref().ok_or("Store unavailable.")?;
    checked_run(repo, job, &cancel)?;
    let target = crate::publish(staging, &file.to_string_lossy(), &destination)?;
    Ok((target, item))
}

async fn processing_permit(
    state: &AppState,
    mut cancel: watch::Receiver<bool>,
) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    tokio::select! {
        biased;
        _=cancel.changed()=>Err(engine::CANCELLED.into()),
        permit=state.runtime.processing.clone().acquire_owned()=>permit.map_err(|_|"The processing queue stopped.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_aliases_share_the_network_limit() {
        let make = |url: &str| Source::Download {
            url: url.into(),
            url_ref: None,
            format: Format::Mp4,
            height: None,
            audio_bitrate: None,
        };
        assert_eq!(
            make("https://youtu.be/a").domain(),
            make("https://www.youtube.com/watch?v=a").domain()
        );
        assert_eq!(
            make("https://x.com/a").domain(),
            make("https://twitter.com/a").domain()
        );
        assert_ne!(
            make("https://media.example.org/a").domain(),
            make("https://other.example.org/a").domain()
        );
    }
    fn fixture() -> (PathBuf, Repository) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "nabnab-queue-{}-{}-{}",
            std::process::id(),
            now(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(root.join("media")).unwrap();
        let repo = Repository::open(&root, &root.join("media")).unwrap();
        (root, repo)
    }
    #[test]
    fn request_replay_and_recovery_are_safe() {
        let (root, mut repo) = fixture();
        let first = enqueue_download(
            &mut repo,
            vec!["https://youtube.com/watch?v=a".into()],
            vec![],
            Format::Mp4,
            None,
            None,
            "request-1",
        )
        .unwrap();
        let replay = enqueue_download(
            &mut repo,
            vec!["https://youtube.com/watch?v=a".into()],
            vec![],
            Format::Mp4,
            None,
            None,
            "request-1",
        )
        .unwrap();
        assert_eq!(first[0].id, replay[0].id);
        assert_eq!(repo.data.jobs.len(), 1);
        let folder = repo.data.settings.folder.clone();
        drop(repo);
        let again = Repository::open(&root, &folder).unwrap();
        assert_eq!(again.data.jobs[0].state, JobState::Paused);
        drop(again);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn enqueue_is_atomic_and_accepts_only_valid_public_web_urls() {
        let (root, mut repo) = fixture();
        assert!(enqueue_download(
            &mut repo,
            vec!["https://example.org/a".into(), "file:///secret".into()],
            vec![],
            Format::Mp4,
            None,
            None,
            "request-1"
        )
        .is_err());
        assert!(repo.data.jobs.is_empty());
        assert!(public_origin("https://example.org/a?token=secret").is_ok());
        assert_eq!(
            public_origin("https://youtube.com/watch?v=a&list=b").unwrap(),
            "https://youtube.com"
        );
        drop(repo);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_jobs_and_event_payloads_never_persist_operational_url_credentials() {
        let (root, mut repo) = fixture();
        let url = "https://example.org/PATH-SECRET-UNIQUE/media?api_key=API-SECRET-UNIQUE&accessToken=ACCESS-SECRET-UNIQUE&signature=SIGNATURE-SECRET-UNIQUE#FRAGMENT-SECRET-UNIQUE";
        let jobs = enqueue_download(
            &mut repo,
            vec![url.into()],
            vec![],
            Format::Mp4,
            None,
            None,
            "private-url",
        )
        .unwrap();
        let payload = serde_json::to_string(&jobs[0]).unwrap();
        let events =
            serde_json::to_string(&serde_json::json!({"event":"job-update", "payload":jobs[0]}))
                .unwrap();
        let saved = fs::read_to_string(root.join("state-v1.json")).unwrap();
        for secret in [
            "PATH-SECRET-UNIQUE",
            "API-SECRET-UNIQUE",
            "ACCESS-SECRET-UNIQUE",
            "SIGNATURE-SECRET-UNIQUE",
            "FRAGMENT-SECRET-UNIQUE",
        ] {
            assert!(!payload.contains(secret));
            assert!(!events.contains(secret));
            assert!(!saved.contains(secret));
        }
        let Source::Download {
            url: public,
            url_ref,
            ..
        } = &jobs[0].source
        else {
            panic!("expected a download");
        };
        assert_eq!(public, "https://example.org");
        let reference = url_ref.as_deref().unwrap();
        assert_eq!(
            crate::sessions::resolve_url(&root, reference)
                .unwrap()
                .as_str(),
            url
        );
        let replay = enqueue_download(
            &mut repo,
            vec![url.into()],
            vec![],
            Format::Mp4,
            None,
            None,
            "private-url",
        )
        .unwrap();
        assert_eq!(replay[0].id, jobs[0].id);
        assert_eq!(fs::read_dir(root.join("job-urls")).unwrap().count(), 1);
        assert!(!root.join("migration-backups").exists());
        drop(repo);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn only_active_or_pending_jobs_recover_paused() {
        let mut job = Job::new(
            Source::Download {
                url: "https://example.org/a".into(),
                url_ref: None,
                format: Format::Mp4,
                height: None,
                audio_bitrate: None,
            },
            "a".into(),
            PathBuf::from("C:/Downloads"),
            "x".into(),
            None,
        );
        for state in [
            JobState::Pending,
            JobState::Downloading,
            JobState::Processing,
            JobState::Verifying,
        ] {
            job.state = state;
            job.recover();
            assert_eq!(job.state, JobState::Paused);
        }
        for state in [JobState::Completed, JobState::Error, JobState::Cancelled] {
            job.state = state;
            job.recover();
            assert_eq!(job.state, state);
        }
    }

    fn state_fixture() -> (PathBuf, AppState, String) {
        let (root, mut repo) = fixture();
        let job = enqueue_download(
            &mut repo,
            vec!["https://youtube.com/watch?v=a".into()],
            vec![],
            Format::Mp4,
            None,
            None,
            "test-request",
        )
        .unwrap()
        .remove(0);
        let state = AppState::default();
        *state.repository.lock().unwrap() = Some(repo);
        (root, state, job.id)
    }
    #[test]
    fn cancellation_is_registered_before_reservation_returns_and_retry_waits_for_cleanup() {
        let (root, state, id) = state_fixture();
        let (first, cancel) = reserve(&state).unwrap();
        assert!(state.runtime.active.lock().unwrap().contains_key(&id));
        action(&state, &[id.clone()], "cancel").unwrap();
        assert!(*cancel.borrow());
        assert!(action(&state, &[id.clone()], "retry").is_err());
        assert!(reserve(&state).is_none());
        let staging = stage(&first).unwrap();
        fs::write(staging.join("partial.mp4"), b"owned partial").unwrap();
        cleanup(&first, &staging).unwrap();
        state.runtime.active.lock().unwrap().remove(&id);
        action(&state, &[id.clone()], "retry").unwrap();
        let (second, new_cancel) = reserve(&state).unwrap();
        assert!(second.run_id > first.run_id);
        assert!(!*new_cancel.borrow());
        assert!(*cancel.borrow());
        assert!(state.runtime.active.lock().unwrap().contains_key(&id));
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_batch_and_failed_save_leave_all_jobs_unchanged() {
        let (root, state, id) = state_fixture();
        assert!(action(&state, &[id.clone(), "missing-id".into()], "cancel").is_err());
        assert_eq!(
            state.repository.lock().unwrap().as_ref().unwrap().data.jobs[0].state,
            JobState::Pending
        );
        fs::rename(root.join("state-v1.json"), root.join("state-kept.json")).unwrap();
        fs::create_dir(root.join("state-v1.json")).unwrap();
        assert!(action(&state, &[id], "cancel").is_err());
        assert_eq!(
            state.repository.lock().unwrap().as_ref().unwrap().data.jobs[0].state,
            JobState::Pending
        );
        assert!(reserve(&state).is_none());
        assert!(state.runtime.active.lock().unwrap().is_empty());
        fs::remove_dir(root.join("state-v1.json")).unwrap();
        fs::rename(root.join("state-kept.json"), root.join("state-v1.json")).unwrap();
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn existing_unowned_directories_are_neither_reused_nor_removed() {
        let (root, state, _) = state_fixture();
        let (job, _) = reserve(&state).unwrap();
        let (_, staging) = stage_path(&job).unwrap();
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("user-file.txt"), b"keep me").unwrap();
        assert!(stage(&job).is_err());
        assert!(cleanup(&job, &staging).is_err());
        assert_eq!(fs::read(staging.join("user-file.txt")).unwrap(), b"keep me");
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reservations_obey_limits_and_do_not_start_while_engines_update() {
        let (root, state, _) = state_fixture();
        {
            let mut guard = state.repository.lock().unwrap();
            let repo = guard.as_mut().unwrap();
            enqueue_download(
                repo,
                vec![
                    "https://youtube.com/watch?v=b".into(),
                    "https://vimeo.com/1234".into(),
                ],
                vec![],
                Format::Mp4,
                None,
                None,
                "second-request",
            )
            .unwrap();
        }
        state.runtime.updating.store(true, Ordering::SeqCst);
        assert!(reserve(&state).is_none());
        assert!(state.runtime.active.lock().unwrap().is_empty());
        state.runtime.updating.store(false, Ordering::SeqCst);
        let (first, _) = reserve(&state).unwrap();
        let (second, _) = reserve(&state).unwrap();
        assert_ne!(first.source.domain(), second.source.domain());
        assert!(reserve(&state).is_none());
        assert_eq!(state.runtime.active.lock().unwrap().len(), 2);
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn namespaced_browser_request_ids_fit_without_sharing_clients() {
        let external = "r".repeat(128);
        assert!(validate_request_id(&format!("{}:{external}", "a".repeat(32))).is_ok());
        assert!(validate_request_id(&"r".repeat(193)).is_err());
    }
    fn journal_fixture(job: &Job, cancel: &watch::Receiver<bool>) -> (PathBuf, Publication) {
        let staging = stage(job).unwrap();
        let file = staging.join("Result ñ.mp4");
        fs::write(&file, b"verified fixture bytes").unwrap();
        let (destination, _) = stage_path(job).unwrap();
        let target = destination.join("Result ñ.mp4");
        let item = target_item(
            library::index(&file, None, None, None).unwrap(),
            target.clone(),
        );
        let journal = Publication {
            schema_version: 1,
            job_id: job.id.clone(),
            source_file: file.clone(),
            target,
            sha256: file_hash(&file, cancel).unwrap(),
            item,
        };
        write_journal(job, &staging, &journal).unwrap();
        (staging, journal)
    }

    fn transform_fixture() -> (PathBuf, AppState, String, PathBuf) {
        let (root, mut repo) = fixture();
        let input = root.join("media").join("original.mp4");
        fs::write(&input, b"unchanged original fixture").unwrap();
        library::import_paths(&mut repo, &[input.clone()]).unwrap();
        let id = repo.data.library[0].id.clone();
        let job = enqueue_tool(
            &mut repo,
            vec![id],
            ToolOptions::ExtractAudio {
                format: "mp3".into(),
            },
            "transform-retry",
        )
        .unwrap()
        .remove(0);
        let state = AppState::default();
        *state.repository.lock().unwrap() = Some(repo);
        (root, state, job.id, input)
    }

    #[test]
    fn failed_transform_retries_archive_partials_and_create_a_fresh_owned_stage() {
        let (root, state, id, input) = transform_fixture();
        let (first, _) = reserve(&state).unwrap();
        let staging = stage(&first).unwrap();
        fs::write(
            staging.join("original.audio.mp3"),
            b"incomplete first output",
        )
        .unwrap();
        state.repository.lock().unwrap().as_mut().unwrap().data.jobs[0].state = JobState::Error;
        state.runtime.active.lock().unwrap().remove(&id);
        action(&state, &[id], "retry").unwrap();
        let (retry, _) = reserve(&state).unwrap();
        let fresh = stage(&retry).unwrap();
        assert_eq!(fresh, staging);
        assert!(!fresh.join("original.audio.mp3").exists());
        let archive = first
            .folder
            .join(format!(".nabnab-{}-interrupted-{}", first.id, first.run_id));
        assert_eq!(
            fs::read(archive.join("original.audio.mp3")).unwrap(),
            b"incomplete first output"
        );
        let owner: Owner = read_owned_json(&fresh.join(OWNER), 4096).unwrap();
        assert_eq!(owner.run_id, retry.run_id);
        assert_eq!(fs::read(input).unwrap(), b"unchanged original fixture");
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn crashed_transform_restarts_fresh_while_download_retries_keep_resume_files() {
        let (root, state, id, input) = transform_fixture();
        let (first, _) = reserve(&state).unwrap();
        let staging = stage(&first).unwrap();
        fs::write(staging.join("partial.mp3"), b"interrupted conversion").unwrap();
        drop(state);
        let again = AppState::default();
        *again.repository.lock().unwrap() = Some(Repository::open(&root, &first.folder).unwrap());
        action(&again, &[id], "retry").unwrap();
        let (retry, _) = reserve(&again).unwrap();
        let fresh = stage(&retry).unwrap();
        assert!(!fresh.join("partial.mp3").exists());
        let archive = first
            .folder
            .join(format!(".nabnab-{}-interrupted-{}", first.id, first.run_id));
        assert_eq!(
            fs::read(archive.join("partial.mp3")).unwrap(),
            b"interrupted conversion"
        );
        assert_eq!(fs::read(input).unwrap(), b"unchanged original fixture");
        drop(again);
        fs::remove_dir_all(root).unwrap();

        let (root, state, id) = state_fixture();
        let (first, _) = reserve(&state).unwrap();
        let staging = stage(&first).unwrap();
        fs::write(staging.join("download.part"), b"resumable download").unwrap();
        state.repository.lock().unwrap().as_mut().unwrap().data.jobs[0].state = JobState::Error;
        state.runtime.active.lock().unwrap().remove(&id);
        action(&state, &[id], "retry").unwrap();
        let (retry, _) = reserve(&state).unwrap();
        assert_eq!(stage(&retry).unwrap(), staging);
        assert_eq!(
            fs::read(staging.join("download.part")).unwrap(),
            b"resumable download"
        );
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn journaled_transform_retries_adopt_the_result_without_archiving_its_record() {
        let (root, state, id, input) = transform_fixture();
        let (first, cancel) = reserve(&state).unwrap();
        let (staging, journal) = journal_fixture(&first, &cancel);
        crate::publish(
            &staging,
            &journal.source_file.to_string_lossy(),
            journal.target.parent().unwrap(),
        )
        .unwrap();
        let record = fs::read(staging.join(JOURNAL)).unwrap();
        drop(state);
        drop(cancel);
        let again = AppState::default();
        *again.repository.lock().unwrap() = Some(Repository::open(&root, &first.folder).unwrap());
        action(&again, &[id], "retry").unwrap();
        let (retry, cancel) = reserve(&again).unwrap();
        assert_eq!(stage(&retry).unwrap(), staging);
        assert_eq!(fs::read(staging.join(JOURNAL)).unwrap(), record);
        let result = recover_publication(&again, &retry, &staging, cancel)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.0, journal.target);
        assert_eq!(fs::read(result.0).unwrap(), b"verified fixture bytes");
        assert_eq!(fs::read(input).unwrap(), b"unchanged original fixture");
        assert!(!first
            .folder
            .join(format!(".nabnab-{}-interrupted-{}", first.id, first.run_id))
            .exists());
        drop(again);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn crash_after_publication_recovers_owned_output_without_redownloading_or_replacing() {
        let (root, state, id) = state_fixture();
        let (job, cancel) = reserve(&state).unwrap();
        let (staging, journal) = journal_fixture(&job, &cancel);
        crate::publish(
            &staging,
            &journal.source_file.to_string_lossy(),
            journal.target.parent().unwrap(),
        )
        .unwrap();
        let folder = job.folder.clone();
        drop(state);
        drop(cancel);
        let again = AppState::default();
        *again.repository.lock().unwrap() = Some(Repository::open(&root, &folder).unwrap());
        assert_eq!(
            again.repository.lock().unwrap().as_ref().unwrap().data.jobs[0].state,
            JobState::Paused
        );
        action(&again, &[id], "retry").unwrap();
        let (retry, cancel) = reserve(&again).unwrap();
        let (output, item) = recover_publication(&again, &retry, &staging, cancel)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output, journal.target);
        assert_eq!(fs::read(&output).unwrap(), b"verified fixture bytes");
        assert_eq!(item.path, output);
        assert_eq!(
            item.id,
            library::index(&output, None, None, None).unwrap().id
        );
        cleanup(&retry, &staging).unwrap();
        assert!(output.is_file());
        drop(again);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn journal_recovery_refuses_changed_targets_and_stale_or_cancelled_runs() {
        let (root, state, _) = state_fixture();
        let (job, cancel) = reserve(&state).unwrap();
        let (staging, journal) = journal_fixture(&job, &cancel);
        {
            let mut guard = state.repository.lock().unwrap();
            guard.as_mut().unwrap().data.jobs[0].run_id += 1;
        }
        assert_eq!(
            recover_publication(&state, &job, &staging, cancel.clone())
                .await
                .unwrap_err(),
            engine::CANCELLED
        );
        assert!(!journal.target.exists());
        assert!(journal.source_file.is_file());
        {
            let mut guard = state.repository.lock().unwrap();
            let current = &mut guard.as_mut().unwrap().data.jobs[0];
            current.run_id = job.run_id;
            current.state = JobState::Cancelled;
        }
        assert_eq!(
            recover_publication(&state, &job, &staging, cancel.clone())
                .await
                .unwrap_err(),
            engine::CANCELLED
        );
        assert!(!journal.target.exists());
        {
            let mut guard = state.repository.lock().unwrap();
            guard.as_mut().unwrap().data.jobs[0].state = JobState::Analyzing;
        }
        fs::write(&journal.target, b"different user file").unwrap();
        assert!(recover_publication(&state, &job, &staging, cancel)
            .await
            .is_err());
        assert_eq!(fs::read(&journal.target).unwrap(), b"different user file");
        assert!(staging.join(JOURNAL).exists());
        drop(state);
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn a_waiting_conversion_can_be_cancelled_before_a_permit_is_available() {
        let state = Arc::new(AppState::default());
        let held = state
            .runtime
            .processing
            .clone()
            .acquire_owned()
            .await
            .unwrap();
        let (sender, cancel) = watch::channel(false);
        let shared = state.clone();
        let pending = tokio::spawn(async move { processing_permit(&shared, cancel).await });
        sender.send(true).unwrap();
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(outcome.unwrap_err(), engine::CANCELLED);
        drop(held);
        assert_eq!(state.runtime.processing.available_permits(), 1);
    }
}
