use super::{from_probe, selected, Patch, Tags, MAX_SELECTION};
use crate::{dependencies, engine, library, media, queue, AppState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};
use tauri::{AppHandle, State};
use tokio::sync::watch;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopySelection {
    pub library_id: String,
    stamp: String,
    patch: Patch,
}

impl CopySelection {
    pub fn validate(&self) -> Result<(), String> {
        if self.library_id.is_empty()
            || self.library_id.len() > 128
            || !self.stamp.split_once(':').is_some_and(|(size, time)| {
                size.parse::<u64>().is_ok() && time.parse::<u128>().is_ok()
            })
            || self.patch.fields().iter().all(|(_, value)| value.is_none())
        {
            return Err("Choose a current audio file and at least one tag to write.".into());
        }
        self.patch.validate()
    }
}

fn extension(path: &Path) -> Result<String, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "mp3" | "m4a" | "m4b" | "flac" | "ogg" | "opus"
    ) {
        return Err("Tagged copies support MP3, M4A, M4B, FLAC, OGG and Opus.".into());
    }
    Ok(extension)
}

fn stamp(path: &Path) -> Result<String, String> {
    let info = fs::metadata(path).map_err(|_| "Could not inspect the original audio file.")?;
    let time = info
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .ok_or("Could not verify when the original audio file was modified.")?;
    Ok(format!("{}:{}", info.len(), time.as_nanos()))
}

