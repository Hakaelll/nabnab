// Real rendered UI with explicit IPC fixtures. OCR engines and native PDF operations are tested separately.
import assert from "node:assert/strict";
import {mkdir, writeFile} from "node:fs/promises";
import {createRequire} from "node:module";

const {chromium}=createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE||"playwright");
const base=process.env.NABNAB_UI_URL||"http://127.0.0.1:1420/";
const directory="artifacts/documents-advanced";
await mkdir(directory,{recursive:true});
await writeFile(`${directory}/result.json`,JSON.stringify({status:"running",evidence:"Frontend IPC fixtures only"},null,2));
const setup=String.raw`
  import {mockIPC,mockWindows} from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri=true;mockWindows("main");
  const copy=value=>structuredClone(value);
  const settings={folder:"C:/Fixture output/Documents",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
  const details=()=>({contentId:null,creator:null,platform:null,sourceIds:[],chapters:[],tracks:[],playbackPosition:0,favorite:false,bookmarks:[],clip:null,ocrRef:null});
  const file=(id,name,kind,extra={})=>({id,name,path:"C:/Fixture originals/"+name,kind,size:184320,duration:null,width:600,height:800,videoCodec:null,audioCodec:null,container:null,addedAt:Date.now(),source:null,parentId:null,collections:["Documentos de prueba"],tags:[],missing:false,details:{...details(),...extra}});
  const state={settings,jobs:[],library:[file("pdf","Informe escaneado.pdf","document",{ocrRef:"ocr:fixture-version"}),file("image","Recorte de prueba.png","image")],collections:["Documentos de prueba"],sources:[],smartCollections:[],drafts:[],presets:[],rules:[]};
  const canvas=document.createElement("canvas");canvas.width=600;canvas.height=800;const ctx=canvas.getContext("2d");
  ctx.fillStyle="#fff";ctx.fillRect(0,0,600,800);ctx.fillStyle="#222";ctx.font="bold 26px Arial";ctx.fillText("Documento de prueba",64,90);ctx.font="20px Arial";ctx.fillText("Hola mundo documento local.",64,200);ctx.font="14px Arial";ctx.fillStyle="#777";ctx.fillText("Fixture de interfaz. No es una ejecución del motor.",64,740);
  const dataUrl=canvas.toDataURL("image/png"),png=Array.from(atob(dataUrl.split(",")[1]),ch=>ch.charCodeAt(0));
  const ocr={schemaVersion:1,sourceFingerprint:"a".repeat(64),languages:["spa"],pages:[
    {page:1,width:0,height:0,text:"Texto original útil que ya estaba en la primera página.",origin:"original",words:[],region:null,corrected:false},
    {page:2,width:600,height:800,text:"Hola mundo documento local.",origin:"ocr",words:[{text:"Hola",x:64,y:178,width:45,height:24,confidence:96},{text:"mundo",x:116,y:178,width:65,height:24,confidence:95},{text:"documento",x:187,y:178,width:102,height:24,confidence:93},{text:"local.",x:296,y:178,width:50,height:24,confidence:94}],region:null,corrected:false}
  ]};
  const fixture=window.documentsFixture={state,calls:[],unexpected:[],status:{installed:false,enabled:false},models:[{id:"spa",name:"Español",bytes:2294433,license:"Apache-2.0",source:"https://fixture.invalid/spa",installed:false},{id:"eng",name:"English",bytes:4113088,license:"Apache-2.0",source:"https://fixture.invalid/eng",installed:false}],modelConsent:false,ocr,pending:null,slowPreview:false,previews:new Map(),clipboard:[]};
  Object.defineProperty(navigator,"clipboard",{configurable:true,value:{writeText:async text=>fixture.clipboard.push(text)}});
  const job=(kind,args)=>{const result={id:"fixture-job-"+state.jobs.length,runId:0,title:"Operación documental de prueba",createdAt:Date.now(),updatedAt:Date.now(),state:"pending",source:{kind,libraryId:args.id,options:copy(args.options??{})},folder:settings.folder,provider:"Local",thumbnail:null,progress:null,outputs:[],error:null,attempt:0,priority:"normal",schedule:{kind:"immediate"},manualOrder:0,groupId:null,options:null,contentId:null,sourceId:null};state.jobs.push(result);return [copy(result)];};
  mockIPC(async(cmd,args={})=>{
    fixture.calls.push({cmd,args:copy(args)});
    if(cmd==="startup")return {...copy(state),folder:settings.folder,enginesReady:true,engineError:null};
    if(cmd==="get_snapshot")return copy(state);
    if(cmd==="local_timezone")return "Europe/Madrid";
    if(cmd==="plugin:window|is_maximized")return false;
    if(cmd.startsWith("plugin:"))return null;
    if(cmd==="media_edit_capabilities")return {burnSubtitles:false};
    if(cmd==="ocr_status")return copy(fixture.status);
    if(cmd==="ocr_models")return copy(fixture.models);
    if(cmd==="install_ocr_model"){if(!fixture.modelConsent)throw Error("OCR language download was cancelled.");fixture.models.find(model=>model.id===args.id).installed=true;return;}
    if(cmd==="queue_ocr")return job("ocr",args);
    if(cmd==="ocr_get")return copy(ocr);
    if(cmd==="ocr_preview")return {png};
    if(cmd==="ocr_correct_page"){const page=ocr.pages.find(page=>page.page===args.page&&page.origin==="ocr");if(!page)throw Error("Choose an OCR-generated page.");page.text=args.text;page.corrected=true;page.words=[];return copy(ocr);}
    if(cmd==="ocr_remove_generated"){ocr.pages=ocr.pages.filter(page=>page.origin!=="ocr");return copy(ocr);}
    if(cmd==="ocr_search")return ocr.pages.filter(page=>page.text.toLowerCase().includes(args.query.toLowerCase())).map(page=>({page:page.page,excerpt:page.text,origin:page.origin,boxes:page.corrected?[]:page.words.slice(0,2).map(({x,y,width,height})=>({x,y,width,height}))}));
    if(cmd==="ocr_save_text")return job("ocrText",args);
    if(cmd==="document_info")return {pages:2,copy:true,assemble:true,print:true};
    if(cmd==="document_page")return {page:args.page,pages:2,png,text:args.page===1?ocr.pages[0]?.text??"":"",textAvailable:args.page===1};
    if(cmd==="library_playback"){const file=state.library.find(file=>file.id===args.id);file.details.playbackPosition=args.position;return;}
    if(cmd==="thumbnail")return dataUrl;
    if(cmd==="document_edit_info")return {pages:2,width:612,height:792,forms:true,signatures:true,edit:true,bookmarks:[{title:"Sección de prueba",page:1,level:1}],bookmarksTruncated:false};
    if(cmd==="document_edit_preview"){
      if(fixture.slowPreview)return new Promise((resolve,reject)=>{fixture.pending={requestId:args.requestId,reject};});
      const previewId="fixture-preview-"+fixture.previews.size;fixture.previews.set(previewId,{id:args.id,options:copy(args.options)});
      return {previewId,page:args.page,pages:2,png,beforePng:png,inputBytes:184320,outputBytes:141312,warnings:["Vista previa de interfaz simulada. La operación nativa se prueba por separado."]};
    }
    if(cmd==="cancel_document_edit_preview"){if(fixture.pending?.requestId===args.requestId){fixture.pending.reject(Error("Cancelled."));fixture.pending=null;}return;}
    if(cmd==="queue_document_edit"){
      const approved=fixture.previews.get(args.previewId);if(!args.consent||!approved||approved.id!==args.id||JSON.stringify(approved.options)!==JSON.stringify(args.options))throw Error("Preview does not match.");
      fixture.previews.delete(args.previewId);return job("documentEdit",args);
    }
    fixture.unexpected.push({cmd,args});throw Error("Unhandled document UI fixture IPC: "+cmd);
  },{shouldMockEvents:true});
  await import("/src/main.ts");
`;

