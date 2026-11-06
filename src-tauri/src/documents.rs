use crate::{engine, model::Progress, AppState};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{AppHandle, State};
use tokio::sync::watch;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DocumentOptions {
    pub operation: String,
    pub pages: String,
    pub rotation: u16,
    pub dpi: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInfo {
    pub pages: u32,
    pub copy: bool,
    pub assemble: bool,
    pub print: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPage {
    pub page: u32,
    pub pages: u32,
    pub png: Vec<u8>,
    pub text: String,
    pub text_available: bool,
}

// Only this application-owned driver runs. Document scripts are never loaded into a WebView.
const INSPECT: &str = "var d=Document.openDocument(scriptArgs[0]);if(d.needsPassword())throw new Error('This document requires a password. Open an authorized decrypted copy.');if(d.isReflowable())d.layout(600,800,12);print(JSON.stringify({pages:d.countPages(),copy:d.hasPermission('copy'),assemble:d.hasPermission('assemble'),print:d.hasPermission('print')}));";

fn document_path(path: &Path, pdf_only: bool) -> Result<PathBuf, String> {
    let path = crate::library::authorized_path(path)?;
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !path.is_file() || !(extension == "pdf" || !pdf_only && extension == "epub") {
        return Err(
            "Choose a local PDF or EPUB from the library. PDF operations require PDF inputs."
                .into(),
        );
    }
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 1024 * 1024 * 1024 {
        return Err("This document exceeds the supported 1 GiB input limit.".into());
    }
    Ok(path)
}

pub fn page_selection(value: &str, count: u32) -> Result<Vec<u32>, String> {
    if count == 0 || count > 10_000 || value.is_empty() || value.len() > 4096 {
        return Err("Choose pages from a document of up to 10,000 pages.".into());
    }
    let mut result = Vec::new();
    for part in value.split(',') {
        let number = |value: &str| -> Result<u32, String> {
            let value = value.trim();
            if value == "N" {
                return Ok(count);
            }
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(
                    "Use page numbers, ranges and N for the final page, for example 1-3,5,N."
                        .into(),
                );
            }
            value
                .parse::<u32>()
                .ok()
                .filter(|page| *page > 0 && *page <= count)
                .ok_or_else(|| "A selected page is outside the document.".into())
        };
        if let Some((from, to)) = part.split_once('-') {
            let from = number(from)?;
            let to = number(to)?;
            if from <= to {
                result.extend(from..=to);
            } else {
                result.extend((to..=from).rev());
            }
        } else {
            result.push(number(part)?);
        }
        if result.len() > 10_000 {
            return Err("Select at most 10,000 pages.".into());
        }
    }
    Ok(result)
}

fn cli_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        format!("//{}", rest.replace('\\', "/"))
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).replace('\\', "/")
    }
}
fn inspect_arguments(script: &Path, input: &Path) -> Vec<String> {
    vec![
        "run".into(),
        cli_path(script),
        cli_path(&input),
    ]
}

async fn inspect(
    mutool: &Path,
    input: &Path,
    staging: &Path,
    cancel: watch::Receiver<bool>,
    run_id: u64,
) -> Result<DocumentInfo, String> {
    let driver = staging.join("nabnab-document-inspect.js");
    if !driver.exists() {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&driver)
            .map_err(|e| e.to_string())?;
        file.write_all(INSPECT.as_bytes())
            .map_err(|e| e.to_string())?;
    } else if fs::read(&driver).map_err(|e| e.to_string())? != INSPECT.as_bytes() {
        return Err("The document inspection driver changed. Start a new job.".into());
    }
    let raw = engine::run_process(
        mutool.to_path_buf(),
        inspect_arguments(&driver, input),
        cancel,
        run_id,
        true,
        |_| {},
    )
    .await?;
    let info: DocumentInfo = serde_json::from_str(raw.trim())
        .map_err(|_| "The document component returned invalid page information.")?;
    if info.pages == 0 || info.pages > 10_000 {
        return Err("Open a document with 1–10,000 pages.".into());
    }
    Ok(info)
}

