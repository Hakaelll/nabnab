use super::{Options, Sample, Selection};
use crate::{file_operations, file_sources, queue, workspaces::Workspace, AppState};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State};
use tokio::sync::{watch, Semaphore, SemaphorePermit};

pub struct BackgroundState {
    access: Semaphore,
    preview: Mutex<Option<Approval>>,
}
impl Default for BackgroundState {
    fn default() -> Self {
        Self {
            access: Semaphore::new(1),
            preview: Mutex::new(None),
        }
    }
}

impl BackgroundState {
    pub fn component_access(&self) -> Result<SemaphorePermit<'_>, String> {
        self.access
            .try_acquire()
            .map_err(|_| "Wait for background removal or installation to finish.".into())
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
            permit = self.access.acquire() => permit.map_err(|_| "The segmentation engine is unavailable.".into()),
        }
    }
}

struct Approval {
    id: u64,
    expires: u64,
    selection: Selection,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    #[serde(flatten)]
    sample: Sample,
    preview_id: u64,
}

#[tauri::command]
pub async fn background_preview(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    library_id: String,
    options: Options,
    request_id: u64,
) -> Result<Preview, String> {
    options.validate()?;
    let (root, library) = file_operations::context(&state)?;
    let input =
        file_operations::selected(&library, std::slice::from_ref(&library_id), 1)?.remove(0);
    if input.kind != "image" {
        return Err("Choose a still image from Library.".into());
    }
    let engines = crate::dependencies::engine_root(&app)?;
    let workspace = Workspace::new(&root)?;
    let cancel = file_sources::begin(&state, request_id)?;
    let result = async {
        let _slot = state.background.permit(cancel.clone()).await?;
        let _processing = queue::processing_permit(&state, cancel.clone()).await?;
        let model = super::component::enabled_model(&crate::components::root(&app)?)?;
        let input_hash = queue::digest(input.path.clone(), cancel.clone()).await?;
        let sample = super::sample(
            &engines,
            &model,
            &workspace.path,
            &input.path,
            options,
            cancel.clone(),
            request_id,
        )
        .await?;
        if queue::digest(input.path, cancel.clone()).await? != input_hash {
            return Err(
                "The image changed while preparing the foreground. Preview it again.".into(),
            );
        }
        if *cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        *state.background.preview.lock().unwrap() = Some(Approval {
            id: request_id,
            expires: queue::now() + 600_000,
            selection: Selection {
                library_id,
                input_hash,
                options,
            },
        });
        Ok(Preview {
            sample,
            preview_id: request_id,
        })
    }
    .await;
    file_sources::finish(&state, request_id);
    result
}

#[tauri::command]
pub fn queue_background(
    app: AppHandle,
    state: State<Arc<AppState>>,
    library_id: String,
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
    let input =
        file_operations::selected(&repo.data.library, std::slice::from_ref(&library_id), 1)?
            .remove(0);
    let mut preview = state.background.preview.lock().unwrap();
    let approved = preview
        .as_ref()
        .filter(|approval| {
            approval.id == preview_id
                && approval.expires > queue::now()
                && approval.selection.library_id == library_id
                && approval.selection.options == options
        })
        .ok_or("Preview the current background settings before adding the export to the queue.")?;
    super::component::enabled_model(&crate::components::root(&app)?)?;
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::Background {
                selection: approved.selection.clone(),
            },
            format!("Foreground · {}", input.name),
        )],
        &client_request,
    )?;
    *preview = None;
    state.runtime.notify.notify_one();
    Ok(jobs)
}
