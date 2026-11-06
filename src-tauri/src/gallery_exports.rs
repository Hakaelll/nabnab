use crate::{
    engine,
    library::{self, LibraryItem},
    model::Progress,
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tokio::sync::watch;

const MAX_TOTAL: u64 = 128 * 1024 * 1024;
const MAX_FILE: u64 = 16 * 1024 * 1024;
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn media_type(path: &Path) -> Result<(&'static str, &'static str), String> {
    match path.extension().and_then(|extension|extension.to_str()).unwrap_or_default().to_ascii_lowercase().as_str(){
        "jpg"|"jpeg"=>Ok(("image/jpeg","img")),"png"=>Ok(("image/png","img")),"gif"=>Ok(("image/gif","img")),"webp"=>Ok(("image/webp","img")),"bmp"=>Ok(("image/bmp","img")),
        "mp4"=>Ok(("video/mp4","video")),"webm"=>Ok(("video/webm","video")),"mp3"=>Ok(("audio/mpeg","audio")),"m4a"=>Ok(("audio/mp4","audio")),"ogg"|"opus"=>Ok(("audio/ogg","audio")),"wav"=>Ok(("audio/wav","audio")),
        _=>Err("HTML galleries support JPEG, PNG, GIF, WebP, BMP, MP4, WebM, MP3, M4A, Ogg/Opus and WAV. Convert other formats first.".into())
    }
}
fn source_origin(source: Option<&str>) -> Option<String> {
    let parsed = url::Url::parse(source?).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    Some(parsed.origin().ascii_serialization())
}

fn write_base64(
    writer: &mut impl Write,
    file: &mut impl Read,
    cancel: &watch::Receiver<bool>,
) -> Result<u64, String> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buffer = [0u8; 3 * 4096];
    let mut total = 0u64;
    loop {
        if *cancel.borrow() {
            return Err(engine::CANCELLED.into());
        }
        let mut length = 0;
        while length < buffer.len() {
            let received = file
                .read(&mut buffer[length..])
                .map_err(|e| e.to_string())?;
            if received == 0 {
                break;
            }
            length += received;
        }
        if length == 0 {
            break;
        }
        total += length as u64;
        if total > MAX_FILE {
            return Err("A gallery file grew beyond the 16 MiB limit while exporting.".into());
        }
        let mut encoded = Vec::with_capacity(length.div_ceil(3) * 4);
        for chunk in buffer[..length].chunks(3) {
            let a = chunk[0];
            let b = chunk.get(1).copied().unwrap_or(0);
            let c = chunk.get(2).copied().unwrap_or(0);
            encoded.push(ALPHABET[(a >> 2) as usize]);
            encoded.push(ALPHABET[(((a & 3) << 4) | (b >> 4)) as usize]);
            encoded.push(if chunk.len() > 1 {
                ALPHABET[(((b & 15) << 2) | (c >> 6)) as usize]
            } else {
                b'='
            });
            encoded.push(if chunk.len() > 2 {
                ALPHABET[(c & 63) as usize]
            } else {
                b'='
            });
        }
        writer.write_all(&encoded).map_err(|e| e.to_string())?;
    }
    Ok(total)
}

fn write_html(
    items: &[(LibraryItem, PathBuf)],
    output: &Path,
    cancel: &watch::Receiver<bool>,
) -> Result<(), String> {
    let mut writer = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)
        .map_err(|e| e.to_string())?;
    writer.write_all(r##"<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; media-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'"><title>nabnab · local gallery</title><style>body{margin:32px;background:#090a0c;color:#f2f2f3;font:15px/1.5 system-ui}h1{font-size:24px}main{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,260px),1fr));gap:24px}figure{margin:0;padding:12px;background:#17191d;border:1px solid #2b2e34;border-radius:10px;overflow:hidden}img,video{display:block;width:100%;height:260px;object-fit:contain}audio{width:100%}figcaption{padding-top:12px;overflow-wrap:anywhere}small{display:block;color:#b2b5bd}a{color:inherit}</style><h1>nabnab · local gallery</h1><p>Embedded local files. Source links open only when selected.</p><main>"##.as_bytes()).map_err(|e|e.to_string())?;
    let mut total = 0u64;
    for (index, (item, path)) in items.iter().enumerate() {
        let (mime, tag) = media_type(path)?;
        write!(
            writer,
            "<figure><{tag} {}src=\"data:{mime};base64,",
            if tag == "img" {
                "alt=\"\" "
            } else {
                "controls preload=\"none\" "
            }
        )
        .map_err(|e| e.to_string())?;
        let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
        total += write_base64(&mut writer, &mut file, cancel)?;
        if total > MAX_TOTAL {
            return Err("The gallery grew beyond its 128 MiB input limit while exporting.".into());
        }
        writer.write_all(b"\">").map_err(|e| e.to_string())?;
        if tag != "img" {
            write!(writer, "</{tag}>").map_err(|e| e.to_string())?;
        }
        write!(
            writer,
            "<figcaption><strong>{}. {}</strong>",
            index + 1,
            escape(&item.name)
        )
        .map_err(|e| e.to_string())?;
        if let Some(creator) = item.details.creator.as_deref() {
            write!(writer, "<small>{}</small>", escape(creator)).map_err(|e| e.to_string())?;
        }
        if let Some(origin) = item.details.source_url.as_deref().and_then(|link|crate::model::validate_url(link).ok()).and_then(|link|{let mut u=url::Url::parse(&link).ok()?;u.set_query(None);u.set_fragment(None);Some(u.to_string())}).or_else(||source_origin(item.source.as_deref())) {
            write!(writer,"<small><a href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a></small>",escape(&origin),escape(&origin)).map_err(|e|e.to_string())?;
        }
        if let Some(identity) = item.details.content_id.as_deref() {
            write!(writer, "<small>{}</small>", escape(identity)).map_err(|e| e.to_string())?;
        }
        writer
            .write_all(b"</figcaption></figure>")
            .map_err(|e| e.to_string())?;
    }
    writer
        .write_all(b"</main></html>")
        .map_err(|e| e.to_string())?;
    writer.sync_all().map_err(|e| e.to_string())
}

