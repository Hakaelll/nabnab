use crate::library::LibraryItem;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

const SCHEMA_VERSION: u32 = 2;
const STATE_FILE: &str = "state-v1.json";
const MAX_STATE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub folder: PathBuf,
    pub language: String,
    pub network_limit: usize,
    pub processing_limit: usize,
    pub reduced_motion: bool,
    pub sidebar_expanded: bool,
    pub tray_enabled: bool,
    pub close_to_tray: bool,
    pub start_with_windows: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folder: PathBuf::new(),
            language: "es".into(),
            network_limit: 3,
            processing_limit: 1,
            reduced_motion: false,
            sidebar_expanded: true,
            tray_enabled: false,
            close_to_tray: false,
            start_with_windows: false,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !self.folder.is_absolute() {
            return Err("Choose an absolute local destination folder.".into());
        }
        if !matches!(self.language.as_str(), "es" | "en") {
            return Err("Choose Spanish or English.".into());
        }
        if !(1..=8).contains(&self.network_limit) || !(1..=4).contains(&self.processing_limit) {
            return Err("Use 1–8 downloads and 1–4 processing jobs.".into());
        }
        if self.close_to_tray && !self.tray_enabled {
            return Err("Enable the tray icon before choosing to close to the tray.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Data {
    pub schema_version: u32,
    pub settings: Settings,
    #[serde(default)]
    pub jobs: Vec<crate::queue::Job>,
    #[serde(default)]
    pub library: Vec<LibraryItem>,
    #[serde(default)]
    pub collections: Vec<String>,
}

impl Data {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "The saved data uses schema {}. Open it with a compatible nabnab version.",
                self.schema_version
            ));
        }
        self.settings.validate()?;
        for job in &self.jobs {
            job.validate_public_source()?;
        }
        Ok(())
    }
}

pub struct Repository {
    pub data: Data,
    root: PathBuf,
    _lock: File,
}

