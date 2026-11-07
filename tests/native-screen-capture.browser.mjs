// Real desktop IPC/queue capture, scoped exclusively to this freshly launched synthetic app window.
import assert from "node:assert/strict";
import {spawn} from "node:child_process";
import {createHash} from "node:crypto";
import {inflateSync} from "node:zlib";
import {mkdir,mkdtemp,readFile,writeFile,readdir} from "node:fs/promises";
import path from "node:path";
import {createRequire} from "node:module";

const devPage=await fetch("http://localhost:1420/",{signal:AbortSignal.timeout(5000)}).catch(()=>null);
assert.ok(devPage?.ok,"Start Vite in the same Windows environment before opening the native test window (localhost:1420).");

const root=path.resolve(import.meta.dirname,"..");
const directory=path.join(root,"artifacts/native-screen-capture");await mkdir(directory,{recursive:true});
const data=await mkdtemp(path.join(directory,"native-run-"));
const output=path.join(data,"output-at-start"),laterOutput=path.join(data,"output-after-start");
await Promise.all([output,laterOutput].map(folder=>mkdir(folder)));
const {chromium}=createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE||"playwright");
const port=Number(process.env.NABNAB_CAPTURE_DEBUG_PORT||9241);
const checks=[],errors=[];let child,browser,page;
const digest=bytes=>createHash("sha256").update(bytes).digest("hex");
const key=value=>value.replace(/^\\\\\?\\/,"").replaceAll("\\","/").toLowerCase();
const pause=ms=>new Promise(resolve=>setTimeout(resolve,ms));
function pngDimensions(bytes){assert.equal(bytes.subarray(0,8).toString("hex"),"89504e470d0a1a0a");assert.equal(bytes.subarray(12,16).toString(),"IHDR");return [bytes.readUInt32BE(16),bytes.readUInt32BE(20)];}
function pixelContent(bytes){
  const [width,height]=pngDimensions(bytes),channels=({0:1,2:3,4:2,6:4})[bytes[25]];assert.equal(bytes[24],8);assert.ok(channels,"Native PNG must have an explicit 8-bit pixel format");const chunks=[];for(let offset=8;offset<bytes.length;){const size=bytes.readUInt32BE(offset),type=bytes.subarray(offset+4,offset+8).toString();if(type==="IDAT")chunks.push(bytes.subarray(offset+8,offset+8+size));offset+=12+size;}const raw=inflateSync(Buffer.concat(chunks)),stride=width*channels;assert.equal(raw.length,(stride+1)*height);let prior=Buffer.alloc(stride),different=0,total=0,first;const colors=new Set();
  const paeth=(a,b,c)=>{const p=a+b-c,pa=Math.abs(p-a),pb=Math.abs(p-b),pc=Math.abs(p-c);return pa<=pb&&pa<=pc?a:pb<=pc?b:c;};
  for(let y=0;y<height;y++){const mode=raw[y*(stride+1)],row=Buffer.alloc(stride);assert.ok(mode<=4);for(let x=0;x<stride;x++){const left=x>=channels?row[x-channels]:0,up=prior[x],corner=x>=channels?prior[x-channels]:0;row[x]=(raw[y*(stride+1)+1+x]+[0,left,up,Math.floor((left+up)/2),paeth(left,up,corner)][mode])&255;}for(let x=0;x<width;x+=3){const p=x*channels,r=row[p],g=channels>=3?row[p+1]:r,b=channels>=3?row[p+2]:r,color=(r<<16)|(g<<8)|b;if(first===undefined)first=color;if(color!==first)different++;if(colors.size<256)colors.add(color);total++;}prior=row;}
  return {sampledColors:colors.size,nonUniformRatio:different/total};
}
function usefulPixels(bytes){const content=pixelContent(bytes);assert.ok(content.sampledColors>24&&content.nonUniformRatio>0.01,`The owned accelerated window must contain actual UI pixels, not a flat/blank surface: ${JSON.stringify(content)}`);return content;}
async function wait(check,timeout=60000){const until=Date.now()+timeout;while(Date.now()<until){const result=await check();if(result)return result;await pause(150);}throw Error("Timed out waiting for native capture workflow.");}
async function run(executable,args){return new Promise((resolve,reject)=>{const process=spawn(executable,args,{cwd:root,windowsHide:true,stdio:["ignore","pipe","pipe"]});let stdout="",stderr="";process.stdout.on("data",bytes=>stdout+=bytes);process.stderr.on("data",bytes=>stderr+=bytes);process.on("error",reject);process.on("exit",code=>code===0?resolve(stdout.trim()):reject(Error(`${path.basename(executable)} exited ${code}: ${stderr.slice(-4000)}`)));});}
async function rpc(command,args={}){return page.evaluate(async({command,args})=>{const{invoke}=await import("/node_modules/@tauri-apps/api/core.js");return invoke(command,args);},{command,args});}
async function rejected(command,args,pattern){let reason;try{await rpc(command,args);}catch(error){reason=String(error);}assert.match(reason||"No rejection",pattern);}
async function snapshot(){return rpc("get_snapshot");}
async function settingsFolder(folder){const current=await snapshot();await rpc("save_settings",{settings:{...current.settings,folder,reducedMotion:true,trayEnabled:false,closeToTray:false}});}
async function completed(captureId){return wait(async()=>{const current=await snapshot();const job=current.jobs.find(job=>job.source.kind==="capture"&&job.source.captureId===captureId);if(job&&["error","cancelled"].includes(job.state))throw Error(JSON.stringify(job.error));if(job?.state!=="completed")return false;const item=current.library.find(item=>item.path===job.outputs[0]);assert.ok(item,"Completed capture was published in Library");return {job,item};},180000);}
async function journal(captureId){return JSON.parse(await readFile(path.join(data,"captures",captureId,"journal.json"),"utf8"));}
async function publishedCache(captureId){await wait(async()=>{const record=await journal(captureId);return record.phase==="published"?record:false;});const names=await readdir(path.join(data,"captures",captureId));assert.ok(!names.some(name=>/\.(mkv|wav|png)$/i.test(name)),"Private media is cleared after queue/library publication");}
async function preview(source){return rpc("screen_capture_preview",{sourceId:source.id,region:null,requestId:++sequence});}
async function started(source,options){const view=await preview(source);const state=await rpc("screen_capture_start",{previewId:view.previewId,options});assert.ok(state.sessionId);await wait(async()=>{const current=await rpc("screen_capture_status");if(current.phase==="error")throw Error(current.reason);return ["recording","replay"].includes(current.phase)?current:false;},20000);return state.sessionId;}
const options=(mode,durationSeconds=30)=>({mode,fps:15,quality:"standard",durationSeconds,diskLimitMiB:128,replaySeconds:60,systemAudioId:null,microphoneId:null});
let sequence=Date.now();

