use crate::{engine, library, media};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio::sync::watch;

pub mod commands;

const SIDE: u32 = 512;
const MAX_PIXELS: u64 = 24_000_000;
const MAX_SOURCE: u64 = 512 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Profile {
    TelegramImage,
    TelegramVideo,
    WhatsappImage,
    WhatsappAnimated,
}

impl Profile {
    fn limit(self) -> usize {
        match self {
            Self::TelegramImage => 512_000,
            Self::TelegramVideo => 256_000,
            Self::WhatsappImage => 100_000,
            Self::WhatsappAnimated => 500_000,
        }
    }

    fn duration_limit(self) -> Option<u32> {
        match self {
            Self::TelegramVideo => Some(3_000),
            Self::WhatsappAnimated => Some(10_000),
            _ => None,
        }
    }

    fn extension(self) -> &'static str {
        if self == Self::TelegramVideo {
            "webm"
        } else {
            "webp"
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::TelegramImage => "telegram-image",
            Self::TelegramVideo => "telegram-video",
            Self::WhatsappImage => "whatsapp-image",
            Self::WhatsappAnimated => "whatsapp-animated",
        }
    }

    fn attempts(self) -> &'static [(u32, u32)] {
        match self {
            Self::TelegramVideo => &[(25, 31), (25, 40), (20, 47), (10, 54)],
            Self::WhatsappAnimated => &[(25, 78), (20, 61), (15, 42), (10, 25)],
            _ => &[(1, 92), (1, 75), (1, 52), (1, 26)],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Crop {
    horizontal: u8,
    vertical: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub profile: Profile,
    crop: Option<Crop>,
    outline: u8,
    start_ms: u32,
    duration_ms: Option<u32>,
}

impl Options {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .crop
            .is_some_and(|crop| crop.horizontal > 100 || crop.vertical > 100)
            || !matches!(self.outline, 0 | 4 | 8)
            || self.start_ms > 24 * 60 * 60 * 1_000
        {
            return Err("Choose a valid crop, outline and starting time.".into());
        }
        match (self.profile.duration_limit(), self.duration_ms) {
            (None, None) => Ok(()),
            (Some(max), Some(duration)) if (200..=max).contains(&duration) => Ok(()),
            _ => Err("Choose a clip length within the selected sticker profile's limits.".into()),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub library_id: String,
    pub input_hash: String,
    pub options: Options,
}

impl Selection {
    pub fn validate(&self) -> Result<(), String> {
        if self.library_id.is_empty()
            || self.library_id.len() > 128
            || self.input_hash.len() != 64
            || !self.input_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("The sticker source fingerprint is invalid.".into());
        }
        self.options.validate()
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Details {
    width: u32,
    height: u32,
    pub duration_ms: Option<u32>,
    pub frame_rate: Option<u32>,
    bytes: usize,
    limit: usize,
    mime: &'static str,
}

pub struct Artifact {
    pub path: PathBuf,
    pub details: Details,
    pub info: Value,
}

pub fn demuxer(path: &Path) -> Result<&'static str, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "png" => Ok("png_pipe"),
        "jpg" | "jpeg" => Ok("jpeg_pipe"),
        "webp" => Ok("webp_pipe"),
        "bmp" => Ok("bmp_pipe"),
        "gif" => Ok("gif"),
        "mp4" | "m4v" | "mov" => Ok("mov"),
        "mkv" | "webm" => Ok("matroska"),
        _ => {
            Err("Choose an imported PNG, JPEG, WebP, BMP, GIF, MP4, MOV, MKV or WebM file.".into())
        }
    }
}

fn filter(options: &Options, stream: u64, fps: u32) -> String {
    let mut input = Vec::new();
    if options.duration_ms.is_some() {
        input.push(format!("fps={fps}"));
    }
    if let Some(crop) = options.crop {
        input.push(format!(
            "crop='min(iw,ih)':'min(iw,ih)':'(iw-ow)*{}/100':'(ih-oh)*{}/100':exact=1",
            crop.horizontal, crop.vertical
        ));
    }
    input.extend([
        "scale=480:480:force_original_aspect_ratio=decrease".into(),
        "setsar=1".into(),
        "format=rgba".into(),
        "pad=512:512:(ow-iw)/2:(oh-ih)/2:color=black@0".into(),
    ]);
    let source = format!("[0:{stream}]{}", input.join(","));
    if options.outline == 0 {
        return format!("{source}[out]");
    }
    let dilation = ",dilation".repeat(options.outline.into());
    format!("{source},split=3[face][base][mask];[mask]alphaextract{dilation}[edge];[base]lutrgb=r=255:g=255:b=255[white];[white][edge]alphamerge[back];[back][face]overlay=shortest=1:format=rgb:alpha=straight[out]")
}

fn seconds(ms: u32) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

fn encode_args(
    input: &Path,
    demuxer: &str,
    output: &Path,
    options: &Options,
    stream: u64,
    fps: u32,
    quality: u32,
) -> Vec<String> {
    let mut args = [
        "-v",
        "error",
        "-nostdin",
        "-n",
        "-xerror",
        "-filter_complex_threads",
        "1",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        demuxer,
        "-f",
        demuxer,
        "-threads",
        "2",
        "-max_pixels",
        "24000000",
    ]
    .map(str::to_owned)
    .to_vec();
    if options.start_ms > 0 {
        args.extend(["-ss".into(), seconds(options.start_ms)]);
    }
    if demuxer == "gif" {
        args.extend(["-ignore_loop".into(), "1".into()]);
    }
    if demuxer == "mov" {
        args.extend(["-enable_drefs".into(), "0".into()]);
    }
    args.extend([
        "-i".into(),
        input.to_string_lossy().into_owned(),
        "-filter_complex".into(),
        filter(options, stream, fps),
        "-map".into(),
        "[out]".into(),
        "-an".into(),
        "-sn".into(),
        "-dn".into(),
        "-map_metadata".into(),
        "-1".into(),
        "-map_chapters".into(),
        "-1".into(),
        "-threads".into(),
        "2".into(),
        "-frames:v".into(),
        options
            .duration_ms
            .map_or(1, |duration| duration * fps / 1000)
            .to_string(),
        "-pix_fmt".into(),
        "yuva420p".into(),
    ]);
    match options.profile {
        Profile::TelegramVideo => args.extend([
            "-c:v".into(),
            "libvpx-vp9".into(),
            "-auto-alt-ref".into(),
            "0".into(),
            "-crf".into(),
            quality.to_string(),
            "-b:v".into(),
            "0".into(),
            "-deadline".into(),
            "good".into(),
            "-cpu-used".into(),
            "4".into(),
        ]),
        Profile::WhatsappAnimated => args.extend([
            "-c:v".into(),
            "libwebp_anim".into(),
            "-quality".into(),
            quality.to_string(),
            "-compression_level".into(),
            "4".into(),
            "-loop".into(),
            "0".into(),
        ]),
        _ => args.extend([
            "-c:v".into(),
            "libwebp".into(),
            "-quality".into(),
            quality.to_string(),
            "-compression_level".into(),
            "4".into(),
        ]),
    }
    args.push(output.to_string_lossy().into_owned());
    args
}

fn little24(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0])
}

