// Portable workflow UI with explicit IPC fixtures. Native ZIP and filesystem checks run separately.
import assert from "node:assert/strict";
import { openTool } from "./tool-navigation.mjs";
import { createRequire } from "node:module";
import { mkdir, writeFile } from "node:fs/promises";
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = "artifacts/portable-collections";
await mkdir(directory, { recursive: true });
const browser = await chromium.launch({ headless: true, ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }) });
const setup = `
  import {mockIPC,mockWindows} from "/node_modules/@tauri-apps/api/mocks.js";
  import {emit} from "/node_modules/@tauri-apps/api/event.js";
  globalThis.isTauri=true;mockWindows("main");
  const copy=value=>structuredClone(value);
  const settings={folder:"C:/Fixture portable output",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
  const file=(id,name)=>({id,name,path:"C:/Fixture input/"+id+".txt",kind:"document",size:100,duration:null,width:null,height:null,videoCodec:null,audioCodec:null,container:null,addedAt:1,source:null,parentId:null,collections:["Selected collection"],tags:[],missing:false,details:{contentId:null,creator:null,platform:null,sourceIds:[],chapters:[],tracks:[],playbackPosition:0,favorite:false,bookmarks:[],clip:null}});
  const state={settings,jobs:[],library:[file("first","First document.txt"),file("second","Second copy.txt"),file("third",'<img src=x onerror="window.portableXss=true">')],collections:["Selected collection"],presets:[],rules:[],sources:[],drafts:[],smartCollections:[]};
  const fixture=window.portableFixture={state,calls:[],fail:false,slow:false,release:null};window.portableXss=false;
  fixture.exportState=async (status, published=false)=>{
    const job=state.jobs[0];
    if (job.state==="cancelled" && status==="pending") job.runId++;
    job.state=status;
    job.updatedAt++;
    if (published) {
      const zip={...file("archive","Selected collection.zip"),kind:"file",path:settings.folder+"/Selected collection.zip",size:150};
      state.library.push(zip);
      job.outputs=[zip.path];
    }
    await emit("job-update",copy(job));
    if (published) await emit("library-update");
  };
  const entries=ids=>ids.map((id,index)=>({id,name:state.library.find(item=>item.id===id).name,size:100,sha256:"a".repeat(64),duplicateOf:index?"First document.txt":null,article:false,ocr:false}));
  const preview=ids=>({entries:entries(ids),bytes:ids.length*100,uniqueBytes:100,duplicateBytes:Math.max(0,ids.length-1)*100,warnings:["Fixture: application tokens and private paths are excluded from metadata."]});
  mockIPC(async(cmd,args={})=>{
    fixture.calls.push({cmd,args:copy(args)});
    if(cmd==="plugin:window|is_maximized")return false;if(cmd==="startup")return {...copy(state),folder:settings.folder,enginesReady:true,engineError:null};if(cmd==="get_snapshot")return copy(state);if(cmd==="local_timezone")return "Europe/Madrid";
    if(cmd==="portable_preview_export"){if(fixture.slow)return new Promise(resolve=>{fixture.release=()=>resolve(preview(args.ids));});return preview(args.ids);}
    if(cmd==="queue_portable_export"){
      const job={id:"zip-job",runId:1,state:"pending",title:"Export portable collection",createdAt:1,updatedAt:1,
        source:{kind:"portableExport",libraryIds:args.ids,options:args.options},folder:settings.folder,provider:"Collections",
        thumbnail:null,progress:null,outputs:[],error:null,attempt:0,groupId:null};
      state.jobs.push(job);return [copy(job)];
    }
    if(cmd==="queue_portable_import")return [];
    if(cmd==="portable_choose_import")return args.folder?"C:/Fixture portable folder":"C:/Fixture portable.zip";
    if(cmd==="portable_preview_import"){if(fixture.fail)throw "Fixture: the selected source changed after review.";return {...preview(["first","second"]),planId:"b".repeat(64),expiresAt:Date.now()+86400000,target:settings.folder};}
    if(cmd==="cancel_portable_preview"){fixture.release?.();fixture.release=null;fixture.slow=false;return;}
    if(cmd==="engine_status"||cmd==="optional_components"||cmd==="sessions_status"||cmd==="list_courses")return [];
    if(cmd==="cache_status")return {bytes:0,files:0};if(cmd==="telegram_status")return {installed:false,accounts:[]};if(cmd==="torrent_status")return {installed:false,enabled:false};if(cmd==="cloud_status")return [];if(cmd==="bridge_status")return {enabled:false,clients:[],port:47883};
  },{shouldMockEvents:true});await import("/src/main.ts");
`;
const results = [];
try {
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    const page = await browser.newPage({ viewport: { width, height } }); const errors = []; page.on("pageerror", error => errors.push(error.message));
    await page.route(base, async route => { const response = await route.fetch(); const html = (await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/, `<script type="module">${setup}</script>`); await route.fulfill({ response, body: html }); });
    await page.goto(base); await page.locator("[data-page='tools']").click(); await openTool(page, "portable");
    assert.equal(await page.locator("[data-action='portable-export-queue']").count(), 0); assert.equal(await page.locator("#portable-ocr").isChecked(), false);
    assert.equal(await page.locator(".portable-file-picker > label").first().evaluate(element => getComputedStyle(element).flexDirection), "row");
    assert.equal(await page.locator(".portable-file-picker img").count(), 0); assert.equal(await page.evaluate(() => window.portableXss), false);
    await page.locator("#portable-item-first").focus(); await page.keyboard.press("Space");
    assert.equal(await page.locator("#portable-item-first").isChecked(), true); assert.equal(await page.locator("#portable-item-first").evaluate(element => element === document.activeElement), true);
    await page.locator("#portable-item-second").check(); await page.locator("#portable-ocr").check();
    await page.locator("#portable-export-form button[type='submit']").click(); await page.locator(".portable-preview").waitFor();
    assert.match(await page.locator(".portable-summary").innerText(), /100 B/); assert.match(await page.locator(".portable-reviewed-files").innerText(), /Contenido idéntico a First document/);
    assert.equal(await page.evaluate(() => window.portableFixture.calls.some(call => call.cmd === "queue_portable_export")), false);
    await page.locator("#portable-notes").uncheck(); assert.equal(await page.locator("[data-action='portable-export-queue']").count(), 0, "Changing included metadata invalidates the previous review.");
    await page.locator("#portable-export-form button[type='submit']").click(); await page.locator("[data-action='portable-export-queue']").waitFor();
    const dimensions = await page.locator(".portable-workspace").evaluate(element => ({ width: element.clientWidth, scroll: element.scrollWidth })); assert.ok(dimensions.scroll <= dimensions.width + 1);
    await page.locator(".portable-preview").scrollIntoViewIfNeeded(); await page.screenshot({ path: `${directory}/export-review-${width}x${height}-fixture.png` });
    await page.locator("[data-action='portable-export-queue']").click(); await page.waitForFunction(() => window.portableFixture.calls.some(call => call.cmd === "queue_portable_export"));
    const queued = await page.evaluate(() => window.portableFixture.calls.find(call => call.cmd === "queue_portable_export").args); assert.deepEqual(queued.ids, ["first", "second"]); assert.deepEqual(queued.options, { metadata: true, notes: false, ocr: true });
    const result = page.getByRole("region", { name: "Último ZIP de esta sesión" });
    await result.getByText("Pendiente", { exact: true }).waitFor();
    await page.waitForFunction(() => document.activeElement.id === "portable-export-heading");
    assert.equal(await result.getByRole("button", { name: "Revisar envío del ZIP" }).count(), 0);
    assert.equal(await page.locator("[data-action='portable-export-queue']").count(), 0);
    await result.getByRole("button", { name: "Ver en Descargas" }).click();
    await page.getByRole("heading", { name: "Export portable collection", exact: true }).waitFor();
    await page.getByRole("button", { name: "Cerrar detalles", exact: true }).click();
    await page.locator("[data-page='tools']").click();
    await page.evaluate(() => window.portableFixture.exportState("cancelled"));
    await result.getByText("Cancelado", { exact: true }).waitFor();
    assert.equal(await result.getByRole("button", { name: "Revisar envío del ZIP" }).count(), 0);
    await page.evaluate(() => window.portableFixture.exportState("pending"));
    await result.getByText("Pendiente", { exact: true }).waitFor();
    await page.evaluate(() => window.portableFixture.exportState("completed", true));
    const reviewZip = result.getByRole("button", { name: "Revisar envío del ZIP" });
    await reviewZip.waitFor();
    assert.ok(await reviewZip.evaluate(button => getComputedStyle(button.querySelector("span")).color === getComputedStyle(button).color));
    assert.match(await result.innerText(), /Selected collection\.zip · 150 B/);
    await page.locator("[data-action='portable-mode'][data-mode='import']").click(); await page.locator("[data-action='portable-choose-zip']").click();
    assert.equal(await page.evaluate(() => window.portableFixture.calls.some(call => call.cmd === "portable_preview_import")), false);
    await page.locator("[data-action='portable-import-preview']").click(); await page.locator("[data-action='portable-import-queue']").waitFor();
    assert.match(await page.locator(".portable-confirm").innerText(), /C:\/Fixture portable output/);
    await page.locator(".portable-preview").scrollIntoViewIfNeeded(); await page.screenshot({ path: `${directory}/import-review-${width}x${height}-fixture.png` });
    await page.locator("[data-action='portable-import-queue']").click(); const imported = await page.evaluate(() => window.portableFixture.calls.find(call => call.cmd === "queue_portable_import").args); assert.equal(imported.planId, "b".repeat(64)); assert.equal("path" in imported, false);
    await page.evaluate(() => { window.portableFixture.fail = true; }); await page.locator("[data-action='portable-import-preview']").click(); await page.locator(".portable-error").waitFor(); assert.equal(await page.locator("[data-action='portable-import-queue']").count(), 0);
    await page.locator("[data-action='portable-choose-folder']").click(); assert.match(await page.locator(".portable-source-path").innerText(), /portable folder/);
    await page.locator("[data-action='portable-mode'][data-mode='export']").click(); await page.evaluate(() => { window.portableFixture.slow = true; }); await page.locator("#portable-export-form button[type='submit']").click(); await page.locator("[data-action='portable-cancel']").click(); assert.equal(await page.locator("[data-action='portable-export-queue']").count(), 0);
    assert.deepEqual(errors, []); results.push({ viewport: { width, height }, status: "passed", checks: ["Explicit library selection and keyboard focus", "SHA-256 duplicate review before export", "Independent metadata, notes and opt-in OCR options", "Import approval references a reviewed plan rather than arbitrary paths", "Review invalidation, folder chooser and cancellation", "Escaped filenames and no horizontal overflow"] }); await page.close();
  }
  await writeFile(`${directory}/verification.json`, JSON.stringify({ evidence: "Rendered UI with IPC fixtures; native file verification is separate.", results }, null, 2) + "\n"); console.log(JSON.stringify(results));
} finally { await browser.close(); }
