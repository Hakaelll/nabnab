use crate::{
    engine,
    library::{self, LibraryItem},
    model::Progress,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use tokio::sync::watch;
mod pixels;
use pixels::Rgba;
const MAX_PIXELS: u64 = 24_000_000;
const MAX_PACKAGE: u64 = 256 * 1024 * 1024;
const ICON_SIZES: [u16; 11] = [16, 24, 32, 48, 64, 96, 128, 192, 256, 512, 1024];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Fit {
    Contain,
    Cover,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Order {
    RowMajor,
    ColumnMajor,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Background {
    Transparent,
    Solid { color: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Options {
    Stitch {
        cuts: Vec<u32>,
    },
    Icons {
        sizes: Vec<u16>,
        fit: Fit,
        background: Background,
    },
    Split {
        columns: u16,
        rows: u16,
        padding: u16,
        order: Order,
    },
    Pack {
        columns: u16,
        frame_width: u16,
        frame_height: u16,
        padding: u16,
        fit: Fit,
    },
}
impl Options {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Stitch { .. } => "stitch",
            Self::Icons { .. } => "icons",
            Self::Split { .. } => "split",
            Self::Pack { .. } => "pack",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageInfo {
    pub id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub index: usize,
    pub cut: u32,
    pub score: f64,
    pub ambiguous: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    pub images: Vec<ImageInfo>,
    pub suggestions: Vec<Suggestion>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputInfo {
    pub name: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub files: Vec<OutputInfo>,
    pub warnings: Vec<String>,
}
struct Built {
    primary: PathBuf,
    thumbnail: PathBuf,
    width: u32,
    height: u32,
    files: Vec<OutputInfo>,
    entries: Vec<(String, PathBuf)>,
    warnings: Vec<String>,
}

fn pixel_bytes(width: u32, height: u32) -> Result<usize, String> {
    let pixels = width as u64 * height as u64;
    if width == 0 || height == 0 || width > 32768 || height > 32768 || pixels > MAX_PIXELS {
        return Err(
            "Image canvases require 1-32768 pixels per side and at most 24 million pixels.".into(),
        );
    }
    Ok((pixels * 4) as usize)
}
fn color(value: &str) -> Result<[u8; 3], String> {
    if value.len() != 7
        || !value.starts_with('#')
        || !value[1..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Use a solid background in #RRGGBB format.".into());
    }
    Ok([
        u8::from_str_radix(&value[1..3], 16).unwrap(),
        u8::from_str_radix(&value[3..5], 16).unwrap(),
        u8::from_str_radix(&value[5..7], 16).unwrap(),
    ])
}
pub fn validate_options(options: &Options, count: usize) -> Result<(), String> {
    if count == 0 || count > 256 {
        return Err("Choose 1-256 library images in the intended order.".into());
    }
    match options {
        Options::Stitch { cuts } => {
            if !(2..=32).contains(&count)
                || cuts.len() != count - 1
                || cuts.iter().any(|cut| *cut > 16383)
            {
                return Err(
                    "Join 2-32 screenshots with one top cut per image after the first.".into(),
                );
            }
        }
        Options::Icons {
            sizes, background, ..
        } => {
            if count != 1
                || sizes.is_empty()
                || sizes.len() > ICON_SIZES.len()
                || !sizes.iter().all(|size| ICON_SIZES.contains(size))
                || !sizes.iter().any(|size| *size <= 256)
            {
                return Err("Choose one image and supported PNG sizes, including at least one size up to 256 for favicon.ico.".into());
            }
            let mut seen = std::collections::HashSet::new();
            if !sizes.iter().all(|s| seen.insert(s)) {
                return Err("Each icon size must be selected only once.".into());
            }
            if let Background::Solid { color: rgb } = background {
                color(rgb)?;
            }
        }
        Options::Split {
            columns,
            rows,
            padding,
            ..
        } => {
            if count != 1
                || *columns == 0
                || *rows == 0
                || *columns as u32 * *rows as u32 > 256
                || *padding > 256
            {
                return Err("Split one image into 1-256 frames with up to 256 pixels of spacing between cells.".into());
            }
        }
        Options::Pack {
            columns,
            frame_width,
            frame_height,
            padding,
            ..
        } => {
            if *columns == 0
                || *columns as usize > count
                || !(1..=4096).contains(frame_width)
                || !(1..=4096).contains(frame_height)
                || *padding > 256
            {
                return Err(
                    "Choose columns within the image count, frame sizes 1-4096 and padding 0-256."
                        .into(),
                );
            }
            pack_dimensions(count, *columns, *frame_width, *frame_height, *padding)?;
        }
    }
    Ok(())
}
fn pack_dimensions(
    count: usize,
    columns: u16,
    width: u16,
    height: u16,
    padding: u16,
) -> Result<(u32, u32), String> {
    let rows = count.div_ceil(columns as usize) as u32;
    let columns = columns as u32;
    let width = columns * width as u32 + (columns + 1) * padding as u32;
    let height = rows * height as u32 + (rows + 1) * padding as u32;
    pixel_bytes(width, height)?;
    Ok((width, height))
}
fn demuxer(path: &Path) -> Result<&'static str, String> {
    match path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Ok("png_pipe"),
        "jpg" | "jpeg" => Ok("jpeg_pipe"),
        _ => Err(
            "Image asset tools accept local PNG and JPEG files. Convert other formats first."
                .into(),
        ),
    }
}
async fn probe_inputs(
    engines: &Path,
    inputs: &[LibraryItem],
    cancel: watch::Receiver<bool>,
) -> Result<Vec<ImageInfo>, String> {
    if inputs.is_empty() || inputs.len() > 256 {
        return Err("Choose 1-256 library images.".into());
    }
    let mut total = 0u64;
    let mut infos = Vec::new();
    for item in inputs {
        if *cancel.borrow() {
            return Err(engine::CANCELLED.into());
        }
        if item.kind != "image" {
            return Err("Choose image records from the library.".into());
        }
        let path = library::authorized_path(&item.path)?;
        demuxer(&path)?;
        let length = fs::metadata(&path).map_err(|e| e.to_string())?.len();
        if length > 64 * 1024 * 1024 {
            return Err("Each image is limited to 64 MiB.".into());
        }
        total += length;
        if total > 512 * 1024 * 1024 {
            return Err("The selected source files exceed the 512 MiB budget.".into());
        }
        let value = crate::media::probe(engines, &path, cancel.clone()).await?;
        let stream = value["streams"]
            .as_array()
            .and_then(|list| list.iter().find(|v| v["codec_type"] == "video"))
            .ok_or("The image has no decodable frame.")?;
        let width = stream["width"]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or("Invalid image width.")?;
        let height = stream["height"]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .ok_or("Invalid image height.")?;
        pixel_bytes(width, height)?;
        if width > 16384 || height > 16384 {
            return Err("Source images are limited to 16384 pixels per side.".into());
        }
        infos.push(ImageInfo {
            id: item.id.clone(),
            name: item.name.clone(),
            width,
            height,
        });
    }
    Ok(infos)
}
fn owned_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| "A generated image path already exists. Existing files were preserved.")?;
    output
        .write_all(bytes)
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())
}
fn png_dimensions(path: &Path) -> Result<(u32, u32), String> {
    use std::io::Read;
    let mut header = [0u8; 24];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut header))
        .map_err(|_| "The generated PNG header is incomplete.")?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" {
        return Err("The image engine did not return PNG data.".into());
    }
    let width = u32::from_be_bytes(header[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(header[20..24].try_into().unwrap());
    pixel_bytes(width, height)?;
    Ok((width, height))
}
fn check_png(staging: &Path, path: &Path, width: u32, height: u32) -> Result<PathBuf, String> {
    let file = crate::checked_output(staging, &path.to_string_lossy())?;
    if png_dimensions(&file) != (Ok((width, height))) {
        return Err("The generated PNG does not match its planned dimensions.".into());
    }
    Ok(file)
}
async fn decode(
    engines: &Path,
    input: &Path,
    width: u32,
    height: u32,
    filter: Option<String>,
    staging: &Path,
    stem: &str,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<Rgba, String> {
    let expected = pixel_bytes(width, height)?;
    let input = library::authorized_path(input)?;
    let raw = staging.join(format!("{stem}.rgba"));
    let mut args = vec![
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-n".into(),
        "-protocol_whitelist".into(),
        "file,pipe".into(),
        "-format_whitelist".into(),
        demuxer(&input)?.into(),
        "-f".into(),
        demuxer(&input)?.into(),
        "-noautorotate".into(),
        "-i".into(),
        input.to_string_lossy().into_owned(),
        "-frames:v".into(),
        "1".into(),
    ];
    args.extend([
        "-vf".into(),
        filter.unwrap_or_else(|| "format=rgba".into()),
        "-pix_fmt".into(),
        "rgba".into(),
        "-f".into(),
        "rawvideo".into(),
        raw.to_string_lossy().into_owned(),
    ]);
    engine::run_process(
        engines.join("ffmpeg.exe"),
        args,
        cancel,
        run_id,
        false,
        |_| {},
    )
    .await?;
    let file = crate::checked_output(staging, &raw.to_string_lossy())?;
    if fs::metadata(&file).map_err(|e| e.to_string())?.len() != expected as u64 {
        return Err("The decoded image does not match the bounded pixel plan.".into());
    }
    let bytes = fs::read(&file).map_err(|e| e.to_string())?;
    fs::remove_file(file).map_err(|e| e.to_string())?;
    Ok(Rgba {
        width,
        height,
        bytes,
    })
}
async fn fitted(
    engines: &Path,
    item: &LibraryItem,
    width: u32,
    height: u32,
    fit: Fit,
    staging: &Path,
    stem: &str,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<Rgba, String> {
    let filter=match fit{Fit::Contain=>format!("format=rgba,scale={width}:{height}:force_original_aspect_ratio=decrease:flags=lanczos,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black@0"),Fit::Cover=>format!("format=rgba,scale={width}:{height}:force_original_aspect_ratio=increase:flags=lanczos,crop={width}:{height}")};
    decode(
        engines,
        &item.path,
        width,
        height,
        Some(filter),
        staging,
        stem,
        cancel,
        run_id,
    )
    .await
}
async fn write_png(
    engines: &Path,
    image: &Rgba,
    staging: &Path,
    name: &str,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<PathBuf, String> {
    let raw = staging.join(format!("{name}.rgba"));
    let output = staging.join(name);
    owned_bytes(&raw, &image.bytes)?;
    let args = vec![
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-n".into(),
        "-protocol_whitelist".into(),
        "file,pipe".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pixel_format".into(),
        "rgba".into(),
        "-video_size".into(),
        format!("{}x{}", image.width, image.height),
        "-i".into(),
        raw.to_string_lossy().into_owned(),
        "-frames:v".into(),
        "1".into(),
        "-c:v".into(),
        "png".into(),
        "-pix_fmt".into(),
        "rgba".into(),
        "-f".into(),
        "image2".into(),
        "-update".into(),
        "1".into(),
        output.to_string_lossy().into_owned(),
    ];
    let result = engine::run_process(
        engines.join("ffmpeg.exe"),
        args,
        cancel,
        run_id,
        false,
        |_| {},
    )
    .await;
    let _ = fs::remove_file(raw);
    result?;
    check_png(staging, &output, image.width, image.height)
}
fn progress(run_id: u64, index: usize, total: usize, emit: &impl Fn(Progress)) {
    emit(Progress {
        request_id: run_id,
        phase: "processing".into(),
        percent: Some(index as f64 / total.max(1) as f64 * 90.0),
        speed: None,
        downloaded: None,
        total: None,
    });
}
pub async fn inspect(
    engines: &Path,
    inputs: &[LibraryItem],
    staging: &Path,
    stitch: bool,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<Inspection, String> {
    let staging = library::authorized_path(staging)?;
    let images = probe_inputs(engines, inputs, cancel.clone()).await?;
    let mut suggestions = Vec::new();
    let mut warnings=vec!["Uses the stored pixel orientation and first frame. PNG transparency is preserved; rotate a copy first if its orientation needs changing.".into()];
    if stitch {
        if !(2..=32).contains(&inputs.len()) {
            return Err("Choose 2-32 screenshots for overlap inspection.".into());
        }
        let width = images[0].width;
        if images.iter().any(|i| i.width != width) {
            return Err(
                "Screenshots need equal pixel widths. Resize or crop a copy before joining.".into(),
            );
        }
        let mut previous = None;
        for (index, (input, info)) in inputs.iter().zip(&images).enumerate() {
            let sample = decode(
                engines,
                &input.path,
                64,
                info.height,
                Some(format!("scale=64:{}:flags=area,format=rgba", info.height)),
                &staging,
                &format!("overlap-{index}"),
                cancel.clone(),
                run_id,
            )
            .await?;
            if let Some(first) = previous {
                suggestions.push(pixels::suggest_overlap(&first, &sample, index, &cancel)?);
            }
            previous = Some(sample);
        }
        warnings.push("Overlap is an estimate over sampled rows, limited to 4096 pixels. Repeated headers, flat areas and animation can produce misleading matches. Review each cut.".into());
    }
    Ok(Inspection {
        images,
        suggestions,
        warnings,
    })
}
fn write_json(staging: &Path, name: &str, value: &serde_json::Value) -> Result<PathBuf, String> {
    let path = staging.join(name);
    owned_bytes(
        &path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )?;
    Ok(path)
}
fn write_ico(staging: &Path, icons: &[(u16, PathBuf)]) -> Result<PathBuf, String> {
    let selected: Vec<_> = icons.iter().filter(|(size, _)| *size <= 256).collect();
    let mut payloads = Vec::new();
    for (size, path) in &selected {
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        if bytes.len() > 16 * 1024 * 1024 || png_dimensions(path)? != (*size as u32, *size as u32) {
            return Err("Invalid bounded PNG icon data.".into());
        }
        payloads.push(bytes);
    }
    let mut output = Vec::new();
    output.extend_from_slice(&[0, 0, 1, 0]);
    output.extend_from_slice(&(selected.len() as u16).to_le_bytes());
    let mut offset = 6 + selected.len() * 16;
    for ((size, _), bytes) in selected.iter().zip(&payloads) {
        let side = if *size == 256 { 0 } else { *size as u8 };
        output.extend_from_slice(&[side, side, 0, 0]);
        output.extend_from_slice(&1u16.to_le_bytes());
        output.extend_from_slice(&32u16.to_le_bytes());
        output.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        output.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += bytes.len();
    }
    for bytes in payloads {
        output.extend(bytes);
    }
    let path = staging.join("favicon.ico");
    owned_bytes(&path, &output)?;
    Ok(path)
}
async fn build(
    engines: &Path,
    inputs: &[LibraryItem],
    staging: &Path,
    options: &Options,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<Built, String> {
    validate_options(options, inputs.len())?;
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let staging = library::authorized_path(staging)?;
    let images = probe_inputs(engines, inputs, cancel.clone()).await?;
    let mut files = Vec::new();
    let mut entries = Vec::new();
    let mut warnings=vec!["PNG/JPEG inputs use their stored pixel orientation and first frame. Output PNGs retain RGBA transparency. Originals are never rewritten.".into()];
    let (primary, thumbnail, width, height) = match options {
        Options::Stitch { cuts } => {
            let width = images[0].width;
            if images.iter().any(|i| i.width != width) {
                return Err("Screenshots need the same pixel width before joining.".into());
            }
            let mut height = images[0].height;
            for (index, cut) in cuts.iter().enumerate() {
                if *cut >= images[index + 1].height {
                    return Err(
                        "A screenshot cut removes the entire image. Reduce its top cut.".into(),
                    );
                }
                height = height
                    .checked_add(images[index + 1].height - cut)
                    .ok_or("The joined screenshot is too tall.")?;
            }
            let mut canvas = Rgba::blank(width, height)?;
            let mut y = 0;
            for (index, (item, info)) in inputs.iter().zip(&images).enumerate() {
                let rgba = decode(
                    engines,
                    &item.path,
                    info.width,
                    info.height,
                    None,
                    &staging,
                    &format!("stitch-input-{index}"),
                    cancel.clone(),
                    run_id,
                )
                .await?;
                let cut = if index == 0 { 0 } else { cuts[index - 1] };
                canvas.paste(&rgba, 0, y, cut)?;
                y += info.height - cut;
                progress(run_id, index + 1, inputs.len(), &emit);
            }
            let file = write_png(
                engines,
                &canvas,
                &staging,
                "joined-screenshots.png",
                cancel.clone(),
                run_id,
            )
            .await?;
            files.push(OutputInfo {
                name: "joined-screenshots.png".into(),
                width,
                height,
            });
            warnings.push("Cuts remove top rows from each screenshot after the first. No automatic match is applied silently.".into());
            (file.clone(), file, width, height)
        }
        Options::Icons {
            sizes,
            fit,
            background,
        } => {
            let mut icons = Vec::new();
            let mut preview = None;
            let mut sorted = sizes.clone();
            sorted.sort_unstable();
            for (index, size) in sorted.iter().enumerate() {
                let mut rgba = fitted(
                    engines,
                    &inputs[0],
                    *size as u32,
                    *size as u32,
                    *fit,
                    &staging,
                    &format!("icon-input-{size}"),
                    cancel.clone(),
                    run_id,
                )
                .await?;
                if let Background::Solid { color: rgb } = background {
                    rgba.flatten(color(rgb)?);
                }
                let name = format!("icon-{size}.png");
                let file =
                    write_png(engines, &rgba, &staging, &name, cancel.clone(), run_id).await?;
                entries.push((name.clone(), file.clone()));
                files.push(OutputInfo {
                    name,
                    width: *size as u32,
                    height: *size as u32,
                });
                icons.push((*size, file.clone()));
                preview = Some(file);
                progress(run_id, index + 1, sorted.len(), &emit);
            }
            entries.push(("favicon.ico".into(), write_ico(&staging, &icons)?));
            let ico_size = *sorted.iter().filter(|size| **size <= 256).max().unwrap();
            files.push(OutputInfo {
                name: "favicon.ico".into(),
                width: ico_size as u32,
                height: ico_size as u32,
            });
            let manifest = write_json(
                &staging,
                "manifest.json",
                &json!({"schemaVersion":1,"kind":"iconPackage","sourceName":images[0].name,"sizes":sorted,"fit":fit,"background":background,"icoEncoding":"PNG payload entries, selected sizes up to 256 pixels","outputs":files,"compatibility":"Validate the package in each target application; no universal compatibility is claimed."}),
            )?;
            entries.push(("manifest.json".into(), manifest));
            warnings.push("favicon.ico embeds the selected PNG sizes up to 256. Larger PNGs stay separate. Test the package in its intended browser or application.".into());
            let preview = preview.unwrap();
            let (w, h) = png_dimensions(&preview)?;
            (staging.join("icon-package.zip"), preview, w, h)
        }
        Options::Split {
            columns,
            rows,
            padding,
            order,
        } => {
            let source = &images[0];
            let gap_x = (*columns as u32 - 1) * *padding as u32;
            let gap_y = (*rows as u32 - 1) * *padding as u32;
            if gap_x >= source.width
                || gap_y >= source.height
                || (source.width - gap_x) % *columns as u32 != 0
                || (source.height - gap_y) % *rows as u32 != 0
            {
                return Err("The grid dimensions and spacing must divide the source exactly, without an outer margin.".into());
            }
            let width = (source.width - gap_x) / *columns as u32;
            let height = (source.height - gap_y) / *rows as u32;
            let rgba = decode(
                engines,
                &inputs[0].path,
                source.width,
                source.height,
                None,
                &staging,
                "split-source",
                cancel.clone(),
                run_id,
            )
            .await?;
            let count = *columns as usize * *rows as usize;
            let preview_count = count.min(16);
            let preview_columns = (preview_count as f64).sqrt().ceil() as u32;
            let mut contact = Rgba::blank(
                preview_columns * 128,
                (preview_count as u32).div_ceil(preview_columns) * 128,
            )?;
            let mut frames = Vec::new();
            for index in 0..count {
                if *cancel.borrow() {
                    return Err(engine::CANCELLED.into());
                }
                let (column, row) = match order {
                    Order::RowMajor => (index % *columns as usize, index / *columns as usize),
                    Order::ColumnMajor => (index / *rows as usize, index % *rows as usize),
                };
                let x = column as u32 * (width + *padding as u32);
                let y = row as u32 * (height + *padding as u32);
                let frame = rgba.crop(x, y, width, height)?;
                let name = format!("frame-{index:04}.png");
                let path =
                    write_png(engines, &frame, &staging, &name, cancel.clone(), run_id).await?;
                entries.push((name.clone(), path));
                files.push(OutputInfo {
                    name: name.clone(),
                    width,
                    height,
                });
                frames.push(json!({"index":index,"file":name,"source":{"x":x,"y":y,"width":width,"height":height}}));
                if index < preview_count {
                    contact.paste(
                        &frame.thumbnail(128)?,
                        (index as u32 % preview_columns) * 128,
                        (index as u32 / preview_columns) * 128,
                        0,
                    )?;
                }
                progress(run_id, index + 1, count, &emit);
            }
            entries.push(("manifest.json".into(),write_json(&staging,"manifest.json",&json!({"schemaVersion":1,"kind":"gridSplit","sourceName":source.name,"sourceWidth":source.width,"sourceHeight":source.height,"columns":columns,"rows":rows,"paddingBetweenCells":padding,"order":order,"frames":frames}))?));
            let thumbnail = write_png(
                engines,
                &contact,
                &staging,
                "frames-preview.png",
                cancel.clone(),
                run_id,
            )
            .await?;
            warnings.push(format!("Preview shows the first {preview_count} of {count} frames. The ZIP contains every frame and its source coordinates."));
            (staging.join("grid-frames.zip"), thumbnail, width, height)
        }
        Options::Pack {
            columns,
            frame_width,
            frame_height,
            padding,
            fit,
        } => {
            let (width, height) = pack_dimensions(
                inputs.len(),
                *columns,
                *frame_width,
                *frame_height,
                *padding,
            )?;
            let mut canvas = Rgba::blank(width, height)?;
            let mut frames = Vec::new();
            for (index, input) in inputs.iter().enumerate() {
                let rgba = fitted(
                    engines,
                    input,
                    *frame_width as u32,
                    *frame_height as u32,
                    *fit,
                    &staging,
                    &format!("pack-frame-{index}"),
                    cancel.clone(),
                    run_id,
                )
                .await?;
                let x = *padding as u32
                    + (index as u32 % *columns as u32) * (*frame_width as u32 + *padding as u32);
                let y = *padding as u32
                    + (index as u32 / *columns as u32) * (*frame_height as u32 + *padding as u32);
                canvas.paste(&rgba, x, y, 0)?;
                frames.push(json!({"index":index,"name":images[index].name,"frame":{"x":x,"y":y,"width":frame_width,"height":frame_height},"rotated":false,"trimmed":false}));
                progress(run_id, index + 1, inputs.len(), &emit);
            }
            let atlas = write_png(
                engines,
                &canvas,
                &staging,
                "atlas.png",
                cancel.clone(),
                run_id,
            )
            .await?;
            files.push(OutputInfo {
                name: "atlas.png".into(),
                width,
                height,
            });
            entries.push(("atlas.png".into(), atlas.clone()));
            entries.push(("atlas.json".into(),write_json(&staging,"atlas.json",&json!({"schemaVersion":1,"image":"atlas.png","width":width,"height":height,"frameWidth":frame_width,"frameHeight":frame_height,"columns":columns,"padding":padding,"outerPadding":padding,"fit":fit,"frames":frames}))?));
            warnings.push("Atlas coordinates use pixels from the top-left corner. Padding includes the outer border; frames retain the chosen input order.".into());
            (staging.join("sprite-atlas.zip"), atlas, width, height)
        }
    };
    let mut total = 0u64;
    for (_, file) in &entries {
        total = total
            .checked_add(fs::metadata(file).map_err(|e| e.to_string())?.len())
            .ok_or("The image package exceeds its byte budget.")?;
        if total > MAX_PACKAGE {
            return Err("The generated image package exceeds 256 MiB.".into());
        }
    }
    Ok(Built {
        primary,
        thumbnail,
        width,
        height,
        files,
        entries,
        warnings,
    })
}
pub async fn execute(
    engines: &Path,
    inputs: &[LibraryItem],
    staging: &Path,
    options: &Options,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    let built = build(
        engines,
        inputs,
        staging,
        options,
        cancel.clone(),
        run_id,
        emit.clone(),
    )
    .await?;
    if !built.entries.is_empty() {
        crate::portable_zip::write_generated(&built.primary, &built.entries, cancel.clone())
            .await?;
    }
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    crate::checked_output(staging, &built.primary.to_string_lossy())
}
pub async fn preview(
    engines: &Path,
    inputs: &[LibraryItem],
    staging: &Path,
    options: &Options,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<Preview, String> {
    let built = build(
        engines,
        inputs,
        staging,
        options,
        cancel.clone(),
        run_id,
        |_| {},
    )
    .await?;
    let (w, h) = png_dimensions(&built.thumbnail)?;
    let scale = (1200.0 / w as f64).min(1600.0 / h as f64).min(1.0);
    let width = (w as f64 * scale).round().max(1.0) as u32;
    let height = (h as f64 * scale).round().max(1.0) as u32;
    let rgba = decode(
        engines,
        &built.thumbnail,
        width,
        height,
        Some(format!("scale={width}:{height}:flags=area,format=rgba")),
        staging,
        "preview-input",
        cancel.clone(),
        run_id,
    )
    .await?;
    let path = write_png(engines, &rgba, staging, "asset-preview.png", cancel, run_id).await?;
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
        return Err("The image preview exceeds 8 MiB.".into());
    }
    Ok(Preview {
        png: fs::read(path).map_err(|e| e.to_string())?,
        width: built.width,
        height: built.height,
        files: built.files,
        warnings: built.warnings,
    })
}

#[cfg(test)]
mod tests;
