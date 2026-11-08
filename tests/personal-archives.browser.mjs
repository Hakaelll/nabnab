import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, channel: "msedge" });
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = "artifacts/personal-archives";
await mkdir(directory, { recursive: true });
const page = await browser.newPage({ viewport: { width: 1180, height: 780 } });
const errors = [];
page.on("pageerror", error => errors.push(error.message));

const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri=true; mockWindows("main");
  const settings={folder:"C:/fixture",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false};
  const state={settings,jobs:[],library:[],collections:[],drafts:[],sources:[],rules:[],smartCollections:[],presets:[],audioPlaylists:[]};
  const time=Date.UTC(2025,0,8), fingerprint="a".repeat(64);
  const row=(id,title,text,author,extra={})=>({id,title,text,author,time,source:"fixture.json",link:null,media:null,threadParent:null,hasThread:false,playedMs:null,trackCount:0,...extra});
  const threadPosts=[
    {row:row("p","Una publicación","Una publicación de prueba","Cuenta de prueba",{link:"https://example.org/post",hasThread:true,threadParent:"99"}),replyTo:null},
    {row:row("r","Primera respuesta","Este es el primer mensaje guardado como respuesta.","Cuenta de prueba",{hasThread:true}),replyTo:1},
    {row:row("siguiente","Otra rama","Una respuesta distinta a la misma publicación.","Cuenta de prueba",{hasThread:true}),replyTo:1},
    ...Array.from({length:50},(_,index)=>({row:row("post-"+index,"Continuación "+(index+4),"Otra publicación guardada.","Cuenta de prueba",{hasThread:true}),replyTo:index+3})),
  ];
  const rows={
    posts:threadPosts.map(item=>item.row),
    messages:[row("m","Conversación de prueba","Texto privado sintético","Persona de prueba")],
    listening:[row("l","Canción de prueba","","Artista de prueba",{playedMs:62000})],
    playlists:[row("s","Lista de prueba","","",{trackCount:1})],
  };
  const summaries=[];
  let imports=0;
  const calls=[];
  window.archiveCalls=calls;
  const preview=(category,records,availability,sensitive=false,formats=["json"])=>({category,records,sensitive,availability,formats,files:availability==="absent"?0:1,bytes:314,start:records?time:null,end:records?time:null});
  mockIPC(async (cmd,args={})=>{
    calls.push({cmd,args:structuredClone(args)});
    if(cmd==="startup")return {...structuredClone(state),folder:settings.folder,enginesReady:true,engineError:null};
    if(cmd==="get_snapshot")return structuredClone(state);
    if(cmd==="plugin:window|is_maximized")return false;
    if(cmd==="personal_archive_list")return structuredClone(summaries);
    if(cmd==="personal_archive_choose")return {fingerprint,source:args.provider+"-export.zip",bytes:10234,categories:args.provider==="spotify"?
      [preview("listening",1,"available"),preview("playlists",1,"available")]:
      [preview("posts",threadPosts.length,"available"),preview("likes",null,"absent",false,[]),preview("bookmarks",null,"unsupported",false,["unsupported"]),preview("media",0,"empty"),preview("messages",null,"requiresSelection",true)]};
    if(cmd==="personal_archive_import"){
      const selected=args.selection;
      const all=[...selected.categories,...selected.sensitive];
      const provider=all.includes("listening")?"spotify":"x";
      const archive={id:String(++imports),provider,source:provider+"-export.zip",fingerprint,mediaBytes:0,account:selected.account,destination:selected.destination,
        categories:Object.fromEntries(all.map(key=>[key,rows[key].length])),formats:Object.fromEntries(all.map(key=>[key,["json"]])),pendingRemoval:false};
      summaries.push(archive);
      return {archive,skippedFiles:0};
    }
    if(cmd==="personal_archive_records"){
      if(args.category==="messages"&&!args.consent)throw "Missing private consent";
      const data=rows[args.category].filter(row=>!args.query||row.title.includes(args.query));
      const isListening=args.category==="listening";
      return {rows:data.slice(args.offset,args.offset+50),total:data.length,start:data.length?time:null,end:data.length?time:null,
        playedMs:isListening?62000:null,completionKnown:0,completions:null,ended:isListening?1:null,artists:isListening?[["Artista de prueba",1]]:[]};
    }
    if(cmd==="personal_archive_thread"){
      if(!summaries.some(item=>item.id===args.id&&item.provider==="x")||args.record!=="p")throw "Unexpected thread selection";
      const offset=args.offset??0;
      return {posts:threadPosts.slice(offset,offset+50),offset,total:threadPosts.length,incomplete:true};
    }
    if(cmd==="personal_archive_match")return {kind:"list",name:"Lista de prueba",rows:[{entry:{kind:"missing",title:"Canción de prueba",locator:""},artist:"Artista de prueba",duration:null,version:null,candidates:[]}]};
    if(cmd==="personal_archive_export")return true;
    if(cmd==="personal_archive_links")return ["https://example.org/post"];
    if(cmd==="batch_create"||cmd==="batch_inspect")return {id:"from-archive",name:"Lote del archivo",createdAt:time,updatedAt:time,savedForLater:false,
      presetId:null,options:null,explorationLimit:200,items:[{id:"post",url:"https://example.org/post",title:"Enlace del archivo",provider:"",creator:"",
        contentId:null,thumbnail:null,duration:null,state:cmd==="batch_create"?"pending":"unsupported",picked:true,excluded:false,options:null,
        resolved:null,destination:null,error:null,list:null,media:null}]};
    if(cmd==="personal_archive_remove"){summaries.splice(summaries.findIndex(item=>item.id===args.id),1);return;}
  }, {shouldMockEvents:true});
  await import("/src/main.ts");
