use crate::options::display_url as public_display_url;
use crate::{engine, failure::Failure, model, options, providers, queue, sessions, AppState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

const MAX_DRAFT_ITEMS: usize = 10_000;
const PAGE_SIZE: usize = 50;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ItemState {
    Pending,
    Ready,
    Available,
    Active,
    Authentication,
    Unsupported,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListProgress {
    pub loaded: usize,
    pub complete: bool,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftItem {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub url_ref: Option<String>,
    pub title: String,
    pub provider: String,
    pub creator: String,
    #[serde(default)]
    pub list_name: String,
    pub content_id: String,
    pub thumbnail: Option<String>,
    pub duration: Option<f64>,
    pub state: ItemState,
    pub picked: bool,
    pub excluded: bool,
    pub options: Option<options::WorkOptions>,
    pub resolved: Option<options::ResolvedOptions>,
    pub destination: Option<String>,
    pub error: Option<String>,
    pub list: Option<ListProgress>,
    pub media: Option<model::Media>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub saved_for_later: bool,
    pub preset_id: Option<String>,
    pub options: Option<options::WorkOptions>,
    pub items: Vec<DraftItem>,
    pub exploration_limit: usize,
}

fn identifier(prefix: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{prefix}-{}-{}",
        queue::now(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn candidate(url: &str) -> DraftItem {
    let identified = providers::identify(url);
    let valid = url.len() <= 8192 && identified.is_ok();
    let display = if valid {
        url.to_owned()
    } else {
        public_display_url(url)
    };
    DraftItem {
        id: identifier("item"),
        url: display,
        url_ref: None,
        title: public_title(url),
        provider: identified.map(|p| p.name).unwrap_or_default(),
        creator: String::new(),
        list_name: String::new(),
        content_id: options::content_identity(url).unwrap_or_default(),
        thumbnail: None,
        duration: None,
        state: if valid {
            ItemState::Pending
        } else {
            ItemState::Unsupported
        },
        picked: valid,
        excluded: false,
        options: None,
        resolved: None,
        destination: None,
        error: (!valid)
            .then(|| "Use a complete public HTTP or HTTPS link without credentials.".into()),
        list: None,
        media: None,
    }
}

fn public_title(input: &str) -> String {
    if let Ok(provider) = providers::identify(input) {
        return if provider.id == providers::ProviderId::Generic {
            provider.host
        } else {
            provider.name
        };
    }
    url::Url::parse(input)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "Unsupported input".into())
}

fn public_list_url(input: &str) -> Option<String> {
    if providers::identify(input).ok()?.id != providers::ProviderId::Youtube {
        return None;
    }
    let parsed = url::Url::parse(input).ok()?;
    let (_, id) = parsed.query_pairs().find(|(key, id)| {
        key == "list"
            && !id.is_empty()
            && id.len() <= 150
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    })?;
    Some(format!("https://www.youtube.com/playlist?list={id}"))
}

fn public_origin(input: &str) -> String {
    url::Url::parse(input)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_default()
}

fn protect_thumbnail(thumbnail: &mut Option<String>) {
    if thumbnail.as_ref().is_some_and(|value| {
        url::Url::parse(value).map_or(true, |url| {
            url.scheme() != "https"
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
        })
    }) {
        *thumbnail = None;
    }
}

pub fn operational_url(root: &Path, item: &DraftItem) -> Result<sessions::OperationalUrl, String> {
    let reference = match &item.url_ref {
        Some(reference) => reference.clone(),
        None => sessions::seal_url(root, &format!("batch-item:{}", item.id), &item.url)?,
    };
    sessions::resolve_url(root, &reference)
}

pub fn protect_drafts(root: &Path, drafts: &mut [Draft]) -> Result<(), String> {
    for draft in drafts {
        if draft.name.contains("://") {
            draft.name = public_title(&draft.name);
        }
        for item in &mut draft.items {
            let operational = if item.url_ref.is_none() && model::validate_url(&item.url).is_err() {
                None
            } else {
                Some(operational_url(root, item)?)
            };
            let source = operational
                .as_ref()
                .map_or(item.url.as_str(), |url| url.as_str());
            let display = if item.list.is_some() {
                public_list_url(source).unwrap_or_else(|| public_display_url(source))
            } else {
                public_display_url(source)
            };
            let origin = public_origin(source);
            let title = public_title(source);
            let identity = if operational.is_some() {
                options::content_identity(source)?
            } else {
                String::new()
            };
            let reference = if let Some(reference) = &item.url_ref {
                Some(reference.clone())
            } else if operational.is_some() {
                Some(sessions::seal_url(
                    root,
                    &format!("batch-item:{}", item.id),
                    source,
                )?)
            } else {
                None
            };
            let thumbnail_is_source = item.thumbnail.as_deref() == Some(source);
            let media_thumbnail_is_source = item
                .media
                .as_ref()
                .and_then(|media| media.thumbnail.as_deref())
                == Some(source);
            if item.title.contains("://") {
                item.title = title;
                item.destination = None;
            }
            if item.content_id != identity {
                item.destination = None;
            }
            item.content_id = identity;
            item.url_ref = reference;
            item.url = display;
            if thumbnail_is_source {
                item.thumbnail = None;
            }
            protect_thumbnail(&mut item.thumbnail);
            if item.creator.contains("://") {
                item.creator = public_title(&item.creator);
            }
            if item.list_name.contains("://") {
                item.list_name = public_title(&item.list_name);
            }
            if let Some(list) = &mut item.list {
                if list.name.contains("://") {
                    list.name = public_title(&list.name);
                }
            }
            if let Some(resolved) = &mut item.resolved {
                resolved.context.url = origin;
                resolved.context.source_url = public_origin(&resolved.context.source_url);
                resolved.context.content_id = item.content_id.clone();
                if resolved.context.title.contains("://") {
                    resolved.context.title = item.title.clone();
                    item.destination = None;
                }
                if resolved.context.list.contains("://") {
                    resolved.context.list = public_title(&resolved.context.list);
                    item.destination = None;
                }
                if resolved.context.creator.contains("://") {
                    resolved.context.creator = public_title(&resolved.context.creator);
                    item.destination = None;
                }
            }
            if let Some(media) = &mut item.media {
                if media.title.contains("://") {
                    media.title = item.title.clone();
                }
                if media_thumbnail_is_source {
                    media.thumbnail = None;
                }
                protect_thumbnail(&mut media.thumbnail);
                if let Some(uploader) = &mut media.uploader {
                    if uploader.contains("://") {
                        *uploader = public_title(uploader);
                    }
                }
                for chapter in &mut media.chapters {
                    if chapter.title.contains("://") {
                        chapter.title = public_title(&chapter.title);
                    }
                }
                for entry in &mut media.entries {
                    entry.url = public_display_url(&entry.url);
                    if entry.title.contains("://") {
                        entry.title = public_title(&entry.url);
                    }
                    protect_thumbnail(&mut entry.thumbnail);
                }
            }
        }
    }
    Ok(())
}

fn candidates(text: &str) -> Result<Vec<DraftItem>, String> {
    if text.len() > 1024 * 1024 {
        return Err("Import at most 1 MB of links at a time.".into());
    }
    let mut seen = HashSet::new();
    let items: Vec<_> = text
        .split_whitespace()
        .filter(|url| seen.insert(*url))
        .map(candidate)
        .collect();
    if items.is_empty() || items.len() > MAX_DRAFT_ITEMS {
        return Err("Choose between 1 and 10000 links for this draft.".into());
    }
    Ok(items)
}

fn context(root: &Path, item: &DraftItem, list: &str) -> Result<options::Context, String> {
    let url = operational_url(root, item)?;
    Ok(options::Context {
        url: url.as_str().into(),
        source_url: String::new(),
        title: item.title.clone(),
        creator: item.creator.clone(),
        list: if item.list_name.is_empty() {
            list.into()
        } else {
            item.list_name.clone()
        },
        content_id: item.content_id.clone(),
        platform: item.provider.clone(),
        content_type: if item
            .media
            .as_ref()
            .is_some_and(|m| !m.has_video && m.has_audio)
        {
            "audio"
        } else {
            "video"
        }
        .into(),
        tags: Vec::new(),
    })
}

fn resolve_item(
    repo: &crate::store::Repository,
    draft: &Draft,
    item: &mut DraftItem,
) -> Result<(), String> {
    let ctx = context(repo.root(), item, &draft.name)?;
    let mut resolved = options::resolve(
        repo,
        &ctx,
        draft.preset_id.as_deref(),
        item.options.as_ref().or(draft.options.as_ref()),
    )?;
    resolved.context.url = public_origin(&ctx.url);
    resolved.context.source_url = public_origin(&resolved.context.source_url);
    item.destination = Some(
        options::preview_path(&repo.data.settings.folder, &ctx, &resolved.values)?
            .to_string_lossy()
            .into_owned(),
    );
    let prepared = options::PreparedDownload {
        url: item.url.clone(),
        title: item.title.clone(),
        content_id: item.content_id.clone(),
        options: resolved.clone(),
        source_id: None,
        group_id: Some(draft.id.clone()),
    };
    if item.state == ItemState::Ready
        || item.state == ItemState::Available
        || item.state == ItemState::Active
    {
        item.state = match queue::prepared_status(repo, &prepared) {
            Some("active") => ItemState::Active,
            Some("available") => ItemState::Available,
            _ => ItemState::Ready,
        };
    }
    item.resolved = Some(resolved);
    Ok(())
}

fn changed<T: Serialize>(a: &T, b: &T) -> bool {
    serde_json::to_value(a).ok() != serde_json::to_value(b).ok()
}

fn commit_draft(repo: &mut crate::store::Repository, mut draft: Draft) -> Result<Draft, String> {
    let previous = repo.data.drafts.clone();
    protect_drafts(repo.root(), std::slice::from_mut(&mut draft))?;
    draft.updated_at = queue::now().max(draft.updated_at);
    if let Some(old) = previous.iter().find(|old| old.id == draft.id) {
        draft.updated_at = draft.updated_at.max(old.updated_at.saturating_add(1));
    }
    if let Some(old) = repo.data.drafts.iter_mut().find(|d| d.id == draft.id) {
        *old = draft.clone();
    } else {
        repo.data.drafts.push(draft.clone());
    }
    if let Err(error) = repo.save() {
        repo.data.drafts = previous;
        return Err(error);
    }
    Ok(draft)
}

fn new_draft(
    repo: &crate::store::Repository,
    text: &str,
    name: Option<String>,
    preset_id: Option<String>,
) -> Result<Draft, String> {
    if preset_id
        .as_ref()
        .is_some_and(|id| !repo.data.presets.iter().any(|p| &p.id == id))
    {
        return Err("The selected preset no longer exists.".into());
    }
    let stamp = queue::now();
    Ok(Draft {
        id: identifier("batch"),
        name: name
            .unwrap_or_else(|| "Lote de enlaces".into())
            .chars()
            .take(160)
            .collect(),
        created_at: stamp,
        updated_at: stamp,
        saved_for_later: false,
        preset_id,
        options: None,
        items: candidates(text)?,
        exploration_limit: 200,
    })
}

pub fn create(
    repo: &mut crate::store::Repository,
    text: &str,
    name: Option<String>,
    preset_id: Option<String>,
) -> Result<Draft, String> {
    let draft = new_draft(repo, text, name, preset_id)?;
    commit_draft(repo, draft)
}

pub fn create_for_request(
    repo: &mut crate::store::Repository,
    text: &str,
    preset_id: Option<String>,
    request: &str,
    saved_for_later: bool,
) -> Result<Draft, String> {
    queue::validate_request_id(request)?;
    let id = format!("browser-{:x}", Sha256::digest(request.as_bytes()));
    if let Some(existing) = repo.data.drafts.iter().find(|draft| draft.id == id) {
        return Ok(existing.clone());
    }
    let mut draft = new_draft(repo, text, None, preset_id)?;
    draft.id = id;
    draft.saved_for_later = saved_for_later;
    commit_draft(repo, draft)
}

#[tauri::command]
pub fn batch_create(
    state: State<Arc<AppState>>,
    text: String,
    name: Option<String>,
    preset_id: Option<String>,
) -> Result<Draft, String> {
    let mut guard = state.repository.lock().unwrap();
    create(
        guard.as_mut().ok_or("Open the app first.")?,
        &text,
        name,
        preset_id,
    )
}

#[tauri::command]
pub fn batch_save(state: State<Arc<AppState>>, draft: Draft) -> Result<Draft, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let mut saved = repo
        .data
        .drafts
        .iter()
        .find(|d| d.id == draft.id)
        .cloned()
        .ok_or("Draft not found.")?;
    if draft.exploration_limit < PAGE_SIZE || draft.exploration_limit > MAX_DRAFT_ITEMS {
        return Err("Use an exploration limit between 50 and 10000 items.".into());
    }
    if draft
        .preset_id
        .as_ref()
        .is_some_and(|id| !repo.data.presets.iter().any(|p| &p.id == id))
    {
        return Err("The selected preset no longer exists.".into());
    }
    if let Some(value) = &draft.options {
        options::validate(value)?;
    }
    let reset = saved.preset_id != draft.preset_id
        || changed(&saved.options, &draft.options)
        || saved.name != draft.name;
    saved.name = draft.name.chars().take(160).collect();
    saved.preset_id = draft.preset_id;
    saved.options = draft.options;
    saved.exploration_limit = draft.exploration_limit;
    saved.saved_for_later = draft.saved_for_later;
    saved.updated_at = queue::now();
    for i in 0..saved.items.len() {
        let Some(incoming) = draft.items.iter().find(|item| item.id == saved.items[i].id) else {
            continue;
        };
        if let Some(value) = &incoming.options {
            options::validate(value)?;
        }
        let item_reset = reset || changed(&saved.items[i].options, &incoming.options);
        saved.items[i].picked = incoming.picked;
        saved.items[i].excluded = incoming.excluded;
        saved.items[i].options = incoming.options.clone();
        if item_reset && saved.items[i].state != ItemState::Unsupported {
            let mut item = saved.items[i].clone();
            resolve_item(repo, &saved, &mut item)?;
            saved.items[i] = item;
        }
    }
    commit_draft(repo, saved)
}

fn prepare_chapters(
    repo: &mut crate::store::Repository,
    draft_id: &str,
    item_id: &str,
    indices: &[usize],
) -> Result<Draft, String> {
    if indices.is_empty() || indices.len() > 1000 {
        return Err("Select between 1 and 1000 available chapters.".into());
    }
    let mut draft = repo
        .data
        .drafts
        .iter()
        .find(|draft| draft.id == draft_id)
        .cloned()
        .ok_or("Draft not found.")?;
    let index = draft
        .items
        .iter()
        .position(|item| item.id == item_id)
        .ok_or("Item not found.")?;
    let mut original = draft.items[index].clone();
    let chapters = original
        .media
        .as_ref()
        .ok_or("Analyze this item before selecting chapters.")?
        .chapters
        .clone();
    let mut selected = indices.to_vec();
    selected.sort_unstable();
    selected.dedup();
    for &index in &selected {
        let chapter = chapters
            .get(index)
            .ok_or("Select a chapter offered by this source.")?;
        if !chapter.start.is_finite()
            || !chapter.end.is_finite()
            || chapter.start < 0.0
            || chapter.end <= chapter.start
        {
            return Err("The provider returned an invalid chapter interval.".into());
        }
    }
    if original.resolved.is_none() {
        resolve_item(repo, &draft, &mut original)?;
    }
    let frozen = original
        .resolved
        .clone()
        .ok_or("Resolve this item before selecting chapters.")?;
    draft.items[index].picked = false;
    for index in selected {
        let selected = &chapters[index];
        let mut values = frozen.values.clone();
        values.clip = Some(options::Clip {
            start: selected.start,
            end: selected.end,
            precise: false,
        });
        let duplicate = draft.items.iter().position(|item| {
            item.content_id == original.content_id
                && item.list.is_none()
                && item
                    .options
                    .as_ref()
                    .or(item.resolved.as_ref().map(|resolved| &resolved.values))
                    .is_some_and(|options| !changed(options, &values))
        });
        if let Some(existing) = duplicate {
            draft.items[existing].picked = true;
            draft.items[existing].excluded = false;
            continue;
        }
        if draft.items.len() >= MAX_DRAFT_ITEMS {
            return Err(
                "The draft has reached 10000 items. Save another draft to continue.".into(),
            );
        }
        let mut item = original.clone();
        item.id = identifier("chapter");
        item.title = format!("{} · {}", original.title, selected.title)
            .chars()
            .take(1024)
            .collect();
        item.duration = Some(selected.end - selected.start);
        item.state = ItemState::Ready;
        item.picked = true;
        item.excluded = false;
        item.list = None;
        item.error = None;
        item.options = Some(values);
        item.resolved = None;
        resolve_item(repo, &draft, &mut item)?;
        if let Some(resolved) = &mut item.resolved {
            resolved.preset_id = frozen.preset_id.clone();
            resolved.preset_revision = frozen.preset_revision;
        }
        draft.items.push(item);
    }
    commit_draft(repo, draft)
}

#[tauri::command]
pub fn batch_chapters(
    state: State<Arc<AppState>>,
    draft_id: String,
    item_id: String,
    indices: Vec<usize>,
) -> Result<Draft, String> {
    let mut guard = state.repository.lock().unwrap();
    prepare_chapters(
        guard.as_mut().ok_or("Open the app first.")?,
        &draft_id,
        &item_id,
        &indices,
    )
}

fn cancellations() -> &'static Mutex<HashMap<String, watch::Sender<bool>>> {
    static ACTIVE: OnceLock<Mutex<HashMap<String, watch::Sender<bool>>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn receiver(id: &str) -> watch::Receiver<bool> {
    let mut active = cancellations().lock().unwrap();
    active
        .entry(id.into())
        .or_insert_with(|| watch::channel(false).0)
        .subscribe()
}

pub fn cancel(id: &str) {
    if let Some(sender) = cancellations().lock().unwrap().remove(id) {
        let _ = sender.send(true);
    }
}

#[tauri::command]
pub fn batch_cancel(state: State<Arc<AppState>>, draft_id: String) -> Result<Draft, String> {
    cancel(&draft_id);
    let guard = state.repository.lock().unwrap();
    guard
        .as_ref()
        .ok_or("Open the app first.")?
        .data
        .drafts
        .iter()
        .find(|d| d.id == draft_id)
        .cloned()
        .ok_or("Draft not found.".into())
}

fn site_gate(url: &str) -> Arc<Semaphore> {
    static GATES: OnceLock<Mutex<HashMap<String, Arc<Semaphore>>>> = OnceLock::new();
    let mut gates = GATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap();
    let key = if gates.len() < 64 {
        providers::scope(url).unwrap_or_default()
    } else {
        "other".into()
    };
    gates
        .entry(key)
        .or_insert_with(|| Arc::new(Semaphore::new(1)))
        .clone()
}

pub struct MetadataPermit {
    _site: OwnedSemaphorePermit,
    _global: OwnedSemaphorePermit,
}

fn metadata_slots() -> &'static Arc<Semaphore> {
    static GLOBAL: OnceLock<Arc<Semaphore>> = OnceLock::new();
    GLOBAL.get_or_init(|| Arc::new(Semaphore::new(2)))
}

pub fn metadata_busy() -> bool {
    metadata_slots().available_permits() < 2
}

pub async fn metadata_permit(url: &str) -> Result<MetadataPermit, String> {
    let site = site_gate(url)
        .acquire_owned()
        .await
        .map_err(|_| engine::CANCELLED)?;
    let global = metadata_slots()
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| engine::CANCELLED)?;
    Ok(MetadataPermit {
        _site: site,
        _global: global,
    })
}

fn parse_page(
    raw: &str,
    parent: &DraftItem,
    start: usize,
) -> Result<Option<(ListProgress, Vec<DraftItem>)>, String> {
    let value: Value = serde_json::from_str(raw).map_err(|_| "Unreadable source information.")?;
    let Some(entries) = value["entries"].as_array() else {
        return Ok(None);
    };
    let complete = entries.len() <= PAGE_SIZE;
    let name: String = value["title"]
        .as_str()
        .unwrap_or(&parent.title)
        .chars()
        .take(240)
        .collect();
    let mut items = Vec::new();
    for entry in entries.iter().take(PAGE_SIZE) {
        let stable = ["webpage_url", "original_url", "url"]
            .into_iter()
            .filter_map(|key| entry[key].as_str())
            .find(|url| model::validate_url(url).is_ok());
        let fallback = entry["id"]
            .as_str()
            .filter(|id| {
                id.bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
            })
            .filter(|_| parent.provider == "YouTube")
            .map(|id| format!("https://www.youtube.com/watch?v={id}"));
        let Some(url) = fallback.or_else(|| stable.map(str::to_owned)) else {
            continue;
        };
        let mut item = candidate(&url);
        item.list_name = name.clone();
        item.title = entry["title"]
            .as_str()
            .unwrap_or("Untitled item")
            .chars()
            .take(1024)
            .collect();
        item.creator = entry["uploader"]
            .as_str()
            .or_else(|| entry["channel"].as_str())
            .unwrap_or_default()
            .chars()
            .take(512)
            .collect();
        item.duration = entry["duration"]
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0);
        item.thumbnail = entry["thumbnail"]
            .as_str()
            .filter(|url| url.starts_with("https://") && model::validate_url(url).is_ok())
            .map(str::to_owned)
            .or_else(|| {
                entry["thumbnails"]
                    .as_array()?
                    .iter()
                    .rev()
                    .filter_map(|thumb| thumb["url"].as_str())
                    .find(|url| url.starts_with("https://") && model::validate_url(url).is_ok())
                    .map(str::to_owned)
            });
        let unavailable = entry["is_live"] == true
            || entry["has_drm"] == true
            || entry["live_status"] == "is_upcoming"
            || entry["availability"] == "private";
        item.state = if unavailable {
            ItemState::Unsupported
        } else {
            ItemState::Ready
        };
        item.picked = !unavailable;
        item.error = unavailable
            .then(|| "Live, private or protected item. Review access at its source.".into());
        items.push(item);
    }
    Ok(Some((
        ListProgress {
            loaded: start.saturating_sub(1) + entries.len().min(PAGE_SIZE),
            complete,
            name,
        },
        items,
    )))
}

