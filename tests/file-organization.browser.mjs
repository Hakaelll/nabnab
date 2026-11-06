// Rendered controller and IPC fixtures. Native file safety has separate Rust tests.
import assert from "node:assert/strict";
import {mkdir,writeFile} from "node:fs/promises";
import {createRequire} from "node:module";
const {chromium}=createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE||"playwright");
const browser=await chromium.launch({headless:true,...(process.env.EDGE_EXECUTABLE?{executablePath:process.env.EDGE_EXECUTABLE}:{channel:"msedge"})});
const base=process.env.NABNAB_UI_URL||"http://127.0.0.1:1420/";
const directory="artifacts/file-organization";await mkdir(directory,{recursive:true});
const setup=`
import {mockIPC,mockWindows} from "/node_modules/@tauri-apps/api/mocks.js";
globalThis.isTauri=true;mockWindows("main");
const copy=value=>structuredClone(value);
const settings={folder:"C:/Fixture organization",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
const item=(id,name,kind,path)=>({id,name,kind,path:path||settings.folder+"/"+name,size:4096,duration:null,width:kind==="image"?520:null,height:kind==="image"?360:null,videoCodec:null,audioCodec:null,container:null,addedAt:1793968800000,source:null,parentId:null,collections:["Reference collection"],tags:[],missing:false,details:{contentId:null,creator:null,platform:null,sourceIds:[],chapters:[],tracks:[],playbackPosition:0,favorite:false,bookmarks:[],clip:null}});
const state={settings,jobs:[],library:[item("text-a","Report alpha.txt","document"),item("text-b","Report copy.txt","document"),item("alias","Shared record.txt","document",settings.folder+"/Report alpha.txt"),item("image-a","Landscape original.png","image"),item("image-b","Landscape smaller.jpg","image")],collections:["Reference collection"],smartCollections:[],sources:[],drafts:[],presets:[],rules:[]};
const fixture=window.organizationFixture={state,calls:[],slow:false,pending:null,history:[]};
const files=ids=>ids.filter(id=>id!=="alias").map(id=>{const value=state.library.find(item=>item.id===id);return {id,recordIds:id==="text-a"?[id,"alias"]:[id],name:value.name,path:value.path,size:value.size,collections:value.collections};});
const analysis=args=>({options:args.options,files:files(args.ids),exactGroups:args.options.kind==="exact"?[{sha256:"a".repeat(64),members:args.ids.filter(id=>id!=="alias"),bytesPerFile:4096}]:[],similarPairs:args.options.kind==="similar"?[{left:args.ids[0],right:args.ids[1],distance:3,bits:64}]:[],warnings:["Fixture comparison only. No file was read or renamed."]});
mockIPC(async(cmd,args={})=>{
fixture.calls.push({cmd,args:copy(args)});
if(cmd==="plugin:window|is_maximized")return false;
if(cmd==="startup")return {...copy(state),folder:settings.folder,enginesReady:true,engineError:null};
if(cmd==="get_snapshot")return copy(state);
if(cmd==="local_timezone")return "Europe/Madrid";
if(cmd==="file_analysis"){if(fixture.slow)return await new Promise(resolve=>{fixture.pending=()=>resolve(analysis(args));});return analysis(args);}
if(cmd==="cancel_file_operation"){fixture.slow=false;fixture.pending?.();fixture.pending=null;return;}
if(cmd==="file_organize_review"){for(const item of state.library.filter(item=>args.ids.includes(item.id)))item.collections.push(args.collection);return;}
if(cmd==="file_rename_preview"){if(args.options.template.includes("collision"))throw "A preview destination already exists. Existing files were preserved.";return {id:"b".repeat(64),createdAt:Date.now(),expiresAt:Date.now()+600000,entries:args.ids.map((id,index)=>({recordIds:id==="text-a"?[id,"alias"]:[id],oldPath:state.library.find(item=>item.id===id).path,newPath:settings.folder+"/review-"+(index+1)+".txt",sha256:"a".repeat(64),size:4096})),warnings:["Fixture plan. Disk and persistence are separate steps; existing files are never overwritten."]};}
if(cmd==="queue_file_rename"){fixture.history=[{id:args.planId,createdAt:Date.now(),updatedAt:Date.now(),state:"completed",entries:[{recordIds:["text-a","alias"],oldPath:settings.folder+"/Report alpha.txt",newPath:settings.folder+"/review-1.txt",sha256:"a".repeat(64),size:4096,state:"recorded",error:null}],error:null}];return [];}
if(cmd==="file_rename_history")return copy(fixture.history);
if(cmd==="queue_file_undo"){fixture.history[0].state="undone";return [];}
if(cmd==="queue_file_analysis")return [];
if(cmd==="thumbnail"){const color=args.id==="image-a"?"#71828c":"#73818a";return "data:image/svg+xml,"+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="520" height="360" viewBox="0 0 520 360"><rect width="520" height="360" fill="'+color+'"/><circle cx="380" cy="100" r="35" fill="#d9d4bb"/><path d="M0 300 140 100 280 270 400 180 520 300V360H0Z" fill="#394a49"/><path d="M0 320 180 230 340 320 520 260V360H0Z" fill="#1e3030"/></svg>');}
if(["engine_status","sessions_status","optional_components","list_courses","cloud_status"].includes(cmd))return [];
if(cmd==="cache_status")return {bytes:0,files:0};
if(cmd==="telegram_status")return {installed:false,accounts:[]};
if(cmd==="torrent_status")return {installed:false,enabled:false,version:null};
if(cmd==="bridge_status")return {enabled:false,clients:[],port:47883};
},{shouldMockEvents:true});
await import("/src/main.ts");
`;
const checks=[];
try{
  for(const[width,height]of[[1180,780],[900,620]]){
    const page=await browser.newPage({viewport:{width,height}});const errors=[];page.on("pageerror",error=>errors.push(error.message));
    await page.route(base,async route=>{const response=await route.fetch();const html=(await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/,`<script type="module">${setup}</script>`);await route.fulfill({response,body:html});});
    await page.goto(base);await page.locator("#url").waitFor();await page.locator("[data-page='tools']").click();await page.locator("[data-action='tool-module'][data-id='fileOrganization']").click();
    const checkOverflow=async label=>{const dimensions=await page.locator(".file-organization").evaluate(element=>({width:element.clientWidth,scroll:element.scrollWidth}));assert.ok(dimensions.scroll<=dimensions.width+1,`${label}: horizontal overflow ${JSON.stringify(dimensions)}`);};
    await page.locator("#file-org-select-text-a").focus();await page.keyboard.press("Space");assert.equal(await page.locator("#file-org-select-text-a").isChecked(),true);assert.equal(await page.locator("#file-org-select-text-a").evaluate(element=>element===document.activeElement),true);await page.keyboard.press("Space");assert.equal(await page.locator("#file-org-select-text-a").isChecked(),false);assert.equal(await page.locator("#file-org-select-text-a").evaluate(element=>element===document.activeElement),true);await page.locator("[data-file-org-select='text-a']").check();await page.locator("[data-file-org-select='text-b']").check();await page.locator("[data-action='file-org-analyze']").click();await page.locator(".org-duplicate-group").waitFor();assert.match(await page.locator(".org-duplicate-group").innerText(),/registros comparten/);
    await page.locator("#file-org-review-text-b").focus();await page.keyboard.press("Space");assert.equal(await page.locator("#file-org-review-text-b").isChecked(),true);assert.equal(await page.locator("#file-org-review-text-b").evaluate(element=>element===document.activeElement),true);await page.locator("#file-org-collection").fill("Copies to review");await page.locator("#file-org-collection-form button[type='submit']").click();
    const organized=await page.evaluate(()=>window.organizationFixture.calls.find(call=>call.cmd==="file_organize_review").args);assert.deepEqual(organized.ids,["text-b"]);assert.equal(organized.collection,"Copies to review");
    await checkOverflow(`exact ${width}`);await page.locator(".org-results").scrollIntoViewIfNeeded();await page.screenshot({path:`${directory}/exact-${width}x${height}-fixture.png`});
    await page.locator("[data-action='file-org-save-report']").click();await page.waitForFunction(()=>window.organizationFixture.calls.some(call=>call.cmd==="queue_file_analysis"));
    await page.locator("[data-action='file-org-mode'][data-mode='similar']").click();await page.locator("[data-file-org-select='image-a']").check();await page.locator("[data-file-org-select='image-b']").check();await page.locator("[data-action='file-org-analyze']").click();await page.locator(".org-similar-pair").waitFor();await page.locator(".org-similar-pair").scrollIntoViewIfNeeded();await page.locator(".org-compare-image img").first().waitFor();assert.equal(await page.locator(".org-pair-images figure").count(),2);assert.match(await page.locator(".org-pair-heading").innerText(),/3 \/ 64/);await checkOverflow(`similar ${width}`);await page.locator(".org-similar-pair").scrollIntoViewIfNeeded();await page.screenshot({path:`${directory}/similar-${width}x${height}-fixture.png`});
    await page.locator("[data-action='file-org-mode'][data-mode='rename']").click();await page.locator("[data-file-org-select='text-a']").check();await page.locator("#file-org-template").fill("collision{extension}");await page.locator("#file-org-rename-form button[type='submit']").click();await page.getByRole("alert").filter({hasText:"destination already exists"}).waitFor();assert.equal(await page.locator("[data-action='file-org-queue-rename']").count(),0);
    await page.locator("#file-org-template").fill("{name}-{counter}{extension}");await page.locator("#file-org-replace").check();await page.locator("#file-org-pattern").fill("^Report ");await page.locator("#file-org-replacement").fill("Document ");await page.locator("#file-org-regex").check();await page.locator("#file-org-rename-form button[type='submit']").click();await page.locator(".org-rename-preview").waitFor();assert.match(await page.locator(".org-rename-preview").innerText(),/relacionados se actualizarán/);await checkOverflow(`rename ${width}`);await page.locator(".org-rename-preview").scrollIntoViewIfNeeded();await page.screenshot({path:`${directory}/rename-${width}x${height}-fixture.png`});
    await page.locator("[data-action='file-org-queue-rename']").click();await page.locator(".org-operation summary").filter({hasText:"Renombrado"}).waitFor();await page.locator(".org-operation summary").click();await page.locator("[data-action='file-org-undo']").click();await page.locator(".org-operation summary").filter({hasText:"Deshecho"}).waitFor();
    await page.locator("[data-action='file-org-mode'][data-mode='exact']").click();await page.locator("[data-file-org-select='text-a']").check();await page.locator("[data-file-org-select='text-b']").check();await page.evaluate(()=>{window.organizationFixture.slow=true;});await page.locator("[data-action='file-org-analyze']").click();await page.locator("[data-action='file-org-cancel']").click();assert.equal(await page.locator(".org-results").count(),0);assert.equal(await page.locator("[data-action='file-org-analyze']").isEnabled(),true);
    await page.locator("[data-action='file-org-mode'][data-mode='exact']").focus();await page.keyboard.press("Tab");const focus=await page.locator("[data-action='file-org-mode'][data-mode='similar']").evaluate(element=>{const style=getComputedStyle(element);return {outline:style.outlineStyle,width:style.outlineWidth};});assert.equal(focus.outline,"solid");assert.notEqual(focus.width,"0px");assert.deepEqual(errors,[]);
    checks.push(`${width}×${height}: exact groups/shared records, review collection, saved report, similar pair images, rename preview/collision, regex input, queued rename/undo, cancellation and visible keyboard focus.`);await page.close();
  }
  await writeFile(`${directory}/verification.json`,JSON.stringify({evidence:"Rendered UI with IPC fixtures; no native files were renamed by this test.",checks},null,2)+"\n");console.log(checks.join("\n"));
}finally{await browser.close();}
