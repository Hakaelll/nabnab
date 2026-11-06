use crate::store::Repository;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_IMPORT_ENTRIES: usize = 10_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub kind: String,
    pub size: u64,
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub container: Option<String>,
    pub added_at: u64,
    pub source: Option<String>,
    pub parent_id: Option<String>,
    pub collections: Vec<String>,
    pub tags: Vec<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub details: crate::library_features::LibraryDetails,
}

fn extension_kind(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "m4v" | "mpeg" | "mpg" | "ts" | "mts" | "m2ts"
        | "3gp" | "ogv" | "mxf" => Some("video"),
        "mp3" | "m4a" | "aac" | "opus" | "ogg" | "flac" | "wav" | "wma" | "aiff" | "aif"
        | "alac" => Some("audio"),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tiff" | "tif" | "avif" | "heic"
        | "heif" => Some("image"),
        "srt" | "vtt" => Some("subtitle"),
        "pdf" | "epub" | "doc" | "docx" | "odt" | "ppt" | "pptx" | "odp" | "xls" | "xlsx" | "ods" | "rtf" | "txt" | "md" | "csv" => Some("document"),
        _ => None,
    }
}

pub fn can_open(item: &LibraryItem) -> bool {
    item.kind != "file" && (extension_kind(&item.path).is_some()
        || (item.kind == "article" && item.details.web_archive_ref.is_some()
            && item.path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("html"))))
}

fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Junctions and other reparse points must not escape a selected folder.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    false
}

pub fn authorized_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Choose an absolute local file or folder.".into());
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|e| format!("The selected path is unavailable: {e}"))?;
        if is_link(&metadata) {
            return Err(
                "Symbolic links and junctions are not imported. Choose the real file or folder."
                    .into(),
            );
        }
    }
    fs::canonicalize(path).map_err(|e| format!("The selected path is unavailable: {e}"))
}

fn path_key(path: &Path) -> String {
    let value = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    return value.to_lowercase();
    #[cfg(not(windows))]
    value
}

fn public_origin(source: Option<String>) -> Option<String> {
    source
        .and_then(|value| crate::model::validate_url(&value).ok())
        .and_then(|value| url::Url::parse(&value).ok())
        .map(|value| value.origin().ascii_serialization())
}

pub fn index(
    file: &Path,
    info: Option<&Value>,
    source: Option<String>,
    parent_id: Option<String>,
) -> Result<LibraryItem, String> {
    let path = authorized_path(file)?;
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Choose a regular media file.".into());
    }
    let kind = extension_kind(&path).ok_or("This file type cannot be added to the library.")?;
    let video = info
        .and_then(|v| v["streams"].as_array())
        .and_then(|streams| {
            streams.iter().find(|stream| {
                stream["codec_type"] == "video" && stream["disposition"]["attached_pic"] != 1
            })
        });
    let audio = info
        .and_then(|v| v["streams"].as_array())
        .and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream["codec_type"] == "audio")
        });
    let kind = if kind == "image" {
        kind
    } else if video.is_some() {
        "video"
    } else if audio.is_some() {
        "audio"
    } else {
        kind
    };
    let duration = info
        .and_then(|v| v["format"]["duration"].as_str())
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0);
    let dimension = |key: &str| {
        video
            .and_then(|v| v[key].as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0 && *v <= 65_536)
    };
    let id = format!("{:x}", Sha256::digest(path_key(&path).as_bytes()));
    let mut details = crate::library_features::LibraryDetails {
        chapters: info
            .map(crate::model::chapters_from_value)
            .unwrap_or_default(),
        ..crate::library_features::LibraryDetails::default()
    };
    if kind == "subtitle" {
        if metadata.len() > 16 * 1024 * 1024 {
            return Err("Import subtitle files of at most 16 MB.".into());
        }
        let text =
            fs::read_to_string(&path).map_err(|e| format!("Could not read subtitles: {e}"))?;
        details.tracks.push(crate::captions::LocalTrack {
            id: id.clone(),
            language: "und".into(),
            origin: crate::captions::TrackOrigin::Imported,
            path: Some(path.clone()),
            segments: crate::captions::parse(&text)?,
            formatting_lost: text.contains('<')
                || text.contains("STYLE")
                || text.contains("align:"),
        });
    }
    Ok(LibraryItem {
        id,
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        path,
        kind: kind.into(),
        size: metadata.len(),
        duration,
        width: dimension("width"),
        height: dimension("height"),
        video_codec: video
            .and_then(|v| v["codec_name"].as_str())
            .map(str::to_owned),
        audio_codec: audio
            .and_then(|v| v["codec_name"].as_str())
            .map(str::to_owned),
        container: info
            .and_then(|v| v["format"]["format_name"].as_str())
            .map(str::to_owned),
        added_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64,
        source: public_origin(source),
        parent_id,
        collections: Vec::new(),
        tags: Vec::new(),
        content_hash: None,
        details,
    })
}