const helper=path.join(data,"owned-window-probe.ps1");
await writeFile(helper,String.raw`param([ValidateSet('title','resize','hotkey')][string]$Mode,[int]$CaptureProcessId,[string]$Title,[string]$Hotkey,[int]$Width,[int]$Height)
$ErrorActionPreference='Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class OwnedCaptureProbe {
 [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] public static extern bool SetWindowTextW(IntPtr hwnd,string text);
 [DllImport("user32.dll",SetLastError=true)] public static extern bool RegisterHotKey(IntPtr hwnd,int id,uint modifiers,uint key);
 [DllImport("user32.dll",SetLastError=true)] public static extern bool UnregisterHotKey(IntPtr hwnd,int id);
 [DllImport("user32.dll",SetLastError=true)] public static extern bool SetWindowPos(IntPtr hwnd,IntPtr after,int x,int y,int width,int height,uint flags);
}
'@
if($Mode -eq 'title' -or $Mode -eq 'resize') {
 $CaptureProcess=[System.Diagnostics.Process]::GetProcessById($CaptureProcessId)
 $CaptureProcess.Refresh()
 $OwnedWindow=$CaptureProcess.MainWindowHandle
 if($OwnedWindow -eq [IntPtr]::Zero){throw 'The explicitly launched app has no main window.'}
 if($Mode -eq 'title'){if(-not [OwnedCaptureProbe]::SetWindowTextW($OwnedWindow,$Title)){throw 'Could not title the explicitly launched synthetic app window.'}}
 else {if($Width -lt 300 -or $Width -gt 2500 -or $Height -lt 300 -or $Height -gt 1800){throw 'Unsafe synthetic window size.'};if(-not [OwnedCaptureProbe]::SetWindowPos($OwnedWindow,[IntPtr]::Zero,0,0,$Width,$Height,6)){throw 'Could not resize the explicitly launched app window.'}}
 @{title=$Title;processId=$CaptureProcessId}|ConvertTo-Json -Compress
} else {
 $VirtualKey=switch($Hotkey){'Ctrl+Shift+F8'{0x77};'Ctrl+Shift+F9'{0x78};'Ctrl+Shift+F10'{0x79};default{throw 'Unknown replay shortcut label.'}}
 $Available=[OwnedCaptureProbe]::RegisterHotKey([IntPtr]::Zero,0x6e62,0x4006,$VirtualKey)
 if($Available){[void][OwnedCaptureProbe]::UnregisterHotKey([IntPtr]::Zero,0x6e62)}
 @{available=$Available;label=$Hotkey}|ConvertTo-Json -Compress
}
`);
async function hotkeyAvailable(label){return JSON.parse(await run("powershell.exe",["-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File",helper,"-Mode","hotkey","-Hotkey",label])).available;}

