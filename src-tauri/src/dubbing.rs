use crate::{engine, library, queue, store::Repository, translation, voice, AppState};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, State};
use tokio::sync::watch;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub draft_id: String,
    pub revision: u64,
    pub settings: voice::VoiceSettings,
}

impl Selection {
    pub fn validate(&self) -> Result<(), String> {
        if !translation::valid_id(&self.draft_id) || self.revision == 0 {
            return Err("Choose a saved translation revision.".into());
        }
        voice::timed::validate(&self.settings)
    }

    pub fn draft(&self, root: &Path) -> Result<translation::Draft, String> {
        self.validate()?;
        let draft = translation::read(root, &self.draft_id)?;
        if draft.revision != self.revision {
            return Err("The translation was edited. Review the current revision and create its voice again.".into());
        }
        Ok(draft)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum VideoMode {
    Alternate,
    Replace,
    Mix,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoOptions {
    pub voice_job_id: String,
    pub video_id: String,
    pub mode: VideoMode,
}

impl VideoOptions {
    pub fn validate(&self) -> Result<(), String> {
        if !translation::valid_id(&self.voice_job_id)
            || self.video_id.len() != 64
            || !self.video_id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Choose a finished voice job and a local video.".into());
        }
        Ok(())
    }
}

#[tauri::command]
pub async fn dub_sample(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    selection: Selection,
    index: usize,
    request_id: u64,
) -> Result<voice::Sample, String> {
    crate::voice_operations::require_enabled(&app)?;
    let root = {
        let guard = state.repository.lock().unwrap();
        translation::commands::completed_root(
            guard.as_ref().ok_or("Open nabnab first.")?,
            &selection.draft_id,
        )?
    };
    let cancel = crate::file_sources::begin(&state, request_id)?;
    let result = match selection.draft(&root) {
        Ok(draft) => {
            voice::timed::sample(
                &root,
                &draft,
                index,
                &selection.settings,
                cancel,
                request_id,
            )
            .await
        }
        Err(error) => Err(error),
    };
    crate::file_sources::finish(&state, request_id);
    result
}

#[tauri::command]
pub fn queue_dub_voice(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    selection: Selection,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    crate::voice_operations::require_enabled(&app)?;
    crate::dependencies::engine_root(&app)?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let root = translation::commands::completed_root(repo, &selection.draft_id)?;
    let draft = selection.draft(&root)?;
    let label = if repo.data.settings.language == "es" {
        "Voz sincronizada"
    } else {
        "Timed voice"
    };
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::DubVoice { selection },
            format!("{label} · {}", draft.name),
        )],
        &client_request,
    )?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}

pub fn video_input<'a>(
    repo: &'a Repository,
    options: &VideoOptions,
) -> Result<(library::LibraryItem, PathBuf, &'a Selection), String> {
    options.validate()?;
    let voice = repo
        .data
        .jobs
        .iter()
        .find(|job| job.id == options.voice_job_id && job.state == queue::JobState::Completed)
        .ok_or("Generate the synchronized WAV before exporting a video.")?;
    let queue::Source::DubVoice { selection } = &voice.source else {
        return Err("Choose a finished dubbing voice job.".into());
    };
    let [output] = voice.outputs.as_slice() else {
        return Err("The synchronized WAV is unavailable.".into());
    };
    if !repo
        .data
        .library
        .iter()
        .any(|item| item.path == *output && item.kind == "audio")
    {
        return Err("The synchronized WAV is missing from Library.".into());
    }
    let video = repo
        .data
        .library
        .iter()
        .find(|item| item.id == options.video_id && item.kind == "video")
        .ok_or("Choose a video from Library.")?
        .clone();
    Ok((video, library::authorized_path(output)?, selection))
}

