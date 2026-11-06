use crate::{
    engine,
    library::{self, LibraryItem},
    model::Progress,
    AppState,
};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::State;
use tokio::sync::watch;

const MAX_PIXELS: usize = 128 * 128;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Color {
    rgb: [u8; 3],
    weight: u64,
}
#[derive(Clone, Copy, Default)]
struct Accumulator {
    weight: u64,
    sums: [u64; 3],
}
impl Accumulator {
    fn add(&mut self, rgb: [u8; 3], weight: u64) {
        self.weight += weight;
        for (channel, sum) in self.sums.iter_mut().enumerate() {
            *sum += rgb[channel] as u64 * weight;
        }
    }
    fn color(self) -> Color {
        Color {
            rgb: self
                .sums
                .map(|sum| ((sum + self.weight / 2) / self.weight) as u8),
            weight: self.weight,
        }
    }
}
fn distance(a: [u8; 3], b: [u8; 3]) -> u64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| {
            let delta = a as i32 - b as i32;
            (delta * delta) as u64
        })
        .sum()
}
fn add_rgba(histogram: &mut BTreeMap<u16, Accumulator>, pixels: &[u8]) -> Result<(), String> {
    if pixels.is_empty() || pixels.len() % 4 != 0 || pixels.len() > MAX_PIXELS * 4 {
        return Err("The image sampler returned invalid or oversized RGBA data.".into());
    }
    for pixel in pixels.chunks_exact(4) {
        let alpha = pixel[3] as u64;
        if alpha == 0 {
            continue;
        }
        let rgb = [pixel[0], pixel[1], pixel[2]];
        let key = ((rgb[0] as u16 >> 3) << 10) | ((rgb[1] as u16 >> 3) << 5) | (rgb[2] as u16 >> 3);
        histogram.entry(key).or_default().add(rgb, alpha);
    }
    Ok(())
}
fn representative_colors(
    histogram: &BTreeMap<u16, Accumulator>,
    count: u8,
) -> Result<Vec<[u8; 3]>, String> {
    if !(2..=16).contains(&count) {
        return Err("Choose a maximum of 2–16 palette colors.".into());
    }
    let mut bins: Vec<_> = histogram
        .values()
        .copied()
        .filter(|bin| bin.weight > 0)
        .map(Accumulator::color)
        .collect();
    if bins.is_empty() {
        return Err("The selected images contain no visible sampled pixels.".into());
    }
    bins.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.rgb.cmp(&b.rgb)));
    let mut centers = vec![bins[0].rgb];
    while centers.len() < (count as usize).min(bins.len()) {
        let next = bins
            .iter()
            .max_by_key(|bin| {
                centers
                    .iter()
                    .map(|center| distance(*center, bin.rgb))
                    .min()
                    .unwrap_or(0)
                    * bin.weight
            })
            .unwrap();
        if centers.contains(&next.rgb) {
            break;
        }
        centers.push(next.rgb);
    }
    let mut clusters = vec![Accumulator::default(); centers.len()];
    for _ in 0..12 {
        clusters.fill(Accumulator::default());
        for bin in &bins {
            let index = centers
                .iter()
                .enumerate()
                .min_by_key(|(_, center)| distance(**center, bin.rgb))
                .unwrap()
                .0;
            clusters[index].add(bin.rgb, bin.weight);
        }
        let updated: Vec<_> = clusters
            .iter()
            .enumerate()
            .map(|(index, cluster)| {
                if cluster.weight > 0 {
                    cluster.color().rgb
                } else {
                    centers[index]
                }
            })
            .collect();
        if updated == centers {
            break;
        }
        centers = updated;
    }
    let mut result: Vec<_> = clusters
        .into_iter()
        .filter(|cluster| cluster.weight > 0)
        .map(Accumulator::color)
        .collect();
    result.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.rgb.cmp(&b.rgb)));
    let mut colors = Vec::new();
    for color in result {
        if !colors.contains(&color.rgb) {
            colors.push(color.rgb);
        }
    }
    Ok(colors)
}
fn hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}
fn encode(colors: &[[u8; 3]], format: &str) -> Result<String, String> {
    match format {
        "hex" => Ok(colors
            .iter()
            .copied()
            .map(hex)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"),
        "css" => {
            let mut text = String::from(":root {\n");
            for (index, color) in colors.iter().enumerate() {
                text.push_str(&format!("  --palette-{}: {};\n", index + 1, hex(*color)));
            }
            text.push_str("}\n");
            Ok(text)
        }
        _ => Err("Choose HEX text or CSS variables.".into()),
    }
}
fn image_format(path: &Path) -> Result<&'static str, String> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Ok("png_pipe"),
        "jpg" | "jpeg" => Ok("jpeg_pipe"),
        "gif" => Ok("gif"),
        "webp" => Ok("webp_pipe"),
        "bmp" => Ok("bmp_pipe"),
        "tif" | "tiff" => Ok("tiff_pipe"),
        "avif" | "heif" | "heic" => Ok("mov"),
        _ => Err("Choose JPEG, PNG, WebP, GIF, BMP, TIFF or a supported AVIF/HEIF image.".into()),
    }
}

