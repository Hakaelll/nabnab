mod listening;
pub(crate) mod parse;
pub(crate) mod preview;

use crate::{file_sources, library, portable_collections::package, queue, sessions, AppState};
use parse::{Category, Classification, Format, Provider, Record};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::watch;

const INDEX: &str = "personal-archive-index";
const MAX_INPUT: u64 = 1024 * 1024 * 1024;
const MAX_MEDIA: u64 = 256 * 1024 * 1024;
const MAX_TEXT: u64 = 8 * 1024 * 1024;
const MAX_TEXT_TOTAL: u64 = 32 * 1024 * 1024;
const MAX_RECORDS: usize = 40_000;
const PAGE: usize = 50;

#[derive(Clone)]
struct Entry {
    name: String,
    size: u64,
    classification: Classification,
}

struct Plan {
    provider: Provider,
    path: PathBuf,
    fingerprint: String,
    zipped: bool,
    entries: Vec<Entry>,
    coverage: BTreeMap<Category, Availability>,
}

#[derive(Default)]
pub struct ArchiveState {
    plan: Mutex<Option<Arc<Plan>>>,
    storage: Mutex<()>,
    preview: crate::playback_preview::PlaybackRequest,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    id: String,
    generation: u32,
    provider: Provider,
    account: String,
    destination: String,
    source: String,
    fingerprint: String,
    categories: BTreeMap<Category, usize>,
    formats: BTreeMap<Category, Vec<Format>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    coverage: BTreeMap<Category, Availability>,
    media_bytes: u64,
    pending_removal: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    archive: Summary,
    skipped_files: usize,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Availability {
    Absent,
    Unsupported,
    Empty,
    OverLimit,
    Available,
    RequiresSelection,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CategoryPreview {
    category: Category,
    sensitive: bool,
    availability: Availability,
    files: usize,
    bytes: u64,
    incompatible_files: usize,
    over_limit_files: usize,
    records: Option<usize>,
    start: Option<u64>,
    end: Option<u64>,
    formats: Vec<Format>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    source: String,
    fingerprint: String,
    bytes: u64,
    categories: Vec<CategoryPreview>,
    dated_post_media: bool,
}

type Records = BTreeMap<Category, Vec<Record>>;

struct Scratch(PathBuf);
impl Scratch {
    fn new(root: &Path) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let folder = sessions::secure_module_directory(root, "personal-archive-staging")?;
        let path = folder.join(format!(
            "{}-{}-{}",
            std::process::id(),
            queue::now(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(|e| e.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn cancelled(cancel: &watch::Receiver<bool>) -> Result<(), String> {
    if *cancel.borrow() {
        Err(crate::engine::CANCELLED.into())
    } else {
        Ok(())
    }
}

fn source_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Export")
        .chars()
        .take(180)
        .collect()
}

fn read_text(path: &Path) -> Result<String, String> {
    let path = library::authorized_path(path)?;
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_TEXT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_TEXT {
        return Err("An export metadata file exceeds 8 MiB.".into());
    }
    String::from_utf8(bytes).map_err(|_| "This export metadata file is not UTF-8.".into())
}

fn interval(rows: &[Record]) -> (Option<u64>, Option<u64>) {
    (
        rows.iter().filter_map(|row| row.time).min(),
        rows.iter().filter_map(|row| row.time).max(),
    )
}

async fn extract(
    root: &Path,
    plan: &Plan,
    scratch: &Scratch,
    entries: &[&Entry],
    cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    if plan.zipped && !entries.is_empty() {
        let names = entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>();
        package::personal_extract(root, &plan.path, &scratch.0, &names, cancel).await?;
    }
    Ok(())
}

fn entry_path(plan: &Plan, scratch: &Scratch, entry: &Entry) -> PathBuf {
    if plan.zipped {
        scratch.0.join(&entry.name)
    } else {
        plan.path.clone()
    }
}

fn classify_entry(provider: Provider, name: String, size: u64) -> Option<Entry> {
    let classification = parse::classify(provider, &name)?;
    Some(Entry {
        name,
        size,
        classification,
    })
}

async fn inspect(
    root: &Path,
    provider: Provider,
    path: PathBuf,
    cancel: watch::Receiver<bool>,
) -> Result<(Plan, Preview), String> {
    let path = library::authorized_path(&path)?;
    let size = crate::portable_zip::regular_file(&path)?;
    if size > MAX_INPUT {
        return Err("Choose an export of at most 1 GiB.".into());
    }
    let zipped = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));
    if !zipped && size > MAX_TEXT {
        return Err("Choose a loose export file under 8 MiB.".into());
    }
    let fingerprint = queue::digest(path.clone(), cancel.clone()).await?;
    let source = source_name(&path);
    let entries = if zipped {
        package::personal_catalog(root, &path, cancel.clone())
            .await?
            .into_iter()
            .filter_map(|item| classify_entry(provider, item.name, item.size))
            .collect()
    } else {
        classify_entry(
            provider,
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&source)
                .into(),
            size,
        )
        .into_iter()
        .filter(|entry| {
            matches!(
                entry.classification.format,
                Format::Json | Format::Csv | Format::Html | Format::Unsupported
            )
        })
        .collect()
    };
    let mut plan = Plan {
        provider,
        path,
        fingerprint: fingerprint.clone(),
        zipped,
        entries,
        coverage: BTreeMap::new(),
    };
    let scratch = Scratch::new(root)?;
    let mut budget = MAX_TEXT_TOTAL;
    let sample = plan
        .entries
        .iter()
        .filter(|entry| {
            !entry.classification.category.sensitive()
                && !matches!(
                    entry.classification.format,
                    Format::Media | Format::Unsupported
                )
                && entry.size <= MAX_TEXT
                && entry.size <= budget
                && {
                    budget -= entry.size;
                    true
                }
        })
        .collect::<Vec<_>>();
    extract(root, &plan, &scratch, &sample, cancel.clone()).await?;
    let sampled = sample.iter().map(|entry| entry.name.as_str()).collect::<HashSet<_>>();
    let mut categories = Vec::new();
    for &category in provider.categories() {
        cancelled(&cancel)?;
        let found = plan
            .entries
            .iter()
            .filter(|entry| entry.classification.category == category)
            .collect::<Vec<_>>();
        let mut formats = Vec::new();
        for entry in &found {
            if !formats.contains(&entry.classification.format) {
                formats.push(entry.classification.format);
            }
        }
        let mut count = 0;
        let mut start = None;
        let mut end = None;
        let mut read = 0;
        let mut deferred = false;
        let mut incompatible_files = 0;
        let mut over_limit_files = 0;
        for entry in &found {
            if entry.classification.format == Format::Unsupported {
                incompatible_files += 1;
                continue;
            }
            if entry.classification.format == Format::Media {
                count += 1;
                read += 1;
                continue;
            }
            if entry.size > MAX_TEXT {
                over_limit_files += 1;
                continue;
            }
            if !sampled.contains(entry.name.as_str()) {
                deferred = true;
                continue;
            }
            if let Ok(rows) = read_text(&entry_path(&plan, &scratch, entry))
                .and_then(|text| parse::parse(provider, entry.classification, &entry.name, &text))
            {
                read += 1;
                count += rows.len();
                let (first, last) = interval(&rows);
                start = start.into_iter().chain(first).min();
                end = end.into_iter().chain(last).max();
            } else {
                incompatible_files += 1;
            }
        }
        let availability = if found.is_empty() {
            Availability::Absent
        } else if over_limit_files > 0 {
            Availability::OverLimit
        } else if category.sensitive() && (deferred || count > 0) {
            Availability::RequiresSelection
        } else if count > 0 {
            Availability::Available
        } else if deferred {
            Availability::RequiresSelection
        } else if read > 0 {
            Availability::Empty
        } else {
            Availability::Unsupported
        };
        categories.push(CategoryPreview {
            category,
            sensitive: category.sensitive(),
            availability,
            files: found.len(),
            bytes: found.iter().map(|entry| entry.size).sum(),
            incompatible_files,
            over_limit_files,
            records: (!deferred && !category.sensitive() && read > 0).then_some(count),
            start,
            end,
            formats,
        });
    }
    if queue::digest(plan.path.clone(), cancel).await? != plan.fingerprint {
        return Err("The export changed while it was being inspected. Choose it again.".into());
    }
    plan.coverage = categories
        .iter()
        .filter(|item| !item.sensitive)
        .map(|item| (item.category, item.availability))
        .collect();
    Ok((
        plan,
        Preview {
            source,
            fingerprint,
            bytes: size,
            categories,
            dated_post_media: zipped && (provider.is_meta() || provider == Provider::X),
        },
    ))
}

#[tauri::command]
pub async fn personal_archive_choose(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    provider: Provider,
    request_id: u64,
) -> Result<Option<Preview>, String> {
    let path = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Personal exports", &["zip", "json", "csv", "html", "htm"])
            .blocking_pick_file()
            .map(|file| file.into_path())
            .transpose()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let Some(path) = path else {
        return Ok(None);
    };
    let root = crate::config_root(&state)?;
    let cancel = file_sources::begin(&state, request_id)?;
    let result = inspect(&root, provider, path, cancel).await;
    file_sources::finish(&state, request_id);
    let (plan, preview) = result?;
    *state.personal_archive.plan.lock().unwrap() = Some(Arc::new(plan));
    Ok(Some(preview))
}

fn load_index(root: &Path) -> Result<Vec<Summary>, String> {
    if !root
        .join("module-secrets")
        .join(format!("{INDEX}.bin"))
        .try_exists()
        .map_err(|e| e.to_string())?
    {
        return Ok(Vec::new());
    }
    let mut bytes = sessions::load_module_secret(root, INDEX)?;
    let result = serde_json::from_slice(&bytes).map_err(|e| e.to_string());
    bytes.fill(0);
    result
}

fn store_index(root: &Path, index: &[Summary]) -> Result<(), String> {
    sessions::save_module_secret(
        root,
        INDEX,
        &serde_json::to_vec(index).map_err(|e| e.to_string())?,
    )
}

fn blob_name(summary: &Summary) -> String {
    format!("{}-{:08x}", summary.id, summary.generation)
}

fn read_records(root: &Path, summary: &Summary) -> Result<Records, String> {
    let mut bytes = sessions::load_personal_archive_record(root, &blob_name(summary))?;
    let result = serde_json::from_slice(&bytes).map_err(|e| e.to_string());
    bytes.fill(0);
    result
}

fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn media_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Invalid archive identity.".into());
    }
    Ok(sessions::secure_module_directory(root, "personal-archive-media")?.join(id))
}

fn media_name(source: &str) -> Result<String, String> {
    let extension = Path::new(source)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| ext.len() <= 5 && ext.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .ok_or("Unsupported media extension.")?;
    Ok(format!("{:x}.{extension}", Sha256::digest(source)))
}

fn media_source(
    state: &AppState,
    id: &str,
    category: Category,
    record: &str,
    consent: bool,
) -> Result<String, String> {
    let (_, rows) = records_for(state, id, category, consent)?;
    rows.into_iter()
        .find(|row| row.id == record)
        .and_then(|row| row.media)
        .ok_or_else(|| "Choose a media file included in this archive.".into())
}

fn managed_media(state: &AppState, id: &str, source: &str) -> Result<PathBuf, String> {
    let root = crate::config_root(state)?;
    library::authorized_path(&media_dir(&root, id)?.join(media_name(source)?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    fingerprint: String,
    account: String,
    destination: String,
    categories: Vec<Category>,
    sensitive: Vec<Category>,
    from: Option<String>,
    to: Option<String>,
}

fn label(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 72 || value.chars().any(char::is_control) {
        return Err("Enter a name of 1–72 characters.".into());
    }
    Ok(value.into())
}

fn date(value: &Option<String>, end: bool) -> Result<Option<u64>, String> {
    value
        .as_ref()
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.len() != 10 {
                return Err("Use a valid date.".into());
            }
            crate::gallery_sources::published_at(&format!("{value}T00:00:00Z"))
                .map(|stamp| if end { stamp + 86_399_999 } else { stamp })
                .ok_or_else(|| "Use a valid date.".into())
        })
        .transpose()
}

fn date_range(
    from: &Option<String>,
    to: &Option<String>,
) -> Result<(Option<u64>, Option<u64>), String> {
    let from = date(from, false)?;
    let to = date(to, true)?;
    if from.zip(to).is_some_and(|(from, to)| from > to) {
        return Err("The start date must precede the end date.".into());
    }
    Ok((from, to))
}

fn in_range(time: Option<u64>, from: Option<u64>, to: Option<u64>) -> bool {
    if from.is_none() && to.is_none() {
        return true;
    }
    time.is_some_and(|stamp| {
        from.is_none_or(|start| stamp >= start) && to.is_none_or(|end| stamp <= end)
    })
}

fn dated_post_media<'a>(plan: &'a Plan, posts: &[Record]) -> Vec<&'a Entry> {
    let linked: HashSet<_> = if plan.provider.is_meta() {
        posts
            .iter()
            .flat_map(|post| &post.attachment_ids)
            .map(String::as_str)
            .collect()
    } else if plan.provider == Provider::X {
        posts.iter().map(|post| post.id.as_str()).collect()
    } else {
        return Vec::new();
    };
    if linked.is_empty() {
        return Vec::new();
    }
    plan.entries
        .iter()
        .filter(|entry| {
            entry.classification.category == Category::Media
                && entry.classification.format == Format::Media
                && if plan.provider == Provider::X {
                    parse::x_attachment_owner(&entry.name)
                        .is_some_and(|id| linked.contains(id.as_str()))
                } else {
                    linked.contains(parse::record_id(&entry.name).as_str())
                }
        })
        .collect()
}

