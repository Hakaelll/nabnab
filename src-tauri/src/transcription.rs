use crate::{captions, engine, model::Progress, tools::ToolOptions};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Emitter};
use tokio::sync::watch;

struct Model {
    id: &'static str,
    bytes: u64,
    sha256: &'static str,
}
const MODELS: [Model; 2] = [
    Model {
        id: "tiny",
        bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    },
    Model {
        id: "small",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
];
fn model(id: &str) -> Result<&'static Model, String> {
    MODELS
        .iter()
        .find(|model| model.id == id)
        .ok_or_else(|| "Choose a supported multilingual model: tiny or small.".into())
}
fn filename(model: &Model) -> String {
    format!("ggml-{}.bin", model.id)
}
fn model_url(model: &Model) -> String {
    format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        filename(model)
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    id: &'static str,
    bytes: u64,
    multilingual: bool,
    license: &'static str,
    source: String,
    installed: bool,
}

#[tauri::command]
pub fn transcription_models(app: AppHandle) -> Result<Vec<ModelStatus>, String> {
    let root = crate::components::root(&app)?.join("models");
    Ok(MODELS
        .iter()
        .map(|model| ModelStatus {
            id: model.id,
            bytes: model.bytes,
            multilingual: true,
            license: "MIT",
            source: model_url(model),
            installed: fs::metadata(root.join(filename(model)))
                .is_ok_and(|meta| meta.is_file() && meta.len() == model.bytes),
        })
        .collect())
}

