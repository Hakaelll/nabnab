// Rendered UI and command contracts with IPC fixtures. This does not certify provider access.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }) });
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = process.env.NABNAB_FILE_SOURCES_ARTIFACTS || "artifacts/file-sources";
await mkdir(directory, { recursive: true });
const page = await browser.newPage({ viewport: { width: 1180, height: 780 } });
const errors = [];
page.on("pageerror", error => errors.push(error.message));
const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri=true; mockWindows("main");
  const copy=value=>structuredClone(value);
  const settings={folder:"C:/Downloads/nabnab-fixture",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
  const state={settings,jobs:[],library:[],collections:[],drafts:[],sources:[],rules:[],smartCollections:[],presets:[]};
  const savedPaper=(id,version,missing=false)=>({
    id,name:"Mi copia "+version+".pdf",path:"C:/Papers/"+id+".pdf",kind:"document",size:512,addedAt:1,
    duration:null,width:null,height:null,source:"https://arxiv.org/abs/2401.00001"+version,parentId:null,collections:[],tags:[],missing,
    details:{contentId:"arxiv:2401.00001"+version,creator:"Fixture Author",platform:"arXiv",sourceIds:[],sourceVersion:version,
      chapters:[],tracks:[],playbackPosition:0,favorite:false,bookmarks:[],clip:null}
  });
  state.library.push(savedPaper("local-v1","v1"),savedPaper("missing-v2","v2",true));
  state.sources.push({id:"follow-arxiv",name:"Artículos de prueba",url:"https://arxiv.org/search/?field=author&query=Fixture+Author",
    kind:"publicSearch",status:"ready",lastCheck:1,settings:{presetId:null,relativeFolder:"",intervalHours:null,automatic:false,pageSize:20},
    baseline:{inspected:2,total:2,complete:true,cursor:2,hasMore:false},
    items:[1,2].map(version=>({contentId:"arxiv:2401.00001v"+version,title:"A versioned paper",creator:"arXiv",
      url:"https://arxiv.org/abs/2401.00001v"+version,publishedAt:null,state:"review",reason:null,jobIds:[]}))
  });
  const fixture=window.fileSourceFixture={state,calls:[],pending:null};
  const personal={servers:[],references:[]};
  const stamp={size:4096,modified:"2026-11-07",etag:'"version-1"'};
  fixture.personalStamp=stamp;
  const connections=["drive","onedrive","dropbox"].map(provider=>({provider,configured:false,connected:false,accountId:null,scope:provider==="drive"?"drive.readonly":provider==="onedrive"?"Files.Read offline_access":"files.metadata.read files.content.read sharing.read account_info.read",redirectUri:"http://127.0.0.1:53687/callback"}));
  const asset=(id,filename,extra={})=>({id,sourceUrl:"https://github.com/example/nabnab-fixture/releases/tag/v1.0",downloadUrl:"https://example.org/"+filename,filename,mime:"application/zip",size:1048576,title:filename,version:"v1.0",original:true,author:null,license:null,attribution:null,sha256:null,platform:null,architecture:null,...extra});
  const sourcePage=(url,extra={})=>({provider:"github",sourceUrl:url,title:"nabnab-fixture",description:"Public source fixture. No remote provider access is represented.",rights:null,entries:[],assets:[],nextCursor:null,warnings:[],...extra});
  const historyUrl="https://zenodo.org/api/records?q=conceptrecid%3A1234&all_versions=true";
  const dataset=(url,id,version)=>sourcePage(url,{provider:"zenodo",title:"Fixture dataset",
    zenodo:{recordId:String(id),familyId:"1234",version,versionsUrl:historyUrl},
    assets:[
      asset("zenodo:"+id+":samples.csv","samples.csv",{sourceUrl:"https://zenodo.org/records/"+id,mime:"text/csv",size:2**31,version,checksum:"md5:"+"a".repeat(32)}),
      asset("zenodo:"+id+":appendix.zip","appendix.zip",{mime:"application/zip",size:null,version}),
      asset("zenodo:"+id+":readme.txt","readme.txt",{mime:"text/plain",size:2048,version}),
    ]});
  const native=(id="doc",name="Project notes")=>({selection:{provider:"drive",accountId:"fixture-account",id,exportMime:null,version:"v1"},name,kind:"document",mime:"application/vnd.google-apps.document",size:null,location:"My Drive/Documents",exports:[{mime:"application/pdf",extension:"pdf",label:"PDF"},{mime:"application/vnd.openxmlformats-officedocument.wordprocessingml.document",extension:"docx",label:"Word (.docx)"}],downloadable:true});
  const binary=(id,name)=>({selection:{provider:"drive",accountId:"fixture-account",id,exportMime:null,version:"v1"},name,kind:"file",mime:"application/pdf",size:2048,location:"My Drive/Documents",exports:[],downloadable:true});
  mockIPC(async(cmd,args={})=>{
    fixture.calls.push({cmd,args:copy(args)});
    if(cmd==="plugin:window|is_maximized")return false;
    if(cmd==="startup")return {...copy(state),folder:settings.folder,enginesReady:true,engineError:null};
    if(cmd==="get_snapshot")return copy(state);
    if(cmd==="local_timezone")return "Romance Standard Time";
    if(cmd==="librivox_catalog_languages")return [{id:5,name:"Spanish"},{id:1,name:"English"},{id:48,name:"North American Indian (undetermined dialect)"}];
    if(cmd==="cloud_status")return copy(connections);
    if(cmd==="cloud_configure"){const connection=connections.find(value=>value.provider===args.provider);connection.configured=true;return copy(connection);}
    if(cmd==="cloud_connect"){const connection=connections.find(value=>value.provider===args.provider);connection.connected=true;connection.accountId="fixture-account";return copy(connection);}
    if(cmd==="cloud_revoke"){const connection=connections.find(value=>value.provider===args.provider);connection.connected=false;connection.accountId=null;return;}
    if(cmd==="cloud_browse")return {entries:args.folder?[binary("nested","Inside folder.pdf")]:[{selection:{provider:"drive",accountId:"fixture-account",id:"folder",exportMime:null,version:null},name:"Documents",kind:"folder",mime:"application/vnd.google-apps.folder",size:null,location:"My Drive",exports:[],downloadable:false},native(),binary("pdf","Original.pdf")],nextCursor:null,partial:false,warnings:[]};
    if(cmd==="cloud_preview_folder")return {entries:[binary("scan","Scan.pdf"),native("nested-doc","Nested document")],nextCursor:null,partial:true,warnings:["Fixture folder limit reached. Only reviewed files are included."]};
    if(cmd==="personal_servers_status")return copy(personal);
    if(cmd==="personal_server_webdav"){personal.servers.push({id:"dav-fixture",name:args.name,kind:"webdav",location:args.root});return copy(personal);}
    if(cmd==="personal_server_choose_key")return "C:/fixture/id_ed25519";
    if(cmd==="personal_server_probe_sftp")return {challenge:args.requestId,fingerprint:"SHA256:fixture-fingerprint"};
    if(cmd==="personal_server_trust_sftp"){personal.servers.push({id:"sftp-fixture",name:args.name,kind:"sftp",location:"sftp.example:22/media"});return copy(personal);}
    if(cmd==="personal_server_nas")return null;
    if(cmd==="personal_server_browse")return args.path.length
      ? (fixture.personalStamp?[{path:["Docs","notes.pdf"],directory:false,stamp:copy(fixture.personalStamp),mime:"application/pdf"}]:[])
      : [{path:["Docs"],directory:true,stamp:{size:null,modified:null,etag:null},mime:null}];
    if(cmd==="personal_server_index"){personal.references=[{id:"ref-fixture",serverName:"Estudio",selection:{serverId:args.serverId,path:args.paths[0],stamp:copy(fixture.personalStamp)}}];return copy(personal);}
    if(cmd==="personal_server_copy")return [];
    if(cmd==="personal_server_remove"){personal.servers=personal.servers.filter(server=>server.id!==args.id);return copy(personal);}
    if(cmd==="queue_public_files"||cmd==="queue_cloud_files")return [];
    if(cmd==="public_source_inspect"){
      if(args.url==="https://librivox.org/search/?language_id=5"){
        const edition=args.cursor?"Cuentos (segunda edición)":"Cuentos de prueba";
        return sourcePage(args.url,{provider:"librivox",title:"LibriVox · Spanish",
          entries:[{id:"librivox:catalog:"+(args.cursor?"cuentos-2":"cuentos"),title:edition,url:"https://librivox.org/cuentos/",prerelease:false,publishedAt:null}],
          nextCursor:args.cursor?null:"2"});
      }
      if(args.url.startsWith("https://arxiv.org/search/")){
        const paper = args.cursor
          ? {id:"arxiv:2401.00002v1",title:"A second paper",url:"https://arxiv.org/abs/2401.00002/error"}
          : {id:"arxiv:2401.00001v2",title:"A versioned paper",url:"https://arxiv.org/abs/2401.00001"};
        return sourcePage(args.url,{provider:"arxiv",title:"arXiv · fixture search",entries:[{...paper,prerelease:false,publishedAt:"2024-01-02"}],nextCursor:args.cursor?null:"2"});
      }
      if(args.url.includes("/error"))throw "Fixture source permission denied. Choose another source or retry.";
      if(args.url.startsWith("https://arxiv.org/abs/"))return sourcePage(args.url,{provider:"arxiv",title:"A versioned paper",assets:[1,2].map(version=>asset("arxiv:2401.00001v"+version,"arxiv-2401.00001v"+version+".pdf",{mime:"application/pdf",size:null,version:"v"+version,publishedAt:"2024-01-02",author:"Fixture Author"}))});
      if(args.url==="10.5281/zenodo.1234")return dataset("https://doi.org/10.5281/zenodo.1234",1236,"v2");
      if(args.url===historyUrl)return sourcePage(historyUrl,{provider:"zenodo",title:"Fixture dataset",
        entries:[{id:"zenodo:1236",title:"v2 · Fixture dataset",url:"https://zenodo.org/records/1236",prerelease:false,publishedAt:"2024-02-02"},{id:"zenodo:1235",title:"v1 · Fixture dataset",url:"https://zenodo.org/records/1235",prerelease:false,publishedAt:"2024-01-02"}]});
      if(args.url==="https://zenodo.org/records/1235")return dataset(args.url,1235,"v1");
      if(args.url==="https://librivox.org/cuentos/"||args.url.includes("librivox.org/api/feed/audiobooks/id/"))return sourcePage("https://librivox.org/api/feed/audiobooks/id/52",{provider:"librivox",title:"Fixture audiobook",assets:[1,2,3].map(chapter=>asset("librivox:52:"+chapter,"chapter-"+chapter+".mp3",{mime:"audio/mpeg",title:"Chapter title",audiobook:{bookId:"librivox:52",bookTitle:"Fixture audiobook",chapter,reader:chapter===2?"Second Reader":"Fixture Reader",language:chapter===2?"French":"Spanish"}}))});
      if(args.url.includes("/slow"))return await new Promise(resolve=>{fixture.pending=()=>resolve(sourcePage(args.url,{title:"Stale result must not appear"}));});
      if(args.url.includes("/empty"))return sourcePage(args.url,{title:"Empty release"});
      if(args.url.includes("archive.org"))return sourcePage(args.url,{provider:"archive",title:"A sample archive item",rights:"Rights supplied by uploader, permission must be checked.",assets:[asset("ia:original","scan.pdf",{mime:"application/pdf"}),asset("ia:derived","scan_text.txt",{mime:"text/plain",original:false})]});
      if(args.url.includes("commons.wikimedia.org"))return sourcePage(args.url,{provider:"commons",title:"File:Fixture image.jpg",assets:[asset("commons:image","Fixture image.jpg",{mime:"image/jpeg",author:"<img src=x onerror=alert(1)>",license:"CC BY 4.0",sha256:"a".repeat(64)})]});
      if(args.cursor==="files2")return sourcePage(args.url,{title:"nabnab-fixture · v1.0",assets:[asset("gh:notes","Release notes.md",{mime:"text/markdown",size:800})]});
      if(args.url.includes("/releases/tag/"))return sourcePage(args.url,{title:"nabnab-fixture · v1.0",assets:[asset("gh:win","nabnab-windows-x64.zip",{platform:"Windows",architecture:"x64",sha256:"a".repeat(64)}),asset("gh:readme","README.txt",{mime:"text/plain",size:500})],nextCursor:"files2"});
      if(args.cursor==="releases2")return sourcePage(args.url,{entries:[{id:"v09",title:"v0.9",url:args.url+"/releases/tag/v0.9",prerelease:false,publishedAt:"2026-09-01"}]});
      return sourcePage(args.url,{entries:[{id:"v1",title:"v1.0",url:args.url+"/releases/tag/v1.0",prerelease:false,publishedAt:"2026-11-01"},{id:"v2",title:"v2.0-preview",url:args.url+"/releases/tag/v2.0-preview",prerelease:true,publishedAt:"2026-11-04"}],nextCursor:"releases2"});
    }
    if(cmd==="cancel_file_source_inspection"){fixture.pending?.();fixture.pending=null;return;}
    if(cmd==="engine_status"||cmd==="sessions_status"||cmd==="optional_components"||cmd==="list_courses")return [];
    if(cmd==="cache_status")return {bytes:0,files:0};
    if(cmd==="telegram_status")return {installed:false,accounts:[]};
    if(cmd==="torrent_status")return {installed:false,enabled:false,version:null};
    if(cmd==="bridge_status")return {enabled:false,clients:[],port:47883};
  },{shouldMockEvents:true});
  await import("/src/main.ts");
`;
await page.route(base, async route => {
  const response = await route.fetch();
  const html = (await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/, `<script type="module">${setup}</script>`);
  await route.fulfill({ response, body: html });
});
const report = [];
async function inspect(url) {
  await page.locator("#file-source-url").fill(url);
  await page.locator("#file-source-form button[type=submit]").click();
}
async function checkNoHorizontalOverflow(label, selector = ".file-sources") {
  const dimensions = await page.locator(selector).evaluate(element => ({ width: element.clientWidth, scroll: element.scrollWidth }));
  assert.ok(dimensions.scroll <= dimensions.width + 1, `${label}: horizontal overflow ${JSON.stringify(dimensions)}`);
}
try {
  await page.goto(base);
  await page.locator("#url").waitFor();
  await page.locator("[data-page='platforms']").click();
  await page.getByRole("heading", { name: "Plataformas", exact: true }).waitFor();
  if (process.argv.includes("--baseline")) {
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({ width, height });
      await page.screenshot({ path: `${directory}/platforms-before-${width}x${height}.png` });
    }
  } else {
    await page.locator("[data-action='open-fileSources']").first().click();
    await page.locator("#file-source-url").waitFor();
    assert.equal(await page.locator("[data-action='file-source-queue']").count(), 0);
    await inspect("https://github.com/example/nabnab-fixture");
    await page.locator("[data-action='file-source-entry'][data-id='v1']").waitFor();
    assert.match(await page.locator(".file-source-entries").innerText(), /Estable/);
    assert.match(await page.locator(".file-source-entries").innerText(), /Prerelease/);
    await page.locator("[data-action='file-source-more']").click();
    await page.locator("[data-action='file-source-entry'][data-id='v09']").waitFor();
    await page.locator("[data-action='file-source-entry'][data-id='v1']").click();
    await page.locator("[data-file-source-select='gh:win']").waitFor();
    assert.equal(await page.locator("[data-file-source-select]:checked").count(), 0);
    assert.equal(await page.locator("[data-action='file-source-queue']").isDisabled(), true);
    await page.locator("[data-file-source-select='gh:win']").check();
    await page.locator("[data-action='file-source-more']").click();
    await page.locator("[data-file-source-select='gh:notes']").waitFor();
    assert.equal(await page.locator("[data-file-source-select='gh:win']").isChecked(), true);
    await page.locator("[data-file-source-select='gh:notes']").check();
    await page.locator("#file-source-collection").fill("Release fixtures");
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({ width, height });
      await checkNoHorizontalOverflow(`release ${width}`);
      await page.screenshot({ path: `${directory}/release-selection-${width}x${height}-fixture.png` });
    }
    await page.locator("[data-action='file-source-queue']").click();
    await page.waitForFunction(() => window.fileSourceFixture.calls.some(call => call.cmd === "queue_public_files"));
    const publicQueue = await page.evaluate(() => window.fileSourceFixture.calls.find(call => call.cmd === "queue_public_files").args);
    assert.deepEqual(publicQueue.assetIds, ["gh:win", "gh:notes"]);
    assert.equal(publicQueue.collection, "Release fixtures");
    assert.match(publicQueue.sourceUrl, /\/releases\/tag\/v1\.0$/);
    report.push("Release version navigation, prerelease labeling, pagination, explicit selection and queue payload passed.");

    await inspect("https://archive.org/details/fixture");
    await page.locator("#file-source-kind").waitFor();
    await page.locator("#file-source-kind").selectOption("original");
    assert.equal(await page.locator("[data-file-source-select]").count(), 1);
    await page.locator("#file-source-kind").selectOption("derivative");
    assert.equal(await page.locator("[data-file-source-select='ia:derived']").count(), 1);
    await inspect("https://commons.wikimedia.org/wiki/File:Fixture_image.jpg");
    await page.locator("[data-file-source-select='commons:image']").waitFor();
    assert.equal(await page.locator(".file-source-list img").count(), 0);
    assert.match(await page.locator(".file-source-list").innerText(), /<img src=x onerror=alert\(1\)>/);
    report.push("Archive originals and derivatives filters passed; Commons author metadata renders as text.");

    const dismissMessage = page.locator("#notification [data-action='dismiss-message']");
    if (await dismissMessage.isVisible()) await dismissMessage.click();
    await page.locator("[data-action='file-source-search-mode'][data-id='arxiv']").click();
    await page.locator("#file-source-arxiv-field").selectOption("author");
    await page.locator("#file-source-catalog-query").fill("Fixture Author");
    await page.locator("#file-source-catalog-form button[type=submit]").click();
    await page.getByRole("button", { name: "Seguir búsqueda" }).waitFor();
    await page.getByRole("button", { name: "Cargar más" }).click();
    const firstPaper = page.getByRole("button", { name: /A versioned paper/ });
    const secondPaper = page.getByRole("button", { name: /A second paper/ });
    await secondPaper.waitFor();
    await firstPaper.press("Enter");
    await page.locator("[data-file-source-select='arxiv:2401.00001v2']").waitFor();
    assert.equal(await page.getByRole("heading", { name: "A versioned paper" }).evaluate(node => node === document.activeElement), true);
    assert.deepEqual(await page.locator(".file-source-row-main strong").allTextContents(), ["Versión 1", "Versión 2"]);
    assert.equal(await page.locator("[data-file-source-select]:checked").count(), 0);
    const abstract = page.locator(".file-source-abstract");
    await abstract.locator("summary").press("Enter");
    assert.equal(await abstract.evaluate(node => node.open), true);
    await abstract.locator("summary").press("Enter");
    await page.getByRole("heading", { name: "Versiones del PDF" }).click();
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({ width, height });
      await page.locator(".file-source-list").scrollIntoViewIfNeeded();
      await checkNoHorizontalOverflow(`arXiv ${width}`);
      await page.screenshot({ path: `${directory}/arxiv-versions-${width}x${height}-fixture.png` });
    }
    const localCopy = page.getByRole("button", { name: "Abrir copia de la versión 1" });
    await localCopy.press("Enter", { timeout: 3000 });
    assert.equal(await page.getByRole("button", { name: "Abrir copia de la versión 2" }).count(), 0);
    const localOpen = await page.evaluate(() => window.fileSourceFixture.calls.find(call => call.cmd === "open_media")?.args);
    assert.deepEqual(localOpen, { id: "local-v1", folder: false });
    await page.getByRole("checkbox", { name: /^Versión 2 · / }).check();
    assert.match(await page.locator(".file-source-selection-title").innerText(), /Tamaño desconocido/);
    const back = page.getByRole("button", { name: "Volver a resultados" });
    const inspectCount = () => page.evaluate(() => window.fileSourceFixture.calls.filter(call => call.cmd === "public_source_inspect").length);
    const callsBeforeBack = await inspectCount();
    await back.press("Enter");
    assert.equal(await inspectCount(), callsBeforeBack);
    assert.equal(await firstPaper.evaluate(node => node === document.activeElement), true);
    await secondPaper.press("Enter");
    await page.getByRole("alert").filter({ hasText: "Fixture source permission denied" }).waitFor();
    assert.equal(await back.evaluate(node => node === document.activeElement), true);
    await back.press("Enter");
    assert.equal(await secondPaper.evaluate(node => node === document.activeElement), true);

    await page.locator('[data-page="library"]').click();
    await page.locator('[data-section="sources"]').click();
    const followed = page.getByRole("button", { name: /Artículos de prueba/ });
    assert.match(await followed.innerText(), /1 pendiente de revisión/);
    await followed.click();
    const versionRow = version => page.locator(".source-item").filter({ hasText: `2401.00001v${version}` });
    assert.match(await versionRow(1).innerText(), /Descargado/);
    assert.match(await versionRow(2).innerText(), /Pendiente de revisar/);
    await page.locator("#source-item-filter").selectOption("downloaded");
    assert.equal(await page.locator(".source-item").count(), 1);
    assert.equal(await versionRow(1).count(), 1);
    await versionRow(1).getByRole("button", { name: "Abrir copia" }).press("Enter");
    assert.deepEqual(await page.evaluate(() => window.fileSourceFixture.calls.filter(call => call.cmd === "open_media").map(call => call.args.id)), ["local-v1", "local-v1"]);
    await page.locator("#source-item-filter").selectOption("review");
    await versionRow(2).getByRole("button", { name: "Elegir archivos" }).press("Enter");
    await page.locator("[data-file-source-select='arxiv:2401.00001v2']").waitFor();
    assert.equal(await page.evaluate(() => window.fileSourceFixture.calls.filter(call => call.cmd === "public_source_inspect").at(-1).args.url), "https://arxiv.org/abs/2401.00001v2");

    await inspect("10.5281/zenodo.1234");
    await page.getByText("Versión v2 · Registro 1236 · Familia 1234", { exact: true }).waitFor();
    await page.locator("[data-file-source-select='zenodo:1236:samples.csv']").check();
    assert.match(await page.locator(".file-source-selection").innerText(), /2\.00 GiB/);
    assert.match(await page.locator(".file-source-list").innerText(), /md5:[a-f0-9]{32}/);
    const fileQuery = page.getByRole("searchbox", { name: "Filtrar archivos cargados" });
    await fileQuery.fill("appendix");
    const appendix = page.locator("[data-file-source-select='zenodo:1236:appendix.zip']");
    await appendix.check();
    assert.match(await page.locator(".file-source-selection").innerText(), /2\.00 GiB \+ tamaño pendiente/);
    assert.equal(await page.locator(".file-source-list input:checked").count(), 1);
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({ width, height });
      await page.locator(".file-source-filter").evaluate(element => element.scrollIntoView({ block: "start" }));
      await page.screenshot({ path: `${directory}/zenodo-filter-${width}x${height}-fixture.png` });
    }
    const review = page.getByRole("button", { name: "Solo seleccionados" });
    await review.press("Space", { timeout: 3000 });
    assert.equal(await review.getAttribute("aria-pressed"), "true");
    assert.equal(await fileQuery.isDisabled(), true);
    assert.equal(await page.locator(".file-source-list input").count(), 2);
    assert.equal(await page.locator(".file-source-list input:checked").count(), 2);
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({ width, height });
      await checkNoHorizontalOverflow(`Zenodo ${width}`);
      await page.locator(".file-source-selection").evaluate(element => element.scrollIntoView({ block: "end" }));
      await page.screenshot({ path: `${directory}/zenodo-${width}x${height}-fixture.png` });
    }
    await appendix.press("Space");
    assert.equal(await review.evaluate(element => element === document.activeElement), true);
    assert.match(await page.locator(".file-source-selection").innerText(), /1 seleccionado.*2\.00 GiB/s);
    assert.doesNotMatch(await page.locator(".file-source-selection").innerText(), /tamaño pendiente/);
    await review.press("Enter");
    assert.equal(await fileQuery.inputValue(), "appendix");
    assert.equal(await fileQuery.isEnabled(), true);
    assert.equal(await appendix.isChecked(), false);
    await review.press("Enter");
    await page.locator("[data-file-source-select='zenodo:1236:samples.csv']").press("Space");
    await page.getByText("No hay archivos seleccionados.", { exact: true }).waitFor();
    assert.equal(await page.locator("[data-action='file-source-queue']").isDisabled(), true);
    await page.getByRole("button", { name: "Historial de versiones" }).click();
    const oldVersion = page.locator("[data-action='file-source-entry'][data-id='zenodo:1235']");
    await oldVersion.click();
    await page.getByText("Versión v1 · Registro 1235 · Familia 1234", { exact: true }).waitFor();
    assert.equal(await review.getAttribute("aria-pressed"), "false");
    const exactFile = page.locator("[data-file-source-select='zenodo:1235:samples.csv']");
    assert.equal(await exactFile.isChecked(), false);
    await exactFile.check();
    await page.locator("[data-action='file-source-queue']").click();
    await page.waitForFunction(() => window.fileSourceFixture.calls.some(call => call.cmd === "queue_public_files" && call.args.sourceUrl === "https://zenodo.org/records/1235" && call.args.assetIds.join() === "zenodo:1235:samples.csv"));
    await page.getByRole("button", { name: "Volver a versiones" }).click();
    assert.equal(await oldVersion.evaluate(node => node === document.activeElement), true);

    await page.locator("[data-action='file-source-search-mode'][data-id='librivox']").click();
    const language = page.getByRole("combobox", { name: "Idioma", exact: true });
    await language.selectOption({ label: "Español" });
    assert.equal(await page.locator("#file-source-catalog-query").inputValue(), "");
    await page.locator("#file-source-catalog-form button[type=submit]").click();
    const edition = page.getByRole("button", { name: "Cuentos de prueba", exact: true });
    await edition.waitFor();
    await page.getByRole("button", { name: "Cargar más" }).click();
    await page.getByRole("button", { name: "Cuentos (segunda edición)", exact: true }).waitFor();
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({ width, height });
      await page.locator("#file-source-catalog-form").scrollIntoViewIfNeeded();
      await checkNoHorizontalOverflow(`LibriVox language ${width}`);
      await page.screenshot({ path: `${directory}/librivox-language-${width}x${height}-fixture.png` });
    }
    await edition.press("Enter");
    await page.locator("[data-file-source-select='librivox:52:1']").waitFor();
    await page.getByRole("button", { name: "Volver a resultados" }).click();
    assert.equal(await edition.evaluate(node => node === document.activeElement), true);
    await edition.press("Enter");
    await page.locator("[data-file-source-select='librivox:52:1']").waitFor();
    const chapterLanguage = page.getByRole("combobox", { name: "Idioma del capítulo", exact: true });
    assert.equal(await chapterLanguage.inputValue(), "Spanish");
    assert.deepEqual(await page.locator(".file-source-research-list .file-source-file-icon").allTextContents(), ["01", "03"]);
    assert.equal(await page.locator(".file-source-filter [aria-live]").textContent(), "2 de 3 capítulos cargados");
    assert.equal(await page.getByRole("checkbox", { name: "Capítulo 1 · Chapter title", exact: true }).count(), 1);
    assert.equal(await page.getByRole("checkbox", { name: "Capítulo 2 · Chapter title", exact: true }).count(), 0);
    assert.match(await page.locator(".file-source-list").innerText(), /Fixture Reader · Español/);
    const narration = page.getByRole("combobox", { name: "Lectura" });
    await narration.focus();
    await narration.selectOption("Fixture Reader");
    assert.equal(await narration.evaluate(select => select === document.activeElement), true);
    assert.deepEqual(await page.locator(".file-source-research-list .file-source-file-icon").allTextContents(), ["01", "03"]);
    await page.locator("[data-action='file-source-select-chapters']").click();
    assert.equal(await page.locator("[data-file-source-select]:checked").count(), 2);
    await chapterLanguage.focus();
    await chapterLanguage.selectOption({ label: "Francés" });
    assert.equal(await chapterLanguage.evaluate(select => select === document.activeElement), true);
    await page.getByText("No hay capítulos con estos filtros.", { exact: true }).waitFor();
    assert.equal(await page.locator(".file-source-filter [aria-live]").textContent(), "0 de 3 capítulos cargados");
    await narration.selectOption("Second Reader");
    assert.deepEqual(await page.locator(".file-source-research-list .file-source-file-icon").allTextContents(), ["02"]);
    assert.equal(await page.locator("[data-file-source-select]:checked").count(), 0);
    await chapterLanguage.selectOption({ label: "Español" });
    await narration.selectOption("Fixture Reader");
    assert.equal(await page.locator("[data-file-source-select]:checked").count(), 2);
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({ width, height });
      await page.locator(".file-source-list").scrollIntoViewIfNeeded();
      await checkNoHorizontalOverflow(`LibriVox ${width}`);
      await page.screenshot({ path: `${directory}/librivox-chapters-${width}x${height}-fixture.png` });
    }
    await page.locator("[data-action='file-source-queue']").click();
    await page.waitForFunction(() => window.fileSourceFixture.calls.some(call =>
      call.cmd === "queue_public_files" && call.args.sourceUrl.includes("librivox.org")));
    const queuedChapters = await page.evaluate(() => window.fileSourceFixture.calls.filter(call => call.cmd === "queue_public_files").at(-1).args.assetIds);
    assert.deepEqual(queuedChapters, ["librivox:52:1", "librivox:52:3"]);
    await page.getByRole("button", { name: "Volver a resultados" }).click();
    await page.getByRole("button", { name: "Seguir idioma" }).click();
    assert.equal(await page.locator("#source-url").inputValue(), "https://librivox.org/search/?language_id=5");
    assert.equal(await page.locator("#source-exploration-limit").getAttribute("step"), "25");
    await page.locator("[data-page='platforms']").click();
    await page.locator("[data-action='open-fileSources']").first().click();
    report.push("arXiv author search, version selection and keyboard return passed. Zenodo DOI with reviewed volume and LibriVox chapter selection by language and narrator passed.");

    await inspect("https://github.com/example/empty");
    await page.getByRole("heading", { name: "Empty release", exact: true }).waitFor();
    assert.equal(await page.locator("[data-file-source-select]").count(), 0);
    await inspect("https://github.com/example/error");
    await page.locator(".file-source-error").waitFor();
    assert.match(await page.locator(".file-source-error").innerText(), /permission denied/);
    assert.equal(await page.locator(".file-source-results").count(), 0);
    await inspect("https://github.com/example/slow");
    await page.locator("[data-action='file-source-cancel']").click();
    await page.waitForFunction(() => window.fileSourceFixture.calls.some(call => call.cmd === "cancel_file_source_inspection"));
    assert.equal(await page.getByRole("heading", { name: "Stale result must not appear" }).count(), 0);
    report.push("Empty, error and cancellation states passed; canceled results do not replace the current page.");

    await page.locator("[data-action='file-source-tab'][data-tab='drive']").click();
    assert.equal(await page.locator("[data-action='cloud-connect'][data-provider='drive']").isDisabled(), true);
    await page.locator("[data-action='cloud-configure-open'][data-provider='drive']").last().click();
    await page.locator("#cloud-client-id").fill("fixture-desktop-client");
    await page.setViewportSize({ width: 1180, height: 780 });
    await page.locator(".cloud-connection-heading").scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${directory}/cloud-oauth-setup-1180x780-fixture.png` });
    await page.locator("#cloud-configure-form button[type=submit]").click();
    await page.locator("[data-action='cloud-connect'][data-provider='drive']").click();
    await page.locator("[data-action='cloud-connect'][data-provider='drive']").waitFor({ state: "detached" });
    await page.locator("[data-action='file-source-cloud-root']").click();
    await page.locator("[data-cloud-source-select='doc']").waitFor();
    const formats = await page.locator("[data-cloud-source-export='doc'] option").allTextContents();
    assert.deepEqual(formats, ["PDF", "Word (.docx)"]);
    assert.equal(await page.locator("[data-cloud-source-export='pdf']").count(), 0);
    await page.locator("[data-cloud-source-export='doc']").selectOption("application/vnd.openxmlformats-officedocument.wordprocessingml.document");
    await page.locator("[data-cloud-source-select='doc']").check();
    await page.locator("[data-action='file-source-queue']").click();
    await page.waitForFunction(() => window.fileSourceFixture.calls.some(call => call.cmd === "queue_cloud_files"));
    const cloudQueue = await page.evaluate(() => window.fileSourceFixture.calls.find(call => call.cmd === "queue_cloud_files").args);
    assert.equal(cloudQueue.selections[0].exportMime, "application/vnd.openxmlformats-officedocument.wordprocessingml.document");
    assert.equal(cloudQueue.selections[0].accountId, "fixture-account");
    assert.equal(JSON.stringify(cloudQueue).includes("token"), false);
    await page.locator("[data-action='file-source-folder'][data-id='folder']").click();
    await page.locator("[data-cloud-source-select='nested']").waitFor();
    await page.locator("[data-action='file-source-parent']").click();
    await page.locator("[data-action='file-source-folder-preview'][data-id='folder']").click();
    await page.locator("[data-cloud-source-select='nested-doc']").waitFor();
    assert.match(await page.locator(".file-source-warnings").innerText(), /Only reviewed files/);
    assert.equal(await page.locator("[data-cloud-source-select]:checked").count(), 0);
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({ width, height });
      await page.locator(".file-source-result-heading").scrollIntoViewIfNeeded();
      await checkNoHorizontalOverflow(`cloud ${width}`);
      await page.screenshot({ path: `${directory}/cloud-folder-review-${width}x${height}-fixture.png` });
    }
    await page.locator("[data-action='cloud-configure-open'][data-provider='drive']").click();
    await page.locator("[data-action='cloud-revoke'][data-provider='drive']").click();
    await page.getByText("Aplicación configurada · autorización pendiente", { exact: true }).waitFor();
    assert.equal(await page.locator("[data-cloud-source-select]").count(), 0);
    report.push("Cloud OAuth setup, explicit browser authorization, MIME-specific native export, folder navigation, bounded batch preview and disconnect passed with IPC fixtures.");
    await page.locator("[data-page='settings']").click();
    await page.locator("[data-action='settings-section'][data-section='connections']").click();
    await page.locator("#settings-title").filter({hasText:"Conexiones de archivos"}).waitFor();
    assert.equal(await page.locator(".cloud-connection").count(),3);
    report.push("Settings file connections navigation and status rendering passed.");

    await page.getByRole("button", {name:"Nextcloud / WebDAV",exact:true}).click();
    await page.locator("#personal-name").fill("Estudio");
    await page.locator("#personal-root").fill("https://home.example:8443/remote.php/dav/files/reader/");
    await page.locator("#personal-username").fill("reader");
    await page.locator("#personal-password").fill("fixture-app-password");
    const certificate = page.locator("#personal-certificate");
    const trustSummary = page.locator("#personal-webdav-trust summary");
    assert.equal(await certificate.isVisible(), false);
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({ width, height });
      await page.locator("#personal-configure").scrollIntoViewIfNeeded();
      await checkNoHorizontalOverflow(`WebDAV setup ${width}`, "#personal-configure");
      await page.screenshot({ path: `${directory}/webdav-setup-${width}x${height}-fixture.png` });
    }
    await trustSummary.press("Enter");
    await certificate.waitFor();
    const pem = "-----BEGIN CERTIFICATE-----\nZml4dHVyZQ==\n-----END CERTIFICATE-----";
    await certificate.fill(pem);
    await checkNoHorizontalOverflow("WebDAV certificate editor", "#personal-configure");
    await page.locator("#personal-configure").scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${directory}/webdav-certificate-900x620-fixture.png` });
    await page.getByRole("button", {name:"Guardar conexión",exact:true}).click();
    assert.equal(await page.evaluate(() => window.fileSourceFixture.calls.find(call => call.cmd === "personal_server_webdav").args.certificatePem), pem);
    await page.getByRole("button", {name:/Estudio.*Nextcloud/}).click();
    await page.getByRole("button", {name:"Docs",exact:true}).click();
    await page.getByRole("checkbox", {name:/notes.pdf/}).check();
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({width,height});
      await page.locator(".personal-review").scrollIntoViewIfNeeded();
      await checkNoHorizontalOverflow(`Personal server ${width}`);
      await page.screenshot({path:`${directory}/personal-server-${width}x${height}-fixture.png`});
    }
    await page.getByRole("button", {name:"Indexar referencia",exact:true}).click();
    await page.getByRole("button", {name:"Referencias guardadas",exact:true}).click();
    await page.getByRole("button", {name:/notes.pdf.*Remota/}).click();
    await page.getByRole("button", {name:"Obtener copia local",exact:true}).click();
    await page.waitForFunction(() => window.fileSourceFixture.calls.some(call => call.cmd === "personal_server_copy"));
    const copyArgs = await page.evaluate(() => window.fileSourceFixture.calls.find(call => call.cmd === "personal_server_copy").args);
    assert.equal(copyArgs.referenceId,"ref-fixture");
    assert.equal(copyArgs.paths,null);
    assert(!JSON.stringify(copyArgs).includes("fixture-app-password"));
    await page.getByRole("button", {name:/notes.pdf.*Remota/}).click();
    await page.getByRole("button", {name:"Revisar en servidor",exact:true}).click();
    const comparison = page.locator(".personal-reference-review");
    await comparison.getByRole("status").filter({hasText:"Los datos visibles coinciden"}).waitFor();
    await page.waitForFunction(() => document.activeElement?.id === "personal-reference-review-title");
    assert.equal(await page.getByRole("checkbox", {name:/notes.pdf/}).isChecked(), true);
    await page.evaluate(() => { window.fileSourceFixture.personalStamp = { ...window.fileSourceFixture.personalStamp, etag:'"version-2"' }; });
    await page.getByRole("button", {name:"Actualizar",exact:true}).click();
    await comparison.getByRole("status").filter({hasText:"El identificador de versión es distinto."}).waitFor();
    assert.equal(await page.evaluate(() => window.fileSourceFixture.calls.filter(call => call.cmd==="personal_server_index").length), 1);
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({width,height});
      await comparison.scrollIntoViewIfNeeded();
      await checkNoHorizontalOverflow(`Personal reference review ${width}`);
      await page.screenshot({path:`${directory}/personal-reference-${width}x${height}-fixture.png`});
    }
    await page.getByRole("button", {name:"Actualizar referencia",exact:true}).click();
    await comparison.getByRole("status").filter({hasText:"Los datos visibles coinciden"}).waitFor();
    const updatedPaths = await page.evaluate(() => window.fileSourceFixture.calls.filter(call => call.cmd==="personal_server_index").at(-1).args.paths);
    assert.deepEqual(updatedPaths, [["Docs","notes.pdf"]]);
    await page.evaluate(() => { window.fileSourceFixture.personalStamp = null; });
    await page.getByRole("button", {name:"Actualizar",exact:true}).click();
    await comparison.getByRole("status").filter({hasText:"El archivo ya no aparece"}).waitFor();
    assert.equal(await page.getByRole("button", {name:"Actualizar referencia",exact:true}).count(), 0);
    await page.getByRole("button", {name:"Volver a referencias",exact:true}).click();
    await page.getByRole("button", {name:/notes.pdf.*Remota/}).waitFor();
    await page.locator("[data-page='settings']").click();
    await page.locator("[data-action='settings-section'][data-section='connections']").click();
    await page.locator("[data-action='personal-remove'][data-id='dav-fixture']").click();
    await page.locator("[data-action='personal-remove'][data-id='dav-fixture']").waitFor({state:"detached"});
    await page.getByRole("button", {name:"SFTP",exact:true}).click();
    await page.locator("#personal-name").fill("Servidor SSH");
    await page.locator("#personal-host").fill("sftp.example");
    await page.locator("#personal-username").fill("reader");
    await page.locator("#personal-root").fill("/media");
    await page.getByRole("button", {name:"Elegir clave",exact:true}).click();
    await page.getByRole("button", {name:"Comprobar huella",exact:true}).click();
    await page.getByText("SHA256:fixture-fingerprint",{exact:true}).waitFor();
    assert.equal(await page.evaluate(() => window.fileSourceFixture.calls.some(call => call.cmd==="personal_server_trust_sftp")),false);
    await page.getByRole("button", {name:"Confiar y guardar",exact:true}).click();
    await page.getByRole("button", {name:/Servidor SSH.*SFTP/}).waitFor();
    await page.getByRole("button", {name:"Carpeta de red / NAS",exact:true}).click();
    await page.locator("#personal-name").fill("Red");
    await page.getByRole("button", {name:"Elegir carpeta",exact:true}).click();
    await page.waitForFunction(() => window.fileSourceFixture.calls.some(call => call.cmd==="personal_server_nas"));
    assert.equal(await page.getByRole("button", {name:/^Red.*NAS/}).count(),0);
    await page.locator("[data-page='library']").click();
    await page.locator("[data-action='library-section'][data-section='references']").click();
    await page.getByRole("button", {name:/notes.pdf.*Servidor desconectado/}).click();
    assert.equal(await page.getByRole("button", {name:"Obtener copia local",exact:true}).isDisabled(),true);
    assert.equal(await page.getByRole("button", {name:"Revisar en servidor",exact:true}).isDisabled(),true);
    report.push("Personal server configuration, scoped browsing, explicit copy, changed/missing reference review, disconnected references, SFTP confirmation and NAS picker cancellation passed.");
    const documentFormats = await page.evaluate(async () => {
      const { createDocuments, documentReaderPanel } = await import("/src/documents-ui.ts");
      const commands = [];
      const items = ["report.docx", "README.md", "notes.txt", "slides.pptx"].map((name,index)=>({id:String(index),name,kind:"document",details:{bookmarks:[]}}));
      const t = (es,en) => en;
      const reader = createDocuments({snapshot:()=>({library:items}),command:async(name)=>{commands.push(name);throw new Error("Unsupported reader request");},refresh:async()=>{},render:()=>{},t});
      for(const item of items) await reader.open(item.id);
      return {commands, panels:items.map(item=>documentReaderPanel(item,t)), pdf:documentReaderPanel({id:"pdf",name:"original.pdf",kind:"document",details:{bookmarks:[]}},t)};
    });
    assert.deepEqual(documentFormats.commands,[]);
    assert(documentFormats.panels.every(panel=>panel.includes("Open it with the system")&&!panel.includes('data-action="document-open"')));
    assert(documentFormats.pdf.includes('data-action="document-open"'));
    report.push("Downloaded Office/text documents retain system opening without invoking the PDF/EPUB reader.");
    assert.deepEqual(errors, []);
    await writeFile(`${directory}/verification.json`, JSON.stringify({ evidence: "Rendered UI with IPC fixtures; no provider account or network access certified.", checks: report, browserErrors: errors }, null, 2) + "\n");
    console.log(report.join("\n"));
  }
} finally { await browser.close(); }
