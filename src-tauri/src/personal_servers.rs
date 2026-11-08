use crate::{file_sources, queue, sessions, AppState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tokio::sync::watch;

mod nas;
mod sftp;
mod webdav;

const SECRET: &str = "personal-servers";
const MAX_SERVERS: usize = 12;
const MAX_REFERENCES: usize = 128;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stamp {
    size: Option<u64>,
    modified: Option<String>,
    etag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    path: Vec<String>,
    directory: bool,
    stamp: Stamp,
    mime: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "config", rename_all = "camelCase")]
enum Config {
    Webdav(webdav::Config),
    Sftp(sftp::Config),
    Nas(PathBuf),
}

impl Config {
    fn display(&self) -> (String, &'static str) {
        match self {
            Self::Webdav(config) => (config.root.clone(), "webdav"),
            Self::Sftp(config) => (
                format!("{}:{}{}", config.host, config.port, config.root),
                "sftp",
            ),
            Self::Nas(root) => (root.to_string_lossy().into_owned(), "nas"),
        }
    }

    fn scope_id(&self) -> String {
        let (location, kind) = self.display();
        let user = match self {
            Self::Webdav(config) => config.username.as_str(),
            Self::Sftp(config) => config.username.as_str(),
            Self::Nas(_) => "",
        };
        fingerprint(&(kind, location, user))
    }

