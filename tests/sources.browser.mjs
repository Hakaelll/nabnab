// This exercises the rendered frontend with IPC fixtures, not a live YouTube check.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }) });
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
await mkdir("artifacts/sources", { recursive: true });
const page = await browser.newPage({ viewport: { width: 1180, height: 780 } });
const failures = [];
page.on("pageerror", error => failures.push(error.message));
const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  import { defaultWorkOptions } from "/src/options.ts";
  globalThis.isTauri=true;mockWindows("main");
  const copy=value=>structuredClone(value);
  const settings={folder:"C:/Downloads/nabnab-fixture",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
  const state={settings,jobs:[],library:[],collections:[],drafts:[],sources:[],rules:[],smartCollections:[],presets:[{id:"video",name:"Vídeo compatible",revision:1,options:defaultWorkOptions()}]};
  const fixture=window.sourceFixture={state,calls:[]};
  const item=(id,title,status,discovery)=>({contentId:"youtube:"+id,title,creator:"Fixture creator",url:"https://www.youtube.com/watch?v="+id,thumbnail:null,duration:120,publishedAt:null,discoveredAt:Date.now(),state:status,discovery,reason:discovery==="ambiguous"?"Incomplete baseline or missing publication date. Review this item before downloading.":null,jobIds:[]});
  mockIPC(async(cmd,args={})=>{
    fixture.calls.push({cmd,args:copy(args)});
    if(cmd==="plugin:window|is_maximized")return false;
    if(cmd==="startup")return {...copy(state),folder:settings.folder,enginesReady:true,engineError:null};
    if(cmd==="get_snapshot")return copy(state);
    if(cmd==="local_timezone")return "Romance Standard Time";
    if(cmd==="add_source"){
      const source={id:"fixture-source",name:"Tutoriales de prueba local",url:args.url,kind:"playlist",status:"ready",createdAt:Date.now(),lastCheck:Date.now(),nextCheck:null,error:null,failures:0,settings:copy(args.settings),baseline:{complete:false,inspected:50,total:180,cursor:50,hasMore:true},items:[item("camera","Camera lesson","observed","history"),item("sound","Sound lesson","observed","history")]};
      state.sources.push(source);return copy(source);
    }
    if(cmd==="check_source"){
      const source=state.sources.find(source=>source.id===args.id);
      if(!source.items.some(item=>item.contentId==="youtube:new"))source.items.push(item("new","New item without a publication date","review","ambiguous"));
      source.lastCheck=Date.now();return copy(source);
    }
    if(cmd==="save_source"){
      const source=state.sources.find(source=>source.id===args.id);source.name=args.name;source.settings=copy(args.settings);source.status=args.paused?"paused":"ready";return copy(source);
    }
    if(cmd==="source_item_action"){
      const source=state.sources.find(source=>source.id===args.id);
      for(const entry of source.items.filter(item=>args.contentIds.includes(item.contentId))){
        if(args.action==="queue"){
          const job={id:"fixture-job-"+state.jobs.length,runId:0,title:entry.title,createdAt:Date.now(),updatedAt:Date.now(),state:"pending",source:{kind:"download",url:entry.url,format:"mp4",height:null,audioBitrate:null,subtitleLanguage:null},folder:settings.folder,provider:"YouTube",thumbnail:null,progress:null,outputs:[],error:null,attempt:0,groupId:"source-fixture",priority:"normal",schedule:{kind:"immediate"},manualOrder:0,options:null,contentId:entry.contentId,sourceId:source.id};
          state.jobs.push(job);entry.jobIds=[job.id];entry.state="queued";
        }else entry.state=args.action==="ignore"?"ignored":"review";
      }
      return copy(source);
    }
    if(cmd==="remove_source"){
      state.sources=state.sources.filter(source=>source.id!==args.id);
      if(args.cancelPending)for(const job of state.jobs)if(job.sourceId===args.id&&job.state==="pending")job.state="cancelled";
      return;
    }
    if(cmd==="engine_status")return [];
    if(cmd==="cache_status")return {bytes:0,files:0};
    if(cmd==="sessions_status")return [];
    if(cmd==="cancel_job")return;
  },{shouldMockEvents:true});
  await import("/src/main.ts");
`;
await page.route(base, async route => {
  const response = await route.fetch();
  const html = (await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^\"]*"><\/script>/, `<script type="module">${setup}</script>`);
  await route.fulfill({ response, body: html });
});

try {
  await page.goto(base);
  await page.locator("#url").waitFor();
  await page.locator("[data-page='library']").click();
  await page.locator("[data-action='library-section'][data-section='sources']").click();
  await page.getByRole("heading", { name: "Añade tu primera fuente" }).waitFor();
  await page.locator("[data-action='source-add']").first().click();
  assert.equal(await page.locator("#source-automatic").isChecked(), false);
  assert.equal(await page.locator("#source-interval").inputValue(), "");
  assert.equal(await page.locator("input[name='source-history'][value='fromNow']").isChecked(), true);
  await page.locator("#source-url").fill("https://www.youtube.com/playlist?list=PLlocalFixture");
  await page.locator("#source-editor button[type='submit']").click();
  await page.getByRole("heading", { name: "Tutoriales de prueba local" }).waitFor();
  assert.equal(await page.evaluate(() => window.sourceFixture.state.jobs.length), 0);
  assert.equal(await page.evaluate(() => window.sourceFixture.state.library.length), 0);
  assert.match(await page.locator("#page-content").innerText(), /exploración parcial/);

  await page.locator("[data-action='source-check']").click();
  await page.getByText("New item without a publication date", { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.sourceFixture.state.jobs.length), 0);
  await page.locator("input[data-source-item='youtube:camera']").check();
  await page.locator("[data-action='source-queue-selected']").click();
  await page.waitForFunction(() => window.sourceFixture.state.jobs.length === 1);
  assert.equal(await page.evaluate(() => window.sourceFixture.state.jobs[0].title), "Camera lesson");
  assert.equal(await page.evaluate(() => window.sourceFixture.state.library.length), 0);
  await page.screenshot({ path: "artifacts/sources/source-review-desktop-fixture.png", fullPage: true });

  await page.locator("[data-action='source-edit']").click();
  await page.locator("#source-preset").selectOption("video");
  await page.locator("#source-automatic").check();
  await page.locator("#source-interval").selectOption("6");
  await page.locator("#source-paused").check();
  await page.locator("#source-editor button[type='submit']").click();
  await page.waitForFunction(() => window.sourceFixture.state.sources[0].status === "paused");
  assert.equal(await page.locator("[data-action='source-check']").isDisabled(), true);
  assert.equal(await page.evaluate(() => window.sourceFixture.state.jobs[0].state), "pending");
  await page.setViewportSize({ width: 900, height: 620 });
  await page.screenshot({ path: "artifacts/sources/source-review-narrow-fixture.png", fullPage: true });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false);

  await page.getByText("Eliminar seguimiento", { exact: true }).click();
  assert.equal(await page.locator("#source-remove-pending").isChecked(), false);
  await page.locator("[data-action='source-remove']").click();
  await page.waitForFunction(() => window.sourceFixture.state.sources.length === 0);
  assert.equal(await page.evaluate(() => window.sourceFixture.state.jobs[0].state), "pending");
  const removals = await page.evaluate(() => window.sourceFixture.calls.filter(call => call.cmd === "remove_source"));
  assert.equal(removals[0].args.cancelPending, false);
  assert.deepEqual(failures, []);
  await writeFile("artifacts/sources/result.json", JSON.stringify({ kind: "frontend IPC fixture", checks: ["manual defaults", "no initial history download", "partial scope visible", "ambiguous new item stays review", "only selected queues", "queued is not a local file", "pause preserves pending jobs", "remove keeps files and pending jobs by default", "900px no horizontal overflow"], failures }, null, 2));
  console.log("Source frontend fixture checks passed. Native logic is checked separately.");
} catch (error) {
  await writeFile("artifacts/sources/failure.json", JSON.stringify({ error: String(error), failures, url: page.url() }, null, 2));
  await page.screenshot({ path: "artifacts/sources/failure.png", fullPage: true }).catch(() => {});
  throw error;
} finally { await browser.close(); }
