// Runs the real debug app, native IPC, scheduler and engines with isolated data.
import assert from "node:assert/strict";
import {createRequire} from "node:module";
import {spawn} from "node:child_process";
import {createHash} from "node:crypto";
import {mkdir,mkdtemp,readFile,writeFile,stat,utimes} from "node:fs/promises";
import path from "node:path";
const {chromium}=createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE||"playwright");
const root=path.resolve(import.meta.dirname,"..");
const executable=process.env.NABNAB_EXECUTABLE||path.join(root,"src-tauri/target/debug/nabnab.exe");
const fixture=path.join(root,"artifacts/fixtures/nabnab prueba.mp4");
const original=createHash("sha256").update(await readFile(fixture)).digest("hex");
await mkdir(path.join(root,"artifacts/native"),{recursive:true});
const data=await mkdtemp(path.join(root,"artifacts/native/run-"));
const output=path.join(data,"outputs");await mkdir(output);
const thumbnails=path.join(data,"thumbnail-cache");
await mkdir(thumbnails);
await writeFile(path.join(thumbnails,".nabnab-cache-v1"),"nabnab thumbnails v1");
await writeFile(path.join(thumbnails,"dead.jpg"),"unrelated");
const seededThumbnails=Array.from({length:514},(_,index)=>path.join(thumbnails,`abc-4-${index}.jpg`));
await Promise.all(seededThumbnails.map(file=>writeFile(file,"test")));
for(const [index,file] of seededThumbnails.slice(0,3).entries()){
  const old=new Date(Date.UTC(2000+index,0,1));
  await utimes(file,old,old);
}
const managed = path.join(data, "managed-engines");
const current = "set-100-1000";
const previous = "set-100-2000";
const obsolete = "set-100-3000";
const interrupted = "set-100-4000";
const unrelated = "set-100-5000";
await Promise.all([current, previous, obsolete, interrupted, unrelated].map(name => mkdir(path.join(managed, name), {recursive:true})));
const active = JSON.stringify({current, previous});
await writeFile(path.join(managed, "active.json"), active);
for (const name of [current, previous, obsolete]) await writeFile(path.join(managed, name, "deno.exe"), name);
await writeFile(path.join(managed, interrupted, "deno.zip"), "interrupted fixture");
await writeFile(path.join(managed, unrelated, "keep.txt"), "unrecognized fixture");
const port=Number(process.env.NABNAB_DEBUG_PORT||9223);
let child,browser,page;
const results=[];
let engines=[];
async function waitFor(check,timeout=30000){const end=Date.now()+timeout;while(Date.now()<end){const value=await check();if(value)return value;await new Promise(r=>setTimeout(r,150));}throw Error("Timed out waiting for the real app.");}
async function launch(){
  child=spawn(executable,[],{cwd:root,windowsHide:true,stdio:"ignore",env:{...process.env,NABNAB_TEST_DATA_DIR:data,WEBVIEW2_USER_DATA_FOLDER:path.join(data,"webview"),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`}});
  await waitFor(async()=>{if(child.exitCode!==null)throw Error(`Native app exited ${child.exitCode}`);try{return(await fetch(`http://127.0.0.1:${port}/json/version`)).ok;}catch{return false;}});
  browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  page=await waitFor(()=>browser.contexts()[0]?.pages().find(p=>/localhost:1420|127\.0\.0\.1:1420/.test(p.url())));
  page.on("pageerror",e=>results.push({pageError:e.message}));
  await page.waitForSelector('[data-page="library"]');
  await waitFor(async()=>{try{return(await rpc("get_snapshot")).settings.folder;}catch{return false;}});
}
async function rpc(cmd,args={}){return page.evaluate(async({cmd,args})=>{const{invoke}=await import('/node_modules/@tauri-apps/api/core.js');return invoke(cmd,args);},{cmd,args});}
async function close(){if(page&&!page.isClosed()){try{await page.evaluate(async()=>{const{getCurrentWindow}=await import('/node_modules/@tauri-apps/api/window.js');await getCurrentWindow().close();});}catch{}}
  if(child&&child.exitCode===null){await Promise.race([new Promise(r=>child.once("exit",r)),new Promise(r=>setTimeout(r,4000))]);if(child.exitCode===null)child.kill();}try{await browser?.close();}catch{}page=null;browser=null;
}
async function completed(id){return waitFor(async()=>{const snapshot=await rpc("get_snapshot");const job=snapshot.jobs.find(j=>j.id===id);if(job?.state==="error")throw Error(`${job.error?.code}: ${job.error?.technical}`);return job?.state==="completed"?job:false;},90000);}
try{
  await launch();
  assert.equal(await readFile(path.join(managed, "active.json"), "utf8"), active);
  for (const name of [current, previous]) assert.equal(await readFile(path.join(managed, name, "deno.exe"), "utf8"), name);
  assert.equal(await readFile(path.join(managed, unrelated, "keep.txt"), "utf8"), "unrecognized fixture");
  for (const name of [obsolete, interrupted]) await assert.rejects(stat(path.join(managed, name)), {code:"ENOENT"});
  results.push({case:"managed engine startup cleanup",status:"passed"});
  assert.deepEqual(await rpc("cache_status"),{files:512,bytes:2048,maxFiles:512,maxBytes:128*1024*1024});
  await assert.rejects(stat(seededThumbnails[0]),{code:"ENOENT"});
  await assert.rejects(stat(seededThumbnails[1]),{code:"ENOENT"});
  assert.equal(await readFile(seededThumbnails[2],"utf8"),"test");
  assert.equal(await readFile(path.join(thumbnails,"dead.jpg"),"utf8"),"unrelated");
  await page.screenshot({path:path.join(data,'home-native.png')});
  const state=await rpc("get_snapshot");
  engines=(await rpc('engine_status')).map(({name,version,status})=>({name,version,status}));
  assert.ok(engines.length===4&&engines.every(engine=>engine.status==='installed'), JSON.stringify(engines));
  assert.equal(state.settings.trayEnabled,false);assert.equal(state.settings.startWithWindows,false);
  await rpc("save_settings",{settings:{...state.settings,folder:output}});
  const imported=await rpc("import_paths",{paths:[fixture]});
  assert.equal(imported.library.length,1);const input=imported.library[0];
  assert.equal(input.videoCodec,"h264");assert.equal(input.audioCodec,"aac");assert.ok(input.duration>5.8&&input.duration<6.2);
  await page.locator('[data-page="library"]').click();
  await page.getByText(input.name,{exact:true}).first().click();
  await page.screenshot({path:path.join(data,"library-native.png")});
  results.push({case:"import and inspect",status:"passed",duration:input.duration,codecs:[input.videoCodec,input.audioCodec]});
  const operations=[
    {kind:"convert",container:"mkv",videoCodec:"copy",audioCodec:"copy"},
    {kind:"extractAudio",format:"mp3"},
    {kind:"trim",start:1,end:4,precise:true},
    {kind:"compress",targetMb:null,quality:"balanced"},
  ];
  for(const options of operations){
    const jobs=await rpc("queue_tool",{ids:[input.id],options,clientRequest:`native-${options.kind}`});
    const job=await completed(jobs[0].id);assert.equal(job.outputs.length,1);assert.ok((await stat(job.outputs[0])).size>0);
    const after=await rpc("get_snapshot");assert.ok(after.library.some(i=>i.parentId===input.id&&i.path===job.outputs[0]));
    const replay=await rpc("queue_tool",{ids:[input.id],options,clientRequest:`native-${options.kind}`});assert.equal(replay[0].id,job.id);
    results.push({case:`native queue ${options.kind}`,status:"passed",bytes:(await stat(job.outputs[0])).size});
  }
  const thumb=await rpc("thumbnail",{id:input.id});assert.match(thumb,/nabmedia\.localhost\/thumbnail\//);
  const cache=await rpc("cache_status");assert.equal(cache.files,cache.maxFiles);assert.ok(cache.bytes>2048&&cache.bytes<=cache.maxBytes);
  await assert.rejects(stat(seededThumbnails[2]),{code:"ENOENT"});
  const thumbnail=await page.evaluate(async url=>{
    const image=new Image();
    image.src=url;
    await image.decode();
    return {width:image.naturalWidth,height:image.naturalHeight};
  },thumb);
  assert.ok(thumbnail.width>0&&thumbnail.width<=320&&thumbnail.height>0&&thumbnail.height<=180);
  assert.equal(await readFile(path.join(thumbnails,"dead.jpg"),"utf8"),"unrelated");
  await page.locator('[data-page="settings"]').click();
  await page.locator('[data-action="settings-section"][data-section="sessions"]').click();
  const cacheSection=page.locator('section[aria-labelledby="thumbnail-cache-heading"]');
  await cacheSection.getByText(/512 miniaturas|512 thumbnails/).first().waitFor();
  await cacheSection.scrollIntoViewIfNeeded();
  await page.screenshot({path:path.join(data,"thumbnail-cache-native.png")});
  const clear=cacheSection.locator('[data-action="clear-cache"]');
  const confirmation=page.locator("#thumbnail-cache-confirm");
  await clear.click();
  const cancel=confirmation.locator("button[autofocus]");
  assert.equal(await cancel.evaluate(button=>document.activeElement===button),true);
  await page.screenshot({path:path.join(data,"thumbnail-cache-confirm.png")});
  await cancel.click();
  await confirmation.waitFor({state:"hidden"});
  assert.deepEqual(await rpc("cache_status"),cache);
  await clear.click();
  await confirmation.locator('[data-action="confirm-clear-cache"]').click();
  await waitFor(async()=>await clear.isDisabled());
  assert.deepEqual(await rpc("cache_status"),{...cache,files:0,bytes:0});
  assert.equal(await page.locator("#thumbnail-cache-heading").evaluate(heading=>document.activeElement===heading),true);
  assert.equal(await readFile(path.join(thumbnails,"dead.jpg"),"utf8"),"unrelated");
  results.push({case:"thumbnail budget at startup and after generation",status:"passed",bytes:cache.bytes,count:cache.files});
  results.push({case:"settings confirm or cancel generated thumbnail cleanup",status:"passed"});
  const probe=await page.evaluate(async id=>{const response=await fetch(`http://nabmedia.localhost/${id}`,{headers:{Range:"bytes=0-31"}});return{status:response.status,bytes:(await response.arrayBuffer()).byteLength};},input.id);
  assert.deepEqual(probe,{status:206,bytes:32});
  const denied=await page.evaluate(async()=>{try{return(await fetch('http://nabmedia.localhost/../../windows/win.ini')).status;}catch{return 0;}});assert.ok([0,404].includes(denied));
  results.push({case:"authorized playback ranges and thumbnail cache",status:"passed"});
  await rpc("update_library",{id:input.id,collections:["Pruebas"],tags:["sintético"]});
  const pending=await rpc("queue_tool",{ids:Array(20).fill(input.id),options:{kind:"compress",targetMb:null,quality:"balanced"},clientRequest:"native-recovery"});
  await rpc("job_action",{ids:pending.map(j=>j.id),action:"cancel"});
  await waitFor(async()=>{const s=await rpc("get_snapshot");return pending.every(j=>s.jobs.find(x=>x.id===j.id)?.state==="cancelled");});
  const before=await rpc("get_snapshot");await close();await launch();
  const recovered=await rpc("get_snapshot");assert.equal(recovered.library.length,before.library.length);
  assert.deepEqual(recovered.library.find(i=>i.id===input.id).tags,["sintético"]);
  assert.equal(recovered.jobs.filter(j=>j.state==="completed").length,before.jobs.filter(j=>j.state==="completed").length);
  assert.ok(recovered.jobs.every(j=>!["pending","analyzing","processing","downloading","verifying"].includes(j.state)));
  const extensionId="a".repeat(32);
  const pairing=await rpc("bridge_pair",{extensionId});
  const bridge=`http://127.0.0.1:${pairing.port}`;
  const headers={Origin:`chrome-extension://${extensionId}`,Authorization:`Bearer ${pairing.token}`};
  assert.equal((await fetch(bridge+"/health",{headers})).status,200);
  assert.equal((await fetch(bridge+"/health",{headers:{Origin:headers.Origin}})).status,401);
  assert.equal((await fetch(bridge+"/health",{headers:{...headers,Origin:headers.Origin+".attacker"}})).status,403);
  const message={version:1,requestId:"native-bridge-replay-1",url:"https://www.w3schools.com/html/mov_bbb.mp4",profile:"mp4"};
  const send=()=>fetch(bridge+"/enqueue",{method:"POST",headers:{...headers,"Content-Type":"application/json"},body:JSON.stringify(message)});
  const receipt=await(await send()).json();const replay=await(await send()).json();
  assert.deepEqual(replay.acceptedJobIds,receipt.acceptedJobIds);assert.equal(receipt.acceptedJobIds.length,1);
  await rpc("job_action",{ids:receipt.acceptedJobIds,action:"cancel"});
  const secondId="b".repeat(32);
  const second=await rpc("bridge_pair",{extensionId:secondId});
  await rpc("bridge_revoke",{extensionId});
  assert.equal((await fetch(bridge+"/health",{headers})).status,403);
  assert.equal((await fetch(bridge+"/health",{headers:{Origin:`chrome-extension://${secondId}`,Authorization:`Bearer ${second.token}`}})).status,200);
  await rpc("bridge_revoke",{extensionId:secondId});
  assert.equal((await rpc("bridge_status")).enabled,false);
  await assert.rejects(fetch(bridge+"/health",{headers}));
  results.push({case:"live bridge authentication, origin checks, receipt deduplication and revocation",status:"passed"});
  if(process.env.NABNAB_OPEN_SAMPLE==='1'){
    // Big Buck Bunny, Blender Foundation, CC BY 3.0. See docs/PLATFORMS.md.
    const url='https://www.w3schools.com/html/mov_bbb.mp4';
    for(const format of ['mp4','mp3']){
      let jobs;
      if(format==='mp4'){
        const previous=new Set((await rpc('get_snapshot')).jobs.map(job=>job.id));
        await page.locator('[data-page="home"]').click();
        await page.locator('#url').fill(url);
        await page.locator('#video-title').waitFor();
        assert.match(await page.locator('#video-title').innerText(),/HTTP media/);
        assert.equal((await rpc('get_snapshot')).jobs.length,previous.size,'Link analysis must not enqueue a download.');
        await page.locator('#output-format').selectOption('mp4');
        await page.screenshot({path:path.join(data,'direct-preview-native.png')});
        await page.locator('#download').click();
        jobs=await waitFor(async()=>{const added=(await rpc('get_snapshot')).jobs.filter(job=>!previous.has(job.id));return added.length?added:false;});
        assert.equal(jobs.length,1);
        await waitFor(()=>page.locator('#url').inputValue().then(value=>value===''));
      }else{
        jobs=await rpc('queue_download',{urls:[url],titles:['Big Buck Bunny · CC BY 3.0'],format,height:null,audioBitrate:128,subtitleLanguage:null,clientRequest:`open-sample-${format}`});
      }
      const job=await completed(jobs[0].id);const snapshot=await rpc('get_snapshot');
      assert.equal(job.provider,'HTTP');
      const item=snapshot.library.find(i=>i.path===job.outputs[0]);assert.ok(item);
      assert.equal(item.audioCodec,format==='mp3'?'mp3':'aac');
      if(format==='mp4')assert.equal(item.videoCodec,'h264');
      assert.equal(path.extname(item.path),'.'+format);assert.ok(item.duration>5);
      results.push({case:`open-license public HTTP ${format}`,status:'passed',bytes:item.size,videoCodec:item.videoCodec,audioCodec:item.audioCodec,duration:item.duration});
      if(format==='mp4'){
        const beforeCount=snapshot.library.length;
        const duplicate=await rpc('queue_download',{urls:[url],titles:['Big Buck Bunny · CC BY 3.0'],format,height:null,audioBitrate:null,subtitleLanguage:null,clientRequest:'open-sample-content-duplicate'});
        const reused=await completed(duplicate[0].id);assert.equal(reused.outputs[0],job.outputs[0]);assert.equal((await rpc('get_snapshot')).library.length,beforeCount);
        results.push({case:'byte-identical content and profile reuse an existing verified result',status:'passed'});
      }
    }
  }
  await rpc("remove_library",{id:input.id});assert.equal(createHash("sha256").update(await readFile(fixture)).digest("hex"),original);
  results.push({case:"cancel, restart, organization and index-only removal",status:"passed"});
  assert.equal(results.filter(r=>r.pageError).length,0);
  await writeFile(path.join(data,"results.json"),JSON.stringify({date:new Date().toISOString(),native:true,isolatedData:true,engines,openLicenseSample:process.env.NABNAB_OPEN_SAMPLE==='1'?{source:'https://www.w3schools.com/html/mov_bbb.mp4',license:'CC BY 3.0',attribution:'Blender Foundation / www.bigbuckbunny.org',licenseSource:'https://peach.blender.org/about/'}:null,results},null,2));
  console.log(JSON.stringify({status:"passed",artifactDirectory:path.relative(root,data),checks:results.length}));
}catch(error){
  const visible=page&&!page.isClosed()?await page.locator('body').innerText().catch(()=>null):null;
  if(page&&!page.isClosed())await page.screenshot({path:path.join(data,'failure.png')}).catch(()=>{});
  await writeFile(path.join(data,'failure.json'),JSON.stringify({date:new Date().toISOString(),native:true,results,visible,error:error instanceof Error?error.message:String(error)},null,2));throw error;
}finally{await close();}