async fn inspect_one(
    app: &AppHandle,
    state: &AppState,
    item: &DraftItem,
    start: usize,
    cancel: watch::Receiver<bool>,
) -> Result<(Option<model::Media>, Option<(ListProgress, Vec<DraftItem>)>), String> {
    let config = crate::config_root(state)?;
    let operational = operational_url(&config, item)?;
    let mut stopped = cancel.clone();
    let _permit = tokio::select! { permit = metadata_permit(operational.as_str()) => permit?, _ = stopped.changed() => return Err(engine::CANCELLED.into()) };
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let url = providers::validate_remote(operational.as_str()).await?;
    {
        let guard = state.repository.lock().unwrap();
        if state
            .runtime
            .updating
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err("Wait for the engine update to finish.".into());
        }
        queue::check_cooldown(guard.as_ref().ok_or("Open the app first.")?, &url)
            .map_err(|e| e.message.to_owned())?;
    }
    if providers::identify(&url)?.id == providers::ProviderId::Generic {
        return Ok((
            Some(crate::direct::inspect(&url, queue::now(), cancel).await?),
            None,
        ));
    }
    let root = engine::engine_dir(app)?;
    let cookies = sessions::materialize(&config, &url)?;
    let mut args = engine::inspect_args(&root, &url);
    if let Some(index) = args.iter().position(|arg| arg == "--playlist-end") {
        args[index + 1] = (start + PAGE_SIZE).to_string();
        args.splice(index..index, ["--playlist-start".into(), start.to_string()]);
    }
    if let Some(file) = &cookies {
        args.splice(
            0..0,
            ["--cookies".into(), file.path.to_string_lossy().into_owned()],
        );
    }
    let raw = engine::run_process(
        crate::updater::executable(app, &root)?,
        args,
        cancel,
        queue::now(),
        true,
        |_| {},
    )
    .await?;
    if let Some(page) = parse_page(&raw, item, start)? {
        return Ok((None, Some(page)));
    }
    Ok((Some(model::parse_media(&raw, queue::now())?), None))
}