pub fn validate_options(options: &DocumentOptions) -> Result<(), String> {
    if !matches!(
        options.operation.as_str(),
        "merge" | "pages" | "rotate" | "text" | "image"
    ) || !matches!(options.rotation, 0 | 90 | 180 | 270)
        || !(72..=300).contains(&options.dpi)
        || options.pages.is_empty()
        || options.pages.len() > 4096
        || !options
            .pages
            .bytes()
            .all(|byte| byte.is_ascii_digit() || b"N,- ".contains(&byte))
    {
        return Err("Choose a supported PDF operation, page range, rotation and DPI.".into());
    }
    Ok(())
}

fn output_arguments(
    inputs: &[PathBuf],
    output: &Path,
    options: &DocumentOptions,
) -> Result<Vec<String>, String> {
    validate_options(options)?;
    if inputs.is_empty() || inputs.len() > 100 {
        return Err("Choose 1–100 PDF inputs.".into());
    }
    if options.operation != "merge" && inputs.len() != 1 {
        return Err(
            "This operation accepts one PDF at a time. Use Merge for multiple PDFs.".into(),
        );
    }
    let mut args = Vec::new();
    if matches!(options.operation.as_str(), "merge" | "pages") {
        args.extend([
            "merge".into(),
            "-o".into(),
            cli_path(output),
        ]);
        for input in inputs {
            args.push(cli_path(&input));
            args.push(options.pages.clone());
        }
    } else {
        args.extend([
            "draw".into(),
            "-q".into(),
            "-F".into(),
            match options.operation.as_str() {
                "text" => "txt",
                "image" => "png",
                _ => "pdf",
            }
            .into(),
            "-o".into(),
            cli_path(output),
        ]);
        if options.operation == "rotate" {
            args.extend(["-R".into(), options.rotation.to_string()]);
        }
        if options.operation == "image" {
            args.extend([
                "-r".into(),
                options.dpi.to_string(),
                "-w".into(),
                "8192".into(),
                "-h".into(),
                "8192".into(),
                "-T".into(), "2".into(), "-B".into(),
                "256".into(),
            ]);
        }
        args.push(cli_path(&inputs[0]));
        args.push(options.pages.clone());
    }
    Ok(args)
}

