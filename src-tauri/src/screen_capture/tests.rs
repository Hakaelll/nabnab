use super::*;
fn options() -> Options {
    Options {
        mode: Mode::Record,
        fps: 15,
        quality: Quality::Standard,
        duration_seconds: 2,
        disk_limit_mi_b: 64,
        replay_seconds: 30,
        system_audio_id: None,
        microphone_id: None,
    }
}
#[test]
fn boundary_options_ids_and_owned_names_are_closed() {
    let mut o = options();
    validate_options(&o).unwrap();
    o.disk_limit_mi_b = 1;
    assert!(validate_options(&o).is_err());
    o.disk_limit_mi_b = 64;
    o.replay_seconds = 3600;
    assert!(validate_options(&o).is_err());
    assert!(!valid_id("../../outside"));
    assert!(owned_name("../clip.mkv").is_err());
    assert!(owned_name("file:clip").is_err());
    assert!(owned_name("video-00000001.mkv").is_ok());
    assert_eq!(base64(b"hello"), "aGVsbG8=");
}
#[test]
fn journal_recovery_never_starts_capture_and_keeps_interrupted_status() {
    let root = std::env::temp_dir().join(id("c"));
    directory(&root).unwrap();
    let key = id("c");
    let dir = capture_dir(&root, &key).unwrap();
    directory(&dir).unwrap();
    let journal = Journal {
        version: 1,
        id: key.clone(),
        kind: "recording".into(),
        source_label: "Synthetic".into(),
        duration_ms: 2000,
        interrupted: false,
        reason: None,
        phase: "recording".into(),
        video_start_us: 1000000,
        videos: vec!["video-00000000.mkv".into()],
        audio: vec![],
        width: 320,
        height: 200,
        destination: root.clone(),
    };
    write_journal(&dir, &journal).unwrap();
    let active = CaptureLease::open(&dir.join("active.lock"), true).unwrap();
    assert!(recover(&root).unwrap().is_empty());
    drop(active);
    let ready = recover(&root).unwrap();
    assert_eq!(ready.len(), 1);
    assert!(ready[0].interrupted);
    assert_eq!(readjournal(&root, &key).unwrap().phase, "interrupted");
    mark_published(&root, &key).unwrap();
    assert!(recover(&root).unwrap().is_empty());
    fs::remove_dir_all(root).unwrap();
}
async fn tool(engines: &Path, args: Vec<String>) -> String {
    let (_keep_cancel, cancel) = watch::channel(false);
    engine::run_process(engines.join("ffmpeg.exe"), args, cancel, 0, false, |_| {})
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "Uses an explicitly supplied managed FFmpeg; generated pixels/audio only"]
async fn native_capture_finalize_real_offsets_and_separate_audio_tracks() {
    let engines = PathBuf::from(
        std::env::var("NABNAB_CAPTURE_FFMPEG")
            .expect("Set NABNAB_CAPTURE_FFMPEG to the managed engine directory"),
    );
    let root = std::env::temp_dir().join(id("c"));
    directory(&root).unwrap();
    let key = id("c");
    let dir = capture_dir(&root, &key).unwrap();
    directory(&dir).unwrap();
    let staging = root.join("output");
    directory(&staging).unwrap();
    tool(
        &engines,
        vec![
            "-v".into(),
            "error".into(),
            "-n".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            "testsrc2=size=320x200:rate=15:duration=4".into(),
            "-c:v".into(),
            "libopenh264".into(),
            "-g".into(),
            "30".into(),
            "-f".into(),
            "segment".into(),
            "-segment_time".into(),
            "2".into(),
            "-reset_timestamps".into(),
            "1".into(),
            dir.join("video-%08d.mkv").to_string_lossy().into_owned(),
        ],
    )
    .await;
    for (kind, freq) in [("system", 440), ("microphone", 880)] {
        tool(
            &engines,
            vec![
                "-v".into(),
                "error".into(),
                "-n".into(),
                "-f".into(),
                "lavfi".into(),
                "-i".into(),
                format!("sine=frequency={freq}:sample_rate=48000:duration=2"),
                "-ac".into(),
                "2".into(),
                "-c:a".into(),
                "pcm_s16le".into(),
                dir.join(format!("{kind}-00000000.wav"))
                    .to_string_lossy()
                    .into_owned(),
            ],
        )
        .await;
    }
    let journal = Journal {
        version: 1,
        id: key.clone(),
        kind: "recording".into(),
        source_label: "Generated fixture".into(),
        duration_ms: 4000,
        interrupted: false,
        reason: None,
        phase: "ready".into(),
        video_start_us: 1_000_000,
        videos: video_files(&dir).unwrap(),
        audio: vec![
            audio::Track {
                kind: "system".into(),
                start_us: 1_500_000,
                files: vec!["system-00000000.wav".into()],
            },
            audio::Track {
                kind: "microphone".into(),
                start_us: 2_000_000,
                files: vec!["microphone-00000000.wav".into()],
            },
        ],
        width: 320,
        height: 200,
        destination: root.clone(),
    };
    write_journal(&dir, &journal).unwrap();
    let (_keep_cancel, cancel) = watch::channel(false);
    let output = finalize(&root, &engines, &key, &staging, cancel, 1, |_| {})
        .await
        .unwrap();
    let (_keep_cancel, cancel) = watch::channel(false);
    let probe = engine::run_process(
        engines.join("ffprobe.exe"),
        vec![
            "-v".into(),
            "error".into(),
            "-show_streams".into(),
            "-show_format".into(),
            "-of".into(),
            "json".into(),
            output.to_string_lossy().into_owned(),
        ],
        cancel,
        0,
        true,
        |_| {},
    )
    .await
    .unwrap();
    let info: serde_json::Value = serde_json::from_str(&probe).unwrap();
    assert_eq!(info["streams"].as_array().unwrap().len(), 4);
    assert_eq!(info["streams"][0]["width"], 320);
    assert_eq!(info["streams"][1]["tags"]["title"], "System + microphone");
    assert_eq!(info["streams"][1]["disposition"]["default"], 1);
    assert_eq!(info["streams"][2]["tags"]["title"], "system");
    assert_eq!(info["streams"][3]["tags"]["title"], "microphone");
    let mut decoded = Vec::new();
    for (index, silence_ms) in [(0, 500), (1, 500), (2, 1000)] {
        let raw = staging.join(format!("track{index}.pcm"));
        tool(
            &engines,
            vec![
                "-v".into(),
                "error".into(),
                "-n".into(),
                "-i".into(),
                output.to_string_lossy().into_owned(),
                "-map".into(),
                format!("0:a:{index}"),
                "-f".into(),
                "s16le".into(),
                raw.to_string_lossy().into_owned(),
            ],
        )
        .await;
        let bytes = fs::read(raw).unwrap();
        let samples: Vec<i16> = bytes
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let first = samples.iter().position(|n| n.abs() > 100).unwrap() / 2;
        assert!(
            (first as i64 - silence_ms * 48).abs() < 500,
            "audio offset {first}"
        );
        decoded.push(samples);
    }
    assert!(decoded[0][..400 * 48 * 2].iter().all(|s| s.abs() <= 1));
    for index in (550 * 48 * 2..950 * 48 * 2).step_by(37) {
        assert!((decoded[0][index] as i32 - decoded[1][index] as i32 / 2).abs() <= 2);
    }
    for index in (1050 * 48 * 2..1450 * 48 * 2).step_by(37) {
        assert!(
            (decoded[0][index] as i32 - (decoded[1][index] as i32 + decoded[2][index] as i32) / 2)
                .abs()
                <= 2
        );
    }
    assert!(dir.join("video-00000000.mkv").exists());
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
struct FixtureWindow {
    tx: std::sync::mpsc::Sender<u32>,
    handle: Option<std::thread::JoinHandle<()>>,
    title: String,
}
#[cfg(windows)]
impl FixtureWindow {
    fn new() -> Self {
        let title = format!("nabnab synthetic capture {}", id("c"));
        let name = title.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let (ready, started) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _physical = native::PhysicalPixels::new();
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let title: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            unsafe {
                let hwnd = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    title.as_ptr(),
                    WS_VISIBLE | WS_POPUP,
                    100,
                    100,
                    320,
                    200,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                );
                assert!(!hwnd.is_null());
                windows_sys::Win32::Graphics::Gdi::UpdateWindow(hwnd);
                ready.send(()).unwrap();
                loop {
                    if let Ok(action) = rx.try_recv() {
                        if action == 1 {
                            SetWindowPos(
                                hwnd,
                                std::ptr::null_mut(),
                                0,
                                0,
                                340,
                                220,
                                SWP_NOMOVE | SWP_NOZORDER,
                            );
                        } else {
                            break;
                        }
                    }
                    let mut msg = std::mem::zeroed();
                    while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                DestroyWindow(hwnd);
            }
        });
        started.recv_timeout(Duration::from_secs(3)).unwrap();
        Self {
            tx,
            handle: Some(handle),
            title,
        }
    }
}
#[cfg(windows)]
impl Drop for FixtureWindow {
    fn drop(&mut self) {
        let _ = self.tx.send(0);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Captures only a newly created synthetic test window; never desktop/microphone"]
async fn native_capture_owned_window_preview_record_stop_replay_and_resize() {
    async fn wait_for_recoverable_segment(controller: &Controller) {
        let start = Instant::now();
        loop {
            let state = controller.status();
            if state.phase == "recording" && state.elapsed_ms >= 2300 {
                return;
            }
            assert!(
                matches!(state.phase.as_str(), "starting" | "recording")
                    && start.elapsed() < Duration::from_secs(10),
                "Capture did not reach a recoverable segment: {} {:?}",
                state.phase,
                state.reason
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let engines = PathBuf::from(std::env::var("NABNAB_CAPTURE_FFMPEG").unwrap());
    let root = std::env::temp_dir().join(id("c"));
    directory(&root).unwrap();
    let fixture = FixtureWindow::new();
    let controller = Controller::new(root.clone(), engines.clone()).unwrap();
    let sources = controller.sources().await.unwrap();
    let source = sources
        .sources
        .iter()
        .find(|s| s.label == fixture.title)
        .expect("synthetic source enumerated");
    let (_keep_cancel, cancel) = watch::channel(true);
    assert!(
        matches!(controller.preview(&source.id, None, cancel).await, Err(e) if e.contains("Cancelled"))
    );
    let (_keep_cancel, cancel) = watch::channel(false);
    assert!(controller.preview("hwnd=0", None, cancel).await.is_err());
    let (_keep_cancel, cancel) = watch::channel(false);
    let preview = controller.preview(&source.id, None, cancel).await.unwrap();
    assert!(preview.png.starts_with("data:image/png;base64,iVBOR"));
    assert_eq!((preview.width, preview.height), (320, 200));
    let shot = controller
        .screenshot(&preview.preview_id, root.clone())
        .await
        .unwrap();
    assert_eq!(shot.kind, "image");
    let state = controller
        .start(&preview.preview_id, options(), root.clone())
        .await
        .unwrap();
    let key = state.session_id.unwrap();
    tokio::time::sleep(Duration::from_millis(2600)).await;
    let state = controller.stop(&key).await.unwrap();
    assert_eq!(state.phase, "ready", "{:?}", state.reason);
    let ready = state.ready_capture.unwrap();
    assert!(!ready.interrupted);
    let staging = root.join("record-output");
    let (_keep_cancel, cancel) = watch::channel(false);
    finalize(&root, &engines, &ready.id, &staging, cancel, 2, |_| {})
        .await
        .unwrap();
    let mut replay = options();
    replay.mode = Mode::Replay;
    replay.duration_seconds = 60;
    let state = controller
        .start(&preview.preview_id, replay, root.clone())
        .await
        .unwrap();
    let key = state.session_id.unwrap();
    tokio::time::sleep(Duration::from_millis(5300)).await;
    let clip = controller.save_replay(&key).await.unwrap();
    assert!(clip.duration_ms >= 2000 && clip.duration_ms <= 6000);
    let saved = readjournal(&root, &clip.id).unwrap();
    controller.stop(&key).await.unwrap();
    for name in &saved.videos {
        assert!(capture_dir(&root, &clip.id).unwrap().join(name).exists());
    }
    let (_keep_cancel, cancel) = watch::channel(false);
    finalize(
        &root,
        &engines,
        &clip.id,
        &root.join("replay-output"),
        cancel,
        3,
        |_| {},
    )
    .await
    .unwrap();
    let mut recording = options();
    recording.duration_seconds = 60;
    let state = controller
        .start(&preview.preview_id, recording, root.clone())
        .await
        .unwrap();
    let key = state.session_id.unwrap();
    wait_for_recoverable_segment(&controller).await;
    fixture.tx.send(1).unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    let state = controller.stop(&key).await.unwrap();
    assert_eq!(state.phase, "interrupted", "{:?}", state.reason);
    assert!(state.ready_capture.unwrap().interrupted);
    let sources = controller.sources().await.unwrap();
    let source = sources
        .sources
        .iter()
        .find(|s| s.label == fixture.title)
        .unwrap();
    let (_keep_cancel, cancel) = watch::channel(false);
    let preview = controller.preview(&source.id, None, cancel).await.unwrap();
    let mut recording = options();
    recording.duration_seconds = 60;
    let state = controller
        .start(&preview.preview_id, recording, root.clone())
        .await
        .unwrap();
    let key = state.session_id.unwrap();
    wait_for_recoverable_segment(&controller).await;
    fixture.tx.send(0).unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    let state = controller.stop(&key).await.unwrap();
    assert_eq!(state.phase, "interrupted", "{:?}", state.reason);
    assert!(state.ready_capture.unwrap().interrupted);
    controller.shutdown().await;
    drop(fixture);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Runs a 66-second opt-in replay against only a new synthetic window"]
async fn native_capture_replay_retains_only_last_activated_minute() {
    let engines = PathBuf::from(std::env::var("NABNAB_CAPTURE_FFMPEG").unwrap());
    let root = std::env::temp_dir().join(id("c"));
    directory(&root).unwrap();
    let fixture = FixtureWindow::new();
    let controller = Controller::new(root.clone(), engines.clone()).unwrap();
    let sources = controller.sources().await.unwrap();
    let source = sources
        .sources
        .iter()
        .find(|s| s.label == fixture.title)
        .unwrap();
    let (_keep_cancel, cancel) = watch::channel(false);
    let preview = controller.preview(&source.id, None, cancel).await.unwrap();
    let mut settings = options();
    settings.mode = Mode::Replay;
    settings.replay_seconds = 60;
    settings.duration_seconds = 120;
    let activation_us = epoch_us();
    let state = controller
        .start(&preview.preview_id, settings, root.clone())
        .await
        .unwrap();
    let key = state.session_id.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let state = controller.status();
            if state.phase == "replay" {
                break;
            }
            assert_eq!(state.phase, "starting", "{:?}", state.reason);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("The opted-in replay begins producing video");
    for _ in 0..22 {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let state = controller.status();
        assert!(
            matches!(state.phase.as_str(), "starting" | "replay"),
            "{:?}",
            state.reason
        );
        assert!(state.bytes < 64 * 1024 * 1024);
    }
    let state = controller.status();
    assert_eq!(state.available_seconds, 60);
    let saved = controller.save_replay(&key).await.unwrap();
    assert_eq!(saved.duration_ms, 60000);
    let journal = readjournal(&root, &saved.id).unwrap();
    assert_eq!(journal.videos.len(), 30);
    assert!(journal.video_start_us >= activation_us + 4_000_000);
    assert!(audio::segment_number(&journal.videos[0]).unwrap() >= 2);
    let live = capture_dir(&root, &key).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while video_files(&live).unwrap().len() > 31 {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("The live ring prunes closed segments while a snapshot is saved");
    let stopped = controller.stop(&key).await.unwrap();
    assert!(
        video_files(&live).unwrap().is_empty(),
        "{:?}",
        stopped.reason
    );
    let (_keep_cancel, cancel) = watch::channel(false);
    let result = finalize(
        &root,
        &engines,
        &saved.id,
        &root.join("minute-output"),
        cancel,
        5,
        |_| {},
    )
    .await
    .unwrap();
    assert!(regular(&result).unwrap() > 1000);
    let (_keep_cancel, cancel) = watch::channel(false);
    let media = crate::media::probe(&engines, &result, cancel)
        .await
        .unwrap();
    let duration: f64 = media["format"]["duration"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    eprintln!("Measured replay duration: {duration:.3} s");
    assert!(
        (59.8..=60.2).contains(&duration),
        "Encoded replay duration: {duration}"
    );
    controller.shutdown().await;
    drop(fixture);
    fs::remove_dir_all(root).unwrap();
}
