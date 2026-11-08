//! Read through Windows' mounted filesystem. Only the native picker grants a root.
use super::{Entry, Stamp};
use crate::model::Progress;
use std::path::{Path, PathBuf};
use tokio::sync::watch;

pub(super) fn resume_size(stamp: &Stamp) -> Option<u64> {
    let digest = stamp.etag.as_deref()?.strip_prefix("nas-")?;
    stamp.size.filter(|size| {
        *size > 0 && digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn mounted_folder_copies_reviewed_bytes_and_rejects_changed_file() {
        let fixture = Fixture(std::env::temp_dir().join(format!(
            "nabnab-nas-{}-{}",
            std::process::id(),
            crate::queue::now()
        )));
        fs::create_dir(&fixture.0).unwrap();
        let origin = fixture.0.join("source");
        fs::create_dir(&origin).unwrap();
        let original = origin.join("notes.txt");
        fs::write(&original, b"reviewed notes").unwrap();
        let root = normalize(&origin).unwrap();
        let (_cancel, token) = watch::channel(false);
        let listed = browse(&root, &[], token.clone()).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, ["notes.txt"]);
        let entry = &listed[0];
        let copy = fixture.0.join("copied.txt");
        download(
            &root,
            &entry.path,
            &entry.stamp,
            &copy,
            token.clone(),
            1,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&copy).unwrap(), b"reviewed notes");

        fs::write(&original, b"changed notes with a new size").unwrap();
        let rejected = fixture.0.join("must-not-publish.txt");
        assert!(download(
            &root,
            &entry.path,
            &entry.stamp,
            &rejected,
            token.clone(),
            2,
            |_| {}
        )
        .await
        .is_err());
        assert!(!rejected.exists());
        assert!(browse(&root, &["..".into()], token).await.is_err());
    }

    #[tokio::test]
    async fn mounted_copy_continues_at_the_saved_offset_and_discards_a_replaced_version() {
        let fixture = Fixture(std::env::temp_dir().join(format!(
            "nabnab-nas-resume-{}-{}",
            std::process::id(),
            crate::queue::now()
        )));
        let origin = fixture.0.join("source");
        fs::create_dir_all(&origin).unwrap();
        let file = origin.join("notes.txt");
        let content: Vec<u8> = (0..210_000).map(|index| (index % 251) as u8).collect();
        fs::write(&file, &content).unwrap();
        let root = normalize(&origin).unwrap();
        let (_idle, ready) = watch::channel(false);
        let entry = browse(&root, &[], ready.clone()).await.unwrap().remove(0);
        let partial = fixture.0.join("copy.part");
        let (pause, copying) = watch::channel(false);
        let result = download(
            &root,
            &entry.path,
            &entry.stamp,
            &partial,
            copying,
            1,
            move |progress| {
                if progress.downloaded == Some(65_536.0) {
                    let _ = pause.send(true);
                }
            },
        )
        .await;
        assert_eq!(result.unwrap_err(), crate::engine::CANCELLED);
        assert_eq!(fs::read(&partial).unwrap(), content[..65_536]);

        let offline = fixture.0.join("offline");
        fs::rename(&origin, &offline).unwrap();
        let error = download(
            &root,
            &entry.path,
            &entry.stamp,
            &partial,
            ready.clone(),
            2,
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(error.starts_with(crate::personal_servers::MOUNTED_INTERRUPTED));
        assert_eq!(fs::read(&partial).unwrap(), content[..65_536]);
        fs::rename(&offline, &origin).unwrap();

        let (observe, first) = std::sync::mpsc::channel();
        download(
            &root,
            &entry.path,
            &entry.stamp,
            &partial,
            ready.clone(),
            3,
            move |progress| {
                let _ = observe.send(progress.downloaded);
            },
        )
        .await
        .unwrap();
        assert_eq!(first.recv().unwrap(), Some(65_536.0));
        assert_eq!(fs::read(&partial).unwrap(), content);

        fs::write(&partial, &content[..65_536]).unwrap();
        let reviewed_time = fs::metadata(&file).unwrap().modified().unwrap();
        fs::remove_file(&file).unwrap();
        fs::write(&file, vec![42; content.len()]).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(reviewed_time)
            .unwrap();
        let actual = browse(&root, &[], ready.clone()).await.unwrap().remove(0);
        assert_eq!(actual.stamp.size, entry.stamp.size);
        assert_eq!(actual.stamp.modified, entry.stamp.modified);
        let error = download(&root, &entry.path, &entry.stamp, &partial, ready, 4, |_| {})
            .await
            .unwrap_err();
        assert_eq!(error, crate::personal_servers::MOUNTED_CHANGED);
        assert!(!partial.exists());
    }
}

#[cfg(windows)]
pub(super) fn normalize(root: &Path) -> Result<PathBuf, String> {
    windows::normalize(root)
}

#[cfg(windows)]
pub(super) async fn browse(
    root: &Path,
    relative: &[String],
    cancel: watch::Receiver<bool>,
) -> Result<Vec<Entry>, String> {
    let (root, relative) = (root.to_owned(), relative.to_owned());
    tokio::task::spawn_blocking(move || windows::browse(&root, &relative, &cancel))
        .await
        .map_err(|_| "The mounted folder inspection stopped unexpectedly.")?
}

#[cfg(windows)]
pub(super) async fn download(
    root: &Path,
    relative: &[String],
    expected: &Stamp,
    destination: &Path,
    cancel: watch::Receiver<bool>,
    run_id: u64,
    emit: impl Fn(Progress) + Send + 'static,
) -> Result<(), String> {
    let (root, relative, expected, destination) = (
        root.to_owned(),
        relative.to_owned(),
        expected.clone(),
        destination.to_owned(),
    );
    tokio::task::spawn_blocking(move || {
        windows::download(
            &root,
            &relative,
            &expected,
            &destination,
            &cancel,
            run_id,
            emit,
        )
    })
    .await
    .map_err(|_| "The mounted file copy stopped unexpectedly.")?
}

#[cfg(not(windows))]
pub(super) fn normalize(_: &Path) -> Result<PathBuf, String> {
    Err("Mounted folders are supported in the Windows build.".into())
}

#[cfg(not(windows))]
pub(super) async fn browse(
    _: &Path,
    _: &[String],
    _: watch::Receiver<bool>,
) -> Result<Vec<Entry>, String> {
    Err("Mounted folders are supported in the Windows build.".into())
}

#[cfg(not(windows))]
pub(super) async fn download(
    _: &Path,
    _: &[String],
    _: &Stamp,
    _: &Path,
    _: watch::Receiver<bool>,
    _: u64,
    _: impl Fn(Progress) + Send + 'static,
) -> Result<(), String> {
    Err("Mounted folders are supported in the Windows build.".into())
}

#[cfg(windows)]
mod windows {
    use super::*;
    use crate::{
        direct,
        engine::CANCELLED,
        personal_servers::{copy_progress, MOUNTED_CHANGED as CHANGED, MOUNTED_INTERRUPTED},
    };
    use sha2::{Digest, Sha256};
    use std::{
        ffi::OsString,
        fs::{self, File, Metadata, OpenOptions},
        io::{Read, Seek, SeekFrom, Write},
        mem::size_of,
        os::windows::{
            ffi::OsStringExt,
            fs::{MetadataExt, OpenOptionsExt},
            io::AsRawHandle,
        },
        path::{Component, Prefix},
        time::{Duration, Instant, UNIX_EPOCH},
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_NOT_CONNECTED, NO_ERROR},
        NetworkManagement::WNet::WNetGetConnectionW,
        Storage::FileSystem::{
            FileBasicInfo, FileIdInfo, GetFileInformationByHandleEx, GetFileType,
            GetFinalPathNameByHandleW, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO,
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
            FILE_LIST_DIRECTORY, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
            FILE_SHARE_WRITE, FILE_TYPE_DISK, VOLUME_NAME_DOS,
        },
    };

    const MAX_CHILDREN: usize = 512;
    const MAX_DEPTH: usize = 64;
    const UNAVAILABLE: &str =
        "The mounted folder is unavailable. Reconnect it in Windows and try again.";
    const UNSAFE_PATH: &str = "Choose a regular Windows folder and stay within its approved root.";

    fn valid_name(name: &str) -> bool {
        let stem = name
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end()
            .to_ascii_uppercase();
        !name.is_empty()
            && name.encode_utf16().count() <= 255
            && !name.ends_with(['.', ' '])
            && !name
                .chars()
                .any(|ch| ch.is_control() || "<>:\"/\\|?*".contains(ch))
            && !matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            )
            && !["COM", "LPT"].iter().any(|prefix| {
                stem.strip_prefix(prefix).is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                })
            })
    }

    fn path_prefix(path: &Path) -> Result<Prefix<'_>, String> {
        let mut components = path.components();
        let Some(Component::Prefix(prefix)) = components.next() else {
            return Err(UNSAFE_PATH.into());
        };
        if components.next() != Some(Component::RootDir)
            || components.any(|component| !matches!(component, Component::Normal(name) if name.to_str().is_some_and(valid_name)))
        {
            return Err(UNSAFE_PATH.into());
        }
        match prefix.kind() {
            Prefix::Disk(_) | Prefix::VerbatimDisk(_) => Ok(prefix.kind()),
            Prefix::UNC(host, share) | Prefix::VerbatimUNC(host, share)
                if host.to_str().is_some_and(valid_name)
                    && share.to_str().is_some_and(|name| {
                        valid_name(name)
                            && !matches!(
                                name.to_ascii_lowercase().as_str(),
                                "ipc$" | "pipe" | "mailslot"
                            )
                    }) =>
            {
                Ok(prefix.kind())
            }
            _ => Err(UNSAFE_PATH.into()),
        }
    }

    fn pin_network_drive(path: &Path) -> Result<PathBuf, String> {
        let (Prefix::Disk(drive) | Prefix::VerbatimDisk(drive)) = path_prefix(path)? else {
            return Ok(path.to_owned());
        };
        let local = [u16::from(drive), u16::from(b':'), 0];
        let mut remote = vec![0u16; 32768];
        let mut len = remote.len() as u32;
        let status = unsafe { WNetGetConnectionW(local.as_ptr(), remote.as_mut_ptr(), &mut len) };
        if status == ERROR_NOT_CONNECTED {
            return Ok(path.to_owned());
        }
        if status != NO_ERROR {
            return Err(UNAVAILABLE.into());
        }
        let end = remote
            .iter()
            .position(|unit| *unit == 0)
            .ok_or(UNSAFE_PATH)?;
        let mut unc = PathBuf::from(OsString::from_wide(&remote[..end]));
        if !matches!(
            path_prefix(&unc)?,
            Prefix::UNC(..) | Prefix::VerbatimUNC(..)
        ) {
            return Err(UNSAFE_PATH.into());
        }
        unc.extend(
            path.components()
                .skip(2)
                .map(|component| component.as_os_str()),
        );
        Ok(unc)
    }

    fn regular(metadata: &Metadata) -> bool {
        (metadata.is_file() || metadata.is_dir())
            && !metadata.file_type().is_symlink()
            && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }

    #[derive(Clone, Copy)]
    enum Access {
        Directory,
        Inspect,
        Copy,
    }

    fn open(path: &Path, access: Access) -> Result<(File, Metadata), String> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
        match access {
            // Holding directory handles without delete sharing prevents rename during traversal.
            Access::Directory => options
                .access_mode(FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE),
            Access::Inspect => options
                .access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE),
            Access::Copy => options.share_mode(FILE_SHARE_READ),
        };
        let file = options.open(path).map_err(|_| UNAVAILABLE)?;
        let metadata = file.metadata().map_err(|_| UNAVAILABLE)?;
        if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK
            || !regular(&metadata)
            || (matches!(access, Access::Directory) != metadata.is_dir())
        {
            return Err(UNSAFE_PATH.into());
        }
        Ok((file, metadata))
    }

    fn resolved_path(file: &File) -> Result<PathBuf, String> {
        let mut buffer = vec![0u16; 512];
        loop {
            let len = unsafe {
                GetFinalPathNameByHandleW(
                    file.as_raw_handle(),
                    buffer.as_mut_ptr(),
                    buffer.len() as u32,
                    FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
                )
            } as usize;
            if len == 0 || len > 32768 {
                return Err("Windows could not verify the resolved mounted path.".into());
            }
            if len < buffer.len() {
                let path = PathBuf::from(OsString::from_wide(&buffer[..len]));
                path_prefix(&path)?;
                return Ok(path);
            }
            buffer.resize(len + 1, 0);
        }
    }

    fn guarded_root(root: &Path) -> Result<Vec<File>, String> {
        let ancestors: Vec<_> = root.ancestors().collect();
        if ancestors.len() > MAX_DEPTH {
            return Err("The mounted folder path is too deep.".into());
        }
        ancestors
            .into_iter()
            .rev()
            .map(|path| open(path, Access::Directory).map(|(file, _)| file))
            .collect()
    }

    pub(super) fn normalize(path: &Path) -> Result<PathBuf, String> {
        let path = pin_network_drive(path)?;
        let guards = guarded_root(&path)?;
        let root = resolved_path(guards.last().ok_or(UNSAFE_PATH)?)?;
        if pin_network_drive(&root)? != root {
            return Err("Windows could not pin this drive to its network share.".into());
        }
        Ok(root)
    }

    struct Folder {
        path: PathBuf,
        _guards: Vec<File>,
    }

    fn folder(root: &Path, relative: &[String]) -> Result<Folder, String> {
        if relative.len() >= MAX_DEPTH || relative.iter().any(|name| !valid_name(name)) {
            return Err(UNSAFE_PATH.into());
        }
        if pin_network_drive(root)? != root {
            return Err(
                "The mounted drive mapping changed. Select the approved share again.".into(),
            );
        }
        let mut guards = guarded_root(root)?;
        if resolved_path(guards.last().ok_or(UNSAFE_PATH)?)? != root {
            return Err("The mounted folder no longer resolves to its approved root.".into());
        }
        let mut path = root.to_owned();
        for component in relative {
            path.push(component);
            let (handle, _) = open(&path, Access::Directory)?;
            if resolved_path(&handle)? != path {
                return Err(UNSAFE_PATH.into());
            }
            guards.push(handle);
        }
        Ok(Folder {
            path,
            _guards: guards,
        })
    }

    fn version(file: &File) -> Option<String> {
        let handle = file.as_raw_handle();
        let mut basic = FILE_BASIC_INFO::default();
        let mut identity = FILE_ID_INFO::default();
        if unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileBasicInfo,
                (&mut basic as *mut FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        } == 0
            || unsafe {
                GetFileInformationByHandleEx(
                    handle,
                    FileIdInfo,
                    (&mut identity as *mut FILE_ID_INFO).cast(),
                    size_of::<FILE_ID_INFO>() as u32,
                )
            } == 0
            || identity.FileId.Identifier == [0; 16]
            || basic.ChangeTime <= 0
            || basic.LastWriteTime <= 0
        {
            return None;
        }
        let mut hash = Sha256::new();
        hash.update(identity.VolumeSerialNumber.to_le_bytes());
        hash.update(identity.FileId.Identifier);
        hash.update(basic.ChangeTime.to_le_bytes());
        hash.update(basic.LastWriteTime.to_le_bytes());
        Some(format!("nas-{hash:x}", hash = hash.finalize()))
    }

    fn stamp(metadata: &Metadata, file: Option<&File>) -> Stamp {
        let modified = metadata
            .modified()
            .ok()
            .filter(|time| {
                time.duration_since(UNIX_EPOCH)
                    .is_ok_and(|age| age.as_secs() < 253_402_300_800)
            })
            .map(httpdate::fmt_http_date);
        Stamp {
            size: metadata.is_file().then(|| metadata.len()),
            modified,
            etag: file.and_then(version),
        }
    }

    fn cancelled(cancel: &watch::Receiver<bool>) -> Result<(), String> {
        if *cancel.borrow() {
            Err(crate::engine::CANCELLED.into())
        } else {
            Ok(())
        }
    }

    pub(super) fn browse(
        root: &Path,
        relative: &[String],
        cancel: &watch::Receiver<bool>,
    ) -> Result<Vec<Entry>, String> {
        cancelled(cancel)?;
        let folder = folder(root, relative)?;
        let mut entries = Vec::new();
        for (index, item) in fs::read_dir(&folder.path)
            .map_err(|_| UNAVAILABLE)?
            .enumerate()
        {
            cancelled(cancel)?;
            if index == MAX_CHILDREN {
                return Err(format!("This folder contains more than {MAX_CHILDREN} entries. Select a narrower mounted folder."));
            }
            let item = item.map_err(|_| UNAVAILABLE)?;
            let Some(name) = item
                .file_name()
                .into_string()
                .ok()
                .filter(|name| valid_name(name))
            else {
                continue;
            };
            let metadata = item.metadata().map_err(|_| UNAVAILABLE)?;
            if !regular(&metadata) {
                continue;
            }
            let directory = metadata.is_dir();
            let item_stamp = if directory {
                stamp(&metadata, None)
            } else if let Ok((file, opened_metadata)) = open(&item.path(), Access::Inspect) {
                if resolved_path(&file).ok().as_ref() != Some(&item.path()) {
                    continue;
                }
                stamp(&opened_metadata, Some(&file))
            } else {
                stamp(&metadata, None)
            };
            let mut path = relative.to_owned();
            path.push(name);
            entries.push(Entry {
                path,
                directory,
                stamp: item_stamp,
                mime: None,
            });
        }
        cancelled(cancel)?;
        entries.sort_by(|a, b| {
            b.directory
                .cmp(&a.directory)
                .then_with(|| a.path.cmp(&b.path))
        });
        Ok(entries)
    }

    pub(super) fn download(
        root: &Path,
        relative: &[String],
        expected: &Stamp,
        destination: &Path,
        cancel: &watch::Receiver<bool>,
        run_id: u64,
        emit: impl Fn(Progress),
    ) -> Result<(), String> {
        cancelled(cancel)?;
        let size = expected.size.filter(|_| expected.etag.is_some())
            .ok_or("This mounted file has no reliable version information. It can only be indexed as a reference.")?;
        let offset = if resume_size(expected).is_some() {
            direct::partial_len(destination)?
                .filter(|offset| *offset < size)
                .unwrap_or(0)
        } else {
            0
        };
        let progress = |written: u64| emit(copy_progress(run_id, size, written));
        let prepared = (|| {
            let (name, parents) = relative.split_last().ok_or(UNSAFE_PATH)?;
            if !valid_name(name) {
                return Err(UNSAFE_PATH.into());
            }
            let folder = folder(root, parents)?;
            let path = folder.path.join(name);
            let (mut source, metadata) = open(&path, Access::Copy)?;
            if resolved_path(&source)? != path || &stamp(&metadata, Some(&source)) != expected {
                return Err(CHANGED.into());
            }
            if offset > 0 {
                source
                    .seek(SeekFrom::Start(offset))
                    .map_err(|_| UNAVAILABLE)?;
            }
            Ok::<_, String>((folder, path, source))
        })();
        let (_folder, path, mut source) = match prepared {
            Ok(prepared) => prepared,
            Err(error) if error == UNAVAILABLE && offset > 0 => {
                progress(offset);
                cancelled(cancel)?;
                return Err(format!("{MOUNTED_INTERRUPTED} {error}"));
            }
            Err(error) => {
                let _ = fs::remove_file(destination);
                return Err(error);
            }
        };
        cancelled(cancel)?;
        let mut output = direct::partial_file(destination, offset)
            .map_err(|_| "Could not create the staged mounted file.")?;
        let mut written = offset;
        progress(written);
        let result = (|| {
            let mut buffer = [0u8; 64 * 1024];
            let mut last = Instant::now();
            loop {
                cancelled(cancel)?;
                let count = source.read(&mut buffer).map_err(|_| UNAVAILABLE)?;
                if count == 0 {
                    break;
                }
                let next = written
                    .checked_add(count as u64)
                    .filter(|total| *total <= size)
                    .ok_or(CHANGED)?;
                output
                    .write_all(&buffer[..count])
                    .map_err(|_| "Could not write the staged mounted file.")?;
                if written == offset || last.elapsed() >= Duration::from_millis(200) {
                    progress(next);
                    last = Instant::now();
                }
                written = next;
            }
            output
                .sync_all()
                .map_err(|_| "Could not finish the staged mounted file.")?;
            let after = source.metadata().map_err(|_| UNAVAILABLE)?;
            if written != size
                || &stamp(&after, Some(&source)) != expected
                || resolved_path(&source)? != path
            {
                return Err(CHANGED.into());
            }
            cancelled(cancel)
        })();
        let keep = result
            .as_ref()
            .is_err_and(|error| matches!(error.as_str(), UNAVAILABLE | CANCELLED))
            && resume_size(expected).is_some_and(|size| written > 0 && written < size)
            && output.sync_all().is_ok();
        drop(output);
        if keep {
            progress(written);
            return if *cancel.borrow() {
                Err(CANCELLED.into())
            } else {
                Err(format!("{MOUNTED_INTERRUPTED} {UNAVAILABLE}"))
            };
        }
        if result.is_err() {
            let _ = fs::remove_file(destination);
        } else {
            progress(written);
        }
        result
    }
}
