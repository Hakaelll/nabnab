use super::{tag, PlaylistEntry, MAX_ENTRIES};
use crate::{delimited, library::LibraryItem, AppState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

const MAX_INPUT: usize = 2 * 1024 * 1024;

#[derive(Default)]
pub struct Browser(Mutex<Option<Table>>);

struct Table {
    id: String,
    name: String,
    directory: PathBuf,
    columns: Vec<String>,
    rows: Vec<Vec<String>>,
}

#[derive(Default)]
struct Row {
    locator: Option<String>,
    id: Option<String>,
    title: Option<String>,
    artist: Option<String>,
    duration: Option<f64>,
    version: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    id: String,
    title: String,
    artist: Option<String>,
    album: Option<String>,
    duration: Option<f64>,
    version_matches: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    entry: PlaylistEntry,
    artist: Option<String>,
    duration: Option<f64>,
    version: Option<String>,
    candidates: Vec<Candidate>,
}

#[derive(Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Preview {
    Csv {
        id: String,
        name: String,
        columns: Vec<String>,
        sample: Vec<Vec<String>>,
        count: usize,
    },
    List {
        name: String,
        rows: Vec<Match>,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Mapping {
    locator: Option<usize>,
    id: Option<usize>,
    title: Option<usize>,
    artist: Option<usize>,
    duration: Option<usize>,
    version: Option<usize>,
    milliseconds: bool,
}

fn duration(value: &str, milliseconds: bool) -> Option<f64> {
    let value = value.trim();
    let result = if value.contains(':') {
        let parts: Vec<_> = value
            .split(':')
            .map(str::parse::<f64>)
            .collect::<Result<_, _>>()
            .ok()?;
        if !(2..=3).contains(&parts.len())
            || parts.iter().any(|part| !part.is_finite() || *part < 0.0)
        {
            return None;
        }
        parts
            .into_iter()
            .fold(0.0, |seconds, part| seconds * 60.0 + part)
    } else {
        let parsed = value.parse::<f64>().ok()?;
        if milliseconds {
            parsed / 1000.0
        } else {
            parsed
        }
    };
    (result.is_finite() && (0.0..=7.0 * 86400.0).contains(&result)).then_some(result)
}

fn locator(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || !super::valid_label(value, 2048) || value.len() > 2048 {
        return Err("A playlist path or URL is empty, unsafe or too long.".into());
    }
    Ok(value.into())
}

fn parse_m3u(text: &str) -> Result<Vec<Row>, String> {
    let mut rows = Vec::new();
    let mut next = Row::default();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if line.starts_with("#EXT-X-") {
            return Err(
                "Choose a track playlist. HLS streaming manifests are not imported.".into(),
            );
        }
        if let Some((length, title)) = line
            .strip_prefix("#EXTINF:")
            .and_then(|line| line.split_once(','))
        {
            next.duration = duration(length, false);
            if let Some((artist, title)) = title.split_once(" - ") {
                next.artist = tag(artist);
                next.title = tag(title);
            } else {
                next.title = tag(title);
            }
        } else if !line.starts_with('#') {
            if rows.len() == MAX_ENTRIES {
                return Err("The playlist exceeds 2000 entries.".into());
            }
            next.locator = Some(locator(line)?);
            rows.push(std::mem::take(&mut next));
        }
    }
    Ok(rows)
}

fn parse_pls(text: &str) -> Result<Vec<Row>, String> {
    let mut rows = BTreeMap::<usize, Row>::new();
    let mut in_playlist = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_playlist = line.eq_ignore_ascii_case("[playlist]");
            continue;
        }
        if !in_playlist {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let split = key
            .find(|ch: char| ch.is_ascii_digit())
            .unwrap_or(key.len());
        let (field, index) = key.split_at(split);
        if !matches!(field, "file" | "title" | "length") {
            continue;
        }
        let index: usize = index
            .parse()
            .ok()
            .filter(|index| (1..=MAX_ENTRIES).contains(index))
            .ok_or("A PLS entry number is invalid or exceeds 2000.")?;
        let row = rows.entry(index).or_default();
        match field {
            "file" if row.locator.is_none() => row.locator = Some(locator(value)?),
            "file" => return Err("The PLS file repeats a track number.".into()),
            "title" => row.title = tag(value),
            "length" => row.duration = duration(value, false),
            _ => (),
        }
    }
    Ok(rows
        .into_values()
        .filter(|row| row.locator.is_some())
        .collect())
}

