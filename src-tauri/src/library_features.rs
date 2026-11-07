use crate::{
    captions::{self, LocalTrack, TrackOrigin},
    library::LibraryItem,
    model::Chapter,
    AppState,
};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf, sync::Arc};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LibraryDetails {
    pub content_id: Option<String>,
    pub creator: Option<String>,
    pub platform: Option<String>,
    pub source_ids: Vec<String>,
    pub source_url:Option<String>,
    pub license: Option<String>,
    pub attribution: Option<String>,
    pub web_archive_ref: Option<String>,
    pub article_annotations: Vec<crate::web_archive::Annotation>,
    pub ocr_ref: Option<String>,
    pub narration: Option<NarrationReference>,
    pub recording: Option<RecordingReference>,
    pub chapters: Vec<Chapter>,
    pub tracks: Vec<LocalTrack>,
    pub playback_position: f64,
    pub favorite: bool,
    pub bookmarks: Vec<Bookmark>,
    pub clip: Option<ClipReference>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NarrationReference {
    pub plan_ref: Option<String>,
    pub text_hash: String,
    pub source_version: String,
    pub voice_name: String,
    pub language: String,
    pub engine_fingerprint: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingReference {
    pub capture_ref: Option<String>,
    pub source_label: String,
    pub kind: String,
    pub duration_ms: u64,
    pub interrupted: bool,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipReference {
    pub requested_start: f64,
    pub requested_end: f64,
    pub actual_start: Option<f64>,
    pub actual_end: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub name: String,
    pub time: f64,
}

pub fn transform_details(
    parent: &LibraryItem,
    options: &crate::tools::ToolOptions,
) -> Result<LibraryDetails, String> {
    let mut details = LibraryDetails {
        content_id: parent.details.content_id.clone(),
        creator: parent.details.creator.clone(),
        platform: parent.details.platform.clone(),
        source_ids: parent.details.source_ids.clone(),
        source_url:parent.details.source_url.clone(),
        narration: parent.details.narration.clone().filter(|_| !matches!(options, crate::tools::ToolOptions::Transcribe {..})).map(|mut reference| { reference.plan_ref = None; reference }),
        chapters: parent.details.chapters.clone(),
        tracks: parent.details.tracks.clone(),
        ..LibraryDetails::default()
    };
    for track in &mut details.tracks {
        track.path = None;
    }
    if matches!(options, crate::tools::ToolOptions::Animate { .. } | crate::tools::ToolOptions::Frame { .. } | crate::tools::ToolOptions::ContactSheet { .. } | crate::tools::ToolOptions::ImageConvert { .. }) || matches!(options, crate::tools::ToolOptions::AudioProcess { operation, .. } if operation == "silence") {
        details.tracks.clear(); details.chapters.clear();
    }
    if let crate::tools::ToolOptions::Trim { start, end, .. } = options {
        details.clip = Some(ClipReference {
            requested_start: *start,
            requested_end: *end,
            actual_start: None,
            actual_end: None,
        });
        for track in &mut details.tracks {
            track.segments = captions::rebase(&track.segments, *start, *end)?;
        }
        details.chapters = details
            .chapters
            .into_iter()
            .filter_map(|chapter| {
                let from = chapter.start.max(*start);
                let until = chapter.end.min(*end);
                (until > from).then(|| Chapter {
                    title: chapter.title,
                    start: from - start,
                    end: until - start,
                })
            })
            .collect();
    }
    Ok(details)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Condition {
    Kind(String),
    Source(String),
    Platform(String),
    Tag(String),
    DownloadedAfter(u64),
    DownloadedBefore(u64),
    MinDuration(f64),
    MaxDuration(f64),
    MinHeight(u32),
    MaxHeight(u32),
    SubtitleLanguage(String),
    HasSubtitles(bool),
    Favorite(bool),
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MatchMode {
    All,
    Any,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartCollection {
    pub id: String,
    pub name: String,
    pub mode: MatchMode,
    pub conditions: Vec<Condition>,
}
fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}
fn condition_matches(item: &LibraryItem, condition: &Condition) -> bool {
    let details = &item.details;
    match condition {
        Condition::Kind(kind) => item.kind == *kind,
        Condition::Source(source) => {
            details.source_ids.contains(source)
                || item.source.as_deref().is_some_and(|s| same(s, source))
        }
        Condition::Platform(platform) => details
            .platform
            .as_deref()
            .is_some_and(|s| same(s, platform)),
        Condition::Tag(tag) => item.tags.iter().any(|s| same(s, tag)),
        Condition::DownloadedAfter(stamp) => item.added_at >= *stamp,
        Condition::DownloadedBefore(stamp) => item.added_at < *stamp,
        Condition::MinDuration(seconds) => item.duration.is_some_and(|s| s >= *seconds),
        Condition::MaxDuration(seconds) => item.duration.is_some_and(|s| s <= *seconds),
        Condition::MinHeight(height) => item.height.is_some_and(|h| h >= *height),
        Condition::MaxHeight(height) => item.height.is_some_and(|h| h <= *height),
        Condition::SubtitleLanguage(language) => details
            .tracks
            .iter()
            .any(|track| same(&track.language, language)),
        Condition::HasSubtitles(required) => !details.tracks.is_empty() == *required,
        Condition::Favorite(required) => details.favorite == *required,
    }
}
pub fn matches(item: &LibraryItem, collection: &SmartCollection) -> bool {
    if collection.conditions.is_empty() {
        return false;
    }
    match collection.mode {
        MatchMode::All => collection
            .conditions
            .iter()
            .all(|c| condition_matches(item, c)),
        MatchMode::Any => collection
            .conditions
            .iter()
            .any(|c| condition_matches(item, c)),
    }
}
impl SmartCollection {
    pub fn validate(&self) -> Result<(), String> {
        valid_name(&self.name)?;
        if self.id.is_empty()
            || self.id.len() > 192
            || self.conditions.is_empty()
            || self.conditions.len() > 24
        {
            return Err("Use a collection ID and between 1 and 24 conditions.".into());
        }
        for c in &self.conditions {
            match c {
                Condition::Kind(v)
                    if !matches!(v.as_str(), "video" | "audio" | "image" | "subtitle") =>
                {
                    return Err("Choose a supported file type.".into())
                }
                Condition::MinDuration(v) | Condition::MaxDuration(v)
                    if !v.is_finite() || *v < 0.0 =>
                {
                    return Err("Use a non-negative duration.".into())
                }
                Condition::Source(v)
                | Condition::Platform(v)
                | Condition::Tag(v)
                | Condition::SubtitleLanguage(v) => {
                    valid_name(v)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}
fn valid_name(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > 160 || value.chars().any(char::is_control)
    {
        return Err("Use a name with 1–160 visible characters.".into());
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SearchHit {
    Article {file_id:String,title:String,block:usize,excerpt:String},
    Ocr {file_id:String,title:String,page:u32,excerpt:String},
    Metadata {
        file_id: String,
        title: String,
        excerpt: String,
    },
    Subtitle {
        file_id: String,
        title: String,
        track_id: String,
        language: String,
        time: f64,
        excerpt: String,
    },
}
pub fn search(
    items: &[LibraryItem],
    query: &str,
    collection: Option<&str>,
    smart: Option<&SmartCollection>,
) -> Vec<SearchHit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return vec![];
    }
    let mut result = Vec::new();
    for item in items.iter().filter(|i| {
        collection.is_none_or(|name| i.collections.iter().any(|c| c == name))
            && smart.is_none_or(|s| matches(i, s))
    }) {
        let details = &item.details;
        let metadata = format!(
            "{} {} {} {} {}",
            item.name,
            details.creator.as_deref().unwrap_or_default(),
            item.tags.join(" "),
            item.collections.join(" "),
            details
                .bookmarks
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        );
        if metadata.to_lowercase().contains(&needle) {
            result.push(SearchHit::Metadata {
                file_id: item.id.clone(),
                title: item.name.clone(),
                excerpt: metadata.chars().take(240).collect(),
            });
        }
        for track in &details.tracks {
            for segment in &track.segments {
                if segment.text.to_lowercase().contains(&needle) {
                    let chars: Vec<char> = segment.text.chars().collect();
                    let offset = segment
                        .text
                        .to_lowercase()
                        .find(&needle)
                        .map(|byte| segment.text.to_lowercase()[..byte].chars().count())
                        .unwrap_or(0);
                    let begin = offset.saturating_sub(60);
                    result.push(SearchHit::Subtitle {
                        file_id: item.id.clone(),
                        title: item.name.clone(),
                        track_id: track.id.clone(),
                        language: track.language.clone(),
                        time: segment.start,
                        excerpt: chars.iter().skip(begin).take(220).collect(),
                    });
                }
                if result.len() >= 100 {
                    return result;
                }
            }
        }
        if result.len() >= 100 {
            return result;
        }
    }
    result
}
#[tauri::command]
pub fn library_search(
    state: State<Arc<AppState>>,
    query: String,
    collection: Option<String>,
    smart_id: Option<String>,
) -> Result<Vec<SearchHit>, String> {
    if query.len() > 2048 {
        return Err("Use a search query of at most 2048 bytes.".into());
    }
    let (root,items)={
        let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open the app first.")?;
        let smart=smart_id.as_ref().and_then(|id|repo.data.smart_collections.iter().find(|s|s.id==*id));
        (repo.root().to_owned(),repo.data.library.iter().filter(|i|collection.as_ref().is_none_or(|name|i.collections.contains(name))&&smart.is_none_or(|s|matches(i,s))).cloned().collect::<Vec<_>>())
    };
    let mut result=search(&items,&query,None,None);let needle=query.trim().to_lowercase();if needle.len()<2{return Ok(result);}
    let mut seen=std::collections::HashSet::new();
    for item in &items {
        if result.len()>=100{break;}
        if let Some(reference)=&item.details.web_archive_ref {
            if seen.insert(format!("article:{reference}")) {
                if let Ok(capture)=crate::web_archive::load(&root,reference){for(block,content)in capture.blocks.iter().enumerate(){if content.text().to_lowercase().contains(&needle){result.push(SearchHit::Article{file_id:item.id.clone(),title:item.name.clone(),block,excerpt:content.text().chars().take(240).collect()});if result.len()>=100{break;}}}}
            }
        }
        if let Some(reference)=&item.details.ocr_ref {
            if seen.insert(reference.clone()) {
                if let Ok(index)=crate::ocr::read_index(&root,reference){if let Ok(hits)=crate::ocr::search(&index,&query){for hit in hits{result.push(SearchHit::Ocr{file_id:item.id.clone(),title:item.name.clone(),page:hit.page,excerpt:hit.excerpt});if result.len()>=100{break;}}}}
            }
        }
    }
    Ok(result)
}
pub(crate) fn edit_item(
    state: &AppState,
    id: &str,
    change: impl FnOnce(&mut LibraryItem) -> Result<(), String>,
) -> Result<(), String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let index = repo
        .data
        .library
        .iter()
        .position(|i| i.id == id)
        .ok_or("Library item not found.")?;
    let previous = repo.data.library[index].clone();
    if let Err(error) = change(&mut repo.data.library[index]) {
        repo.data.library[index] = previous;
        return Err(error);
    }
    if let Err(error) = repo.save() {
        repo.data.library[index] = previous;
        return Err(error);
    }
    Ok(())
}
#[tauri::command]
pub fn library_playback(
    state: State<Arc<AppState>>,
    id: String,
    position: f64,
) -> Result<(), String> {
    if !position.is_finite() || position < 0.0 {
        return Err("Use a valid playback position.".into());
    }
    edit_item(&state, &id, |item| {
        item.details.playback_position = position.min(item.duration.unwrap_or(position));
        Ok(())
    })
}
#[tauri::command]
pub fn library_favorite(
    state: State<Arc<AppState>>,
    id: String,
    favorite: bool,
) -> Result<(), String> {
    edit_item(&state, &id, |item| {
        item.details.favorite = favorite;
        Ok(())
    })
}
#[tauri::command]
pub fn library_bookmark(
    state: State<Arc<AppState>>,
    id: String,
    bookmark: Bookmark,
    remove: bool,
) -> Result<(), String> {
    valid_name(&bookmark.name)?;
    if bookmark.id.is_empty()
        || bookmark.id.len() > 192
        || !bookmark.time.is_finite()
        || bookmark.time < 0.0
    {
        return Err("Use a valid bookmark and time.".into());
    }
    edit_item(&state, &id, |item| {
        if item.duration.is_some_and(|d| bookmark.time > d) {
            return Err("The bookmark is outside this file.".into());
        }
        item.details.bookmarks.retain(|b| b.id != bookmark.id);
        if !remove {
            if item.details.bookmarks.len() >= 256 {
                return Err("Use at most 256 bookmarks per file.".into());
            }
            item.details.bookmarks.push(bookmark);
            item.details
                .bookmarks
                .sort_by(|a, b| a.time.total_cmp(&b.time));
        }
        Ok(())
    })
}
#[tauri::command]
pub fn smart_collection_save(
    state: State<Arc<AppState>>,
    collection: SmartCollection,
) -> Result<(), String> {
    collection.validate()?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let previous = repo.data.smart_collections.clone();
    if let Some(item) = repo
        .data
        .smart_collections
        .iter_mut()
        .find(|c| c.id == collection.id)
    {
        *item = collection;
    } else {
        if repo.data.smart_collections.len() >= 512 {
            return Err("Use at most 512 smart collections.".into());
        }
        repo.data.smart_collections.push(collection);
    }
    if let Err(error) = repo.save() {
        repo.data.smart_collections = previous;
        return Err(error);
    }
    Ok(())
}
#[tauri::command]
pub fn smart_collection_remove(state: State<Arc<AppState>>, id: String) -> Result<(), String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let previous = repo.data.smart_collections.clone();
    repo.data.smart_collections.retain(|c| c.id != id);
    if let Err(error) = repo.save() {
        repo.data.smart_collections = previous;
        return Err(error);
    }
    Ok(())
}
pub fn read_track(
    path: PathBuf,
    language: String,
    origin: TrackOrigin,
) -> Result<LocalTrack, String> {
    if language != "und" && crate::model::normalized_subtitle_language(&language).is_none() {
        return Err("Use a supported subtitle language code, such as es or en.".into());
    }
    let path = crate::library::authorized_path(&path)?;
    if !matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("srt" | "vtt")
    ) {
        return Err("Choose an SRT or VTT file.".into());
    }
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
        return Err("Import subtitle files of at most 16 MB.".into());
    }
    let input = fs::read_to_string(&path).map_err(|e| format!("Could not read subtitles: {e}"))?;
    let segments = captions::parse(&input)?;
    let formatting_lost = input.contains('<')
        || input.contains("STYLE")
        || input.contains("align:")
        || input.contains("{\\");
    use sha2::{Digest, Sha256};
    let id = format!(
        "{:x}",
        Sha256::digest(format!("{}:{language}", path.display()).as_bytes())
    );
    Ok(LocalTrack {
        id,
        language,
        origin,
        path: Some(path),
        segments,
        formatting_lost,
    })
}
#[tauri::command]
pub async fn subtitle_import(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    language: String,
    path: Option<PathBuf>,
) -> Result<LocalTrack, String> {
    let track = tauri::async_runtime::spawn_blocking(move || {
        let path = match path {
            Some(path) => path,
            None => app
                .dialog()
                .file()
                .add_filter("Subtitles", &["srt", "vtt"])
                .blocking_pick_file()
                .ok_or("Cancelled.")?
                .into_path()
                .map_err(|_| "Choose a local subtitle file.")?,
        };
        read_track(path, language, TrackOrigin::Imported)
    })
    .await
    .map_err(|e| e.to_string())??;
    edit_item(&state, &id, |item| {
        item.details.tracks.retain(|t| t.id != track.id);
        if item.details.tracks.len() >= 32 {
            return Err("Use at most 32 subtitle tracks per file.".into());
        }
        item.details.tracks.push(track.clone());
        Ok(())
    })?;
    Ok(track)
}
#[tauri::command]
pub async fn subtitle_export(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    track_id: String,
    format: String,
) -> Result<PathBuf, String> {
    if !matches!(format.as_str(), "txt" | "srt" | "vtt") {
        return Err("Choose text, SRT or VTT.".into());
    }
    let track = {
        let guard = state.repository.lock().unwrap();
        let repo = guard.as_ref().ok_or("Open the app first.")?;
        repo.data
            .library
            .iter()
            .find(|i| i.id == id)
            .and_then(|i| i.details.tracks.iter().find(|t| t.id == track_id))
            .cloned()
            .ok_or("Subtitle track not found.")?
    };
    tauri::async_runtime::spawn_blocking(move || {
        let path = app
            .dialog()
            .file()
            .add_filter("Subtitles", &[format.as_str()])
            .set_file_name(format!("subtitles.{}", format))
            .blocking_save_file()
            .ok_or("Cancelled.")?
            .into_path()
            .map_err(|_| "Choose a local destination.")?;
        let parent = path.parent().ok_or("Choose a local destination.")?;
        crate::library::authorized_path(parent)?;
        let text = match format.as_str() {
            "srt" => captions::to_srt(&track.segments),
            "vtt" => captions::to_vtt(&track.segments),
            _ => captions::plain_text(&track.segments),
        };
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("Could not create export. Choose an unused name: {e}"))?;
        file.write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(path)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub async fn subtitle_rebuild(
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
) -> Result<usize, String> {
    if ids.is_empty() || ids.len() > 20 {
        return Err("Rebuild between 1 and 20 files at a time.".into());
    }
    let items = {
        let guard = state.repository.lock().unwrap();
        let repo = guard.as_ref().ok_or("Open the app first.")?;
        repo.data
            .library
            .iter()
            .filter(|i| ids.contains(&i.id))
            .map(|i| (i.id.clone(), i.details.tracks.clone()))
            .collect::<Vec<_>>()
    };
    let rebuilt = tauri::async_runtime::spawn_blocking(move || {
        let mut result = Vec::new();
        for (id, tracks) in items {
            for track in tracks {
                if let Some(path) = track.path {
                    result.push((id.clone(), read_track(path, track.language, track.origin)?));
                }
            }
        }
        Ok::<_, String>(result)
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open the app first.")?;
    let previous = repo.data.library.clone();
    let count = rebuilt.len();
    for (id, track) in rebuilt {
        if let Some(item) = repo.data.library.iter_mut().find(|i| i.id == id) {
            if let Some(old) = item.details.tracks.iter_mut().find(|t| t.id == track.id) {
                *old = track;
            }
        }
    }
    if let Err(error) = repo.save() {
        repo.data.library = previous;
        return Err(error);
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item() -> LibraryItem {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nabnab-search-{}-{}-{}.mp4",
            std::process::id(),
            crate::queue::now(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::write(&path, b"fixture").unwrap();
        let item = crate::library::index(&path, None, None, None).unwrap();
        fs::remove_file(path).unwrap();
        item
    }
    #[test]
    fn phrase_search_returns_real_file_and_segment_time_and_collection_scope() {
        let mut item = item();
        item.name = "Camera tutorial.mp4".into();
        item.collections = vec!["Reference".into()];
        item.details.tracks = vec![LocalTrack {
            id: "es".into(),
            language: "es".into(),
            origin: TrackOrigin::Imported,
            path: None,
            formatting_lost: false,
            segments: captions::parse(
                "1\n00:02:14,500 --> 00:02:18,000\nUse the camera transition here\n",
            )
            .unwrap(),
        }];
        assert_eq!(
            search(
                &[item.clone()],
                "camera transition",
                Some("Reference"),
                None
            ),
            vec![SearchHit::Subtitle {
                file_id: item.id.clone(),
                title: "Camera tutorial.mp4".into(),
                track_id: "es".into(),
                language: "es".into(),
                time: 134.5,
                excerpt: "Use the camera transition here".into()
            }]
        );
        assert!(search(&[item], "camera transition", Some("Other"), None).is_empty());
    }
    #[test]
    fn smart_collection_membership_changes_with_metadata_without_copying_files() {
        let mut first = item();
        first.kind = "video".into();
        first.height = Some(1080);
        first.tags = vec!["learning".into()];
        let collection = SmartCollection {
            id: "reference".into(),
            name: "References".into(),
            mode: MatchMode::All,
            conditions: vec![Condition::Tag("learning".into()), Condition::MinHeight(720)],
        };
        assert!(matches(&first, &collection));
        first.height = None;
        assert!(!matches(&first, &collection));
        let any = SmartCollection {
            mode: MatchMode::Any,
            ..collection
        };
        assert!(matches(&first, &any));
        assert!(!SmartCollection {
            id: "x".into(),
            name: "X".into(),
            mode: MatchMode::All,
            conditions: vec![]
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn local_chapter_clip_keeps_content_reference_and_rebases_real_caption_segments() {
        let mut parent = item();
        parent.details.content_id = Some("youtube:original".into());
        parent.details.chapters = vec![Chapter {
            title: "Camera".into(),
            start: 134.0,
            end: 150.0,
        }];
        parent.details.tracks = vec![LocalTrack {
            id: "es".into(),
            language: "es".into(),
            origin: TrackOrigin::Imported,
            path: Some(parent.path.with_extension("srt")),
            formatting_lost: false,
            segments: vec![captions::Segment {
                start: 133.0,
                end: 137.0,
                text: "Camera transition".into(),
            }],
        }];
        let details = transform_details(
            &parent,
            &crate::tools::ToolOptions::Trim {
                start: 134.0,
                end: 140.0,
                precise: true,
            },
        )
        .unwrap();
        assert_eq!(details.content_id.as_deref(), Some("youtube:original"));
        assert_eq!(
            details.tracks[0].segments,
            vec![captions::Segment {
                start: 0.0,
                end: 3.0,
                text: "Camera transition".into()
            }]
        );
        assert!(details.tracks[0].path.is_none());
        assert_eq!(details.chapters[0].start, 0.0);
        assert_eq!(details.chapters[0].end, 6.0);
        let clip = details.clip.unwrap();
        assert_eq!(clip.requested_start, 134.0);
        assert_eq!(clip.requested_end, 140.0);
        assert!(clip.actual_start.is_none());
        assert_eq!(parent.details.tracks[0].segments[0].start, 133.0);
    }
}