#[tauri::command]
pub async fn batch_inspect(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    draft_id: String,
    item_id: String,
) -> Result<Draft, String> {
    static WORKERS: Semaphore = Semaphore::const_new(3);
    let cancel = receiver(&draft_id);
    let mut stop = cancel.clone();
    let _worker = tokio::select! { permit = WORKERS.acquire() => permit.map_err(|_| engine::CANCELLED)?, _ = stop.changed() => return Err(engine::CANCELLED.into()) };
    let (item, start) = {
        let guard = state.repository.lock().unwrap();
        let draft = guard
            .as_ref()
            .ok_or("Open the app first.")?
            .data
            .drafts
            .iter()
            .find(|d| d.id == draft_id)
            .ok_or("Draft not found.")?;
        let item = draft
            .items
            .iter()
            .find(|i| i.id == item_id)
            .cloned()
            .ok_or("Item not found.")?;
        let start = item.list.as_ref().map_or(1, |list| list.loaded + 1);
        if item.list.as_ref().is_some_and(|list| list.complete) {
            return Ok(draft.clone());
        }
        if start > draft.exploration_limit {
            return Err(
                "Increase the exploration limit to load more items. This list is incomplete."
                    .into(),
            );
        }
        (item, start)
    };
    let result = inspect_one(&app, &state, &item, start, cancel.clone()).await;
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let mut draft = repo
        .data
        .drafts
        .iter()
        .find(|d| d.id == draft_id)
        .cloned()
        .ok_or("Draft was removed.")?;
    let index = draft
        .items
        .iter()
        .position(|i| i.id == item_id)
        .ok_or("Item not found.")?;
    match result {
        Ok((media, page)) => {
            let mut target = draft.items[index].clone();
            target.state = ItemState::Ready;
            target.error = None;
            if let Some(media) = media {
                target.title = media.title.clone();
                target.creator = media.uploader.clone().unwrap_or_default();
                target.duration = media.duration;
                target.thumbnail = media.thumbnail.clone();
                target.media = Some(media);
                resolve_item(repo, &draft, &mut target)?;
            }
            if let Some((progress, mut items)) = page {
                target.title = progress.name.clone();
                target.picked = false;
                target.list = Some(progress);
                let mut existing: HashSet<_> = draft
                    .items
                    .iter()
                    .filter(|entry| entry.id != item_id && entry.list.is_none())
                    .map(|i| i.content_id.clone())
                    .collect();
                items.retain(|i| existing.insert(i.content_id.clone()));
                for item in &mut items {
                    resolve_item(repo, &draft, item)?;
                }
                if draft.items.len() + items.len() > MAX_DRAFT_ITEMS {
                    return Err(
                        "The draft has reached 10000 items. Save another draft to continue.".into(),
                    );
                }
                draft.items.extend(items);
            }
            draft.items[index] = target;
        }
        Err(raw) => {
            if raw == engine::CANCELLED {
                return Err(raw);
            }
            let failure = Failure::classify(&raw, "extraction");
            draft.items[index].state = if matches!(failure.code, "authentication" | "forbidden") {
                ItemState::Authentication
            } else {
                ItemState::Unsupported
            };
            draft.items[index].error = Some(format!(
                "{} {}",
                failure.message,
                crate::failure::sanitize(&raw)
            ));
            if failure.code == "rate_limited" {
                drop(guard);
                queue::record_metadata_failure(&state, &item.url, &failure)?;
                return Err(failure.message.into());
            }
        }
    }
    draft.updated_at = queue::now();
    let saved = commit_draft(repo, draft)?;
    let _ = app.emit("batch-update", &saved);
    Ok(saved)
}

