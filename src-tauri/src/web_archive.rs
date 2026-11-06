use crate::{engine, library, model::Progress, AppState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::{Path, PathBuf}, sync::Arc};
use tauri::State;
use tokio::sync::watch;

const MAX_CAPTURE: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Capture {
    pub schema_version: u32,
    pub mode: Mode,
    pub title: String,
    pub author: Option<String>,
    pub url: String,
    pub published_at: Option<String>,
    pub archived_at: String,
    pub blocks: Vec<Block>,
    pub resources: Vec<Resource>,
    pub warnings: Vec<String>,
    pub selection_context: Option<SelectionContext>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase")]
pub enum Mode { Reading, Selection, Page }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct SelectionContext { pub prefix:String, pub suffix:String }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="kind",rename_all="camelCase",rename_all_fields="camelCase",deny_unknown_fields)]
pub enum Block {
    Heading {text:String,level:u8}, Paragraph {text:String}, Quote {text:String},
    Code {text:String}, ListItem {text:String}, Image {text:String,resource_id:String},
}
impl Block { pub fn text(&self)->&str {match self {Self::Heading{text,..}|Self::Paragraph{text}|Self::Quote{text}|Self::Code{text}|Self::ListItem{text}|Self::Image{text,..}=>text}} }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Resource { pub id:String, pub data_url:String, pub width:u32, pub height:u32 }
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all="lowercase")]
pub enum Format { Html, Epub, Md }
impl Format {pub fn extension(self)->&'static str {match self {Self::Html=>"html",Self::Epub=>"epub",Self::Md=>"md"}}}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Request {pub version:u32,pub request_id:String,pub capture:Capture,pub format:Format,pub collection:String,pub tags:Vec<String>}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Receipt {pub version:u32,pub request_id:String,pub accepted_job_ids:Vec<String>,pub kind:&'static str}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Annotation {pub id:String,pub version:String,pub block:usize,pub quote:String,pub prefix:String,pub suffix:String,pub note:String,pub created_at:u64}

