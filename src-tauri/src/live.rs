use crate::{engine,model::Progress,AppState};
use serde_json::Value;
use std::{collections::HashMap,path::{Path,PathBuf},process::Stdio,sync::{Arc,Mutex,OnceLock},time::{Duration,Instant}};
use tauri::{AppHandle,State};
use tokio::{io::{AsyncBufReadExt,AsyncWriteExt,BufReader},sync::watch};
static RECORDERS:OnceLock<Mutex<HashMap<String,watch::Sender<bool>>>>=OnceLock::new();
struct Recorder(String);
impl Drop for Recorder{fn drop(&mut self){if let Some(recorders)=RECORDERS.get(){recorders.lock().unwrap().remove(&self.0);}}}
#[tauri::command]
pub fn stop_recording(state:State<Arc<AppState>>,id:String)->Result<(),String>{
    let guard=state.repository.lock().unwrap();let repo=guard.as_ref().ok_or("Open nabnab first.")?;
    if !repo.data.jobs.iter().any(|job|job.id==id&&job.state.active()){return Err("This recording is not active.".into());}
    RECORDERS.get().and_then(|r|r.lock().ok()?.get(&id).cloned()).ok_or("The recorder is not ready yet. Wait for recording to start.")?.send(true).map_err(|_|"The recording has already stopped.".into())
}
pub fn validate(url:&str,minutes:u32,max_mb:Option<u32>)->Result<(),String>{
    if !(1..=1440).contains(&minutes)||max_mb.is_some_and(|size|!(16..=131072).contains(&size)){return Err("Choose 1–1440 minutes and, optionally, a 16–131072 MB size limit.".into());}
    if !matches!(crate::providers::identify(url)?.id,crate::providers::ProviderId::Youtube|crate::providers::ProviderId::Twitch){return Err("Live recording currently supports YouTube and Twitch only.".into());}Ok(())
}
fn arguments(url:&str,output:&Path,minutes:u32,max_mb:Option<u32>)->Vec<String>{
    let mut args:Vec<String>=["-hide_banner","-loglevel","warning","-n","-rw_timeout","20000000","-protocol_whitelist","http,https,tcp,tls,crypto","-i"].into_iter().map(str::to_owned).collect();
    args.push(url.into());args.extend(["-t".into(),(minutes*60).to_string(),"-map".into(),"0:v?".into(),"-map".into(),"0:a?".into(),"-c".into(),"copy".into()]);
    if let Some(mb)=max_mb{args.extend(["-fs".into(),(u64::from(mb)*1024*1024).to_string()]);}
    args.extend(["-f".into(),"mpegts".into(),"-progress".into(),"pipe:1".into(),output.to_string_lossy().into_owned()]);args
}
pub async fn record(app:&AppHandle,state:&Arc<AppState>,job_id:&str,url:&str,staging:&Path,minutes:u32,max_mb:Option<u32>,mut cancel:watch::Receiver<bool>,run_id:u64,emit:impl Fn(Progress)+Clone+Send+Sync+'static)->Result<PathBuf,String>{
    validate(url,minutes,max_mb)?;
    let url=crate::providers::validate_remote(url).await?;
    let root=engine::engine_dir(app)?;
    let cookies=crate::sessions::materialize(&crate::config_root(state)?,&url)?;
    let mut args=engine::inspect_args(&root,&url);
    if let Some(i)=args.iter().position(|arg|arg=="--flat-playlist"){args.remove(i);}
    args.retain(|arg|arg!="--yes-playlist");
    args.splice(0..0,["--no-playlist".into(),"-f".into(),"best[protocol*=m3u8]/best".into()]);
    if let Some(cookie)=&cookies{args.splice(0..0,["--cookies".into(),cookie.path.to_string_lossy().into_owned()]);}
    let raw=engine::run_process(crate::updater::executable(app,&root)?,args,cancel.clone(),run_id,true,|_|{}).await?;
    let metadata:Value=serde_json::from_str(&raw).map_err(|_|"The live source returned invalid metadata.")?;
    if metadata["is_live"]!=true||metadata["has_drm"]==true{return Err("This link is not an active, unprotected live stream. Published videos use the normal download flow.".into());}
    let stream=metadata["url"].as_str().ok_or("No combined live stream is available for this source and session.")?;
    crate::providers::validate_remote(stream).await?;
    let output=staging.join(format!("recording-{}.ts",crate::queue::now()));
    let mut command=tokio::process::Command::new(root.join("ffmpeg.exe"));command.args(arguments(stream,&output,minutes,max_mb)).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]command.creation_flags(0x08000000);
    let mut child=command.spawn().map_err(|_|"Could not start the live recorder.")?;
    let mut stdin=child.stdin.take().ok_or("The recorder has no stop channel.")?;
    let stdout=child.stdout.take().ok_or("The recorder has no progress channel.")?;
    let stderr=child.stderr.take().ok_or("The recorder has no diagnostic channel.")?;
    let started=Instant::now();let progress=emit.clone();
    let updates=tokio::spawn(async move{let mut lines=BufReader::new(stdout).lines();while let Ok(Some(line))=lines.next_line().await{if let Some(bytes)=line.strip_prefix("total_size=").and_then(|s|s.parse::<f64>().ok()){progress(Progress{request_id:run_id,phase:format!("recording:{}",started.elapsed().as_secs()),percent:None,speed:None,downloaded:Some(bytes),total:None});}}});
    let errors=tokio::spawn(async move{let mut lines=BufReader::new(stderr).lines();let mut interrupted=false;while let Ok(Some(line))=lines.next_line().await{let lower=line.to_ascii_lowercase();if lower.contains("failed")||lower.contains("error")||lower.contains("missed"){interrupted=true;}}interrupted});
    let(sender,mut stop)=watch::channel(false);RECORDERS.get_or_init(Default::default).lock().unwrap().insert(job_id.into(),sender);let _recorder=Recorder(job_id.into());
    let status=tokio::select!{result=child.wait()=>result.map_err(|_|"The live recorder could not finish.")?,_=stop.changed()=>{stdin.write_all(b"q\n").await.map_err(|_|"The recorder could not receive the stop request.")?;tokio::time::timeout(Duration::from_secs(30),child.wait()).await.map_err(|_|"Finalization timed out. Captured segments remain recoverable in the job folder.")?.map_err(|_|"Recording finalization failed.")?},_=cancel.changed()=>{let _=stdin.write_all(b"q\n").await;let _=tokio::time::timeout(Duration::from_secs(15),child.wait()).await;updates.abort();errors.abort();return Err(engine::CANCELLED.into());},_=tokio::time::sleep(Duration::from_secs(u64::from(minutes)*60+180))=>{let _=stdin.write_all(b"q\n").await;return Err("Recording deadline reached. Captured segments remain recoverable in the job folder.".into());}};
    let _=updates.await;let interrupted=errors.await.unwrap_or(true);
    if !status.success()||interrupted{return Err("The live source disconnected or reported missing segments. The partial transport-stream recording was kept in the job folder; continuity is not guaranteed.".into());}
    if !output.is_file()||std::fs::metadata(&output).map_err(|e|e.to_string())?.len()==0{return Err("The live source produced no captured data.".into());}Ok(output)
}
#[cfg(test)]mod tests{use super::*;#[test]fn recorder_has_finite_limits_and_does_not_seek_to_an_unavailable_start(){let args=arguments("https://cdn.example.org/live.m3u8",Path::new("recording.ts"),60,Some(1024));assert!(args.windows(2).any(|p|p==["-t","3600"]));assert!(args.windows(2).any(|p|p==["-fs","1073741824"]));assert!(!args.iter().any(|p|p=="-ss"));assert!(validate("https://youtube.com/watch?v=1",60,None).is_ok());assert!(validate("https://youtube.com/watch?v=1",0,None).is_err());assert!(validate("https://example.org/live",60,None).is_err());}}
