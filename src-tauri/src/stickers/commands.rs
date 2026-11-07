use super::{Details, Options, Selection};
use crate::{file_operations, file_sources, queue, workspaces::Workspace, AppState};
use serde::Serialize;
use std::{
    fs,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, State};

#[derive(Default)]
pub struct PreviewState(Mutex<Option<Approval>>);

struct Approval {
    request_id: u64,
    expires: u64,
    selection: Selection,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    #[serde(flatten)]
    details: Details,
    data: Vec<u8>,
    preview_id: u64,
}

#[tauri::command]
pub async fn sticker_preview(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    library_id: String,
    options: Options,
    request_id: u64,
) -> Result<Preview, String> {
    options.validate()?;
    let (root, library) = file_operations::context(&state)?;
    let input = file_operations::selected(&library, &[library_id.clone()], 1)?.remove(0);
    super::demuxer(&input.path)?;
    let engines = crate::dependencies::engine_root(&app)?;
    let workspace = Workspace::new(&root)?;
    let cancel = file_sources::begin(&state, request_id)?;
    let result = async {
        let _permit = queue::processing_permit(&state, cancel.clone()).await?;
        let input_hash = queue::digest(input.path.clone(), cancel.clone()).await?;
        let artifact = super::encode(
            &engines,
            &input.path,
            &workspace.path,
            &options,
            cancel.clone(),
            request_id,
        )
        .await?;
        if queue::digest(input.path, cancel.clone()).await? != input_hash {
            return Err(
                "The selected file changed while generating the sticker. Preview it again.".into(),
            );
        }
        if *cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        let data = fs::read(&artifact.path).map_err(|error| error.to_string())?;
        *state.sticker_preview.0.lock().unwrap() = Some(Approval {
            request_id,
            expires: queue::now() + 600_000,
            selection: Selection {
                library_id,
                input_hash,
                options,
            },
        });
        Ok(Preview {
            details: artifact.details,
            data,
            preview_id: request_id,
        })
    }
    .await;
    file_sources::finish(&state, request_id);
    result
}

#[tauri::command]
pub fn queue_sticker(
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
    let input = file_operations::selected(&repo.data.library, &[library_id.clone()], 1)?.remove(0);
    let mut approval = state.sticker_preview.0.lock().unwrap();
    let approved = approval
        .as_ref()
        .filter(|item| {
            item.request_id == preview_id
                && item.expires > queue::now()
                && item.selection.library_id == library_id
                && item.selection.options == options
        })
        .ok_or("Preview these sticker settings again before adding the export to the queue.")?;
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::Sticker {
                selection: approved.selection.clone(),
            },
            format!("Sticker · {}", input.name),
        )],
        &client_request,
    )?;
    *approval = None;
    state.runtime.notify.notify_one();
    Ok(jobs)
}