fn clean_source(input:&str)->Result<String,String>{
    let mut url=url::Url::parse(&crate::model::validate_url(input)?).map_err(|_|"Invalid article source URL.")?;
    url.set_fragment(None);
    let pairs:Vec<_>=url.query_pairs().filter(|(k,_)|{let k=k.to_ascii_lowercase();!k.starts_with("utm_")&&!k.starts_with("x-amz-")&&!k.starts_with("x-goog-")&&! ["token","auth","secret","key","signature","credential","session","password"].iter().any(|s|k.contains(s))}).map(|(k,v)|(k.into_owned(),v.into_owned())).collect();
    url.set_query(None);if !pairs.is_empty(){url.query_pairs_mut().extend_pairs(pairs);}
    Ok(url.into())
}
fn decode_png(resource:&Resource)->Result<Vec<u8>,String>{
    let encoded=resource.data_url.strip_prefix("data:image/png;base64,").ok_or("Archive resources must be local PNG data, not network URLs.")?;
    if encoded.len()>350000 || encoded.len()%4!=0 || resource.width==0 || resource.height==0 || resource.width>2048 || resource.height>2048 {return Err("An archive image exceeds its size or dimension limit.".into());}
    let mut output=Vec::with_capacity(encoded.len()/4*3);
    let value=|c:u8|->Result<u32,String>{Ok(match c {b'A'..=b'Z'=>(c-b'A')as u32,b'a'..=b'z'=>(c-b'a'+26)as u32,b'0'..=b'9'=>(c-b'0'+52)as u32,b'+'=>62,b'/'=>63,_=>return Err("Invalid archive image encoding.".into())})};
    let mut chunks=encoded.as_bytes().chunks_exact(4).peekable();
    while let Some(c)=chunks.next(){
        if c[0]==b'='||c[1]==b'='||((c[2]==b'='||c[3]==b'=')&&chunks.peek().is_some())||c[2]==b'='&&c[3]!=b'=' {return Err("Invalid archive image padding.".into());}
        let n=(value(c[0])?<<18)|(value(c[1])?<<12)|(if c[2]==b'=' {0}else{value(c[2])?}<<6)|if c[3]==b'=' {0}else{value(c[3])?};
        output.push((n>>16)as u8);if c[2]!=b'=' {output.push((n>>8)as u8);}if c[3]!=b'=' {output.push(n as u8);}
    }
    if output.len()<24||!output.starts_with(b"\x89PNG\r\n\x1a\n")||&output[12..16]!=b"IHDR"||u32::from_be_bytes(output[16..20].try_into().unwrap())!=resource.width||u32::from_be_bytes(output[20..24].try_into().unwrap())!=resource.height {return Err("The archive resource does not match its PNG dimensions.".into());}
    Ok(output)
}
impl Capture {
    pub fn validate(mut self)->Result<Self,String>{
        if self.schema_version!=1||self.title.trim().is_empty()||self.title.len()>2000||self.author.as_ref().is_some_and(|a|a.len()>2000)||self.archived_at.len()>80||self.published_at.as_ref().is_some_and(|a|a.len()>80)||self.blocks.is_empty()||self.blocks.len()>4000||self.resources.len()>32||self.warnings.len()>64 {return Err("The archive capture exceeds its supported metadata limits.".into());}
        self.url=clean_source(&self.url)?;
        if !self.archived_at.contains('T')||!self.archived_at.ends_with('Z'){return Err("Use a separate ISO archive timestamp.".into());}
        let mut ids=std::collections::HashSet::new();let mut image_bytes=0;
        for resource in &self.resources {
            if !resource.id.strip_prefix("image-").is_some_and(|s|!s.is_empty()&&s.len()<=2&&s.bytes().all(|c|c.is_ascii_digit()))||!ids.insert(&resource.id){return Err("Archive image identifiers must be unique.".into());}
            image_bytes+=resource.data_url.len();decode_png(resource)?;
        }
        if image_bytes>2000000{return Err("Archive resources exceed the 2 MB capture limit.".into());}
        let mut text=0;for block in &self.blocks {
            text+=block.text().len();
            match block {Block::Heading{level,..}if !(1..=6).contains(level)=>return Err("Invalid article heading level.".into()),Block::Image{resource_id,..}if !ids.contains(resource_id)=>return Err("An article image has no local resource.".into()),_=>{}}
        }
        if text==0||text>1000000||self.warnings.iter().any(|w|w.len()>2000)||self.selection_context.as_ref().is_some_and(|s|s.prefix.len()>1000||s.suffix.len()>1000){return Err("The capture text exceeds its limit or is empty.".into());}
        if serde_json::to_vec(&self).map_err(|e|e.to_string())?.len()>MAX_CAPTURE {return Err("This capture exceeds its 4 MiB limit.".into());}
        Ok(self)
    }
}
fn valid_reference(reference:&str)->bool{reference.len()==64&&reference.bytes().all(|b|b.is_ascii_hexdigit())}
pub fn save(root:&Path,capture:Capture)->Result<String,String>{
    let capture=capture.validate()?;let bytes=serde_json::to_vec(&capture).map_err(|e|e.to_string())?;
    let reference=format!("{:x}",Sha256::digest(&bytes));let directory=crate::sessions::secure_module_directory(root,"web-archives")?;let path=directory.join(format!("{reference}.json"));
    if !path.exists(){let mut file=fs::OpenOptions::new().create_new(true).write(true).open(&path).map_err(|e|e.to_string())?;file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|e|e.to_string())?;}
    Ok(reference)
}
pub fn load(root:&Path,reference:&str)->Result<Capture,String>{
    if !valid_reference(reference){return Err("Invalid local article version.".into());}
    let directory=crate::sessions::secure_module_directory(root,"web-archives")?;
    let path=library::authorized_path(&directory.join(format!("{reference}.json")))?;
    if path.parent()!=Some(fs::canonicalize(directory).map_err(|e|e.to_string())?.as_path())||fs::metadata(&path).map_err(|e|e.to_string())?.len()>MAX_CAPTURE as u64{return Err("The local article version is outside its archive or too large.".into());}
    let bytes=fs::read(path).map_err(|e|e.to_string())?;
    if format!("{:x}",Sha256::digest(&bytes))!=reference{return Err("The saved article version changed. Existing annotations were not moved.".into());}
    serde_json::from_slice::<Capture>(&bytes).map_err(|_|"The local article version is unreadable.".to_owned())?.validate()
}
fn escape(value:&str)->String{value.replace('&',"&amp;").replace('<',"&lt;").replace('>',"&gt;").replace('"',"&quot;").replace('\'',"&#39;")}
fn markdown_text(value:&str)->String{
    let escaped=escape(value);let mut output=String::new();for character in escaped.chars(){if "\\`*_{}[]()#+-.!|".contains(character){output.push('\\');}output.push(character);}output
}
fn markdown(capture:&Capture)->String{
    let mut output=format!("# {}\n\nSource: {}\n\nArchived: {}\n\n",markdown_text(&capture.title),markdown_text(&capture.url),markdown_text(&capture.archived_at));
    if let Some(author)=&capture.author{output.push_str(&format!("Author: {}\n\n",markdown_text(author)));}
    if let Some(published)=&capture.published_at{output.push_str(&format!("Published: {}\n\n",markdown_text(published)));}
    for block in &capture.blocks {match block {
        Block::Heading{level,text}=>output.push_str(&format!("{} {}\n\n","#".repeat(*level as usize),markdown_text(text))),
        Block::Code{text}=>{for line in text.lines(){output.push_str("    ");output.push_str(line);output.push('\n');}output.push('\n');},
        Block::Quote{text}=>{for line in text.lines(){output.push_str("> ");output.push_str(&markdown_text(line));output.push('\n');}output.push('\n');},
        Block::ListItem{text}=>output.push_str(&format!("- {}\n\n",markdown_text(text))),
        Block::Image{resource_id,text}=>if let Some(resource)=capture.resources.iter().find(|r|&r.id==resource_id){output.push_str(&format!("![{}]({})\n\n",markdown_text(text),resource.data_url));},
        Block::Paragraph{text}=>output.push_str(&format!("{}\n\n",markdown_text(text))),
    }}
    if !capture.warnings.is_empty(){output.push_str("## Capture notes\n\n");for warning in &capture.warnings{output.push_str(&format!("- {}\n",markdown_text(warning)));}}
    output
}
fn body(capture:&Capture,epub:bool)->String{
    let mut output=String::new();for block in &capture.blocks {let text=escape(block.text());match block {
        Block::Heading{level,..}=>output.push_str(&format!("<h{level}>{text}</h{level}>")),
        Block::Paragraph{..}=>output.push_str(&format!("<p>{text}</p>")),Block::Quote{..}=>output.push_str(&format!("<blockquote>{text}</blockquote>")),Block::Code{..}=>output.push_str(&format!("<pre>{text}</pre>")),Block::ListItem{..}=>output.push_str(&format!("<ul><li>{text}</li></ul>")),
        Block::Image{resource_id,..}=>if let Some(resource)=capture.resources.iter().find(|r|&r.id==resource_id){let src=if epub{format!("resources/{}.png",resource.id)}else{resource.data_url.clone()};output.push_str(&format!("<figure><img src=\"{}\" alt=\"{text}\" width=\"{}\" height=\"{}\" /></figure>",escape(&src),resource.width,resource.height));}
    }}output
}
fn html(capture:&Capture,epub:bool)->String{
    format!("{}<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><meta charset=\"utf-8\" /><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src {}; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; script-src 'none'\" /><title>{}</title><style>body{{font:18px/1.65 system-ui;max-width:46rem;margin:3rem auto;padding:0 1.5rem;color:#202020;background:#fff}}img{{max-width:100%;height:auto}}pre{{white-space:pre-wrap}}header,footer{{font-size:14px;color:#555}}h1,h2,h3{{line-height:1.25}}</style></head><body><header><h1>{}</h1><p>{}</p><p>{}</p><p>Archived: {}{}</p></header><main>{}</main><footer>{}</footer></body></html>",if epub{"<?xml version=\"1.0\" encoding=\"UTF-8\"?>"}else{"<!doctype html>"},if epub{"'self' data:"}else{"data:"},escape(&capture.title),escape(&capture.title),escape(capture.author.as_deref().unwrap_or("")),format!("<a href=\"{}\" target=\"_blank\" rel=\"noreferrer noopener\">{}</a>",escape(&capture.url),escape(&capture.url)),escape(&capture.archived_at),capture.published_at.as_ref().map(|p|format!(" · Published: {}",escape(p))).unwrap_or_default(),body(capture,epub),capture.warnings.iter().map(|w|escape(w)).collect::<Vec<_>>().join("<br />"))
}
fn crc32(bytes:&[u8])->u32{let mut value=!0u32;for b in bytes{value^=*b as u32;for _ in 0..8{value=(value>>1)^(0xedb88320u32.wrapping_mul(value&1));}}!value}
fn stored_zip(path:&Path,entries:Vec<(String,Vec<u8>)>)->Result<(),String>{
    let mut output=Vec::new();let mut central=Vec::new();let count=entries.len();
    for(name,data)in entries {if name.len()>200||name.starts_with('/')||name.split('/').any(|p|matches!(p,""|"."|".."))||name.contains(['\\',':'])||data.len()>16*1024*1024{return Err("Invalid archive package entry.".into());}
        let offset=output.len()as u32;let crc=crc32(&data);let size=data.len()as u32;let name=name.as_bytes();
        output.extend(0x04034b50u32.to_le_bytes());for n in [20u16,0x800,0,0,0]{output.extend(n.to_le_bytes());}for n in [crc,size,size]{output.extend(n.to_le_bytes());}output.extend((name.len()as u16).to_le_bytes());output.extend(0u16.to_le_bytes());output.extend(name);output.extend(&data);
        central.extend(0x02014b50u32.to_le_bytes());for n in [20u16,20,0x800,0,0,0]{central.extend(n.to_le_bytes());}for n in [crc,size,size]{central.extend(n.to_le_bytes());}central.extend((name.len()as u16).to_le_bytes());for _ in 0..4{central.extend(0u16.to_le_bytes());}central.extend(0u32.to_le_bytes());central.extend(offset.to_le_bytes());central.extend(name);
    }
    let offset=output.len()as u32;let central_size=central.len()as u32;output.extend(central);output.extend(0x06054b50u32.to_le_bytes());for _ in 0..2{output.extend(0u16.to_le_bytes());}for _ in 0..2{output.extend((count as u16).to_le_bytes());}for n in [central_size,offset]{output.extend(n.to_le_bytes());}output.extend(0u16.to_le_bytes());
    let mut file=fs::OpenOptions::new().create_new(true).write(true).open(path).map_err(|e|e.to_string())?;file.write_all(&output).and_then(|_|file.sync_all()).map_err(|e|e.to_string())
}
pub async fn execute(root:&Path,reference:&str,format:Format,staging:&Path,cancel:watch::Receiver<bool>,run_id:u64,emit:impl Fn(Progress))->Result<PathBuf,String>{
    if *cancel.borrow(){return Err(engine::CANCELLED.into());}let capture=load(root,reference)?;
    let path=staging.join(format!("article-{}.{}",&reference[..12],format.extension()));
    match format {
        Format::Html=>{let mut file=fs::OpenOptions::new().create_new(true).write(true).open(&path).map_err(|e|e.to_string())?;file.write_all(html(&capture,false).as_bytes()).and_then(|_|file.sync_all()).map_err(|e|e.to_string())?;},
        Format::Md=>{let mut file=fs::OpenOptions::new().create_new(true).write(true).open(&path).map_err(|e|e.to_string())?;file.write_all(markdown(&capture).as_bytes()).and_then(|_|file.sync_all()).map_err(|e|e.to_string())?;},
        Format::Epub=>{
            let mut manifest=String::new();let mut entries=vec![("mimetype".into(),b"application/epub+zip".to_vec()),("META-INF/container.xml".into(),b"<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\" /></rootfiles></container>".to_vec()),("OEBPS/article.xhtml".into(),html(&capture,true).into_bytes())];
            for resource in &capture.resources {manifest.push_str(&format!("<item id=\"{}\" href=\"resources/{}.png\" media-type=\"image/png\" />",resource.id,resource.id));entries.push((format!("OEBPS/resources/{}.png",resource.id),decode_png(resource)?));}
            let opf=format!("<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"id\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:identifier id=\"id\">urn:sha256:{reference}</dc:identifier><dc:title>{}</dc:title><dc:language>und</dc:language><dc:source>{}</dc:source><meta property=\"dcterms:modified\">{}</meta></metadata><manifest><item id=\"article\" href=\"article.xhtml\" media-type=\"application/xhtml+xml\" /><item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\" />{manifest}</manifest><spine><itemref idref=\"article\" /></spine></package>",escape(&capture.title),escape(&capture.url),escape(&format!("{}Z",capture.archived_at.split('.').next().unwrap_or(&capture.archived_at).trim_end_matches('Z'))));
            entries.push(("OEBPS/content.opf".into(),opf.into_bytes()));entries.push(("OEBPS/nav.xhtml".into(),format!("<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>{}</title></head><body><nav epub:type=\"toc\"><ol><li><a href=\"article.xhtml\">{}</a></li></ol></nav></body></html>",escape(&capture.title),escape(&capture.title)).into_bytes()));stored_zip(&path,entries)?;
        }
    }
    if *cancel.borrow(){return Err(engine::CANCELLED.into());}emit(Progress{request_id:run_id,phase:"processing".into(),percent:Some(100.0),speed:None,downloaded:None,total:None});Ok(path)
}