fn filtered_rows(
    plan: &Plan,
    scratch: &Scratch,
    entry: &Entry,
    from: Option<u64>,
    to: Option<u64>,
) -> Result<Vec<Record>, String> {
    let text = read_text(&entry_path(plan, scratch, entry))?;
    let mut rows = parse::parse(plan.provider, entry.classification, &entry.name, &text)?;
    rows.retain(|row| in_range(row.time, from, to));
    Ok(rows)
}

fn selected_categories(
    provider: Provider,
    categories: Vec<Category>,
    sensitive: Vec<Category>,
) -> Result<HashSet<Category>, String> {
    if categories.iter().any(|category| category.sensitive())
        || sensitive.iter().any(|category| !category.sensitive())
    {
        return Err("Select messages and contacts separately.".into());
    }
    let selected = categories.into_iter().chain(sensitive).collect::<HashSet<_>>();
    if selected.is_empty()
        || selected.iter().any(|category| !provider.categories().contains(category))
    {
        return Err("Select at least one category included by this provider.".into());
    }
    Ok(selected)
}

struct PreparedSelection<'a> {
    incoming: Records,
    imported_formats: BTreeMap<Category, Vec<Format>>,
    media_entries: Vec<&'a Entry>,
    first_error: Option<String>,
    skipped_files: usize,
}

