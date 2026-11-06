use super::*;

#[derive(Deserialize)] #[serde(rename_all = "camelCase")]
pub struct InspectedEntry { pub name: String, pub size: u64, pub sha256: String }
#[derive(Deserialize)] #[serde(rename_all = "camelCase")]
pub struct Inspection { pub manifest: String, pub entries: Vec<InspectedEntry> }

async fn run(root: &Path, action: &str, archive: &Path, destination: &Path, names: &[String], receiver: watch::Receiver<bool>) -> Result<PathBuf, String> {
    if !cfg!(windows) { return Err("Importing compressed portable ZIPs requires the Windows ZIP reader. A selected portable folder can be imported without it.".into()); }
    let directory = directory(root)?.join(format!("zip-{}", nonce())); fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let partials = directory.join("parts"); fs::create_dir(&partials).map_err(|e| e.to_string())?;
    let script = directory.join("portable-zip.ps1"); fs::write(&script, include_str!("zip.ps1")).map_err(|e| e.to_string())?;
    let request = directory.join("names.json"); write_json(&request, &names)?; let report = directory.join("report.json");
    let system = std::env::var_os("SystemRoot").map(PathBuf::from).ok_or("The Windows ZIP reader is unavailable.")?;
    let executable = system.join("System32/WindowsPowerShell/v1.0/powershell.exe");
    crate::engine::run_process(executable, vec!["-NoProfile".into(), "-NonInteractive".into(), "-ExecutionPolicy".into(), "Bypass".into(), "-File".into(), script.to_string_lossy().into_owned(), "-Action".into(), action.into(), "-Archive".into(), archive.to_string_lossy().into_owned(), "-Destination".into(), destination.to_string_lossy().into_owned(), "-Report".into(), report.to_string_lossy().into_owned(), "-Names".into(), request.to_string_lossy().into_owned(), "-PartRoot".into(), partials.to_string_lossy().into_owned()], receiver, 0, false, |_| {}).await?;
    Ok(report)
}
pub async fn inspect(root: &Path, archive: &Path, receiver: watch::Receiver<bool>) -> Result<Inspection, String> {
    let report = run(root, "Inspect", archive, root, &[], receiver).await?; read_json(&report, 24 * 1024 * 1024)
}
pub async fn extract(root: &Path, archive: &Path, destination: &Path, names: &[String], receiver: watch::Receiver<bool>) -> Result<(), String> {
    for name in names { portable_zip::validate_name(name)?; }
    let report = run(root, "Extract", archive, destination, names, receiver).await?; let done: bool = read_json(&report, 1024)?; if done { Ok(()) } else { Err("The portable ZIP was not extracted.".into()) }
}
