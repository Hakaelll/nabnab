use super::{Draft, Edit, Export, Input, Selection};
use crate::{captions::Cue, library::LibraryItem, queue, store::Repository, AppState};
use serde::Serialize;
use std::{path::PathBuf, sync::Arc};
use tauri::{AppHandle, State};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePreview {
    source_hash: String,
    count: usize,
    cues: Vec<Cue>,
}

pub fn input_item(repo: &Repository, input: &Input) -> Result<LibraryItem, String> {
    repo.data
        .library
        .iter()
        .find(|item| {
            item.id == input.library_id
                && item
                    .details
                    .tracks
                    .iter()
                    .any(|track| track.id == input.track_id)
        })
        .cloned()
        .ok_or_else(|| "Choose a local subtitle track from Library.".into())
}

pub fn completed_root(repo: &Repository, id: &str) -> Result<PathBuf, String> {
    if !super::valid_id(id)
        || !repo.data.jobs.iter().any(|job| {
            job.id == id
                && job.state == queue::JobState::Completed
                && matches!(job.source, queue::Source::CaptionTranslation { .. })
        })
    {
        return Err("Wait for the subtitle translation to complete.".into());
    }
    Ok(repo.root().to_owned())
}

#[tauri::command]
pub async fn translation_source(
    state: State<'_, Arc<AppState>>,
    input: Input,
) -> Result<SourcePreview, String> {
    let item = {
        let guard = state.repository.lock().unwrap();
        input_item(guard.as_ref().ok_or("Open nabnab first.")?, &input)?
    };
    tokio::task::spawn_blocking(move || {
        let (cues, source_hash) = super::source(&item, &input)?;
        Ok(SourcePreview {
            count: cues.len(),
            source_hash,
            cues: cues.into_iter().take(4).collect(),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn queue_translation(
    app: AppHandle,
    state: State<Arc<AppState>>,
    selection: Selection,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    selection.validate()?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let previous: Vec<_> = repo
        .data
        .jobs
        .iter()
        .filter(|job| job.client_request == client_request)
        .cloned()
        .collect();
    if !previous.is_empty() {
        return Ok(previous);
    }
    selection.pair.enabled(&crate::components::root(&app)?)?;
    crate::dependencies::engine_root(&app)?;
    let item = input_item(repo, &selection.input)?;
    let (_, to) = selection.pair.languages();
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::CaptionTranslation { selection },
            format!("{} · {to}", item.name),
        )],
        &client_request,
    )?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}

#[tauri::command]
pub async fn translation_drafts(state: State<'_, Arc<AppState>>) -> Result<Vec<String>, String> {
    let (root, ids) = {
        let guard = state.repository.lock().unwrap();
        let repo = guard.as_ref().ok_or("Open nabnab first.")?;
        let ids = repo
            .data
            .jobs
            .iter()
            .rev()
            .filter(|job| {
                job.state == queue::JobState::Completed
                    && matches!(job.source, queue::Source::CaptionTranslation { .. })
            })
            .map(|job| job.id.clone())
            .collect::<Vec<_>>();
        (repo.root().to_owned(), ids)
    };
    tokio::task::spawn_blocking(move || {
        let directory = super::directory(&root)?;
        Ok(ids
            .into_iter()
            .filter(|id| super::valid_id(id) && directory.join(format!("{id}.json")).is_file())
            .collect())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn translation_read(
    state: State<'_, Arc<AppState>>,
    draft_id: String,
) -> Result<Draft, String> {
    let root = {
        let guard = state.repository.lock().unwrap();
        completed_root(guard.as_ref().ok_or("Open nabnab first.")?, &draft_id)?
    };
    tokio::task::spawn_blocking(move || super::read(&root, &draft_id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn translation_edit(
    state: State<'_, Arc<AppState>>,
    draft_id: String,
    revision: u64,
    edits: Vec<Edit>,
) -> Result<Draft, String> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let guard = state.repository.lock().unwrap();
        let root = completed_root(guard.as_ref().ok_or("Open nabnab first.")?, &draft_id)?;
        let mut draft = super::read(&root, &draft_id)?;
        super::edit(&mut draft, revision, edits)?;
        if draft.revision != revision {
            super::write(&root, &draft)?;
        }
        Ok(draft)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn translation_remove(state: State<Arc<AppState>>, draft_id: String) -> Result<(), String> {
    let guard = state.repository.lock().unwrap();
    let repo = guard.as_ref().ok_or("Open nabnab first.")?;
    let root = completed_root(repo, &draft_id)?;
    let in_use = repo.data.jobs.iter().any(|job| {
        (job.state.active() || matches!(job.state, queue::JobState::Pending | queue::JobState::Paused))
            && matches!(&job.source, queue::Source::CaptionTranslationExport { options } if options.draft_id == draft_id)
    });
    if in_use {
        return Err("Wait for exports using this translation to finish.".into());
    }
    super::remove(&root, &draft_id)
}

#[tauri::command]
pub fn queue_translation_export(
    state: State<Arc<AppState>>,
    options: Export,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let previous: Vec<_> = repo
        .data
        .jobs
        .iter()
        .filter(|job| job.client_request == client_request)
        .cloned()
        .collect();
    if !previous.is_empty() {
        return Ok(previous);
    }
    let root = completed_root(repo, &options.draft_id)?;
    let draft = super::read(&root, &options.draft_id)?;
    if draft.revision != options.revision {
        return Err("Reload the current translation before exporting it.".into());
    }
    let (_, to) = draft.selection.pair.languages();
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::CaptionTranslationExport { options },
            format!("{} · {to}", draft.name),
        )],
        &client_request,
    )?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}
