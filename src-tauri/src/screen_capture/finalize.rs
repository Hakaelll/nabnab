use super::*;

fn list(path: &Path, dir: &Path, names: &[String]) -> Result<(), String> {
    let mut bytes = String::from("ffconcat version 1.0\n");
    for name in names {
        owned_name(name)?;
        let input = dir.join(name);
        regular(&input)?;
        let input = input
            .to_str()
            .ok_or("Capture path is not Unicode.")?
            .replace('\\', "/")
            .replace('\'', "'\\''");
        bytes.push_str(&format!("file '{input}'\n"));
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    output
        .write_all(bytes.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn input_list(args: &mut Vec<String>, path: &Path, formats: &str) {
    args.extend([
        "-protocol_whitelist".into(),
        "file,pipe".into(),
        "-format_whitelist".into(),
        formats.into(),
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        path.to_string_lossy().into_owned(),
    ]);
}
pub async fn finalize(
    root: &Path,
    engines: &Path,
    capture_id: &str,
    staging: &Path,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let journal = readjournal(root, capture_id)?;
    if !matches!(journal.phase.as_str(), "ready" | "interrupted") || journal.videos.is_empty() {
        return Err("Stop or snapshot the capture before finalizing it.".into());
    }
    let dir = capture_dir(root, capture_id)?;
    directory(staging)?;
    if journal.kind == "image" {
        if journal.videos != ["capture.png"] {
            return Err("Invalid screenshot journal.".into());
        }
        let source = dir.join("capture.png");
        if regular(&source)? > MAX_PIXELS * 4 + 1024 * 1024 {
            return Err("Screenshot exceeds its limit.".into());
        }
        let output = staging.join("Screenshot.png");
        let mut input = fs::File::open(source).map_err(|e| e.to_string())?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|e| e.to_string())?;
        std::io::copy(&mut input, &mut file).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        return Ok(output);
    }
    let video_list = staging.join("capture-video.ffconcat");
    list(&video_list, &dir, &journal.videos)?;
    let mut args = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-v".into(),
        "error".into(),
        "-n".into(),
    ];
    input_list(&mut args, &video_list, "concat,matroska,webm");
    let video_probe = engine::run_process(
        engines.join("ffprobe.exe"),
        {
            let mut p = vec!["-v".into(), "error".into()];
            input_list(&mut p, &video_list, "concat,matroska,webm");
            p.extend([
                "-show_entries".into(),
                "format=duration:stream=codec_type,width,height".into(),
                "-of".into(),
                "json".into(),
            ]);
            p
        },
        cancel.clone(),
        run_id,
        true,
        |_| {},
    )
    .await?;
    let info: serde_json::Value = serde_json::from_str(&video_probe).map_err(|e| e.to_string())?;
    // concat duration may be unspecified for a crash-truncated final MKV. A bounded packet scan verifies decodable video below.
    let duration = info["format"]["duration"]
        .as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(journal.duration_ms.max(1) as f64 / 1000.0)
        .min(14402.0);
    let mut tracks = Vec::new();
    for track in &journal.audio {
        if track.files.is_empty() {
            continue;
        }
        if !matches!(track.kind.as_str(), "system" | "microphone") {
            return Err("Invalid capture audio track.".into());
        }
        let path = staging.join(format!("capture-{}.ffconcat", track.kind));
        list(&path, &dir, &track.files)?;
        input_list(&mut args, &path, "concat,wav");
        tracks.push(track);
    }
    let mut filters = Vec::new();
    for (i, t) in tracks.iter().enumerate() {
        let delta = (t.start_us - journal.video_start_us) as f64 / 1_000_000.0;
        let trim = (-delta).max(0.0);
        let delay = (delta.max(0.0) * 1000.0).round() as u64;
        filters.push(format!("[{}:a:0]atrim=start={trim:.6},asetpts=PTS-STARTPTS,adelay={delay}:all=1,apad=whole_dur={duration:.6}[a{i}]",i+1));
    }
    if tracks.len() == 2 {
        filters.extend([
            "[a0]asplit=2[original0][mix0]".into(),
            "[a1]asplit=2[original1][mix1]".into(),
            "[mix0][mix1]amix=inputs=2:normalize=1:duration=longest[mixed]".into(),
        ]);
    }
    if !filters.is_empty() {
        args.extend(["-filter_complex".into(), filters.join(";")]);
    }
    args.extend(["-map".into(), "0:v:0".into(), "-c:v".into(), "copy".into()]);
    if tracks.len() == 2 {
        args.extend([
            "-map".into(),
            "[mixed]".into(),
            "-metadata:s:a:0".into(),
            "title=System + microphone".into(),
            "-disposition:a:0".into(),
            "default".into(),
        ]);
    }
    for (i, t) in tracks.iter().enumerate() {
        let index = if tracks.len() == 2 { i + 1 } else { i };
        args.extend([
            "-map".into(),
            if tracks.len() == 2 {
                format!("[original{i}]")
            } else {
                format!("[a{i}]")
            },
            format!("-metadata:s:a:{index}"),
            format!("title={}", t.kind),
            format!("-disposition:a:{index}"),
            if tracks.len() == 2 { "0" } else { "default" }.into(),
        ]);
    }
    let output = staging.join(if journal.interrupted {
        "Recording-partial.mkv"
    } else {
        "Recording.mkv"
    });
    args.extend([
        "-c:a".into(),
        "pcm_s16le".into(),
        "-t".into(),
        format!("{duration:.6}"),
        "-map_metadata".into(),
        "-1".into(),
        "-metadata".into(),
        format!(
            "comment={}",
            if journal.interrupted {
                "Interrupted capture; available segments recovered"
            } else {
                "Explicit local screen capture"
            }
        ),
        "-f".into(),
        "matroska".into(),
        output.to_string_lossy().into_owned(),
    ]);
    emit(Progress {
        request_id: run_id,
        phase: "processing".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
    engine::run_process(
        engines.join("ffmpeg.exe"),
        args,
        cancel.clone(),
        run_id,
        false,
        emit,
    )
    .await?;
    if regular(&output)? < 100 {
        return Err("Capture finalization produced no video.".into());
    }
    // Decode every frame: a successful remux alone does not prove that an interrupted segment is intact.
    engine::tool(
        engines,
        "ffmpeg.exe",
        vec![
            "-nostdin".into(),
            "-v".into(),
            "error".into(),
            "-xerror".into(),
            "-protocol_whitelist".into(),
            "file,pipe".into(),
            "-f".into(),
            "matroska".into(),
            "-i".into(),
            output.to_string_lossy().into_owned(),
            "-map".into(),
            "0:v:0".into(),
            "-an".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ],
        cancel,
    )
    .await?;
    for name in [
        "capture-video.ffconcat",
        "capture-system.ffconcat",
        "capture-microphone.ffconcat",
    ] {
        let path = staging.join(name);
        if path.exists() {
            regular(&path)?;
            fs::remove_file(path).map_err(|e| e.to_string())?;
        }
    }
    Ok(output)
}
