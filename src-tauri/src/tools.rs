use crate::{
    engine,
    model::{Format, Progress},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio::sync::watch;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ToolOptions {
    Convert {
        container: String,
        video_codec: String,
        audio_codec: String,
    },
    ExtractAudio {
        format: String,
    },
    Trim {
        start: f64,
        end: f64,
        precise: bool,
    },
    Compress {
        target_mb: Option<f64>,
        quality: String,
    },
}

// Local media must not turn into an HLS, DASH, concat or network input after probing.
const LOCAL_FORMATS: &str =
    "mov,matroska,webm,mp3,ogg,wav,flac,aac,avi,asf,flv,mpeg,mpegts,nut,ac3,eac3,aiff,ape";

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .filter(|n: &f64| n.is_finite())
}
fn track<'a>(info: &'a Value, kind: &str) -> Option<&'a Value> {
    info["streams"].as_array()?.iter().find(|s| {
        s["codec_type"] == kind && (kind != "video" || s["disposition"]["attached_pic"] != 1)
    })
}
fn codec<'a>(stream: Option<&'a Value>) -> Option<&'a str> {
    stream
        .and_then(|s| s["codec_name"].as_str())
        .filter(|s| !s.is_empty())
}
fn duration(info: &Value) -> Result<f64, String> {
    number(&info["format"]["duration"])
        .filter(|n| *n > 0.0)
        .ok_or_else(|| {
            "The file has no valid duration. Choose a finished audio or video file.".into()
        })
}
fn copied_video_fits(container: &str, codec: &str) -> bool {
    match container {
        "mp4" => matches!(codec, "h264" | "hevc" | "mpeg4" | "av1"),
        "webm" => matches!(codec, "vp8" | "vp9" | "av1"),
        "mkv" => true,
        _ => false,
    }
}
fn copied_audio_fits(container: &str, codec: &str) -> bool {
    match container {
        "mp4" => matches!(codec, "aac" | "mp3"),
        "webm" => matches!(codec, "opus" | "vorbis"),
        "mkv" => true,
        _ => false,
    }
}
fn original_audio(codec: &str) -> Result<(&'static str, &'static str), String> {
    match codec {
        "mp3" => Ok(("mp3", "mp3")),
        "aac" | "alac" => Ok(("m4a", "mp4")),
        "opus" => Ok(("opus", "ogg")),
        "vorbis" => Ok(("ogg", "ogg")),
        "flac" => Ok(("flac", "flac")),
        "pcm_s16le" | "pcm_s24le" | "pcm_s32le" | "pcm_f32le" | "pcm_f64le" => Ok(("wav", "wav")),
        _ => Err(
            "Original audio cannot be saved in a supported container. Choose MP3 or M4A instead."
                .into(),
        ),
    }
}

pub fn validate_options(options: &ToolOptions, info: &Value) -> Result<(), String> {
    let length = duration(info)?;
    let video = codec(track(info, "video"));
    let audio = codec(track(info, "audio"));
    if video.is_none() && audio.is_none() {
        return Err("Choose a file with a supported video or audio track.".into());
    }
    match options {
        ToolOptions::Convert {
            container,
            video_codec,
            audio_codec,
        } => {
            let video = video.ok_or(
                "Convert video requires a video track. Use Extract audio for an audio file.",
            )?;
            if !matches!(container.as_str(), "mp4" | "mkv" | "webm") {
                return Err("Choose MP4, MKV or WebM as the output container.".into());
            }
            if !matches!(video_codec.as_str(), "copy" | "h264" | "vp9")
                || !matches!(audio_codec.as_str(), "copy" | "aac" | "opus" | "mp3")
            {
                return Err("Choose one of the supported video and audio codecs.".into());
            }
            let effective_video = if video_codec == "copy" {
                video
            } else {
                video_codec
            };
            if !copied_video_fits(container, effective_video) {
                return Err("This video codec does not fit the selected container. Choose MKV or another codec.".into());
            }
            if let Some(audio) = audio {
                let effective_audio = if audio_codec == "copy" {
                    audio
                } else {
                    audio_codec
                };
                if !copied_audio_fits(container, effective_audio) {
                    return Err("This audio codec does not fit the selected container. Choose MKV or another codec.".into());
                }
            }
        }
        ToolOptions::ExtractAudio { format } => {
            let audio = audio.ok_or("The selected file has no audio track.")?;
            match format.as_str() {
                "original" => {
                    original_audio(audio)?;
                }
                "mp3" | "m4a" => {}
                _ => return Err("Choose original audio, MP3 or M4A.".into()),
            }
        }
        ToolOptions::Trim { start, end, .. } => {
            if !start.is_finite()
                || !end.is_finite()
                || *start < 0.0
                || *end <= *start
                || *end > length + 0.05
            {
                return Err(
                    "Choose a start and end inside the file, with the end after the start.".into(),
                );
            }
            if video.is_none() {
                original_audio(audio.unwrap_or_default())?;
            }
        }
        ToolOptions::Compress { target_mb, quality } => {
            if video.is_none() {
                return Err("Video compression requires a video track.".into());
            }
            if !matches!(quality.as_str(), "balanced" | "small" | "high") {
                return Err("Choose balanced, small or high quality.".into());
            }
            if let Some(target) = target_mb {
                if !target.is_finite() || *target <= 0.0 || *target > 1_000_000.0 {
                    return Err("Choose a positive target size up to 1,000,000 MiB.".into());
                }
                compression_bitrate(info, *target, audio.is_some())?;
            }
        }
    }
    Ok(())
}

