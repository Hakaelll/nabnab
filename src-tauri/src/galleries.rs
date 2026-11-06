use crate::{engine, queue, AppState};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, sync::Arc};
use tauri::{AppHandle, State};
use tokio::sync::watch;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    pub position: usize,
    pub title: String,
    pub author: Option<String>,
    pub kind: String,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub date: Option<String>,
    #[serde(skip)]
    pub url: String,
    #[serde(skip)]
    pub referer: Option<String>,
    #[serde(skip)]
    pub source_url: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gallery {
    pub id: String,
    pub origin: String,
    pub assets: Vec<Asset>,
    pub offset: usize,
    pub has_more: bool,
    pub warnings: Vec<String>,
}

pub fn public_backlink(input: &str) -> Option<String> {
    let mut url = url::Url::parse(input).ok()?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

pub fn approved_referer(source: &str, media: &str) -> Option<String> {
    let source = url::Url::parse(source).ok()?;
    let media = url::Url::parse(media).ok()?;
    let source_host = source.host_str()?;
    let media_host = media.host_str()?;
    if (source_host == "pixiv.net" || source_host.ends_with(".pixiv.net"))
        && media.scheme() == "https"
        && (media_host == "pximg.net" || media_host.ends_with(".pximg.net"))
    {
        Some("https://www.pixiv.net/".into())
    } else {
        None
    }
}

pub fn supported(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .is_some_and(|host| {
            [
                "pinterest.com",
                "pin.it",
                "artstation.com",
                "deviantart.com",
                "pixiv.net",
                "flickr.com",
                "tumblr.com",
                "imgur.com",
                "instagram.com",
                "x.com",
                "twitter.com",
                "reddit.com",
                "bsky.app",
                "threads.net",
                "threads.com",
                "facebook.com",
            ]
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
        })
}

#[tauri::command]
pub async fn gallery_preview(
    state: State<'_, Arc<AppState>>,
    gallery_id: String,
    asset_id: String,
) -> Result<(String, Vec<u8>), String> {
    let (url,referer) = {
        let cache = state.galleries.lock().unwrap();
        cache
            .get(&gallery_id)
            .and_then(|g| g.assets.iter().find(|a| a.id == asset_id))
            .filter(|a| a.kind != "video")
            .map(|a| (a.url.clone(),a.referer.clone()))
            .ok_or("No image preview is available.")?
    };
    crate::direct::image_preview_with_referer(&url,referer.as_deref()).await
}
pub fn parse(raw: &str, source: &str, offset: usize, limit: usize) -> Result<Gallery, String> {
    let messages: Vec<Value> =
        serde_json::from_str(raw).map_err(|_| "The gallery engine returned invalid metadata.")?;
    let gallery_id = format!("gallery:{:x}", Sha256::digest(source.as_bytes()));
    let mut seen = HashSet::new();
    let mut assets = Vec::new();
    let mut inspected = 0usize;
    let mut warnings = Vec::new();
    for message in messages {
        if message[0] == -1 {
            return Err(crate::failure::sanitize(
                message[1]["message"]
                    .as_str()
                    .unwrap_or("The gallery extractor could not access this content."),
            ));
        }
        if message[0] != 3 {
            continue;
        }
        inspected += 1;
        if inspected > limit {
            continue;
        }
        let Some(url) = message[1].as_str() else {
            continue;
        };
        if url.starts_with("ytdl:") {
            if warnings.is_empty() {
                warnings.push("Some video attachments require an extractor manifest and cannot be downloaded through the direct gallery route. Supported files remain selectable.".into());
            }
            continue;
        }
        let url = crate::model::validate_url(url)?;
        let path = url::Url::parse(&url)
            .map_err(|_| "Invalid media URL.")?
            .path()
            .to_ascii_lowercase();
        if [".m3u8", ".mpd", ".ism", ".smil"]
            .iter()
            .any(|extension| path.ends_with(extension))
        {
            if warnings.is_empty() {
                warnings.push("Some video attachments only expose streaming manifests. Supported direct files remain selectable.".into());
            }
            continue;
        }
        let data = &message[2];
        let text = |keys: &[&str]| {
            keys.iter()
                .find_map(|key| data[*key].as_str())
                .map(|s| s.chars().take(512).collect::<String>())
        };
        let extension = data["extension"]
            .as_str()
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = match extension.as_str() {
            "mp4" | "webm" | "mov" | "mkv" => "video",
            "gif" | "webp" => "animation",
            "jpg" | "jpeg" | "png" | "avif" | "bmp" => "image",
            _ => continue,
        };
        let mut stable = url::Url::parse(&url).map_err(|_| "Invalid media URL.")?;
        let mut query:Vec<(String,String)>=stable.query_pairs().filter(|(key,_)|{let key=key.to_ascii_lowercase();!key.contains("token")&&!key.contains("signature")&&!key.starts_with("x-amz-")&&!key.starts_with("x-goog-")&&!matches!(key.as_str(),"sig"|"expires"|"policy"|"key-pair-id"|"auth"|"authorization")}).map(|(k,v)|(k.into_owned(),v.into_owned())).collect();query.sort();
        stable.set_query(None);if !query.is_empty(){stable.query_pairs_mut().extend_pairs(query);}
        stable.set_fragment(None);
        let id = format!(
            "gallery:{:x}",
            Sha256::digest(
                format!(
                    "{}|{}|{}|{}",
                    stable, extension, data["width"], data["height"]
                )
                .as_bytes()
            )
        );
        if !seen.insert(id.clone()) {
            continue;
        }
        assets.push(Asset {
            id,
            position: offset + inspected,
            title: text(&["title", "description", "filename"])
                .unwrap_or_else(|| format!("Media {}", offset + inspected)),
            author: text(&["author", "user", "username", "uploader"]),
            kind: kind.into(),
            width: data["width"].as_u64().filter(|v| *v > 0),
            height: data["height"].as_u64().filter(|v| *v > 0),
            date: text(&["date"]),
            referer: approved_referer(source, &url),
            source_url: public_backlink(source),
            url,
        });
    }
    let has_more = inspected > limit;
    Ok(Gallery {
        id: gallery_id,
        origin: url::Url::parse(source)
            .map_err(|_| "Invalid source.")?
            .origin()
            .ascii_serialization(),
        assets,
        offset,
        has_more,
        warnings,
    })
}

#[tauri::command]
pub async fn inspect_gallery(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    url: String,
    offset: usize,
    limit: usize,
) -> Result<Gallery, String> {
    if limit == 0 || limit > 100 || offset > 10000 {
        return Err("Inspect 1–100 files per page, up to 10000 positions.".into());
    }
    let url = crate::providers::validate_remote(&url).await?;
    if !supported(&url) {
        return Err("Choose a supported gallery source.".into());
    }
    let executable = crate::components::executable(&app, "gallery")?;
    let cookie_root = crate::config_root(&state)?;
    let cookies = crate::sessions::materialize(&cookie_root, &url)?;
    let config = crate::gallery_auth::materialize(&cookie_root, &url)?;
    let mut args = vec![
        "--ignore-config".into(),
        "--no-input".into(),
        "--no-postprocessors".into(),
        "--dump-json".into(),
        "--range".into(),
        format!("{}-{}", offset + 1, offset + limit + 1),
        "-o".into(),
        "extractor.retries=2".into(),
        "-o".into(),
        "extractor.timeout=20".into(),
    ];
    config.append_args(&mut args);
    if let Some(cookies) = &cookies {
        args.extend([
            "--cookies".into(),
            cookies.path.to_string_lossy().into_owned(),
        ]);
    }
    args.extend(["--".into(), url.clone()]);
    let (_sender, cancel) = watch::channel(false);
    let raw = engine::run_process(executable, args, cancel, 0, true, |_| {})
        .await
        .map_err(|error| config.redact(error))?;
    let gallery = parse(&raw, &url, offset, limit).map_err(|error| config.redact(error))?;
    let mut cache = state.galleries.lock().unwrap();
    if cache.len() >= 20 {
        cache.clear();
    }
    cache.insert(gallery.id.clone(), gallery.clone());
    Ok(gallery)
}
#[tauri::command]
pub fn queue_gallery(
    state: State<Arc<AppState>>,
    gallery_id: String,
    ids: Vec<String>,
    collection: String,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    let gallery = state
        .galleries
        .lock()
        .unwrap()
        .get(&gallery_id)
        .cloned()
        .ok_or("Inspect the gallery again before downloading.")?;
    if ids.is_empty() || ids.len() > 100 || collection.trim().is_empty() || collection.len() > 120 {
        return Err("Select files and a collection name of at most 120 characters.".into());
    }
    let assets = ids
        .iter()
        .map(|id| {
            gallery
                .assets
                .iter()
                .find(|asset| &asset.id == id)
                .cloned()
                .ok_or("Select only inspected attachments.")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    if repo
        .data
        .jobs
        .iter()
        .any(|job| job.client_request == client_request)
    {
        return Ok(repo
            .data
            .jobs
            .iter()
            .filter(|j| j.client_request == client_request)
            .cloned()
            .collect());
    }
    let assets: Vec<_> = assets
        .into_iter()
        .filter(|asset| {
            !repo.data.jobs.iter().any(|job| {
                job.content_id.as_deref() == Some(&asset.id)
                    && job.folder == repo.data.settings.folder
                    && retains_download(job.state, &job.outputs)
            })
        })
        .collect();
    if assets.is_empty() {
        return Ok(vec![]);
    }
    let jobs=queue::enqueue_tasks(repo,assets.iter().map(|asset|(queue::Source::Download{url:asset.url.clone(),url_ref:None,format:if asset.kind=="audio"{crate::model::Format::AudioOriginal}else{crate::model::Format::Source},height:None,audio_bitrate:None,subtitle_language:None},format!("{:04} {}",asset.position,asset.title))).collect(),&client_request)?;
    let mut result = Vec::new();
    for (job, asset) in jobs.iter().zip(&assets) {
        let job = repo
            .data
            .jobs
            .iter_mut()
            .find(|j| j.id == job.id)
            .ok_or("Queued job disappeared.")?;
        job.content_id = Some(asset.id.clone());
        job.provider=url::Url::parse(&gallery.origin).ok().and_then(|u|u.host_str().map(str::to_owned)).unwrap_or_else(||"Gallery".into());
        job.provenance=Some(queue::DownloadProvenance{creator:asset.author.clone(),source_url:asset.source_url.clone(),referer:asset.referer.clone(),position:asset.position});
        job.associations.push(queue::OutputAssociation {
            source_id: Some(gallery.id.clone()),
            collection: Some(collection.trim().into()),
            tags: asset.author.iter().cloned().collect(),
        });
        result.push(job.clone());
    }
    repo.save()?;
    state.runtime.notify.notify_one();
    Ok(result)
}

fn retains_download(state: queue::JobState, outputs: &[std::path::PathBuf]) -> bool {
    state.active()
        || matches!(state, queue::JobState::Pending | queue::JobState::Paused)
        || (state == queue::JobState::Completed
            && !outputs.is_empty()
            && outputs.iter().all(|path| path.is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_attachment_order_unknown_dimensions_and_stable_identity() {
        let raw = r#"[[2,{"category":"pinterest"}],[3,"https://i.pinimg.com/a.jpg?token=a",{"title":"First","extension":"jpg","width":800}],[3,"https://i.pinimg.com/b.mp4",{"title":"Second","extension":"mp4"}],[3,"https://i.pinimg.com/c.png",{"extension":"png"}]]"#;
        let gallery = parse(raw, "https://pinterest.com/a/board/", 10, 2).unwrap();
        assert_eq!(gallery.assets.len(), 2);
        assert!(gallery.has_more);
        assert_eq!(gallery.assets[0].position, 11);
        assert_eq!(gallery.assets[1].title, "Second");
        assert_eq!(gallery.assets[0].height, None);
        let changed = parse(
            &raw.replace("token=a", "token=b"),
            "https://pinterest.com/a/board/",
            10,
            2,
        )
        .unwrap();
        assert_eq!(gallery.assets[0].id, changed.assets[0].id);
        let public = serde_json::to_string(&gallery).unwrap();
        assert!(!public.contains("token="));
        assert!(!supported("https://pinterest.com.attacker.org/a"));
    }
    #[test]
    fn cursor_counts_unsupported_files_and_keeps_supported_mixed_attachments() {
        let raw = r#"[[3,"https://i.pximg.net/frames.zip",{"extension":"zip"}],[3,"ytdl:https://instagram.com/p/a/2.mp4",{"extension":"mp4"}],[3,"https://cdn.example.org/a.jpg",{"extension":"jpg"}],[3,"https://cdn.example.org/b.jpg",{"extension":"jpg"}]]"#;
        let page = parse(raw, "https://www.instagram.com/p/a/", 20, 3).unwrap();
        assert_eq!(page.assets.len(), 1);
        assert_eq!(page.assets[0].position, 23);
        assert!(page.has_more);
        assert_eq!(page.warnings.len(), 1);
        assert_eq!(
            page.assets[0].source_url.as_deref(),
            Some("https://www.instagram.com/p/a/")
        );
    }
    #[test]
    fn errors_cannot_masquerade_as_an_empty_gallery() {
        assert!(parse(
            r#"[[-1,{"message":"Access denied"}]]"#,
            "https://pinterest.com/a/",
            0,
            50
        )
        .err()
        .unwrap()
        .contains("Access denied"));
    }
    #[test]
    fn only_pixiv_images_receive_fixed_public_referer() {
        assert_eq!(
            approved_referer(
                "https://www.pixiv.net/artworks/1?token=secret",
                "https://i.pximg.net/a.jpg"
            )
            .as_deref(),
            Some("https://www.pixiv.net/")
        );
        assert_eq!(
            approved_referer("https://pixiv.net.evil.test/1", "https://i.pximg.net/a.jpg"),
            None
        );
        assert_eq!(
            approved_referer("https://pixiv.net/1", "https://pximg.net.evil.test/a.jpg"),
            None
        );
        let page = parse(
            r#"[[3,"https://i.pximg.net/a.jpg?token=secret",{"extension":"jpg"}]]"#,
            "https://pixiv.net/artworks/1?auth=secret#secret",
            0,
            1,
        )
        .unwrap();
        assert!(!serde_json::to_string(&page).unwrap().contains("secret"));
        assert_eq!(
            page.assets[0].source_url.as_deref(),
            Some("https://pixiv.net/artworks/1")
        );
    }
    #[test]
    fn completed_download_only_blocks_while_output_still_exists() {
        let path = std::env::temp_dir().join(format!(
            "nabnab-gallery-output-test-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        assert!(retains_download(
            queue::JobState::Completed,
            std::slice::from_ref(&path)
        ));
        std::fs::remove_file(&path).unwrap();
        assert!(!retains_download(queue::JobState::Completed, &[path]));
        assert!(!retains_download(queue::JobState::Completed, &[]));
        assert!(retains_download(queue::JobState::Pending, &[]));
        assert!(!retains_download(queue::JobState::Error, &[]));
    }
}
