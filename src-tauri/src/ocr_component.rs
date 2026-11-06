use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
#[cfg(test)]
use tokio::sync::watch;

pub const VERSION: &str = "5.5.4";
pub const DOWNLOAD_BYTES: u64 = 6_678_346;
pub const SOURCE: &str = "https://cygwin.com/packages/summary/tesseract-ocr.html";
pub const LICENSE: &str = "Apache-2.0 · LGPL/GPL runtime exceptions · BSD/MIT";
const MIRROR: &str = "https://mirrors.kernel.org/sourceware/cygwin/";
const MODEL_COMMIT: &str = "87416418657359cb625c412a48b6e1d6d41c29bd";

#[derive(Deserialize)]
struct Package {
    id: String,
    version: String,
    path: String,
    bytes: u64,
    sha512: String,
    source: String,
    license: String,
}
#[derive(Deserialize, Serialize)]
struct Inventory {
    files: Vec<(String, String)>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub bytes: u64,
    pub license: &'static str,
    pub source: String,
    pub installed: bool,
}
struct Model {
    id: &'static str,
    name: &'static str,
    bytes: u64,
    hash: &'static str,
}
const MODELS: [Model; 2] = [
    Model {
        id: "eng",
        name: "English",
        bytes: 4_113_088,
        hash: "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2",
    },
    Model {
        id: "spa",
        name: "Español",
        bytes: 2_294_433,
        hash: "6f2e04d02774a18f01bed44b1111f2cd7f3ba7ac9dc4373cd3f898a40ea6b464",
    },
];

