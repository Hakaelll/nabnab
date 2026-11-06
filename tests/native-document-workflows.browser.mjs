// Native IPC, engines, queue and library in fresh app data. Inputs are generated fixtures; no accounts or providers.
import assert from "node:assert/strict";
import {spawn} from "node:child_process";
import {createHash} from "node:crypto";
import {mkdir,mkdtemp,readFile,writeFile} from "node:fs/promises";
import path from "node:path";
import {createRequire} from "node:module";

const root=path.resolve(import.meta.dirname,"..");
const directory=path.join(root,"artifacts/document-workflows");await mkdir(directory,{recursive:true});
const data=await mkdtemp(path.join(directory,"native-run-"));
const components=path.join(data,"components"),output=path.join(data,"output"),fixtures=path.join(data,"fixtures");
await Promise.all([components,output,fixtures].map(folder=>mkdir(folder)));
const port=Number(process.env.NABNAB_DOCUMENT_DEBUG_PORT||9237);
const {chromium}=createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE||"playwright");
const results=[];const errors=[];let child,browser,page;
const digest=bytes=>createHash("sha256").update(bytes).digest("hex");
async function wait(check,timeout=60000){const until=Date.now()+timeout;while(Date.now()<until){const result=await check();if(result)return result;await new Promise(resolve=>setTimeout(resolve,150));}throw Error("Timed out waiting for native document workflow.");}
async function run(executable,args,extraEnv={}){
  await new Promise((resolve,reject)=>{const process=spawn(executable,args,{cwd:root,windowsHide:true,env:{...globalThis.process.env,...extraEnv},stdio:["ignore","pipe","pipe"]});let log="";
    for(const stream of [process.stdout,process.stderr])stream.on("data",chunk=>{log=(log+chunk.toString()).slice(-16000);});
    process.once("error",reject);process.once("exit",code=>code===0?resolve():reject(Error(`${path.basename(executable)} exited ${code}: ${log}`)));
  });
}
async function rpc(command,args={}){return page.evaluate(async({command,args})=>{const {invoke}=await import("/node_modules/@tauri-apps/api/core.js");return invoke(command,args);},{command,args});}
async function completed(jobs){
  assert.ok(jobs.length);return wait(async()=>{const snapshot=await rpc("get_snapshot");const selected=jobs.map(job=>snapshot.jobs.find(candidate=>candidate.id===job.id));const failed=selected.find(job=>job&&["error","cancelled"].includes(job.state));if(failed)throw Error(JSON.stringify({job:failed.title,state:failed.state,error:failed.error}));return selected.every(job=>job?.state==="completed")?snapshot:false;},180000);
}
async function rejects(command,args,pattern){let caught;try{await rpc(command,args);}catch(error){caught=String(error);}assert.ok(caught,`${command} must reject the request`);assert.match(caught,pattern);return caught;}
async function refreshUi(){await page.reload();await page.locator('[data-page="library"]').waitFor();await wait(async()=>{try{return (await rpc("get_snapshot")).settings.folder===output;}catch{return false;}});}
async function screenshot(name,anchor){if(anchor)await anchor.evaluate(element=>element.scrollIntoView({block:"start"}));await page.screenshot({path:path.join(data,name),animations:"disabled"});}
async function close(){
  try{await page?.evaluate(async()=>{const {getCurrentWindow}=await import("/node_modules/@tauri-apps/api/window.js");await getCurrentWindow().close();});}catch{}
  if(child&&child.exitCode===null){await Promise.race([new Promise(resolve=>child.once("exit",resolve)),new Promise(resolve=>setTimeout(resolve,4000))]);if(child.exitCode===null)child.kill();}
  try{await browser?.close();}catch{}
}

