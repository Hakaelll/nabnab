use crate::{
    galleries::{Asset, Gallery},
    AppState,
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tauri::State;

const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FederatedKind {
    Peertube,
    Mastodon,
}

#[derive(Debug, PartialEq, Eq)]
struct Target {
    origin: String,
    id: String,
    playlist: bool,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
fn text(value: &str, limit: usize) -> String {
    value
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

fn target(input: &str, kind: FederatedKind) -> Result<Target, String> {
    let normalized = crate::model::validate_url(input)?;
    let parsed =
        url::Url::parse(&normalized).map_err(|_| "Use a complete federated post or video URL.")?;
    let parts: Vec<_> = parsed
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
    let (id, playlist) = match (kind, parts.as_slice()) {
        (FederatedKind::Peertube, ["w", id])
        | (FederatedKind::Peertube, ["videos", "watch", id]) => (*id, false),
        (FederatedKind::Peertube, ["video-playlists", "watch", id])
        | (FederatedKind::Peertube, ["w", "p", id]) => (*id, true),
        (FederatedKind::Mastodon, [account, id]) if account.starts_with('@') => (*id, false),
        (FederatedKind::Mastodon, ["users", _, "statuses", id]) => (*id, false),
        _ => return Err(match kind {
            FederatedKind::Peertube => {
                "Use a PeerTube video or playlist page from the instance you approve."
            }
            FederatedKind::Mastodon => {
                "Use a Mastodon status URL, such as /@account/123 or /users/account/statuses/123."
            }
        }
        .into()),
    };
    if !identifier(id) || kind == FederatedKind::Mastodon && !id.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("The federated content identifier is invalid.".into());
    }
    Ok(Target {
        origin: parsed.origin().ascii_serialization(),
        id: id.into(),
        playlist,
    })
}

async fn public_json(
    input: &str,
    origin: &str,
    bearer: Option<&crate::federated_auth::Bearer>,
) -> Result<Value, String> {
    let mut url = url::Url::parse(&crate::model::validate_url(input)?)
        .map_err(|_| "Invalid instance API URL.")?;
    for hop in 0..=5 {
        if url.origin().ascii_serialization() != origin {
            return Err("The instance API cannot redirect to another origin. Approve that instance separately.".into());
        }
        let host = url.host_str().ok_or("The instance API URL has no host.")?;
        let port = url
            .port_or_known_default()
            .ok_or("Unsupported instance API scheme.")?;
        let addresses: Vec<_> = tokio::time::timeout(
            Duration::from_secs(8),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| "Instance hostname lookup timed out.")?
        .map_err(|_| "The instance hostname could not be resolved.")?
        .collect();
        if addresses.is_empty()
            || addresses
                .iter()
                .any(|address| !crate::providers::is_public_ip(address.ip()))
        {
            return Err("Local or reserved instance destinations are not supported.".into());
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addresses)
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(30))
            .user_agent("nabnab public media inspector")
            .build()
            .map_err(|_| "Could not prepare the instance API request.")?;
        let mut request = client
            .get(url.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::ACCEPT_ENCODING, "identity");
        if let Some(bearer) = bearer {
            request = request.header(reqwest::header::AUTHORIZATION, bearer.header_for(&url)?);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| "The instance API request failed. Check the connection and retry.")?;
        if response.status().is_redirection() {
            if hop == 5 {
                return Err("The instance exceeded the five-redirect limit.".into());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or("The instance returned an invalid redirect.")?;
            let next = url
                .join(location)
                .map_err(|_| "The instance returned an invalid redirect URL.")?;
            crate::model::validate_url(next.as_str())?;
            url = next;
            continue;
        }
        match response.status().as_u16() {
            401 | 403 => return Err("The instance requires a valid authorized user token with read:statuses scope, or denies this content. No account action or access bypass was attempted.".into()),
            404 => return Err("This instance did not return the requested public video, playlist or status.".into()),
            429 => return Err("The instance requested a pause. Retry later; no downloads were queued.".into()),
            _ if !response.status().is_success() => return Err(format!("The instance returned HTTP {}.",response.status().as_u16())),
            _ => {}
        }
        if response
            .content_length()
            .is_some_and(|size| size > MAX_JSON_BYTES as u64)
        {
            return Err("The instance API response exceeds 8 MiB.".into());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The instance API transfer was interrupted.")?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_JSON_BYTES {
                return Err("The instance API response exceeds 8 MiB.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        return serde_json::from_slice(&bytes)
            .map_err(|_| "The instance did not return valid public API JSON.".into());
    }
    Err("The instance redirect limit was exceeded.".into())
}

fn public_media(origin: &str, value: &str) -> Option<String> {
    let resolved = url::Url::parse(origin).ok()?.join(value).ok()?;
    crate::model::validate_url(resolved.as_str()).ok()
}
fn positive(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .filter(|value| *value > 0 && *value <= 1_000_000)
}

fn peertube_asset(value: &Value, origin: &str, position: usize) -> Result<Option<Asset>, String> {
    if value["isLive"] == true {
        return Err("Active PeerTube streams require a recording workflow; they are not presented as finished video downloads.".into());
    }
    if value["downloadEnabled"] == false {
        return Ok(None);
    }
    let video_id = value["uuid"]
        .as_str()
        .or_else(|| value["shortUUID"].as_str())
        .map(str::to_owned)
        .or_else(|| value["id"].as_u64().map(|v| v.to_string()))
        .filter(|id| identifier(id))
        .ok_or("The instance returned no stable video identifier.")?;
    let mut variants = Vec::new();
    let root_files = value["files"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let streaming_files = value["streamingPlaylists"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|playlist| playlist["files"].as_array().into_iter().flatten());
    for file in root_files.iter().chain(streaming_files) {
        let Some(url) = file["fileDownloadUrl"]
            .as_str()
            .or_else(|| file["fileUrl"].as_str())
            .and_then(|value| public_media(origin, value))
        else {
            continue;
        };
        let path = url::Url::parse(&url)
            .ok()
            .map(|url| url.path().to_lowercase())
            .unwrap_or_default();
        if path.ends_with(".m3u8") || path.ends_with(".mpd") || file["has_drm"] == true {
            continue;
        }
        let height = positive(&file["height"]).or_else(|| positive(&file["resolution"]["id"]));
        let width = positive(&file["width"]);
        let kind = if file["hasVideo"] == false && file["hasAudio"] == true
            || [".mp3", ".m4a", ".opus", ".ogg", ".flac", ".wav"]
                .iter()
                .any(|extension| path.ends_with(extension))
        {
            "audio"
        } else {
            "video"
        };
        variants.push((height.unwrap_or(0), width, height, kind, url));
    }
    variants.sort_by_key(|variant| variant.0);
    let Some((_, width, height, kind, url)) = variants.pop() else {
        return Ok(None);
    };
    Ok(Some(Asset {
        id: format!(
            "federated:{}",
            hash(&format!("{origin}\0peertube\0{video_id}"))
        ),
        position,
        title: text(value["name"].as_str().unwrap_or("PeerTube video"), 512),
        author: value["account"]["displayName"]
            .as_str()
            .or_else(|| value["account"]["name"].as_str())
            .map(|v| text(v, 512)),
        kind: kind.into(),
        width,
        height,
        date: value["publishedAt"].as_str().map(|v| text(v, 64)),
        referer: None,
        source_url: None,
        url,
    }))
}

fn mastodon_gallery(
    value: &Value,
    target: &Target,
    offset: usize,
    limit: usize,
    authenticated: bool,
) -> Result<Gallery, String> {
    let status = if value["reblog"].is_object() {
        &value["reblog"]
    } else {
        value
    };
    if !matches!(status["visibility"].as_str(), Some("public" | "unlisted"))
        && !(authenticated && matches!(status["visibility"].as_str(), Some("private" | "direct")))
    {
        return Err("Only public or unlisted statuses can be inspected without an authorized Mastodon account.".into());
    }
    let attachments = status["media_attachments"]
        .as_array()
        .ok_or("The Mastodon status did not return a media attachment list.")?;
    let status_id = status["uri"]
        .as_str()
        .or_else(|| status["id"].as_str())
        .ok_or("The Mastodon status has no stable identity.")?;
    let mut assets = Vec::new();
    let mut known = HashSet::new();
    for (position, attachment) in attachments.iter().enumerate().skip(offset).take(limit) {
        let Some(id) = attachment["id"].as_str().filter(|id| identifier(id)) else {
            continue;
        };
        let kind = match attachment["type"].as_str() {
            Some("image") => "image",
            Some("video") => "video",
            Some("gifv") => "animation",
            Some("audio") => "audio",
            _ => continue,
        };
        let Some(url) = attachment["url"]
            .as_str()
            .or_else(|| attachment["remote_url"].as_str())
            .and_then(|url| public_media(&target.origin, url))
        else {
            continue;
        };
        let identity = format!(
            "federated:{}",
            hash(&format!("{}\0mastodon\0{status_id}\0{id}", target.origin))
        );
        if !known.insert(identity.clone()) {
            continue;
        }
        assets.push(Asset {
            id: identity,
            position: position + 1,
            title: attachment["description"]
                .as_str()
                .filter(|v| !v.trim().is_empty())
                .map(|v| text(v, 512))
                .unwrap_or_else(|| format!("Media {}", position + 1)),
            author: status["account"]["acct"]
                .as_str()
                .or_else(|| status["account"]["display_name"].as_str())
                .map(|v| text(v, 512)),
            kind: kind.into(),
            width: positive(&attachment["meta"]["original"]["width"]),
            height: positive(&attachment["meta"]["original"]["height"]),
            date: status["created_at"].as_str().map(|v| text(v, 64)),
            referer: None,
            source_url: None,
            url,
        });
    }
    Ok(Gallery {
        id: format!(
            "federated:{}",
            hash(&format!("{}\0mastodon\0{}", target.origin, target.id))
        ),
        origin: target.origin.clone(),
        assets,
        offset,
        has_more: offset.saturating_add(limit) < attachments.len(),
        warnings: vec![],
    })
}

async fn inspect(
    target: &Target,
    kind: FederatedKind,
    offset: usize,
    limit: usize,
    root: &std::path::Path,
) -> Result<Gallery, String> {
    match kind {
        FederatedKind::Mastodon => {
            let bearer = if target.origin.starts_with("https://") {
                crate::federated_auth::load(root, &target.origin)?
            } else {
                None
            };
            let value = public_json(
                &format!("{}/api/v1/statuses/{}", target.origin, target.id),
                &target.origin,
                bearer.as_ref(),
            )
            .await?;
            mastodon_gallery(&value, target, offset, limit, bearer.is_some())
        }
        FederatedKind::Peertube if target.playlist => {
            let value = public_json(
                &format!(
                    "{}/api/v1/video-playlists/{}/videos?start={offset}&count={limit}",
                    target.origin, target.id
                ),
                &target.origin,
                None,
            )
            .await?;
            let entries = value["data"]
                .as_array()
                .ok_or("The instance did not return a PeerTube playlist page.")?;
            let mut assets = Vec::new();
            let mut known = HashSet::new();
            for (index, entry) in entries.iter().take(limit).enumerate() {
                let video = &entry["video"];
                let Some(id) = video["uuid"]
                    .as_str()
                    .or_else(|| video["shortUUID"].as_str())
                    .map(str::to_owned)
                    .or_else(|| video["id"].as_u64().map(|v| v.to_string()))
                    .filter(|id| identifier(id))
                else {
                    continue;
                };
                let detail = public_json(
                    &format!("{}/api/v1/videos/{id}", target.origin),
                    &target.origin,
                    None,
                )
                .await?;
                let position = entry["position"]
                    .as_u64()
                    .and_then(|p| usize::try_from(p).ok())
                    .unwrap_or(offset + index + 1);
                if let Some(asset) = peertube_asset(&detail, &target.origin, position)? {
                    if known.insert(asset.id.clone()) {
                        assets.push(asset);
                    }
                }
            }
            let total = value["total"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok());
            let has_more = total
                .map(|total| offset.saturating_add(entries.len()) < total)
                .unwrap_or(entries.len() == limit);
            Ok(Gallery {
                id: format!(
                    "federated:{}",
                    hash(&format!(
                        "{}\0peertube-playlist\0{}",
                        target.origin, target.id
                    ))
                ),
                origin: target.origin.clone(),
                assets,
                offset,
                has_more,
                warnings: vec![],
            })
        }
        FederatedKind::Peertube => {
            let value = public_json(
                &format!("{}/api/v1/videos/{}", target.origin, target.id),
                &target.origin,
                None,
            )
            .await?;
            let assets = if offset == 0 {
                peertube_asset(&value, &target.origin, 1)?
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
            Ok(Gallery {
                id: format!(
                    "federated:{}",
                    hash(&format!("{}\0peertube\0{}", target.origin, target.id))
                ),
                origin: target.origin.clone(),
                assets,
                offset,
                has_more: false,
                warnings: vec![],
            })
        }
    }
}

#[tauri::command]
pub async fn inspect_federated(
    state: State<'_, Arc<AppState>>,
    url: String,
    kind: FederatedKind,
    consent: bool,
    offset: usize,
    limit: usize,
) -> Result<Gallery, String> {
    if !consent {
        return Err("Approve contact with this instance for this inspection. It will not become a trusted account origin.".into());
    }
    if !(1..=50).contains(&limit) || offset > 10000 {
        return Err("Inspect 1–50 federated items per page, up to 10000 positions.".into());
    }
    let target = target(&url, kind)?;
    let root = crate::config_root(&state)?;
    let gallery = tokio::time::timeout(
        Duration::from_secs(120),
        inspect(&target, kind, offset, limit, &root),
    )
    .await
    .map_err(|_| "The federated inspection exceeded two minutes. Try a smaller page.")??;
    let mut cache = state.galleries.lock().unwrap();
    if cache.len() >= 20 {
        cache.clear();
    }
    cache.insert(gallery.id.clone(), gallery.clone());
    Ok(gallery)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_instance_paths_without_a_fixed_domain_list() {
        let first = target(
            "https://video.example.org/w/aB3Cd4?token=secret",
            FederatedKind::Peertube,
        )
        .unwrap();
        assert_eq!(
            first,
            Target {
                origin: "https://video.example.org".into(),
                id: "aB3Cd4".into(),
                playlist: false
            }
        );
        assert!(
            target(
                "https://another.example.org/video-playlists/watch/playlist-id",
                FederatedKind::Peertube
            )
            .unwrap()
            .playlist
        );
        assert!(
            target(
                "https://another.example.org/w/p/playlist-id",
                FederatedKind::Peertube
            )
            .unwrap()
            .playlist
        );
        assert_eq!(
            target(
                "https://social.example.org/@account/123456",
                FederatedKind::Mastodon
            )
            .unwrap()
            .id,
            "123456"
        );
        assert_eq!(
            target(
                "https://social.example.org/users/account/statuses/789",
                FederatedKind::Mastodon
            )
            .unwrap()
            .id,
            "789"
        );
        assert!(target("http://127.0.0.1/w/secret", FederatedKind::Peertube).is_err());
        assert!(target(
            "https://social.example.org/@account/not-a-number",
            FederatedKind::Mastodon
        )
        .is_err());
        assert!(target(
            "https://social.example.org/@account/123/other",
            FederatedKind::Mastodon
        )
        .is_err());
    }
    #[test]
    fn peertube_chooses_reported_best_direct_file_and_respects_disabled_downloads() {
        let value = serde_json::json!({"uuid":"abc-123","name":"A real video","downloadEnabled":true,"files":[{"resolution":{"id":360},"fileDownloadUrl":"https://cdn.example.org/360.mp4"},{"resolution":{"id":1080},"fileDownloadUrl":"https://cdn.example.org/1080.mp4?token=secret"}],"account":{"displayName":"Author"}});
        let asset = peertube_asset(&value, "https://video.example.org", 7)
            .unwrap()
            .unwrap();
        assert_eq!(asset.position, 7);
        assert_eq!(asset.height, Some(1080));
        assert_eq!(asset.width, None);
        assert_eq!(asset.author.as_deref(), Some("Author"));
        assert_eq!(asset.url, "https://cdn.example.org/1080.mp4?token=secret");
        assert!(!serde_json::to_string(&asset)
            .unwrap()
            .contains("token=secret"));
        let mut disabled = value;
        disabled["downloadEnabled"] = Value::Bool(false);
        assert!(peertube_asset(&disabled, "https://video.example.org", 1)
            .unwrap()
            .is_none());
        disabled["isLive"] = Value::Bool(true);
        assert!(peertube_asset(&disabled, "https://video.example.org", 1).is_err());
    }
    #[test]
    fn mastodon_keeps_all_supported_attachment_identities_without_secret_urls() {
        let value = serde_json::json!({"id":"123","uri":"https://social.example.org/users/a/statuses/123","visibility":"public","account":{"acct":"a@example.org"},"media_attachments":[{"id":"1","type":"image","url":"https://cdn.example.org/image.jpg?token=one","description":"First","meta":{"original":{"width":800,"height":600}}},{"id":"2","type":"gifv","url":"https://cdn.example.org/animation.mp4"},{"id":"3","type":"audio","url":"https://cdn.example.org/audio.ogg"}]});
        let target = target("https://social.example.org/@a/123", FederatedKind::Mastodon).unwrap();
        let gallery = mastodon_gallery(&value, &target, 0, 2, false).unwrap();
        assert_eq!(gallery.assets.len(), 2);
        assert!(gallery.has_more);
        assert_eq!(gallery.assets[1].kind, "animation");
        assert_eq!(gallery.assets[0].width, Some(800));
        let public = serde_json::to_string(&gallery).unwrap();
        assert!(!public.contains("token=one"));
        assert!(!public.contains("cdn.example.org"));
        let changed = mastodon_gallery(
            &serde_json::from_str::<Value>(&value.to_string().replace("token=one", "token=two"))
                .unwrap(),
            &target,
            0,
            2,
            false,
        )
        .unwrap();
        assert_eq!(gallery.assets[0].id, changed.assets[0].id);
        let later = mastodon_gallery(&value, &target, 2, 2, false).unwrap();
        assert_eq!(later.assets[0].kind, "audio");
        assert!(!later.has_more);
        let mut private = value;
        private["visibility"] = Value::String("private".into());
        assert!(mastodon_gallery(&private, &target, 0, 50, false).is_err());
        assert_eq!(
            mastodon_gallery(&private, &target, 0, 50, true)
                .unwrap()
                .assets
                .len(),
            3
        );
    }
}
