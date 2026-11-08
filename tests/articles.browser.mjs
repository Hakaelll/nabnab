// Exercises the rendered article reader with explicit IPC fixtures. Native archiving is tested separately.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = process.env.NABNAB_ARTICLE_ARTIFACTS || "artifacts/articles";
await mkdir(directory, { recursive: true });
const browser = await chromium.launch({ headless: true, ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }) });
const results = [];
const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri=true; mockWindows("main");
  const copy=value=>structuredClone(value);
  const settings={folder:"C:/Fixture archives",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
  const details=version=>({contentId:null,creator:"Fixture author",platform:null,sourceIds:[],sourceUrl:"https://source.example/article",chapters:[],tracks:[],playbackPosition:0,favorite:false,bookmarks:[],clip:null,webArchiveRef:version,articleAnnotations:[]});
  const item=(id,title,version)=>({id,path:"C:/Fixture archives/"+id+".nabarticle.json",name:title,kind:"article",size:4096,duration:null,width:null,height:null,videoCodec:null,audioCodec:null,container:null,addedAt:1793968800000,source:"https://source.example/article",parentId:null,collections:["Reading fixtures"],tags:[],missing:false,details:details(version)});
  const state={settings,jobs:[],library:[item("article-v1","A local article", "a".repeat(64)),item("article-v2","A local article, version 2", "b".repeat(64)),item("article-error","Unavailable local article", "c".repeat(64)),item("article-slow","Loading local article", "d".repeat(64))],collections:["Reading fixtures"],smartCollections:[],sources:[],drafts:[],presets:[],rules:[]};
  state.library[0].details.articleAnnotations=[
    {id:"old-version",version:"0".repeat(64),block:2,quote:"repeated phrase",prefix:"First ",suffix:" end. Second repeated phrase finish.",note:"A note on another immutable version.",createdAt:1793968800000},
    {id:"ambiguous",version:"a".repeat(64),block:8,quote:"echo",prefix:"",suffix:"",note:"An ambiguous anchor must not move silently.",createdAt:1793968800000}
  ];
  const doc={schemaVersion:1,mode:"reading",title:"A local article",author:"Fixture author",url:"https://source.example/article",publishedAt:"2026-10-20T10:00:00Z",archivedAt:"2026-11-06T10:00:00Z",blocks:[
    {kind:"heading",level:2,text:"A passage worth keeping"},
    {kind:"paragraph",text:'This is an offline reading fixture. <script>window.articleXss=true</script> <img src="https://source.example/injected" onerror="window.articleXss=true">'},
    {kind:"paragraph",text:"First repeated phrase end. Second repeated phrase finish."},
    {kind:"quote",text:"A local copy preserves its own text and annotations."},
    {kind:"code",text:'<script src="https://source.example/code.js"></script>'},
    {kind:"listItem",text:"A keyboard-accessible annotation target."},
    {kind:"image",text:"A locally embedded PNG fixture",resourceId:"local-png"},
    {kind:"image",text:"Unpreserved remote resource",resourceId:"remote-image"},
    {kind:"paragraph",text:"echo echo"}
  ],resources:[{id:"local-png",dataUrl:"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jZ1kAAAAASUVORK5CYII=",width:1,height:1},{id:"remote-image",dataUrl:"https://source.example/image.png",width:100,height:100}],warnings:["Static fixture: interactive controls were omitted."],selectionContext:null};
  const fixture=window.articleFixture={state,calls:[],releaseSlow:null};
  window.articleXss=false;
  mockIPC(async(cmd,args={})=>{
    fixture.calls.push({cmd,args:copy(args)});
    if(cmd==="plugin:window|is_maximized")return false;
    if(cmd==="startup")return {...copy(state),folder:settings.folder,enginesReady:true,engineError:null};
    if(cmd==="get_snapshot")return copy(state);
    if(cmd==="local_timezone")return "Europe/Madrid";
    if(cmd==="archive_document"){
      if(args.id==="article-error")throw "The selected local archive is unavailable. Locate or recapture it.";
      if(args.id==="article-slow")return new Promise(resolve=>{fixture.releaseSlow=()=>resolve({...copy(doc),title:"Late article result"});});
      if(args.id==="article-v2")return {...copy(doc),mode:"selection",title:"A local article, version 2",archivedAt:"2026-11-06T11:00:00Z",blocks:[{kind:"paragraph",text:"A new capture has different text and no relocated annotations."},{kind:"paragraph",text:"🧩"+"x".repeat(119)+"marker"}],warnings:[],selectionContext:{prefix:"Before the selection <script> text",suffix:"After the selection"}};
      return copy(doc);
    }
    if(cmd==="archive_add_annotation"){
      const item=state.library.find(value=>value.id===args.id);
      item.details.articleAnnotations.push({id:"saved-note",version:item.details.webArchiveRef,block:args.block,quote:args.quote,prefix:args.prefix,suffix:args.suffix,note:args.note,createdAt:1793968800000});
      return copy(state);
    }
    if(cmd==="archive_remove_annotation"){
      const item=state.library.find(value=>value.id===args.id);item.details.articleAnnotations=item.details.articleAnnotations.filter(value=>value.id!==args.annotationId);return copy(state);
    }
    if(cmd==="queue_article_export")return [];
    if(cmd==="device_share_status")return {state:"ready",devices:[{id:"reader-fixture",name:"Lector de prueba"}],share:true,shareText:true};
    if(cmd==="device_share_prepare")return {token:"reader-share-token",device:{id:args.deviceId,name:"Lector de prueba"},entries:[{name:args.selection.text,bytes:15}],total:15};
    if(cmd==="engine_status"||cmd==="sessions_status"||cmd==="optional_components"||cmd==="list_courses")return [];
    if(cmd==="cache_status")return {bytes:0,files:0};
    if(cmd==="telegram_status")return {installed:false,accounts:[]};
    if(cmd==="torrent_status")return {installed:false,enabled:false,version:null};
    if(cmd==="cloud_status")return [];
    if(cmd==="bridge_status")return {enabled:false,clients:[],port:47883};
  },{shouldMockEvents:true});
  await import("/src/main.ts");
`;

try {
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    const page = await browser.newPage({ viewport: { width, height } });
    const errors = [], remoteRequests = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.route("https://source.example/**", route => { remoteRequests.push(route.request().url()); return route.abort(); });
    await page.route(base, async route => {
      const response = await route.fetch();
      const html = (await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/, `<script type="module">${setup.replace(/<\/script/gi, "<\\/script")}</script>`);
      await route.fulfill({ response, body: html });
    });
    const dialog = page.locator("#article-dialog");
    async function open(id) {
      if (await page.locator("#detail-dialog").isVisible()) await page.locator("#detail-dialog [data-action='close-inspector']").click();
      await page.locator(`[data-action='inspect-media'][data-id='${id}']`).click();
      await dialog.waitFor({ state: "visible" });
    }
    await page.goto(base);
    await page.locator("[data-page='library']").click();
    await open("article-v1");
    await dialog.locator("#article-reader-title").waitFor();
    assert.match(await dialog.innerText(), /20 oct|20 Oct/);
    assert.match(await dialog.innerText(), /6 nov|6 Nov/);
    assert.equal(await dialog.locator("script, iframe, form[action], img[src^='http']").count(), 0);
    assert.equal(await dialog.locator(".article-text img").count(), 1);
    assert.equal(await dialog.locator("mark").count(), 0, "Foreign-version and ambiguous anchors must not be highlighted.");
    assert.equal(await page.evaluate(() => window.articleXss), false);
    assert.match(await dialog.locator("[data-article-block='1']").innerText(), /<script>/);
    assert.match(await dialog.locator("[data-article-note='old-version']").innerText(), /otra versión/);
    assert.match(await dialog.locator("[data-article-note='ambiguous']").innerText(), /inequívoca/);
    const selectSecondPhrase = () => page.evaluate(() => {
      const block = document.querySelector("[data-article-block='2']"), node = block.firstChild;
      const text = "repeated phrase", start = block.textContent.lastIndexOf(text);
      const range = document.createRange(); range.setStart(node, start); range.setEnd(node, start + text.length);
      const selection = window.getSelection(); selection.removeAllRanges(); selection.addRange(range);
      document.dispatchEvent(new Event("selectionchange"));
    });
    await selectSecondPhrase();
    await dialog.locator("[data-action='article-share-selection']").click();
    const share = page.locator(".device-share[open]");
    const shareText = share.locator("#share-text-value");
    assert.equal(await shareText.inputValue(), "repeated phrase");
    await share.locator("#share-device").selectOption("reader-fixture");
    const maxText = "🙂".repeat(2048);
    await shareText.fill(maxText + "a");
    await share.screenshot({ path: `${directory}/share-limit-${width}.png` });
    assert.equal(await share.locator("#share-review").isDisabled(), true);
    assert.equal(await shareText.getAttribute("aria-invalid"), "true");
    assert.equal(await share.locator("#share-text-bytes").textContent(), "8193 / 8192 B");
    assert.equal(await page.evaluate(() => window.articleFixture.calls.some(call => call.cmd === "device_share_prepare")), false);
    await shareText.fill(maxText);
    assert.equal(await share.locator("#share-review").isEnabled(), true);
    assert.equal(await shareText.getAttribute("aria-invalid"), "false");
    assert.equal(await share.locator("#share-text-bytes").textContent(), "8192 / 8192 B");
    await shareText.fill("repeated phrase");
    await share.locator("#share-review").click();
    await share.locator("#share-send").waitFor();
    const request = await page.evaluate(() => window.articleFixture.calls.find(call => call.cmd === "device_share_prepare").args);
    assert.deepEqual(request.selection, { kind: "text", text: "repeated phrase" });
    await share.locator("#share-close").click();
    await share.waitFor({ state: "hidden" });
    await selectSecondPhrase();
    await dialog.locator("[data-action='article-use-selection']").click();
    await dialog.locator("#article-note").fill('<img src="https://source.example/note" onerror="window.articleXss=true"> A note for the second phrase.');
    await dialog.locator("#article-note-form button[type='submit']").click();
    await dialog.locator("mark[data-article-annotations='saved-note']").waitFor();
    const anchor = await page.evaluate(() => window.articleFixture.calls.find(call => call.cmd === "archive_add_annotation").args);
    assert.equal(anchor.id, "article-v1"); assert.equal(anchor.block, 2); assert.equal(anchor.quote, "repeated phrase");
    assert.equal(anchor.prefix, "First repeated phrase end. Second "); assert.equal(anchor.suffix, " finish.");
    assert.equal(await dialog.locator("[data-article-block='2'] mark").count(), 1);
    assert.equal(await dialog.locator("[data-article-block='2']").innerText(), "First repeated phrase end. Second repeated phrase finish.");
    assert.equal(await dialog.locator("[data-article-note='saved-note'] img").count(), 0);
    await dialog.locator("[data-action='article-go-note'][data-annotation='saved-note']").click();
    assert.equal(await dialog.locator("[data-article-block='2']").evaluate(element => element === document.activeElement), true);
    const dimensions = await dialog.evaluate(element => ({ width: element.clientWidth, scrollWidth: element.scrollWidth, height: element.clientHeight, viewport: window.innerHeight }));
    assert.ok(dimensions.scrollWidth <= dimensions.width + 1); assert.ok(dimensions.height < dimensions.viewport);
    await dialog.locator(".article-reader-content").evaluate(element => { element.scrollTop = 0; });
    await page.screenshot({ path: `${directory}/reader-${width}x${height}-fixture.png` });
    await dialog.locator(".article-reader-content").evaluate(element => { element.scrollTop = element.scrollHeight; });
    await page.screenshot({ path: `${directory}/notes-${width}x${height}-fixture.png` });
    for (const format of ["html", "epub", "md"]) await dialog.locator(`[data-action='article-export'][data-format='${format}']`).click();
    const exports = await page.evaluate(() => window.articleFixture.calls.filter(call => call.cmd === "queue_article_export").map(call => call.args));
    assert.deepEqual(exports.map(call => call.format), ["html", "epub", "md"]);
    assert.ok(exports.every(call => call.id === "article-v1" && typeof call.clientRequest === "string"));
    await page.keyboard.press("Escape");
    await dialog.waitFor({ state: "hidden" });
    await page.waitForFunction(() => document.activeElement?.matches("[data-action='article-open'][data-id='article-v1']"));
    if (width < 1120) assert.equal(await page.locator("#detail-dialog").evaluate(element => element.open), true, "Escape closes only the topmost reader dialog.");

    await open("article-v2");
    await dialog.getByText("A new capture has different text and no relocated annotations.", { exact: true }).waitFor();
    assert.equal(await dialog.locator("mark, [data-article-note]").count(), 0);
    await dialog.locator(".article-selection-context summary").click();
    assert.match(await dialog.locator(".article-selection-context").innerText(), /Before the selection <script> text/);
    await page.evaluate(() => {
      const node = document.querySelector("[data-article-block='1']").firstChild;
      const range = document.createRange(); range.setStart(node, node.textContent.indexOf("marker")); range.setEnd(node, node.textContent.length);
      const selection = window.getSelection(); selection.removeAllRanges(); selection.addRange(range); document.dispatchEvent(new Event("selectionchange"));
    });
    await dialog.locator("[data-action='article-use-selection']").click();
    await dialog.locator("#article-note-form button[type='submit']").click();
    await dialog.locator("mark[data-article-annotations='saved-note']").waitFor();
    const unicodeAnchor = await page.evaluate(() => window.articleFixture.calls.filter(call => call.cmd === "archive_add_annotation").at(-1).args);
    assert.equal(unicodeAnchor.prefix, "🧩" + "x".repeat(119));
    assert.equal(unicodeAnchor.suffix, "");
    await dialog.locator("[data-action='article-annotate-block'][data-block='0']").focus();
    await page.keyboard.press("Enter");
    await dialog.locator("#article-note").waitFor();
    assert.equal(await dialog.locator("#article-note").evaluate(element => element === document.activeElement), true);
    await dialog.locator("[data-action='article-cancel-note']").click();
    await dialog.locator("[data-action='article-close']").click();

    await open("article-v1");
    await dialog.locator("[data-action='article-remove-note'][data-annotation='saved-note']").click();
    await page.waitForFunction(() => !window.articleFixture.state.library[0].details.articleAnnotations.some(note => note.id === "saved-note"));
    assert.equal(await dialog.locator("mark").count(), 0);
    await page.keyboard.press("Escape");
    await open("article-error");
    await dialog.locator("[role='alert']").waitFor();
    assert.match(await dialog.innerText(), /archive is unavailable/);
    await page.keyboard.press("Escape");
    await open("article-slow");
    await dialog.locator("[role='status']").waitFor();
    await dialog.locator("[data-action='article-close']").click();
    await page.evaluate(() => window.articleFixture.releaseSlow());
    await page.waitForTimeout(50);
    assert.equal(await dialog.isVisible(), false, "Closing during a local read must not reopen a stale result.");
    assert.deepEqual(remoteRequests, []);
    assert.deepEqual(errors, []);
    results.push({ viewport: { width, height }, status: "passed", evidence: "Rendered UI with IPC fixtures", checks: ["Escaped blocks and PNG-only images without remote requests", "Exact textual selection and duplicate-quote context", "Immutable-version and ambiguous-anchor separation", "Escaped note text, removal and export payloads", "Keyboard annotation, Escape focus return and canceled local read", "No horizontal overflow"] });
    await page.close();
  }
  await writeFile(`${directory}/verification.json`, JSON.stringify({ nativeArchiveTested: false, results }, null, 2) + "\n");
  console.log(JSON.stringify(results));
} finally { await browser.close(); }