try{
  // This ignored test verifies upstream archives and models. It is not a product installation or consent bypass.
  await run("cargo.exe",["test","--offline","--manifest-path","src-tauri/Cargo.toml","provision_native_document_workflow_components","--","--ignored","--nocapture"],{
    NABNAB_TEST_DOCUMENT_WORKFLOW_COMPONENTS:components,
    NABNAB_TEST_OCR_PACKAGES:path.join(root,".cache/ocr/packages"),
    NABNAB_TEST_TESSDATA:path.join(root,".cache/ocr/models"),
    NABNAB_TEST_MUPDF_ARCHIVE:path.join(root,"artifacts/component-verification/mupdf.zip"),
  });
  console.log("Verified optional engines provisioned in fresh native fixture data.");
  if(process.env.NABNAB_DOCUMENT_SKIP_BUILD!=="1")await run("cargo.exe",["build","--offline","--manifest-path","src-tauri/Cargo.toml"]);
  console.log("Native debug build ready.");
  const mutool=path.join(components,"documents-1.28.5/mutool.exe");
  const typed=path.join(fixtures,"typed.pdf"),firstPage=path.join(fixtures,"original-text.pdf"),image=path.join(fixtures,"scan.png"),input=path.join(fixtures,"Native mixed scan.pdf");
  const typedDriver=path.join(fixtures,"typed.js"),scanDriver=path.join(fixtures,"scan.js");
  await writeFile(typedDriver,"var w=new DocumentWriter(scriptArgs[0],'pdf','compress');var dev=w.beginPage([0,0,612,792]);var t=new Text();t.showString(new Font('Times-Roman'),[26,0,0,-26,72,120],scriptArgs[1],0);dev.fillText(t,Matrix.identity,ColorSpace.DeviceGray,[0],1);w.endPage();w.close();");
  await run(mutool,["run",typedDriver,typed,"Hola mundo documento local."]);await run(mutool,["draw","-q","-r","150","-o",image,typed,"1"]);await run(mutool,["run",typedDriver,firstPage,"Original useful text remains intact."]);
  await writeFile(scanDriver,"var d=new PDFDocument();d.graftPage(-1,Document.openDocument(scriptArgs[0]),0);var image=d.addImage(new Image(scriptArgs[1]));var page=d.addPage([0,0,612,792],0,{XObject:{Scan:image}},'q 612 0 0 792 0 0 cm /Scan Do Q');d.insertPage(-1,page);d.save(scriptArgs[2],'compress');");
  await run(mutool,["run",scanDriver,firstPage,image,input]);const originalHash=digest(await readFile(input));
  child=spawn(path.join(root,"src-tauri/target/debug/nabnab.exe"),[],{cwd:root,windowsHide:true,stdio:"ignore",env:{...process.env,NABNAB_TEST_DATA_DIR:data,WEBVIEW2_USER_DATA_FOLDER:path.join(data,"webview"),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`}});
  await wait(async()=>{if(child.exitCode!==null)throw Error(`Native debug app exited ${child.exitCode}`);try{return(await fetch(`http://127.0.0.1:${port}/json/version`)).ok;}catch{return false;}});
  browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);page=await wait(()=>browser.contexts()[0]?.pages().find(page=>/:1420/.test(page.url())));
  page.on("pageerror",error=>errors.push(error.message));
  await page.locator('[data-page="library"]').waitFor();await wait(async()=>{try{return(await rpc("get_snapshot")).settings.folder;}catch{return false;}});
  let snapshot=await rpc("get_snapshot");await rpc("save_settings",{settings:{...snapshot.settings,folder:output,reducedMotion:true,reducedTransparency:true,trayEnabled:false,closeToTray:false}});
  const modules=await rpc("optional_components");assert.ok(modules.find(module=>module.id==="ocr"&&module.installed&&module.enabled));assert.ok(modules.find(module=>module.id==="documents"&&module.installed&&module.enabled));assert.ok((await rpc("ocr_models")).every(model=>model.installed));
  snapshot=await rpc("import_paths",{paths:[input,image]});const source=snapshot.library.find(item=>item.path.toLowerCase().endsWith("native mixed scan.pdf")),sourceImage=snapshot.library.find(item=>item.kind==="image");assert.ok(source&&sourceImage);
  const ocrOptions={languages:["eng","spa"],pages:"1-2",dpi:150,region:null,output:"text",acknowledgeStructureChanges:false};
  const textJobs=await rpc("queue_ocr",{id:source.id,options:ocrOptions,clientRequest:"native-ocr-text"});snapshot=await completed(textJobs);
  let textItem=snapshot.library.find(item=>textJobs[0].outputs.includes(item.path));if(!textItem)textItem=snapshot.library.find(item=>item.parentId===source.id&&item.name.endsWith(".txt"));assert.ok(textItem?.details.ocrRef);
  const searchableOptions={...ocrOptions,output:"searchablePdf",acknowledgeStructureChanges:true};
  await rejects("queue_ocr",{id:source.id,options:{...searchableOptions,acknowledgeStructureChanges:false},clientRequest:"native-ocr-no-consent"},/acknowledge|signature|structure/i);
  const pdfJobs=await rpc("queue_ocr",{id:source.id,options:searchableOptions,clientRequest:"native-ocr-searchable"});snapshot=await completed(pdfJobs);
  const searchable=snapshot.library.find(item=>item.parentId===source.id&&item.name.endsWith(".pdf"));assert.ok(searchable?.details.ocrRef);
  let index=await rpc("ocr_get",{id:searchable.id});assert.equal(index.pages[0].origin,"original");assert.equal(index.pages[1].origin,"ocr");assert.match(index.pages[1].text,/hola mundo/i);assert.ok(index.pages[1].words.length);
  const hits=await rpc("library_search",{query:"Hola mundo",collection:null,smartId:null});assert.ok(hits.some(hit=>hit.fileId===source.id&&hit.kind==="ocr"&&hit.page===2));
  await refreshUi();await page.locator('[data-page="library"]').click();await page.locator('#library-search').fill("Hola mundo");
  await wait(async()=>await page.locator(`[data-action="inspect-media"][data-id="${sourceImage.id}"]`).count()===0);
  await page.locator(`[data-action="inspect-media"][data-id="${source.id}"]`).click();await page.locator('#ocr-review-page').waitFor();
  await wait(async()=>await page.locator('#ocr-review-page').inputValue()==="2");await wait(async()=>await page.locator('.ocr-preview-layer > span').count()>=2);
  await screenshot("native-ocr-search.png",page.locator('.ocr-preview'));
  await page.locator('#ocr-corrected-text').fill("Hola mundo, texto corregido de prueba.");await page.locator('#ocr-correction-form button[type="submit"]').click();
  await wait(async()=>{const current=await rpc("ocr_get",{id:source.id});return current.pages.find(page=>page.page===2)?.corrected;});
  assert.equal(await page.locator('.ocr-preview-layer > span').count(),0);const corrected=await rpc("ocr_get",{id:source.id});assert.equal(corrected.pages[1].text,"Hola mundo, texto corregido de prueba.");
  const correctedJobs=await rpc("ocr_save_text",{id:source.id,clientRequest:"native-ocr-corrected-export"});snapshot=await completed(correctedJobs);const correctedOutput=snapshot.library.find(item=>item.parentId===source.id&&item.id!==textItem.id&&item.name.endsWith(".txt"));assert.ok(correctedOutput);assert.match(await readFile(correctedOutput.path,"utf8"),/texto corregido de prueba/);
  await screenshot("native-ocr-corrected.png",page.locator('.ocr-review'));
  await rpc("ocr_remove_generated",{id:source.id});index=await rpc("ocr_get",{id:source.id});assert.equal(index.pages.length,1);assert.equal(index.pages[0].origin,"original");assert.equal(digest(await readFile(input)),originalHash);
  results.push({case:"real OCR text/searchable PDF/correction/index removal and global phrase search",passed:true,originalHashPreserved:true,textOutput:textItem.name,searchableOutput:searchable.name,correctedTextOutput:correctedOutput.name,sourceTextPageSkipped:true,ocrCoordinates:true,globalSearchOpenedPage:2});

  const pdfOptions={kind:"number",pages:"1-2",start:1,prefix:"Native page ",fontSize:12,position:"bottom"};
  const info=await rpc("document_edit_info",{id:source.id});assert.equal(info.pages,2);
  const preview=await rpc("document_edit_preview",{id:source.id,options:pdfOptions,page:2,requestId:70001});assert.ok(preview.previewId);assert.ok(preview.png.length&&preview.beforePng.length);
  await rejects("queue_document_edit",{id:source.id,options:pdfOptions,consent:false,previewId:preview.previewId,clientRequest:"native-pdf-no-consent"},/acknowledge|review|signature|structure/i);
  await rejects("queue_document_edit",{id:source.id,options:{...pdfOptions,start:10},consent:true,previewId:preview.previewId,clientRequest:"native-pdf-changed-options"},/preview|match/i);
  const editJobs=await rpc("queue_document_edit",{id:source.id,options:pdfOptions,consent:true,previewId:preview.previewId,clientRequest:"native-pdf-numbered"});snapshot=await completed(editJobs);
  const numbered=snapshot.library.find(item=>item.parentId===source.id&&item.id!==searchable.id&&item.name.endsWith(".pdf"));assert.ok(numbered);assert.notEqual(digest(await readFile(numbered.path)),originalHash);assert.equal(digest(await readFile(input)),originalHash);
  results.push({case:"native PDF preview approval bound to options and consent, then queued numbered copy",passed:true,originalHashPreserved:true,output:numbered.name});

  const extensionId="b".repeat(32),pairing=await rpc("bridge_pair",{extensionId});
  const resource=await page.evaluate(()=>{const canvas=document.createElement("canvas");canvas.width=100;canvas.height=50;const context=canvas.getContext("2d");context.fillStyle="#fff";context.fillRect(0,0,100,50);context.fillStyle="#222";context.fillText("Native fixture",4,25);return canvas.toDataURL("image/png");});
  const capture={schemaVersion:1,mode:"reading",title:"Native offline article",author:"Local fixture author",url:"https://example.org/native-article?access_token=fixture-secret&utm_source=fixture",publishedAt:"2026-11-01",archivedAt:"2026-11-06T12:00:00.123Z",blocks:[{kind:"heading",text:"Offline article fixture",level:2},{kind:"paragraph",text:"A native archived phrase remains readable offline."},{kind:"image",text:"Local PNG fixture",resourceId:"image-0"}],resources:[{id:"image-0",dataUrl:resource,width:100,height:50}],warnings:["Generated local fixture, not content extracted from a real website."],selectionContext:null};
  async function archive(requestId,capture){const response=await fetch(`http://127.0.0.1:${pairing.port}/archive`,{method:"POST",headers:{Origin:"chrome-extension://"+extensionId,Authorization:"Bearer "+pairing.token,"Content-Type":"application/json"},body:JSON.stringify({version:1,requestId,capture,format:"html",collection:"Native archive",tags:["native-fixture"]})});const body=await response.json();assert.equal(response.status,200,JSON.stringify(body));return body;}
  const receipt=await archive("native-article-v1",capture);snapshot=await completed(receipt.acceptedJobIds.map(id=>({id})));
  const article=snapshot.library.find(item=>item.kind==="article"&&item.name.endsWith(".html"));assert.ok(article?.details.webArchiveRef);
  const archived=await rpc("archive_document",{id:article.id});assert.equal(archived.url,"https://example.org/native-article");assert.equal(archived.resources.length,1);assert.equal(archived.archivedAt,capture.archivedAt);assert.equal(archived.publishedAt,capture.publishedAt);
  const html=await readFile(article.path,"utf8");assert.ok(html.includes("script-src 'none'"));assert.ok(!html.includes("fixture-secret"));assert.ok(html.includes(resource));
  await rejects("archive_add_annotation",{id:article.id,block:1,quote:"phrase",prefix:"wrong ",suffix:"",note:"invalid context"},/context|match/i);
  snapshot=await rpc("archive_add_annotation",{id:article.id,block:1,quote:"native archived phrase",prefix:"A ",suffix:" remains readable offline.",note:"Native version-bound note"});
  const noted=snapshot.library.find(item=>item.id===article.id);assert.equal(noted.details.articleAnnotations.length,1);assert.equal(noted.details.articleAnnotations[0].version,article.details.webArchiveRef);
  const epubJobs=await rpc("queue_article_export",{id:article.id,format:"epub",clientRequest:"native-article-epub"});snapshot=await completed(epubJobs);const epub=snapshot.library.find(item=>item.parentId===article.id&&item.name.endsWith(".epub"));assert.ok(epub);assert.equal((await readFile(epub.path)).subarray(0,2).toString(),"PK");
  const nextCapture={...capture,archivedAt:"2026-11-06T12:05:00.123Z",blocks:[...capture.blocks.slice(0,1),{kind:"paragraph",text:"A different native article version with changed content."},capture.blocks[2]]};
  const next=await archive("native-article-v2",nextCapture);snapshot=await completed(next.acceptedJobIds.map(id=>({id})));const second=snapshot.library.find(item=>item.kind==="article"&&item.name.endsWith(".html")&&item.id!==article.id);assert.ok(second);assert.notEqual(second.details.webArchiveRef,article.details.webArchiveRef);assert.equal(second.details.articleAnnotations.length,0);
  const articleHits=await rpc("library_search",{query:"native archived phrase",collection:null,smartId:null});assert.ok(articleHits.some(hit=>hit.fileId===article.id&&hit.kind==="article"&&hit.block===1));
  await refreshUi();await page.locator('[data-page="library"]').click();await page.locator('#library-search').fill("native archived phrase");await wait(async()=>await page.locator(`[data-action="inspect-media"][data-id="${sourceImage.id}"]`).count()===0);
  await page.locator(`[data-action="inspect-media"][data-id="${article.id}"]`).click();await page.locator('#article-dialog[open]').waitFor();await page.getByText("Native version-bound note",{exact:true}).waitFor();
  await screenshot("native-article-offline.png",page.locator('#article-dialog .article-heading'));
  await page.locator('[data-action="article-close"]').click();await page.locator('#library-search').fill("");await screenshot("native-library-results.png");
  const final=await rpc("get_snapshot");assert.ok(final.jobs.every(job=>job.state==="completed"));assert.equal(digest(await readFile(input)),originalHash);assert.deepEqual(errors,[]);
  results.push({case:"authenticated local archive bridge, offline reader, version-bound annotations, EPUB and full-text library search",passed:true,articleVersions:2,annotationMoved:false,epubOutput:epub.name,sourceParametersRemoved:true,pngEmbedded:true});
  results.push({evidence:"Real native IPC and local engines; synthetic original inputs; optional engines provisioned by an ignored test helper into fresh isolated app data, not installed through a product consent flow",data,completedJobs:final.jobs.length,libraryItems:final.library.length,pageErrors:errors});
  await writeFile(path.join(data,"results.json"),JSON.stringify(results,null,2));await writeFile(path.join(directory,"results.json"),JSON.stringify(results,null,2));console.log(JSON.stringify({passed:true,data,completedJobs:final.jobs.length,libraryItems:final.library.length}));
}catch(error){
  await page?.screenshot({path:path.join(data,"failure.png"),fullPage:true}).catch(()=>{});await writeFile(path.join(data,"failure.json"),JSON.stringify({error:String(error),pageErrors:errors,results},null,2));throw error;
}finally{await close();}
