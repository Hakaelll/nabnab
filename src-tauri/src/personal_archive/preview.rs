use super::{managed_media, media_source, valid_id, Category, MAX_MEDIA};
use crate::{dependencies, AppState};
use std::{fs::File, path::Path, process::Stdio, sync::Arc, time::Duration};
use tauri::{ipc::Response, AppHandle, State, Webview};
use tokio::{io::AsyncReadExt, process::Command};

const MAX_FRAME: usize = 256 * 1024;

fn demuxer(source: &str) -> Option<&'static str> {
    let extension = Path::new(source)
        .extension()?
        .to_str()?
        .to_ascii_lowercase();
    Some(match extension.as_str() {
        "jpg" | "jpeg" => "jpeg_pipe",
        "png" => "png_pipe",
        "webp" => "webp_pipe",
        "gif" => "gif",
        "mp4" | "mov" => "mov",
        "webm" => "matroska",
        _ => return None,
    })
}

async fn frame(executable: &Path, input: File, format: &str) -> Result<Vec<u8>, String> {
    let mut command = Command::new(executable);
    command
        .args([
            "-hide_banner",
            "-v",
            "error",
            "-nostdin",
            "-max_alloc",
            "134217728",
            "-threads",
            "1",
            "-max_pixels",
            "24000000",
            "-protocol_whitelist",
            "fd",
            "-format_whitelist",
            format,
            "-f",
            format,
            "-i",
            "fd:",
            "-map",
            "0:V:0",
            "-map_metadata",
            "-1",
            "-an",
            "-sn",
            "-dn",
            "-filter_threads",
            "1",
            "-vf",
            "scale=w='min(iw,900)':h='min(ih,600)':force_original_aspect_ratio=decrease,setsar=1",
            "-frames:v",
            "1",
            "-c:v",
            "mjpeg",
            "-threads",
            "1",
            "-q:v",
            "5",
            "-f",
            "image2pipe",
            "pipe:1",
        ])
        .stdin(Stdio::from(input))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|_| "Could not start the local preview.")?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or("Could not read the local preview.")?
        .take(MAX_FRAME as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "Could not read the local preview.")?;
    if bytes.len() > MAX_FRAME
        || !child.wait().await.is_ok_and(|status| status.success())
        || !bytes.starts_with(&[0xff, 0xd8])
        || !bytes.ends_with(&[0xff, 0xd9])
    {
        return Err("This file could not be previewed.".into());
    }
    Ok(bytes)
}

#[tauri::command]
pub async fn personal_archive_preview_media(
    app: AppHandle,
    webview: Webview,
    state: State<'_, Arc<AppState>>,
    id: String,
    category: Category,
    record: String,
    consent: bool,
    request: String,
) -> Result<Response, String> {
    if webview.label() != "main"
        || !valid_id(&id)
        || !valid_id(&record)
        || request.len() != 36
        || !request
            .bytes()
            .all(|byte| byte == b'-' || byte.is_ascii_hexdigit())
    {
        return Err("Invalid archive preview request.".into());
    }
    let source_state = state.inner().clone();
    state
        .personal_archive
        .preview
        .run(&request, async move {
            let (input, format) = tokio::task::spawn_blocking(move || {
                let source = media_source(&source_state, &id, category, &record, consent)?;
                let format = demuxer(&source).ok_or("This media format has no visual preview.")?;
                let path = managed_media(&source_state, &id, &source)?;
                let file = File::open(path).map_err(|_| "The managed copy is unavailable.")?;
                let metadata = file
                    .metadata()
                    .map_err(|_| "The managed copy is unavailable.")?;
                if !metadata.is_file() || metadata.len() > MAX_MEDIA {
                    return Err("The file exceeds the archive preview limit.".into());
                }
                Ok::<_, String>((file, format))
            })
            .await
            .map_err(|_| "Could not open the selected archive file.")??;
            let executable = dependencies::engine_root(&app)?.join("ffmpeg.exe");
            let bytes =
                tokio::time::timeout(Duration::from_secs(5), frame(&executable, input, format))
                    .await
                    .map_err(|_| "The archive preview took too long.")??;
            Ok(Response::new(bytes))
        })
        .await
}

#[tauri::command]
pub fn personal_archive_preview_cancel(
    webview: Webview,
    state: State<'_, Arc<AppState>>,
    request: String,
) {
    if webview.label() == "main" {
        state.personal_archive.preview.cancel(&request);
    }
}
