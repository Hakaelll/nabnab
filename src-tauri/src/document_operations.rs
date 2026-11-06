use crate::{document_edits, ocr, queue, AppState};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, path::PathBuf, sync::{Arc,Mutex}};
use tauri::{AppHandle,State};
use tokio::sync::watch;

#[derive(Default)]
pub struct PreviewState { approved:Mutex<HashMap<String,Approval>> }
struct Approval {id:String,options:String,input_hash:String,expires:u64}
use crate::workspaces::Workspace;

fn input(state:&AppState,id:&str)->Result<(PathBuf,crate::library::LibraryItem),String>{
    let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open nabnab first.")?;let item=repo.data.library.iter().find(|i|i.id==id).cloned().ok_or("Choose a local library file.")?;crate::library::authorized_path(&item.path)?;Ok((repo.root().to_owned(),item))
}
fn pdf(item:&crate::library::LibraryItem)->Result<(),String>{if !item.path.extension().is_some_and(|e|e.eq_ignore_ascii_case("pdf")){return Err("Choose a PDF from the library for this operation.".into());}Ok(())}
#[tauri::command]
pub async fn document_edit_info(app:AppHandle,state:State<'_,Arc<AppState>>,id:String)->Result<document_edits::Info,String>{
    let(root,item)=input(&state,&id)?;pdf(&item)?;let workspace=Workspace::new(&root)?;let(_sender,cancel)=watch::channel(false);
    document_edits::inspect(&crate::components::executable(&app,"documents")?,&item.path,&workspace.path,cancel,queue::now()).await
}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Preview {#[serde(flatten)]preview:document_edits::Preview,preview_id:String}
#[tauri::command]
pub async fn document_edit_preview(app:AppHandle,state:State<'_,Arc<AppState>>,id:String,options:document_edits::Options,page:u32,request_id:u64)->Result<Preview,String>{
    document_edits::validate_options(&options)?;let(root,item)=input(&state,&id)?;pdf(&item)?;
    let executable=crate::components::executable(&app,"documents")?;let workspace=Workspace::new(&root)?;let cancel=crate::file_sources::begin(&state,request_id)?;
    let result=async{
        let before=queue::digest(item.path.clone(),cancel.clone()).await?;
        let preview=document_edits::preview(&executable,&item.path,&workspace.path,&options,page,cancel.clone(),request_id).await?;
        if queue::digest(item.path.clone(),cancel.clone()).await?!=before{return Err("The PDF changed while its preview was generated. Review it again.".into());}
        if *cancel.borrow(){return Err(crate::engine::CANCELLED.into());}
        let serialized=serde_json::to_string(&options).map_err(|e|e.to_string())?;
        let preview_id=format!("{:x}",Sha256::digest(format!("{id}:{before}:{serialized}:{}",workspace.path.display()).as_bytes()));
        let mut approvals=state.document_previews.approved.lock().unwrap();approvals.retain(|_,a|a.expires>queue::now());if approvals.len()>=32{approvals.clear();}
        approvals.insert(preview_id.clone(),Approval{id,options:serialized,input_hash:before,expires:queue::now()+10*60*1000});Ok(Preview{preview,preview_id})
    }.await;
    crate::file_sources::finish(&state,request_id);result
}
#[tauri::command]
pub fn cancel_document_edit_preview(state:State<Arc<AppState>>,request_id:u64){crate::file_sources::cancel_file_source_inspection(state,request_id);}
#[tauri::command]
pub async fn queue_document_edit(app:AppHandle,state:State<'_,Arc<AppState>>,id:String,options:document_edits::Options,consent:bool,preview_id:String,client_request:String)->Result<Vec<queue::Job>,String>{
    if !consent{return Err("Review the preview and acknowledge possible changes to forms, signatures and document structure.".into());}
    document_edits::validate_options(&options)?;crate::components::executable(&app,"documents")?;
    let(_,item)=input(&state,&id)?;pdf(&item)?;let(_sender,cancel)=watch::channel(false);let input_hash=queue::digest(item.path.clone(),cancel).await?;
    let serialized=serde_json::to_string(&options).map_err(|e|e.to_string())?;
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    let previous:Vec<_>=repo.data.jobs.iter().filter(|j|j.client_request==client_request).cloned().collect();if !previous.is_empty(){return Ok(previous);}
    let mut approvals=state.document_previews.approved.lock().unwrap();let approval=approvals.get(&preview_id).ok_or("Generate a new PDF preview before adding this operation to the queue.")?;
    if approval.id!=id||approval.options!=serialized||approval.input_hash!=input_hash||approval.expires<=queue::now(){return Err("The preview expired or no longer matches the input and options. Review a new preview.".into());}
    let jobs=queue::enqueue_tasks(repo,vec![(queue::Source::DocumentEdit{library_id:id,options,input_hash},format!("{} · PDF",item.name))],&client_request)?;
    approvals.remove(&preview_id);state.runtime.notify.notify_one();Ok(jobs)
}

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct OcrStatus {installed:bool,enabled:bool}
#[tauri::command]
pub fn ocr_status(app:AppHandle)->Result<OcrStatus,String>{let root=crate::components::root(&app)?;Ok(OcrStatus{installed:crate::ocr_component::installed(&root),enabled:root.join("ocr.enabled").is_file()})}
#[tauri::command]
pub fn ocr_models(app:AppHandle)->Result<Vec<crate::ocr_component::ModelStatus>,String>{Ok(crate::ocr_component::models(&crate::components::root(&app)?))}
#[tauri::command]
pub async fn install_ocr_model(app:AppHandle,id:String)->Result<(),String>{crate::ocr_component::install_model(&app,&id).await}
#[tauri::command]
pub fn queue_ocr(app:AppHandle,state:State<Arc<AppState>>,id:String,options:ocr::OcrOptions,client_request:String)->Result<Vec<queue::Job>,String>{
    ocr::validate_options(&options)?;let root=crate::components::root(&app)?;crate::components::executable(&app,"ocr")?;crate::ocr_component::verify_models(&root,&options.languages)?;
    let(_,item)=input(&state,&id)?;if !(item.kind=="image"||item.path.extension().is_some_and(|e|e.eq_ignore_ascii_case("pdf"))){return Err("Choose an image or PDF from the library.".into());}
    if item.path.extension().is_some_and(|e|e.eq_ignore_ascii_case("pdf")){crate::components::executable(&app,"documents")?;}
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    let jobs=queue::enqueue_tasks(repo,vec![(queue::Source::Ocr{library_id:id,options},format!("{} · OCR",item.name))],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}
fn indexed(state:&AppState,id:&str)->Result<(PathBuf,String),String>{let(root,item)=input(state,id)?;Ok((root,item.details.ocr_ref.ok_or("This file has no generated OCR index.")?))}
#[tauri::command]
pub fn ocr_get(state:State<Arc<AppState>>,id:String)->Result<ocr::OcrDocument,String>{let(root,reference)=indexed(&state,&id)?;ocr::read_index(&root,&reference)}
#[derive(Serialize)]
pub struct OcrPreview {png:Vec<u8>}
#[tauri::command]
pub async fn ocr_preview(app:AppHandle,state:State<'_,Arc<AppState>>,id:String,page:u32)->Result<OcrPreview,String>{
    let(root,reference)=indexed(&state,&id)?;let index=ocr::read_index(&root,&reference)?;
    let selected=index.pages.iter().find(|p|p.page==page).cloned().ok_or("Choose an indexed OCR page.")?;
    let candidates={let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open nabnab first.")?;let mut candidates=Vec::new();let mut current=Some(id.as_str());let mut seen=std::collections::HashSet::new();while let Some(id)=current{if !seen.insert(id.to_owned())||candidates.len()>=32{break;}let Some(item)=repo.data.library.iter().find(|i|i.id==id)else{break;};candidates.push(item.clone());current=item.parent_id.as_deref();}candidates};
    let(_sender,cancel)=watch::channel(false);let mut source=None;
    for item in candidates.into_iter().filter(|i|i.kind=="image"||i.path.extension().is_some_and(|e|e.eq_ignore_ascii_case("pdf"))){if let Ok(hash)=queue::digest(item.path.clone(),cancel.clone()).await{if hash==index.source_fingerprint{source=Some(item);break;}}}
    let source=source.ok_or("The original OCR source is missing or changed. Its old index was kept; run OCR again before showing positions on a new image.")?;
    if source.path.extension().is_some_and(|e|e.eq_ignore_ascii_case("pdf")){return Ok(OcrPreview{png:crate::documents::document_page(app,state,source.id,page).await?.png});}
    let workspace=Workspace::new(&root)?;let output=workspace.path.join("ocr-preview.png");
    let crop=selected.region.map(|r|format!("crop={}:{}:{}:{},",r.width,r.height,r.x,r.y)).unwrap_or_default();
    let args=vec!["-hide_banner".into(),"-nostdin".into(),"-v".into(),"error".into(),"-protocol_whitelist".into(),"file,pipe".into(),"-i".into(),source.path.to_string_lossy().into_owned(),"-vf".into(),format!("{crop}scale=1600:1600:force_original_aspect_ratio=decrease"),"-frames:v".into(),"1".into(),"-threads".into(),"2".into(),"-n".into(),output.to_string_lossy().into_owned()];
    crate::engine::run_process(crate::dependencies::engine_root(&app)?.join("ffmpeg.exe"),args,cancel,queue::now(),true,|_|{}).await?;
    let output=crate::checked_output(&workspace.path,&output.to_string_lossy())?;if fs::metadata(&output).map_err(|e|e.to_string())?.len()>8*1024*1024{return Err("The OCR preview exceeds its 8 MiB limit.".into());}
    Ok(OcrPreview{png:fs::read(output).map_err(|e|e.to_string())?})
}
#[tauri::command]
pub fn ocr_correct_page(state:State<Arc<AppState>>,id:String,page:u32,text:String)->Result<ocr::OcrDocument,String>{let(root,reference)=indexed(&state,&id)?;ocr::correct_page(&root,&reference,page,&text)}
#[tauri::command]
pub fn ocr_remove_generated(state:State<Arc<AppState>>,id:String)->Result<ocr::OcrDocument,String>{let(root,reference)=indexed(&state,&id)?;ocr::remove_generated(&root,&reference)}
#[tauri::command]
pub fn ocr_search(state:State<Arc<AppState>>,id:String,query:String)->Result<Vec<ocr::Hit>,String>{let(root,reference)=indexed(&state,&id)?;ocr::search(&ocr::read_index(&root,&reference)?,&query)}
#[tauri::command]
pub fn ocr_save_text(state:State<Arc<AppState>>,id:String,client_request:String)->Result<Vec<queue::Job>,String>{
    let(root,reference)=indexed(&state,&id)?;let text=ocr::corrected_text(&ocr::read_index(&root,&reference)?);if text.trim().is_empty(){return Err("There is no indexed text to export.".into());}
    let expected_hash=format!("{:x}",Sha256::digest(text.as_bytes()));let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    let jobs=queue::enqueue_tasks(repo,vec![(queue::Source::OcrText{library_id:id,reference,expected_hash},"OCR · corrected text".into())],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}
