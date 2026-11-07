use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Manager};
use std::sync::Arc;
use tauri_plugin_dialog::{DialogExt,MessageDialogButtons};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_size: Option<u64>,
    pub license: &'static str,
    pub source: &'static str,
    pub installed: bool,
    pub enabled: bool,
    pub available: bool,
    pub note: Option<&'static str>,
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
        "torrent" | "files" => "aria2c.exe",
        "transcription" => "whisper-cli.exe",
        "documents" => "mutool.exe",
        "ocr" => "tesseract.exe",
        "telegram" => "tdjson.dll",
        _ => return Err("Unknown component.".into()),
    };
    let root = root(app)?;
    if !root.join(format!("{id}.enabled")).exists() {
        return Err(format!(
            "Enable the {id} component in Settings → Components."
        ));
    }
    if id=="ocr" {return crate::ocr_component::verify(&root);}
    if id!="gallery" {return crate::component_archives::verify(&root,if id=="files"{"torrent"}else{id});}
    let path = root.join(name);
    if !path.is_file() {
        return Err(format!(
            "Install the {id} component in Settings → Components."
        ));
    }
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
        installed_size: None,
        license: "GPL-2.0",
        source: GALLERY_URL,
        installed: root.join("gallery-dl.exe").is_file(),
        enabled: root.join("gallery.enabled").exists(),
        available: cfg!(target_os = "windows") && cfg!(target_arch = "x86_64"),
        note: None,
    }];
    entries.extend(crate::component_archives::PACKAGES.iter().map(|p|{let id=if p.id=="torrent"{"files"}else{p.id};Component{id,name:p.name,version:p.version,size:p.bytes as u64,installed_size:None,license:p.license,source:p.url,installed:crate::component_archives::installed(&root,p.id),enabled:root.join(format!("{id}.enabled")).exists(),available:cfg!(target_os="windows")&&cfg!(target_arch="x86_64"),note:if p.id=="torrent"{Some("Instalar Archivos no activa BitTorrent; P2P requiere consentimiento en su selector.")}else if p.id=="telegram"{Some("Paquete tdlib.native de ForNeVeR en NuGet. Requiere Microsoft Visual C++ Redistributable 2019 o posterior; no inicia sesión.")}else{None}}}));
    entries.push(Component{id:"ocr",name:"OCR local · Tesseract",version:crate::ocr_component::VERSION,size:crate::ocr_component::DOWNLOAD_BYTES,installed_size:None,license:crate::ocr_component::LICENSE,source:crate::ocr_component::SOURCE,installed:crate::ocr_component::installed(&root),enabled:root.join("ocr.enabled").is_file(),available:cfg!(all(target_os="windows",target_arch="x86_64")),note:Some("Motor local opcional. Modelos español/inglés por separado, con consentimiento y tamaño visible.")});
    entries.push(Component{id:"voice",name:"Voz local · Windows Desktop",version:"System.Speech / SAPI",size:0,installed_size:None,license:"Microsoft Windows terms",source:"https://learn.microsoft.com/dotnet/api/system.speech.synthesis.speechsynthesizer",installed:cfg!(windows),enabled:root.join("voice.enabled").is_file(),available:cfg!(windows),note:Some("Usa voces Microsoft Desktop ya instaladas y compatibles con uso offline. No descarga modelos ni utiliza Edge TTS. Las voces e idiomas se gestionan en Windows; nabnab solo activa esta función.")});
    entries.push(Component{id:"capture",name:"Captura local · Pantalla y repetición",version:"FFmpeg / WASAPI",size:0,installed_size:None,license:"FFmpeg LGPL; WASAPI binding MIT",source:"https://ffmpeg.org/ffmpeg-devices.html#gdigrab",installed:crate::dependencies::engine_root(&app).is_ok(),enabled:root.join("capture.enabled").is_file(),available:cfg!(windows),note:Some("Utiliza el FFmpeg existente y las APIs de Windows. Activar el componente no inicia capturas. Pantalla, audio y micrófono requieren selección explícita en la herramienta.")});
    entries.push(crate::upscale::component::status(&root));
    Ok(entries)
}
#[tauri::command]
pub async fn install_component(app: AppHandle, state: tauri::State<'_, Arc<crate::AppState>>, id: String) -> Result<(), String> {
    if id == crate::upscale::component::ID {
        let _access = state.upscale.component_access()?;
        return crate::upscale::component::install(&app).await;
    }
    let id=if id=="files"{"torrent".to_owned()}else{id};
    if id=="ocr" {return crate::ocr_component::install(&app).await;}
    if id != "gallery" { return crate::component_archives::install(&app,&id).await; }
    if !cfg!(target_os = "windows") || !cfg!(target_arch = "x86_64") {
        return Err("No verified package is available for this component and system.".into());
    }
    let root = root(&app)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let root=crate::library::authorized_path(&root)?;
    if root.join("gallery-dl.exe").exists(){return Err("This gallery component is already present. Existing files were kept.".into());}
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
    let temp = root.join(format!("gallery-dl-{}.pending",crate::queue::now()));
    use std::io::Write;
    let mut pending=fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e|e.to_string())?;
    pending.write_all(&bytes).map_err(|e|e.to_string())?;pending.sync_all().map_err(|e|e.to_string())?;drop(pending);
    fs::rename(temp, root.join("gallery-dl.exe")).map_err(|e| e.to_string())?;
    fs::write(root.join("gallery.enabled"), b"1").map_err(|e| e.to_string())
}
#[tauri::command]
pub fn enable_component(app: AppHandle, state:tauri::State<Arc<crate::AppState>>, id: String, enabled: bool) -> Result<(), String> {
    if !matches!(id.as_str(), "gallery" | "torrent" | "files" | "transcription" | "documents" | "telegram" | "ocr" | "voice" | "capture" | "upscale") {
        return Err("Unknown component.".into());
    }
    if id=="torrent"&&enabled {return Err("Activate BitTorrent from its selector after reviewing the P2P notice.".into());}
    if matches!(id.as_str(),"voice"|"capture")&&!cfg!(windows){return Err("This local component requires Windows.".into());}
    if id=="capture"&&enabled {crate::dependencies::engine_root(&app)?;}
    let _upscale = if id == "upscale" { Some(state.upscale.component_access()?) } else { None };
    if !enabled {idle(&state,&id)?;}
    if id=="telegram"&&!enabled {crate::telegram::shutdown();}
    let root = root(&app)?;
    if id == "upscale" && enabled && !crate::upscale::component::status(&root).installed {
        return Err("Install Real-ESRGAN before enabling it.".into());
    }
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

fn idle(state:&crate::AppState,id:&str)->Result<(),String>{
    if id=="capture"&&crate::capture_operations::active(state){return Err("Stop the recording or replay buffer before disabling screen capture.".into());}
    let active={let guard=state.repository.lock().unwrap();guard.as_ref().is_some_and(|repo|repo.data.jobs.iter().any(|job|job.state.active()&&match(&job.source,id){(crate::queue::Source::Voice{..},"voice")=>true,(crate::queue::Source::Torrent{..},"torrent"|"files")=>true,(crate::queue::Source::Document{..}|crate::queue::Source::DocumentEdit{..},"documents")=>true,(crate::queue::Source::Ocr{..},"documents"|"ocr")=>true,(crate::queue::Source::Telegram{..},"telegram")=>true,(crate::queue::Source::Transform{options:crate::tools::ToolOptions::Transcribe{..},..},"transcription")=>true,(_,"gallery")=>job.content_id.as_deref().is_some_and(|v|v.starts_with("gallery:")),_=>false}))};
    if active || id=="gallery"&&crate::gallery_sources::active_checks(state){return Err("This component has active work. Stop it or wait for it to finish before disabling or removing the component.".into());}Ok(())
}

#[tauri::command]
pub async fn remove_component(app:AppHandle,state:tauri::State<'_,Arc<crate::AppState>>,id:String)->Result<bool,String>{
    if !matches!(id.as_str(),"gallery"|"files"|"torrent"|"transcription"|"documents"|"telegram"|"ocr"|"upscale"){return Err("This component cannot be removed by the package manager.".into());}
    idle(&state,&id)?;
    let message = if id == "upscale" { "Remove Real-ESRGAN and its installed model files? Your images and results will be kept." } else { "Remove this optional engine? Downloaded files, models and account sessions will be kept." };
    let dialog=app.clone();let approved=tauri::async_runtime::spawn_blocking(move||dialog.dialog().message(message).title("nabnab · Remove component").buttons(MessageDialogButtons::OkCancel).blocking_show()).await.map_err(|_|"The component confirmation could not be shown.")?;
    if !approved{return Ok(false);}idle(&state,&id)?;if id=="telegram"{crate::telegram::shutdown();}
    let root=root(&app)?;
    let _upscale = if id == "upscale" { Some(state.upscale.component_access()?) } else { None };
    if id=="gallery"{let file=root.join("gallery-dl.exe");if file.exists(){let file=crate::library::authorized_path(&file)?;fs::remove_file(file).map_err(|e|e.to_string())?;}}else if id=="ocr"{crate::ocr_component::remove(&root)?;}else if id=="upscale"{crate::upscale::component::remove(&root)?;}else{crate::component_archives::remove(&root,if id=="files"{"torrent"}else{&id})?;}
    for marker in [format!("{id}.enabled"),if id=="files"{"torrent.enabled".into()}else{format!("{id}.enabled")}] {let path=root.join(marker);if path.exists(){fs::remove_file(path).map_err(|e|e.to_string())?;}}
    Ok(true)
}