fn normal(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn item_title(item: &LibraryItem) -> String {
    item.details
        .audio
        .as_ref()
        .and_then(|tags| tags.title.clone())
        .or_else(|| {
            item.path
                .file_stem()
                .and_then(|name| name.to_str())
                .and_then(tag)
        })
        .unwrap_or_else(|| tag(&item.name).unwrap_or_else(|| "Untitled track".into()))
}

fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(path) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{}", path).to_lowercase();
        }
        text.strip_prefix(r"\\?\")
            .unwrap_or(&text)
            .replace('/', "\\")
            .to_lowercase()
    }
    #[cfg(not(windows))]
    text.into_owned()
}

fn local_path(value: &str, directory: &Path) -> Option<PathBuf> {
    if value.starts_with("file:") {
        return url::Url::parse(value).ok()?.to_file_path().ok();
    }
    if value.contains("://") || value.starts_with("spotify:") {
        return None;
    }
    let path = Path::new(value);
    Some(if path.is_absolute() {
        path.to_owned()
    } else {
        directory.join(path)
    })
}

fn local(item: &LibraryItem) -> PlaylistEntry {
    PlaylistEntry::Local {
        id: item.id.clone(),
        title: item_title(item),
    }
}

fn match_rows(rows: Vec<Row>, directory: &Path, library: &[LibraryItem]) -> Vec<Match> {
    let mut by_id = HashMap::<&str, Vec<&LibraryItem>>::new();
    let mut by_path = HashMap::new();
    let mut by_title = HashMap::<String, Vec<&LibraryItem>>::new();
    for item in library.iter().filter(|item| item.kind == "audio") {
        by_id.entry(&item.id).or_default().push(item);
        if let Some(id) = item
            .details
            .content_id
            .as_deref()
            .filter(|id| *id != item.id)
        {
            by_id.entry(id).or_default().push(item);
        }
        by_path.insert(path_key(&item.path), item);
        by_title
            .entry(normal(&item_title(item)))
            .or_default()
            .push(item);
    }
    rows.into_iter()
        .map(|row| {
            let title = row
                .title
                .clone()
                .or_else(|| {
                    row.locator
                        .as_deref()
                        .and_then(|value| Path::new(value).file_name()?.to_str())
                        .and_then(tag)
                })
                .or_else(|| row.id.as_deref().and_then(tag))
                .unwrap_or_else(|| "Untitled track".into());
            let id_matches = row.id.as_deref().and_then(|id| by_id.get(id));
            let exact_path = row
                .locator
                .as_deref()
                .and_then(|value| local_path(value, directory))
                .and_then(|path| by_path.get(&path_key(&path)).copied());
            let exact = id_matches
                .filter(|items| items.len() == 1)
                .map(|items| items[0])
                .or(exact_path);
            let entry = if let Some(item) = exact {
                local(item)
            } else if let Some(url) = row
                .locator
                .as_ref()
                .filter(|url| crate::model::validate_url(url).is_ok())
            {
                PlaylistEntry::Remote {
                    url: url.clone(),
                    title: title.clone(),
                }
            } else {
                PlaylistEntry::Missing {
                    locator: row
                        .locator
                        .clone()
                        .or(row.id.clone())
                        .unwrap_or_else(|| title.clone()),
                    title: title.clone(),
                }
            };
            let mut candidates = Vec::new();
            if exact.is_none() {
                let suggestions = id_matches.or_else(|| by_title.get(&normal(&title)));
                let expected_artist = row.artist.as_deref().map(normal);
                let expected_version = row.version.as_deref().map(normal);
                for item in suggestions.into_iter().flatten() {
                    let tags = item.details.audio.as_ref();
                    let artist = tags
                        .and_then(|tags| tags.artist.clone())
                        .or_else(|| item.details.creator.clone());
                    let album = tags.and_then(|tags| tags.album.clone());
                    let version_matches = expected_version.as_ref().is_some_and(|version| {
                        normal(&format!("{} {}", item.name, album.as_deref().unwrap_or("")))
                            .contains(version)
                    });
                    let score = u8::from(expected_artist.as_ref().is_some_and(|expected| {
                        artist
                            .as_deref()
                            .is_some_and(|name| normal(name) == *expected)
                    })) * 4
                        + u8::from(row.duration.is_some_and(|seconds| {
                            item.duration
                                .is_some_and(|actual| (actual - seconds).abs() <= 2.0)
                        })) * 2
                        + u8::from(version_matches) * 3;
                    candidates.push((
                        score,
                        Candidate {
                            id: item.id.clone(),
                            title: item_title(item),
                            artist,
                            album,
                            duration: item.duration,
                            version_matches,
                        },
                    ));
                }
                candidates.sort_by(|left, right| {
                    right
                        .0
                        .cmp(&left.0)
                        .then_with(|| left.1.id.cmp(&right.1.id))
                });
            }
            Match {
                entry,
                artist: row.artist,
                duration: row.duration,
                version: row.version,
                candidates: candidates
                    .into_iter()
                    .take(8)
                    .map(|(_, item)| item)
                    .collect(),
            }
        })
        .collect()
}