try{
  child=spawn(path.join(root,"src-tauri/target/debug/nabnab.exe"),[],{cwd:root,windowsHide:true,stdio:"ignore",env:{...process.env,NABNAB_TEST_DATA_DIR:data,WEBVIEW2_USER_DATA_FOLDER:path.join(data,"webview"),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`}});
  await wait(async()=>{if(child.exitCode!==null)throw Error(`Native app exited ${child.exitCode}`);try{return(await fetch(`http://127.0.0.1:${port}/json/version`)).ok;}catch{return false;}});
  browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);page=await wait(()=>browser.contexts()[0]?.pages().find(page=>/:1420/.test(page.url())));page.on("pageerror",error=>errors.push(error.message));
  await page.locator('[data-page="library"]').waitFor();await wait(async()=>{try{return(await snapshot()).settings.folder;}catch{return false;}});await settingsFolder(output);
  const disabled=await rpc("screen_capture_sources");assert.equal(disabled.available,false);assert.deepEqual(disabled.sources,[]);await rpc("enable_component",{id:"capture",enabled:true});
  const title=`nabnab synthetic capture IPC ${child.pid}`;
  const owned=JSON.parse(await run("powershell.exe",["-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File",helper,"-Mode","title","-CaptureProcessId",String(child.pid),"-Title",title]));assert.equal(owned.processId,child.pid);
  const catalog=await rpc("screen_capture_sources");assert.equal(catalog.available,true);const source=catalog.sources.find(source=>source.kind==="window"&&source.label===title);assert.ok(source,"Only the explicitly launched app window can be selected by this test");
  await rejected("screen_capture_preview",{sourceId:"hwnd=0",region:null,requestId:++sequence},/available|select|refresh/i);await rejected("screen_capture_start",{previewId:"forged-preview",options:options("record")},/preview/i);
  const initialPreview=await preview(source);assert.deepEqual([initialPreview.width,initialPreview.height],[source.width,source.height]);const previewBytes=Buffer.from(initialPreview.png.split(",")[1],"base64");assert.ok(pngDimensions(previewBytes)[0]<=960);await writeFile(path.join(data,"actual-window-preview.png"),previewBytes);const previewContent=usefulPixels(previewBytes);
  const imageReady=await rpc("screen_capture_screenshot",{previewId:initialPreview.previewId});const image=await completed(imageReady.id);assert.deepEqual(pngDimensions(await readFile(image.item.path)),[source.width,source.height]);assert.equal(image.item.kind,"image");assert.equal(image.item.details.recording.kind,"image");assert.equal(image.item.details.recording.interrupted,false);const imageHash=digest(await readFile(image.item.path));await publishedCache(imageReady.id);
  const screenshotContent=usefulPixels(await readFile(image.item.path));checks.push({case:"Explicit component, opaque source/preview boundary, real accelerated-window PNG pixels queued and published",passed:true,dimensions:[source.width,source.height],pngSha256:imageHash,previewContent,screenshotContent});

  const ffprobe=path.join(root,".cache/ffmpeg-build/bundle/ffprobe.exe");
  const automatic=await started(source,options("record",4));
  const activeJournal=await journal(automatic);assert.equal(activeJournal.phase,"recording");await wait(async()=>{const names=await readdir(path.join(data,"captures",automatic));return names.some(name=>name.endsWith(".mkv"));});assert.equal((await journal(automatic)).phase,"recording");
  await settingsFolder(laterOutput);
  const automaticOutput=await completed(automatic);assert.equal(key(automaticOutput.job.folder),key(output));assert.equal(key(path.dirname(automaticOutput.item.path)),key(output));
  const actualProbe=JSON.parse(await run(ffprobe,["-v","error","-show_streams","-show_format","-of","json",automaticOutput.item.path]));const video=actualProbe.streams.find(stream=>stream.codec_type==="video");const actualDuration=Number(actualProbe.format.duration);assert.ok(actualDuration>0&&actualDuration<=6);assert.deepEqual([video.width,video.height],[Math.ceil(source.width/2)*2,Math.ceil(source.height/2)*2]);assert.deepEqual([automaticOutput.item.width,automaticOutput.item.height],[video.width,video.height]);assert.ok(Math.abs(automaticOutput.item.duration-actualDuration)<0.02);assert.ok(Math.abs(automaticOutput.item.details.recording.durationMs-actualDuration*1000)<2);assert.equal(automaticOutput.item.details.recording.interrupted,false);await publishedCache(automatic);
  const decodedFrame=path.join(data,"actual-recording-frame.png");await run(path.join(root,".cache/ffmpeg-build/bundle/ffmpeg.exe"),["-nostdin","-v","error","-n","-i",automaticOutput.item.path,"-frames:v","1",decodedFrame]);const videoContent=usefulPixels(await readFile(decodedFrame));checks.push({case:"Duration-limit capture binds destination before start; actual video UI pixels, ffprobe metadata and cache lifecycle",passed:true,duration:actualDuration,destinationBound:true,dimensions:[video.width,video.height],videoContent});
  await page.locator('[data-page="library"]').click();await page.locator(`.media-play[data-id="${automaticOutput.item.id}"]`).first().click();const player=page.locator(`video[data-local-player="${automaticOutput.item.id}"]`);await player.waitFor();const playback=await player.evaluate(video=>new Promise(resolve=>{video.pause();let timer;const finish=result=>{clearTimeout(timer);video.removeEventListener("loadedmetadata",loaded);video.removeEventListener("error",failed);resolve(result);};const loaded=()=>finish({state:"loadedmetadata",readyState:video.readyState,width:video.videoWidth,height:video.videoHeight,duration:video.duration,paused:video.paused,source:video.currentSrc});const failed=()=>finish({state:"error",code:video.error?.code,message:video.error?.message,readyState:video.readyState,source:video.currentSrc});if(video.readyState>=1){loaded();return;}if(video.error){failed();return;}video.addEventListener("loadedmetadata",loaded);video.addEventListener("error",failed);timer=setTimeout(()=>finish({state:"timeout",readyState:video.readyState,source:video.currentSrc}),15000);}));assert.match(playback.source,/^http:\/\/nabmedia\.localhost\//);if(playback.state==="loadedmetadata"){assert.ok(playback.readyState>=1);assert.equal(playback.paused,true);assert.deepEqual([playback.width,playback.height],[video.width,video.height]);assert.ok(Math.abs(playback.duration-actualDuration)<0.15);checks.push({case:"Published recording loads real metadata in the native nabmedia Library player without playback",passed:true,playback});}else{assert.equal(playback.state,"error");assert.ok([3,4].includes(playback.code),"Network/protocol failure must not be described as unsupported media");await page.locator('.stage-error:not([hidden])').waitFor();checks.push({case:"Native Library player reports this MKV as unsupported instead of claiming playback",passed:true,limitation:true,playback});}await page.screenshot({path:path.join(data,"recording-native-player.png"),animations:"disabled"});await page.locator("#stage-back").click();

  const manual=await started(source,options("record",30));await page.locator('[data-page="tools"]').click();await page.locator('[data-page="home"]').click();await page.locator('#capture-indicator:not([hidden])').waitFor();await page.locator('#capture-indicator [data-action="screen-capture-stop"]').waitFor();await pause(1400);await page.screenshot({path:path.join(data,"recording-indicator-home.png"),animations:"disabled"});await page.locator('#capture-indicator [data-action="screen-capture-stop"]').click();const manualOutput=await completed(manual);assert.equal(manualOutput.item.details.recording.interrupted,false);await publishedCache(manual);
  checks.push({case:"Persistent recording indicator and Stop remain usable from Home",passed:true});

  const replay=await started(source,options("replay",30));const replayStatus=await wait(async()=>{const state=await rpc("screen_capture_status");return state.phase==="replay"&&state.availableSeconds>=2?state:false;},15000);assert.match(replayStatus.shortcutLabel,/^Ctrl\+Shift\+F(?:8|9|10)$/);assert.equal(await hotkeyAvailable(replayStatus.shortcutLabel),false,"Registered replay shortcut is actually held by the app");assert.ok(replayStatus.availableSeconds<=8);await page.locator('#capture-indicator [data-action="screen-capture-save-replay"]').waitFor();await page.screenshot({path:path.join(data,"replay-indicator-home.png"),animations:"disabled"});const before=(await snapshot()).jobs.map(job=>job.id);const beforeSave=await rpc("screen_capture_status");await page.locator('#capture-indicator [data-action="screen-capture-save-replay"]').click();const replayJob=await wait(async()=>{const current=await snapshot();return current.jobs.find(job=>!before.includes(job.id)&&job.source.kind==="capture");});const saved=await completed(replayJob.source.captureId);assert.ok(saved.item.duration>0&&saved.item.duration<=beforeSave.elapsedMs/1000+2);assert.equal(saved.item.details.recording.interrupted,false);await page.locator('#capture-indicator [data-action="screen-capture-stop"]').click();await wait(async()=>!(await rpc("screen_capture_status")).shortcutLabel);assert.equal(await hotkeyAvailable(replayStatus.shortcutLabel),true);await publishedCache(replayJob.source.captureId);assert.ok((await readdir(path.join(data,"captures",replay))).every(name=>!name.endsWith(".mkv")&&!name.endsWith(".wav")));
  checks.push({case:"Opt-in replay snapshots only elapsed media, queues a stable result, and releases its OS shortcut when stopped",passed:true,shortcut:replayStatus.shortcutLabel,savedSeconds:saved.item.duration});

  const shortReplay=await started(source,options("replay",4));const shortStatus=await wait(async()=>{const state=await rpc("screen_capture_status");return state.sessionId===shortReplay&&state.shortcutLabel?state:false;});const shortKey=shortStatus.shortcutLabel;await wait(async()=>{const state=await rpc("screen_capture_status");return state.phase==="idle"&&!state.shortcutLabel;},15000);assert.equal(await hotkeyAvailable(shortKey),true,"Automatic replay stop releases the actual chosen hotkey");assert.ok((await readdir(path.join(data,"captures",shortReplay))).every(name=>!name.endsWith(".mkv")&&!name.endsWith(".wav")));assert.equal(digest(await readFile(image.item.path)),imageHash);
  checks.push({case:"Replay duration autostop releases the global hotkey; previously published PNG remains unchanged",passed:true,shortcut:shortKey});
  const interrupted=await started(source,options("record",30));await pause(1200);await run("powershell.exe",["-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File",helper,"-Mode","resize","-CaptureProcessId",String(child.pid),"-Width",String(source.width+62),"-Height",String(source.height+82)]);const partial=await wait(async()=>{const state=await rpc("screen_capture_status");return state.sessionId===interrupted&&state.phase==="interrupted"?state:false;},15000);assert.equal(partial.readyCapture.interrupted,true);assert.ok(!(await snapshot()).jobs.some(job=>job.source.kind==="capture"&&job.source.captureId===interrupted),"Interrupted sessions are not automatically reported as completed jobs");assert.equal((await journal(interrupted)).phase,"interrupted");await rpc("queue_screen_capture_recovery",{captureId:interrupted,clientRequest:"native-explicit-partial-recovery"});const recovered=await completed(interrupted);assert.equal(recovered.item.details.recording.interrupted,true);assert.match(recovered.item.path,/partial/i);await publishedCache(interrupted);checks.push({case:"Resizing only the owned app window yields a preserved interruption; explicit partial recovery keeps partial metadata",passed:true});
  assert.deepEqual(errors,[]);
  await page.locator('[data-page="library"]').click();await page.screenshot({path:path.join(data,"capture-results-library.png"),animations:"disabled"});
  await writeFile(path.join(data,"results.json"),JSON.stringify({scope:"Actual native IPC/FFmpeg/queue/library. Only this fresh app process's own HWND; microphone/system audio always off. Backend 66-second replay proof is tested separately.",checks,errors},null,2));console.log(JSON.stringify({data,checks:checks.length,passed:true},null,2));
}catch(error){await writeFile(path.join(data,"results.json"),JSON.stringify({passed:false,checks,errors,error:String(error)},null,2));throw error;}
finally{
  try{const state=await rpc("screen_capture_status");if(state.sessionId&&["starting","recording","replay","stopping"].includes(state.phase))await rpc("screen_capture_stop",{sessionId:state.sessionId});}catch{}
  try{await page?.evaluate(async()=>{const{getCurrentWindow}=await import("/node_modules/@tauri-apps/api/window.js");await getCurrentWindow().close();});}catch{}
  if(child?.exitCode===null){await Promise.race([new Promise(resolve=>child.once("exit",resolve)),pause(4000)]);if(child.exitCode===null)child.kill();}
  try{await browser?.close();}catch{}
}
