//! Legacy engine regressions with explicit HTTP fixtures. Native direct HTTP is tested separately.
use crate::{
    engine,
    model::{self, Format, Progress},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

fn fixture_base_args(root: &Path) -> Vec<String> {
    let mut args = engine::base_args(root);
    // Only this test module enables the generic extractor for its controlled HTTP fixtures.
    args.extend(["--use-extractors".into(), "default".into()]);
    args
}
fn fixture_download_args(
    root: &Path,
    format: Format,
    height: Option<u32>,
    staging: &Path,
    url: &str,
) -> Vec<String> {
    let mut args = engine::download_args(root, format, height, staging, url);
    if crate::providers::identify(url)
        .map_or(true, |p| p.id == crate::providers::ProviderId::Generic)
    {
        let index = args
            .iter()
            .position(|arg| arg == "--")
            .expect("URL argument boundary");
        args.splice(index..index, ["--use-extractors".into(), "default".into()]);
    }
    args
}
fn engines() -> PathBuf {
    std::env::var_os("YTNABBER_TEST_ENGINES")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engines"))
}
fn probe(root: &Path, file: &Path) -> serde_json::Value {
    let output = Command::new(root.join("ffprobe.exe"))
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(file)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
#[ignore = "Requires bundled Windows engines; uses only a local HTTP fixture"]
async fn real_engines_stop_on_http_429() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
        time::Duration,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let requests = count.clone();
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    let server = std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            if let Ok((mut socket, _)) = listener.accept() {
                socket
                    .set_read_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                let mut buffer = [0; 4096];
                let _ = socket.read(&mut buffer);
                requests.fetch_add(1, Ordering::SeqCst);
                let _ = socket.write_all(b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 120\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    });
    let root = engines();
    let (_sender, cancel) = watch::channel(false);
    let mut args = fixture_base_args(&root);
    args.extend([
        "--dump-single-json".into(),
        "--".into(),
        format!("http://{address}/fixture.mp4"),
    ]);
    let outcome = engine::run(&root, args, cancel, 99, true, |_| {}).await;
    stopped.store(true, Ordering::SeqCst);
    server
        .join()
        .expect("The local HTTP fixture must stop after the engine exits");
    let raw = outcome.expect_err("The HTTP 429 fixture must fail with a structured rate limit");
    let diagnostic = crate::failure::sanitize(&raw);
    assert_eq!(
        crate::failure::Failure::classify(&raw, "extraction").code,
        "rate_limited",
        "{diagnostic}"
    );
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "No immediate HTTP retries allowed"
    );
}

