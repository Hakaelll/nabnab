use crate::model::Format;
use serde_json::Value;

fn compatible_h264(stream: &Value) -> bool {
    stream["codec_name"] == "h264"
        && matches!(stream["pix_fmt"].as_str(), Some("yuv420p" | "yuvj420p"))
}
fn video_track(stream: &Value) -> bool {
    stream["codec_type"] == "video" && stream["disposition"]["attached_pic"] != 1
}
fn subtitle_tracks(info: &Value) -> impl Iterator<Item = &Value> {
    info["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|stream| {
            stream["codec_type"] == "subtitle"
                && stream["codec_name"]
                    .as_str()
                    .is_some_and(|codec| !codec.is_empty() && !matches!(codec, "none" | "unknown"))
        })
}
fn subtitle_count(info: &Value) -> usize {
    subtitle_tracks(info).count()
}
fn subtitle_language_counts(info: &Value) -> std::collections::BTreeMap<&'static str, usize> {
    let mut counts = std::collections::BTreeMap::new();
    for language in subtitle_tracks(info)
        .filter_map(|stream| stream["tags"]["language"].as_str())
        .filter_map(crate::model::normalized_subtitle_language)
    {
        *counts.entry(language).or_insert(0) += 1;
    }
    counts
}

pub fn validate_subtitle(info: &Value, language: &str) -> Result<(), String> {
    let expected = crate::model::normalized_subtitle_language(language)
        .ok_or("Choose one available manual subtitle language with a recognized ISO code.")?;
    if !subtitle_language_counts(info).contains_key(expected) {
        return Err("The selected subtitles were not found with the requested language in the finished media. Choose another manual language or download without subtitles. The verified media was kept for recovery.".into());
    }
    Ok(())
}
fn validate_preserved_subtitles(source: &Value, output: &Value) -> Result<(), String> {
    let required = subtitle_language_counts(source);
    let actual = subtitle_language_counts(output);
    if subtitle_count(output) < subtitle_count(source)
        || required
            .iter()
            .any(|(language, count)| actual.get(language).copied().unwrap_or(0) < *count)
    {
        return Err("The MP4 conversion did not preserve the embedded subtitles. Choose original quality or download without subtitles. The original media was kept.".into());
    }
    Ok(())
}

