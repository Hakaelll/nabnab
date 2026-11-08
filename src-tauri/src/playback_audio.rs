use crate::{library, AppState};
use serde::Serialize;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::Arc,
};
use tauri::State;

#[derive(Clone, Copy)]
struct Atom {
    start: u64,
    data: u64,
    end: u64,
    kind: [u8; 4],
}

#[derive(Serialize)]
pub struct AudioTrack {
    id: u32,
    language: Option<String>,
    title: Option<String>,
    codec: Option<&'static str>,
    #[serde(skip)]
    type_offset: u64,
    #[serde(skip)]
    flags: (u64, u8),
}

const INVALID: &str = "This MP4 does not support choosing a local audio track.";

pub enum Patch {
    Hide(u64),
    Enable(u64, u8),
}

pub fn supported(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4") || ext.eq_ignore_ascii_case("m4v"))
}

fn read(file: &mut File, at: u64, into: &mut [u8]) -> Result<(), &'static str> {
    file.seek(SeekFrom::Start(at))
        .and_then(|_| file.read_exact(into))
        .map_err(|_| INVALID)
}

fn atoms(
    file: &mut File,
    mut start: u64,
    end: u64,
    budget: &mut usize,
) -> Result<Vec<Atom>, &'static str> {
    let mut children = Vec::new();
    while start < end {
        if *budget == 0 || end - start < 8 {
            return Err(INVALID);
        }
        *budget -= 1;
        let mut header = [0u8; 8];
        read(file, start, &mut header)?;
        let kind = [header[4], header[5], header[6], header[7]];
        let size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let (header, size) = match size {
            0 => (8, end - start),
            1 if end - start >= 16 => {
                let mut extended = [0u8; 8];
                read(file, start + 8, &mut extended)?;
                (16, u64::from_be_bytes(extended))
            }
            1 => return Err(INVALID),
            size => (8, u64::from(size)),
        };
        if size < header || size > end - start {
            return Err(INVALID);
        }
        children.push(Atom {
            start,
            data: start + header,
            end: start + size,
            kind,
        });
        start += size;
    }
    Ok(children)
}

fn unique(atoms: &[Atom], kind: &[u8; 4]) -> Result<Atom, &'static str> {
    let mut matches = atoms.iter().filter(|atom| &atom.kind == kind);
    let first = matches.next().ok_or(INVALID)?;
    if matches.next().is_some() {
        return Err(INVALID);
    }
    Ok(*first)
}

fn field<const N: usize>(
    file: &mut File,
    atom: Atom,
    offset: u64,
) -> Result<[u8; N], &'static str> {
    if offset + N as u64 > atom.end - atom.data {
        return Err(INVALID);
    }
    let mut bytes = [0; N];
    read(file, atom.data + offset, &mut bytes)?;
    Ok(bytes)
}

fn handler_name(file: &mut File, handler: Atom) -> Result<Option<String>, &'static str> {
    let count = (handler.end - handler.data).saturating_sub(24).min(256);
    if count == 0 {
        return Ok(None);
    }
    let mut name = vec![0; count as usize];
    read(file, handler.data + 24, &mut name)?;
    let text = String::from_utf8_lossy(&name);
    let title = text.split('\0').next().unwrap_or_default().trim();
    Ok(
        (!title.is_empty() && !title.eq_ignore_ascii_case("SoundHandler"))
            .then(|| {
                title
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .take(100)
                    .collect::<String>()
            })
            .filter(|title| !title.is_empty()),
    )
}

fn language(file: &mut File, media: &[Atom]) -> Result<Option<String>, &'static str> {
    let header = unique(media, b"mdhd")?;
    let offset = match field::<1>(file, header, 0)?[0] {
        0 => 20,
        1 => 32,
        _ => return Err(INVALID),
    };
    let packed = u16::from_be_bytes(field(file, header, offset)?);
    let bytes = [10, 5, 0].map(|shift| b'`' + ((packed >> shift) & 31) as u8);
    Ok(bytes
        .iter()
        .all(u8::is_ascii_lowercase)
        .then(|| String::from_utf8_lossy(&bytes).into_owned())
        .filter(|code| code != "und"))
}