// FFprobe's WebP decoder does not expose animation frame timing. Read the
// bounded RIFF metadata emitted by the pinned encoder instead.
fn webp_duration(bytes: &[u8], animated: bool) -> Option<Option<u32>> {
    if bytes.get(..4)? != b"RIFF"
        || bytes.get(8..12)? != b"WEBP"
        || u32::from_le_bytes(bytes.get(4..8)?.try_into().ok()?) as usize + 8 != bytes.len()
    {
        return None;
    }
    let mut chunks = bytes.get(12..)?;
    let mut canvas = false;
    let mut image = false;
    let mut frames = 0;
    let mut duration = 0u32;
    while !chunks.is_empty() {
        let size = u32::from_le_bytes(chunks.get(4..8)?.try_into().ok()?) as usize;
        let data = chunks.get(8..8usize.checked_add(size)?)?;
        match chunks.get(..4)? {
            b"VP8X" if data.len() == 10 => {
                canvas = data[0] & 0x10 != 0
                    && (data[0] & 2 != 0) == animated
                    && little24(&data[4..7]) + 1 == SIDE
                    && little24(&data[7..10]) + 1 == SIDE;
            }
            b"VP8 " | b"VP8L" => image = true,
            b"ANMF" if data.len() > 16 => {
                let frame_ms = little24(&data[12..15]);
                if frame_ms < 8 {
                    return None;
                }
                duration = duration.checked_add(frame_ms)?;
                frames += 1;
            }
            _ => {}
        }
        chunks = chunks.get(8usize.checked_add(size)?.checked_add(size % 2)?..)?;
    }
    if !canvas {
        return None;
    }
    match animated {
        true if frames >= 2 && duration <= 10_000 => Some(Some(duration)),
        false if image && frames == 0 => Some(None),
        _ => None,
    }
}