fn packages() -> Result<Vec<Package>, String> {
    serde_json::from_str(include_str!("ocr/packages.json"))
        .map_err(|_| "The OCR package manifest is invalid.".into())
}
fn component_directory(root: &Path) -> PathBuf {
    root.join(format!("ocr-{VERSION}"))
}
pub fn installed(root: &Path) -> bool {
    component_directory(root).join("inventory.json").is_file()
}
pub fn models_directory(root: &Path) -> PathBuf {
    root.join("ocr-models")
}
fn model(id: &str) -> Result<&'static Model, String> {
    MODELS
        .iter()
        .find(|model| model.id == id)
        .ok_or_else(|| "Choose an OCR language: eng or spa.".into())
}
fn model_url(model: &Model) -> String {
    format!("https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/{MODEL_COMMIT}/{}.traineddata", model.id)
}
fn sha256(path: &Path) -> Result<String, String> {
    let mut input = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let count = input.read(&mut bytes).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn safe_relative(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 240
        && !name.contains(['\\', ':'])
        && !name.starts_with('/')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
pub fn verify(root: &Path) -> Result<PathBuf, String> {
    if !root.join("ocr.enabled").is_file() {
        return Err("Enable OCR in Settings → Components.".into());
    }
    let directory = crate::library::authorized_path(&component_directory(root))?;
    let receipt = fs::read(directory.join("inventory.json"))
        .map_err(|_| "Install the OCR component in Settings → Components.")?;
    if receipt.len() > 65536 {
        return Err("The OCR component inventory exceeds its limit.".into());
    }
    let inventory: Inventory =
        serde_json::from_slice(&receipt).map_err(|_| "The OCR component inventory is invalid.")?;
    if inventory.files.len() > 100
        || !inventory
            .files
            .iter()
            .any(|(name, _)| name == "usr/bin/tesseract.exe")
    {
        return Err("The OCR component inventory is incomplete.".into());
    }
    for (name, expected) in inventory.files {
        if !safe_relative(&name) || expected.len() != 64 {
            return Err("Invalid OCR component inventory entry.".into());
        }
        let path = crate::library::authorized_path(&directory.join(name))?;
        if !path.starts_with(&directory) || sha256(&path)? != expected {
            return Err("OCR component integrity check failed. Reinstall the component.".into());
        }
    }
    Ok(directory.join("usr/bin/tesseract.exe"))
}
pub fn models(root: &Path) -> Vec<ModelStatus> {
    MODELS
        .iter()
        .map(|model| ModelStatus {
            id: model.id,
            name: model.name,
            bytes: model.bytes,
            license: "Apache-2.0",
            source: model_url(model),
            installed: fs::metadata(
                models_directory(root).join(format!("{}.traineddata", model.id)),
            )
            .is_ok_and(|file| file.is_file() && file.len() == model.bytes),
        })
        .collect()
}
pub fn verify_models(root: &Path, languages: &[String]) -> Result<PathBuf, String> {
    if languages.is_empty() || languages.len() > 2 {
        return Err("Choose one or both installed OCR languages.".into());
    }
    let directory = crate::library::authorized_path(&models_directory(root))?;
    for language in languages {
        let model = model(language)?;
        let file =
            crate::library::authorized_path(&directory.join(format!("{}.traineddata", model.id)))?;
        if file.parent() != Some(directory.as_path())
            || fs::metadata(&file).map_err(|e| e.to_string())?.len() != model.bytes
            || sha256(&file)? != model.hash
        {
            return Err(format!(
                "Install or repair the {} OCR model before recognizing text.",
                model.name
            ));
        }
    }
    let font = crate::library::authorized_path(&directory.join("pdf.ttf"))?;
    let original = crate::library::authorized_path(
        &component_directory(root).join("usr/share/tessdata/pdf.ttf"),
    )?;
    if sha256(&font)? != sha256(&original)? {
        return Err(
            "The OCR PDF font failed its integrity check. Repair the OCR component.".into(),
        );
    }
    Ok(directory)
}

async fn download(url: &str, bytes: u64) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| "The pinned OCR download could not be reached.")?
        .error_for_status()
        .map_err(|_| "The pinned OCR download failed.")?;
    let mut output = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The OCR download was interrupted.")?
    {
        if output.len() as u64 + chunk.len() as u64 > bytes {
            return Err("The OCR download exceeds its pinned size.".into());
        }
        output.extend_from_slice(&chunk);
    }
    if output.len() as u64 != bytes {
        return Err("The OCR download did not reach its pinned size.".into());
    }
    Ok(output)
}
fn tar() -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
        .join("System32/tar.exe")
}
async fn tar_output(archive: &Path, entry: Option<&str>) -> Result<Vec<u8>, String> {
    let mut command = tokio::process::Command::new(tar());
    if let Some(entry) = entry {
        command.args(["-xOf"]).arg(archive).arg(entry);
    } else {
        command.args(["-tf"]).arg(archive);
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = command
        .output()
        .await
        .map_err(|_| "Windows tar.exe is required to unpack the optional OCR component.")?;
    if !output.status.success() || output.stdout.len() > 32 * 1024 * 1024 {
        return Err("The verified OCR package could not be unpacked within its limits.".into());
    }
    Ok(output.stdout)
}
fn allowed_entry(name: &str) -> bool {
    if !safe_relative(name) {
        return false;
    }
    if name == "usr/bin/tesseract.exe" || name == "usr/share/tessdata/pdf.ttf" {
        return true;
    }
    if let Some(name) = name.strip_prefix("usr/bin/") {
        return !name.contains('/')
            && name.ends_with(".dll")
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b));
    }
    name.starts_with("usr/share/doc/") && !name.ends_with('/') && {
        let file = name
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        file.starts_with("license")
            || file.starts_with("copying")
            || file.contains("copyright")
            || file.ends_with(".readme")
    }
}
struct Pending(PathBuf);
impl Drop for Pending {
    fn drop(&mut self) {
        if self
            .0
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("ocr-install-"))
        {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}

async fn install_at(root: &Path, local_packages: Option<&Path>) -> Result<(), String> {
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let root = crate::library::authorized_path(root)?;
    let destination = component_directory(&root);
    if destination.exists() {
        return Err("This OCR engine version already exists. Disable and remove it before reinstalling. Existing models are kept.".into());
    }
    let pending = root.join(format!(
        "ocr-install-{}-{}",
        std::process::id(),
        crate::queue::now()
    ));
    fs::create_dir(&pending).map_err(|e| e.to_string())?;
    let pending = Pending(pending);
    let mut inventory = Inventory { files: Vec::new() };
    let mut notices = format!("nabnab optional OCR: Tesseract {VERSION}\nPackages downloaded directly from the Cygwin mirror. No package install scripts are executed.\nhttps://cygwin.com/licensing.html\nSource packages and licenses:\n");
    for package in packages()? {
        let bytes = match local_packages {
            Some(local) => fs::read(
                local.join(
                    package
                        .path
                        .rsplit('/')
                        .next()
                        .ok_or("Invalid pinned package path.")?,
                ),
            )
            .map_err(|e| e.to_string())?,
            None => download(&format!("{MIRROR}{}", package.path), package.bytes).await?,
        };
        if bytes.len() as u64 != package.bytes
            || format!("{:x}", Sha512::digest(&bytes)) != package.sha512
        {
            return Err("OCR package SHA-512 mismatch. Nothing was installed.".into());
        }
        let archive = pending.0.join("package.tar");
        fs::write(&archive, bytes).map_err(|e| e.to_string())?;
        let listing = String::from_utf8(tar_output(&archive, None).await?)
            .map_err(|_| "The OCR package contains invalid paths.")?;
        if listing.lines().count() > 5000 {
            return Err("The OCR archive has too many entries.".into());
        }
        for name in listing.lines().filter(|name| allowed_entry(name)) {
            let content = tar_output(&archive, Some(name)).await?;
            let path = pending.0.join(name);
            fs::create_dir_all(path.parent().ok_or("Invalid OCR destination.")?)
                .map_err(|e| e.to_string())?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|_| "The OCR archives contain duplicate names. Nothing was activated.")?;
            output
                .write_all(&content)
                .and_then(|_| output.sync_all())
                .map_err(|e| e.to_string())?;
            inventory
                .files
                .push((name.into(), format!("{:x}", Sha256::digest(&content))));
        }
        fs::remove_file(archive).map_err(|e| e.to_string())?;
        notices.push_str(&format!(
            "{} {}: {}\nSource: {}\n",
            package.id, package.version, package.license, package.source
        ));
    }
    if !pending.0.join("usr/bin/tesseract.exe").is_file() || inventory.files.len() > 100 {
        return Err("The OCR engine package is incomplete.".into());
    }
    fs::write(pending.0.join("NOTICES.txt"), notices).map_err(|e| e.to_string())?;
    inventory.files.push((
        "NOTICES.txt".into(),
        sha256(&pending.0.join("NOTICES.txt"))?,
    ));
    fs::write(
        pending.0.join("inventory.json"),
        serde_json::to_vec(&inventory).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let models = models_directory(&root);
    fs::create_dir_all(&models).map_err(|e| e.to_string())?;
    let models = crate::library::authorized_path(&models)?;
    let font = pending.0.join("usr/share/tessdata/pdf.ttf");
    if models.join("pdf.ttf").exists() {
        if sha256(&models.join("pdf.ttf"))? != sha256(&font)? {
            return Err("An existing OCR font differs from the selected component. Existing files were kept.".into());
        }
    } else {
        let mut target = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(models.join("pdf.ttf"))
            .map_err(|e| e.to_string())?;
        target
            .write_all(&fs::read(font).map_err(|e| e.to_string())?)
            .and_then(|_| target.sync_all())
            .map_err(|e| e.to_string())?;
    }
    fs::rename(&pending.0, &destination)
        .map_err(|_| "Could not activate the verified OCR component.")?;
    fs::write(root.join("ocr.enabled"), b"1").map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn install(app: &AppHandle) -> Result<(), String> {
    if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return Err("The pinned OCR component is available for Windows x64.".into());
    }
    let dialog = app.clone();
    let accepted = tauri::async_runtime::spawn_blocking(move || dialog.dialog().message(format!("Download Tesseract {VERSION} and its app-local runtime? Download: 6.7 MB. Allow about 40 MB installed. Licenses: {LICENSE}. Languages are downloaded separately. No system installation or service is created.")).title("nabnab · Optional local OCR").buttons(MessageDialogButtons::OkCancel).blocking_show()).await.map_err(|_| "The OCR consent dialog could not be shown.")?;
    if !accepted {
        return Err("OCR component installation was cancelled.".into());
    }
    install_at(&crate::components::root(app)?, None).await
}
pub async fn install_model(app: &AppHandle, id: &str) -> Result<(), String> {
    let selected = model(id)?;
    let root = crate::components::root(app)?;
    verify(&root)?;
    let dialog = app.clone();
    let message = format!("Download the {} local OCR model? {} bytes, Apache-2.0, official tessdata_fast. Text recognition then runs offline.", selected.name, selected.bytes);
    if !tauri::async_runtime::spawn_blocking(move || {
        dialog
            .dialog()
            .message(message)
            .title("nabnab · OCR language")
            .buttons(MessageDialogButtons::OkCancel)
            .blocking_show()
    })
    .await
    .map_err(|_| "The OCR language consent dialog could not be shown.")?
    {
        return Err("OCR language download was cancelled.".into());
    }
    let bytes = download(&model_url(selected), selected.bytes).await?;
    if format!("{:x}", Sha256::digest(&bytes)) != selected.hash {
        return Err("The OCR language SHA-256 did not match. Nothing was installed.".into());
    }
    let directory = models_directory(&root);
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let directory = crate::library::authorized_path(&directory)?;
    let path = directory.join(format!("{}.traineddata", selected.id));
    if path.exists() {
        return if sha256(&path)? == selected.hash {
            Ok(())
        } else {
            Err("The existing OCR model differs from the pinned version. Remove it before installing again.".into())
        };
    }
    let pending = directory.join(format!("{}-{}.pending", selected.id, crate::queue::now()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(pending, path).map_err(|e| e.to_string())
}
pub fn remove(root: &Path) -> Result<(), String> {
    let directory = crate::library::authorized_path(&component_directory(root))?;
    let receipt = fs::read(directory.join("inventory.json")).map_err(|e| e.to_string())?;
    let inventory: Inventory = serde_json::from_slice(&receipt).map_err(|e| e.to_string())?;
    if inventory.files.len() > 100 {
        return Err("Invalid OCR inventory; existing files were kept.".into());
    }
    let mut paths = Vec::new();
    for (name, _) in inventory.files {
        if !safe_relative(&name) {
            return Err("Invalid OCR inventory path; existing files were kept.".into());
        }
        let path = crate::library::authorized_path(&directory.join(name))?;
        if !path.starts_with(&directory) {
            return Err("The OCR path escaped its component directory.".into());
        }
        paths.push(path);
    }
    let mut directories = Vec::new();
    for path in &paths {
        let mut parent = path.parent();
        while let Some(folder) =
            parent.filter(|folder| folder.starts_with(&directory) && *folder != directory.as_path())
        {
            directories.push(folder.to_owned());
            parent = folder.parent();
        }
    }
    for path in paths {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    fs::remove_file(directory.join("inventory.json")).map_err(|e| e.to_string())?;
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    directories.dedup();
    for folder in directories {
        let _ = fs::remove_dir(folder);
    }
    let _ = fs::remove_dir(&directory);
    if root.join("ocr.enabled").exists() {
        fs::remove_file(root.join("ocr.enabled")).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Explicit fixture provisioning only: requires fresh workspace artifacts/document-workflows/native-run-*/components, pinned package/model paths; no network or system install"]
    async fn provision_native_document_workflow_components() {
        let target=PathBuf::from(std::env::var_os("NABNAB_TEST_DOCUMENT_WORKFLOW_COMPONENTS").expect("Set an explicit fresh workflow component directory"));
        let workspace=Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let boundary=crate::library::authorized_path(&workspace.join("artifacts/document-workflows")).expect("Create the explicit artifact boundary first");
        let target=crate::library::authorized_path(&target).unwrap();
        assert_eq!(target.file_name().and_then(|name|name.to_str()),Some("components"));
        let run=target.parent().unwrap();assert_eq!(run.parent(),Some(boundary.as_path()));
        assert!(run.file_name().unwrap().to_string_lossy().starts_with("native-run-"));
        assert_eq!(fs::read_dir(&target).unwrap().count(),0,"Provision only a fresh empty isolated fixture directory");
        let packages=PathBuf::from(std::env::var_os("NABNAB_TEST_OCR_PACKAGES").expect("Set pinned local OCR packages"));
        let models=PathBuf::from(std::env::var_os("NABNAB_TEST_TESSDATA").expect("Set verified local OCR language models"));
        let mupdf=PathBuf::from(std::env::var_os("NABNAB_TEST_MUPDF_ARCHIVE").expect("Set pinned local MuPDF archive"));
        install_at(&target,Some(&packages)).await.unwrap();
        for model in MODELS {
            let source=crate::library::authorized_path(&models.join(format!("{}.traineddata",model.id))).unwrap();
            assert_eq!(fs::metadata(&source).unwrap().len(),model.bytes);assert_eq!(sha256(&source).unwrap(),model.hash);
            fs::copy(source,models_directory(&target).join(format!("{}.traineddata",model.id))).unwrap();
        }
        verify(&target).unwrap();verify_models(&target,&["eng".into(),"spa".into()]).unwrap();
        crate::component_archives::provision_document_workflow_fixture(&target,&mupdf).await.unwrap();
        crate::component_archives::verify(&target,"documents").unwrap();
        eprintln!("Prepared verified app-local OCR and MuPDF fixtures in the explicitly authorized workspace run directory.");
    }
    #[test]
    fn manifests_and_extraction_exclude_system_utilities() {
        let packages = packages().unwrap();
        assert_eq!(packages.len(), 21);
        assert_eq!(
            packages.iter().map(|p| p.bytes).sum::<u64>(),
            DOWNLOAD_BYTES
        );
        for package in packages {
            assert!(package.path.starts_with("x86_64/release/"));
            assert_eq!(package.sha512.len(), 128);
            assert!(package.source.starts_with(MIRROR));
        }
        for name in [
            "usr/bin/tesseract.exe",
            "usr/bin/cygwin1.dll",
            "usr/share/tessdata/pdf.ttf",
            "usr/share/doc/tesseract-ocr/LICENSE",
        ] {
            assert!(allowed_entry(name));
        }
        for name in [
            "usr/bin/bash.exe",
            "usr/bin/regtool.exe",
            "../../evil.dll",
            "usr/bin/../evil.dll",
            "usr/bin/folder/x.dll",
            "C:/windows/evil.dll",
            "usr/share/tessdata/configs/pdf",
        ] {
            assert!(!allowed_entry(name));
        }
    }
    #[tokio::test]
    #[ignore = "Requires NABNAB_TEST_OCR_PACKAGES with the pinned packages; no network or system install"]
    async fn real_component_installs_and_verifies_app_local() {
        let packages = PathBuf::from(
            std::env::var_os("NABNAB_TEST_OCR_PACKAGES")
                .expect("Set pinned local package directory"),
        );
        let root = std::env::temp_dir().join(format!(
            "nabnab-ocr-component-{}-{}",
            std::process::id(),
            crate::queue::now()
        ));
        fs::create_dir(&root).unwrap();
        install_at(&root, Some(&packages)).await.unwrap();
        let executable = verify(&root).unwrap();
        assert!(!executable.parent().unwrap().join("regtool.exe").exists());
        let (_sender, cancel) = watch::channel(false);
        let version = crate::engine::run_process(
            executable.clone(),
            vec!["--version".into()],
            cancel,
            0,
            true,
            |_| {},
        )
        .await
        .unwrap();
        assert!(version.contains("tesseract 5.5.4"));
        fs::write(&executable, b"tampered").unwrap();
        assert!(verify(&root).is_err());
        remove(&root).unwrap();
        assert!(!installed(&root));
        assert!(models_directory(&root).join("pdf.ttf").is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
