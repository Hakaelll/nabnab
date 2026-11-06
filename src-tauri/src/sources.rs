use crate::{engine, failure::Failure, options, providers, queue, AppState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    Channel,
    Playlist,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SourceStatus {
    #[default]
    Ready,
    Checking,
    Paused,
    RequiresSession,
    Error,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum History {
    #[default]
    FromNow,
    ReviewHistory,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceSettings {
    pub preset_id: Option<String>,
    pub relative_folder: String,
    pub interval_hours: Option<u8>,
    pub automatic: bool,
    pub automatic_limit: usize,
    pub exploration_limit: usize,
    pub page_size: usize,
    pub history: History,
}
impl Default for SourceSettings {
    fn default() -> Self {
        Self {
            preset_id: None,
            relative_folder: String::new(),
            interval_hours: None,
            automatic: false,
            automatic_limit: 20,
            exploration_limit: 200,
            page_size: 50,
            history: History::FromNow,
        }
    }
}
impl SourceSettings {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .interval_hours
            .is_some_and(|hours| ![1, 6, 24].contains(&hours))
        {
            return Err("Choose manual checks or an interval of 1, 6 or 24 hours.".into());
        }
        if !(1..=1000).contains(&self.automatic_limit)
            || !(1..=1000).contains(&self.page_size)
            || self.exploration_limit < self.page_size
            || self.exploration_limit > 100_000
        {
            return Err("Use 1–1000 items per page or automatic check, and an exploration limit between the page size and 100000.".into());
        }
        if self.automatic && self.preset_id.is_none() {
            return Err("Choose a saved preset before allowing automatic additions.".into());
        }
        let options = options::WorkOptions {
            relative_folder: self.relative_folder.clone(),
            ..Default::default()
        };
        options::validate(&options)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SourceItemState {
    Observed,
    Review,
    Queued,
    Downloaded,
    Ignored,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Discovery {
    History,
    Published,
    Added,
    Ambiguous,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceItem {
    pub content_id: String,
    pub title: String,
    #[serde(default)]
    pub creator: String,
    pub url: String,
    pub thumbnail: Option<String>,
    pub duration: Option<f64>,
    pub published_at: Option<u64>,
    pub discovered_at: u64,
    pub state: SourceItemState,
    pub discovery: Discovery,
    pub reason: Option<String>,
    #[serde(default)]
    pub job_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Baseline {
    pub complete: bool,
    pub inspected: usize,
    pub total: Option<usize>,
    pub cursor: usize,
    pub has_more: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowedSource {
    pub id: String,
    pub name: String,
    pub url: String,
    pub kind: SourceKind,
    pub status: SourceStatus,
    pub created_at: u64,
    pub last_check: Option<u64>,
    pub next_check: Option<u64>,
    pub error: Option<String>,
    pub failures: u32,
    pub settings: SourceSettings,
    pub baseline: Baseline,
    pub items: Vec<SourceItem>,
}

impl FollowedSource {
    pub fn validate(&self) -> Result<(), String> {
        let (canonical, kind) = source_url(&self.url)?;
        if canonical != self.url || source_id(&self.url) != self.id || kind != self.kind {
            return Err("The saved source identity does not match its stable YouTube URL.".into());
        }
        self.settings.validate()?;
        let mut identities = HashSet::new();
        for item in &self.items {
            if options::content_identity(&item.url)? != item.content_id
                || !identities.insert(&item.content_id)
            {
                return Err(
                    "The saved source contains invalid or repeated content identities.".into(),
                );
            }
        }
        Ok(())
    }

    pub fn recover(&mut self) {
        if self.status == SourceStatus::Checking {
            self.status = SourceStatus::Ready;
        }
    }
}

fn source_url(input: &str) -> Result<(String, SourceKind), String> {
    let normalized = crate::model::validate_url(input)?;
    let parsed = url::Url::parse(&normalized).map_err(|_| "Invalid source URL.")?;
    if providers::identify(&normalized)?.id != providers::ProviderId::Youtube {
        return Err(
            "Source following currently supports YouTube channels and playlists only.".into(),
        );
    }
    if let Some(list) = parsed
        .query_pairs()
        .find(|(key, _)| key == "list")
        .map(|(_, value)| value.into_owned())
    {
        if list.is_empty()
            || list.len() > 150
            || !list
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
        {
            return Err("Invalid YouTube playlist identifier.".into());
        }
        return Ok((
            format!("https://www.youtube.com/playlist?list={list}"),
            SourceKind::Playlist,
        ));
    }
    let segments: Vec<_> = parsed
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
    let (path, rest) = match segments.as_slice() {
        [handle, rest @ ..] if handle.starts_with('@') && handle.len() > 1 => {
            ((*handle).to_owned(), rest)
        }
        [prefix @ ("channel" | "c" | "user"), id, rest @ ..] if !id.is_empty() => {
            (format!("{prefix}/{id}"), rest)
        }
        _ => {
            return Err("Paste a YouTube channel or playlist link, not an individual video.".into())
        }
    };
    if !rest.is_empty()
        && !(rest.len() == 1 && ["videos", "shorts", "streams", "featured"].contains(&rest[0]))
    {
        return Err("Choose a channel's videos, shorts or streams view.".into());
    }
    let tab = rest
        .first()
        .filter(|part| ["videos", "shorts", "streams"].contains(part))
        .copied()
        .unwrap_or("videos");
    Ok((
        format!("https://www.youtube.com/{path}/{tab}"),
        SourceKind::Channel,
    ))
}

fn source_id(url: &str) -> String {
    format!("yt-{:x}", Sha256::digest(url.as_bytes()))
}

struct Page {
    name: Option<String>,
    items: Vec<SourceItem>,
    inspected: usize,
    has_more: bool,
    total: Option<usize>,
}

fn parse_page(raw: &str, count: usize, stamp: u64) -> Result<Page, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| "The source returned unreadable list information.")?;
    let entries = value["entries"]
        .as_array()
        .ok_or("This source did not return an enumerable YouTube list.")?;
    let mut identities = HashSet::new();
    let items = entries
        .iter()
        .take(count)
        .filter_map(|entry| {
            let id = entry["id"].as_str()?;
            if id.is_empty()
                || id.len() > 128
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                || !identities.insert(id)
            {
                return None;
            }
            let title = entry["title"]
                .as_str()
                .unwrap_or("Unavailable item")
                .chars()
                .take(1024)
                .collect::<String>();
            let unavailable = matches!(
                entry["availability"].as_str(),
                Some("private" | "premium_only" | "subscriber_only" | "needs_auth")
            ) || ["[Private video]", "[Deleted video]"].contains(&title.as_str())
                || entry["has_drm"] == true
                || entry["is_live"] == true
                || entry["live_status"] == "is_upcoming";
            Some(SourceItem {
                content_id: format!("youtube:{id}"),
                title,
                creator: entry["uploader"]
                    .as_str()
                    .or_else(|| entry["channel"].as_str())
                    .unwrap_or_default()
                    .chars()
                    .take(512)
                    .collect(),
                url: format!("https://www.youtube.com/watch?v={id}"),
                thumbnail: entry["thumbnail"]
                    .as_str()
                    .and_then(|url| crate::model::validate_url(url).ok())
                    .or_else(|| {
                        entry["thumbnails"].as_array().and_then(|images| {
                            images.iter().rev().find_map(|image| {
                                image["url"]
                                    .as_str()
                                    .and_then(|url| crate::model::validate_url(url).ok())
                            })
                        })
                    }),
                duration: entry["duration"]
                    .as_f64()
                    .filter(|duration| duration.is_finite() && *duration > 0.0),
                published_at: entry["timestamp"]
                    .as_f64()
                    .filter(|time| time.is_finite() && *time > 0.0 && *time < 253_402_300_799.0)
                    .map(|time| (time * 1000.0) as u64),
                discovered_at: stamp,
                state: if unavailable {
                    SourceItemState::Unavailable
                } else {
                    SourceItemState::Review
                },
                discovery: Discovery::Ambiguous,
                reason: unavailable.then(|| "The source reports this item as unavailable.".into()),
                job_ids: Vec::new(),
            })
        })
        .collect();
    Ok(Page {
        name: value["title"]
            .as_str()
            .map(|s| s.chars().take(240).collect()),
        items,
        inspected: entries.len().min(count),
        has_more: entries.len() > count,
        total: value["playlist_count"].as_u64().map(|count| count as usize),
    })
}

fn apply_page(
    source: &mut FollowedSource,
    page: Page,
    continuation: bool,
    stamp: u64,
) -> Vec<String> {
    let initial = source.last_check.is_none();
    let baseline_complete = source.baseline.complete;
    let mut automatic = Vec::new();
    for mut item in page.items {
        if let Some(known) = source
            .items
            .iter_mut()
            .find(|known| known.content_id == item.content_id)
        {
            known.title = item.title;
            if !item.creator.is_empty() {
                known.creator = item.creator;
            }
            known.thumbnail = item.thumbnail;
            known.duration = item.duration;
            known.published_at = item.published_at.or(known.published_at);
            if item.state == SourceItemState::Unavailable
                && !matches!(
                    known.state,
                    SourceItemState::Downloaded | SourceItemState::Queued
                )
            {
                known.state = SourceItemState::Unavailable;
                known.reason = item.reason;
            }
            continue;
        }
        if item.state != SourceItemState::Unavailable {
            if initial || continuation {
                item.discovery = Discovery::History;
                item.state = if source.settings.history == History::FromNow {
                    SourceItemState::Observed
                } else {
                    SourceItemState::Review
                };
                item.reason=Some("Initial history is never downloaded automatically. Select an item to queue it.".into());
            } else if !baseline_complete || item.published_at.is_none() {
                item.discovery = Discovery::Ambiguous;
                item.reason=Some("Incomplete baseline or missing publication date. Review this item before downloading.".into());
            } else if source.kind == SourceKind::Channel
                && item
                    .published_at
                    .is_some_and(|time| time < source.created_at)
            {
                item.discovery = Discovery::Ambiguous;
                item.reason = Some(
                    "This older channel item may belong to history not previously inspected."
                        .into(),
                );
            } else {
                item.discovery = if item
                    .published_at
                    .is_some_and(|time| time >= source.created_at)
                {
                    Discovery::Published
                } else {
                    Discovery::Added
                };
                if source.settings.automatic && automatic.len() < source.settings.automatic_limit {
                    automatic.push(item.content_id.clone());
                } else if source.settings.automatic {
                    item.reason = Some(
                        "The automatic check limit was reached. This item remains for review."
                            .into(),
                    );
                }
            }
        }
        source.items.push(item);
    }
    let offset = if continuation {
        source.baseline.cursor
    } else {
        0
    };
    source.baseline.cursor = offset + page.inspected;
    source.baseline.inspected = source.baseline.inspected.max(source.baseline.cursor);
    source.baseline.total = page.total.or(source.baseline.total);
    source.baseline.has_more = page.has_more
        || source
            .baseline
            .total
            .is_some_and(|total| total > source.baseline.cursor);
    // A later complete page can finish the baseline, but never retroactively authorizes its history.
    if !source.baseline.has_more {
        source.baseline.complete = true;
    }
    if source.name == source.url {
        if let Some(name) = page.name.filter(|name| !name.trim().is_empty()) {
            source.name = name;
        }
    }
    source.last_check = Some(stamp);
    source.next_check = source
        .settings
        .interval_hours
        .map(|hours| stamp + u64::from(hours) * 3_600_000);
    source.status = SourceStatus::Ready;
    source.failures = 0;
    source.error = None;
    automatic
}

fn reconcile(source: &mut FollowedSource, jobs: &[queue::Job]) {
    for item in &mut source.items {
        if item.job_ids.is_empty() {
            continue;
        }
        let linked: Vec<_> = item
            .job_ids
            .iter()
            .filter_map(|id| jobs.iter().find(|job| job.id == *id))
            .collect();
        if linked
            .iter()
            .all(|job| job.state == queue::JobState::Completed)
            && linked.len() == item.job_ids.len()
        {
            item.state = SourceItemState::Downloaded;
            item.reason = None;
        } else if linked.iter().any(|job| {
            job.state.active()
                || matches!(
                    job.state,
                    queue::JobState::Pending | queue::JobState::Paused
                )
        }) {
            item.state = SourceItemState::Queued;
        } else if linked.iter().any(|job| {
            matches!(
                job.state,
                queue::JobState::Error | queue::JobState::Cancelled
            )
        }) {
            item.state = SourceItemState::Review;
            item.reason =
                Some("A related output did not finish. Review its jobs in Downloads.".into());
        }
    }
}

fn queue_items(
    repo: &mut crate::store::Repository,
    source: &FollowedSource,
    ids: &[String],
    manual: bool,
) -> Result<Vec<queue::Job>, String> {
    let selected: HashSet<_> = ids.iter().collect();
    let mut prepared = Vec::new();
    for item in source.items.iter().filter(|item| {
        selected.contains(&item.content_id) && item.state != SourceItemState::Unavailable
    }) {
        let context = options::Context {
            url: item.url.clone(),
            source_url: source.url.clone(),
            title: item.title.clone(),
            creator: if item.creator.is_empty() && source.kind == SourceKind::Channel {
                source.name.clone()
            } else {
                item.creator.clone()
            },
            list: source.name.clone(),
            content_id: item.content_id.clone(),
            platform: "YouTube".into(),
            content_type: "video".into(),
            tags: Vec::new(),
        };
        let mut resolved =
            options::resolve(repo, &context, source.settings.preset_id.as_deref(), None)?;
        if !source.settings.relative_folder.is_empty() {
            resolved.values.relative_folder = source.settings.relative_folder.clone();
            resolved
                .origins
                .insert("relativeFolder".into(), "source".into());
        }
        prepared.push(options::PreparedDownload {
            url: item.url.clone(),
            title: item.title.clone(),
            content_id: item.content_id.clone(),
            options: resolved,
            source_id: Some(source.id.clone()),
            group_id: Some(format!("source-{}", source.id)),
        });
    }
    if prepared.is_empty() {
        return Ok(Vec::new());
    }
    let identities: Vec<_> = prepared
        .iter()
        .map(|item| (&item.content_id, &item.options.values))
        .collect();
    let mut request = format!(
        "source-{:x}",
        Sha256::digest(
            format!(
                "{}:{}",
                source.id,
                serde_json::to_string(&identities).map_err(|error| error.to_string())?
            )
            .as_bytes()
        )
    );
    if manual {
        static ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        request.push_str(&format!(
            "-{}-{}",
            queue::now(),
            ATTEMPT.fetch_add(1, Ordering::Relaxed)
        ));
    }
    queue::enqueue_prepared(repo, prepared, &request)
}

fn running_checks() -> &'static Mutex<HashMap<String, watch::Sender<bool>>> {
    static ACTIVE: OnceLock<Mutex<HashMap<String, watch::Sender<bool>>>> = OnceLock::new();
    ACTIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

struct CheckGuard {
    id: String,
}
impl Drop for CheckGuard {
    fn drop(&mut self) {
        running_checks().lock().unwrap().remove(&self.id);
    }
}

async fn inspect_page(
    app: &AppHandle,
    state: &AppState,
    source: &FollowedSource,
    continuation: bool,
    mut cancel: watch::Receiver<bool>,
) -> Result<Page, String> {
    source.validate()?;
    let _permit = tokio::select! {
        permit=crate::batch::metadata_permit(&source.url)=>permit?,
        _=cancel.changed()=>return Err(engine::CANCELLED.into()),
    };
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let (config, root) = {
        let guard = state.repository.lock().unwrap();
        let repo = guard.as_ref().ok_or("Open the app first.")?;
        queue::check_cooldown(repo, &source.url).map_err(|failure| failure.message.to_owned())?;
        if state.runtime.updating.load(Ordering::SeqCst) {
            return Err("Wait for the engine update to finish.".into());
        }
        (repo.root().to_owned(), engine::engine_dir(app)?)
    };
    providers::validate_remote(&source.url).await?;
    let start = if continuation {
        source.baseline.cursor + 1
    } else {
        1
    };
    if continuation && start > source.settings.exploration_limit {
        return Err(
            "The exploration limit was reached. Increase it explicitly to continue.".into(),
        );
    }
    let count = source
        .settings
        .page_size
        .min(source.settings.exploration_limit.saturating_sub(start - 1));
    if count == 0 {
        return Err("Increase the exploration limit to continue.".into());
    }
    let mut args = engine::base_args(&root);
    args.retain(|arg| arg != "--no-playlist");
    args.extend([
        "--yes-playlist".into(),
        "--flat-playlist".into(),
        "--playlist-start".into(),
        start.to_string(),
        "--playlist-end".into(),
        (start + count).to_string(),
        "--dump-single-json".into(),
        "--skip-download".into(),
        "--ignore-errors".into(),
    ]);
    let cookies = crate::sessions::materialize(&config, &source.url)?;
    if let Some(file) = &cookies {
        args.extend(["--cookies".into(), file.path.to_string_lossy().into_owned()]);
    }
    args.extend(["--".into(), source.url.clone()]);
    let raw = engine::run_process(
        crate::updater::executable(app, &root)?,
        args,
        cancel.clone(),
        queue::now(),
        true,
        |_| {},
    )
    .await?;
    let mut page = parse_page(&raw, count, queue::now())?;
    drop(_permit);
    if !continuation
        && source.last_check.is_some()
        && source.baseline.complete
        && source.settings.automatic
    {
        enrich_new_items(app, state, source, &mut page, cancel).await?;
    }
    Ok(page)
}

async fn enrich_new_items(
    app: &AppHandle,
    state: &AppState,
    source: &FollowedSource,
    page: &mut Page,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    let known: HashSet<_> = source
        .items
        .iter()
        .map(|item| item.content_id.as_str())
        .collect();
    let already_dated = page
        .items
        .iter()
        .filter(|item| {
            !known.contains(item.content_id.as_str())
                && item.published_at.is_some()
                && item.state != SourceItemState::Unavailable
        })
        .count();
    let remaining = source
        .settings
        .automatic_limit
        .saturating_sub(already_dated);
    for item in page
        .items
        .iter_mut()
        .filter(|item| {
            !known.contains(item.content_id.as_str())
                && item.published_at.is_none()
                && item.state != SourceItemState::Unavailable
        })
        .take(remaining)
    {
        let _permit = tokio::select! {
            permit=crate::batch::metadata_permit(&item.url)=>permit?,
            _=cancel.changed()=>return Err(engine::CANCELLED.into()),
        };
        if *cancel.borrow() {
            return Err(engine::CANCELLED.into());
        }
        let config = {
            let guard = state.repository.lock().unwrap();
            let repo = guard.as_ref().ok_or("Open the app first.")?;
            queue::check_cooldown(repo, &item.url).map_err(|failure| failure.technical)?;
            if state.runtime.updating.load(Ordering::SeqCst) {
                return Err("Wait for the engine update to finish.".into());
            }
            repo.root().to_owned()
        };
        let root = engine::engine_dir(app)?;
        let cookies = crate::sessions::materialize(&config, &item.url)?;
        let mut args = engine::base_args(&root);
        args.extend(["--dump-single-json".into(), "--skip-download".into()]);
        if let Some(file) = &cookies {
            args.extend(["--cookies".into(), file.path.to_string_lossy().into_owned()]);
        }
        args.extend(["--".into(), item.url.clone()]);
        let raw = engine::run_process(
            crate::updater::executable(app, &root)?,
            args,
            cancel.clone(),
            queue::now(),
            true,
            |_| {},
        )
        .await?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|_| "The new item returned unreadable information.")?;
        item.published_at = value["timestamp"]
            .as_f64()
            .filter(|time| time.is_finite() && *time > 0.0 && *time < 253_402_300_799.0)
            .map(|time| (time * 1000.0) as u64);
        if value["has_drm"] == true
            || value["is_live"] == true
            || value["live_status"] == "is_upcoming"
        {
            item.state = SourceItemState::Unavailable;
            item.reason = Some(
                "Live, upcoming or protected content is not available for automatic download."
                    .into(),
            );
        }
        if let Some(creator) = value["uploader"]
            .as_str()
            .or_else(|| value["channel"].as_str())
        {
            item.creator = creator.chars().take(512).collect();
        }
    }
    Ok(())
}

async fn check(
    app: &AppHandle,
    state: &Arc<AppState>,
    id: &str,
    continuation: bool,
) -> Result<FollowedSource, String> {
    let receiver = {
        let mut active = running_checks().lock().unwrap();
        if active.contains_key(id) {
            return Err("This source is already being checked.".into());
        }
        let (sender, receiver) = watch::channel(false);
        active.insert(id.into(), sender);
        receiver
    };
    let _guard = CheckGuard { id: id.into() };
    let initial = {
        let mut guard = state.repository.lock().unwrap();
        let repo = guard.as_mut().ok_or("Open the app first.")?;
        let source = repo
            .data
            .sources
            .iter_mut()
            .find(|source| source.id == id)
            .ok_or("Source not found.")?;
        if source.status == SourceStatus::Paused {
            return Err("Resume future checks before checking this source.".into());
        }
        if continuation && !source.baseline.has_more && source.last_check.is_some() {
            return Err("The last exploration has no further page.".into());
        }
        source.status = SourceStatus::Checking;
        let initial = source.clone();
        repo.save()?;
        initial
    };
    let _ = app.emit("sources-changed", ());
    let result = inspect_page(app, state, &initial, continuation, receiver).await;
    match result {
        Ok(page) => {
            let mut guard = state.repository.lock().unwrap();
            let repo = guard.as_mut().ok_or("Open the app first.")?;
            let index = repo
                .data
                .sources
                .iter()
                .position(|source| source.id == id)
                .ok_or("The source was removed. No new jobs were added.")?;
            if repo.data.sources[index].status == SourceStatus::Paused {
                return Ok(repo.data.sources[index].clone());
            }
            let old = repo.data.sources[index].clone();
            let ids = apply_page(
                &mut repo.data.sources[index],
                page,
                continuation,
                queue::now(),
            );
            let source = repo.data.sources[index].clone();
            let jobs = queue_items(repo, &source, &ids, false);
            match jobs {
                Ok(jobs) => {
                    for item in &mut repo.data.sources[index].items {
                        let related: Vec<_> = jobs
                            .iter()
                            .filter(|job| {
                                job.content_id.as_deref() == Some(item.content_id.as_str())
                            })
                            .map(|job| job.id.clone())
                            .collect();
                        if !related.is_empty() {
                            item.state = SourceItemState::Queued;
                            item.job_ids = related;
                            item.reason = None;
                        }
                    }
                }
                Err(error) => {
                    repo.data.sources[index].error = Some(format!(
                        "Items discovered, but automatic additions failed: {}",
                        crate::failure::sanitize(&error)
                    ));
                }
            }
            reconcile(&mut repo.data.sources[index], &repo.data.jobs);
            if let Err(error) = repo.save() {
                repo.data.sources[index] = old;
                return Err(error);
            }
            let updated = repo.data.sources[index].clone();
            drop(guard);
            state.runtime.notify.notify_one();
            let _ = app.emit("sources-changed", ());
            Ok(updated)
        }
        Err(error) => {
            let failure = Failure::classify(&error, "extraction");
            let _ = queue::record_metadata_failure(state, &initial.url, &failure);
            let mut guard = state.repository.lock().unwrap();
            let repo = guard.as_mut().ok_or("Open the app first.")?;
            if let Some(source) = repo.data.sources.iter_mut().find(|source| source.id == id) {
                if source.status != SourceStatus::Paused {
                    source.status =
                        if failure.code == "authentication" || failure.code == "forbidden" {
                            SourceStatus::RequiresSession
                        } else {
                            SourceStatus::Error
                        };
                    source.failures = source.failures.saturating_add(1);
                    source.error = Some(crate::failure::sanitize(&error));
                    let wait = (60_000u64.saturating_mul(1u64 << source.failures.min(10)))
                        .min(24 * 3_600_000)
                        .max(
                            failure
                                .retry_after
                                .map(|seconds| queue::cooldown_seconds(Some(seconds)) * 1000)
                                .unwrap_or(0),
                        );
                    source.next_check = if source.status == SourceStatus::RequiresSession {
                        None
                    } else {
                        source
                            .settings
                            .interval_hours
                            .map(|_| queue::now().saturating_add(wait))
                    };
                }
                repo.save()?;
            }
            let _ = app.emit("sources-changed", ());
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn add_source(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    url: String,
    settings: SourceSettings,
) -> Result<FollowedSource, String> {
    settings.validate()?;
    let (url, kind) = source_url(&url)?;
    let id = source_id(&url);
    {
        let mut guard = state.repository.lock().unwrap();
        let repo = guard.as_mut().ok_or("Open the app first.")?;
        if let Some(source) = repo.data.sources.iter().find(|source| source.id == id) {
            return Ok(source.clone());
        }
        if let Some(preset) = &settings.preset_id {
            if !repo
                .data
                .presets
                .iter()
                .any(|candidate| candidate.id == *preset)
            {
                return Err("Preset not found.".into());
            }
        }
        repo.data.sources.push(FollowedSource {
            id: id.clone(),
            name: url.clone(),
            url,
            kind,
            status: SourceStatus::Ready,
            created_at: queue::now(),
            last_check: None,
            next_check: None,
            error: None,
            failures: 0,
            settings,
            baseline: Baseline::default(),
            items: Vec::new(),
        });
        if let Err(error) = repo.save() {
            repo.data.sources.pop();
            return Err(error);
        }
    }
    // Initial exploration only records history; automatic additions start with a later check.
    let result = check(&app, state.inner(), &id, false).await;
    if result.is_err() {
        let guard = state.repository.lock().unwrap();
        if let Some(source) = guard
            .as_ref()
            .and_then(|repo| repo.data.sources.iter().find(|source| source.id == id))
        {
            return Ok(source.clone());
        }
    }
    result
}

#[tauri::command]
pub async fn check_source(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    continuation: bool,
) -> Result<FollowedSource, String> {
    check(&app, state.inner(), &id, continuation).await
}

#[tauri::command]
pub fn save_source(
    app: AppHandle,
    state: State<Arc<AppState>>,
    id: String,
    name: String,
    settings: SourceSettings,
    paused: bool,
) -> Result<FollowedSource, String> {
    settings.validate()?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    if let Some(preset) = &settings.preset_id {
        if !repo
            .data
            .presets
            .iter()
            .any(|candidate| candidate.id == *preset)
        {
            return Err("Preset not found.".into());
        }
    }
    let index = repo
        .data
        .sources
        .iter()
        .position(|source| source.id == id)
        .ok_or("Source not found.")?;
    let old = repo.data.sources[index].clone();
    let source = &mut repo.data.sources[index];
    source.name = if name.trim().is_empty() {
        source.name.clone()
    } else {
        name.trim().chars().take(240).collect()
    };
    source.settings = settings;
    source.next_check = source
        .settings
        .interval_hours
        .map(|hours| queue::now() + u64::from(hours) * 3_600_000);
    source.status = if paused {
        SourceStatus::Paused
    } else {
        SourceStatus::Ready
    };
    if paused {
        if let Some(sender) = running_checks().lock().unwrap().get(&id) {
            let _ = sender.send(true);
        }
    }
    let updated = source.clone();
    if let Err(error) = repo.save() {
        repo.data.sources[index] = old;
        return Err(error);
    }
    let _ = app.emit("sources-changed", ());
    Ok(updated)
}

#[tauri::command]
pub fn source_item_action(
    app: AppHandle,
    state: State<Arc<AppState>>,
    id: String,
    content_ids: Vec<String>,
    action: String,
) -> Result<FollowedSource, String> {
    if content_ids.is_empty() || content_ids.len() > 1000 {
        return Err("Select between 1 and 1000 source items.".into());
    }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let index = repo
        .data
        .sources
        .iter()
        .position(|source| source.id == id)
        .ok_or("Source not found.")?;
    let source = repo.data.sources[index].clone();
    match action.as_str() {
        "queue" => {
            let jobs = queue_items(repo, &source, &content_ids, true)?;
            for item in &mut repo.data.sources[index].items {
                let related: Vec<_> = jobs
                    .iter()
                    .filter(|job| job.content_id.as_deref() == Some(item.content_id.as_str()))
                    .map(|job| job.id.clone())
                    .collect();
                if !related.is_empty() {
                    item.job_ids = related;
                    item.state = SourceItemState::Queued;
                    item.reason = None;
                }
            }
        }
        "ignore" | "review" => {
            for item in &mut repo.data.sources[index].items {
                if content_ids.contains(&item.content_id)
                    && !matches!(
                        item.state,
                        SourceItemState::Queued | SourceItemState::Downloaded
                    )
                {
                    item.state = if action == "ignore" {
                        SourceItemState::Ignored
                    } else {
                        SourceItemState::Review
                    };
                    item.reason = None;
                }
            }
        }
        _ => return Err("Unknown source item action.".into()),
    }
    reconcile(&mut repo.data.sources[index], &repo.data.jobs);
    repo.save()?;
    let updated = repo.data.sources[index].clone();
    drop(guard);
    state.runtime.notify.notify_one();
    let _ = app.emit("sources-changed", ());
    Ok(updated)
}

#[tauri::command]
pub fn remove_source(
    app: AppHandle,
    state: State<Arc<AppState>>,
    id: String,
    cancel_pending: bool,
) -> Result<(), String> {
    if let Some(sender) = running_checks().lock().unwrap().get(&id) {
        let _ = sender.send(true);
    }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let old = repo.data.clone();
    repo.data.sources.retain(|source| source.id != id);
    if cancel_pending {
        for job in &mut repo.data.jobs {
            if (job.source_id.as_deref() == Some(&id)
                || job
                    .associations
                    .iter()
                    .any(|link| link.source_id.as_deref() == Some(&id)))
                && matches!(
                    job.state,
                    queue::JobState::Pending | queue::JobState::Paused
                )
            {
                job.state = queue::JobState::Cancelled;
                job.updated_at = queue::now();
            }
        }
    }
    if let Err(error) = repo.save() {
        repo.data = old;
        return Err(error);
    }
    drop(guard);
    let _ = app.emit("sources-changed", ());
    Ok(())
}

pub fn start(app: AppHandle, state: Arc<AppState>) {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(12)).await;
        loop {
            let next = {
                let mut guard = state.repository.lock().unwrap();
                guard.as_mut().and_then(|repo| {
                    let jobs = &repo.data.jobs;
                    for source in &mut repo.data.sources {
                        reconcile(source, jobs);
                    }
                    repo.data
                        .sources
                        .iter()
                        .filter(|source| {
                            source.settings.interval_hours.is_some()
                                && !matches!(
                                    source.status,
                                    SourceStatus::Paused | SourceStatus::RequiresSession
                                )
                                && source.next_check.unwrap_or(0) <= queue::now()
                        })
                        .filter(|source| !running_checks().lock().unwrap().contains_key(&source.id))
                        .filter(|_| {
                            !repo
                                .data
                                .jobs
                                .iter()
                                .any(|job| job.state.active() && job.provider == "YouTube")
                        })
                        .min_by_key(|source| source.next_check.unwrap_or(0))
                        .map(|source| source.id.clone())
                })
            };
            if let Some(id) = next {
                let _ = check(&app, &state, &id, false).await;
            }
            tokio::time::sleep(std::time::Duration::from_secs(12)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source() -> FollowedSource {
        FollowedSource {
            id: "source".into(),
            name: "List".into(),
            url: "https://www.youtube.com/playlist?list=PLfixture".into(),
            kind: SourceKind::Playlist,
            status: SourceStatus::Ready,
            created_at: 10_000,
            last_check: None,
            next_check: None,
            error: None,
            failures: 0,
            settings: SourceSettings {
                automatic: true,
                preset_id: Some("preset".into()),
                ..Default::default()
            },
            baseline: Baseline::default(),
            items: Vec::new(),
        }
    }
    fn page(ids: &[(&str, Option<u64>)], has_more: bool) -> Page {
        let raw = serde_json::json!({"title":"List","entries":ids.iter().map(|(id,time)|serde_json::json!({"id":id,"title":id,"timestamp":time})).collect::<Vec<_>>()});
        let mut page = parse_page(&raw.to_string(), ids.len(), 15_000).unwrap();
        page.has_more = has_more;
        page
    }

    #[test]
    fn initial_history_is_observed_and_never_automatically_downloaded() {
        let mut source = source();
        assert!(apply_page(
            &mut source,
            page(&[("one", Some(1)), ("two", Some(2))], false),
            false,
            20_000
        )
        .is_empty());
        assert!(source.baseline.complete);
        assert_eq!(source.items[0].state, SourceItemState::Observed);
        source.settings.history = History::ReviewHistory;
        assert!(apply_page(
            &mut source,
            page(&[("older", Some(1))], false),
            true,
            21_000
        )
        .is_empty());
        assert_eq!(source.items[2].state, SourceItemState::Review);
    }
    #[test]
    fn reordered_list_does_not_duplicate_and_old_new_addition_is_distinct() {
        let mut source = source();
        apply_page(
            &mut source,
            page(&[("one", Some(1)), ("two", Some(2))], false),
            false,
            20_000,
        );
        let auto = apply_page(
            &mut source,
            page(
                &[("two", Some(2)), ("new", Some(1)), ("one", Some(1))],
                false,
            ),
            false,
            30_000,
        );
        assert_eq!(auto, vec!["youtube:new"]);
        assert_eq!(source.items.len(), 3);
        assert_eq!(source.items[2].discovery, Discovery::Added);
        assert!(
            apply_page(&mut source, page(&[("new", Some(1))], false), false, 40_000).is_empty()
        );
        assert_eq!(source.items[0].state, SourceItemState::Observed);
    }
    #[test]
    fn partial_baseline_or_missing_dates_require_review_and_missing_page_does_not_delete() {
        let mut source = source();
        apply_page(&mut source, page(&[("one", Some(1))], true), false, 20_000);
        assert!(!source.baseline.complete);
        assert!(apply_page(&mut source, page(&[("two", Some(2))], true), false, 30_000).is_empty());
        assert_eq!(source.items[1].discovery, Discovery::Ambiguous);
        source.baseline.complete = true;
        assert!(apply_page(&mut source, page(&[("three", None)], false), false, 40_000).is_empty());
        assert_eq!(source.items[2].state, SourceItemState::Review);
        assert_eq!(source.items.len(), 3);
    }
    #[test]
    fn automatic_limit_leaves_excess_for_review() {
        let mut source = source();
        source.settings.automatic_limit = 1;
        apply_page(&mut source, page(&[("one", Some(1))], false), false, 20_000);
        let auto = apply_page(
            &mut source,
            page(&[("two", Some(22)), ("three", Some(23))], false),
            false,
            30_000,
        );
        assert_eq!(auto, vec!["youtube:two"]);
        assert_eq!(source.items[2].state, SourceItemState::Review);
        assert!(source.items[2].reason.as_deref().unwrap().contains("limit"));
    }
    #[test]
    fn following_accepts_only_enumerable_youtube_urls() {
        assert_eq!(
            source_url("https://youtube.com/watch?v=v&list=PLone").unwrap(),
            (
                "https://www.youtube.com/playlist?list=PLone".into(),
                SourceKind::Playlist
            )
        );
        assert_eq!(
            source_url("https://youtube.com/@creator").unwrap().0,
            "https://www.youtube.com/@creator/videos"
        );
        for url in [
            "https://x.com/profile",
            "https://youtu.be/video",
            "https://www.youtube.com/watch?v=video",
            "https://youtube.com/@creator/unknown",
        ] {
            assert!(source_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn discovery_is_not_download_success_and_failed_outputs_return_to_review() {
        let mut source = source();
        apply_page(&mut source, page(&[("one", Some(1))], false), false, 20_000);
        reconcile(&mut source, &[]);
        assert_eq!(source.items[0].state, SourceItemState::Observed);
        let mut job: queue::Job = serde_json::from_value(serde_json::json!({
            "id":"job","runId":0,"clientRequest":"source-fixture","title":"Fixture",
            "createdAt":0,"updatedAt":0,"state":"pending",
            "source":{"kind":"transform","libraryId":"local","options":{"kind":"trim","start":0,"end":1,"precise":false}},
            "folder":"C:/Downloads","provider":"FFmpeg","thumbnail":null,"progress":null,
            "outputs":[],"error":null,"attempt":0,"groupId":null
        })).unwrap();
        source.items[0].job_ids = vec!["job".into()];
        reconcile(&mut source, &[job.clone()]);
        assert_eq!(source.items[0].state, SourceItemState::Queued);
        job.state = queue::JobState::Error;
        reconcile(&mut source, &[job.clone()]);
        assert_eq!(source.items[0].state, SourceItemState::Review);
        job.state = queue::JobState::Completed;
        reconcile(&mut source, &[job]);
        assert_eq!(source.items[0].state, SourceItemState::Downloaded);
    }
}