fn compression_bitrate(info: &Value, target_mb: f64, has_audio: bool) -> Result<u64, String> {
    let rate = target_mb * 1024.0 * 1024.0 * 8.0 * 0.95 / duration(info)?
        - if has_audio { 128_000.0 } else { 0.0 };
    if !(100_000.0..=200_000_000.0).contains(&rate) {
        return Err("The target size is outside the supported bitrate range for this duration. Choose another target size.".into());
    }
    Ok(rate.round() as u64)
}

struct Plan {
    args: Vec<String>,
    extension: &'static str,
    container: &'static str,
    suffix: &'static str,
    video: Option<String>,
    audio: Option<String>,
    expected_duration: Option<f64>,
}
fn add(args: &mut Vec<String>, values: &[&str]) {
    args.extend(values.iter().map(|s| (*s).to_owned()));
}
fn map_track(args: &mut Vec<String>, stream: &Value) -> Result<(), String> {
    let index = stream["index"]
        .as_u64()
        .ok_or("FFprobe returned an invalid stream index.")?;
    args.extend(["-map".into(), format!("0:{index}")]);
    Ok(())
}
fn encode_video(args: &mut Vec<String>, codec: &str) {
    add(
        args,
        &[
            "-c:v",
            match codec {
                "h264" => "libopenh264",
                "vp9" => "libvpx-vp9",
                _ => "copy",
            },
        ],
    );
    if codec != "copy" {
        add(args, &["-pix_fmt", "yuv420p", "-b:v", "8M"]);
    }
}
fn encode_audio(args: &mut Vec<String>, codec: &str) {
    add(
        args,
        &[
            "-c:a",
            match codec {
                "mp3" => "libmp3lame",
                "opus" => "libopus",
                "aac" => "aac",
                _ => "copy",
            },
        ],
    );
    if codec != "copy" {
        add(args, &["-b:a", "192k"]);
    }
}

fn initial_plan(info: &Value) -> Result<Plan, String> {
    Ok(Plan {
        args: Vec::new(),
        extension: "mp4",
        container: "mp4",
        suffix: "converted",
        video: codec(track(info, "video")).map(str::to_owned),
        audio: codec(track(info, "audio")).map(str::to_owned),
        expected_duration: Some(duration(info)?),
    })
}

fn configure_audio(p: &mut Plan, format: &str, stream: &Value) -> Result<(), String> {
    p.suffix = "audio";
    p.video = None;
    map_track(&mut p.args, stream)?;
    let source = stream["codec_name"]
        .as_str()
        .ok_or("FFprobe did not identify the audio codec.")?;
    let encoder = match format {
        "mp3" => {
            p.extension = "mp3";
            p.container = "mp3";
            p.audio = Some("mp3".into());
            if source == "mp3" {
                "copy"
            } else {
                "mp3"
            }
        }
        "m4a" => {
            p.extension = "m4a";
            p.container = "mp4";
            p.audio = Some("aac".into());
            if source == "aac" {
                "copy"
            } else {
                "aac"
            }
        }
        "opus" => {
            p.extension = "opus";
            p.container = "ogg";
            p.audio = Some("opus".into());
            if source == "opus" {
                "copy"
            } else {
                "opus"
            }
        }
        "original" => {
            let (extension, container) = original_audio(source)?;
            p.extension = extension;
            p.container = container;
            "copy"
        }
        _ => return Err("Unsupported audio output profile.".into()),
    };
    add(&mut p.args, &["-vn"]);
    encode_audio(&mut p.args, encoder);
    Ok(())
}