#[tauri::command]
pub fn audio_tags_queue_copies(
    app: AppHandle,
    state: State<Arc<AppState>>,
    ids: Vec<String>,
    patch: Patch,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    dependencies::engine_root(&app)?;
    patch.validate()?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let items = selected(repo, &ids, MAX_SELECTION)?;
    let tasks = items
        .into_iter()
        .map(|(_, item)| {
            let path = library::authorized_path(&item.path)?;
            extension(&path)?;
            let selection = CopySelection {
                library_id: item.id,
                stamp: stamp(&path)?,
                patch: patch.clone(),
            };
            selection.validate()?;
            Ok((queue::Source::AudioTagsCopy { selection }, item.name))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let jobs = queue::enqueue_tasks(repo, tasks, &client_request)?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}

fn command_args(input: &Path) -> Vec<String> {
    [
        "-hide_banner",
        "-v",
        "error",
        "-nostdin",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        "mp3,flac,mov,ogg",
        "-i",
    ]
    .into_iter()
    .map(str::to_owned)
    .chain([input.to_string_lossy().into_owned()])
    .collect()
}

async fn audio_hash(
    engines: &Path,
    input: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<String, String> {
    let mut args = command_args(input);
    args.extend(
        [
            "-map",
            "0:a",
            "-c:a",
            "copy",
            "-f",
            "streamhash",
            "-hash",
            "sha256",
            "-",
        ]
        .map(str::to_owned),
    );
    let raw =
        engine::run_process(engines.join("ffmpeg.exe"), args, cancel, 0, true, |_| {}).await?;
    let result: Vec<_> = raw
        .lines()
        .filter(|line| line.contains(",a,SHA256="))
        .collect();
    if result.is_empty() {
        return Err("Could not verify the encoded audio streams.".into());
    }
    Ok(result.join("\n"))
}

fn audio_only(info: &Value) -> bool {
    info["streams"].as_array().is_some_and(|streams| {
        streams.iter().any(|stream| stream["codec_type"] == "audio")
            && streams.iter().all(|stream| {
                stream["codec_type"] == "audio"
                    || stream["codec_type"] == "video" && stream["disposition"]["attached_pic"] == 1
            })
    })
}

fn patched_fields_match(expected: &Tags, actual: &Tags, patch: &Patch) -> bool {
    (patch.title.is_none() || expected.title == actual.title)
        && (patch.artist.is_none() || expected.artist == actual.artist)
        && (patch.album_artist.is_none() || expected.album_artist == actual.album_artist)
        && (patch.album.is_none() || expected.album == actual.album)
        && (patch.genre.is_none() || expected.genre == actual.genre)
        && (patch.disc.is_none() || expected.disc == actual.disc)
        && (patch.track.is_none() || expected.track == actual.track)
}

async fn tagged_copy(
    engines: &Path,
    input: &Path,
    expected_stamp: &str,
    patch: &Patch,
    staging: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<(PathBuf, Value), String> {
    let input = library::authorized_path(input)?;
    let extension = extension(&input)?;
    if stamp(&input)? != expected_stamp {
        return Err("The original audio changed. Preview the tags and queue a new copy.".into());
    }
    let source = media::probe(engines, &input, cancel.clone()).await?;
    if !audio_only(&source) {
        return Err("Choose an audio file with optional embedded cover art.".into());
    }
    let original_hash = audio_hash(engines, &input, cancel.clone()).await?;
    let stem = input
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("audio")
        .chars()
        .take(45)
        .collect::<String>();
    let filename = format!("{stem} (tags).{extension}");
    crate::direct::validate_filename(&filename)?;
    let output = staging.join(filename);
    if fs::symlink_metadata(&output).is_ok() {
        return Err("A tagged copy already exists in this job.".into());
    }
    let mut args = command_args(&input);
    args.extend(
        [
            "-n",
            "-map",
            "0:a",
            "-map",
            "0:v?",
            "-map_metadata",
            "0",
            "-map_chapters",
            "0",
            "-c",
            "copy",
        ]
        .map(str::to_owned),
    );
    for (key, value) in patch.fields() {
        if let Some(value) = value {
            let scope = if matches!(extension.as_str(), "ogg" | "opus") {
                "-metadata:s:a"
            } else {
                "-metadata"
            };
            args.extend([scope.into(), format!("{key}={}", value.trim())]);
        }
    }
    if extension == "m4b" {
        args.extend(["-f".into(), "mp4".into()]);
    }
    args.push(output.to_string_lossy().into_owned());
    let result = async {
        engine::tool(engines, "ffmpeg.exe", args, cancel.clone()).await?;
        let copied = media::probe(engines, &output, cancel.clone()).await?;
        if !audio_only(&copied)
            || stamp(&input)? != expected_stamp
            || original_hash != audio_hash(engines, &output, cancel.clone()).await?
        {
            return Err(
                "The copy did not preserve the original encoded audio. It was not published."
                    .into(),
            );
        }
        let mut expected = from_probe(&source);
        patch.apply(&mut expected);
        if !patched_fields_match(&expected, &from_probe(&copied), patch) {
            return Err(
                "This audio container could not preserve the chosen tags. No copy was published."
                    .into(),
            );
        }
        Ok(copied)
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&output);
    }
    result.map(|info| (output, info))
}

pub async fn execute(
    state: &AppState,
    engines: &Path,
    selection: &CopySelection,
    staging: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<(PathBuf, Value), String> {
    selection.validate()?;
    let item = super::audio_item(state, &selection.library_id)?;
    tagged_copy(
        engines,
        &item.path,
        &selection.stamp,
        &selection.patch,
        staging,
        cancel,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires the verified production FFmpeg engines"]
    async fn real_engines_tag_copy_preserves_encoded_stream_and_original() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engines");
        let fixture = std::env::temp_dir().join(format!(
            "nabnab-audio-tags-{}-{}",
            std::process::id(),
            queue::now()
        ));
        fs::create_dir(&fixture).unwrap();
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(fixture.clone());
        let (sender, token) = watch::channel(false);
        let patch: Patch =
            serde_json::from_value(serde_json::json!({"title":"Local copy", "track":"4"})).unwrap();
        for (extension, codec) in [
            ("mp3", "libmp3lame"),
            ("m4a", "aac"),
            ("m4b", "aac"),
            ("flac", "flac"),
            ("ogg", "vorbis"),
            ("opus", "libopus"),
        ] {
            let input = fixture.join(format!("original.{extension}"));
            let mut args: Vec<_> = [
                "-hide_banner",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=44100:cl=stereo",
                "-t",
                "0.25",
                "-c:a",
                codec,
                "-metadata",
                "title=Original",
            ]
            .map(str::to_owned)
            .into();
            if extension == "m4b" {
                args.extend(["-f".into(), "mp4".into()]);
            }
            if extension == "ogg" {
                args.extend(["-strict".into(), "experimental".into()]);
            }
            args.push(input.to_string_lossy().into_owned());
            engine::tool(&root, "ffmpeg.exe", args, token.clone())
                .await
                .unwrap_or_else(|error| panic!("{extension} fixture: {error}"));
            let previous = fs::read(&input).unwrap();
            let (copy, info) = tagged_copy(
                &root,
                &input,
                &stamp(&input).unwrap(),
                &patch,
                &fixture,
                token.clone(),
            )
            .await
            .unwrap_or_else(|error| panic!("{extension} tagged copy: {error}"));
            assert_ne!(copy, input);
            assert_eq!(fs::read(&input).unwrap(), previous);
            let tags = from_probe(&info);
            assert_eq!(tags.title.as_deref(), Some("Local copy"));
            assert_eq!(tags.track, Some(4));
            assert_eq!(
                audio_hash(&root, &input, token.clone()).await.unwrap(),
                audio_hash(&root, &copy, token.clone()).await.unwrap()
            );
        }
        drop(sender);
    }
}
