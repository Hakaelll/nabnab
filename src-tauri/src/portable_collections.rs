use crate::{library::{self, LibraryItem}, model::Progress, portable_zip, store::Repository};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::{HashMap, HashSet}, fs::{self, File, OpenOptions}, io::{Read, Write}, path::{Path, PathBuf}, sync::atomic::{AtomicU64, Ordering}};
use tokio::sync::watch;

pub(crate) mod package;
const MAX_FILES: usize = 10_000;
const MAX_ITEMS: usize = 2_000;
const MAX_BYTES: u64 = 128 * 1024 * 1024 * 1024;
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportOptions { pub metadata: bool, pub notes: bool, pub ocr: bool }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileEntry { pub path: String, pub size: u64, pub sha256: String }
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Metadata {
    pub collections: Vec<String>, pub tags: Vec<String>, pub creator: Option<String>, pub platform: Option<String>,
    pub source_url: Option<String>, pub license: Option<String>, pub attribution: Option<String>, pub favorite: bool,
    pub playback_position: f64, pub bookmarks: Vec<crate::library_features::Bookmark>, pub chapters: Vec<crate::model::Chapter>,
    pub tracks: Vec<crate::captions::LocalTrack>,
    #[serde(default, skip_serializing_if="Option::is_none")] pub narration: Option<crate::library_features::NarrationReference>,
    #[serde(default, skip_serializing_if="Option::is_none")] pub recording: Option<crate::library_features::RecordingReference>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Item {
    pub id: String, pub name: String, pub filename: String, pub file: String, pub parent_id: Option<String>, pub metadata: Metadata,
    pub capture: Option<String>, pub ocr: Option<String>, pub annotations: Vec<crate::web_archive::Annotation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Manifest { pub schema_version: u32, pub format: String, pub created_at: u64, pub items: Vec<Item>, pub files: Vec<FileEntry> }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewEntry { pub id: String, pub name: String, pub size: u64, pub sha256: String, pub duplicate_of: Option<String>, pub article: bool, pub ocr: bool }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPreview { pub entries: Vec<PreviewEntry>, pub bytes: u64, pub unique_bytes: u64, pub duplicate_bytes: u64, pub warnings: Vec<String> }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview { pub plan_id: String, pub expires_at: u64, pub entries: Vec<PreviewEntry>, pub bytes: u64, pub unique_bytes: u64, pub duplicate_bytes: u64, pub target: PathBuf, pub warnings: Vec<String> }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan { nonce: String, created_at: u64, expires_at: u64, source: PathBuf, zipped: bool, source_hash: Option<String>, target: PathBuf, manifest: Manifest, existing: Vec<LibraryItem> }
struct PreviewDirectory { path: PathBuf, parent: PathBuf }
impl PreviewDirectory { fn new(root: &Path) -> Result<Self, String> { let parent = library::authorized_path(&directory(root)?)?; let path = parent.join(format!("metadata-{}", nonce())); fs::create_dir(&path).map_err(|e| e.to_string())?; Ok(Self { path, parent }) } }
impl Drop for PreviewDirectory { fn drop(&mut self) { if self.path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("metadata-")) && library::authorized_path(&self.path).is_ok_and(|path| path.parent() == Some(self.parent.as_path())) { let _ = fs::remove_dir_all(&self.path); } } }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedImport { pub plan_id: String, pub items: Vec<LibraryItem>, pub report_path: PathBuf }