pub fn validate(info: &Value, format: Format) -> Result<(), String> {
    let streams = info["streams"]
        .as_array()
        .ok_or("FFprobe found no streams.")?;
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    let video = streams.iter().find(|s| video_track(s));
    let container = info["format"]["format_name"].as_str().unwrap_or_default();
    let number = |name: &str| {
        info["format"][name]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    if number("duration") <= 0.0
        || !number("duration").is_finite()
        || number("size") <= 0.0
        || !number("size").is_finite()
    {
        return Err("FFprobe found an empty file or invalid duration.".into());
    }
    let codec = audio
        .and_then(|s| s["codec_name"].as_str())
        .unwrap_or_default();
    let valid = match format {
        Format::Mp3 => codec == "mp3" && container == "mp3" && video.is_none(),
        Format::M4a => {
            codec == "aac" && container.split(',').any(|c| c == "m4a") && video.is_none()
        }
        Format::Mp4 => {
            video.is_some_and(compatible_h264)
                && (audio.is_none() || codec == "aac")
                && container.split(',').any(|c| c == "mp4")
                && info["format"]["tags"]["major_brand"] != "qt  "
        }
        Format::Source => video.is_some() && !container.is_empty(),
        Format::Opus => codec == "opus" && container == "ogg" && video.is_none(),
        Format::AudioOriginal => {
            audio.is_some() && !codec.is_empty() && !container.is_empty() && video.is_none()
        }
    };
    if !valid {
        return Err("The generated codecs or container do not match the requested format. The recoverable file was kept.".into());
    }
    Ok(())
}

use std::path::{Path, PathBuf};
use tokio::sync::watch;

pub async fn probe(
    root: &Path,
    file: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<Value, String> {
    let file = crate::library::authorized_path(file)?;
    if !file.is_file() {
        return Err("Choose an existing local media file.".into());
    }
    let mut args: Vec<String> = [
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-format_whitelist",
        "mov,matroska,webm,mp3,ogg,wav,flac,aac,avi,asf,flv,mpeg,mpegts,nut,ac3,eac3,aiff,ape",
        "-show_streams",
        "-show_chapters",
        "-show_format",
        "-of",
        "json",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    args.push(file.to_string_lossy().into_owned());
    let raw = crate::engine::tool(root, "ffprobe.exe", args, cancel).await?;
    serde_json::from_str(&raw).map_err(|_| "FFprobe returned invalid media information.".into())
}

pub async fn prepare(
    root: &Path,
    file: &Path,
    format: Format,
    cancel: watch::Receiver<bool>,
) -> Result<PathBuf, String> {
    let info = probe(root, file, cancel.clone()).await?;
    if format != Format::Mp4 || validate(&info, format).is_ok() {
        validate(&info, format)?;
        return Ok(file.to_owned());
    }
    let streams = info["streams"]
        .as_array()
        .ok_or("FFprobe found no streams.")?;
    let video = streams
        .iter()
        .find(|s| video_track(s))
        .ok_or("The downloaded file has no video.")?;
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    let output = file.with_file_name(format!(
        "{}.compatible.mp4",
        file.file_stem().unwrap_or_default().to_string_lossy()
    ));
    if output.is_file() {
        if probe(root, &output, cancel.clone())
            .await
            .is_ok_and(|output| {
                validate(&output, format).is_ok()
                    && validate_preserved_subtitles(&info, &output).is_ok()
            })
        {
            return Ok(output);
        }
        if *cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        std::fs::remove_file(&output)
            .map_err(|e| format!("Could not replace an incomplete conversion: {e}"))?;
    }
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-protocol_whitelist", "file", "-format_whitelist", "mov,matroska,webm,avi,mp3,aac,ogg,wav,flac,asf,mpeg,mpegts,aiff,flv,mp3,wv,ape,amr,ac3,eac3,dts,tta,alac", "-i"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    args.push(file.to_string_lossy().into_owned());
    let video_index = video["index"]
        .as_u64()
        .ok_or("FFprobe returned an invalid video stream index.")?;
    args.extend(["-map".into(), format!("0:{video_index}")]);
    args.extend(["-map", "0:a:0?", "-c:v"].into_iter().map(str::to_owned));
    args.push(
        if compatible_h264(video) {
            "copy"
        } else {
            "libopenh264"
        }
        .into(),
    );
    if !compatible_h264(video) {
        // Keep source dimensions; use a generous bitrate only when video conversion is unavoidable.
        args.extend(
            ["-b:v", "12M", "-pix_fmt", "yuv420p"]
                .into_iter()
                .map(str::to_owned),
        );
    }
    args.push("-c:a".into());
    args.push(
        if audio.is_none_or(|s| s["codec_name"] == "aac") {
            "copy"
        } else {
            "aac"
        }
        .into(),
    );
    if audio.is_some_and(|s| s["codec_name"] != "aac") {
        args.extend(["-b:a".into(), "192k".into()]);
    }
    args.extend([
        "-map".into(),
        "0:s?".into(),
        "-c:s".into(),
        "mov_text".into(),
    ]);
    args.extend([
        "-movflags".into(),
        "+faststart".into(),
        output.to_string_lossy().into_owned(),
    ]);
    crate::engine::tool(root, "ffmpeg.exe", args, cancel.clone()).await?;
    let result = probe(root, &output, cancel).await?;
    validate(&result, format)?;
    validate_preserved_subtitles(&info, &result)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn subtitle_language_must_match_the_requested_primary_language() {
        let mut info = fixture("mov,mp4,m4a,3gp,3g2,mj2", "aac", Some("h264"));
        info["streams"].as_array_mut().unwrap().push(
            json!({"codec_type":"subtitle","codec_name":"mov_text","tags":{"language":"eng"}}),
        );
        assert!(validate_subtitle(&info, "es").is_err());
        assert!(validate_subtitle(&info, "en-US").is_ok());
        for language in [
            None,
            Some("und"),
            Some("unknown"),
            Some("xyz"),
            Some("mul"),
            Some("zxx"),
        ] {
            let track = info["streams"].as_array_mut().unwrap().last_mut().unwrap();
            track["tags"] = language
                .map(|tag| json!({"language":tag}))
                .unwrap_or(json!({}));
            assert!(validate_subtitle(&info, "es").is_err(), "{language:?}");
            assert!(validate_subtitle(&info, "en").is_err(), "{language:?}");
        }
    }
    #[test]
    fn subtitle_preservation_keeps_known_languages_not_just_track_count() {
        let mut source = fixture("matroska,webm", "opus", Some("vp9"));
        source["streams"]
            .as_array_mut()
            .unwrap()
            .push(json!({"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"spa"}}));
        let mut output = source.clone();
        output["streams"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["tags"]["language"] = json!("eng");
        assert!(validate_preserved_subtitles(&source, &output).is_err());
        output["streams"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap()["tags"]["language"] = json!("es-ES");
        assert!(validate_preserved_subtitles(&source, &output).is_ok());
        source["streams"]
            .as_array_mut()
            .unwrap()
            .push(json!({"codec_type":"subtitle","codec_name":"subrip","tags":{"language":"spa"}}));
        output["streams"].as_array_mut().unwrap().push(
            json!({"codec_type":"subtitle","codec_name":"mov_text","tags":{"language":"eng"}}),
        );
        assert_eq!(subtitle_count(&source), subtitle_count(&output));
        assert!(validate_preserved_subtitles(&source, &output).is_err());
    }
    #[test]
    fn subtitle_validation_requires_an_embedded_track_and_preserves_existing_tracks() {
        let plain = fixture("mov,mp4,m4a,3gp,3g2,mj2", "aac", Some("h264"));
        assert!(validate_subtitle(&plain, "en").is_err());
        let mut captioned = plain.clone();
        captioned["streams"].as_array_mut().unwrap().push(
            json!({"codec_type":"subtitle","codec_name":"mov_text","tags":{"language":"eng"}}),
        );
        assert!(validate_subtitle(&captioned, "en").is_ok());
        assert!(validate_subtitle(&captioned, "en-US").is_ok());
        assert!(validate_subtitle(&captioned, "all").is_err());
        assert!(validate_preserved_subtitles(&captioned, &plain).is_err());
        assert!(validate_preserved_subtitles(&captioned, &captioned).is_ok());
        let mut unknown = plain;
        unknown["streams"]
            .as_array_mut()
            .unwrap()
            .push(json!({"codec_type":"subtitle","codec_name":"unknown"}));
        assert!(validate_subtitle(&unknown, "en").is_err());
    }
    #[test]
    fn compatible_profile_remuxes_quicktime_instead_of_relabeling_it_mp4() {
        let mut info = fixture("mov,mp4,m4a,3gp,3g2,mj2", "aac", Some("h264"));
        info["format"]["tags"]["major_brand"] = json!("qt  ");
        assert!(validate(&info, Format::Mp4).is_err());
        assert!(validate(&info, Format::Source).is_ok());
    }

    fn fixture(container: &str, audio: &str, video: Option<&str>) -> Value {
        let mut streams = vec![json!({"codec_type":"audio", "codec_name":audio})];
        if let Some(codec) = video {
            streams.push(json!({"codec_type":"video", "codec_name":codec,"pix_fmt":"yuv420p"}));
        }
        json!({"streams":streams,"format":{"format_name":container,"duration":"10.0","size":"1000"}})
    }

    #[test]
    fn rejects_mislabeled_opus_and_invalid_media() {
        assert!(validate(&fixture("mp3", "opus", None), Format::Mp3).is_err());
        assert!(validate(
            &fixture("mov,mp4,m4a,3gp,3g2,mj2", "opus", None),
            Format::M4a
        )
        .is_err());
        assert!(validate(
            &fixture("mov,mp4,m4a,3gp,3g2,mj2", "opus", Some("vp9")),
            Format::Mp4
        )
        .is_err());
        assert!(validate(&fixture("mp3", "mp3", None), Format::Mp3).is_ok());
        assert!(validate(
            &fixture("mov,mp4,m4a,3gp,3g2,mj2", "aac", Some("h264")),
            Format::Mp4
        )
        .is_ok());
        for field in ["duration", "size"] {
            let mut empty = fixture("mp3", "mp3", None);
            empty["format"][field] = json!("NaN");
            assert!(validate(&empty, Format::Mp3).is_err());
        }
    }
    #[test]
    fn compatible_mp4_rejects_ten_bit_h264() {
        let mut info = fixture("mov,mp4,m4a,3gp,3g2,mj2", "aac", Some("h264"));
        info["streams"][1]["pix_fmt"] = json!("yuv420p10le");
        assert!(validate(&info, Format::Mp4).is_err());
        assert!(validate(&info, Format::Source).is_ok());
    }
    #[test]
    fn audio_cover_art_is_not_a_video_track() {
        let mut info = fixture("mp3", "mp3", Some("mjpeg"));
        info["streams"][1]["disposition"] = json!({"attached_pic":1});
        assert!(validate(&info, Format::Mp3).is_ok());
    }

    #[test]
    fn original_profiles_validate_real_tracks_without_forcing_lossy_conversion() {
        assert!(validate(
            &fixture("matroska,webm", "opus", Some("vp9")),
            Format::Source
        )
        .is_ok());
        assert!(validate(&fixture("matroska,webm", "opus", Some("vp9")), Format::Mp4).is_err());
        assert!(validate(&fixture("ogg", "opus", None), Format::AudioOriginal).is_ok());
        assert!(validate(
            &fixture("mov,mp4,m4a,3gp,3g2,mj2", "aac", None),
            Format::AudioOriginal
        )
        .is_ok());
        assert!(validate(&fixture("ogg", "opus", None), Format::Mp3).is_err());
        assert!(validate(
            &fixture("matroska,webm", "opus", Some("vp9")),
            Format::AudioOriginal
        )
        .is_err());
    }

    #[tokio::test]
    #[ignore = "Requires managed FFmpeg/ffprobe; embeds synthetic local captions and verifies MP4 conversion without network"]
    async fn real_engines_subtitles_survive_copy_and_compatible_video_conversion() {
        use std::fs;
        let root = std::env::var_os("NABNAB_TEST_ENGINES")
            .or_else(|| std::env::var_os("YTNABBER_TEST_ENGINES"))
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("engines"));
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("nabnab-subtitles-{}-{nonce}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let original = directory.join("Original ñ.mp4");
        let subtitles = directory.join("Subtítulos es.srt");
        let english_subtitles = directory.join("English cache.srt");
        fs::write(&subtitles,"1\n00:00:00,500 --> 00:00:01,500\nUna prueba de subtítulos ñ\n\n2\n00:00:02,000 --> 00:00:03,000\nContenido local de prueba\n").unwrap();
        fs::write(
            &english_subtitles,
            "1\n00:00:00,500 --> 00:00:01,500\nA synthetic English cache caption\n",
        )
        .unwrap();
        let (_sender, cancel) = watch::channel(false);
        let mut args: Vec<String> = [
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-n",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x120:rate=12:duration=4",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=4",
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-c:v",
            "libopenh264",
            "-b:v",
            "500k",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "128k",
            "-shortest",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        args.push(original.to_string_lossy().into_owned());
        crate::engine::tool(&root, "ffmpeg.exe", args, cancel.clone())
            .await
            .unwrap();
        let original_bytes = fs::read(&original).unwrap();
        for (name, video, audio, subtitle) in [
            ("Con subtítulos.mp4", "copy", "copy", "mov_text"),
            (
                "Original con subtítulos.mkv",
                "libvpx-vp9",
                "libopus",
                "srt",
            ),
        ] {
            let input = directory.join(name);
            let mut args: Vec<String> =
                ["-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-i"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect();
            args.push(original.to_string_lossy().into_owned());
            args.push("-i".into());
            args.push(subtitles.to_string_lossy().into_owned());
            args.extend(
                [
                    "-map",
                    "0:v:0",
                    "-map",
                    "0:a:0",
                    "-map",
                    "1:0",
                    "-c:v",
                    video,
                    "-c:a",
                    audio,
                    "-c:s",
                    subtitle,
                    "-metadata:s:s:0",
                    "language=spa",
                ]
                .into_iter()
                .map(str::to_owned),
            );
            if video != "copy" {
                args.extend(
                    ["-b:v", "400k", "-b:a", "96k"]
                        .into_iter()
                        .map(str::to_owned),
                );
            }
            args.push(input.to_string_lossy().into_owned());
            crate::engine::tool(&root, "ffmpeg.exe", args, cancel.clone())
                .await
                .unwrap();
            let input_bytes = fs::read(&input).unwrap();
            if video != "copy" {
                let cached = input.with_file_name(format!(
                    "{}.compatible.mp4",
                    input.file_stem().unwrap().to_string_lossy()
                ));
                fs::copy(&original, &cached).unwrap();
                let cached_info = probe(&root, &cached, cancel.clone()).await.unwrap();
                validate(&cached_info, Format::Mp4).unwrap();
                assert_eq!(subtitle_count(&cached_info), 0);
            }
            let output = prepare(&root, &input, Format::Mp4, cancel.clone())
                .await
                .unwrap();
            let info = probe(&root, &output, cancel.clone()).await.unwrap();
            validate(&info, Format::Mp4).unwrap();
            validate_subtitle(&info, "es").unwrap();
            assert!(validate_subtitle(&info, "en").is_err());
            let tracks = info["streams"].as_array().unwrap();
            let caption = tracks
                .iter()
                .find(|s| s["codec_type"] == "subtitle")
                .unwrap();
            assert_eq!(caption["codec_name"], "mov_text");
            assert_eq!(caption["tags"]["language"], "spa");
            assert_eq!(subtitle_count(&info), 1);
            if video == "copy" {
                assert_eq!(output, input);
            } else {
                assert_ne!(output, input);
                assert!(tracks
                    .iter()
                    .any(|s| s["codec_type"] == "video" && s["codec_name"] == "h264"));
                assert!(tracks
                    .iter()
                    .any(|s| s["codec_type"] == "audio" && s["codec_name"] == "aac"));
            }
            let mut extract: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-i"]
                .into_iter()
                .map(str::to_owned)
                .collect();
            extract.push(output.to_string_lossy().into_owned());
            extract.extend(
                ["-map", "0:s:0", "-f", "srt", "pipe:1"]
                    .into_iter()
                    .map(str::to_owned),
            );
            let text = crate::engine::run_process(
                root.join("ffmpeg.exe"),
                extract.clone(),
                cancel.clone(),
                0,
                true,
                |_| {},
            )
            .await
            .unwrap();
            assert!(text.contains("Una prueba de subtítulos ñ"));
            assert!(text.contains("Contenido local de prueba"));
            let completed_bytes = fs::read(&output).unwrap();
            assert_eq!(
                prepare(&root, &input, Format::Mp4, cancel.clone())
                    .await
                    .unwrap(),
                output
            );
            assert_eq!(fs::read(&output).unwrap(), completed_bytes);
            if video != "copy" {
                fs::remove_file(&output).unwrap();
                let mut wrong: Vec<String> =
                    ["-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-i"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect();
                wrong.push(original.to_string_lossy().into_owned());
                wrong.push("-i".into());
                wrong.push(english_subtitles.to_string_lossy().into_owned());
                wrong.extend(
                    [
                        "-map",
                        "0:v:0",
                        "-map",
                        "0:a:0",
                        "-map",
                        "1:0",
                        "-c:v",
                        "copy",
                        "-c:a",
                        "copy",
                        "-c:s",
                        "mov_text",
                        "-metadata:s:s:0",
                        "language=eng",
                    ]
                    .into_iter()
                    .map(str::to_owned),
                );
                wrong.push(output.to_string_lossy().into_owned());
                crate::engine::tool(&root, "ffmpeg.exe", wrong, cancel.clone())
                    .await
                    .unwrap();
                let wrong_info = probe(&root, &output, cancel.clone()).await.unwrap();
                validate(&wrong_info, Format::Mp4).unwrap();
                assert_eq!(subtitle_count(&wrong_info), 1);
                assert!(validate_subtitle(&wrong_info, "en").is_ok());
                assert!(validate_subtitle(&wrong_info, "es").is_err());
                assert_eq!(
                    prepare(&root, &input, Format::Mp4, cancel.clone())
                        .await
                        .unwrap(),
                    output
                );
                let repaired = probe(&root, &output, cancel.clone()).await.unwrap();
                validate_subtitle(&repaired, "es").unwrap();
                assert!(validate_subtitle(&repaired, "en").is_err());
                let text = crate::engine::run_process(
                    root.join("ffmpeg.exe"),
                    extract,
                    cancel.clone(),
                    0,
                    true,
                    |_| {},
                )
                .await
                .unwrap();
                assert!(text.contains("Una prueba de subtítulos ñ"));
                assert!(!text.contains("A synthetic English cache caption"));
            }
            assert_eq!(fs::read(&input).unwrap(), input_bytes);
            assert_eq!(fs::read(&original).unwrap(), original_bytes);
        }
        assert_eq!(directory.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    #[ignore = "Requires bundled Windows FFprobe; uses a loopback request tripwire, no Internet"]
    async fn real_engines_probe_rejects_manifests_disguised_as_local_media() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let contacted = Arc::new(AtomicBool::new(false));
        let signal = contacted.clone();
        let tripwire = tokio::spawn(async move {
            while let Ok((mut connection, _)) = listener.accept().await {
                signal.store(true, Ordering::SeqCst);
                let _ = connection
                    .write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
            }
        });
        let engines = Path::new(env!("CARGO_MANIFEST_DIR")).join("engines");
        let directory = std::env::temp_dir().join(format!(
            "nabnab-local-probe-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let fixtures = [
            format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nhttp://{address}/segment.ts\n#EXT-X-ENDLIST\n"),
            format!("<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"PT1S\"><Period><AdaptationSet mimeType=\"video/mp4\"><Representation id=\"1\" bandwidth=\"1000\"><BaseURL>http://{address}/segment.mp4</BaseURL></Representation></AdaptationSet></Period></MPD>"),
            format!("ffconcat version 1.0\nfile 'http://{address}/segment.mp4'\n"),
        ];
        for (index, content) in fixtures.iter().enumerate() {
            let input = directory.join(format!("manifest-{index}.mp4"));
            std::fs::write(&input, content).unwrap();
            let (sender, cancel) = watch::channel(false);
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                probe(&engines, &input, cancel),
            )
            .await;
            let _ = sender.send(true);
            assert!(
                matches!(result, Ok(Err(_))),
                "A disguised manifest must be rejected promptly"
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        tripwire.abort();
        assert!(
            !contacted.load(Ordering::SeqCst),
            "FFprobe must not request any manifest resource"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
