use crate::{engine, failure::Failure, model, providers};
use serde_json::json;
use std::{fs, path::PathBuf};
use tokio::sync::watch;

#[tokio::test]
#[ignore = "Reads metadata for two explicitly CC BY 3.0 Blender samples over the Internet; no cookies or media download"]
async fn open_licensed_platform_metadata() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("engines");
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../artifacts/platforms");
    fs::create_dir_all(&output).unwrap();
    let mut reports = vec![];
    for (index, (platform, url)) in [
        ("YouTube", "https://www.youtube.com/watch?v=aqz-KE-bpKQ"),
        ("Vimeo", "https://vimeo.com/1084537"),
    ]
    .into_iter()
    .enumerate()
    {
        let (_sender, cancel) = watch::channel(false);
        let id = index as u64 + 1;
        let result = async {
            let url = providers::validate_remote(url).await?;
            let mut args = engine::inspect_args(&root, &url);
            let delimiter = args
                .iter()
                .position(|arg| arg == "--")
                .ok_or("Missing URL boundary.")?;
            args.splice(
                delimiter..delimiter,
                [
                    "--retries".into(),
                    "0".into(),
                    "--extractor-retries".into(),
                    "0".into(),
                ],
            );
            let raw = engine::run_process(root.join("yt-dlp.exe"), args, cancel, id, true, |_| {})
                .await?;
            model::parse_media(&raw, id)
        }
        .await;
        let report = match result {
            Ok(media) => {
                let expected = media.has_video
                    && media.has_audio
                    && media.duration.is_some_and(|value| value > 500.0)
                    && media.title.to_ascii_lowercase().contains("bunny");
                json!({"platform":platform,"url":url,"status":if expected{"metadata_verified"}else{"unexpected_metadata"},"title":media.title,"duration":media.duration,"heights":media.heights,"hasVideo":media.has_video,"hasAudio":media.has_audio,"mediaDownloaded":false,"sessionsUsed":false})
            }
            Err(raw) => {
                let failure = Failure::classify(&raw, "metadata");
                json!({"platform":platform,"url":url,"status":"blocked","code":failure.code,"message":failure.message,"technical":failure.technical,"mediaDownloaded":false,"sessionsUsed":false})
            }
        };
        println!("{}: {}", platform, report["status"]);
        reports.push(report);
    }
    let version = fs::read_to_string(root.join("versions.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok());
    fs::write(output.join("open-license-metadata.json"),serde_json::to_vec_pretty(&json!({"testedAtUnixMs":crate::queue::now(),"license":"CC BY 3.0","attribution":"Blender Foundation / www.bigbuckbunny.org","licenseSource":"https://peach.blender.org/about/","engines":version,"results":reports})).unwrap()).unwrap();
    assert!(reports.iter().all(|item|item["status"]=="metadata_verified"),"Some platforms could not be certified. See artifacts/platforms/open-license-metadata.json; no access controls were bypassed.");
}