    fn id(&self) -> String {
        let scope = self.scope_id();
        match self {
            Self::Sftp(config) => match &config.host_key {
                Some(key) => fingerprint(&(scope, &key.fingerprint)),
                None => scope,
            },
            _ => scope,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Connection {
    id: String,
    name: String,
    config: Config,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerView {
    id: String,
    name: String,
    kind: &'static str,
    location: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    server_id: String,
    path: Vec<String>,
    stamp: Stamp,
}

impl Selection {
    pub fn server_id(&self) -> &str {
        &self.server_id
    }

    pub fn validate(&self) -> Result<(), String> {
        if !valid_id(&self.server_id)
            || self.path.is_empty()
            || !valid_relative(&self.path)
            || self
                .stamp
                .etag
                .as_ref()
                .is_some_and(|value| value.len() > 512 || value.chars().any(char::is_control))
            || self
                .stamp
                .modified
                .as_ref()
                .is_some_and(|value| value.len() > 160 || value.chars().any(char::is_control))
        {
            return Err("Invalid personal server file selection.".into());
        }
        Ok(())
    }

    fn filename(&self) -> Result<&str, String> {
        self.validate()?;
        let name = self.path.last().ok_or("Select a file.")?;
        crate::direct::validate_filename(name)?;
        Ok(name)
    }

    pub fn content_id(&self) -> String {
        format!("personal:{}", fingerprint(self))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reference {
    id: String,
    server_name: String,
    selection: Selection,
}

#[derive(Default, Serialize, Deserialize)]
struct Vault {
    connections: Vec<Connection>,
    references: Vec<Reference>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    servers: Vec<ServerView>,
    references: Vec<Reference>,
}

struct Trust {
    challenge: u64,
    created: Instant,
    config: sftp::Config,
}

#[derive(Default)]
pub struct BrowserState {
    vault: Mutex<()>,
    pages: Mutex<HashMap<String, Vec<Entry>>>,
    trust: Mutex<Option<Trust>>,
}

fn fingerprint(value: &impl Serialize) -> String {
    let bytes = serde_json::to_vec(value).expect("personal server identity is serializable");
    format!("{:x}", Sha256::digest(bytes))
}

fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_relative(parts: &[String]) -> bool {
    parts.len() <= 32
        && parts.iter().map(|part| part.len()).sum::<usize>() <= 4096
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 512
                && !matches!(part.as_str(), "." | "..")
                && !part
                    .chars()
                    .any(|ch| ch.is_control() || matches!(ch, '/' | '\\' | ':'))
        })
}

async fn cancelled(mut cancel: watch::Receiver<bool>) {
    while !*cancel.borrow_and_update() {
        if cancel.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

fn load(root: &Path) -> Result<Vault, String> {
    let path = root.join("module-secrets").join(format!("{SECRET}.bin"));
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vault::default()),
        Err(_) => return Err("Could not read personal server storage.".into()),
        Ok(_) => (),
    }
    let mut bytes = sessions::load_module_secret(root, SECRET)?;
    let parsed = serde_json::from_slice(&bytes);
    bytes.fill(0);
    parsed.map_err(|_| "Personal server storage could not be read.".into())
}

fn save(root: &Path, data: &Vault) -> Result<(), String> {
    let mut bytes =
        serde_json::to_vec(data).map_err(|_| "Could not prepare personal server storage.")?;
    let result = if bytes.len() <= 65536 {
        sessions::save_module_secret(root, SECRET, &bytes)
    } else {
        Err("Personal server storage is full. Remove references before adding more.".into())
    };
    bytes.fill(0);
    result
}

fn snapshot(data: Vault) -> Snapshot {
    Snapshot {
        servers: data
            .connections
            .into_iter()
            .map(|connection| {
                let (location, kind) = connection.config.display();
                ServerView {
                    id: connection.id,
                    name: connection.name,
                    kind,
                    location,
                }
            })
            .collect(),
        references: data.references,
    }
}

fn connected<'a>(data: &'a Vault, id: &str) -> Result<&'a Connection, String> {
    data.connections
        .iter()
        .find(|server| server.id == id)
        .ok_or_else(|| {
            "This server connection was removed. Reconnect it to access the files.".into()
        })
}

fn store_connection(state: &AppState, name: String, config: Config) -> Result<Snapshot, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 100 || name.chars().any(char::is_control) {
        return Err("Use a server name of 1–100 characters.".into());
    }
    let root = crate::config_root(state)?;
    let _guard = state.personal_servers.vault.lock().unwrap();
    let mut data = load(&root)?;
    let id = config.id();
    let scope = config.scope_id();
    let connection = Connection {
        id: id.clone(),
        name: name.into(),
        config,
    };
    if let Some(existing) = data
        .connections
        .iter_mut()
        .find(|server| server.config.scope_id() == scope)
    {
        if let (Config::Sftp(old), Config::Sftp(new)) = (&existing.config, &connection.config) {
            let old_key = old.host_key.as_ref().map(|key| &key.fingerprint);
            let new_key = new.host_key.as_ref().map(|key| &key.fingerprint);
            if old_key != new_key {
                return Err("The SFTP host key changed. Verify the server before removing the old connection and trusting a new key.".into());
            }
        }
        if let (Config::Webdav(old), Config::Webdav(new)) = (&existing.config, &connection.config) {
            if old.certificate_pem != new.certificate_pem {
                return Err("The WebDAV trust settings changed. Verify the certificate and disconnect the old connection before saving a different certificate.".into());
            }
        }
        *existing = connection;
    } else {
        if data.connections.len() >= MAX_SERVERS {
            return Err("Remove a personal server before adding another.".into());
        }
        data.connections.push(connection);
    }
    save(&root, &data)?;
    state.personal_servers.pages.lock().unwrap().remove(&id);
    Ok(snapshot(data))
}

#[tauri::command]
pub fn personal_servers_status(state: State<Arc<AppState>>) -> Result<Snapshot, String> {
    let root = crate::config_root(&state)?;
    let _guard = state.personal_servers.vault.lock().unwrap();
    load(&root).map(snapshot)
}

#[tauri::command]
pub fn personal_server_webdav(
    state: State<Arc<AppState>>,
    name: String,
    root: String,
    username: String,
    password: String,
    certificate_pem: Option<String>,
) -> Result<Snapshot, String> {
    let config = webdav::normalize(webdav::Config {
        root,
        username,
        password,
        certificate_pem,
    })?;
    store_connection(&state, name, Config::Webdav(config))
}

#[tauri::command]
pub async fn personal_server_choose_key(app: AppHandle) -> Result<Option<PathBuf>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_file()
            .map(|file| {
                let path = file
                    .into_path()
                    .map_err(|_| "Choose a local OpenSSH private key.")?;
                crate::library::authorized_path(&path)
            })
            .transpose()
    })
    .await
    .map_err(|_| "Could not open the key chooser.")?
}

#[tauri::command]
pub async fn personal_server_nas(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    name: String,
) -> Result<Option<Snapshot>, String> {
    let root = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .map(|folder| {
                let path = folder.into_path().map_err(|_| "Choose a mounted folder.")?;
                nas::normalize(&path)
            })
            .transpose()
    })
    .await
    .map_err(|_| "Could not open the folder chooser.")??;
    root.map(|root| store_connection(&state, name, Config::Nas(root)))
        .transpose()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Challenge {
    challenge: u64,
    fingerprint: String,
}

