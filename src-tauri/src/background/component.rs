use crate::{components, library, workspaces::Workspace};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::AppHandle;

pub const ID: &str = "background";
const FILE: &str = "u2netp-309c8469.onnx";
const URL: &str = "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx";
const SIZE: u64 = 4_574_861;
const HASH: &str = "309c8469258dda742793dce0ebea8e6dd393174f89934733ecc8b14c76f4ddd8";

fn available() -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64"))
        && std::env::var_os("SystemRoot").is_some_and(|root| {
            PathBuf::from(root)
                .join("System32/Windows.AI.MachineLearning.dll")
                .is_file()
        })
}

pub fn status(root: &Path) -> components::Component {
    components::Component {
        id: ID,
        name: "U²-Net small · Windows ML",
        version: "u2netp · 309c8469",
        size: SIZE,
        installed_size: Some(SIZE),
        license: "Apache-2.0 · Microsoft Windows terms",
        source: URL,
        installed: root.join(FILE).is_file(),
        enabled: root.join("background.enabled").is_file(),
        available: available(),
        note: Some("Windows x64 · Windows ML · CPU"),
    }
}

pub fn verify(path: &Path) -> Result<PathBuf, String> {
    let path = library::authorized_path(path)
        .map_err(|_| "Install the U²-Net model in Settings → Components.")?;
    if crate::portable_zip::regular_file(&path)? != SIZE {
        return Err(
            "The segmentation model has an unexpected size. Remove and reinstall it.".into(),
        );
    }
    let file = fs::File::open(&path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(SIZE as usize);
    file.take(SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 != SIZE || format!("{:x}", Sha256::digest(&bytes)) != HASH {
        return Err(
            "The segmentation model failed its integrity check. Remove and reinstall it.".into(),
        );
    }
    Ok(path)
}

pub fn enabled_model(root: &Path) -> Result<PathBuf, String> {
    if !root.join("background.enabled").is_file() {
        return Err("Enable the U²-Net component in Settings → Components.".into());
    }
    library::authorized_path(&root.join(FILE))
        .map_err(|_| "Install the U²-Net model in Settings → Components.".into())
}

pub async fn install(app: &AppHandle) -> Result<(), String> {
    if !available() {
        return Err("This component requires Windows x64 with Windows ML.".into());
    }
    let root = components::root(app)?;
    let destination = root.join(FILE);
    if destination.exists() {
        return Err("This segmentation model is already installed.".into());
    }
    let workspace = Workspace::new(&root)?;
    let model = workspace.path.join(FILE);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&model)
        .map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(URL)
        .send()
        .await
        .map_err(|_| "The official U²-Net model could not be reached.")?
        .error_for_status()
        .map_err(|_| "The official U²-Net download failed.")?;
    let mut bytes = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Model download interrupted.")?
    {
        bytes += chunk.len() as u64;
        if bytes > SIZE {
            return Err("The U²-Net model exceeds its pinned size.".into());
        }
        file.write_all(&chunk).map_err(|e| e.to_string())?;
    }
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    verify(&model)?;
    fs::rename(model, destination).map_err(|e| e.to_string())?;
    fs::write(root.join("background.enabled"), b"1").map_err(|e| e.to_string())
}

pub fn remove(root: &Path) -> Result<(), String> {
    let path = root.join(FILE);
    if path.exists() {
        fs::remove_file(library::authorized_path(&path)?).map_err(|e| e.to_string())?;
    }
    Ok(())
}
