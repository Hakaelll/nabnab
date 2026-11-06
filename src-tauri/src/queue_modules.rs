use super::*;

#[tauri::command]
pub fn queue_advanced_file(app:AppHandle,state:tauri::State<Arc<AppState>>,url:String,connections:u8,checksum:Option<String>,client_request:String)->Result<Vec<Job>,String>{
    crate::model::validate_url(&url)?;if !(1..=16).contains(&connections){return Err("Choose 1–16 connections.".into());}let checksum=crate::advanced_files::validate_checksum(checksum.as_deref())?;crate::components::executable(&app,"files")?;
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    let jobs=enqueue_tasks(repo,vec![(Source::AdvancedFile{url,url_ref:None,connections,checksum},"HTTP file".into())],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}
#[tauri::command]
pub fn queue_palette(state:tauri::State<Arc<AppState>>,ids:Vec<String>,colors:u8,format:String,client_request:String)->Result<Vec<Job>,String>{
    if ids.is_empty()||ids.len()>200||!(2..=16).contains(&colors)||!matches!(format.as_str(),"css"|"hex"){return Err("Choose 1–200 images, 2–16 colors and HEX or CSS output.".into());}
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    for id in &ids{let item=repo.data.library.iter().find(|i|&i.id==id&&i.kind=="image").ok_or("Choose imported images.")?;library::authorized_path(&item.path)?;}
    let jobs=enqueue_tasks(repo,vec![(Source::Palette{library_ids:ids,colors,format},"Palette".into())],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}
#[tauri::command]
pub fn queue_audio_preview(state:tauri::State<Arc<AppState>>,library_id:String,options:ToolOptions,client_request:String)->Result<Vec<Job>,String>{
    crate::audio_preview::validate_options(&options)?;let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    let item=repo.data.library.iter().find(|i|i.id==library_id&&matches!(i.kind.as_str(),"audio"|"video")).ok_or("Choose imported audio or video.")?;library::authorized_path(&item.path)?;
    let jobs=enqueue_tasks(repo,vec![(Source::AudioPreview{library_id,options},"Audio preview · 20 s".into())],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}

#[tauri::command]
pub fn queue_source_export(state:tauri::State<Arc<AppState>>,url:String,operation:String,format:String,client_request:String)->Result<Vec<Job>,String>{
    crate::source_exports::validate(&url,&operation,&format)?;
    let title=format!("{} · {}",providers::identify(&url)?.name,operation);
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    let jobs=enqueue_tasks(repo,vec![(Source::ProviderExport{url,url_ref:None,operation,format},title)],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}

#[tauri::command]
pub fn queue_caption_edit(state:tauri::State<Arc<AppState>>,library_id:String,track_id:String,options:crate::captions_editor::CaptionEditOptions,client_request:String)->Result<Vec<Job>,String>{
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    let input=repo.data.library.iter().find(|i|i.id==library_id).ok_or("Choose a subtitle source in the library.")?;
    let track=input.details.tracks.iter().find(|t|t.id==track_id).ok_or("Choose an existing subtitle track.")?;
    crate::captions_editor::transform_segments(&track.segments,&options)?;
    let title=format!("{} · subtitles",input.name);
    let jobs=enqueue_tasks(repo,vec![(Source::CaptionEdit{library_id,track_id,options},title)],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}
#[tauri::command]
pub fn queue_media_edit(app:AppHandle,state:tauri::State<Arc<AppState>>,ids:Vec<String>,options:crate::media_edits::MediaEditOptions,client_request:String)->Result<Vec<Job>,String>{
    crate::media_edits::validate_options(&options,ids.len())?;
    let _=crate::dependencies::engine_root(&app)?;
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    for id in &ids{let item=repo.data.library.iter().find(|i|&i.id==id).ok_or("Choose imported local media.")?;library::authorized_path(&item.path)?;}
    let jobs=enqueue_tasks(repo,vec![(Source::MediaEdit{library_ids:ids,options},"Media composition".into())],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}

pub(super) async fn run(app:&AppHandle,state:&Arc<AppState>,job:&Job,staging:&Path,cancel:watch::Receiver<bool>)->Result<Option<PublishedOutput>,String>{
    let handle=app.clone();let shared=state.clone();let id=job.id.clone();let run_id=job.run_id;
    let emit=move|update|progress(&handle,&shared,&id,run_id,update);
    let output=match &job.source{
        Source::AdvancedFile{url_ref,connections,checksum,..}=>{
            let root={let guard=state.repository.lock().unwrap();guard.as_ref().ok_or("Open nabnab first.")?.root().to_owned()};let url=crate::sessions::resolve_url(&root,url_ref.as_deref().ok_or("The file link is unavailable. Add it again.")?)?;
            phase(app,state,&job.id,run_id,JobState::Downloading)?;
            Some((crate::advanced_files::execute(app,url.as_str(),staging,*connections,checksum.as_deref(),cancel.clone(),run_id,emit.clone()).await?,Some(public_origin(url.as_str())?),None))
        },
        Source::Palette{library_ids,colors,format}=>{
            let inputs={let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open nabnab first.")?;library_ids.iter().map(|id|repo.data.library.iter().find(|i|&i.id==id).cloned().ok_or("The image is no longer in the library.")).collect::<Result<Vec<_>,_>>()?};let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            Some((crate::palettes::execute(&crate::dependencies::engine_root(app)?,&inputs,staging,*colors,format,cancel.clone(),run_id,emit.clone()).await?,None,None))
        },
        Source::AudioPreview{library_id,options}=>{
            let input={let guard=state.repository.lock().unwrap();guard.as_ref().ok_or("Open nabnab first.")?.data.library.iter().find(|i|&i.id==library_id).cloned().ok_or("The audio source is no longer in the library.")?};let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            Some((crate::audio_preview::execute(&crate::dependencies::engine_root(app)?,&input,staging,options,cancel.clone(),run_id,emit.clone()).await?,None,Some(library_id.clone())))
        },
        Source::ProviderExport{url_ref,operation,format,..}=>{
            let root={let guard=state.repository.lock().unwrap();guard.as_ref().ok_or("Open nabnab first.")?.root().to_owned()};
            let operational=crate::sessions::resolve_url(&root,url_ref.as_deref().ok_or("The export source is unavailable. Add it again.")?)?;
            phase(app,state,&job.id,run_id,JobState::Downloading)?;
            Some((crate::source_exports::execute(app,state,&crate::dependencies::engine_root(app)?,operational.as_str(),operation,format,staging,cancel.clone(),run_id,emit.clone()).await?,Some(public_origin(operational.as_str())?),None))
        },
        Source::CaptionEdit{library_id,track_id,options}=>{
            let input={let guard=state.repository.lock().unwrap();guard.as_ref().ok_or("Open nabnab first.")?.data.library.iter().find(|i|&i.id==library_id).cloned().ok_or("The subtitle source is no longer in the library.")?};
            phase(app,state,&job.id,run_id,JobState::Processing)?;
            Some((crate::captions_editor::execute(&input,track_id,options,staging)?,None,Some(library_id.clone())))
        },
        Source::MediaEdit{library_ids,options}=>{
            let inputs={let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open nabnab first.")?;library_ids.iter().map(|id|repo.data.library.iter().find(|i|&i.id==id).cloned().ok_or("The selected media is no longer in the library.")).collect::<Result<Vec<_>,_>>()?};
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            Some((crate::media_edits::execute(&crate::dependencies::engine_root(app)?,&inputs,staging,options,cancel.clone(),run_id,emit.clone()).await?,None,library_ids.first().cloned()))
        },
        Source::Document{library_ids,options}=>{
            let paths={let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open nabnab first.")?;library_ids.iter().map(|id|repo.data.library.iter().find(|i|&i.id==id).map(|i|i.path.clone()).ok_or("A selected document is no longer in the library.")).collect::<Result<Vec<_>,_>>()?};
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            Some((crate::documents::execute(&crate::components::executable(app,"documents")?,&paths,staging,options,cancel.clone(),run_id,emit.clone()).await?,None,None))
        },
        Source::GalleryExport{library_ids,export_kind}=>{
            let inputs={let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open nabnab first.")?;library_ids.iter().map(|id|repo.data.library.iter().find(|i|&i.id==id).cloned().ok_or("A selected gallery file is no longer in the library.")).collect::<Result<Vec<_>,_>>()?};
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            Some((crate::gallery_exports::execute(&crate::dependencies::engine_root(app)?,&inputs,staging,export_kind,cancel.clone(),run_id,emit.clone()).await?,None,None))
        },
        Source::Telegram{attachment}=>{phase(app,state,&job.id,run_id,JobState::Downloading)?;Some((crate::telegram::download(app,attachment,staging,cancel.clone(),run_id,emit.clone()).await?,None,None))},
        Source::Live{url_ref,minutes,max_mb,..}=>{
            let root={let guard=state.repository.lock().unwrap();guard.as_ref().ok_or("Open nabnab first.")?.root().to_owned()};
            let operational=crate::sessions::resolve_url(&root,url_ref.as_deref().ok_or("The recording link is unavailable. Add it again.")?)?;
            phase(app,state,&job.id,run_id,JobState::Downloading)?;
            Some((crate::live::record(app,state,&job.id,operational.as_str(),staging,*minutes,*max_mb,cancel.clone(),run_id,emit.clone()).await?,Some(public_origin(operational.as_str())?),None))
        },
        _=>None,
    };
    let Some((file,source,parent))=output else{return Ok(None);};
    phase(app,state,&job.id,run_id,JobState::Verifying)?;
    let mut item=library::index(&file,None,source,parent).or_else(|_|library::index_file(&file))?;
    if let Source::CaptionEdit{library_id,track_id,..}=&job.source{let guard=state.repository.lock().unwrap();if let Some(track)=guard.as_ref().and_then(|repo|repo.data.library.iter().find(|i|&i.id==library_id)).and_then(|i|i.details.tracks.iter().find(|t|&t.id==track_id)){for generated in &mut item.details.tracks{generated.language=track.language.clone();}}}
    annotate_output(&mut item,job,staging);
    Ok(Some(publish_verified_output(state,job,staging,&file,item,cancel).await?))
}

#[tauri::command]
pub fn queue_document(app:AppHandle,state:tauri::State<Arc<AppState>>,ids:Vec<String>,options:crate::documents::DocumentOptions,client_request:String)->Result<Vec<Job>,String>{
    crate::components::executable(&app,"documents")?;crate::documents::validate_options(&options)?;
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    if ids.is_empty()||ids.len()>100{return Err("Select 1–100 imported PDF files.".into());}
    for id in &ids{let item=repo.data.library.iter().find(|i|&i.id==id).ok_or("Choose an imported document.")?;let path=library::authorized_path(&item.path)?;if !path.extension().is_some_and(|e|e.eq_ignore_ascii_case("pdf")){return Err("PDF tools require imported PDF files.".into());}}
    let jobs=enqueue_tasks(repo,vec![(Source::Document{library_ids:ids,options},"PDF".into())],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}
#[tauri::command]
pub fn queue_gallery_export(state:tauri::State<Arc<AppState>>,ids:Vec<String>,kind:String,client_request:String)->Result<Vec<Job>,String>{
    if !matches!(kind.as_str(),"html"|"contactSheet")||ids.is_empty()||ids.len()>200{return Err("Select 1–200 local files and a supported export format.".into());}
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    for id in &ids{let item=repo.data.library.iter().find(|i|&i.id==id).ok_or("Choose imported gallery files.")?;library::authorized_path(&item.path)?;}
    let title=if kind=="html"{"Galería HTML"}else{"Hoja de contacto"};
    let jobs=enqueue_tasks(repo,vec![(Source::GalleryExport{library_ids:ids,export_kind:kind},title.into())],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}
#[tauri::command]
pub async fn queue_telegram(state:tauri::State<'_,Arc<AppState>>,attachments:Vec<crate::telegram::TelegramAttachment>,collection:String,client_request:String)->Result<Vec<Job>,String>{
    if attachments.is_empty()||attachments.len()>200||collection.len()>120{return Err("Select 1–200 authorized attachments and a collection name up to 120 characters.".into());}
    let mut checked=Vec::new();let mut identities=std::collections::HashSet::new();
    for attachment in attachments{let attachment=crate::telegram::validate_attachment(&attachment).await?;if identities.insert(attachment.id.clone()){checked.push(attachment);}}
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    let previous:Vec<_>=repo.data.jobs.iter().filter(|j|j.client_request==client_request).cloned().collect();if !previous.is_empty(){return Ok(previous);}
    let checked:Vec<_>=checked.into_iter().filter(|a|!repo.data.jobs.iter().any(|j|j.content_id.as_deref()==Some(&a.id)&&!matches!(j.state,JobState::Error|JobState::Cancelled))).collect();if checked.is_empty(){return Ok(vec![]);}
    let jobs=enqueue_tasks(repo,checked.iter().map(|a|(Source::Telegram{attachment:a.clone()},a.title.clone())).collect(),&client_request)?;
    let mut result=Vec::new();for(job,attachment)in jobs.iter().zip(&checked){let item=repo.data.jobs.iter_mut().find(|j|j.id==job.id).ok_or("Queued attachment disappeared.")?;item.content_id=Some(attachment.id.clone());item.associations.push(OutputAssociation{source_id:Some(format!("telegram:{}:{}",attachment.account_id,attachment.chat_id)),collection:(!collection.trim().is_empty()).then(||collection.trim().to_owned()),tags:vec![]});result.push(item.clone());}repo.save()?;state.runtime.notify.notify_one();Ok(result)
}
#[tauri::command]
pub fn queue_live(state:tauri::State<Arc<AppState>>,url:String,minutes:u32,max_mb:Option<u32>,client_request:String)->Result<Vec<Job>,String>{
    crate::live::validate(&url,minutes,max_mb)?;
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    let jobs=enqueue_tasks(repo,vec![(Source::Live{url,url_ref:None,minutes,max_mb},"Directo".into())],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}
