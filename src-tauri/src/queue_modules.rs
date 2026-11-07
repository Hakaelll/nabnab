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
    let mut narration=None;
    let mut media_info=None;
    let output=match &job.source{
        Source::Voice {plan_id}=>{
            if !crate::components::root(app)?.join("voice.enabled").is_file(){return Err("Enable Local voice in Settings → Components before generating narration.".into());}
            let (root,_)=crate::file_operations::context(state)?;
            let plan=crate::voice::read_plan(&root,plan_id)?;
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            let output=crate::voice::execute(&root,plan_id,staging,cancel.clone(),run_id,emit.clone()).await?;
            let file=output.file.clone();let parent=plan.origin.library_id.clone();
            narration=Some((plan,output));
            Some((file,None,parent))
        },
        Source::Capture {capture_id}=>{
            let (root,_)=crate::file_operations::context(state)?;
            let journal=crate::screen_capture::readjournal(&root,capture_id)?;
            if crate::library::authorized_path(&journal.destination)?!=crate::library::authorized_path(&job.folder)?{return Err("The capture's authorized output directory changed. Review its recovery before continuing.".into());}
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            let engines=crate::dependencies::engine_root(app)?;
            let file=crate::screen_capture::finalize(&root,&engines,capture_id,staging,cancel.clone(),run_id,emit.clone()).await?;
            let info=crate::media::probe(&engines,&file,cancel.clone()).await?;
            media_info=Some(info);
            Some((file,None,None))
        },
        Source::ImageAssets {library_ids,options,input_hashes}=>{
            let (_,library)=crate::file_operations::context(state)?;
            let inputs=crate::file_operations::selected(&library,library_ids,256)?;
            if inputs.len()!=input_hashes.len(){return Err("The image preview is incomplete. Review it again.".into());}
            for (input,expected) in inputs.iter().zip(input_hashes){if &digest(input.path.clone(),cancel.clone()).await?!=expected{return Err("An original image changed after its preview. Review it again.".into());}}
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            let file=crate::image_assets::execute(&crate::dependencies::engine_root(app)?,&inputs,staging,options,cancel.clone(),run_id,emit.clone()).await?;
            for (input,expected) in inputs.iter().zip(input_hashes){if &digest(input.path.clone(),cancel.clone()).await?!=expected{return Err("An image changed during production. The proposed output was not published; review a new preview.".into());}}
            Some((file,None,library_ids.first().cloned()))
        },
        Source::Sticker { selection } => {
            selection.validate()?;
            let (_, library) = crate::file_operations::context(state)?;
            let input = crate::file_operations::selected(&library, std::slice::from_ref(&selection.library_id), 1)?.remove(0);
            let _permit = processing_permit(state, cancel.clone()).await?;
            phase(app, state, &job.id, run_id, JobState::Processing)?;
            if digest(input.path.clone(), cancel.clone()).await? != selection.input_hash {
                return Err("The sticker source changed after preview. Review it again.".into());
            }
            let engines = crate::dependencies::engine_root(app)?;
            let artifact = crate::stickers::encode(&engines, &input.path, staging, &selection.options, cancel.clone(), run_id).await?;
            if digest(input.path, cancel.clone()).await? != selection.input_hash {
                return Err("The sticker source changed during processing. Preview it again.".into());
            }
            media_info = Some(artifact.info);
            Some((artifact.path, None, Some(selection.library_id.clone())))
        },
        Source::Upscale { selection } => {
            selection.validate()?;
            let (_, library) = crate::file_operations::context(state)?;
            let input = crate::file_operations::selected(&library, std::slice::from_ref(&selection.library_id), 1)?.remove(0);
            let _gpu = state.upscale.permit(cancel.clone()).await?;
            let _processing = processing_permit(state, cancel.clone()).await?;
            phase(app, state, &job.id, run_id, JobState::Processing)?;
            if digest(input.path.clone(), cancel.clone()).await? != selection.input_hash {
                return Err("The image changed after the upscale preview. Review it again.".into());
            }
            let runtime = crate::upscale::component::ready(&crate::components::root(app)?, &selection.options.model_files())?;
            let engines = crate::dependencies::engine_root(app)?;
            let output = crate::upscale::execute(&engines, &runtime, staging, &input.path, selection.options, cancel.clone(), run_id).await?;
            if digest(input.path, cancel.clone()).await? != selection.input_hash {
                return Err("The image changed during upscaling. Preview the source again.".into());
            }
            media_info = Some(crate::media::probe(&engines, &output, cancel.clone()).await?);
            Some((output, None, Some(selection.library_id.clone())))
        },
        Source::FileAnalysis {library_ids,options}=>{
            let (_,library)=crate::file_operations::context(state)?;
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            let ffmpeg=if matches!(options,crate::file_organization::AnalyzeOptions::Similar {..}){crate::dependencies::engine_root(app)?.join("ffmpeg.exe")}else{PathBuf::new()};
            let analysis=crate::file_organization::analyze(&ffmpeg,staging,&library,library_ids,options,cancel.clone(),run_id,emit.clone()).await?;
            let file=staging.join(format!("file-analysis-{run_id}.json"));
            write_report(&file,&analysis)?;
            Some((file,None,None))
        },
        Source::FileRename {..}|Source::FileUndo {..}=>{
            let (operation_id,undo)=match &job.source{Source::FileRename {plan_id}=>(plan_id.clone(),false),Source::FileUndo {operation_id}=>(operation_id.clone(),true),_=>unreachable!()};
            let (root,library)=crate::file_operations::context(state)?;
            phase(app,state,&job.id,run_id,JobState::Processing)?;
            let token=cancel.clone();let updates=emit.clone();
            let verified=tokio::task::spawn_blocking(move||crate::file_organization::preflight_rename(&root,&library,&operation_id,undo,token,run_id,updates)).await.map_err(|e|e.to_string())??;
            let shared=state.clone();let token=cancel.clone();
            let journal=tokio::task::spawn_blocking(move||{
                let mut guard=shared.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
                if undo{crate::file_organization::undo_rename(repo,verified,token)}else{crate::file_organization::execute_rename(repo,verified,token)}
            }).await.map_err(|e|e.to_string())??;
            let file=staging.join(format!("rename-{}-{run_id}.json",journal.id));
            write_report(&file,&journal)?;
            if !journal.successful(){return Err(format!("The rename operation stopped with partial results. Open Review and rename → History to inspect, resume or undo operation {}. {}",journal.id,journal.error.as_deref().unwrap_or("Some files were preserved at their previous location.")));}
            Some((file,None,None))
        },
        Source::PortableExport {library_ids,options}=>{
            let (root,library)=crate::file_operations::context(state)?;
            let inputs=crate::file_operations::selected(&library,library_ids,2000)?;
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            Some((crate::portable_collections::execute_export(&root,&inputs,staging,options,cancel.clone(),run_id,emit.clone()).await?,None,None))
        },
        Source::PortableImport {plan_id}=>{
            let (root,_)=crate::file_operations::context(state)?;
            crate::portable_collections::validate_import_plan(&root,plan_id,&job.folder)?;
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            let prepared=crate::portable_collections::execute_import(&root,plan_id,&job.folder,cancel.clone(),run_id,emit.clone()).await?;
            if *cancel.borrow(){return Err(crate::engine::CANCELLED.into());}
            {let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;crate::portable_collections::commit_import(repo,&prepared)?;}
            let file=staging.join(format!("collection-import-{run_id}.json"));
            fs::copy(&prepared.report_path,&file).map_err(|e|e.to_string())?;
            Some((file,None,None))
        },
        Source::DocumentEdit {library_id,options,input_hash}=>{
            let input={let guard=state.repository.lock().unwrap();guard.as_ref().ok_or("Open nabnab first.")?.data.library.iter().find(|i|&i.id==library_id).cloned().ok_or("The PDF is no longer in the library.")?};
            if &digest(input.path.clone(),cancel.clone()).await?!=input_hash{return Err("The original PDF changed after its preview. Review it again.".into());}
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            Some((crate::document_edits::execute(&crate::components::executable(app,"documents")?,&input.path,staging,options,cancel.clone(),run_id,emit.clone()).await?,None,Some(library_id.clone())))
        },
        Source::Ocr {library_id,options}=>{
            let (root,input)={let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open nabnab first.")?;(repo.root().to_owned(),repo.data.library.iter().find(|i|&i.id==library_id).cloned().ok_or("The OCR source is no longer in the library.")?)};
            let component_root=crate::components::root(app)?;let tesseract=crate::components::executable(app,"ocr")?;let models=crate::ocr_component::verify_models(&component_root,&options.languages)?;
            let mutool=if input.path.extension().is_some_and(|e|e.eq_ignore_ascii_case("pdf")){Some(crate::components::executable(app,"documents")?)}else{None};
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            let result=crate::ocr::execute(&tesseract,&models,mutool.as_deref(),&crate::dependencies::engine_root(app)?.join("ffmpeg.exe"),&input.path,staging,options,cancel.clone(),run_id,emit.clone()).await?;
            crate::ocr::write_index(&root,&format!("{}-{}",job.id,job.run_id),&result.index)?;
            Some((result.file,None,Some(library_id.clone())))
        },
        Source::OcrText {library_id,reference,expected_hash}=>{
            let root={let guard=state.repository.lock().unwrap();guard.as_ref().ok_or("Open nabnab first.")?.root().to_owned()};
            let text=crate::ocr::corrected_text(&crate::ocr::read_index(&root,reference)?);
            if &format!("{:x}",Sha256::digest(text.as_bytes()))!=expected_hash{return Err("The corrected OCR text changed. Export it again from the current index.".into());}
            let file=staging.join(format!("ocr-corrected-{run_id}.txt"));let mut writer=fs::OpenOptions::new().create_new(true).write(true).open(&file).map_err(|e|e.to_string())?;writer.write_all(text.as_bytes()).and_then(|_|writer.sync_all()).map_err(|e|e.to_string())?;
            Some((file,None,Some(library_id.clone())))
        },
        Source::WebArchive {reference,format,..}=>{
            let root={let guard=state.repository.lock().unwrap();guard.as_ref().ok_or("Open nabnab first.")?.root().to_owned()};
            let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            let capture=crate::web_archive::load(&root,reference)?;
            Some((crate::web_archive::execute(&root,reference,*format,staging,cancel.clone(),run_id,emit.clone()).await?,Some(capture.url),None))
        },
        Source::PublicFile {selection} => {
            phase(app,state,&job.id,run_id,JobState::Downloading)?;
            let asset=crate::public_sources::resolve(&selection.source_url,&selection.asset_id,cancel.clone()).await?;
            selection.verify(&asset)?;
            let file=crate::direct::download_file(&asset.download_url,staging,&asset.filename,cancel.clone(),run_id,emit.clone()).await?;
            if let Some(expected)=&asset.sha256 {
                if !digest(file.clone(),cancel.clone()).await?.eq_ignore_ascii_case(expected) {
                    return Err("The downloaded file does not match its published SHA-256. It was not added to the library.".into());
                }
            }
            if let Some(expected) = asset.checksum.as_deref().and_then(|value| value.strip_prefix("md5:")) {
                if !md5_digest(file.clone(), cancel.clone()).await?.eq_ignore_ascii_case(expected) {
                    return Err("The downloaded file does not match its published MD5 checksum. It was not added to the library.".into());
                }
            }
            if asset.id.starts_with("arxiv:") {
                let mut signature = [0u8; 5];
                if fs::File::open(&file).and_then(|mut file| file.read_exact(&mut signature)).is_err()
                    || &signature != b"%PDF-"
                {
                    return Err("arXiv did not return a PDF. No document was added to the library.".into());
                }
            }
            Some((file,Some(selection.source_url.clone()),None))
        },
        Source::CloudFile {selection,..} => {
            let root={let guard=state.repository.lock().unwrap();guard.as_ref().ok_or("Open nabnab first.")?.root().to_owned()};
            phase(app,state,&job.id,run_id,JobState::Downloading)?;
            let files=crate::cloud_storage::execute(&root,selection,staging,cancel.clone(),run_id,emit.clone()).await?;
            if files.len()!=1 {return Err("Select individual files after reviewing the folder contents.".into());}
            Some((files.into_iter().next().ok_or("No cloud file was downloaded.")?,None,None))
        },
        Source::PersonalFile {selection} => {
            phase(app, state, &job.id, run_id, JobState::Downloading)?;
            let file = crate::personal_servers::execute(state, selection, staging, run_id, cancel.clone()).await?;
            Some((file, None, None))
        },
        Source::Attribution {entries} => {
            let _permit=processing_permit(state,cancel.clone()).await?;
            phase(app,state,&job.id,run_id,JobState::Processing)?;
            let file=staging.join(format!("commons-attribution-{}.json",job.id));
            let bytes=serde_json::to_vec_pretty(&serde_json::json!({"schemaVersion":1,"archivedAt":now(),"notice":"Metadata as reported by each source. Missing rights are not an assertion of permission.","files":entries})).map_err(|e|e.to_string())?;
            let mut output=fs::OpenOptions::new().write(true).create_new(true).open(&file).map_err(|e|e.to_string())?;
            output.write_all(&bytes).and_then(|_|output.sync_all()).map_err(|e|e.to_string())?;
            Some((file,None,None))
        },
        Source::AdvancedFile{url_ref,connections,checksum,..}=>{
            let root={let guard=state.repository.lock().unwrap();guard.as_ref().ok_or("Open nabnab first.")?.root().to_owned()};let url=crate::sessions::resolve_url(&root,url_ref.as_deref().ok_or("The file link is unavailable. Add it again.")?)?;
            phase(app,state,&job.id,run_id,JobState::Downloading)?;
            Some((crate::advanced_files::execute(app,url.as_str(),staging,*connections,checksum.as_deref(),cancel.clone(),run_id,emit.clone()).await?,Some(public_origin(url.as_str())?),None))
        },
        Source::Palette{library_ids,colors,format}=>{
            let inputs={let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open nabnab first.")?;library_ids.iter().map(|id|repo.data.library.iter().find(|i|&i.id==id).cloned().ok_or("The image is no longer in the library.")).collect::<Result<Vec<_>,_>>()?};let _permit=processing_permit(state,cancel.clone()).await?;phase(app,state,&job.id,run_id,JobState::Processing)?;
            Some((crate::palettes::execute(&crate::dependencies::engine_root(app)?,&inputs,staging,*colors,format,cancel.clone(),run_id,emit.clone()).await?,None,None))
        },
        Source::AudioTagsCopy {selection} => {
            let engines = crate::dependencies::engine_root(app)?;
            let _permit = processing_permit(state, cancel.clone()).await?;
            phase(app, state, &job.id, run_id, JobState::Processing)?;
            let (file, info) = crate::audio_library::tag_copy::execute(state, &engines, selection, staging, cancel.clone()).await?;
            media_info = Some(info);
            Some((file, None, Some(selection.library_id.clone())))
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
    let mut item=library::index(&file,media_info.as_ref(),source,parent).or_else(|_|library::index_file(&file))?;
    if matches!(job.source,Source::Voice {..}|Source::DocumentEdit {..}|Source::Ocr {..}|Source::OcrText {..}|Source::AudioTagsCopy {..}|Source::Sticker {..} | Source::Upscale {..}) {
        if let Some(parent)=&item.parent_id {
            let guard=state.repository.lock().unwrap();
            if let Some(original)=guard.as_ref().and_then(|repo|repo.data.library.iter().find(|i|&i.id==parent)) {
                item.collections=original.collections.clone();item.tags=original.tags.clone();item.source=original.source.clone();
                item.details.content_id=original.details.content_id.clone();item.details.creator=original.details.creator.clone();item.details.source_url=original.details.source_url.clone();item.details.license=original.details.license.clone();item.details.attribution=original.details.attribution.clone();
                if matches!(job.source, Source::AudioTagsCopy { .. }) {
                    item.details.audiobook = original.details.audiobook.clone();
                    item.details.audio_cover = original.details.audio_cover.clone();
                }
            }
        }
    }
    if let Some((plan,output))=narration {
        item.duration=output.chapters.last().map(|chapter|chapter.end);
        item.audio_codec=Some("pcm_s16le".into());
        item.details.chapters=output.chapters;
        item.details.narration=Some(crate::library_features::NarrationReference {plan_ref:Some(plan.id),text_hash:plan.text_hash,source_version:plan.source_version,voice_name:plan.voice_name,language:plan.language,engine_fingerprint:plan.engine_fingerprint});
    }
    match &job.source {
        Source::Capture {capture_id}=>{
            let (root,_)=crate::file_operations::context(state)?;
            let journal=crate::screen_capture::readjournal(&root,capture_id)?;
            if journal.kind!="image"&&!item.duration.is_some_and(|duration|duration.is_finite()&&duration>0.0){return Err("The finalized capture has no verified duration. Its recovery files were preserved.".into());}
            let duration_ms=if journal.kind=="image"{0}else{(item.duration.unwrap_or_default()*1000.0).round() as u64};
            item.details.recording=Some(crate::library_features::RecordingReference{capture_ref:Some(capture_id.clone()),source_label:journal.source_label,kind:journal.kind,duration_ms,interrupted:journal.interrupted,reason:journal.reason});
        },
        Source::Ocr {..}=>{item.details.ocr_ref=Some(format!("ocr:{}-{}",job.id,job.run_id));},
        Source::OcrText {reference,..}=>{item.details.ocr_ref=Some(reference.clone());},
        Source::WebArchive {reference,collection,tags,parent_id,..}=>{
            let root={let guard=state.repository.lock().unwrap();guard.as_ref().ok_or("Open nabnab first.")?.root().to_owned()};
            let capture=crate::web_archive::load(&root,reference)?;
            item.kind="article".into();item.parent_id=parent_id.clone();item.details.web_archive_ref=Some(reference.clone());item.details.content_id=Some(format!("article:{}",capture.url));item.details.creator=capture.author;item.details.source_url=Some(capture.url);item.tags=tags.clone();
            if !collection.trim().is_empty(){item.collections.push(collection.trim().to_owned());}
        },
        Source::PublicFile {selection} => {
            item.source=Some(selection.source_url.clone());
            item.details.source_url=Some(selection.source_url.clone());
            item.details.content_id=Some(selection.asset_id.clone());
            item.details.creator=selection.author.clone();
            item.details.license=selection.license.clone();
            item.details.attribution=selection.attribution.clone();
            item.details.source_version = selection.version.clone();
            item.details.source_published_at = selection.published_at.clone();
            item.details.source_checksum = selection.checksum.clone();
            item.details.audiobook = selection.audiobook.clone();
            if let Some(chapter) = &selection.audiobook {
                item.kind = "audio".into();
                item.details.platform = Some("LibriVox".into());
                item.collections.push(format!("{} · LibriVox {}", chapter.book_title, chapter.book_id));
            }
            if !selection.collection.is_empty(){item.collections.push(selection.collection.clone());}
        },
        Source::CloudFile {selection,collection} => {
            item.details.content_id=Some(format!("cloud:{}:{}:{}",selection.provider.name(),selection.account_id,selection.id));
            if !collection.is_empty(){item.collections.push(collection.clone());}
        },
        Source::PersonalFile {selection} => {
            item.details.content_id = Some(selection.content_id());
            item.details.platform = Some("Personal server".into());
        },
        Source::Attribution {entries} => if let Some(first)=entries.first(){if !first.collection.is_empty(){item.collections.push(first.collection.clone());}},
        _=>{}
    }
    if let Source::CaptionEdit{library_id,track_id,..}=&job.source{let guard=state.repository.lock().unwrap();if let Some(track)=guard.as_ref().and_then(|repo|repo.data.library.iter().find(|i|&i.id==library_id)).and_then(|i|i.details.tracks.iter().find(|t|&t.id==track_id)){for generated in &mut item.details.tracks{generated.language=track.language.clone();}}}
    annotate_output(&mut item,job,staging);
    Ok(Some(publish_verified_output(state,job,staging,&file,item,cancel).await?))
}

fn write_report(path:&Path,value:&impl serde::Serialize)->Result<(),String>{
    let bytes=serde_json::to_vec_pretty(value).map_err(|e|e.to_string())?;
    let mut file=fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e|e.to_string())?;
    file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|e|e.to_string())
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