pub async fn accept(state:&Arc<AppState>,client:&str,request:Request)->Result<Receipt,String>{
    if request.version!=1||client.len()!=32||request.collection.len()>120||request.tags.len()>50||request.tags.iter().any(|t|t.len()>120){return Err("Invalid archive request or organization limits.".into());}
    crate::queue::validate_request_id(&request.request_id)?;
    let client_request=format!("web-{:x}",Sha256::digest(format!("{client}:{}",request.request_id).as_bytes()));
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    let previous:Vec<_>=repo.data.jobs.iter().filter(|j|j.client_request==client_request).map(|j|j.id.clone()).collect();
    if !previous.is_empty(){return Ok(Receipt{version:1,request_id:request.request_id,accepted_job_ids:previous,kind:"archive"});}
    let title=request.capture.title.clone();let reference=save(repo.root(),request.capture)?;
    let jobs=crate::queue::enqueue_tasks(repo,vec![(crate::queue::Source::WebArchive{reference,format:request.format,collection:request.collection,tags:request.tags,parent_id:None},title)],&client_request)?;
    state.runtime.notify.notify_one();Ok(Receipt{version:1,request_id:request.request_id,accepted_job_ids:jobs.into_iter().map(|j|j.id).collect(),kind:"archive"})
}

#[tauri::command]
pub fn archive_document(state:State<Arc<AppState>>,id:String)->Result<Capture,String>{
    let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open nabnab first.")?;let item=repo.data.library.iter().find(|i|i.id==id).ok_or("Choose a local article.")?;
    load(repo.root(),item.details.web_archive_ref.as_deref().ok_or("This file has no archived article version.")?)
}

