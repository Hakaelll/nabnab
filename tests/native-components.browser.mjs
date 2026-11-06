// Real optional-component installation in isolated app data; never authorizes an account.
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {spawn} from 'node:child_process';
import {copyFile,mkdir,mkdtemp,writeFile} from 'node:fs/promises';
import path from 'node:path';

const {chromium}=createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE||'playwright');
const root=path.resolve(import.meta.dirname,'..');
const base=path.join(root,'artifacts/components-native');await mkdir(base,{recursive:true});
const data=await mkdtemp(path.join(base,'run-'));
const port=9239;
let child,browser,page;
async function wait(check,timeout=30000){
  const end=Date.now()+timeout;
  while(Date.now()<end){const value=await check();if(value)return value;await new Promise(resolve=>setTimeout(resolve,150));}
  throw Error('Timed out waiting for the native component fixture.');
}
async function rpc(command,args={}){
  return page.evaluate(async({command,args})=>{
    const {invoke}=await import('/node_modules/@tauri-apps/api/core.js');return invoke(command,args);
  },{command,args});
}
try{
  child=spawn(path.join(root,'src-tauri/target/debug/nabnab.exe'),[],{
    cwd:root,windowsHide:true,stdio:'ignore',
    env:{...process.env,NABNAB_TEST_DATA_DIR:data,WEBVIEW2_USER_DATA_FOLDER:path.join(data,'webview'),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`},
  });
  await wait(async()=>{
    if(child.exitCode!==null)throw Error(`Native fixture exited ${child.exitCode}`);
    try{return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok;}catch{return false;}
  });
  browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  page=await wait(()=>browser.contexts()[0]?.pages().find(page=>page.url().includes(':1420')));
  await wait(async()=>{try{return (await rpc('get_snapshot')).settings.folder;}catch{return false;}});
  const before=(await rpc('optional_components')).find(item=>item.id==='telegram');
  assert.equal(before.installed,false);assert.equal(before.available,true);
  assert.equal(before.version,'1.8.69');assert.equal(before.size,13998492);
  await page.locator('[data-page="settings"]').click();
  await page.locator('[data-action="settings-section"][data-section="engines"]').click();
  const install=page.locator('[data-action="component-install"][data-id="telegram"]');
  await install.waitFor();assert.equal(await install.isEnabled(),true);
  await install.click();
  await wait(async()=>(await rpc('optional_components')).find(item=>item.id==='telegram')?.installed,180000);
  const toggle=page.locator('[data-action="component-toggle"][data-id="telegram"][data-enabled="false"]');
  await toggle.waitFor({timeout:30000});
  assert.deepEqual((await rpc('telegram_status')).accounts,[]);
  assert.equal((await rpc('telegram_status')).installed,true);
  await page.screenshot({path:path.join(data,'telegram-installed.png')});
  await toggle.click();
  await wait(async()=>(await rpc('optional_components')).find(item=>item.id==='telegram')?.enabled===false);
  assert.equal((await rpc('telegram_status')).installed,true);
  assert.deepEqual((await rpc('get_snapshot')).jobs,[]);
  if(process.env.NABNAB_TEST_SPEECH){
    await rpc('install_component',{id:'transcription'});
    if(process.env.NABNAB_TEST_MODEL_CACHE){
      const models=path.join(data,'components/models');await mkdir(models,{recursive:true});
      await copyFile(process.env.NABNAB_TEST_MODEL_CACHE,path.join(models,'ggml-tiny.bin'));
    }
    await rpc('install_transcription_model',{id:'tiny'});
    const output=path.join(data,'media');await mkdir(output);
    const state=await rpc('get_snapshot');await rpc('save_settings',{settings:{...state.settings,folder:output}});
    const imported=await rpc('import_paths',{paths:[path.resolve(process.env.NABNAB_TEST_SPEECH)]});
    const input=imported.library.find(item=>item.kind==='audio');assert.ok(input);
    const jobs=await rpc('queue_tool',{ids:[input.id],options:{kind:'transcribe',model:'tiny',language:process.env.NABNAB_TEST_SPEECH_LANGUAGE||'en',threads:2},clientRequest:'native-transcription-fixture'});
    await wait(async()=>{const job=(await rpc('get_snapshot')).jobs.find(item=>item.id===jobs[0].id);if(job?.state==='error')throw Error(JSON.stringify(job.error));return job?.state==='completed';},180000);
    const completed=await rpc('get_snapshot');
    const transcript=completed.library.find(item=>item.parentId===input.id&&item.details.tracks.some(track=>track.origin==='automatic'));
    assert.ok(transcript,'A generated transcript must be published in the common library');
    assert.ok(transcript.details.tracks[0].segments.length>0);
    const hits=await rpc('library_search',{query:process.env.NABNAB_TEST_SPEECH_QUERY||'local',collection:null});
    assert.ok(hits.some(hit=>hit.kind==='subtitle'),'Generated speech must be searchable by an actual recognized word');
    assert.ok(completed.library.some(item=>item.id===input.id));
    console.log(JSON.stringify({case:'real multilingual tiny transcription through queue and library search',passed:true,segments:transcript.details.tracks[0].segments,data}));
  }
  const result={case:'Telegram component installed through Settings, verified and disabled without account authorization',passed:true,data};
  await writeFile(path.join(base,'results.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
}finally{
  try{await page?.evaluate(async()=>{const {getCurrentWindow}=await import('/node_modules/@tauri-apps/api/window.js');await getCurrentWindow().close();});}catch{}
  if(child&&child.exitCode===null){await Promise.race([new Promise(resolve=>child.once('exit',resolve)),new Promise(resolve=>setTimeout(resolve,4000))]);if(child.exitCode===null)child.kill();}
  try{await browser?.close();}catch{}
}