pub async fn execute(
    engines: &Path,
    inputs: &[LibraryItem],
    staging: &Path,
    colors: u8,
    format: &str,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    if inputs.is_empty()
        || inputs.len() > 200
        || !(2..=16).contains(&colors)
        || !matches!(format, "hex" | "css")
    {
        return Err("Choose 1–200 images, 2–16 colors and HEX or CSS output.".into());
    }
    if inputs.iter().any(|input| input.kind != "image") {
        return Err("Palette extraction accepts library images only.".into());
    }
    let staging = library::authorized_path(staging)?;
    let mut histogram = BTreeMap::new();
    for (index, item) in inputs.iter().enumerate() {
        if *cancel.borrow() {
            return Err(engine::CANCELLED.into());
        }
        let input = library::authorized_path(&item.path)?;
        let input_format = image_format(&input)?;
        if fs::metadata(&input).map_err(|e| e.to_string())?.len() > 256 * 1024 * 1024 {
            return Err("Palette inputs are limited to 256 MiB per image.".into());
        }
        let sample = staging.join(format!("palette-sample-{run_id}-{index}.rgba"));
        let args = vec![
            "-nostdin".into(),
            "-v".into(),
            "error".into(),
            "-n".into(),
            "-protocol_whitelist".into(),
            "file,pipe".into(),
            "-format_whitelist".into(),
            input_format.into(),
            "-f".into(),
            input_format.into(),
            "-i".into(),
            input.to_string_lossy().into_owned(),
            "-frames:v".into(),
            "1".into(),
            "-vf".into(),
            "scale=128:128:force_original_aspect_ratio=decrease:flags=area,format=rgba".into(),
            "-pix_fmt".into(),
            "rgba".into(),
            "-f".into(),
            "rawvideo".into(),
            sample.to_string_lossy().into_owned(),
        ];
        engine::run_process(
            engines.join("ffmpeg.exe"),
            args,
            cancel.clone(),
            run_id,
            false,
            emit.clone(),
        )
        .await?;
        let sample = crate::checked_output(&staging, &sample.to_string_lossy())?;
        if fs::metadata(&sample).map_err(|e| e.to_string())?.len() as usize > MAX_PIXELS * 4 {
            return Err("The palette image sample exceeds its pixel limit.".into());
        }
        add_rgba(
            &mut histogram,
            &fs::read(&sample).map_err(|e| e.to_string())?,
        )?;
        let _ = fs::remove_file(sample);
        emit(Progress {
            request_id: run_id,
            phase: "processing".into(),
            percent: Some((index + 1) as f64 / inputs.len() as f64 * 90.0),
            speed: None,
            downloaded: None,
            total: None,
        });
    }
    let colors = representative_colors(&histogram, colors)?;
    let text = encode(&colors, format)?;
    let output = staging.join(format!(
        "palette-{run_id}.{}",
        if format == "css" { "css" } else { "txt" }
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&output)
        .map_err(|_| {
            "The palette output already exists or cannot be created. No file was replaced."
        })?;
    file.write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    crate::checked_output(&staging, &output.to_string_lossy())
}

fn parse_colors(text: &str) -> Vec<String> {
    let mut colors = Vec::new();
    for token in text.split(|character: char| !character.is_ascii_hexdigit() && character != '#') {
        if token.len() == 7
            && token.starts_with('#')
            && token[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            let color = token.to_ascii_lowercase();
            if !colors.contains(&color) {
                colors.push(color);
                if colors.len() == 16 {
                    break;
                }
            }
        }
    }
    colors
}
#[tauri::command]
pub fn palette_preview(state: State<'_, Arc<AppState>>, id: String) -> Result<Vec<String>, String> {
    let path = {
        let guard = state.repository.lock().unwrap();
        let repo = guard.as_ref().ok_or("Open the app first.")?;
        repo.data
            .library
            .iter()
            .find(|item| item.id == id)
            .ok_or("Choose a saved palette file.")?
            .path
            .clone()
    };
    let path = library::authorized_path(&path)?;
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "txt" | "css"))
        || fs::metadata(&path).map_err(|e| e.to_string())?.len() > 64 * 1024
    {
        return Err("Choose a palette TXT or CSS file up to 64 KiB.".into());
    }
    let colors = parse_colors(
        &fs::read_to_string(path).map_err(|_| "The palette is not readable UTF-8 text.")?,
    );
    if colors.is_empty() {
        return Err("This file contains no six-digit HEX colors.".into());
    }
    Ok(colors)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_uses_actual_weighted_colors_and_ignores_fully_transparent_pixels() {
        let mut histogram = BTreeMap::new();
        add_rgba(
            &mut histogram,
            &[
                255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 255, 0, 255, 0, 0,
            ],
        )
        .unwrap();
        let colors = representative_colors(&histogram, 2).unwrap();
        assert_eq!(encode(&colors, "hex").unwrap(), "#ff0000\n#0000ff\n");
        assert_eq!(
            encode(&colors, "css").unwrap(),
            ":root {\n  --palette-1: #ff0000;\n  --palette-2: #0000ff;\n}\n"
        );
    }
    #[test]
    fn partial_alpha_is_weighted_and_solid_images_do_not_invent_colors() {
        let mut histogram = BTreeMap::new();
        add_rgba(&mut histogram, &[255, 0, 0, 20, 0, 0, 255, 255]).unwrap();
        assert_eq!(
            representative_colors(&histogram, 2).unwrap(),
            vec![[0, 0, 255], [255, 0, 0]]
        );
        let mut solid = BTreeMap::new();
        add_rgba(&mut solid, &[12, 34, 56, 255]).unwrap();
        assert_eq!(
            representative_colors(&solid, 16).unwrap(),
            vec![[12, 34, 56]]
        );
        assert!(representative_colors(&solid, 1).is_err());
    }
    #[test]
    fn invalid_samples_and_empty_visible_images_are_rejected() {
        let mut histogram = BTreeMap::new();
        assert!(add_rgba(&mut histogram, &[1, 2, 3]).is_err());
        add_rgba(&mut histogram, &[1, 2, 3, 0]).unwrap();
        assert!(representative_colors(&histogram, 4).is_err());
        assert_eq!(
            parse_colors(":root { --a: #FF0000; --b: #00ff00; --again: #ff0000 }"),
            vec!["#ff0000", "#00ff00"]
        );
    }
    #[tokio::test]
    #[ignore = "Requires bundled FFmpeg; samples synthetic local PNG images only."]
    async fn real_engines_palette_exports_actual_colors_without_modifying_images() {
        let engines = std::env::var_os("NABNAB_TEST_ENGINES")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engines"));
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let work =
            std::env::temp_dir().join(format!("nabnab-palette-{}-{nonce}", std::process::id()));
        fs::create_dir(&work).unwrap();
        let (_sender, cancel) = watch::channel(false);
        let mut inputs = Vec::new();
        let mut originals = Vec::new();
        for (index, color) in [[255u8, 0, 0, 255], [0, 0, 255, 255], [0, 255, 0, 0]]
            .into_iter()
            .enumerate()
        {
            let raw = work.join(format!("fixture-{index}.rgba"));
            fs::write(&raw, color.repeat(32 * 32)).unwrap();
            let image = work.join(format!("image-{index}.png"));
            let args = vec![
                "-nostdin".into(),
                "-v".into(),
                "error".into(),
                "-n".into(),
                "-f".into(),
                "rawvideo".into(),
                "-pix_fmt".into(),
                "rgba".into(),
                "-s".into(),
                "32x32".into(),
                "-i".into(),
                raw.to_string_lossy().into_owned(),
                "-frames:v".into(),
                "1".into(),
                "-c:v".into(),
                "png".into(),
                "-update".into(),
                "1".into(),
                image.to_string_lossy().into_owned(),
            ];
            engine::tool(&engines, "ffmpeg.exe", args, cancel.clone())
                .await
                .unwrap();
            originals.push(fs::read(&image).unwrap());
            inputs.push(library::index(&image, None, None, None).unwrap());
        }
        for (index, format) in ["hex", "css"].into_iter().enumerate() {
            let stage = work.join(format!("stage-{index}"));
            fs::create_dir(&stage).unwrap();
            let output = execute(
                &engines,
                &inputs,
                &stage,
                8,
                format,
                cancel.clone(),
                index as u64,
                |_| {},
            )
            .await
            .unwrap();
            assert_eq!(
                parse_colors(&fs::read_to_string(output).unwrap()),
                vec!["#0000ff", "#ff0000"]
            );
        }
        for (index, input) in inputs.iter().enumerate() {
            assert_eq!(fs::read(&input.path).unwrap(), originals[index]);
        }
        assert_eq!(work.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(work).unwrap();
    }
}
