use crate::{engine, model::Progress};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tokio::sync::watch;

const MAX_INDEX: usize = 16 * 1024 * 1024;
const MAX_PAGES: usize = 200;
const MAX_WORDS: usize = 20_000;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Output {
    Text,
    SearchablePdf,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrOptions {
    pub languages: Vec<String>,
    pub pages: String,
    pub dpi: u16,
    pub region: Option<Region>,
    pub output: Output,
    pub acknowledge_structure_changes: bool,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    Original,
    Ocr,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub text: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub confidence: f32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrPage {
    pub page: u32,
    pub width: u32,
    pub height: u32,
    pub text: String,
    pub origin: Origin,
    pub words: Vec<Word>,
    #[serde(default)]
    pub region: Option<Region>,
    #[serde(default)]
    pub corrected: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrDocument {
    pub schema_version: u32,
    pub source_fingerprint: String,
    pub languages: Vec<String>,
    pub pages: Vec<OcrPage>,
}
#[derive(Debug)]
pub struct OcrResult {
    pub file: PathBuf,
    pub index: OcrDocument,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub page: u32,
    pub excerpt: String,
    pub origin: Origin,
    pub boxes: Vec<Region>,
}

pub fn validate_options(options: &OcrOptions) -> Result<(), String> {
    if options.languages.is_empty()
        || options.languages.len() > 2
        || options
            .languages
            .iter()
            .any(|language| !matches!(language.as_str(), "eng" | "spa"))
        || options.languages.iter().collect::<HashSet<_>>().len() != options.languages.len()
        || !(150..=300).contains(&options.dpi)
        || options.pages.is_empty()
        || options.pages.len() > 4096
        || !options
            .pages
            .bytes()
            .all(|byte| byte.is_ascii_digit() || b"N,- ".contains(&byte))
    {
        return Err(
            "Choose English and/or Spanish OCR, 150–300 DPI and a valid page selection.".into(),
        );
    }
    if let Some(region) = options.region {
        if region.width == 0
            || region.height == 0
            || u64::from(region.width) * u64::from(region.height) > 40_000_000
            || region.x.checked_add(region.width).is_none()
            || region.y.checked_add(region.height).is_none()
        {
            return Err("Choose a nonempty OCR region of at most 40 million pixels.".into());
        }
    }
    if options.output == Output::SearchablePdf && !options.acknowledge_structure_changes {
        return Err("Acknowledge that saving a searchable copy may change PDF structure, forms and signatures. Original signature validity is not preserved by this operation.".into());
    }
    Ok(())
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    output
        .write_all(bytes)
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())
}
fn cancellation(cancel: &watch::Receiver<bool>) -> Result<(), String> {
    if *cancel.borrow() {
        Err(engine::CANCELLED.into())
    } else {
        Ok(())
    }
}
async fn fingerprint(input: PathBuf, cancel: watch::Receiver<bool>) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let mut input = fs::File::open(input).map_err(|e| e.to_string())?;
        let mut bytes = [0; 65536];
        let mut hash = Sha256::new();
        loop {
            cancellation(&cancel)?;
            let count = input.read(&mut bytes).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            hash.update(&bytes[..count]);
        }
        Ok(format!("{:x}", hash.finalize()))
    })
    .await
    .map_err(|e| e.to_string())?
}
fn png_dimensions(path: &Path) -> Result<(u32, u32), String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
        return Err("The rendered OCR image exceeds 128 MiB.".into());
    }
    let mut header = [0; 24];
    file.read_exact(&mut header)
        .map_err(|_| "The OCR renderer returned an incomplete PNG.")?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" {
        return Err("The OCR renderer did not return a PNG image.".into());
    }
    let width = u32::from_be_bytes(
        header[16..20]
            .try_into()
            .map_err(|_| "Invalid PNG width.")?,
    );
    let height = u32::from_be_bytes(
        header[20..24]
            .try_into()
            .map_err(|_| "Invalid PNG height.")?,
    );
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 40_000_000 {
        return Err("OCR accepts images up to 40 million pixels per page.".into());
    }
    Ok((width, height))
}