#[tokio::test]
#[ignore = "Requires bundled Windows engines; no Internet needed"]
async fn real_engines_offline_codec_conversion() {
    let root = engines();
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.cache")
        .join(format!("codec test á-{}", std::process::id()));
    fs::create_dir_all(&folder).unwrap();
    let source = folder.join("Vídeo ｜ source.webm");
    let (_sender, cancel) = watch::channel(false);
    let mut args: Vec<String> = [
        "-v",
        "error",
        "-nostdin",
        "-n",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x180:rate=24",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000",
        "-t",
        "2",
        "-c:v",
        "libvpx-vp9",
        "-c:a",
        "libopus",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(source.to_string_lossy().into_owned());
    engine::tool(&root, "ffmpeg.exe", args, cancel.clone())
        .await
        .unwrap();
    let url = url::Url::from_file_path(fs::canonicalize(&source).unwrap()).unwrap();
    for (format, extension) in [
        (Format::Mp4, "mp4"),
        (Format::Source, "source"),
        (Format::Mp3, "mp3"),
        (Format::M4a, "m4a"),
        (Format::Opus, "opus"),
    ] {
        let staging = folder.join(extension);
        fs::create_dir(&staging).unwrap();
        let mut args = fixture_download_args(&root, format, None, &staging, url.as_str());
        args.insert(0, "--enable-file-urls".into());
        let file = engine::run(&root, args, cancel.clone(), 90, false, |_| {})
            .await
            .unwrap();
        let original_file = PathBuf::from(&file);
        let file = crate::media::prepare(&root, &original_file, format, cancel.clone())
            .await
            .unwrap();
        let info = probe(&root, &file);
        crate::media::validate(&info, format).unwrap();
        if format == Format::Mp4 {
            let modified = file.metadata().unwrap().modified().unwrap();
            assert_eq!(
                crate::media::prepare(&root, &original_file, format, cancel.clone())
                    .await
                    .unwrap(),
                file
            );
            assert_eq!(
                file.metadata().unwrap().modified().unwrap(),
                modified,
                "A valid recovered conversion must be reused"
            );
        }
        if format == Format::Source {
            assert!(info["streams"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["codec_name"] == "vp9"));
        }
        let args = vec![
            "-v".into(),
            "error".into(),
            "-i".into(),
            file.to_string_lossy().into_owned(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ];
        engine::tool(&root, "ffmpeg.exe", args, cancel.clone())
            .await
            .unwrap();
        println!(
            "Verified {format:?}: {} bytes, validated and fully decoded",
            file.metadata().unwrap().len()
        );
    }
    let cover = folder.join("audio with cover.mp3");
    let mut args: Vec<String> = ["-v", "error", "-nostdin", "-n", "-i"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    args.push(source.to_string_lossy().into_owned());
    args.extend(
        [
            "-map",
            "0:a:0",
            "-map",
            "0:v:0",
            "-c:a",
            "libmp3lame",
            "-c:v",
            "mjpeg",
            "-frames:v",
            "1",
            "-disposition:v:0",
            "attached_pic",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    args.push(cover.to_string_lossy().into_owned());
    engine::tool(&root, "ffmpeg.exe", args, cancel.clone())
        .await
        .unwrap();
    assert_eq!(
        crate::media::prepare(&root, &cover, Format::Mp3, cancel)
            .await
            .unwrap(),
        cover
    );
    println!("Verified MP3 cover art is preserved without another transcode");
    fs::remove_dir_all(folder).unwrap();
}

#[tokio::test]
#[ignore = "Requires bundled Windows engines; no Internet needed"]
async fn real_engines_720p_and_1080p_compatible_profiles() {
    let root = engines();
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.cache")
        .join(format!("HD test á-{}", std::process::id()));
    fs::create_dir_all(&folder).unwrap();
    let (_sender, cancel) = watch::channel(false);
    for (width, height) in [(1280, 720), (1920, 1080)] {
        let source = folder.join(format!("source-{height}.mp4"));
        let mut args: Vec<String> = ["-v", "error", "-nostdin", "-n", "-f", "lavfi", "-i"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        args.push(format!("testsrc2=size={width}x{height}:rate=24"));
        args.extend(
            [
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "1",
                "-c:v",
                "libopenh264",
                "-b:v",
                "6M",
                "-c:a",
                "aac",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        args.push(source.to_string_lossy().into_owned());
        engine::tool(&root, "ffmpeg.exe", args, cancel.clone())
            .await
            .unwrap();
        let url = url::Url::from_file_path(fs::canonicalize(&source).unwrap()).unwrap();
        let info = serde_json::json!({"id":format!("hd-{height}"),"title":format!("Vídeo ｜ {height}p"),"extractor":"generic","webpage_url":url,"formats":[{"format_id":"fixture","url":url,"ext":"mp4","height":height,"width":width,"vcodec":"avc1","acodec":"mp4a","protocol":"file"}]});
        let fixture = folder.join(format!("{height}.info.json"));
        fs::write(&fixture, serde_json::to_vec(&info).unwrap()).unwrap();
        let staging = folder.join(height.to_string());
        fs::create_dir(&staging).unwrap();
        let mut args =
            fixture_download_args(&root, Format::Mp4, Some(height), &staging, url.as_str());
        args.truncate(args.len() - 2);
        args.extend([
            "--enable-file-urls".into(),
            "--load-info-json".into(),
            fixture.to_string_lossy().into_owned(),
        ]);
        let downloaded = engine::run(&root, args, cancel.clone(), height as u64, false, |_| {})
            .await
            .unwrap();
        let file =
            crate::media::prepare(&root, Path::new(&downloaded), Format::Mp4, cancel.clone())
                .await
                .unwrap();
        assert_eq!(
            file,
            PathBuf::from(downloaded),
            "Compatible tracks must not be transcoded"
        );
        let probed = probe(&root, &file);
        crate::media::validate(&probed, Format::Mp4).unwrap();
        assert!(probed["streams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["height"] == height));
        println!("Verified {height}p H.264/AAC with exact format selection and no extra transcode");
    }
    fs::remove_dir_all(folder).unwrap();
}

#[tokio::test]
#[ignore = "Downloads two short public Big Buck Bunny samples; requires Internet and bundled engines"]
async fn real_engines_public_hd_samples() {
    let root = engines();
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.cache")
        .join(format!(
            "public-hd-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&folder).unwrap();
    let (_sender, cancel) = watch::channel(false);
    // Two seconds of Blender Foundation's open movie keep traffic and fixture storage small.
    let url = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";
    for height in [720, 1080] {
        let staging = folder.join(height.to_string());
        fs::create_dir(&staging).unwrap();
        let mut args = fixture_download_args(&root, Format::Mp4, Some(height), &staging, url);
        let before_url = args.len() - 2;
        args.splice(
            before_url..before_url,
            [
                "--download-sections",
                "*0-2",
                "--retries",
                "0",
                "--extractor-retries",
                "0",
                "--fragment-retries",
                "0",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        let downloaded = engine::run(&root, args, cancel.clone(), height as u64, false, |_| {})
            .await
            .unwrap();
        let file =
            crate::media::prepare(&root, Path::new(&downloaded), Format::Mp4, cancel.clone())
                .await
                .unwrap();
        let info = probe(&root, &file);
        crate::media::validate(&info, Format::Mp4).unwrap();
        assert!(info["streams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["height"] == height));
        let decode = vec![
            "-v".into(),
            "error".into(),
            "-i".into(),
            file.to_string_lossy().into_owned(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ];
        engine::tool(&root, "ffmpeg.exe", decode, cancel.clone())
            .await
            .unwrap();
        println!(
            "Verified public {height}p sample: H.264/AAC, downloaded, probed and fully decoded"
        );
    }
    fs::remove_dir_all(folder).unwrap();
}

#[tokio::test]
#[ignore = "Requires bundled Windows engines; no Internet needed"]
async fn real_engines_missing_dependencies_and_processing_failures() {
    let root = engines();
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.cache")
        .join(format!("missing-tools-{}", std::process::id()));
    fs::create_dir_all(&folder).unwrap();
    let (_sender, cancel) = watch::channel(false);
    let missing = engine::run(
        &folder,
        vec!["--version".into()],
        cancel.clone(),
        100,
        true,
        |_| {},
    )
    .await
    .unwrap_err();
    assert_eq!(
        crate::failure::Failure::classify(&missing, "extraction").code,
        "dependency"
    );
    let corrupt = folder.join("corrupt.mp3");
    fs::write(&corrupt, b"not an audio file").unwrap();
    assert!(
        crate::media::prepare(&root, &corrupt, Format::Mp3, cancel.clone())
            .await
            .is_err()
    );
    assert!(
        corrupt.is_file(),
        "Verification failure must preserve the input"
    );
    let error = engine::tool(
        &root,
        "ffmpeg.exe",
        vec![
            "-v".into(),
            "error".into(),
            "-i".into(),
            folder.join("missing.mp4").to_string_lossy().into_owned(),
        ],
        cancel,
    )
    .await
    .unwrap_err();
    assert_eq!(
        crate::failure::Failure::classify(&error, "download").code,
        "processing"
    );
    fs::remove_dir_all(folder).unwrap();
}
#[tokio::test]
#[ignore = "Requires prepared Windows engines and Internet access; run npm run test:engines"]
async fn real_engines_download_convert_report_progress_and_cancel() {
    let root = engines();
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.cache")
        .join(format!(
            "smoke-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&folder).unwrap();
    let url = "https://www.w3schools.com/html/mov_bbb.mp4";
    let (_sender, receiver) = watch::channel(false);
    let mut args = fixture_base_args(&root);
    args.extend([
        "--dump-single-json".into(),
        "--skip-download".into(),
        "--".into(),
        url.into(),
    ]);
    let raw = engine::run(&root, args, receiver, 1, true, |_| {})
        .await
        .unwrap();
    let media = model::parse_media(&raw, 1).unwrap();
    assert!(media.has_video && media.has_audio);
    println!("Metadata: {}", media.title);
    for (id, format, extension, codec) in [
        (2, Format::Mp4, "mp4", "h264"),
        (3, Format::Mp3, "mp3", "mp3"),
        (4, Format::M4a, "m4a", "aac"),
    ] {
        let staging = folder.join(extension);
        fs::create_dir(&staging).unwrap();
        let (_sender, receiver) = watch::channel(false);
        let events = Arc::new(Mutex::new(Vec::<Progress>::new()));
        let recorded = events.clone();
        let file = engine::run(
            &root,
            fixture_download_args(&root, format, None, &staging, url),
            receiver,
            id,
            false,
            move |progress| recorded.lock().unwrap().push(progress),
        )
        .await
        .unwrap();
        let file = PathBuf::from(file);
        assert_eq!(file.extension().unwrap(), extension);
        let info = probe(&root, &file);
        assert!(
            info["format"]["duration"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap()
                > 9.0
        );
        assert!(
            info["streams"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["codec_name"] == codec),
            "{info}"
        );
        let events = events.lock().unwrap();
        assert!(
            events.iter().any(|p| p.percent.is_some()),
            "Expected real transfer progress"
        );
        if format != Format::Mp4 {
            assert!(
                events.iter().any(|p| p.phase == "processing"),
                "Expected indeterminate conversion status"
            );
            assert!(info["streams"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s["codec_type"] != "video"));
        }
        let saved = crate::publish(&staging, &file.to_string_lossy(), &folder).unwrap();
        assert!(saved.is_file());
        println!(
            "Verified {}: {} bytes; {} progress events",
            extension,
            saved.metadata().unwrap().len(),
            events.len()
        );
    }
    let staging = folder.join("cancelled");
    fs::create_dir(&staging).unwrap();
    let (sender, receiver) = watch::channel(false);
    let mut args = fixture_download_args(&root, Format::Mp4, None, &staging, url);
    args.splice(0..0, ["--limit-rate".into(), "10K".into()]);
    let start = std::time::Instant::now();
    let result = engine::run(&root, args, receiver, 5, false, move |progress| {
        if progress.downloaded.is_some_and(|v| v > 0.0) {
            let _ = sender.send(true);
        }
    })
    .await;
    assert_eq!(result.unwrap_err(), engine::CANCELLED);
    assert!(start.elapsed().as_secs() < 15);
    assert!(!staging.read_dir().unwrap().any(|p| p
        .unwrap()
        .path()
        .extension()
        .is_some_and(|ext| ext == "mp4")));
    println!(
        "Cancellation completed in {:.2}s",
        start.elapsed().as_secs_f64()
    );
    let partial = staging
        .read_dir()
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|file| file.extension().is_some_and(|ext| ext == "part"))
        .expect("Interrupted transfer must leave resumable bytes in this fixture");
    let completed = folder
        .read_dir()
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|file| file.extension().is_some_and(|ext| ext == "mp4"))
        .unwrap();
    let original = fs::read(completed).unwrap();
    // Small cancelled transfers can still be buffered; seed a persisted interruption fixture.
    fs::write(partial, &original[..32768]).unwrap();
    let resumed = Command::new(root.join("yt-dlp.exe"))
        .arg("--no-quiet")
        .args(fixture_download_args(
            &root,
            Format::Mp4,
            None,
            &staging,
            url,
        ))
        .output()
        .unwrap();
    let diagnostic = String::from_utf8_lossy(&resumed.stderr);
    assert!(
        resumed.status.success(),
        "{}",
        crate::failure::sanitize(&diagnostic)
    );
    let stdout = String::from_utf8(resumed.stdout).unwrap();
    assert!(
        stdout.contains("Resuming download at byte")
            || diagnostic.contains("Resuming download at byte"),
        "{}",
        crate::failure::sanitize(&format!("{stdout}\n{diagnostic}"))
    );
    let resumed_file = stdout
        .lines()
        .find_map(|line| line.strip_prefix("__FILE__"))
        .unwrap();
    crate::media::validate(&probe(&root, Path::new(resumed_file)), Format::Mp4).unwrap();
    assert_eq!(fs::read(resumed_file).unwrap(), original);
    println!("Persisted interruption fixture resumed from 32 KiB and matched the complete media.");
    let (_sender, receiver) = watch::channel(false);
    let mut args = fixture_base_args(&root);
    args.extend([
        "--dump-single-json".into(),
        "--".into(),
        "https://www.w3schools.com/this-video-does-not-exist-404.mp4".into(),
    ]);
    assert!(engine::run(&root, args, receiver, 6, true, |_| {})
        .await
        .is_err());
    println!("Unavailable-source failure verified.");
    fs::remove_dir_all(folder).unwrap();
}

#[tokio::test]
#[ignore = "Requires prepared Windows engines and Internet access; run npm run test:engines"]
async fn real_engines_youtube_metadata_and_separate_track_merge() {
    let root = engines();
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.cache")
        .join(format!(
            "youtube-smoke-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&folder).unwrap();
    let url = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
    let (_sender, receiver) = watch::channel(false);
    let mut args = fixture_base_args(&root);
    args.extend([
        "--dump-single-json".into(),
        "--skip-download".into(),
        "--".into(),
        url.into(),
    ]);
    let raw = engine::run(&root, args, receiver, 20, true, |_| {})
        .await
        .unwrap();
    let media = model::parse_media(&raw, 20).unwrap();
    assert_eq!(media.title, "Me at the zoo");
    assert!(media.duration.is_some() && media.thumbnail.is_some());
    let height = *media.heights.last().unwrap();
    let mut download_args = fixture_download_args(&root, Format::Mp4, Some(height), &folder, url);
    download_args.splice(0..0, ["--verbose".into(), "--no-cache-dir".into()]);
    let output = Command::new(root.join("yt-dlp.exe"))
        .args(download_args)
        .env("DENO_DIR", folder.join("deno-cache"))
        .output()
        .unwrap();
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{}",
        crate::failure::sanitize(&diagnostic)
    );
    assert!(
        diagnostic.contains("JS runtimes: deno-"),
        "Bundled Deno must be recognized"
    );
    assert!(
        diagnostic.contains("[jsc:deno] Solving JS challenges"),
        "An actual EJS challenge must run"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let file = stdout
        .lines()
        .find_map(|line| line.strip_prefix("__FILE__"))
        .expect("Actual completed download path");
    let (_probe_sender, probe_cancel) = watch::channel(false);
    let file = crate::media::prepare(&root, Path::new(&file), Format::Mp4, probe_cancel)
        .await
        .unwrap();
    let info = probe(&root, &file);
    crate::media::validate(&info, Format::Mp4).unwrap();
    let streams = info["streams"].as_array().unwrap();
    assert!(streams
        .iter()
        .any(|s| s["codec_type"] == "video" && s["height"].as_u64() == Some(height as u64)));
    assert!(streams.iter().any(|s| s["codec_type"] == "audio"));
    println!(
        "YouTube {}p: bundled Deno solved EJS; video + audio downloaded, merged and probed successfully.",
        height
    );
    fs::remove_dir_all(folder).unwrap();
}

#[tokio::test]
#[ignore = "Requires prepared Windows engines and Internet access; run npm run test:engines"]
async fn real_engines_windows_filename_regression() {
    let root = engines();
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.cache")
        .join("prueba-á")
        .join(format!(
            "filename-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&folder).unwrap();
    let (sender, receiver) = watch::channel(false);
    let mut args = fixture_download_args(
        &root,
        Format::Mp4,
        None,
        &folder,
        "https://www.w3schools.com/html/mov_bbb.mp4",
    );
    let template = args.iter().position(|a| a == "-o").unwrap() + 1;
    args[template] = "Vídeo ｜ Windows [%(id)s].%(ext)s".into();
    let file = engine::run(&root, args, receiver, 30, false, |_| {})
        .await
        .unwrap();

    let saved = crate::publish(&folder, &file, folder.parent().unwrap()).unwrap();
    assert!(saved.is_file());
    fs::remove_file(saved).unwrap();
    fs::remove_dir_all(folder).unwrap();
    drop(sender);
}

#[tokio::test]
#[ignore = "Requires prepared Windows engines and Internet access; run npm run test:engines"]
async fn real_engines_cancel_during_ffmpeg_conversion() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let root = engines();
    let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.cache")
        .join(format!(
            "conversion-cancel-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    fs::create_dir_all(&folder).unwrap();
    let (sender, receiver) = watch::channel(false);
    let started = Arc::new(AtomicBool::new(false));
    let output_seen = Arc::new(AtomicBool::new(false));
    let checked_output = output_seen.clone();
    let destination = folder.clone();
    let mut args = fixture_download_args(
        &root,
        Format::Mp3,
        None,
        &folder,
        "https://www.w3schools.com/html/mov_bbb.mp4",
    );
    // Read at source speed so the test can cancel a live FFmpeg process rather than a finished conversion.
    args.splice(0..0, ["--postprocessor-args".into(), "ffmpeg_i:-re".into()]);
    let result = engine::run(&root, args, receiver, 40, false, move |progress| {
        if progress.phase == "processing" && !started.swap(true, Ordering::SeqCst) {
            let sender = sender.clone();
            let destination = destination.clone();
            let checked_output = checked_output.clone();
            tokio::spawn(async move {
                for _ in 0..40 {
                    let writing = fs::read_dir(&destination).unwrap().any(|entry| {
                        entry
                            .unwrap()
                            .path()
                            .extension()
                            .is_some_and(|ext| ext == "mp3")
                    });
                    if writing {
                        checked_output.store(true, Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        let _ = sender.send(true);
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                let _ = sender.send(true);
            });
        }
    })
    .await;
    assert_eq!(result.unwrap_err(), engine::CANCELLED);
    assert!(
        output_seen.load(Ordering::SeqCst),
        "FFmpeg must have started writing the MP3 before cancellation"
    );
    fs::remove_dir_all(folder).unwrap();
    println!("Cancelled a live FFmpeg conversion and removed its unlocked partial files.");
}
