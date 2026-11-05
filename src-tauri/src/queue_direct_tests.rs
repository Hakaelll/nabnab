use super::*;
use serde_json::json;
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "nabnab-direct-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

#[tokio::test]
async fn queued_direct_url_validation_allows_offline_receipt_recovery() {
    let root = directory("offline-queue-url");
    let bytes = b"synthetic owned input for receipt recovery";
    let url = "https://unresolvable-nabnab-fixture.example.org/media.mp4";
    let downloaded = root.join("direct-source.mp4");
    fs::write(&downloaded, bytes).unwrap();
    fs::write(
        root.join(".direct-source.completed.json"),
        serde_json::to_vec(&json!({
            "urlHash":format!("{:x}", Sha256::digest(url.as_bytes())),
            "filename":"direct-source.mp4",
            "bytes":bytes.len(),
            "sha256":format!("{:x}", Sha256::digest(bytes)),
        }))
        .unwrap(),
    )
    .unwrap();
    let (_sender, cancel) = watch::channel(false);
    // run uses this queue boundary, which must not resolve a host before checking the local receipt.
    let checked = validate_download_url(url).await.unwrap();
    assert_eq!(checked, url);
    let recovered = crate::direct::download(&checked, &root, cancel.clone(), 2, |_| {})
        .await
        .unwrap();
    assert_eq!(recovered, downloaded);
    assert_eq!(fs::read(&recovered).unwrap(), bytes);
    fs::write(&recovered, b"tampered owned input").unwrap();
    assert!(crate::direct::download(&checked, &root, cancel, 3, |_| {})
        .await
        .unwrap_err()
        .contains("changed or is incomplete"));
    assert!(validate_download_url("file:///secret.mp4").await.is_err());
    assert!(validate_download_url("http://127.0.0.1/media.mp4")
        .await
        .is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_names_follow_observed_containers_and_tracks_instead_of_http_labels() {
    let video = json!({"codec_type":"video", "codec_name":"h264"});
    let audio = json!({"codec_type":"audio", "codec_name":"aac"});
    assert_eq!(direct_extension(&json!({"streams":[video.clone(), audio.clone()], "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2"}})).unwrap(), "mp4");
    assert_eq!(
        direct_extension(
            &json!({"streams":[audio.clone()], "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2"}})
        )
        .unwrap(),
        "m4a"
    );
    assert_eq!(direct_extension(&json!({"streams":[video, audio], "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2", "tags":{"major_brand":"qt  "}}})).unwrap(), "mov");
    assert_eq!(direct_extension(&json!({"streams":[{"codec_type":"audio", "codec_name":"opus"},{"codec_type":"video", "codec_name":"mjpeg", "disposition":{"attached_pic":1}}], "format":{"format_name":"ogg"}})).unwrap(), "opus");
    assert_eq!(direct_extension(&json!({"streams":[{"codec_type":"video", "codec_name":"vp9"}], "format":{"format_name":"matroska,webm"}})).unwrap(), "mkv");
    assert!(direct_extension(&json!({"streams":[], "format":{"format_name":"mov,mp4"}})).is_err());
    assert!(direct_extension(&json!({"streams":[{"codec_type":"video", "codec_name":"h264"}], "format":{"format_name":"hls"}})).is_err());
}

