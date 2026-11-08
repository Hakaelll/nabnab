use crate::{library, playback_audio, AppState};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};
use tauri::{
    http::{Request, Response},
    AppHandle, Manager,
};
fn range(value: Option<&str>, size: u64) -> Result<(u64, u64), String> {
    let Some(value) = value else {
        return Ok((0, size.min(4 * 1024 * 1024).saturating_sub(1)));
    };
    let value = value.strip_prefix("bytes=").ok_or("Invalid byte range.")?;
    if value.contains(',') {
        return Err("Multiple ranges unsupported.".into());
    }
    let (start, end) = value.split_once('-').ok_or("Invalid byte range.")?;
    let (start, end) = if start.is_empty() {
        let suffix = end.parse::<u64>().map_err(|_| "Invalid suffix.")?;
        if suffix == 0 {
            return Err("Empty suffix.".into());
        }
        (size.saturating_sub(suffix), size.saturating_sub(1))
    } else {
        let start = start.parse::<u64>().map_err(|_| "Invalid offset.")?;
        let end = if end.is_empty() {
            size.saturating_sub(1)
        } else {
            end.parse().map_err(|_| "Invalid end.")?
        };
        (start, end)
    };
    if start >= size || end < start {
        return Err("Range outside file.".into());
    }
    Ok((
        start,
        end.min(size - 1)
            .min(start.saturating_add(4 * 1024 * 1024 - 1)),
    ))
}
pub fn serve(app: &AppHandle, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let response = |status, body: Vec<u8>| {
        Response::builder()
            .status(status)
            .header("Content-Type", "text/plain")
            .body(body)
            .unwrap()
    };
    if !matches!(request.method().as_str(), "GET" | "HEAD") {
        return response(405, vec![]);
    }
    let origin = request
        .headers()
        .get("Origin")
        .and_then(|s| s.to_str().ok())
        .unwrap_or("");
    if !origin.is_empty()
        && !matches!(
            origin,
            "http://tauri.localhost"
                | "https://tauri.localhost"
                | "tauri://localhost"
                | "http://localhost:1420"
                | "http://127.0.0.1:1420"
        )
    {
        return response(403, vec![]);
    }
    let raw_path = request.uri().path().trim_start_matches('/');
    let thumbnail = raw_path.starts_with("thumbnail/");
    let audio = match request.uri().query() {
        Some(query) if !thumbnail => {
            let Some(id) = query
                .strip_prefix("audio=")
                .and_then(|value| value.parse::<u32>().ok())
            else {
                return response(404, vec![]);
            };
            Some(id)
        }
        _ => None,
    };
    let id = if thumbnail {
        raw_path.trim_start_matches("thumbnail/")
    } else {
        raw_path
    };
    if id.is_empty()
        || id.len() > 100
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return response(404, vec![]);
    }
    let state = app.state::<Arc<AppState>>();
    let file = {
        let guard = state.repository.lock().unwrap();
        guard
            .as_ref()
            .and_then(|r| {
                r.data.library.iter().find(|i| i.id == id).map(|item| {
                    if thumbnail {
                        crate::support::thumbnail_path(&r.root().join("thumbnail-cache"), item).ok()
                    } else {
                        (audio.is_none() || item.kind == "video").then(|| item.path.clone())
                    }
                })
            })
            .flatten()
    };
    let Some(path) = file else {
        return response(404, vec![]);
    };
    let Ok(path) = library::authorized_path(&path) else {
        return response(404, b"Not found. Locate the file in the library.".to_vec());
    };
    let Ok(mut file) = File::open(&path) else {
        return response(404, vec![]);
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    if size == 0 {
        return response(404, vec![]);
    }
    let patch = if let Some(track) = audio {
        if !playback_audio::supported(&path) {
            return response(404, vec![]);
        }
        let Ok(offsets) = playback_audio::audio_patch(&mut file, size, track) else {
            return response(404, vec![]);
        };
        offsets
    } else {
        Vec::new()
    };
    let Ok((start, end)) = range(
        request.headers().get("Range").and_then(|v| v.to_str().ok()),
        size,
    ) else {
        return Response::builder()
            .status(416)
            .header("Content-Range", format!("bytes */{size}"))
            .body(vec![])
            .unwrap();
    };
    let mime = match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "m4a" | "m4b" | "aac" => "audio/mp4",
        "opus" | "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    };
    let mut data = vec![];
    if request.method() != "HEAD" {
        if file.seek(SeekFrom::Start(start)).is_err()
            || file.take(end - start + 1).read_to_end(&mut data).is_err()
        {
            return response(500, vec![]);
        }
        playback_audio::apply_audio_patch(&mut data, start, patch);
    }
    let mut builder = Response::builder()
        .status(
            if request.headers().contains_key("Range") || end + 1 < size {
                206
            } else {
                200
            },
        )
        .header("Content-Type", mime)
        .header("Accept-Ranges", "bytes")
        .header("Content-Length", (end - start + 1).to_string())
        .header("Content-Range", format!("bytes {start}-{end}/{size}"))
        .header("X-Content-Type-Options", "nosniff")
        .header("Cache-Control", "private, no-store");
    if !origin.is_empty() {
        builder = builder.header("Access-Control-Allow-Origin", origin);
    }
    builder.body(data).unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_single_ranges() {
        assert_eq!(range(Some("bytes=20-40"), 100).unwrap(), (20, 40));
        assert_eq!(range(Some("bytes=-10"), 100).unwrap(), (90, 99));
        assert_eq!(range(None, 100_000_000).unwrap(), (0, 4 * 1024 * 1024 - 1));
        for r in [
            "bytes=100-",
            "bytes=1-0",
            "bytes=1-2,3-4",
            "bytes=-0",
            "bytes=bad-",
        ] {
            assert!(range(Some(r), 100).is_err(), "{r}");
        }
    }
}
