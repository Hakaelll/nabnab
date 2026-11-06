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
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gallery {
    pub id: String,
    pub origin: String,
    pub assets: Vec<Asset>,
    pub offset: usize,
    pub has_more: bool,
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
pub async fn gallery_preview(state: State<'_,Arc<AppState>>, gallery_id:String, asset_id:String)->Result<(String,Vec<u8>),String>{
    let url={let cache=state.galleries.lock().unwrap();cache.get(&gallery_id).and_then(|g|g.assets.iter().find(|a|a.id==asset_id)).filter(|a|a.kind!="video").map(|a|a.url.clone()).ok_or("No image preview is available.")?};
    crate::direct::image_preview(&url).await
}
pub fn parse(raw: &str, source: &str, offset: usize, limit: usize) -> Result<Gallery, String> {
    let messages: Vec<Value> =
        serde_json::from_str(raw).map_err(|_| "The gallery engine returned invalid metadata.")?;
    let gallery_id = format!("gallery:{:x}", Sha256::digest(source.as_bytes()));
    let mut seen = HashSet::new();
    let mut assets = Vec::new();
    for message in messages {
        if message[0] != 3 {
            continue;
        }
        let Some(url) = message[1].as_str() else {
            continue;
        };
        let url = crate::model::validate_url(url)?;
        if !seen.insert(url.clone()) {
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
        stable.set_query(None);
        stable.set_fragment(None);
        let id = format!(
            "gallery:{:x}",
            Sha256::digest(format!("{gallery_id}|{stable}").as_bytes())
        );
        assets.push(Asset {
            id,
            position: offset + assets.len() + 1,
            title: text(&["title", "description", "filename"])
                .unwrap_or_else(|| format!("Media {}", offset + assets.len() + 1)),
            author: text(&["author", "user", "username", "uploader"]),
            kind: kind.into(),
            width: data["width"].as_u64().filter(|v| *v > 0),
            height: data["height"].as_u64().filter(|v| *v > 0),
            date: text(&["date"]),
            url,
        });
    }
    let has_more = assets.len() > limit;
    assets.truncate(limit);
    Ok(Gallery {
        id: gallery_id,
        origin: url::Url::parse(source)
            .map_err(|_| "Invalid source.")?
            .origin()
            .ascii_serialization(),
        assets,
        offset,
        has_more,
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
    let mut args = vec![
        "--ignore-config".into(),
        "--no-input".into(),
        "--dump-json".into(),
        "--range".into(),
        format!("{}-{}", offset + 1, offset + limit + 1),
        "-o".into(),
        "extractor.retries=2".into(),
        "-o".into(),
        "extractor.timeout=20".into(),
    ];
    if let Some(cookies) = &cookies {
        args.extend([
            "--cookies".into(),
            cookies.path.to_string_lossy().into_owned(),
        ]);
    }
    args.extend(["--".into(), url.clone()]);
    let (_sender, cancel) = watch::channel(false);
    let raw = engine::run_process(executable, args, cancel, 0, true, |_| {}).await?;
    let gallery = parse(&raw, &url, offset, limit)?;
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
                    && !matches!(
                        job.state,
                        queue::JobState::Error | queue::JobState::Cancelled
                    )
            })
        })
        .collect();
    if assets.is_empty() {
        return Ok(vec![]);
    }
    let jobs = queue::enqueue_download(
        repo,
        assets.iter().map(|a| a.url.clone()).collect(),
        assets
            .iter()
            .map(|a| format!("{:04} {}", a.position, a.title))
            .collect(),
        crate::model::Format::Source,
        None,
        None,
        None,
        &client_request,
    )?;
    let mut result = Vec::new();
    for (job, asset) in jobs.iter().zip(&assets) {
        let job = repo
            .data
            .jobs
            .iter_mut()
            .find(|j| j.id == job.id)
            .ok_or("Queued job disappeared.")?;
        job.content_id = Some(asset.id.clone());
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
}
