use super::{analyze::fingerprint, cancelled, path_key, selected};
use crate::{library, store::Repository};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::sync::watch;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_JOURNAL: u64 = 4 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Replacement {
    pub pattern: String,
    pub replacement: String,
    pub regex: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameOptions {
    pub template: String,
    pub start: u32,
    pub padding: u8,
    pub date: String,
    pub replacement: Option<Replacement>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameEntry {
    pub record_ids: Vec<String>,
    pub old_path: PathBuf,
    pub new_path: PathBuf,
    pub sha256: String,
    pub size: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePlan {
    pub id: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub entries: Vec<RenameEntry>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationState {
    Queued,
    Running,
    Completed,
    Partial,
    Undoing,
    Undone,
    UndoPartial,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryState {
    Pending,
    Moved,
    Recorded,
    Undoing,
    Undone,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEntry {
    #[serde(flatten)]
    pub file: RenameEntry,
    pub state: EntryState,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameJournal {
    pub schema_version: u32,
    pub id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub state: OperationState,
    pub entries: Vec<JournalEntry>,
    pub error: Option<String>,
}
impl RenameJournal {
    pub fn successful(&self) -> bool {
        matches!(
            self.state,
            OperationState::Completed | OperationState::Undone
        )
    }
}

fn directory(root: &Path) -> Result<PathBuf, String> {
    crate::sessions::secure_module_directory(root, "file-operations")
}
fn reference(id: &str) -> Result<(), String> {
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        Err("Choose a saved file operation from its local history.".into())
    } else {
        Ok(())
    }
}
fn owned_path(root: &Path, id: &str, suffix: &str) -> Result<PathBuf, String> {
    reference(id)?;
    Ok(directory(root)?.join(format!("{id}.{suffix}.json")))
}
fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let path = library::authorized_path(path)?;
    if !path.is_file() || fs::metadata(&path).map_err(|e| e.to_string())?.len() > MAX_JOURNAL {
        return Err("The file operation record is invalid or too large.".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|_| "The file operation record is unreadable.".into())
}
fn write<T: Serialize>(path: &Path, value: &T, new: bool) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_JOURNAL {
        return Err("The file operation record exceeds its size limit.".into());
    }
    let parent = path.parent().ok_or("Invalid operation record path.")?;
    let pending = parent.join(format!(
        "operation-{}-{}.pending",
        crate::queue::now(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&pending)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let from: Vec<u16> = pending.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH
                    | if new {
                        0
                    } else {
                        windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                    },
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    #[cfg(not(windows))]
    if new {
        fs::hard_link(&pending, path).map_err(|e| e.to_string())?;
        fs::remove_file(&pending).map_err(|e| e.to_string())?;
    } else {
        fs::rename(&pending, path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn options(options: &RenameOptions) -> Result<Option<regex::Regex>, String> {
    if options.template.is_empty()
        || options.template.len() > 240
        || options.start > 1_000_000
        || options.padding > 8
        || options.date.len() != 10
        || !options.date.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 4 | 7) {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
    {
        return Err("Use a filename template up to 240 bytes, counter 0–1000000, padding 0–8 and a date in YYYY-MM-DD format.".into());
    }
    let mut unknown = options.template.clone();
    for token in ["{name}", "{ext}", "{extension}", "{counter}", "{date}"] {
        unknown = unknown.replace(token, "");
    }
    if unknown.contains(['{', '}']) {
        return Err(
            "Use only {name}, {ext}, {extension}, {counter} and {date} in filename templates."
                .into(),
        );
    }
    if let Some(replacement) = &options.replacement {
        if replacement.pattern.is_empty()
            || replacement.pattern.len() > 512
            || replacement.replacement.len() > 512
        {
            return Err(
                "Use a nonempty replacement pattern and replacement of up to 512 bytes each."
                    .into(),
            );
        }
        if replacement.regex {
            return RegexBuilder::new(&replacement.pattern)
                .size_limit(65536)
                .dfa_size_limit(65536)
                .build()
                .map(Some)
                .map_err(|_| {
                    "The regular expression is invalid or exceeds the bounded expression budget."
                        .into()
                });
        }
    }
    Ok(None)
}
fn filename(
    path: &Path,
    options: &RenameOptions,
    regex: Option<&regex::Regex>,
    position: usize,
) -> Result<String, String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("A selected filename is not valid UTF-8.")?;
    let extension = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let suffix = if extension.is_empty() {
        String::new()
    } else {
        format!(".{extension}")
    };
    let stem = if let Some(replacement) = &options.replacement {
        if let Some(regex) = regex {
            regex
                .replace_all(stem, replacement.replacement.as_str())
                .into_owned()
        } else {
            stem.replace(&replacement.pattern, &replacement.replacement)
        }
    } else {
        stem.into()
    };
    if stem.len() > 2048 {
        return Err("The replacement produces an excessive filename.".into());
    }
    let counter = options
        .start
        .checked_add(position as u32)
        .ok_or("The filename counter is too large.")?;
    let counter = format!("{:0width$}", counter, width = options.padding as usize);
    let name = options
        .template
        .replace("{name}", &stem)
        .replace("{extension}", &suffix)
        .replace("{ext}", extension)
        .replace("{counter}", &counter)
        .replace("{date}", &options.date);
    crate::direct::validate_filename(&name)?;
    Ok(name)
}
fn validate_entry(entry: &RenameEntry) -> Result<(), String> {
    if entry.old_path.parent() != entry.new_path.parent()
        || !entry.old_path.is_absolute()
        || entry.record_ids.is_empty()
        || entry.record_ids.len() > 10000
        || entry.sha256.len() != 64
        || !entry.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || entry.old_path == entry.new_path
    {
        return Err("The rename record does not describe a bounded same-folder rename.".into());
    }
    crate::direct::validate_filename(
        entry
            .new_path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid renamed filename.")?,
    )?;
    Ok(())
}
pub fn plan_rename(
    root: &Path,
    library: &[library::LibraryItem],
    ids: &[String],
    configuration: &RenameOptions,
    cancel: watch::Receiver<bool>,
) -> Result<RenamePlan, String> {
    let regex = options(configuration)?;
    let mut seen = HashSet::new();
    let mut targets = HashSet::new();
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    for item in selected(library, ids)? {
        cancelled(&cancel)?;
        let old = library::authorized_path(&item.path)?;
        if !old.is_file() {
            return Err("Rename regular library files only.".into());
        }
        if !seen.insert(path_key(&old)) {
            continue;
        }
        let name = filename(&old, configuration, regex.as_ref(), seen.len() - 1)?;
        let new = old.with_file_name(name);
        if old == new {
            continue;
        }
        if path_key(&old) == path_key(&new) {
            return Err("For a case-only rename, first choose a distinct intermediate name. No file was renamed.".into());
        }
        if !targets.insert(path_key(&new)) {
            return Err("Two selected files would have the same destination. Add {counter} or change the template.".into());
        }
        if fs::symlink_metadata(&new).is_ok() {
            return Err("A preview destination already exists. Existing files will not be replaced; change the template.".into());
        }
        let aliases = library
            .iter()
            .filter(|i| path_key(&i.path) == path_key(&old))
            .map(|i| i.id.clone())
            .collect::<Vec<_>>();
        if aliases.len() > 1 {
            warnings.push(format!(
                "{} library records share {}. All their paths will be updated.",
                aliases.len(),
                item.name
            ));
        }
        entries.push(RenameEntry {
            record_ids: aliases,
            old_path: old.clone(),
            new_path: new,
            sha256: fingerprint(&old, &cancel)?,
            size: fs::metadata(&old).map_err(|e| e.to_string())?.len(),
        });
    }
    if entries.is_empty() {
        return Err("The template leaves every selected filename unchanged.".into());
    }
    let now = crate::queue::now();
    let bytes = serde_json::to_vec(&entries).map_err(|e| e.to_string())?;
    let id = format!(
        "{:x}",
        Sha256::digest(
            [
                bytes,
                format!("{now}:{}", SEQUENCE.fetch_add(1, Ordering::Relaxed)).into_bytes()
            ]
            .concat()
        )
    );
    warnings.push("Disk renames and library persistence are separate steps. The durable operation history records partial work and supports recovery; it is not an atomic disk/database transaction.".into());
    let plan = RenamePlan {
        id: id.clone(),
        created_at: now,
        expires_at: now + 10 * 60 * 1000,
        entries,
        warnings,
    };
    write(&owned_path(root, &id, "plan")?, &plan, true)?;
    Ok(plan)
}
fn verify_source(library: &[library::LibraryItem], entry: &RenameEntry) -> Result<(), String> {
    validate_entry(entry)?;
    let old = library::authorized_path(&entry.old_path)?;
    if old != entry.old_path
        || !old.is_file()
        || fs::metadata(&old).map_err(|e| e.to_string())?.len() != entry.size
    {
        return Err("An original file changed since preview. Its existing contents were preserved; make a new plan.".into());
    }
    let aliases = library
        .iter()
        .filter(|i| path_key(&i.path) == path_key(&old))
        .map(|i| i.id.clone())
        .collect::<HashSet<_>>();
    if aliases != entry.record_ids.iter().cloned().collect() {
        return Err(
            "The library records associated with a path changed. Review a new rename plan.".into(),
        );
    }
    if fs::symlink_metadata(&entry.new_path).is_ok() {
        return Err("A file appeared at the preview destination. It was not overwritten. Change the template or move that file yourself.".into());
    }
    Ok(())
}
pub fn approve_rename(repo: &Repository, plan_id: &str) -> Result<RenameJournal, String> {
    let operation = owned_path(repo.root(), plan_id, "operation")?;
    if operation.exists() {
        return load_journal(repo.root(), plan_id);
    }
    let plan: RenamePlan = read(&owned_path(repo.root(), plan_id, "plan")?)?;
    if plan.id != plan_id
        || plan.expires_at <= crate::queue::now()
        || plan.entries.is_empty()
        || plan.entries.len() > 200
    {
        return Err("The rename preview expired. Generate a new preview before queuing it.".into());
    }
    for entry in &plan.entries {
        verify_source(&repo.data.library, entry)?;
    }
    let journal = RenameJournal {
        schema_version: 1,
        id: plan.id,
        created_at: crate::queue::now(),
        updated_at: crate::queue::now(),
        state: OperationState::Queued,
        entries: plan
            .entries
            .into_iter()
            .map(|file| JournalEntry {
                file,
                state: EntryState::Pending,
                error: None,
            })
            .collect(),
        error: None,
    };
    write(&operation, &journal, true)?;
    Ok(journal)
}
fn load_journal(root: &Path, id: &str) -> Result<RenameJournal, String> {
    let journal: RenameJournal = read(&owned_path(root, id, "operation")?)?;
    if journal.schema_version != 1
        || journal.id != id
        || journal.entries.is_empty()
        || journal.entries.len() > 200
    {
        return Err("The saved rename operation is invalid.".into());
    }
    for entry in &journal.entries {
        validate_entry(&entry.file)?;
    }
    Ok(journal)
}
fn save_journal(root: &Path, journal: &mut RenameJournal) -> Result<(), String> {
    journal.updated_at = crate::queue::now();
    write(&owned_path(root, &journal.id, "operation")?, journal, false)
}
struct VerifiedFile {
    file: fs::File,
    path: PathBuf,
    size: u64,
    modified: Option<std::time::SystemTime>,
}
pub struct VerifiedOperation {
    journal: RenameJournal,
    files: Vec<Option<VerifiedFile>>,
    undo: bool,
}
fn open_stable(
    path: &Path,
    expected: &RenameEntry,
    cancel: &watch::Receiver<bool>,
) -> Result<VerifiedFile, String> {
    let path = library::authorized_path(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
        };
        options
            .access_mode(FILE_GENERIC_READ | DELETE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE);
    }
    let mut file = options.open(&path).map_err(|_| {
        "Close programs that are writing to a selected file, then retry this operation."
    })?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() != expected.size {
        return Err("A selected file changed since preview. Its contents were preserved.".into());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        cancelled(cancel)?;
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if format!("{:x}", digest.finalize()) != expected.sha256 {
        return Err("A selected file's content changed since preview. Its contents were preserved; make a new plan.".into());
    }
    Ok(VerifiedFile {
        file,
        path,
        size: metadata.len(),
        modified: metadata.modified().ok(),
    })
}
#[cfg(windows)]
fn file_identity(file: &fs::File) -> Result<(u32, u32, u32), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err("Could not verify a selected file's identity.".into());
    }
    Ok((
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
    ))
}
fn verify_handle(file: &VerifiedFile) -> Result<(), String> {
    let path = library::authorized_path(&file.path)?;
    if path != file.path {
        return Err("A selected file path changed after verification.".into());
    }
    let current = fs::File::open(&path).map_err(|e| e.to_string())?;
    let metadata = current.metadata().map_err(|e| e.to_string())?;
    if metadata.len() != file.size || metadata.modified().ok() != file.modified {
        return Err("A selected file changed after verification. No file was replaced.".into());
    }
    #[cfg(windows)]
    if file_identity(&current)? != file_identity(&file.file)? {
        return Err(
            "A different file appeared at the verified source path. No file was renamed.".into(),
        );
    }
    Ok(())
}
pub fn preflight_rename(
    root: &Path,
    library: &[library::LibraryItem],
    operation_id: &str,
    undo: bool,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(crate::model::Progress),
) -> Result<VerifiedOperation, String> {
    let journal = load_journal(root, operation_id)?;
    let mut files = Vec::new();
    if !undo
        && matches!(
            journal.state,
            OperationState::Undoing | OperationState::Undone | OperationState::UndoPartial
        )
    {
        return Err(
            "This operation is being undone or was already undone. Make a new rename preview."
                .into(),
        );
    }
    for (position, entry) in journal.entries.iter().enumerate() {
        cancelled(&cancel)?;
        let file = &entry.file;
        let verified = if (!undo && entry.state == EntryState::Recorded)
            || (undo && entry.state == EntryState::Undone)
        {
            None
        } else if undo
            && entry.state == EntryState::Pending
            && fs::symlink_metadata(&file.old_path).is_ok()
        {
            Some(open_stable(&file.old_path, file, &cancel)?)
        } else if undo
            && entry.state == EntryState::Undoing
            && fs::symlink_metadata(&file.new_path).is_err()
        {
            Some(open_stable(&file.old_path, file, &cancel)?)
        } else if undo {
            if fs::symlink_metadata(&file.old_path).is_ok() {
                return Err("Undo's original name is occupied. It was not overwritten; resolve the collision and retry undo.".into());
            }
            Some(open_stable(&file.new_path, file, &cancel)?)
        } else if fs::symlink_metadata(&file.old_path).is_ok() {
            verify_source(library, file)?;
            Some(open_stable(&file.old_path, file, &cancel)?)
        } else {
            Some(open_stable(&file.new_path, file, &cancel)?)
        };
        files.push(verified);
        emit(crate::model::Progress {
            request_id: run_id,
            phase: "verifying".into(),
            percent: Some((position + 1) as f64 / journal.entries.len() as f64 * 100.0),
            speed: None,
            downloaded: None,
            total: None,
        });
    }
    Ok(VerifiedOperation {
        journal,
        files,
        undo,
    })
}
fn move_noreplace(verified: &VerifiedFile, to: &Path) -> Result<(), String> {
    if fs::symlink_metadata(to).is_ok() {
        return Err("A destination file appeared after preview. It was not overwritten.".into());
    }
    verify_handle(verified)?;
    #[cfg(windows)]
    {
        use std::{
            mem::{offset_of, size_of},
            os::windows::{ffi::OsStrExt, io::AsRawHandle},
        };
        use windows_sys::Win32::Storage::FileSystem::{
            FileRenameInfo, SetFileInformationByHandle, FILE_RENAME_INFO,
        };
        let name: Vec<u16> = to
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let length = offset_of!(FILE_RENAME_INFO, FileName) + name.len() * 2;
        let mut buffer = vec![0u64; (length.max(size_of::<FILE_RENAME_INFO>()) + 7) / 8];
        let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        // Rename the verified handle itself, so replacing the old path cannot select another file.
        unsafe {
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = std::ptr::null_mut();
            (*info).FileNameLength = ((name.len() - 1) * 2) as u32;
            std::ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
            if SetFileInformationByHandle(
                verified.file.as_raw_handle(),
                FileRenameInfo,
                info.cast(),
                length as u32,
            ) == 0
            {
                return Err(format!(
                    "The file could not be renamed without replacing another file: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(&verified.path, to)
            .map_err(|e| format!("Could not reserve the renamed file without replacement: {e}"))?;
        fs::remove_file(&verified.path).map_err(|e| {
            format!("The new name exists but the original link could not be removed: {e}")
        })?;
    }
    Ok(())
}
fn remap(repo: &mut Repository, from: &Path, to: &Path) {
    let from_key = path_key(from);
    for item in &mut repo.data.library {
        if path_key(&item.path) == from_key {
            item.path = to.to_owned();
            item.name = to
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
        }
        for track in &mut item.details.tracks {
            if track
                .path
                .as_ref()
                .is_some_and(|path| path_key(path) == from_key)
            {
                track.path = Some(to.to_owned());
            }
        }
    }
    for job in &mut repo.data.jobs {
        if job.state == crate::queue::JobState::Completed {
            for output in &mut job.outputs {
                if path_key(output) == from_key {
                    *output = to.to_owned();
                }
            }
        }
    }
}
fn checked_verified(
    repo: &Repository,
    verified: &VerifiedOperation,
    undo: bool,
) -> Result<(), String> {
    if verified.undo != undo
        || serde_json::to_vec(&load_journal(repo.root(), &verified.journal.id)?)
            .map_err(|e| e.to_string())?
            != serde_json::to_vec(&verified.journal).map_err(|e| e.to_string())?
    {
        return Err(
            "The operation changed after verification. Retry to verify its current state.".into(),
        );
    }
    Ok(())
}
pub fn execute_rename(
    repo: &mut Repository,
    verified: VerifiedOperation,
    cancel: watch::Receiver<bool>,
) -> Result<RenameJournal, String> {
    checked_verified(repo, &verified, false)?;
    let mut journal = verified.journal;
    if journal.state == OperationState::Completed {
        return Ok(journal);
    }
    journal.state = OperationState::Running;
    journal.error = None;
    save_journal(repo.root(), &mut journal)?;
    for position in 0..journal.entries.len() {
        if journal.entries[position].state == EntryState::Recorded {
            continue;
        }
        let result = (|| {
            cancelled(&cancel)?;
            let file = journal.entries[position].file.clone();
            validate_entry(&file)?;
            let handle = verified.files[position]
                .as_ref()
                .ok_or("The verified source handle is missing.")?;
            verify_handle(handle)?;
            if handle.path == file.old_path {
                verify_source(&repo.data.library, &file)?;
                move_noreplace(handle, &file.new_path)?;
            } else if fs::symlink_metadata(&file.old_path).is_ok() {
                return Err("A file appeared at the old name while recovering this operation. It was not changed.".into());
            }
            journal.entries[position].state = EntryState::Moved;
            journal.entries[position].error = None;
            save_journal(repo.root(), &mut journal)?;
            remap(repo, &file.old_path, &file.new_path);
            repo.save()?;
            journal.entries[position].state = EntryState::Recorded;
            save_journal(repo.root(), &mut journal)
        })();
        if let Err(error) = result {
            journal.state = OperationState::Partial;
            journal.error = Some(error.clone());
            journal.entries[position].error = Some(error);
            save_journal(repo.root(), &mut journal)?;
            return Ok(journal);
        }
    }
    journal.state = OperationState::Completed;
    save_journal(repo.root(), &mut journal)?;
    Ok(journal)
}
pub fn undo_rename(
    repo: &mut Repository,
    verified: VerifiedOperation,
    cancel: watch::Receiver<bool>,
) -> Result<RenameJournal, String> {
    checked_verified(repo, &verified, true)?;
    let mut journal = verified.journal;
    if journal.state == OperationState::Undone {
        return Ok(journal);
    }
    journal.state = OperationState::Undoing;
    journal.error = None;
    save_journal(repo.root(), &mut journal)?;
    for position in (0..journal.entries.len()).rev() {
        if journal.entries[position].state == EntryState::Undone {
            continue;
        }
        let result = (|| {
            cancelled(&cancel)?;
            let file = journal.entries[position].file.clone();
            validate_entry(&file)?;
            let handle = verified.files[position]
                .as_ref()
                .ok_or("The verified undo handle is missing.")?;
            verify_handle(handle)?;
            if journal.entries[position].state == EntryState::Pending
                && handle.path == file.old_path
            {
                journal.entries[position].state = EntryState::Undone;
                return save_journal(repo.root(), &mut journal);
            }
            if journal.entries[position].state == EntryState::Undoing
                && handle.path == file.old_path
            {
                remap(repo, &file.new_path, &file.old_path);
                repo.save()?;
                journal.entries[position].state = EntryState::Undone;
                journal.entries[position].error = None;
                return save_journal(repo.root(), &mut journal);
            }
            if fs::symlink_metadata(&file.old_path).is_ok() {
                return Err("Undo stopped because the original name is occupied. That file was not overwritten. Resolve the collision and retry undo.".into());
            }
            journal.entries[position].state = EntryState::Undoing;
            save_journal(repo.root(), &mut journal)?;
            move_noreplace(handle, &file.old_path)?;
            remap(repo, &file.new_path, &file.old_path);
            repo.save()?;
            journal.entries[position].state = EntryState::Undone;
            journal.entries[position].error = None;
            save_journal(repo.root(), &mut journal)
        })();
        if let Err(error) = result {
            journal.state = OperationState::UndoPartial;
            journal.error = Some(error.clone());
            journal.entries[position].error = Some(error);
            save_journal(repo.root(), &mut journal)?;
            return Ok(journal);
        }
    }
    journal.state = OperationState::Undone;
    save_journal(repo.root(), &mut journal)?;
    Ok(journal)
}
pub fn rename_history(repo: &Repository) -> Result<Vec<RenameJournal>, String> {
    let mut history = Vec::new();
    for entry in fs::read_dir(directory(repo.root())?).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(id) = name.strip_suffix(".operation.json") else {
            continue;
        };
        if reference(id).is_err() {
            continue;
        }
        history.push(load_journal(repo.root(), id)?);
    }
    history.sort_by_key(|operation| std::cmp::Reverse(operation.created_at));
    history.truncate(50);
    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup() -> (PathBuf, Repository, Vec<String>) {
        let root = std::env::temp_dir().join(format!(
            "nabnab-rename-{}-{}-{}",
            std::process::id(),
            crate::queue::now(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let folder = root.join("files");
        fs::create_dir_all(&folder).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let folder = fs::canonicalize(folder).unwrap();
        let mut repo = Repository::open(&root, &folder).unwrap();
        let mut ids = Vec::new();
        for (name, text) in [
            ("first.txt", b"first contents".as_slice()),
            ("second.txt", b"second contents".as_slice()),
        ] {
            let path = folder.join(name);
            fs::write(&path, text).unwrap();
            let item = library::index(&path, None, None, None).unwrap();
            ids.push(item.id.clone());
            repo.data.library.push(item);
        }
        repo.save().unwrap();
        (root, repo, ids)
    }
    fn configuration() -> RenameOptions {
        RenameOptions {
            template: "review-{counter}{extension}".into(),
            start: 1,
            padding: 2,
            date: "2026-11-06".into(),
            replacement: None,
        }
    }
    fn plan(repo: &Repository, ids: &[String]) -> RenamePlan {
        let (_sender, cancel) = watch::channel(false);
        plan_rename(
            repo.root(),
            &repo.data.library,
            ids,
            &configuration(),
            cancel,
        )
        .unwrap()
    }
    fn verify(repo: &Repository, id: &str, undo: bool) -> VerifiedOperation {
        let (_sender, cancel) = watch::channel(false);
        preflight_rename(repo.root(), &repo.data.library, id, undo, cancel, 1, |_| {}).unwrap()
    }
    #[test]
    fn rename_keeps_stable_ids_updates_shared_records_tracks_outputs_and_undo_refuses_overwrite() {
        let (root, mut repo, ids) = setup();
        let old = repo.data.library[0].path.clone();
        let mut alias = repo.data.library[0].clone();
        alias.id = "same-physical-file".into();
        alias.collections = vec!["Another collection".into()];
        repo.data.library.push(alias);
        repo.data.library[1]
            .details
            .tracks
            .push(crate::captions::LocalTrack {
                id: "track".into(),
                language: "es".into(),
                origin: crate::captions::TrackOrigin::Imported,
                path: Some(old.clone()),
                segments: vec![],
                formatting_lost: false,
            });
        crate::queue::enqueue_tasks(
            &mut repo,
            vec![(
                crate::queue::Source::Palette {
                    library_ids: vec![ids[0].clone()],
                    colors: 4,
                    format: "hex".into(),
                },
                "Existing result".into(),
            )],
            "existing-result",
        )
        .unwrap();
        repo.data.jobs[0].state = crate::queue::JobState::Completed;
        repo.data.jobs[0].outputs = vec![old.clone()];
        repo.save().unwrap();
        let plan = plan(&repo, &ids[..1]);
        assert_eq!(plan.entries[0].record_ids.len(), 2);
        approve_rename(&repo, &plan.id).unwrap();
        let verified = verify(&repo, &plan.id, false);
        let (_sender, cancel) = watch::channel(false);
        let journal = execute_rename(&mut repo, verified, cancel).unwrap();
        assert_eq!(journal.state, OperationState::Completed);
        let new = &plan.entries[0].new_path;
        assert_eq!(repo.data.library[0].id, ids[0]);
        assert_eq!(repo.data.library[0].path, *new);
        assert_eq!(repo.data.library[2].path, *new);
        assert_eq!(
            repo.data.library[1].details.tracks[0].path,
            Some(new.clone())
        );
        assert_eq!(repo.data.jobs[0].outputs, vec![new.clone()]);
        assert_eq!(fs::read(new).unwrap(), b"first contents");
        assert!(!old.exists());
        fs::write(&old, b"a later unrelated file").unwrap();
        let (_sender, cancel) = watch::channel(false);
        assert!(preflight_rename(
            repo.root(),
            &repo.data.library,
            &plan.id,
            true,
            cancel,
            2,
            |_| {}
        )
        .is_err());
        assert_eq!(fs::read(&old).unwrap(), b"a later unrelated file");
        assert_eq!(fs::read(new).unwrap(), b"first contents");
        fs::remove_file(&old).unwrap();
        let verified = verify(&repo, &plan.id, true);
        let (_sender, cancel) = watch::channel(false);
        let undone = undo_rename(&mut repo, verified, cancel).unwrap();
        assert_eq!(undone.state, OperationState::Undone, "{:?}", undone.error);
        assert_eq!(repo.data.library[0].path, old);
        assert_eq!(repo.data.jobs[0].outputs, vec![old.clone()]);
        assert_eq!(fs::read(&old).unwrap(), b"first contents");
        drop(repo);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn destination_appearing_after_preflight_leaves_a_durable_partial_operation() {
        let (root, mut repo, ids) = setup();
        let plan = plan(&repo, &ids);
        approve_rename(&repo, &plan.id).unwrap();
        let verified = verify(&repo, &plan.id, false);
        fs::write(&plan.entries[1].new_path, b"later file").unwrap();
        let (_sender, cancel) = watch::channel(false);
        let journal = execute_rename(&mut repo, verified, cancel).unwrap();
        assert_eq!(journal.state, OperationState::Partial);
        assert_eq!(journal.entries[0].state, EntryState::Recorded);
        assert_eq!(journal.entries[1].state, EntryState::Pending);
        assert_eq!(fs::read(&plan.entries[1].new_path).unwrap(), b"later file");
        let folder = repo.data.settings.folder.clone();
        drop(repo);
        let mut reopened = Repository::open(&root, &folder).unwrap();
        assert_eq!(
            rename_history(&reopened).unwrap()[0].state,
            OperationState::Partial
        );
        assert_eq!(reopened.data.library[0].path, plan.entries[0].new_path);
        let verified = verify(&reopened, &plan.id, true);
        let (_sender, cancel) = watch::channel(false);
        assert_eq!(
            undo_rename(&mut reopened, verified, cancel).unwrap().state,
            OperationState::Undone
        );
        assert_eq!(
            fs::read(&plan.entries[0].old_path).unwrap(),
            b"first contents"
        );
        assert_eq!(
            fs::read(&plan.entries[1].old_path).unwrap(),
            b"second contents"
        );
        assert_eq!(fs::read(&plan.entries[1].new_path).unwrap(), b"later file");
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn interrupted_move_is_adopted_on_retry_without_a_second_rename() {
        let (root, mut repo, ids) = setup();
        let plan = plan(&repo, &ids[..1]);
        approve_rename(&repo, &plan.id).unwrap();
        let verified = verify(&repo, &plan.id, false);
        move_noreplace(
            verified.files[0].as_ref().unwrap(),
            &plan.entries[0].new_path,
        )
        .unwrap();
        drop(verified);
        let recovered = verify(&repo, &plan.id, false);
        let (_sender, cancel) = watch::channel(false);
        let journal = execute_rename(&mut repo, recovered, cancel).unwrap();
        assert_eq!(journal.state, OperationState::Completed);
        assert_eq!(repo.data.library[0].path, plan.entries[0].new_path);
        let replay = verify(&repo, &plan.id, false);
        let (_sender, cancel) = watch::channel(false);
        assert!(execute_rename(&mut repo, replay, cancel)
            .unwrap()
            .successful());
        drop(repo);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn bounded_regex_templates_refuse_traversal_and_case_insensitive_collisions() {
        let (root, repo, ids) = setup();
        let mut options = configuration();
        options.template = "{name}-{counter}{extension}".into();
        options.replacement = Some(Replacement {
            pattern: "^(first)$".into(),
            replacement: "$1-edited".into(),
            regex: true,
        });
        let (_sender, cancel) = watch::channel(false);
        let plan =
            plan_rename(repo.root(), &repo.data.library, &ids[..1], &options, cancel).unwrap();
        assert_eq!(
            plan.entries[0].new_path.file_name().unwrap(),
            "first-edited-01.txt"
        );
        options.template = "../escape.txt".into();
        let (_sender, cancel) = watch::channel(false);
        assert!(plan_rename(repo.root(), &repo.data.library, &ids, &options, cancel).is_err());
        options.template = "same.txt".into();
        let (_sender, cancel) = watch::channel(false);
        assert!(plan_rename(repo.root(), &repo.data.library, &ids, &options, cancel).is_err());
        assert_eq!(
            fs::read(&repo.data.library[0].path).unwrap(),
            b"first contents"
        );
        drop(repo);
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn preflight_handles_exclude_writers_until_operation_finishes() {
        let (root, repo, ids) = setup();
        let plan = plan(&repo, &ids[..1]);
        approve_rename(&repo, &plan.id).unwrap();
        let verified = verify(&repo, &plan.id, false);
        assert!(fs::OpenOptions::new()
            .write(true)
            .open(&plan.entries[0].old_path)
            .is_err());
        drop(verified);
        assert!(fs::OpenOptions::new()
            .write(true)
            .open(&plan.entries[0].old_path)
            .is_ok());
        drop(repo);
        fs::remove_dir_all(root).unwrap();
    }
}
