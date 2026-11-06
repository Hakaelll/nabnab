use crate::{engine, model::Progress};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use tokio::sync::watch;

pub const OVERLAY_HELPER: &str = include_str!("document_edits/overlay.js");
const DRIVER: &str = include_str!("document_edits/driver.js");
const MAX_INPUT: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    High,
    Balanced,
    Small,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Position {
    Top,
    Bottom,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Bookmark {
    pub title: String,
    pub page: u32,
    pub level: u8,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Options {
    Compress {
        quality: Quality,
    },
    Crop {
        pages: String,
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    },
    Number {
        pages: String,
        start: u32,
        prefix: String,
        font_size: u16,
        position: Position,
    },
    Watermark {
        pages: String,
        text: String,
        font_size: u16,
        opacity: u8,
    },
    Bookmarks {
        entries: Vec<Bookmark>,
    },
}
impl Options {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Compress { .. } => "compress",
            Self::Crop { .. } => "crop",
            Self::Number { .. } => "number",
            Self::Watermark { .. } => "watermark",
            Self::Bookmarks { .. } => "bookmarks",
        }
    }
    fn pages(&self) -> Option<&str> {
        match self {
            Self::Crop { pages, .. }
            | Self::Number { pages, .. }
            | Self::Watermark { pages, .. } => Some(pages),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub pages: u32,
    pub width: f64,
    pub height: f64,
    pub forms: bool,
    pub signatures: bool,
    pub edit: bool,
    pub copy: bool,
    pub print: bool,
    pub bookmarks: Vec<Bookmark>,
    pub bookmarks_truncated: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub page: u32,
    pub pages: u32,
    pub png: Vec<u8>,
    pub before_png: Vec<u8>,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub warnings: Vec<String>,
}

pub fn validate_options(options: &Options) -> Result<(), String> {
    if let Some(pages) = options.pages() {
        if pages.is_empty()
            || pages.len() > 4096
            || !pages
                .bytes()
                .all(|b| b.is_ascii_digit() || b"N,- ".contains(&b))
        {
            return Err("Use a bounded page selection such as 1-3,5,N.".into());
        }
    }
    let text_ok =
        |text: &str, max: usize| text.chars().count() <= max && !text.chars().any(char::is_control);
    match options {
        Options::Compress { .. } => {}
        Options::Crop {
            left,
            top,
            right,
            bottom,
            ..
        } => {
            if ![left, top, right, bottom]
                .iter()
                .all(|v| v.is_finite() && **v >= 0.0 && **v <= 10000.0)
                || left + top + right + bottom <= 0.0
            {
                return Err("Choose nonnegative crop margins in points, up to 10000 each, with at least one positive margin.".into());
            }
        }
        Options::Number {
            start,
            prefix,
            font_size,
            ..
        } => {
            if *start > 1_000_000 || !(6..=72).contains(font_size) || !text_ok(prefix, 80) {
                return Err("Choose a start from 0-1000000, a font size from 6-72 points and a prefix of up to 80 characters.".into());
            }
        }
        Options::Watermark {
            text,
            font_size,
            opacity,
            ..
        } => {
            if text.trim().is_empty()
                || !text_ok(text, 200)
                || !(6..=144).contains(font_size)
                || !(5..=90).contains(opacity)
            {
                return Err("Enter a watermark of 1-200 characters, font size 6-144 points and opacity 5-90 percent.".into());
            }
        }
        Options::Bookmarks { entries } => {
            if entries.len() > 500 {
                return Err("Edit at most 500 internal PDF bookmarks.".into());
            }
            let mut last = 0;
            for entry in entries {
                if entry.title.trim().is_empty()
                    || !text_ok(&entry.title, 200)
                    || entry.page == 0
                    || entry.page > 10000
                    || entry.level == 0
                    || entry.level > 4
                    || entry.level > last + 1
                {
                    return Err("Bookmark titles need 1-200 characters, pages 1-10000 and consecutive levels 1-4 starting at level 1.".into());
                }
                last = entry.level;
            }
        }
    }
    Ok(())
}
fn input_path(input: &Path) -> Result<PathBuf, String> {
    let input = crate::library::authorized_path(input)?;
    if !input.is_file()
        || !input
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("pdf"))
        || fs::metadata(&input).map_err(|e| e.to_string())?.len() > MAX_INPUT
    {
        return Err("Choose a local PDF of at most 1 GiB from the library.".into());
    }
    Ok(input)
}
fn cli_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!("//{}", rest.replace('\\', "/"))
    } else {
        path.strip_prefix(r"\\?\")
            .unwrap_or(&path)
            .replace('\\', "/")
    }
}
fn owned_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            let resolved = crate::library::authorized_path(path)?;
            if !resolved.is_file()
                || fs::metadata(&resolved).map_err(|e| e.to_string())?.len() != bytes.len() as u64
                || fs::read(resolved).map_err(|e| e.to_string())? != bytes
            {
                return Err(
                    "The owned PDF edit driver or options changed. Start a new job.".into(),
                );
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)
                .map_err(|e| e.to_string())?;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}