pub fn index_file(file: &Path) -> Result<LibraryItem, String> {
    let path = authorized_path(file)?;
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() == 0 { return Err("The downloaded file is empty or not a regular file.".into()); }
    Ok(LibraryItem { id: format!("{:x}", Sha256::digest(path_key(&path).as_bytes())), name: path.file_name().ok_or("Missing filename.")?.to_string_lossy().into_owned(), path, kind: "file".into(), size: metadata.len(), duration: None, width: None, height: None, video_codec: None, audio_codec: None, container: None, added_at: crate::queue::now(), source: None, parent_id: None, collections: vec![], tags: vec![], content_hash: None, details: Default::default() })
}

fn collect_files(
    path: &Path,
    boundary: &Path,
    files: &mut Vec<PathBuf>,
    count: &mut usize,
) -> Result<(), String> {
    let mut pending = vec![path.to_owned()];
    while let Some(path) = pending.pop() {
        *count += 1;
        if *count > MAX_IMPORT_ENTRIES {
            return Err("Import at most 10,000 entries at a time. Choose a smaller folder.".into());
        }
        let metadata = fs::symlink_metadata(&path)
            .map_err(|e| format!("Could not read selected media: {e}"))?;
        if is_link(&metadata) {
            return Err("The selected folder contains a symbolic link or junction. Import its real files separately.".into());
        }
        let canonical = fs::canonicalize(&path).map_err(|e| e.to_string())?;
        if !canonical.starts_with(boundary) {
            return Err("An import path left the selected folder.".into());
        }
        if metadata.is_dir() {
            let entries = fs::read_dir(&path)
                .map_err(|e| format!("Could not read the selected folder: {e}"))?;
            for entry in entries {
                if *count + pending.len() >= MAX_IMPORT_ENTRIES {
                    return Err(
                        "Import at most 10,000 entries at a time. Choose a smaller folder.".into(),
                    );
                }
                pending.push(entry.map_err(|e| e.to_string())?.path());
            }
        } else if metadata.is_file() && extension_kind(&canonical).is_some() {
            files.push(canonical);
        }
    }
    Ok(())
}

pub fn import_paths(repository: &mut Repository, paths: &[PathBuf]) -> Result<usize, String> {
    if paths.len() > MAX_IMPORT_ENTRIES {
        return Err("Import at most 10,000 entries at a time.".into());
    }
    let mut files = Vec::new();
    let mut count = 0;
    for path in paths {
        let canonical = authorized_path(path)?;
        if canonical.is_file() && extension_kind(&canonical).is_none() {
            return Err("This file type cannot be added to the library.".into());
        }
        collect_files(&canonical, &canonical, &mut files, &mut count)?;
    }
    let mut known: HashSet<_> = repository
        .data
        .library
        .iter()
        .map(|item| path_key(&item.path))
        .collect();
    let mut additions = Vec::new();
    for file in files {
        if known.insert(path_key(&file)) {
            additions.push(index(&file, None, None, None)?);
        }
    }
    let added = additions.len();
    if added == 0 {
        return Ok(0);
    }
    let previous_len = repository.data.library.len();
    repository.data.library.extend(additions);
    if let Err(error) = repository.save() {
        repository.data.library.truncate(previous_len);
        return Err(error);
    }
    Ok(added)
}

fn labels(values: Vec<String>) -> Result<Vec<String>, String> {
    if values.len() > 64 {
        return Err("Use at most 64 collections or tags per file.".into());
    }
    let mut result = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if value.chars().count() > 80 || value.chars().any(char::is_control) {
            return Err("Collection and tag names must contain 1–80 visible characters.".into());
        }
        if !result
            .iter()
            .any(|v: &String| v.eq_ignore_ascii_case(value))
        {
            result.push(value.to_owned());
        }
    }
    Ok(result)
}

