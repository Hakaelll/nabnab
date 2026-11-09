mod cache;
pub(crate) use cache::{cache_status, clear_cache, CacheStatus};
pub(crate) mod timed;
mod wave;
use crate::{
    engine,
    model::{Chapter, Progress},
    workspaces::Workspace,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::sync::watch;

pub const MAX_TEXT: usize = 1_000_000;
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SEGMENTS: usize = 4096;
const SEGMENT_CHARS: usize = 480;
const DRIVER: &str = include_str!("voice/desktop.ps1");
static ACTIVE: Mutex<Option<HashSet<String>>> = Mutex::new(None);
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Voice {
    pub id: String,
    pub name: String,
    pub language: String,
    pub enabled: bool,
    pub fingerprint: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceStatus {
    pub available: bool,
    pub engine_version: String,
    pub voices: Vec<Voice>,
    pub note: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoiceSettings {
    pub voice_id: String,
    pub rate: i8,
    pub volume: u8,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OriginKind {
    Text,
    Library,
    Note,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Origin {
    pub title: String,
    pub library_id: Option<String>,
    pub kind: OriginKind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Segment {
    pub index: usize,
    pub text: String,
    pub start_chars: usize,
    pub end_chars: usize,
    pub cached: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VoicePlan {
    pub schema_version: u8,
    pub id: String,
    pub created_at: u64,
    pub text: String,
    pub text_hash: String,
    pub source_version: String,
    pub origin: Origin,
    pub settings: VoiceSettings,
    pub voice_name: String,
    pub language: String,
    pub engine_fingerprint: String,
    pub segments: Vec<Segment>,
    pub warnings: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentStatus {
    pub index: usize,
    pub cached: bool,
    pub duration: Option<f64>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStatus {
    pub plan_id: String,
    pub text_hash: String,
    pub source_version: String,
    pub engine_fingerprint: String,
    pub active: bool,
    pub completed: usize,
    pub total: usize,
    pub duration: f64,
    pub bytes: u64,
    pub segments: Vec<SegmentStatus>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub wav: Vec<u8>,
    pub duration: f64,
    pub segment_text: String,
    pub truncated: bool,
}
#[derive(Debug)]
pub struct VoiceOutput {
    pub file: PathBuf,
    pub chapters: Vec<Chapter>,
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn driver_hash() -> &'static str {
    static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HASH.get_or_init(|| hash(DRIVER.as_bytes()))
}
fn cancelled(cancel: &watch::Receiver<bool>) -> Result<(), String> {
    if *cancel.borrow() {
        Err(engine::CANCELLED.into())
    } else {
        Ok(())
    }
}
pub fn validate_settings(settings: &VoiceSettings) -> Result<(), String> {
    if !cache::valid_id(&settings.voice_id)
        || !(-10..=10).contains(&settings.rate)
        || settings.volume > 100
    {
        Err("Choose an installed voice, speed -10–10 and volume 0–100.".into())
    } else {
        Ok(())
    }
}
fn validate_origin(origin: &Origin) -> Result<(), String> {
    if origin.title.trim().is_empty()
        || origin.title.len() > 2000
        || origin.title.chars().any(char::is_control)
        || origin
            .library_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 192 || id.chars().any(char::is_control))
        || matches!(origin.kind, OriginKind::Text) && origin.library_id.is_some()
        || !matches!(origin.kind, OriginKind::Text) && origin.library_id.is_none()
    {
        return Err("The narration needs a valid title and source identity.".into());
    }
    Ok(())
}
fn validate_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty()
        || text.len() > MAX_TEXT_BYTES
        || text.chars().count() > MAX_TEXT
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
    {
        Err("Choose nonempty text up to 1000000 Unicode characters and 4 MiB, without unsupported control characters.".into())
    } else {
        Ok(())
    }
}
fn split(text: &str) -> Result<Vec<Segment>, String> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut start = 0;
    let mut result = Vec::new();
    while start < chars.len() {
        while start < chars.len() && chars[start].is_whitespace() {
            start += 1;
        }
        if start == chars.len() {
            break;
        }
        let limit = (start + SEGMENT_CHARS).min(chars.len());
        let mut end = limit;
        if let Some(at) = (start..limit).find(|&at| {
            chars[at] == '\n'
                && (chars.get(at + 1) == Some(&'\n')
                    || chars.get(at + 1) == Some(&'\r') && chars.get(at + 2) == Some(&'\n'))
        }) {
            end = at;
        } else if limit < chars.len() {
            if let Some(at) = (start + 80..limit).rev().find(|&at| {
                matches!(chars[at], '.' | '?' | '!' | ';')
                    && chars.get(at + 1).is_some_and(|c| c.is_whitespace())
            }) {
                end = at + 1;
            } else if let Some(at) = (start + 80..limit)
                .rev()
                .find(|&at| chars[at].is_whitespace())
            {
                end = at;
            }
        }
        while end > start && chars[end - 1].is_whitespace() {
            end -= 1;
        }
        if end == start {
            start += 1;
            continue;
        }
        result.push(Segment {
            index: result.len(),
            text: chars[start..end].iter().collect(),
            start_chars: start,
            end_chars: end,
            cached: false,
        });
        start = end;
        if result.len() > MAX_SEGMENTS {
            return Err(
                "The narration exceeds 4096 paragraphs or segments. Choose a shorter section."
                    .into(),
            );
        }
    }
    Ok(result)
}
fn plan_identity(plan: &VoicePlan) -> Result<String, String> {
    let identity = serde_json::json!({
        "schemaVersion": 1,
        "driver": driver_hash(),
        "textHash": plan.text_hash,
        "sourceVersion": plan.source_version,
        "origin": plan.origin,
        "settings": plan.settings,
        "voiceName": plan.voice_name,
        "language": plan.language,
        "engineFingerprint": plan.engine_fingerprint
    });
    Ok(hash(
        &serde_json::to_vec(&identity).map_err(|e| e.to_string())?,
    ))
}
fn segment_key(plan: &VoicePlan, text: &str) -> Result<String, String> {
    segment_key_fields(&plan.settings, &plan.engine_fingerprint, text)
}
fn plan_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !cache::valid_id(id) {
        return Err("Choose a prepared local narration plan.".into());
    }
    Ok(cache::directory(root, "voice-plans")?.join(format!("{id}.json")))
}
fn load_plan(root: &Path, id: &str, refresh_cache: bool) -> Result<VoicePlan, String> {
    let mut plan: VoicePlan = cache::read_json(&plan_path(root, id)?, 32 * 1024 * 1024)?;
    validate_text(&plan.text)?;
    validate_settings(&plan.settings)?;
    validate_origin(&plan.origin)?;
    if plan.schema_version != 1
        || plan.id != id
        || !cache::valid_id(&plan.source_version)
        || !cache::valid_id(&plan.engine_fingerprint)
        || plan.voice_name.is_empty()
        || plan.voice_name.len() > 500
        || plan.language.is_empty()
        || plan.language.len() > 80
        || plan.warnings.len() > 16
        || plan.warnings.iter().any(|warning| warning.len() > 2000)
        || plan.text_hash != hash(plan.text.as_bytes())
        || plan_identity(&plan)? != id
    {
        return Err("The local narration plan changed or belongs to another engine version. Prepare it again.".into());
    }
    let expected = split(&plan.text)?;
    if expected.len() != plan.segments.len()
        || expected.iter().zip(&plan.segments).any(|(a, b)| {
            a.index != b.index
                || a.text != b.text
                || a.start_chars != b.start_chars
                || a.end_chars != b.end_chars
        })
    {
        return Err(
            "The narration segment boundaries no longer match its saved text version.".into(),
        );
    }
    if refresh_cache {
        let directory = cache::directory(root, "voice-cache")?;
        let (_sender, cancel) = watch::channel(false);
        for segment in &mut plan.segments {
            segment.cached = cache::cached_at(
                &directory,
                &segment_key_fields(&plan.settings, &plan.engine_fingerprint, &segment.text)?,
                false,
                &cancel,
            )
            .is_some();
        }
    }
    Ok(plan)
}
pub fn read_plan(root: &Path, id: &str) -> Result<VoicePlan, String> {
    load_plan(root, id, true)
}
fn segment_key_fields(
    settings: &VoiceSettings,
    engine_fingerprint: &str,
    text: &str,
) -> Result<String, String> {
    let identity = serde_json::json!({
        "pipeline": driver_hash(),
        "sampleRate": wave::SAMPLE_RATE,
        "engineFingerprint": engine_fingerprint,
        "settings": settings,
        "text": text
    });
    Ok(hash(
        &serde_json::to_vec(&identity).map_err(|e| e.to_string())?,
    ))
}
fn powershell() -> Result<PathBuf, String> {
    if !cfg!(windows) {
        return Err("Local desktop speech is available on Windows with System.Speech and compatible Microsoft Desktop voices.".into());
    }
    let root = std::env::var_os("SystemRoot").ok_or("Windows SystemRoot is unavailable.")?;
    crate::library::authorized_path(
        &PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
}
fn driver(workspace: &Path) -> Result<PathBuf, String> {
    let path = workspace.join("nabnab-desktop-voice.ps1");
    cache::write_new(&path, DRIVER.as_bytes())?;
    Ok(path)
}
pub async fn status(root: &Path, cancel: watch::Receiver<bool>) -> Result<VoiceStatus, String> {
    cancelled(&cancel)?;
    let executable = match powershell() {
        Ok(path) => path,
        Err(note) => {
            return Ok(VoiceStatus {
                available: false,
                engine_version: String::new(),
                voices: vec![],
                note,
            })
        }
    };
    let workspace = Workspace::new(root)?;
    let script = driver(&workspace.path)?;
    let raw = engine::run_process(
        executable,
        vec![
            "-NoLogo".into(),
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-File".into(),
            script.to_string_lossy().into_owned(),
            "status".into(),
        ],
        cancel,
        0,
        true,
        |_| {},
    )
    .await?;
    if raw.len() > 65536 {
        return Err("Windows speech returned excessive voice metadata.".into());
    }
    let result: VoiceStatus = serde_json::from_str(raw.trim_start_matches('\u{feff}').trim())
        .map_err(|_| "Windows speech returned unreadable installed-voice metadata.")?;
    if result.voices.len() > 128
        || result.engine_version.len() > 2000
        || result.note.len() > 4000
        || result.voices.iter().any(|voice| {
            !cache::valid_id(&voice.id)
                || !cache::valid_id(&voice.fingerprint)
                || voice.name.len() > 500
                || voice.language.len() > 80
        })
    {
        return Err("Windows speech returned invalid voice identities.".into());
    }
    Ok(result)
}
async fn current_voice(
    root: &Path,
    plan: &VoicePlan,
    cancel: watch::Receiver<bool>,
) -> Result<Voice, String> {
    let status = status(root, cancel).await?;
    let voice=status.voices.into_iter().find(|voice|voice.id==plan.settings.voice_id&&voice.enabled).ok_or("The selected local desktop voice is unavailable. Install a compatible Windows voice manually, refresh the list and prepare a new plan.")?;
    if voice.fingerprint != plan.engine_fingerprint {
        return Err("The desktop speech engine or reported voice version changed. Prepare a new plan; audio from the old voice configuration was not reused.".into());
    }
    Ok(voice)
}
pub async fn prepare(
    root: &Path,
    text: &str,
    source_version: &str,
    origin: Origin,
    settings: VoiceSettings,
    cancel: watch::Receiver<bool>,
) -> Result<VoicePlan, String> {
    validate_text(text)?;
    validate_settings(&settings)?;
    validate_origin(&origin)?;
    if !cache::valid_id(source_version) {
        return Err(
            "The narration requires a SHA-256 identity for its source text version.".into(),
        );
    }
    let status = status(root, cancel.clone()).await?;
    let voice = status.voices.iter()
        .find(|voice| voice.id == settings.voice_id && voice.enabled)
        .ok_or("Select an installed Microsoft Desktop voice. This feature does not download voices or use an online fallback.")?;
    let mut plan = VoicePlan {
        schema_version: 1,
        id: String::new(),
        created_at: crate::queue::now(),
        text: text.into(),
        text_hash: hash(text.as_bytes()),
        source_version: source_version.into(),
        origin,
        settings,
        voice_name: voice.name.clone(),
        language: voice.language.clone(),
        engine_fingerprint: voice.fingerprint.clone(),
        segments: split(text)?,
        warnings: vec![
            "Installed Microsoft Desktop SAPI voices run locally. No voice binary is bundled or downloaded by nabnab.".into(),
            "Voice compatibility is bound to the engine and voice metadata Windows reports, including AdditionalInfo. Windows does not expose a cryptographic model-file manifest through this API.".into(),
            "Timing belongs to complete synthesized segments. Word-by-word alignment is not claimed.".into(),
            "Limits: 1000000 Unicode characters / 4 MiB, 4096 segments and 3 GiB of audio. Text length does not guarantee an exact audio duration or size. Cached audio also occupies local disk space.".into()
        ],
    };
    plan.id = plan_identity(&plan)?;
    cancelled(&cancel)?;
    {
        let _guard = cache::WRITE_LOCK.lock().unwrap();
        let path = plan_path(root, &plan.id)?;
        if !path.exists() {
            cache::write_new(
                &path,
                &serde_json::to_vec(&plan).map_err(|e| e.to_string())?,
            )?;
        }
    }
    read_plan(root, &plan.id)
}
pub fn plan_status(root: &Path, id: &str) -> Result<PlanStatus, String> {
    let plan = load_plan(root, id, false)?;
    let directory = cache::directory(root, "voice-cache")?;
    let (_sender, cancel) = watch::channel(false);
    let mut duration = 0.0;
    let mut bytes = 0;
    let mut completed = 0;
    let mut segments = Vec::new();
    for segment in &plan.segments {
        let cached = cache::cached_at(
            &directory,
            &segment_key(&plan, &segment.text)?,
            false,
            &cancel,
        );
        let seconds = cached.as_ref().map(|entry| entry.wave.duration());
        if let Some(seconds) = seconds {
            duration += seconds;
            bytes += cached.as_ref().map_or(0, |entry| entry.wave.length);
            completed += 1;
        }
        segments.push(SegmentStatus {
            index: segment.index,
            cached: seconds.is_some(),
            duration: seconds,
        });
    }
    let active = ACTIVE
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|active| active.contains(id));
    Ok(PlanStatus {
        plan_id: plan.id,
        text_hash: plan.text_hash,
        source_version: plan.source_version,
        engine_fingerprint: plan.engine_fingerprint,
        active,
        completed,
        total: segments.len(),
        duration,
        bytes,
        segments,
    })
}
pub fn segment_audio(root: &Path, id: &str, index: usize) -> Result<Vec<u8>, String> {
    let _cache = cache::IN_USE.try_read().map_err(|_| "Wait for the voice cache to finish clearing.")?;
    let plan = load_plan(root, id, false)?;
    let segment = plan
        .segments
        .get(index)
        .ok_or("Choose a segment from this narration plan.")?;
    let (_sender, cancel) = watch::channel(false);
    cache::audio(root, &segment_key(&plan, &segment.text)?, &cancel)
}

#[derive(Clone)]
struct Work {
    key: String,
    text: String,
    weight: usize,
}
#[derive(Clone)]
struct Pending {
    work: Work,
    output: PathBuf,
}
async fn generate(
    root: &Path,
    settings: &VoiceSettings,
    engine_fingerprint: &str,
    works: Vec<Work>,
    completed: usize,
    total: usize,
    initial_bytes: u64,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<(), String> {
    if works.is_empty() {
        return Ok(());
    }
    cancelled(&cancel)?;
    let workspace = Workspace::new(root)?;
    let script = driver(&workspace.path)?;
    let pending = works
        .into_iter()
        .enumerate()
        .map(|(index, work)| Pending {
            work,
            output: workspace.path.join(format!("segment-{index:04}.wav")),
        })
        .collect::<Vec<_>>();
    let input = workspace.path.join("voice-request.json");
    let items = pending
        .iter()
        .map(|pending| {
            serde_json::json!({
                "text": pending.work.text,
                "output": pending.output,
                "partial": pending.output.with_extension("part.wav")
            })
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&serde_json::json!({
        "voiceId": settings.voice_id,
        "rate": settings.rate,
        "volume": settings.volume,
        "engineFingerprint": engine_fingerprint,
        "items": items
    }))
    .map_err(|e| e.to_string())?;
    cache::write_new(&input, &bytes)?;
    let executable = powershell()?;
    let (callback_cancel, process_cancel) = watch::channel(false);
    let parent_cancel = cancel.clone();
    let relay_cancel = callback_cancel.clone();
    let relay = tokio::spawn(async move {
        let mut parent = parent_cancel;
        if !*parent.borrow() {
            let _ = parent.changed().await;
        }
        let _ = relay_cancel.send(true);
    });
    let state = Arc::new(Mutex::new((
        0usize,
        completed,
        initial_bytes,
        None::<String>,
    )));
    let updates = state.clone();
    let cache_root = root.to_owned();
    let cancelled_on_error = callback_cancel.clone();
    let callback_token = process_cancel.clone();
    let pending = Arc::new(pending);
    let callback_pending = pending.clone();
    let update = emit.clone();
    let result = engine::run_process(
        executable,
        vec![
            "-NoLogo".into(), "-NoProfile".into(), "-NonInteractive".into(),
            "-ExecutionPolicy".into(), "Bypass".into(), "-File".into(),
            script.to_string_lossy().into_owned(), "synthesize".into(),
            input.to_string_lossy().into_owned(),
        ],
        process_cancel,
        run_id,
        false,
        move |progress| {
            let finished = progress.downloaded.unwrap_or(0.0).max(0.0)
                .min(callback_pending.len() as f64) as usize;
            let mut state = updates.lock().unwrap();
            while state.0 < finished && state.3.is_none() {
                let item = &callback_pending[state.0];
                let result = cache::publish(&cache_root, &item.work.key, &item.output, &callback_token)
                    .and_then(|_| {
                        let cached = cache::cached(&cache_root, &item.work.key, false, &callback_token)
                            .ok_or("The completed voice segment could not be verified.")?;
                        let bytes = cached.wave.length.checked_mul(item.work.weight as u64)
                            .and_then(|value| state.2.checked_add(value))
                            .filter(|bytes| *bytes <= wave::MAX_OUTPUT)
                            .ok_or("The narration exceeds its 3 GiB audio limit. Choose a shorter text section.")?;
                        state.2 = bytes;
                        Ok(())
                    });
                if let Err(error) = result {
                    state.3 = Some(error);
                    let _ = cancelled_on_error.send(true);
                    break;
                }
                let _ = fs::remove_file(&item.output);
                state.0 += 1;
                state.1 += item.work.weight;
            }
            update(Progress {
                request_id: run_id,
                phase: "processing".into(),
                percent: Some(state.1 as f64 / total.max(1) as f64 * 90.0),
                speed: None,
                downloaded: None,
                total: None,
            });
        },
    ).await;
    relay.abort();
    let state = state.lock().unwrap();
    if let Some(error) = &state.3 {
        return Err(error.clone());
    }
    result?;
    cancelled(&cancel)?;
    if state.0 != pending.len() {
        return Err("The desktop speech process ended before every requested segment was committed. Completed segments were preserved for resume.".into());
    }
    Ok(())
}
struct ActiveGuard(String);
impl ActiveGuard {
    fn new(id: &str) -> Result<Self, String> {
        let mut active = ACTIVE.lock().unwrap();
        if !active.get_or_insert_with(HashSet::new).insert(id.into()) {
            return Err("This narration plan is already running. Pause it before starting another execution.".into());
        }
        Ok(Self(id.into()))
    }
}
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        if let Some(active) = ACTIVE.lock().unwrap().as_mut() {
            active.remove(&self.0);
        }
    }
}
pub async fn sample(
    root: &Path,
    id: &str,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<Sample, String> {
    let _cache = cache::IN_USE.read().await;
    let plan = load_plan(root, id, false)?;
    current_voice(root, &plan, cancel.clone()).await?;
    let first = plan
        .segments
        .first()
        .ok_or("The narration has no text segments.")?;
    let mut chars = first.text.chars().take(200).collect::<Vec<_>>();
    if first.text.chars().count() > 200 {
        if let Some(end) = (80..chars.len())
            .rev()
            .find(|&position| chars[position].is_whitespace())
        {
            chars.truncate(end);
        }
    }
    let segment_text = chars.iter().collect::<String>();
    let key = segment_key(&plan, &segment_text)?;
    if cache::cached(root, &key, true, &cancel).is_none() {
        generate(
            root,
            &plan.settings,
            &plan.engine_fingerprint,
            vec![Work {
                key: key.clone(),
                text: segment_text.clone(),
                weight: 1,
            }],
            0,
            1,
            0,
            cancel.clone(),
            run_id,
            emit.clone(),
        )
        .await?;
    }
    let cached = cache::cached(root, &key, true, &cancel)
        .ok_or("The local speech sample was not completed.")?;
    let (wav, duration, truncated) = wave::sample_bytes(&cached.path, 30)?;
    cancelled(&cancel)?;
    emit(Progress {
        request_id: run_id,
        phase: "processing".into(),
        percent: Some(100.0),
        speed: None,
        downloaded: None,
        total: None,
    });
    Ok(Sample {
        wav,
        duration,
        segment_text,
        truncated,
    })
}
fn output_name(plan: &VoicePlan) -> Result<String, String> {
    let mut name = String::new();
    for character in plan.origin.title.chars() {
        let character = if character.is_control() || "<>:\"/\\|?*".contains(character) {
            '_'
        } else {
            character
        };
        if name.len() + character.len_utf8() > 120 {
            break;
        }
        name.push(character);
    }
    let stem = name.trim_matches([' ', '.']);
    let stem = if stem.is_empty() { "narration" } else { stem };
    let language=plan.language.chars().filter(|c|c.is_ascii_alphanumeric() || *c=='-').take(20).collect::<String>();
    let name = format!("{stem} ({language}).wav");
    crate::direct::validate_filename(&name)?;
    Ok(name)
}
fn chapter_title(index: usize, text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut characters = normalized.chars();
    let mut excerpt = characters.by_ref().take(20).collect::<String>();
    if characters.next().is_some() {
        excerpt.push('…');
    }
    format!("{} · {excerpt}", index + 1)
}
fn combine(
    root: &Path,
    plan: &VoicePlan,
    staging: &Path,
    cancel: &watch::Receiver<bool>,
) -> Result<VoiceOutput, String> {
    let staging = crate::library::authorized_path(staging)?;
    if !staging.is_dir() {
        return Err("Choose an owned narration output directory.".into());
    }
    let file = staging.join(output_name(plan)?);
    if fs::symlink_metadata(&file).is_ok() {
        return Err("The narration output already exists. Existing files were preserved.".into());
    }
    let directory = cache::directory(root, "voice-cache")?;
    let mut segments = Vec::new();
    let mut length = 0u64;
    for segment in &plan.segments {
        cancelled(cancel)?;
        let cached = cache::cached_at(&directory, &segment_key(plan, &segment.text)?, true, cancel)
            .ok_or(
                "A narration segment is incomplete or changed. Resume this plan to regenerate it.",
            )?;
        length = length
            .checked_add(cached.wave.length)
            .filter(|length| *length <= wave::MAX_OUTPUT)
            .ok_or("The narration exceeds its 3 GiB audio limit.")?;
        segments.push(cached);
    }
    let partial = staging.join(".voice-final.part.wav");
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|e| e.to_string())?;
    let mut chapters = Vec::new();
    let mut samples = 0u64;
    let result = (|| {
        output
            .write_all(&wave::header(length)?)
            .map_err(|e| e.to_string())?;
        for (segment, cached) in plan.segments.iter().zip(&segments) {
            cancelled(cancel)?;
            let start = samples as f64 / f64::from(wave::SAMPLE_RATE);
            wave::copy_samples(&cached.path, &cached.wave, &mut output, cancel)?;
            samples += cached.wave.length / 2;
            let end = samples as f64 / f64::from(wave::SAMPLE_RATE);
            chapters.push(Chapter {
                title: chapter_title(segment.index, &segment.text),
                start,
                end,
            });
        }
        cancelled(cancel)?;
        output.sync_all().map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })();
    drop(output);
    if let Err(error) = result {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    let publication =
        cancelled(cancel).and_then(|_| crate::direct::move_completed(&partial, &file));
    if let Err(error) = publication {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    Ok(VoiceOutput { file, chapters })
}
pub async fn execute(
    root: &Path,
    id: &str,
    staging: &Path,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<VoiceOutput, String> {
    let _cache = cache::IN_USE.read().await;
    cancelled(&cancel)?;
    let plan = load_plan(root, id, false)?;
    let _active = ActiveGuard::new(id)?;
    current_voice(root, &plan, cancel.clone()).await?;
    let mut work = Vec::<Work>::new();
    let mut positions = std::collections::HashMap::<String, usize>::new();
    let mut verified = std::collections::HashMap::<String, Option<u64>>::new();
    let mut complete = 0;
    let mut bytes = 0u64;
    let directory = cache::directory(root, "voice-cache")?;
    for segment in &plan.segments {
        cancelled(&cancel)?;
        let key = segment_key(&plan, &segment.text)?;
        let length = *verified.entry(key.clone()).or_insert_with(|| {
            cache::cached_at(&directory, &key, true, &cancel).map(|cached| cached.wave.length)
        });
        if let Some(length) = length {
            complete += 1;
            bytes = bytes
                .checked_add(length)
                .filter(|bytes| *bytes <= wave::MAX_OUTPUT)
                .ok_or("The narration exceeds its 3 GiB audio limit.")?;
        } else if let Some(position) = positions.get(&key) {
            work[*position].weight += 1;
        } else {
            positions.insert(key.clone(), work.len());
            work.push(Work {
                key,
                text: segment.text.clone(),
                weight: 1,
            });
        }
    }
    generate(
        root,
        &plan.settings,
        &plan.engine_fingerprint,
        work,
        complete,
        plan.segments.len(),
        bytes,
        cancel.clone(),
        run_id,
        emit.clone(),
    )
    .await?;
    cancelled(&cancel)?;
    let root = root.to_owned();
    let staging = staging.to_owned();
    let token = cancel.clone();
    let output = tokio::task::spawn_blocking(move || combine(&root, &plan, &staging, &token))
        .await
        .map_err(|e| e.to_string())??;
    emit(Progress {
        request_id: run_id,
        phase: "processing".into(),
        percent: Some(100.0),
        speed: None,
        downloaded: None,
        total: None,
    });
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    fn directory() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nabnab-voice-{}-{}-{}",
            std::process::id(),
            crate::queue::now(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn fixture_plan(root: &Path, text: &str) -> VoicePlan {
        let mut plan = VoicePlan {
            schema_version: 1,
            id: String::new(),
            created_at: crate::queue::now(),
            text: text.into(),
            text_hash: hash(text.as_bytes()),
            source_version: hash(b"source-version"),
            origin: Origin {
                title: "Voice fixture".into(),
                library_id: None,
                kind: OriginKind::Text,
            },
            settings: VoiceSettings {
                voice_id: "a".repeat(64),
                rate: 0,
                volume: 100,
            },
            voice_name: "Fixture voice".into(),
            language: "en-US".into(),
            engine_fingerprint: "b".repeat(64),
            segments: split(text).unwrap(),
            warnings: vec![],
        };
        plan.id = plan_identity(&plan).unwrap();
        cache::write_new(
            &plan_path(root, &plan.id).unwrap(),
            &serde_json::to_vec(&plan).unwrap(),
        )
        .unwrap();
        plan
    }
    fn fixture_wave(root: &Path, name: &str, samples: usize) -> PathBuf {
        let path = root.join(name);
        let mut bytes = wave::header((samples * 2) as u64).unwrap();
        for index in 0..samples {
            bytes.extend(((index % 32000) as i16).to_le_bytes());
        }
        fs::write(&path, bytes).unwrap();
        path
    }
    #[test]
    fn exported_narration_uses_a_readable_title_and_language() {
        let root=directory();let mut plan=fixture_plan(&root,"Texto sintético.");
        plan.origin.title="Un artículo: ideas para leer".into();plan.language="es-ES".into();
        assert_eq!(output_name(&plan).unwrap(),"Un artículo_ ideas para leer (es-ES).wav");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unicode_segments_keep_actual_codepoint_boundaries_and_book_limits() {
        let text = format!(
            "  Hola 🧩, primer párrafo.\r\n\r\n{}\n\nÚltima línea.",
            "á🙂 palabra ".repeat(100)
        );
        let chars = text.chars().collect::<Vec<_>>();
        let segments = split(&text).unwrap();
        assert!(segments.len() > 3);
        for segment in &segments {
            assert_eq!(
                segment.text,
                chars[segment.start_chars..segment.end_chars]
                    .iter()
                    .collect::<String>()
            );
            assert!(segment.end_chars - segment.start_chars <= SEGMENT_CHARS);
            assert!(!segment.text.trim().is_empty());
        }
        assert_eq!(segments[0].text, "Hola 🧩, primer párrafo.");
        assert_eq!(segments.last().unwrap().text, "Última línea.");
        assert!(validate_text(&"x".repeat(1_000_000)).is_ok());
        assert!(split(&"x".repeat(100_001)).unwrap().len() > 200);
        assert!(validate_text(&"x".repeat(1_000_001)).is_err());
        assert!(validate_text("text\0hidden").is_err());
        assert!(split(&"paragraph\n\n".repeat(MAX_SEGMENTS + 1)).is_err());
    }
    #[test]
    fn cache_configuration_and_saved_plan_are_bound_to_text_voice_and_versions() {
        let root = directory();
        let plan = fixture_plan(&root, "First paragraph.\n\nSecond paragraph.");
        let text = &plan.segments[0].text;
        let key = segment_key(&plan, text).unwrap();
        let mut changed = plan.clone();
        changed.settings.rate = 1;
        assert_ne!(key, segment_key(&changed, text).unwrap());
        changed = plan.clone();
        changed.settings.volume = 80;
        assert_ne!(key, segment_key(&changed, text).unwrap());
        changed = plan.clone();
        changed.engine_fingerprint = "c".repeat(64);
        assert_ne!(key, segment_key(&changed, text).unwrap());
        changed = plan.clone();
        changed.settings.voice_id = "d".repeat(64);
        assert_ne!(key, segment_key(&changed, text).unwrap());
        changed = plan.clone();
        changed.source_version = "e".repeat(64);
        assert_ne!(
            plan_identity(&plan).unwrap(),
            plan_identity(&changed).unwrap()
        );
        changed = plan.clone();
        changed.voice_name = "Other voice".into();
        assert_ne!(plan.id, plan_identity(&changed).unwrap());
        changed = plan.clone();
        changed.language = "es-ES".into();
        assert_ne!(plan.id, plan_identity(&changed).unwrap());
        assert_ne!(key, segment_key(&plan, "Different text.").unwrap());
        let read = read_plan(&root, &plan.id).unwrap();
        assert_eq!(read.origin.title, "Voice fixture");
        assert_eq!(read.language, "en-US");
        assert_eq!(read.text_hash, plan.text_hash);
        let mut tampered = plan.clone();
        tampered.segments[0].text = "Changed boundary".into();
        fs::write(
            plan_path(&root, &plan.id).unwrap(),
            serde_json::to_vec(&tampered).unwrap(),
        )
        .unwrap();
        assert!(read_plan(&root, &plan.id).is_err());
        assert!(read_plan(&root, "../other").is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn status_is_lightweight_but_audio_checks_integrity_before_use() {
        let root = directory();
        let plan = fixture_plan(&root, "First paragraph.\n\nSecond paragraph.");
        let input = fixture_wave(&root, "generated.wav", 2205);
        let key = segment_key(&plan, &plan.segments[0].text).unwrap();
        let (_sender, cancel) = watch::channel(false);
        cache::publish(&root, &key, &input, &cancel).unwrap();
        let status = plan_status(&root, &plan.id).unwrap();
        assert_eq!(status.completed, 1);
        assert_eq!(status.total, 2);
        assert_eq!(status.bytes, 4410);
        assert!((status.duration - 0.1).abs() < 1e-9);
        let serialized = serde_json::to_value(&status).unwrap();
        assert!(serialized["segments"][0].get("text").is_none());
        assert!(segment_audio(&root, &plan.id, 0)
            .unwrap()
            .starts_with(b"RIFF"));
        assert!(segment_audio(&root, &plan.id, 1).is_err());
        let path = cache::directory(&root, "voice-cache")
            .unwrap()
            .join(format!("{key}.wav"));
        let mut changed = fs::read(&path).unwrap();
        *changed.last_mut().unwrap() ^= 0x7f;
        fs::write(&path, &changed).unwrap();
        assert!(segment_audio(&root, &plan.id, 0).is_err());
        cache::publish(&root, &key, &input, &cancel).unwrap();
        assert_eq!(
            segment_audio(&root, &plan.id, 0).unwrap(),
            fs::read(&input).unwrap()
        );
        let cache_dir = cache::directory(&root, "voice-cache").unwrap();
        let pending = cache_dir.join("voice-10-2.pending");
        let unrelated = cache_dir.join("my-export.wav");
        fs::write(&pending, b"unfinished").unwrap();
        fs::write(&unrelated, b"untouched").unwrap();
        let status = serde_json::to_value(cache_status(&root).unwrap()).unwrap();
        assert_eq!(status["files"], 3);
        assert!(status["bytes"].as_u64().unwrap() > 4410);

        let active = cache::IN_USE.try_read().unwrap();
        assert!(clear_cache(&root).is_err());
        assert!(pending.is_file());
        drop(active);

        let empty = serde_json::json!({"bytes": 0, "files": 0});
        assert_eq!(serde_json::to_value(clear_cache(&root).unwrap()).unwrap(), empty);
        assert_eq!(serde_json::to_value(cache_status(&root).unwrap()).unwrap(), empty);
        assert_eq!(serde_json::to_value(clear_cache(&root).unwrap()).unwrap(), empty);
        assert_eq!(fs::read(unrelated).unwrap(), b"untouched");
        assert_eq!(plan_status(&root, &plan.id).unwrap().completed, 0);
        assert_eq!(read_plan(&root, &plan.id).unwrap().text, plan.text);
        assert!(segment_audio(&root, &plan.id, 0).is_err());

        cache::publish(&root, &key, &input, &cancel).unwrap();
        assert_eq!(segment_audio(&root, &plan.id, 0).unwrap(), fs::read(input).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn wav_concatenation_has_real_segment_times_and_preserves_cached_audio() {
        let root = directory();
        let plan = fixture_plan(&root, "One.\n\nTwo.");
        let a = fixture_wave(&root, "a.wav", 2205);
        let b = fixture_wave(&root, "b.wav", 4410);
        let (_sender, cancel) = watch::channel(false);
        for (index, path) in [&a, &b].into_iter().enumerate() {
            cache::publish(
                &root,
                &segment_key(&plan, &plan.segments[index].text).unwrap(),
                path,
                &cancel,
            )
            .unwrap();
        }
        let original = fs::read(&a).unwrap();
        let stage = root.join("stage");
        fs::create_dir(&stage).unwrap();
        let result = combine(&root, &plan, &stage, &cancel).unwrap();
        let file = wave::inspect(&result.file).unwrap();
        assert!((file.duration() - 0.3).abs() < 1e-9);
        assert_eq!(result.chapters.len(), 2);
        assert_eq!(result.chapters[0].start, 0.0);
        assert!((result.chapters[0].end - 0.1).abs() < 1e-9);
        assert_eq!(result.chapters[1].start, result.chapters[0].end);
        assert!((result.chapters[1].end - file.duration()).abs() < 1e-9);
        assert_eq!(fs::read(a).unwrap(), original);
        assert!(combine(&root, &plan, &stage, &cancel).is_err());
        let (_, stopped) = watch::channel(true);
        assert!(combine(&root, &plan, &root, &stopped).is_err());
        assert_eq!(plan_status(&root, &plan.id).unwrap().completed, 2);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_and_truncated_wav_files_are_not_publishable_and_samples_are_bounded() {
        let root = directory();
        let path = fixture_wave(&root, "valid.wav", wave::SAMPLE_RATE as usize * 31);
        let (bytes, duration, truncated) = wave::sample_bytes(&path, 30).unwrap();
        assert!(truncated);
        assert_eq!(duration, 30.0);
        assert_eq!(bytes.len(), 44 + wave::SAMPLE_RATE as usize * 2 * 30);
        let mut bytes = fs::read(&path).unwrap();
        bytes.pop();
        fs::write(&path, &bytes).unwrap();
        assert!(wave::inspect(&path).is_err());
        let mut bytes = wave::header(20).unwrap();
        bytes[22] = 2;
        bytes.extend([0; 20]);
        fs::write(&path, bytes).unwrap();
        assert!(wave::inspect(&path).is_err());
        assert!(wave::header(3 * 1024 * 1024 * 1024).is_ok());
        assert!(wave::header(3 * 1024 * 1024 * 1024 + 2).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn maximum_multibyte_chapters_fit_publication_record_without_changing_times() {
        let root = directory();
        let path = fixture_wave(&root, "narration.wav", 1);
        let text = "🧩".repeat(100);
        let mut item = crate::library::index(&path, None, None, None).unwrap();
        item.details.chapters = (0..MAX_SEGMENTS)
            .map(|index| Chapter {
                title: chapter_title(index, &text),
                start: index as f64 * 0.25,
                end: (index + 1) as f64 * 0.25,
            })
            .collect();
        item.duration = Some(MAX_SEGMENTS as f64 * 0.25);
        item.details.narration = Some(crate::library_features::NarrationReference {
            plan_ref: Some("a".repeat(64)),
            text_hash: "b".repeat(64),
            source_version: "c".repeat(64),
            voice_name: "Microsoft Helena Desktop".into(),
            language: "es-ES".into(),
            engine_fingerprint: "d".repeat(64),
        });
        let bytes = serde_json::to_vec(&item).unwrap();
        assert!(bytes.len() < 1024 * 1024, "{} bytes", bytes.len());
        assert_eq!(
            item.details.chapters[0].title,
            format!("1 · {}…", "🧩".repeat(20))
        );
        let restored: crate::library::LibraryItem = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(restored.details.chapters.len(), MAX_SEGMENTS);
        for (expected, actual) in item.details.chapters.iter().zip(&restored.details.chapters) {
            assert_eq!(expected.title, actual.title);
            assert_eq!(expected.start, actual.start);
            assert_eq!(expected.end, actual.end);
        }
        assert_eq!(restored.duration, Some(1024.0));
        println!("Maximum voice chapter metadata: {} bytes", bytes.len());
        fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    #[ignore = "Requires installed Microsoft Desktop SAPI voices; set NABNAB_TEST_VOICE=1. Writes synthetic WAV only, never plays speakers."]
    async fn real_desktop_voice_sample_partial_cancel_resume_and_large_plan() {
        assert_eq!(std::env::var("NABNAB_TEST_VOICE").as_deref(), Ok("1"));
        let root = directory();
        let (_sender, cancel) = watch::channel(false);
        let installed = status(&root, cancel.clone()).await.unwrap();
        assert!(installed.available, "{}", installed.note);
        let voice = installed
            .voices
            .iter()
            .find(|voice| voice.enabled && voice.language == "es-ES")
            .or_else(|| installed.voices.iter().find(|voice| voice.enabled))
            .unwrap();
        let settings = VoiceSettings {
            voice_id: voice.id.clone(),
            rate: 0,
            volume: 100,
        };
        let origin = Origin {
            title: "Prueba local de voz".into(),
            library_id: None,
            kind: OriginKind::Text,
        };
        let text = "Una muestra breve, creada en este equipo para verificar la voz local.";
        let plan = prepare(
            &root,
            text,
            &hash(text.as_bytes()),
            origin.clone(),
            settings.clone(),
            cancel.clone(),
        )
        .await
        .unwrap();
        let sample = sample(&root, &plan.id, cancel.clone(), 1, |_| {})
            .await
            .unwrap();
        assert!(sample.wav.len() > 44);
        assert!(sample.duration > 0.0 && sample.duration <= 30.0);
        assert_eq!(sample.segment_text, text);
        println!(
            "Local voice sample: {} ({}) {:.3}s, {} bytes",
            voice.name,
            voice.language,
            sample.duration,
            sample.wav.len()
        );
        if let Some(english) = installed.voices.iter().find(|candidate| {
            candidate.enabled && candidate.language.starts_with("en") && candidate.id != voice.id
        }) {
            let text = "A short synthetic sample verifies this installed English voice without using a remote service.";
            let english_plan = prepare(
                &root,
                text,
                &hash(text.as_bytes()),
                origin.clone(),
                VoiceSettings {
                    voice_id: english.id.clone(),
                    rate: 0,
                    volume: 100,
                },
                cancel.clone(),
            )
            .await
            .unwrap();
            let sample = super::sample(&root, &english_plan.id, cancel.clone(), 4, |_| {})
                .await
                .unwrap();
            assert!(sample.wav.len() > 44 && sample.duration > 0.0 && sample.duration <= 30.0);
            assert_eq!(sample.segment_text, text);
            println!(
                "Local voice sample: {} ({}) {:.3}s, {} bytes",
                english.name,
                english.language,
                sample.duration,
                sample.wav.len()
            );
        }
        let large = "Texto de libro para preparar, sin generar audio masivo. ".repeat(2200);
        assert!(large.chars().count() > 100_000);
        let large_plan = prepare(
            &root,
            &large,
            &hash(large.as_bytes()),
            origin.clone(),
            settings.clone(),
            cancel.clone(),
        )
        .await
        .unwrap();
        assert_eq!(large_plan.text, large);
        assert_eq!(plan_status(&root, &large_plan.id).unwrap().completed, 0);
        let text="Primer segmento sintético para comprobar una pausa local.\n\nSegundo segmento sintético, distinto del primero.\n\nTercer segmento sintético para completar la prueba de reanudación.\n\nCuarto segmento sintético que termina esta narración local.";
        let plan = prepare(
            &root,
            text,
            &hash(text.as_bytes()),
            origin,
            settings,
            cancel.clone(),
        )
        .await
        .unwrap();
        let stage = root.join("paused");
        fs::create_dir(&stage).unwrap();
        let (sender, stopped) = watch::channel(false);
        let first = Arc::new(Mutex::new(None::<Vec<u8>>));
        let observed = first.clone();
        let plan_id = plan.id.clone();
        let source = root.clone();
        let result = execute(&root, &plan.id, &stage, stopped, 2, move |progress| {
            if progress.percent.is_some_and(|p| p > 0.0) {
                let mut first = observed.lock().unwrap();
                if first.is_none() {
                    let status = plan_status(&source, &plan_id).unwrap();
                    assert!(status.active && status.completed >= 1);
                    *first = Some(segment_audio(&source, &plan_id, 0).unwrap());
                    let _ = sender.send(true);
                }
            }
        })
        .await;
        assert!(result.is_err());
        assert!(first.lock().unwrap().is_some());
        assert!(plan_status(&root, &plan.id).unwrap().completed >= 1);
        assert!(!plan_status(&root, &plan.id).unwrap().active);
        let stage = root.join("resumed");
        fs::create_dir(&stage).unwrap();
        let output = execute(&root, &plan.id, &stage, cancel.clone(), 3, |_| {})
            .await
            .unwrap();
        assert_eq!(output.chapters.len(), 4);
        assert_eq!(plan_status(&root, &plan.id).unwrap().completed, 4);
        assert_eq!(
            segment_audio(&root, &plan.id, 0).unwrap(),
            first.lock().unwrap().clone().unwrap()
        );
        let actual = wave::inspect(&output.file).unwrap().duration();
        assert!((output.chapters.last().unwrap().end - actual).abs() < 1e-9);
        assert_eq!(fs::read_dir(&stage).unwrap().count(), 1);
        let persisted = read_plan(&root, &plan.id).unwrap();
        assert_eq!(persisted.source_version, plan.source_version);
        assert_eq!(persisted.text_hash, hash(text.as_bytes()));
        assert_eq!(persisted.voice_name, voice.name);
        assert_eq!(persisted.language, voice.language);
        assert_eq!(persisted.origin.title, "Prueba local de voz");
        fs::remove_dir_all(root).unwrap();
    }
}
