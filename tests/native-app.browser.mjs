// Runs the real debug app, native IPC, scheduler and engines with isolated data.
import assert from "node:assert/strict";
import {createRequire} from "node:module";
import {spawn} from "node:child_process";
import {createHash} from "node:crypto";
import {mkdir,mkdtemp,readFile,writeFile,stat} from "node:fs/promises";
import path from "node:path";
const {chromium}=createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE||"playwright");
const root=path.resolve(import.meta.dirname,"..");
const executable=process.env.NABNAB_EXECUTABLE||path.join(root,"src-tauri/target/debug/nabnab.exe");
const fixture=path.join(root,"artifacts/fixtures/nabnab prueba.mp4");
const original=createHash("sha256").update(await readFile(fixture)).digest("hex");
await mkdir(path.join(root,"artifacts/native"),{recursive:true});
const data=await mkdtemp(path.join(root,"artifacts/native/run-"));
const output=path.join(data,"outputs");await mkdir(output);
const port=Number(process.env.NABNAB_DEBUG_PORT||9223);
let child,browser,page;
const results=[];
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
  const state=await rpc("get_snapshot");
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
  const cache=await rpc("cache_status");assert.ok(cache.files>=1&&cache.bytes>0);
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
  await rpc("bridge_revoke",{extensionId});
  results.push({case:"live bridge authentication, origin checks, receipt deduplication and revocation",status:"passed"});
  await rpc("remove_library",{id:input.id});assert.equal(createHash("sha256").update(await readFile(fixture)).digest("hex"),original);
  results.push({case:"cancel, restart, organization and index-only removal",status:"passed"});
  assert.equal(results.filter(r=>r.pageError).length,0);
  await writeFile(path.join(data,"results.json"),JSON.stringify({date:new Date().toISOString(),native:true,isolatedData:true,results},null,2));
  console.log(JSON.stringify({status:"passed",artifactDirectory:path.relative(root,data),checks:results.length}));
}finally{await close();}
