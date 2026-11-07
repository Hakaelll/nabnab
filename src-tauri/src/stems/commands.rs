use super::{component, Selection};
use crate::{
    engine, file_operations, file_sources, library, queue, workspaces::Workspace, AppState,
};
use serde::Serialize;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{ipc::Response, AppHandle, State};
use tokio::sync::{watch, Semaphore, SemaphorePermit};

pub struct StemsState {
    access: Semaphore,
    preview: Mutex<Option<Prepared>>,
}

impl Default for StemsState {
    fn default() -> Self {
        Self {
            access: Semaphore::new(1),
            preview: Mutex::new(None),
        }
    }
}

impl StemsState {
    pub fn component_access(&self) -> Result<SemaphorePermit<'_>, String> {
        self.access
            .try_acquire()
            .map_err(|_| "Wait for the audio separation or installation to finish.".into())
    }

    pub async fn permit(
        &self,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<SemaphorePermit<'_>, String> {
        if *cancel.borrow() {
            return Err(engine::CANCELLED.into());
        }
        tokio::select! {
            biased;
            _ = cancel.changed() => Err(engine::CANCELLED.into()),
            permit = self.access.acquire() => permit.map_err(|_| "The separation engine is unavailable.".into()),
        }
    }
}

struct Prepared {
    id: u64,
    expires: u64,
    selection: Selection,
    samples: Vec<PathBuf>,
    _workspace: Workspace,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    preview_id: u64,
    seconds: f64,
    channels: u8,
    original_channels: u64,
}

pub fn selected(items: &[library::LibraryItem], id: &str) -> Result<library::LibraryItem, String> {
    let input = file_operations::selected(items, &[id.to_owned()], 1)?.remove(0);
    if !matches!(input.kind.as_str(), "audio" | "video") {
        return Err("Choose an audio or video file from Library.".into());
    }
    Ok(input)
}

#[tauri::command]
pub async fn stems_preview(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    library_id: String,
    start: f64,
    request_id: u64,
) -> Result<Preview, String> {
    let (root, items) = file_operations::context(&state)?;
    let input = selected(&items, &library_id)?;
    let engines = crate::dependencies::engine_root(&app)?;
    let cancel = file_sources::begin(&state, request_id)?;
    let result = async {
        let _slot = state.stems.permit(cancel.clone()).await?;
        let _processing = queue::processing_permit(&state, cancel.clone()).await?;
        state.stems.preview.lock().unwrap().take();
        let workspace = Workspace::new(&root)?;
        let runtime = component::enabled(&crate::components::root(&app)?)?;
        let input_hash = queue::digest(input.path.clone(), cancel.clone()).await?;
        let audio = super::separate(
            &engines,
            &runtime,
            &workspace.path,
            &workspace.path,
            &input.path,
            Some(start),
            cancel.clone(),
        )
        .await?;
        if queue::digest(input.path, cancel.clone()).await? != input_hash {
            return Err("The audio changed while preparing the sample. Preview it again.".into());
        }
        let mut preview = state.stems.preview.lock().unwrap();
        if *cancel.borrow() {
            return Err(engine::CANCELLED.into());
        }
        *preview = Some(Prepared {
            id: request_id,
            expires: queue::now() + 600_000,
            selection: Selection {
                library_id,
                input_hash,
            },
            samples: audio.original.into_iter().chain(audio.tracks).collect(),
            _workspace: workspace,
        });
        Ok(Preview {
            preview_id: request_id,
            seconds: audio.seconds,
            channels: audio.channels,
            original_channels: audio.original_channels,
        })
    }
    .await;
    file_sources::finish(&state, request_id);
    result
}

#[tauri::command]
pub fn stems_sample(
    state: State<Arc<AppState>>,
    preview_id: u64,
    track: usize,
) -> Result<Response, String> {
    let preview = state.stems.preview.lock().unwrap();
    let prepared = preview
        .as_ref()
        .filter(|value| value.id == preview_id && value.expires > queue::now())
        .ok_or("Generate a new audio sample.")?;
    let path = prepared.samples.get(track).ok_or("Invalid sample track.")?;
    let path = library::authorized_path(path)?;
    if crate::portable_zip::regular_file(&path)?
        > (super::SAMPLE_SECONDS * super::SAMPLE_RATE * 4 + 512) as u64
    {
        return Err("The audio sample is too large.".into());
    }
    Ok(Response::new(
        fs::read(path).map_err(|error| error.to_string())?,
    ))
}

#[tauri::command]
pub fn stems_release(state: State<Arc<AppState>>, preview_id: u64) {
    let mut preview = state.stems.preview.lock().unwrap();
    if preview.as_ref().is_some_and(|value| value.id == preview_id) {
        preview.take();
    }
}

#[tauri::command]
pub fn queue_stems(
    app: AppHandle,
    state: State<Arc<AppState>>,
    preview_id: u64,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let previous = repo
        .data
        .jobs
        .iter()
        .filter(|job| job.client_request == client_request)
        .cloned()
        .collect::<Vec<_>>();
    if !previous.is_empty() {
        return Ok(previous);
    }
    let preview = state.stems.preview.lock().unwrap();
    let prepared = preview
        .as_ref()
        .filter(|value| value.id == preview_id && value.expires > queue::now())
        .ok_or("Generate a new sample before adding the separation to the queue.")?;
    let input = selected(&repo.data.library, &prepared.selection.library_id)?;
    component::enabled(&crate::components::root(&app)?)?;
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::Stems {
                selection: prepared.selection.clone(),
            },
            format!("Stems · {}", input.name),
        )],
        &client_request,
    )?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}
