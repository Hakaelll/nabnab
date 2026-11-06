use crate::{cloud_storage, public_sources, queue, AppState};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::{Arc, Mutex}};
use tauri::State;
use tokio::sync::watch;

#[derive(Default)]
pub struct BrowserState {
    active: Mutex<HashMap<u64, watch::Sender<bool>>>,
    pages: Mutex<HashMap<String, public_sources::Page>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicSelection {
    pub source_url: String,
    pub asset_id: String,
    pub filename: String,
    pub sha256: Option<String>,
    pub version: Option<String>,
    pub size: Option<u64>,
    pub author: Option<String>,
    pub license: Option<String>,
    pub attribution: Option<String>,
    pub collection: String,
}

impl PublicSelection {
    pub fn validate(&self) -> Result<(), String> {
        let url = url::Url::parse(&self.source_url).map_err(|_|"Invalid public source identity.")?;
        if !public_sources::supported(&self.source_url) || url.query().is_some() || url.fragment().is_some()
            || self.asset_id.is_empty() || self.asset_id.len()>2048
            || self.asset_id.chars().any(char::is_control)
            || self.sha256.as_ref().is_some_and(|hash| hash.len()!=64 || !hash.bytes().all(|v|v.is_ascii_hexdigit()))
        {return Err("The selected public file identity or checksum is invalid.".into());}
        crate::direct::validate_filename(&self.filename)?;
        collection(&self.collection)?;
        Ok(())
    }
    pub fn verify(&self, asset: &public_sources::Asset) -> Result<(), String> {
        self.validate()?;
        if self.asset_id!=asset.id || self.source_url!=asset.source_url || self.filename!=asset.filename
            || self.version!=asset.version || self.size!=asset.size
            || self.sha256.as_ref().is_some_and(|hash|asset.sha256.as_ref()!=Some(hash))
        { return Err("The selected source file changed. Inspect it again to create a new local version.".into()); }
        Ok(())
    }
}

fn begin(state: &AppState, request: u64) -> Result<watch::Receiver<bool>, String> {
    let mut active = state.file_sources.active.lock().unwrap();
    if active.contains_key(&request) { return Err("This request identifier is already active.".into()); }
    if active.len() >= 8 { return Err("Finish or cancel an existing file source request first.".into()); }
    let (sender, receiver) = watch::channel(false);
    active.insert(request, sender);
    Ok(receiver)
}
fn finish(state: &AppState, request: u64) { state.file_sources.active.lock().unwrap().remove(&request); }

#[tauri::command]
pub fn cancel_file_source_inspection(state: State<Arc<AppState>>, request_id: u64) {
    if let Some(sender) = state.file_sources.active.lock().unwrap().remove(&request_id) { let _ = sender.send(true); }
}

#[tauri::command]
pub async fn public_source_inspect(state: State<'_, Arc<AppState>>, url: String, cursor: Option<String>, request_id: u64) -> Result<public_sources::Page, String> {
    let cancel = begin(&state, request_id)?;
    let result = public_sources::inspect(&url, cursor.as_deref(), cancel.clone()).await;
    finish(&state, request_id);
    let page = result?;
    if *cancel.borrow() { return Err(crate::engine::CANCELLED.into()); }
    let mut pages = state.file_sources.pages.lock().unwrap();
    if pages.len() >= 16 && !pages.contains_key(&page.source_url) { pages.clear(); }
    if cursor.is_some() {
        if let Some(previous) = pages.get_mut(&page.source_url) {
            for asset in &page.assets {
                if !previous.assets.iter().any(|a| a.id == asset.id) { previous.assets.push(asset.clone()); }
            }
            if previous.assets.len() > 2000 { previous.assets.drain(..previous.assets.len()-2000); }
        } else { pages.insert(page.source_url.clone(), page.clone()); }
    } else { pages.insert(page.source_url.clone(), page.clone()); }
    Ok(page)
}

fn collection(value: &str) -> Result<String, String> {
    if value.len() > 120 || value.chars().any(char::is_control) { return Err("Use a collection name up to 120 characters without control characters.".into()); }
    Ok(value.trim().to_owned())
}

#[tauri::command]
pub fn queue_public_files(state: State<Arc<AppState>>, source_url: String, asset_ids: Vec<String>, collection: String, client_request: String) -> Result<Vec<queue::Job>, String> {
    let collection = self::collection(&collection)?;
    if asset_ids.is_empty() || asset_ids.len() > 199 { return Err("Select 1–199 files from the inspected source.".into()); }
    let tasks = {
        let pages = state.file_sources.pages.lock().unwrap();
        let page = pages.get(&source_url).ok_or("Inspect the source again before downloading its files.")?;
        let mut tasks = Vec::new();
        let mut selected = std::collections::HashSet::new();
        let mut attribution = Vec::new();
        for id in asset_ids {
            if !selected.insert(id.clone()) { continue; }
            let asset = page.assets.iter().find(|a| a.id == id).ok_or("Select only files listed in the inspected source.")?;
            crate::direct::validate_filename(&asset.filename)?;
            let selection = PublicSelection { source_url: asset.source_url.clone(), asset_id: asset.id.clone(), filename: asset.filename.clone(), sha256: asset.sha256.clone(), version: asset.version.clone(), size: asset.size, author: asset.author.clone(), license: asset.license.clone(), attribution: asset.attribution.clone(), collection: collection.clone() };
            if matches!(page.provider, public_sources::Provider::Commons) { attribution.push(selection.clone()); }
            tasks.push((queue::Source::PublicFile { selection }, asset.title.clone()));
        }
        if !attribution.is_empty() { tasks.push((queue::Source::Attribution { entries: attribution }, "Wikimedia Commons · attribution".into())); }
        tasks
    };
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let jobs = queue::enqueue_tasks(repo, tasks, &client_request)?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}

#[tauri::command]
pub fn cloud_status(state: State<Arc<AppState>>) -> Result<Vec<cloud_storage::ConnectionStatus>, String> { cloud_storage::status(&crate::config_root(&state)?) }

#[tauri::command]
pub fn cloud_configure(state: State<Arc<AppState>>, provider: cloud_storage::Provider, client_id: String, client_secret: Option<String>) -> Result<cloud_storage::ConnectionStatus, String> {
    cloud_storage::configure(&crate::config_root(&state)?, provider, client_id, client_secret)
}

#[tauri::command]
pub async fn cloud_connect(state: State<'_, Arc<AppState>>, provider: cloud_storage::Provider, request_id: u64) -> Result<cloud_storage::ConnectionStatus, String> {
    let root = crate::config_root(&state)?;
    let cancel = begin(&state, request_id)?;
    let result = cloud_storage::connect(root, provider, cancel).await;
    finish(&state, request_id);
    result
}

#[tauri::command]
pub fn cloud_revoke(state: State<Arc<AppState>>, provider: cloud_storage::Provider) -> Result<cloud_storage::ConnectionStatus, String> { cloud_storage::revoke(&crate::config_root(&state)?, provider) }

#[tauri::command]
pub async fn cloud_preview_folder(state: State<'_, Arc<AppState>>, selection: cloud_storage::Selection, request_id: u64) -> Result<cloud_storage::Page, String> {
    let root = crate::config_root(&state)?;
    let cancel = begin(&state, request_id)?;
    let result = cloud_storage::preview_folder(&root, &selection, cancel).await;
    finish(&state, request_id);
    result
}

#[tauri::command]
pub async fn cloud_browse(state: State<'_, Arc<AppState>>, provider: cloud_storage::Provider, folder: Option<String>, cursor: Option<String>, shared_url: Option<String>, request_id: u64) -> Result<cloud_storage::Page, String> {
    let root = crate::config_root(&state)?;
    let cancel = begin(&state, request_id)?;
    let result = cloud_storage::browse(&root, provider, folder, cursor, shared_url, cancel).await;
    finish(&state, request_id);
    result
}

#[tauri::command]
pub fn queue_cloud_files(state: State<Arc<AppState>>, selections: Vec<cloud_storage::Selection>, collection: String, client_request: String) -> Result<Vec<queue::Job>, String> {
    let collection = self::collection(&collection)?;
    if selections.is_empty() || selections.len() > 200 { return Err("Select 1–200 authorized files.".into()); }
    let mut tasks = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for selection in selections {
        cloud_storage::validate_selection(&selection)?;
        if !seen.insert(serde_json::to_string(&selection).map_err(|e|e.to_string())?) { continue; }
        tasks.push((queue::Source::CloudFile { selection, collection: collection.clone() }, "Cloud file".into()));
    }
    let mut guard = state.repository.lock().unwrap();
    let repo = guard.as_mut().ok_or("Open nabnab first.")?;
    let jobs = queue::enqueue_tasks(repo, tasks, &client_request)?;
    state.runtime.notify.notify_one();
    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn selected() -> PublicSelection {
        PublicSelection { source_url:"https://github.com/example/tool/releases/tag/v1".into(), asset_id:"github:123".into(), filename:"tool.zip".into(), sha256:None, version:Some("2026-11-06".into()), size:Some(7), author:None, license:None, attribution:None, collection:"Releases".into() }
    }
    fn asset() -> public_sources::Asset {
        let selection=selected();
        public_sources::Asset {id:selection.asset_id,source_url:selection.source_url,download_url:"https://github.com/example/tool/releases/download/v1/tool.zip".into(),filename:selection.filename,mime:"application/zip".into(),size:selection.size,title:"tool.zip".into(),version:selection.version,original:true,author:None,license:None,attribution:None,sha256:None,platform:None,architecture:None}
    }
    #[test]
    fn selected_identity_rejects_source_parameters_and_control_paths() {
        let mut selection=selected(); selection.validate().unwrap();
        selection.source_url.push_str("?access_token=secret"); assert!(selection.validate().is_err());
        selection=selected();selection.filename="owner.json".into();assert!(selection.validate().is_err());
        selection=selected();selection.sha256=Some("short".into());assert!(selection.validate().is_err());
    }
    #[test]
    fn changed_file_requires_reselection_before_download_publication() {
        let selection=selected(); let mut current=asset(); selection.verify(&current).unwrap();
        current.version=Some("new".into());assert!(selection.verify(&current).is_err());
        current=asset();current.size=Some(8);assert!(selection.verify(&current).is_err());
        current=asset();current.id="github:456".into();assert!(selection.verify(&current).is_err());
    }
    #[test]
    fn persisted_jobs_contain_identity_and_metadata_without_download_url() {
        let source=queue::Source::PublicFile {selection:selected()};
        let serialized=serde_json::to_string(&source).unwrap();
        assert!(serialized.contains("assetId")&&serialized.contains("Releases"));
        assert!(!serialized.contains("downloadUrl")&&!serialized.contains("releases/download"));
        let restored:queue::Source=serde_json::from_str(&serialized).unwrap();
        assert!(matches!(restored,queue::Source::PublicFile {..}));
    }
    #[test]
    fn inspection_ids_cannot_cancel_or_remove_a_new_request() {
        let state=AppState::default();let receiver=begin(&state,7).unwrap();
        assert!(begin(&state,7).is_err());
        assert!(!*receiver.borrow());finish(&state,7);
        assert!(begin(&state,7).is_ok());
    }
}