fn cancel(receiver: &watch::Receiver<bool>) -> Result<(), String> { if *receiver.borrow() { Err(crate::engine::CANCELLED.into()) } else { Ok(()) } }
fn hash(value: &[u8]) -> String { format!("{:x}", Sha256::digest(value)) }
fn valid_hash(value: &str) -> bool { value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) }
fn nonce() -> String { hash(format!("{}:{}:{}", crate::queue::now(), std::process::id(), SEQUENCE.fetch_add(1, Ordering::SeqCst)).as_bytes()) }
pub(super) fn fingerprint(path: &Path, receiver: &watch::Receiver<bool>) -> Result<String, String> {
    let expected = portable_zip::regular_file(path)?; let mut file = File::open(path).map_err(|e| e.to_string())?; let mut digest = Sha256::new(); let mut bytes = 0u64; let mut buffer = [0u8; 64 * 1024];
    loop { cancel(receiver)?; let read = file.read(&mut buffer).map_err(|e| e.to_string())?; if read == 0 { break; } bytes += read as u64; if bytes > expected { return Err("A selected package file changed while being inspected.".into()); } digest.update(&buffer[..read]); }
    if bytes != expected { return Err("A selected package file changed while being inspected.".into()); } Ok(format!("{:x}", digest.finalize()))
}
fn source_url(value: Option<&str>) -> Option<String> { let mut parsed = url::Url::parse(value?).ok()?; if !matches!(parsed.scheme(), "https" | "http") || !parsed.username().is_empty() || parsed.password().is_some() { return None; } parsed.set_query(None); parsed.set_fragment(None); Some(parsed.to_string()) }
fn text(value: &str, limit: usize) -> String {
    let lower = value.to_ascii_lowercase();
    if ["bearer ", "access_token", "api_key", "password=", "token=", "auth=", "session=", "signature=", "x-amz-", "x-goog-", ":\\", "file://"].iter().any(|needle| lower.contains(needle)) { return "[sensitive metadata omitted]".into(); }
    value.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')).take(limit).collect()
}
fn narration_metadata(value: &Option<crate::library_features::NarrationReference>) -> Option<crate::library_features::NarrationReference> {
    value.as_ref().map(|value| crate::library_features::NarrationReference {plan_ref:None,text_hash:value.text_hash.clone(),source_version:value.source_version.clone(),voice_name:text(&value.voice_name,200),language:text(&value.language,40),engine_fingerprint:value.engine_fingerprint.clone()})
}
fn recording_metadata(value: &Option<crate::library_features::RecordingReference>) -> Option<crate::library_features::RecordingReference> {
    value.as_ref().map(|value| crate::library_features::RecordingReference {capture_ref:None,source_label:text(&value.source_label,500),kind:value.kind.clone(),duration_ms:value.duration_ms,interrupted:value.interrupted,reason:value.reason.as_deref().map(|reason|text(reason,2000))})
}
fn metadata(item: &LibraryItem, options: &ExportOptions) -> Metadata {
    if !options.metadata { let mut metadata = Metadata::default(); if options.notes { metadata.bookmarks = item.details.bookmarks.iter().take(400).map(|bookmark| crate::library_features::Bookmark { id: hash(format!("bookmark:{}:{}", bookmark.name, bookmark.time).as_bytes()), name: text(&bookmark.name, 2000), time: bookmark.time }).collect(); } return metadata; }
    let details = &item.details;
    Metadata { narration:narration_metadata(&details.narration),recording:recording_metadata(&details.recording),collections: item.collections.iter().take(200).map(|value| text(value, 120)).collect(), tags: item.tags.iter().take(200).map(|value| text(value, 120)).collect(), creator: details.creator.as_deref().map(|v| text(v, 2000)), platform: details.platform.as_deref().map(|v| text(v, 200)), source_url: source_url(details.source_url.as_deref().or(item.source.as_deref())), license: details.license.as_deref().map(|v| text(v, 2000)), attribution: details.attribution.as_deref().map(|v| text(v, 8000)), favorite: details.favorite, playback_position: details.playback_position, bookmarks: if options.notes { details.bookmarks.iter().take(400).map(|b| crate::library_features::Bookmark { id: hash(format!("bookmark:{}:{}", b.name, b.time).as_bytes()), name: text(&b.name, 2000), time: b.time }).collect() } else { vec![] }, chapters: details.chapters.iter().take(if details.narration.is_some(){4096}else{1000}).map(|c| crate::model::Chapter { title: text(&c.title, 2000), start: c.start, end: c.end }).collect(), tracks: details.tracks.iter().take(20).enumerate().map(|(index, track)| crate::captions::LocalTrack { id: format!("track-{index}"), language: text(&track.language, 80), origin: track.origin.clone(), path: None, segments: track.segments.clone(), formatting_lost: track.formatting_lost }).collect() }
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path, max: u64) -> Result<T, String> { if portable_zip::regular_file(path)? > max { return Err("Package metadata exceeds its size limit.".into()); } serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|_| "The package metadata is unreadable or has an unsupported schema.".into()) }
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> { let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?; if bytes.len() as u64 > MAX_MANIFEST { return Err("The portable metadata exceeds 16 MiB.".into()); } let mut file = OpenOptions::new().write(true).create_new(true).open(path).map_err(|e| e.to_string())?; file.write_all(&bytes).and_then(|_| file.sync_all()).map_err(|e| e.to_string()) }
fn directory(root: &Path) -> Result<PathBuf, String> { crate::sessions::secure_module_directory(root, "portable-plans") }
fn managed_path(root: &Path, name: &str) -> Result<PathBuf, String> {
    portable_zip::validate_name(name)?; let mut current = root.to_owned();
    for component in name.split('/') { current.push(component); if let Ok(metadata) = fs::symlink_metadata(&current) { #[cfg(windows)] { use std::os::windows::fs::MetadataExt; if metadata.file_attributes() & 0x400 != 0 { return Err("Portable files cannot follow reparse points.".into()); } } if metadata.file_type().is_symlink() { return Err("Portable files cannot follow symbolic links.".into()); } } }
    Ok(current)
}
fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.schema_version != 1 || manifest.format != "nabnab-portable" || manifest.items.is_empty() || manifest.items.len() > MAX_ITEMS || manifest.files.is_empty() || manifest.files.len() > MAX_FILES { return Err("This is not a supported bounded nabnab portable collection.".into()); }
    let mut files = HashSet::new(); let mut total = 0u64;
    for entry in &manifest.files { portable_zip::validate_name(&entry.path)?; if entry.path.eq_ignore_ascii_case("manifest.json") || !files.insert(entry.path.to_lowercase()) || !valid_hash(&entry.sha256) { return Err("The package manifest contains colliding paths or invalid checksums.".into()); } total = total.checked_add(entry.size).filter(|value| *value <= MAX_BYTES).ok_or("The portable collection exceeds 128 GiB.")?; }
    let mut ids = HashSet::new();
    for item in &manifest.items {
        if item.id.is_empty() || item.id.len() > 100 || !item.id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)) || !ids.insert(&item.id) || item.name.is_empty() || item.name.len() > 1000 || item.annotations.len() > 400 { return Err("Invalid portable item identity, name or annotation count.".into()); }
        crate::direct::validate_filename(&item.filename)?;
        for path in std::iter::once(&item.file).chain(item.capture.iter()).chain(item.ocr.iter()) { if !manifest.files.iter().any(|entry| &entry.path == path) { return Err("A portable item references an unlisted relative file.".into()); } }
        if item.capture.as_ref().is_some_and(|path| manifest.files.iter().any(|entry| &entry.path == path && entry.size > 4 * 1024 * 1024)) || item.ocr.as_ref().is_some_and(|path| manifest.files.iter().any(|entry| &entry.path == path && entry.size > MAX_MANIFEST)) { return Err("An article capture or OCR index exceeds its metadata budget.".into()); }
        let meta = &item.metadata;
        if meta.collections.len() > 200 || meta.tags.len() > 200 || meta.bookmarks.len() > 400 || meta.chapters.len() > 4096 || meta.narration.is_none() && meta.chapters.len() > 1000 || meta.tracks.len() > 20 || !meta.playback_position.is_finite() || meta.playback_position < 0.0 || meta.tracks.iter().any(|track| track.path.is_some() || track.segments.len() > 100000) || meta.bookmarks.iter().any(|b| !b.time.is_finite() || b.time < 0.0) || meta.chapters.iter().any(|c| !c.start.is_finite() || !c.end.is_finite() || c.start < 0.0 || c.end < c.start) { return Err("The portable metadata exceeds its limits or includes private file paths.".into()); }
        if meta.narration.as_ref().is_some_and(|value|value.plan_ref.is_some()||!valid_hash(&value.text_hash)||!valid_hash(&value.source_version)||!valid_hash(&value.engine_fingerprint)||value.voice_name.len()>800||value.language.len()>160) || meta.recording.as_ref().is_some_and(|value|value.capture_ref.is_some()||!matches!(value.kind.as_str(),"image"|"recording")||value.source_label.len()>2000||value.reason.as_ref().is_some_and(|reason|reason.len()>8000)) { return Err("Portable production metadata contains private cache references or invalid provenance.".into()); }
        for note in &item.annotations { if !valid_hash(&note.version) || note.quote.is_empty() || note.quote.len() > 8000 || note.prefix.len() > 1000 || note.suffix.len() > 1000 || note.note.len() > 8000 { return Err("Invalid portable article annotation.".into()); } }
        if item.capture.is_none() && !item.annotations.is_empty() { return Err("Article notes require their validated local capture.".into()); }
    }
    for item in &manifest.items { if item.parent_id.as_ref().is_some_and(|parent| parent == &item.id || !ids.contains(parent)) { return Err("Portable parent relationships must reference a different selected item.".into()); } let mut seen = HashSet::new(); let mut current = item.parent_id.as_deref(); while let Some(id) = current { if !seen.insert(id) { return Err("The portable collection contains a parent cycle.".into()); } current = manifest.items.iter().find(|item| item.id == id).and_then(|item| item.parent_id.as_deref()); } }
    Ok(())
}
fn copy_verified(source: &Path, destination: &Path, expected: &FileEntry, receiver: &watch::Receiver<bool>) -> Result<(), String> {
    if destination.exists() { if portable_zip::regular_file(destination)? == expected.size && fingerprint(destination, receiver)? == expected.sha256 { return Ok(()); } return Err("A file appeared at the import destination. It was not overwritten.".into()); }
    if let Some(parent) = destination.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let partial_directory = destination.parent().ok_or("Missing destination directory.")?.join(".nabnab-parts"); fs::create_dir_all(&partial_directory).map_err(|e| e.to_string())?; let partial_directory = library::authorized_path(&partial_directory)?;
    let partial = partial_directory.join(format!("{}.part", hash(destination.file_name().ok_or("Missing destination filename.")?.to_string_lossy().as_bytes())));
    if fs::symlink_metadata(&partial).is_ok() { portable_zip::regular_file(&partial)?; fs::remove_file(&partial).map_err(|e| e.to_string())?; }
    let mut input = File::open(source).map_err(|e| e.to_string())?; let mut output = OpenOptions::new().write(true).create_new(true).open(&partial).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new(); let mut bytes = 0u64; let mut buffer = [0u8; 64 * 1024];
    loop { cancel(receiver)?; let read = input.read(&mut buffer).map_err(|e| e.to_string())?; if read == 0 { break; } bytes += read as u64; if bytes > expected.size { return Err("A portable source changed or exceeded its declared size.".into()); } digest.update(&buffer[..read]); output.write_all(&buffer[..read]).map_err(|e| e.to_string())?; }
    if bytes != expected.size || format!("{:x}", digest.finalize()) != expected.sha256 { return Err("A portable file failed its declared SHA-256. No original was changed.".into()); }
    output.sync_all().map_err(|e| e.to_string())?; drop(output); cancel(receiver)?;
    crate::direct::move_completed(&partial, destination)
}
fn preview(items: &[LibraryItem], receiver: &watch::Receiver<bool>) -> Result<ExportPreview, String> {
    if items.is_empty() || items.len() > MAX_ITEMS { return Err("Select 1–2,000 library files for a portable collection.".into()); }
    let mut entries = Vec::new(); let mut hashes = HashMap::new(); let mut bytes = 0u64; let mut unique_bytes = 0u64;
    for item in items { let path = library::authorized_path(&item.path)?; let size = portable_zip::regular_file(&path)?; bytes = bytes.checked_add(size).filter(|value| *value <= MAX_BYTES).ok_or("The selected collection exceeds 128 GiB.")?; let sha256 = fingerprint(&path, receiver)?; let duplicate_of = hashes.get(&sha256).cloned(); if duplicate_of.is_none() { hashes.insert(sha256.clone(), item.name.clone()); unique_bytes += size; } entries.push(PreviewEntry { id: item.id.clone(), name: item.name.clone(), size, sha256, duplicate_of, article: item.details.web_archive_ref.is_some(), ocr: item.details.ocr_ref.is_some() }); }
    Ok(ExportPreview { entries, bytes, unique_bytes, duplicate_bytes: bytes - unique_bytes, warnings: vec!["Selected file contents are copied unchanged. Review their own contents before sharing; application sessions, cookies, absolute library paths and signed source URLs are excluded from package metadata.".into()] })
}
pub async fn preview_export(_root: &Path, items: &[LibraryItem], _options: &ExportOptions, receiver: watch::Receiver<bool>) -> Result<ExportPreview, String> { let items = items.to_owned(); tokio::task::spawn_blocking(move || preview(&items, &receiver)).await.map_err(|_| "The portable preview worker failed.".to_string())? }
pub async fn execute_export(root: &Path, items: &[LibraryItem], staging: &Path, options: &ExportOptions, receiver: watch::Receiver<bool>, run_id: u64, emit: impl Fn(Progress)) -> Result<PathBuf, String> {
    let root = root.to_owned(); let items = items.to_owned(); let output_directory = staging.to_owned(); let staging = staging.to_owned(); let options = options.clone(); let cancel_copy = receiver.clone();
    let (manifest, files) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let checked = preview(&items, &cancel_copy)?; let payload = staging.join("portable-payload"); fs::create_dir(&payload).map_err(|e| e.to_string())?; fs::create_dir(payload.join("files")).map_err(|e| e.to_string())?;
        let ids: HashMap<_, _> = items.iter().enumerate().map(|(index, item)| (item.id.clone(), format!("item-{index:04}"))).collect(); let mut manifest = Manifest { schema_version: 1, format: "nabnab-portable".into(), created_at: crate::queue::now(), items: vec![], files: vec![] }; let mut content = HashMap::<String, String>::new();
        for (item, checked) in items.iter().zip(checked.entries) {
            cancel(&cancel_copy)?; let id = ids.get(&item.id).ok_or("Missing portable identity.")?.clone(); let name = item.path.file_name().ok_or("Missing selected filename.")?.to_string_lossy().into_owned(); crate::direct::validate_filename(&name)?;
            let relative = if let Some(existing) = content.get(&checked.sha256) { existing.clone() } else { let relative = format!("files/{id}-{name}"); let entry = FileEntry { path: relative.clone(), size: checked.size, sha256: checked.sha256.clone() }; copy_verified(&item.path, &managed_path(&payload, &relative)?, &entry, &cancel_copy)?; manifest.files.push(entry); content.insert(checked.sha256, relative.clone()); relative };
            let mut portable = Item { id: id.clone(), name: text(&item.name, 1000), filename: name, file: relative, parent_id: item.parent_id.as_ref().and_then(|parent| ids.get(parent).cloned()), metadata: metadata(item, &options), capture: None, ocr: None, annotations: vec![] };
            if let Some(reference) = &item.details.web_archive_ref {
                let mut capture = crate::web_archive::load(&root, reference)?; capture.url = source_url(Some(&capture.url)).ok_or("Invalid article source.")?; capture.author = capture.author.as_deref().map(|value| text(value, 2000)); let capture = capture.validate()?;
                let bytes = serde_json::to_vec(&capture).map_err(|e| e.to_string())?; let relative = format!("captures/{id}.json"); let destination = managed_path(&payload, &relative)?; fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?; let mut output = OpenOptions::new().write(true).create_new(true).open(&destination).map_err(|e| e.to_string())?; output.write_all(&bytes).map_err(|e| e.to_string())?;
                let capture_hash = hash(&bytes); manifest.files.push(FileEntry { path: relative.clone(), size: bytes.len() as u64, sha256: capture_hash.clone() }); portable.capture = Some(relative);
                if options.notes { for note in &item.details.article_annotations { if note.version == *reference && anchor_valid(&capture, note) { let mut note = note.clone(); note.version = capture_hash.clone(); note.note = text(&note.note, 8000); portable.annotations.push(note); } } }
            }
            if options.ocr { if let Some(reference) = &item.details.ocr_ref { let index = crate::ocr::read_index(&root, reference)?; let bytes = serde_json::to_vec(&index).map_err(|e| e.to_string())?; let relative = format!("ocr/{id}.json"); let destination = managed_path(&payload, &relative)?; fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?; let mut output = OpenOptions::new().write(true).create_new(true).open(destination).map_err(|e| e.to_string())?; output.write_all(&bytes).map_err(|e| e.to_string())?; manifest.files.push(FileEntry { path: relative.clone(), size: bytes.len() as u64, sha256: hash(&bytes) }); portable.ocr = Some(relative); } }
            manifest.items.push(portable);
        }
        validate_manifest(&manifest)?; write_json(&payload.join("manifest.json"), &manifest)?;
        let mut files: Vec<_> = manifest.files.iter().map(|entry| Ok((entry.path.clone(), managed_path(&payload, &entry.path)?))).collect::<Result<_, String>>()?; files.push(("manifest.json".into(), payload.join("manifest.json"))); Ok((manifest, files))
    }).await.map_err(|_| "The collection export worker failed.".to_string())??;
    cancel(&receiver)?; emit(Progress { request_id: run_id, phase: "processing".into(), percent: Some(60.0), speed: None, downloaded: None, total: None });
    let output = output_directory.join(format!("collection-{}.zip", manifest.created_at)); portable_zip::write_generated(&output, &files, receiver).await?; Ok(output)
}
fn anchor_valid(capture: &crate::web_archive::Capture, note: &crate::web_archive::Annotation) -> bool { capture.blocks.get(note.block).is_some_and(|block| !matches!(block, crate::web_archive::Block::Image {..}) && block.text().match_indices(&note.quote).any(|(start, _)| block.text()[..start].ends_with(&note.prefix) && block.text()[start + note.quote.len()..].starts_with(&note.suffix))) }
fn validate_payloads(manifest: &Manifest, root: &Path, receiver: &watch::Receiver<bool>) -> Result<(), String> {
    for item in &manifest.items {
        cancel(receiver)?;
        if let Some(name) = &item.capture {
            if !matches!(extension(Path::new(&item.filename)).as_str(), "html" | "epub" | "md") { return Err("An article needs HTML, EPUB or Markdown and a validated local capture.".into()); }
            let entry = manifest.files.iter().find(|entry| &entry.path == name).ok_or("Missing article capture.")?; let path = managed_path(root, name)?;
            if fingerprint(&path, receiver)? != entry.sha256 { return Err("The article capture does not match its declared SHA-256.".into()); }
            let capture: crate::web_archive::Capture = read_json(&path, 4 * 1024 * 1024)?; let capture = capture.validate()?;
            for note in &item.annotations { if note.version != entry.sha256 || !anchor_valid(&capture, note) { return Err("An article note does not match its package capture and context.".into()); } }
        }
        if let Some(name) = &item.ocr { let entry = manifest.files.iter().find(|entry| &entry.path == name).ok_or("Missing OCR metadata.")?; let path = managed_path(root, name)?; if fingerprint(&path, receiver)? != entry.sha256 { return Err("The OCR index does not match its declared SHA-256.".into()); } let index: crate::ocr::OcrDocument = read_json(&path, MAX_MANIFEST)?; crate::ocr::validate_index(&index)?; }
    }
    Ok(())
}

