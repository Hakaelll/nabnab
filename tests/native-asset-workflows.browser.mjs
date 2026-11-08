// Real native IPC, image engines, common queue and filesystem operations on generated local fixtures.
import assert from "node:assert/strict";
import { openTool } from "./tool-navigation.mjs";
import {spawn} from "node:child_process";
import {createHash} from "node:crypto";
import {mkdir,mkdtemp,readFile,writeFile,copyFile} from "node:fs/promises";
import {deflateSync} from "node:zlib";
import path from "node:path";
import {createRequire} from "node:module";

const root=path.resolve(import.meta.dirname,"..");
const directory=path.join(root,"artifacts/asset-workflows");await mkdir(directory,{recursive:true});
const data=await mkdtemp(path.join(directory,"native-run-"));
const output=path.join(data,"output"),fixtures=path.join(data,"fixtures");await Promise.all([output,fixtures].map(p=>mkdir(p)));
const {chromium}=createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE||"playwright");
const port=Number(process.env.NABNAB_ASSET_DEBUG_PORT||9239);
const checks=[],errors=[];let child,browser,page;
const hash=bytes=>createHash("sha256").update(bytes).digest("hex");
function pathKey(value){if(value.startsWith("\\\\?\\"))value=value.slice(4);return value.replaceAll("\\","/").toLowerCase();}
function pngDimensions(bytes){assert.deepEqual(bytes.subarray(0,8),Buffer.from([137,80,78,71,13,10,26,10]));assert.equal(bytes.subarray(12,16).toString(),"IHDR");return [bytes.readUInt32BE(16),bytes.readUInt32BE(20)];}
function crc32(bytes){let crc=0xffffffff;for(const byte of bytes){crc^=byte;for(let bit=0;bit<8;bit++)crc=(crc>>>1)^((crc&1)?0xedb88320:0);}return (crc^0xffffffff)>>>0;}
function chunk(name,bytes){const tag=Buffer.from(name),size=Buffer.alloc(4),crc=Buffer.alloc(4);size.writeUInt32BE(bytes.length);crc.writeUInt32BE(crc32(Buffer.concat([tag,bytes])));return Buffer.concat([size,tag,bytes,crc]);}
function png(yOffset=0,level=6){const width=96,height=80;const scan=Buffer.alloc((width*4+1)*height);for(let y=0;y<height;y++)for(let x=0;x<width;x++){const i=y*(width*4+1)+1+x*4;scan[i]=(x*7+(y+yOffset)*3)%256;scan[i+1]=((x*11)^(y+yOffset)*5)%256;scan[i+2]=(y+yOffset)*13%256;scan[i+3]=x<8?0:255;}const header=Buffer.alloc(13);header.writeUInt32BE(width);header.writeUInt32BE(height,4);header[8]=8;header[9]=6;return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk("IHDR",header),chunk("IDAT",deflateSync(scan,{level})),chunk("IEND",Buffer.alloc(0))]);}
function zipEntries(bytes){let eocd=-1;for(let i=bytes.length-22;i>=Math.max(0,bytes.length-65557);i--)if(bytes.readUInt32LE(i)===0x06054b50){eocd=i;break;}assert.ok(eocd>=0);const entries=new Map();let cursor=bytes.readUInt32LE(eocd+16);for(let i=0;i<bytes.readUInt16LE(eocd+10);i++){assert.equal(bytes.readUInt32LE(cursor),0x02014b50);const method=bytes.readUInt16LE(cursor+10),size=bytes.readUInt32LE(cursor+24),nameLength=bytes.readUInt16LE(cursor+28),extra=bytes.readUInt16LE(cursor+30),comment=bytes.readUInt16LE(cursor+32),offset=bytes.readUInt32LE(cursor+42);assert.equal(method,0,"Generated archives use stored ZIP entries.");const name=bytes.subarray(cursor+46,cursor+46+nameLength).toString();const start=offset+30+bytes.readUInt16LE(offset+26)+bytes.readUInt16LE(offset+28);entries.set(name,bytes.subarray(start,start+size));cursor+=46+nameLength+extra+comment;}return entries;}
async function wait(check,timeout=60000){const until=Date.now()+timeout;while(Date.now()<until){const result=await check();if(result)return result;await new Promise(resolve=>setTimeout(resolve,150));}throw Error("Timed out waiting for native asset workflow.");}
async function rpc(command,args={}){return page.evaluate(async({command,args})=>{const {invoke}=await import("/node_modules/@tauri-apps/api/core.js");return invoke(command,args);},{command,args});}
async function rejected(command,args,pattern){let reason;try{await rpc(command,args);}catch(error){reason=String(error);}assert.match(reason||"No rejection",pattern);}
async function completed(jobs){assert.ok(jobs.length);return wait(async()=>{const snapshot=await rpc("get_snapshot");const states=jobs.map(job=>snapshot.jobs.find(j=>j.id===job.id));const failed=states.find(j=>j&&["error","cancelled"].includes(j.state));if(failed)throw Error(JSON.stringify(failed.error));return states.every(j=>j?.state==="completed")?snapshot:false;},180000);}
async function result(jobs){const snapshot=await completed(jobs);const job=snapshot.jobs.find(j=>j.id===jobs[0].id);return {snapshot,job,item:snapshot.library.find(i=>i.path===job.outputs[0])};}
try{
  const paths=["Capture A.png","Capture B.png","Same pixels.png","Exact copy.png"].map(name=>path.join(fixtures,name));
  await writeFile(paths[0],png());await writeFile(paths[1],png(60));await writeFile(paths[2],png(0,1));await copyFile(paths[0],paths[3]);
  const originals=await Promise.all(paths.map(async p=>[p,hash(await readFile(p))]));
  child=spawn(path.join(root,"src-tauri/target/debug/nabnab.exe"),[],{cwd:root,windowsHide:true,stdio:"ignore",env:{...process.env,NABNAB_TEST_DATA_DIR:data,WEBVIEW2_USER_DATA_FOLDER:path.join(data,"webview"),WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS:`--remote-debugging-port=${port}`}});
  await wait(async()=>{if(child.exitCode!==null)throw Error(`Native app exited ${child.exitCode}`);try{return(await fetch(`http://127.0.0.1:${port}/json/version`)).ok;}catch{return false;}});
  browser=await chromium.connectOverCDP(`http://127.0.0.1:${port}`);page=await wait(()=>browser.contexts()[0]?.pages().find(p=>/:1420/.test(p.url())));page.on("pageerror",e=>errors.push(e.message));
  await page.locator('[data-page="library"]').waitFor();await wait(async()=>{try{return(await rpc("get_snapshot")).settings.folder;}catch{return false;}});
  let snapshot=await rpc("get_snapshot");await rpc("save_settings",{settings:{...snapshot.settings,folder:output,reducedMotion:true,trayEnabled:false,closeToTray:false}});
  snapshot=await rpc("import_paths",{paths});const items=paths.map(p=>snapshot.library.find(i=>pathKey(i.path)===pathKey(p)));assert.ok(items.every(Boolean));const ids=items.map(i=>i.id);
  const exact=await rpc("file_analysis",{ids,options:{kind:"exact"},requestId:81001});assert.equal(exact.exactGroups.length,1);assert.deepEqual(new Set(exact.exactGroups[0].members),new Set([ids[0],ids[3]]));
  const similar=await rpc("file_analysis",{ids:[ids[0],ids[2]],options:{kind:"similar",maxDistance:0},requestId:81002});assert.equal(similar.similarPairs.length,1);assert.equal(similar.similarPairs[0].distance,0);assert.notEqual(originals[0][1],originals[2][1]);
  await rpc("file_organize_review",{ids:[ids[3]],collection:"Revisar copias"});snapshot=await rpc("get_snapshot");assert.ok(snapshot.library.find(i=>i.id===ids[3]).collections.includes("Revisar copias"));
  await completed(await rpc("queue_file_analysis",{ids,options:{kind:"exact"},clientRequest:"native-exact-report"}));
  checks.push({case:"Exact SHA-256 vs perceptual similarity, review collection and queued report",passed:true});

  const inspection=await rpc("image_assets_inspect",{ids:ids.slice(0,2),stitch:true,requestId:81003});assert.equal(inspection.suggestions[0].cut,20);
  const stitch={kind:"stitch",cuts:[20]};const stitchedPreview=await rpc("image_assets_preview",{ids:ids.slice(0,2),options:stitch,requestId:81004});assert.equal(stitchedPreview.width,96);assert.equal(stitchedPreview.height,140);
  await rejected("queue_image_assets",{ids:ids.slice(0,2),options:{...stitch,cuts:[19]},previewId:stitchedPreview.previewId,clientRequest:"native-changed-cut"},/preview|match/i);
  const stitched=await result(await rpc("queue_image_assets",{ids:ids.slice(0,2),options:stitch,previewId:stitchedPreview.previewId,clientRequest:"native-stitch"}));assert.deepEqual(pngDimensions(await readFile(stitched.item.path)),[96,140]);
  const pack={kind:"pack",columns:2,frameWidth:48,frameHeight:40,padding:2,fit:"contain"};const atlasPreview=await rpc("image_assets_preview",{ids:[ids[0],ids[2]],options:pack,requestId:81005});
  const atlas=await result(await rpc("queue_image_assets",{ids:[ids[0],ids[2]],options:pack,previewId:atlasPreview.previewId,clientRequest:"native-atlas"}));
  const atlasFiles=zipEntries(await readFile(atlas.item.path));assert.ok([...atlasFiles.keys()].some(name=>name.endsWith(".png")));const atlasJson=[...atlasFiles.entries()].find(([name])=>/atlas.*\.json$/.test(name));assert.ok(atlasJson);const coordinates=JSON.parse(atlasJson[1]);assert.ok(JSON.stringify(coordinates).includes('"x"'));
  const icons={kind:"icons",sizes:[16,32,256],fit:"contain",background:{kind:"transparent"}};const iconPreview=await rpc("image_assets_preview",{ids:[ids[0]],options:icons,requestId:81006});const iconOutput=await result(await rpc("queue_image_assets",{ids:[ids[0]],options:icons,previewId:iconPreview.previewId,clientRequest:"native-icons"}));const iconFiles=zipEntries(await readFile(iconOutput.item.path));assert.ok(iconFiles.has("favicon.ico"));assert.equal(iconFiles.get("favicon.ico").readUInt16LE(4),3);
  checks.push({case:"Overlap suggestion and corrected stitch, preview binding, atlas JSON and transparent icon package",passed:true,stitchDimensions:[96,140],atlasFiles:[...atlasFiles.keys()],iconFiles:[...iconFiles.keys()]});

  const renameOptions={template:"renamed-{counter}{extension}",start:7,padding:3,date:"2026-11-06",replacement:null};
  const plan=await rpc("file_rename_preview",{ids:[ids[0]],options:renameOptions,requestId:81007});assert.equal(plan.entries.length,1);assert.match(plan.entries[0].newPath,/renamed-007\.png$/);
  await writeFile(plan.entries[0].newPath,"late collision");await rejected("queue_file_rename",{planId:plan.id,clientRequest:"native-rename-collision"},/appeared|exists|occupied|collision/i);
  const safePlan=await rpc("file_rename_preview",{ids:[ids[0]],options:{...renameOptions,start:8},requestId:81008});assert.notEqual(safePlan.entries[0].newPath,plan.entries[0].newPath);
  await completed(await rpc("queue_file_rename",{planId:safePlan.id,clientRequest:"native-rename"}));snapshot=await rpc("get_snapshot");assert.equal(snapshot.library.find(i=>i.id===ids[0]).path,safePlan.entries[0].newPath);assert.equal(hash(await readFile(safePlan.entries[0].newPath)),originals[0][1]);
  await completed(await rpc("queue_file_undo",{operationId:safePlan.id,clientRequest:"native-undo"}));snapshot=await rpc("get_snapshot");assert.equal(pathKey(snapshot.library.find(i=>i.id===ids[0]).path),pathKey(paths[0]));assert.equal((await readFile(plan.entries[0].newPath,"utf8")),"late collision");
  const history=await rpc("file_rename_history");assert.equal(history.find(j=>j.id===safePlan.id).state,"undone");
  checks.push({case:"Rename collision refusal, durable queued rename, preserved stable ID and undo",passed:true,externalCollisionPreserved:true});

  const exportOptions={metadata:true,notes:true,ocr:true};const exportPreview=await rpc("portable_preview_export",{ids,options:exportOptions,requestId:81009});assert.ok(exportPreview.duplicateBytes>0);
  const exported=await result(await rpc("queue_portable_export",{ids,options:exportOptions,clientRequest:"native-portable-export"}));const portableFiles=zipEntries(await readFile(exported.item.path));const manifest=JSON.parse(portableFiles.get("manifest.json"));assert.equal(manifest.items.length,4);assert.ok(!JSON.stringify(manifest).includes(fixtures));
  const importPreview=await rpc("portable_preview_import",{path:exported.item.path,requestId:81010});assert.equal(importPreview.entries.length,4);assert.ok(importPreview.duplicateBytes>0);
  const imported=await result(await rpc("queue_portable_import",{planId:importPreview.planId,clientRequest:"native-portable-import"}));assert.ok(imported.item.name.endsWith(".json"));assert.equal(imported.snapshot.library.filter(i=>ids.includes(i.id)).length,4);
  checks.push({case:"Portable ZIP manifest, duplicate preview and queued idempotent library import",passed:true,files:portableFiles.size,duplicateBytes:importPreview.duplicateBytes});

  await page.reload();await page.locator('[data-page="tools"]').click();await openTool(page, "imageAssets");await page.locator('[data-action="asset-kind"][data-kind="pack"]').click();await page.locator(`[data-asset-input="${ids[0]}"]`).check();await page.locator(`[data-asset-input="${ids[2]}"]`).check();await page.locator('[data-action="asset-preview"]').click();await page.locator('.asset-preview-image img').waitFor();await page.locator('.asset-preview-image').scrollIntoViewIfNeeded();await page.screenshot({path:path.join(data,"native-atlas-preview.png"),animations:"disabled"});
  await openTool(page, "fileOrganization");await page.locator(`[data-file-org-select="${ids[0]}"]`).check();await page.locator(`[data-file-org-select="${ids[3]}"]`).check();await page.locator('[data-action="file-org-analyze"]').click();await page.locator('.org-duplicate-group').waitFor();await page.locator('.org-results').scrollIntoViewIfNeeded();await page.screenshot({path:path.join(data,"native-file-organization.png"),animations:"disabled"});
  for(const [file,expected] of originals)assert.equal(hash(await readFile(file)),expected);
  assert.deepEqual(errors,[]);checks.push({case:"Original hashes unchanged and rendered native workspaces",passed:true});
  await writeFile(path.join(data,"results.json"),JSON.stringify({scope:"Generated local fixtures, real native IPC/engines/queue/filesystem; no provider or account access",checks,errors},null,2));
  console.log(JSON.stringify({data,checks:checks.length,passed:true},null,2));
}finally{
  try{await page?.evaluate(async()=>{const{getCurrentWindow}=await import("/node_modules/@tauri-apps/api/window.js");await getCurrentWindow().close();});}catch{}
  if(child?.exitCode===null){await Promise.race([new Promise(resolve=>child.once("exit",resolve)),new Promise(resolve=>setTimeout(resolve,4000))]);if(child.exitCode===null)child.kill();}
  try{await browser?.close();}catch{}
}