fn audio_codec(
    file: &mut File,
    media: &[Atom],
    budget: &mut usize,
) -> Result<Option<&'static str>, &'static str> {
    let info = unique(media, b"minf")?;
    let info = atoms(file, info.data, info.end, budget)?;
    let samples = unique(&info, b"stbl")?;
    let samples = atoms(file, samples.data, samples.end, budget)?;
    let description = unique(&samples, b"stsd")?;
    if u32::from_be_bytes(field(file, description, 4)?) != 1 {
        return Ok(None);
    }
    let entries = atoms(file, description.data + 8, description.end, budget)?;
    Ok(match entries.as_slice() {
        [entry] => match &entry.kind {
            b"mp4a" => Some("mp4a.40.2"),
            b"ac-3" => Some("ac-3"),
            b"ec-3" => Some("ec-3"),
            b"Opus" => Some("opus"),
            _ => None,
        },
        _ => None,
    })
}

fn tracks(file: &mut File, size: u64) -> Result<Vec<AudioTrack>, &'static str> {
    let mut budget = 512;
    let top = atoms(file, 0, size, &mut budget)?;
    unique(&top, b"ftyp")?;
    let movie = unique(&top, b"moov")?;
    let children = atoms(file, movie.data, movie.end, &mut budget)?;
    if top.iter().any(|atom| &atom.kind == b"moof")
        || children.iter().any(|atom| &atom.kind == b"mvex")
    {
        return Ok(Vec::new());
    }
    let mut audio: Vec<AudioTrack> = Vec::new();
    let mut video = false;
    for track in children.into_iter().filter(|atom| &atom.kind == b"trak") {
        let boxes = atoms(file, track.data, track.end, &mut budget)?;
        let media = unique(&boxes, b"mdia")?;
        let media = atoms(file, media.data, media.end, &mut budget)?;
        let handler = unique(&media, b"hdlr")?;
        let kind = field::<4>(file, handler, 8)?;
        video |= &kind == b"vide";
        if &kind != b"soun" {
            continue;
        }
        let header = unique(&boxes, b"tkhd")?;
        let offset = match field::<1>(file, header, 0)?[0] {
            0 => 12,
            1 => 20,
            _ => return Err(INVALID),
        };
        let id = u32::from_be_bytes(field(file, header, offset)?);
        if id == 0 || audio.iter().any(|track| track.id == id) {
            return Err(INVALID);
        }
        audio.push(AudioTrack {
            id,
            title: handler_name(file, handler)?,
            language: language(file, &media)?,
            codec: audio_codec(file, &media, &mut budget)?,
            type_offset: track.start + 4,
            flags: (header.data + 3, field::<1>(file, header, 3)?[0]),
        });
    }
    if video && audio.len() > 1 {
        Ok(audio)
    } else {
        Ok(Vec::new())
    }
}

pub fn audio_patch(file: &mut File, size: u64, selected: u32) -> Result<Vec<Patch>, &'static str> {
    let tracks = tracks(file, size)?;
    let selected_track = tracks
        .iter()
        .find(|track| track.id == selected && track.codec.is_some())
        .ok_or(INVALID)?;
    let (flags, value) = selected_track.flags;
    Ok(tracks
        .into_iter()
        .filter(|track| track.id != selected)
        .map(|track| Patch::Hide(track.type_offset))
        .chain(std::iter::once(Patch::Enable(flags, value | 3)))
        .collect())
}

pub fn apply_audio_patch(data: &mut [u8], start: u64, patches: Vec<Patch>) {
    for patch in &patches {
        let (offset, replacement): (u64, &[u8]) = match patch {
            // Keeping the box length preserves every media sample offset.
            Patch::Hide(offset) => (*offset, b"free"),
            Patch::Enable(offset, flags) => (*offset, std::slice::from_ref(flags)),
        };
        for (index, value) in replacement.iter().enumerate() {
            if let Some(position) = (offset + index as u64).checked_sub(start) {
                if let Ok(position) = usize::try_from(position) {
                    if let Some(byte) = data.get_mut(position) {
                        *byte = *value;
                    }
                }
            }
        }
    }
}

#[tauri::command]
pub async fn playback_audio_tracks(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<Vec<AudioTrack>, String> {
    let path = {
        let guard = state.repository.lock().unwrap();
        guard
            .as_ref()
            .and_then(|repo| {
                repo.data
                    .library
                    .iter()
                    .find(|item| item.id == id && item.kind == "video")
            })
            .map(|item| item.path.clone())
            .ok_or("Video not found.")?
    };
    if !supported(&path) {
        return Ok(Vec::new());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let path = library::authorized_path(&path)?;
        let mut file = File::open(path).map_err(|_| INVALID)?;
        let size = file.metadata().map_err(|_| INVALID)?.len();
        Ok(tracks(&mut file, size).unwrap_or_default())
    })
    .await
    .map_err(|_| INVALID)?
}