async fn prepare_selection<'a>(
    root: &Path,
    plan: &'a Plan,
    scratch: &Scratch,
    selected: &HashSet<Category>,
    from: Option<u64>,
    to: Option<u64>,
    cancel: watch::Receiver<bool>,
) -> Result<PreparedSelection<'a>, String> {
    let date_filter = from.is_some() || to.is_some();
    let entries = plan
        .entries
        .iter()
        .filter(|entry| {
            selected.contains(&entry.classification.category)
                && entry.classification.format != Format::Unsupported
                && (entry.classification.format != Format::Media || !date_filter)
        })
        .collect::<Vec<_>>();
    let mut text_bytes = 0;
    let mut media_bytes = 0;
    for entry in &entries {
        if entry.classification.format == Format::Media {
            media_bytes += entry.size;
        } else {
            if entry.size > MAX_TEXT {
                return Err("A selected metadata file exceeds 8 MiB.".into());
            }
            text_bytes += entry.size;
        }
    }
    if text_bytes > MAX_TEXT_TOTAL
        || media_bytes > MAX_MEDIA
        || text_bytes + media_bytes > MAX_MEDIA
    {
        return Err("Choose fewer categories or a smaller export. One import can extract up to 256 MiB, including up to 32 MiB of metadata.".into());
    }
    extract(root, plan, scratch, &entries, cancel.clone()).await?;
    let mut incoming = Records::new();
    let mut imported_formats = BTreeMap::<Category, Vec<Format>>::new();
    let mut media_entries = Vec::new();
    let mut first_error = None;
    let mut skipped_files = plan
        .entries
        .iter()
        .filter(|entry| {
            selected.contains(&entry.classification.category)
                && entry.classification.format == Format::Unsupported
        })
        .count();
    for entry in entries {
        cancelled(&cancel)?;
        let category = entry.classification.category;
        if entry.classification.format == Format::Media {
            media_entries.push(entry);
            continue;
        }
        let rows = match filtered_rows(plan, scratch, entry, from, to) {
            Ok(rows) => rows,
            Err(error) => {
                first_error.get_or_insert(error);
                skipped_files += 1;
                continue;
            }
        };
        if !rows.is_empty() {
            let formats = imported_formats.entry(category).or_default();
            if !formats.contains(&entry.classification.format) {
                formats.push(entry.classification.format);
            }
            incoming.entry(category).or_default().extend(rows);
        }
        if incoming.values().map(Vec::len).sum::<usize>() + media_entries.len() > MAX_RECORDS {
            return Err("Too many indexed records. Choose a narrower date range.".into());
        }
    }
    if date_filter && selected.contains(&Category::Media) {
        if let Some(posts) = incoming.get(&Category::Posts) {
            media_entries = dated_post_media(plan, posts);
            if text_bytes + media_entries.iter().map(|entry| entry.size).sum::<u64>() > MAX_MEDIA {
                return Err("The selected posts and their media exceed 256 MiB. Choose a narrower date range.".into());
            }
        }
    }
    if incoming.values().map(Vec::len).sum::<usize>() + media_entries.len() > MAX_RECORDS {
        return Err("Too many indexed records. Choose a narrower date range.".into());
    }
    Ok(PreparedSelection {
        incoming,
        imported_formats,
        media_entries,
        first_error,
        skipped_files,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeReview {
    records: BTreeMap<Category, usize>,
    attachments: usize,
    attachment_bytes: u64,
    skipped_files: usize,
}

async fn review_range(
    root: &Path,
    plan: &Plan,
    categories: Vec<Category>,
    from: Option<u64>,
    to: Option<u64>,
    cancel: watch::Receiver<bool>,
) -> Result<RangeReview, String> {
    let selected = selected_categories(plan.provider, categories, Vec::new())?;
    if from.is_none() && to.is_none() {
        return Err("Choose at least one date to review the import range.".into());
    }
    if queue::digest(plan.path.clone(), cancel.clone()).await? != plan.fingerprint {
        return Err("The export changed after preview. Choose it again.".into());
    }
    let scratch = Scratch::new(root)?;
    let prepared = prepare_selection(root, plan, &scratch, &selected, from, to, cancel.clone()).await?;
    if queue::digest(plan.path.clone(), cancel).await? != plan.fingerprint {
        return Err("The export changed during review. Choose it again.".into());
    }
    let records = selected
        .into_iter()
        .filter(|category| *category != Category::Media)
        .map(|category| {
            let count = prepared.incoming.get(&category).map_or(0, |rows| {
                rows.iter().map(|row| row.id.as_str()).collect::<HashSet<_>>().len()
            });
            (category, count)
        })
        .collect();
    Ok(RangeReview {
        records,
        attachments: prepared.media_entries.len(),
        attachment_bytes: prepared.media_entries.iter().map(|entry| entry.size).sum(),
        skipped_files: prepared.skipped_files,
    })
}

#[tauri::command]
pub async fn personal_archive_review_range(
    state: State<'_, Arc<AppState>>,
    fingerprint: String,
    categories: Vec<Category>,
    from: Option<String>,
    to: Option<String>,
    request_id: u64,
) -> Result<RangeReview, String> {
    let plan = state
        .personal_archive
        .plan
        .lock()
        .unwrap()
        .clone()
        .filter(|plan| plan.fingerprint == fingerprint)
        .ok_or("Preview the export again before reviewing the date range.")?;
    let (from, to) = date_range(&from, &to)?;
    let root = crate::config_root(&state)?;
    let cancel = file_sources::begin(&state, request_id)?;
    let result = review_range(&root, &plan, categories, from, to, cancel).await;
    file_sources::finish(&state, request_id);
    result
}

fn merge(records: &mut Records, new: Records) -> Result<(), String> {
    for (category, rows) in new {
        let saved = records.entry(category).or_default();
        let mut positions = saved
            .iter()
            .enumerate()
            .map(|(index, row)| (row.id.clone(), index))
            .collect::<HashMap<_, _>>();
        for row in rows {
            if let Some(&index) = positions.get(&row.id) {
                if category == Category::Listening && saved[index].listening_item.is_none() {
                    saved[index].listening_item = row.listening_item;
                }
                let attachments = &mut saved[index].attachment_ids;
                for id in row.attachment_ids {
                    if attachments.len() < parse::MAX_POST_ATTACHMENTS && !attachments.contains(&id)
                    {
                        attachments.push(id);
                    }
                }
            } else {
                positions.insert(row.id.clone(), saved.len());
                saved.push(row);
            }
        }
    }
    if records.values().map(Vec::len).sum::<usize>() > MAX_RECORDS {
        return Err(
            "This import exceeds 40000 indexed records. Choose a narrower date range.".into(),
        );
    }
    Ok(())
}

async fn import(
    root: &Path,
    archive: &ArchiveState,
    plan: Arc<Plan>,
    selection: Selection,
    cancel: watch::Receiver<bool>,
) -> Result<ImportResult, String> {
    let account = label(&selection.account)?;
    let destination = label(&selection.destination)?;
    let selected = selected_categories(plan.provider, selection.categories, selection.sensitive)?;
    let (from, to) = date_range(&selection.from, &selection.to)?;
    if queue::digest(plan.path.clone(), cancel.clone()).await? != plan.fingerprint {
        return Err("The export changed after preview. Choose it again.".into());
    }
    let scratch = Scratch::new(root)?;
    let PreparedSelection {
        mut incoming,
        mut imported_formats,
        media_entries,
        first_error,
        skipped_files,
    } = prepare_selection(root, &plan, &scratch, &selected, from, to, cancel.clone()).await?;
    if from.is_some() || to.is_some() {
        extract(root, &plan, &scratch, &media_entries, cancel.clone()).await?;
    }
    let mut copies = Vec::new();
    for entry in media_entries {
        cancelled(&cancel)?;
        let category = entry.classification.category;
        copies.push((
            entry_path(&plan, &scratch, entry),
            media_name(&entry.name)?,
            entry.size,
        ));
        incoming
            .entry(category)
            .or_default()
            .push(parse::media_record(&entry.name));
        let formats = imported_formats.entry(category).or_default();
        if !formats.contains(&Format::Media) {
            formats.push(Format::Media);
        }
    }
    if incoming.values().all(Vec::is_empty) {
        return Err(first_error.unwrap_or_else(|| {
            "The selected categories contain no records in this date range.".into()
        }));
    }
    if queue::digest(plan.path.clone(), cancel.clone()).await? != plan.fingerprint {
        return Err("The export changed during import. Choose it again.".into());
    }
    cancelled(&cancel)?;
    let id = format!(
        "{:x}",
        Sha256::digest(format!(
            "{:?}\n{}\n{}",
            plan.provider,
            account.to_lowercase(),
            plan.fingerprint
        ))
    );
    let _lock = archive.storage.lock().unwrap();
    let mut index = load_index(root)?;
    let previous = index.iter().find(|item| item.id == id).cloned();
    if previous.as_ref().is_some_and(|item| item.pending_removal) {
        return Err(
            "Finish removing the previous import before importing this export again.".into(),
        );
    }
    if previous.is_none() && index.len() >= 64 {
        return Err("Remove an existing import before adding another package.".into());
    }
    let mut records = previous
        .as_ref()
        .map(|item| read_records(root, item))
        .transpose()?
        .unwrap_or_default();
    merge(&mut records, incoming)?;
    let mut formats = previous
        .as_ref()
        .map(|item| item.formats.clone())
        .unwrap_or_default();
    for (category, found) in imported_formats {
        let known = formats.entry(category).or_default();
        for format in found {
            if !known.contains(&format) {
                known.push(format);
            }
        }
    }
    let media = media_dir(root, &id)?;
    let bytes = copies
        .iter()
        .filter(|(_, name, _)| !media.join(name).is_file())
        .map(|(_, _, size)| size)
        .sum::<u64>();
    if index
        .iter()
        .map(|item| item.media_bytes)
        .sum::<u64>()
        .saturating_add(bytes)
        > 2 * MAX_INPUT
    {
        return Err("Personal archives reach the 2 GiB managed media limit. Remove an import before copying more media.".into());
    }
    let summary = Summary {
        id,
        generation: previous
            .as_ref()
            .map_or(1, |item| item.generation.saturating_add(1)),
        provider: plan.provider,
        account,
        destination,
        source: source_name(&plan.path),
        fingerprint: plan.fingerprint.clone(),
        categories: records
            .iter()
            .map(|(category, rows)| (*category, rows.len()))
            .collect(),
        formats,
        coverage: plan.coverage.clone(),
        media_bytes: previous.as_ref().map_or(0, |item| item.media_bytes) + bytes,
        pending_removal: false,
    };
    let mut payload = serde_json::to_vec(&records).map_err(|e| e.to_string())?;
    let saved = sessions::save_personal_archive_record(root, &blob_name(&summary), &payload);
    payload.fill(0);
    saved?;
    let mut added = Vec::new();
    let result = (|| {
        if !copies.is_empty() {
            fs::create_dir_all(&media).map_err(|e| e.to_string())?;
            let media = library::authorized_path(&media)?;
            for (input, name, size) in copies {
                let output = media.join(name);
                if output.try_exists().map_err(|e| e.to_string())? {
                    continue;
                }
                let input = library::authorized_path(&input)?;
                if fs::metadata(&input).map_err(|e| e.to_string())?.len() != size {
                    return Err("An extracted media file changed.".into());
                }
                fs::rename(input, &output).map_err(|e| e.to_string())?;
                added.push(output);
            }
        }
        index.retain(|item| item.id != summary.id);
        index.push(summary.clone());
        store_index(root, &index)
    })();
    if let Err(error) = result {
        for path in added {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_file(sessions::personal_archive_record_path(
            root,
            &blob_name(&summary),
        )?);
        return Err(error);
    }
    if let Some(old) = previous {
        let _ = fs::remove_file(sessions::personal_archive_record_path(
            root,
            &blob_name(&old),
        )?);
    }
    Ok(ImportResult {
        archive: summary,
        skipped_files,
    })
}

#[tauri::command]
pub async fn personal_archive_import(
    state: State<'_, Arc<AppState>>,
    selection: Selection,
    request_id: u64,
) -> Result<ImportResult, String> {
    let plan = state
        .personal_archive
        .plan
        .lock()
        .unwrap()
        .clone()
        .filter(|plan| plan.fingerprint == selection.fingerprint)
        .ok_or("Preview the export again before importing.")?;
    let root = crate::config_root(&state)?;
    let cancel = file_sources::begin(&state, request_id)?;
    let result = import(&root, &state.personal_archive, plan, selection, cancel).await;
    file_sources::finish(&state, request_id);
    result
}

#[tauri::command]
pub fn personal_archive_list(state: State<Arc<AppState>>) -> Result<Vec<Summary>, String> {
    let root = crate::config_root(&state)?;
    let _lock = state.personal_archive.storage.lock().unwrap();
    load_index(&root)
}

fn records_for(
    state: &AppState,
    id: &str,
    category: Category,
    consent: bool,
) -> Result<(Summary, Vec<Record>), String> {
    if category.sensitive() && !consent {
        return Err("Open this private category explicitly to read its records.".into());
    }
    let (summary, mut records) = archive_records(state, id)?;
    Ok((summary, records.remove(&category).unwrap_or_default()))
}

fn archive_records(state: &AppState, id: &str) -> Result<(Summary, Records), String> {
    let root = crate::config_root(state)?;
    let _lock = state.personal_archive.storage.lock().unwrap();
    let summary = load_index(&root)?
        .into_iter()
        .find(|item| item.id == id && !item.pending_removal)
        .ok_or("This personal import was removed.")?;
    let records = read_records(&root, &summary)?;
    Ok((summary, records))
}

fn x_attachment_counts(records: &Records) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for owner in records
        .get(&Category::Media)
        .into_iter()
        .flatten()
        .filter_map(|row| parse::x_attachment_owner(row.media.as_deref()?))
    {
        *counts.entry(owner).or_default() += 1;
    }
    counts
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RowView {
    id: String,
    source: String,
    title: String,
    text: String,
    time: Option<u64>,
    link: Option<String>,
    author: Option<String>,
    thread_parent: Option<String>,
    has_thread: bool,
    attachment_count: usize,
    played_ms: Option<u64>,
    track_count: usize,
    media: Option<String>,
}

impl From<Record> for RowView {
    fn from(row: Record) -> Self {
        Self {
            id: row.id,
            source: row.source,
            title: row.title,
            text: row.text,
            time: row.time,
            link: row.link,
            author: row.author,
            thread_parent: row.thread_parent,
            has_thread: false,
            attachment_count: 0,
            played_ms: row.played_ms,
            track_count: row.tracks.len(),
            media: row.media,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    rows: Vec<RowView>,
    total: usize,
    start: Option<u64>,
    end: Option<u64>,
    played_ms: Option<u64>,
    completions: Option<usize>,
    completion_known: usize,
    ended: Option<usize>,
    listening_groups: Option<listening::GroupPage>,
}

#[derive(Deserialize)]
pub struct DateBounds {
    from: Option<String>,
    to: Option<String>,
}

#[tauri::command(async)]
pub fn personal_archive_records(
    state: State<Arc<AppState>>,
    id: String,
    category: Category,
    consent: bool,
    query: String,
    offset: usize,
    listening: Option<listening::View>,
    dates: Option<DateBounds>,
) -> Result<Page, String> {
    if query.chars().count() > 120 {
        return Err("Use a shorter archive search.".into());
    }
    if category.sensitive() && !consent {
        return Err("Open this private category explicitly to read its records.".into());
    }
    let bounds = dates
        .map(|dates| date_range(&dates.from, &dates.to))
        .transpose()?;
    let (summary, mut records) = archive_records(&state, &id)?;
    let mut rows = records.remove(&category).unwrap_or_default();
    if listening.is_some()
        && (summary.provider != Provider::Spotify || category != Category::Listening)
    {
        return Err("Listening summaries are available for imported Spotify history.".into());
    }
    let show_threads = summary.provider == Provider::X && category == Category::Posts;
    let x_counts = if show_threads {
        x_attachment_counts(&records)
    } else {
        HashMap::new()
    };
    let meta_media = if summary.provider.is_meta() && category == Category::Posts {
        records
            .get(&Category::Media)
            .into_iter()
            .flatten()
            .filter(|row| row.media.is_some())
            .map(|row| row.id.as_str())
            .collect::<HashSet<_>>()
    } else {
        HashSet::new()
    };
    let parents: HashSet<_> = if show_threads {
        rows.iter().filter_map(Record::reply_parent).collect()
    } else {
        HashSet::new()
    };
    if let Some((from, to)) = bounds {
        rows.retain(|row| in_range(row.time, from, to));
    }
    let query = query.trim().to_lowercase();
    if !query.is_empty() {
        rows.retain(|row| {
            [&row.title, &row.text]
                .into_iter()
                .chain(row.author.as_ref())
                .chain(
                    row.listening_item
                        .as_ref()
                        .and_then(|item| item.album.as_ref()),
                )
                .any(|text| text.to_lowercase().contains(&query))
        });
    }
    if let Some(listening::View { by, key: Some(key) }) = &listening {
        if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Choose a group from this Spotify history.".into());
        }
        rows.retain(|row| listening::key(row, *by).as_ref() == Some(key));
    }
    let (start, end) = interval(&rows);
    let played_ms = rows
        .iter()
        .filter_map(|row| row.played_ms)
        .reduce(u64::saturating_add);
    let completion_known = rows.iter().filter(|row| row.completed.is_some()).count();
    let completions = (completion_known > 0).then(|| {
        rows.iter()
            .filter(|row| row.completed == Some(true))
            .count()
    });
    let ended = rows.iter().any(|row| row.reached_end.is_some()).then(|| {
        rows.iter()
            .filter(|row| row.reached_end == Some(true))
            .count()
    });
    let total = rows.len();
    let listening_groups = listening
        .filter(|view| view.key.is_none())
        .map(|view| listening::group_page(&rows, view.by, offset));
    if listening_groups.is_some() {
        rows.clear();
    }
    Ok(Page {
        rows: rows
            .into_iter()
            .skip(offset)
            .take(PAGE)
            .map(|row| {
                let has_thread =
                    show_threads && (parents.contains(&row.id) || row.reply_parent().is_some());
                RowView {
                    has_thread,
                    attachment_count: if show_threads {
                        x_counts.get(&row.id).copied().unwrap_or(0)
                    } else {
                        row.attachment_ids
                            .iter()
                            .filter(|id| meta_media.contains(id.as_str()))
                            .count()
                    },
                    ..RowView::from(row)
                }
            })
            .collect(),
        total,
        start,
        end,
        played_ms,
        completions,
        completion_known,
        ended,
        listening_groups,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ThreadPost {
    row: RowView,
    reply_to: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPage {
    posts: Vec<ThreadPost>,
    offset: usize,
    total: usize,
    incomplete: bool,
}

fn archived_thread(
    rows: Vec<Record>,
    record: &str,
    offset: Option<usize>,
) -> Result<ThreadPage, String> {
    let by_id: HashMap<_, _> = rows.iter().map(|row| (row.id.as_str(), row)).collect();
    let parents: HashMap<_, _> = rows
        .iter()
        .filter_map(|row| Some((row.id.as_str(), row.reply_parent()?)))
        .collect();
    let mut replies = HashMap::<&str, Vec<&Record>>::new();
    for (child, parent) in &parents {
        replies.entry(parent).or_default().push(by_id[child]);
    }
    for siblings in replies.values_mut() {
        siblings.sort_by(|a, b| {
            a.time
                .unwrap_or(u64::MAX)
                .cmp(&b.time.unwrap_or(u64::MAX))
                .then_with(|| a.id.cmp(&b.id))
        });
    }

    let mut root = by_id
        .get(record)
        .ok_or("The selected post is no longer in this export.")?
        .id
        .as_str();
    let mut seen = HashSet::from([root]);
    let mut incomplete = false;
    while let Some(parent) = parents.get(root) {
        if !seen.insert(parent) || !by_id.contains_key(parent.as_str()) {
            incomplete = true;
            root = parent;
            break;
        }
        root = parent;
    }
    let mut stack = vec![root];
    let mut ordered = Vec::new();
    let mut positions = HashMap::new();
    seen.clear();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(row) = by_id.get(id) {
            ordered.push(*row);
            positions.insert(id, ordered.len());
        }
        if let Some(children) = replies.get(id) {
            stack.extend(children.iter().rev().map(|row| row.id.as_str()));
        }
    }
    let total = ordered.len();
    let selected = positions
        .get(record)
        .copied()
        .ok_or("The selected post could not be placed in the thread.")?;
    let offset = offset.unwrap_or(selected - 1).min(total - 1) / PAGE * PAGE;
    let posts = ordered
        .into_iter()
        .skip(offset)
        .take(PAGE)
        .map(|row| ThreadPost {
            row: RowView::from(row.clone()),
            reply_to: parents
                .get(row.id.as_str())
                .and_then(|parent| positions.get(parent.as_str()))
                .copied(),
        })
        .collect();
    Ok(ThreadPage {
        posts,
        offset,
        total,
        incomplete,
    })
}

#[tauri::command]
pub async fn personal_archive_thread(
    state: State<'_, Arc<AppState>>,
    id: String,
    record: String,
    offset: Option<usize>,
) -> Result<ThreadPage, String> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let (summary, mut records) = archive_records(&state, &id)?;
        if summary.provider != Provider::X {
            return Err("Threads are available for imported X posts.".into());
        }
        let rows = records.remove(&Category::Posts).unwrap_or_default();
        let mut page = archived_thread(rows, &record, offset)?;
        let counts = x_attachment_counts(&records);
        for post in &mut page.posts {
            post.row.attachment_count = counts.get(&post.row.id).copied().unwrap_or(0);
        }
        Ok(page)
    })
    .await
    .map_err(|_| "Could not read the archived thread.")?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentPage {
    rows: Vec<RowView>,
    total: usize,
    offset: usize,
}

fn attachment_page(
    provider: Provider,
    mut records: Records,
    post: &str,
    offset: usize,
) -> Result<AttachmentPage, String> {
    let media = records.remove(&Category::Media).unwrap_or_default();
    let post_record = records
        .get(&Category::Posts)
        .and_then(|rows| rows.iter().find(|row| row.id == post))
        .ok_or("The selected post is no longer in this export.")?;
    let rows: Vec<_> = if provider.is_meta() {
        let mut by_id = media
            .into_iter()
            .filter(|row| row.media.is_some())
            .map(|row| (row.id.clone(), row))
            .collect::<HashMap<_, _>>();
        post_record
            .attachment_ids
            .iter()
            .filter_map(|id| by_id.remove(id))
            .collect()
    } else {
        let mut rows: Vec<_> = media
            .into_iter()
            .filter(|row| {
                row.media
                    .as_deref()
                    .and_then(parse::x_attachment_owner)
                    .as_deref()
                    == Some(post)
            })
            .collect();
        rows.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.source.cmp(&b.source)));
        rows
    };
    let total = rows.len();
    let offset = offset.min(total.saturating_sub(1)) / PAGE * PAGE;
    let rows = rows
        .into_iter()
        .skip(offset)
        .take(PAGE)
        .map(RowView::from)
        .collect();
    Ok(AttachmentPage {
        rows,
        total,
        offset,
    })
}

#[tauri::command(async)]
pub fn personal_archive_attachments(
    state: State<Arc<AppState>>,
    id: String,
    post: String,
    offset: usize,
) -> Result<AttachmentPage, String> {
    let (summary, records) = archive_records(&state, &id)?;
    if summary.provider != Provider::X && !summary.provider.is_meta() {
        return Err(
            "Post attachments are available for imported X, Instagram and Facebook posts.".into(),
        );
    }
    attachment_page(summary.provider, records, &post, offset)
}

#[tauri::command]
pub fn personal_archive_links(
    state: State<Arc<AppState>>,
    id: String,
    category: Category,
    records: Vec<String>,
) -> Result<Vec<String>, String> {
    if records.is_empty() || records.len() > 100 {
        return Err("Select 1–100 records with public links.".into());
    }
    let (_, rows) = records_for(&state, &id, category, false)?;
    let mut seen = HashSet::new();
    Ok(rows
        .into_iter()
        .filter(|row| records.contains(&row.id))
        .filter_map(|row| row.link)
        .filter(|link| seen.insert(link.clone()))
        .collect())
}

fn csv_field(text: &str) -> String {
    let escaped = text.replace('"', "\"\"");
    let marker = if text.trim_start().starts_with(['=', '+', '-', '@'])
        || text.starts_with(['\t', '\r', '\n'])
    {
        "'"
    } else {
        ""
    };
    format!("\"{marker}{escaped}\"")
}

async fn save_as(
    app: AppHandle,
    suggested: String,
    extension: String,
) -> Result<Option<PathBuf>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_file_name(&suggested)
            .add_filter("Export", &[&extension])
            .blocking_save_file()
            .map(|file| file.into_path())
            .transpose()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

fn new_output(path: &Path) -> Result<File, String> {
    let parent = path.parent().ok_or("Choose an existing output folder.")?;
    let parent = library::authorized_path(parent)?;
    let name = path.file_name().ok_or("Choose a file name.")?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(parent.join(name))
        .map_err(|_| "Choose a new file. Existing files are preserved.".into())
}

#[tauri::command]
pub async fn personal_archive_export(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    category: Category,
    records: Vec<String>,
    consent: bool,
) -> Result<bool, String> {
    if records.is_empty() || records.len() > 200 {
        return Err("Select 1–200 archive records.".into());
    }
    let (_, rows) = records_for(&state, &id, category, consent)?;
    let rows = rows
        .into_iter()
        .filter(|row| records.contains(&row.id))
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Err("The selected archive records are unavailable.".into());
    }
    let Some(path) = save_as(app, "nabnab-personal-selection.csv".into(), "csv".into()).await?
    else {
        return Ok(false);
    };
    let mut output = new_output(&path)?;
    writeln!(
        output,
        "Title,Author,Timestamp (UTC milliseconds),Text,Public URL,Source"
    )
    .map_err(|e| e.to_string())?;
    for row in rows {
        let time = row.time.map(|stamp| stamp.to_string()).unwrap_or_default();
        let fields: [&str; 6] = [
            &row.title,
            row.author.as_deref().unwrap_or(""),
            &time,
            &row.text,
            row.link.as_deref().unwrap_or(""),
            &row.source,
        ];
        writeln!(
            output,
            "{}",
            fields
                .into_iter()
                .map(csv_field)
                .collect::<Vec<_>>()
                .join(",")
        )
        .map_err(|e| e.to_string())?;
    }
    output.sync_all().map_err(|e| e.to_string())?;
    Ok(true)
}

#[tauri::command]
pub async fn personal_archive_copy_media(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    category: Category,
    record: String,
    consent: bool,
) -> Result<bool, String> {
    let source = media_source(&state, &id, category, &record, consent)?;
    let filename = source.rsplit('/').next().unwrap_or(&source).to_owned();
    let extension = Path::new(&filename)
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or("This media file has no extension.")?
        .to_owned();
    let Some(path) = save_as(app, filename, extension).await? else {
        return Ok(false);
    };
    let input = managed_media(&state, &id, &source)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut input = File::open(input).map_err(|e| e.to_string())?;
        let mut output = new_output(&path)?;
        std::io::copy(&mut input, &mut output)
            .and_then(|_| output.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(true)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn personal_archive_remove(state: State<Arc<AppState>>, id: String) -> Result<(), String> {
    let root = crate::config_root(&state)?;
    remove(&root, &state.personal_archive, &id)
}

fn remove(root: &Path, archive: &ArchiveState, id: &str) -> Result<(), String> {
    let media = media_dir(root, id)?;
    let _lock = archive.storage.lock().unwrap();
    let mut index = load_index(root)?;
    let position = index
        .iter()
        .position(|item| item.id == id)
        .ok_or("This personal import was removed.")?;
    index[position].pending_removal = true;
    store_index(root, &index)?;
    let data = sessions::secure_module_directory(root, "personal-archive-data")?;
    let prefix = format!("{id}-");
    for item in fs::read_dir(data).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        if item
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix(&prefix))
            .and_then(|name| name.strip_suffix(".bin"))
            .is_some_and(|generation| {
                generation.len() == 8 && generation.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            fs::remove_file(item.path())
                .map_err(|e| format!("Retry removing the protected records: {e}"))?;
        }
    }
    if media.try_exists().map_err(|e| e.to_string())? {
        fs::remove_dir_all(library::authorized_path(&media)?)
            .map_err(|e| format!("Retry removing the managed copies: {e}"))?;
    }
    index.remove(position);
    store_index(root, &index)
}

#[tauri::command]
pub fn personal_archive_match(
    state: State<Arc<AppState>>,
    id: String,
    record: String,
) -> Result<crate::audio_library::playlist_import::Preview, String> {
    let (_, rows) = records_for(&state, &id, Category::Playlists, false)?;
    let row = rows
        .into_iter()
        .find(|row| row.id == record && !row.tracks.is_empty())
        .ok_or("Choose a playlist containing tracks.")?;
    crate::audio_library::playlist_import::from_archive(&state, row.title, row.tracks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[tokio::test]
    async fn imported_x_threads_keep_branches_and_link_only_matching_local_attachments() {
        let fixture = Scratch(std::env::temp_dir().join(format!(
            "nabnab-thread-{}-{}",
            std::process::id(),
            queue::now()
        )));
        fs::create_dir(&fixture.0).unwrap();
        let tweet = |id: u64, parent: Option<u64>| {
            serde_json::json!({"tweet": {
                "id_str": id.to_string(), "full_text": format!("Post {id}"), "timestamp": 1717200000 + id,
                "in_reply_to_status_id_str": parent.map(|value| value.to_string()),
            }})
        };
        let mut posts = vec![tweet(1, None), tweet(2, Some(1)), tweet(3, Some(1))];
        posts.extend((4..=55).map(|id| tweet(id, Some(if id == 4 { 2 } else { id - 1 }))));
        posts.extend([
            tweet(56, Some(99)),
            tweet(57, Some(99)),
            tweet(58, Some(59)),
            tweet(59, Some(58)),
        ]);
        let data = fixture.0.join("tweets.js");
        fs::write(
            &data,
            format!("window.YTD.tweets.part0 = {};", serde_json::json!(posts)),
        )
        .unwrap();
        let export = fixture.0.join("x-export.zip");
        let media = fixture.0.join("photo.png");
        fs::write(&media, b"synthetic media bytes").unwrap();
        let (_sender, cancel) = watch::channel(false);
        crate::portable_zip::write_generated(
            &export,
            &[
                ("data/tweets.js".into(), data),
                ("data/tweets_media/1-b-photo.png".into(), media.clone()),
                ("data/tweets_media/1-a-video.mp4".into(), media.clone()),
                ("data/tweets_media/11-photo.png".into(), media.clone()),
                ("data/tweets_media/99-orphan.png".into(), media.clone()),
                ("data/tweets_media/1+not-an-id.png".into(), media.clone()),
                ("data/messages/tweets_media/1-private.png".into(), media),
            ],
            cancel.clone(),
        )
        .await
        .unwrap();
        let root = fixture.0.join("profile");
        let (plan, preview) = inspect(&root, Provider::X, export, cancel.clone())
            .await
            .unwrap();
        let plan = Arc::new(plan);
        let storage = ArchiveState::default();
        let selection = |categories, sensitive| Selection {
            fingerprint: preview.fingerprint.clone(),
            account: "Cuenta de prueba".into(),
            destination: "Archivo".into(),
            categories,
            sensitive,
            from: None,
            to: None,
        };
        let first = import(
            &root,
            &storage,
            plan.clone(),
            selection(vec![Category::Posts], vec![]),
            cancel.clone(),
        )
        .await
        .unwrap()
        .archive;
        assert!(x_attachment_counts(&read_records(&root, &first).unwrap()).is_empty());
        let archive = import(
            &root,
            &storage,
            plan.clone(),
            selection(vec![Category::Media], vec![Category::Messages]),
            cancel.clone(),
        )
        .await
        .unwrap()
        .archive;
        assert_eq!(archive.id, first.id);
        let mut records = read_records(&root, &archive).unwrap();
        let key = |id| format!("{:x}", Sha256::digest(format!("{id}")));
        assert_eq!(records[&Category::Messages].len(), 1);
        let counts = x_attachment_counts(&records);
        assert_eq!(counts[&key(1)], 2);
        assert_eq!(counts[&key(11)], 1);
        assert!(!counts.contains_key(&key(2)));
        let attachments =
            attachment_page(Provider::X, records.clone(), &key(1), usize::MAX).unwrap();
        assert_eq!((attachments.total, attachments.offset), (2, 0));
        assert_eq!(
            attachments
                .rows
                .iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>(),
            ["1-a-video.mp4", "1-b-photo.png"]
        );
        assert!(attachment_page(Provider::X, records.clone(), &key(99), 0).is_err());
        let dated = import(
            &root,
            &storage,
            plan,
            Selection {
                account: "Cuenta con intervalo".into(),
                from: Some("2024-06-01".into()),
                to: Some("2024-06-01".into()),
                ..selection(
                    vec![Category::Posts, Category::Media],
                    vec![Category::Messages],
                )
            },
            cancel,
        )
        .await
        .unwrap()
        .archive;
        let dated_records = read_records(&root, &dated).unwrap();
        assert_eq!(dated_records[&Category::Media].len(), 3);
        assert!(!dated_records.contains_key(&Category::Messages));
        assert_eq!(x_attachment_counts(&dated_records)[&key(1)], 2);
        let rows = records.remove(&Category::Posts).unwrap();

        let first = archived_thread(rows.clone(), &key(1), None).unwrap();
        assert!(!first.incomplete);
        assert_eq!((first.offset, first.total, first.posts.len()), (0, 55, 50));
        assert_eq!(first.posts[0].row.text, "Post 1");
        assert_eq!(first.posts[1].reply_to, Some(1));
        assert_eq!(first.posts[2].row.text, "Post 4");
        assert_eq!(first.posts[2].reply_to, Some(2));
        let last = archived_thread(rows.clone(), &key(55), None).unwrap();
        assert_eq!((last.offset, last.posts.len()), (50, 5));
        assert_eq!(last.posts[4].row.text, "Post 3");
        assert_eq!(last.posts[4].reply_to, Some(1));
        assert!(last.posts.iter().any(|post| post.row.id == key(55)));
        let gap = archived_thread(rows.clone(), &key(56), None).unwrap();
        assert!(gap.incomplete);
        assert_eq!(gap.total, 2);
        assert_eq!(gap.posts[0].reply_to, None);
        let cycle = archived_thread(rows, &key(58), None).unwrap();
        assert!(cycle.incomplete);
        assert_eq!(cycle.total, 2);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_zip_can_be_imported_in_parts_and_withdrawn_without_changing_the_original() {
        let temporary = std::env::temp_dir().join(format!(
            "nabnab-archive-test-{}-{}",
            std::process::id(),
            queue::now()
        ));
        fs::create_dir(&temporary).unwrap();
        let fixture = Scratch(temporary);
        let files = [
            (
                "your_facebook_activity/posts/your_posts.json",
                r#"[{"id":"1","content":"A fixture post","timestamp":1717200000,"attachments":[{"data":[
                    {"media":{"uri":"your_facebook_activity/posts/media/day/clip.mp4"}},
                    {"media":{"uri":"your_facebook_activity/posts/media/day/photo.jpg"}},
                    {"media":{"uri":"your_facebook_activity/messages/inbox/thread/photos/photo.jpg"}},
                    {"media":{"uri":"your_facebook_activity/posts/media/day/missing.jpg"}}
                ]}]},{"id":"2","content":"The following day","timestamp":1717286400,"attachments":[{"data":[
                    {"media":{"uri":"your_facebook_activity/posts/media/other/photo.jpg"}}
                ]}]}]"#,
            ),
            (
                "your_facebook_activity/messages/inbox/thread/message_1.json",
                r#"{"messages":[{"content":"Synthetic private message","sender_name":"Example Person","timestamp_ms":1717200000000}]}"#,
            ),
            (
                "your_facebook_activity/messages/inbox/thread/photos/photo.jpg",
                "managed fixture bytes",
            ),
            (
                "security/session_cookies.json",
                r#"[{"content":"excluded cookie fixture"}]"#,
            ),
            (
                "your_facebook_activity/posts/media/day/photo.jpg",
                "public photo",
            ),
            (
                "your_facebook_activity/posts/media/day/clip.mp4",
                "public video",
            ),
            (
                "your_facebook_activity/posts/media/other/photo.jpg",
                "same filename, different path",
            ),
            ("your_facebook_activity/posts/your_posts_future.json", r#"{"futureSchema":true}"#),
            ("your_facebook_activity/posts/your_posts_other.xml", "<posts/>"),
            ("your_facebook_activity/saved_items.json", "[]"),
        ];
        let mut inputs = files
            .iter()
            .enumerate()
            .map(|(index, (name, content))| {
                let file = fixture.0.join(index.to_string());
                fs::write(&file, content).unwrap();
                (name.to_string(), file)
            })
            .collect::<Vec<_>>();
        let oversized = fixture.0.join("oversized");
        File::create(&oversized).unwrap().set_len(MAX_TEXT + 1).unwrap();
        inputs.push(("your_facebook_activity/contacts/contacts.json".into(), oversized));
        let source = fixture.0.join("facebook-export.zip");
        let (_sender, cancel) = watch::channel(false);
        crate::portable_zip::write_generated(&source, &inputs, cancel.clone())
            .await
            .unwrap();
        let original = fs::read(&source).unwrap();
        let root = fixture.0.join("profile");
        let (plan, preview) = inspect(&root, Provider::Facebook, source.clone(), cancel.clone())
            .await
            .unwrap();
        assert!(preview.dated_post_media);
        let from = Some("2024-06-01".to_string());
        let to = from.clone();
        let (start, end) = date_range(&from, &to).unwrap();
        let review = review_range(
            &root, &plan, vec![Category::Posts, Category::Media], start, end, cancel.clone(),
        )
            .await
            .unwrap();
        assert_eq!(review.records, BTreeMap::from([(Category::Posts, 1)]));
        assert_eq!(review.attachments, 2);
        assert_eq!(review.attachment_bytes, (files[4].1.len() + files[5].1.len()) as u64);
        assert_eq!(review.skipped_files, 2);
        let posts = review_range(&root, &plan, vec![Category::Posts], start, end, cancel.clone())
            .await
            .unwrap();
        assert_eq!(posts.records, review.records);
        assert_eq!(posts.attachments, 0);
        assert!(review_range(&root, &plan, vec![Category::Messages], start, end, cancel.clone())
            .await
            .is_err_and(|error| error.contains("Select messages and contacts separately")));
        assert!(load_index(&root).unwrap().is_empty());
        assert!(!root.join("personal-archive-media").exists());
        let messages = preview
            .categories
            .iter()
            .find(|item| item.category == Category::Messages)
            .unwrap();
        assert!(matches!(
            messages.availability,
            Availability::RequiresSelection
        ));
        assert_eq!(messages.records, None);
        assert_eq!(messages.files, 2);
        assert_eq!(messages.incompatible_files, 0);
        let category = |value| preview.categories.iter().find(|item| item.category == value).unwrap();
        let posts = category(Category::Posts);
        assert!(matches!(posts.availability, Availability::Available));
        assert_eq!(posts.records, Some(2));
        assert_eq!(posts.incompatible_files, 2);
        assert!(matches!(category(Category::Saved).availability, Availability::Empty));
        let contacts = category(Category::Contacts);
        assert!(matches!(contacts.availability, Availability::OverLimit));
        assert_eq!(contacts.over_limit_files, 1);
        assert_eq!(contacts.records, None);
        let plan = Arc::new(plan);
        let archive = ArchiveState::default();
        let selection = |categories, sensitive| Selection {
            fingerprint: preview.fingerprint.clone(),
            account: "Fixture account".into(),
            destination: "Fixture group".into(),
            categories,
            sensitive,
            from: None,
            to: None,
        };
        let imported = import(
            &root,
            &archive,
            plan.clone(),
            Selection {
                from: Some("2024-06-01".into()),
                to: Some("2024-06-01".into()),
                ..selection(vec![Category::Posts], vec![])
            },
            cancel.clone(),
        )
        .await
        .unwrap();
        assert_eq!(imported.skipped_files, 2);
        let imported = imported.archive;
        assert_eq!(imported.categories, BTreeMap::from([(Category::Posts, 1)]));
        assert_eq!(imported.media_bytes, 0);
        let coverage = serde_json::json!({"posts": "available", "saved": "empty", "media": "available"});
        let saved = load_index(&root).unwrap();
        assert_eq!(serde_json::to_value(&saved[0]).unwrap()["coverage"], coverage);
        let first = read_records(&root, &imported).unwrap();
        let post_id = first[&Category::Posts][0].id.clone();
        assert_eq!(
            attachment_page(Provider::Facebook, first, &post_id, 0)
                .unwrap()
                .total,
            0
        );
        let private = import(
            &root,
            &archive,
            plan.clone(),
            selection(vec![], vec![Category::Messages]),
            cancel.clone(),
        )
        .await
        .unwrap()
        .archive;
        assert_eq!(private.id, imported.id);
        let saved = load_index(&root).unwrap();
        assert_eq!(serde_json::to_value(&saved[0]).unwrap()["coverage"], coverage);
        let repeated = import(
            &root,
            &archive,
            plan.clone(),
            Selection {
                from: Some("2024-06-01".into()),
                to: Some("2024-06-01".into()),
                ..selection(
                    vec![Category::Posts, Category::Media],
                    vec![Category::Messages],
                )
            },
            cancel.clone(),
        )
        .await
        .unwrap()
        .archive;
        assert_eq!(
            repeated.categories,
            BTreeMap::from([
                (Category::Posts, 1),
                (Category::Messages, 2),
                (Category::Media, 2)
            ])
        );
        assert_eq!(
            repeated.media_bytes,
            [2, 4, 5]
                .iter()
                .map(|index| files[*index].1.len() as u64)
                .sum::<u64>()
        );
        let attached = attachment_page(
            Provider::Facebook,
            read_records(&root, &repeated).unwrap(),
            &post_id,
            0,
        )
        .unwrap();
        assert_eq!(attached.total, 2);
        assert_eq!(
            attached
                .rows
                .iter()
                .map(|row| row.source.as_str())
                .collect::<Vec<_>>(),
            [files[5].0, files[4].0]
        );
        assert_eq!(
            repeated.formats[&Category::Messages],
            [Format::Json, Format::Media]
        );
        let media = media_dir(&root, &repeated.id).unwrap();
        assert!(!media.join(media_name(files[6].0).unwrap()).exists());
        assert_eq!(
            fs::read(media.join(media_name(files[2].0).unwrap())).unwrap(),
            b"managed fixture bytes"
        );
        let retried = import(
            &root,
            &archive,
            plan,
            Selection {
                from: Some("2024-06-01".into()),
                to: Some("2024-06-01".into()),
                ..selection(vec![Category::Posts, Category::Media], vec![])
            },
            cancel,
        )
        .await
        .unwrap()
        .archive;
        assert_eq!(retried.categories, repeated.categories);
        assert_eq!(retried.media_bytes, repeated.media_bytes);
        let record_path =
            sessions::personal_archive_record_path(&root, &blob_name(&retried)).unwrap();
        let stored = fs::read(&record_path).unwrap();
        let phrase = b"Synthetic private message";
        assert!(!stored.windows(phrase.len()).any(|part| part == phrase));
        assert!(read_records(&root, &retried).unwrap()[&Category::Messages]
            .iter()
            .any(|row| row.text == "Synthetic private message"));
        assert_eq!(
            fs::read_dir(root.join("personal-archive-staging"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            fs::read_dir(root.join("portable-plans")).unwrap().count(),
            0
        );
        remove(&root, &archive, &repeated.id).unwrap();
        assert!(load_index(&root).unwrap().is_empty());
        assert!(!record_path.exists());
        assert!(!media.exists());
        assert_eq!(fs::read(&source).unwrap(), original);
    }

    #[test]
    fn repeated_and_partial_imports_preserve_prior_categories_without_duplicates() {
        let first = parse::media_record("media/photo.jpg");
        let second = parse::media_record("media/clip.mp4");
        let mut saved = Records::from([
            (Category::Media, vec![first.clone()]),
            (
                Category::Messages,
                vec![parse::media_record("messages/photo.jpg")],
            ),
        ]);
        merge(
            &mut saved,
            Records::from([(Category::Media, vec![first.clone(), second.clone()])]),
        )
        .unwrap();
        assert_eq!(saved[&Category::Media], [first, second]);
        assert_eq!(saved[&Category::Messages].len(), 1);
    }

    #[test]
    fn date_filters_and_csv_protect_unknown_times_and_formula_cells() {
        let day = Some("2024-06-01".into());
        let start = date(&day, false).unwrap();
        let end = date(&day, true).unwrap();
        assert!(in_range(start, start, end));
        assert!(in_range(end, start, end));
        assert!(!in_range(None, start, end));
        assert!(in_range(None, None, None));
        assert_eq!(csv_field("  =2+2"), "\"'  =2+2\"");
        assert_eq!(csv_field("hello \"there\""), "\"hello \"\"there\"\"\"");
    }
}
