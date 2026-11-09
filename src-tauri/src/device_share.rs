use crate::{library, AppState};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime},
};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tokio::{io::AsyncReadExt, process::Command};

const CLI: &str = "kdeconnect-cli.exe";
const MAX_FILES: usize = 50;
const MAX_OUTPUT: u64 = 64 * 1024;

#[derive(Default)]
pub struct ShareState {
    chosen: Mutex<Option<PathBuf>>,
    plan: Mutex<Option<Prepared>>,
    sending: tokio::sync::Mutex<()>,
}

#[derive(Clone, Serialize)]
struct Device {
    id: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    state: &'static str,
    devices: Vec<Device>,
    share: bool,
    share_text: bool,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Payload {
    Files { ids: Vec<String> },
    Link { url: String },
    Text { text: String },
}

#[derive(Clone, Serialize)]
struct Entry {
    name: String,
    bytes: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    token: String,
    device: Device,
    entries: Vec<Entry>,
    total: u64,
}

#[derive(PartialEq, Eq)]
struct FileVersion {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

struct Prepared {
    preview: Preview,
    cli: FileVersion,
    files: Vec<FileVersion>,
    args: Vec<OsString>,
    expires: Instant,
}

struct Reply {
    stdout: String,
    stderr: String,
    success: bool,
}

fn file_version(path: &Path) -> Result<FileVersion, String> {
    let path = library::authorized_path(path)?;
    let info = fs::metadata(&path).map_err(|_| "The selected file is unavailable.")?;
    if !info.is_file() {
        return Err("Choose a local file, not a folder.".into());
    }
    Ok(FileVersion {
        path,
        bytes: info.len(),
        modified: info
            .modified()
            .map_err(|_| "Could not identify this file version.")?,
    })
}

fn cli_path(candidate: &Path) -> Option<FileVersion> {
    if !candidate.file_name()?.to_str()?.eq_ignore_ascii_case(CLI) {
        return None;
    }
    file_version(candidate).ok()
}

fn detected(state: &ShareState) -> Option<FileVersion> {
    if !cfg!(windows) {
        return None;
    }
    if let Some(chosen) = state.chosen.lock().unwrap().as_ref() {
        return cli_path(chosen);
    }
    let dirs = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .flat_map(|root| {
            [
                root.join("KDE Connect/bin"),
                root.join("KDE Connect"),
                root.join("Programs/KDE Connect/bin"),
            ]
        });
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain(dirs)
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| cli_path(&dir.join(CLI)))
}

async fn run(
    cli: &Path,
    args: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>,
) -> Result<Reply, String> {
    let mut command = Command::new(cli);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(parent) = cli.parent() {
        command.current_dir(parent);
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|_| "KDE Connect CLI could not start.")?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or("Could not read KDE Connect CLI.")?
        .take(MAX_OUTPUT + 1);
    let mut stderr = child
        .stderr
        .take()
        .ok_or("Could not read KDE Connect CLI.")?
        .take(MAX_OUTPUT + 1);
    let mut out = Vec::new();
    let mut err = Vec::new();
    let status = tokio::time::timeout(Duration::from_secs(10), async {
        tokio::try_join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err))?;
        if out.len() as u64 > MAX_OUTPUT || err.len() as u64 > MAX_OUTPUT {
            return Err(std::io::Error::other("KDE Connect output limit"));
        }
        child.wait().await
    })
    .await
    .map_err(|_| "KDE Connect CLI did not reply in time.")?
    .map_err(|_| "KDE Connect CLI returned an unreadable reply.")?;
    Ok(Reply {
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
        success: status.success(),
    })
}

