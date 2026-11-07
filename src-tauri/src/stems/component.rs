use crate::{components, engine, library, queue, workspaces::Workspace};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::AppHandle;
use tokio::sync::watch;

pub const ID: &str = "stems";
pub const VERSION: &str = "Spleeter 2 stems FP16 · sherpa-onnx 2.0.1";
const DIRECTORY: &str = "stems-sherpa-2-0-1";
const RUNTIME_PREFIX: &str = "sherpa-onnx-v2.0.1-win-x64-shared-MD-MinSizeRel-no-tts-lib/lib";
const MODEL_PREFIX: &str = "sherpa-onnx-spleeter-2stems-fp16";
const RUNTIME: (&str, u64, &str) = (
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/sherpa-onnx-v2.0.1-win-x64-shared-MD-MinSizeRel-no-tts-lib.tar.bz2",
    5_870_406,
    "eb36c8e09fdfbeb61abea7e64d8084e7b5fa779bc7405bf7000a4b3c431c23fd",
);
const MODEL: (&str, u64, &str) = (
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/source-separation-models/sherpa-onnx-spleeter-2stems-fp16.tar.bz2",
    35_271_738,
    "d54561979bd2e08a51e7dbd99ac36bb47564e089eefd403636dbca93e811bba2",
);

pub const LIBRARY: &str = "sherpa-onnx-c-api.dll";
pub const VOCALS: &str = "vocals.fp16.onnx";
pub const INSTRUMENTAL: &str = "accompaniment.fp16.onnx";
const FILES: [(&str, u64, &str); 5] = [
    (
        LIBRARY,
        2_829_824,
        "b3a0bcf2d7f6da61563ff941fdecc27938e22be67e2cb8617078659c45927af5",
    ),
    (
        "onnxruntime.dll",
        14_203_904,
        "9ab1e925c610b42410b1ebacc61306436282522221bf3b3c6ea5950d000d44be",
    ),
    (
        "onnxruntime_providers_shared.dll",
        10_752,
        "3ef52275f70976fd0ac83f6a4e516cad045622851d5d4ce692f0926333cc8062",
    ),
    (
        VOCALS,
        19_681_017,
        "24cef84aedcd1fe87c0b743ef3370ad34dc1fabf6c9014d6128a75a538c7b668",
    ),
    (
        INSTRUMENTAL,
        19_681_024,
        "d14cea55793cc531a5875f5f4da08207d1c5ab9292e8e0099a104eecb014fcc0",
    ),
];

fn tar() -> Option<PathBuf> {
    if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return None;
    }
    let path = PathBuf::from(std::env::var_os("SystemRoot")?).join("System32/tar.exe");
    path.is_file().then_some(path)
}

pub fn status(root: &Path) -> components::Component {
    let directory = root.join(DIRECTORY);
    components::Component {
        id: ID,
        name: "Spleeter · sherpa-onnx",
        version: VERSION,
        size: RUNTIME.1 + MODEL.1,
        installed_size: Some(FILES.iter().map(|file| file.1).sum()),
        license: "Spleeter MIT · sherpa-onnx Apache-2.0 · ONNX Runtime MIT",
        source: MODEL.0,
        installed: FILES
            .iter()
            .all(|(name, _, _)| directory.join(name).is_file()),
        enabled: root.join("stems.enabled").is_file(),
        available: tar().is_some(),
        note: Some("Windows x64 · CPU · 2 pistas / 2 stems · Microsoft Visual C++ Runtime"),
    }
}

pub fn enabled(root: &Path) -> Result<PathBuf, String> {
    let status = status(root);
    if !status.installed || !status.enabled {
        return Err("Install and enable Spleeter in Settings → Components.".into());
    }
    library::authorized_path(&root.join(DIRECTORY))
}

pub fn verify(root: &Path) -> Result<(), String> {
    let (_sender, cancel) = watch::channel(false);
    for (name, size, hash) in FILES {
        let file = library::authorized_path(&root.join(name))?;
        if crate::portable_zip::regular_file(&file)? != size
            || queue::file_hash(&file, &cancel)? != hash
        {
            return Err("The Spleeter files failed their integrity check. Remove and reinstall the component.".into());
        }
    }
    Ok(())
}

async fn download(
    client: &reqwest::Client,
    package: (&str, u64, &str),
    path: &Path,
) -> Result<(), String> {
    let (url, size, expected) = package;
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| "The official Spleeter package could not be downloaded.")?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut received = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The component download was interrupted.")?
    {
        received += chunk.len() as u64;
        if received > size {
            return Err("The component exceeds its pinned size.".into());
        }
        file.write_all(&chunk).map_err(|error| error.to_string())?;
        hash.update(&chunk);
    }
    if received != size || format!("{:x}", hash.finalize()) != expected {
        return Err("The component download failed its integrity check.".into());
    }
    file.sync_all().map_err(|error| error.to_string())
}

pub async fn install(app: &AppHandle) -> Result<(), String> {
    let tar = tar().ok_or("This component requires Windows x64 with the system tar tool.")?;
    let root = components::root(app)?;
    let workspace = Workspace::new(&root)?;
    let staged = workspace.path.join(DIRECTORY);
    let destination = root.join(DIRECTORY);
    if fs::symlink_metadata(&destination).is_ok() {
        return Err("Remove the existing Spleeter component before installing it again.".into());
    }
    fs::create_dir(&staged).map_err(|error| error.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| error.to_string())?;
    let (_sender, cancel) = watch::channel(false);
    for (package, prefix, strip, files) in [
        (RUNTIME, RUNTIME_PREFIX, "2", &FILES[..3]),
        (MODEL, MODEL_PREFIX, "1", &FILES[3..]),
    ] {
        let archive = workspace.path.join("component.tar.bz2");
        download(&client, package, &archive).await?;
        let mut args = vec![
            "-xf".into(),
            archive.to_string_lossy().into_owned(),
            "-C".into(),
            staged.to_string_lossy().into_owned(),
            "--strip-components".into(),
            strip.into(),
        ];
        args.extend(files.iter().map(|(name, _, _)| format!("{prefix}/{name}")));
        engine::run_process(tar.clone(), args, cancel.clone(), 0, false, |_| {}).await?;
        fs::remove_file(archive).map_err(|error| error.to_string())?;
    }
    let verified = staged.clone();
    tokio::task::spawn_blocking(move || verify(&verified))
        .await
        .map_err(|error| error.to_string())??;
    fs::rename(staged, destination).map_err(|error| error.to_string())?;
    fs::write(root.join("stems.enabled"), b"1").map_err(|error| error.to_string())
}

pub fn remove(root: &Path) -> Result<(), String> {
    let directory = root.join(DIRECTORY);
    if fs::symlink_metadata(&directory).is_err() {
        return Ok(());
    }
    let directory = library::authorized_path(&directory)?;
    for (name, _, _) in FILES {
        let file = directory.join(name);
        if fs::symlink_metadata(&file).is_ok() {
            fs::remove_file(library::authorized_path(&file)?).map_err(|error| error.to_string())?;
        }
    }
    fs::remove_dir(directory).map_err(|error| error.to_string())
}
