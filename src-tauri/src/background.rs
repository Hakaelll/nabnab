pub mod commands;
pub mod component;
pub mod worker;

use crate::{engine, image_assets, library, media};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio::sync::watch;

const MAX_PIXELS: u64 = 24_000_000;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const PREVIEW_EDGE: u32 = 640;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    Png,
    Webp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Fill {
    Transparent,
    Solid { rgb: [u8; 3] },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    pub format: Format,
    pub fill: Fill,
    edge: i8,
    softness: u8,
}

impl Options {
    pub fn validate(self) -> Result<(), String> {
        if !(-8..=8).contains(&self.edge) || self.softness > 4 {
            return Err(
                "Choose an edge adjustment from −8 to 8 pixels and smoothing from 0 to 4.".into(),
            );
        }
        Ok(())
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
            return Err("The segmentation input fingerprint is invalid.".into());
        }
        self.options.validate()
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dimensions {
    width: u32,
    height: u32,
}

fn demuxer(path: &Path) -> Result<&'static str, String> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Ok("png_pipe"),
        "jpg" | "jpeg" => Ok("jpeg_pipe"),
        "webp" => Ok("webp_pipe"),
        _ => Err("Choose a still PNG, JPEG or WebP image from Library.".into()),
    }
}

fn image_dimensions(info: &Value) -> Result<Dimensions, String> {
    let stream = info["streams"]
        .as_array()
        .and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream["codec_type"] == "video")
        })
        .ok_or("The image has no readable frame.")?;
    let width = stream["width"].as_u64().unwrap_or(0);
    let height = stream["height"].as_u64().unwrap_or(0);
    if width == 0 || height == 0 || width > 8192 || height > 8192 || width * height > MAX_PIXELS {
        return Err(
            "Choose an image of at most 8192 pixels per side and 24 million pixels.".into(),
        );
    }
    Ok(Dimensions {
        width: width as u32,
        height: height as u32,
    })
}

async fn inspect(
    engines: &Path,
    input: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<Dimensions, String> {
    demuxer(input)?;
    let length = crate::portable_zip::regular_file(input)?;
    if length == 0 || length > MAX_BYTES {
        return Err("Choose a non-empty image of up to 64 MiB.".into());
    }
    image_dimensions(&media::probe(engines, input, cancel).await?)
}

fn input_args(input: &Path) -> Result<Vec<String>, String> {
    let input = library::authorized_path(input)?;
    let format = demuxer(&input)?;
    Ok([
        "-v",
        "error",
        "-nostdin",
        "-n",
        "-xerror",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        format,
        "-f",
        format,
        "-noautorotate",
        "-max_pixels",
        "24000000",
        "-threads",
        "2",
        "-i",
        &input.to_string_lossy(),
    ]
    .map(str::to_owned)
    .to_vec())
}

struct MaskFiles {
    rgb: PathBuf,
    mask: PathBuf,
}
impl MaskFiles {
    fn new(directory: &Path) -> Self {
        Self {
            rgb: directory.join("segment-rgb.raw"),
            mask: directory.join("segment-mask.raw"),
        }
    }
}
impl Drop for MaskFiles {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.rgb);
        let _ = fs::remove_file(&self.mask);
    }
}

async fn infer(
    engines: &Path,
    model: &Path,
    input: &Path,
    files: &MaskFiles,
    cancel: watch::Receiver<bool>,
    request: u64,
) -> Result<(), String> {
    let mut args = input_args(input)?;
    args.extend(
        [
            "-vf",
            "scale=320:320:flags=lanczos,format=rgb24",
            "-filter_threads",
            "1",
            "-frames:v",
            "1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            &files.rgb.to_string_lossy(),
        ]
        .map(str::to_owned),
    );
    engine::run_process(
        engines.join("ffmpeg.exe"),
        args,
        cancel.clone(),
        request,
        false,
        |_| {},
    )
    .await?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    engine::run_process(
        executable,
        [
            "--internal-background-mask",
            &model.to_string_lossy(),
            &files.rgb.to_string_lossy(),
            &files.mask.to_string_lossy(),
        ]
        .map(str::to_owned)
        .to_vec(),
        cancel,
        request,
        false,
        |_| {},
    )
    .await?;
    if crate::portable_zip::regular_file(&files.mask)? != u64::from(worker::SIDE).pow(2) {
        return Err("The local model returned a mask with an invalid size.".into());
    }
    Ok(())
}

fn filters(dimensions: Dimensions, options: Options, preview: bool) -> String {
    let Dimensions { width, height } = dimensions;
    let mut graph = format!("[1:v]scale={width}:{height}:flags=lanczos,format=gray");
    let kernel = if options.edge < 0 {
        ",erosion"
    } else {
        ",dilation"
    };
    graph.push_str(&kernel.repeat(usize::from(options.edge.unsigned_abs())));
    if options.softness > 0 {
        graph.push_str(&format!(",gblur=sigma={}", options.softness));
    }
    graph.push_str("[predicted];[0:v]format=rgba,split=2[color][alpha];[alpha]alphaextract[original];[original][predicted]blend=all_mode=multiply:shortest=1[opacity];");
    let (visible_w, visible_h) = if preview && width.max(height) > PREVIEW_EDGE {
        let largest = width.max(height);
        (
            (width * PREVIEW_EDGE / largest).max(1),
            (height * PREVIEW_EDGE / largest).max(1),
        )
    } else {
        (width, height)
    };
    if preview {
        graph.push_str(&format!("[opacity]split=2[apply][mask];[mask]scale={visible_w}:{visible_h}:flags=area,format=gray[mask_preview];"));
    } else {
        graph.push_str("[opacity]null[apply];");
    }
    match options.fill {
        Fill::Transparent => graph.push_str("[color][apply]alphamerge[composite];"),
        Fill::Solid { rgb: [r, g, b] } => graph.push_str(&format!(
            "[color]split=2[foreground][fill];[fill]lutrgb=r={r}:g={g}:b={b}:a=255[base];[foreground][apply]alphamerge[cutout];[base][cutout]overlay=shortest=1:format=rgb:alpha=straight[composite];"
        )),
    }
    graph.push_str(&format!(
        "[composite]scale={visible_w}:{visible_h}:flags=area[result]"
    ));
    graph
}