pub fn parse_tsv(raw: &str, page: u32, width: u32, height: u32) -> Result<OcrPage, String> {
    if raw.len() > 8 * 1024 * 1024 || !raw.lines().next().is_some_and(|line| line.starts_with("level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext")) {
        return Err("The OCR engine returned invalid or excessive TSV coordinates.".into());
    }
    let mut words = Vec::new();
    let mut text = String::new();
    let mut previous = String::new();
    for line in raw.lines().skip(1) {
        let fields: Vec<_> = line.splitn(12, '\t').collect();
        if fields.len() != 12 {
            return Err("The OCR engine returned an incomplete TSV row.".into());
        }
        if fields[0] != "5" || fields[11].trim().is_empty() {
            continue;
        }
        let number = |index: usize| {
            fields[index]
                .parse::<u32>()
                .map_err(|_| "Invalid OCR word coordinates.")
        };
        let x = number(6)?;
        let y = number(7)?;
        let w = number(8)?;
        let h = number(9)?;
        let confidence = fields[10]
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite() && (-1.0..=100.0).contains(value))
            .ok_or("Invalid OCR confidence.")?;
        if u64::from(x) + u64::from(w) > u64::from(width)
            || u64::from(y) + u64::from(h) > u64::from(height)
            || w == 0
            || h == 0
        {
            return Err("The OCR word coordinates leave the rendered page.".into());
        }
        let word: String = fields[11]
            .trim()
            .chars()
            .filter(|ch| !ch.is_control())
            .take(512)
            .collect();
        let group = fields[2..5].join(":");
        if !text.is_empty() {
            text.push(if group == previous { ' ' } else { '\n' });
        }
        text.push_str(&word);
        previous = group;
        words.push(Word {
            text: word,
            x,
            y,
            width: w,
            height: h,
            confidence,
        });
        if words.len() > MAX_WORDS || text.len() > 1_000_000 {
            return Err("The OCR page exceeds its text or coordinate limit.".into());
        }
    }
    Ok(OcrPage {
        page,
        width,
        height,
        text,
        origin: Origin::Ocr,
        words,
        region: None,
        corrected: false,
    })
}
fn useful_text(text: &str) -> bool {
    text.chars().filter(|ch| ch.is_alphanumeric()).count() >= 12
        && text.split_whitespace().count() >= 2
}

async fn recognize(
    tesseract: &Path,
    models: &Path,
    image: &Path,
    output: &Path,
    options: &OcrOptions,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<(), String> {
    let args = vec![
        crate::documents::cli_path(image),
        crate::documents::cli_path(output),
        "--tessdata-dir".into(),
        crate::documents::cli_path(models),
        "-l".into(),
        options.languages.join("+"),
        "--oem".into(),
        "1".into(),
        "--psm".into(),
        "3".into(),
        "--dpi".into(),
        options.dpi.to_string(),
        "-c".into(),
        "tessedit_create_tsv=1".into(),
        "-c".into(),
        "tessedit_create_pdf=1".into(),
        "-c".into(),
        "textonly_pdf=1".into(),
        "-c".into(),
        "debug_file=/dev/null".into(),
    ];
    engine::run_process(tesseract.to_owned(), args, cancel, run_id, true, |_| {}).await?;
    Ok(())
}
async fn render_image(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    region: Option<Region>,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<(), String> {
    let probe = ffmpeg.with_file_name(if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });
    let raw = engine::run_process(
        probe,
        vec![
            "-v".into(),
            "error".into(),
            "-select_streams".into(),
            "v:0".into(),
            "-show_entries".into(),
            "stream=width,height".into(),
            "-of".into(),
            "json".into(),
            input.to_string_lossy().into_owned(),
        ],
        cancel.clone(),
        run_id,
        true,
        |_| {},
    )
    .await?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).map_err(|_| "The image dimensions could not be read.")?;
    let width = value["streams"][0]["width"]
        .as_u64()
        .ok_or("The image has no width.")?;
    let height = value["streams"][0]["height"]
        .as_u64()
        .ok_or("The image has no height.")?;
    if width == 0 || height == 0 || width.saturating_mul(height) > 40_000_000 {
        return Err(
            "OCR accepts images up to 40 million pixels. Crop or resize the image first.".into(),
        );
    }
    let mut args: Vec<String> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-n",
        "-threads",
        "2",
        "-i",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(input.to_string_lossy().into_owned());
    if let Some(region) = region {
        if u64::from(region.x) + u64::from(region.width) > width
            || u64::from(region.y) + u64::from(region.height) > height
        {
            return Err("The OCR region leaves the original image.".into());
        }
        args.extend([
            "-vf".into(),
            format!(
                "crop={}:{}:{}:{}:exact=1",
                region.width, region.height, region.x, region.y
            ),
        ]);
    }
    args.extend([
        "-frames:v".into(),
        "1".into(),
        "-threads".into(),
        "2".into(),
        output.to_string_lossy().into_owned(),
    ]);
    engine::run_process(ffmpeg.to_owned(), args, cancel, run_id, true, |_| {}).await?;
    Ok(())
}

