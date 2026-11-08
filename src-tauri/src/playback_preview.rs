use crate::{dependencies, library, AppState};
use std::{
    future::Future,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{AppHandle, State};
use tokio::{io::AsyncReadExt, process::Command, sync::watch};

const MAX_FRAME_BYTES: usize = 48 * 1024;

#[derive(Default)]
pub struct PlaybackRequest(Mutex<Option<(String, watch::Sender<bool>)>>);

impl PlaybackRequest {
    pub(crate) async fn run<T>(
        &self,
        id: &str,
        operation: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        let mut cancel = self.start(id.to_owned());
        let result = tokio::select! {
            biased;
            _ = cancel.changed() => Err("Playback request cancelled.".into()),
            result = operation => result,
        };
        self.cancel(id);
        result
    }

    pub(crate) fn start(&self, request: String) -> watch::Receiver<bool> {
        let (cancel, receiver) = watch::channel(false);
        if let Some((_, previous)) = self.0.lock().unwrap().replace((request, cancel)) {
            let _ = previous.send(true);
        }
        receiver
    }

    pub(crate) fn cancel(&self, request: &str) {
        let mut active = self.0.lock().unwrap();
        if active.as_ref().is_some_and(|(id, _)| id == request) {
            if let Some((_, cancel)) = active.take() {
                let _ = cancel.send(true);
            }
        }
    }
}

#[tauri::command]
pub fn playback_preview_cancel(state: State<'_, Arc<AppState>>, request: String) {
    state.playback_preview.cancel(&request);
}

#[tauri::command]
pub async fn playback_preview_frame(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    time: u32,
    request: String,
) -> Result<Vec<u8>, String> {
    if request.len() != 36
        || !request
            .bytes()
            .all(|byte| byte == b'-' || byte.is_ascii_hexdigit())
    {
        return Err("Invalid frame request.".into());
    }
    let path = {
        let guard = state.repository.lock().unwrap();
        let item = guard
            .as_ref()
            .ok_or("Open the app first.")?
            .data
            .library
            .iter()
            .find(|item| item.id == id && item.kind == "video")
            .ok_or("Video not found.")?;
        if item
            .duration
            .is_some_and(|duration| f64::from(time) >= duration)
        {
            return Err("Frame position is outside the video.".into());
        }
        library::authorized_path(&item.path)?
    };
    let executable = dependencies::engine_root(&app)?.join("ffmpeg.exe");
    let mut cancelled = state.playback_preview.start(request.clone());
    let extract = async {
        let mut command = Command::new(executable);
        command
            .args([
                "-hide_banner",
                "-v",
                "error",
                "-nostdin",
                "-threads",
                "1",
                "-protocol_whitelist",
                "file",
                "-format_whitelist",
                "mov,matroska,webm,avi,asf,mpeg,mpegts,ogg,flv",
                "-ss",
                &time.to_string(),
                "-i",
            ])
            .arg(path)
            .args([
                "-map",
                "0:V:0",
                "-an",
                "-sn",
                "-dn",
                "-filter_threads",
                "1",
                "-vf",
                "scale=224:126:force_original_aspect_ratio=decrease",
                "-frames:v",
                "1",
                "-c:v",
                "mjpeg",
                "-q:v",
                "6",
                "-f",
                "image2pipe",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or("Could not open frame output.")?
            .take(MAX_FRAME_BYTES as u64 + 1)
            .read_to_end(&mut output)
            .await
            .map_err(|error| error.to_string())?;
        let status = child.wait().await.map_err(|error| error.to_string())?;
        if !status.success()
            || output.len() > MAX_FRAME_BYTES
            || !output.starts_with(&[0xff, 0xd8])
            || !output.ends_with(&[0xff, 0xd9])
        {
            return Err("No preview frame is available.".into());
        }
        Ok(output)
    };
    let result = tokio::select! {
        _ = cancelled.changed() => Err("Frame preview cancelled.".into()),
        _ = tokio::time::sleep(Duration::from_secs(2)) => Err("Frame preview timed out.".into()),
        result = extract => result,
    };
    state.playback_preview.cancel(&request);
    result
}
