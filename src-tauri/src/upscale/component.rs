use crate::{components, library, portable_collections::package, workspaces::Workspace};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::AppHandle;
use tokio::sync::watch;

pub const ID: &str = "upscale";
pub const VERSION: &str = "2022-04-24";
pub const URL: &str = "https://github.com/xinntao/Real-ESRGAN/releases/download/v0.2.5.0/realesrgan-ncnn-vulkan-20220424-windows.zip";
pub const DOWNLOAD_BYTES: u64 = 45_474_481;
const ARCHIVE_SHA: &str = "abc02804e17982a3be33675e4d471e91ea374e65b70167abc09e31acb412802d";
const DIRECTORY: &str = "upscale-2022-04-24";
const EXE: &str = "realesrgan-ncnn-vulkan.exe";
const FILES: [(&str, u64, &str); 10] = [
    (
        EXE,
        6_161_408,
        "07e49f7cbb4ede01ae4dd4c399d3a7e5846e3d2085c3128eff881e55cb7b1a0c",
    ),
    (
        "vcomp140.dll",
        182_704,
        "8f72ef2e483465444b2059fc6744d6cb22cd8d8a27f6fa56befd2a42dcd0f78b",
    ),
    (
        "models/realesrgan-x4plus.param",
        116_029,
        "35330ececcea33b6c397a72548e788d5d53becee4734c50b7fada36e89f10a86",
    ),
    (
        "models/realesrgan-x4plus.bin",
        33_424_520,
        "713ee713b0353afaa27976f0563a64a5043bd70b9bd8936c2e26e25ebcdbcddf",
    ),
    (
        "models/realesr-animevideov3-x2.param",
        3_173,
        "b88ff4f00ebf019a7fdac17fdd45a7fd3665d37509efc5baf2e4da2e24420a04",
    ),
    (
        "models/realesr-animevideov3-x2.bin",
        1_247_368,
        "548a36f9c3f4ab8da56cd3b13badf23968bee207b396dad14d04b830e5f2ab2d",
    ),
    (
        "models/realesr-animevideov3-x3.param",
        3_173,
        "d1a5755008791d09b57e3425fc9dd0bd26b00fdf79c606210bc0e693f8230881",
    ),
    (
        "models/realesr-animevideov3-x3.bin",
        1_247_368,
        "548a36f9c3f4ab8da56cd3b13badf23968bee207b396dad14d04b830e5f2ab2d",
    ),
    (
        "models/realesr-animevideov3-x4.param",
        3_077,
        "850a248e7c14c27e5bd8cf7265113a9441036a7db63963bb8aa5169d788a435e",
    ),
    (
        "models/realesr-animevideov3-x4.bin",
        1_247_368,
        "548a36f9c3f4ab8da56cd3b13badf23968bee207b396dad14d04b830e5f2ab2d",
    ),
];

pub fn status(root: &Path) -> components::Component {
    components::Component {
        id: ID,
        name: "Real-ESRGAN · ncnn Vulkan",
        version: VERSION,
        size: DOWNLOAD_BYTES,
        installed_size: Some(FILES.iter().map(|entry| entry.1).sum()),
        license: "MIT · BSD-3-Clause · Microsoft OpenMP",
        source: URL,
        installed: root.join(DIRECTORY).join(EXE).is_file(),
        enabled: root.join("upscale.enabled").is_file(),
        available: cfg!(all(target_os = "windows", target_arch = "x86_64")),
        note: Some("Windows x64 · Vulkan GPU"),
    }
}

fn verify_files(directory: &Path, model: Option<&str>) -> Result<PathBuf, String> {
    let directory = library::authorized_path(directory)
        .map_err(|_| "Install Real-ESRGAN in Settings → Components.")?;
    let mut buffer = [0u8; 64 * 1024];
    for (name, size, expected) in FILES {
        if let (Some(model), Some(filename)) = (model, name.strip_prefix("models/")) {
            if !filename
                .strip_prefix(model)
                .is_some_and(|suffix| matches!(suffix, ".bin" | ".param"))
            {
                continue;
            }
        }
        let file = library::authorized_path(&directory.join(name))
            .map_err(|_| "Real-ESRGAN files are incomplete. Remove and reinstall the component.")?;
        let mut file = fs::File::open(file).map_err(|error| error.to_string())?;
        if file.metadata().map_err(|error| error.to_string())?.len() != size {
            return Err("A Real-ESRGAN file changed. Remove and reinstall the component.".into());
        }
        let mut hash = Sha256::new();
        let mut read = 0u64;
        loop {
            let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            read += count as u64;
            if read > size {
                return Err("A Real-ESRGAN file changed during verification.".into());
            }
            hash.update(&buffer[..count]);
        }
        if read != size || format!("{:x}", hash.finalize()) != expected {
            return Err("A Real-ESRGAN file failed its integrity check. Remove and reinstall the component.".into());
        }
    }
    Ok(directory)
}

