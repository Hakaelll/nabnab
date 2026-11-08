use super::{EmbeddedTrack, MAX_SCRIPT};
use crate::{dependencies, library, AppState};
use serde::Deserialize;
use std::{
    future::Future,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tauri::AppHandle;
use tokio::{io::AsyncReadExt, process::Command};

#[derive(Deserialize, Default)]
struct Tags {
    title: Option<String>,
    language: Option<String>,
}

#[derive(Deserialize)]
struct Probe {
    streams: Vec<Stream>,
}

#[derive(Deserialize)]
struct Stream {
    index: u32,
    codec_name: Option<String>,
    #[serde(default)]
    tags: Tags,
}

pub(super) fn input(
    app: &AppHandle,
    state: &AppState,
    video: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let path = {
        let guard = state.repository.lock().unwrap();
        let item = guard
            .as_ref()
            .ok_or("Open the app first.")?
            .data
            .library
            .iter()
            .find(|item| item.id == video && item.kind == "video")
            .ok_or("Video not found.")?;
        library::authorized_path(&item.path)?
    };
    if !path.is_file()
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mkv"))
    {
        return Err("Choose a local Matroska video.".into());
    }
    Ok((dependencies::engine_root(app)?, path))
}

fn command(executable: PathBuf) -> Command {
    let mut command = Command::new(executable);
    command
        .args([
            "-v",
            "error",
            "-max_alloc",
            "67108864",
            "-protocol_whitelist",
            "file",
            "-format_whitelist",
            "matroska,webm",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .creation_flags(0x08000000);
    command
}

async fn output(mut command: Command, limit: usize) -> Result<Vec<u8>, String> {
    let mut child = command
        .spawn()
        .map_err(|_| "Could not open the media engine.")?;
    let mut output = Vec::new();
    child
        .stdout
        .take()
        .ok_or("Could not open subtitle output.")?
        .take(limit as u64 + 1)
        .read_to_end(&mut output)
        .await
        .map_err(|_| "Could not read subtitle output.")?;
    if output.len() > limit {
        return Err("The embedded subtitle data exceeds the size limit.".into());
    }
    if !child
        .wait()
        .await
        .map_err(|_| "The media engine stopped.")?
        .success()
    {
        return Err("Could not read embedded subtitles from this video.".into());
    }
    Ok(output)
}

pub(super) async fn bounded<T>(
    operation: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::time::timeout(Duration::from_secs(10), operation)
        .await
        .map_err(|_| "Reading embedded subtitles timed out.")?
}

pub(super) async fn tracks(root: &Path, path: &Path) -> Result<Vec<EmbeddedTrack>, String> {
    let mut probe = command(root.join("ffprobe.exe"));
    probe
        .args([
            "-select_streams",
            "s",
            "-show_entries",
            "stream=index,codec_name:stream_tags=title,language",
            "-of",
            "json",
            "-i",
        ])
        .arg(path);
    let bytes = output(probe, 128 * 1024).await?;
    let probe: Probe =
        serde_json::from_slice(&bytes).map_err(|_| "Could not read the embedded subtitle list.")?;
    let label = |value: Option<String>| {
        value
            .map(|text| {
                text.trim()
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .take(120)
                    .collect::<String>()
            })
            .filter(|text| !text.is_empty())
    };
    Ok(probe
        .streams
        .into_iter()
        .filter_map(|stream| {
            let codec = match stream.codec_name.as_deref() {
                Some("ass") => "ASS",
                Some("ssa") => "SSA",
                _ => return None,
            };
            Some(EmbeddedTrack {
                index: stream.index,
                codec,
                title: label(stream.tags.title),
                language: label(stream.tags.language),
            })
        })
        .collect())
}

pub(super) async fn extract(root: &Path, path: &Path, index: u32) -> Result<Vec<u8>, String> {
    if !tracks(root, path)
        .await?
        .iter()
        .any(|track| track.index == index)
    {
        return Err("The selected track is not an embedded ASS/SSA subtitle.".into());
    }
    let mut ffmpeg = command(root.join("ffmpeg.exe"));
    ffmpeg.args(["-nostdin", "-i"]).arg(path).args([
        "-map",
        &format!("0:{index}"),
        "-c:s",
        "copy",
        "-f",
        "ass",
        "pipe:1",
    ]);
    output(ffmpeg, MAX_SCRIPT).await
}
