use serde::{Deserialize, Serialize};
use sha2::{Digest,Sha256};
use std::{fs,path::{Path,PathBuf}};
use tauri::AppHandle;

pub struct Package {pub id:&'static str,pub name:&'static str,pub version:&'static str,pub bytes:usize,pub license:&'static str,pub url:&'static str,hash:&'static str,executable:&'static str}
pub const PACKAGES:[Package;2]=[
    Package{id:"torrent",name:"Archivos / BitTorrent · aria2",version:"1.37.0",bytes:2_475_379,license:"GPL-2.0+",url:"https://github.com/aria2/aria2/releases/download/release-1.37.0/aria2-1.37.0-win-64bit-build1.zip",hash:"67d015301eef0b612191212d564c5bb0a14b5b9c4796b76454276a4d28d9b288",executable:"aria2c.exe"},
    Package{id:"transcription",name:"Transcripción · whisper.cpp",version:"1.7.6",bytes:3_675_974,license:"MIT",url:"https://github.com/ggml-org/whisper.cpp/releases/download/v1.7.6/whisper-bin-x64.zip",hash:"0d2eca299c248f965bd0341bcb219db4b433c7f0c0ce2200d4df85765e8156a9",executable:"whisper-cli.exe"}
];
#[derive(Serialize,Deserialize)]struct Inventory{files:Vec<(String,String)>}
fn digest(path:&Path)->Result<String,String>{Ok(format!("{:x}",Sha256::digest(fs::read(path).map_err(|e|e.to_string())?)))}
pub fn verify(root:&Path,id:&str)->Result<PathBuf,String>{
    let p=PACKAGES.iter().find(|p|p.id==id).ok_or("Unknown archive component.")?;
    let inventory:Inventory=serde_json::from_slice(&fs::read(root.join(format!("{id}.inventory.json"))).map_err(|_|"The component has no verified inventory. Install it in Settings → Components.")?).map_err(|_|"Invalid component inventory.")?;
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
    if root.join(p.executable).exists(){return Err("The component executable is already present. Existing files were kept.".into());}
    let client=reqwest::Client::builder().timeout(std::time::Duration::from_secs(180)).build().map_err(|e|e.to_string())?;
    let mut response=client.get(p.url).send().await.map_err(|_|"The official component package could not be reached.")?.error_for_status().map_err(|_|"The official component download failed.")?;
    let mut bytes=Vec::new();while let Some(chunk)=response.chunk().await.map_err(|_|"Component transfer interrupted.")?{if bytes.len()+chunk.len()>p.bytes{return Err("Component exceeds its pinned size.".into());}bytes.extend_from_slice(&chunk);}
    if bytes.len()!=p.bytes||format!("{:x}",Sha256::digest(&bytes))!=p.hash{return Err("Component size or checksum mismatch. Nothing was installed.".into());}
    let pending=root.join(format!("{id}-install-{}",crate::queue::now()));fs::create_dir(&pending).map_err(|e|e.to_string())?;
    let archive=pending.join("package.zip");fs::write(&archive,bytes).map_err(|e|e.to_string())?;
    let script=r#"$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.IO.Compression.FileSystem; $z=[IO.Compression.ZipFile]::OpenRead($env:NAB_COMPONENT_ZIP); try { foreach($e in $z.Entries) { $n=$e.Name; if($n -eq $env:NAB_COMPONENT_EXE -or $n -match '^[a-zA-Z0-9_.-]+\.dll$' -or $n -match '^(LICENSE|COPYING)([.][a-zA-Z0-9_-]+)?$') { if($e.Length -gt 67108864){throw 'Component entry exceeds size limit'}; if($n -match '^(LICENSE|COPYING)'){$n=$env:NAB_COMPONENT_ID+'-'+$n+'.txt'}; [IO.Compression.ZipFileExtensions]::ExtractToFile($e,[IO.Path]::Combine($env:NAB_COMPONENT_TARGET,$n),$false) } } } finally { $z.Dispose() }"#;
    let mut command=tokio::process::Command::new("powershell.exe");command.args(["-NoProfile","-NonInteractive","-Command",script]).env("NAB_COMPONENT_ZIP",&archive).env("NAB_COMPONENT_TARGET",&pending).env("NAB_COMPONENT_EXE",p.executable).env("NAB_COMPONENT_ID",id);
    #[cfg(windows)]command.creation_flags(0x08000000);
    let result=command.output().await.map_err(|_|"Could not unpack verified component archive.")?;
    if !result.status.success()||!pending.join(p.executable).is_file(){return Err("The verified archive could not be unpacked. Existing files were kept.".into());}
    let mut inventory=Inventory{files:vec![]};for entry in fs::read_dir(&pending).map_err(|e|e.to_string())?{let path=entry.map_err(|e|e.to_string())?.path();let name=path.file_name().ok_or("Invalid package entry.")?.to_string_lossy().into_owned();if name=="package.zip"{continue;}if root.join(&name).exists(){return Err("A package file would replace an existing component. Existing files were kept.".into());}inventory.files.push((name,digest(&path)?));}
    for(name,_)in &inventory.files{fs::rename(pending.join(name),root.join(name)).map_err(|e|e.to_string())?;}
    fs::write(root.join(format!("{id}.inventory.json")),serde_json::to_vec(&inventory).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    if id!="torrent"{fs::write(root.join(format!("{id}.enabled")),b"1").map_err(|e|e.to_string())?;}
    Ok(())
}
#[cfg(test)]mod tests{use super::*;#[test]fn sources_are_fixed_and_pinned(){for p in &PACKAGES{assert!(p.url.starts_with("https://github.com/"));assert!(p.url.contains("/releases/download/"));assert_eq!(p.hash.len(),64);assert!(p.bytes>0);}}}
