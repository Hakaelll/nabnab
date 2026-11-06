use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(crate) struct Workspace {
    pub path: PathBuf,
    root: PathBuf,
}
impl Workspace {
    pub fn new(root: &Path) -> Result<Self, String> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = crate::sessions::secure_module_directory(root, "local-previews")?;
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
