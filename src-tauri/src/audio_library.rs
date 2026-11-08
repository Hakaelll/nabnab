use crate::{library::LibraryItem, library_features, queue, store, AppState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

pub(crate) mod playlist_import;
pub(crate) mod tag_copy;

const MAX_SELECTION: usize = 200;
const MAX_PLAYLISTS: usize = 100;
const MAX_ENTRIES: usize = 2000;
const MAX_LIBRARY_ENTRIES: usize = 12000;
const MAX_LYRICS: usize = 512 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub disc: Option<u16>,
    pub track: Option<u16>,
    pub embedded_cover: bool,
}

fn tag(value: &str) -> Option<String> {
    let value: String = value
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(256)
        .collect();
    (!value.is_empty()).then_some(value)
}

fn track_number(value: &str) -> Option<u16> {
    value
        .split('/')
        .next()?
        .trim()
        .parse()
        .ok()
        .filter(|value| *value > 0)
}

pub fn from_probe(info: &Value) -> Tags {
    let streams = info["streams"].as_array();
    let audio = streams.and_then(|items| items.iter().find(|item| item["codec_type"] == "audio"));
    let format_tags = info["format"]["tags"].as_object();
    let stream_tags = audio.and_then(|item| item["tags"].as_object());
    let value = |names: &[&str]| {
        [format_tags, stream_tags]
            .into_iter()
            .flatten()
            .find_map(|tags| {
                tags.iter()
                    .find(|(key, _)| names.iter().any(|name| key.eq_ignore_ascii_case(name)))
                    .and_then(|(_, value)| value.as_str())
                    .and_then(tag)
            })
    };
    Tags {
        title: value(&["title"]),
        artist: value(&["artist", "author"]),
        album_artist: value(&["album_artist", "albumartist", "album artist"]),
        album: value(&["album"]),
        genre: value(&["genre"]),
        disc: value(&["disc", "discnumber"])
            .as_deref()
            .and_then(track_number),
        track: value(&["track", "tracknumber"])
            .as_deref()
            .and_then(track_number),
        embedded_cover: streams.is_some_and(|items| {
            items.iter().any(|item| {
                item["codec_type"] == "video" && item["disposition"]["attached_pic"] == 1
            })
        }),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Patch {
    title: Option<String>,
    artist: Option<String>,
    album_artist: Option<String>,
    album: Option<String>,
    genre: Option<String>,
    disc: Option<String>,
    track: Option<String>,
}

fn valid_label(value: &str, max: usize) -> bool {
    value.chars().count() <= max && !value.chars().any(char::is_control)
}

impl Patch {
    fn fields(&self) -> [(&'static str, Option<&str>); 7] {
        [
            ("title", self.title.as_deref()),
            ("artist", self.artist.as_deref()),
            ("album_artist", self.album_artist.as_deref()),
            ("album", self.album.as_deref()),
            ("genre", self.genre.as_deref()),
            ("disc", self.disc.as_deref()),
            ("track", self.track.as_deref()),
        ]
    }

    fn validate(&self) -> Result<(), String> {
        for value in [
            &self.title,
            &self.artist,
            &self.album_artist,
            &self.album,
            &self.genre,
        ]
        .into_iter()
        .flatten()
        {
            if !valid_label(value, 256) {
                return Err(
                    "Audio tags may contain at most 256 characters without control characters."
                        .into(),
                );
            }
        }
        for value in [&self.disc, &self.track].into_iter().flatten() {
            if !value.trim().is_empty()
                && value
                    .trim()
                    .parse::<u16>()
                    .ok()
                    .filter(|value| *value > 0)
                    .is_none()
            {
                return Err(
                    "Enter a positive disc or track number, or leave it blank to clear it.".into(),
                );
            }
        }
        Ok(())
    }

    fn apply(&self, tags: &mut Tags) {
        if let Some(value) = &self.title {
            tags.title = tag(value);
        }
        if let Some(value) = &self.artist {
            tags.artist = tag(value);
        }
        if let Some(value) = &self.album_artist {
            tags.album_artist = tag(value);
        }
        if let Some(value) = &self.album {
            tags.album = tag(value);
        }
        if let Some(value) = &self.genre {
            tags.genre = tag(value);
        }
        if let Some(value) = &self.disc {
            tags.disc = track_number(value);
        }
        if let Some(value) = &self.track {
            tags.track = track_number(value);
        }
    }
}

fn selected(
    repo: &store::Repository,
    ids: &[String],
    max: usize,
) -> Result<Vec<(usize, LibraryItem)>, String> {
    if ids.is_empty() || ids.len() > max {
        return Err(format!("Choose 1–{max} audio files."));
    }
    let mut seen = HashSet::new();
    ids.iter()
        .filter(|id| seen.insert(*id))
        .map(|id| {
            repo.data
                .library
                .iter()
                .enumerate()
                .find(|(_, item)| item.kind == "audio" && &item.id == id)
                .map(|(index, item)| (index, item.clone()))
                .ok_or_else(|| "A selected audio file is no longer in the library.".into())
        })
        .collect()
}

#[tauri::command]
pub fn audio_tags_edit(
    state: State<Arc<AppState>>,
    ids: Vec<String>,
    patch: Patch,
) -> Result<(), String> {
    patch.validate()?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let before = selected(repo, &ids, MAX_SELECTION)?;
    for (index, _) in &before {
        patch.apply(
            repo.data.library[*index]
                .details
                .audio
                .get_or_insert_with(Tags::default),
        );
    }
    if let Err(error) = repo.save() {
        for (index, item) in before {
            repo.data.library[index] = item;
        }
        return Err(error);
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadReport {
    updated: usize,
    unavailable: usize,
}

#[tauri::command]
pub async fn audio_tags_read(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    ids: Vec<String>,
    request_id: u64,
) -> Result<ReadReport, String> {
    let engines = crate::dependencies::engine_root(&app)?;
    let before = {
        let guard = state.repository.lock().unwrap();
        selected(guard.as_ref().ok_or("Open nabnab first.")?, &ids, 50)?
    };
    let cancel = crate::file_sources::begin(&state, request_id)?;
    let mut discovered = Vec::new();
    for (_, item) in &before {
        if *cancel.borrow() {
            break;
        }
        if let Ok(info) = crate::media::probe(&engines, &item.path, cancel.clone()).await {
            discovered.push((item, from_probe(&info)));
        }
    }
    crate::file_sources::finish(&state, request_id);
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let mut changed = Vec::new();
    for (before, tags) in discovered {
        let Some(item) = repo.data.library.iter_mut().find(|item| {
            item.id == before.id
                && item.path == before.path
                && item.details.audio == before.details.audio
        }) else {
            continue;
        };
        changed.push((item.id.clone(), item.details.audio.replace(tags)));
    }
    if !changed.is_empty() {
        if let Err(error) = repo.save() {
            for (id, previous) in changed {
                if let Some(item) = repo.data.library.iter_mut().find(|item| item.id == id) {
                    item.details.audio = previous;
                }
            }
            return Err(error);
        }
    }
    Ok(ReadReport {
        updated: changed.len(),
        unavailable: before.len().saturating_sub(changed.len()),
    })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PlaylistMatchHints {
    id: Option<String>,
    artist: Option<String>,
    duration: Option<f64>,
    version: Option<String>,
    path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlaylistEntry {
    Local { id: String, title: String },
    Remote {
        url: String,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hints: Option<PlaylistMatchHints>,
    },
    Missing {
        locator: String,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hints: Option<PlaylistMatchHints>,
    },
}

impl PlaylistEntry {
    fn validate(&self) -> Result<(), String> {
        let (title, valid) = match self {
            Self::Local { id, title } => (
                title,
                !id.is_empty()
                    && id.len() <= 128
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
            ),
            Self::Remote { url, title, .. } => (
                title,
                url.len() <= 2048 && crate::model::validate_url(url).is_ok(),
            ),
            Self::Missing { locator, title, .. } => {
                (title, !locator.is_empty() && valid_label(locator, 2048))
            }
        };
        if !valid || title.trim().is_empty() || !valid_label(title, 256) {
            return Err("The playlist contains an invalid entry.".into());
        }
        if let Self::Remote { hints: Some(hints), .. } | Self::Missing { hints: Some(hints), .. } = self {
            if [&hints.id, &hints.artist, &hints.version].into_iter().flatten().any(|value| !valid_label(value, 256))
                || hints.duration.is_some_and(|value| !value.is_finite() || !(0.0..=604800.0).contains(&value))
                || hints.path.as_deref().is_some_and(|value| !valid_label(value, 2048) || !Path::new(value).is_absolute())
            {
                return Err("The playlist contains invalid recording metadata.".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub entries: Vec<PlaylistEntry>,
}

pub fn validate_playlists(playlists: &[Playlist]) -> Result<(), String> {
    let mut ids = HashSet::new();
    if playlists.len() > MAX_PLAYLISTS
        || playlists
            .iter()
            .map(|list| list.entries.len())
            .sum::<usize>()
            > MAX_LIBRARY_ENTRIES
    {
        return Err("The audio playlist library exceeds its storage limit.".into());
    }
    for list in playlists {
        if list.id.len() != 64
            || !list.id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !ids.insert(&list.id)
            || !valid_label(&list.name, 100)
            || list.name.trim().is_empty()
            || list.entries.len() > MAX_ENTRIES
        {
            return Err("The saved audio playlist has an invalid identity, name or size.".into());
        }
        for entry in &list.entries {
            entry.validate()?;
        }
    }
    Ok(())
}

fn playlist_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let value = format!(
        "{}:{}:{}",
        std::process::id(),
        queue::now(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    format!("{:x}", Sha256::digest(value))
}

#[tauri::command]
pub fn audio_playlist_save(
    state: State<Arc<AppState>>,
    id: Option<String>,
    name: String,
    entries: Vec<PlaylistEntry>,
) -> Result<Vec<Playlist>, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let mut next = repo.data.audio_playlists.clone();
    if let Some(id) = id {
        let list = next
            .iter_mut()
            .find(|list| list.id == id)
            .ok_or("The playlist was removed.")?;
        list.name = name.trim().into();
        list.entries = entries;
    } else {
        next.push(Playlist {
            id: playlist_id(),
            name: name.trim().into(),
            entries,
        });
    }
    replace_playlists(repo, next)
}

fn replace_playlists(
    repo: &mut store::Repository,
    next: Vec<Playlist>,
) -> Result<Vec<Playlist>, String> {
    validate_playlists(&next)?;
    let previous = std::mem::replace(&mut repo.data.audio_playlists, next);
    if let Err(error) = repo.save() {
        repo.data.audio_playlists = previous;
        return Err(error);
    }
    Ok(repo.data.audio_playlists.clone())
}

#[tauri::command]
pub fn audio_playlist_remove(
    state: State<Arc<AppState>>,
    id: String,
) -> Result<Vec<Playlist>, String> {
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let mut next = repo.data.audio_playlists.clone();
    next.retain(|list| list.id != id);
    replace_playlists(repo, next)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyric {
    pub time: f64,
    pub text: String,
}

fn lyric_time(value: &str) -> Option<f64> {
    let (minutes, seconds) = value.split_once(':')?;
    let minutes: u32 = minutes.parse().ok()?;
    let seconds: f64 = seconds.parse().ok()?;
    (minutes <= 1440 && (0.0..60.0).contains(&seconds)).then(|| f64::from(minutes) * 60.0 + seconds)
}

fn parse_lrc(raw: &str) -> Result<Vec<Lyric>, String> {
    let mut offset = 0.0;
    let mut lyrics = Vec::new();
    for line in raw.trim_start_matches('\u{feff}').lines() {
        let mut remaining = line.trim();
        if let Some(value) = remaining
            .strip_prefix("[offset:")
            .and_then(|value| value.strip_suffix(']'))
        {
            if let Ok(value) = value.trim().parse::<i32>() {
                offset = f64::from(value.clamp(-600_000, 600_000)) / 1000.0;
            }
            continue;
        }
        let mut times = Vec::new();
        while let Some((time, rest)) = remaining
            .strip_prefix('[')
            .and_then(|value| value.split_once(']'))
        {
            let Some(time) = lyric_time(time) else {
                break;
            };
            if times.len() == 32 {
                return Err("A lyric line has too many timestamps.".into());
            }
            times.push(time);
            remaining = rest;
        }
        if let Some(text) = tag(remaining) {
            for time in times {
                if lyrics.len() == 3000 {
                    return Err("The LRC file exceeds 3000 lyric lines.".into());
                }
                lyrics.push(Lyric {
                    time,
                    text: text.clone(),
                });
            }
        }
    }
    for line in &mut lyrics {
        line.time = (line.time + offset).max(0.0);
    }
    lyrics.sort_by(|left, right| left.time.total_cmp(&right.time));
    if lyrics.is_empty() {
        return Err("This LRC file has no supported timed lyrics.".into());
    }
    Ok(lyrics)
}

fn read_lrc(path: &Path) -> Result<Vec<Lyric>, String> {
    let path = crate::library::authorized_path(path)?;
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lrc"))
        || !path.is_file()
    {
        return Err("Choose a local LRC file.".into());
    }
    let mut text = String::new();
    fs::File::open(path)
        .map_err(|_| "Could not open the LRC file.")?
        .take(MAX_LYRICS as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|_| "The LRC file must use UTF-8.")?;
    if text.len() > MAX_LYRICS {
        return Err("Choose an LRC file under 512 KiB.".into());
    }
    parse_lrc(&text)
}

fn audio_item(state: &AppState, id: &str) -> Result<LibraryItem, String> {
    let guard = state.repository.lock().unwrap();
    guard
        .as_ref()
        .ok_or("Open nabnab first.")?
        .data
        .library
        .iter()
        .find(|item| item.id == id && item.kind == "audio")
        .cloned()
        .ok_or_else(|| "Choose a library audio file.".into())
}

#[tauri::command]
pub fn audio_lyrics(state: State<Arc<AppState>>, id: String) -> Result<Vec<Lyric>, String> {
    let item = audio_item(&state, &id)?;
    if let Some(lines) = item.details.lyrics {
        return Ok(lines);
    }
    let audio = crate::library::authorized_path(&item.path)?;
    let nearby = audio.with_extension("lrc");
    match fs::symlink_metadata(&nearby) {
        Ok(_) => read_lrc(&nearby),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err("Could not inspect the local LRC file.".into()),
    }
}

#[tauri::command]
pub async fn audio_lyrics_choose(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<Option<Vec<Lyric>>, String> {
    audio_item(&state, &id)?;
    let lines = tauri::async_runtime::spawn_blocking(move || {
        let selected = app
            .dialog()
            .file()
            .add_filter("LRC", &["lrc"])
            .blocking_pick_file();
        let path = selected
            .map(|file| file.into_path().map_err(|_| "Choose a local LRC file."))
            .transpose()?;
        path.as_deref().map(read_lrc).transpose()
    })
    .await
    .map_err(|_| "Could not read the selected LRC file.")??;
    if let Some(lines) = &lines {
        library_features::edit_item(&state, &id, |item| {
            item.details.lyrics = Some(lines.clone());
            Ok(())
        })?;
    }
    Ok(lines)
}

#[tauri::command]
pub async fn audio_cover_choose(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    remove: bool,
) -> Result<bool, String> {
    audio_item(&state, &id)?;
    if remove {
        library_features::edit_item(&state, &id, |item| {
            item.details.audio_cover = None;
            Ok(())
        })?;
        return Ok(true);
    }
    let engines = crate::dependencies::engine_root(&app)?;
    let selected = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Cover art", &["jpg", "jpeg", "png", "webp"])
            .blocking_pick_file()
            .map(|file| file.into_path().map_err(|_| "Choose a local cover image."))
            .transpose()
    })
    .await
    .map_err(|_| "Could not open the cover chooser.")??;
    let Some(path): Option<PathBuf> = selected else {
        return Ok(false);
    };
    let path = crate::library::authorized_path(&path)?;
    let metadata = fs::metadata(&path).map_err(|_| "Could not inspect the local cover.")?;
    if !metadata.is_file()
        || metadata.len() > 24 * 1024 * 1024
        || !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "jpg" | "jpeg" | "png" | "webp"
                )
            })
    {
        return Err("Choose a local JPEG, PNG or WebP image under 24 MiB.".into());
    }
    let (_sender, cancel) = tokio::sync::watch::channel(false);
    let info = crate::media::probe(&engines, &path, cancel).await?;
    let valid = info["streams"].as_array().is_some_and(|streams| {
        streams.iter().any(|stream| {
            stream["codec_type"] == "video"
                && stream["width"]
                    .as_u64()
                    .zip(stream["height"].as_u64())
                    .is_some_and(|(width, height)| {
                        (1..=5000).contains(&width) && (1..=5000).contains(&height)
                    })
        })
    });
    if !valid {
        return Err("Choose a valid image up to 5000 × 5000 pixels.".into());
    }
    library_features::edit_item(&state, &id, |item| {
        item.details.audio_cover = Some(path);
        Ok(())
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_preserves_album_artist_disc_and_embedded_art() {
        let info = serde_json::json!({"format":{"tags":{"ARTIST":"Solo","album_artist":"Ensemble","DISC":"2/3","track":"9/12"}},
            "streams":[{"codec_type":"video","disposition":{"attached_pic":1}}]});
        let tags = from_probe(&info);
        assert_eq!(tags.artist.as_deref(), Some("Solo"));
        assert_eq!(tags.album_artist.as_deref(), Some("Ensemble"));
        assert_eq!(tags.disc, Some(2));
        assert_eq!(tags.track, Some(9));
        assert!(tags.embedded_cover);
    }

    #[test]
    fn timed_lyrics_keep_repeated_timestamps_and_apply_offset() {
        let result =
            parse_lrc("[ar:Artist]\n[offset:-500]\n[00:03.40][00:08.30]Line\n[00:01.00]Start")
                .unwrap();
        assert_eq!(
            result
                .iter()
                .map(|cue| cue.text.as_str())
                .collect::<Vec<_>>(),
            ["Start", "Line", "Line"]
        );
        assert_eq!(result[0].time, 0.5);
        assert!((result[1].time - 2.9).abs() < 0.00001);
        assert!((result[2].time - 7.8).abs() < 0.00001);
    }
}