fn read_data(path: &Path) -> Result<Data, String> {
    let mut file = File::open(path).map_err(|e| format!("Could not read saved data: {e}"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_STATE_BYTES {
        return Err("The saved data exceeds the supported size. No data was replaced.".into());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let data: Data = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Saved data is invalid. Keep state-v1.json for recovery: {e}"))?;
    if data.schema_version == 1 {
        data.settings.validate()?;
    } else {
        data.validate()?;
    }
    Ok(data)
}

fn replace_file(from: &Path, to: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };
        let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(format!(
                "Could not commit saved data: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    fs::rename(from, to).map_err(|e| format!("Could not commit saved data: {e}"))
}

impl Repository {
    pub fn open(root: &Path, default_folder: &Path) -> Result<Self, String> {
        fs::create_dir_all(root).map_err(|e| format!("Could not create the data folder: {e}"))?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let lock = options.open(root.join("nabnab.lock")).map_err(|e| {
            format!("Could not lock nabnab data. Close the other nabnab instance and retry: {e}")
        })?;
        #[cfg(not(windows))]
        lock.try_lock()
            .map_err(|e| format!("Close the other nabnab instance: {e}"))?;

        let path = root.join(STATE_FILE);
        let mut data = if path.exists() {
            read_data(&path)?
        } else {
            let legacy = root.join("destination.json");
            let folder = if legacy.exists() {
                let bytes = fs::read(&legacy)
                    .map_err(|e| format!("Could not read the previous destination: {e}"))?;
                let folder: PathBuf = serde_json::from_slice(&bytes).map_err(|e| {
                    format!("The previous destination file is invalid. It was not modified: {e}")
                })?;
                if !folder.is_absolute() {
                    return Err(
                        "The previous destination is not absolute. It was not modified.".into(),
                    );
                }
                let backup = root.join("destination.v0.backup.json");
                if backup.exists() {
                    if fs::read(&backup).map_err(|e| e.to_string())? != bytes {
                        return Err("The migration backup differs from the previous settings. Keep both files and resolve this before retrying.".into());
                    }
                } else {
                    let temporary = temporary_path(root, "destination-v0-backup");
                    write_temporary(&temporary, &bytes)?;
                    replace_file(&temporary, &backup)?;
                }
                folder
            } else {
                default_folder.to_owned()
            };
            Data {
                schema_version: SCHEMA_VERSION,
                settings: Settings {
                    folder,
                    ..Settings::default()
                },
                jobs: Vec::new(),
                library: Vec::new(),
                collections: Vec::new(),
            }
        };
        if data.schema_version == 1 {
            let previous = fs::read(&path)
                .map_err(|_| "Could not read the previous repository for backup.")?;
            crate::sessions::protect_repository_backup(root, &previous)?;
            for job in &mut data.jobs {
                job.protect_source(root)?;
            }
            data.schema_version = SCHEMA_VERSION;
        }
        data.validate()?;
        for job in &mut data.jobs {
            job.recover();
        }
        let repository = Self {
            data,
            root: root.to_owned(),
            _lock: lock,
        };
        repository.save()?;
        Ok(repository)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn save(&self) -> Result<(), String> {
        self.data.validate()?;
        let bytes = serde_json::to_vec(&self.data).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(
                "The data exceeds the supported size. Existing saved data was kept.".into(),
            );
        }
        let temporary = temporary_path(&self.root, "state-v1");
        write_temporary(&temporary, &bytes)?;
        // A migration is committed only after its complete serialized data validates.
        read_data(&temporary)?;
        replace_file(&temporary, &self.root.join(STATE_FILE))
    }
}

fn temporary_path(root: &Path, prefix: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    root.join(format!("{prefix}-{}-{nonce}.new", std::process::id()))
}

fn write_temporary(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Could not write saved data: {e}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("Could not flush saved data: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);
    fn directory() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "nabnab-store-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn fresh_install_and_second_start_keep_settings_and_collections() {
        let root = directory();
        let folder = root.join("media");
        let mut repository = Repository::open(&root, &folder).unwrap();
        assert_eq!(repository.data.schema_version, 2);
        assert_eq!(repository.data.settings.folder, folder);
        assert_eq!(repository.data.settings.network_limit, 3);
        repository.data.collections.push("Music".into());
        repository.data.settings.reduced_motion = true;
        repository.save().unwrap();
        drop(repository);
        let repository = Repository::open(&root, &root).unwrap();
        assert_eq!(repository.data.settings.folder, folder);
        assert_eq!(repository.data.collections, vec!["Music"]);
        assert!(repository.data.settings.reduced_motion);
        drop(repository);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_backs_up_real_legacy_destination_and_is_idempotent() {
        let root = directory();
        let folder = root.join("Unicode á media");
        fs::create_dir(&folder).unwrap();
        let media = folder.join("keep.mp4");
        fs::write(&media, b"untouched").unwrap();
        let legacy = serde_json::to_vec(&folder).unwrap();
        fs::write(root.join("destination.json"), &legacy).unwrap();
        let repository = Repository::open(&root, &root).unwrap();
        assert_eq!(repository.data.settings.folder, folder);
        assert_eq!(
            fs::read(root.join("destination.v0.backup.json")).unwrap(),
            legacy
        );
        drop(repository);
        let repository = Repository::open(&root, &root).unwrap();
        assert_eq!(repository.data.settings.folder, folder);
        assert_eq!(fs::read(root.join("destination.json")).unwrap(), legacy);
        assert_eq!(fs::read(media).unwrap(), b"untouched");
        drop(repository);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_and_future_data_are_never_overwritten() {
        let root = directory();
        fs::write(root.join(STATE_FILE), b"{truncated").unwrap();
        assert!(Repository::open(&root, &root).is_err());
        assert_eq!(fs::read(root.join(STATE_FILE)).unwrap(), b"{truncated");
        fs::remove_file(root.join(STATE_FILE)).unwrap();
        let repository = Repository::open(&root, &root).unwrap();
        drop(repository);
        let mut future: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(STATE_FILE)).unwrap()).unwrap();
        future["schemaVersion"] = serde_json::json!(999);
        let bytes = serde_json::to_vec(&future).unwrap();
        fs::write(root.join(STATE_FILE), &bytes).unwrap();
        assert!(Repository::open(&root, &root).is_err());
        assert_eq!(fs::read(root.join(STATE_FILE)).unwrap(), bytes);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_legacy_data_does_not_mark_migration_complete() {
        let root = directory();
        fs::write(root.join("destination.json"), b"invalid").unwrap();
        assert!(Repository::open(&root, &root).is_err());
        assert!(!root.join(STATE_FILE).exists());
        assert_eq!(fs::read(root.join("destination.json")).unwrap(), b"invalid");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repository_lock_is_exclusive_and_released_on_drop() {
        let root = directory();
        let repository = Repository::open(&root, &root).unwrap();
        assert!(Repository::open(&root, &root).is_err());
        drop(repository);
        assert!(Repository::open(&root, &root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_settings_do_not_replace_last_valid_commit() {
        let root = directory();
        let mut repository = Repository::open(&root, &root).unwrap();
        let before = fs::read(root.join(STATE_FILE)).unwrap();
        repository.data.settings.network_limit = 0;
        assert!(repository.save().is_err());
        assert_eq!(fs::read(root.join(STATE_FILE)).unwrap(), before);
        drop(repository);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn windows_preferences_default_off_and_require_a_visible_recovery_control() {
        let root = directory();
        let legacy_settings = serde_json::json!({"folder": root, "language":"es", "networkLimit":3, "processingLimit":1, "reducedMotion":false, "sidebarExpanded":true});
        let mut settings: Settings = serde_json::from_value(legacy_settings).unwrap();
        assert!(!settings.tray_enabled);
        assert!(!settings.close_to_tray);
        assert!(!settings.start_with_windows);
        settings.close_to_tray = true;
        assert!(settings.validate().is_err());
        settings.tray_enabled = true;
        assert!(settings.validate().is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn schema_one_urls_migrate_with_a_protected_backup_and_idempotent_references() {
        let root = directory();
        let url = "https://example.org/LEGACY-PATH-SECRET?api_key=LEGACY-API-SECRET&accessToken=LEGACY-ACCESS-SECRET#LEGACY-FRAGMENT-SECRET";
        let mut repository = Repository::open(&root, &root).unwrap();
        crate::queue::enqueue_download(
            &mut repository,
            vec![url.into()],
            vec![],
            crate::model::Format::Mp4,
            None,
            None,
            "legacy-migration",
        )
        .unwrap();
        let mut legacy = repository.data.clone();
        legacy.schema_version = 1;
        let crate::queue::Source::Download {
            url: source,
            url_ref,
            ..
        } = &mut legacy.jobs[0].source
        else {
            panic!("expected download");
        };
        let reference = url_ref.take().unwrap();
        *source = url.into();
        let bytes = serde_json::to_vec(&legacy).unwrap();
        drop(repository);
        fs::remove_file(root.join("job-urls").join(format!("{reference}.bin"))).unwrap();
        fs::write(root.join(STATE_FILE), &bytes).unwrap();

        let migrated = Repository::open(&root, &root).unwrap();
        assert_eq!(migrated.data.schema_version, 2);
        assert_eq!(migrated.data.jobs.len(), 1);
        assert_eq!(migrated.data.jobs[0].state, crate::queue::JobState::Paused);
        let crate::queue::Source::Download {
            url: public,
            url_ref,
            ..
        } = &migrated.data.jobs[0].source
        else {
            panic!("expected download");
        };
        assert_eq!(public, "https://example.org");
        assert_eq!(url_ref.as_deref(), Some(reference.as_str()));
        assert_eq!(
            crate::sessions::resolve_url(&root, &reference)
                .unwrap()
                .as_str(),
            url
        );
        let saved = fs::read_to_string(root.join(STATE_FILE)).unwrap();
        let backup = fs::read(root.join("migration-backups/state-v1.dpapi")).unwrap();
        for secret in [
            "LEGACY-PATH-SECRET",
            "LEGACY-API-SECRET",
            "LEGACY-ACCESS-SECRET",
            "LEGACY-FRAGMENT-SECRET",
        ] {
            assert!(!saved.contains(secret));
            assert!(!backup
                .windows(secret.len())
                .any(|part| part == secret.as_bytes()));
        }
        crate::sessions::protect_repository_backup(&root, &bytes).unwrap();
        assert!(
            crate::sessions::protect_repository_backup(&root, b"different prior repository")
                .is_err()
        );
        let cipher = fs::read(root.join("job-urls").join(format!("{reference}.bin"))).unwrap();
        drop(migrated);
        let again = Repository::open(&root, &root).unwrap();
        assert_eq!(again.data.schema_version, 2);
        assert_eq!(again.data.jobs.len(), 1);
        assert_eq!(fs::read_dir(root.join("job-urls")).unwrap().count(), 1);
        assert_eq!(
            fs::read(root.join("job-urls").join(format!("{reference}.bin"))).unwrap(),
            cipher
        );
        assert_eq!(
            fs::read(root.join("migration-backups/state-v1.dpapi")).unwrap(),
            backup
        );
        drop(again);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn plaintext_operational_urls_cannot_be_written_back_into_schema_two() {
        let root = directory();
        let mut repository = Repository::open(&root, &root).unwrap();
        crate::queue::enqueue_download(
            &mut repository,
            vec!["https://example.org/media?api_key=WRITE-GUARD-SECRET".into()],
            vec![],
            crate::model::Format::Mp4,
            None,
            None,
            "write-guard",
        )
        .unwrap();
        let before = fs::read(root.join(STATE_FILE)).unwrap();
        let crate::queue::Source::Download { url, url_ref, .. } =
            &mut repository.data.jobs[0].source
        else {
            panic!("expected download");
        };
        *url = "https://example.org/media?api_key=WRITE-GUARD-SECRET".into();
        *url_ref = None;
        assert!(repository.save().is_err());
        assert_eq!(fs::read(root.join(STATE_FILE)).unwrap(), before);
        assert!(!root.join("migration-backups").exists());
        drop(repository);
        fs::remove_dir_all(root).unwrap();
    }
}
