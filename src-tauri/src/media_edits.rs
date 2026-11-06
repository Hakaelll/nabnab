use crate::{
    engine,
    library::{self, LibraryItem},
    model::Progress,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use tauri::AppHandle;
use tokio::sync::watch;

const LOCAL_FORMATS: &str =
    "mov,matroska,webm,mp3,ogg,wav,flac,aac,avi,asf,flv,mpeg,mpegts,nut,ac3,eac3,aiff,ape";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MediaEditOptions {
    Join {
        mode: String,
        width: u32,
        height: u32,
        fps: u32,
    },
    ReplaceAudio,
    BurnSubtitles {
        track_id: String,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaEditCapabilities {
    burn_subtitles: bool,
}
#[tauri::command]
pub async fn media_edit_capabilities(app: AppHandle) -> Result<MediaEditCapabilities, String> {
    let root = crate::dependencies::engine_root(&app)?;
    let (_sender, cancel) = watch::channel(false);
    let filters = engine::run_process(
        root.join("ffmpeg.exe"),
        vec!["-hide_banner".into(), "-filters".into()],
        cancel,
        0,
        true,
        |_| {},
    )
    .await?;
    Ok(MediaEditCapabilities {
        burn_subtitles: filters.split_whitespace().any(|value| value == "subtitles"),
    })
}

pub fn validate_options(options: &MediaEditOptions, count: usize) -> Result<(), String> {
    match options {
        MediaEditOptions::Join {
            mode,
            width,
            height,
            fps,
        } => {
            if !(2..=20).contains(&count)
                || !matches!(mode.as_str(), "copy" | "encode")
                || !(16..=8192).contains(width)
                || !(16..=8192).contains(height)
                || width % 2 != 0
                || height % 2 != 0
                || !(1..=120).contains(fps)
            {
                return Err("Join 2–20 clips. Choose copy or encode, even dimensions of 16–8192 pixels and 1–120 FPS.".into());
            }
        }
        MediaEditOptions::ReplaceAudio => {
            if count != 2 {
                return Err("Choose a video followed by an audio source.".into());
            }
        }
        MediaEditOptions::BurnSubtitles { track_id } => {
            if count != 1 || track_id.is_empty() || track_id.len() > 192 {
                return Err("Choose one video and an existing local subtitle track.".into());
            }
        }
    }
    Ok(())
}
fn stream<'a>(info: &'a Value, kind: &str) -> Option<&'a Value> {
    info["streams"].as_array()?.iter().find(|stream| {
        stream["codec_type"] == kind
            && (kind != "video" || stream["disposition"]["attached_pic"] != 1)
    })
}
fn duration(info: &Value) -> Result<f64, String> {
    info["format"]["duration"]
        .as_f64()
        .or_else(|| info["format"]["duration"].as_str()?.parse().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| "Choose finished media with a valid duration.".into())
}
async fn probe(
    engines: &Path,
    input: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<Value, String> {
    let raw = engine::tool(
        engines,
        "ffprobe.exe",
        vec![
            "-v".into(),
            "error".into(),
            "-protocol_whitelist".into(),
            "file,pipe".into(),
            "-format_whitelist".into(),
            LOCAL_FORMATS.into(),
            "-show_streams".into(),
            "-show_format".into(),
            "-show_data_hash".into(),
            "sha256".into(),
            "-of".into(),
            "json".into(),
            input.to_string_lossy().into_owned(),
        ],
        cancel,
    )
    .await?;
    serde_json::from_str(&raw).map_err(|_| "FFprobe returned invalid composition metadata.".into())
}
fn signature(info: &Value, kind: &str) -> Option<Vec<Value>> {
    let stream = stream(info, kind)?;
    let keys: &[&str] = if kind == "video" {
        &[
            "codec_name",
            "profile",
            "level",
            "pix_fmt",
            "width",
            "height",
            "sample_aspect_ratio",
            "r_frame_rate",
            "time_base",
            "extradata_hash",
            "extradata_size",
        ]
    } else {
        &[
            "codec_name",
            "profile",
            "sample_fmt",
            "sample_rate",
            "channels",
            "channel_layout",
            "time_base",
            "extradata_hash",
            "extradata_size",
        ]
    };
    Some(keys.iter().map(|key| stream[*key].clone()).collect())
}
fn copy_compatible(infos: &[Value]) -> bool {
    infos.first().is_some_and(|first| {
        infos.iter().all(|info| {
            signature(info, "video") == signature(first, "video")
                && signature(info, "audio") == signature(first, "audio")
        })
    })
}
fn filter_filename(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text
        .strip_prefix("\\\\?\\")
        .unwrap_or(&text)
        .replace('\\', "/");
    let first: String = text
        .chars()
        .flat_map(|character| {
            if "\\':".contains(character) {
                vec!['\\', character]
            } else {
                vec![character]
            }
        })
        .collect();
    first
        .chars()
        .flat_map(|character| {
            if "\\'[],; ".contains(character) {
                vec!['\\', character]
            } else {
                vec![character]
            }
        })
        .collect()
}
fn concat_filename(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix("\\\\?\\")
        .unwrap_or(&text)
        .replace('\\', "/")
        .replace('\'', "'\\''")
}
fn encoding_args() -> Vec<String> {
    [
        "-c:v",
        "libopenh264",
        "-b:v",
        "8M",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-ar",
        "48000",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn normalized_join(
    infos: &[Value],
    width: u32,
    height: u32,
    fps: u32,
) -> Result<(String, bool), String> {
    let audio = infos.iter().any(|info| stream(info, "audio").is_some());
    let mut graph = String::new();
    for (index, info) in infos.iter().enumerate() {
        graph.push_str(&format!("[{index}:v:0]scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={fps},format=yuv420p,setpts=PTS-STARTPTS[v{index}];"));
        if audio {
            let length = duration(info)?;
            if stream(info, "audio").is_some() {
                graph.push_str(&format!("[{index}:a:0]aformat=sample_rates=48000:channel_layouts=stereo,apad,atrim=duration={length:.6},asetpts=PTS-STARTPTS[a{index}];"));
            } else {
                graph.push_str(&format!("anullsrc=r=48000:cl=stereo,atrim=duration={length:.6},asetpts=PTS-STARTPTS[a{index}];"));
            }
        }
    }
    for index in 0..infos.len() {
        graph.push_str(&format!("[v{index}]"));
        if audio {
            graph.push_str(&format!("[a{index}]"));
        }
    }
    graph.push_str(&format!(
        "concat=n={}:v=1:a={}[v]{}",
        infos.len(),
        if audio { 1 } else { 0 },
        if audio { "[a]" } else { "" }
    ));
    Ok((graph, audio))
}

pub async fn execute(
    engines: &Path,
    inputs: &[LibraryItem],
    staging: &Path,
    options: &MediaEditOptions,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    validate_options(options, inputs.len())?;
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let staging = library::authorized_path(staging)?;
    let mut paths = Vec::new();
    let mut infos = Vec::new();
    for input in inputs {
        let path = library::authorized_path(&input.path)?;
        let info = probe(engines, &path, cancel.clone()).await?;
        duration(&info)?;
        paths.push(path);
        infos.push(info);
    }
    if stream(&infos[0], "video").is_none() {
        return Err("The first input must contain a video track.".into());
    }
    let mut args: Vec<String> = ["-nostdin", "-v", "error", "-n"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut expected_audio = stream(&infos[0], "audio").is_some();
    let expected_duration: f64;
    let (suffix, extension) = match options {
        MediaEditOptions::Join {
            mode,
            width,
            height,
            fps,
        } => {
            if infos.iter().any(|info| stream(info, "video").is_none()) {
                return Err("Every joined clip must contain video.".into());
            }
            expected_duration = infos
                .iter()
                .map(duration)
                .collect::<Result<Vec<_>, _>>()?
                .iter()
                .sum();
            if mode == "copy" {
                if !copy_compatible(&infos) {
                    return Err("Stream copying requires matching codecs, resolution, FPS, time bases, audio layout and codec initialization data. Choose re-encoding for these clips.".into());
                }
                let list = staging.join("nabnab-join.ffconcat");
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&list)
                    .map_err(|_| "The join staging file already exists. Start a new job.")?;
                file.write_all(b"ffconcat version 1.0\n")
                    .map_err(|e| e.to_string())?;
                for path in &paths {
                    writeln!(file, "file '{}'", concat_filename(path))
                        .map_err(|e| e.to_string())?;
                }
                drop(file);
                args.extend([
                    "-protocol_whitelist".into(),
                    "file,pipe".into(),
                    "-format_whitelist".into(),
                    format!("concat,{LOCAL_FORMATS}"),
                    "-f".into(),
                    "concat".into(),
                    "-safe".into(),
                    "0".into(),
                    "-i".into(),
                    list.to_string_lossy().into_owned(),
                    "-map".into(),
                    "0:v:0".into(),
                    "-map".into(),
                    "0:a:0?".into(),
                    "-c".into(),
                    "copy".into(),
                ]);
            } else {
                for path in &paths {
                    args.extend([
                        "-protocol_whitelist".into(),
                        "file,pipe".into(),
                        "-format_whitelist".into(),
                        LOCAL_FORMATS.into(),
                        "-i".into(),
                        path.to_string_lossy().into_owned(),
                    ]);
                }
                let (graph, audio) = normalized_join(&infos, *width, *height, *fps)?;
                expected_audio = audio;
                args.extend(["-filter_complex".into(), graph, "-map".into(), "[v]".into()]);
                if audio {
                    args.extend(["-map".into(), "[a]".into()]);
                }
                args.extend(encoding_args());
            }
            ("joined", "mkv")
        }
        MediaEditOptions::ReplaceAudio => {
            if stream(&infos[1], "audio").is_none() {
                return Err("The second input must contain audio.".into());
            }
            expected_duration = duration(&infos[0])?;
            expected_audio = true;
            for path in &paths {
                args.extend([
                    "-protocol_whitelist".into(),
                    "file,pipe".into(),
                    "-format_whitelist".into(),
                    LOCAL_FORMATS.into(),
                    "-i".into(),
                    path.to_string_lossy().into_owned(),
                ]);
            }
            args.extend([
                "-map".into(),
                "0:v:0".into(),
                "-map".into(),
                "1:a:0".into(),
                "-c:v".into(),
                "copy".into(),
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                "192k".into(),
                "-af".into(),
                format!("apad,atrim=duration={expected_duration:.6}"),
                "-t".into(),
                format!("{expected_duration:.6}"),
            ]);
            ("audio-replaced", "mkv")
        }
        MediaEditOptions::BurnSubtitles { track_id } => {
            let filters = engine::run_process(
                engines.join("ffmpeg.exe"),
                vec!["-hide_banner".into(), "-filters".into()],
                cancel.clone(),
                run_id,
                true,
                |_| {},
            )
            .await?;
            if !filters.split_whitespace().any(|value| value == "subtitles") {
                return Err("The installed FFmpeg component has no subtitles filter. Burning subtitles requires an FFmpeg build with libass; soft subtitle embedding is available separately.".into());
            }
            let track = inputs[0]
                .details
                .tracks
                .iter()
                .find(|track| track.id == *track_id)
                .ok_or("Choose an available local subtitle track.")?;
            if track.segments.is_empty() {
                return Err("The selected subtitle track has no segments.".into());
            }
            let subtitle = staging.join("nabnab-burn.srt");
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&subtitle)
                .map_err(|_| "The subtitle staging file already exists. Start a new job.")?;
            file.write_all(crate::captions::to_srt(&track.segments).as_bytes())
                .map_err(|e| e.to_string())?;
            drop(file);
            expected_duration = duration(&infos[0])?;
            args.extend([
                "-protocol_whitelist".into(),
                "file,pipe".into(),
                "-format_whitelist".into(),
                LOCAL_FORMATS.into(),
                "-i".into(),
                paths[0].to_string_lossy().into_owned(),
                "-map".into(),
                "0:v:0".into(),
                "-map".into(),
                "0:a:0?".into(),
                "-vf".into(),
                format!("subtitles=filename={}", filter_filename(&subtitle)),
            ]);
            args.extend(encoding_args());
            ("subtitles-burned", "mp4")
        }
    };
    args.extend([
        "-sn".into(),
        "-dn".into(),
        "-map_metadata".into(),
        "-1".into(),
    ]);
    let output = staging.join(format!("{suffix}-{run_id}.{extension}"));
    if output.exists() {
        return Err(
            "The composition output already exists. Start a new job to preserve it.".into(),
        );
    }
    args.push(output.to_string_lossy().into_owned());
    emit(Progress {
        request_id: run_id,
        phase: "processing".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
    engine::run_process(
        engines.join("ffmpeg.exe"),
        args,
        cancel.clone(),
        run_id,
        false,
        emit.clone(),
    )
    .await?;
    let output = crate::checked_output(&staging, &output.to_string_lossy())?;
    emit(Progress {
        request_id: run_id,
        phase: "verifying".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
    let info = probe(engines, &output, cancel.clone()).await?;
    if stream(&info, "video").is_none()
        || stream(&info, "audio").is_some() != expected_audio
        || (duration(&info)? - expected_duration).abs() > (expected_duration * 0.02).max(0.5)
    {
        return Err("The composition output did not preserve the expected tracks or duration. Nothing was published.".into());
    }
    if fs::metadata(&output).map_err(|e| e.to_string())?.len() == 0 {
        return Err("The composition output is empty.".into());
    }
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn info() -> Value {
        json!({"format":{"duration":"2"},"streams":[{"codec_type":"video","codec_name":"h264","width":320,"height":240,"r_frame_rate":"30/1","extradata_hash":"SHA256:one"},{"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":2}]})
    }
    #[test]
    fn stream_copy_rejects_changes_that_need_reencoding() {
        let original = info();
        assert!(copy_compatible(&[original.clone(), original.clone()]));
        for (key, value) in [
            ("width", json!(640)),
            ("r_frame_rate", json!("25/1")),
            ("extradata_hash", json!("SHA256:two")),
        ] {
            let mut changed = original.clone();
            changed["streams"][0][key] = value;
            assert!(!copy_compatible(&[original.clone(), changed]));
        }
    }
    #[test]
    fn encoded_join_normalizes_video_and_fills_missing_audio() {
        let original = info();
        let mut silent = original.clone();
        silent["streams"].as_array_mut().unwrap().pop();
        let (graph, audio) = normalized_join(&[original, silent], 640, 360, 25).unwrap();
        assert!(audio);
        assert!(graph.contains("scale=640:360"));
        assert!(graph.contains("fps=25"));
        assert!(graph.contains("anullsrc=r=48000:cl=stereo,atrim=duration=2.000000"));
        assert!(graph.ends_with("concat=n=2:v=1:a=1[v][a]"));
    }
    #[test]
    fn subtitle_filter_escapes_both_filter_parser_layers() {
        assert_eq!(
            filter_filename(Path::new("C:\\Media\\clip's.srt")),
            r"C\\:/Media/clip\\\'s.srt"
        );
    }
    #[tokio::test]
    #[ignore = "Requires bundled FFmpeg/ffprobe; creates synthetic local video/audio only."]
    async fn real_engines_join_and_audio_replacement_preserve_originals() {
        let engines = std::env::var_os("NABNAB_TEST_ENGINES")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engines"));
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let work =
            std::env::temp_dir().join(format!("nabnab-media-edits-{}-{nonce}", std::process::id()));
        fs::create_dir(&work).unwrap();
        let (_sender, cancel) = watch::channel(false);
        let first = work.join("First clip's video.mp4");
        let silent = work.join("Silent video.mp4");
        let audio = work.join("New audio.wav");
        let mut args: Vec<String> = [
            "-nostdin",
            "-v",
            "error",
            "-n",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x120:rate=12:duration=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
            "-c:v",
            "libopenh264",
            "-b:v",
            "500k",
            "-c:a",
            "aac",
            "-shortest",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        args.push(first.to_string_lossy().into_owned());
        engine::tool(&engines, "ffmpeg.exe", args, cancel.clone())
            .await
            .unwrap();
        let mut args: Vec<String> = [
            "-nostdin",
            "-v",
            "error",
            "-n",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=128x96:rate=10:duration=2",
            "-c:v",
            "libopenh264",
            "-b:v",
            "500k",
            "-an",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        args.push(silent.to_string_lossy().into_owned());
        engine::tool(&engines, "ffmpeg.exe", args, cancel.clone())
            .await
            .unwrap();
        let mut args: Vec<String> = [
            "-nostdin",
            "-v",
            "error",
            "-n",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000:duration=1",
            "-c:a",
            "pcm_s16le",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        args.push(audio.to_string_lossy().into_owned());
        engine::tool(&engines, "ffmpeg.exe", args, cancel.clone())
            .await
            .unwrap();
        let originals = [
            fs::read(&first).unwrap(),
            fs::read(&silent).unwrap(),
            fs::read(&audio).unwrap(),
        ];
        let mut first_item = library::index(&first, None, None, None).unwrap();
        first_item.details.tracks.push(crate::captions::LocalTrack { id: "burn-fixture".into(), language: "en".into(), origin: crate::captions::TrackOrigin::Imported, path: None, segments: vec![crate::captions::Segment { start: 0.0, end: 1.5, text: "Synthetic local subtitle".into() }], formatting_lost: false });
        let silent_item = library::index(&silent, None, None, None).unwrap();
        let audio_item = library::index(&audio, None, None, None).unwrap();
        for (index, (inputs, options)) in [
            (
                vec![first_item.clone(), first_item.clone()],
                MediaEditOptions::Join {
                    mode: "copy".into(),
                    width: 160,
                    height: 120,
                    fps: 12,
                },
            ),
            (
                vec![first_item.clone(), silent_item],
                MediaEditOptions::Join {
                    mode: "encode".into(),
                    width: 160,
                    height: 120,
                    fps: 12,
                },
            ),
            (vec![first_item.clone(), audio_item], MediaEditOptions::ReplaceAudio),
            (vec![first_item], MediaEditOptions::BurnSubtitles { track_id: "burn-fixture".into() }),
        ]
        .into_iter()
        .enumerate()
        {
            let stage = work.join(format!("stage-{index}"));
            fs::create_dir(&stage).unwrap();
            let output = execute(
                &engines,
                &inputs,
                &stage,
                &options,
                cancel.clone(),
                index as u64,
                |_| {},
            )
            .await
            .unwrap_or_else(|error| panic!("{options:?}: {error}"));
            let result = probe(&engines, &output, cancel.clone()).await.unwrap();
            assert_eq!(stream(&result, "video").unwrap()["width"], 160);
            assert_eq!(stream(&result, "video").unwrap()["height"], 120);
            assert!((duration(&result).unwrap() - if index >= 2 { 2.0 } else { 4.0 }).abs() < 0.3);
            if index == 1 {
                assert_eq!(stream(&result, "video").unwrap()["r_frame_rate"], "12/1");
                assert_eq!(stream(&result, "audio").unwrap()["channels"], 2);
                assert_eq!(stream(&result, "audio").unwrap()["sample_rate"], "48000");
            }
        }
        assert_eq!(fs::read(first).unwrap(), originals[0]);
        assert_eq!(fs::read(silent).unwrap(), originals[1]);
        assert_eq!(fs::read(audio).unwrap(), originals[2]);
        assert_eq!(work.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(work).unwrap();
    }
}