#[tauri::command]
pub async fn personal_server_probe_sftp(
    state: State<'_, Arc<AppState>>,
    host: String,
    port: u16,
    username: String,
    root: String,
    identity: PathBuf,
    request_id: u64,
) -> Result<Challenge, String> {
    let mut config = sftp::normalize(sftp::Config {
        host,
        port,
        username,
        root,
        identity,
        host_key: None,
    })?;
    let cancel = file_sources::begin(&state, request_id)?;
    let result = sftp::probe(&config, cancel.clone()).await;
    file_sources::finish(&state, request_id);
    let key = result?;
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    let fingerprint = key.fingerprint.clone();
    config.host_key = Some(key);
    *state.personal_servers.trust.lock().unwrap() = Some(Trust {
        challenge: request_id,
        created: Instant::now(),
        config,
    });
    Ok(Challenge {
        challenge: request_id,
        fingerprint,
    })
}

#[tauri::command]
pub fn personal_server_trust_sftp(
    state: State<Arc<AppState>>,
    name: String,
    challenge: u64,
    fingerprint: String,
) -> Result<Snapshot, String> {
    let pending = state
        .personal_servers
        .trust
        .lock()
        .unwrap()
        .take()
        .filter(|pending| {
            pending.challenge == challenge
                && pending.created.elapsed() < Duration::from_secs(120)
                && pending
                    .config
                    .host_key
                    .as_ref()
                    .is_some_and(|key| key.fingerprint == fingerprint)
        })
        .ok_or("The SFTP fingerprint confirmation expired or did not match.")?;
    store_connection(&state, name, Config::Sftp(pending.config))
}

#[tauri::command]
pub fn personal_server_remove(
    state: State<Arc<AppState>>,
    id: String,
    reference: bool,
) -> Result<Snapshot, String> {
    let root = crate::config_root(&state)?;
    let _guard = state.personal_servers.vault.lock().unwrap();
    let mut data = load(&root)?;
    if reference {
        data.references.retain(|entry| entry.id != id);
    } else {
        data.connections.retain(|server| server.id != id);
    }
    save(&root, &data)?;
    if !reference {
        state.personal_servers.pages.lock().unwrap().remove(&id);
    }
    Ok(snapshot(data))
}

#[tauri::command]
pub async fn personal_server_browse(
    state: State<'_, Arc<AppState>>,
    server_id: String,
    path: Vec<String>,
    request_id: u64,
) -> Result<Vec<Entry>, String> {
    if !valid_relative(&path) {
        return Err("Choose a folder within the saved root.".into());
    }
    let root = crate::config_root(&state)?;
    let config = {
        let _guard = state.personal_servers.vault.lock().unwrap();
        connected(&load(&root)?, &server_id)?.config.clone()
    };
    let cancel = file_sources::begin(&state, request_id)?;
    let result = match &config {
        Config::Webdav(config) => webdav::browse(config, &path, cancel.clone()).await,
        Config::Sftp(config) => sftp::browse(config, &path, cancel.clone()).await,
        Config::Nas(root) => nas::browse(root, &path, cancel.clone()).await,
    };
    file_sources::finish(&state, request_id);
    if *cancel.borrow() {
        return Err(crate::engine::CANCELLED.into());
    }
    let mut entries = result?;
    entries.retain(|entry| {
        entry.path.len() == path.len() + 1
            && entry.path.starts_with(&path)
            && valid_relative(&entry.path)
    });
    entries.sort_by(|left, right| {
        right
            .directory
            .cmp(&left.directory)
            .then_with(|| left.path.cmp(&right.path))
    });
    let _guard = state.personal_servers.vault.lock().unwrap();
    connected(&load(&root)?, &server_id)?;
    state
        .personal_servers
        .pages
        .lock()
        .unwrap()
        .insert(server_id, entries.clone());
    Ok(entries)
}

fn reviewed(
    state: &AppState,
    server_id: &str,
    paths: Vec<Vec<String>>,
) -> Result<Vec<Selection>, String> {
    if paths.is_empty() || paths.len() > 200 {
        return Err("Choose 1–200 reviewed files.".into());
    }
    let pages = state.personal_servers.pages.lock().unwrap();
    let page = pages
        .get(server_id)
        .ok_or("Browse the server again before selecting files.")?;
    let mut seen = HashSet::new();
    let mut selections = Vec::new();
    for path in paths {
        if !seen.insert(path.clone()) {
            continue;
        }
        let entry = page
            .iter()
            .find(|entry| !entry.directory && entry.path == path)
            .ok_or("Select files from the current folder listing.")?;
        let selection = Selection {
            server_id: server_id.into(),
            path,
            stamp: entry.stamp.clone(),
        };
        selection.validate()?;
        selections.push(selection);
    }
    Ok(selections)
}

