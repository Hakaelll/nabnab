use crate::{engine, image_assets, library, media};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio::sync::watch;

pub mod commands;
pub mod component;

pub const MAX_BATCH: usize = 12;
const MAX_PIXELS: u64 = 24_000_000;
const MAX_INPUT: u64 = 64 * 1024 * 1024;
const SAMPLE_SIDE: u32 = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Model {
    Photo,
    Illustration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Options {
    model: Model,
    pub scale: u32,
    tile: u32,
}

impl Options {
    pub fn validate(self) -> Result<(), String> {
        if !matches!(self.tile, 64 | 128)
            || !matches!(self.scale, 2..=4)
            || (self.model == Model::Photo && self.scale != 4)
        {
            return Err(
                "Choose a scale supported by this model and a 64 or 128 pixel tile.".into(),
            );
        }
        Ok(())
    }

    fn model(self) -> &'static str {
        match self.model {
            Model::Photo => "realesrgan-x4plus",
            Model::Illustration => "realesr-animevideov3",
        }
    }

    pub fn model_files(self) -> String {
        match self.model {
            Model::Photo => self.model().into(),
            Model::Illustration => format!("{}-x{}", self.model(), self.scale),
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
            return Err("The upscale input fingerprint is invalid.".into());
        }
        self.options.validate()
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    x: u32,
    y: u32,
}

impl Anchor {
    fn rect(self, width: u32, height: u32) -> Result<(u32, u32, u32, u32), String> {
        if self.x > 100 || self.y > 100 {
            return Err("Choose a crop position between 0 and 100.".into());
        }
        let (w, h) = (width.min(SAMPLE_SIDE), height.min(SAMPLE_SIDE));
        Ok((
            w,
            h,
            (width - w) * self.x / 100,
            (height - h) * self.y / 100,
        ))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dimensions {
    width: u32,
    height: u32,
    output_width: u32,
    output_height: u32,
    rgba_bytes: u64,
}

impl Dimensions {
    fn new(width: u32, height: u32, options: Options) -> Result<Self, String> {
        options.validate()?;
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err("Choose an image with usable dimensions.".into());
        }
        let (output_width, output_height) = (width * options.scale, height * options.scale);
        let output_pixels = u64::from(output_width) * u64::from(output_height);
        if output_width > 8192 || output_height > 8192 || output_pixels > MAX_PIXELS {
            return Err("The result exceeds 8192 pixels per side or 24 million pixels. Choose a smaller source or scale.".into());
        }
        let rgba_bytes = (u64::from(width) * u64::from(height) + output_pixels) * 4;
        Ok(Self {
            width,
            height,
            output_width,
            output_height,
            rgba_bytes,
        })
    }
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

pub async fn inspect(
    engines: &Path,
    input: &Path,
    options: Options,
    cancel: watch::Receiver<bool>,
) -> Result<Dimensions, String> {
    demuxer(input)?;
    let length = crate::portable_zip::regular_file(input)?;
    if length == 0 || length > MAX_INPUT {
        return Err("Choose a non-empty image of up to 64 MiB.".into());
    }
    let probe = media::probe(engines, input, cancel).await?;
    let video = probe["streams"]
        .as_array()
        .and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream["codec_type"] == "video")
        })
        .ok_or("The image has no readable frame.")?;
    let dimension = |key: &str| {
        video[key]
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or_default()
    };
    Dimensions::new(dimension("width"), dimension("height"), options)
}