fn finish_plan(p: &mut Plan) {
    add(&mut p.args, &["-sn", "-dn", "-map_metadata", "-1"]);
    if p.container == "mp4" {
        add(&mut p.args, &["-movflags", "+faststart"]);
    }
}

fn plan(options: &ToolOptions, info: &Value) -> Result<Plan, String> {
    validate_options(options, info)?;
    let video_track = track(info, "video");
    let audio_track = track(info, "audio");
    let video = codec(video_track);
    let audio = codec(audio_track);
    let mut p = initial_plan(info)?;
    match options {
        ToolOptions::Convert {
            container,
            video_codec,
            audio_codec,
        } => {
            match container.as_str() {
                "mkv" => {
                    p.extension = "mkv";
                    p.container = "matroska";
                }
                "webm" => {
                    p.extension = "webm";
                    p.container = "webm";
                }
                _ => {}
            }
            if let Some(stream) = video_track {
                map_track(&mut p.args, stream)?;
            }
            if let Some(stream) = audio_track {
                map_track(&mut p.args, stream)?;
            }
            encode_video(&mut p.args, video_codec);
            if audio.is_some() {
                encode_audio(&mut p.args, audio_codec);
            }
            if video_codec != "copy" {
                p.video = Some(video_codec.clone());
            }
            if audio.is_some() && audio_codec != "copy" {
                p.audio = Some(audio_codec.clone());
            }
        }
        ToolOptions::ExtractAudio { format } => {
            configure_audio(
                &mut p,
                format,
                audio_track.ok_or("The selected file has no audio track.")?,
            )?;
        }
        ToolOptions::Trim {
            start,
            end,
            precise,
        } => {
            p.suffix = "trimmed";
            if let Some(stream) = video_track {
                map_track(&mut p.args, stream)?;
            }
            if let Some(stream) = audio_track {
                map_track(&mut p.args, stream)?;
            }
            p.args.extend(["-t".into(), format!("{:.6}", end - start)]);
            if *precise {
                if video.is_some() {
                    encode_video(&mut p.args, "h264");
                    p.video = Some("h264".into());
                }
                if audio.is_some() {
                    encode_audio(&mut p.args, "aac");
                    p.audio = Some("aac".into());
                }
                if video.is_none() {
                    p.extension = "m4a";
                }
                p.expected_duration = Some(end - start);
            } else {
                add(
                    &mut p.args,
                    &["-c", "copy", "-avoid_negative_ts", "make_zero"],
                );
                if video.is_some() {
                    p.extension = "mkv";
                    p.container = "matroska";
                } else {
                    let (extension, container) = original_audio(audio.unwrap_or_default())?;
                    p.extension = extension;
                    p.container = container;
                }
                p.expected_duration = None;
            }
        }
        ToolOptions::Compress { target_mb, quality } => {
            p.suffix = "compressed";
            if let Some(stream) = video_track {
                map_track(&mut p.args, stream)?;
            }
            if let Some(stream) = audio_track {
                map_track(&mut p.args, stream)?;
            }
            let bitrate = if let Some(target) = target_mb {
                compression_bitrate(info, *target, audio.is_some())?
            } else {
                let factor = match quality.as_str() {
                    "high" => 0.75,
                    "small" => 0.35,
                    _ => 0.55,
                };
                let rate = number(&info["format"]["bit_rate"])
                    .or_else(|| {
                        number(&info["format"]["size"])
                            .map(|size| size * 8.0 / duration(info).unwrap_or(1.0))
                    })
                    .unwrap_or(8_000_000.0);
                (rate * factor).clamp(100_000.0, 200_000_000.0).round() as u64
            };
            add(
                &mut p.args,
                &["-c:v", "libopenh264", "-pix_fmt", "yuv420p", "-b:v"],
            );
            p.args.push(bitrate.to_string());
            p.video = Some("h264".into());
            if audio.is_some() {
                add(&mut p.args, &["-c:a", "aac", "-b:a", "128k"]);
                p.audio = Some("aac".into());
            }
        }
    }
    finish_plan(&mut p);
    Ok(p)
}

fn download_audio_plan(format: Format, bitrate: Option<u16>, info: &Value) -> Result<Plan, String> {
    let requested = match format {
        Format::Mp3 => "mp3",
        Format::M4a => "m4a",
        Format::Opus => "opus",
        Format::AudioOriginal => "original",
        Format::Mp4 | Format::Source => return Err("Choose an audio output profile.".into()),
    };
    if bitrate
        .is_some_and(|bitrate| format != Format::Mp3 || !matches!(bitrate, 128 | 192 | 256 | 320))
    {
        return Err("Choose a supported MP3 bitrate.".into());
    }
    let mut p = initial_plan(info)?;
    configure_audio(
        &mut p,
        requested,
        track(info, "audio").ok_or("The selected file has no audio track.")?,
    )?;
    if format == Format::Mp3 {
        if let Some(index) = p.args.iter().position(|argument| argument == "-b:a") {
            p.args[index + 1] = format!("{}k", bitrate.unwrap_or(192));
        }
    }
    finish_plan(&mut p);
    Ok(p)
}

