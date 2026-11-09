use super::{Anchor, Dimensions, Options, Sample, Selection, MAX_BATCH};
use crate::{
    file_operations, file_sources, library::LibraryItem, queue, workspaces::Workspace, AppState,
};
use serde::Serialize;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, State};
use tokio::sync::{watch, Semaphore, SemaphorePermit};

pub struct UpscaleState {
    access: Semaphore,
    preview: Mutex<Option<Approval>>,
}

impl Default for UpscaleState {
    fn default() -> Self {
        Self {
            access: Semaphore::new(1),
            preview: Mutex::new(None),
        }
    }
}

impl UpscaleState {
    pub fn component_access(&self) -> Result<SemaphorePermit<'_>, String> {
        self.access
            .try_acquire()
            .map_err(|_| "Wait for the active upscale or installation to finish.".into())
    }

    pub async fn permit(
        &self,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<SemaphorePermit<'_>, String> {
        if *cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        tokio::select! {
            biased;
            _ = cancel.changed() => Err(crate::engine::CANCELLED.into()),
            permit = self.access.acquire() => permit.map_err(|_| "The upscale engine is unavailable.".into()),
        }
    }
}

struct Approval {
    id: u64,
    expires: u64,
    selections: Vec<Selection>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    #[serde(flatten)]
    sample: Sample,
    preview_id: u64,
    batch: Vec<BatchImage>,
}

#[derive(Serialize)]
pub struct BatchImage {
    id: String,
    #[serde(flatten)]
    dimensions: Dimensions,
}

async fn inspect_image(
    engines: &Path,
    input: &LibraryItem,
    options: Options,
    cancel: watch::Receiver<bool>,
) -> Result<BatchImage, String> {
    if input.kind != "image" {
        return Err(format!(
            "{}: Choose a still image from Library.",
            input.name
        ));
    }
    let dimensions = super::inspect(engines, &input.path, options, cancel)
        .await
        .map_err(|error| {
            if error == crate::engine::CANCELLED {
                error
            } else {
                format!("{}: {error}", input.name)
            }
        })?;
    Ok(BatchImage {
        id: input.id.clone(),
        dimensions,
    })
}

#[tauri::command]
pub async fn upscale_inspect(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    options: Options,
    request_id: u64,
) -> Result<Vec<BatchImage>, String> {
    options.validate()?;
    let (_, library) = file_operations::context(&state)?;
    let inputs = file_operations::selected(&library, &ids, MAX_BATCH)?;
    let engines = crate::dependencies::engine_root(&app)?;
    let cancel = file_sources::begin(&state, request_id)?;
    let result = async {
        let _processing = queue::processing_permit(&state, cancel.clone()).await?;
        let mut batch = Vec::with_capacity(inputs.len());
        for input in &inputs {
            batch.push(inspect_image(&engines, input, options, cancel.clone()).await?);
        }
        Ok(batch)
    }
    .await;
    file_sources::finish(&state, request_id);
    result
}

#[tauri::command]
pub async fn upscale_preview(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    sample_id: String,
    options: Options,
    anchor: Anchor,
    request_id: u64,
) -> Result<Preview, String> {
    options.validate()?;
    let (root, library) = file_operations::context(&state)?;
    let inputs = file_operations::selected(&library, &ids, MAX_BATCH)?;
    let position = ids
        .iter()
        .position(|id| id == &sample_id)
        .ok_or("Choose an image from the current batch to preview.")?;
    let engines = crate::dependencies::engine_root(&app)?;
    let workspace = Workspace::new(&root)?;
    let cancel = file_sources::begin(&state, request_id)?;
    let result = async {
        let _gpu = state.upscale.permit(cancel.clone()).await?;
        let _processing = queue::processing_permit(&state, cancel.clone()).await?;
        let runtime =
            super::component::ready(&crate::components::root(&app)?, &options.model_files())?;
        let mut batch = Vec::with_capacity(inputs.len());
        let mut selections = Vec::with_capacity(inputs.len());
        for input in &inputs {
            batch.push(inspect_image(&engines, input, options, cancel.clone()).await?);
            let input_hash = queue::digest(input.path.clone(), cancel.clone()).await?;
            selections.push(Selection {
                library_id: input.id.clone(),
                input_hash,
                options,
            });
        }
        let input = &inputs[position].path;
        let sample = super::sample(
            &engines,
            &runtime,
            &workspace.path,
            input,
            &batch[position].dimensions,
            options,
            anchor,
            cancel.clone(),
            request_id,
        )
        .await?;
        if queue::digest(input.clone(), cancel.clone()).await? != selections[position].input_hash {
            return Err("The sample image changed during preview. Review it again.".into());
        }
        if *cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        *state.upscale.preview.lock().unwrap() = Some(Approval {
            id: request_id,
            expires: queue::now() + 600_000,
            selections,
        });
        Ok(Preview {
            sample,
            preview_id: request_id,
            batch,
        })
    }
    .await;
    file_sources::finish(&state, request_id);
    result
}

#[tauri::command]
pub fn queue_upscale(
    app: AppHandle,
    state: State<Arc<AppState>>,
    ids: Vec<String>,
    options: Options,
    preview_id: u64,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    options.validate()?;
    crate::dependencies::engine_root(&app)?;
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
    let inputs = file_operations::selected(&repo.data.library, &ids, MAX_BATCH)?;
    let mut preview = state.upscale.preview.lock().unwrap();
    let approved = preview
        .as_ref()
        .filter(|approval| {
            approval.id == preview_id
                && approval.expires > queue::now()
                && approval.selections.len() == ids.len()
                && approval.selections.iter().zip(&ids).all(|(selection, id)| {
                    &selection.library_id == id && selection.options == options
                })
        })
        .ok_or("Generate a new upscale preview for this batch and its current settings.")?;
    let status = super::component::status(&crate::components::root(&app)?);
    if !status.installed || !status.enabled {
        return Err("Install and enable Real-ESRGAN before starting the batch.".into());
    }
    let jobs = approved
        .selections
        .iter()
        .zip(inputs)
        .map(|(selection, item)| {
            (
                queue::Source::Upscale {
                    selection: selection.clone(),
                },
                format!("{} · {}×", item.name, options.scale),
            )
        })
        .collect();
    let queued = queue::enqueue_tasks(repo, jobs, &client_request)?;
    *preview = None;
    state.runtime.notify.notify_one();
    Ok(queued)
}
