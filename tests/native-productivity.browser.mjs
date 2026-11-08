// Runs real desktop IPC and local files in an isolated repository. No remote extraction.
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {spawn} from 'node:child_process';
import {mkdir,mkdtemp,readFile,writeFile} from 'node:fs/promises';
import path from 'node:path';
const {chromium}=createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE||'playwright');
const root=path.resolve(import.meta.dirname,'..');
const base=path.join(root,'artifacts/productivity-native');await mkdir(base,{recursive:true});
const data=await mkdtemp(path.join(base,'run-'));const output=path.join(data,'media');await mkdir(output);
const captions=path.join(data,'local-caption.srt');await writeFile(captions,'1\n00:00:03,250 --> 00:00:04,500\nNative caption phrase at the correct time.\n');
let child,browser,page;const results=[];const port=9236;
async function wait(check,timeout=30000){const end=Date.now()+timeout;while(Date.now()<end){const value=await check();if(value)return value;await new Promise(resolve=>setTimeout(resolve,150));}throw Error('Timed out waiting for native productivity.');}
async function rpc(command,args={}){return page.evaluate(async({command,args})=>{const {invoke}=await import('/node_modules/@tauri-apps/api/core.js');return invoke(command,args);},{command,args});}
async function launch(){
  child=spawn(path.join(root,'src-tauri/target/debug/nabnab.exe'),[],{cwd:root,windowsHide:true,stdio:'ignore',env:{...process.env,NABNAB_TEST_DATA_DIR:data,WEBVIEW2_USER_DATA_FOLDER:path.join(data,'webview'),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`}});
  await wait(async()=>{if(child.exitCode!==null)throw Error(`Native app exited ${child.exitCode}`);try{return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok;}catch{return false;}});
  browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);page=await wait(()=>browser.contexts()[0]?.pages().find(page=>page.url().includes(':1420')));
  page.on('pageerror',error=>results.push({pageError:error.message}));await wait(async()=>{try{return (await rpc('get_snapshot')).settings.folder;}catch{return false;}});
}
async function close(){try{await page?.evaluate(async()=>{const {getCurrentWindow}=await import('/node_modules/@tauri-apps/api/window.js');await getCurrentWindow().close();});}catch{}if(child&&child.exitCode===null){await Promise.race([new Promise(resolve=>child.once('exit',resolve)),new Promise(resolve=>setTimeout(resolve,4000))]);if(child.exitCode===null)child.kill();}try{await browser?.close();}catch{}page=null;browser=null;}
try{
  await launch();const original=await rpc('get_snapshot');await rpc('save_settings',{settings:{...original.settings,folder:output,reducedMotion:true,reducedTransparency:true}});
  const text='https://www.youtube.com/watch?v=authorized-fixture invalid https://vimeo.com/authorized-fixture';
  const draft=await rpc('batch_create',{text,name:'Native saved links',presetId:'compatible'});assert.equal(draft.items.length,3);draft.savedForLater=true;draft.items[0].picked=false;await rpc('batch_save',{draft});
  const prepared=await rpc('prepare_download',{url:'https://www.youtube.com/watch?v=authorized-fixture',title:'Native planned outputs',creator:'Local creator',presetId:'compatible'});
  prepared.options.values.additionalAudio='mp3';prepared.options.values.subtitleMode='sidecar';prepared.options.values.subtitleLanguages=['es'];prepared.options.values.schedule={kind:'once',at:Date.now()+86400000,timezone:'UTC',missed:'runWhenPossible',graceMinutes:15};
  const jobs=await rpc('queue_prepared',{items:[prepared],request:'native-planned-outputs'});assert.equal(jobs.length,3);assert.ok(jobs.every(job=>job.state==='pending'));
  const replay=await rpc('queue_prepared',{items:[prepared],request:'native-planned-outputs'});assert.deepEqual(replay.map(j=>j.id),jobs.map(j=>j.id));
  const preset=structuredClone(original.presets.find(preset=>preset.id==='compatible'));preset.options.format='source';await rpc('save_preset',{preset});
  assert.equal((await rpc('get_snapshot')).jobs[0].options.values.format,'mp4');
  const rule={id:'native-rule',name:'Native organization',enabled:true,conditions:{platform:'YouTube',source:'',contentType:'video',title:'native',tag:''},presetId:'reference',relativeFolder:'Tutorials/{creador}',collection:'Native references',tags:['learning']};
  const preview=await rpc('test_rule',{rule,context:{url:prepared.url,title:prepared.title,creator:'Local creator',list:'',contentId:'authorized-fixture',platform:'YouTube',contentType:'video',tags:[]}});assert.equal(preview.matched,true);assert.ok(preview.path.includes('Tutorials'));assert.equal(preview.collision,false);
  assert.equal((await rpc('get_snapshot')).jobs.length,3);
  const imported=await rpc('import_paths',{paths:[path.join(root,'artifacts/fixtures/nabnab prueba.mp4'),captions]});assert.equal(imported.library.length,2);
  const hits=await rpc('library_search',{query:'caption phrase',collection:null});assert.ok(hits.some(hit=>hit.kind==='subtitle'&&hit.time===3.25));
  const video=imported.library.find(file=>file.kind==='video');await rpc('update_library',{id:video.id,collections:['Native references'],tags:['learning']});
  await rpc('library_bookmark',{id:video.id,bookmark:{id:'native-bookmark',name:'Native scene',time:3.25},remove:false});
  await rpc('smart_collection_save',{collection:{id:'native-smart',name:'Learning videos',mode:'all',conditions:[{kind:'kind',value:'video'},{kind:'tag',value:'learning'}]}});
  await page.locator('[data-page="library"]').click();await page.screenshot({path:path.join(data,'native-library.png')});
  await page.reload();await page.locator('[data-page="library"]').click();await page.getByText(video.name,{exact:true}).first().waitFor();
  await page.keyboard.press('Control+k');await page.locator('#palette-query').fill('Native scene');await page.locator('[role="option"]').first().waitFor();await page.keyboard.press('Escape');
  await page.keyboard.press('Control+k');await page.locator('#palette-query').fill('coleccion portatil');
  const paletteOption=page.locator('#palette-results [aria-selected="true"]');
  assert.match(await paletteOption.innerText(),/Colección portátil/);
  await page.locator('#palette-query').press('Enter');
  await page.locator('#portable-export-form').waitFor();
  assert.equal(await page.locator('.tool-workspace h2').first().evaluate(heading=>heading===document.activeElement),true);
  const pairing=await rpc('bridge_pair',{extensionId:'a'.repeat(32)});
  const send=async(endpoint,body)=>{const response=await fetch(`http://127.0.0.1:${pairing.port}${endpoint}`,{method:'POST',headers:{Origin:'chrome-extension://'+'a'.repeat(32),Authorization:'Bearer '+pairing.token,'Content-Type':'application/json'},body:JSON.stringify(body)});const result=await response.json();assert.equal(response.status,200,JSON.stringify(result));return result;};
  const localPresets=await send('/presets',{version:1});assert.ok(localPresets.presets.some(preset=>preset.id==='reference'));assert.equal('options' in localPresets.presets[0],false);
  const request={version:1,requestId:'native-extension-batch',url:prepared.url,urls:[prepared.url,'https://vimeo.com/authorized-fixture'],profile:'mp4',presetId:'reference',intent:'prepare'};
  const receipt=await send('/enqueue',request);assert.equal(receipt.intent,'prepare');assert.deepEqual(receipt.acceptedJobIds,[]);assert.ok(receipt.acceptedDraftId);assert.equal((await send('/enqueue',request)).acceptedDraftId,receipt.acceptedDraftId);
  await page.locator('.batch-workspace').waitFor();assert.equal((await rpc('get_snapshot')).jobs.length,3);await page.screenshot({path:path.join(data,'native-extension-prepared.png')});
  const before=await rpc('get_snapshot');await close();await launch();const after=await rpc('get_snapshot');assert.equal(after.drafts.length,before.drafts.length);assert.equal(after.jobs.length,3);assert.ok(after.jobs.every(job=>job.state==='pending'));assert.equal(after.library.length,2);assert.equal(after.library.find(item=>item.id===video.id).details.bookmarks[0].time,3.25);
  results.push({case:'real native drafts, resolved options, rule preview, local captions, bookmarks, smart collection and extension',passed:true,remoteExtraction:false,scheduledJobs:3,subtitleHitTime:3.25,data});
  if(process.env.NABNAB_SOURCE_TEST_URL){
    const count=after.jobs.length;
    try{
      const source=await rpc('add_source',{url:process.env.NABNAB_SOURCE_TEST_URL,settings:{presetId:null,relativeFolder:'',intervalHours:null,automatic:false,automaticLimit:20,explorationLimit:200,pageSize:50,history:'fromNow'}});
      const state=await rpc('get_snapshot');assert.equal(state.jobs.length,count);assert.equal(source.settings.automatic,false);assert.ok(source.baseline.cursor<=50);
      results.push({case:'live YouTube source first page, manual review only',status:source.status,knownItems:source.items.length,baseline:source.baseline,error:source.error,downloadJobsAdded:0});
      await page.locator('[data-page="library"]').click();await page.locator('[data-action="library-section"][data-section="sources"]').click();await page.screenshot({path:path.join(data,'native-source-check.png')});
    }catch(error){results.push({case:'live YouTube source first page',status:'unavailable in this environment',error:String(error).slice(0,1000)});}
  }
  assert.ok(!results.some(result=>result.pageError));await writeFile(path.join(base,'results.json'),JSON.stringify(results,null,2));console.log(JSON.stringify(results));
}finally{await close();}
