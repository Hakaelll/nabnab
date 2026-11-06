use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub size: u64,
    pub license: &'static str,
    pub source: &'static str,
    pub installed: bool,
    pub enabled: bool,
    pub available: bool,
}

const GALLERY_HASH: &str = "aaeae0e65f056efe3ff8a6ea8ffe4d39ccfef712f14ac7ab8bbf04a61ed50202";
const GALLERY_URL: &str =
    "https://github.com/mikf/gallery-dl/releases/download/v1.31.10/gallery-dl.exe";
pub fn root(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(root) = crate::test_data_dir() {
        return Ok(root.join("components"));
    }
    Ok(app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("components"))
}
pub fn executable(app: &AppHandle, id: &str) -> Result<PathBuf, String> {
    let name = match id {
        "gallery" => "gallery-dl.exe",
        "torrent" => "aria2c.exe",
        "transcription" => "whisper-cli.exe",
        _ => return Err("Unknown component.".into()),
    };
    let root = root(app)?;
    if !root.join(format!("{id}.enabled")).exists() {
        return Err(format!(
            "Enable the {id} component in Settings → Components."
        ));
    }
    let path = root.join(name);
    if !path.is_file() {
        return Err(format!(
            "Install the {id} component in Settings → Components."
        ));
    }
    if id != "gallery" { return crate::component_archives::verify(&root,id); }
    if id == "gallery"
        && format!(
            "{:x}",
            Sha256::digest(fs::read(&path).map_err(|e| e.to_string())?)
        ) != GALLERY_HASH
    {
        return Err("The gallery component failed its integrity check. Reinstall it.".into());
    }
    Ok(path)
}
#[tauri::command]
pub fn optional_components(app: AppHandle) -> Result<Vec<Component>, String> {
    let root = root(&app)?;
    let mut entries=vec![Component {
        id: "gallery",
        name: "Galerías · gallery-dl",
        version: "1.31.10",
        size: 22_482_737,
        license: "GPL-2.0",
        source: GALLERY_URL,
        installed: root.join("gallery-dl.exe").is_file(),
        enabled: root.join("gallery.enabled").exists(),
        available: cfg!(target_os = "windows") && cfg!(target_arch = "x86_64"),
    }];
    entries.extend(crate::component_archives::PACKAGES.iter().map(|p|Component{id:p.id,name:p.name,version:p.version,size:p.bytes as u64,license:p.license,source:p.url,installed:root.join(format!("{}.inventory.json",p.id)).exists(),enabled:root.join(format!("{}.enabled",p.id)).exists(),available:cfg!(target_os="windows")&&cfg!(target_arch="x86_64")}));
    Ok(entries)
}
#[tauri::command]
pub async fn install_component(app: AppHandle, id: String) -> Result<(), String> {
    if id != "gallery" { return crate::component_archives::install(&app,&id).await; }
    if !cfg!(target_os = "windows") || !cfg!(target_arch = "x86_64") {
        return Err("No verified package is available for this component and system.".into());
    }
    let root = root(&app)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(GALLERY_URL)
        .send()
        .await
        .map_err(|_| "The official component download could not be reached.")?
        .error_for_status()
        .map_err(|_| "The official component download failed.")?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Component transfer interrupted.")?
    {
        if bytes.len() + chunk.len() > 22_482_737 {
            return Err("The component exceeds its pinned size.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.len() != 22_482_737 || format!("{:x}", Sha256::digest(&bytes)) != GALLERY_HASH {
        return Err("Component checksum mismatch. Nothing was installed.".into());
    }
    let temp = root.join("gallery-dl.pending");
    fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    fs::rename(temp, root.join("gallery-dl.exe")).map_err(|e| e.to_string())?;
    fs::write(root.join("gallery.enabled"), b"1").map_err(|e| e.to_string())
}
#[tauri::command]
pub fn enable_component(app: AppHandle, id: String, enabled: bool) -> Result<(), String> {
    if !matches!(id.as_str(), "gallery" | "torrent" | "transcription") {
        return Err("Unknown component.".into());
    }
    if id=="torrent"&&enabled {return Err("Activate BitTorrent from its selector after reviewing the P2P notice.".into());}
    let root = root(&app)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let path = root.join(format!("{id}.enabled"));
    if enabled {
        fs::write(path, b"1").map_err(|e| e.to_string())
    } else if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}