#[tauri::command]
pub fn queue_dub_video(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    options: VideoOptions,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    crate::dependencies::engine_root(&app)?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let (video, _, selection) = video_input(repo, &options)?;
    let root = translation::commands::completed_root(repo, &selection.draft_id)?;
    let draft = selection.draft(&root)?;
    library::authorized_path(&video.path)?;
    let label = if repo.data.settings.language == "es" {
        "Copia con doblaje"
    } else {
        "Dubbed copy"
    };
    let jobs = queue::enqueue_tasks(
        repo,
        vec![(
            queue::Source::DubVideo {
                options,
                pair: draft.selection.pair,
            },
            format!("{label} · {}", video.name),
        )],
        &client_request,
    )?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}

fn duration(info: &serde_json::Value) -> Result<f64, String> {
    info["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| "The input has no verified duration.".into())
}

fn stream_count(info: &serde_json::Value, kind: &str) -> usize {
    info["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|stream| stream["codec_type"] == kind)
        .count()
}

pub async fn mux(
    engines: &Path,
    video: &Path,
    voice: &Path,
    staging: &Path,
    mode: VideoMode,
    language: &str,
    cancel: watch::Receiver<bool>,
) -> Result<(PathBuf, serde_json::Value), String> {
    let video = library::authorized_path(video)?;
    let original = crate::media::probe(engines, &video, cancel.clone()).await?;
    let voice_info = crate::media::probe(engines, voice, cancel.clone()).await?;
    let seconds = duration(&original)?;
    if duration(&voice_info)? > seconds + 0.02 {
        return Err("DUB_DURATION".into());
    }
    if stream_count(&voice_info, "audio") != 1 || stream_count(&voice_info, "video") != 0 {
        return Err("The synchronized WAV was replaced or cannot be read.".into());
    }
    let originals = stream_count(&original, "audio");
    if mode == VideoMode::Mix && originals == 0 {
        return Err("DUB_NO_ORIGINAL".into());
    }
    let output = staging.join("Dubbed copy.mkv");
    let mut args = vec![
        "-v".into(),
        "error".into(),
        "-nostdin".into(),
        "-n".into(),
        "-protocol_whitelist".into(),
        "file".into(),
        "-i".into(),
        video.to_string_lossy().into_owned(),
        "-protocol_whitelist".into(),
        "file".into(),
        "-i".into(),
        voice.to_string_lossy().into_owned(),
        "-map".into(),
        "0:V:0".into(),
        "-map".into(),
        "0:s?".into(),
        "-c:v".into(),
        "copy".into(),
        "-c:s".into(),
        "copy".into(),
    ];
    let index = if mode == VideoMode::Alternate {
        originals
    } else {
        0
    };
    for (index, stream) in original["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|stream| stream["codec_type"] == "subtitle")
        .enumerate()
    {
        if stream["codec_name"] == "mov_text" {
            args.extend([format!("-c:s:{index}"), "srt".into()]);
        }
    }
    if mode == VideoMode::Alternate {
        args.extend(["-map", "0:a?", "-c:a", "copy"].map(str::to_owned));
    }
    let filter = if mode == VideoMode::Mix {
        "[1:a]apad[dub];[dub][0:a:0]amix=inputs=2:duration=first:weights='0.8 0.2':normalize=0[speech]"
    } else {
        "[1:a]apad[speech]"
    };
    args.extend(["-filter_complex", filter, "-map", "[speech]"].map(str::to_owned));
    args.extend([
        format!("-c:a:{index}"),
        "aac".into(),
        format!("-metadata:s:a:{index}"),
        format!("language={language}"),
        format!("-metadata:s:a:{index}"),
        "title=Local translated voice".into(),
        format!("-disposition:a:{index}"),
        if index == 0 {
            "default".into()
        } else {
            "0".into()
        },
        "-t".into(),
        format!("{seconds:.6}"),
        output.to_string_lossy().into_owned(),
    ]);
    engine::tool(engines, "ffmpeg.exe", args, cancel.clone()).await?;
    let info = crate::media::probe(engines, &output, cancel).await?;
    if stream_count(&info, "audio") != index + 1
        || stream_count(&info, "video") != 1
        || (duration(&info)? - seconds).abs() > 0.25
    {
        return Err(
            "The dubbed copy could not be verified. The synchronized WAV is still available."
                .into(),
        );
    }
    Ok((output, info))
}
