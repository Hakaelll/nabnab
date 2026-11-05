use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub request_id: u64,
    pub title: String,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub uploader: Option<String>,
    pub heights: Vec<u32>,
    pub has_audio: bool,
    pub has_video: bool,
    #[serde(default)]
    pub entries: Vec<MediaEntry>,
    #[serde(default)]
    pub capabilities: crate::providers::Capabilities,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaEntry {
    pub id: String,
    pub title: String,
    pub url: String,
    pub thumbnail: Option<String>,
    pub duration: Option<f64>,
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Mp4,
    Mp3,
    M4a,
    Opus,
    Source,
    #[serde(rename = "audio")]
    AudioOriginal,
}

impl Format {
    pub fn is_video(self) -> bool {
        matches!(self, Self::Mp4 | Self::Source)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DownloadKey {
    pub url: String,
    pub folder: std::path::PathBuf,
    pub format: Format,
    pub height: Option<u32>,
    pub audio_bitrate: u16,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub request_id: u64,
    pub phase: String,
    pub percent: Option<f64>,
    pub speed: Option<f64>,
    pub downloaded: Option<f64>,
    pub total: Option<f64>,
}

pub fn validate_url(input: &str) -> Result<String, String> {
    let parsed = url::Url::parse(input.trim())
        .map_err(|_| "Paste a complete http:// or https:// video URL.")?;
    if !matches!(parsed.scheme(), "https" | "http")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("Use a public HTTP or HTTPS video URL without credentials.".into());
    }
    let host = parsed.host_str().unwrap_or_default();
    let local_error = "Local or reserved network URLs are not supported.";
    match parsed.host() {
        Some(url::Host::Ipv4(ip)) if !crate::providers::is_public_ip(ip.into()) => {
            return Err(local_error.into());
        }
        Some(url::Host::Ipv6(ip)) if !crate::providers::is_public_ip(ip.into()) => {
            return Err(local_error.into());
        }
        Some(url::Host::Domain(_)) => {
            let host = host.trim_end_matches('.');
            if !host.contains('.')
                || [
                    "localhost",
                    "local",
                    "internal",
                    "lan",
                    "home",
                    "invalid",
                    "test",
                    "example",
                ]
                .iter()
                .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
            {
                return Err(local_error.into());
            }
        }
        _ => {}
    }
    Ok(parsed.to_string())
}

fn thumbnail(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| s.starts_with("https://"))
        .and_then(|s| validate_url(s).ok())
}

fn duration(value: &Value) -> Option<f64> {
    value.as_f64().filter(|v| v.is_finite() && *v > 0.0)
}

pub fn parse_media(raw: &str, request_id: u64) -> Result<Media, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| "The source returned unreadable video information.")?;
    if value["is_live"] == true || value["live_status"] == "is_upcoming" {
        return Err("Choose a finished video. Live and upcoming streams are not supported.".into());
    }
    if value["has_drm"] == true {
        return Err("Protected media cannot be downloaded by nabnab.".into());
    }
    let entries: Vec<MediaEntry> = value["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .take(200)
        .filter(|entry| {
            entry["has_drm"] != true
                && entry["is_live"] != true
                && entry["live_status"] != "is_upcoming"
        })
        .filter_map(|entry| {
            let url = ["webpage_url", "original_url", "url"]
                .into_iter()
                .filter_map(|field| entry[field].as_str())
                .find_map(|url| validate_url(url).ok())?;
            Some(MediaEntry {
                id: entry["id"]
                    .as_str()
                    .unwrap_or_default()
                    .chars()
                    .take(256)
                    .collect(),
                title: entry["title"]
                    .as_str()
                    .unwrap_or("Untitled item")
                    .chars()
                    .take(1024)
                    .collect(),
                url,
                thumbnail: thumbnail(&entry["thumbnail"]).or_else(|| {
                    entry["thumbnails"].as_array().and_then(|images| {
                        images
                            .iter()
                            .rev()
                            .find_map(|image| thumbnail(&image["url"]))
                    })
                }),
                duration: duration(&entry["duration"]),
            })
        })
        .collect();
    let batch = value.get("entries").is_some() || value["_type"] == "playlist";
    if batch && entries.is_empty() {
        return Err("This list has no supported public items. Private, live and protected entries are excluded.".into());
    }
    let empty = Vec::new();
    let formats = value["formats"].as_array().unwrap_or(&empty);
    let mut heights = BTreeSet::new();
    let mut has_audio = false;
    let mut has_video = false;
    for f in formats {
        if f["has_drm"] == true {
            continue;
        }
        let video = f["vcodec"].as_str().map_or_else(
            || f["video_ext"].as_str().is_some_and(|ext| ext != "none"),
            |codec| codec != "none",
        );
        // Direct media links can omit codecs; let the engine probe their audio.
        let audio = f["acodec"].as_str().map_or_else(
            || video || f["audio_ext"].as_str().is_some_and(|ext| ext != "none"),
            |codec| codec != "none",
        );
        has_video |= video;
        has_audio |= audio;
        if video {
            if let Some(height) = f["height"].as_u64().filter(|h| *h > 0 && *h <= 16384) {
                heights.insert(height as u32);
            }
        }
    }
    if !batch && !has_audio && !has_video {
        return Err("This source has no supported public formats.".into());
    }
    Ok(Media {
        request_id,
        title: value["title"]
            .as_str()
            .unwrap_or("Untitled video")
            .chars()
            .take(1024)
            .collect(),
        duration: duration(&value["duration"]),
        thumbnail: thumbnail(&value["thumbnail"]),
        uploader: value["uploader"]
            .as_str()
            .map(|s| s.chars().take(512).collect()),
        heights: heights.into_iter().rev().collect(),
        has_audio,
        has_video,
        capabilities: crate::providers::Capabilities {
            video: has_video,
            audio: has_audio,
            batches: batch,
            subtitles: value["subtitles"]
                .as_object()
                .is_some_and(|s| !s.is_empty()),
            images: false,
            verified: false,
        },
        entries,
    })
}