fn anchor_exists(text:&str,quote:&str,prefix:&str,suffix:&str)->bool{
    text.match_indices(quote).filter(|(start,_)|text[..*start].ends_with(prefix)&&text[*start+quote.len()..].starts_with(suffix)).take(2).count()==1
}
#[tauri::command]
pub fn archive_add_annotation(state:State<Arc<AppState>>,id:String,block:usize,quote:String,prefix:String,suffix:String,note:String)->Result<crate::Snapshot,String>{
    if quote.is_empty()||quote.len()>8000||prefix.len()>1000||suffix.len()>1000||note.len()>8000{return Err("Select text within one article paragraph and keep the note under 8000 bytes.".into());}
    let (reference,capture)={let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open nabnab first.")?;let item=repo.data.library.iter().find(|i|i.id==id).ok_or("Choose a local article.")?;let reference=item.details.web_archive_ref.clone().ok_or("This item has no local article version.")?;let capture=load(repo.root(),&reference)?;(reference,capture)};
    let selected=capture.blocks.get(block).filter(|b|!matches!(b,Block::Image{..})).ok_or("Choose a text block in this article version.")?;
    if !anchor_exists(selected.text(),&quote,&prefix,&suffix){return Err("The selected text or context does not match this local version. Select it again.".into());}
    let created_at=crate::queue::now();let annotation=Annotation{id:format!("{:x}",Sha256::digest(format!("{reference}:{block}:{quote}:{note}:{created_at}").as_bytes())),version:reference.clone(),block,quote,prefix,suffix,note,created_at};
    crate::library_features::edit_item(&state,&id,|item|{if item.details.web_archive_ref.as_deref()!=Some(reference.as_str()){return Err("This article version changed before the note was saved.".into());}if item.details.article_annotations.len()>=400{return Err("This version already has 400 annotations.".into());}if !item.details.article_annotations.iter().any(|a|a.id==annotation.id){item.details.article_annotations.push(annotation);}Ok(())})?;
    crate::snapshot(&state)
}
#[tauri::command]
pub fn archive_remove_annotation(state:State<Arc<AppState>>,id:String,annotation_id:String)->Result<crate::Snapshot,String>{
    crate::library_features::edit_item(&state,&id,|item|{item.details.article_annotations.retain(|a|a.id!=annotation_id);Ok(())})?;crate::snapshot(&state)
}
#[tauri::command]
pub fn queue_article_export(state:State<Arc<AppState>>,id:String,format:Format,client_request:String)->Result<Vec<crate::queue::Job>,String>{
    let mut guard=state.repository.lock().unwrap();let repo=guard.as_mut().ok_or("Open nabnab first.")?;
    let item=repo.data.library.iter().find(|i|i.id==id).cloned().ok_or("Choose a local article.")?;
    let reference=item.details.web_archive_ref.ok_or("This item has no local article version.")?;let capture=load(repo.root(),&reference)?;
    let source=crate::queue::Source::WebArchive {reference,format,collection:item.collections.first().cloned().unwrap_or_default(),tags:item.tags,parent_id:Some(id)};
    let jobs=crate::queue::enqueue_tasks(repo,vec![(source,format!("{} · {}",capture.title,format.extension()))],&client_request)?;state.runtime.notify.notify_one();Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture()->Capture {Capture{schema_version:1,mode:Mode::Reading,title:"Saved <article>".into(),author:Some("Author".into()),url:"https://example.org/article?id=42&access_token=secret&utm_source=tracking".into(),published_at:Some("2025-01-02".into()),archived_at:"2026-11-06T12:00:00.123Z".into(),blocks:vec![Block::Heading{text:"Original heading".into(),level:2},Block::Paragraph{text:"A selected phrase remains readable offline. <script>alert(1)</script>".into()}],resources:vec![],warnings:vec!["Static copy".into()],selection_context:None}}
    fn directory()->PathBuf{let path=std::env::temp_dir().join(format!("nabnab-article-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));fs::create_dir_all(&path).unwrap();path}
    #[test]
    fn capture_strips_access_parameters_and_exports_only_escaped_blocks(){
        let capture=capture().validate().unwrap();assert_eq!(capture.url,"https://example.org/article?id=42");
        let output=html(&capture,false);assert!(output.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));assert!(!output.contains("<script"));assert!(!output.contains("secret")&&!output.contains("tracking"));assert!(output.contains("script-src 'none'"));assert!(output.contains("2025-01-02")&&output.contains("2026-11-06T12:00:00.123Z"));
    }
    #[test]
    fn typed_capture_refuses_executable_html_and_remote_resource_fields(){
        let mut value=serde_json::to_value(capture()).unwrap();value["blocks"][0]["html"]="<script>steal()</script>".into();assert!(serde_json::from_value::<Capture>(value).is_err());
        let mut capture=capture();capture.resources.push(Resource{id:"image-0".into(),data_url:"https://example.org/tracker.png".into(),width:1,height:1});assert!(capture.validate().is_err());
    }
    #[test]
    fn markdown_keeps_literal_remote_image_examples_inert(){
        let mut capture=capture();capture.blocks=vec![Block::Paragraph{text:"![pixel](https://tracker.example/pixel)".into()},Block::Code{text:"![code](https://tracker.example/code)\n<script>active()</script>".into()}];
        let output=markdown(&capture);
        assert!(output.contains("\\!\\[pixel\\]\\(https://tracker\\.example/pixel\\)"));
        assert!(output.contains("\n    ![code](https://tracker.example/code)\n    <script>active()</script>\n"));
        assert!(output.contains("Author: Author")&&output.contains("Published: 2025\\-01\\-02"));
    }
    #[test]
    fn selected_quotes_need_a_unique_exact_context_and_version(){
        let text="first phrase here; second phrase there";
        assert!(!anchor_exists(text,"phrase","",""));assert!(anchor_exists(text,"phrase","second "," there"));assert!(!anchor_exists(text,"phrase","first "," there"));
        let root=directory();let old=save(&root,capture()).unwrap();let mut next=capture();next.archived_at="2026-11-06T12:01:00.123Z".into();let new=save(&root,next).unwrap();assert_ne!(old,new);assert_eq!(load(&root,&old).unwrap().archived_at,"2026-11-06T12:00:00.123Z");
        fs::write(root.join("web-archives").join(format!("{old}.json")),b"changed").unwrap();assert!(load(&root,&old).unwrap_err().contains("changed"));fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn archive_formats_are_new_files_and_cancellation_preserves_sources(){
        let root=directory();let source=save(&root,capture()).unwrap();let capture_path=root.join("web-archives").join(format!("{source}.json"));let original=fs::read(&capture_path).unwrap();let(_sender,cancel)=watch::channel(false);
        for format in [Format::Html,Format::Md,Format::Epub]{let staging=root.join(format.extension());fs::create_dir(&staging).unwrap();let path=execute(&root,&source,format,&staging,cancel.clone(),1,|_|{}).await.unwrap();let bytes=fs::read(path).unwrap();assert!(bytes.len()>100);if matches!(format,Format::Epub){assert!(bytes.starts_with(b"PK\x03\x04"));assert!(bytes.windows(b"application/epub+zip".len()).any(|w|w==b"application/epub+zip"));assert!(bytes.windows(b"OEBPS/nav.xhtml".len()).any(|w|w==b"OEBPS/nav.xhtml"));}assert!(execute(&root,&source,format,&staging,cancel.clone(),2,|_|{}).await.is_err());}
        let(_,cancel)=watch::channel(true);assert_eq!(execute(&root,&source,Format::Html,&root,cancel,3,|_|{}).await.unwrap_err(),engine::CANCELLED);assert_eq!(fs::read(capture_path).unwrap(),original);fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    #[ignore="Requires a verified MuPDF executable; opens the generated EPUB without network access"]
    async fn real_epub_contains_readable_archived_text(){
        let Some(mutool)=std::env::var_os("NABNAB_TEST_MUTOOL").map(PathBuf::from)else{return;};
        let root=directory();let reference=save(&root,capture()).unwrap();let(_,cancel)=watch::channel(false);let path=execute(&root,&reference,Format::Epub,&root,cancel,1,|_|{}).await.unwrap();
        let text=root.join("text.txt");let output=std::process::Command::new(mutool).args(["draw","-F","txt","-o"]).arg(&text).arg(path).output().unwrap();assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));let extracted=fs::read_to_string(text).unwrap();assert!(extracted.split_whitespace().collect::<Vec<_>>().join(" ").contains("A selected phrase remains readable offline."),"Extracted EPUB text: {extracted:?}");fs::remove_dir_all(root).unwrap();
    }
}