fn plan(root: &Path, id: &str) -> Result<Plan, String> { if !valid_hash(id) { return Err("Invalid portable import plan identity.".into()); } let path = directory(root)?.join(format!("{id}.json")); let payload: Plan = read_json(&path, MAX_MANIFEST)?; if hash(&serde_json::to_vec(&payload).map_err(|e| e.to_string())?) != id { return Err("The portable import plan changed. Preview the source again.".into()); } validate_manifest(&payload.manifest)?; if payload.expires_at < crate::queue::now() { return Err("The portable import preview expired. Preview the source again.".into()); } Ok(payload) }
pub fn validate_import_plan(root: &Path, id: &str, target: &Path) -> Result<(), String> { let payload = plan(root, id)?; if library::authorized_path(target)? != payload.target { return Err("The granted import destination changed. Preview the collection again.".into()); } Ok(()) }
pub async fn preview_import(root: &Path, source: &Path, target: &Path, existing: &[LibraryItem], receiver: watch::Receiver<bool>) -> Result<ImportPreview, String> {
    let root = root.to_owned(); let source = library::authorized_path(source)?; let target = library::authorized_path(target)?; let existing = existing.to_owned();
    let zipped = source.is_file(); let inspected = if zipped { Some(package::inspect(&root, &source, receiver.clone()).await?) } else { None };
    let metadata_directory = if let Some(inspected) = &inspected {
        let manifest: Manifest = serde_json::from_str(&inspected.manifest).map_err(|_| "The ZIP manifest has an unsupported schema.")?; validate_manifest(&manifest)?;
        let names: HashSet<_> = manifest.items.iter().flat_map(|item| item.capture.iter().chain(item.ocr.iter())).cloned().collect();
        if names.is_empty() { None } else { let directory = PreviewDirectory::new(&root)?; package::extract(&root, &source, &directory.path, &names.into_iter().collect::<Vec<_>>(), receiver.clone()).await?; Some(directory) }
    } else { None };
    let metadata_root = metadata_directory.as_ref().map(|directory| directory.path.clone()).unwrap_or_else(|| source.clone());
    tokio::task::spawn_blocking(move || {
        let (manifest, source_hash) = if let Some(inspected) = inspected { let manifest: Manifest = serde_json::from_str(&inspected.manifest).map_err(|_| "The ZIP manifest has an unsupported schema.")?; validate_manifest(&manifest)?; if inspected.entries.len() != manifest.files.len() + 1 { return Err("The ZIP contains files outside its portable manifest.".into()); } for entry in &manifest.files { if !inspected.entries.iter().any(|actual| actual.name == entry.path && actual.size == entry.size && actual.sha256 == entry.sha256) { return Err("A ZIP file does not match the portable manifest hash or size.".into()); } } (manifest, Some(fingerprint(&source, &receiver)?)) } else { let manifest: Manifest = read_json(&managed_path(&source, "manifest.json")?, MAX_MANIFEST)?; validate_manifest(&manifest)?; for entry in &manifest.files { let path = managed_path(&source, &entry.path)?; if portable_zip::regular_file(&path)? != entry.size || fingerprint(&path, &receiver)? != entry.sha256 { return Err("A selected portable folder file does not match its manifest.".into()); } } (manifest, None) };
        validate_payloads(&manifest, &metadata_root, &receiver)?;
        let mut known = HashMap::new(); let wanted: HashSet<_> = manifest.files.iter().map(|entry| entry.size).collect();
        for item in existing.iter().filter(|item| item.parent_id.is_none() && item.details.web_archive_ref.is_none() && item.details.ocr_ref.is_none() && item.details.narration.is_none() && item.details.recording.is_none() && wanted.contains(&item.size)).take(10000) { if let Ok(actual) = fingerprint(&item.path, &receiver) { known.entry(format!("{actual}|{}", extension(&item.path))).or_insert_with(|| item.clone()); } cancel(&receiver)?; }
        let mut entries = Vec::new(); let mut bytes = 0u64; let mut unique_bytes = 0u64; let mut seen = HashSet::new(); let mut reusable = Vec::new();
        for item in &manifest.items { let file = manifest.files.iter().find(|entry| entry.path == item.file).ok_or("Missing portable file.")?; let duplicate = if ordinary_item(item) { known.get(&format!("{}|{}", file.sha256, extension(Path::new(&item.filename)))) } else { None }; bytes += file.size; if !ordinary_item(item) || seen.insert(format!("{}|{}", file.sha256, extension(Path::new(&item.filename)))) && duplicate.is_none() { unique_bytes += file.size; } if let Some(existing) = duplicate { if !reusable.iter().any(|item: &LibraryItem| item.id == existing.id) { reusable.push(existing.clone()); } } entries.push(PreviewEntry { id: item.id.clone(), name: item.name.clone(), size: file.size, sha256: file.sha256.clone(), duplicate_of: duplicate.map(|item| item.name.clone()), article: item.capture.is_some(), ocr: item.ocr.is_some() }); }
        let created_at = crate::queue::now(); let expires_at = created_at + 24 * 60 * 60 * 1000; let payload = Plan { nonce: nonce(), created_at, expires_at, source, zipped, source_hash, target: target.clone(), manifest, existing: reusable }; let id = hash(&serde_json::to_vec(&payload).map_err(|e| e.to_string())?); write_json(&directory(&root)?.join(format!("{id}.json")), &payload)?;
        Ok(ImportPreview { plan_id: id, expires_at, entries, bytes, unique_bytes, duplicate_bytes: bytes - unique_bytes, target, warnings: vec!["Only declared, hash-verified files are imported. Exact ordinary files may reuse an existing library record; article and OCR versions remain separate. Article payloads are regenerated from their validated local capture. Executable files are never run.".into()] })
    }).await.map_err(|_| "The portable import preview worker failed.".to_string())?
}