pub fn update(
    repository: &mut Repository,
    id: &str,
    collections: Vec<String>,
    tags: Vec<String>,
) -> Result<(), String> {
    let collections = labels(collections)?;
    let tags = labels(tags)?;
    let position = repository
        .data
        .library
        .iter()
        .position(|item| item.id == id)
        .ok_or("This library item no longer exists.")?;
    let previous_item = repository.data.library[position].clone();
    let previous_collections = repository.data.collections.clone();
    for collection in &collections {
        if !repository.data.collections.contains(collection) {
            repository.data.collections.push(collection.clone());
        }
    }
    repository.data.library[position].collections = collections;
    repository.data.library[position].tags = tags;
    if let Err(error) = repository.save() {
        repository.data.library[position] = previous_item;
        repository.data.collections = previous_collections;
        return Err(error);
    }
    Ok(())
}

pub fn remove(repository: &mut Repository, id: &str) -> Result<(), String> {
    let position = repository
        .data
        .library
        .iter()
        .position(|item| item.id == id)
        .ok_or("This library item no longer exists.")?;
    let item = repository.data.library.remove(position);
    if let Err(error) = repository.save() {
        repository.data.library.insert(position, item);
        return Err(error);
    }
    Ok(())
}

pub fn locate(repository: &mut Repository, id: &str, path: &Path) -> Result<(), String> {
    let position = repository
        .data
        .library
        .iter()
        .position(|item| item.id == id)
        .ok_or("This library item no longer exists.")?;
    let previous = repository.data.library[position].clone();
    let mut replacement = index(
        path,
        None,
        previous.source.clone(),
        previous.parent_id.clone(),
    )?;
    if repository
        .data
        .library
        .iter()
        .enumerate()
        .any(|(index, item)| {
            index != position && path_key(&item.path) == path_key(&replacement.path)
        })
    {
        return Err("The chosen file is already in the library.".into());
    }
    replacement.id = previous.id.clone();
    replacement.added_at = previous.added_at;
    replacement.collections = previous.collections.clone();
    replacement.tags = previous.tags.clone();
    replacement.details = previous.details.clone();
    repository.data.library[position] = replacement;
    if let Err(error) = repository.save() {
        repository.data.library[position] = previous;
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renamed_documents_cannot_open_executables_or_shell_shortcuts() {
        let root = directory();
        let file = root.join("reading.txt");
        fs::write(&file, b"local document").unwrap();
        let mut item = index(&file, None, None, None).unwrap();
        assert!(can_open(&item));
        for extension in ["exe", "cmd", "ps1", "lnk", "url", "cpl", "hta"] {
            item.path = root.join(format!("reading.{extension}"));
            assert!(!can_open(&item), "The actual extension must guard system-open.");
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn subtitle_files_index_real_timestamps_and_removal_retires_their_search_entries() {
        let root = directory();
        let path = root.join("reference.srt");
        fs::write(
            &path,
            "1\n00:02:14,500 --> 00:02:17,000\nThe camera transition\n",
        )
        .unwrap();
        let mut repository = Repository::open(&root.join("data"), &root).unwrap();
        import_paths(&mut repository, &[path.clone()]).unwrap();
        assert_eq!(repository.data.library[0].kind, "subtitle");
        assert_eq!(
            repository.data.library[0].details.tracks[0].segments[0].start,
            134.5
        );
        assert_eq!(
            crate::library_features::search(
                &repository.data.library,
                "camera transition",
                None,
                None
            )
            .len(),
            1
        );
        let id = repository.data.library[0].id.clone();
        remove(&mut repository, &id).unwrap();
        assert!(crate::library_features::search(
            &repository.data.library,
            "camera transition",
            None,
            None
        )
        .is_empty());
        assert!(path.is_file());
        drop(repository);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn probed_tracks_determine_audio_and_video_filters() {
        let root = std::env::temp_dir().join(format!(
            "nabnab-kind-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("audio-in-video-container.mp4");
        fs::write(&file, b"original").unwrap();
        let info = serde_json::json!({"streams":[{"codec_type":"audio","codec_name":"aac"},{"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}}],"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"10"}});
        assert_eq!(index(&file, Some(&info), None, None).unwrap().kind, "audio");
        assert_eq!(fs::read(&file).unwrap(), b"original");
        fs::remove_dir_all(root).unwrap();
    }
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    fn directory() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "nabnab-library-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn import_deduplicates_without_copying_or_deleting_originals() {
        let root = directory();
        let media = root.join("media");
        fs::create_dir(&media).unwrap();
        let file = media.join("Audio á.opus");
        fs::write(&file, b"original").unwrap();
        fs::write(media.join("ignored.exe"), b"not media").unwrap();
        let mut repository = Repository::open(&root.join("data"), &media).unwrap();
        assert_eq!(
            import_paths(&mut repository, &[media.clone(), file.clone()]).unwrap(),
            1
        );
        assert_eq!(import_paths(&mut repository, &[file.clone()]).unwrap(), 0);
        assert_eq!(repository.data.library[0].kind, "audio");
        assert_eq!(repository.data.library[0].size, 8);
        assert_eq!(
            repository.data.library[0].path,
            fs::canonicalize(&file).unwrap()
        );
        let id = repository.data.library[0].id.clone();
        remove(&mut repository, &id).unwrap();
        assert!(repository.data.library.is_empty());
        assert_eq!(fs::read(&file).unwrap(), b"original");
        drop(repository);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn inspector_metadata_and_origin_discard_signed_url_parameters() {
        let root = directory();
        let file = root.join("clip.mp4");
        fs::write(&file, b"verified fixture").unwrap();
        let info = serde_json::json!({"streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080},{"codec_type":"audio","codec_name":"aac"}],"format":{"duration":"12.5","format_name":"mov,mp4,m4a,3gp,3g2,mj2"}});
        let item = index(
            &file,
            Some(&info),
            Some("https://example.com/clip?token=secret#private".into()),
            Some("original-id".into()),
        )
        .unwrap();
        assert_eq!(item.width, Some(1920));
        assert_eq!(item.height, Some(1080));
        assert_eq!(item.duration, Some(12.5));
        assert_eq!(item.video_codec.as_deref(), Some("h264"));
        assert_eq!(item.audio_codec.as_deref(), Some("aac"));
        assert_eq!(item.source.as_deref(), Some("https://example.com"));
        assert_eq!(item.parent_id.as_deref(), Some("original-id"));
        assert!(!serde_json::to_string(&item).unwrap().contains("secret"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_file_can_be_relocated_while_preserving_organization() {
        let root = directory();
        let original = root.join("old.mp4");
        fs::write(&original, b"original").unwrap();
        let mut repository = Repository::open(&root.join("data"), &root).unwrap();
        import_paths(&mut repository, &[original.clone()]).unwrap();
        let id = repository.data.library[0].id.clone();
        update(
            &mut repository,
            &id,
            vec![" Films ".into(), "Films".into()],
            vec!["travel".into()],
        )
        .unwrap();
        let moved = root.join("new.mp4");
        fs::rename(&original, &moved).unwrap();
        assert!(!repository.data.library[0].path.is_file());
        assert!(index(&original, None, None, None).is_err());
        locate(&mut repository, &id, &moved).unwrap();
        let item = &repository.data.library[0];
        assert_eq!(item.id, id);
        assert_eq!(item.collections, vec!["Films"]);
        assert_eq!(item.tags, vec!["travel"]);
        assert_eq!(item.path, fs::canonicalize(&moved).unwrap());
        assert_eq!(fs::read(&moved).unwrap(), b"original");
        drop(repository);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_import_and_invalid_labels_leave_library_unchanged() {
        let root = directory();
        let file = root.join("clip.mp4");
        fs::write(&file, b"original").unwrap();
        let mut repository = Repository::open(&root.join("data"), &root).unwrap();
        assert!(import_paths(&mut repository, &[file.clone(), root.join("missing.mp4")]).is_err());
        assert!(repository.data.library.is_empty());
        import_paths(&mut repository, &[file]).unwrap();
        let id = repository.data.library[0].id.clone();
        assert!(update(&mut repository, &id, vec!["bad\nname".into()], vec![]).is_err());
        assert!(repository.data.library[0].collections.is_empty());
        assert!(import_paths(&mut repository, &[PathBuf::from("relative.mp4")]).is_err());
        drop(repository);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_cannot_escape_authorized_folder() {
        let root = directory();
        let selected = root.join("selected");
        fs::create_dir(&selected).unwrap();
        let outside = root.join("outside.mp4");
        fs::write(&outside, b"outside").unwrap();
        std::os::unix::fs::symlink(&outside, selected.join("link.mp4")).unwrap();
        let mut repository = Repository::open(&root.join("data"), &root).unwrap();
        assert!(import_paths(&mut repository, &[selected]).is_err());
        assert!(repository.data.library.is_empty());
        assert_eq!(fs::read(&outside).unwrap(), b"outside");
        drop(repository);
        fs::remove_dir_all(root).unwrap();
    }
}
