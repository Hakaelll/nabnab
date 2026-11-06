// Rendered section layouts with IPC fixtures. No provider or engine access is asserted.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({headless:true,channel:"msedge"});
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = "artifacts/section-design";
await mkdir(directory,{recursive:true});
const before = process.argv.includes("--before");
const page = await browser.newPage({viewport:{width:1180,height:780}});
const errors=[];page.on("pageerror",error=>errors.push(error.message));
const setup=`
import {mockIPC,mockWindows} from "/node_modules/@tauri-apps/api/mocks.js";
globalThis.isTauri=true;mockWindows("main");
const settings={folder:"C:/Downloads/Personal archive",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:false,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
const library=[{id:"clip",name:"Entrevista de archivo.mp4",kind:"video",path:"C:/Media/Entrevista de archivo.mp4",size:120000000,duration:436,width:1920,height:1080,videoCodec:"h264",audioCodec:"aac",container:"mp4",addedAt:Date.now(),source:null,parentId:null,collections:["Entrevistas"],tags:[],missing:false}];
const jobs=[
{id:"live",title:"Un recorrido por la ciudad",state:"downloading",provider:"YouTube",source:{kind:"download",url:"https://youtube.com/watch?v=fixture",format:"source",height:1080,audioBitrate:null},progress:{phase:"downloading",percent:43.2,speed:6291456,downloaded:120000000,total:280000000},outputs:[],error:null},
{id:"queued",title:"Entrevista de archivo · Audio",state:"pending",provider:"FFmpeg",source:{kind:"transform",libraryId:"clip",options:{kind:"extractAudio",format:"mp3"}},progress:null,outputs:[],error:null},
{id:"complete",title:"Documento de referencia.pdf",state:"completed",provider:"Google Drive",source:{kind:"cloudFile",selection:{provider:"drive",accountId:"fixture",id:"doc",version:"v1",exportMime:null},collection:"Lecturas"},progress:null,outputs:["C:/Downloads/Documento de referencia.pdf"],error:null},
{id:"failed",title:"Colección de imágenes",state:"error",provider:"Wikimedia Commons",source:{kind:"publicFile",selection:{sourceUrl:"https://commons.wikimedia.org/wiki/File:Fixture.jpg",filename:"Fixture.jpg",assetId:"commons:fixture",collection:"Referencias",author:null,license:null,attribution:null,sha256:null,version:null,size:null}},progress:null,outputs:[],error:{code:"network",message:"La conexión se interrumpió. Puedes reintentar el trabajo.",technical:"UI fixture only",retryable:true,retryAfter:null}}
].map((job,index)=>({...job,runId:1,createdAt:Date.now()-index*60000,updatedAt:Date.now(),folder:settings.folder,thumbnail:null,attempt:1,groupId:null}));
const state={settings,library,jobs,collections:["Entrevistas","Lecturas"],drafts:[],sources:[],rules:[],presets:[],smartCollections:[]};
window.sectionFixture={state,calls:[]};
mockIPC(async(cmd,args={})=>{
window.sectionFixture.calls.push({cmd,args});
if(cmd==="startup")return {...structuredClone(state),folder:settings.folder,enginesReady:true,engineError:null};
if(cmd==="get_snapshot")return structuredClone(state);
if(cmd==="local_timezone")return "Romance Standard Time";
if(cmd==="engine_status")return ["yt-dlp","FFmpeg","ffprobe","Deno"].map(name=>({name,version:"Installed fixture",path:"C:/Components/"+name,size:1024,status:"installed",source:"Local UI fixture"}));
if(cmd==="optional_components"||cmd==="sessions_status"||cmd==="list_courses"||cmd==="ocr_models")return [];
if(cmd==="ocr_status")return {installed:false,enabled:false};
if(cmd==="cache_status")return {bytes:0,files:0};
if(cmd==="cloud_status")return ["drive","onedrive","dropbox"].map(provider=>({provider,configured:false,connected:false,accountId:null,scope:"Read selected files",redirectUri:"http://127.0.0.1:53687/callback"}));
if(cmd==="bridge_status")return {enabled:false,clients:[],port:49587};
if(cmd==="save_settings"){state.settings=args.settings;return structuredClone(state);}
if(cmd==="job_action"){for(const job of jobs)if(args.ids.includes(job.id))job.state=args.action==="retry"?"pending":"cancelled";return structuredClone(state);}
if(cmd==="telegram_status")return {installed:false,accounts:[]};
if(cmd==="torrent_status")return {installed:false,enabled:false,version:null};
if(cmd==="thumbnail")return "data:image/svg+xml,"+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90"><path fill="#303236" d="M0 0h160v90H0z"/><path fill="#56595e" d="m0 80 55-50 40 35 25-20 40 35v10H0z"/></svg>');
}, {shouldMockEvents:true});
await import("/src/main.ts");`;
await page.route(base,async route=>{const response=await route.fetch();const html=(await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/,`<script type="module">${setup}</script>`);await route.fulfill({response,body:html});});
const checks=[];
async function capture(section,width,height){
  await page.setViewportSize({width,height});
  await page.locator(`[data-page="${section}"]`).click();
  await page.locator("#page-content h1").waitFor();
  await page.locator("#page-content").evaluate(el=>el.scrollTop=0);
  const dimensions=await page.locator("#page-content").evaluate(el=>({width:el.clientWidth,scroll:el.scrollWidth}));
  assert(dimensions.scroll<=dimensions.width+2,`${section} ${width} overflow ${JSON.stringify(dimensions)}`);
  await page.screenshot({path:`${directory}/${section}-${before?"before":"after"}-${width}x${height}.png`});
}
try{
  await page.goto(base);await page.locator("#url").waitFor();
  if(!before)await page.addStyleTag({url:"/src/sections.css"});
  for(const size of [[1180,780],[900,620]])for(const section of ["downloads","tools","platforms","settings"])await capture(section,...size);
  if(!before){
    await page.locator('[data-page="downloads"]').click();
    assert.equal(await page.locator(".job-row").count(),4);
    await page.locator("[data-action='job-filter'][data-filter='error']").click();
    assert.equal(await page.locator(".job-row").count(),1);
    await page.locator("[data-action='job-filter'][data-filter='all']").click();
    await page.locator("#job-search").fill("Documento");assert.equal(await page.locator(".job-row").count(),1);
    checks.push("Queue filters and search preserve actual rows and actions.");
    await page.locator('[data-page="tools"]').click();
    await page.locator('[data-action="tool-kind"][data-kind="extractAudio"]').click();
    await page.locator("#tool-audio-format").waitFor();
    await page.locator("[data-action='pick-tool-library']").click();
    await page.locator('[data-tool-input="clip"]').check();
    await page.locator("[data-action='finish-tool-picker']").click();
    assert.equal(await page.locator(".tool-inputs li").count(),1);
    assert.equal(await page.locator("#tool-form button[type=submit]").isEnabled(),true);
    checks.push("Tool navigation, library input selection and options remain connected.");
    for(const module of ['imageAssets','fileOrganization','portable']){
      await page.locator('[data-action="tool-module"][data-id="'+module+'"]').click();
      const heading=page.locator('.tool-workspace h2').first();
      assert.ok(await heading.evaluate(element=>Number.parseFloat(getComputedStyle(element).fontSize))>=18,'Module titles must keep their visual hierarchy.');
    }
    checks.push("Specialized tool titles preserve their hierarchy in the actual application shell.");
    await page.locator('[data-page="platforms"]').click();
    await page.locator("[data-action='platform-group'][data-id='files']").click();
    assert.equal(await page.locator(".platform-row[data-id='github']").count(),1);
    assert.equal(await page.locator(".platform-row[data-id='youtube']").count(),0);
    await page.locator("#platform-search").fill("Dropbox");assert.equal(await page.locator(".platform-row").count(),1);
    checks.push("Platform categories and name search retain their source actions.");
    await page.locator('[data-page="settings"]').click();
    await page.locator("[data-action='settings-section'][data-section='appearance']").click();
    await page.locator("#reduced-transparency").check();
    await page.locator("#appearance-settings button[type=submit]").click();
    assert.equal(await page.evaluate(()=>window.sectionFixture.state.settings.reducedTransparency),true);
    await page.locator("[data-action='settings-section'][data-section='general']").click();
    await page.locator("#language").focus();assert.equal(await page.locator("#language").evaluate(el=>el===document.activeElement),true);
    checks.push("Settings navigation, persistent appearance preference and keyboard focus work.");
  }
  assert.deepEqual(errors,[]);
  await writeFile(`${directory}/${before?"baseline":"verification"}.json`,JSON.stringify({evidence:"Rendered UI with IPC fixtures",checks,browserErrors:errors},null,2)+"\n");
  console.log(before?"Captured existing sections at both desktop sizes.":checks.join("\n"));
}finally{await browser.close();}
