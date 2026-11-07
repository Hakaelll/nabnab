use super::*;

#[derive(Deserialize)] #[serde(rename_all = "camelCase")]
pub struct InspectedEntry { pub name: String, pub size: u64, pub sha256: String }
#[derive(Deserialize)] #[serde(rename_all = "camelCase")]
pub struct Inspection { pub manifest: String, pub entries: Vec<InspectedEntry> }

struct ZipReport { directory: PathBuf, path: PathBuf }
impl Drop for ZipReport {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.directory); }
}

async fn run(root: &Path, action: &str, archive: &Path, destination: &Path, names: &[String], receiver: watch::Receiver<bool>) -> Result<ZipReport, String> {
    if !cfg!(windows) { return Err("Importing compressed portable ZIPs requires the Windows ZIP reader. A selected portable folder can be imported without it.".into()); }
    let directory = directory(root)?.join(format!("zip-{}", nonce())); fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let report = ZipReport { path: directory.join("report.json"), directory };
    let partials = report.directory.join("parts"); fs::create_dir(&partials).map_err(|e| e.to_string())?;
    let script = report.directory.join("portable-zip.ps1"); fs::write(&script, include_str!("zip.ps1")).map_err(|e| e.to_string())?;
    let request = report.directory.join("names.json"); write_json(&request, &names)?;
    let system = std::env::var_os("SystemRoot").map(PathBuf::from).ok_or("The Windows ZIP reader is unavailable.")?;
    let executable = system.join("System32/WindowsPowerShell/v1.0/powershell.exe");
    crate::engine::run_process(executable, vec!["-NoProfile".into(), "-NonInteractive".into(), "-ExecutionPolicy".into(), "Bypass".into(), "-File".into(), script.to_string_lossy().into_owned(), "-Action".into(), action.into(), "-Archive".into(), archive.to_string_lossy().into_owned(), "-Destination".into(), destination.to_string_lossy().into_owned(), "-Report".into(), report.path.to_string_lossy().into_owned(), "-Names".into(), request.to_string_lossy().into_owned(), "-PartRoot".into(), partials.to_string_lossy().into_owned()], receiver, 0, false, |_| {}).await?;
    Ok(report)
}
pub async fn inspect(root: &Path, archive: &Path, receiver: watch::Receiver<bool>) -> Result<Inspection, String> {
    let report = run(root, "Inspect", archive, root, &[], receiver).await?; read_json(&report.path, 24 * 1024 * 1024)
}
pub async fn extract(root: &Path, archive: &Path, destination: &Path, names: &[String], receiver: watch::Receiver<bool>) -> Result<(), String> {
    for name in names { portable_zip::validate_name(name)?; }
    let report = run(root, "Extract", archive, destination, names, receiver).await?; let done: bool = read_json(&report.path, 1024)?; if done { Ok(()) } else { Err("The portable ZIP was not extracted.".into()) }
}

#[derive(Clone, Deserialize)]
pub struct CatalogEntry { pub name: String, pub size: u64 }

pub async fn personal_catalog(root: &Path, archive: &Path, receiver: watch::Receiver<bool>) -> Result<Vec<CatalogEntry>, String> {
    let report = run(root, "PersonalCatalog", archive, root, &[], receiver).await?;
    read_json(&report.path, 12 * 1024 * 1024)
}

pub async fn personal_extract(root: &Path, archive: &Path, destination: &Path, names: &[String], receiver: watch::Receiver<bool>) -> Result<(), String> {
    for name in names { portable_zip::validate_name(name)?; }
    let report = run(root, "PersonalExtract", archive, destination, names, receiver).await?;
    let complete: bool = read_json(&report.path, 1024)?;
    if complete { Ok(()) } else { Err("The selected personal export could not be extracted.".into()) }
}