async fn verify(
    engines: &Path,
    path: &Path,
    profile: Profile,
    fps: u32,
    cancel: watch::Receiver<bool>,
) -> Result<(Details, Value), String> {
    let bytes = fs::metadata(path).map_err(|error| error.to_string())?.len() as usize;
    let limit = profile.limit();
    if bytes == 0 || bytes > limit {
        return Err("The sticker exceeds its file size limit.".into());
    }
    let (duration_ms, info, mime) = if profile == Profile::TelegramVideo {
        let info = media::probe(engines, path, cancel).await?;
        let streams = info["streams"]
            .as_array()
            .ok_or("The sticker has no video stream.")?;
        let [video] = streams.as_slice() else {
            return Err("The sticker must contain only its silent video stream.".into());
        };
        let duration = info["format"]["duration"]
            .as_str()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| (0.08..=3.0).contains(value))
            .ok_or("The sticker exceeds Telegram's video duration limit.")?;
        if video["codec_name"] != "vp9"
            || video["width"] != SIDE
            || video["height"] != SIDE
            || video["tags"]["alpha_mode"] != "1"
            || video["r_frame_rate"] != format!("{fps}/1")
            || info["format"]["format_name"]
                .as_str()
                .is_none_or(|name| !name.split(',').any(|name| name == "webm"))
        {
            return Err(
                "The sticker is not a 512 px transparent VP9 WebM at the planned frame rate."
                    .into(),
            );
        }
        (Some((duration * 1000.0).round() as u32), info, "video/webm")
    } else {
        let animated = profile == Profile::WhatsappAnimated;
        let raw = fs::read(path).map_err(|error| error.to_string())?;
        let duration = webp_duration(&raw, animated).ok_or(if animated {
            "This excerpt did not produce an animated WebP with at least two valid frames. Choose a moving excerpt."
        } else { "The sticker is not a transparent 512 px WebP." })?;
        let info = json!({"streams":[{"codec_type":"video","codec_name":"webp","width":SIDE,"height":SIDE}],
            "format":{"format_name":"webp","duration":duration.map(seconds)}});
        (duration, info, "image/webp")
    };
    Ok((
        Details {
            width: SIDE,
            height: SIDE,
            duration_ms,
            frame_rate: duration_ms.map(|_| fps),
            bytes,
            limit,
            mime,
        },
        info,
    ))
}

