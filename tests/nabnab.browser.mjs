// UI contract checks use explicit IPC fixtures. Native media/security checks run separately.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { resolve } from "node:path";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }) });
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const results = [];
await mkdir("artifacts/nabnab", { recursive: true });
const landscape = '<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="900" viewBox="0 0 1600 900"><defs><linearGradient id="sky" x2="0" y2="1"><stop stop-color="#a7c4cc"/><stop offset="1" stop-color="#4b6968"/></linearGradient></defs><path fill="url(#sky)" d="M0 0h1600v900H0z"/><path fill="#56765f" d="M0 500 300 300 600 650 1000 250 1600 600v300H0z"/><path fill="#2d493d" d="m0 700 500-180 380 180 720-200v400H0z"/><text x="70" y="100" font-family="sans-serif" font-size="40" fill="white">Local thumbnail fixture</text></svg>';
const portrait = '<svg xmlns="http://www.w3.org/2000/svg" width="900" height="1600" viewBox="0 0 900 1600"><path fill="#8ea8ac" d="M0 0h900v1600H0z"/><path fill="#526e60" d="M0 850 250 390 650 1100 900 600v1000H0z"/><path fill="#2d493d" d="m0 1300 500-400 400 300v400H0z"/><text x="50" y="100" font-family="sans-serif" font-size="36" fill="white">Portrait UI fixture</text></svg>';
const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri = true; mockWindows("main");
  const settings = {folder:"C:/Downloads",language:"es",networkLimit:3,processingLimit:1,reducedMotion:false,reducedTransparency:false,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
  const state = {settings,jobs:[],library:[],collections:[]};
  window.fixture = {calls:[],metadata:[],state};
  const copy = value => structuredClone(value);
  const job = (title,source) => ({id:crypto.randomUUID(),runId:1,title,createdAt:Date.now(),updatedAt:Date.now(),state:"pending",source,folder:state.settings.folder,provider:source.kind==="download"?"yt-dlp":"FFmpeg",thumbnail:null,progress:null,outputs:[],error:null,attempt:0,groupId:null});
  mockIPC(async (cmd,args={}) => {
    window.fixture.calls.push({cmd,args:copy(args)});
    if(cmd==="plugin:window|is_maximized") return false;
    if(cmd==="startup") return {...copy(state),folder:state.settings.folder,enginesReady:true,engineError:null};
    if(cmd==="get_snapshot") return copy(state);
    if(cmd==="inspect_media") return new Promise(resolve=>window.fixture.metadata.push({id:args.requestId,url:args.url,resolve}));
    if(cmd==="queue_download") { const jobs=args.urls.map((url,index)=>job(args.titles[index]||"Fixture download",{kind:"download",url,format:args.format,height:args.height,audioBitrate:args.audioBitrate,subtitleLanguage:args.subtitleLanguage}));state.jobs.push(...jobs);return copy(jobs); }
    if(cmd==="queue_tool") {const jobs=args.ids.map(id=>job("Fixture local operation",{kind:"transform",libraryId:id,options:args.options}));state.jobs.push(...jobs);return copy(jobs);}
    if(cmd==="job_action"){for(const item of state.jobs)if(args.ids.includes(item.id)){item.state=args.action==="cancel"?"cancelled":args.action==="pause"?"paused":"pending";item.updatedAt=Date.now();}return copy(state);}
    if(cmd==="import_media") {if(!state.library.length)state.library=Array.from({length:1000},(_,index)=>({id:"local-"+index,path:"C:/Media/Clip "+index+".mp4",name:"Clip "+String(index).padStart(4,"0")+".mp4",kind:index%3===1?"audio":"video",size:1048576+index,duration:30,width:1280,height:720,videoCodec:index%3===1?null:"h264",audioCodec:"aac",container:"mp4",addedAt:Date.now()-index,source:null,parentId:null,collections:[],tags:[],missing:index===0}));return copy(state);}
    if(cmd==="remove_library"){state.library=state.library.filter(item=>item.id!==args.id);return copy(state);}
    if(cmd==="update_library"){const item=state.library.find(item=>item.id===args.id);item.collections=args.collections;item.tags=args.tags;state.collections=[...new Set(state.library.flatMap(item=>item.collections))];return copy(state);}
    if(cmd==="save_settings"){state.settings=args.settings;return copy(state);}
    if(cmd==="engine_status")return ["yt-dlp","FFmpeg","ffprobe","Deno"].map(name=>({name,version:"fixture",path:"C:/Engines/"+name,size:1024,status:"installed",source:"UI fixture"}));
    if(cmd==="sessions_status")return [];
    if(cmd==="bridge_status")return {enabled:false,clients:[],port:47883};
    if(cmd==="cache_status"||cmd==="clear_cache")return {bytes:0,files:0};
    if(cmd==="thumbnail")return "data:image/svg+xml,"+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90"><rect width="160" height="90" fill="#22252b"/></svg>');
    if(cmd==="export_diagnostics")return "C:/Diagnostics/nabnab.json";
    if(cmd==="import_url_file")return ["https://example.org/a","https://example.org/b"];
  },{shouldMockEvents:true});
  await import("/src/main.ts");
`;
async function mockPage(page) {
  await page.addInitScript(()=>{
    const raf=requestAnimationFrame.bind(window);window.visualFrames=0;window.requestAnimationFrame=callback=>raf(time=>{window.visualFrames++;callback(time);});
    window.visualPaints={field:[],emblem:[],longTasks:[]};
    const record=(canvas,start)=>{const key=canvas.id==='field'?'field':canvas.id==='emblem'?'emblem':null;if(key&&window.visualPaints[key].length<5000)window.visualPaints[key].push({time:performance.now(),cost:performance.now()-start});};
    const upload=CanvasRenderingContext2D.prototype.putImageData;
    CanvasRenderingContext2D.prototype.putImageData=function(...args){const start=performance.now();const result=upload.apply(this,args);record(this.canvas,start);return result;};
    if(typeof WebGL2RenderingContext!=='undefined'){const draw=WebGL2RenderingContext.prototype.drawArrays;WebGL2RenderingContext.prototype.drawArrays=function(...args){const start=performance.now();const result=draw.apply(this,args);record(this.canvas,start);return result;};}
    new PerformanceObserver(list=>{window.visualPaints.longTasks.push(...list.getEntries().map(entry=>({time:entry.startTime,duration:entry.duration})));}).observe({type:'longtask',buffered:true});
  });
  await page.route(base, async route => {
    const response=await route.fetch();
    const html=await response.text();
    assert.match(html,/<script type="module" src="\/src\/main\.ts[^"]*"><\/script>/);
    await route.fulfill({response,body:html.replace(/<script type="module" src="\/src\/main\.ts[^"]*"><\/script>/,'<script type="module">'+setup+'</script>')});
  });
  await page.route("http://nabmedia.localhost/**",route=>route.fulfill({status:404,body:"UI fixture has no real media"}));
  await page.route("https://example.org/thumbnail-fixture",route=>route.fulfill({contentType:'image/svg+xml',body:landscape}));
  await page.route("https://example.org/portrait-fixture",route=>route.fulfill({contentType:'image/svg+xml',body:portrait}));
}
async function assertNoOverflow(page) {
  const result=await page.evaluate(()=>({document:document.documentElement.scrollWidth,width:innerWidth,main:document.querySelector('#main').scrollWidth,mainWidth:document.querySelector('#main').clientWidth}));
  assert(result.document<=result.width+1,JSON.stringify(result));
  assert(result.main<=result.mainWidth+1,JSON.stringify(result));
}
async function measureGlass(page) {
  const cdp=await page.context().newCDPSession(page);await cdp.send('Performance.enable');
  await page.addStyleTag({content:'.measure-no-blur .is-home .sidebar,.measure-no-blur .url-control,.measure-no-blur .destination-row,.measure-no-blur .is-home select,.measure-no-blur .notification{backdrop-filter:none!important}'});
  const samples=[];
  for(const blur of [true,false,false,true,true,false,false,true,true,false]){
    await page.evaluate(blur=>document.documentElement.classList.toggle('measure-no-blur',!blur),blur);await page.waitForTimeout(250);
    const before=(await cdp.send('Performance.getMetrics')).metrics;
    const start=await page.evaluate(()=>performance.now());await page.waitForTimeout(2000);const end=await page.evaluate(()=>performance.now());
    const paints=await page.evaluate(({start,end})=>({field:window.visualPaints.field.filter(frame=>frame.time>=start&&frame.time<=end),emblem:window.visualPaints.emblem.filter(frame=>frame.time>=start&&frame.time<=end),longTasks:window.visualPaints.longTasks.filter(task=>task.time>=start&&task.time<=end)}),{start,end});
    const after=(await cdp.send('Performance.getMetrics')).metrics;
    const metric=(values,name)=>values.find(value=>value.name===name)?.value??0;
    const stats=frames=>{const intervals=frames.slice(1).map((frame,index)=>frame.time-frames[index].time).sort((a,b)=>a-b);return {draws:frames.length,hz:frames.length/((end-start)/1000),medianIntervalMs:intervals[Math.floor(intervals.length/2)]??null,maxIntervalMs:intervals.at(-1)??null};};
    assert(paints.field.length>=10&&paints.emblem.length>=10,'The material sample executes actual field and emblem draws');
    samples.push({blur,windowMs:end-start,field:stats(paints.field),emblem:stats(paints.emblem),mainThreadTaskMs:(metric(after,'TaskDuration')-metric(before,'TaskDuration'))*1000,longTasks:paints.longTasks});
  }
  await page.evaluate(()=>document.documentElement.classList.remove('measure-no-blur'));
  const renderer=await page.locator('#field').evaluate(canvas=>{const gl=canvas.getContext('webgl2');if(!gl)return 'Canvas2D fallback';const extension=gl.getExtension('WEBGL_debug_renderer_info');return gl.getParameter(extension?extension.UNMASKED_RENDERER_WEBGL:gl.RENDERER);});
  const report={claim:'Observed draw cadence and main-thread activity for the existing animated Home, alternating local glass blur and blur disabled while retaining identical tint. This does not measure GPU compositing cost or certify presentation frame rate.',browser:await browser.version(),renderer,viewport:{width:1280,height:800,scale:1},samples};
  await writeFile('artifacts/nabnab/glass-cadence.json',JSON.stringify(report,null,2));await cdp.detach();
}
async function captureWalkthrough() {
  const page=await browser.newPage({viewport:{width:1280,height:800}});
  const cdp=await page.context().newCDPSession(page);
  const frameDir=`artifacts/nabnab/video-${Date.now()}`;await mkdir(frameDir,{recursive:true});
  const frames=[],writes=[];
  cdp.on('Page.screencastFrame',frame=>{
    const file=`frame-${String(frames.length).padStart(5,'0')}.jpg`;frames.push({file,time:frame.metadata.timestamp});
    writes.push(writeFile(`${frameDir}/${file}`,Buffer.from(frame.data,'base64')).finally(()=>cdp.send('Page.screencastFrameAck',{sessionId:frame.sessionId}).catch(()=>{})));
  });
  await cdp.send('Page.startScreencast',{format:'jpeg',quality:88,maxWidth:1280,maxHeight:800,everyNthFrame:1});
  await mockPage(page);await page.goto(base);await page.locator('#url').waitFor();await page.waitForTimeout(700);
  await page.locator('#url').fill('https://example.org/open-movie');await page.locator('#load').click();await page.waitForFunction(()=>window.fixture.metadata.length===1);await page.waitForTimeout(450);
  await page.evaluate(()=>{const request=window.fixture.metadata[0];request.resolve({requestId:request.id,title:'Big Buck Bunny · Blender Open Movie',duration:65,thumbnail:'https://example.org/thumbnail-fixture',uploader:'Blender Studio',heights:[1080,720],hasAudio:true,hasVideo:true,entries:[],subtitleLanguages:['en','es'],capabilities:{video:true,audio:true,images:false,batches:false,subtitles:true,verified:false}});});
  await page.locator('#download').waitFor();await page.waitForTimeout(650);await page.locator('#toggle-sidebar').click();await page.waitForTimeout(450);await page.locator('#toggle-sidebar').click();await page.waitForTimeout(450);
  await page.screenshot({path:'artifacts/nabnab/glass-preview-1280-1x.png'});
  await page.locator('#download').click();await page.locator('[data-page=downloads]').click();
  assert(await page.locator('#page-content').evaluate(element=>element.getAnimations().length>0),'Pointer navigation has a bounded surface transition');
  await page.keyboard.press('Control+1');assert.equal(await page.locator('#home-page').isVisible(),true);
  assert.equal(await page.locator('#home-content').evaluate(element=>element.getAnimations().length),0,'Keyboard navigation interrupts motion and arrives immediately');
  await page.locator('[data-page=downloads]').click();await page.waitForTimeout(500);
  await page.locator('[data-page=library]').click();await page.locator('[data-action=import-files]').first().click();await page.locator('.media-open').nth(2).click();await page.waitForTimeout(700);
  await page.locator('#inspector [data-action=tool-media]').click();await page.waitForTimeout(550);await page.locator('[data-page=settings]').click();await page.waitForTimeout(550);
  await page.locator('[data-page=home]').click();await page.waitForTimeout(900);await page.screenshot({path:'artifacts/nabnab/glass-home-1280-1x.png'});
  await cdp.send('Page.stopScreencast');await Promise.all(writes);await cdp.detach();assert(frames.length>20,'The video contains actual browser screencast frames');
  const concat=frames.map((frame,index)=>`file '${frame.file}'\nduration ${Math.max(.01,(frames[index+1]?.time??frame.time+.1)-frame.time)}`).join('\n');
  await writeFile(`${frameDir}/frames.ffconcat`,`ffconcat version 1.0\n${concat}\nfile '${frames.at(-1).file}'\n`);
  await promisify(execFile)(resolve('src-tauri/engines/ffmpeg.exe'),['-hide_banner','-loglevel','error','-nostdin','-y','-safe','0','-f','concat','-i',resolve(`${frameDir}/frames.ffconcat`),'-t','30','-vf','fps=25,format=yuv420p','-c:v','libopenh264','-b:v','4M','-threads','1','-movflags','+faststart',resolve('artifacts/nabnab/glass-walkthrough.mp4')]);
  await writeFile('artifacts/nabnab/glass-video.json',JSON.stringify({source:'Actual Edge CDP screencast. IPC and thumbnail fixtures are explicit; native transport is tested separately.',frames:frames.length,capturedSeconds:frames.at(-1).time-frames[0].time,path:'artifacts/nabnab/glass-walkthrough.mp4'},null,2));
  await page.emulateMedia({forcedColors:'active'});assert.equal(await page.locator('#field').evaluate(element=>getComputedStyle(element).display),'none');assert.equal(await page.locator('.url-control').evaluate(element=>getComputedStyle(element).backdropFilter),'none');
  await page.close();
}
try {
  if(process.env.NABNAB_UI_VIDEO_ONLY==='1'){await captureWalkthrough();console.log('Glass walkthrough and pointer/keyboard/forced-colors checks passed.');}
  else {
  const plain=await browser.newPage({viewport:{width:1180,height:780}});
  await plain.goto(base);await plain.locator('#url').waitFor();
  assert.match(await plain.locator('body').innerText(),/aplicación de escritorio nabnab/);
  assert.equal(await plain.locator('#load').isDisabled(),true);
  await plain.screenshot({path:'artifacts/nabnab/browser-mode.png'});await plain.close();
  for(const [width,height,scale] of [[1280,800,1],[1180,780,1],[900,620,1],[1180,780,1.25],[1180,780,1.5]]){
    const page=await browser.newPage({viewport:{width,height},deviceScaleFactor:scale});
    await page.emulateMedia({reducedMotion:scale>1?'reduce':'no-preference'});
    const errors=[];page.on('pageerror',error=>errors.push(error.message));await mockPage(page);await page.goto(base);
    await page.waitForFunction(()=>window.fixture?.calls.some(call=>call.cmd==='startup'));
    await page.locator('#url').waitFor();await assertNoOverflow(page);
    if(width===1280&&process.env.NABNAB_UI_BENCHMARK==='1')await measureGlass(page);
    const field=await page.locator('#field').boundingBox();assert.deepEqual(field,{x:0,y:0,width,height});
    assert.equal(await page.locator('#field').evaluate(element=>getComputedStyle(element).position),'fixed');
    assert.equal(await page.locator('#field').evaluate(element=>getComputedStyle(element).opacity),'1');
    const inputHeight=(await page.locator('.url-control').boundingBox()).height;assert(inputHeight>=52&&inputHeight<=56);
    await page.screenshot({path:`artifacts/nabnab/home-${width}-${scale}x.png`});
    await page.locator('#toggle-sidebar').click();assert.equal((await page.locator('.sidebar').boundingBox()).width,64);
    await assertNoOverflow(page);assert.deepEqual(await page.locator('#field').boundingBox(),field);
    await page.screenshot({path:`artifacts/nabnab/home-collapsed-${width}-${scale}x.png`});
    await page.locator('#toggle-sidebar').click();assert.equal((await page.locator('.sidebar').boundingBox()).width,200);
    const source=page.locator('#url');
    await page.locator('#source-status').evaluate(element=>window.fixture.statusNode=element);
    const idleBox=await page.locator('.url-control').boundingBox();
    await source.fill('https://example.org/old');await source.press('Enter');
    await page.waitForFunction(()=>window.fixture.metadata.length===1);
    const box=await page.locator('.url-control').boundingBox();
    const sourceScroll=await page.locator('#main').evaluate(element=>element.scrollTop);
    assert(Math.abs(box.y-idleBox.y)<1,'Analysis keeps the idle input position');
    assert.equal(await page.locator('#source-form').getAttribute('aria-busy'),'true');
    assert.equal(await page.locator('.url-control').evaluate(element=>getComputedStyle(element,'::after').height),'1px');
    await page.screenshot({path:`artifacts/nabnab/analysis-${width}-${scale}x.png`});
    await source.fill('https://example.org/new');await source.press('Enter');
    await page.waitForFunction(()=>window.fixture.metadata.length===2);
    await page.evaluate(()=>{
      const media=(id,title)=>({requestId:id,title,duration:65,thumbnail:'https://example.org/thumbnail-fixture',uploader:'Blender Studio',heights:[720,360],hasAudio:true,hasVideo:true,entries:[],subtitleLanguages:['en','es'],capabilities:{video:true,audio:true,images:false,batches:false,subtitles:true,verified:false}});
      window.fixture.mediaTemplate=media(0,'Vídeo <b>literal</b>');
      window.fixture.metadata[1].resolve(media(window.fixture.metadata[1].id,'Vídeo <b>literal</b>'));
      window.fixture.metadata[0].resolve(media(window.fixture.metadata[0].id,'Stale result'));
    });
    await page.locator('#video-title').waitFor();
    assert.match(await page.locator('#source-status').innerText(),/Contenido listo/);
    assert.equal(await page.evaluate(()=>document.querySelector('#source-status')===window.fixture.statusNode),true,'Analysis updates an existing polite live region');
    assert.equal(await page.evaluate(()=>document.activeElement?.id),'url','Analysis announces completion without moving input focus');
    assert.equal(await page.locator('#video-title').innerText(),'Vídeo <b>literal</b>');
    assert.equal(await page.locator('#video-title b').count(),0);
    await page.evaluate(()=>{window.fixture.mediaTemplate.title='Big Buck Bunny · Blender Open Movie';});
    assert.equal(await page.evaluate(()=>window.fixture.calls.filter(call=>call.cmd==='queue_download').length),0);
    assert.equal(await page.locator('#subtitles').inputValue(),'');
    await page.locator('#subtitles').selectOption('es');
    await page.locator('#load').click();await page.waitForFunction(()=>window.fixture.metadata.length===3);
    await page.evaluate(()=>{const pending=window.fixture.metadata[2];pending.resolve({...window.fixture.mediaTemplate,requestId:pending.id});});
    await page.locator('#subtitles').waitFor();
    assert.equal(await page.locator('#subtitles').inputValue(),'','Forced analysis of the same URL clears old subtitle selection');
    await page.locator('#subtitles').selectOption('es');
    await source.fill('https://example.org/changed');await source.press('Enter');await page.waitForFunction(()=>window.fixture.metadata.length===4);
    await page.evaluate(()=>{const pending=window.fixture.metadata[3];pending.resolve({...window.fixture.mediaTemplate,requestId:pending.id});});
    await page.locator('#subtitles').waitFor();assert.equal(await page.locator('#subtitles').inputValue(),'','A different link clears old subtitle selection');
    await page.locator('#subtitles').selectOption('es');await page.locator('#subtitles').focus();
    await page.locator('#subtitles').evaluate(element=>window.fixture.focusedSubtitle=element);
    await page.evaluate(async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('library-update');});
    await page.waitForFunction(()=>document.querySelector('#subtitles')!==window.fixture.focusedSubtitle);
    assert.equal(await page.evaluate(()=>document.activeElement?.id),'subtitles');assert.equal(await page.locator('#subtitles').inputValue(),'es');
    const after=await page.locator('.url-control').boundingBox();const previewScroll=await page.locator('#main').evaluate(element=>element.scrollTop);assert(after.y+previewScroll<box.y+sourceScroll,'Loaded composition moves to its compact position');
    await page.waitForFunction(()=>document.querySelector('.preview-image')?.naturalWidth===1600);
    const mediaFrame=await page.locator('.preview-media').boundingBox();const previewWidth=(await page.locator('.source-preview').boundingBox()).width;
    assert(Math.abs(mediaFrame.width-previewWidth)<1,'Landscape preview uses the full content width');assert(Math.abs(mediaFrame.width/mediaFrame.height-1600/900)<.01);
    await assertNoOverflow(page);await page.locator('#main').evaluate(element=>element.scrollTop=0);await page.screenshot({path:`artifacts/nabnab/preview-${width}-${scale}x.png`});
    if(height>=780){const downloadBox=await page.locator('#download').boundingBox();assert(downloadBox.y+downloadBox.height<=height-4,`The main download action fits completely: ${JSON.stringify({width,height,scale,downloadBox})}`);}
    await page.locator('#download').click();
    await page.waitForFunction(()=>window.fixture.calls.some(call=>call.cmd==='queue_download'));
    assert.equal(await page.evaluate(()=>window.fixture.calls.find(call=>call.cmd==='queue_download').args.subtitleLanguage),'es');
    assert.equal(await source.inputValue(),'');
    await page.locator('[data-page=downloads]').click();await page.locator('.job-row').waitFor();
    assert.equal(await page.locator('#field').evaluate(element=>element.hidden),true);
    await page.waitForTimeout(150);const stoppedFrames=await page.evaluate(()=>window.visualFrames);await page.waitForTimeout(150);assert.equal(await page.evaluate(()=>window.visualFrames),stoppedFrames,'Background animation stops outside Home');
    const focusedJobId=await page.locator('[data-select-job]').first().getAttribute('data-select-job');
    await page.locator('[data-select-job]').first().focus();
    await page.evaluate(async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');const item=window.fixture.state.jobs[0];item.runId=2;item.state='processing';item.updatedAt=Date.now()+2;item.progress={phase:'processing',percent:100,speed:null,downloaded:null,total:null};await emit('job-update',structuredClone(item));});
    await page.waitForFunction(()=>document.querySelector('.job-row')?.dataset.state==='processing');
    assert.equal(await page.evaluate(()=>document.activeElement?.getAttribute('data-select-job')),focusedJobId,'Progress rerenders preserve the focused queue checkbox');
    assert.doesNotMatch(await page.locator('.job-row').innerText(),/100(?:\.0)?%/);
    await page.screenshot({path:`artifacts/nabnab/active-downloads-${width}-${scale}x.png`});
    if(width===1280){await page.locator('#toggle-sidebar').click();await page.screenshot({path:'artifacts/nabnab/active-downloads-collapsed-1280-1x.png'});await page.locator('#toggle-sidebar').click();}
    await page.locator('.row-actions [data-action=cancel-job]').focus();
    await page.locator('.row-actions [data-action=cancel-job]').evaluate(element=>window.fixture.focusedAction=element);
    await page.evaluate(async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');const item=window.fixture.state.jobs[0];item.updatedAt=Date.now()+20;await emit('job-update',structuredClone(item));});
    await page.waitForFunction(()=>document.querySelector('.row-actions [data-action=cancel-job]')!==window.fixture.focusedAction);
    assert.deepEqual(await page.evaluate(()=>[document.activeElement?.getAttribute('data-action'),document.activeElement?.getAttribute('data-id')]),['cancel-job',focusedJobId],'Progress rerenders preserve the focused row action');
    await page.evaluate(async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('job-update',{...structuredClone(window.fixture.state.jobs[0]),runId:1,state:'completed',updatedAt:Date.now()+9999});});
    await page.waitForTimeout(150);assert.equal(await page.locator('.job-row').getAttribute('data-state'),'processing');
    await page.locator('[data-action=inspect-job]').click();
    const jobInspector=width<1120?page.locator('#detail-dialog'):page.locator('#inspector');
    assert.match(await jobInspector.locator('.metadata-list').innerText(),/Subtítulos\s+es/);
    if(width<1120){assert.equal(await page.locator('#detail-dialog').evaluate(dialog=>dialog.open),true);await page.keyboard.press('Escape');}
    else {
      await page.locator('[data-select-job]').first().focus();
      await page.locator('[data-select-job]').first().evaluate(element=>window.fixture.focusedCheckbox=element);
      await page.evaluate(async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');const item=window.fixture.state.jobs[0];item.updatedAt=Date.now()+30;await emit('job-update',structuredClone(item));});
      await page.waitForFunction(()=>document.querySelector('[data-select-job]')!==window.fixture.focusedCheckbox);
      assert.equal(await page.evaluate(()=>document.activeElement?.getAttribute('data-select-job')),focusedJobId,'Refreshing the inspector does not steal focus from the queue');
      await page.locator('#inspector [data-action=close-inspector]').click();
    }
    await page.locator('.row-actions [data-action=cancel-job]').click();await page.waitForFunction(()=>document.querySelector('.job-row')?.dataset.state==='cancelled');
    await page.screenshot({path:`artifacts/nabnab/downloads-${width}-${scale}x.png`});
    if((width===1280||width===1180)&&scale===1){
      await page.evaluate(async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');const item=window.fixture.state.jobs[0];item.runId=3;item.state='error';item.updatedAt=Date.now()+10;item.error={code:'network',message:'Connection failed. Check the connection and retry.',technical:'Network fixture. Sensitive URLs are excluded.',retryable:true,retryAfter:null,phase:'download'};await emit('job-update',structuredClone(item));});
      await page.waitForFunction(()=>document.querySelector('.job-row')?.dataset.state==='error');await page.locator('[data-action=inspect-job]').click();
      await page.screenshot({path:`artifacts/nabnab/error-${width}-1x.png`});await page.locator('#inspector [data-action=close-inspector]').click();
      const malformed=await page.evaluate(async()=>{const {errorLabel,errorDetails}=await import('/src/presentation.ts');return [errorLabel({code:1,message:7,technical:false}),errorDetails({code:'x',message:'x',technical:'x',phase:'x',retryable:'yes',retryAfter:-1})];});
      assert.deepEqual(malformed,['[object Object]','[object Object]']);
    }
    await page.locator('[data-page=library]').click();await page.locator('[data-action=import-files]').first().click();
    await page.waitForFunction(()=>document.querySelectorAll('.media-item').length===60);
    assert.equal(await page.locator('.media-item').count(),60);
    assert.match(await page.locator('.media-item').first().innerText(),/No encontrado/);
    await page.locator('[data-action=library-next]').click();assert.equal(await page.locator('.media-item').count(),60);
    assert.match(await page.locator('.media-item').first().innerText(),/Clip 0060/);
    await page.locator('[data-action=library-previous]').click();
    if(width===1280){
      await page.evaluate(async()=>{window.fixture.fullLibrary=window.fixture.state.library;window.fixture.state.library=window.fixture.fullLibrary.slice(0,61);const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('library-update');});
      await page.locator('[data-action=library-next]').focus();await page.keyboard.press('Enter');
      await page.waitForFunction(()=>document.querySelectorAll('.media-item').length===1);
      assert.equal(await page.evaluate(()=>document.activeElement?.getAttribute('data-id')),'local-60','The last page focuses its first visible result');
      const focusBox=await page.locator('.media-open').boundingBox();assert(focusBox.y>=36&&focusBox.y+focusBox.height<=height,'Pagination focus remains visible');
      await page.keyboard.press('Tab');assert.notEqual(await page.evaluate(()=>document.activeElement?.tagName),'BODY');
      await page.evaluate(async()=>{window.fixture.state.library=window.fixture.fullLibrary;const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('library-update');});
      await page.locator('[data-action=library-previous]').click();
    }
    await page.locator('.media-open').nth(2).click();
    const inspector=width<1120?page.locator('#detail-dialog'):page.locator('#inspector');
    await inspector.locator('#media-collections').fill('Examples');await inspector.locator('#media-tags').fill('demo, local');await inspector.locator('#organize-media button[type=submit]').click();
    await page.waitForFunction(()=>window.fixture.state.library[2].collections.includes('Examples'));
    await page.screenshot({path:`artifacts/nabnab/inspector-${width}-${scale}x.png`});
    await inspector.evaluate(element=>element.scrollTop=0);await page.locator('#main').evaluate(element=>element.scrollTop=0);
    await page.screenshot({path:`artifacts/nabnab/inspector-top-${width}-${scale}x.png`});
    await inspector.locator('[data-action=close-inspector]').click();
    await assertNoOverflow(page);await page.screenshot({path:`artifacts/nabnab/library-${width}-${scale}x.png`});
    await page.locator('.media-open').nth(2).click();await inspector.locator('[data-action=tool-media]').click();
    await page.locator('[data-action=tool-kind][data-kind=trim]').click();
    await page.locator('#tool-start').fill('2');await page.locator('#tool-end').fill('8');await page.locator('#tool-precise').check();
    await page.locator('[data-page=downloads]').click();await page.locator('[data-page=tools]').click();assert.equal(await page.locator('#tool-start').inputValue(),'2');
    await assertNoOverflow(page);await page.locator('#main').evaluate(element=>element.scrollTop=0);await page.screenshot({path:`artifacts/nabnab/tools-${width}-${scale}x.png`});
    await page.locator('#tool-form button[type=submit]').click();await page.waitForFunction(()=>window.fixture.calls.some(call=>call.cmd==='queue_tool'));
    const queued=await page.evaluate(()=>window.fixture.calls.find(call=>call.cmd==='queue_tool').args);assert.deepEqual(queued.options,{kind:'trim',start:2,end:8,precise:true});
    if(width===1180&&scale===1){
      for(const [kind,options]of [['convert',{kind:'convert',container:'mkv',videoCodec:'copy',audioCodec:'copy'}],['extractAudio',{kind:'extractAudio',format:'m4a'}],['compress',{kind:'compress',quality:'small',targetMb:12}]]){
        await page.locator('[data-page=tools]').click();await page.locator(`[data-action=tool-kind][data-kind=${kind}]`).click();
        if(kind==='convert'){await page.locator('#tool-container').selectOption('mkv');await page.locator('#tool-video-codec').selectOption('copy');await page.locator('#tool-audio-codec').selectOption('copy');}
        if(kind==='extractAudio')await page.locator('#tool-audio-format').selectOption('m4a');
        if(kind==='compress'){await page.locator('#tool-compression').selectOption('small');await page.locator('#tool-target').fill('12');}
        const count=await page.evaluate(()=>window.fixture.calls.filter(call=>call.cmd==='queue_tool').length);await page.locator('#tool-form button[type=submit]').click();
        await page.waitForFunction(count=>window.fixture.calls.filter(call=>call.cmd==='queue_tool').length===count+1,count);
        assert.deepEqual(await page.evaluate(()=>window.fixture.calls.filter(call=>call.cmd==='queue_tool').at(-1).args.options),options);
      }
    }
    await page.locator('[data-page=settings]').click();await page.locator('#language').selectOption('en');await page.locator('#general-settings button[type=submit]').click();
    assert.equal(await page.locator('html').getAttribute('lang'),'en');
    assert.match(await page.locator('[data-page=downloads]').innerText(),/Downloads/);
    await page.locator('[data-action=settings-section][data-section=engines]').click();await page.locator('.engine-list li').first().waitFor();
    await assertNoOverflow(page);await page.screenshot({path:`artifacts/nabnab/settings-${width}-${scale}x.png`});
    await page.locator('[data-action=settings-section][data-section=appearance]').click();
    await page.locator('#reduced-transparency').check();await page.locator('#reduced-motion').check();await page.locator('#appearance-settings button[type=submit]').click();
    assert.equal(await page.locator('html').evaluate(element=>element.classList.contains('reduced-transparency')),true);
    assert.equal(await page.evaluate(()=>window.fixture.state.settings.reducedTransparency),true,'Reduced transparency persists through save_settings');
    await page.locator('[data-page=home]').click();
    assert.equal(await page.locator('.url-control').evaluate(element=>getComputedStyle(element).backdropFilter),'none');
    assert.equal(await page.locator('.url-control').evaluate(element=>getComputedStyle(element).backgroundColor),'rgb(16, 17, 20)');
    await page.waitForTimeout(100);const reducedFrames=await page.evaluate(()=>window.visualFrames);await page.waitForTimeout(120);assert.equal(await page.evaluate(()=>window.visualFrames),reducedFrames);
    if(width===1280)await page.screenshot({path:'artifacts/nabnab/reduced-transparency-1280-1x.png'});
    await page.locator('[data-page=settings]').click();await page.locator('[data-action=settings-section][data-section=appearance]').click();
    await page.locator('#reduced-transparency').uncheck();await page.locator('#reduced-motion').uncheck();await page.locator('#appearance-settings button[type=submit]').click();
    await page.locator('[data-action=settings-section][data-section=sessions]').click();
    assert.equal(await page.locator('#session-site').inputValue(),'youtube');
    await page.locator('#session-form button[type=submit]').click();
    await page.waitForFunction(()=>window.fixture.calls.some(call=>call.cmd==='import_session'&&call.args.site==='youtube'));
    await page.locator('#session-site').selectOption('twitter');await page.locator('#session-form button[type=submit]').click();
    await page.waitForFunction(()=>window.fixture.calls.some(call=>call.cmd==='import_session'&&call.args.site==='twitter'));
    assert.equal(await page.locator('#session-site option[value=pinterest]').count(),1);
    await page.locator('[data-action=export-diagnostics]').click();
    assert(await page.evaluate(()=>window.fixture.calls.some(call=>call.cmd==='export_diagnostics')));
    await page.locator('[data-page=library]').click();
    if(width>=1120||!(await page.locator('#detail-dialog').evaluate(dialog=>dialog.open)))await page.locator('.media-open').nth(2).click();
    await inspector.locator('[data-action=remove-media]').click();
    assert.equal(await page.evaluate(()=>window.fixture.calls.some(call=>/delete_file|trash_file/.test(call.cmd))),false);
    await page.locator('[data-page=home]').click();
    const metadataCount=await page.evaluate(()=>window.fixture.metadata.length);
    await source.fill('https://example.org/after-enqueue');await source.press('Enter');
    await page.waitForFunction(count=>window.fixture.metadata.length===count+1,metadataCount);
    await page.evaluate(()=>{const pending=window.fixture.metadata.at(-1);pending.resolve({...window.fixture.mediaTemplate,requestId:pending.id});});
    await page.locator('#subtitles').waitFor();assert.equal(await page.locator('#subtitles').inputValue(),'','The next preview starts without the previous subtitle choice');
    await page.locator('#subtitles').selectOption('es');await page.locator('#output-format').selectOption('mp3');
    assert.equal(await page.locator('#subtitles').count(),0,'Audio output hides subtitle controls');
    const audioRequests=await page.evaluate(()=>window.fixture.calls.filter(call=>call.cmd==='queue_download').length);
    await page.locator('#download').click();await page.waitForFunction(count=>window.fixture.calls.filter(call=>call.cmd==='queue_download').length===count+1,audioRequests);
    assert.equal(await page.evaluate(()=>window.fixture.calls.filter(call=>call.cmd==='queue_download').at(-1).args.subtitleLanguage),null,'Audio output never sends a stale subtitle language');
    await source.fill('https://example.org/batch-one\nhttps://example.org/batch-two');
    await page.locator('.source-batch').waitFor();await page.locator('#output-format').selectOption('mp4');
    assert((await source.boundingBox()).height>52,'Multiple URLs grow the source input');assert((await source.boundingBox()).height<=128,'Source input growth stays bounded');
    assert.equal(await page.locator('#subtitles').count(),0,'Batch output hides subtitle controls');
    const batchRequests=await page.evaluate(()=>window.fixture.calls.filter(call=>call.cmd==='queue_download').length);
    await page.locator('#download').click();await page.waitForFunction(count=>window.fixture.calls.filter(call=>call.cmd==='queue_download').length===count+1,batchRequests);
    const batchPayload=await page.evaluate(()=>window.fixture.calls.filter(call=>call.cmd==='queue_download').at(-1).args);
    assert.equal(batchPayload.subtitleLanguage,null);assert.deepEqual(batchPayload.urls,['https://example.org/batch-one','https://example.org/batch-two']);
    const portraitCount=await page.evaluate(()=>window.fixture.metadata.length);
    await source.fill('https://example.org/portrait');await source.press('Enter');await page.waitForFunction(count=>window.fixture.metadata.length===count+1,portraitCount);
    await page.evaluate(()=>{const pending=window.fixture.metadata.at(-1);pending.resolve({...window.fixture.mediaTemplate,requestId:pending.id,title:'Portrait media · UI fixture',thumbnail:'https://example.org/portrait-fixture'});});
    await page.waitForFunction(()=>document.querySelector('.preview-media')?.dataset.orientation==='portrait');
    const portraitFrame=await page.locator('.preview-media').boundingBox();assert(Math.abs(portraitFrame.width/portraitFrame.height-900/1600)<.01);assert(portraitFrame.height<=height*.42+1);
    await page.locator('#main').evaluate(element=>element.scrollTop=0);await page.screenshot({path:`artifacts/nabnab/portrait-${width}-${scale}x.png`});await assertNoOverflow(page);
    if(width===1280){await page.locator('#toggle-sidebar').click();await page.screenshot({path:'artifacts/nabnab/portrait-collapsed-1280-1x.png'});await page.locator('#toggle-sidebar').click();}
    assert.deepEqual(errors,[]);
    results.push({width,height,scale,checks:'IPC UI fixtures: full-window field, natural thumbnail ratios, complete CTA, metadata/subtitle reset races, explicit subtitle enqueue, audio/batch exclusions, stable polite status, keyboard focus, last-page focus, phases, stale run, cancel, bounded 1000-item pagination, library organization, tool options, language, sessions, reduced motion/transparency persistence, diagnostics, safe removal'});
    await page.close();
  }
  await writeFile('artifacts/nabnab/browser-results.json',JSON.stringify(results,null,2));
  await captureWalkthrough();
  console.log(JSON.stringify(results,null,2));
  }
} finally { await browser.close(); }