pub fn format_args(format: Format, height: Option<u32>) -> Vec<String> {
    match format {
        Format::Mp4 | Format::Source => {
            let filter = height.map(|h| format!("[height={h}]")).unwrap_or_default();
            let selection = if format == Format::Mp4 {
                format!("bv{filter}[vcodec^=avc1]+ba[acodec^=mp4a]/b{filter}[vcodec^=avc1][acodec^=mp4a]/bv{filter}+ba/b{filter}/bv{filter}")
            } else {
                format!("bv{filter}+ba/b{filter}/bv{filter}")
            };
            let mut args = vec![
                "-f".into(),
                selection,
                "--merge-output-format".into(),
                if format == Format::Mp4 { "mp4" } else { "mkv" }.into(),
            ];
            if format == Format::Mp4 {
                args.extend(["--remux-video".into(), "mp4".into()]);
            }
            args
        }
        Format::Mp3 => vec![
            "-f",
            "bestaudio/best",
            "-x",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "192K",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        Format::M4a => vec![
            "-f",
            "bestaudio[acodec^=mp4a]/bestaudio[acodec=aac]/bestaudio/best",
            "-x",
            "--audio-format",
            "m4a",
            "--audio-quality",
            "0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        Format::Opus => [
            "-f",
            "bestaudio[acodec=opus]/bestaudio/best",
            "-x",
            "--audio-format",
            "opus",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        Format::AudioOriginal => ["-f", "bestaudio/best", "-x", "--audio-format", "best"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    }
}

pub fn parse_progress(line: &str, request_id: u64) -> Option<Progress> {
    let raw = line.strip_prefix("__PROGRESS__")?;
    let p: Value = serde_json::from_str(raw).ok()?;
    let downloaded = p["downloaded_bytes"].as_f64();
    let total = p["total_bytes"]
        .as_f64()
        .or_else(|| p["total_bytes_estimate"].as_f64());
    let percent = downloaded
        .zip(total)
        .filter(|(_, t)| *t > 0.0)
        .map(|(d, t)| (100.0 * d / t).clamp(0.0, 100.0));
    Some(Progress {
        request_id,
        phase: "downloading".into(),
        percent,
        speed: p["speed"].as_f64(),
        downloaded,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_boundary() {
        assert!(validate_url("https://www.youtube.com/watch?v=abc&list=x").is_ok());
        for u in [
            "file:///secret",
            "--exec=bad",
            "https://user:pass@host.com",
            "http://localhost/x",
            "http://192.168.1.1/a",
            "http://[::1]/a",
            "http://[::ffff:127.0.0.1]/x",
            "http://2130706433/x",
            "http://0x7f000001/x",
            "http://0177.0.0.1/x",
            "http://100.64.0.1/x",
            "http://224.0.0.1/x",
            "http://server/x",
            "https://files.internal/x",
            "https://printer.local./x",
        ] {
            assert!(validate_url(u).is_err(), "{u}");
        }
    }
    #[test]
    fn metadata_only_lists_real_non_drm_heights() {
        let m = parse_media(r#"{"title":"Test","formats":[{"vcodec":"avc1","height":720,"acodec":"none"},{"vcodec":"vp9","height":720},{"vcodec":"av1","height":2160,"has_drm":true},{"vcodec":"none","acodec":"aac"}]}"#, 7).unwrap();
        assert_eq!(m.heights, vec![720]);
        assert!(m.has_audio && m.has_video);
        assert!(parse_media(r#"{"entries":[]}"#, 1).is_err());
        assert!(parse_media(r#"{"is_live":true}"#, 1).is_err());
    }
    #[test]
    fn direct_media_without_codec_metadata_remains_usable() {
        let media = parse_media(r#"{"title":"Direct file","formats":[{"vcodec":null,"ext":"mp4","video_ext":"mp4","audio_ext":"none"}]}"#, 1).unwrap();
        assert!(media.has_video && media.has_audio);
        assert!(media.heights.is_empty());
        assert_eq!(media.duration, None);
    }
    #[test]
    fn progress_handles_unknown_size_and_clamps() {
        let p = parse_progress(
            r#"__PROGRESS__{"downloaded_bytes":50,"total_bytes":100,"speed":24}"#,
            2,
        )
        .unwrap();
        assert_eq!(p.percent, Some(50.0));
        assert_eq!(p.speed, Some(24.0));
        assert_eq!(
            parse_progress(r#"__PROGRESS__{"downloaded_bytes":50}"#, 1)
                .unwrap()
                .percent,
            None
        );
        assert_eq!(
            parse_progress(
                r#"__PROGRESS__{"downloaded_bytes":120,"total_bytes":100}"#,
                1
            )
            .unwrap()
            .percent,
            Some(100.0)
        );
        assert!(parse_progress("bad", 1).is_none());
    }
    #[test]
    fn quality_is_exact_and_audio_has_no_resolution() {
        assert!(format_args(Format::Mp4, Some(720))[1].contains("[height=720]"));
        assert!(!format_args(Format::M4a, None).join(" ").contains("height"));
        assert!(format_args(Format::Mp3, None).contains(&"mp3".to_owned()));
    }
    #[test]
    fn compatible_profiles_prefer_real_codecs_and_default_to_192k() {
        assert!(format_args(Format::Mp4, Some(1080))[1].contains("[vcodec^=avc1]"));
        assert!(format_args(Format::M4a, None)[1].contains("[acodec^=mp4a]"));
        assert!(format_args(Format::Mp3, None).contains(&"192K".to_owned()));
    }
    #[test]
    fn original_profiles_keep_codecs_and_do_not_force_mp4_or_opus() {
        let source = format_args(Format::Source, None);
        assert!(source.contains(&"mkv".into()));
        assert!(!source.contains(&"--remux-video".into()));
        assert!(!source.contains(&"mp4".into()));
        let audio = format_args(Format::AudioOriginal, None);
        assert_eq!(
            audio,
            vec!["-f", "bestaudio/best", "-x", "--audio-format", "best"]
        );
        assert_eq!(
            serde_json::to_string(&Format::AudioOriginal).unwrap(),
            "\"audio\""
        );
    }
    #[test]
    fn batch_metadata_excludes_private_live_and_protected_entries() {
        let media = parse_media(
            r#"{"_type":"playlist","title":"A list","entries":[
            {"id":"one","title":"First","url":"https://www.youtube.com/watch?v=one","duration":12},
            {"id":"two","title":"Bad","url":"file:///C:/secret"},
            {"id":"three","url":"http://127.0.0.1/secret"},
            {"id":"four","url":"https://example.org/a","has_drm":true},
            {"id":"five","url":"https://example.org/b","is_live":true},
            {"id":"six","url":"an-id-is-not-a-url"}
        ]}"#,
            9,
        )
        .unwrap();
        assert_eq!(media.entries.len(), 1);
        assert_eq!(media.entries[0].id, "one");
        assert_eq!(media.entries[0].duration, Some(12.0));
        assert!(media.capabilities.batches);
        assert!(!media.has_video && !media.has_audio);
        assert!(media.heights.is_empty());
    }
    #[test]
    fn batch_metadata_is_bounded_and_single_media_reports_observed_capabilities() {
        let entries: Vec<Value> = (0..250).map(|n| serde_json::json!({"id":n.to_string(),"url":format!("https://example.org/{n}")})).collect();
        let raw = serde_json::json!({"entries":entries}).to_string();
        assert_eq!(parse_media(&raw, 1).unwrap().entries.len(), 200);
        let media = parse_media(r#"{"formats":[{"vcodec":"none","acodec":"opus"}],"subtitles":{"en":[{"ext":"vtt"}]},"thumbnail":"https://127.0.0.1/secret"}"#, 2).unwrap();
        assert!(media.capabilities.audio && media.capabilities.subtitles);
        assert!(!media.capabilities.video && !media.capabilities.batches);
        assert_eq!(media.thumbnail, None);
        assert!(parse_media(r#"{"has_drm":true,"formats":[{"vcodec":"h264"}]}"#, 3).is_err());
        let persisted = serde_json::to_string(&media).unwrap();
        assert!(serde_json::from_str::<Media>(&persisted).is_ok());
    }
    #[test]
    fn recovery_keys_separate_sources_formats_destinations_and_qualities() {
        let key = DownloadKey {
            url: "https://example.org/a".into(),
            folder: "C:/Downloads".into(),
            format: Format::Mp3,
            height: None,
            audio_bitrate: 192,
        };
        let mut other = key.clone();
        assert!(key == other);
        other.audio_bitrate = 320;
        assert!(key != other);
        other = key.clone();
        other.url.push('b');
        assert!(key != other);
        other = key.clone();
        other.format = Format::M4a;
        assert!(key != other);
        other = key.clone();
        other.folder.push("other");
        assert!(key != other);
        other = key.clone();
        other.height = Some(720);
        assert!(key != other);
        let args = crate::engine::download_args(
            std::path::Path::new("C:/Tools with spaces"),
            Format::Mp3,
            None,
            std::path::Path::new("C:/Downloads"),
            &key.url,
        );
        assert!(args.contains(&"--continue".into()));
        assert!(!args.contains(&"--no-continue".into()));
    }
}