const browser=await chromium.launch({headless:true,...(process.env.EDGE_EXECUTABLE?{executablePath:process.env.EDGE_EXECUTABLE}:{channel:"msedge"})});
const results=[];
try{
  for(const [width,height] of [[1180,780],[900,620]]){
    const page=await browser.newPage({viewport:{width,height}}),errors=[];
    await page.context().grantPermissions(["local-network-access"],{origin:base});
    page.on("pageerror",error=>errors.push(error.message));
    page.on("console",message=>{if(message.type()==="error")errors.push(message.text());});
    await page.route(base,async route=>{const response=await route.fetch(),html=await response.text(),entry=/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/;assert.match(html,entry);await route.fulfill({response,body:html.replace(entry,'<script type="module">'+setup+'</script>')});});
    await page.route("http://nabmedia.localhost/**",async route=>{await route.fulfill({status:200,contentType:"image/png",body:Buffer.from(await page.evaluate(()=>{const canvas=document.createElement("canvas");canvas.width=600;canvas.height=800;return canvas.toDataURL().split(",")[1];}),"base64")});});
    const calls=cmd=>page.evaluate(cmd=>window.documentsFixture.calls.filter(call=>call.cmd===cmd),cmd);
    const last=async cmd=>(await calls(cmd)).at(-1)?.args;
    const blur=()=>page.locator(".ocr-heading h2").click();
    async function assertNoOverflow(){
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1),false,`${width}px global overflow`);
      assert.deepEqual(await page.evaluate(()=>[...document.querySelectorAll('#ocr-form,.pdf-edit-review,.ocr-review')].filter(element=>element.clientWidth>0&&element.scrollWidth>element.clientWidth+1).map(element=>element.id||element.className)),[],`${width}px document panel overflow`);
    }
    async function screenshot(name,locator){
      await locator.evaluate(element=>element.scrollIntoView({block:"start",inline:"nearest"}));
      await page.evaluate(async()=>{await document.fonts.ready;await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));});
      await page.screenshot({path:`${directory}/${name}-${width}.png`,animations:"disabled"});
      await assertNoOverflow();
    }
    try{
      await page.goto(base);await page.locator("#url").waitFor();
      await page.locator('[data-page="tools"]').click();
      assert.equal(await page.locator("#ocr-form").count(),0,"Advanced forms are opened from the tools directory");
      await page.locator('[data-action="tool-module"][data-id="ocr"]').click();await page.locator("#ocr-form").waitFor();
      await page.getByText("Componente ausente",{exact:true}).waitFor();
      assert.equal(await page.locator('#ocr-form button[type="submit"]').isDisabled(),true);
      assert.equal((await calls("install_ocr_model")).length,0,"No model installs during startup");
      await screenshot("ocr-missing",page.locator(".ocr-tools"));
      await page.evaluate(()=>{window.documentsFixture.status={installed:true,enabled:false};});
      await page.locator('[data-action="ocr-refresh"]').click();await page.getByText("OCR desactivado",{exact:true}).waitFor();
      await page.evaluate(()=>{window.documentsFixture.status.enabled=true;});await page.locator('[data-action="ocr-refresh"]').click();
      await page.locator('#ocr-input').selectOption("image");
      assert.equal(await page.locator('#ocr-form button[type="submit"]').isDisabled(),true,"Missing language prevents queue");
      await page.locator('#ocr-input').focus();await page.keyboard.press("Tab");
      assert.equal(await page.evaluate(()=>document.activeElement?.getAttribute("data-ocr-language")),"spa","Language controls follow source in keyboard order");
      await page.keyboard.press("Space");assert.equal(await page.locator('[data-ocr-language="spa"]').isChecked(),false);
      await page.keyboard.press("Space");assert.equal(await page.locator('[data-ocr-language="spa"]').isChecked(),true);
      await page.locator('[data-action="ocr-install-model"][data-id="spa"]').click();
      await page.getByText("OCR language download was cancelled.",{exact:true}).waitFor();
      assert.equal((await calls("queue_ocr")).length,0);
      await page.evaluate(()=>{window.documentsFixture.modelConsent=true;});
      await page.locator('[data-action="ocr-install-model"][data-id="spa"]').click();
      await page.waitForFunction(()=>window.documentsFixture.models.find(model=>model.id==="spa").installed);
      await page.locator('#ocr-crop').check();
      for(const [key,value] of Object.entries({x:15,y:20,width:200,height:100})){await page.locator('#ocr-region-'+key).fill(String(value));await blur();}
      await page.locator('#ocr-form button[type="submit"]').click();await page.waitForFunction(()=>window.documentsFixture.calls.some(call=>call.cmd==="queue_ocr"));
      assert.deepEqual((await last("queue_ocr")).options,{languages:["spa"],pages:"1",dpi:200,region:{x:15,y:20,width:200,height:100},output:"text",acknowledgeStructureChanges:false});
      await page.locator('#ocr-input').selectOption("pdf");await page.locator('#ocr-output').selectOption("searchablePdf");
      await page.locator('#ocr-pages').fill("1-2");await blur();await page.locator('#ocr-dpi').selectOption("300");
      assert.equal(await page.locator('#ocr-acknowledge').isChecked(),false);
      await page.locator('#ocr-form button[type="submit"]').click();assert.equal((await calls("queue_ocr")).length,1,"PDF acknowledgement must gate submission");
      await page.locator('#ocr-acknowledge').check();await page.locator('#ocr-form button[type="submit"]').click();await page.waitForFunction(()=>window.documentsFixture.calls.filter(call=>call.cmd==="queue_ocr").length===2);
      assert.deepEqual((await last("queue_ocr")).options,{languages:["spa"],pages:"1-2",dpi:300,region:null,output:"searchablePdf",acknowledgeStructureChanges:true});
      await screenshot("ocr-configured",page.locator("#ocr-form"));

      await page.locator('[data-action="tool-module"][data-id="pdfEdits"]').click();
      assert.equal(await page.locator("#ocr-form").count(),0,"Changing module removes the previous advanced form");
      await page.locator('#document-edits > summary').click();await page.locator('#pdf-edit-input').selectOption("pdf");
      await page.locator('#pdf-edit-quality').waitFor();
      assert.equal(await page.locator('#document-edit-form button[type="submit"]').isDisabled(),true);
      assert.equal(await page.locator('#pdf-edit-consent').isDisabled(),true);
      await page.locator('[data-action="pdf-edit-preview"]').click();await page.locator('.pdf-edit-images').waitFor();
      assert.equal(await page.locator('#document-edit-form button[type="submit"]').isDisabled(),true,"Preview does not grant consent");
      await page.locator('#pdf-edit-consent').check();assert.equal(await page.locator('#document-edit-form button[type="submit"]').isEnabled(),true);
      await page.locator('#pdf-edit-quality').selectOption("small");
      assert.equal(await page.locator('.pdf-edit-images').count(),0,"Changing parameters removes stale preview");
      assert.equal(await page.locator('#pdf-edit-consent').isChecked(),false);assert.equal(await page.locator('#document-edit-form button[type="submit"]').isDisabled(),true);
      await page.locator('[data-action="pdf-edit-preview"]').click();await page.locator('.pdf-edit-images').waitFor();
      await page.locator('#pdf-edit-consent').check();await screenshot("pdf-preview",page.locator(".pdf-edit-preview-heading"));
      const expectedPreview=(await last("document_edit_preview"));
      await page.locator('#document-edit-form button[type="submit"]').click();await page.waitForFunction(()=>window.documentsFixture.calls.some(call=>call.cmd==="queue_document_edit"));
      const queued=await last("queue_document_edit");assert.equal(queued.consent,true);assert.equal(queued.previewId,"fixture-preview-1");assert.deepEqual(queued.options,expectedPreview.options);
      assert.equal(await page.locator('#document-edit-form button[type="submit"]').isDisabled(),true,"Queue consumes the reviewed preview");
      await page.evaluate(()=>{window.documentsFixture.slowPreview=true;});await page.locator('[data-action="pdf-edit-preview"]').click();
      await page.waitForFunction(()=>window.documentsFixture.pending!==null);
      await page.locator('[data-action="pdf-edit-cancel"]').click();await page.locator('.pdf-edit-warning[role="alert"]').waitFor();
      assert.equal((await calls("queue_document_edit")).length,1);assert.equal((await last("cancel_document_edit_preview")).requestId,(await last("document_edit_preview")).requestId);

      await page.locator('[data-page="library"]').click();await page.locator('[data-action="inspect-media"][data-id="pdf"]').click();
      await page.locator('#ocr-review-page').waitFor();assert.equal(await page.locator('#ocr-corrected-text').getAttribute("readonly"),"");
      assert.equal(await page.locator('#ocr-corrected-text').inputValue(),"Texto original útil que ya estaba en la primera página.");
      await page.locator('#ocr-review-page').selectOption("2");await page.locator('#ocr-search').fill("Hola mundo");await page.locator('#ocr-search-form button').click();
      await page.waitForFunction(()=>document.querySelectorAll('.ocr-preview-layer > span').length===2);
      assert.equal((await last("ocr_search")).query,"Hola mundo");
      await screenshot("ocr-search",page.locator(".ocr-preview"));
      await page.locator('#ocr-corrected-text').fill("Hola mundo, documento corregido.");await page.locator('#ocr-correction-form button[type="submit"]').click();
      await page.waitForFunction(()=>window.documentsFixture.ocr.pages.find(page=>page.page===2).corrected);
      assert.equal((await last("ocr_correct_page")).text,"Hola mundo, documento corregido.");
      assert.equal(await page.locator('.ocr-preview-layer > span').count(),0,"Corrections must drop stale coordinate highlights");
      await page.locator('[data-action="ocr-copy"]').click();assert.equal(await page.evaluate(()=>window.documentsFixture.clipboard.at(-1)),"Hola mundo, documento corregido.");
      await page.locator('[data-action="ocr-save-text"]').click();await page.waitForFunction(()=>window.documentsFixture.calls.some(call=>call.cmd==="ocr_save_text"));
      assert.equal((await last("ocr_save_text")).id,"pdf");assert.ok((await last("ocr_save_text")).clientRequest);
      await page.locator('[data-action="ocr-remove-generated"]').click();await page.waitForFunction(()=>window.documentsFixture.ocr.pages.length===1);
      assert.equal(await page.locator('#ocr-corrected-text').inputValue(),"Texto original útil que ya estaba en la primera página.");assert.equal(await page.locator('[data-action="ocr-remove-generated"]').isDisabled(),true);
      assert.equal(await page.evaluate(()=>window.documentsFixture.state.library[0].path),"C:/Fixture originals/Informe escaneado.pdf");
      await assertNoOverflow();assert.deepEqual(errors,[]);assert.deepEqual(await page.evaluate(()=>window.documentsFixture.unexpected),[]);
      results.push({width,height,evidence:"Rendered frontend with IPC fixtures; native engines and permissions tested separately",checks:["component missing vs disabled","no implicit model install","keyboard order and checkbox activation","cancelled model consent leaves queue empty","selected region and PDF options","PDF acknowledgement","preview and consent gates","option changes invalidate preview","queued approval ID","preview cancellation","original text readonly","phrase word boxes","correction drops boxes","clipboard","save corrected result","remove generated index keeps original","no global or panel horizontal overflow"],pageErrors:errors});
    }catch(error){await page.screenshot({path:`${directory}/failure-${width}.png`,fullPage:true}).catch(()=>{});await writeFile(`${directory}/failure-${width}.json`,JSON.stringify({error:String(error),errors,calls:await page.evaluate(()=>window.documentsFixture?.calls??[])},null,2));await writeFile(`${directory}/result.json`,JSON.stringify({status:"failed",width,error:String(error),errors,evidence:"Frontend IPC fixtures only"},null,2));throw error;}
    finally{await page.close();}
  }
  await writeFile(`${directory}/result.json`,JSON.stringify(results,null,2));console.log("Advanced document UI fixtures passed at 1180px and 900px. Native evidence is separate.");
}finally{await browser.close();}