fn prepared_selection(
    repo: &crate::store::Repository,
    draft: &Draft,
) -> Result<Vec<options::PreparedDownload>, String> {
    let mut prepared = Vec::new();
    for item in draft.items.iter().filter(|i| {
        i.picked
            && !i.excluded
            && i.list.is_none()
            && matches!(
                i.state,
                ItemState::Ready | ItemState::Available | ItemState::Active
            )
    }) {
        let mut selected = item.clone();
        if selected.resolved.is_none() {
            resolve_item(repo, draft, &mut selected)?;
        }
        let url = operational_url(repo.root(), &selected)?.as_str().to_owned();
        prepared.push(options::PreparedDownload {
            url,
            title: selected.title,
            content_id: selected.content_id,
            options: selected
                .resolved
                .ok_or("Resolve the item before queueing.")?,
            source_id: None,
            group_id: Some(draft.id.clone()),
        });
    }
    if prepared.is_empty() {
        return Err("Select at least one prepared item.".into());
    }
    Ok(prepared)
}

#[tauri::command]
pub fn batch_enqueue(
    app: AppHandle,
    state: State<Arc<AppState>>,
    draft_id: String,
    request: String,
) -> Result<Vec<queue::Job>, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let draft = repo
        .data
        .drafts
        .iter()
        .find(|d| d.id == draft_id)
        .cloned()
        .ok_or("Draft not found.")?;
    let prepared = prepared_selection(repo, &draft)?;
    let jobs = queue::enqueue_prepared(repo, prepared, &request)?;
    let mut refreshed = draft.clone();
    for index in 0..refreshed.items.len() {
        if refreshed.items[index].list.is_some() || refreshed.items[index].resolved.is_none() {
            continue;
        }
        let mut item = refreshed.items[index].clone();
        // Preserve the prepared snapshot when a preset has since changed.
        let prepared = options::PreparedDownload {
            url: item.url.clone(),
            title: item.title.clone(),
            content_id: item.content_id.clone(),
            options: item.resolved.clone().ok_or("Missing prepared options.")?,
            source_id: None,
            group_id: Some(refreshed.id.clone()),
        };
        item.state = match queue::prepared_status(repo, &prepared) {
            Some("active") => ItemState::Active,
            Some("available") => ItemState::Available,
            _ => item.state,
        };
        refreshed.items[index] = item;
    }
    refreshed.updated_at = queue::now();
    commit_draft(repo, refreshed)?;
    drop(guard);
    state.runtime.notify.notify_one();
    let _ = app.emit("queue-changed", ());
    Ok(jobs)
}

