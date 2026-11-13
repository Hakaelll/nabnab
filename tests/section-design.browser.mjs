// Rendered section layouts with IPC fixtures. No provider or engine access is asserted.
import assert from "node:assert/strict";
import { openTool } from "./tool-navigation.mjs";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({headless:true,channel:"msedge"});
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = process.env.NABNAB_SECTION_ARTIFACTS || "artifacts/section-design";
await mkdir(directory,{recursive:true});
const before = process.argv.includes("--before");
const page = await browser.newPage({viewport:{width:1180,height:780}});
const errors=[];page.on("pageerror",error=>errors.push(error.message));
const setup=`
import {mockIPC,mockWindows} from "/node_modules/@tauri-apps/api/mocks.js";
globalThis.isTauri=true;mockWindows("main");
const settings={folder:"C:/Downloads/Personal archive",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:false,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
const library=[{id:"clip",name:"Entrevista de archivo.mp4",kind:"video",path:"C:/Media/Entrevista de archivo.mp4",size:120000000,duration:436,width:1920,height:1080,videoCodec:"h264",audioCodec:"aac",container:"mp4",addedAt:Date.now(),source:null,parentId:null,collections:["Entrevistas"],tags:[],missing:false}];
const jobs=[
{id:"live",title:"Un recorrido por la ciudad",state:"downloading",provider:"YouTube",source:{kind:"download",url:"https://youtube.com/watch?v=fixture",format:"source",height:1080,audioBitrate:null},progress:{phase:"downloading",percent:43.2,speed:6291456,downloaded:120000000,total:280000000},outputs:[],error:null},
{id:"queued",title:"Entrevista de archivo · Audio",state:"pending",provider:"FFmpeg",source:{kind:"transform",libraryId:"clip",options:{kind:"extractAudio",format:"mp3"}},progress:null,outputs:[],error:null},
{id:"saved-clip",title:"Entrevista de archivo",state:"completed",provider:"YouTube",source:{kind:"download",url:"https://youtube.com/watch?v=fixture2",format:"source"},progress:null,outputs:[library[0].path],error:null},
{id:"complete",title:"Documento de referencia.pdf",state:"completed",provider:"Google Drive",source:{kind:"cloudFile",selection:{provider:"drive",accountId:"fixture",id:"doc",version:"v1",exportMime:null},collection:"Lecturas"},progress:null,outputs:["C:/Downloads/Documento de referencia.pdf"],error:null},
{id:"failed",title:"Colección de imágenes",state:"error",provider:"Wikimedia Commons",source:{kind:"publicFile",selection:{sourceUrl:"https://commons.wikimedia.org/wiki/File:Fixture.jpg",filename:"Fixture.jpg",assetId:"commons:fixture",collection:"Referencias",author:null,license:null,attribution:null,sha256:null,version:null,size:null}},progress:null,outputs:[],error:{code:"network",message:"La conexión se interrumpió. Puedes reintentar el trabajo.",technical:"UI fixture only",retryable:true,retryAfter:null}}
].map((job,index)=>({...job,runId:1,createdAt:Date.now()-index*60000,updatedAt:Date.now(),folder:settings.folder,thumbnail:null,attempt:1,groupId:null}));
const state={settings,library,jobs,collections:["Entrevistas","Lecturas"],drafts:[],sources:[],rules:[],presets:[],smartCollections:[]};
const components=[
  {id:"upscale",name:"Ampliar imagen · Real-ESRGAN",version:"0.2.5.0",size:38482789,installedSize:43253760,license:"MIT · BSD-3-Clause",source:"https://github.com/xinntao/Real-ESRGAN/releases/tag/v0.2.5.0",installed:true,enabled:true,available:true,note:"Windows x64 · Requiere una GPU compatible con Vulkan."},
  {id:"stems",name:"Separar pistas · Spleeter",version:"2.0.1",size:41104179,installedSize:56413389,license:"Apache-2.0 · MIT",source:"https://github.com/k2-fsa/sherpa-onnx/releases/tag/v2.0.1",installed:false,enabled:false,available:true,note:"Windows x64 · CPU · Requiere Microsoft Visual C++ Redistributable."},
];
window.sectionFixture={state,calls:[],voiceCache:{bytes:67108864,files:24}};
mockIPC(async(cmd,args={})=>{
window.sectionFixture.calls.push({cmd,args});
if(cmd==="startup")return {...structuredClone(state),folder:settings.folder,enginesReady:true,engineError:null};
if(cmd==="get_snapshot")return structuredClone(state);
if(cmd==="library_search"){
  if(args.query==="consulta tardia")return new Promise((_,reject)=>{window.sectionFixture.rejectSearch=()=>reject(new Error("Stale search fixture"));});
  return [];
}
if(cmd==="local_timezone")return "Romance Standard Time";
if(cmd==="engine_status")return ["yt-dlp","FFmpeg","ffprobe","Deno"].map(name=>({name,version:"Installed fixture",path:"C:/Components/"+name,size:1024,status:"installed",source:"Local UI fixture"}));
if(cmd==="optional_components")return structuredClone(components);
if(cmd==="enable_component"){components.find(component=>component.id===args.id).enabled=args.enabled;return true;}
if(cmd==="remove_component"){
  if(!window.sectionFixture.confirmComponentRemoval)return false;
  const component=components.find(component=>component.id===args.id);
  component.installed=false;component.enabled=false;return true;
}
if(cmd==="sessions_status"||cmd==="list_courses"||cmd==="ocr_models"||cmd==="translation_drafts"||cmd==="personal_archive_list")return [];
if(cmd==="ocr_status")return {installed:false,enabled:false};
if(cmd==="cache_status")return {bytes:0,files:0};
if(cmd==="voice_cache_status")return {...window.sectionFixture.voiceCache};
if(cmd==="clear_voice_cache")return window.sectionFixture.voiceCache={bytes:0,files:0};
if(cmd==="cloud_status")return ["drive","onedrive","dropbox"].map(provider=>({provider,configured:false,connected:false,accountId:null,scope:"Read selected files",redirectUri:"http://127.0.0.1:53687/callback"}));
if(cmd==="bridge_status")return {enabled:false,clients:[],port:49587};
if(cmd==="save_settings"){state.settings=args.settings;return structuredClone(state);}
if(cmd==="job_action"){for(const job of jobs)if(args.ids.includes(job.id))job.state=args.action==="retry"?"pending":"cancelled";return structuredClone(state);}
if(cmd==="telegram_status")return {installed:false,accounts:[]};
if(cmd==="torrent_status")return {installed:false,enabled:false,version:null};
if(cmd==="thumbnail")return "data:image/svg+xml,"+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90"><path fill="#303236" d="M0 0h160v90H0z"/><path fill="#56595e" d="m0 80 55-50 40 35 25-20 40 35v10H0z"/></svg>');
}, {shouldMockEvents:true});
await import("/src/main.ts");`;
await page.route(base,async route=>{const response=await route.fetch();const html=(await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/,`<script type="module">${setup}</script>`);await route.fulfill({response,body:html});});
const checks=[];
async function capture(section,width,height){
  await page.setViewportSize({width,height});
  await page.locator(`[data-page="${section}"]`).click();
  await page.locator("#page-content h1").waitFor();
  await page.locator("#main").evaluate(el=>el.scrollTop=0);
  const dimensions=await page.locator("#page-content").evaluate(el=>({width:el.clientWidth,scroll:el.scrollWidth}));
  assert(dimensions.scroll<=dimensions.width+2,`${section} ${width} overflow ${JSON.stringify(dimensions)}`);
  await page.screenshot({path:`${directory}/${section}-${before?"before":"after"}-${width}x${height}.png`});
}
try{
  await page.goto(base);await page.locator("#url").waitFor();
  await page.locator('[data-page="settings"]').click();
  await page.locator('[data-action="settings-section"][data-section="engines"]').click();
  const optional = page.locator(".optional-components");
  const upscale = optional.locator(".component-row", { hasText: "Real-ESRGAN" });
  const stems = optional.locator(".component-row", { hasText: "Spleeter" });
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    await optional.scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${directory}/components-${before ? "before" : "after"}-${width}.png` });
  }
  if (!before) {
    await stems.getByText("Procedencia y licencia", { exact: true }).press("Enter");
    await stems.getByText(/Apache-2\.0 · MIT/).waitFor();
    await upscale.getByRole("button", { name: "Desactivar", exact: true }).press("Enter");
    const enable = upscale.getByRole("button", { name: "Activar", exact: true });
    await enable.waitFor();
    assert.ok(await enable.evaluate(element => document.activeElement === element));
    assert.match(await upscale.innerText(), /41.3 MiB en disco/);
    assert.match(await stems.innerText(), /39.2 MiB de descarga/);
    assert.equal(await stems.locator("details").getAttribute("open"), "");
    assert.ok(await optional.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    const remove = upscale.getByRole("button", { name: "Eliminar", exact: true });
    await remove.press("Enter");
    await page.waitForFunction(() => window.sectionFixture.calls.some(call => call.cmd === "remove_component"));
    assert.ok(await remove.evaluate(element => document.activeElement === element));
    assert.equal(await enable.isVisible(), true);
    await page.evaluate(() => { window.sectionFixture.confirmComponentRemoval = true; });
    await remove.press("Enter");
    const install = upscale.getByRole("button", { name: "Instalar", exact: true });
    await install.waitFor();
    assert.ok(await install.evaluate(element => document.activeElement === element));
    assert.match(await upscale.innerText(), /36.7 MiB de descarga · 41.3 MiB tras instalar/);
    console.log("Component size, license disclosure, disable, removal and focus passed at desktop widths.");
  }
  await page.locator('[data-action="settings-section"][data-section="sessions"]').click();
  const thumbnails = page.getByRole("heading", { name: "Caché de miniaturas" });
  const speech = page.getByRole("region", { name: "Caché de voz" });
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    await (before ? thumbnails : speech).scrollIntoViewIfNeeded();
    assert.ok(await page.locator(".settings-content").evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    await page.screenshot({ path: `${directory}/voice-cache-${before ? "before" : "after"}-${width}.png` });
  }
  if (!before) {
    const clearSpeech = speech.getByRole("button", { name: "Vaciar caché de voz…" });
    assert.match(await speech.innerText({ timeout: 3000 }), /64\.0 MB/);
    await clearSpeech.press("Space");
    const confirm = page.getByRole("dialog", { name: "¿Vaciar la caché de voz?" });
    assert.ok(await confirm.getByRole("button", { name: "Cancelar" }).evaluate(element => document.activeElement === element));
    await page.keyboard.press("Escape");
    assert.equal(await page.evaluate(() => window.sectionFixture.calls.filter(call => call.cmd === "clear_voice_cache").length), 0);
    await clearSpeech.press("Space");
    await confirm.getByRole("button", { name: "Vaciar caché de voz", exact: true }).press("Space");
    await speech.getByText("Caché vacía").waitFor();
    assert.equal(await clearSpeech.isDisabled(), true);
    assert.ok(await speech.getByRole("heading").evaluate(element => document.activeElement === element));
    assert.deepEqual(await page.evaluate(() => window.sectionFixture.calls.filter(call => ["clear_voice_cache", "clear_cache"].includes(call.cmd)).map(call => call.cmd)), ["clear_voice_cache"]);
    checks.push("Voice cache usage, confirmation, and keyboard focus are independent of thumbnails.");
  }
  for(const size of [[1180,780],[900,620]])for(const section of ["downloads","library","tools","platforms","settings"])await capture(section,...size);
  for (const [width,height] of [[1180,780],[900,620]]) {
    await page.setViewportSize({width,height});
    await page.locator('button[data-page="library"]').click();
    await page.keyboard.press("Control+k");
    const palette=page.locator(".action-palette");
    await palette.waitFor();
    assert.ok(await palette.evaluate(dialog=>dialog.scrollWidth<=dialog.clientWidth+1 && dialog.getBoundingClientRect().bottom<=innerHeight));
    await page.screenshot({path:`${directory}/palette-${before?"before":"after"}-${width}x${height}.png`,animations:"disabled"});
    await page.keyboard.press("Escape");
  }
  if(!before){
    await page.locator('[data-page="downloads"]').click();
    const sidebar = page.getByRole("navigation", { name: "Navegación principal" });
    const downloadNav = sidebar.getByRole("button", { name: "Descargas" });
    const sidebarWidth = width => page.waitForFunction(width => Math.abs(document.querySelector(".sidebar").getBoundingClientRect().width - width) < 1, width);
    assert.equal(await sidebar.locator(".sidebar-header, #toggle-sidebar, .sidebar-mark").count(), 0);
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({ width, height });
      await page.mouse.move(width - 40, height - 40);
      await sidebarWidth(64);
      const mainBounds = await page.locator("#main").boundingBox();
      const icon = await downloadNav.locator("svg").first().boundingBox();
      await page.screenshot({ path: `${directory}/sidebar-compact-${width}.png` });
      await downloadNav.hover();
      await sidebarWidth(200);
      assert.match(await sidebar.evaluate(node => getComputedStyle(node).backgroundColor), /^rgba\(9, 10, 12, 0\.66/);
      assert.equal(await sidebar.evaluate(node => getComputedStyle(node).backdropFilter), "blur(4px)");
      assert.equal(await downloadNav.locator(".nav-label").evaluate(label => getComputedStyle(label).opacity), "1");
      assert.deepEqual(await page.locator("#main").boundingBox(), mainBounds);
      assert.deepEqual(await downloadNav.locator("svg").first().boundingBox(), icon);
      await page.mouse.move(150, icon.y + 9);
      assert.equal((await sidebar.boundingBox()).width, 200);
      await page.screenshot({ path: `${directory}/sidebar-expanded-${width}.png` });
      await page.mouse.move(width - 40, height - 40);
      await sidebarWidth(64);
    }
    await page.getByRole("button", { name: "Cerrar", exact: true }).focus();
    await page.keyboard.press("Tab");
    assert.equal(await sidebar.getByRole("button", { name: "Inicio" }).evaluate(button => button === document.activeElement), true);
    await sidebarWidth(200);
    await page.keyboard.press("Tab");
    assert.equal(await downloadNav.evaluate(button => button === document.activeElement), true);
    await page.locator("#job-search").focus();
    await sidebarWidth(64);
    await downloadNav.click();
    await sidebarWidth(64);
    await page.mouse.move(400, 180);
    await downloadNav.hover();
    await sidebarWidth(200);
    await page.mouse.move(400, 180);
    await sidebarWidth(64);
    checks.push("Sidebar expands by pointer or keyboard and keeps the workspace and icons fixed.");
    assert.equal(await page.locator(".job-row").count(),5);
    assert.deepEqual(await page.locator(".job-section-heading h2").allTextContents(),["En curso","En espera","Finalizadas"]);
    assert.equal(await page.getByRole("progressbar", {name:"Un recorrido por la ciudad"}).getAttribute("aria-valuenow"),"43.2");
    assert.ok(await page.locator(".job-row.is-transferring").evaluate(row => row.querySelector(".progress-rail").clientWidth > row.clientWidth * 0.75));
    await page.getByText("43.2%",{exact:true}).waitFor();
    const preview=page.locator(".job-list .row-thumbnail[data-thumbnail-id='clip']");
    await preview.scrollIntoViewIfNeeded();
    await preview.locator("img").waitFor();
    await page.getByRole("button",{name:"Buscar en nabnab"}).click();
    const palette=page.locator(".action-palette");
    const query=palette.getByRole("combobox");
    const selected=palette.locator('[role="option"][aria-selected="true"]');
    const searchButton=page.getByRole("button",{name:"Buscar en nabnab",exact:true});
    await palette.waitFor();
    assert.equal(await palette.evaluate(element=>getComputedStyle(element).backdropFilter),"blur(18px)");
    await palette.getByRole("option",{name:"Descargas Sección",exact:true}).hover();
    assert.match(await selected.innerText(),/Descargas/);
    await page.keyboard.press("ArrowDown");
    assert.match(await selected.innerText(),/Biblioteca/);
    await page.keyboard.press("Escape");
    assert.equal(await searchButton.evaluate(button=>button===document.activeElement),true);
    assert.equal(await page.locator(".sidebar .nav-item[aria-current='page']").getAttribute("data-page"),"downloads");
    assert.ok(await page.locator(".sidebar").evaluate(sidebar => sidebar.scrollWidth <= sidebar.clientWidth + 1));
    await page.locator("[data-action='job-filter'][data-filter='error']").click();
    assert.equal(await page.locator(".job-row").count(),1);
    await page.locator("[data-action='job-filter'][data-filter='all']").click();
    await page.locator("#job-search").fill("Documento");assert.equal(await page.locator(".job-row").count(),1);
    checks.push("Queue filters and search preserve actual rows and actions.");
    await page.keyboard.press("Control+k");
    await query.fill("consulta tardia");
    await page.waitForFunction(()=>typeof window.sectionFixture.rejectSearch==="function");
    await query.fill("no hay coincidencia 1234");
    await palette.getByRole("status").getByText("0 resultados",{exact:true}).waitFor();
    await page.evaluate(()=>window.sectionFixture.rejectSearch());
    assert.equal(await palette.getByRole("status").innerText(),"0 resultados");
    assert.equal(await query.getAttribute("aria-activedescendant"),null);
    await palette.getByRole("button",{name:"Borrar búsqueda",exact:true}).click();
    assert.equal(await query.inputValue(),"");
    await query.fill("traducir subtitulos");
    assert.match(await selected.innerText(),/Traducir subtítulos/);
    await page.keyboard.press("Control+k");
    assert.equal(await query.evaluate(input=>input.selectionEnd-input.selectionStart), "traducir subtitulos".length);
    await query.press("Enter");
    await page.locator('#local-tool-translation[aria-pressed="true"]').waitFor();
    assert.equal(await page.locator(".tool-workspace h2").first().evaluate(heading=>heading===document.activeElement),true);
    await page.keyboard.press("Control+k");
    await query.fill("extraer audio");
    await query.press("Enter");
    await page.locator("#tool-audio-format").waitFor();
    await page.keyboard.press("Control+k");
    await query.fill("archivos personales");
    assert.match(await selected.innerText(),/Archivos personales/);
    await query.press("Enter");
    await page.locator('[data-action="library-section"][data-section="archives"][aria-current="page"]').waitFor();
    checks.push("Global search supports pointer and keyboard selection, accent-free tool names, direct destinations, and stale search isolation.");
    await page.locator('[data-page="tools"]').click();
    await page.getByRole("button",{name:"Ver categorías",exact:true}).click();
    assert.equal(await page.locator(".tool-catalog-item:visible").count(),7);
    assert.equal(await page.locator(".tool-studio").isVisible(),false);
    await page.locator(".tool-catalog-item[data-id='image']").press("Enter");
    await page.locator(".tool-catalog-panel:visible").getByRole("button",{name:"Eliminar fondo"}).waitFor();
    assert.equal(await page.locator(".tool-studio").isVisible(),false);
    await page.locator("#tool-search").fill("traducir subtitulos");
    assert.equal(await page.locator(".tool-catalog-item[data-id='subtitles']").isVisible(),true);
    assert.equal(await page.locator(".tool-catalog-item:visible").count(),1);
    await page.locator("#tool-search").fill("Extraer audio");
    assert.equal(await page.locator(".tool-catalog-item:visible").count(),1);
    const audioCategory=page.locator(".tool-catalog-item[data-id='audio']");
    await audioCategory.focus();await page.keyboard.press("Enter");
    await page.locator(".tool-catalog-panel:visible").getByRole("button",{name:"Extraer audio"}).press("Enter");
    await page.locator("#tool-audio-format").waitFor();
    assert.equal(await page.locator(".tool-directory [data-tool-category='video']").isVisible(),false);
    await page.getByRole("button",{name:"Ver categorías",exact:true}).click();
    await page.locator(".tool-catalog-panel:visible").getByRole("button",{name:"Extraer audio"}).click();
    await page.locator("#tool-audio-format").waitFor();
    await page.locator("[data-action='pick-tool-library']").click();
    await page.locator('[data-tool-input="clip"]').check();
    await page.locator("[data-action='finish-tool-picker']").click();
    assert.equal(await page.locator(".tool-inputs li").count(),1);
    assert.equal(await page.locator("#tool-form button[type=submit]").isEnabled(),true);
    checks.push("Tool category search, keyboard activation, return to categories, library input selection and options passed.");
    for(const module of ['imageAssets','fileOrganization','portable']){
      await openTool(page,module);
      const heading=page.locator('.tool-workspace h2').first();
      assert.ok(await heading.evaluate(element=>Number.parseFloat(getComputedStyle(element).fontSize))>=18,'Module titles must keep their visual hierarchy.');
    }
    checks.push("Specialized tool titles preserve their hierarchy in the actual application shell.");
    await page.locator('[data-page="platforms"]').click();
    await page.locator("[data-action='platform-group'][data-id='files']").click();
    assert.equal(await page.locator(".platform-row[data-id='github']").count(),1);
    assert.equal(await page.locator(".platform-row[data-id='youtube']").count(),0);
    await page.locator("#platform-search").fill("Dropbox");assert.equal(await page.locator(".platform-row").count(),1);
    checks.push("Platform categories and name search retain their source actions.");
    await page.locator('[data-page="settings"]').click();
    await page.locator("[data-action='settings-section'][data-section='appearance']").click();
    await page.locator("#reduced-transparency").check();
    await page.locator("#appearance-settings button[type=submit]").click();
    assert.equal(await page.evaluate(()=>window.sectionFixture.state.settings.reducedTransparency),true);
    await sidebar.getByRole("button", { name: "Ajustes" }).hover();
    await sidebarWidth(200);
    assert.equal(await sidebar.evaluate(node => getComputedStyle(node).backgroundColor), "rgb(14, 16, 19)");
    assert.equal(await sidebar.evaluate(node => getComputedStyle(node).backdropFilter), "none");
    await page.mouse.move(420, 140);
    await page.keyboard.press("Control+k");
    assert.equal(await palette.evaluate(element=>getComputedStyle(element).backdropFilter),"none");
    await page.keyboard.press("Escape");
    await page.locator("[data-action='settings-section'][data-section='general']").click();
    await page.locator("#language").focus();assert.equal(await page.locator("#language").evaluate(el=>el===document.activeElement),true);
    checks.push("Settings navigation, persistent appearance preference and keyboard focus work.");
  }
  assert.deepEqual(errors,[]);
  await writeFile(`${directory}/${before?"baseline":"verification"}.json`,JSON.stringify({evidence:"Rendered UI with IPC fixtures",checks,browserErrors:errors},null,2)+"\n");
  console.log(before?"Captured existing sections at both desktop sizes.":checks.join("\n"));
}catch(error){
  console.error({checks,browserErrors:errors});
  await page.screenshot({path:`${directory}/failure.png`,animations:"disabled"}).catch(()=>{});
  throw error;
}finally{await browser.close();}
