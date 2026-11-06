// UI fixtures exercise the real frontend. Native batch persistence is tested in batch.rs.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }) });
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
await mkdir("artifacts/batch", { recursive: true });
const page = await browser.newPage({ viewport: { width: 1440, height: 980 } });
const failures=[];
page.on("pageerror",error=>failures.push(error.message));
const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  import { batchCandidates } from "/src/batch-types.ts";
  import { defaultWorkOptions } from "/src/options.ts";
  globalThis.isTauri=true;mockWindows("main");
  const copy=value=>structuredClone(value);
  const settings={folder:"C:/Downloads/nabnab-fixture",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
  const state={settings,jobs:[],library:[],collections:[],drafts:[],sources:[],rules:[],smartCollections:[],presets:[{id:"video",name:"Vídeo compatible",revision:1,options:defaultWorkOptions()},{id:"audio",name:"Audio",revision:1,options:{...defaultWorkOptions(),format:"audio"}}]};
  const fixture=window.batchFixture={state,calls:[],errors:[],holdAnalysis:true,analysisWaiters:[],holdSaveResponses:false,saveResponses:[]};
  let sequence=0;
  const stamp=()=>Date.now()+(++sequence);
  mockIPC(async(cmd,args={})=>{
    fixture.calls.push({cmd,args:copy(args)});
    if(cmd==="plugin:window|is_maximized")return false;
    if(cmd==="startup")return {...copy(state),folder:settings.folder,enginesReady:true,engineError:null};
    if(cmd==="get_snapshot")return copy(state);
    if(cmd==="local_timezone")return "Romance Standard Time";
    if(cmd==="batch_create"){
      const createdAt=stamp();
      const draft={id:"fixture-batch-"+sequence,name:args.name||"Lote de prueba local",createdAt,updatedAt:createdAt,savedForLater:false,presetId:args.presetId??null,options:null,explorationLimit:200,
      items:batchCandidates(args.text).map((item,index)=>({id:"fixture-item-"+index,url:item.url,title:item.url.includes("one")?"Camera lesson":item.url.includes("two")?"Sound lesson":item.url,provider:item.valid?"YouTube":"",creator:"Fixture creator",contentId:"fixture-content-"+index,thumbnail:null,duration:item.valid?120:null,state:item.valid?"pending":"unsupported",picked:item.valid,excluded:false,options:null,resolved:null,destination:null,error:item.valid?null:"Invalid fixture URL",list:null,media:null}))};
      state.drafts.push(draft);return copy(draft);
    }
    if(cmd==="batch_inspect"){
      if(fixture.holdAnalysis)await new Promise(resolve=>fixture.analysisWaiters.push(resolve));
      await new Promise(resolve=>setTimeout(resolve,30));
      const draft=state.drafts.find(draft=>draft.id===args.draftId);const item=draft.items.find(item=>item.id===args.itemId);
      const values=item.options??draft.options??state.presets.find(preset=>preset.id===draft.presetId)?.options??defaultWorkOptions();
      item.state="ready";item.resolved={version:1,presetId:draft.presetId,presetRevision:1,values:copy(values),origins:{options:"UI fixture"}};
      item.media={requestId:1,title:item.title,duration:120,thumbnail:null,uploader:item.creator,heights:[720],hasAudio:true,hasVideo:true,entries:[],capabilities:{video:true,audio:true,images:false,batches:false,subtitles:false,verified:false},chapters:[{title:"Opening fixture",start:0,end:30},{title:"Detail fixture",start:30,end:120}]};
      item.destination="C:/Downloads/nabnab-fixture/"+item.title;draft.updatedAt=stamp();return copy(draft);
    }
    if(cmd==="batch_save"){
      const index=state.drafts.findIndex(draft=>draft.id===args.draft.id);const draft=copy(state.drafts[index]);
      for(const field of ["name","presetId","options","explorationLimit","savedForLater"])draft[field]=copy(args.draft[field]);
      for(const item of draft.items){const incoming=args.draft.items.find(value=>value.id===item.id);if(incoming){item.picked=incoming.picked;item.excluded=incoming.excluded;item.options=copy(incoming.options);}}
      for(const item of draft.items)if(item.state==="ready"){
        const values=item.options??draft.options??state.presets.find(preset=>preset.id===draft.presetId)?.options??defaultWorkOptions();
        item.resolved={version:1,presetId:draft.presetId,presetRevision:1,values:copy(values),origins:{options:item.options?"Cambios del elemento":"UI fixture"}};
        item.destination="C:/Downloads/nabnab-fixture/"+(values.relativeFolder?values.relativeFolder+"/":"")+item.title;
      }
      draft.updatedAt=stamp();state.drafts[index]=draft;const response=copy(draft);
      if(fixture.holdSaveResponses)await new Promise(resolve=>fixture.saveResponses.push(resolve));
      return response;
    }
    if(cmd==="batch_cancel")return copy(state.drafts.find(draft=>draft.id===args.draftId));
    if(cmd==="batch_chapters"){
      const draft=state.drafts.find(draft=>draft.id===args.draftId),original=draft.items.find(item=>item.id===args.itemId);
      original.picked=false;
      for(const index of [...new Set(args.indices)].sort((a,b)=>a-b)){
        const chapter=original.media.chapters[index],id="fixture-chapter-"+index;
        const values={...copy(original.resolved.values),clip:{start:chapter.start,end:chapter.end,precise:false}};
        if(!draft.items.some(item=>item.id===id))draft.items.push({...copy(original),id,title:original.title+" · "+chapter.title,state:"ready",picked:true,duration:chapter.end-chapter.start,options:values,resolved:{...copy(original.resolved),values}});
      }
      draft.updatedAt=stamp();return copy(draft);
    }
    if(cmd==="batch_enqueue"){
      const draft=state.drafts.find(draft=>draft.id===args.draftId);
      const jobs=draft.items.filter(item=>item.picked&&!item.excluded&&item.state==="ready").map(item=>({id:crypto.randomUUID(),runId:0,title:item.title,createdAt:stamp(),updatedAt:stamp(),state:"pending",source:{kind:"download",url:item.url,format:item.resolved.values.format,height:null,audioBitrate:null},folder:settings.folder,provider:"YouTube",thumbnail:null,progress:null,outputs:[],error:null,attempt:0,groupId:draft.id}));
      state.jobs.push(...jobs);for(const item of draft.items)if(jobs.some(job=>job.title===item.title))item.state="active";draft.updatedAt=stamp();return copy(jobs);
    }
    if(cmd==="batch_remove"){state.drafts=state.drafts.filter(draft=>draft.id!==args.id);return;}
    if(cmd==="save_settings"){state.settings=args.settings;return copy(state);}
    if(cmd==="engine_status")return ["yt-dlp","FFmpeg","ffprobe","Deno"].map(name=>({name,version:"UI fixture",path:"C:/Engines/"+name,size:1024,status:"installed",source:"UI fixture"}));
    if(cmd==="sessions_status"||cmd==="library_search"||cmd==="search_library")return [];
    if(cmd==="bridge_status")return {enabled:false,clients:[],port:47883};
    if(cmd==="cache_status")return {bytes:0,files:0};
    if(cmd==="cancel_job")return;
    if(cmd==="queue_download")throw new Error("Mixed batches must use the batch preparer.");
  },{shouldMockEvents:true});
  await import("/src/main.ts");
`;
await page.route(base,async route=>{
  const response=await route.fetch();let html=await response.text();
  const entry=/<script\b[^>]*\bsrc="\/src\/main\.ts[^"]*"[^>]*><\/script>/;
  assert.match(html,entry,"The dev page must contain its main.ts entry script");
  html=html.replace(entry,()=>`<script type="module">${setup}</script>`);
  await route.fulfill({response,body:html});
});

try{
  await page.goto(base);
  await page.locator("#url").waitFor();
  await page.locator("#url").fill("https://www.youtube.com/watch?v=one invalid https://www.youtube.com/watch?v=two");
  await page.locator("#url").press("Enter");
  await page.locator(".batch-workspace").waitFor();
  await page.waitForFunction(()=>window.batchFixture.analysisWaiters.length===2);
  await page.locator("#batch-options-folder").fill("Unsaved/{creador}");
  await page.locator("#batch-options-clip-start").fill("2");
  await page.locator("#batch-options-schedule-kind").selectOption("window");
  await page.locator("#batch-options-schedule-from").fill("13:45");
  await page.evaluate(()=>{window.batchFixture.holdSaveResponses=true;});
  await page.locator("[data-batch-pick='fixture-item-0']").uncheck();
  await page.waitForFunction(()=>window.batchFixture.saveResponses.length===1);
  await page.locator("[data-batch-pick='fixture-item-2']").uncheck();
  await page.evaluate(()=>{const fixture=window.batchFixture;fixture.holdAnalysis=false;fixture.analysisWaiters.splice(0).forEach(resolve=>resolve());});
  await page.waitForFunction(()=>window.batchFixture.state.drafts[0]?.items.filter(item=>item.state==="ready").length===2);
  await page.waitForFunction(()=>[...document.querySelectorAll(".batch-item-state")].filter(item=>item.textContent.includes("Preparado")).length===2);
  assert.equal(await page.locator("#batch-options-folder").inputValue(),"Unsaved/{creador}");
  assert.equal(await page.locator("#batch-options-clip-start").inputValue(),"2");
  assert.equal(await page.locator("#batch-options-clip-end").inputValue(),"");
  assert.equal(await page.locator("#batch-options-schedule-kind").inputValue(),"window");
  assert.equal(await page.locator("#batch-options-schedule-from").inputValue(),"13:45");
  assert.equal(await page.locator("[data-batch-pick='fixture-item-2']").isChecked(),false);
  assert.equal(await page.evaluate(()=>window.batchFixture.state.drafts[0].options),null,"Typing options must not apply them");
  await page.evaluate(()=>window.batchFixture.saveResponses.shift()());
  await page.waitForFunction(()=>window.batchFixture.saveResponses.length===1);
  assert.equal(await page.locator("[data-batch-pick='fixture-item-2']").isChecked(),false,"An older save response must preserve the newer selection");
  await page.locator("[data-batch-pick='fixture-item-0']").check();
  await page.evaluate(()=>{window.batchFixture.holdSaveResponses=false;window.batchFixture.saveResponses.shift()();});
  await page.waitForFunction(()=>window.batchFixture.state.drafts[0].items[0].picked&&!window.batchFixture.state.drafts[0].items[2].picked);
  assert.equal(await page.evaluate(()=>window.batchFixture.state.drafts[0].items.filter(item=>item.state==="ready").length),2,"Saving editable fields must retain analysis metadata");
  assert.equal(await page.locator(".batch-item").count(),3);
  assert.equal(await page.locator(".batch-item-state").filter({hasText:"No compatible"}).count(),1);
  assert.equal(await page.evaluate(()=>window.batchFixture.calls.filter(call=>call.cmd==="batch_enqueue"||call.cmd==="queue_download").length),0);

  await page.locator("[data-action='batch-inspect'][data-id='fixture-item-0']").click();
  assert.equal(await page.locator("#batch-options-folder").inputValue(),"","An item has its own unapplied editor");
  await page.locator("#batch-options-folder").fill("Item unsaved");
  await page.locator("[data-action='batch-common']").click();
  assert.equal(await page.locator("#batch-options-folder").inputValue(),"Unsaved/{creador}");
  await page.locator("[data-action='batch-inspect'][data-id='fixture-item-0']").click();
  assert.equal(await page.locator("#batch-options-folder").inputValue(),"Item unsaved");
  await page.locator("[data-action='batch-common']").click();
  await page.locator("#batch-preset").selectOption("audio");
  await page.waitForFunction(()=>window.batchFixture.state.drafts[0].presetId==="audio");
  assert.equal(await page.locator("#batch-options-folder").inputValue(),"","Changing preset discards unapplied options from the old preset");
  assert.equal(await page.locator("#batch-options-clip-start").inputValue(),"");

  await page.locator("#batch-search").fill("Camera");
  await page.locator("#batch-selection").selectOption("visible");
  await page.waitForFunction(()=>window.batchFixture.state.drafts[0].items.filter(item=>item.picked).length===1);
  assert.equal(await page.locator(".batch-item").count(),1);
  await page.locator("#batch-selection").selectOption("all");
  await page.waitForFunction(()=>window.batchFixture.state.drafts[0].items.filter(item=>item.picked).length===3);
  await page.locator("#batch-selection").selectOption("visible");
  await page.locator("[data-action='batch-inspect']").click();
  await page.locator("#batch-options-format").selectOption("mp3");
  await page.locator("#batch-options-folder").fill("Tutoriales/{creador}");
  await page.locator("[data-action='batch-apply-options']").click();
  await page.waitForFunction(()=>window.batchFixture.state.drafts[0].items[0].options?.format==="mp3");
  assert.match(await page.locator(".batch-item-state").innerText(),/Opciones propias/);
  await page.locator("[data-action='batch-save-later']").click();
  await page.waitForFunction(()=>window.batchFixture.state.drafts[0].savedForLater);
  assert.equal(await page.evaluate(()=>window.batchFixture.state.jobs.length),0);
  await page.locator("#batch-search").fill("");
  await page.screenshot({path:"artifacts/batch/batch-desktop-fixture.png",fullPage:true});
  await page.setViewportSize({width:900,height:720});
  await page.screenshot({path:"artifacts/batch/batch-narrow-fixture.png",fullPage:true});
  const overflow=await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1);
  assert.equal(overflow,false,"Batch must fit a narrow desktop window");
  await page.locator("[data-action='batch-enqueue']").click();
  await page.waitForFunction(()=>window.batchFixture.state.jobs.length===1);
  const queued=await page.evaluate(()=>window.batchFixture.state.jobs.map(job=>({title:job.title,format:job.source.format})));
  assert.deepEqual(queued,[{title:"Camera lesson",format:"mp3"}]);
  await page.setViewportSize({width:1440,height:980});
  await page.locator(".batch-chapters summary").click();
  await page.locator("[data-batch-chapter='0']").check();
  await page.locator("[data-batch-chapter='1']").check();
  await page.locator("[data-action='batch-chapters']").click();
  await page.waitForFunction(()=>window.batchFixture.state.drafts[0].items.length===5);
  const chapterRequest=await page.evaluate(()=>window.batchFixture.calls.filter(call=>call.cmd==="batch_chapters").at(-1).args);
  assert.deepEqual(chapterRequest.indices,[0,1]);
  assert.equal(await page.evaluate(()=>window.batchFixture.state.jobs.length),1,"Preparing chapters must not download them");
  await page.screenshot({path:"artifacts/batch/chapters-fixture.png",fullPage:true});
  assert.deepEqual(failures,[]);
  await writeFile("artifacts/batch/result.json",JSON.stringify({kind:"frontend IPC fixture",checks:["mixed invalid preserved","unapplied inputs survive analysis","save and inspection responses retain optimistic selection and metadata","per-item editor isolation","preset change discards unapplied old options","visible and entire selection distinct","individual MP3 override","save without enqueue","only selected enqueued","900px no horizontal overflow","actual chapter indices delivered without download"],failures},null,2));
  console.log("Batch frontend fixture checks passed. Native behavior is tested separately.");
}catch(error){
  const diagnostics=await page.evaluate(()=>({notification:document.querySelector("#notification")?.textContent,calls:window.batchFixture?.calls.slice(-15).map(call=>({cmd:call.cmd,args:call.args.draft?{id:call.args.draft.id,items:call.args.draft.items.length,picked:call.args.draft.items.filter(item=>item.picked).map(item=>item.id)}:call.args})),body:document.body.innerText.slice(0,6000)})).catch(()=>null);
  console.error(JSON.stringify({failures,diagnostics},null,2));
  await page.screenshot({path:"artifacts/batch/failure.png",fullPage:true}).catch(()=>{});
  throw error;
}finally{await browser.close();}
