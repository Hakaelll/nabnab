use serde::{Deserialize, Serialize};
use sha2::{Digest,Sha256};
use std::{fs,path::{Path,PathBuf}};
use tauri::AppHandle;

pub struct Package {pub id:&'static str,pub name:&'static str,pub version:&'static str,pub bytes:usize,pub license:&'static str,pub url:&'static str,hash:&'static str,executable:&'static str}
pub const PACKAGES:[Package;4]=[
    Package{id:"telegram",name:"Telegram · TDLib",version:"1.8.69",bytes:13_998_492,license:"BSL-1.0 · Apache-2.0 · Zlib",url:"https://api.nuget.org/v3-flatcontainer/tdlib.native.win-x64/1.8.69/tdlib.native.win-x64.1.8.69.nupkg",hash:"2e0c337a1d005fd86a9457bc9d5abd6c68a95ba299173a5b0930785b68e0ef50",executable:"tdjson.dll"},
    Package{id:"documents",name:"Documentos · MuPDF",version:"1.28.5",bytes:94_292_330,license:"AGPL-3.0",url:"https://github.com/ArtifexSoftware/mupdf-downloads/releases/download/1.28.5/mupdf-1.28.5-windows.zip",hash:"13476a53e84f5527983158acb6442c7853b3cdc022f3b24880780dcfba1fcc99",executable:"mutool.exe"},
    Package{id:"torrent",name:"Archivos / BitTorrent · aria2",version:"1.37.0",bytes:2_475_379,license:"GPL-2.0+",url:"https://github.com/aria2/aria2/releases/download/release-1.37.0/aria2-1.37.0-win-64bit-build1.zip",hash:"67d015301eef0b612191212d564c5bb0a14b5b9c4796b76454276a4d28d9b288",executable:"aria2c.exe"},
    Package{id:"transcription",name:"Transcripción · whisper.cpp",version:"1.7.6",bytes:3_675_974,license:"MIT",url:"https://github.com/ggml-org/whisper.cpp/releases/download/v1.7.6/whisper-bin-x64.zip",hash:"0d2eca299c248f965bd0341bcb219db4b433c7f0c0ce2200d4df85765e8156a9",executable:"whisper-cli.exe"}
];
#[derive(Serialize,Deserialize)]struct Inventory{files:Vec<(String,String)>}
pub fn installed(root:&Path,id:&str)->bool {PACKAGES.iter().find(|p|p.id==id).is_some_and(|p|root.join(format!("{}-{}",p.id,p.version)).join("inventory.json").is_file()||root.join(format!("{id}.inventory.json")).is_file())}
fn digest(path:&Path)->Result<String,String>{Ok(format!("{:x}",Sha256::digest(fs::read(path).map_err(|e|e.to_string())?)))}
pub fn verify(root:&Path,id:&str)->Result<PathBuf,String>{
    let p=PACKAGES.iter().find(|p|p.id==id).ok_or("Unknown archive component.")?;
    let version=root.join(format!("{}-{}",p.id,p.version));let (root,inventory_path)=if version.is_dir(){(version.clone(),version.join("inventory.json"))}else{(root.to_owned(),root.join(format!("{id}.inventory.json")))};
    let inventory:Inventory=serde_json::from_slice(&fs::read(inventory_path).map_err(|_|"The component has no verified inventory. Install it in Settings → Components.")?).map_err(|_|"Invalid component inventory.")?;
    if inventory.files.len()>50||!inventory.files.iter().any(|(name,_)|name==p.executable){return Err("Incomplete component inventory.".into());}
    for(name,hash)in inventory.files{
        if name.contains(['/', '\\', ':'])||hash.len()!=64{return Err("Invalid component inventory path.".into());}
        let file=crate::library::authorized_path(&root.join(name))?;
        if digest(&file)?!=hash{return Err("Component integrity check failed. Existing files were kept.".into());}
    }
    crate::library::authorized_path(&root.join(p.executable))
}
pub async fn install(app:&AppHandle,id:&str)->Result<(),String>{
    let p=PACKAGES.iter().find(|p|p.id==id).ok_or("Unknown archive component.")?;
    if !cfg!(target_os="windows")||!cfg!(target_arch="x86_64"){return Err("No verified component package is available for this system.".into());}
    let root=crate::components::root(app)?;fs::create_dir_all(&root).map_err(|e|e.to_string())?;let root=crate::library::authorized_path(&root)?;
    let destination=root.join(format!("{}-{}",p.id,p.version));
    if destination.exists(){return Err("This component version is already present. Existing files were kept.".into());}
    let client=reqwest::Client::builder().timeout(std::time::Duration::from_secs(180)).build().map_err(|e|e.to_string())?;
    let mut response=client.get(p.url).send().await.map_err(|_|"The pinned component package could not be reached.")?.error_for_status().map_err(|_|"The pinned component download failed.")?;
    let mut bytes=Vec::new();while let Some(chunk)=response.chunk().await.map_err(|_|"Component transfer interrupted.")?{if bytes.len()+chunk.len()>p.bytes{return Err("Component exceeds its pinned size.".into());}bytes.extend_from_slice(&chunk);}
    activate_archive(&root,p,bytes).await
}
async fn activate_archive(root:&Path,p:&Package,bytes:Vec<u8>)->Result<(),String>{
    let id=p.id;
    let destination=root.join(format!("{}-{}",p.id,p.version));
    if destination.exists(){return Err("This component version is already present. Existing files were kept.".into());}
    if bytes.len()!=p.bytes||format!("{:x}",Sha256::digest(&bytes))!=p.hash{return Err("Component size or checksum mismatch. Nothing was installed.".into());}
    let pending=root.join(format!("{id}-install-{}",crate::queue::now()));fs::create_dir(&pending).map_err(|e|e.to_string())?;
    let archive=pending.join("package.zip");fs::write(&archive,bytes).map_err(|e|e.to_string())?;
    let script=r#"$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.IO.Compression.FileSystem; $z=[IO.Compression.ZipFile]::OpenRead($env:NAB_COMPONENT_ZIP); try { foreach($e in $z.Entries) { $n=$e.Name; if($n -eq $env:NAB_COMPONENT_EXE -or $n -match '^[a-zA-Z0-9_.-]+\.dll$' -or ($env:NAB_COMPONENT_ID -eq 'telegram' -and ($n -eq 'tdlib.native.win-x64.nuspec' -or $n -eq 'README.md')) -or $n -match '^(LICENSE|COPYING)([.][a-zA-Z0-9_-]+)?$') { if($e.Length -gt 67108864){throw 'Component entry exceeds size limit'}; if($n -match '^(LICENSE|COPYING)'){$n=$env:NAB_COMPONENT_ID+'-'+$n+'.txt'}; [IO.Compression.ZipFileExtensions]::ExtractToFile($e,[IO.Path]::Combine($env:NAB_COMPONENT_TARGET,$n),$false) } } } finally { $z.Dispose() }"#;
    let mut command=tokio::process::Command::new("powershell.exe");command.args(["-NoProfile","-NonInteractive","-Command",script]).env("NAB_COMPONENT_ZIP",&archive).env("NAB_COMPONENT_TARGET",&pending).env("NAB_COMPONENT_EXE",p.executable).env("NAB_COMPONENT_ID",id);
    #[cfg(windows)]command.creation_flags(0x08000000);
    let result=command.output().await.map_err(|_|"Could not unpack verified component archive.")?;
    if !result.status.success()||!pending.join(p.executable).is_file(){return Err("The verified archive could not be unpacked. Existing files were kept.".into());}
    if id=="telegram"{fs::write(pending.join("telegram-NOTICES.txt"),include_str!("../../licenses/tdlib-native.txt")).map_err(|e|e.to_string())?;}
    let mut inventory=Inventory{files:vec![]};for entry in fs::read_dir(&pending).map_err(|e|e.to_string())?{let path=entry.map_err(|e|e.to_string())?.path();let name=path.file_name().ok_or("Invalid package entry.")?.to_string_lossy().into_owned();if name=="package.zip"{continue;}inventory.files.push((name,digest(&path)?));}
    fs::write(pending.join("inventory.json"),serde_json::to_vec(&inventory).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    fs::rename(&pending,&destination).map_err(|_|"Could not activate the verified component. The complete package was kept for retry.")?;
    fs::write(root.join(format!("{}.enabled",if id=="torrent"{"files"}else{id})),b"1").map_err(|e|e.to_string())?;
    Ok(())
}
pub fn remove(root:&Path,id:&str)->Result<(),String>{
    let p=PACKAGES.iter().find(|p|p.id==id).ok_or("Unknown component package.")?;
    let version=root.join(format!("{}-{}",p.id,p.version));let directory=if version.is_dir(){crate::library::authorized_path(&version)?}else{crate::library::authorized_path(root)?};
    let receipt=if version.is_dir(){directory.join("inventory.json")}else{directory.join(format!("{id}.inventory.json"))};
    let inventory:Inventory=serde_json::from_slice(&fs::read(&receipt).map_err(|_|"Component inventory is unavailable; files were kept.")?).map_err(|_|"Invalid component inventory; files were kept.")?;
    if inventory.files.len()>50{return Err("Component inventory exceeds its limit.".into());}
    for(name,_)in &inventory.files{if name.is_empty()||name.contains(['/', '\\', ':']){return Err("Invalid component filename; files were kept.".into());}let path=directory.join(name);if path.exists(){let path=crate::library::authorized_path(&path)?;if path.parent()!=Some(directory.as_path()){return Err("Component path escaped its directory.".into());}}}
    for(name,_)in inventory.files{let path=directory.join(name);if path.exists(){fs::remove_file(path).map_err(|e|e.to_string())?;}}
    fs::remove_file(receipt).map_err(|e|e.to_string())?;
    if version.is_dir(){let archive=directory.join("package.zip");if archive.exists(){crate::library::authorized_path(&archive)?;fs::remove_file(archive).map_err(|e|e.to_string())?;}let _=fs::remove_dir(&directory);}
    Ok(())
}
#[cfg(test)]mod tests{use super::*;#[test]fn sources_are_fixed_and_pinned(){for p in &PACKAGES{assert!(p.url.starts_with("https://github.com/")&&p.url.contains("/releases/download/")||p.url=="https://api.nuget.org/v3-flatcontainer/tdlib.native.win-x64/1.8.69/tdlib.native.win-x64.1.8.69.nupkg");assert_eq!(p.hash.len(),64);assert!(p.bytes>0);}}}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Requires NABNAB_TEST_TDLIB_PACKAGE pointing to the pinned NuGet package; no account or network access."]
    async fn real_tdlib_archive_installs_atomically_and_rejects_tampering() {
        let package=std::env::var_os("NABNAB_TEST_TDLIB_PACKAGE").expect("Set the pinned package path");
        let p=PACKAGES.iter().find(|p|p.id=="telegram").unwrap();
        let root=std::env::temp_dir().join(format!("nabnab-tdlib-package-{}-{}",std::process::id(),crate::queue::now()));
        fs::create_dir(&root).unwrap();
        let bytes=fs::read(package).unwrap();
        let mut damaged=bytes.clone();damaged[0]^=1;
        assert!(activate_archive(&root,p,damaged).await.unwrap_err().contains("checksum"));
        assert_eq!(fs::read_dir(&root).unwrap().count(),0);
        activate_archive(&root,p,bytes.clone()).await.unwrap();
        let dll=verify(&root,"telegram").unwrap();
        assert!(root.join("telegram.enabled").is_file());
        assert!(dll.parent().unwrap().join("libcrypto-3-x64.dll").is_file());
        assert!(dll.parent().unwrap().join("telegram-LICENSE.txt.txt").is_file());
        assert!(activate_archive(&root,p,bytes).await.unwrap_err().contains("already present"));
        fs::write(&dll,b"tampered").unwrap();
        assert!(verify(&root,"telegram").unwrap_err().contains("integrity"));
        remove(&root,"telegram").unwrap();
        assert!(!installed(&root,"telegram"));
        assert_eq!(root.parent(),Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(root).unwrap();
    }
}