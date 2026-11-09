use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const DIRECTORY: &str = "local-previews";

// The caller holds the repository lock and has yet to start any workers.
pub(crate) fn cleanup(root: &Path) -> Result<(), String> {
    let directory = root.join(DIRECTORY);
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
        Ok(_) => {}
    }
    let directory = crate::library::authorized_path(&directory)?;
    let entries = fs::read_dir(directory).map_err(|e| e.to_string())?;
    let mut result = Ok(());
    for entry in entries {
        if let Err(error) = entry.and_then(|entry| {
            let name = entry.file_name();
            let Some((time, sequence)) = name.to_str().and_then(|name| name.split_once('-')) else {
                return Ok(());
            };
            if [time, sequence]
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            {
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.is_dir() && !crate::library::is_link(&metadata) {
                    fs::remove_dir_all(path)?;
                }
            }
            Ok(())
        }) {
            if error.kind() != std::io::ErrorKind::NotFound {
                result = Err(format!("Could not remove abandoned tool previews: {error}"));
            }
        }
    }
    result
}

pub(crate) struct Workspace {
    pub path: PathBuf,
    root: PathBuf,
}
impl Workspace {
    pub fn new(root: &Path) -> Result<Self, String> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = crate::sessions::secure_module_directory(root, DIRECTORY)?;
        let path = root.join(format!(
            "{}-{}",
            crate::queue::now(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).map_err(|e| e.to_string())?;
        Ok(Self { path, root })
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let (Ok(path), Ok(root)) = (fs::canonicalize(&self.path), fs::canonicalize(&self.root))
        else {
            return;
        };
        if path.parent() == Some(root.as_path()) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
