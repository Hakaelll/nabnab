// UI contract checks use explicit IPC fixtures. Native media/security checks run separately.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }) });
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const results = [];
await mkdir("artifacts/nabnab", { recursive: true });
const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri = true; mockWindows("main");
  const settings = {folder:"C:/Downloads",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
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
    if(cmd==="queue_download") { const jobs=args.urls.map((url,index)=>job(args.titles[index]||"Fixture download",{kind:"download",url,format:args.format,height:args.height,audioBitrate:args.audioBitrate}));state.jobs.push(...jobs);return copy(jobs); }
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
  await page.route(base, async route => {
    const response=await route.fetch();
    const html=await response.text();
    assert.match(html,/<script type="module" src="\/src\/main\.ts[^"]*"><\/script>/);
    await route.fulfill({response,body:html.replace(/<script type="module" src="\/src\/main\.ts[^"]*"><\/script>/,'<script type="module">'+setup+'</script>')});
  });
  await page.route("http://nabmedia.localhost/**",route=>route.fulfill({status:404,body:"UI fixture has no real media"}));
}
async function assertNoOverflow(page) {
  const result=await page.evaluate(()=>({document:document.documentElement.scrollWidth,width:innerWidth,main:document.querySelector('#main').scrollWidth,mainWidth:document.querySelector('#main').clientWidth}));
  assert(result.document<=result.width+1,JSON.stringify(result));
  assert(result.main<=result.mainWidth+1,JSON.stringify(result));
}
try {
  const plain=await browser.newPage({viewport:{width:1180,height:780}});
  await plain.goto(base);await plain.locator('#url').waitFor();
  assert.match(await plain.locator('body').innerText(),/aplicación de escritorio nabnab/);
  assert.equal(await plain.locator('#load').isDisabled(),true);
  await plain.screenshot({path:'artifacts/nabnab/browser-mode.png'});await plain.close();
  for(const [width,height,scale] of [[1180,780,1],[900,620,1],[1180,780,1.25],[1180,780,1.5]]){
    const page=await browser.newPage({viewport:{width,height},deviceScaleFactor:scale});
    const errors=[];page.on('pageerror',error=>errors.push(error.message));await mockPage(page);await page.goto(base);
    await page.waitForFunction(()=>window.fixture?.calls.some(call=>call.cmd==='startup'));
    await page.locator('#url').waitFor();await assertNoOverflow(page);
    await page.screenshot({path:`artifacts/nabnab/home-${width}-${scale}x.png`});
    const source=page.locator('#url');
    await source.fill('https://example.org/old');await source.press('Enter');
    await page.waitForFunction(()=>window.fixture.metadata.length===1);
    const box=await page.locator('.url-control').boundingBox();
    assert.equal(await page.locator('#source-form').getAttribute('aria-busy'),'true');
    assert.equal(await page.locator('.url-control').evaluate(element=>getComputedStyle(element,'::after').height),'1px');
    await page.screenshot({path:`artifacts/nabnab/analysis-${width}-${scale}x.png`});
    await source.fill('https://example.org/new');await source.press('Enter');
    await page.waitForFunction(()=>window.fixture.metadata.length===2);
    await page.evaluate(()=>{
      const media=(id,title)=>({requestId:id,title,duration:30,thumbnail:null,uploader:'Authorized UI fixture',heights:[720,360],hasAudio:true,hasVideo:true,entries:[],capabilities:{video:true,audio:true,images:false,batches:false,subtitles:false,verified:false}});
      window.fixture.metadata[1].resolve(media(window.fixture.metadata[1].id,'Vídeo <b>literal</b>'));
      window.fixture.metadata[0].resolve(media(window.fixture.metadata[0].id,'Stale result'));
    });
    await page.locator('#video-title').waitFor();
    assert.equal(await page.locator('#video-title').innerText(),'Vídeo <b>literal</b>');
    assert.equal(await page.locator('#video-title b').count(),0);
    assert.equal(await page.evaluate(()=>window.fixture.calls.filter(call=>call.cmd==='queue_download').length),0);
    const after=await page.locator('.url-control').boundingBox();assert(Math.abs(after.y-box.y)<1);
    await assertNoOverflow(page);await page.screenshot({path:`artifacts/nabnab/preview-${width}-${scale}x.png`});
    await page.locator('#download').click();
    await page.waitForFunction(()=>window.fixture.calls.some(call=>call.cmd==='queue_download'));
    assert.equal(await source.inputValue(),'');
    await page.locator('[data-page=downloads]').click();await page.locator('.job-row').waitFor();
    await page.evaluate(async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');const item=window.fixture.state.jobs[0];item.runId=2;item.state='processing';item.updatedAt=Date.now()+2;item.progress={phase:'processing',percent:100,speed:null,downloaded:null,total:null};await emit('job-update',structuredClone(item));});
    await page.waitForFunction(()=>document.querySelector('.job-row')?.dataset.state==='processing');
    assert.doesNotMatch(await page.locator('.job-row').innerText(),/100(?:\.0)?%/);
    await page.evaluate(async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');await emit('job-update',{...structuredClone(window.fixture.state.jobs[0]),runId:1,state:'completed',updatedAt:Date.now()+9999});});
    await page.waitForTimeout(150);assert.equal(await page.locator('.job-row').getAttribute('data-state'),'processing');
    await page.locator('[data-action=inspect-job]').click();
    if(width<1120){assert.equal(await page.locator('#detail-dialog').evaluate(dialog=>dialog.open),true);await page.keyboard.press('Escape');}
    else await page.locator('#inspector [data-action=close-inspector]').click();
    await page.locator('.row-actions [data-action=cancel-job]').click();await page.waitForFunction(()=>document.querySelector('.job-row')?.dataset.state==='cancelled');
    await page.screenshot({path:`artifacts/nabnab/downloads-${width}-${scale}x.png`});
    if(width===1180&&scale===1){
      await page.evaluate(async()=>{const {emit}=await import('/node_modules/@tauri-apps/api/event.js');const item=window.fixture.state.jobs[0];item.runId=3;item.state='error';item.updatedAt=Date.now()+10;item.error={code:'network',message:'Connection failed. Check the connection and retry.',technical:'Network fixture. Sensitive URLs are excluded.',retryable:true,retryAfter:null,phase:'download'};await emit('job-update',structuredClone(item));});
      await page.waitForFunction(()=>document.querySelector('.job-row')?.dataset.state==='error');await page.locator('[data-action=inspect-job]').click();
      await page.screenshot({path:'artifacts/nabnab/error-1180-1x.png'});await page.locator('#inspector [data-action=close-inspector]').click();
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
    await page.locator('.media-open').nth(2).click();
    const inspector=width<1120?page.locator('#detail-dialog'):page.locator('#inspector');
    await inspector.locator('#media-collections').fill('Examples');await inspector.locator('#media-tags').fill('demo, local');await inspector.locator('#organize-media button[type=submit]').click();
    await page.waitForFunction(()=>window.fixture.state.library[2].collections.includes('Examples'));
    await page.screenshot({path:`artifacts/nabnab/inspector-${width}-${scale}x.png`});
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
    await page.locator('[data-action=settings-section][data-section=sessions]').click();await page.locator('[data-action=export-diagnostics]').click();
    assert(await page.evaluate(()=>window.fixture.calls.some(call=>call.cmd==='export_diagnostics')));
    await page.locator('[data-page=library]').click();
    if(width>=1120||!(await page.locator('#detail-dialog').evaluate(dialog=>dialog.open)))await page.locator('.media-open').nth(2).click();
    await inspector.locator('[data-action=remove-media]').click();
    assert.equal(await page.evaluate(()=>window.fixture.calls.some(call=>/delete_file|trash_file/.test(call.cmd))),false);
    assert.deepEqual(errors,[]);
    results.push({width,height,scale,checks:'IPC UI fixtures: metadata races, explicit enqueue, phases, stale run, cancel, 1000-item pagination, library organization, trim options, language, diagnostics, safe removal'});
    await page.close();
  }
  await writeFile('artifacts/nabnab/browser-results.json',JSON.stringify(results,null,2));
  console.log(JSON.stringify(results,null,2));
} finally { await browser.close(); }