pub fn ready(root: &Path, model: &str) -> Result<PathBuf, String> {
    if !root.join("upscale.enabled").is_file() {
        return Err("Enable Real-ESRGAN in Settings → Components.".into());
    }
    verify_files(&root.join(DIRECTORY), Some(model))
}

pub fn executable(directory: &Path) -> PathBuf {
    directory.join(EXE)
}

async fn activate(root: &Path, archive: &Path, workspace: &Workspace) -> Result<(), String> {
    let destination = root.join(DIRECTORY);
    if destination.exists() {
        return Err("This Real-ESRGAN version is already present.".into());
    }
    let extracted = workspace.path.join("component");
    fs::create_dir(&extracted).map_err(|error| error.to_string())?;
    let names = FILES
        .iter()
        .map(|entry| entry.0.to_owned())
        .collect::<Vec<_>>();
    let (_sender, cancel) = watch::channel(false);
    package::personal_extract(root, archive, &extracted, &names, cancel).await?;
    verify_files(&extracted, None)?;
    fs::rename(&extracted, destination).map_err(|error| error.to_string())?;
    fs::write(root.join("upscale.enabled"), b"1").map_err(|error| error.to_string())
}

pub async fn install(app: &AppHandle) -> Result<(), String> {
    if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return Err("This Real-ESRGAN build requires Windows x64 with Vulkan.".into());
    }
    let root = components::root(app)?;
    if root.join(DIRECTORY).exists() {
        return Err("This Real-ESRGAN version is already present.".into());
    }
    let workspace = Workspace::new(&root)?;
    let archive = workspace.path.join("realesrgan.zip");
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&archive)
        .map_err(|error| error.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| error.to_string())?;
    let mut response = client
        .get(URL)
        .send()
        .await
        .map_err(|_| "The official Real-ESRGAN package could not be reached.")?
        .error_for_status()
        .map_err(|_| "The official Real-ESRGAN download failed.")?;
    let mut length = 0u64;
    let mut hash = Sha256::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Real-ESRGAN transfer interrupted.")?
    {
        length += chunk.len() as u64;
        if length > DOWNLOAD_BYTES {
            return Err("Real-ESRGAN exceeded its pinned download size.".into());
        }
        hash.update(&chunk);
        output
            .write_all(&chunk)
            .map_err(|error| error.to_string())?;
    }
    if length != DOWNLOAD_BYTES || format!("{:x}", hash.finalize()) != ARCHIVE_SHA {
        return Err("The Real-ESRGAN archive failed its pinned integrity check.".into());
    }
    output.sync_all().map_err(|error| error.to_string())?;
    drop(output);
    activate(&root, &archive, &workspace).await
}

pub fn remove(root: &Path) -> Result<(), String> {
    let directory = root.join(DIRECTORY);
    if !directory.exists() {
        return Ok(());
    }
    let directory = library::authorized_path(&directory)?;
    let models = directory.join("models");
    if models.exists() {
        library::authorized_path(&models)?;
    }
    for (name, _, _) in FILES {
        let file = directory.join(name);
        if file.exists() {
            fs::remove_file(library::authorized_path(&file)?).map_err(|error| error.to_string())?;
        }
    }
    if models.exists() {
        fs::remove_dir(models)
            .map_err(|_| "The model directory contains other files. They were preserved.")?;
    }
    fs::remove_dir(directory)
        .map_err(|_| "The Real-ESRGAN directory contains other files. They were preserved.".into())
}

#[cfg(test)]
pub async fn provision(root: &Path, archive: &Path) -> Result<(), String> {
    let bytes = fs::read(archive).map_err(|error| error.to_string())?;
    if bytes.len() as u64 != DOWNLOAD_BYTES
        || format!("{:x}", Sha256::digest(&bytes)) != ARCHIVE_SHA
    {
        return Err("Invalid fixture archive.".into());
    }
    let workspace = Workspace::new(root)?;
    activate(root, archive, &workspace).await
}