fn driver(staging: &Path) -> Result<PathBuf, String> {
    let path = staging.join("nabnab-document-edits.js");
    owned_file(&path, format!("{OVERLAY_HELPER}\n{DRIVER}").as_bytes())?;
    Ok(path)
}
fn ascii_json(value: &serde_json::Value) -> String {
    let mut out = String::new();
    for code in value.to_string().encode_utf16() {
        if code < 128 {
            out.push(char::from_u32(code as u32).unwrap());
        } else {
            out.push_str(&format!("\\u{code:04x}"));
        }
    }
    out
}

pub async fn inspect(
    mutool: &Path,
    input: &Path,
    staging: &Path,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<Info, String> {
    let input = input_path(input)?;
    let script = driver(staging)?;
    let raw = engine::run_process(
        mutool.to_path_buf(),
        vec![
            "run".into(),
            cli_path(&script),
            "inspect".into(),
            cli_path(&input),
        ],
        cancel,
        run_id,
        true,
        |_| {},
    )
    .await?;
    let info: Info = serde_json::from_str(raw.trim())
        .map_err(|_| "The PDF component returned invalid edit information.")?;
    if info.pages == 0
        || info.pages > 10000
        || !info.width.is_finite()
        || !info.height.is_finite()
        || info.width <= 0.0
        || info.height <= 0.0
        || info.width > 100000.0
        || info.height > 100000.0
        || info.bookmarks.len() > 500
    {
        return Err("This PDF exceeds the supported page or geometry limits.".into());
    }
    Ok(info)
}
pub fn warnings(info: &Info, options: &Options) -> Vec<String> {
    let mut result=vec!["A new PDF will be written. Rewriting can change form behavior, annotations, metadata and document structure. Digital signature validity is not preserved or certified.".into()];
    if info.forms {
        result.push("The input contains an AcroForm. Review every field in the new copy before relying on it.".into());
    }
    if info.signatures {
        result.push("Signature fields or signature permissions were detected. Keep the original for signature verification.".into());
    }
    match options{Options::Crop{..}=>result.push("Crop changes the visible page boundary. It does not securely remove hidden text or images and is not redaction.".into()),Options::Compress{quality:Quality::Balanced|Quality::Small}=>result.push("This quality can downsample and recompress color and gray images with loss. Text and vector graphics are not rasterized. The final size is not guaranteed to be smaller.".into()),Options::Compress{..}=>result.push("Lossless compression preserves image pixels, but the final size is not guaranteed to be smaller.".into()),Options::Bookmarks{..}=>{result.push("This replaces the document's embedded table of contents with the reviewed internal page links. It does not replace library bookmarks.".into());if info.bookmarks_truncated{result.push("Existing bookmarks were only partly loaded or included destinations that cannot be represented. Review the replacement list before continuing.".into());}},_=>{}}
    result
}
fn compression_args(input: &Path, output: &Path, quality: Quality) -> Vec<String> {
    let mut args = vec![
        "clean".into(),
        "-gggg".into(),
        "-z".into(),
        "-f".into(),
        "-i".into(),
        "-e".into(),
        "90".into(),
        "-m".into(),
        "-Z".into(),
    ];
    if quality != Quality::High {
        let (dpi, target, jpeg) = if quality == Quality::Balanced {
            (225, 150, 82)
        } else {
            (144, 96, 65)
        };
        for color in ["color", "gray"] {
            args.extend([
                format!("--{color}-image-subsample-method"),
                "bicubic".into(),
                format!("--{color}-image-subsample-dpi"),
                format!("{dpi},{target}"),
                format!("--{color}-image-recompress-method"),
                format!("jpeg:{jpeg}"),
            ]);
        }
        args.extend(["--recompress-images-when".into(), "smaller".into()]);
    }
    args.extend([cli_path(input), cli_path(output)]);
    args
}
pub async fn execute(
    mutool: &Path,
    input: &Path,
    staging: &Path,
    options: &Options,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    validate_options(options)?;
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let mutool = crate::library::authorized_path(mutool)?;
    let input = input_path(input)?;
    let staging = crate::library::authorized_path(staging)?;
    let info = inspect(&mutool, &input, &staging, cancel.clone(), run_id).await?;
    if !info.edit {
        return Err("This PDF does not permit editing. Keep the original and use an authorized editable copy.".into());
    }
    let selected = if let Some(pages) = options.pages() {
        let mut seen = std::collections::HashSet::new();
        crate::documents::page_selection(pages, info.pages)?
            .into_iter()
            .filter(|p| seen.insert(*p))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if let Options::Bookmarks { entries } = options {
        if entries.iter().any(|b| b.page > info.pages) {
            return Err("A bookmark points outside the document.".into());
        }
    }
    let output = staging.join(format!("document-edit-{}.pdf", options.name()));
    let partial = staging.join(".document-edit-output.part.pdf");
    if fs::symlink_metadata(&output).is_ok() || fs::symlink_metadata(&partial).is_ok() {
        return Err(
            "The PDF output or owned partial already exists. Existing files were preserved.".into(),
        );
    }
    let pending = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&partial)
        .map_err(|e| e.to_string())?;
    drop(pending);
    let args = if let Options::Compress { quality } = options {
        compression_args(&input, &partial, *quality)
    } else {
        let request = serde_json::json!({"options":options,"selectedPages":selected});
        let settings = staging.join("nabnab-document-edit-options.json");
        owned_file(&settings, ascii_json(&request).as_bytes())?;
        vec![
            "run".into(),
            cli_path(&driver(&staging)?),
            "edit".into(),
            cli_path(&input),
            cli_path(&settings),
            cli_path(&partial),
            cli_path(&staging.join("nabnab-document-edit-overlay.pdf")),
        ]
    };
    emit(Progress {
        request_id: run_id,
        phase: "processing".into(),
        percent: Some(0.0),
        speed: None,
        downloaded: None,
        total: None,
    });
    let result = engine::run_process(
        mutool.clone(),
        args,
        cancel.clone(),
        run_id,
        false,
        emit.clone(),
    )
    .await;
    if let Err(error) = result {
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    if *cancel.borrow() {
        let _ = fs::remove_file(&partial);
        return Err(engine::CANCELLED.into());
    }
    let checked = crate::checked_output(&staging, &partial.to_string_lossy())?;
    let metadata = fs::metadata(&checked).map_err(|e| e.to_string())?;
    if metadata.len() == 0 || metadata.len() > 4 * MAX_INPUT {
        return Err("The PDF edit returned an empty or oversized result.".into());
    }
    let verify_dir = staging.join("document-edit-verification");
    fs::create_dir(&verify_dir).map_err(|e| e.to_string())?;
    let verified = inspect(&mutool, &checked, &verify_dir, cancel.clone(), run_id).await?;
    if verified.pages != info.pages {
        return Err("The edited PDF changed its page count unexpectedly.".into());
    }
    crate::direct::move_completed(&checked, &output)?;
    emit(Progress {
        request_id: run_id,
        phase: "verifying".into(),
        percent: Some(100.0),
        speed: None,
        downloaded: None,
        total: None,
    });
    Ok(output)
}
async fn render_page(
    mutool: &Path,
    input: &Path,
    folder: &Path,
    name: &str,
    page: u32,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<Vec<u8>, String> {
    let output = folder.join(name);
    let args = vec![
        "draw".into(),
        "-q".into(),
        "-F".into(),
        "png".into(),
        "-r".into(),
        "100".into(),
        "-w".into(),
        "1200".into(),
        "-h".into(),
        "1600".into(),
        "-T".into(),
        "2".into(),
        "-B".into(),
        "256".into(),
        "-o".into(),
        cli_path(&output),
        cli_path(input),
        page.to_string(),
    ];
    engine::run_process(mutool.to_path_buf(), args, cancel, run_id, true, |_| {}).await?;
    let output = crate::checked_output(folder, &output.to_string_lossy())?;
    if fs::metadata(&output).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
        return Err("The PDF preview exceeds 8 MiB.".into());
    }
    let bytes = fs::read(output).map_err(|e| e.to_string())?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The PDF component returned an invalid preview image.".into());
    }
    Ok(bytes)
}
pub async fn preview(
    mutool: &Path,
    input: &Path,
    staging: &Path,
    options: &Options,
    page: u32,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<Preview, String> {
    validate_options(options)?;
    let input = input_path(input)?;
    let info = inspect(mutool, &input, staging, cancel.clone(), run_id).await?;
    if page == 0 || page > info.pages {
        return Err("Choose a preview page inside the PDF.".into());
    }
    let before_png = render_page(
        mutool,
        &input,
        staging,
        "document-edit-before.png",
        page,
        cancel.clone(),
        run_id,
    )
    .await?;
    let output = execute(
        mutool,
        &input,
        staging,
        options,
        cancel.clone(),
        run_id,
        |_| {},
    )
    .await?;
    let png = render_page(
        mutool,
        &output,
        staging,
        "document-edit-after.png",
        page,
        cancel,
        run_id,
    )
    .await?;
    Ok(Preview {
        page,
        pages: info.pages,
        png,
        before_png,
        input_bytes: fs::metadata(input).map_err(|e| e.to_string())?.len(),
        output_bytes: fs::metadata(output).map_err(|e| e.to_string())?.len(),
        warnings: warnings(&info, options),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn options_reject_invalid_geometry_injection_and_broken_bookmark_hierarchy() {
        for value in [f64::NAN, -1.0, 10001.0] {
            assert!(validate_options(&Options::Crop {
                pages: "1-N".into(),
                left: value,
                top: 1.0,
                right: 0.0,
                bottom: 0.0
            })
            .is_err());
        }
        assert!(serde_json::from_value::<Options>(
            serde_json::json!({"kind":"compress","quality":"high","script":"malicious"})
        )
        .is_err());
        for entries in [
            vec![Bookmark {
                title: "Chapter".into(),
                page: 1,
                level: 2,
            }],
            vec![
                Bookmark {
                    title: "A".into(),
                    page: 1,
                    level: 1,
                },
                Bookmark {
                    title: "B".into(),
                    page: 2,
                    level: 3,
                },
            ],
        ] {
            assert!(validate_options(&Options::Bookmarks { entries }).is_err());
        }
        assert!(validate_options(&Options::Watermark {
            pages: "1-N".into(),
            text: "Own copy".into(),
            font_size: 20,
            opacity: 20
        })
        .is_ok());
    }
    #[test]
    fn unicode_options_remain_data_and_quality_arguments_are_fixed() {
        let text = ascii_json(&serde_json::json!({"text":"Página (own) \\ 😀"}));
        assert!(text.is_ascii());
        let decoded: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(decoded["text"], "Página (own) \\ 😀");
        let args = compression_args(
            Path::new("Input %d.pdf"),
            Path::new("Output.pdf"),
            Quality::Balanced,
        );
        assert!(args.contains(&"jpeg:82".into()));
        assert!(args.contains(&"225,150".into()));
        assert_eq!(args[args.len() - 2], "Input %d.pdf");
    }
    #[test]
    fn crop_and_signature_warnings_are_explicit() {
        let info = Info {
            pages: 1,
            width: 300.0,
            height: 200.0,
            forms: true,
            signatures: true,
            edit: true,
            copy: true,
            print: true,
            bookmarks: vec![],
            bookmarks_truncated: false,
        };
        let warnings = warnings(
            &info,
            &Options::Crop {
                pages: "1".into(),
                left: 10.0,
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
            },
        );
        assert!(warnings.iter().any(|w| w.contains("not redaction")));
        assert!(warnings.iter().any(|w| w.contains("Signature fields")));
        assert!(warnings.iter().any(|w| w.contains("AcroForm")));
    }
    #[tokio::test]
    #[ignore = "Requires the deliberately installed MuPDF1.28.5 component in NABNAB_TEST_MUTOOL; local synthetic documents only."]
    async fn real_document_edits_preserve_original_and_apply_visible_changes() {
        let mutool = PathBuf::from(
            std::env::var_os("NABNAB_TEST_MUTOOL")
                .expect("Set NABNAB_TEST_MUTOOL to the installed MuPDF component."),
        );
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let work =
            std::env::temp_dir().join(format!("nabnab-pdf-edits-{}-{nonce}", std::process::id()));
        fs::create_dir(&work).unwrap();
        let generator = work.join("generate.js");
        fs::write(&generator,r#"var d=new PDFDocument();var f=d.addSimpleFont(new Font('Helvetica'));for(var i=1;i<=3;i++)d.insertPage(-1,d.addPage([0,0,300,200],i===3?90:0,{Font:{F1:f}},'BT /F1 12 Tf 20 100 Td (Original page '+i+') Tj ET'));var sig=d.addObject({FT:'Sig',T:d.newString('Signature field')});d.getTrailer().get('Root').put('AcroForm',{Fields:[sig]});d.save(scriptArgs[0],'');"#).unwrap();
        let input = work.join("Original %d.pdf");
        let (_sender, cancel) = watch::channel(false);
        engine::run_process(
            mutool.clone(),
            vec!["run".into(), cli_path(&generator), cli_path(&input)],
            cancel.clone(),
            0,
            true,
            |_| {},
        )
        .await
        .unwrap();
        let original = fs::read(&input).unwrap();
        let choices = vec![
            Options::Compress {
                quality: Quality::High,
            },
            Options::Compress {
                quality: Quality::Balanced,
            },
            Options::Compress {
                quality: Quality::Small,
            },
            Options::Crop {
                pages: "1-N".into(),
                left: 10.0,
                top: 20.0,
                right: 30.0,
                bottom: 40.0,
            },
            Options::Number {
                pages: "3,2,1".into(),
                start: 7,
                prefix: "Página ".into(),
                font_size: 12,
                position: Position::Bottom,
            },
            Options::Watermark {
                pages: "1".into(),
                text: "Own copy".into(),
                font_size: 22,
                opacity: 25,
            },
            Options::Bookmarks {
                entries: vec![
                    Bookmark {
                        title: "Chapter one".into(),
                        page: 1,
                        level: 1,
                    },
                    Bookmark {
                        title: "Página dos".into(),
                        page: 2,
                        level: 2,
                    },
                    Bookmark {
                        title: "Chapter three".into(),
                        page: 3,
                        level: 1,
                    },
                ],
            },
        ];
        for (index, options) in choices.iter().enumerate() {
            let stage = work.join(format!("stage-{index}"));
            fs::create_dir(&stage).unwrap();
            let output = execute(
                &mutool,
                &input,
                &stage,
                options,
                cancel.clone(),
                index as u64,
                |_| {},
            )
            .await
            .unwrap();
            assert_ne!(output, input);
            assert_eq!(fs::read(&input).unwrap(), original);
            assert!(fs::read(&output).unwrap().starts_with(b"%PDF-"));
            let check = stage.join("check");
            fs::create_dir(&check).unwrap();
            let info = inspect(&mutool, &output, &check, cancel.clone(), 0)
                .await
                .unwrap();
            assert_eq!(info.pages, 3);
            assert!(info.forms);
            assert!(info.signatures);
            match options {
                Options::Crop { .. } => {
                    assert_eq!(info.width, 260.0);
                    assert_eq!(info.height, 140.0);
                }
                Options::Number { .. } | Options::Watermark { .. } => {
                    let text = stage.join("text.txt");
                    engine::run_process(
                        mutool.clone(),
                        vec![
                            "draw".into(),
                            "-q".into(),
                            "-F".into(),
                            "txt".into(),
                            "-o".into(),
                            cli_path(&text),
                            cli_path(&output),
                            "1-N".into(),
                        ],
                        cancel.clone(),
                        0,
                        true,
                        |_| {},
                    )
                    .await
                    .unwrap();
                    let text = fs::read_to_string(text).unwrap();
                    assert!(text.contains("Original page 1"));
                    assert!(text.contains("Original page 2"));
                    if matches!(options, Options::Number { .. }) {
                        assert!(text.contains("Página 7"));
                        assert!(text.contains("Página 8"));
                        assert!(text.contains("Página 9"));
                        assert!(!text.contains('\u{fffd}'));
                    } else {
                        assert_eq!(text.matches("Own copy").count(), 1);
                    }
                }
                Options::Bookmarks { entries } => assert_eq!(&info.bookmarks, entries),
                _ => {}
            }
        }
        let stage = work.join("preview");
        fs::create_dir(&stage).unwrap();
        let result = preview(&mutool, &input, &stage, &choices[4], 3, cancel.clone(), 0)
            .await
            .unwrap();
        assert!(result.png.starts_with(b"\x89PNG"));
        assert_ne!(result.png, result.before_png);
        assert_eq!(result.input_bytes, original.len() as u64);
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("Signature fields")));
        assert_eq!(fs::read(&input).unwrap(), original);
        let stage = work.join("cancelled");
        fs::create_dir(&stage).unwrap();
        let (_sender, cancelled) = watch::channel(true);
        assert_eq!(
            execute(&mutool, &input, &stage, &choices[4], cancelled, 0, |_| {})
                .await
                .unwrap_err(),
            engine::CANCELLED
        );
        assert_eq!(fs::read_dir(stage).unwrap().count(), 0);
        fs::remove_dir_all(work).unwrap();
    }
}