fn open(path: &Path) -> Result<(PathBuf, String), String> {
    let path = crate::library::authorized_path(path)?;
    let file = fs::File::open(&path).map_err(|_| "Could not open the playlist.")?;
    if !file
        .metadata()
        .map_err(|_| "Could not inspect the playlist.")?
        .is_file()
    {
        return Err("Choose a regular local playlist file.".into());
    }
    let mut text = String::new();
    file.take((MAX_INPUT + 1) as u64)
        .read_to_string(&mut text)
        .map_err(|_| "The playlist must use UTF-8.")?;
    if text.len() > MAX_INPUT {
        return Err("Choose a playlist under 2 MiB.".into());
    }
    if text.starts_with('\u{feff}') {
        text = text.trim_start_matches('\u{feff}').into();
    }
    Ok((path, text))
}

#[tauri::command]
pub async fn audio_playlist_choose(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<Option<Preview>, String> {
    let result = tauri::async_runtime::spawn_blocking(move || {
        let selected = app
            .dialog()
            .file()
            .add_filter("Playlist", &["m3u", "m3u8", "pls", "csv"])
            .blocking_pick_file();
        let path = selected
            .map(|file| file.into_path().map_err(|_| "Choose a local playlist."))
            .transpose()?;
        path.as_deref().map(open).transpose()
    })
    .await
    .map_err(|_| "Could not open the selected playlist.")??;
    let Some((path, text)) = result else {
        return Ok(None);
    };
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .and_then(tag)
        .unwrap_or_else(|| "Imported playlist".into())
        .chars()
        .take(100)
        .collect::<String>();
    let directory = path
        .parent()
        .ok_or("The playlist has no local folder.")?
        .to_owned();
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let rows = match extension.as_str() {
        "m3u" | "m3u8" => parse_m3u(&text)?,
        "pls" => parse_pls(&text)?,
        "csv" => {
            let mut rows =
                delimited::parse(&text, delimited::delimiter(&text), MAX_ENTRIES + 1, 64)?
                    .into_iter();
            let columns = rows.next().ok_or("The CSV playlist is empty.")?;
            let rows: Vec<_> = rows.collect();
            if rows.is_empty() {
                return Err("The CSV playlist has a header and no tracks.".into());
            }
            let id = format!(
                "{:x}",
                Sha256::digest(format!("{}\n{text}", path.display()))
            );
            let preview = Preview::Csv {
                id: id.clone(),
                name: name.clone(),
                columns: columns.clone(),
                sample: rows
                    .iter()
                    .take(4)
                    .map(|row| {
                        row.iter()
                            .map(|field| field.chars().take(160).collect())
                            .collect()
                    })
                    .collect(),
                count: rows.len(),
            };
            *state.audio_playlist_browser.0.lock().unwrap() = Some(Table {
                id,
                name,
                directory,
                columns,
                rows,
            });
            return Ok(Some(preview));
        }
        _ => return Err("Choose an M3U, M3U8, PLS or CSV playlist.".into()),
    };
    if rows.is_empty() {
        return Err("The selected playlist has no track entries.".into());
    }
    Ok(Some(resolve(&state, name, &directory, rows)?))
}

#[tauri::command]
pub fn audio_playlist_map(
    state: State<Arc<AppState>>,
    id: String,
    mapping: Mapping,
) -> Result<Preview, String> {
    let browser = state.audio_playlist_browser.0.lock().unwrap();
    let table = browser
        .as_ref()
        .filter(|table| table.id == id)
        .ok_or("Choose the CSV playlist again.")?;
    let Mapping {
        locator: path_column,
        id: id_column,
        title,
        artist,
        duration: duration_column,
        version,
        milliseconds,
    } = mapping;
    if path_column.is_none() && id_column.is_none() && title.is_none()
        || [
            path_column,
            id_column,
            title,
            artist,
            duration_column,
            version,
        ]
        .into_iter()
        .flatten()
        .any(|index| index >= table.columns.len())
    {
        return Err("Map at least a title, identifier or path to an existing CSV column.".into());
    }
    let mut rows = Vec::new();
    for fields in &table.rows {
        let get = |index: Option<usize>| {
            index
                .and_then(|index| fields.get(index))
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
        };
        let raw_path = get(path_column).map(locator).transpose()?;
        let id = get(id_column).and_then(tag);
        let title = get(title).and_then(tag);
        if raw_path.is_none() && id.is_none() && title.is_none() {
            continue;
        }
        rows.push(Row {
            locator: raw_path,
            id,
            title,
            artist: get(artist).and_then(tag),
            duration: get(duration_column).and_then(|value| duration(value, milliseconds)),
            version: get(version).and_then(tag),
        });
    }
    if rows.is_empty() {
        return Err("The mapped columns contain no tracks.".into());
    }
    resolve(&state, table.name.clone(), &table.directory, rows)
}

fn resolve(
    state: &AppState,
    name: String,
    directory: &Path,
    rows: Vec<Row>,
) -> Result<Preview, String> {
    let guard = state.repository.lock().unwrap();
    let library = &guard.as_ref().ok_or("Open nabnab first.")?.data.library;
    Ok(Preview::List {
        name,
        rows: match_rows(rows, directory, library),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, album: &str) -> LibraryItem {
        serde_json::from_value(serde_json::json!({"id":id,"path":format!("C:/library/{id}.wav"),"name":format!("{id}.wav"),
            "kind":"audio","size":42,"duration":200,"addedAt":1,"collections":[],"tags":[],
            "details":{"audio":{"title":"La canción","artist":"Una artista","album":album}}})).unwrap()
    }

    #[test]
    fn m3u_and_pls_keep_order_and_do_not_treat_stream_manifests_as_tracks() {
        let rows = parse_m3u(
            "#EXTM3U\n#EXTINF:200,Una artista - La canción\nhttps://example.org/track\nmissing.mp3",
        )
        .unwrap();
        assert_eq!(rows[0].artist.as_deref(), Some("Una artista"));
        assert_eq!(rows[1].locator.as_deref(), Some("missing.mp3"));
        let rows = parse_pls("[playlist]\nFile2=two.wav\nFile1=one.wav\nTitle2=Two").unwrap();
        assert_eq!(rows[0].locator.as_deref(), Some("one.wav"));
        assert_eq!(rows[1].title.as_deref(), Some("Two"));
        assert!(parse_m3u("#EXTM3U\n#EXT-X-VERSION:4\nvideo.ts").is_err());
    }

    #[test]
    fn shared_title_only_suggests_versions_while_explicit_id_resolves() {
        let mut first = track("studio", "Studio recording");
        first.details.content_id = Some("exported-studio-id".into());
        let library = vec![first, track("concert", "Live recording")];
        let rows = vec![
            Row {
                title: Some("La canción".into()),
                artist: Some("Una artista".into()),
                duration: Some(200.0),
                version: Some("Live".into()),
                locator: Some("https://example.org/track".into()),
                ..Row::default()
            },
            Row {
                id: Some("exported-studio-id".into()),
                title: Some("La canción".into()),
                ..Row::default()
            },
        ];
        let result = match_rows(rows, Path::new("C:/exports"), &library);
        assert!(matches!(result[0].entry, PlaylistEntry::Remote { .. }));
        assert_eq!(
            result[0]
                .candidates
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["concert", "studio"]
        );
        assert!(matches!(&result[1].entry, PlaylistEntry::Local { id, .. } if id == "studio"));
        assert!(result[1].candidates.is_empty());
    }
}