fn image_demuxer(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("png_pipe"),
        "jpg" | "jpeg" => Some("jpeg_pipe"),
        "webp" => Some("webp_pipe"),
        "bmp" => Some("bmp_pipe"),
        "gif" => Some("gif"),
        _ => None,
    }
}

pub async fn execute(
    engines: &Path,
    inputs: &[LibraryItem],
    staging: &Path,
    kind: &str,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    if !matches!(kind, "html" | "contactSheet") || inputs.is_empty() || inputs.len() > 200 {
        return Err("Choose HTML or contact sheet and 1–200 gallery files.".into());
    }
    if kind == "contactSheet" && inputs.len() > 64 {
        return Err("A contact sheet supports up to 64 images per job.".into());
    }
    let staging = library::authorized_path(staging)?;
    let mut total = 0u64;
    let mut items = Vec::new();
    for input in inputs {
        let path = library::authorized_path(&input.path)?;
        let (mime, _) = media_type(&path)?;
        if kind == "contactSheet" && !mime.starts_with("image/") {
            return Err(
                "Contact sheets accept images. Export mixed media as an HTML gallery.".into(),
            );
        }
        let size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
        if size == 0 || size > MAX_FILE {
            return Err("Gallery exports accept files of 1 byte to 16 MiB each.".into());
        }
        total = total.checked_add(size).ok_or("Invalid export size.")?;
        if total > MAX_TOTAL {
            return Err(
                "This gallery exceeds the 128 MiB input limit. Split it into smaller exports."
                    .into(),
            );
        }
        items.push((input.clone(), path));
    }
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let output = staging.join(format!(
        "gallery-{run_id}.{}",
        if kind == "html" { "html" } else { "png" }
    ));
    if output.exists() {
        return Err("The gallery export already exists. Start a new job to preserve it.".into());
    }
    emit(Progress {
        request_id: run_id,
        phase: "processing".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
    if kind == "html" {
        let file = output.clone();
        let cancelled = cancel.clone();
        tauri::async_runtime::spawn_blocking(move || write_html(&items, &file, &cancelled))
            .await
            .map_err(|e| e.to_string())??;
    } else {
        for (index, (_, input)) in items.iter().enumerate() {
            let frame = staging.join(format!("gallery-frame-{index:04}.png"));
            if frame.exists() {
                return Err("Contact sheet staging already has frames. Start a new job.".into());
            }
            let mut args = vec![
                "-nostdin".into(),
                "-v".into(),
                "error".into(),
                "-n".into(),
                "-protocol_whitelist".into(),
                "file,pipe".into(),
                "-format_whitelist".into(),
                "gif,png_pipe,jpeg_pipe,webp_pipe,bmp_pipe".into(),
            ];
            if let Some(format) = image_demuxer(input) {
                args.extend(["-f".into(), format.into()]);
            }
            args.extend(["-i".into(),input.to_string_lossy().into_owned(),"-frames:v".into(),"1".into(),"-vf".into(),"scale=240:180:force_original_aspect_ratio=decrease,pad=240:180:(ow-iw)/2:(oh-ih)/2:color=0x101114".into(),"-update".into(),"1".into(),frame.to_string_lossy().into_owned()]);
            engine::run_process(
                engines.join("ffmpeg.exe"),
                args,
                cancel.clone(),
                run_id,
                false,
                emit.clone(),
            )
            .await?;
            let frame = crate::checked_output(&staging, &frame.to_string_lossy())?;
            if fs::metadata(frame).map_err(|e| e.to_string())?.len() == 0 {
                return Err("An image could not be rendered for the contact sheet.".into());
            }
            emit(Progress {
                request_id: run_id,
                phase: "processing".into(),
                percent: Some((index + 1) as f64 / items.len() as f64 * 80.0),
                speed: None,
                downloaded: None,
                total: None,
            });
        }
        let columns = (items.len() as f64).sqrt().ceil() as usize;
        let rows = items.len().div_ceil(columns);
        let args = vec![
            "-nostdin".into(),
            "-v".into(),
            "error".into(),
            "-n".into(),
            "-protocol_whitelist".into(),
            "file,pipe".into(),
            "-f".into(),
            "image2".into(),
            "-start_number".into(),
            "0".into(),
            "-i".into(),
            staging
                .join("gallery-frame-%04d.png")
                .to_string_lossy()
                .into_owned(),
            "-frames:v".into(),
            "1".into(),
            "-vf".into(),
            format!(
                "tile={columns}x{rows}:nb_frames={}:padding=4:margin=4",
                items.len()
            ),
            "-update".into(),
            "1".into(),
            output.to_string_lossy().into_owned(),
        ];
        engine::run_process(
            engines.join("ffmpeg.exe"),
            args,
            cancel.clone(),
            run_id,
            false,
            emit,
        )
        .await?;
    }
    let output = crate::checked_output(&staging, &output.to_string_lossy())?;
    if fs::metadata(&output).map_err(|e| e.to_string())?.len() == 0 {
        return Err("The gallery export is empty.".into());
    }
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streamed_base64_has_literal_expected_bytes() {
        let (_sender, cancel) = watch::channel(false);
        for (input, expected) in [
            (b"f".as_slice(), "Zg=="),
            (b"fo".as_slice(), "Zm8="),
            (b"foo".as_slice(), "Zm9v"),
            (b"foobar".as_slice(), "Zm9vYmFy"),
        ] {
            let mut output = Vec::new();
            let mut source = input;
            assert_eq!(
                write_base64(&mut output, &mut source, &cancel).unwrap(),
                input.len() as u64
            );
            assert_eq!(output, expected.as_bytes());
        }
    }
    #[test]
    fn exported_attribution_never_contains_credentials_paths_or_queries() {
        assert_eq!(
            source_origin(Some(
                "https://user:secret@example.org/private/feed?token=secret"
            )),
            Some("https://example.org".into())
        );
        assert_eq!(source_origin(Some("file:///private")), None);
        assert_eq!(
            escape("<script a='x'>&"),
            "&lt;script a=&#39;x&#39;&gt;&amp;"
        );
    }
    #[tokio::test]
    #[ignore = "Requires bundled FFmpeg/ffprobe; uses copied project icon fixtures only."]
    async fn real_engines_gallery_exports_embed_files_and_render_the_selected_images() {
        let engines = std::env::var_os("NABNAB_TEST_ENGINES")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engines"));
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let work = std::env::temp_dir().join(format!(
            "nabnab-gallery-export-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&work).unwrap();
        let original =
            fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("icons/32x32.png")).unwrap();
        let mut items = Vec::new();
        for index in 0..2 {
            let path = work.join(format!("source-{index}.png"));
            fs::write(&path, &original).unwrap();
            let mut item = library::index(
                &path,
                None,
                Some("https://example.org/private?token=secret".into()),
                None,
            )
            .unwrap();
            item.details.creator = Some("<script>alert('unsafe')</script>".into());
            items.push(item);
        }
        let (_sender, cancel) = watch::channel(false);
        let stage = work.join("html");
        fs::create_dir(&stage).unwrap();
        let output = execute(&engines, &items, &stage, "html", cancel.clone(), 1, |_| {})
            .await
            .unwrap();
        let html = fs::read_to_string(output).unwrap();
        assert_eq!(html.matches("src=\"data:image/png;base64,").count(), 2);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("token=secret"));
        assert!(!html.contains("src=\"http"));
        let stage = work.join("sheet");
        fs::create_dir(&stage).unwrap();
        let output = execute(
            &engines,
            &items,
            &stage,
            "contactSheet",
            cancel.clone(),
            2,
            |_| {},
        )
        .await
        .unwrap();
        let raw = engine::tool(
            &engines,
            "ffprobe.exe",
            vec![
                "-v".into(),
                "error".into(),
                "-f".into(),
                "png_pipe".into(),
                "-show_streams".into(),
                "-of".into(),
                "json".into(),
                output.to_string_lossy().into_owned(),
            ],
            cancel,
        )
        .await
        .unwrap();
        let info: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(info["streams"][0]["width"], 492);
        assert_eq!(info["streams"][0]["height"], 188);
        for item in items {
            assert_eq!(fs::read(item.path).unwrap(), original);
        }
        assert_eq!(work.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(work).unwrap();
    }
}