pub async fn encode(
    engines: &Path,
    input: &Path,
    directory: &Path,
    options: &Options,
    cancel: watch::Receiver<bool>,
    request: u64,
) -> Result<Artifact, String> {
    options.validate()?;
    let input = library::authorized_path(input)?;
    let length = crate::portable_zip::regular_file(&input)?;
    if length == 0 || length > MAX_SOURCE {
        return Err("Choose a non-empty source of up to 512 MiB.".into());
    }
    let demuxer = demuxer(&input)?;
    if (options.duration_ms.is_some() || options.start_ms > 0)
        && !matches!(demuxer, "mov" | "matroska" | "gif")
    {
        return Err("Choose a video or GIF for a timed sticker excerpt.".into());
    }
    let probe = media::probe(engines, &input, cancel.clone()).await?;
    let video = probe["streams"]
        .as_array()
        .and_then(|streams| {
            streams.iter().find(|stream| {
                stream["codec_type"] == "video" && stream["disposition"]["attached_pic"] != 1
            })
        })
        .ok_or("The selected file has no usable image or video stream.")?;
    let width = video["width"].as_u64().unwrap_or(0);
    let height = video["height"].as_u64().unwrap_or(0);
    if width == 0 || height == 0 || width.saturating_mul(height) > MAX_PIXELS {
        return Err("Choose a source of up to 24 million pixels per frame.".into());
    }
    let stream = video["index"]
        .as_u64()
        .ok_or("The input video stream is unavailable.")?;
    let output = directory.join(format!(
        "sticker-{}.{}",
        options.profile.name(),
        options.profile.extension()
    ));
    for &(fps, quality) in options.profile.attempts() {
        let args = encode_args(&input, demuxer, &output, options, stream, fps, quality);
        engine::run_process(
            engines.join("ffmpeg.exe"),
            args,
            cancel.clone(),
            request,
            false,
            |_| {},
        )
        .await?;
        let path = crate::checked_output(directory, &output.to_string_lossy())?;
        if fs::metadata(&path)
            .map_err(|error| error.to_string())?
            .len()
            <= options.profile.limit() as u64
        {
            let (details, info) = verify(engines, &path, options.profile, fps, cancel).await?;
            return Ok(Artifact {
                path,
                details,
                info,
            });
        }
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Err("The sticker could not fit the selected profile. Try a shorter or simpler excerpt.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(profile: Profile) -> Options {
        Options {
            profile,
            crop: Some(Crop {
                horizontal: 72,
                vertical: 50,
            }),
            outline: 4,
            start_ms: 120,
            duration_ms: profile.duration_limit().map(|_| 800),
        }
    }

    #[test]
    fn a_telegram_clip_cannot_exceed_its_profile_duration() {
        let mut selected = options(Profile::TelegramVideo);
        selected.duration_ms = Some(3001);
        assert!(selected.validate().is_err());
        selected.duration_ms = Some(3000);
        assert!(selected.validate().is_ok());
        selected.profile = Profile::TelegramImage;
        assert!(selected.validate().is_err());
    }

    #[tokio::test]
    #[ignore = "requires the verified production FFmpeg engines"]
    async fn real_engines_create_four_bounded_transparent_sticker_profiles() {
        let engines =
            fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("engines")).unwrap();
        let root = std::env::temp_dir().join(format!(
            "nabnab-sticker-test-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        fs::create_dir(&root).unwrap();
        let workspace = crate::workspaces::Workspace::new(&root).unwrap();
        let (_sender, cancel) = watch::channel(false);
        let source = workspace.path.join("fixture.mp4");
        engine::run_process(
            engines.join("ffmpeg.exe"),
            [
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=180x132:rate=25",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=200:sample_rate=44100",
                "-c:v",
                "mpeg4",
                "-c:a",
                "aac",
                "-t",
                "1.24",
                &source.to_string_lossy(),
            ]
            .map(str::to_owned)
            .to_vec(),
            cancel.clone(),
            1,
            false,
            |_| {},
        )
        .await
        .unwrap();
        let original = fs::read(&source).unwrap();
        let gif = workspace.path.join("motion.gif");
        engine::run_process(
            engines.join("ffmpeg.exe"),
            [
                "-v",
                "error",
                "-i",
                &source.to_string_lossy(),
                "-vf",
                "fps=10,scale=120:-1",
                "-loop",
                "0",
                &gif.to_string_lossy(),
            ]
            .map(str::to_owned)
            .to_vec(),
            cancel.clone(),
            2,
            false,
            |_| {},
        )
        .await
        .unwrap();
        for profile in [
            Profile::TelegramImage,
            Profile::TelegramVideo,
            Profile::WhatsappImage,
            Profile::WhatsappAnimated,
        ] {
            let selected = options(profile);
            let input = if profile == Profile::WhatsappAnimated {
                &gif
            } else {
                &source
            };
            let result = encode(
                &engines,
                input,
                &workspace.path,
                &selected,
                cancel.clone(),
                3,
            )
            .await
            .unwrap_or_else(|error| panic!("{}: {error}", profile.name()));
            assert_eq!(result.details.width, 512);
            assert_eq!(result.details.height, 512);
            assert!(result.details.bytes <= profile.limit());
            assert_eq!(result.details.duration_ms, selected.duration_ms);
            if profile != Profile::WhatsappAnimated {
                let raw = workspace.path.join("pixels.rgba");
                let mut args = vec!["-v".into(), "error".into()];
                if profile == Profile::TelegramVideo {
                    args.extend(["-c:v".into(), "libvpx-vp9".into()]);
                }
                args.extend([
                    "-i".into(),
                    result.path.to_string_lossy().into_owned(),
                    "-frames:v".into(),
                    "1".into(),
                    "-f".into(),
                    "rawvideo".into(),
                    "-pix_fmt".into(),
                    "rgba".into(),
                    raw.to_string_lossy().into_owned(),
                ]);
                engine::run_process(
                    engines.join("ffmpeg.exe"),
                    args,
                    cancel.clone(),
                    4,
                    false,
                    |_| {},
                )
                .await
                .unwrap();
                let bytes = fs::read(&raw).unwrap();
                assert_eq!(bytes.len(), 512 * 512 * 4);
                assert_eq!(bytes[3], 0);
                assert_eq!(bytes[(256 * 512 + 256) * 4 + 3], 255);
                fs::remove_file(raw).unwrap();
            }
        }
        let (_stopped, cancelled) = watch::channel(true);
        assert!(encode(
            &engines,
            &source,
            &workspace.path,
            &options(Profile::TelegramImage),
            cancelled,
            5
        )
        .await
        .is_err());
        assert_eq!(fs::read(source).unwrap(), original);
        drop(workspace);
        fs::remove_dir(root.join("local-previews")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