pub async fn execute(
    tesseract: &Path,
    models: &Path,
    mutool: Option<&Path>,
    ffmpeg: &Path,
    input: &Path,
    staging: &Path,
    options: &OcrOptions,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<OcrResult, String> {
    validate_options(options)?;
    cancellation(&cancel)?;
    let input = crate::library::authorized_path(input)?;
    let staging = crate::library::authorized_path(staging)?;
    if !staging.is_dir()
        || !input.is_file()
        || fs::metadata(&input).map_err(|e| e.to_string())?.len() > 1024 * 1024 * 1024
    {
        return Err(
            "Choose an imported regular image or PDF up to 1 GiB and an owned job folder.".into(),
        );
    }
    let pdf = input
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    if !pdf
        && !input
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                [
                    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "avif", "heic", "heif",
                    "gif",
                ]
                .contains(&extension.to_ascii_lowercase().as_str())
            })
    {
        return Err("Choose an image or PDF for OCR.".into());
    }
    if pdf && options.region.is_some() {
        return Err(
            "Image regions apply to images only. Choose PDF pages for document OCR.".into(),
        );
    }
    if !pdf && options.output == Output::SearchablePdf {
        return Err("Searchable PDF keeps an original PDF's appearance. For an image, save the recognized text.".into());
    }
    let source_fingerprint = fingerprint(input.clone(), cancel.clone()).await?;
    let mut index = OcrDocument {
        schema_version: 1,
        source_fingerprint: source_fingerprint.clone(),
        languages: options.languages.clone(),
        pages: Vec::new(),
    };
    let pages = if pdf {
        let mutool = mutool.ok_or("Install and enable Documents · MuPDF for PDF OCR.")?;
        let info =
            crate::documents::inspect(mutool, &input, &staging, cancel.clone(), run_id).await?;
        if !info.copy || !info.print {
            return Err("The PDF does not permit text extraction and rendering for OCR. No restrictions were bypassed.".into());
        }
        crate::documents::page_selection(&options.pages, info.pages)?
            .into_iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    } else {
        vec![1]
    };
    if pages.is_empty() || pages.len() > MAX_PAGES {
        return Err("Recognize 1–200 PDF pages per job. Split larger selections into jobs.".into());
    }
    let mut pages = pages;
    pages.sort_unstable();
    let mut overlays: Vec<(u32, String)> = Vec::new();
    for (position, page) in pages.iter().enumerate() {
        cancellation(&cancel)?;
        let image = staging.join(format!("ocr-page-{page}.png"));
        let original = if pdf {
            engine::run_process(
                mutool.ok_or("MuPDF is unavailable.")?.to_owned(),
                vec![
                    "draw".into(),
                    "-q".into(),
                    "-F".into(),
                    "txt".into(),
                    crate::documents::cli_path(&input),
                    page.to_string(),
                ],
                cancel.clone(),
                run_id,
                true,
                |_| {},
            )
            .await?
        } else {
            String::new()
        };
        if useful_text(&original) {
            index.pages.push(OcrPage {
                page: *page,
                width: 0,
                height: 0,
                text: original.chars().take(1_000_000).collect(),
                origin: Origin::Original,
                words: vec![],
                region: None,
                corrected: false,
            });
        } else {
            if pdf {
                engine::run_process(
                    mutool.ok_or("MuPDF is unavailable.")?.to_owned(),
                    vec![
                        "draw".into(),
                        "-q".into(),
                        "-F".into(),
                        "png".into(),
                        "-r".into(),
                        options.dpi.to_string(),
                        "-w".into(),
                        "6000".into(),
                        "-h".into(),
                        "6000".into(),
                        "-T".into(),
                        "2".into(),
                        "-B".into(),
                        "256".into(),
                        "-o".into(),
                        crate::documents::cli_path(&image),
                        crate::documents::cli_path(&input),
                        page.to_string(),
                    ],
                    cancel.clone(),
                    run_id,
                    true,
                    |_| {},
                )
                .await?;
            } else {
                render_image(
                    ffmpeg,
                    &input,
                    &image,
                    options.region,
                    cancel.clone(),
                    run_id,
                )
                .await?;
            }
            let image = crate::checked_output(&staging, &image.to_string_lossy())?;
            let (width, height) = png_dimensions(&image)?;
            let base = staging.join(format!("ocr-result-{page}"));
            recognize(
                tesseract,
                models,
                &image,
                &base,
                options,
                cancel.clone(),
                run_id,
            )
            .await?;
            let tsv =
                crate::checked_output(&staging, &base.with_extension("tsv").to_string_lossy())?;
            if fs::metadata(&tsv).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
                return Err("The OCR coordinate output exceeds its size limit.".into());
            }
            let mut recognized = parse_tsv(
                &fs::read_to_string(&tsv).map_err(|e| e.to_string())?,
                *page,
                width,
                height,
            )?;
            recognized.region = options.region;
            index.pages.push(recognized);
            if options.output == Output::SearchablePdf {
                let overlay =
                    crate::checked_output(&staging, &base.with_extension("pdf").to_string_lossy())?;
                overlays.push((*page, crate::documents::cli_path(&overlay)));
            } else {
                let _ = fs::remove_file(base.with_extension("pdf"));
            }
            fs::remove_file(image).map_err(|e| e.to_string())?;
            fs::remove_file(tsv).map_err(|e| e.to_string())?;
        }
        validate_index(&index)?;
        emit(Progress {
            request_id: run_id,
            phase: "processing".into(),
            percent: Some((position + 1) as f64 / pages.len() as f64 * 90.0),
            speed: None,
            downloaded: None,
            total: None,
        });
    }
    cancellation(&cancel)?;
    if fingerprint(input.clone(), cancel.clone()).await? != source_fingerprint {
        return Err("The source changed during OCR. The result was not published; the original was not modified.".into());
    }
    let file = staging.join(if options.output == Output::SearchablePdf {
        "searchable.pdf"
    } else {
        "recognized.txt"
    });
    if options.output == Output::SearchablePdf {
        let mut script = String::from(include_str!("document_edits/overlay.js"));
        script.push_str("\nvar doc=Document.openDocument(scriptArgs[0]);if(doc.needsPassword())throw new Error('Authorized decrypted PDF required.');var plan=JSON.parse(readFile(scriptArgs[1]).asString());for(var i=0;i<plan.length;++i){var overlay=Document.openDocument(plan[i][1]);nabnabAppendOverlay(doc,plan[i][0]-1,overlay,0);}doc.save(scriptArgs[2],'compress');");
        let driver = staging.join("nabnab-ocr-overlay.js");
        write_new(&driver, script.as_bytes())?;
        let plan = staging.join("ocr-overlays.json");
        write_new(
            &plan,
            &serde_json::to_vec(&overlays).map_err(|e| e.to_string())?,
        )?;
        engine::run_process(
            mutool.ok_or("MuPDF is unavailable.")?.to_owned(),
            vec![
                "run".into(),
                crate::documents::cli_path(&driver),
                crate::documents::cli_path(&input),
                crate::documents::cli_path(&plan),
                crate::documents::cli_path(&file),
            ],
            cancel.clone(),
            run_id,
            true,
            |_| {},
        )
        .await?;
        for (_, overlay) in overlays {
            fs::remove_file(PathBuf::from(overlay)).map_err(|e| e.to_string())?;
        }
    } else {
        write_new(&file, corrected_text(&index).as_bytes())?;
    }
    cancellation(&cancel)?;
    let file = crate::checked_output(&staging, &file.to_string_lossy())?;
    Ok(OcrResult { file, index })
}

