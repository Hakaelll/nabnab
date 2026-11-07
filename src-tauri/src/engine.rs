use crate::model::{parse_progress, Progress};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tauri::AppHandle;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    sync::watch,
};

pub const CANCELLED: &str = "Cancelled.";

pub fn engine_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let root = crate::dependencies::engine_root(app)?;
    for name in ["yt-dlp.exe", "ffmpeg.exe", "ffprobe.exe", "deno.exe"] {
        if !root.join(name).is_file() {
            return Err("The download tools are missing. Repair nabnab's engines in Settings, or run npm run engines:prepare when developing.".into());
        }
    }
    Ok(root)
}

pub fn subtitle_args(
    language: Option<&str>,
    format: crate::model::Format,
) -> Result<Vec<String>, String> {
    let Some(language) = language else {
        return Ok(Vec::new());
    };
    if !format.is_video() {
        return Err("Subtitles can be embedded only with a video output profile.".into());
    }
    if crate::model::normalized_subtitle_language(language).is_none() {
        return Err("Choose one available manual subtitle language.".into());
    }
    Ok(vec![
        "--write-subs".into(),
        "--sub-langs".into(),
        format!("^{language}$"),
        "--embed-subs".into(),
        "--no-write-auto-subs".into(),
    ])
}

pub fn advanced_subtitle_args(
    languages: &[String],
    mode: &str,
    automatic: bool,
    format: crate::model::Format,
) -> Result<Vec<String>, String> {
    if mode == "none" {
        return Ok(Vec::new());
    }
    if !matches!(mode, "sidecar" | "embed" | "only") || languages.is_empty() || languages.len() > 10
    {
        return Err("Choose one to ten available subtitle tracks and their output mode.".into());
    }
    if mode == "embed" && !format.is_video() {
        return Err("Embed subtitles with a compatible video container.".into());
    }
    if languages
        .iter()
        .any(|language| crate::model::normalized_subtitle_language(language).is_none())
    {
        return Err("Choose available subtitle language codes.".into());
    }
    let mut args = vec![
        if automatic {
            "--write-auto-subs"
        } else {
            "--write-subs"
        }
        .into(),
        if automatic {
            "--no-write-subs"
        } else {
            "--no-write-auto-subs"
        }
        .into(),
        "--sub-langs".into(),
        languages
            .iter()
            .map(|language| format!("^{language}$"))
            .collect::<Vec<_>>()
            .join(","),
        "--sub-format".into(),
        "srt/vtt/best".into(),
        "--convert-subs".into(),
        "srt".into(),
    ];
    if mode == "embed" {
        args.push("--embed-subs".into());
    }
    if mode == "only" {
        args.push("--skip-download".into());
    }
    Ok(args)
}

pub fn section_args(start: f64, end: f64, precise: bool) -> Result<Vec<String>, String> {
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
        return Err("Choose a valid fragment interval.".into());
    }
    let mut args = vec!["--download-sections".into(), format!("*{start}-{end}")];
    if precise {
        args.push("--force-keyframes-at-cuts".into());
    }
    Ok(args)
}

pub fn base_args(root: &Path) -> Vec<String> {
    vec![
        "--ignore-config".into(),
        "--no-plugin-dirs".into(),
        "--use-extractors".into(),
        "default,-udemy.*".into(),
        "--no-remote-components".into(),
        "--no-playlist".into(),
        "--no-color".into(),
        "--encoding".into(),
        "utf-8".into(),
        "--windows-filenames".into(),
        "--no-js-runtimes".into(),
        "--js-runtimes".into(),
        format!("deno:{}", root.join("deno.exe").display()),
        "--ffmpeg-location".into(),
        root.to_string_lossy().into_owned(),
        "--socket-timeout".into(),
        "20".into(),
        "--retries".into(),
        "2".into(),
        "--extractor-retries".into(),
        "2".into(),
        "--fragment-retries".into(),
        "2".into(),
        "--abort-on-unavailable-fragments".into(),
        "--retry-sleep".into(),
        "http:exp=2:30".into(),
        "--retry-sleep".into(),
        "fragment:exp=2:30".into(),
        "--retry-sleep".into(),
        "extractor:exp=2:30".into(),
    ]
}

pub fn inspect_args(root: &Path, url: &str) -> Vec<String> {
    let mut args = base_args(root);
    args.retain(|arg| arg != "--no-playlist");
    args.extend([
        "--yes-playlist".into(),
        "--flat-playlist".into(),
        "--playlist-end".into(),
        "200".into(),
        "--dump-single-json".into(),
        "--skip-download".into(),
        "--".into(),
        url.into(),
    ]);
    args
}

#[cfg(windows)]
pub(crate) struct ProcessTree(isize);
#[cfg(windows)]
impl ProcessTree {
    pub(crate) fn attach(child: &tokio::process::Child) -> Result<Self, String> {
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of_val(&info) as u32,
            );
            let assigned = child
                .raw_handle()
                .is_some_and(|process| AssignProcessToJobObject(job, process as _) != 0);
            if set == 0 || !assigned {
                let error = std::io::Error::last_os_error().to_string();
                CloseHandle(job);
                return Err(error);
            }
            Ok(Self(job as isize))
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}

