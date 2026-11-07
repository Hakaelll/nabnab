//! Explicit local capture. Only the desktop UI may call the controller; queue jobs only finalize IDs.
use crate::{engine, model::Progress};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicI64, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::watch,
};
mod audio;
mod finalize;
mod native;
mod wgc;

const VIDEO_STALL_TIMEOUT: Duration = Duration::from_secs(5);
pub use finalize::finalize;
const MAX_PIXELS: u64 = 24_000_000;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Record,
    Replay,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    Standard,
    High,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub mode: Mode,
    pub fps: u32,
    pub quality: Quality,
    pub duration_seconds: u32,
    pub disk_limit_mi_b: u32,
    pub replay_seconds: u32,
    pub system_audio_id: Option<String>,
    pub microphone_id: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioSource {
    pub id: String,
    pub label: String,
    pub kind: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sources {
    pub available: bool,
    pub reason: Option<String>,
    pub sources: Vec<Source>,
    pub audio: Vec<AudioSource>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub preview_id: String,
    pub png: String,
    pub width: u32,
    pub height: u32,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyCapture {
    pub id: String,
    pub kind: String,
    pub duration_ms: u64,
    pub interrupted: bool,
    pub reason: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub session_id: Option<String>,
    pub phase: String,
    pub elapsed_ms: u64,
    pub bytes: u64,
    pub replay_seconds: Option<u32>,
    pub available_seconds: u32,
    pub source_label: Option<String>,
    pub system_audio: bool,
    pub microphone: bool,
    pub disk_limit_mi_b: Option<u32>,
    pub reason: Option<String>,
    pub ready_capture: Option<ReadyCapture>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            session_id: None,
            phase: "idle".into(),
            elapsed_ms: 0,
            bytes: 0,
            replay_seconds: None,
            available_seconds: 0,
            source_label: None,
            system_audio: false,
            microphone: false,
            disk_limit_mi_b: None,
            reason: None,
            ready_capture: None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    pub version: u32,
    pub id: String,
    pub kind: String,
    pub source_label: String,
    pub duration_ms: u64,
    pub interrupted: bool,
    pub reason: Option<String>,
    pub phase: String,
    pub video_start_us: i64,
    pub videos: Vec<String>,
    pub audio: Vec<audio::Track>,
    pub width: u32,
    pub height: u32,
    pub destination: PathBuf,
}
#[derive(Clone)]
struct Choice {
    target: native::Target,
    region: Option<Region>,
    expires: Instant,
}
struct Active {
    id: String,
    stop: watch::Sender<bool>,
}
struct Shared {
    root: PathBuf,
    engines: PathBuf,
    state: Mutex<State>,
    sources: Mutex<HashMap<String, native::Target>>,
    audio_sources: Mutex<HashMap<String, audio::Device>>,
    previews: Mutex<HashMap<String, Choice>>,
    active: Mutex<Option<Active>>,
    files: Arc<tokio::sync::Mutex<()>>,
}
#[derive(Clone)]
pub struct Controller(Arc<Shared>);

fn id(prefix: &str) -> String {
    format!(
        "{prefix}-{:x}-{:x}-{:x}",
        std::process::id(),
        epoch_us(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}
fn epoch_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(i64::MAX as u128) as i64
}
fn valid_id(id: &str) -> bool {
    id.len() >= 8
        && id.len() <= 100
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b'-' || b == b'c' || b == b's' || b == b'p')
}
fn regular(path: &Path) -> Result<u64, String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err("Capture links are not allowed.".into());
        }
    }
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("Capture must be a regular owned file.".into());
    }
    Ok(meta.len())
}
fn directory(path: &Path) -> Result<(), String> {
    if !path.exists() {
        fs::create_dir_all(path).map_err(|e| e.to_string())?;
    }
    let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err("Capture directories cannot be links.".into());
        }
    }
    if !m.is_dir() || m.file_type().is_symlink() {
        return Err("Capture directory is unavailable.".into());
    }
    Ok(())
}
fn capture_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Invalid capture ID.".into());
    }
    let base = root.join("captures");
    directory(&base)?;
    let dir = base.join(id);
    if dir.exists() {
        directory(&dir)?;
    }
    Ok(dir)
}
fn write_journal(dir: &Path, journal: &Journal) -> Result<(), String> {
    let next = dir.join("journal.next");
    if next.exists() {
        regular(&next)?;
        fs::remove_file(&next).map_err(|e| e.to_string())?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&next)
        .map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(journal).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    let destination = dir.join("journal.json");
    if destination.exists() {
        regular(&destination)?;
    }
    fs::rename(&next, &destination).map_err(|e| e.to_string())
}
pub fn readjournal(root: &Path, id: &str) -> Result<Journal, String> {
    let dir = capture_dir(root, id)?;
    let path = dir.join("journal.json");
    if regular(&path)? > 2 * 1024 * 1024 {
        return Err("Capture journal is too large.".into());
    }
    let journal: Journal = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if journal.version != 1
        || journal.id != id
        || !matches!(journal.kind.as_str(), "image" | "recording")
    {
        return Err("Capture journal is incompatible.".into());
    }
    if journal.videos.len() > 7202
        || journal.audio.len() > 2
        || journal.duration_ms > 14_402_000
        || journal.width < 2
        || journal.height < 2
        || journal.width as u64 * journal.height as u64 > MAX_PIXELS
        || !(0..=i64::MAX / 4).contains(&journal.video_start_us)
    {
        return Err("Capture journal exceeds its segment bounds.".into());
    }
    for name in &journal.videos {
        owned_name(name)?;
        if journal.kind == "recording"
            && (!name.starts_with("video-")
                || !name.ends_with(".mkv")
                || audio::segment_number(name).is_none())
        {
            return Err("Invalid capture video segment.".into());
        }
    }
    for track in &journal.audio {
        if !matches!(track.kind.as_str(), "system" | "microphone")
            || track.files.len() > 7202
            || !(0..=i64::MAX / 4).contains(&track.start_us)
        {
            return Err("Invalid capture audio journal.".into());
        }
        for name in &track.files {
            owned_name(name)?;
            if !name.starts_with(&format!("{}-", track.kind))
                || !name.ends_with(".wav")
                || audio::segment_number(name).is_none()
            {
                return Err("Invalid capture audio segment.".into());
            }
        }
    }
    Ok(journal)
}
fn owned_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 100
        || name.contains(['/', '\\', ':'])
        || name.starts_with('.')
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    {
        return Err("Invalid owned capture filename.".into());
    }
    Ok(())
}
fn ready(j: &Journal) -> ReadyCapture {
    ReadyCapture {
        id: j.id.clone(),
        kind: j.kind.clone(),
        duration_ms: j.duration_ms,
        interrupted: j.interrupted,
        reason: j.reason.clone(),
    }
}
pub fn recover(root: &Path) -> Result<Vec<ReadyCapture>, String> {
    let base = root.join("captures");
    if !base.exists() {
        return Ok(vec![]);
    }
    directory(&base)?;
    let mut result = Vec::new();
    for entry in fs::read_dir(base).map_err(|e| e.to_string())?.take(100_000) {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(mut j) = readjournal(root, &name) else {
            continue;
        };
        let _lease = match CaptureLease::open(&entry.path().join("active.lock"), false) {
            Ok(l) => l,
            Err(_) => continue,
        };
        if j.phase == "published" || j.phase == "discarded" {
            continue;
        }
        if j.phase == "recording" {
            j.interrupted = true;
            j.reason = Some("Capture was interrupted. Recover the available segments.".into());
            j.phase = "interrupted".into();
            write_journal(&entry.path(), &j)?;
        }
        if !j.videos.is_empty() {
            result.push(ready(&j));
            if result.len() >= 1000 {
                break;
            }
        }
    }
    Ok(result)
}
pub fn mark_published(root: &Path, id: &str) -> Result<(), String> {
    let mut j = readjournal(root, id)?;
    j.phase = "published".into();
    let dir = capture_dir(root, id)?;
    write_journal(&dir, &j)?;
    for name in j
        .videos
        .iter()
        .chain(j.audio.iter().flat_map(|a| a.files.iter()))
    {
        let path = dir.join(name);
        if path.exists() {
            regular(&path)?;
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
pub fn validate_options(o: &Options) -> Result<(), String> {
    if !matches!(o.fps, 15 | 30)
        || !(1..=14400).contains(&o.duration_seconds)
        || !(64..=4096).contains(&o.disk_limit_mi_b)
        || !matches!(o.replay_seconds, 30 | 60 | 120)
    {
        return Err(
            "Choose 15/30 fps, 1–14400 seconds, 64–4096 MiB and a 30/60/120 second replay window."
                .into(),
        );
    }
    Ok(())
}
impl Controller {
    pub fn new(root: PathBuf, engines: PathBuf) -> Result<Self, String> {
        crate::sessions::secure_module_directory(&root, "captures")?;
        Ok(Self(Arc::new(Shared {
            root,
            engines,
            state: Mutex::new(State::default()),
            sources: Mutex::new(HashMap::new()),
            audio_sources: Mutex::new(HashMap::new()),
            previews: Mutex::new(HashMap::new()),
            active: Mutex::new(None),
            files: Arc::new(tokio::sync::Mutex::new(())),
        })))
    }
    pub async fn sources(&self) -> Result<Sources, String> {
        let engines = self.0.engines.clone();
        let (_keep_cancel, cancel) = watch::channel(false);
        let devices = engine::tool(
            &engines,
            "ffmpeg.exe",
            vec!["-hide_banner".into(), "-devices".into()],
            cancel,
        )
        .await;
        // FFmpeg writes its device table to stderr; its gdigrab support is checked again by an explicit preview.
        if !self.0.engines.join("ffmpeg.exe").is_file() || devices.is_err() {
            return Ok(Sources {
                available: false,
                reason: Some("The managed FFmpeg capture engine is unavailable.".into()),
                sources: vec![],
                audio: vec![],
            });
        }
        let targets = tokio::task::spawn_blocking(native::enumerate)
            .await
            .map_err(|e| e.to_string())??;
        let mut sources = Vec::new();
        let mut map = HashMap::new();
        for target in targets {
            let key = id("s");
            sources.push(Source {
                id: key.clone(),
                label: target.label.clone(),
                kind: target.kind().into(),
                width: target.width,
                height: target.height,
            });
            map.insert(key, target);
        }
        *self.0.sources.lock().unwrap() = map;
        let devices = tokio::task::spawn_blocking(audio::devices)
            .await
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let mut audio = Vec::new();
        let mut map = HashMap::new();
        for device in devices {
            let key = id("s");
            audio.push(AudioSource {
                id: key.clone(),
                label: device.label.clone(),
                kind: device.kind.clone(),
            });
            map.insert(key, device);
        }
        *self.0.audio_sources.lock().unwrap() = map;
        Ok(Sources {
            available: true,
            reason: None,
            sources,
            audio,
        })
    }
    pub async fn preview(
        &self,
        source_id: &str,
        region: Option<Region>,
        cancel: watch::Receiver<bool>,
    ) -> Result<Preview, String> {
        let target = self
            .0
            .sources
            .lock()
            .unwrap()
            .get(source_id)
            .cloned()
            .ok_or("Refresh and select an available capture source.")?
            .lease()?;
        target.validate()?;
        let (width, height) = target.dimensions(region)?;
        let key = id("p");
        let dir = capture_dir(&self.0.root, &key)?;
        directory(&dir)?;
        let path = dir.join("preview.png");
        let mut args = vec![
            "-nostdin".into(),
            "-hide_banner".into(),
            "-v".into(),
            "error".into(),
            "-n".into(),
        ];
        let raw = still_input(&target, region, &dir, &mut args, cancel.clone()).await?;
        args.extend([
            "-frames:v".into(),
            "1".into(),
            "-vf".into(),
            "scale=w='min(960,iw)':h=-1".into(),
            "-f".into(),
            "image2".into(),
            path.to_string_lossy().into_owned(),
        ]);
        let result = engine::tool(&self.0.engines, "ffmpeg.exe", args, cancel).await;
        if let Err(e) = result {
            let _ = fs::remove_dir_all(&dir);
            return Err(e);
        }
        target.validate()?;
        if regular(&path)? > 8 * 1024 * 1024 {
            return Err("Capture preview exceeded its limit.".into());
        }
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        if let Some(raw) = raw {
            regular(&raw)?;
            fs::remove_file(raw).map_err(|e| e.to_string())?;
        }
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&dir);
        let mut previews = self.0.previews.lock().unwrap();
        previews.retain(|_, p| p.expires > Instant::now());
        if previews.len() > 32 {
            previews.clear();
        }
        let warning = if target.hwnd != 0 {
            "Windows Graphics Capture targets only this window. Protected content can be blank; verify this preview."
        } else {
            "GDI captures visible desktop content. Protected surfaces can appear blank; verify this preview."
        };
        previews.insert(
            key.clone(),
            Choice {
                target,
                region,
                expires: Instant::now() + Duration::from_secs(120),
            },
        );
        Ok(Preview {
            preview_id: key,
            png: format!("data:image/png;base64,{}", base64(&bytes)),
            width,
            height,
            warnings: vec![warning.into()],
        })
    }
    fn choice(&self, key: &str) -> Result<Choice, String> {
        let choice = self
            .0
            .previews
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or("Preview the selected source before capturing.")?;
        if choice.expires < Instant::now() {
            return Err("The capture preview expired. Preview again.".into());
        }
        choice.target.validate()?;
        Ok(choice)
    }
    pub async fn start(
        &self,
        preview_id: &str,
        options: Options,
        destination: PathBuf,
    ) -> Result<State, String> {
        validate_options(&options)?;
        let choice = self.choice(preview_id)?;
        let destination = destination_directory(&destination)?;
        let (width, height) = choice.target.dimensions(choice.region)?;
        if options.disk_limit_mi_b as u64 * 1024 * 1024
            < width as u64 * height as u64 * 4 + 16 * 1024 * 1024
        {
            return Err("Increase the capture disk budget for this source resolution, or select a smaller region.".into());
        }
        let mut selected_audio = Vec::new();
        {
            let devices = self.0.audio_sources.lock().unwrap();
            for (key, kind) in [
                (&options.system_audio_id, "system"),
                (&options.microphone_id, "microphone"),
            ] {
                if let Some(key) = key {
                    let device = devices
                        .get(key)
                        .filter(|d| d.kind == kind)
                        .cloned()
                        .ok_or("Select an available audio device.")?;
                    selected_audio.push(device);
                }
            }
        }
        let key = id("c");
        let (stop, rx) = watch::channel(false);
        {
            let mut active = self.0.active.lock().unwrap();
            if active.is_some() {
                return Err("Stop the current capture or replay buffer first.".into());
            }
            *active = Some(Active {
                id: key.clone(),
                stop,
            });
        }
        let state = State {
            session_id: Some(key.clone()),
            phase: "starting".into(),
            source_label: Some(choice.target.label.clone()),
            system_audio: options.system_audio_id.is_some(),
            microphone: options.microphone_id.is_some(),
            disk_limit_mi_b: Some(options.disk_limit_mi_b),
            replay_seconds: (options.mode == Mode::Replay).then_some(options.replay_seconds),
            ..State::default()
        };
        *self.0.state.lock().unwrap() = state.clone();
        let controller = self.clone();
        tokio::spawn(async move {
            controller
                .run(key, choice, options, selected_audio, rx, destination)
                .await;
        });
        Ok(state)
    }
    pub fn status(&self) -> State {
        self.0.state.lock().unwrap().clone()
    }
    pub async fn stop(&self, session_id: &str) -> Result<State, String> {
        {
            let active = self.0.active.lock().unwrap();
            if let Some(a) = active.as_ref() {
                if a.id != session_id {
                    return Err("The capture session changed.".into());
                }
                let _ = a.stop.send(true);
            } else {
                return Ok(self.status());
            }
        }
        for _ in 0..100 {
            if self.0.active.lock().unwrap().is_none() {
                return Ok(self.status());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("Capture is still stopping. Its recoverable files remain available.".into())
    }
    pub async fn shutdown(&self) {
        let key = self.0.active.lock().unwrap().as_ref().map(|a| a.id.clone());
        if let Some(key) = key {
            let _ = self.stop(&key).await;
        }
    }
    pub async fn screenshot(
        &self,
        preview_id: &str,
        destination: PathBuf,
    ) -> Result<ReadyCapture, String> {
        let destination = destination_directory(&destination)?;
        let choice = self.choice(preview_id)?;
        let (width, height) = choice.target.dimensions(choice.region)?;
        let key = id("c");
        let dir = capture_dir(&self.0.root, &key)?;
        directory(&dir)?;
        let path = dir.join("capture.png");
        let mut args = vec!["-nostdin".into(), "-v".into(), "error".into(), "-n".into()];
        let (_keep_cancel, cancel) = watch::channel(false);
        let raw = still_input(
            &choice.target,
            choice.region,
            &dir,
            &mut args,
            cancel.clone(),
        )
        .await?;
        args.extend([
            "-frames:v".into(),
            "1".into(),
            path.to_string_lossy().into_owned(),
        ]);
        engine::tool(&self.0.engines, "ffmpeg.exe", args, cancel).await?;
        if let Some(raw) = raw {
            regular(&raw)?;
            fs::remove_file(raw).map_err(|e| e.to_string())?;
        }
        choice.target.validate()?;
        regular(&path)?;
        let j = Journal {
            version: 1,
            id: key,
            kind: "image".into(),
            source_label: choice.target.label,
            duration_ms: 0,
            interrupted: false,
            reason: None,
            phase: "ready".into(),
            video_start_us: 0,
            videos: vec!["capture.png".into()],
            audio: vec![],
            width,
            height,
            destination,
        };
        write_journal(&dir, &j)?;
        Ok(ready(&j))
    }
    pub async fn save_replay(&self, session_id: &str) -> Result<ReadyCapture, String> {
        let files = self.0.files.clone().lock_owned().await;
        let state = self.status();
        if state.session_id.as_deref() != Some(session_id) || state.phase != "replay" {
            return Err("Activate a replay buffer and wait for a complete segment.".into());
        }
        let mut journal = readjournal(&self.0.root, session_id)?;
        if journal.videos.is_empty() {
            return Err("The replay buffer has no complete segment yet.".into());
        }
        let source = capture_dir(&self.0.root, session_id)?;
        journal.id = id("c");
        journal.phase = "ready".into();
        let dest = capture_dir(&self.0.root, &journal.id)?;
        tokio::task::spawn_blocking(move || {
            let mut inputs = Vec::new();
            for name in journal
                .videos
                .iter()
                .chain(journal.audio.iter().flat_map(|a| a.files.iter()))
            {
                let from = source.join(name);
                regular(&from)?;
                let mut options = OpenOptions::new();
                options.read(true);
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    use windows_sys::Win32::Storage::FileSystem::{
                        FILE_SHARE_DELETE, FILE_SHARE_READ,
                    };
                    options.share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE);
                }
                let input = options.open(from).map_err(|e| e.to_string())?;
                inputs.push((name, input));
            }
            // Read handles keep completed segments available while the live ring prunes them.
            drop(files);
            directory(&dest)?;
            for (name, mut input) in inputs {
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(dest.join(name))
                    .map_err(|e| e.to_string())?;
                std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
                output.sync_all().map_err(|e| e.to_string())?;
            }
            write_journal(&dest, &journal)?;
            Ok(ready(&journal))
        })
        .await
        .map_err(|error| format!("Could not save the replay snapshot: {error}"))?
    }

    async fn run(
        &self,
        key: String,
        choice: Choice,
        options: Options,
        devices: Vec<audio::Device>,
        stop: watch::Receiver<bool>,
        destination: PathBuf,
    ) {
        let result = self
            .run_capture(&key, &choice, &options, devices, stop, destination)
            .await;
        if let Err(error) = result {
            let mut state = self.0.state.lock().unwrap();
            state.phase = "error".into();
            state.reason = Some(error);
        }
        *self.0.active.lock().unwrap() = None;
    }
    async fn run_capture(
        &self,
        key: &str,
        choice: &Choice,
        options: &Options,
        devices: Vec<audio::Device>,
        mut stop: watch::Receiver<bool>,
        destination: PathBuf,
    ) -> Result<(), String> {
        let dir = capture_dir(&self.0.root, key)?;
        directory(&dir)?;
        let _capture_lease = CaptureLease::open(&dir.join("active.lock"), true)?;
        let (width, height) = choice.target.dimensions(choice.region)?;
        let mut journal = Journal {
            version: 1,
            id: key.into(),
            kind: "recording".into(),
            source_label: choice.target.label.clone(),
            duration_ms: 0,
            interrupted: false,
            reason: None,
            phase: "recording".into(),
            video_start_us: 0,
            videos: vec![],
            audio: vec![],
            width,
            height,
            destination,
        };
        write_journal(&dir, &journal)?;
        let mut recorders = Vec::new();
        let (audio_stop, audio_rx) = watch::channel(false);
        let _stop_guard = AudioStop(audio_stop.clone());
        for device in devices {
            let kind = device.kind.clone();
            recorders.push((
                kind,
                audio::Recorder::start(device, dir.clone(), audio_rx.clone()),
            ));
        }
        let mut args = vec![
            "-hide_banner".into(),
            "-loglevel".into(),
            "info".into(),
            "-n".into(),
            "-copyts".into(),
        ];
        let window_capture = choice.target.hwnd != 0;
        if window_capture {
            args.push("-nostdin".into());
            args.extend(raw_input(width, height, options.fps, "pipe:0".into()));
        } else {
            args.extend(choice.target.input_args(choice.region, options.fps)?);
        }
        args.extend([
            "-an".into(),
            "-vf".into(),
            "showinfo,setpts=PTS-STARTPTS,pad=ceil(iw/2)*2:ceil(ih/2)*2".into(),
            "-c:v".into(),
            "libopenh264".into(),
            "-rc_mode".into(),
            "bitrate".into(),
            "-b:v".into(),
            if options.quality == Quality::High {
                "8000k"
            } else {
                "4000k"
            }
            .into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-g".into(),
            (options.fps * 2).to_string(),
            "-force_key_frames".into(),
            "expr:gte(t,n_forced*2)".into(),
            "-fps_mode".into(),
            "cfr".into(),
            "-maxrate".into(),
            if options.quality == Quality::High {
                "16000k"
            } else {
                "8000k"
            }
            .into(),
            "-bufsize".into(),
            "16000k".into(),
            "-f".into(),
            "segment".into(),
            "-segment_time".into(),
            "2".into(),
            "-reset_timestamps".into(),
            "1".into(),
            "-segment_format".into(),
            "matroska".into(),
            dir.join("video-%08d.mkv").to_string_lossy().into_owned(),
        ]);
        let mut command = Command::new(self.0.engines.join("ffmpeg.exe"));
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let spawn = command.spawn();
        let mut child = match spawn {
            Ok(child) => child,
            Err(e) => {
                let _ = audio_stop.send(true);
                for (_, r) in &mut recorders {
                    let _ = r.finish();
                }
                return Err(format!("Could not start screen recording: {e}"));
            }
        };
        #[cfg(windows)]
        let _tree = engine::ProcessTree::attach(&child)?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Capture errors are unavailable.")?;
        let first_pts = Arc::new(AtomicI64::new(0));
        let pts = first_pts.clone();
        let last_frame = Arc::new(AtomicU64::new(0));
        let frame_tick = last_frame.clone();
        let clock_fault = Arc::new(AtomicU64::new(0));
        let fault = clock_fault.clone();
        let frame_clock = Instant::now();
        let mut graphics = window_capture
            .then(|| wgc::Stream::start(choice.target.clone(), options.fps))
            .transpose()?;
        let frame_pump = if let Some(stream) = &mut graphics {
            let (_unused, empty) = tokio::sync::mpsc::channel(1);
            let mut frames = std::mem::replace(&mut stream.frames, empty);
            let mut stdin = child
                .stdin
                .take()
                .ok_or("The video encoder input is unavailable.")?;
            let first = first_pts.clone();
            let tick = last_frame.clone();
            let failure = stream.error.clone();
            let mut stopped = stop.clone();
            Some(tokio::spawn(async move {
                loop {
                    let frame = tokio::select! {_=stopped.changed()=>break,frame=frames.recv()=>match frame{Some(frame)=>frame,None=>break}};
                    if first.load(Ordering::Relaxed) == 0 {
                        first.store(frame.epoch_us, Ordering::Relaxed);
                    }
                    // Finish each complete raw frame before closing stdin; cancellation midway corrupts the final packet.
                    let result = match tokio::time::timeout(
                        VIDEO_STALL_TIMEOUT,
                        stdin.write_all(&frame.bytes),
                    )
                    .await
                    {
                        Ok(result) => result,
                        Err(_) => {
                            *failure.lock().unwrap()=Some("The video encoder stalled while accepting a window frame. Recover the available recording.".into());
                            break;
                        }
                    };
                    if let Err(error) = result {
                        *failure.lock().unwrap() = Some(format!(
                            "The video encoder stopped accepting capture frames: {error}"
                        ));
                        break;
                    }
                    tick.store(
                        (frame_clock.elapsed().as_millis() as u64).max(1),
                        Ordering::Relaxed,
                    );
                }
                drop(frames);
                let _ = stdin.shutdown().await;
            }))
        } else {
            None
        };
        let mut previous_pts = 0;
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            let mut tail = Vec::new();
            while let Ok(Some(line)) = reader.next_line().await {
                if let Some(value) = line
                    .split(" pts:")
                    .nth(1)
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.parse::<i64>().ok())
                {
                    if value > 1_000_000_000_000_000 {
                        if previous_pts != 0
                            && (value < previous_pts || value - previous_pts > 2_000_000)
                        {
                            fault.store(1, Ordering::Relaxed);
                        }
                        previous_pts = value;
                        frame_tick.store(
                            (frame_clock.elapsed().as_millis() as u64).max(1),
                            Ordering::Relaxed,
                        );
                        if pts.load(Ordering::Relaxed) == 0 {
                            pts.store(value, Ordering::Relaxed);
                        }
                    }
                }
                if !line.contains("showinfo") {
                    if line.len() < 4096 {
                        tail.push(line);
                        if tail.len() > 5 {
                            tail.remove(0);
                        }
                    }
                }
            }
            tail.join("\n")
        });
        let started = Instant::now();
        let mut media_started: Option<Instant> = None;
        let limit = options.disk_limit_mi_b as u64 * 1024 * 1024;
        let reserve = width as u64 * height as u64 * 4 + 8 * 1024 * 1024;
        let mut interrupted = false;
        let mut reason = None;
        let mut unexpected_exit = false;
        let audio_logs = Arc::new(
            recorders
                .iter()
                .map(|(kind, recorder)| (kind.clone(), recorder.log.clone()))
                .collect::<Vec<_>>(),
        );
        loop {
            if *stop.borrow() {
                break;
            }
            if let Some(stream) = &graphics {
                if let Some(error) = stream.error.lock().unwrap().clone() {
                    interrupted = true;
                    reason = Some(error);
                    break;
                }
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                interrupted = true;
                unexpected_exit = true;
                reason = Some(format!(
                    "The screen capture process ended unexpectedly ({status})."
                ));
                break;
            }
            if let Err(e) = choice.target.validate() {
                interrupted = true;
                reason = Some(e);
                break;
            }
            for (_, r) in &recorders {
                if let Some(error) = r.log.lock().unwrap().error.clone() {
                    interrupted = true;
                    reason = Some(error);
                    break;
                }
            }
            if interrupted {
                break;
            }
            let first = first_pts.load(Ordering::Relaxed);
            let last = last_frame.load(Ordering::Relaxed);
            if clock_fault.load(Ordering::Relaxed) != 0
                || (last > 0
                    && (frame_clock.elapsed().as_millis() as u64).saturating_sub(last)
                        > VIDEO_STALL_TIMEOUT.as_millis() as u64)
            {
                interrupted = true;
                reason=Some("Video frames stopped or their clock was interrupted. Recover the available recording.".into());
                break;
            }
            let ready_audio = recorders
                .iter()
                .all(|(_, r)| r.log.lock().unwrap().initialized);
            if first > 0 && ready_audio && last > 0 {
                media_started.get_or_insert_with(Instant::now);
            }
            if first == 0 && started.elapsed() > Duration::from_secs(8) {
                interrupted = true;
                reason =
                    Some("The source did not produce a video frame. Check its preview.".into());
                break;
            }
            let files_lock = self.0.files.clone().lock_owned().await;
            let checkpoint_dir = dir.clone();
            let logs = audio_logs.clone();
            let mode = options.mode;
            let replay_seconds = options.replay_seconds;
            let elapsed = media_started
                .map(|start| start.elapsed().as_millis() as u64)
                .unwrap_or(0);
            let (next_journal, bytes) = tokio::task::spawn_blocking(move || {
                let _files = files_lock;
                let dir = checkpoint_dir;
                let mut journal = journal;
                let mut files = video_files(&dir)?;
                // The final file is still open. It is recoverable, but never included in a live replay snapshot.
                let active_file = files.pop();
                if first > 0 {
                    journal.video_start_us = first;
                }
                if mode == Mode::Replay && first > 0 {
                    let keep = (replay_seconds as usize / 2).max(1);
                    while files.len() > keep {
                        let name = files.remove(0);
                        regular(&dir.join(&name))?;
                        fs::remove_file(dir.join(name)).map_err(|e| e.to_string())?;
                    }
                    let earliest = files
                        .first()
                        .and_then(|n| audio::segment_number(n))
                        .unwrap_or(0) as i64
                        * 2_000_000
                        + first;
                    for (_, r) in logs.iter() {
                        audio::trim(r, &dir, earliest)?;
                    }
                    journal.video_start_us = earliest;
                }
                journal.videos = files;
                if mode == Mode::Record {
                    if let Some(name) = active_file {
                        journal.videos.push(name);
                    }
                }
                journal.audio = logs
                    .iter()
                    .map(|(kind, r)| audio::snapshot(r, kind, mode == Mode::Record))
                    .collect();
                journal.duration_ms = if mode == Mode::Replay {
                    journal.videos.len() as u64 * 2000
                } else {
                    elapsed
                };
                write_journal(&dir, &journal)?;
                let bytes = directory_bytes(&dir)?;
                Ok::<_, String>((journal, bytes))
            })
            .await
            .map_err(|error| format!("Could not save capture progress: {error}"))??;
            journal = next_journal;
            {
                let mut s = self.0.state.lock().unwrap();
                s.elapsed_ms = media_started
                    .map(|start| start.elapsed().as_millis() as u64)
                    .unwrap_or(0);
                s.bytes = bytes;
                s.available_seconds = (journal.videos.len() as u32 * 2).min(options.replay_seconds);
                if media_started.is_some() {
                    s.phase = if options.mode == Mode::Replay {
                        "replay"
                    } else {
                        "recording"
                    }
                    .into();
                }
            }
            if bytes >= limit.saturating_sub(reserve) {
                reason = Some("The configured disk limit was reached.".into());
                break;
            }
            if media_started
                .is_some_and(|start| start.elapsed().as_secs() >= options.duration_seconds as u64)
            {
                reason = Some("The configured duration limit was reached.".into());
                break;
            }
            tokio::select! {_=tokio::time::sleep(Duration::from_millis(100))=>{},_=stop.changed()=>{}}
        }
        {
            let mut s = self.0.state.lock().unwrap();
            s.phase = "stopping".into();
        }
        let _ = audio_stop.send(true);
        if let Some(stream) = &graphics {
            stream.stop();
        }
        if let Some(pump) = frame_pump {
            if tokio::time::timeout(Duration::from_secs(4), pump)
                .await
                .is_err()
            {
                interrupted = true;
                reason=Some("The window capture encoder could not stop cleanly. Recover the available segments.".into());
            }
        }
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(b"q\n").await;
            let _ = stdin.shutdown().await;
        }
        let closed = tokio::time::timeout(Duration::from_secs(4), child.wait()).await;
        if !matches!(closed,Ok(Ok(s)) if s.success()) && !unexpected_exit {
            interrupted = true;
            reason.get_or_insert(
                "The recording could not close cleanly. Recover the available segments.".into(),
            );
            let _ = child.kill().await;
        }
        for (_, recorder) in &mut recorders {
            if let Err(e) = recorder.finish() {
                interrupted = true;
                reason = Some(e);
            }
            if let Some(error) = recorder.log.lock().unwrap().error.clone() {
                interrupted = true;
                reason = Some(error);
            }
        }
        if let Some(stream) = &mut graphics {
            stream.finish().await;
            if let Some(error) = stream.error.lock().unwrap().clone() {
                interrupted = true;
                reason = Some(error);
            }
        }
        let tail = match tokio::time::timeout(Duration::from_secs(2), stderr_task).await {
            Ok(Ok(tail)) => tail,
            _ => String::new(),
        };
        let _files = self.0.files.lock().await;
        let first = first_pts.load(Ordering::Relaxed);
        journal.videos = video_files(&dir)?;
        journal.video_start_us = first
            + journal
                .videos
                .first()
                .and_then(|n| audio::segment_number(n))
                .unwrap_or(0) as i64
                * 2_000_000;
        journal.audio = recorders
            .iter()
            .map(|(kind, r)| audio::snapshot(&r.log, kind, false))
            .collect();
        journal.interrupted = interrupted;
        journal.reason = reason.clone();
        journal.phase = if interrupted { "interrupted" } else { "ready" }.into();
        journal.duration_ms = if options.mode == Mode::Replay {
            (journal.videos.len() as u64 * 2000).min(options.replay_seconds as u64 * 1000)
        } else {
            media_started
                .map(|start| start.elapsed().as_millis() as u64)
                .unwrap_or(0)
        };
        if first == 0 || journal.videos.is_empty() {
            return Err(format!(
                "{} {}",
                reason.unwrap_or_else(|| "The capture produced no recoverable video.".into()),
                tail
            ));
        }
        if options.mode == Mode::Replay && !interrupted {
            journal.phase = "discarded".into();
            for name in journal
                .videos
                .iter()
                .chain(journal.audio.iter().flat_map(|a| a.files.iter()))
            {
                let p = dir.join(name);
                regular(&p)?;
                fs::remove_file(p).map_err(|e| e.to_string())?;
            }
            write_journal(&dir, &journal)?;
            *self.0.state.lock().unwrap() = State {
                reason,
                ..State::default()
            };
        } else {
            write_journal(&dir, &journal)?;
            let mut s = self.0.state.lock().unwrap();
            s.phase = journal.phase.clone();
            s.reason = reason;
            s.ready_capture = Some(ready(&journal));
            s.bytes = directory_bytes(&dir)?;
        }
        Ok(())
    }
}

