use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{AppHandle, Manager};

#[derive(Default, Deserialize, Serialize)]
struct Active {
    current: String,
    previous: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    current: String,
    latest: String,
    available: bool,
    can_rollback: bool,
}

fn valid_version(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|c| c.is_ascii_digit()))
}
fn active(data: &Path) -> Active {
    fs::read(data.join("active.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
fn version_path(data: &Path, bundled: &Path, version: &str) -> Option<PathBuf> {
    if version == "bundled" {
        Some(bundled.join("yt-dlp.exe"))
    } else if valid_version(version) {
        Some(data.join(version).join("yt-dlp.exe"))
    } else {
        None
    }
}
pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(root) = crate::test_data_dir() {
        return Ok(root.join("engine-updates"));
    }
    app.path()
        .app_local_data_dir()
        .map(|p| p.join("engine-updates"))
        .map_err(|e| e.to_string())
}
pub fn executable(app: &AppHandle, bundled: &Path) -> Result<PathBuf, String> {
    Ok(selected(&data_dir(app)?, bundled))
}
fn selected(data: &Path, bundled: &Path) -> PathBuf {
    let state = active(data);
    version_path(data, bundled, &state.current)
        .filter(|p| p.is_file())
        .or_else(|| version_path(data, bundled, &state.previous).filter(|p| p.is_file()))
        .unwrap_or_else(|| bundled.join("yt-dlp.exe"))
}
fn save(data: &Path, state: &Active) -> Result<(), String> {
    let temporary = data.join("active.new");
    let mut file = fs::File::create(&temporary).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec(state).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = data
            .join("active.json")
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                0x1 | 0x8,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    #[cfg(not(windows))]
    fs::rename(temporary, data.join("active.json")).map_err(|e| e.to_string())?;
    Ok(())
}
fn checksum(sums: &str) -> Result<String, String> {
    sums.lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            let hash = words.next()?;
            let filename = words.next()?.trim_start_matches('*');
            (filename == "yt-dlp.exe"
                && hash.len() == 64
                && hash.bytes().all(|c| c.is_ascii_hexdigit()))
            .then(|| hash.to_lowercase())
        })
        .ok_or_else(|| "The official release has no valid Windows checksum.".into())
}
fn verify(bytes: &[u8], expected: &str) -> Result<(), String> {
    if !bytes.starts_with(b"MZ") || format!("{:x}", Sha256::digest(bytes)) != expected {
        return Err(
            "Engine integrity verification failed. The previous engine is unchanged.".into(),
        );
    }
    Ok(())
}
async fn get(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > limit {
            return Err("The engine release exceeds the expected size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub async fn update(data: &Path, bundled: &Path, mode: &str) -> Result<Status, String> {
    if !matches!(mode, "check" | "install" | "rollback") {
        return Err("Unknown engine update action.".into());
    }
    fs::create_dir_all(data).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(data.join("update.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|_| "Another engine update is running.")?;
    let mut state = active(data);
    let current = selected(data, bundled);
    let current_is_bundled = current == bundled.join("yt-dlp.exe");
    let (_sender, cancel) = tokio::sync::watch::channel(false);
    let installed = crate::engine::run_process(
        current,
        vec!["--version".into()],
        cancel.clone(),
        0,
        true,
        |_| {},
    )
    .await?
    .trim()
    .to_owned();
    if !valid_version(&installed) {
        return Err("The installed engine returned an invalid version.".into());
    }
    if mode == "rollback" {
        let previous = version_path(data, bundled, &state.previous)
            .filter(|p| p.is_file())
            .ok_or("No previous engine is available.")?;
        let restored =
            crate::engine::run_process(previous, vec!["--version".into()], cancel, 0, true, |_| {})
                .await?;
        std::mem::swap(&mut state.current, &mut state.previous);
        save(data, &state)?;
        return Ok(Status {
            current: restored.trim().into(),
            latest: String::new(),
            available: false,
            can_rollback: true,
        });
    }
    let client = reqwest::Client::builder()
        .user_agent("YTNabber-engine-updater")
        .https_only(true)
        .timeout(Duration::from_secs(180))
        .connect_timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() >= 5
                || url.scheme() != "https"
                || !matches!(
                    url.host_str(),
                    Some(
                        "github.com"
                            | "api.github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                attempt.error("Unexpected engine release redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())?;
    let release: serde_json::Value = serde_json::from_slice(
        &get(
            &client,
            "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest",
            1024 * 1024,
        )
        .await?,
    )
    .map_err(|e| e.to_string())?;
    let latest = release["tag_name"]
        .as_str()
        .filter(|v| valid_version(v))
        .ok_or("Invalid official engine release version.")?
        .to_owned();
    if release["prerelease"] != false || release["draft"] != false {
        return Err("The engine release is not a stable public release.".into());
    }
    let numbers = |v: &str| {
        v.split('.')
            .map(|p| p.parse::<u32>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let available = numbers(&latest) > numbers(&installed);
    if mode == "install" && available {
        let base = format!("https://github.com/yt-dlp/yt-dlp/releases/download/{latest}");
        let sums = get(&client, &format!("{base}/SHA2-256SUMS"), 256 * 1024).await?;
        let expected = checksum(std::str::from_utf8(&sums).map_err(|e| e.to_string())?)?;
        let bytes = get(&client, &format!("{base}/yt-dlp.exe"), 64 * 1024 * 1024).await?;
        verify(&bytes, &expected)?;
        let directory = data.join(&latest);
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let candidate = directory.join("yt-dlp.exe");
        if candidate.exists() {
            verify(&fs::read(&candidate).map_err(|e| e.to_string())?, &expected)?;
        } else {
            let temporary = directory.join("download.partial");
            fs::write(&temporary, &bytes).map_err(|e| e.to_string())?;
            fs::rename(&temporary, &candidate).map_err(|e| e.to_string())?;
        }
        let actual = crate::engine::run_process(
            candidate,
            vec!["--version".into()],
            cancel,
            0,
            true,
            |_| {},
        )
        .await?;
        if actual.trim() != latest {
            return Err("The downloaded engine failed its version check; the previous engine remains active.".into());
        }
        let old = if current_is_bundled {
            "bundled".into()
        } else {
            installed.clone()
        };
        state = Active {
            current: latest.clone(),
            previous: old,
        };
        save(data, &state)?;
        // Old files in use are left for a later update; only version directories owned by this updater are candidates.
        for entry in fs::read_dir(data).map_err(|e| e.to_string())?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if valid_version(&name)
                && name != state.current
                && name != state.previous
                && entry.file_type().is_ok_and(|t| t.is_dir())
            {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(Status {
        current: if mode == "install" && available {
            latest.clone()
        } else {
            installed
        },
        latest,
        available: available && mode == "check",
        can_rollback: version_path(data, bundled, &state.previous).is_some_and(|p| p.is_file()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_untrusted_paths_versions_and_corrupt_executables() {
        for bad in ["../bad", "2026/10/26", "nightly", "2026.1", "2026.1.2.exe"] {
            assert!(!valid_version(bad));
        }
        let bytes = b"MZ harmless fixture";
        let hash = format!("{:x}", Sha256::digest(bytes));
        assert!(verify(bytes, &hash).is_ok());
        assert!(verify(b"MZ corrupted fixture", &hash).is_err());
        assert_eq!(checksum(&format!("{hash}  yt-dlp.exe\n")).unwrap(), hash);
        assert!(checksum(&format!("{hash}  unrelated.exe")).is_err());
    }
    #[test]
    fn atomically_switches_and_falls_back_without_overwriting_engines() {
        let data = std::env::temp_dir().join(format!("ytnabber-updater-{}", std::process::id()));
        fs::create_dir_all(data.join("2026.10.26")).unwrap();
        fs::write(data.join("yt-dlp.exe"), b"bundled").unwrap();
        fs::write(data.join("2026.10.26/yt-dlp.exe"), b"updated").unwrap();
        save(
            &data,
            &Active {
                current: "2026.10.26".into(),
                previous: "bundled".into(),
            },
        )
        .unwrap();
        assert_eq!(selected(&data, &data), data.join("2026.10.26/yt-dlp.exe"));
        save(
            &data,
            &Active {
                current: "2026.11.01".into(),
                previous: "bundled".into(),
            },
        )
        .unwrap();
        assert_eq!(selected(&data, &data), data.join("yt-dlp.exe"));
        assert_eq!(fs::read(data.join("yt-dlp.exe")).unwrap(), b"bundled");
        fs::remove_dir_all(data).unwrap();
    }
    #[tokio::test]
    #[ignore = "Requires Internet and YTNABBER_OLD_ENGINE pointing to a verified older official executable"]
    async fn official_engine_update_and_rollback() {
        let old = PathBuf::from(
            std::env::var("YTNABBER_OLD_ENGINE")
                .expect("Set YTNABBER_OLD_ENGINE to a verified older official yt-dlp.exe"),
        );
        let data = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../.cache")
            .join(format!("updater test-{}", std::process::id()));
        let bundled = data.join("bundled");
        fs::create_dir_all(&bundled).unwrap();
        let original = fs::read(old).unwrap();
        fs::write(bundled.join("yt-dlp.exe"), &original).unwrap();
        let status = update(&data, &bundled, "check").await.unwrap();
        assert!(status.available, "This fixture needs an older engine");
        let installed = update(&data, &bundled, "install").await.unwrap();
        assert_eq!(installed.current, status.latest);
        assert!(installed.can_rollback && !installed.available);
        assert_ne!(selected(&data, &bundled), bundled.join("yt-dlp.exe"));
        let restored = update(&data, &bundled, "rollback").await.unwrap();
        assert_eq!(restored.current, status.current);
        assert_eq!(selected(&data, &bundled), bundled.join("yt-dlp.exe"));
        assert_eq!(fs::read(bundled.join("yt-dlp.exe")).unwrap(), original);
        fs::remove_dir_all(data).unwrap();
    }
}