pub async fn execute(
    mutool: &Path,
    inputs: &[PathBuf],
    staging: &Path,
    options: &DocumentOptions,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    validate_options(options)?;
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let mutool = crate::library::authorized_path(mutool)?;
    let staging = crate::library::authorized_path(staging)?;
    let inputs: Vec<_> = inputs
        .iter()
        .map(|input| document_path(input, true))
        .collect::<Result<_, _>>()?;
    if inputs.is_empty() || inputs.len() > 100 {
        return Err("Choose 1–100 PDF inputs.".into());
    }
    for input in &inputs {
        let info = inspect(&mutool, input, &staging, cancel.clone(), run_id).await?;
        let pages = page_selection(&options.pages, info.pages)?;
        if options.operation == "image" && pages.len() != 1 {
            return Err(
                "Export one page per image job; prepare multiple jobs to export multiple pages."
                    .into(),
            );
        }
        if options.operation == "text" && !info.copy {
            return Err("This document does not permit text extraction.".into());
        }
        if options.operation == "image" && !info.print {
            return Err("This document does not permit page export.".into());
        }
        if matches!(options.operation.as_str(), "merge" | "pages" | "rotate") && !info.assemble {
            return Err("This document does not permit page assembly or rotation.".into());
        }
    }
    let extension = match options.operation.as_str() {
        "text" => "txt",
        "image" => "png",
        _ => "pdf",
    };
    let stem =
        crate::options::safe_segment(&inputs[0].file_stem().unwrap_or_default().to_string_lossy())
            .replace('%', "_");
    let output = staging.join(format!("{stem}.{}-{run_id}.{extension}", options.operation));
    if output.exists() {
        return Err("The document output already exists. Start a new job to preserve it.".into());
    }
    let args = output_arguments(&inputs, &output, options)?;
    emit(Progress {
        request_id: run_id,
        phase: "processing".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
    engine::run_process(
        mutool.clone(),
        args,
        cancel.clone(),
        run_id,
        false,
        emit.clone(),
    )
    .await?;
    let output = crate::checked_output(&staging, &output.to_string_lossy())?;
    let size = fs::metadata(&output).map_err(|e| e.to_string())?.len();
    if size == 0 {
        return Err("The document component produced no output.".into());
    }
    emit(Progress {
        request_id: run_id,
        phase: "verifying".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
    if extension == "txt" {
        if size > 32 * 1024 * 1024 {
            return Err("The extracted text exceeds the supported 32 MiB limit.".into());
        }
        if fs::read_to_string(&output)
            .map_err(|_| "The extracted text is not valid UTF-8.")?
            .trim()
            .is_empty()
        {
            return Err(
                "No text was found. This may be a scanned PDF. Text extraction is not OCR.".into(),
            );
        }
    } else if extension == "pdf" {
        inspect(&mutool, &output, &staging, cancel.clone(), run_id).await?;
    } else {
        let bytes = fs::read(&output).map_err(|e| e.to_string())?;
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("The component did not produce a valid PNG page.".into());
        }
    }
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    Ok(output)
}

fn library_document(state: &AppState, id: &str) -> Result<PathBuf, String> {
    let guard = state.repository.lock().unwrap();
    let repo = guard.as_ref().ok_or("Open the app first.")?;
    let item = repo
        .data
        .library
        .iter()
        .find(|item| item.id == id)
        .ok_or("Choose an imported document.")?;
    document_path(&item.path, false)
}
struct Preview(PathBuf);
impl Drop for Preview {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn preview_folder(app: &AppHandle) -> Result<Preview, String> {
    let root = crate::components::root(app)?.join("document-previews");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let root = crate::library::authorized_path(&root)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let path = root.join(format!("preview-{}-{nonce}", std::process::id()));
    fs::create_dir(&path).map_err(|e| e.to_string())?;
    Ok(Preview(path))
}
#[tauri::command]
pub async fn document_info(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<DocumentInfo, String> {
    let input = library_document(&state, &id)?;
    let mutool = crate::components::executable(&app, "documents")?;
    let preview = preview_folder(&app)?;
    let (_sender, cancel) = watch::channel(false);
    inspect(&mutool, &input, &preview.0, cancel, 0).await
}
#[tauri::command]
pub async fn document_page(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    page: u32,
) -> Result<DocumentPage, String> {
    let input = library_document(&state, &id)?;
    let mutool = crate::components::executable(&app, "documents")?;
    let preview = preview_folder(&app)?;
    let (_sender, cancel) = watch::channel(false);
    let info = inspect(&mutool, &input, &preview.0, cancel.clone(), 0).await?;
    if page == 0 || page > info.pages {
        return Err("Choose a page inside the document.".into());
    }
    let png = preview.0.join("page.png");
    let args = vec![
        "draw".into(),
        "-q".into(),
        "-W".into(),
        "600".into(),
        "-H".into(),
        "800".into(),
        "-S".into(),
        "12".into(),
        "-F".into(),
        "png".into(),
        "-r".into(),
        "120".into(),
        "-w".into(),
        "1400".into(),
        "-h".into(),
        "2000".into(),
        "-T".into(), "2".into(), "-B".into(),
        "256".into(),
        "-o".into(),
        cli_path(&png),
        cli_path(&input),
        page.to_string(),
    ];
    engine::run_process(mutool.clone(), args, cancel.clone(), 0, true, |_| {}).await?;
    let png = crate::checked_output(&preview.0, &png.to_string_lossy())?;
    if fs::metadata(&png).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
        return Err("The rendered page exceeds the 8 MiB reader limit.".into());
    }
    let png = fs::read(png).map_err(|e| e.to_string())?;
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The reader returned an invalid page image.".into());
    }
    let text = if info.copy {
        engine::run_process(
            mutool,
            vec![
                "draw".into(),
                "-q".into(),
                "-W".into(),
                "600".into(),
                "-H".into(),
                "800".into(),
                "-S".into(),
                "12".into(),
                "-F".into(),
                "txt".into(),
                cli_path(&input),
                page.to_string(),
            ],
            cancel,
            0,
            true,
            |_| {},
        )
        .await?
    } else {
        String::new()
    };
    let text: String = text.chars().take(1_000_000).collect();
    let text_available = !text.trim().is_empty();
    Ok(DocumentPage {
        page,
        pages: info.pages,
        png,
        text,
        text_available,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentHit {
    pub page: u32,
    pub excerpt: String,
}

fn text_hits(text: &str, query: &str, pages: u32) -> Result<Vec<DocumentHit>, String> {
    if pages > 1 && !text.contains('\u{000c}') {
        return Err(
            "The component did not preserve page boundaries. Search the current page instead."
                .into(),
        );
    }
    let query = query.to_lowercase();
    Ok(text
        .split('\u{000c}')
        .take(pages as usize)
        .enumerate()
        .filter_map(|(index, page)| {
            let lower = page.to_lowercase();
            let offset = lower.find(&query)?;
            let before = lower[..offset].chars().count();
            let excerpt = page
                .chars()
                .skip(before.saturating_sub(60))
                .take(220)
                .collect();
            Some(DocumentHit {
                page: index as u32 + 1,
                excerpt,
            })
        })
        .take(100)
        .collect())
}

#[tauri::command]
pub async fn document_search(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    query: String,
) -> Result<Vec<DocumentHit>, String> {
    let query = query.trim();
    if !(2..=160).contains(&query.chars().count()) {
        return Err("Search for 2–160 characters.".into());
    }
    let input = library_document(&state, &id)?;
    let mutool = crate::components::executable(&app, "documents")?;
    let preview = preview_folder(&app)?;
    let (_sender, cancel) = watch::channel(false);
    let info = inspect(&mutool, &input, &preview.0, cancel.clone(), 0).await?;
    if !info.copy {
        return Err("This document does not permit text search or extraction.".into());
    }
    let text = engine::run_process(
        mutool,
        vec![
            "draw".into(),
            "-q".into(),
            "-W".into(),
            "600".into(),
            "-H".into(),
            "800".into(),
            "-S".into(),
            "12".into(),
            "-F".into(),
            "txt".into(),
            cli_path(&input),
        ],
        cancel,
        0,
        true,
        |_| {},
    )
    .await?;
    if text.trim().is_empty() {
        return Err("No extractable text was found. Text search is not OCR.".into());
    }
    text_hits(&text, query, info.pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_search_respects_page_boundaries_and_does_not_claim_ocr() {
        let hits = text_hits(
            "First page\u{000c}\nEspañol con subtítulos\u{000c}",
            "ESPAÑOL",
            2,
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].page, 2);
        assert!(hits[0].excerpt.contains("Español"));
        assert!(text_hits("page one and two with no boundary", "page", 2).is_err());
    }
    #[test]
    fn page_ranges_preserve_order_duplicates_and_reverse_ranges() {
        assert_eq!(
            page_selection("3,1-2,N,4-2", 5).unwrap(),
            vec![3, 1, 2, 5, 4, 3, 2]
        );
        for value in ["0", "6", "1;bad", "-o", "1-2-3", "", "1,,2"] {
            assert!(page_selection(value, 5).is_err(), "{value}");
        }
    }
    #[test]
    fn arguments_separate_untrusted_paths_from_operations() {
        let inputs = vec![PathBuf::from("one.pdf"), PathBuf::from("two.pdf")];
        let options = DocumentOptions {
            operation: "merge".into(),
            pages: "1-N".into(),
            rotation: 0,
            dpi: 144,
        };
        assert_eq!(
            output_arguments(&inputs, Path::new("result.pdf"), &options).unwrap(),
            vec![
                "merge",
                "-o",
                "result.pdf",
                "one.pdf",
                "1-N",
                "two.pdf",
                "1-N"
            ]
        );
        let mut bad = options;
        bad.operation = "run".into();
        assert!(output_arguments(&inputs, Path::new("result.pdf"), &bad).is_err());
        bad.operation = "text".into();
        for range in ["1;rm", "%d", "../../file", "1\n-o file", "1\t2"] {
            bad.pages = range.into();
            assert!(validate_options(&bad).is_err(), "{range}");
        }
    }

    fn pdf_fixture() -> Vec<u8> {
        let first = "BT /F1 12 Tf 20 100 Td (First lesson) Tj ET";
        let second = "BT /F1 12 Tf 20 100 Td (Second lesson) Tj ET";
        let objects=vec!["<< /Type /Catalog /Pages 2 0 R >>".into(),"<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".into(),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R /Resources << /Font << /F1 7 0 R >> >> >>".into(),format!("<< /Length {} >>\nstream\n{first}\nendstream",first.len()),"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 6 0 R /Resources << /Font << /F1 7 0 R >> >> >>".into(),format!("<< /Length {} >>\nstream\n{second}\nendstream",second.len()),"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into()];
        let mut result = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(result.len());
            result.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
        }
        let xref = result.len();
        result.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            result.extend(format!("{offset:010} 00000 n \n").as_bytes());
        }
        result.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        result
    }

    #[tokio::test]
    #[ignore = "Requires a deliberately installed MuPDF component via NABNAB_TEST_MUTOOL; synthetic local PDFs only."]
    async fn real_documents_preserve_inputs_and_verify_pdf_png_and_text() {
        let mutool = std::env::var_os("NABNAB_TEST_MUTOOL")
            .map(PathBuf::from)
            .expect("Set NABNAB_TEST_MUTOOL to the optional installed component.");
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let work = std::env::temp_dir().join(format!(
            "nabnab-document-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&work).unwrap();
        let original = pdf_fixture();
        let input = work.join("Original %d.pdf");
        fs::write(&input, &original).unwrap();
        let (_sender, cancel) = watch::channel(false);
        for (index, (operation, pages, rotation)) in [
            ("pages", "2,1", 0),
            ("rotate", "1-N", 90),
            ("text", "1-N", 0),
            ("image", "2", 0),
            ("merge", "1-N", 0),
        ]
        .into_iter()
        .enumerate()
        {
            let stage = work.join(format!("stage-{index}"));
            fs::create_dir(&stage).unwrap();
            let inputs = if operation == "merge" {
                vec![input.clone(), input.clone()]
            } else {
                vec![input.clone()]
            };
            let output = execute(
                &mutool,
                &inputs,
                &stage,
                &DocumentOptions {
                    operation: operation.into(),
                    pages: pages.into(),
                    rotation,
                    dpi: 144,
                },
                cancel.clone(),
                index as u64,
                |_| {},
            )
            .await
            .unwrap_or_else(|error| panic!("{operation}: {error}"));
            if operation == "text" {
                let text = fs::read_to_string(&output).unwrap();
                assert!(text.contains("First lesson"));
                assert!(text.contains("Second lesson"));
            } else if operation == "image" {
                assert!(fs::read(&output).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
            } else {
                let info = inspect(&mutool, &output, &stage, cancel.clone(), 0)
                    .await
                    .unwrap();
                assert_eq!(info.pages, if operation == "merge" { 4 } else { 2 });
            }
            assert_eq!(fs::read(&input).unwrap(), original);
        }
        assert_eq!(work.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(work).unwrap();
    }
}