`;
await page.route(base, async route => {
  const response = await route.fetch();
  const html = (await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/, `<script type="module">${setup}</script>`);
  await route.fulfill({ response, body: html });
});
const action = name => page.locator(`[data-action="archive-${name}"]`);
const calls = () => page.evaluate(() => window.archiveCalls);
const categories = id => page.locator(`[data-action="archive-category"][data-id="${id}"]`);
const archivesTab = page.locator("[data-section='archives']");

try {
  await page.goto(base);
  await page.locator("#url").waitFor();
  await page.locator("[data-page='library']").click();
  await archivesTab.click();
  await page.getByText("Aún no hay exportaciones importadas.").waitFor();
  await action("new").click();
  await page.locator("#archive-provider").selectOption("x");
  await action("choose").click();
  await page.getByText("Formato no compatible", { exact: false }).waitFor();
  await page.getByText("No incluido", { exact: false }).waitFor();
  await page.getByText("Vacío", { exact: false }).waitFor();
  assert.equal(await page.locator("input[value='messages']").isChecked(), false);
  assert.equal(await page.locator("input[value='bookmarks']").isDisabled(), true);
  assert.equal((await calls()).filter(call => call.cmd === "personal_archive_records").length, 0);
  await page.locator("#archive-account").fill("Cuenta de prueba");
  await page.locator("#archive-destination").fill("Archivo de 2025");
  await page.locator("#archive-from").fill("2025-01-01");
  for (const [width, height] of [[1180, 890], [900, 620]]) {
    await page.setViewportSize({ width, height });
    const layout = await page.locator(".personal-archives").evaluate(element => {
      const panel = element.querySelector(".archive-panel");
      return [panel.scrollWidth, panel.clientWidth];
    });
    assert(layout[0] <= layout[1] + 1, `Archive form overflows at ${width}px`);
    await page.screenshot({ path: `${directory}/selection-${width}.png` });
  }
  await page.locator("input[value='messages']").check();
  await page.getByRole("button", { name: "Importar selección" }).click();
  await page.getByText("Una publicación", { exact: true }).waitFor();
  assert.equal(await page.locator("#main").evaluate(element => element.scrollTop), 0);
  const [{ args: { selection } }] = (await calls()).filter(call => call.cmd === "personal_archive_import");
  assert.deepEqual(selection.categories, ["posts"]);
  assert.deepEqual(selection.sensitive, ["messages"]);
  assert.equal(selection.from, "2025-01-01");
  await page.locator("#archive-query").fill("Una publicación");
  await page.getByRole("button", { name: "Buscar", exact: true }).click();
  const threadButton = page.locator('[data-action="archive-thread"][data-id="p"]');
  await threadButton.focus();
  await page.keyboard.press("Enter");
  const threadTitle = page.getByRole("heading", { name: "Hilo en esta exportación" });
  await threadTitle.waitFor();
  assert.equal(await threadTitle.evaluate(element => element === document.activeElement), true);
  await page.getByText("La cadena de respuestas está incompleta en esta exportación.").waitFor();
  assert.equal(await page.locator(".archive-thread-posts [aria-current='true']").innerText().then(text => text.includes("Una publicación de prueba")), true);
  assert.equal(await page.getByText("Responde a la publicación 1", { exact: true }).count(), 2);
  for (const [width, height] of [[1180, 890], [900, 620]]) {
    await page.setViewportSize({ width, height });
    const fits = await page.locator(".archive-thread").evaluate(element => element.scrollWidth <= element.clientWidth + 1);
    assert.equal(fits, true, `Thread overflows at ${width}px`);
    await page.locator("#main").evaluate(element => { element.scrollTop = 0; });
    await page.screenshot({ path: `${directory}/thread-${width}.png` });
  }
  await action("thread-next").click();
  await page.getByText("51–53 / 53", { exact: true }).waitFor();
  await action("thread-back").click();
  assert.equal(await page.locator("#archive-query").inputValue(), "Una publicación");
  assert.equal(await threadButton.evaluate(element => element === document.activeElement), true);
  assert.equal((await calls()).some(call => call.cmd === "batch_create"), false);
  await categories("messages").click();
  await action("reveal").waitFor();
  assert.equal((await calls()).filter(call => call.cmd === "personal_archive_records" && call.args.category === "messages").length, 0);
  await action("reveal").click();
  await page.getByText("Conversación de prueba", { exact: true }).waitFor();
  assert.equal(await page.locator("#archive-query").evaluate(element => element === document.activeElement), true);
  assert.equal(await action("links").count(), 0);
  await page.getByRole("checkbox", { name: /Conversación de prueba/ }).check();
  await action("export").click();
  assert.equal((await calls()).filter(call => call.cmd === "personal_archive_export").length, 0);
  await action("export-confirm").click();
  await page.getByText("Selección exportada.").waitFor();
  assert.equal((await calls()).find(call => call.cmd === "personal_archive_export").args.consent, true);
  await page.locator("[data-section='files']").click();
  await archivesTab.click();
  await action("reveal").waitFor();
  assert.equal(await page.getByText("Conversación de prueba", { exact: true }).count(), 0);
  await categories("posts").click();
  await page.getByRole("checkbox", { name: /Una publicación/ }).check();
  await action("links").click();
  await page.getByText("Enlace del archivo", { exact: true }).waitFor();
  assert.equal((await calls()).find(call => call.cmd === "batch_create").args.text, "https://example.org/post");
  assert.equal((await calls()).some(call => call.cmd === "batch_queue"), false);
  await page.locator("[data-page='library']").click();
  await page.locator(".archive-provenance summary").click();
  await action("remove").click();
  await action("remove-confirm").click();
  await page.getByText("Aún no hay exportaciones importadas.").waitFor();

  await action("new").click();
  await page.locator("#archive-provider").selectOption("spotify");
  await action("choose").click();
  await page.locator("#archive-account").fill("Cuenta de audio");
  await page.locator("#archive-destination").fill("Historial de prueba");
  await page.getByRole("button", { name: "Importar selección" }).click();
  await page.getByText("Reproducciones completas", { exact: true }).waitFor();
  assert.equal(await page.getByText("Sin datos", { exact: true }).count(), 1);
  await page.setViewportSize({ width: 1180, height: 890 });
  assert.equal(await page.locator("#main").evaluate(element => element.scrollTop), 0);
  await page.locator("#notification [data-action='dismiss-message']").click();
  await page.screenshot({ path: `${directory}/listening.png` });
  const listeningArchive = action("open").getByText("spotify-export.zip", { exact: true });
  for (const [destination, privateMessages] of [["Archivo 2025", true], ["Archivo 2026", false]]) {
    await action("new").click();
    await page.locator("#archive-provider").selectOption("x");
    await action("choose").click();
    await page.locator("#archive-account").fill("Cuenta de prueba");
    await page.locator("#archive-destination").fill(destination);
    if (privateMessages) await page.locator("input[value='messages']").check();
    await page.getByRole("button", { name: "Importar selección" }).click();
    await page.locator("#archive-current").getByText("Cuenta de prueba", { exact: true }).waitFor();
  }
  const index = page.getByRole("navigation", { name: "Cuentas 2" });
  assert.equal(await index.getByRole("region", { name: "Cuenta de prueba" }).getByRole("button").count(), 2);
  const firstExport = index.getByRole("button", { name: /Archivo 2025/ });
  const secondExport = index.getByRole("button", { name: /Archivo 2026/ });
  await firstExport.focus();
  await page.keyboard.press("Enter");
  await categories("messages").waitFor();
  assert.equal(await firstExport.getAttribute("aria-current"), "true");
  assert.equal(await firstExport.evaluate(element => element === document.activeElement), true);
  await categories("messages").focus();
  await page.keyboard.press("Space");
  await action("reveal").waitFor();
  assert.equal(await categories("messages").evaluate(element => element === document.activeElement), true);
  await action("reveal").click();
  await page.getByText("Conversación de prueba", { exact: true }).waitFor();
  await secondExport.click();
  await page.getByText("Una publicación", { exact: true }).waitFor();
  await firstExport.click();
  await categories("messages").click();
  await action("reveal").waitFor();
  assert.equal(await page.getByText("Conversación de prueba", { exact: true }).count(), 0);
  await page.locator("#notification [data-action='dismiss-message']").click();
  await page.setViewportSize({ width: 900, height: 620 });
  const overflows = await page.locator(".archive-workspace, .archive-index, .archive-browser").evaluateAll(
    elements => elements.some(element => element.scrollWidth > element.clientWidth + 1)
  );
  assert.equal(overflows, false, "Archive workspace overflows at minimum size");
  await page.screenshot({ path: `${directory}/private-900.png` });
  await listeningArchive.click();
  await page.getByText("Reproducciones completas", { exact: true }).waitFor();
  await page.setViewportSize({ width: 1180, height: 890 });
  await page.screenshot({ path: `${directory}/accounts.png` });
  await categories("playlists").click();
  await action("match").click();
  await page.getByRole("heading", { name: "Audio", exact: true }).waitFor();
  await page.getByText("Canción de prueba", { exact: true }).waitFor();
  assert.deepEqual(errors, []);
  console.log("Verified archived X threads, pagination and focus return, account grouping, private consent, removal, Spotify history and local playlist matching in Edge.");
} finally {
  await browser.close();
}
