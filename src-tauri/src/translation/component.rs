use crate::{components, engine, library, queue, sessions, workspaces::Workspace};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::AppHandle;
use tokio::sync::{watch, Semaphore, SemaphorePermit};

const DIRECTORY: &str = "translation-bergamot-0-4-9";
const RUNTIME: (&str, u64, &str) = (
    "https://registry.npmjs.org/@browsermt/bergamot-translator/-/bergamot-translator-0.4.9.tgz",
    1_852_075,
    "9011be93222d839d7448ffdf00549d53ce8f541fd782ffc79779d1756397c41f",
);
const REGISTRY: &str = "https://bergamot.s3.amazonaws.com/models/index.json";
const VOCAB: (&str, u64, &str) = (
    "https://bergamot.s3.amazonaws.com/models/esen/vocab.esen.spm",
    825_463,
    "909b1eea1face0d7f90a474fe29a8c0fef8d104b6e41e65616f864c964ba8845",
);
const VOCAB_FILE: &str = "vocab.esen.spm";
const VOCAB_TRANSFER: u64 = 415_686;
const RUNTIME_FILES: [(&str, u64, &str); 3] = [
    (
        "bergamot-translator-worker.js",
        80_474,
        "748b2418418a2ffc6e70721aeb10098d8e6fb589ea156b91f4ae8bc3490d8f7a",
    ),
    (
        "bergamot-translator-worker.wasm",
        5_174_294,
        "95a2b58dd6773bf1b3f345d71f9149928b9f75f4ec9c9064c0b3e42c298671b2",
    ),
    (
        "translator-worker.js",
        17_191,
        "90257af9ee0758983501bdce77ca7ec168cd7360d4689298c20a81b0251d6b62",
    ),
];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Pair {
    EsEn,
    EnEs,
}

struct Package {
    id: &'static str,
    name: &'static str,
    folder: &'static str,
    model: (&'static str, u64, &'static str),
    shortlist: (&'static str, u64, &'static str),
    transfer: [u64; 2],
}

const ES_EN: Package = Package {
    id: "translate-es-en",
    name: "Bergamot · ES → EN",
    folder: "es-en",
    model: (
        "https://bergamot.s3.amazonaws.com/models/esen/model.esen.intgemm.alphas.bin",
        17_140_755,
        "4b6b7f451094aaa447d012658af158ffc708fc8842dde2f871a58404f5457fe0",
    ),
    shortlist: (
        "https://bergamot.s3.amazonaws.com/models/esen/lex.50.50.esen.s2t.bin",
        3_860_888,
        "f11a2c23ef85ab1fee1c412b908d69bc20d66fd59faa8f7da5a5f0347eddf969",
    ),
    transfer: [13_203_714, 1_977_498],
};
const EN_ES: Package = Package {
    id: "translate-en-es",
    name: "Bergamot · EN → ES",
    folder: "en-es",
    model: (
        "https://bergamot.s3.amazonaws.com/models/enes/model.enes.intgemm.alphas.bin",
        17_140_755,
        "fa7460037a3163e03fe1d23602f964bff2331da6ee813637e092ddf37156ef53",
    ),
    shortlist: (
        "https://bergamot.s3.amazonaws.com/models/enes/lex.50.50.enes.s2t.bin",
        3_347_104,
        "3a113d713dec3cf1d12bba5b138ae616e28bba4bbc7fe7fd39ba145e26b86d7f",
    ),
    transfer: [12_599_289, 1_719_302],
};

static ACCESS: Semaphore = Semaphore::const_new(1);

pub fn component_access() -> Result<SemaphorePermit<'static>, String> {
    ACCESS
        .try_acquire()
        .map_err(|_| "Wait for the translation or installation to finish.".into())
}

pub async fn processing_access(
    mut cancel: watch::Receiver<bool>,
) -> Result<SemaphorePermit<'static>, String> {
    if *cancel.borrow() {
        return Err(engine::CANCELLED.into());
    }
    tokio::select! {
        biased;
        _ = cancel.changed() => Err(engine::CANCELLED.into()),
        permit = ACCESS.acquire() => permit.map_err(|_| "The translation component is unavailable.".into()),
    }
}

impl Pair {
    pub const ALL: [Self; 2] = [Self::EsEn, Self::EnEs];