pub async fn execute_import(root: &Path, id: &str, target: &Path, receiver: watch::Receiver<bool>, run_id: u64, emit: impl Fn(Progress)) -> Result<PreparedImport, String> {
    validate_import_plan(root, id, target)?; let payload = plan(root, id)?; let parent = directory(root)?; let work = parent.join(format!("work-{id}"));
    if !work.exists() { fs::create_dir(&work).map_err(|e| e.to_string())?; } let work = library::authorized_path(&work)?;
    let source_root = if payload.zipped { if fingerprint(&payload.source, &receiver)? != payload.source_hash.as_deref().ok_or("The ZIP plan has no source identity.")? { return Err("The selected ZIP changed after preview. Preview it again.".into()); } let extract = work.join("extracted"); if !extract.exists() { fs::create_dir(&extract).map_err(|e| e.to_string())?; } package::extract(root, &payload.source, &extract, &payload.manifest.files.iter().map(|entry| entry.path.clone()).collect::<Vec<_>>(), receiver.clone()).await?; extract } else { payload.source.clone() };
    let destination = payload.target.join(format!("nabnab-import-{}", &id[..16])); if !destination.exists() { fs::create_dir(&destination).map_err(|e| e.to_string())?; write_json(&destination.join(".nabnab-import-owner.json"), &id)?; } else if read_json::<String>(&managed_path(&destination, ".nabnab-import-owner.json")?, 1024)? != id { return Err("The import destination belongs to another operation. Existing files were preserved.".into()); }
    let destination = library::authorized_path(&destination)?;
    let mut imported = Vec::<LibraryItem>::new(); let mut mapping = HashMap::new(); let mut reused = HashMap::<String, usize>::new();
    for (index, portable) in payload.manifest.items.iter().enumerate() {
        cancel(&receiver)?; let entry = payload.manifest.files.iter().find(|entry| entry.path == portable.file).ok_or("Missing portable main file.")?; let source = managed_path(&source_root, &entry.path)?;
        if portable_zip::regular_file(&source)? != entry.size || fingerprint(&source, &receiver)? != entry.sha256 { return Err("A portable input changed after preview.".into()); }
        if ordinary_item(portable) { if let Some(existing) = payload.existing.iter().find(|item| extension(&item.path) == extension(Path::new(&portable.filename)) && item.size == entry.size && fingerprint(&item.path, &receiver).is_ok_and(|hash| hash == entry.sha256)) { let mut item = existing.clone(); merge_labels(&mut item, &portable.metadata); merge_bookmarks(&mut item, &portable.metadata.bookmarks); mapping.insert(portable.id.clone(), item.id.clone()); imported.push(item); continue; } }
        let reusable = portable.capture.is_none() && portable.ocr.is_none() && portable.parent_id.is_none();
        let reuse_key = format!("{}|{}", entry.sha256, extension(Path::new(&portable.filename)));
        if reusable { if let Some(index) = reused.get(&reuse_key).copied() { let mut item = imported[index].clone(); merge_labels(&mut item, &portable.metadata); merge_bookmarks(&mut item, &portable.metadata.bookmarks); imported[index] = item.clone(); mapping.insert(portable.id.clone(), item.id.clone()); imported.push(item); continue; } }
        let output = managed_path(&destination, &format!("files/{}-{}", portable.id, portable.filename))?;
        let mut article_ref = None; let mut annotations = vec![];
        if let Some(capture_path) = &portable.capture {
            let capture_entry = payload.manifest.files.iter().find(|entry| &entry.path == capture_path).ok_or("Missing portable capture.")?; let path = managed_path(&source_root, capture_path)?; if fingerprint(&path, &receiver)? != capture_entry.sha256 { return Err("The portable article capture changed.".into()); } let capture: crate::web_archive::Capture = read_json(&path, 4 * 1024 * 1024)?; let capture = capture.validate()?;
            let extension = output.extension().and_then(|ext| ext.to_str()).unwrap_or_default(); let format = match extension.to_ascii_lowercase().as_str() { "html" => crate::web_archive::Format::Html, "epub" => crate::web_archive::Format::Epub, "md" => crate::web_archive::Format::Md, _ => return Err("A portable article must use HTML, EPUB or Markdown and a validated capture.".into()) };
            let reference = crate::web_archive::save(root, capture.clone())?; let regenerated = work.join(format!("article-{}", portable.id)); if !regenerated.exists() { fs::create_dir(&regenerated).map_err(|e| e.to_string())?; } let expected_name = regenerated.join(format!("article-{}.{}", &reference[..12], format.extension())); if expected_name.exists() { portable_zip::regular_file(&expected_name)?; fs::remove_file(&expected_name).map_err(|e| e.to_string())?; } let safe = crate::web_archive::execute(root, &reference, format, &regenerated, receiver.clone(), run_id, |_| {}).await?; let generated = FileEntry { path: String::new(), size: portable_zip::regular_file(&safe)?, sha256: fingerprint(&safe, &receiver)? }; copy_verified(&safe, &output, &generated, &receiver)?;
            for note in &portable.annotations { if note.version != capture_entry.sha256 || !anchor_valid(&capture, note) { return Err("A portable annotation does not match its immutable capture. It was not reassigned.".into()); } let mut note = note.clone(); note.version = reference.clone(); note.id = hash(format!("{id}:{}:{}:{reference}", portable.id, note.id).as_bytes()); note.note = text(&note.note, 8000); annotations.push(note); } article_ref = Some(reference);
        } else { copy_verified(&source, &output, entry, &receiver)?; }
        let mut item = library::index(&output, None, source_url(portable.metadata.source_url.as_deref()), None).or_else(|_| library::index_file(&output))?; item.name = text(&portable.name, 1000); item.content_hash = Some(fingerprint(&output, &receiver)?); apply_metadata(&mut item, &portable.metadata); item.details.web_archive_ref = article_ref; item.details.article_annotations = annotations; if item.details.web_archive_ref.is_some() { item.kind = "article".into(); }
        if let Some(ocr_path) = &portable.ocr { let entry = payload.manifest.files.iter().find(|entry| &entry.path == ocr_path).ok_or("Missing OCR metadata.")?; let path = managed_path(&source_root, ocr_path)?; if fingerprint(&path, &receiver)? != entry.sha256 { return Err("The portable OCR metadata changed.".into()); } let index: crate::ocr::OcrDocument = read_json(&path, MAX_MANIFEST)?; crate::ocr::validate_index(&index)?; item.details.ocr_ref = Some(crate::ocr::write_index(root, &format!("portable-{}-{}", &id[..20], portable.id), &index)?); }
        if reusable { reused.insert(reuse_key, imported.len()); }
        mapping.insert(portable.id.clone(), item.id.clone()); imported.push(item); emit(Progress { request_id: run_id, phase: "processing".into(), percent: Some((index + 1) as f64 * 100.0 / payload.manifest.items.len() as f64), speed: None, downloaded: None, total: None });
    }
    for (item, portable) in imported.iter_mut().zip(&payload.manifest.items) { if let Some(parent) = &portable.parent_id { item.parent_id = mapping.get(parent).cloned(); } }
    let report_path = destination.join("import-report.json"); let prepared = PreparedImport { plan_id: id.into(), items: imported, report_path: report_path.clone() }; let report = serde_json::json!({"schemaVersion":1,"notice":"Selected collection import, not a system backup.","items":payload.manifest.items.iter().map(|item| serde_json::json!({"id":item.id,"name":text(&item.name,1000)})).collect::<Vec<_>>()}); if !report_path.exists() { write_json(&report_path, &report)?; } else if read_json::<serde_json::Value>(&report_path, MAX_MANIFEST)? != report { return Err("A different import report appeared at the destination. It was preserved.".into()); } Ok(prepared)
}
fn ordinary_item(item:&Item)->bool{item.capture.is_none()&&item.ocr.is_none()&&item.parent_id.is_none()&&item.metadata.narration.is_none()&&item.metadata.recording.is_none()}
fn extension(path: &Path) -> String { path.extension().and_then(|value| value.to_str()).unwrap_or_default().to_ascii_lowercase() }
fn merge_bookmarks(item: &mut LibraryItem, bookmarks: &[crate::library_features::Bookmark]) { for bookmark in bookmarks { if !item.details.bookmarks.iter().any(|existing| existing.time == bookmark.time && existing.name == bookmark.name) { item.details.bookmarks.push(crate::library_features::Bookmark { id: bookmark.id.clone(), name: text(&bookmark.name, 2000), time: bookmark.time }); } } }
fn merge_labels(item: &mut LibraryItem, metadata: &Metadata) { for value in &metadata.collections { let value = text(value, 120); if !item.collections.contains(&value) { item.collections.push(value); } } for value in &metadata.tags { let value = text(value, 120); if !item.tags.contains(&value) { item.tags.push(value); } } }
fn apply_metadata(item: &mut LibraryItem, metadata: &Metadata) {
    item.details.narration=narration_metadata(&metadata.narration);item.details.recording=recording_metadata(&metadata.recording);
    merge_labels(item, metadata); item.details.creator = metadata.creator.as_deref().map(|v| text(v, 2000)); item.details.platform = metadata.platform.as_deref().map(|v| text(v, 200)); item.details.source_url = source_url(metadata.source_url.as_deref()); item.source = item.details.source_url.clone(); item.details.license = metadata.license.as_deref().map(|v| text(v, 2000)); item.details.attribution = metadata.attribution.as_deref().map(|v| text(v, 8000)); item.details.favorite = metadata.favorite; item.details.playback_position = metadata.playback_position; merge_bookmarks(item, &metadata.bookmarks); item.details.chapters = metadata.chapters.clone(); item.details.tracks = metadata.tracks.clone(); for track in &mut item.details.tracks { track.path = None; }
}
pub fn commit_import(repo: &mut Repository, prepared: &PreparedImport) -> Result<usize, String> {
    let payload = plan(repo.root(), &prepared.plan_id)?; let expected = payload.target.join(format!("nabnab-import-{}", &prepared.plan_id[..16]));
    for item in &prepared.items {
        let path = library::authorized_path(&item.path)?; if !path.starts_with(&expected) && !repo.data.library.iter().any(|existing| existing.id == item.id && existing.path == path) { return Err("An imported file left its granted destination.".into()); }
        if portable_zip::regular_file(&path)? != item.size { return Err("An imported file changed before indexing.".into()); }
    }
    let previous = repo.data.library.clone(); let collections = repo.data.collections.clone(); let mut added = 0;
    for item in &prepared.items {
        if let Some(existing) = repo.data.library.iter_mut().find(|existing| existing.id == item.id) { for collection in &item.collections { if !existing.collections.contains(collection) { existing.collections.push(collection.clone()); } } for tag in &item.tags { if !existing.tags.contains(tag) { existing.tags.push(tag.clone()); } } merge_bookmarks(existing, &item.details.bookmarks); } else { repo.data.library.push(item.clone()); added += 1; }
    }
    for item in &repo.data.library { for collection in &item.collections { if !repo.data.collections.contains(collection) { repo.data.collections.push(collection.clone()); } } }
    if let Err(error) = repo.save() { repo.data.library = previous; repo.data.collections = collections; return Err(error); } Ok(added)
}