async fn png(
    engines: &Path,
    input: &Path,
    output: &Path,
    filter: &str,
    cancel: watch::Receiver<bool>,
    request: u64,
) -> Result<(), String> {
    let input = library::authorized_path(input)?;
    let demuxer = demuxer(&input)?;
    let args = [
        "-v",
        "error",
        "-nostdin",
        "-n",
        "-xerror",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        demuxer,
        "-f",
        demuxer,
        "-noautorotate",
        "-max_pixels",
        "24000000",
        "-threads",
        "2",
        "-i",
        &input.to_string_lossy(),
        "-vf",
        filter,
        "-filter_threads",
        "1",
        "-frames:v",
        "1",
        "-map_metadata",
        "-1",
        "-c:v",
        "png",
        "-pix_fmt",
        "rgba",
        "-threads",
        "2",
        &output.to_string_lossy(),
    ]
    .map(str::to_owned)
    .to_vec();
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

async fn infer(
    runtime: &Path,
    input: &Path,
    output: &Path,
    options: Options,
    cancel: watch::Receiver<bool>,
    request: u64,
) -> Result<(), String> {
    let model_dir = runtime.join("models");
    // The upstream Windows binary appends forward slashes to the model path.
    let model_dir = model_dir.to_string_lossy();
    let model_dir = model_dir.strip_prefix(r"\\?\").unwrap_or(&model_dir);
    if !Path::new(model_dir).is_absolute() || model_dir.encode_utf16().count() > 200 {
        return Err(
            "Real-ESRGAN requires a local model directory with a short Windows path.".into(),
        );
    }
    let args = [
        "-i",
        &input.to_string_lossy(),
        "-o",
        &output.to_string_lossy(),
        "-m",
        model_dir,
        "-n",
        options.model(),
        "-s",
        &options.scale.to_string(),
        "-t",
        &options.tile.to_string(),
        "-j",
        "1:1:1",
        "-f",
        "png",
    ]
    .map(str::to_owned)
    .to_vec();
    engine::run_process(
        component::executable(runtime),
        args,
        cancel,
        request,
        false,
        |_| {},
    )
    .await?;
    if !output.is_file() {
        return Err("Real-ESRGAN returned no image. Check the Vulkan graphics driver or try 64 pixel tiles.".into());
    }
    Ok(())
}

fn checked_png(directory: &Path, output: &Path, expected: (u32, u32)) -> Result<PathBuf, String> {
    let output = crate::checked_output(directory, &output.to_string_lossy())?;
    if image_assets::png_dimensions(&output)? != expected {
        return Err("The generated PNG does not match the expected scale.".into());
    }
    Ok(output)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    before: Vec<u8>,
    after: Vec<u8>,
    crop_width: u32,
    crop_height: u32,
    crop_x: u32,
    crop_y: u32,
}

pub async fn sample(
    engines: &Path,
    runtime: &Path,
    directory: &Path,
    input: &Path,
    dimensions: &Dimensions,
    options: Options,
    anchor: Anchor,
    cancel: watch::Receiver<bool>,
    request: u64,
) -> Result<Sample, String> {
    let (width, height, x, y) = anchor.rect(dimensions.width, dimensions.height)?;
    let crop = directory.join("crop.png");
    let after = directory.join("after.png");
    let before = directory.join("before.png");
    png(
        engines,
        input,
        &crop,
        &format!("format=rgba,crop={width}:{height}:{x}:{y}:exact=1"),
        cancel.clone(),
        request,
    )
    .await?;
    let (target_w, target_h) = (width * options.scale, height * options.scale);
    infer(runtime, &crop, &after, options, cancel.clone(), request).await?;
    png(
        engines,
        &crop,
        &before,
        &format!("scale={target_w}:{target_h}:flags=bicubic"),
        cancel,
        request,
    )
    .await?;
    let read = |file: &Path| {
        fs::read(checked_png(directory, file, (target_w, target_h))?)
            .map_err(|error| error.to_string())
    };
    Ok(Sample {
        before: read(&before)?,
        after: read(&after)?,
        crop_width: width,
        crop_height: height,
        crop_x: x,
        crop_y: y,
    })
}

pub async fn execute(
    engines: &Path,
    runtime: &Path,
    directory: &Path,
    input: &Path,
    options: Options,
    cancel: watch::Receiver<bool>,
    request: u64,
) -> Result<PathBuf, String> {
    let dimensions = inspect(engines, input, options, cancel.clone()).await?;
    let normalized = directory.join(format!("normalized-{request}.png"));
    let output = directory.join(format!("upscaled-{}x.png", options.scale));
    let result = async {
        png(
            engines,
            input,
            &normalized,
            "format=rgba",
            cancel.clone(),
            request,
        )
        .await?;
        infer(runtime, &normalized, &output, options, cancel, request).await?;
        checked_png(
            directory,
            &output,
            (dimensions.output_width, dimensions.output_height),
        )
    }
    .await;
    if normalized.is_file() {
        fs::remove_file(&normalized).map_err(|error| error.to_string())?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_supported_model_factors_and_bounded_outputs_are_accepted() {
        let photo = Options {
            model: Model::Photo,
            scale: 4,
            tile: 64,
        };
        assert!(Options { scale: 2, ..photo }.validate().is_err());
        assert!(Dimensions::new(4096, 4096, photo).is_err());
        assert_eq!(
            Anchor { x: 100, y: 0 }.rect(320, 90).unwrap(),
            (160, 90, 160, 0)
        );
        assert_eq!(
            Dimensions::new(320, 90, photo).unwrap().rgba_bytes,
            1_958_400
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "requires NABNAB_TEST_UPSCALE_ZIP, production FFmpeg and a Vulkan GPU"]
    async fn real_engines_verify_installation_and_compare_the_same_crop() {
        let archive = std::env::var_os("NABNAB_TEST_UPSCALE_ZIP")
            .map(PathBuf::from)
            .expect("Set NABNAB_TEST_UPSCALE_ZIP");
        let engines =
            fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("engines")).unwrap();
        let root = std::env::temp_dir().join(format!(
            "nabnab-upscale-test-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        fs::create_dir(&root).unwrap();
        let workspace = crate::workspaces::Workspace::new(&root).unwrap();
        let components = workspace.path.join("components");
        fs::create_dir(&components).unwrap();
        component::provision(&components, &archive).await.unwrap();
        let status = component::status(&components);
        assert!(status.installed && status.enabled);
        assert_eq!(status.installed_size, Some(43_636_188));
        let options = Options {
            model: Model::Illustration,
            scale: 3,
            tile: 64,
        };
        let runtime = component::ready(&components, &options.model_files()).unwrap();
        let source = workspace.path.join("input.png");
        let (_sender, cancel) = watch::channel(false);
        engine::run_process(
            engines.join("ffmpeg.exe"),
            [
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=200x120:rate=1",
                "-frames:v",
                "1",
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
        let dims = inspect(&engines, &source, options, cancel.clone())
            .await
            .unwrap();
        let sample = sample(
            &engines,
            &runtime,
            &workspace.path,
            &source,
            &dims,
            options,
            Anchor { x: 75, y: 0 },
            cancel.clone(),
            2,
        )
        .await
        .unwrap();
        assert_eq!(
            (
                sample.crop_width,
                sample.crop_height,
                sample.crop_x,
                sample.crop_y
            ),
            (160, 120, 30, 0)
        );
        assert_ne!(sample.before, sample.after);
        let output = execute(
            &engines,
            &runtime,
            &workspace.path,
            &source,
            options,
            cancel,
            3,
        )
        .await
        .unwrap();
        assert_eq!(image_assets::png_dimensions(&output).unwrap(), (600, 360));
        assert!(!workspace.path.join("normalized-3.png").exists());
        assert_eq!(fs::read(&source).unwrap(), original);
        fs::write(
            runtime.join("models/realesr-animevideov3-x3.param"),
            b"changed model",
        )
        .unwrap();
        assert!(component::ready(&components, &options.model_files()).is_err());
        component::remove(&components).unwrap();
        assert!(!component::status(&components).installed);
        drop(workspace);
        fs::remove_dir(root.join("local-previews")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