#[tauri::command]
pub fn batch_remove(state: State<Arc<AppState>>, id: String) -> Result<(), String> {
    cancel(&id);
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let previous = repo.data.drafts.clone();
    repo.data.drafts.retain(|draft| draft.id != id);
    if let Err(error) = repo.save() {
        repo.data.drafts = previous;
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_candidates_keep_errors_without_losing_valid_links() {
        let items = candidates("https://www.youtube.com/watch?v=one bad file:///secret https://www.youtube.com/watch?v=two").unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(
            items
                .iter()
                .filter(|i| i.state == ItemState::Pending)
                .count(),
            2
        );
        assert_eq!(items[1].state, ItemState::Unsupported);
        assert!(items[0].picked && !items[1].picked);
        let credentials = candidates("https://user:secret@example.org/video").unwrap();
        assert_eq!(credentials[0].state, ItemState::Unsupported);
        assert!(!serde_json::to_string(&credentials)
            .unwrap()
            .contains("secret"));
    }
    #[test]
    fn page_lookahead_reports_partial_scope_and_stable_source_links() {
        let parent = candidate("https://www.youtube.com/playlist?list=PLtest");
        let entries: Vec<_> = (0..51).map(|i| serde_json::json!({"id":format!("video{i}"),"title":format!("Video {i}"),"url":format!("video{i}")})).collect();
        let raw = serde_json::json!({"title":"List","entries":entries}).to_string();
        let (page, items) = parse_page(&raw, &parent, 1).unwrap().unwrap();
        assert_eq!(page.loaded, 50);
        assert!(!page.complete);
        assert_eq!(items.len(), 50);
        assert_eq!(items[0].url, "https://www.youtube.com/watch?v=video0");
        let (page, _) = parse_page(r#"{"entries":[]}"#, &parent, 51)
            .unwrap()
            .unwrap();
        assert_eq!(page.loaded, 50);
        assert!(page.complete);
        assert_eq!(
            public_list_url("https://www.youtube.com/watch?v=video0&list=PLtest&token=SECRET"),
            Some("https://www.youtube.com/playlist?list=PLtest".into())
        );
    }
    #[test]
    fn cancellation_is_per_draft_and_later_analysis_can_resume() {
        let first = receiver("test-first");
        let second = receiver("test-second");
        cancel("test-first");
        assert!(*first.borrow());
        assert!(!*second.borrow());
        assert!(!*receiver("test-first").borrow());
        cancel("test-first");
        cancel("test-second");
    }
    #[test]
    fn selected_draft_and_frozen_overrides_survive_restart_without_downloads() {
        let root = std::env::temp_dir().join(identifier("nabnab-batch-test"));
        let folder = root.join("media");
        let mut repo = crate::store::Repository::open(&root, &folder).unwrap();
        repo.data.presets.push(options::Preset {
            id: "test-preset".into(),
            name: "Video".into(),
            revision: 1,
            options: options::WorkOptions::default(),
        });
        let mut draft = create(
            &mut repo,
            "https://www.youtube.com/watch?v=one invalid https://www.youtube.com/watch?v=two",
            None,
            Some("test-preset".into()),
        )
        .unwrap();
        draft.saved_for_later = true;
        draft.items[0].state = ItemState::Ready;
        draft.items[0].options = Some(options::WorkOptions {
            format: model::Format::Mp3,
            ..options::WorkOptions::default()
        });
        draft.items[2].state = ItemState::Ready;
        draft.items[2].picked = false;
        let mut item = draft.items[0].clone();
        resolve_item(&repo, &draft, &mut item).unwrap();
        draft.items[0] = item;
        let id = draft.id.clone();
        commit_draft(&mut repo, draft).unwrap();
        let preset = repo
            .data
            .presets
            .iter_mut()
            .find(|preset| preset.id == "test-preset")
            .unwrap();
        preset.revision = 2;
        preset.options.format = model::Format::Source;
        repo.save().unwrap();
        drop(repo);
        let repo = crate::store::Repository::open(&root, &folder).unwrap();
        let restored = repo
            .data
            .drafts
            .iter()
            .find(|draft| draft.id == id)
            .unwrap();
        let prepared = prepared_selection(&repo, restored).unwrap();
        assert!(restored.saved_for_later);
        assert_eq!(restored.items.len(), 3);
        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].url, "https://www.youtube.com/watch?v=one");
        assert_eq!(prepared[0].options.values.format, model::Format::Mp3);
        assert_eq!(prepared[0].options.preset_revision, Some(1));
        assert!(repo.data.jobs.is_empty());
        assert!(repo.data.library.is_empty());
        drop(repo);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn extension_replay_commits_one_saved_draft_and_monotonic_updates() {
        let root = std::env::temp_dir().join(identifier("nabnab-browser-draft-test"));
        let folder = root.join("media");
        let mut repo = crate::store::Repository::open(&root, &folder).unwrap();
        let first = create_for_request(
            &mut repo,
            "https://www.youtube.com/watch?v=one invalid",
            None,
            "client:request-1",
            true,
        )
        .unwrap();
        let replay = create_for_request(
            &mut repo,
            "https://www.youtube.com/watch?v=two",
            None,
            "client:request-1",
            false,
        )
        .unwrap();
        assert_eq!(first.id, replay.id);
        assert_eq!(repo.data.drafts.len(), 1);
        assert!(replay.saved_for_later);
        assert_eq!(replay.items.len(), 2);
        assert_eq!(replay.items[0].url, "https://www.youtube.com/watch?v=one");
        assert!(repo.data.jobs.is_empty());
        assert!(repo.data.library.is_empty());
        let stamp = replay.updated_at;
        let saved = commit_draft(&mut repo, replay).unwrap();
        assert!(saved.updated_at > stamp);
        drop(repo);
        let repo = crate::store::Repository::open(&root, &folder).unwrap();
        assert_eq!(repo.data.drafts.len(), 1);
        assert_eq!(repo.data.drafts[0].id, first.id);
        drop(repo);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[cfg(windows)]
    fn signed_draft_urls_are_protected_and_round_trip_exactly_after_restart() {
        let root = std::env::temp_dir().join(identifier("nabnab-private-draft-test"));
        let folder = root.join("media");
        let original =
            "https://cdn.example.org/private/SECRET-file.mp4?token=SECRET-token&expires=123456";
        let mut repo = crate::store::Repository::open(&root, &folder).unwrap();
        let mut draft = create(&mut repo, original, None, None).unwrap();
        assert_eq!(draft.items[0].url, "https://cdn.example.org");
        assert!(draft.items[0]
            .url_ref
            .as_deref()
            .is_some_and(sessions::valid_url_reference));
        draft.items[0].state = ItemState::Ready;
        let mut item = draft.items[0].clone();
        resolve_item(&repo, &draft, &mut item).unwrap();
        draft.items[0] = item;
        let draft = commit_draft(&mut repo, draft).unwrap();
        assert!(!serde_json::to_string(&draft).unwrap().contains("SECRET"));
        assert!(!std::fs::read_to_string(root.join("state-v1.json"))
            .unwrap()
            .contains("SECRET"));
        let id = draft.id.clone();
        drop(repo);
        let repo = crate::store::Repository::open(&root, &folder).unwrap();
        let restored = repo
            .data
            .drafts
            .iter()
            .find(|draft| draft.id == id)
            .unwrap();
        assert_eq!(
            operational_url(repo.root(), &restored.items[0])
                .unwrap()
                .as_str(),
            original
        );
        assert_eq!(
            prepared_selection(&repo, restored).unwrap()[0].url,
            original
        );
        assert_eq!(
            restored.items[0].resolved.as_ref().unwrap().context.url,
            "https://cdn.example.org"
        );

        let mut legacy = restored.clone();
        legacy.items[0].url_ref = None;
        legacy.items[0].url = original.into();
        legacy.items[0].title = original.into();
        legacy.items[0].content_id = original.into();
        legacy.items[0].destination = Some(original.into());
        legacy.items[0].thumbnail = Some(original.into());
        legacy.items[0].media = Some(serde_json::from_value(serde_json::json!({
            "requestId":1,"title":original,"thumbnail":original,"duration":null,"uploader":original,
            "heights":[],"hasAudio":true,"hasVideo":true,
            "entries":[{"id":"local","title":original,"url":original,"thumbnail":original,"duration":null}]
        })).unwrap());
        let resolved = legacy.items[0].resolved.as_mut().unwrap();
        resolved.context.url = original.into();
        resolved.context.source_url = original.into();
        resolved.context.title = original.into();
        resolved.context.content_id = original.into();
        protect_drafts(repo.root(), std::slice::from_mut(&mut legacy)).unwrap();
        assert!(!serde_json::to_string(&legacy).unwrap().contains("SECRET"));
        assert!(legacy.items[0].destination.is_none());
        assert!(legacy.items[0].thumbnail.is_none());
        assert_eq!(
            legacy.items[0].media.as_ref().unwrap().entries[0].url,
            "https://cdn.example.org"
        );
        assert_eq!(
            operational_url(repo.root(), &legacy.items[0])
                .unwrap()
                .as_str(),
            original
        );
        drop(repo);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[cfg(windows)]
    fn real_chapter_selection_keeps_source_identity_intervals_and_deduplicates() {
        let root = std::env::temp_dir().join(identifier("nabnab-chapter-draft-test"));
        let folder = root.join("media");
        let mut repo = crate::store::Repository::open(&root, &folder).unwrap();
        let mut draft = create(
            &mut repo,
            "https://www.youtube.com/watch?v=chapter-fixture",
            None,
            None,
        )
        .unwrap();
        draft.items[0].media = Some(model::parse_media(r#"{"title":"Source fixture","duration":30,"formats":[{"vcodec":"h264","acodec":"aac"}],"chapters":[{"title":"Opening","start_time":0,"end_time":10},{"title":"Middle","start_time":10,"end_time":22},{"title":"End","start_time":22,"end_time":30}]}"#, 1).unwrap());
        draft.items[0].title = "Source fixture".into();
        draft.items[0].state = ItemState::Ready;
        let item = draft.items[0].id.clone();
        let id = draft.id.clone();
        let reference = draft.items[0].url_ref.clone();
        let identity = draft.items[0].content_id.clone();
        commit_draft(&mut repo, draft).unwrap();
        let prepared = prepare_chapters(&mut repo, &id, &item, &[2, 0, 0]).unwrap();
        assert_eq!(prepared.items.len(), 3);
        assert!(!prepared.items[0].picked);
        let outputs = prepared_selection(&repo, &prepared).unwrap();
        assert_eq!(outputs.len(), 2);
        let clips: Vec<_> = outputs
            .iter()
            .map(|output| {
                let clip = output.options.values.clip.as_ref().unwrap();
                (clip.start, clip.end, clip.precise)
            })
            .collect();
        assert_eq!(clips, vec![(0.0, 10.0, false), (22.0, 30.0, false)]);
        assert!(prepared
            .items
            .iter()
            .all(|item| item.content_id == identity && item.url_ref == reference));
        assert!(prepared.items[1].title.ends_with("Opening"));
        assert!(prepared.items[2].title.ends_with("End"));
        assert_eq!(
            prepare_chapters(&mut repo, &id, &item, &[0, 2])
                .unwrap()
                .items
                .len(),
            3
        );
        assert!(prepare_chapters(&mut repo, &id, &item, &[99]).is_err());
        assert_eq!(repo.data.drafts[0].items.len(), 3);
        assert!(repo.data.jobs.is_empty());
        assert!(repo.data.library.is_empty());
        drop(repo);
        std::fs::remove_dir_all(root).unwrap();
    }
}