fn emit_phase(emit: &impl Fn(Progress), request_id: u64, phase: &str) {
    emit(Progress {
        request_id,
        phase: phase.into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
}

#[cfg(test)]
pub async fn run(
    root: &Path,
    args: Vec<String>,
    cancel: watch::Receiver<bool>,
    request_id: u64,
    metadata: bool,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<String, String> {
    run_process(
        root.join("yt-dlp.exe"),
        args,
        cancel,
        request_id,
        metadata,
        emit,
    )
    .await
}

pub async fn tool(
    root: &Path,
    name: &str,
    args: Vec<String>,
    cancel: watch::Receiver<bool>,
) -> Result<String, String> {
    run_process(
        root.join(name),
        args,
        cancel,
        0,
        name == "ffprobe.exe",
        |_| {},
    )
    .await
}

pub async fn run_process(
    executable: PathBuf,
    args: Vec<String>,
    cancel: watch::Receiver<bool>,
    request_id: u64,
    metadata: bool,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<String, String> {
    let mut command = Command::new(executable);
    command.args(args);
    run_configured_process(command, cancel, request_id, metadata, emit).await
}

pub(crate) async fn run_configured_process(
    mut command: Command,
    mut cancel: watch::Receiver<bool>,
    request_id: u64,
    metadata: bool,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<String, String> {
    if *cancel.borrow() {
        return Err(CANCELLED.into());
    }
    let tool_name = Path::new(command.as_std().get_program())
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Could not start the download tools: {e}"))?;
    #[cfg(windows)]
    let tree = ProcessTree::attach(&child)?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not read the download process.")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("Could not read process errors.")?;
    let err_emit = emit.clone();
    let (rate_limited, mut rate_limit) = watch::channel(false);
    let _rate_guard = rate_limited.clone();
    let mut err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut tail = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            let lower = line.to_lowercase();
            if lower.contains("http error 429")
                || lower.contains("http 429")
                || lower.contains("too many requests")
            {
                let _ = rate_limited.send(true);
            }
            if let Some(progress) = parse_progress(&line, request_id) {
                err_emit(progress);
                continue;
            }
            if line.starts_with("__POST__")
                || line.starts_with("[Merger]")
                || line.starts_with("[ExtractAudio]")
                || line.starts_with("[VideoRemuxer]")
            {
                emit_phase(&err_emit, request_id, "processing");
                continue;
            }
            tail.push(line);
            if tail.len() > 12 {
                tail.remove(0);
            }
        }
        tail.join("\n")
    });
    let output = async {
        let mut result = String::new();
        if metadata {
            let mut reader = BufReader::new(stdout).take(32 * 1024 * 1024);
            reader
                .read_to_string(&mut result)
                .await
                .map_err(|e| e.to_string())?;
        } else {
            let mut lines = BufReader::new(stdout).lines();
            while let Some(line) = lines.next_line().await.map_err(|e| e.to_string())? {
                if let Some(progress) = parse_progress(&line, request_id) {
                    emit(progress);
                } else if line.starts_with("__POST__")
                    || line.starts_with("[Merger]")
                    || line.starts_with("[ExtractAudio]")
                    || line.starts_with("[VideoRemuxer]")
                {
                    emit_phase(&emit, request_id, "processing");
                } else if line.starts_with("__TRANSFER__") {
                    emit_phase(&emit, request_id, "downloading");
                } else if let Some(file) = line.strip_prefix("__FILE__") {
                    result = file.to_owned();
                }
            }
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        Ok::<_, String>((status, result))
    };
    // Metadata gets a fixed deadline; downloads allow slow transfers and conversion.
    let timeout = if metadata {
        Duration::from_secs(120)
    } else {
        Duration::from_secs(24 * 60 * 60)
    };
    let outcome = tokio::select! {
        _ = rate_limit.changed() => Err("RATE_LIMITED".to_owned()),
        _ = cancel.changed() => Err(CANCELLED.to_owned()),
        _ = tokio::time::sleep(timeout) => Err("The source timed out. Check your connection and try again.".to_owned()),
        result = output => result,
    };
    #[cfg(windows)]
    drop(tree);
    if outcome.is_err() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    let errors = match tokio::time::timeout(Duration::from_secs(3), &mut err_task).await {
        Ok(Ok(tail)) => tail,
        _ => {
            err_task.abort();
            String::new()
        }
    };
    let (status, result) = outcome.map_err(|error| {
        if error == "RATE_LIMITED" {
            format!("HTTP Error 429\n{errors}")
        } else {
            error
        }
    })?;
    if !status.success() {
        if status.code() == Some(0xc0000135u32 as i32) {
            return Err(format!(
                "Could not start {tool_name}: required DLLs are missing. Repair the installation."
            ));
        }
        let error: String = if errors.is_empty() {
            "The download engine stopped unexpectedly. Try another public video URL.".into()
        } else {
            errors.chars().take(2400).collect()
        };
        return Err(format!("{tool_name}: {error}"));
    }
    Ok(result)
}

pub fn download_args(
    root: &Path,
    format: crate::model::Format,
    height: Option<u32>,
    staging: &Path,
    url: &str,
) -> Vec<String> {
    let mut args = base_args(root);
    args.extend(crate::model::format_args(format, height));
    args.extend([
        "--no-overwrites".into(),
        "--continue".into(),
        "--no-simulate".into(),
        "--newline".into(),
        "--progress".into(),
        "--progress-delta".into(),
        "0.3".into(),
        "--progress-template".into(),
        "download:__PROGRESS__%(progress)j".into(),
        "--progress-template".into(),
        "postprocess:__POST__%(progress)j".into(),
        "--print".into(),
        "before_dl:__TRANSFER__".into(),
        "--print".into(),
        "after_move:__FILE__%(filepath)s".into(),
        "--paths".into(),
        staging.to_string_lossy().into_owned(),
        "-o".into(),
        "%(title).150B [%(id)s].%(ext)s".into(),
        "--".into(),
        url.to_owned(),
    ]);
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subtitle_outputs_and_sections_keep_provenance_and_real_engine_options() {
        let languages =
            ["es", "en", "fr", "de", "it", "pt", "ja", "ko", "zh", "nl"].map(str::to_owned);
        let arguments =
            advanced_subtitle_args(&languages, "embed", false, crate::model::Format::Mp4).unwrap();
        let index = arguments
            .iter()
            .position(|value| value == "--sub-langs")
            .unwrap();
        assert_eq!(
            arguments[index + 1],
            "^es$,^en$,^fr$,^de$,^it$,^pt$,^ja$,^ko$,^zh$,^nl$"
        );
        let automatic =
            advanced_subtitle_args(&["es".into()], "only", true, crate::model::Format::Source)
                .unwrap();
        assert!(automatic.contains(&"--write-auto-subs".into()));
        assert!(automatic.contains(&"--no-write-subs".into()));
        assert!(automatic.contains(&"--skip-download".into()));
        assert!(
            advanced_subtitle_args(&["es".into()], "embed", false, crate::model::Format::Mp3)
                .is_err()
        );
        assert_eq!(
            section_args(134.5, 150.0, false).unwrap(),
            vec!["--download-sections", "*134.5-150"]
        );
        assert!(section_args(134.5, 150.0, true)
            .unwrap()
            .contains(&"--force-keyframes-at-cuts".into()));
        assert!(section_args(150.0, 134.5, false).is_err());
    }

    #[test]
    fn subtitles_are_one_exact_manual_language_and_never_an_audio_profile() {
        assert_eq!(
            subtitle_args(None, crate::model::Format::Mp3).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            subtitle_args(Some("en-US"), crate::model::Format::Mp4).unwrap(),
            vec![
                "--write-subs",
                "--sub-langs",
                "^en-US$",
                "--embed-subs",
                "--no-write-auto-subs"
            ]
        );
        assert!(subtitle_args(Some("es"), crate::model::Format::Source).is_ok());
        for language in [
            "all",
            "ALL",
            "live_chat",
            "en.*",
            "en,es",
            "--exec",
            "en\n--exec",
            "../en",
            "en--US",
            "a",
            "123",
            "en-é",
            "und",
            "xyz",
            "mul",
            "zxx",
        ] {
            assert!(
                subtitle_args(Some(language), crate::model::Format::Mp4).is_err(),
                "{language}"
            );
        }
        for format in [
            crate::model::Format::Mp3,
            crate::model::Format::M4a,
            crate::model::Format::Opus,
            crate::model::Format::AudioOriginal,
        ] {
            assert!(subtitle_args(Some("en"), format).is_err());
        }
    }

    #[test]
    fn analysis_is_bounded_and_does_not_download_or_enable_remote_code() {
        let args = inspect_args(
            Path::new("C:/Managed engines"),
            "https://www.youtube.com/playlist?list=one",
        );
        assert!(args.contains(&"--skip-download".into()));
        assert!(args.contains(&"--flat-playlist".into()));
        let limit = args.iter().position(|s| s == "--playlist-end").unwrap();
        assert_eq!(args[limit + 1], "200");
        assert!(args.contains(&"--no-remote-components".into()));
        assert!(!args.contains(&"--no-playlist".into()));
        assert_eq!(args[args.len() - 2], "--");
        assert_eq!(
            args.last().unwrap(),
            "https://www.youtube.com/playlist?list=one"
        );
    }

    #[test]
    fn downloads_stay_single_item_with_managed_runtime_and_no_user_config() {
        let args = download_args(
            Path::new("C:/Managed engines"),
            crate::model::Format::AudioOriginal,
            None,
            Path::new("C:/Output"),
            "https://example.org/a",
        );
        assert!(args.contains(&"--no-playlist".into()));
        assert!(args.contains(&"--ignore-config".into()));
        assert!(args.contains(&"--no-plugin-dirs".into()));
        assert!(args.contains(&"--no-remote-components".into()));
        assert!(args
            .iter()
            .any(|arg| arg.starts_with("deno:C:/Managed engines")));
    }
}