async fn render(
    engines: &Path,
    input: &Path,
    mask: &Path,
    dimensions: Dimensions,
    options: Options,
    output: &Path,
    mask_preview: Option<&Path>,
    cancel: watch::Receiver<bool>,
    request: u64,
) -> Result<(), String> {
    let mut args = input_args(input)?;
    let size = format!("{0}x{0}", worker::SIDE);
    let filter = filters(dimensions, options, mask_preview.is_some());
    let png = mask_preview.is_some() || options.format == Format::Png;
    args.extend(
        [
            "-protocol_whitelist",
            "file",
            "-format_whitelist",
            "rawvideo",
            "-f",
            "rawvideo",
            "-pixel_format",
            "gray",
            "-video_size",
            &size,
            "-framerate",
            "1",
            "-i",
            &mask.to_string_lossy(),
            "-filter_complex_threads",
            "1",
            "-filter_complex",
            &filter,
            "-map",
            "[result]",
            "-frames:v",
            "1",
            "-map_metadata",
            "-1",
            "-threads",
            "2",
        ]
        .map(str::to_owned),
    );
    args.extend(if png {
        ["-c:v", "png", "-pix_fmt", "rgba"]
            .map(str::to_owned)
            .to_vec()
    } else {
        [
            "-c:v",
            "libwebp",
            "-pix_fmt",
            "bgra",
            "-lossless",
            "1",
            "-compression_level",
            "4",
        ]
        .map(str::to_owned)
        .to_vec()
    });
    args.push(output.to_string_lossy().into_owned());
    if let Some(mask_preview) = mask_preview {
        args.extend(
            [
                "-map",
                "[mask_preview]",
                "-frames:v",
                "1",
                "-map_metadata",
                "-1",
                "-threads",
                "2",
                "-c:v",
                "png",
                "-pix_fmt",
                "gray",
                &mask_preview.to_string_lossy(),
            ]
            .map(str::to_owned),
        );
    }
    engine::run_process(
        engines.join("ffmpeg.exe"),
        args,
        cancel,
        request,
        false,
        |_| {},
    )
    .await?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    dimensions: Dimensions,
    result: Vec<u8>,
    mask: Vec<u8>,
}

pub async fn sample(
    engines: &Path,
    model: &Path,
    directory: &Path,
    input: &Path,
    options: Options,
    cancel: watch::Receiver<bool>,
    request: u64,
) -> Result<Sample, String> {
    options.validate()?;
    let dimensions = inspect(engines, input, cancel.clone()).await?;
    let files = MaskFiles::new(directory);
    infer(engines, model, input, &files, cancel.clone(), request).await?;
    let result = directory.join("result.png");
    let mask = directory.join("mask.png");
    render(
        engines,
        input,
        &files.mask,
        dimensions,
        options,
        &result,
        Some(&mask),
        cancel,
        request,
    )
    .await?;
    let read = |path: &Path| -> Result<_, String> {
        let file = crate::checked_output(directory, &path.to_string_lossy())?;
        let (width, height) = image_assets::png_dimensions(&file)?;
        if width.max(height) > PREVIEW_EDGE {
            return Err("The segmentation sample exceeds its size limit.".into());
        }
        fs::read(file).map_err(|e| e.to_string())
    };
    Ok(Sample {
        dimensions,
        result: read(&result)?,
        mask: read(&mask)?,
    })
}

pub async fn execute(
    engines: &Path,
    model: &Path,
    directory: &Path,
    input: &Path,
    options: Options,
    cancel: watch::Receiver<bool>,
    request: u64,
) -> Result<(PathBuf, Value), String> {
    options.validate()?;
    let expected = inspect(engines, input, cancel.clone()).await?;
    let files = MaskFiles::new(directory);
    infer(engines, model, input, &files, cancel.clone(), request).await?;
    let ext = if options.format == Format::Png {
        "png"
    } else {
        "webp"
    };
    let output = directory.join(format!("foreground.{ext}"));
    render(
        engines,
        input,
        &files.mask,
        expected,
        options,
        &output,
        None,
        cancel.clone(),
        request,
    )
    .await?;
    let output = crate::checked_output(directory, &output.to_string_lossy())?;
    let info = media::probe(engines, &output, cancel).await?;
    let actual = image_dimensions(&info)?;
    if (actual.width, actual.height) != (expected.width, expected.height) {
        return Err("The background result does not match the original dimensions.".into());
    }
    Ok((output, info))
}