    fn package(self) -> &'static Package {
        match self {
            Self::EsEn => &ES_EN,
            Self::EnEs => &EN_ES,
        }
    }

    pub fn for_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pair| pair.package().id == id)
    }

    pub fn languages(self) -> (&'static str, &'static str) {
        match self {
            Self::EsEn => ("es", "en"),
            Self::EnEs => ("en", "es"),
        }
    }

    pub fn files(self) -> (&'static str, &'static str) {
        let package = self.package();
        (filename(package.model.0), filename(package.shortlist.0))
    }

    pub fn status(self, root: &Path, available: bool) -> components::Component {
        let package = self.package();
        let directory = root.join(DIRECTORY);
        let runtime = directory.join("runtime");
        let (model, shortlist) = self.files();
        let shared = installed_runtime(&runtime);
        let installed = shared
            && [model, shortlist]
                .into_iter()
                .all(|name| directory.join(package.folder).join(name).is_file());
        let other_installed = Self::ALL
            .into_iter()
            .filter(|pair| *pair != self)
            .any(|pair| {
                let (model, shortlist) = pair.files();
                [model, shortlist]
                    .into_iter()
                    .all(|name| directory.join(pair.package().folder).join(name).is_file())
            });
        let shared_download = if shared {
            0
        } else {
            RUNTIME.1 + VOCAB_TRANSFER
        };
        let shared_bytes = VOCAB.1 + RUNTIME_FILES.iter().map(|file| file.1).sum::<u64>();
        components::Component {
            id: package.id,
            name: package.name,
            version: "Bergamot 0.4.9 · Firefox Translations",
            size: package.transfer.into_iter().sum::<u64>() + shared_download,
            installed_size: Some(package.model.1 + package.shortlist.1 + if !shared || installed && !other_installed { shared_bytes } else { 0 }),
            license: "MPL-2.0 (runtime and weights)",
            source: REGISTRY,
            installed,
            enabled: root.join(format!("{}.enabled", package.id)).is_file(),
            available: available && tar().is_some(),
            note: Some("Windows x64 · CPU / WASM · Deno de nabnab / bundled Deno · 5.8 MiB de runtime compartido / shared runtime"),
        }
    }

    pub fn enabled(self, root: &Path) -> Result<(PathBuf, PathBuf), String> {
        let status = self.status(root, true);
        if !status.installed || !status.enabled {
            return Err(
                "Install and enable this translation direction in Settings → Components.".into(),
            );
        }
        let root = root.join(DIRECTORY);
        Ok((
            library::authorized_path(&root.join("runtime"))?,
            library::authorized_path(&root.join(self.package().folder))?,
        ))
    }
}

fn filename(url: &str) -> &str {
    url.rsplit('/').next().unwrap_or_default()
}

fn tar() -> Option<PathBuf> {
    if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return None;
    }
    let path = PathBuf::from(std::env::var_os("SystemRoot")?).join("System32/tar.exe");
    path.is_file().then_some(path)
}

fn installed_runtime(root: &Path) -> bool {
    root.join(VOCAB_FILE).is_file()
        && RUNTIME_FILES
            .iter()
            .all(|(name, _, _)| root.join(name).is_file())
}

async fn verify_file(
    path: &Path,
    size: u64,
    sha256: &str,
    cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    let path = library::authorized_path(path)?;
    if crate::portable_zip::regular_file(&path)? != size
        || queue::digest(path, cancel).await? != sha256
    {
        return Err(
            "The Bergamot component failed its integrity check. Remove and reinstall it.".into(),
        );
    }
    Ok(())
}

async fn verify_runtime(runtime: &Path, cancel: watch::Receiver<bool>) -> Result<(), String> {
    for (name, size, hash) in RUNTIME_FILES
        .into_iter()
        .chain([(VOCAB_FILE, VOCAB.1, VOCAB.2)])
    {
        verify_file(&runtime.join(name), size, hash, cancel.clone()).await?;
    }
    Ok(())
}

pub async fn verify(
    runtime: &Path,
    directory: &Path,
    pair: Pair,
    cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    verify_runtime(runtime, cancel.clone()).await?;
    let package = pair.package();
    for (url, size, hash) in [package.model, package.shortlist] {
        verify_file(&directory.join(filename(url)), size, hash, cancel.clone()).await?;
    }
    Ok(())
}