fn valid_id(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn capabilities(help: &str) -> (bool, bool, bool) {
    let has = |flag| {
        help.split_whitespace()
            .any(|value| value.trim_end_matches(',') == flag)
    };
    (has("--share"), has("--share-text"), has("--id-name-only"))
}

async fn available(cli: &Path) -> Result<Vec<String>, String> {
    let reply = run(cli, ["--list-available", "--id-only"]).await?;
    if !reply.success {
        return Err("KDE Connect could not list paired devices.".into());
    }
    let ids: BTreeSet<_> = reply
        .stdout
        .lines()
        .map(str::trim)
        .filter(|id| valid_id(id))
        .map(str::to_owned)
        .take(200)
        .collect();
    if ids.is_empty() {
        let own = run(cli, ["--my-id"]).await?;
        if !own.success || !own.stderr.trim().is_empty() || !valid_id(own.stdout.trim()) {
            return Err("KDE Connect is installed but its device service is unavailable.".into());
        }
    } else if !reply.stderr.trim().is_empty() {
        return Err("KDE Connect could not verify its paired devices.".into());
    }
    Ok(ids.into_iter().collect())
}

async fn devices(cli: &Path, names: bool) -> Result<Vec<Device>, String> {
    let ids = available(cli).await?;
    let named = if ids.is_empty() {
        String::new()
    } else {
        let args = if names {
            vec!["--list-available", "--id-name-only"]
        } else {
            vec!["--list-available"]
        };
        run(cli, args)
            .await
            .ok()
            .filter(|value| value.success)
            .map(|value| value.stdout)
            .unwrap_or_default()
    };
    Ok(ids
        .into_iter()
        .map(|id| {
            let label = named
                .lines()
                .find_map(|line| {
                    if names {
                        line.strip_prefix(&id)
                            .and_then(|rest| rest.strip_prefix(' '))
                    } else {
                        let (name, suffix) =
                            line.strip_prefix("- ")?.split_once(&format!(": {id} "))?;
                        (!suffix.is_empty()).then_some(name)
                    }
                })
                .unwrap_or(&id);
            let name: String = label
                .chars()
                .filter(|character| !character.is_control())
                .take(100)
                .collect();
            Device {
                name: if name.trim().is_empty() {
                    id.clone()
                } else {
                    name
                },
                id,
            }
        })
        .collect())
}

#[tauri::command]
pub async fn device_share_status(state: State<'_, Arc<AppState>>) -> Result<Status, String> {
    let empty = |state| Status {
        state,
        devices: Vec::new(),
        share: false,
        share_text: false,
    };
    if !cfg!(windows) {
        return Ok(empty("unsupported"));
    }
    let Some(cli) = detected(&state.device_share) else {
        return Ok(empty("missing"));
    };
    let Ok(help) = run(&cli.path, ["--help"]).await else {
        return Ok(empty("offline"));
    };
    let (share, share_text, names) = capabilities(&help.stdout);
    if !help.success || (!share && !share_text) {
        return Ok(empty("incompatible"));
    }
    Ok(match devices(&cli.path, names).await {
        Ok(devices) => Status {
            state: "ready",
            devices,
            share,
            share_text,
        },
        Err(_) => empty("offline"),
    })
}

#[tauri::command]
pub async fn device_share_choose_cli(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("This integration supports KDE Connect for Windows.".into());
    }
    let path = tokio::task::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Choose kdeconnect-cli.exe")
            .add_filter("KDE Connect CLI", &["exe"])
            .blocking_pick_file()
    })
    .await
    .map_err(|_| "Could not open the KDE Connect file picker.")?;
    let Some(path) = path else {
        return Ok(());
    };
    let path = path
        .into_path()
        .map_err(|_| "Choose an installed KDE Connect CLI.")?;
    let cli = cli_path(&path).ok_or("Choose the installed kdeconnect-cli.exe executable.")?;
    let help = run(&cli.path, ["--help"]).await?;
    let (share, share_text, _) = capabilities(&help.stdout);
    if !help.success || (!share && !share_text) {
        return Err("This executable does not expose the KDE Connect share commands.".into());
    }
    *state.device_share.chosen.lock().unwrap() = Some(cli.path);
    *state.device_share.plan.lock().unwrap() = None;
    Ok(())
}

fn public_link(raw: &str) -> Result<String, String> {
    const REJECT: &str =
        "Use a public link without access parameters. For private material, choose its local file.";
    if raw.len() > 2048 || raw.chars().any(char::is_control) {
        return Err(REJECT.into());
    }
    let link = crate::model::validate_url(raw).map_err(|_| REJECT)?;
    let url = url::Url::parse(&link).map_err(|_| REJECT)?;
    if url.fragment().is_some() {
        return Err(REJECT.into());
    }
    if let Some(query) = url.query() {
        let video = query
            .strip_prefix("v=")
            .filter(|id| id.len() == 11 && valid_id(id));
        if !matches!(
            url.host_str(),
            Some("www.youtube.com" | "youtube.com" | "m.youtube.com")
        ) || url.path() != "/watch"
            || video.is_none()
        {
            return Err(REJECT.into());
        }
    }
    Ok(link)
}