fn verify_model(path: &Path, model: &Model) -> Result<PathBuf, String> {
    let path = crate::library::authorized_path(path)?;
    let mut file =
        fs::File::open(&path).map_err(|_| "The transcription model could not be opened.")?;
    if file.metadata().map_err(|e| e.to_string())?.len() != model.bytes {
        return Err(
            "The transcription model is incomplete. Download it again in Tools → Transcription."
                .into(),
        );
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let length = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    if format!("{:x}", hash.finalize()) != model.sha256 {
        return Err("The transcription model checksum does not match the pinned model. No transcription was started.".into());
    }
    Ok(path)
}

struct Pending(PathBuf);
impl Drop for Pending {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[tauri::command]
pub async fn install_transcription_model(app: AppHandle, id: String) -> Result<(), String> {
    let selected = model(&id)?;
    let root = crate::components::root(&app)?.join("models");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let root = crate::library::authorized_path(&root)?;
    let target = root.join(filename(selected));
    if target.exists() {
        let checked = target.clone();
        return tauri::async_runtime::spawn_blocking(move || {
            verify_model(&checked, selected).map(|_| ())
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let pending_path = root.join(format!(
        "{}.{}-{nonce}.pending",
        filename(selected),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&pending_path)
        .map_err(|_| "The temporary model file could not be created.")?;
    let pending = Pending(pending_path);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1800))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(model_url(selected))
        .send()
        .await
        .map_err(|_| "The official model source could not be reached.")?
        .error_for_status()
        .map_err(|_| "The official model download failed.")?;
    let mut received = 0u64;
    let mut hash = Sha256::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Model download interrupted. Retry the download.")?
    {
        received = received
            .checked_add(chunk.len() as u64)
            .ok_or("Invalid model size.")?;
        if received > selected.bytes {
            return Err("The model exceeded its pinned size. Nothing was installed.".into());
        }
        hash.update(&chunk);
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        let _ = app.emit(
            "transcription-model-progress",
            serde_json::json!({"id":selected.id,"downloaded":received,"total":selected.bytes}),
        );
    }
    if received != selected.bytes || format!("{:x}", hash.finalize()) != selected.sha256 {
        return Err("Model size or checksum mismatch. Nothing was installed.".into());
    }
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    // A concurrent installer must not replace another model after verification.
    fs::hard_link(&pending.0, &target).map_err(|e| {
        format!("The verified model could not be installed without replacing a file: {e}")
    })?;
    Ok(())
}

fn validate_options(options: &ToolOptions) -> Result<(&str, &str, u8), String> {
    let ToolOptions::Transcribe {
        model: selected,
        language,
        threads,
    } = options
    else {
        return Err("Choose transcription options.".into());
    };
    model(selected)?;
    if language != "auto"
        && (!matches!(language.len(), 2 | 3)
            || !language.bytes().all(|byte| byte.is_ascii_lowercase()))
    {
        return Err(
            "Choose automatic language detection or a two/three-letter language code.".into(),
        );
    }
    if !(1..=8).contains(threads) {
        return Err("Choose 1–8 CPU threads for transcription.".into());
    }
    Ok((selected, language, *threads))
}

fn whisper_arguments(
    model: &Path,
    audio: &Path,
    output: &Path,
    language: &str,
    threads: u8,
) -> Vec<String> {
    vec![
        "--model".into(),
        model.to_string_lossy().into_owned(),
        "--file".into(),
        audio.to_string_lossy().into_owned(),
        "--output-file".into(),
        output.to_string_lossy().into_owned(),
        "--output-srt".into(),
        "--language".into(),
        language.into(),
        "--threads".into(),
        threads.to_string(),
        "--no-gpu".into(),
    ]
}

pub async fn execute(
    engines: &Path,
    whisper: &Path,
    models_root: &Path,
    input: &Path,
    staging: &Path,
    options: &ToolOptions,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<(PathBuf, captions::LocalTrack), String> {
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let (selected, language, threads) = validate_options(options)?;
    let selected = model(selected)?;
    let model_path = models_root.join(filename(selected));
    let model_path =
        tauri::async_runtime::spawn_blocking(move || verify_model(&model_path, selected))
            .await
            .map_err(|e| e.to_string())??;
    let input = crate::library::authorized_path(input)?;
    let staging = crate::library::authorized_path(staging)?;
    let whisper = crate::library::authorized_path(whisper)?;
    if !input.is_file() || !staging.is_dir() {
        return Err("Choose a local media file and an owned job staging folder.".into());
    }
    let info = crate::media::probe(engines, &input, cancel.clone()).await?;
    if !info["streams"]
        .as_array()
        .is_some_and(|streams| streams.iter().any(|stream| stream["codec_type"] == "audio"))
    {
        return Err("Transcription requires a readable audio track.".into());
    }
    let audio = staging.join("transcription-input.wav");
    let stem =
        crate::options::safe_segment(&input.file_stem().unwrap_or_default().to_string_lossy());
    let prefix = staging.join(format!("{stem}.generated-{run_id}"));
    let output = PathBuf::from(format!("{}.srt", prefix.to_string_lossy()));
    if audio.exists() || output.exists() {
        return Err("This transcription staging folder already contains results. Start a new job to preserve them.".into());
    }
    emit(Progress {
        request_id: run_id,
        phase: "processing".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
    let args = vec![
        "-nostdin".into(),
        "-v".into(),
        "error".into(),
        "-n".into(),
        "-protocol_whitelist".into(),
        "file,pipe".into(),
        "-format_whitelist".into(),
        "mov,matroska,webm,mp3,ogg,wav,flac,aac,avi,asf,flv,mpeg,mpegts,nut,ac3,eac3,aiff,ape"
            .into(),
        "-i".into(),
        input.to_string_lossy().into_owned(),
        "-map".into(),
        "0:a:0".into(),
        "-vn".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        "16000".into(),
        "-c:a".into(),
        "pcm_s16le".into(),
        audio.to_string_lossy().into_owned(),
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
    let detected_threads = std::thread::available_parallelism()
        .map(|available| available.get())
        .unwrap_or(1)
        .min(threads as usize) as u8;
    engine::run_process(
        whisper,
        whisper_arguments(&model_path, &audio, &prefix, language, detected_threads),
        cancel.clone(),
        run_id,
        false,
        emit.clone(),
    )
    .await?;
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    emit(Progress {
        request_id: run_id,
        phase: "verifying".into(),
        percent: None,
        speed: None,
        downloaded: None,
        total: None,
    });
    let output = crate::checked_output(&staging, &output.to_string_lossy())?;
    if fs::metadata(&output).map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
        return Err("The generated transcript exceeds the supported 16 MB caption limit.".into());
    }
    let segments = captions::parse(&fs::read_to_string(&output).map_err(|_| {
        "The model produced no readable transcript. The input may contain no speech."
    })?)?;
    fs::write(&output, captions::to_srt(&segments)).map_err(|e| e.to_string())?;
    let track = captions::LocalTrack {
        id: format!("generated-{run_id}"),
        language: language.into(),
        origin: captions::TrackOrigin::Automatic,
        path: None,
        segments,
        formatting_lost: true,
    };
    Ok((output, track))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transcription_uses_multilingual_models_and_bounded_cpu_without_remote_input() {
        let options = ToolOptions::Transcribe {
            model: "tiny".into(),
            language: "es".into(),
            threads: 4,
        };
        assert_eq!(validate_options(&options).unwrap(), ("tiny", "es", 4));
        for (selected, language, threads) in [
            ("tiny.en", "es", 4),
            ("../tiny", "es", 4),
            ("tiny", "es;bad", 4),
            ("tiny", "es", 0),
            ("small", "auto", 9),
        ] {
            assert!(validate_options(&ToolOptions::Transcribe {
                model: selected.into(),
                language: language.into(),
                threads
            })
            .is_err());
        }
        let args = whisper_arguments(
            Path::new("model.bin"),
            Path::new("audio.wav"),
            Path::new("output"),
            "es",
            2,
        );
        assert!(args.windows(2).any(|values| values == ["--language", "es"]));
        assert!(args.windows(2).any(|values| values == ["--threads", "2"]));
        assert!(args.iter().any(|value| value == "--no-gpu"));
        assert!(!args.iter().any(|value| value.starts_with("http")));
        assert_eq!(model("tiny").unwrap().bytes, 77_691_713);
    }
}