fn validate_index(index: &OcrDocument) -> Result<(), String> {
    if index.schema_version != 1
        || index.source_fingerprint.len() != 64
        || !index
            .source_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || index.pages.len() > MAX_PAGES
        || index.languages.is_empty()
        || index.languages.len() > 2
        || index
            .languages
            .iter()
            .any(|language| !matches!(language.as_str(), "eng" | "spa"))
    {
        return Err("Invalid OCR index identity or scope.".into());
    }
    let mut known = HashSet::new();
    for page in &index.pages {
        if page.page == 0
            || page.page > 10_000
            || !known.insert(page.page)
            || page.text.len() > 1_000_000
            || page.words.len() > MAX_WORDS
            || u64::from(page.width) * u64::from(page.height) > 40_000_000
            || page
                .text
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t' | '\u{000c}'))
        {
            return Err("The OCR index contains invalid or excessive page data.".into());
        }
        if let Some(region) = page.region {
            if region.width != page.width
                || region.height != page.height
                || region.x.checked_add(region.width).is_none()
                || region.y.checked_add(region.height).is_none()
            {
                return Err("The OCR index region does not match its recognized image.".into());
            }
        }
        for word in &page.words {
            if word.text.len() > 2048
                || word.text.chars().any(char::is_control)
                || !word.confidence.is_finite()
                || !(-1.0..=100.0).contains(&word.confidence)
                || u64::from(word.x) + u64::from(word.width) > u64::from(page.width)
                || u64::from(word.y) + u64::from(word.height) > u64::from(page.height)
            {
                return Err("Invalid OCR word text or coordinates.".into());
            }
        }
    }
    if serde_json::to_vec(index).map_err(|e| e.to_string())?.len() > MAX_INDEX {
        return Err("The OCR index exceeds 16 MiB. Choose fewer pages.".into());
    }
    Ok(())
}
fn reference_name(reference: &str) -> Result<&str, String> {
    let id = reference
        .strip_prefix("ocr:")
        .ok_or("Invalid OCR index reference.")?;
    if id.is_empty()
        || id.len() > 160
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
    {
        return Err("Invalid OCR index identity.".into());
    }
    Ok(id)
}
pub fn write_index(root: &Path, job_id: &str, index: &OcrDocument) -> Result<String, String> {
    validate_index(index)?;
    let reference = format!("ocr:{job_id}");
    let id = reference_name(&reference)?;
    let directory = root.join("ocr-index");
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let directory = crate::library::authorized_path(&directory)?;
    let path=directory.join(format!("{id}.json"));
    let bytes=serde_json::to_vec(index).map_err(|e|e.to_string())?;
    if fs::symlink_metadata(&path).is_ok(){
        let existing=crate::library::authorized_path(&path)?;
        if existing.parent()!=Some(directory.as_path())||fs::metadata(&existing).map_err(|e|e.to_string())?.len()>MAX_INDEX as u64{return Err("Invalid existing OCR index path or size.".into());}
        if fs::read(existing).map_err(|e|e.to_string())?==bytes{return Ok(reference);}
        return Err("This OCR attempt already owns a different index. Existing corrections were preserved.".into());
    }
    write_new(
        &path,
        &bytes,
    )?;
    Ok(reference)
}
pub fn read_index(root: &Path, reference: &str) -> Result<OcrDocument, String> {
    let id = reference_name(reference)?;
    let directory = crate::library::authorized_path(&root.join("ocr-index"))?;
    let path = crate::library::authorized_path(&directory.join(format!("{id}.json")))?;
    if path.parent() != Some(directory.as_path())
        || fs::metadata(&path).map_err(|e| e.to_string())?.len() > MAX_INDEX as u64
    {
        return Err("The OCR index is outside its directory or exceeds its size limit.".into());
    }
    let index = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|_| "The OCR index is unreadable.")?;
    validate_index(&index)?;
    Ok(index)
}
pub fn corrected_text(index: &OcrDocument) -> String {
    index
        .pages
        .iter()
        .map(|page| {
            format!(
                "Page {} · {}\n{}",
                page.page,
                if page.origin == Origin::Ocr {
                    "OCR"
                } else {
                    "Original text"
                },
                page.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
pub fn correct_page(
    root: &Path,
    reference: &str,
    page_number: u32,
    text: &str,
) -> Result<OcrDocument, String> {
    let mut index = read_index(root, reference)?;
    let page = index
        .pages
        .iter_mut()
        .find(|page| page.page == page_number && page.origin == Origin::Ocr)
        .ok_or("Choose an OCR-generated page to correct its index.")?;
    if text.len() > 1_000_000
        || text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err("Corrected OCR text exceeds its limit or contains control characters.".into());
    }
    page.text = text.into();
    page.corrected = true;
    page.words.clear();
    replace_index(root, reference, &index)?;
    Ok(index)
}
pub fn remove_generated(root: &Path, reference: &str) -> Result<OcrDocument, String> {
    let mut index = read_index(root, reference)?;
    index.pages.retain(|page| page.origin != Origin::Ocr);
    replace_index(root, reference, &index)?;
    Ok(index)
}
fn replace_index(root: &Path, reference: &str, index: &OcrDocument) -> Result<(), String> {
    validate_index(index)?;
    let id = reference_name(reference)?;
    let directory = crate::library::authorized_path(&root.join("ocr-index"))?;
    let target = crate::library::authorized_path(&directory.join(format!("{id}.json")))?;
    if target.parent() != Some(directory.as_path()) {
        return Err("Invalid OCR index destination.".into());
    }
    let pending = directory.join(format!("{id}-{}.pending", crate::queue::now()));
    write_new(
        &pending,
        &serde_json::to_vec(index).map_err(|e| e.to_string())?,
    )?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = pending.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                    | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    #[cfg(not(windows))]
    fs::rename(pending, target).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn search(index: &OcrDocument, query: &str) -> Result<Vec<Hit>, String> {
    let query = query.trim();
    if !(2..=160).contains(&query.chars().count()) {
        return Err("Search OCR for 2–160 characters.".into());
    }
    let needle = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    Ok(index
        .pages
        .iter()
        .filter_map(|page| {
            let normalized = page.text.split_whitespace().collect::<Vec<_>>().join(" ");
            let lower = normalized.to_lowercase();
            let offset = lower.find(&needle)?;
            let before = lower[..offset].chars().count();
            let excerpt = normalized
                .chars()
                .skip(before.saturating_sub(60))
                .take(220)
                .collect();
            let boxes = if page.corrected {
                vec![]
            } else {
                let mut position = 0usize;
                page.words
                    .iter()
                    .filter(|word| {
                        let length = word.text.to_lowercase().len();
                        let overlaps =
                            position < offset + needle.len() && position + length > offset;
                        position += length + 1;
                        overlaps
                    })
                    .take(100)
                    .map(|word| Region {
                        x: word.x,
                        y: word.y,
                        width: word.width,
                        height: word.height,
                    })
                    .collect()
            };
            Some(Hit {
                page: page.page,
                excerpt,
                origin: page.origin,
                boxes,
            })
        })
        .take(100)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    const TSV:&str="level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n5\t1\t1\t1\t1\t1\t10\t20\t30\t15\t95.5\tHola\n5\t1\t1\t1\t1\t2\t45\t20\t50\t15\t90\tmundo\n";
    fn index() -> OcrDocument {
        OcrDocument {
            schema_version: 1,
            source_fingerprint: "a".repeat(64),
            languages: vec!["spa".into()],
            pages: vec![parse_tsv(TSV, 1, 100, 100).unwrap()],
        }
    }
    #[test]
    fn tsv_preserves_text_coordinates_and_confidence() {
        let page = &index().pages[0];
        assert_eq!(page.text, "Hola mundo");
        assert_eq!(page.words[0].x, 10);
        assert_eq!(page.words[0].confidence, 95.5);
        assert_eq!(page.words[1].width, 50);
        assert!(parse_tsv(TSV, 1, 50, 50).is_err());
    }
    #[test]
    fn search_and_correction_keep_original_text_distinct() {
        let mut index = index();
        assert_eq!(
            search(&index, "Hola").unwrap()[0].boxes[0],
            Region {
                x: 10,
                y: 20,
                width: 30,
                height: 15
            }
        );
        index.pages[0].origin = Origin::Original;
        assert_eq!(search(&index, "mundo").unwrap()[0].origin, Origin::Original);
        assert!(search(&index, "x").is_err());
    }

    #[test]
    fn phrase_search_highlights_each_recognized_word() {
        let data = index();
        let hits = search(&data, "hola mundo").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].boxes.len(), 2);
    }

    #[tokio::test]
    #[ignore = "Requires NABNAB_TEST_TESSERACT, NABNAB_TEST_TESSDATA, NABNAB_TEST_MUTOOL and NABNAB_TEST_FFMPEG; all local fixtures"]
    async fn real_ocr_preserves_pdf_appearance_original_text_and_source() {
        let tesseract =
            PathBuf::from(std::env::var_os("NABNAB_TEST_TESSERACT").expect("Set Tesseract path"));
        let models =
            PathBuf::from(std::env::var_os("NABNAB_TEST_TESSDATA").expect("Set OCR models path"));
        let mutool = PathBuf::from(std::env::var_os("NABNAB_TEST_MUTOOL").expect("Set MuPDF path"));
        let ffmpeg =
            PathBuf::from(std::env::var_os("NABNAB_TEST_FFMPEG").expect("Set FFmpeg path"));
        let root = std::env::temp_dir().join(format!(
            "nabnab-ocr-real-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        fs::create_dir(&root).unwrap();
        let (_sender, cancel) = watch::channel(false);
        let fixture = root.join("fixture.js");
        let original = root.join("typed.pdf");
        let image = root.join("scan.png");
        let input = root.join("mixed.pdf");
        write_new(&fixture,br#"var w=new DocumentWriter(scriptArgs[0],'pdf','compress');var dev=w.beginPage([0,0,612,792]);var t=new Text();t.showString(new Font('Times-Roman'),[26,0,0,-26,72,120],'Hola mundo documento local.',0);dev.fillText(t,Matrix.identity,ColorSpace.DeviceGray,[0],1);w.endPage();w.close();"#).unwrap();
        engine::run_process(
            mutool.clone(),
            vec![
                "run".into(),
                crate::documents::cli_path(&fixture),
                crate::documents::cli_path(&original),
            ],
            cancel.clone(),
            0,
            true,
            |_| {},
        )
        .await
        .unwrap();
        engine::run_process(
            mutool.clone(),
            vec![
                "draw".into(),
                "-q".into(),
                "-r".into(),
                "150".into(),
                "-o".into(),
                crate::documents::cli_path(&image),
                crate::documents::cli_path(&original),
                "1".into(),
            ],
            cancel.clone(),
            0,
            true,
            |_| {},
        )
        .await
        .unwrap();
        let scan_driver = root.join("scan.js");
        write_new(&scan_driver,br#"var d=new PDFDocument();d.graftPage(-1,Document.openDocument(scriptArgs[0]),0);var image=d.addImage(new Image(scriptArgs[1]));var page=d.addPage([0,0,612,792],0,{XObject:{Scan:image}},'q 612 0 0 792 0 0 cm /Scan Do Q');d.insertPage(-1,page);d.save(scriptArgs[2],'compress');"#).unwrap();
        engine::run_process(
            mutool.clone(),
            vec![
                "run".into(),
                crate::documents::cli_path(&scan_driver),
                crate::documents::cli_path(&original),
                crate::documents::cli_path(&image),
                crate::documents::cli_path(&input),
            ],
            cancel.clone(),
            0,
            true,
            |_| {},
        )
        .await
        .unwrap();
        let before = fs::read(&input).unwrap();
        let staging = root.join("job");
        fs::create_dir(&staging).unwrap();
        let options = OcrOptions {
            languages: vec!["eng".into(), "spa".into()],
            pages: "1-2".into(),
            dpi: 150,
            region: None,
            output: Output::SearchablePdf,
            acknowledge_structure_changes: true,
        };
        let result = execute(
            &tesseract,
            &models,
            Some(&mutool),
            &ffmpeg,
            &input,
            &staging,
            &options,
            cancel.clone(),
            1,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(result.index.pages.len(), 2);
        assert_eq!(result.index.pages[0].origin, Origin::Original);
        assert_eq!(result.index.pages[1].origin, Origin::Ocr);
        assert!(
            result.index.pages[1]
                .text
                .to_lowercase()
                .contains("hola mundo"),
            "{}",
            result.index.pages[1].text
        );
        assert!(!result.index.pages[1].words.is_empty());
        assert_eq!(search(&result.index, "Hola mundo").unwrap().len(), 2);
        let text = engine::run_process(
            mutool.clone(),
            vec![
                "draw".into(),
                "-q".into(),
                "-F".into(),
                "txt".into(),
                crate::documents::cli_path(&result.file),
            ],
            cancel.clone(),
            0,
            true,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(text.matches("Hola mundo").count(), 2, "{text}");
        assert_eq!(fs::read(&input).unwrap(), before);
        let render_before = root.join("before.png");
        let render_after = root.join("after.png");
        for (pdf, png) in [(&input, &render_before), (&result.file, &render_after)] {
            engine::run_process(
                mutool.clone(),
                vec![
                    "draw".into(),
                    "-q".into(),
                    "-r".into(),
                    "100".into(),
                    "-o".into(),
                    crate::documents::cli_path(png),
                    crate::documents::cli_path(pdf),
                    "2".into(),
                ],
                cancel.clone(),
                0,
                true,
                |_| {},
            )
            .await
            .unwrap();
        }
        assert_eq!(
            fs::read(render_before).unwrap(),
            fs::read(render_after).unwrap(),
            "OCR must not change the page's visible rendering"
        );
        let image_stage = root.join("image-job");
        fs::create_dir(&image_stage).unwrap();
        let options = OcrOptions {
            output: Output::Text,
            pages: "1".into(),
            region: Some(Region {
                x: 100,
                y: 100,
                width: 1100,
                height: 400,
            }),
            ..options
        };
        let recognized = execute(
            &tesseract,
            &models,
            None,
            &ffmpeg,
            &image,
            &image_stage,
            &options,
            cancel.clone(),
            2,
            |_| {},
        )
        .await
        .unwrap();
        assert!(recognized.index.pages[0]
            .text
            .to_lowercase()
            .contains("hola mundo"));
        assert_eq!(recognized.index.pages[0].region, options.region);
        let stopped = root.join("cancelled");
        fs::create_dir(&stopped).unwrap();
        let (_, cancelled) = watch::channel(true);
        assert_eq!(
            execute(
                &tesseract,
                &models,
                None,
                &ffmpeg,
                &image,
                &stopped,
                &options,
                cancelled,
                3,
                |_| {}
            )
            .await
            .unwrap_err(),
            engine::CANCELLED
        );
        assert_eq!(fs::read(&input).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn index_paths_and_limits_reject_traversal() {
        for reference in ["ocr:../escape", "ocr:C:secret", "ocr:/x", "other:id"] {
            assert!(reference_name(reference).is_err());
        }
        let mut index = index();
        index.pages[0].words[0].x = 1000;
        assert!(validate_index(&index).is_err());
    }
    #[test]
    fn correcting_or_removing_ocr_does_not_modify_original_pages() {
        let root = std::env::temp_dir().join(format!(
            "nabnab-ocr-index-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        fs::create_dir(&root).unwrap();
        let mut original = index().pages[0].clone();
        original.page = 2;
        original.origin = Origin::Original;
        let mut data = index();
        data.pages.push(original);
        let reference = write_index(&root, "job1", &data).unwrap();
        assert_eq!(write_index(&root,"job1",&data).unwrap(),reference);
        let corrected = correct_page(&root, &reference, 1, "Texto corregido").unwrap();
        assert_eq!(corrected.pages[0].text, "Texto corregido");
        assert!(corrected.pages[0].words.is_empty());
        assert!(corrected.pages[0].corrected);
        assert!(write_index(&root,"job1",&data).is_err());
        let retried=write_index(&root,"job1-run2",&data).unwrap();
        assert_ne!(retried,reference);
        assert_eq!(read_index(&root,&reference).unwrap().pages[0].text,"Texto corregido");
        assert_eq!(read_index(&root,&retried).unwrap().pages[0].text,"Hola mundo");
        assert!(correct_page(&root, &reference, 2, "No").is_err());
        let removed = remove_generated(&root, &reference).unwrap();
        assert_eq!(removed.pages.len(), 1);
        assert_eq!(removed.pages[0].origin, Origin::Original);
        assert_eq!(removed.pages[0].text, "Hola mundo");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn useful_text_skips_existing_layers_but_not_page_numbers() {
        assert!(!useful_text("123"));
        assert!(useful_text("Original useful text on this page."));
    }
}