#[tauri::command]
pub fn device_share_source_url(state: State<'_, Arc<AppState>>, id: String) -> Option<String> {
    let guard = state.repository.lock().unwrap();
    let item = guard.as_ref()?.data.library.iter().find(|item| item.id == id)?;
    let details = &item.details;
    let url = crate::public_sources::research_share_url(
        details.source_url.as_deref()?,
        details.content_id.as_deref()?,
        details.attribution.as_deref(),
    )?;
    public_link(&url).ok()
}

fn payload(
    state: &AppState,
    share: Payload,
) -> Result<(Vec<Entry>, Vec<FileVersion>, Vec<OsString>), String> {
    match share {
        Payload::Files { ids } => {
            if ids.is_empty() || ids.len() > MAX_FILES {
                return Err(
                    "Choose up to 50 local files, or export the collection as a ZIP.".into(),
                );
            }
            let paths = {
                let guard = state.repository.lock().unwrap();
                let repo = guard.as_ref().ok_or("Open nabnab first.")?;
                let mut seen = BTreeSet::new();
                ids.iter()
                    .filter(|id| seen.insert(*id))
                    .map(|id| {
                        repo.data
                            .library
                            .iter()
                            .find(|item| &item.id == id)
                            .map(|item| (item.name.clone(), item.path.clone()))
                            .ok_or("One selected file is no longer in Library.")
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };
            let mut args = Vec::new();
            let mut files = Vec::new();
            let mut entries = Vec::new();
            let mut units = 0;
            for (name, path) in paths {
                let file = file_version(&path).map_err(|_| {
                    "One selected file is unavailable. Check the selection in Library."
                })?;
                if files
                    .iter()
                    .any(|existing: &FileVersion| existing.path == file.path)
                {
                    continue;
                }
                let url = url::Url::from_file_path(&file.path)
                    .map_err(|_| "A selected path cannot be shared with KDE Connect.")?;
                units += url.as_str().len() + 3;
                if units > 24_000 {
                    return Err("The selected paths are too long for one request. Send fewer files or a portable ZIP.".into());
                }
                if args.is_empty() {
                    args.push(format!("--share={url}").into());
                } else {
                    if args.len() == 1 {
                        args.push("--".into());
                    }
                    args.push(url.to_string().into());
                }
                entries.push(Entry {
                    name,
                    bytes: file.bytes,
                });
                files.push(file);
            }
            Ok((entries, files, args))
        }
        Payload::Link { url } => {
            let url = public_link(&url)?;
            Ok((
                vec![Entry {
                    bytes: url.len() as u64,
                    name: url.clone(),
                }],
                Vec::new(),
                vec![format!("--share={url}").into()],
            ))
        }
        Payload::Text { text } => {
            if text.trim().is_empty()
                || text.len() > 8192
                || text.chars().any(|character| {
                    character.is_control() && !matches!(character, '\r' | '\n' | '\t')
                })
            {
                return Err("Share between 1 byte and 8 KiB of visible text.".into());
            }
            static LINKS: OnceLock<regex::Regex> = OnceLock::new();
            let links =
                LINKS.get_or_init(|| regex::Regex::new(r#"(?i)https?://[^\s<>"']+"#).unwrap());
            for link in links.find_iter(&text) {
                public_link(link.as_str().trim_end_matches(['.', ',', '!', ')', ';']))?;
            }
            Ok((
                vec![Entry {
                    bytes: text.len() as u64,
                    name: text.clone(),
                }],
                Vec::new(),
                vec![format!("--share-text={text}").into()],
            ))
        }
    }
}

#[tauri::command]
pub async fn device_share_prepare(
    state: State<'_, Arc<AppState>>,
    device_id: String,
    selection: Payload,
) -> Result<Preview, String> {
    if !valid_id(&device_id) {
        return Err("Choose a reachable paired device.".into());
    }
    let share = &state.device_share;
    let _guard = share
        .sending
        .try_lock()
        .map_err(|_| "A KDE Connect request is already in progress.")?;
    *share.plan.lock().unwrap() = None;
    let cli = detected(share).ok_or("KDE Connect CLI is unavailable.")?;
    let help = run(&cli.path, ["--help"]).await?;
    let (files, text, names) = capabilities(&help.stdout);
    if !help.success
        || if matches!(selection, Payload::Text { .. }) {
            !text
        } else {
            !files
        }
    {
        return Err(
            "This KDE Connect installation does not support the selected share method.".into(),
        );
    }
    let (entries, files, payload_args) = payload(&state, selection)?;
    let device = devices(&cli.path, names)
        .await?
        .into_iter()
        .find(|device| device.id == device_id)
        .ok_or("The paired device is no longer reachable.")?;
    let mut args = vec![OsString::from(format!("--device={}", device.id))];
    args.extend(payload_args);
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let token = format!(
        "{}-{}",
        crate::queue::now(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let total = entries
        .iter()
        .try_fold(0u64, |total, item| total.checked_add(item.bytes))
        .ok_or("Selection size exceeds the supported limit.")?;
    let preview = Preview {
        token,
        device,
        entries,
        total,
    };
    *share.plan.lock().unwrap() = Some(Prepared {
        preview: preview.clone(),
        cli,
        files,
        args,
        expires: Instant::now() + Duration::from_secs(300),
    });
    Ok(preview)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Receipt {
    HandedOff,
    Unconfirmed,
}

#[tauri::command]
pub async fn device_share_send(
    state: State<'_, Arc<AppState>>,
    token: String,
) -> Result<Receipt, String> {
    let share = &state.device_share;
    let _guard = share
        .sending
        .try_lock()
        .map_err(|_| "A KDE Connect request is already in progress.")?;
    let plan = {
        let mut pending = share.plan.lock().unwrap();
        if !pending
            .as_ref()
            .is_some_and(|plan| plan.preview.token == token && plan.expires > Instant::now())
        {
            return Err("Review the selection again before sending.".into());
        }
        pending
            .take()
            .ok_or("Review the selection again before sending.")?
    };
    if detected(share).as_ref() != Some(&plan.cli) {
        return Err(
            "The selected KDE Connect installation changed. Review the selection again.".into(),
        );
    }
    if !available(&plan.cli.path)
        .await?
        .contains(&plan.preview.device.id)
    {
        return Err("The paired device is no longer reachable. Review the selection again.".into());
    }
    for file in &plan.files {
        if file_version(&file.path).ok().as_ref() != Some(file) {
            return Err("One selected file changed. Review the files again before sending.".into());
        }
    }
    let file_count = plan.files.len();
    match run(&plan.cli.path, plan.args).await {
        Ok(reply)
            if reply.success
                && reply.stderr.trim().is_empty()
                && !reply.stdout.trim().is_empty()
                && (file_count == 0
                    || reply
                        .stdout
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .count()
                        == file_count) =>
        {
            Ok(Receipt::HandedOff)
        }
        _ => Ok(Receipt::Unconfirmed),
    }
}

#[tauri::command]
pub fn device_share_discard(state: State<'_, Arc<AppState>>, token: String) {
    let mut plan = state.device_share.plan.lock().unwrap();
    if plan
        .as_ref()
        .is_some_and(|plan| plan.preview.token == token)
    {
        *plan = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_local_and_private_links() {
        for url in [
            "https://alice:secret@example.org/file",
            "http://192.168.1.5/file",
            "https://example.org/f?token=x",
            "https://example.org/f#key",
            "https://youtube.com/watch?v=abcdefghijk&sig=x",
        ] {
            assert!(public_link(url).is_err(), "accepted {url}");
        }
        assert_eq!(
            public_link("https://www.youtube.com/watch?v=abcdefghijk").unwrap(),
            "https://www.youtube.com/watch?v=abcdefghijk"
        );
        assert!(public_link("https://example.org/public").is_ok());
    }
}