fn video_files(dir: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let e = entry.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("video-")
            && name.ends_with(".mkv")
            && audio::segment_number(&name).is_some()
        {
            regular(&e.path())?;
            files.push(name);
        }
    }
    files.sort();
    Ok(files)
}
fn directory_bytes(dir: &Path) -> Result<u64, String> {
    let mut bytes = 0u64;
    for e in fs::read_dir(dir).map_err(|e| e.to_string())? {
        bytes = bytes
            .checked_add(regular(&e.map_err(|e| e.to_string())?.path())?)
            .ok_or("Capture storage limit exceeded.")?;
    }
    Ok(bytes)
}
struct AudioStop(watch::Sender<bool>);
impl Drop for AudioStop {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}
struct CaptureLease {
    _file: Option<fs::File>,
}
impl CaptureLease {
    fn open(path: &Path, create: bool) -> Result<Self, String> {
        if !create && !path.exists() {
            return Ok(Self { _file: None });
        }
        if path.exists() {
            regular(path)?;
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(create);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        Ok(Self {
            _file: Some(options.open(path).map_err(|e| e.to_string())?),
        })
    }
}
fn destination_directory(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Choose an absolute authorized capture destination.".into());
    }
    let path = crate::library::authorized_path(path)?;
    if !path.is_dir() {
        return Err("The selected capture destination is unavailable.".into());
    }
    Ok(path)
}
fn raw_input(width: u32, height: u32, fps: u32, input: String) -> Vec<String> {
    vec![
        "-f".into(),
        "rawvideo".into(),
        "-pixel_format".into(),
        "bgra".into(),
        "-video_size".into(),
        format!("{width}x{height}"),
        "-framerate".into(),
        fps.to_string(),
        "-i".into(),
        input,
    ]
}
async fn still_input(
    target: &native::Target,
    region: Option<Region>,
    dir: &Path,
    args: &mut Vec<String>,
    cancel: watch::Receiver<bool>,
) -> Result<Option<PathBuf>, String> {
    if target.hwnd == 0 {
        args.extend(target.input_args(region, 1)?);
        return Ok(None);
    }
    let frame = wgc::one(target.clone(), cancel).await?;
    let raw = dir.join("frame.bgra");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&raw)
        .map_err(|e| e.to_string())?;
    file.write_all(&frame.bytes).map_err(|e| e.to_string())?;
    drop(file);
    args.extend(raw_input(
        target.width,
        target.height,
        1,
        raw.to_string_lossy().into_owned(),
    ));
    Ok(Some(raw))
}

fn base64(bytes: &[u8]) -> String {
    const CH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for b in bytes.chunks(3) {
        let n = ((b[0] as u32) << 16)
            | ((b.get(1).copied().unwrap_or(0) as u32) << 8)
            | b.get(2).copied().unwrap_or(0) as u32;
        s.push(CH[(n >> 18) as usize] as char);
        s.push(CH[((n >> 12) & 63) as usize] as char);
        s.push(if b.len() > 1 {
            CH[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        s.push(if b.len() > 2 {
            CH[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    s
}

#[cfg(test)]
mod tests;