#[cfg(test)] mod tests {
    use super::*;
    struct Fixture { root: PathBuf, config: PathBuf, input: PathBuf, target: PathBuf, staging: PathBuf }
    impl Fixture {
        fn new() -> Self { let root = std::env::temp_dir().join(format!("nabnab-portable-test-{}", nonce())); fs::create_dir(&root).unwrap(); let config = root.join("config"); let input = root.join("input"); let target = root.join("target"); let staging = root.join("staging"); for directory in [&config, &input, &target, &staging] { fs::create_dir(directory).unwrap(); } Self { root, config, input, target, staging } }
        fn item(&self, name: &str, bytes: &[u8]) -> LibraryItem { let path = self.input.join(name); fs::write(&path, bytes).unwrap(); library::index(&path, None, None, None).or_else(|_| library::index_file(&path)).unwrap() }
    }
    impl Drop for Fixture { fn drop(&mut self) { if self.root.starts_with(std::env::temp_dir()) && self.root.file_name().unwrap().to_string_lossy().starts_with("nabnab-portable-test-") { let _ = fs::remove_dir_all(&self.root); } } }
    fn options() -> ExportOptions { ExportOptions { metadata: true, notes: true, ocr: false } }
    fn receiver() -> (watch::Sender<bool>, watch::Receiver<bool>) { watch::channel(false) }
    #[tokio::test]
    async fn production_metadata_round_trips_without_private_cache_references_or_reassigning_versions() {
        let fixture=Fixture::new();
        let mut narration=fixture.item("narration.wav",b"synthetic package voice bytes");
        narration.details.narration=Some(crate::library_features::NarrationReference {plan_ref:Some("f".repeat(64)),text_hash:"a".repeat(64),source_version:"b".repeat(64),voice_name:"Offline fixture voice".into(),language:"es-ES".into(),engine_fingerprint:"c".repeat(64)});
        let mut recording=fixture.item("capture.mkv",b"synthetic package capture bytes");
        recording.details.recording=Some(crate::library_features::RecordingReference {capture_ref:Some("c-private-session".into()),source_label:"C:\\Users\\Private\\secret.txt".into(),kind:"recording".into(),duration_ms:1200,interrupted:true,reason:Some("token=private".into())});
        let items=vec![narration,recording];
        let (_sender,cancel)=receiver();
        let archive=execute_export(&fixture.config,&items,&fixture.staging,&options(),cancel.clone(),501,|_|{}).await.unwrap();
        let preview=preview_import(&fixture.config,&archive,&fixture.target,&items,cancel.clone()).await.unwrap();
        assert_eq!(preview.duplicate_bytes,0,"Production versions retain separate provenance.");
        let prepared=execute_import(&fixture.config,&preview.plan_id,&fixture.target,cancel,502,|_|{}).await.unwrap();
        let voice=prepared.items.iter().find_map(|item|item.details.narration.as_ref()).unwrap();
        assert!(voice.plan_ref.is_none());
        assert_eq!(voice.voice_name,"Offline fixture voice");
        assert_eq!(voice.text_hash,"a".repeat(64));
        let capture=prepared.items.iter().find_map(|item|item.details.recording.as_ref()).unwrap();
        assert!(capture.capture_ref.is_none());
        assert_eq!(capture.duration_ms,1200);
        assert!(capture.interrupted);
        assert_eq!(capture.source_label,"[sensitive metadata omitted]");
        assert_eq!(capture.reason.as_deref(),Some("[sensitive metadata omitted]"));
    }
    #[tokio::test]
    async fn preview_and_package_deduplicate_bytes_and_remove_private_metadata() {
        let fixture = Fixture::new(); let mut one = fixture.item("one.txt", b"123456789"); let two = fixture.item("two.bin", b"123456789");
        one.source = Some("https://example.org/origin?token=secret&X-Amz-Signature=private#fragment".into()); one.details.creator = Some("C:\\private\\person".into()); one.collections = vec!["Selected".into()];
        one.details.tracks.push(crate::captions::LocalTrack { id: "C:\\private\\track".into(), language: "en".into(), origin: crate::captions::TrackOrigin::Imported, path: Some(fixture.input.join("private.srt")), segments: vec![], formatting_lost: false });
        let (_sender, cancel) = receiver(); let preview = preview_export(&fixture.config, &[one.clone(), two.clone()], &options(), cancel.clone()).await.unwrap();
        assert_eq!(preview.bytes, 18); assert_eq!(preview.unique_bytes, 9); assert_eq!(preview.duplicate_bytes, 9); assert_eq!(preview.entries[1].duplicate_of.as_deref(), Some("one.txt"));
        let archive = execute_export(&fixture.config, &[one, two], &fixture.staging, &options(), cancel, 1, |_| {}).await.unwrap(); assert!(archive.is_file());
        let raw = fs::read_to_string(fixture.staging.join("portable-payload/manifest.json")).unwrap(); assert!(!raw.contains("secret")); assert!(!raw.contains("private\\\\")); assert!(!raw.contains("Signature")); assert!(!raw.contains(&fixture.root.to_string_lossy().to_string()));
        let manifest: Manifest = serde_json::from_str(&raw).unwrap(); assert_eq!(manifest.files.len(), 1); assert_eq!(manifest.items[0].filename, "one.txt"); assert_eq!(manifest.items[1].filename, "two.bin"); assert_eq!(manifest.items[0].metadata.source_url.as_deref(), Some("https://example.org/origin")); assert!(manifest.items[0].metadata.tracks[0].path.is_none()); assert_eq!(fs::read(fixture.input.join("one.txt")).unwrap(), b"123456789");
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn native_zip_import_restores_parent_links_and_commits_idempotently() {
        let fixture = Fixture::new(); let original = fixture.item("original.txt", b"original"); let mut child = fixture.item("child.bin", b"child"); child.parent_id = Some(original.id.clone()); child.collections = vec!["Portable".into()]; let (_sender, cancel) = receiver();
        let archive = execute_export(&fixture.config, &[original, child], &fixture.staging, &options(), cancel.clone(), 1, |_| {}).await.unwrap();
        let preview = preview_import(&fixture.config, &archive, &fixture.target, &[], cancel.clone()).await.unwrap(); assert_eq!(preview.entries.len(), 2); validate_import_plan(&fixture.config, &preview.plan_id, &fixture.target).unwrap(); assert!(validate_import_plan(&fixture.config, &preview.plan_id, &fixture.input).is_err());
        let prepared = execute_import(&fixture.config, &preview.plan_id, &fixture.target, cancel, 2, |_| {}).await.unwrap(); assert_eq!(fs::read(&prepared.items[0].path).unwrap(), b"original"); assert_eq!(fs::read(&prepared.items[1].path).unwrap(), b"child"); assert_eq!(prepared.items[1].parent_id.as_deref(), Some(prepared.items[0].id.as_str()));
        let mut repo = Repository::open(&fixture.config, &fixture.target).unwrap(); assert_eq!(commit_import(&mut repo, &prepared).unwrap(), 2); assert_eq!(commit_import(&mut repo, &prepared).unwrap(), 0); assert_eq!(repo.data.library.len(), 2); assert_eq!(repo.data.collections, vec!["Portable".to_owned()]);
    }
    #[tokio::test]
    async fn import_regenerates_untrusted_article_and_restores_only_matching_notes() {
        let fixture = Fixture::new(); let capture = crate::web_archive::Capture { schema_version: 1, mode: crate::web_archive::Mode::Reading, title: "Kept article".into(), author: None, url: "https://example.org/article?token=private".into(), published_at: None, archived_at: "2026-11-06T00:00:00Z".into(), blocks: vec![crate::web_archive::Block::Paragraph { text: "A kept phrase in the immutable local version.".into() }], resources: vec![], warnings: vec![], selection_context: None };
        let reference = crate::web_archive::save(&fixture.config, capture).unwrap(); let (_sender, cancel) = receiver(); let original = crate::web_archive::execute(&fixture.config, &reference, crate::web_archive::Format::Html, &fixture.input, cancel.clone(), 1, |_| {}).await.unwrap(); let original_bytes = fs::read(&original).unwrap(); let mut item = library::index_file(&original).unwrap(); item.kind = "article".into(); item.details.web_archive_ref = Some(reference.clone()); item.details.article_annotations.push(crate::web_archive::Annotation { id: "a".repeat(64), version: reference, block: 0, quote: "kept phrase".into(), prefix: "A ".into(), suffix: " in the immutable local version.".into(), note: "A note".into(), created_at: 1 });
        execute_export(&fixture.config, &[item], &fixture.staging, &options(), cancel.clone(), 1, |_| {}).await.unwrap(); let folder = fixture.staging.join("portable-payload"); let mut manifest: Manifest = read_json(&folder.join("manifest.json"), MAX_MANIFEST).unwrap(); let file = manifest.files.iter_mut().find(|entry| entry.path == manifest.items[0].file).unwrap(); let malicious = b"<script src='https://invalid.example/run.js'>unsafe()</script>"; fs::write(folder.join(&file.path), malicious).unwrap(); file.size = malicious.len() as u64; file.sha256 = hash(malicious); fs::write(folder.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap();
        let preview = preview_import(&fixture.config, &folder, &fixture.target, &[], cancel.clone()).await.unwrap(); let prepared = execute_import(&fixture.config, &preview.plan_id, &fixture.target, cancel, 2, |_| {}).await.unwrap(); let restored = &prepared.items[0]; assert_eq!(restored.kind, "article"); assert_eq!(restored.details.article_annotations.len(), 1); assert_eq!(restored.details.article_annotations[0].version, restored.details.web_archive_ref.clone().unwrap()); let safe = fs::read_to_string(&restored.path).unwrap(); assert!(safe.contains("A kept phrase in the immutable local version.")); assert!(safe.contains("script-src 'none'")); assert!(!safe.contains("unsafe()")); assert_eq!(fs::read(original).unwrap(), original_bytes);
    }
    #[tokio::test]
    async fn changed_folder_input_and_plan_tampering_are_rejected() {
        let fixture = Fixture::new(); let item = fixture.item("sample.txt", b"selected"); let (_sender, cancel) = receiver(); execute_export(&fixture.config, &[item], &fixture.staging, &options(), cancel.clone(), 1, |_| {}).await.unwrap(); let folder = fixture.staging.join("portable-payload"); let preview = preview_import(&fixture.config, &folder, &fixture.target, &[], cancel.clone()).await.unwrap(); let manifest: Manifest = read_json(&folder.join("manifest.json"), MAX_MANIFEST).unwrap(); fs::write(folder.join(&manifest.items[0].file), b"modified").unwrap(); assert!(execute_import(&fixture.config, &preview.plan_id, &fixture.target, cancel, 2, |_| {}).await.is_err());
        let path = directory(&fixture.config).unwrap().join(format!("{}.json", preview.plan_id)); let mut payload: Plan = read_json(&path, MAX_MANIFEST).unwrap(); payload.target = fixture.input.clone(); fs::write(&path, serde_json::to_vec(&payload).unwrap()).unwrap(); assert!(validate_import_plan(&fixture.config, &preview.plan_id, &fixture.input).is_err());
    }
    #[tokio::test]
    async fn exact_existing_file_is_reused_without_overwrite_and_bookmarks_are_merged() {
        let fixture = Fixture::new(); let mut incoming = fixture.item("incoming.txt", b"identical content"); incoming.collections = vec!["Imported".into()]; incoming.details.bookmarks.push(crate::library_features::Bookmark { id: "portable-mark".into(), name: "Kept note".into(), time: 0.0 }); let mut existing = fixture.item("existing.txt", b"identical content"); existing.content_hash = Some("0".repeat(64)); let (_sender, cancel) = receiver();
        execute_export(&fixture.config, &[incoming], &fixture.staging, &options(), cancel.clone(), 1, |_| {}).await.unwrap(); let folder = fixture.staging.join("portable-payload"); let preview = preview_import(&fixture.config, &folder, &fixture.target, &[existing.clone()], cancel.clone()).await.unwrap(); assert_eq!(preview.entries[0].duplicate_of.as_deref(), Some("existing.txt")); assert_eq!(preview.unique_bytes, 0);
        let prepared = execute_import(&fixture.config, &preview.plan_id, &fixture.target, cancel, 2, |_| {}).await.unwrap(); assert_eq!(prepared.items[0].path, existing.path); assert_eq!(prepared.items[0].details.bookmarks[0].name, "Kept note"); let mut repo = Repository::open(&fixture.config, &fixture.target).unwrap(); repo.data.library.push(existing.clone()); repo.save().unwrap(); assert_eq!(commit_import(&mut repo, &prepared).unwrap(), 0); assert_eq!(repo.data.library.len(), 1); assert_eq!(repo.data.library[0].collections, vec!["Imported".to_owned()]); assert_eq!(repo.data.library[0].details.bookmarks.len(), 1); assert_eq!(fs::read(existing.path).unwrap(), b"identical content");
    }
    #[tokio::test]
    async fn selected_ocr_index_is_validated_and_restored_without_private_paths() {
        let fixture = Fixture::new(); let mut item = fixture.item("scan.pdf", b"%PDF-1.4 selected original"); let index = crate::ocr::OcrDocument { schema_version: 1, source_fingerprint: hash(b"%PDF-1.4 selected original"), languages: vec!["eng".into()], pages: vec![crate::ocr::OcrPage { page: 1, width: 100, height: 100, text: "A recognized phrase".into(), origin: crate::ocr::Origin::Ocr, words: vec![], region: None, corrected: false }] }; item.details.ocr_ref = Some(crate::ocr::write_index(&fixture.config, "portable-fixture", &index).unwrap()); let (_sender, cancel) = receiver(); let options = ExportOptions { metadata: true, notes: true, ocr: true };
        execute_export(&fixture.config, &[item], &fixture.staging, &options, cancel.clone(), 1, |_| {}).await.unwrap(); let folder = fixture.staging.join("portable-payload"); let preview = preview_import(&fixture.config, &folder, &fixture.target, &[], cancel.clone()).await.unwrap(); assert!(preview.entries[0].ocr); let prepared = execute_import(&fixture.config, &preview.plan_id, &fixture.target, cancel, 2, |_| {}).await.unwrap(); let restored = crate::ocr::read_index(&fixture.config, prepared.items[0].details.ocr_ref.as_deref().unwrap()).unwrap(); assert_eq!(restored.pages[0].text, "A recognized phrase"); assert_eq!(restored.source_fingerprint, index.source_fingerprint); assert_eq!(fs::read(fixture.input.join("scan.pdf")).unwrap(), b"%PDF-1.4 selected original");
    }
    #[tokio::test]
    async fn missing_later_file_does_not_commit_an_earlier_prefix() {
        let fixture = Fixture::new(); let mut first = fixture.item("first.txt", b"first"); first.collections = vec!["Should not appear".into()]; let second = fixture.item("second.txt", b"second"); let (_sender, cancel) = receiver(); execute_export(&fixture.config, &[first, second], &fixture.staging, &options(), cancel.clone(), 1, |_| {}).await.unwrap(); let preview = preview_import(&fixture.config, &fixture.staging.join("portable-payload"), &fixture.target, &[], cancel.clone()).await.unwrap(); let prepared = execute_import(&fixture.config, &preview.plan_id, &fixture.target, cancel, 2, |_| {}).await.unwrap(); let mut repo = Repository::open(&fixture.config, &fixture.target).unwrap(); let before = serde_json::to_vec(&repo.data.library).unwrap(); let collections = repo.data.collections.clone(); fs::remove_file(&prepared.items[1].path).unwrap(); assert!(commit_import(&mut repo, &prepared).is_err()); assert_eq!(serde_json::to_vec(&repo.data.library).unwrap(), before); assert_eq!(repo.data.collections, collections);
    }
    #[tokio::test]
    async fn preview_rejects_invalid_article_and_ocr_before_destination_writes() {
        let fixture = Fixture::new(); let item = fixture.item("selected.txt", b"selected bytes"); let (_sender, cancel) = receiver(); execute_export(&fixture.config, &[item], &fixture.staging, &options(), cancel.clone(), 1, |_| {}).await.unwrap(); let folder = fixture.staging.join("portable-payload"); let mut manifest: Manifest = read_json(&folder.join("manifest.json"), MAX_MANIFEST).unwrap(); manifest.items[0].filename = "article.html".into(); manifest.items[0].capture = Some("captures/article.json".into());
        let mut capture = crate::web_archive::Capture { schema_version: 1, mode: crate::web_archive::Mode::Reading, title: "Untrusted capture".into(), author: None, url: "file:///C:/private/data".into(), archived_at: "2026-11-06T00:00:00Z".into(), published_at: None, blocks: vec![crate::web_archive::Block::Paragraph { text: "Kept text".into() }], resources: vec![], warnings: vec![], selection_context: None };
        fs::create_dir(folder.join("captures")).unwrap(); let bytes = serde_json::to_vec(&capture).unwrap(); fs::write(folder.join("captures/article.json"), &bytes).unwrap(); manifest.files.push(FileEntry { path: "captures/article.json".into(), size: bytes.len() as u64, sha256: hash(&bytes) }); fs::write(folder.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap(); assert!(preview_import(&fixture.config, &folder, &fixture.target, &[], cancel.clone()).await.is_err()); assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 0);
        capture.url = "https://example.org/article".into(); let bytes = serde_json::to_vec(&capture).unwrap(); fs::write(folder.join("captures/article.json"), &bytes).unwrap(); manifest.files[1].size = bytes.len() as u64; manifest.files[1].sha256 = hash(&bytes); let bad_ocr = crate::ocr::OcrDocument { schema_version: 1, source_fingerprint: "a".repeat(64), languages: vec!["eng".into()], pages: vec![crate::ocr::OcrPage { page: 1, width: 10, height: 10, text: "Invalid coordinates".into(), origin: crate::ocr::Origin::Ocr, words: vec![crate::ocr::Word { text: "outside".into(), x: 100, y: 0, width: 5, height: 5, confidence: 99.0 }], region: None, corrected: false }] }; let bytes = serde_json::to_vec(&bad_ocr).unwrap(); fs::create_dir(folder.join("ocr")).unwrap(); fs::write(folder.join("ocr/index.json"), &bytes).unwrap(); manifest.items[0].ocr = Some("ocr/index.json".into()); manifest.files.push(FileEntry { path: "ocr/index.json".into(), size: bytes.len() as u64, sha256: hash(&bytes) }); fs::write(folder.join("manifest.json"), serde_json::to_vec(&manifest).unwrap()).unwrap(); assert!(preview_import(&fixture.config, &folder, &fixture.target, &[], cancel).await.is_err()); assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 0);
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn native_zip_reader_bounds_actual_expansion_not_only_declared_lengths() {
        let fixture = Fixture::new(); let item = fixture.item("oversized.bin", &vec![0; 1024 * 1024]); let (_sender, cancel) = receiver(); let archive = execute_export(&fixture.config, &[item], &fixture.staging, &options(), cancel.clone(), 1, |_| {}).await.unwrap(); let mut bytes = fs::read(&archive).unwrap(); bytes[22..26].copy_from_slice(&1u32.to_le_bytes()); let central = bytes.windows(4).position(|bytes| bytes == b"PK\x01\x02").unwrap(); bytes[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes()); fs::write(&archive, bytes).unwrap(); assert!(package::inspect(&fixture.config, &archive, cancel).await.is_err()); assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 0);
    }
    #[test]
    fn zip_driver_source_contains_no_binary_control_characters() {
        let script = include_bytes!("portable_collections/zip.ps1");
        assert!(script.iter().all(|byte| *byte >= 32 || matches!(*byte, b'\n' | b'\r' | b'\t')));
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn native_zip_manifest_is_bounded_before_decoding_when_declared_length_lies() {
        let fixture = Fixture::new(); let mut body = vec![b' '; 16 * 1024 * 1024 + 65536]; body[..2].copy_from_slice(b"{}");
        let manifest = fixture.item("large-manifest.json", &body); let archive = fixture.staging.join("lying-manifest.zip"); let (_sender, cancel) = receiver(); portable_zip::write_generated(&archive, &[("manifest.json".into(), manifest.path)], cancel.clone()).await.unwrap();
        let mut bytes = fs::read(&archive).unwrap(); bytes[22..26].copy_from_slice(&1u32.to_le_bytes()); let central = bytes.windows(4).position(|bytes| bytes == b"PK\x01\x02").unwrap(); bytes[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes()); fs::write(&archive, bytes).unwrap();
        let failure = package::inspect(&fixture.config, &archive, cancel).await.err().expect("A manifest may not exceed the size declared by its entry.");
        assert!(failure.contains("ZIP manifest expanded beyond"), "{failure}"); assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 0);
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn native_zip_rejects_backslash_member_names() {
        let fixture = Fixture::new(); let manifest = fixture.item("manifest.json", b"{}"); let member = fixture.item("member.txt", b"data"); let archive = fixture.staging.join("backslash.zip"); let (_sender, cancel) = receiver(); portable_zip::write_generated(&archive, &[("manifest.json".into(), manifest.path), ("safe/item.txt".into(), member.path)], cancel.clone()).await.unwrap();
        let mut bytes = fs::read(&archive).unwrap(); let positions: Vec<_> = bytes.windows(b"safe/item.txt".len()).enumerate().filter_map(|(index, part)| (part == b"safe/item.txt").then_some(index)).collect(); assert_eq!(positions.len(), 2); for index in positions { bytes[index + 4] = b'\\'; } fs::write(&archive, bytes).unwrap();
        let failure = package::inspect(&fixture.config, &archive, cancel).await.err().expect("Backslash member paths must be rejected by the native reader."); assert!(failure.contains("unsafe or colliding paths"), "{failure}");
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn native_zip_extraction_partials_do_not_share_member_namespace() {
        let fixture = Fixture::new(); let first = fixture.item("first.bin", b"first member"); let second = fixture.item("second.bin", b"second member"); let archive = fixture.staging.join("partial-name-collision.zip"); let (_sender, cancel) = receiver(); portable_zip::write_generated(&archive, &[("foo.nabnab-extract-part".into(), first.path), ("foo".into(), second.path)], cancel.clone()).await.unwrap();
        package::extract(&fixture.config, &archive, &fixture.target, &["foo.nabnab-extract-part".into(), "foo".into()], cancel).await.unwrap(); assert_eq!(fs::read(fixture.target.join("foo.nabnab-extract-part")).unwrap(), b"first member"); assert_eq!(fs::read(fixture.target.join("foo")).unwrap(), b"second member"); assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 2);
    }
    #[test]
    fn manifest_rejects_traversal_cycles_unknown_schema_and_private_track_paths() {
        let base = Manifest { schema_version: 1, format: "nabnab-portable".into(), created_at: 1, files: vec![FileEntry { path: "files/data.bin".into(), size: 1, sha256: "a".repeat(64) }], items: vec![Item { id: "item-0".into(), name: "data".into(), filename: "data.bin".into(), file: "files/data.bin".into(), parent_id: None, metadata: Metadata::default(), capture: None, ocr: None, annotations: vec![] }] };
        validate_manifest(&base).unwrap(); let mut bad = base.clone(); bad.files[0].path = "../outside.bin".into(); assert!(validate_manifest(&bad).is_err()); let mut bad = base.clone(); bad.schema_version = 99; assert!(validate_manifest(&bad).is_err()); let mut bad = base.clone(); bad.items[0].parent_id = Some("item-0".into()); assert!(validate_manifest(&bad).is_err()); let mut bad = base; bad.files[0].size = MAX_BYTES + 1; assert!(validate_manifest(&bad).is_err());
    }
    #[tokio::test]
    async fn canceled_preview_has_no_plan_or_published_output() { let fixture = Fixture::new(); let item = fixture.item("sample.txt", b"selected"); let (sender, cancel) = receiver(); sender.send(true).unwrap(); assert!(preview_export(&fixture.config, &[item], &options(), cancel).await.is_err()); assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 0); }
    #[test]
    fn partial_names_cannot_collide_with_another_imported_file() { let fixture = Fixture::new(); let first = fixture.item("first.bin", b"first"); let second = fixture.item("second.bin", b"second"); let (_sender, cancel) = receiver(); let a = fixture.target.join("entry.nabnab-part"); let b = fixture.target.join("entry"); copy_verified(&first.path, &a, &FileEntry { path: String::new(), size: 5, sha256: hash(b"first") }, &cancel).unwrap(); copy_verified(&second.path, &b, &FileEntry { path: String::new(), size: 6, sha256: hash(b"second") }, &cancel).unwrap(); assert_eq!(fs::read(a).unwrap(), b"first"); assert_eq!(fs::read(b).unwrap(), b"second"); }
}