async fn download_model(
    client: &reqwest::Client,
    package: (&str, u64, &str),
    transfer: u64,
    path: &Path,
    cancel: watch::Receiver<bool>,
) -> Result<(), String> {
    let (url, size, hash) = package;
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| "The official translation model could not be downloaded.")?;
    let gzip = match response
        .headers()
        .get(reqwest::header::CONTENT_ENCODING)
        .and_then(|header| header.to_str().ok())
    {
        None | Some("identity") => false,
        Some("gzip") => true,
        _ => return Err("The translation model uses an unexpected transfer encoding.".into()),
    };
    let limit = if gzip { transfer + 65_536 } else { size };
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The translation model download was interrupted.")?
    {
        if bytes.len() as u64 + chunk.len() as u64 > limit {
            return Err("The translation model exceeds its download limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let destination = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .map_err(|error| error.to_string())?;
        let written = if gzip {
            let decoder = flate2::read::GzDecoder::new(bytes.as_slice());
            std::io::copy(&mut decoder.take(size + 1), &mut output)
                .map_err(|_| "The translation model could not be decompressed.")?
        } else {
            output
                .write_all(&bytes)
                .map_err(|error| error.to_string())?;
            bytes.len() as u64
        };
        if written != size {
            return Err("The translation model has an unexpected decompressed size.".into());
        }
        output.sync_all().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    verify_file(path, size, hash, cancel).await
}

pub async fn install(app: &AppHandle, pair: Pair) -> Result<(), String> {
    let tar = tar().ok_or("Bergamot requires Windows x64 and the system tar tool.")?;
    crate::dependencies::engine_root(app)?;
    let root = components::root(app)?;
    let directory = sessions::secure_module_directory(&root, DIRECTORY)?;
    let workspace = Workspace::new(&root)?;
    let package = pair.package();
    let staged = workspace.path.join(package.folder);
    let model = directory.join(package.folder);
    let runtime = directory.join("runtime");
    if fs::symlink_metadata(&model).is_ok() {
        return Err("Remove the existing translation model before installing it again.".into());
    }
    fs::create_dir(&staged).map_err(|error| error.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| error.to_string())?;
    let (_sender, cancel) = watch::channel(false);
    for (file, transfer) in [package.model, package.shortlist]
        .into_iter()
        .zip(package.transfer)
    {
        download_model(
            &client,
            file,
            transfer,
            &staged.join(filename(file.0)),
            cancel.clone(),
        )
        .await?;
    }
    if fs::symlink_metadata(&runtime).is_ok() {
        verify_runtime(&runtime, cancel.clone()).await?;
    } else {
        let staged_runtime = workspace.path.join("runtime");
        fs::create_dir(&staged_runtime).map_err(|error| error.to_string())?;
        let archive = workspace.path.join("bergamot.tgz");
        components::download_pinned(&client, RUNTIME, &archive).await?;
        let mut args = vec![
            "-xf".into(),
            archive.to_string_lossy().into_owned(),
            "-C".into(),
            staged_runtime.to_string_lossy().into_owned(),
            "--strip-components".into(),
            "2".into(),
        ];
        args.extend(
            RUNTIME_FILES
                .iter()
                .map(|(name, _, _)| format!("package/worker/{name}")),
        );
        engine::run_process(tar, args, cancel.clone(), 0, false, |_| {}).await?;
        fs::remove_file(archive).map_err(|error| error.to_string())?;
        download_model(
            &client,
            VOCAB,
            VOCAB_TRANSFER,
            &staged_runtime.join(VOCAB_FILE),
            cancel.clone(),
        )
        .await?;
        verify_runtime(&staged_runtime, cancel.clone()).await?;
        fs::rename(staged_runtime, &runtime).map_err(|error| error.to_string())?;
    }
    fs::rename(staged, model).map_err(|error| error.to_string())?;
    fs::write(root.join(format!("{}.enabled", package.id)), b"1").map_err(|error| error.to_string())
}

fn remove_files(
    directory: &Path,
    names: impl IntoIterator<Item = &'static str>,
) -> Result<(), String> {
    if fs::symlink_metadata(directory).is_err() {
        return Ok(());
    }
    let directory = library::authorized_path(directory)?;
    for name in names {
        let path = directory.join(name);
        if fs::symlink_metadata(&path).is_ok() {
            fs::remove_file(library::authorized_path(&path)?).map_err(|error| error.to_string())?;
        }
    }
    fs::remove_dir(directory).map_err(|error| error.to_string())
}

pub fn remove(root: &Path, pair: Pair) -> Result<(), String> {
    let directory = root.join(DIRECTORY);
    let (model, shortlist) = pair.files();
    remove_files(&directory.join(pair.package().folder), [model, shortlist])?;
    if Pair::ALL
        .into_iter()
        .all(|pair| fs::symlink_metadata(directory.join(pair.package().folder)).is_err())
    {
        remove_files(
            &directory.join("runtime"),
            RUNTIME_FILES
                .map(|file| file.0)
                .into_iter()
                .chain([VOCAB_FILE]),
        )?;
    }
    Ok(())
}