#[tauri::command]
pub fn personal_server_index(
    state: State<Arc<AppState>>,
    server_id: String,
    paths: Vec<Vec<String>>,
) -> Result<Snapshot, String> {
    let selections = reviewed(&state, &server_id, paths)?;
    let root = crate::config_root(&state)?;
    let _guard = state.personal_servers.vault.lock().unwrap();
    let mut data = load(&root)?;
    let name = connected(&data, &server_id)?.name.clone();
    for selection in selections {
        let id = fingerprint(&(&selection.server_id, &selection.path));
        let reference = Reference {
            id: id.clone(),
            server_name: name.clone(),
            selection,
        };
        if let Some(existing) = data.references.iter_mut().find(|entry| entry.id == id) {
            *existing = reference;
        } else {
            if data.references.len() >= MAX_REFERENCES {
                return Err("Remove a saved reference before indexing another.".into());
            }
            data.references.push(reference);
        }
    }
    save(&root, &data)?;
    Ok(snapshot(data))
}

#[tauri::command]
pub fn personal_server_copy(
    state: State<Arc<AppState>>,
    server_id: Option<String>,
    paths: Option<Vec<Vec<String>>>,
    reference_id: Option<String>,
    client_request: String,
) -> Result<Vec<queue::Job>, String> {
    let root = crate::config_root(&state)?;
    let selections = match (server_id, paths, reference_id) {
        (Some(id), Some(paths), None) => reviewed(&state, &id, paths)?,
        (None, None, Some(id)) => {
            let _guard = state.personal_servers.vault.lock().unwrap();
            let data = load(&root)?;
            let reference = data
                .references
                .into_iter()
                .find(|entry| entry.id == id)
                .ok_or("This remote reference was removed.")?;
            vec![reference.selection]
        }
        _ => return Err("Choose reviewed files or a saved reference.".into()),
    };
    {
        let _guard = state.personal_servers.vault.lock().unwrap();
        let data = load(&root)?;
        for selection in &selections {
            connected(&data, &selection.server_id)?;
        }
    }
    let tasks = selections
        .into_iter()
        .map(|selection| {
            let title = selection.filename()?.to_owned();
            Ok((queue::Source::PersonalFile { selection }, title))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let jobs = queue::enqueue_tasks(repo, tasks, &client_request)?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}

pub async fn execute(
    state: &AppState,
    selection: &Selection,
    staging: &Path,
    run_id: u64,
    cancel: watch::Receiver<bool>,
) -> Result<PathBuf, String> {
    let name = selection.filename()?;
    let root = crate::config_root(state)?;
    let config = {
        let _guard = state.personal_servers.vault.lock().unwrap();
        connected(&load(&root)?, &selection.server_id)?
            .config
            .clone()
    };
    let temporary = staging.join(format!(".personal-{run_id}.part"));
    let result = match &config {
        Config::Webdav(config) => {
            webdav::download(
                config,
                &selection.path,
                &selection.stamp,
                &temporary,
                cancel.clone(),
            )
            .await
        }
        Config::Sftp(config) => {
            sftp::download(
                config,
                &selection.path,
                &selection.stamp,
                &temporary,
                cancel.clone(),
            )
            .await
        }
        Config::Nas(root) => {
            nas::download(
                root,
                &selection.path,
                &selection.stamp,
                &temporary,
                cancel.clone(),
            )
            .await
        }
    };
    let destination = staging.join(name);
    let result = result.and_then(|_| {
        if *cancel.borrow() {
            return Err(crate::engine::CANCELLED.into());
        }
        let _guard = state.personal_servers.vault.lock().unwrap();
        connected(&load(&root)?, &selection.server_id)?;
        if destination.exists() {
            return Err("A file with this name is already staged for this job.".into());
        }
        fs::rename(&temporary, &destination)
            .map_err(|_| "Could not finalize the personal server copy.".into())
    });
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map(|_| destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_identity_changes_with_resource_stamp_and_rejects_escapes() {
        let mut selection = Selection {
            server_id: "a".repeat(64),
            path: vec!["docs".into(), "report.pdf".into()],
            stamp: Stamp {
                size: Some(22),
                modified: Some("time".into()),
                etag: None,
            },
        };
        assert_eq!(selection.filename().unwrap(), "report.pdf");
        let original = selection.content_id();
        selection.stamp.size = Some(23);
        assert_ne!(original, selection.content_id());
        for invalid in ["..", "other/report.pdf", "other\\report.pdf", "data:stream"] {
            selection.path[0] = invalid.into();
            assert!(selection.validate().is_err());
        }
    }
}
