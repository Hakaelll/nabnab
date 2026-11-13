use super::{
    cache, cancelled, generate, segment_key_fields, status, wave, Sample, VoiceSettings, Work,
};
use crate::{captions, engine, model::Progress, translation::Draft, workspaces::Workspace};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::{Seek, SeekFrom, Write},
    path::Path,
};
use tokio::sync::watch;

const MAX_SECONDS: f64 = 2.0 * 60.0 * 60.0;
const MAX_TEMPO: f64 = 1.25;

struct Slot {
    index: usize,
    start: u64,
    end: u64,
    text: String,
}

pub fn validate(settings: &VoiceSettings) -> Result<(), String> {
    super::validate_settings(settings)?;
    if !(-3..=3).contains(&settings.rate) {
        return Err("Choose a voice rate between −3 and +3 for timed speech.".into());
    }
    Ok(())
}

fn samples(seconds: f64) -> u64 {
    (seconds * f64::from(wave::SAMPLE_RATE)).round() as u64
}

fn timing_error(indices: impl IntoIterator<Item = usize>) -> String {
    format!(
        "DUB_TIMING {}",
        indices
            .into_iter()
            .take(16)
            .map(|index| (index + 1).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn speech(text: &str, index: usize) -> Result<String, String> {
    let text = captions::readable_text(text);
    if text.is_empty() || text.chars().count() > super::SEGMENT_CHARS {
        return Err(timing_error([index]));
    }
    Ok(text)
}

fn windows(draft: &Draft) -> Result<(Vec<(usize, u64, u64)>, u64), String> {
    if draft.original.is_empty()
        || draft.original.len() > super::MAX_SEGMENTS
        || draft.original.len() != draft.translated.len()
    {
        return Err("Timed speech supports up to 4096 translated cues per file.".into());
    }
    let mut sorted = draft.original.iter().enumerate().collect::<Vec<_>>();
    sorted.sort_by(|(_, left), (_, right)| left.segment.start.total_cmp(&right.segment.start));
    let mut duration = 0;
    let mut result = Vec::with_capacity(sorted.len());
    for (position, &(index, cue)) in sorted.iter().enumerate() {
        let start = cue.segment.start;
        let end = cue.segment.end;
        if !start.is_finite()
            || !end.is_finite()
            || start < 0.0
            || end > MAX_SECONDS
            || end <= start
        {
            return Err(
                "Timed speech requires valid cue intervals within the first two hours.".into(),
            );
        }
        duration = duration.max(samples(end));
        let boundary = sorted
            .get(position + 1)
            .map_or(end, |(_, next)| end.min(next.segment.start));
        result.push((index, samples(start), samples(boundary)));
    }
    Ok((result, duration))
}

fn slots(draft: &Draft) -> Result<(Vec<Slot>, u64), String> {
    let (windows, duration) = windows(draft)?;
    let mut result = Vec::with_capacity(windows.len());
    for (index, start, end) in windows {
        if end <= start {
            return Err(timing_error([index]));
        }
        result.push(Slot {
            index,
            start,
            end,
            text: speech(&draft.translated[index], index)?,
        });
    }
    Ok((result, duration))
}

async fn fingerprint(
    root: &Path,
    settings: &VoiceSettings,
    language: &str,
    cancel: watch::Receiver<bool>,
) -> Result<String, String> {
    validate(settings)?;
    status(root, cancel)
        .await?
        .voices
        .into_iter()
        .find(|voice| {
            voice.enabled
                && voice.id == settings.voice_id
                && voice.language.split('-').next() == Some(language)
        })
        .map(|voice| voice.fingerprint)
        .ok_or_else(|| {
            "Choose an installed Windows desktop voice for the translation language.".into()
        })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedSample {
    #[serde(flatten)]
    voice: Sample,
    speech_seconds: f64,
    available_seconds: f64,
    fit: Fit,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
enum Fit {
    Fits,
    NeedsAcceleration,
    TooLong,
}

pub async fn sample(
    root: &Path,
    engines: &Path,
    draft: &Draft,
    index: usize,
    settings: &VoiceSettings,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<TimedSample, String> {
    let _cache = cache::IN_USE.read().await;
    let (windows, _) = windows(draft)?;
    let (_, start, end) = windows.into_iter().find(|(cue, _, _)| *cue == index)
        .ok_or("Choose a cue from this translation.")?;
    let text = draft
        .translated
        .get(index)
        .ok_or("Choose a cue from this translation.")?;
    let text = speech(text, index)?;
    let (_, language) = draft.selection.pair.languages();
    let fingerprint = fingerprint(root, settings, language, cancel.clone()).await?;
    let key = segment_key_fields(settings, &fingerprint, &text)?;
    if cache::cached(root, &key, true, &cancel).is_none() {
        generate(
            root,
            settings,
            &fingerprint,
            vec![Work {
                key: key.clone(),
                text: text.clone(),
                weight: 1,
            }],
            0,
            1,
            0,
            cancel.clone(),
            run_id,
            |_| {},
        )
        .await?;
    }
    let cached =
        cache::cached(root, &key, true, &cancel).ok_or("The sample could not be verified.")?;
    let length = cached.wave.length / 2;
    let available = end.saturating_sub(start);
    let (bytes, fit) = match tempo(length, available) {
        Some(1.0) => (wave::sample_bytes(&cached.path, 12)?, Fit::Fits),
        Some(rate) => {
            let scratch = Workspace::new(root)?;
            let fitted = scratch.path.join("fitted.wav");
            let audio = accelerate(engines, &cached.path, &fitted, rate, cancel.clone()).await?;
            let fit = if audio.length / 2 <= available { Fit::NeedsAcceleration } else { Fit::TooLong };
            (wave::sample_bytes(&fitted, 12)?, fit)
        }
        None => (wave::sample_bytes(&cached.path, 12)?, Fit::TooLong),
    };
    let (wav, duration, truncated) = bytes;
    Ok(TimedSample {
        voice: Sample { wav, duration, segment_text: text, truncated },
        speech_seconds: length as f64 / f64::from(wave::SAMPLE_RATE),
        available_seconds: available as f64 / f64::from(wave::SAMPLE_RATE),
        fit,
    })
}

async fn accelerate(
    engines: &Path,
    input: &Path,
    output: &Path,
    rate: f64,
    cancel: watch::Receiver<bool>,
) -> Result<wave::Wave, String> {
    engine::tool(
        engines,
        "ffmpeg.exe",
        vec![
            "-v".into(), "error".into(), "-y".into(), "-i".into(), input.to_string_lossy().into_owned(),
            "-af".into(), format!("atempo={rate:.8}"), "-c:a".into(), "pcm_s16le".into(), output.to_string_lossy().into_owned(),
        ],
        cancel,
    ).await?;
    wave::inspect(output)
}

fn tempo(length: u64, available: u64) -> Option<f64> {
    if available == 0 { return None; }
    let ratio = length as f64 / available as f64;
    (ratio <= MAX_TEMPO).then(|| {
        if ratio <= 1.0 {
            1.0
        } else {
            (ratio * 1.015).min(MAX_TEMPO)
        }
    })
}

pub async fn render(
    root: &Path,
    engines: &Path,
    staging: &Path,
    draft: &Draft,
    settings: &VoiceSettings,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<std::path::PathBuf, String> {
    let _cache = cache::IN_USE.read().await;
    let (slots, duration) = slots(draft)?;
    let (_, language) = draft.selection.pair.languages();
    let fingerprint = fingerprint(root, settings, language, cancel.clone()).await?;
    let directory = cache::directory(root, "voice-cache")?;
    let mut keys = Vec::with_capacity(slots.len());
    let mut work = BTreeMap::<String, Work>::new();
    let mut existing = BTreeMap::new();
    for slot in &slots {
        cancelled(&cancel)?;
        let key = segment_key_fields(settings, &fingerprint, &slot.text)?;
        let cached = existing
            .entry(key.clone())
            .or_insert_with(|| cache::cached_at(&directory, &key, true, &cancel));
        if cached.is_none() {
            work.entry(key.clone())
                .and_modify(|item| item.weight += 1)
                .or_insert_with(|| Work {
                    key: key.clone(),
                    text: slot.text.clone(),
                    weight: 1,
                });
        }
        keys.push(key);
    }
    let complete = slots.len() - work.values().map(|item| item.weight).sum::<usize>();
    generate(
        root,
        settings,
        &fingerprint,
        work.into_values().collect(),
        complete,
        slots.len(),
        0,
        cancel.clone(),
        run_id,
        emit,
    )
    .await?;
    let mut durations = Vec::with_capacity(slots.len());
    let mut overlong = Vec::new();
    for (slot, key) in slots.iter().zip(&keys) {
        cancelled(&cancel)?;
        let cached = existing.get_mut(key).ok_or("Missing speech cache key.")?;
        if cached.is_none() {
            *cached = cache::cached_at(&directory, key, true, &cancel);
        }
        let length = cached
            .as_ref()
            .ok_or("A synthesized cue could not be verified.")?
            .wave
            .length
            / 2;
        let rate = tempo(length, slot.end - slot.start);
        if rate.is_none() {
            overlong.push(slot.index);
        }
        durations.push(rate.unwrap_or(1.0));
    }
    if !overlong.is_empty() {
        return Err(timing_error(overlong));
    }

    let scratch = Workspace::new(staging)?;
    let compressed = scratch.path.join("fitted.wav");
    let output = staging.join("Translated voice.wav");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .map_err(|error| error.to_string())?;
    file.write_all(&wave::header(duration * 2)?)
        .map_err(|error| error.to_string())?;
    // The unwritten PCM regions in this newly created file are zero-filled silence.
    file.set_len(44 + duration * 2)
        .map_err(|error| error.to_string())?;
    for ((slot, key), rate) in slots.iter().zip(keys).zip(durations) {
        cancelled(&cancel)?;
        let cached = existing
            .get(&key)
            .and_then(Option::as_ref)
            .ok_or("A cached cue is unavailable.")?;
        let (path, audio) = if rate > 1.0 {
            (&compressed, accelerate(engines, &cached.path, &compressed, rate, cancel.clone()).await?)
        } else {
            (&cached.path, cached.wave.clone())
        };
        if audio.length / 2 > slot.end - slot.start {
            return Err(timing_error([slot.index]));
        }
        file.seek(SeekFrom::Start(44 + slot.start * 2))
            .map_err(|error| error.to_string())?;
        wave::copy_samples(path, &audio, &mut file, &cancel)?;
    }
    file.sync_all().map_err(|error| error.to_string())?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_speech_never_requests_more_than_twenty_five_percent_acceleration() {
        assert_eq!(tempo(800, 1000), Some(1.0));
        assert!((1.18..=1.25).contains(&tempo(1200, 1000).unwrap()));
        assert_eq!(tempo(1251, 1000), None);
        assert_eq!(timing_error([0, 3]), "DUB_TIMING 1, 4");
    }
}
