// Shared rendered-UI data. No fixture here certifies OS capture hardware or installed voices.
import { deflateSync } from "node:zlib";
function previewPng() {
  function crc(bytes) { let value = 0xffffffff; for (const byte of bytes) { value ^= byte; for (let bit = 0; bit < 8; bit++) value = value >>> 1 ^ (value & 1 ? 0xedb88320 : 0); } return (value ^ 0xffffffff) >>> 0; }
  function chunk(type, data) { const name = Buffer.from(type), size = Buffer.alloc(4), checksum = Buffer.alloc(4); size.writeUInt32BE(data.length); checksum.writeUInt32BE(crc(Buffer.concat([name, data]))); return Buffer.concat([size, name, data, checksum]); }
  const width = 640, height = 360, header = Buffer.alloc(13); header.writeUInt32BE(width); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 6;
  const data = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) { const offset = y * (width * 4 + 1) + 1 + x * 4, shade = y < 30 ? 52 : x < 130 ? 36 : x > 190 && x < 570 && y > 85 && y < 250 ? 72 : 24; data[offset] = shade; data[offset + 1] = shade + 2; data[offset + 2] = shade + 5; data[offset + 3] = 255; }
  return "data:image/png;base64," + Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header), chunk("IDAT", deflateSync(data)), chunk("IEND", Buffer.alloc(0))]).toString("base64");
}
function wav(seconds) {
  const samples = Math.round(8000 * seconds), bytes = Buffer.alloc(44 + samples * 2);
  bytes.write("RIFF", 0); bytes.writeUInt32LE(bytes.length - 8, 4); bytes.write("WAVEfmt ", 8); bytes.writeUInt32LE(16, 16); bytes.writeUInt16LE(1, 20); bytes.writeUInt16LE(1, 22); bytes.writeUInt32LE(8000, 24); bytes.writeUInt32LE(16000, 28); bytes.writeUInt16LE(2, 32); bytes.writeUInt16LE(16, 34); bytes.write("data", 36); bytes.writeUInt32LE(samples * 2, 40);
  return bytes.toString("base64");
}
export const setup = `
  import {mockIPC,mockWindows} from "/node_modules/@tauri-apps/api/mocks.js";globalThis.isTauri=true;mockWindows("main");
  const copy=value=>structuredClone(value);
  const settings={folder:"C:/Fixture local output",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
  const details={contentId:null,creator:null,platform:null,sourceIds:[],chapters:[],tracks:[],playbackPosition:0,favorite:false,bookmarks:[],clip:null,webArchiveRef:"a".repeat(64),articleAnnotations:[{id:"note-1",version:"a".repeat(64),block:0,quote:"A passage",prefix:"",suffix:"",note:"A local note to listen to.",createdAt:1}]};
  const state={settings,jobs:[],library:[{id:"article",path:"C:/Fixture/article.html",name:"A saved article",kind:"article",size:1000,duration:null,width:null,height:null,videoCodec:null,audioCodec:null,container:null,addedAt:1,source:"https://fixture.invalid/article",parentId:null,collections:[],tags:[],missing:false,details}],collections:[],presets:[],rules:[],drafts:[],sources:[],smartCollections:[]};
  const idle=()=>({sessionId:null,phase:"idle",elapsedMs:0,bytes:0,replaySeconds:null,availableSeconds:0,sourceLabel:null,systemAudio:false,microphone:false,diskLimitMiB:null,reason:null,readyCapture:null});
  const fixture=window.localMediaFixture={state,calls:[],capture:idle(),captureAvailable:true,previewSlow:false,releasePreview:null,sourceClosed:false,voiceEnabled:true,plan:null,voiceActive:false,voiceCompleted:0,voiceSlow:false,releaseVoice:null};
  const png="${previewPng()}";
  const sampleWav=[...Uint8Array.from(atob("${wav(0.4)}"),char=>char.charCodeAt(0))];
  const segmentWav=[...Uint8Array.from(atob("${wav(6)}"),char=>char.charCodeAt(0))];
  function voiceStatus(){const plan=fixture.plan;return {planId:plan.id,textHash:plan.textHash,sourceVersion:plan.sourceVersion,engineFingerprint:plan.engineFingerprint,active:fixture.voiceActive,completed:fixture.voiceCompleted,total:plan.segments.length,duration:fixture.voiceCompleted*6,bytes:fixture.voiceCompleted*96000,segments:plan.segments.map(segment=>({index:segment.index,cached:segment.index<fixture.voiceCompleted,duration:segment.index<fixture.voiceCompleted?6:null}))};}
  mockIPC(async(cmd,args={})=>{
    fixture.calls.push({cmd,args:copy(args)});
    if(cmd==="plugin:window|is_maximized")return false;if(cmd==="startup")return {...copy(state),folder:settings.folder,enginesReady:true,engineError:null};if(cmd==="get_snapshot")return copy(state);if(cmd==="local_timezone")return "Europe/Madrid";
    if(cmd==="screen_capture_sources")return {available:fixture.captureAvailable,reason:fixture.captureAvailable?null:"Fixture: screen capture component is disabled.",sources:[{id:"monitor-1",label:"Fixture monitor",kind:"monitor",width:1920,height:1080},{id:"window-1",label:"Fixture window <img src=x>",kind:"window",width:1280,height:720}],audio:[{id:"system-1",label:"Fixture speakers",kind:"system"},{id:"mic-1",label:"Fixture microphone",kind:"microphone"}]};
    if(cmd==="screen_capture_status")return copy(fixture.capture);
    if(cmd==="screen_capture_recoverable")return [{id:"recovery-1",kind:"recording",durationMs:12000,interrupted:true,reason:"Fixture window closed; partial footage retained."}];
    if(cmd==="screen_capture_preview"){if(fixture.previewError)throw "Fixture: selected window is no longer available.";const preview={previewId:"preview-1",png,width:args.region?.width??1920,height:args.region?.height??1080,warnings:["Fixture preview only; this is not live OS capture."]};if(fixture.previewSlow)return new Promise(resolve=>{fixture.releasePreview=()=>resolve(preview);});return preview;}
    if(cmd==="cancel_screen_capture_preview"){fixture.releasePreview?.();fixture.releasePreview=null;fixture.previewSlow=false;return;}
    if(cmd==="screen_capture_start"){fixture.capture={...idle(),sessionId:"capture-1",phase:args.options.mode==="replay"?"replay":"recording",elapsedMs:2000,bytes:1024,replaySeconds:args.options.mode==="replay"?args.options.replaySeconds:null,availableSeconds:args.options.mode==="replay"?2:0,sourceLabel:"Fixture monitor",systemAudio:Boolean(args.options.systemAudioId),microphone:Boolean(args.options.microphoneId),diskLimitMiB:args.options.diskLimitMiB};return copy(fixture.capture);}
    if(cmd==="screen_capture_stop"){fixture.capture={...fixture.capture,phase:fixture.sourceClosed?"interrupted":"ready",reason:fixture.sourceClosed?"Fixture: the selected window closed during recording.":null,readyCapture:{id:"ready-1",kind:"recording",durationMs:fixture.capture.elapsedMs,interrupted:fixture.sourceClosed,reason:fixture.sourceClosed?"Window closed":null}};return copy(fixture.capture);}
    if(cmd==="screen_capture_save_replay")return {id:"replay-1",kind:"recording",durationMs:Math.floor(fixture.capture.availableSeconds)*1000,interrupted:false,reason:null};
    if(cmd==="screen_capture_screenshot")return {id:"image-1",kind:"image",durationMs:0,interrupted:false,reason:null};
    if(cmd==="queue_screen_capture_recovery")return [];
    if(cmd==="voice_status")return {available:true,enabled:fixture.voiceEnabled,engineVersion:"fixture",note:"Fixture installed offline voices; no real OS voice is represented.",voices:[{id:"es-1",name:"Fixture Spanish voice",language:"es-ES",enabled:true,fingerprint:"e".repeat(64)},{id:"en-1",name:"Fixture English voice",language:"en-US",enabled:true,fingerprint:"f".repeat(64)}]};
    if(cmd==="voice_prepare"){
      if(fixture.voicePrepareError)throw "Fixture: the installed voice is no longer available.";
      const text=args.source.kind==="text"?args.source.text:args.source.kind==="note"?"A local note to listen to.":"A saved article paragraph. Another local paragraph.";
      const segments=text.includes("LONG_FIXTURE")?Array.from({length:125},(_,index)=>({index,text:"A bounded visible segment "+index,startChars:index*30,endChars:index*30+29,cached:false})):[{index:0,text:text.slice(0,Math.ceil(text.length/2)),startChars:0,endChars:Math.ceil(text.length/2),cached:false},{index:1,text:text.slice(Math.ceil(text.length/2)),startChars:Math.ceil(text.length/2),endChars:text.length,cached:false}];
      const plan={schemaVersion:1,id:"plan-"+fixture.calls.filter(call=>call.cmd==="voice_prepare").length,createdAt:1,text,textHash:"a".repeat(64),sourceVersion:"b".repeat(64),origin:{title:args.source.kind==="text"?args.source.title:"A saved local source",libraryId:args.source.kind==="text"?null:args.source.id,kind:args.source.kind},settings:args.settings,engineFingerprint:"c".repeat(64),segments,warnings:["Fixture: actual narration is local and bound to this source version."]};fixture.plan=plan;fixture.voiceCompleted=0;fixture.voiceActive=false;if(fixture.voiceSlow)return new Promise(resolve=>{fixture.releaseVoice=()=>resolve(copy(plan));});return copy(plan);
    }
    if(cmd==="voice_plan_status")return voiceStatus();
    if(cmd==="voice_sample")return {wav:sampleWav,duration:.4,segmentText:fixture.plan.segments[0].text.slice(0,400),truncated:true};
    if(cmd==="queue_voice"){fixture.voiceActive=true;fixture.voiceCompleted=1;const job={id:"voice-job-1",runId:1,title:"Fixture narration",createdAt:1,updatedAt:1,state:"processing",source:{kind:"voice",planId:args.planId},folder:settings.folder,provider:"Local voice fixture",thumbnail:null,progress:null,outputs:[],error:null,attempt:0,groupId:null};state.jobs=state.jobs.filter(item=>item.id!==job.id);state.jobs.push(job);return [copy(job)];}
    if(cmd==="voice_segment_audio")return segmentWav;
    if(cmd==="voice_pause_job"){fixture.voiceActive=false;state.jobs.find(job=>job.id===args.id).state="paused";return copy(state);}
    if(cmd==="job_action"){for(const job of state.jobs.filter(job=>args.ids.includes(job.id))){job.state=args.action==="cancel"?"cancelled":"processing";fixture.voiceActive=args.action!=="cancel";}return copy(state);}
    if(cmd==="cancel_voice_request"){fixture.releaseVoice?.();fixture.releaseVoice=null;fixture.voiceSlow=false;return;}
    if(cmd==="optional_components"||cmd==="engine_status"||cmd==="sessions_status"||cmd==="list_courses")return [];
    if(cmd==="cache_status")return {bytes:0,files:0};if(cmd==="telegram_status")return {installed:false,accounts:[]};if(cmd==="torrent_status")return {installed:false,enabled:false};if(cmd==="cloud_status")return [];if(cmd==="bridge_status")return {enabled:false,clients:[],port:47883};
  },{shouldMockEvents:true});await import("/src/main.ts");
`;
export async function routeFixture(page, base) {
  await page.route(base, async route => { const response = await route.fetch(); const body = (await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/, `<script type="module">${setup.replace(/<\/script/gi, "<\\/script")}</script>`); await route.fulfill({ response, body }); });
}