#[tokio::test]
async fn direct_alias_preserves_the_receipt_input_and_archives_partial_aliases() {
    let root = directory("aliases");
    let staging = root.join(".nabnab-123-456");
    fs::create_dir(&staging).unwrap();
    let input = staging.join("direct-source.mp3");
    fs::write(&input, b"complete immutable input fixture").unwrap();
    let info = json!({"streams":[{"codec_type":"video", "codec_name":"h264"}],"format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2"}});
    let (_sender, cancel) = watch::channel(false);
    let alias = normalized_direct_input(&input, &staging, &info, 1, cancel.clone())
        .await
        .unwrap();
    assert_eq!(alias.extension().unwrap(), "mp4");
    assert_eq!(alias.file_name().unwrap(), "HTTP media [123-456].mp4");
    assert_eq!(
        fs::read(&alias).unwrap(),
        b"complete immutable input fixture"
    );
    assert_eq!(
        fs::read(&input).unwrap(),
        b"complete immutable input fixture"
    );
    assert_eq!(
        normalized_direct_input(&input, &staging, &info, 2, cancel.clone())
            .await
            .unwrap(),
        alias
    );
    // Replace the alias entry, so this fixture models a failed copy without changing a hard-linked source.
    fs::remove_file(&alias).unwrap();
    fs::write(&alias, b"partial alias").unwrap();
    assert_eq!(
        normalized_direct_input(&input, &staging, &info, 3, cancel.clone())
            .await
            .unwrap(),
        alias
    );
    let archive = alias.with_file_name("HTTP media [123-456].mp4.interrupted-3");
    assert_eq!(fs::read(archive).unwrap(), b"partial alias");
    assert_eq!(
        fs::read(&alias).unwrap(),
        b"complete immutable input fixture"
    );
    assert_eq!(
        fs::read(&input).unwrap(),
        b"complete immutable input fixture"
    );
    let second_stage = root.join(".nabnab-123-457");
    fs::create_dir(&second_stage).unwrap();
    let second_input = second_stage.join("direct-source.mp3");
    fs::write(&second_input, b"second immutable input fixture").unwrap();
    let second_alias = normalized_direct_input(&second_input, &second_stage, &info, 4, cancel)
        .await
        .unwrap();
    assert_eq!(
        second_alias.file_name().unwrap(),
        "HTTP media [123-457].mp4"
    );
    let destination = root.join("published");
    fs::create_dir(&destination).unwrap();
    let first = crate::publish(&staging, &alias.to_string_lossy(), &destination).unwrap();
    let second =
        crate::publish(&second_stage, &second_alias.to_string_lossy(), &destination).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        fs::read(first).unwrap(),
        b"complete immutable input fixture"
    );
    assert_eq!(fs::read(second).unwrap(), b"second immutable input fixture");
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
#[ignore = "Requires bundled Windows engines; synthetic local HTTP fixture exercises genuine direct transport and every download profile"]
async fn real_engines_direct_http_profiles_publish_real_containers_without_losing_cached_input() {
    let engines = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engines");
    let root = directory("http-profiles");
    let original = root.join("Original ñ QuickTime.mov");
    let (_sender, cancel) = watch::channel(false);
    let args = [
        "-v",
        "error",
        "-nostdin",
        "-n",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=160x120:rate=12:duration=2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000:duration=2",
        "-map",
        "0:v:0",
        "-map",
        "1:a:0",
        "-c:v",
        "libopenh264",
        "-b:v",
        "400k",
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
    .chain([original.to_string_lossy().into_owned()])
    .collect();
    engine::tool(&engines, "ffmpeg.exe", args, cancel.clone())
        .await
        .unwrap();
    let original_bytes = fs::read(&original).unwrap();
    let info = media::probe(&engines, &original, cancel.clone())
        .await
        .unwrap();
    assert_eq!(
        info["format"]["tags"]["major_brand"]
            .as_str()
            .unwrap()
            .trim(),
        "qt"
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let content = Arc::new(original_bytes.clone());
    let server = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while request.len() < 16 * 1024 && !request.windows(4).any(|bytes| bytes == b"\r\n\r\n")
            {
                let count = stream.read(&mut chunk).await.unwrap_or(0);
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
            }
            observed.fetch_add(1, Ordering::SeqCst);
            // The header deliberately lies. Only the real local probe may choose publication format.
            let header = format!("HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: {}\r\nETag: \"synthetic-quicktime-v1\"\r\nConnection: close\r\n\r\n", content.len());
            let _ = stream.write_all(header.as_bytes()).await;
            let _ = stream.write_all(content.as_slice()).await;
            let _ = stream.shutdown().await;
        }
    });
    let url = format!(
        "http://fixture.invalid:{}/media.mp4?api_key=fictional-local-only",
        address.port()
    );
    let profiles = [
        (Format::Source, None, "mov", Some("h264"), "aac"),
        (Format::Mp4, None, "mp4", Some("h264"), "aac"),
        (Format::Mp3, Some(128), "mp3", None, "mp3"),
        (Format::M4a, None, "m4a", None, "aac"),
        (Format::Opus, None, "opus", None, "opus"),
        (Format::AudioOriginal, None, "m4a", None, "aac"),
    ];
    let destination = root.join("published");
    fs::create_dir(&destination).unwrap();
    let mut published_paths = std::collections::HashSet::new();
    for (index, (format, bitrate, extension, video, audio)) in profiles.into_iter().enumerate() {
        let stage = root.join(format!("stage-{index}"));
        fs::create_dir(&stage).unwrap();
        let raw = crate::direct::download_fixture(
            &url,
            &stage,
            cancel.clone(),
            index as u64 + 1,
            |_| {},
            address,
        )
        .await
        .unwrap();
        assert_eq!(raw.extension().unwrap(), "mp3");
        assert_eq!(fs::read(&raw).unwrap(), original_bytes);
        let prepared = prepare_direct_download(
            &engines,
            &raw,
            &stage,
            format,
            bitrate,
            cancel.clone(),
            index as u64 + 1,
            |_| {},
        )
        .await
        .unwrap();
        let info = media::probe(&engines, &prepared, cancel.clone())
            .await
            .unwrap();
        media::validate(&info, format).unwrap();
        assert_eq!(prepared.extension().unwrap(), extension);
        let streams = info["streams"].as_array().unwrap();
        let actual_video = streams.iter().find(|stream| {
            stream["codec_type"] == "video" && stream["disposition"]["attached_pic"] != 1
        });
        assert_eq!(
            actual_video.and_then(|stream| stream["codec_name"].as_str()),
            video
        );
        assert_eq!(
            streams
                .iter()
                .find(|stream| stream["codec_type"] == "audio")
                .unwrap()["codec_name"],
            audio
        );
        if format == Format::Mp4 {
            assert_ne!(
                info["format"]["tags"]["major_brand"]
                    .as_str()
                    .unwrap_or("")
                    .trim(),
                "qt"
            );
        }
        if format == Format::Mp3 {
            assert_eq!(
                streams
                    .iter()
                    .find(|stream| stream["codec_type"] == "audio")
                    .unwrap()["bit_rate"],
                "128000"
            );
        }
        let published = crate::publish(&stage, &prepared.to_string_lossy(), &destination).unwrap();
        assert!(published_paths.insert(published.clone()));
        assert_eq!(published.extension().unwrap(), extension);
        if format == Format::Source {
            assert_eq!(fs::read(&published).unwrap(), original_bytes);
        }
        let before_reuse = requests.load(Ordering::SeqCst);
        let reused = crate::direct::download_fixture(
            &url,
            &stage,
            cancel.clone(),
            index as u64 + 100,
            |_| {},
            address,
        )
        .await
        .unwrap();
        assert_eq!(reused, raw);
        assert_eq!(requests.load(Ordering::SeqCst), before_reuse);
        assert_eq!(fs::read(&raw).unwrap(), original_bytes);
        assert_eq!(fs::read(&original).unwrap(), original_bytes);
    }
    assert_eq!(requests.load(Ordering::SeqCst), profiles.len());
    assert_eq!(fs::read_dir(destination).unwrap().count(), 6);
    server.abort();
    fs::remove_dir_all(root).unwrap();
}
