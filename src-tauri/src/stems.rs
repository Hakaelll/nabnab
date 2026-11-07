pub mod commands;
pub mod component;
#[cfg(windows)]
pub mod worker;

use crate::{engine, library, media};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio::sync::watch;

const SAMPLE_RATE: usize = 44_100;
const MAX_SECONDS: usize = 30 * 60;
const MAX_FRAMES: usize = MAX_SECONDS * SAMPLE_RATE;
pub const SAMPLE_SECONDS: usize = 8;
pub const STEMS: [&str; 2] = ["Vocals", "Instrumental"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub library_id: String,
    pub input_hash: String,
}

impl Selection {
    pub fn validate(&self) -> Result<(), String> {
        if ![&self.library_id, &self.input_hash]
            .into_iter()
            .all(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err("Invalid audio selection.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reference {
    pub set_id: String,
    pub role: String,
    pub model: String,
}

pub struct ResultAudio {
    pub seconds: f64,
    pub channels: u8,
    pub original_channels: u64,
    pub original: Option<PathBuf>,
    pub tracks: Vec<PathBuf>,
}

async fn encode(
    engines: &Path,
    raw: &Path,
    output: &Path,
    channels: u8,
    frames: usize,
    title: Option<&str>,
    cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    let mut args: Vec<String> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-n",
        "-f",
        "f32le",
        "-ar",
        "44100",
        "-ac",
        "2",
        "-i",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(raw.to_string_lossy().into_owned());
    let mix = if channels == 1 {
        "pan=mono|c0=0.5*c0+0.5*c1,"
    } else {
        ""
    };
    let original = output.file_stem().is_some_and(|stem| stem == "Original");
    let limiter = if original {
        ""
    } else {
        "alimiter=limit=0.98:level=0:latency=1,"
    };
    args.extend([
        "-af".into(),
        format!("{mix}{limiter}atrim=end_sample={frames}"),
    ]);
    if output
        .extension()
        .is_some_and(|extension| extension == "flac")
    {
        args.extend([
            "-c:a".into(),
            "flac".into(),
            "-sample_fmt".into(),
            "s16".into(),
        ]);
        if let Some(title) = title {
            let role = output.file_stem().unwrap_or_default().to_string_lossy();
            args.extend(["-metadata".into(), format!("title={title} · {role}")]);
        }
    } else {
        args.extend(["-c:a".into(), "pcm_s16le".into()]);
    }
    args.push(output.to_string_lossy().into_owned());
    engine::tool(engines, "ffmpeg.exe", args, cancel.clone()).await?;
    let info = media::probe(engines, output, cancel).await?;
    let audio = &info["streams"][0];
    if audio["codec_type"] != "audio"
        || audio["sample_rate"].as_str() != Some("44100")
        || audio["channels"].as_u64() != Some(channels.into())
        || audio["duration_ts"].as_u64() != Some(frames as u64)
    {
        return Err("The separated tracks could not be aligned to the source audio.".into());
    }
    Ok(())
}

pub async fn separate(
    engines: &Path,
    runtime: &Path,
    scratch: &Path,
    output: &Path,
    source: &Path,
    start: Option<f64>,
    cancel: watch::Receiver<bool>,
) -> Result<ResultAudio, String> {
    let source = library::authorized_path(source)?;
    let info = media::probe(engines, &source, cancel.clone()).await?;
    let stream = info["streams"]
        .as_array()
        .and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream["codec_type"] == "audio")
        })
        .ok_or("Choose a file with an audio track.")?;
    let original_channels = stream["channels"]
        .as_u64()
        .filter(|&count| count > 0)
        .ok_or("The audio channel layout could not be read.")?;
    let channels = original_channels.min(2) as u8;
    let mix = scratch.join("mix.f32");
    let mut args: Vec<String> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-nostdin",
        "-n",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        "mov,matroska,webm,mp3,ogg,wav,flac,aac,avi,asf,flv,mpeg,mpegts,nut,ac3,eac3,aiff,ape",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if let Some(start) = start {
        if !start.is_finite() || !(0.0..=MAX_SECONDS as f64).contains(&start) {
            return Err("Choose a valid sample start time in seconds.".into());
        }
        args.extend(["-ss".into(), start.to_string()]);
    }
    args.extend(["-i".into(), source.to_string_lossy().into_owned()]);
    let seconds = if start.is_some() {
        SAMPLE_SECONDS
    } else {
        MAX_SECONDS + 1
    };
    args.extend([
        "-map".into(),
        "0:a:0".into(),
        "-t".into(),
        seconds.to_string(),
    ]);
    if channels == 1 {
        args.extend(["-af".into(), "pan=stereo|c0=c0|c1=c0".into()]);
    }
    args.extend([
        "-ar".into(),
        "44100".into(),
        "-ac".into(),
        "2".into(),
        "-f".into(),
        "f32le".into(),
        mix.to_string_lossy().into_owned(),
    ]);
    engine::tool(engines, "ffmpeg.exe", args, cancel.clone()).await?;
    let bytes = crate::portable_zip::regular_file(&mix)?;
    let frames = (bytes / 8) as usize;
    if frames == 0 || bytes % 8 != 0 {
        return Err(
            "This fragment contains no readable audio. Choose an earlier sample start.".into(),
        );
    }
    if frames > MAX_FRAMES {
        return Err("This component can separate up to 30 minutes of audio per file.".into());
    }
    let args = [
        "--internal-stems",
        &runtime.to_string_lossy(),
        &mix.to_string_lossy(),
        &scratch.to_string_lossy(),
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    engine::run_process(executable, args, cancel.clone(), 0, false, |_| {}).await?;
    let original = if start.is_some() {
        let path = output.join("Original.wav");
        encode(engines, &mix, &path, channels, frames, None, cancel.clone()).await?;
        Some(path)
    } else {
        None
    };
    fs::remove_file(mix).map_err(|error| error.to_string())?;
    let title = source.file_stem().unwrap_or_default().to_string_lossy();
    let mut tracks = Vec::new();
    for stem in STEMS {
        let raw = scratch.join(format!("{stem}.f32"));
        if crate::portable_zip::regular_file(&raw)? != bytes {
            return Err("The separation worker returned incomplete audio.".into());
        }
        let extension = if start.is_some() { "wav" } else { "flac" };
        let path = output.join(format!("{stem}.{extension}"));
        encode(
            engines,
            &raw,
            &path,
            channels,
            frames,
            Some(&title),
            cancel.clone(),
        )
        .await?;
        fs::remove_file(raw).map_err(|error| error.to_string())?;
        tracks.push(path);
    }
    Ok(ResultAudio {
        seconds: frames as f64 / SAMPLE_RATE as f64,
        channels,
        original_channels,
        original,
        tracks,
    })
}