fn safe_stem(input: &Path) -> String {
    let raw = input.file_stem().unwrap_or_default().to_string_lossy();
    let value: String = raw
        .chars()
        .filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c))
        .take(100)
        .collect();
    let value = value.trim_matches([' ', '.']);
    if value.is_empty() {
        "media".into()
    } else {
        value.into()
    }
}
fn checked_paths(input: &Path, staging: &Path) -> Result<(PathBuf, PathBuf), String> {
    let text = input.to_string_lossy();
    let network_path = text.starts_with("\\\\") && !text.starts_with("\\\\?\\");
    let extended_network_path = text.to_ascii_lowercase().starts_with("\\\\?\\unc\\");
    if text.contains("://") || network_path || extended_network_path {
        return Err("Choose an authorized local media file.".into());
    }
    if !input.is_absolute()
        || !staging.is_absolute()
        || fs::symlink_metadata(input)
            .map_err(|_| "The input file was not found. Locate it in the library.")?
            .file_type()
            .is_symlink()
        || fs::symlink_metadata(staging)
            .map_err(|_| "The job staging folder is unavailable.")?
            .file_type()
            .is_symlink()
    {
        return Err(
            "The input and staging folder must be absolute local paths without symbolic links."
                .into(),
        );
    }
    let input =
        fs::canonicalize(input).map_err(|e| format!("Could not open the input file: {e}"))?;
    let staging =
        fs::canonicalize(staging).map_err(|e| format!("Could not open the staging folder: {e}"))?;
    if !input.is_file() || !staging.is_dir() {
        return Err("Choose an existing file and a valid job staging folder.".into());
    }
    Ok((input, staging))
}
async fn probe_local(
    root: &Path,
    file: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<Value, String> {
    let mut args = Vec::new();
    add(
        &mut args,
        &[
            "-v",
            "error",
            "-protocol_whitelist",
            "file,pipe",
            "-format_whitelist",
            LOCAL_FORMATS,
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ],
    );
    args.push(file.to_string_lossy().into_owned());
    let raw = engine::tool(root, "ffprobe.exe", args, cancel).await?;
    serde_json::from_str(&raw)
        .map_err(|_| "FFprobe returned invalid local media information.".into())
}
fn phase(emit: &impl Fn(Progress), run_id: u64, phase: &str) {
    emit(Progress {
        request_id: run_id,
        phase: phase.into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
}
fn verify(info: &Value, p: &Plan) -> Result<(), String> {
    let actual_duration = duration(info)?;
    if number(&info["format"]["size"]).is_none_or(|n| n <= 0.0) {
        return Err("FFprobe found an empty output. The file was not published.".into());
    }
    if codec(track(info, "video")) != p.video.as_deref()
        || codec(track(info, "audio")) != p.audio.as_deref()
    {
        return Err(
            "The output codecs do not match the selected operation. The file was not published."
                .into(),
        );
    }
    if !info["format"]["format_name"]
        .as_str()
        .unwrap_or_default()
        .split(',')
        .any(|c| c == p.container)
    {
        return Err(
            "The output container does not match its extension. The file was not published.".into(),
        );
    }
    if let Some(expected) = p.expected_duration {
        let tolerance = (expected * 0.02).max(0.25);
        if (actual_duration - expected).abs() > tolerance {
            return Err("The output duration does not match the selected operation. The file was not published.".into());
        }
    }
    Ok(())
}

pub async fn execute(
    root: &Path,
    input: &Path,
    staging: &Path,
    options: &ToolOptions,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let (input, staging) = checked_paths(input, staging)?;
    let info = probe_local(root, &input, cancel.clone()).await?;
    let p = plan(options, &info)?;
    let seek = match options {
        ToolOptions::Trim { start, precise, .. } => Some((*start, *precise)),
        _ => None,
    };
    execute_plan(root, input, staging, p, seek, cancel, run_id, emit).await
}

pub async fn execute_download_audio(
    root: &Path,
    input: &Path,
    staging: &Path,
    format: Format,
    bitrate: Option<u16>,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let (input, staging) = checked_paths(input, staging)?;
    let info = probe_local(root, &input, cancel.clone()).await?;
    let p = download_audio_plan(format, bitrate, &info)?;
    let output = planned_output(&input, &staging, &p);
    if output == input {
        return Err("The audio output cannot replace its original input.".into());
    }
    if fs::symlink_metadata(&output).is_ok() {
        let existing = crate::library::authorized_path(&output)?;
        if existing.parent() != Some(staging.as_path()) {
            return Err("The audio output escaped its job folder.".into());
        }
        phase(&emit, run_id, "verifying");
        if probe_local(root, &existing, cancel.clone())
            .await
            .is_ok_and(|info| verify(&info, &p).is_ok())
        {
            if *cancel.borrow() {
                return Err(engine::CANCELLED.into());
            }
            return Ok(existing);
        }
        if *cancel.borrow() {
            return Err(engine::CANCELLED.into());
        }
        let archive = output.with_file_name(format!(
            "{}.interrupted-{run_id}",
            output.file_name().unwrap_or_default().to_string_lossy()
        ));
        if fs::symlink_metadata(&archive).is_ok() {
            return Err(
                "An interrupted audio archive already exists. No file was replaced.".into(),
            );
        }
        fs::rename(&output, &archive).map_err(|_| {
            "Could not preserve the interrupted audio result. No original file was replaced."
        })?;
    }
    execute_plan(root, input, staging, p, None, cancel, run_id, emit).await
}

fn planned_output(input: &Path, staging: &Path, p: &Plan) -> PathBuf {
    staging.join(format!("{}.{}.{}", safe_stem(input), p.suffix, p.extension))
}

async fn execute_plan(
    root: &Path,
    input: PathBuf,
    staging: PathBuf,
    p: Plan,
    seek: Option<(f64, bool)>,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    let output = planned_output(&input, &staging, &p);
    if output == input || fs::symlink_metadata(&output).is_ok() {
        return Err(
            "The output already exists. Start a new job to preserve the original file.".into(),
        );
    }
    let mut args = Vec::new();
    add(
        &mut args,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-n",
            "-protocol_whitelist",
            "file,pipe",
            "-format_whitelist",
            LOCAL_FORMATS,
        ],
    );
    if let Some((start, false)) = seek {
        args.extend(["-ss".into(), format!("{start:.6}")]);
    }
    args.extend(["-i".into(), input.to_string_lossy().into_owned()]);
    if let Some((start, true)) = seek {
        args.extend(["-ss".into(), format!("{start:.6}")]);
    }
    args.extend(p.args.iter().cloned());
    args.push(output.to_string_lossy().into_owned());
    phase(&emit, run_id, "processing");
    engine::run_process(
        root.join("ffmpeg.exe"),
        args,
        cancel.clone(),
        run_id,
        false,
        emit.clone(),
    )
    .await?;
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    phase(&emit, run_id, "verifying");
    let output =
        fs::canonicalize(output).map_err(|_| "FFmpeg did not create the expected output.")?;
    if output.parent() != Some(staging.as_path())
        || !output.is_file()
        || fs::metadata(&output).map_err(|e| e.to_string())?.len() == 0
    {
        return Err("The output is empty or outside the job staging folder.".into());
    }
    verify(&probe_local(root, &output, cancel.clone()).await?, &p)?;
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info(video: Option<&str>, audio: Option<&str>) -> Value {
        let mut streams = Vec::new();
        if let Some(codec) = video {
            streams.push(json!({"index":0,"codec_type":"video","codec_name":codec,"width":1280,"height":720}));
        }
        if let Some(codec) = audio {
            streams.push(json!({"index":1,"codec_type":"audio","codec_name":codec}));
        }
        json!({"streams":streams,"format":{"duration":"10.0","size":"1000000","bit_rate":"800000","format_name":"mov,mp4,m4a,3gp,3g2,mj2"}})
    }
    #[test]
    fn tool_options_use_the_ipc_camel_case_contract() {
        let value =
            json!({"kind":"convert","container":"mp4","videoCodec":"h264","audioCodec":"aac"});
        let parsed: ToolOptions = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
        assert!(serde_json::from_value::<ToolOptions>(
            json!({"kind":"trim","start":0,"end":2,"precise":true,"command":"bad"})
        )
        .is_err());
    }
    #[test]
    fn copy_checks_container_compatibility_before_running_ffmpeg() {
        let source = info(Some("vp9"), Some("opus"));
        let options = ToolOptions::Convert {
            container: "mp4".into(),
            video_codec: "copy".into(),
            audio_codec: "copy".into(),
        };
        assert!(validate_options(&options, &source).is_err());
        let options = ToolOptions::Convert {
            container: "mkv".into(),
            video_codec: "copy".into(),
            audio_codec: "copy".into(),
        };
        let p = plan(&options, &source).unwrap();
        assert_eq!(
            (p.extension, p.video.as_deref(), p.audio.as_deref()),
            ("mkv", Some("vp9"), Some("opus"))
        );
        assert!(p.args.windows(2).any(|v| v == ["-c:v", "copy"]));
    }
    #[test]
    fn original_opus_is_saved_as_opus_and_mp3_requires_an_encoder() {
        let source = info(Some("h264"), Some("opus"));
        let original = plan(
            &ToolOptions::ExtractAudio {
                format: "original".into(),
            },
            &source,
        )
        .unwrap();
        assert_eq!(
            (
                original.extension,
                original.container,
                original.audio.as_deref()
            ),
            ("opus", "ogg", Some("opus"))
        );
        assert!(original.args.windows(2).any(|v| v == ["-c:a", "copy"]));
        let converted = plan(
            &ToolOptions::ExtractAudio {
                format: "mp3".into(),
            },
            &source,
        )
        .unwrap();
        assert_eq!(converted.audio.as_deref(), Some("mp3"));
        assert!(converted
            .args
            .windows(2)
            .any(|v| v == ["-c:a", "libmp3lame"]));
        assert!(plan(
            &ToolOptions::ExtractAudio {
                format: "original".into()
            },
            &info(None, Some("unknown"))
        )
        .is_err());
    }

    #[test]
    fn download_audio_profiles_keep_codecs_containers_and_effective_bitrates_honest() {
        let source = info(Some("h264"), Some("aac"));
        for bitrate in [128, 192, 256, 320] {
            let p = download_audio_plan(Format::Mp3, Some(bitrate), &source).unwrap();
            assert_eq!(
                (p.extension, p.container, p.audio.as_deref()),
                ("mp3", "mp3", Some("mp3"))
            );
            assert!(p.args.windows(2).any(|pair| pair == ["-c:a", "libmp3lame"]));
            assert!(p
                .args
                .windows(2)
                .any(|pair| pair[0] == "-b:a" && pair[1] == format!("{bitrate}k")));
        }
        let copied = download_audio_plan(Format::Mp3, Some(320), &info(None, Some("mp3"))).unwrap();
        assert!(copied.args.windows(2).any(|pair| pair == ["-c:a", "copy"]));
        assert!(!copied.args.iter().any(|argument| argument == "-b:a"));
        let opus = download_audio_plan(Format::Opus, None, &source).unwrap();
        assert_eq!(
            (opus.extension, opus.container, opus.audio.as_deref()),
            ("opus", "ogg", Some("opus"))
        );
        assert!(opus.args.windows(2).any(|pair| pair == ["-c:a", "libopus"]));
        let original =
            download_audio_plan(Format::AudioOriginal, None, &info(None, Some("opus"))).unwrap();
        assert_eq!((original.extension, original.container), ("opus", "ogg"));
        assert!(original
            .args
            .windows(2)
            .any(|pair| pair == ["-c:a", "copy"]));
        assert!(download_audio_plan(Format::Mp4, None, &source).is_err());
        assert!(download_audio_plan(Format::Mp3, Some(640), &source).is_err());
        assert!(download_audio_plan(Format::M4a, Some(128), &source).is_err());
        assert!(validate_options(
            &ToolOptions::ExtractAudio {
                format: "opus".into()
            },
            &source
        )
        .is_err());
    }

    #[tokio::test]
    #[ignore = "Requires bundled FFmpeg/ffprobe; verifies direct-download audio profiles with synthetic local media"]
    async fn real_engines_download_audio_profiles_preserve_input_and_recover_partial_results() {
        let engines = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engines");
        let work = std::env::temp_dir().join(format!(
            "nabnab-direct-audio-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        fs::create_dir_all(&work).unwrap();
        let input = work.join("Downloaded ñ original.wav");
        let (_sender, cancel) = watch::channel(false);
        let args = [
            "-v",
            "error",
            "-nostdin",
            "-n",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
            "-c:a",
            "pcm_s16le",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain([input.to_string_lossy().into_owned()])
        .collect();
        engine::tool(&engines, "ffmpeg.exe", args, cancel.clone())
            .await
            .unwrap();
        let original = fs::read(&input).unwrap();
        for (index, (format, bitrate, extension, codec, container)) in [
            (Format::Mp3, Some(128), "mp3", "mp3", "mp3"),
            (Format::M4a, None, "m4a", "aac", "mp4"),
            (Format::Opus, None, "opus", "opus", "ogg"),
            (Format::AudioOriginal, None, "wav", "pcm_s16le", "wav"),
        ]
        .into_iter()
        .enumerate()
        {
            let stage = work.join(format!("stage-{index}"));
            fs::create_dir(&stage).unwrap();
            let output = execute_download_audio(
                &engines,
                &input,
                &stage,
                format,
                bitrate,
                cancel.clone(),
                index as u64 + 1,
                |_| {},
            )
            .await
            .unwrap();
            assert_eq!(output.extension().unwrap(), extension);
            let info = probe_local(&engines, &output, cancel.clone())
                .await
                .unwrap();
            assert_eq!(info["streams"][0]["codec_name"], codec);
            assert!(info["format"]["format_name"]
                .as_str()
                .unwrap()
                .split(',')
                .any(|actual| actual == container));
            if format == Format::Mp3 {
                assert_eq!(info["streams"][0]["bit_rate"], "128000");
            }
            let complete = fs::read(&output).unwrap();
            let stamp = fs::metadata(&output).unwrap().modified().unwrap();
            let reused = execute_download_audio(
                &engines,
                &input,
                &stage,
                format,
                bitrate,
                cancel.clone(),
                index as u64 + 100,
                |_| {},
            )
            .await
            .unwrap();
            assert_eq!(reused, output);
            assert_eq!(fs::metadata(&output).unwrap().modified().unwrap(), stamp);
            assert_eq!(fs::read(&output).unwrap(), complete);
            fs::write(&output, b"partial interrupted audio").unwrap();
            let run_id = index as u64 + 200;
            let recovered = execute_download_audio(
                &engines,
                &input,
                &stage,
                format,
                bitrate,
                cancel.clone(),
                run_id,
                |_| {},
            )
            .await
            .unwrap();
            assert_eq!(recovered, output);
            let archived = output.with_file_name(format!(
                "{}.interrupted-{run_id}",
                output.file_name().unwrap().to_string_lossy()
            ));
            assert_eq!(fs::read(archived).unwrap(), b"partial interrupted audio");
            assert_eq!(fs::read(&input).unwrap(), original);
            assert!(probe_local(&engines, &recovered, cancel.clone())
                .await
                .is_ok());
        }
        fs::remove_dir_all(work).unwrap();
    }
    #[test]
    fn trim_validates_bounds_and_distinguishes_precise_and_keyframe_cuts() {
        let source = info(Some("vp9"), Some("opus"));
        for (start, end) in [
            (-1.0, 2.0),
            (2.0, 2.0),
            (5.0, 2.0),
            (0.0, 11.0),
            (f64::NAN, 2.0),
        ] {
            assert!(validate_options(
                &ToolOptions::Trim {
                    start,
                    end,
                    precise: true
                },
                &source
            )
            .is_err());
        }
        let exact = plan(
            &ToolOptions::Trim {
                start: 1.0,
                end: 3.0,
                precise: true,
            },
            &source,
        )
        .unwrap();
        assert_eq!(exact.expected_duration, Some(2.0));
        assert_eq!(
            (exact.video.as_deref(), exact.audio.as_deref()),
            (Some("h264"), Some("aac"))
        );
        let fast = plan(
            &ToolOptions::Trim {
                start: 1.0,
                end: 3.0,
                precise: false,
            },
            &source,
        )
        .unwrap();
        assert_eq!(fast.extension, "mkv");
        assert_eq!(fast.expected_duration, None);
        assert!(fast.args.windows(2).any(|v| v == ["-c", "copy"]));
    }
    #[test]
    fn target_size_reserves_audio_and_muxing_and_rejects_impossible_values() {
        let source = info(Some("h264"), Some("aac"));
        assert_eq!(compression_bitrate(&source, 1.0, true).unwrap(), 668918);
        for size in [0.0, -1.0, f64::INFINITY, 0.01] {
            assert!(validate_options(
                &ToolOptions::Compress {
                    target_mb: Some(size),
                    quality: "balanced".into()
                },
                &source
            )
            .is_err());
        }
        assert!(validate_options(
            &ToolOptions::Compress {
                target_mb: None,
                quality: "pretend-gpu".into()
            },
            &source
        )
        .is_err());
        assert!(validate_options(
            &ToolOptions::Compress {
                target_mb: None,
                quality: "balanced".into()
            },
            &info(None, Some("aac"))
        )
        .is_err());
    }
    #[test]
    fn output_verification_rejects_wrong_codecs_empty_files_and_wrong_duration() {
        let source = info(Some("h264"), Some("aac"));
        let p = plan(
            &ToolOptions::Convert {
                container: "mp4".into(),
                video_codec: "copy".into(),
                audio_codec: "copy".into(),
            },
            &source,
        )
        .unwrap();
        assert!(verify(&source, &p).is_ok());
        let mut bad = source.clone();
        bad["streams"][0]["codec_name"] = json!("vp9");
        assert!(verify(&bad, &p).is_err());
        let mut bad = source.clone();
        bad["format"]["duration"] = json!("2.0");
        assert!(verify(&bad, &p).is_err());
        let mut bad = source;
        bad["format"]["size"] = json!("0");
        assert!(verify(&bad, &p).is_err());
    }
    #[test]
    fn filenames_cannot_escape_the_owned_staging_folder() {
        assert_eq!(safe_stem(Path::new("A<bad>:name?.mp4")), "Abadname");
        assert_eq!(safe_stem(Path::new("....mp4")), "media");
        assert_eq!(safe_stem(Path::new("Unicode ñ 雪.mp4")), "Unicode ñ 雪");
        assert!(checked_paths(
            Path::new("https://example.com/video.mp4"),
            Path::new("C:/stage")
        )
        .is_err());
        assert!(checked_paths(Path::new("relative.mp4"), Path::new("C:/stage")).is_err());
    }

    #[tokio::test]
    #[ignore = "Requires the managed FFmpeg/ffprobe engine bundle; uses synthetic local media only."]
    async fn real_engines_local_tools_preserve_originals_and_verify_outputs() {
        let engines = std::env::var_os("NABNAB_TEST_ENGINES")
            .or_else(|| std::env::var_os("YTNABBER_TEST_ENGINES"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engines"));
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let work =
            std::env::temp_dir().join(format!("nabnab-tools-{}-{nonce}", std::process::id()));
        fs::create_dir(&work).unwrap();
        let input = work.join("Original ñ 雪.mp4");
        let (_sender, cancel) = watch::channel(false);
        let mut fixture = Vec::new();
        add(
            &mut fixture,
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-n",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x120:rate=12:duration=6",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=6",
                "-map",
                "0:v:0",
                "-map",
                "1:a:0",
                "-c:v",
                "libopenh264",
                "-b:v",
                "500k",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-shortest",
            ],
        );
        fixture.push(input.to_string_lossy().into_owned());
        engine::tool(&engines, "ffmpeg.exe", fixture, cancel.clone())
            .await
            .unwrap();
        let original = fs::read(&input).unwrap();
        let operations = [
            ToolOptions::Convert {
                container: "mkv".into(),
                video_codec: "copy".into(),
                audio_codec: "copy".into(),
            },
            ToolOptions::Convert {
                container: "webm".into(),
                video_codec: "vp9".into(),
                audio_codec: "opus".into(),
            },
            ToolOptions::ExtractAudio {
                format: "mp3".into(),
            },
            ToolOptions::ExtractAudio {
                format: "original".into(),
            },
            ToolOptions::Trim {
                start: 1.0,
                end: 3.0,
                precise: true,
            },
            ToolOptions::Trim {
                start: 1.0,
                end: 3.0,
                precise: false,
            },
            ToolOptions::Compress {
                target_mb: Some(0.25),
                quality: "balanced".into(),
            },
        ];
        for (index, options) in operations.iter().enumerate() {
            let stage = work.join(format!("stage-{index}"));
            fs::create_dir(&stage).unwrap();
            let phases = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let observed = phases.clone();
            let output = execute(
                &engines,
                &input,
                &stage,
                options,
                cancel.clone(),
                index as u64 + 1,
                move |progress| observed.lock().unwrap().push(progress),
            )
            .await
            .unwrap();
            assert!(output.is_file());
            assert_eq!(
                output.parent(),
                Some(fs::canonicalize(&stage).unwrap().as_path())
            );
            assert_eq!(fs::read(&input).unwrap(), original);
            assert!(phases
                .lock()
                .unwrap()
                .iter()
                .any(|p| p.phase == "verifying"));
            assert!(phases.lock().unwrap().iter().all(|p| p.percent.is_none()));
            assert!(execute(
                &engines,
                &input,
                &stage,
                options,
                cancel.clone(),
                100,
                |_| {}
            )
            .await
            .is_err());
        }
        let (stopped, stopped_cancel) = watch::channel(true);
        assert_eq!(
            execute(
                &engines,
                &input,
                &work,
                &operations[0],
                stopped_cancel,
                101,
                |_| {}
            )
            .await
            .unwrap_err(),
            engine::CANCELLED
        );
        drop(stopped);
        assert_eq!(fs::read(&input).unwrap(), original);
        assert_eq!(work.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(&work).unwrap();
    }
}
