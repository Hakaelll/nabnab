use crate::{
    engine,
    library::{self, LibraryItem},
    model::Progress,
    tools::{self, ToolOptions},
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio::sync::watch;

pub fn validate_options(options: &ToolOptions) -> Result<(), String> {
    if !matches!(options,ToolOptions::AudioProcess{operation,profile}if matches!(operation.as_str(),"normalize"|"silence"|"denoise")&&matches!(profile.as_str(),"podcast"|"broadcast"|"music"))
    {
        return Err(
            "Choose normalization, silence removal or noise reduction for the audio preview."
                .into(),
        );
    }
    Ok(())
}

pub async fn execute(
    engines: &Path,
    input: &LibraryItem,
    staging: &Path,
    options: &ToolOptions,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + Sync + Clone + 'static,
) -> Result<PathBuf, String> {
    validate_options(options)?;
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    let input = library::authorized_path(&input.path)?;
    let staging = library::authorized_path(staging)?;
    let info = crate::media::probe(engines, &input, cancel.clone()).await?;
    if !info["streams"]
        .as_array()
        .is_some_and(|streams| streams.iter().any(|stream| stream["codec_type"] == "audio"))
    {
        return Err("Audio preview requires an input audio track.".into());
    }
    let preview = staging.join(format!("preview-first-20s-{run_id}.wav"));
    if preview.exists() {
        return Err("This preview staging file already exists. Start a new preview job.".into());
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
        "-t".into(),
        "20".into(),
        "-ar".into(),
        "48000".into(),
        "-c:a".into(),
        "pcm_f32le".into(),
        preview.to_string_lossy().into_owned(),
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
    let preview = crate::checked_output(&staging, &preview.to_string_lossy())?;
    if fs::metadata(&preview).map_err(|e| e.to_string())?.len() == 0 {
        return Err("The selected input produced no preview audio.".into());
    }
    tools::execute(engines, &preview, &staging, options, cancel, run_id, emit).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previews_only_accept_real_audio_processing_options() {
        for operation in ["normalize", "silence", "denoise"] {
            assert!(validate_options(&ToolOptions::AudioProcess {
                operation: operation.into(),
                profile: "podcast".into()
            })
            .is_ok());
        }
        assert!(validate_options(&ToolOptions::Mute).is_err());
        assert!(validate_options(&ToolOptions::AudioProcess {
            operation: "magic".into(),
            profile: "podcast".into()
        })
        .is_err());
    }
    #[tokio::test]
    #[ignore = "Requires bundled FFmpeg/ffprobe; generates synthetic local audio only."]
    async fn real_engines_audio_preview_processes_only_the_first_twenty_seconds() {
        let engines = std::env::var_os("NABNAB_TEST_ENGINES")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engines"));
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let work = std::env::temp_dir().join(format!(
            "nabnab-audio-preview-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&work).unwrap();
        let input = work.join("original.wav");
        let (_sender, cancel) = watch::channel(false);
        let mut args: Vec<String> = [
            "-nostdin",
            "-v",
            "error",
            "-n",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=30",
            "-c:a",
            "pcm_s16le",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        args.push(input.to_string_lossy().into_owned());
        engine::tool(&engines, "ffmpeg.exe", args, cancel.clone())
            .await
            .unwrap();
        let original = fs::read(&input).unwrap();
        let item = library::index(&input, None, None, None).unwrap();
        for (index, operation) in ["normalize", "silence", "denoise"].into_iter().enumerate() {
            let stage = work.join(format!("stage-{index}"));
            fs::create_dir(&stage).unwrap();
            let result = execute(
                &engines,
                &item,
                &stage,
                &ToolOptions::AudioProcess {
                    operation: operation.into(),
                    profile: "podcast".into(),
                },
                cancel.clone(),
                index as u64,
                |_| {},
            )
            .await
            .unwrap();
            assert!(result
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("preview-first-20s"));
            let info = crate::media::probe(&engines, &result, cancel.clone())
                .await
                .unwrap();
            let duration = info["format"]["duration"]
                .as_str()
                .unwrap()
                .parse::<f64>()
                .unwrap();
            assert!((duration - 20.0).abs() < 0.25);
            assert_eq!(fs::read(&input).unwrap(), original);
        }
        assert_eq!(work.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(work).unwrap();
    }
}
