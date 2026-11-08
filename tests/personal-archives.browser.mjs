import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, channel: "msedge" });
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = process.env.NABNAB_ARCHIVE_ARTIFACTS || "artifacts/personal-archives";
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
  const row=(id,title,text,author,extra={})=>({id,title,text,author,time,source:"fixture.json",link:null,media:null,threadParent:null,hasThread:false,attachmentCount:0,playedMs:null,trackCount:0,...extra});
  const threadPosts=[
    {row:row("p","Una publicación","Una publicación de prueba","Cuenta de prueba",{link:"https://example.org/post",hasThread:true,threadParent:"99"}),replyTo:null},
    {row:row("r","Primera respuesta","Este es el primer mensaje guardado como respuesta.","Cuenta de prueba",{hasThread:true,time:time+86400000}),replyTo:1},
    {row:row("siguiente","Otra rama","Una respuesta distinta a la misma publicación.","Cuenta de prueba",{hasThread:true}),replyTo:1},
    ...Array.from({length:50},(_,index)=>({row:row("post-"+index,"Continuación "+(index+4),"Otra publicación guardada.","Cuenta de prueba",{hasThread:true}),replyTo:index+3})),
  ];
  const rows={
    posts:threadPosts.map(item=>item.row),
    media:[
      row("photo","Lago de prueba.jpg","",null,{media:"media/Lago de prueba.jpg"}),
      row("broken","Vídeo dañado.mp4","",null,{media:"media/Vídeo dañado.mp4"}),
      row("sound","Audio de prueba.mp3","",null,{media:"media/Audio de prueba.mp3"}),
    ],
    messages:[row("m","Conversación de prueba","Texto privado sintético","Persona de prueba")],
    listening:[
      row("l","Canción de prueba","","Artista de prueba",{playedMs:132000}),
      row("l2","Canción de prueba","","Artista de prueba",{playedMs:48000}),
      row("l3","Episodio de prueba","","Programa de prueba",{playedMs:186000}),
    ],
    playlists:[row("s","Lista de prueba","","",{trackCount:1})],
  };
  const xAttachments=rows.media.slice(0,2).map(item=>({...item,title:"100-"+item.title,source:"data/tweets_media/100-"+item.title}));
  const summaries=[];
  const group=(key,title,playedMs,extra={})=>({key:key.repeat(64),title,creator:null,album:null,episode:false,identified:false,events:1,playedMs,completions:null,completionKnown:0,...extra});
  const listeningGroups={
    items:[
      group("a","Episodio de prueba",186000,{creator:"Programa de prueba",episode:true,identified:true}),
      group("b","Canción de prueba",132000,{creator:"Artista de prueba",album:"En directo",identified:true}),
      group("c","Canción de prueba",48000,{creator:"Artista de prueba",album:"En estudio",identified:true}),
    ],
    creators:[group("d","Programa de prueba",186000),group("e","Artista de prueba",180000,{events:2})],
  };
  let imports=0, provider="x";
  const calls=[];
  window.archiveCalls=calls;
  const preview=(category,records,availability,sensitive=false,formats=["json"])=>({category,records,sensitive,availability,formats,files:availability==="absent"?0:1,bytes:314,start:records?time:null,end:records?time:null});
  mockIPC(async (cmd,args={})=>{
    calls.push({cmd,args:structuredClone(args)});
    if(cmd==="startup")return {...structuredClone(state),folder:settings.folder,enginesReady:true,engineError:null};
    if(cmd==="get_snapshot")return structuredClone(state);
    if(cmd==="plugin:window|is_maximized")return false;
    if(cmd==="personal_archive_list")return structuredClone(summaries);
    if(cmd==="personal_archive_choose"){
      provider=args.provider;
      return {fingerprint,source:provider+"-export.zip",bytes:10234,datedPostMedia:["x","instagram","facebook"].includes(provider),categories:provider==="instagram"?
      [preview("media",3,"available",false,["media"])] : args.provider==="spotify"?
      [preview("listening",3,"available"),preview("playlists",1,"available")]:
      [preview("posts",threadPosts.length,"available"),preview("likes",null,"absent",false,[]),preview("bookmarks",null,"unsupported",false,["unsupported"]),
        imports===0?preview("media",0,"empty"):{...preview("media",2,"available",false,["media"]),files:3,bytes:6252},preview("messages",null,"requiresSelection",true)]};
    }
    if(cmd==="personal_archive_review_media") return {files:2,bytes:4096,skippedPostFiles:0};
    if(cmd==="personal_archive_import"){
      const selected=args.selection;
      const all=[...selected.categories,...selected.sensitive];
      const archive={id:String(++imports),provider,source:provider+"-export.zip",fingerprint,mediaBytes:0,account:selected.account,destination:selected.destination,
        categories:Object.fromEntries(all.map(key=>[key,key==="media"&&provider==="x"?2:rows[key].length])),formats:Object.fromEntries(all.map(key=>[key,["json"]])),pendingRemoval:false};
      summaries.push(archive);
      return {archive,skippedFiles:0};
    }
    if(cmd==="personal_archive_records"){
      if(args.category==="messages"&&!args.consent)throw "Missing private consent";
      let data=rows[args.category].filter(row=>!args.query||row.title.includes(args.query));
      if(args.dates) {
        const from=args.dates.from?Date.parse(args.dates.from+"T00:00:00Z"):-Infinity;
        const to=args.dates.to?Date.parse(args.dates.to+"T23:59:59.999Z"):Infinity;
        data=data.filter(row=>row.time!==null&&row.time>=from&&row.time<=to);
      }
      if(args.category==="posts"&&summaries.find(item=>item.id===args.id)?.categories.media) data=data.map(row=>row.id==="p"?{...row,attachmentCount:2}:row);
      const isListening=args.category==="listening";
      if(args.listening?.key) {
        const ids={a:["l3"],b:["l"],c:["l2"],d:["l3"],e:["l","l2"]};
        data=data.filter(row=>ids[args.listening.key[0]].includes(row.id));
      }
      const groups=args.listening&&!args.listening.key?listeningGroups[args.listening.by]:null;
      const dates=data.map(row=>row.time).filter(time=>time!==null);
      return {rows:groups?[]:data.slice(args.offset,args.offset+50),total:data.length,start:dates.length?Math.min(...dates):null,end:dates.length?Math.max(...dates):null,
        playedMs:isListening?data.reduce((total,row)=>total+row.playedMs,0):null,completionKnown:0,completions:null,ended:isListening?1:null,
        listeningGroups:groups?{groups,total:groups.length}:null};
    }
    if(cmd==="personal_archive_thread"){
      if(!summaries.some(item=>item.id===args.id&&item.provider==="x")||args.record!=="p")throw "Unexpected thread selection";
      const offset=args.offset??0;
      return {posts:threadPosts.slice(offset,offset+50),offset,total:threadPosts.length,incomplete:true};
    }
    if(cmd==="personal_archive_attachments"){
      if(args.post!=="p"||!summaries.find(item=>item.id===args.id&&item.provider==="x")?.categories.media)throw "Unexpected attachments request";
      return {rows:xAttachments,total:xAttachments.length,offset:0};
    }
    if(cmd==="personal_archive_preview_media"){
      if(args.record==="broken")throw "This file could not be previewed.";
      if(args.record!=="photo"||args.category!=="media")throw "Unexpected preview request";
      await new Promise(resolve=>setTimeout(resolve,350));
      const canvas=document.createElement("canvas"); canvas.width=560; canvas.height=320;
      const drawing=canvas.getContext("2d");
      const horizon=drawing.createLinearGradient(0,0,0,320);
      horizon.addColorStop(0,"#5b7893"); horizon.addColorStop(.52,"#cfad92"); horizon.addColorStop(.53,"#496576"); horizon.addColorStop(1,"#1a3545");
      drawing.fillStyle=horizon; drawing.fillRect(0,0,560,320);
      drawing.fillStyle="#394d57"; drawing.beginPath(); drawing.moveTo(0,170); drawing.lineTo(130,70); drawing.lineTo(255,170); drawing.lineTo(330,103); drawing.lineTo(460,170); drawing.fill();
      return await new Promise(resolve=>canvas.toBlob(blob=>resolve(blob.arrayBuffer()),"image/jpeg"));
    }
    if(cmd==="personal_archive_preview_cancel")return;
    if(cmd==="personal_archive_copy_media")return true;
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
  const summary = page.locator("#archive-selection-summary");
  const importSelection = page.getByRole("button", { name: "Importar selección", exact: true });
  const postChoice = page.locator("input[name='archive-category'][value='posts']");
  assert.equal(await summary.innerText(), "1 categoría · 1 archivo · 314 B");
  await postChoice.press("Space");
  assert.equal(await summary.innerText(), "Elige al menos una categoría.");
  assert.equal(await importSelection.isDisabled(), true);
  assert.equal(await postChoice.evaluate(input => input === document.activeElement), true);
  await postChoice.press("Space");
  assert.equal(await importSelection.isEnabled(), true);
  assert.equal(await page.locator("input[value='messages']").isChecked(), false);
  assert.equal(await page.locator("input[value='bookmarks']").isDisabled(), true);
  assert.equal((await calls()).filter(call => call.cmd === "personal_archive_records").length, 0);
  await page.locator("#archive-account").fill("Cuenta de prueba");
  await page.locator("#archive-destination").fill("Archivo de 2025");
  await page.locator("#archive-from").fill("2025-01-01");
  assert.equal(await page.getByText(/Marca Publicaciones y Multimedia para incluir los adjuntos vinculados/).count(), 1);
  for (const [width, height] of [[1180, 890], [900, 620]]) {
    await page.setViewportSize({ width, height });
    const layout = await page.locator(".personal-archives").evaluate(element => {
      const panel = element.querySelector(".archive-panel");
      return [panel.scrollWidth, panel.clientWidth];
    });
    assert(layout[0] <= layout[1] + 1, `Archive form overflows at ${width}px`);
    await importSelection.scrollIntoViewIfNeeded();
    const note = await page.getByText(/Marca Publicaciones y Multimedia para incluir los adjuntos vinculados/).boundingBox();
    assert.ok(note && note.y >= 0 && note.y + note.height < height);
    await page.screenshot({ path: `${directory}/selection-${width}.png` });
  }
  await page.locator("input[value='messages']").check();
  assert.equal(await summary.innerText(), "2 categorías · 1 con datos privados · 2 archivos · 628 B");
  assert.equal((await calls()).filter(call => call.cmd === "personal_archive_import" || call.cmd === "personal_archive_records").length, 0);
  await importSelection.scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${directory}/selection-private-900.png` });
  await importSelection.click();
  await page.getByText("Una publicación", { exact: true }).waitFor();
  assert.equal(await page.locator("#main").evaluate(element => element.scrollTop), 0);
  const [{ args: { selection } }] = (await calls()).filter(call => call.cmd === "personal_archive_import");
  assert.deepEqual(selection.categories, ["posts"]);
  assert.deepEqual(selection.sensitive, ["messages"]);
  assert.equal(selection.from, "2025-01-01");
  const archiveSearch = page.getByRole("button", { name: "Buscar", exact: true });
  const fromDate = page.getByLabel("Desde (UTC)", { exact: true });
  const toDate = page.getByLabel("Hasta (UTC)", { exact: true });
  await page.locator(".archive-browser-heading").evaluate(node => node.scrollIntoView({ block: "start" }));
  await page.screenshot({ path: `${directory}/archive-search-before.png` });
  await page.getByText("Fechas (UTC)", { exact: true }).click({ timeout: 3000 });
  await page.locator('input[name="archive-record"][value="p"]').check();
  await fromDate.fill("2025-01-09");
  await toDate.fill("2025-01-09");
  await archiveSearch.click();
  await page.locator(".archive-record").getByText("Primera respuesta", { exact: true }).waitFor();
  assert.equal(await page.locator(".archive-record").count(), 1);
  assert.equal(await action("export").isDisabled(), true);
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    const search = page.locator("#archive-search-form");
    assert.ok(await search.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await page.locator(".archive-browser-heading").evaluate(node => node.scrollIntoView({ block: "start" }));
    await page.screenshot({ path: `${directory}/archive-search-${width}.png` });
  }
  await fromDate.fill("2025-01-10");
  await archiveSearch.click();
  await page.getByRole("alert").getByText("La fecha inicial debe ser anterior o igual a la final.", { exact: true }).waitFor();
  assert.equal(await page.locator(".archive-record").count(), 1);
  await fromDate.fill("");
  await toDate.fill("");
  await archiveSearch.click();
  await page.locator(".archive-record").getByText("Una publicación", { exact: true }).waitFor();
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
  const tracksView = page.getByRole("button", { name: "Pistas y episodios", exact: true });
  await tracksView.press("Enter");
  const groups = page.getByRole("region", { name: "Pistas y episodios" });
  const liveTrack = groups.getByRole("button", { name: /Canción de prueba.*En directo.*2,2 min/ });
  await liveTrack.waitFor();
  assert.equal(await groups.getByRole("button", { name: /Canción de prueba/ }).count(), 2);
  assert.equal(await groups.getByRole("button", { name: /Episodio de prueba.*Programa de prueba/ }).count(), 1);
  for (const [width, height] of [[1180, 890], [900, 620]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await groups.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await page.locator("#main").evaluate(node => { node.scrollTop = 0; });
    await page.screenshot({ path: `${directory}/listening-groups-${width}.png` });
  }
  await liveTrack.press("Enter");
  const selectedTrack = page.getByRole("heading", { name: "Canción de prueba", exact: true });
  await selectedTrack.waitFor();
  assert.equal(await selectedTrack.evaluate(node => node === document.activeElement), true);
  assert.equal(await page.locator(".archive-record").count(), 1);
  await page.getByRole("button", { name: "Volver al resumen" }).press("Enter");
  assert.equal(await liveTrack.evaluate(node => node === document.activeElement), true);
  await page.getByRole("button", { name: "Artistas y programas" }).press("Enter");
  const artists = page.getByRole("region", { name: "Artistas y programas" });
  await artists.getByRole("button", { name: /Artista de prueba.*3 min.*2 eventos/ }).waitFor();
  assert.equal((await calls()).some(call => call.cmd === "personal_archive_records" && call.args.listening?.key === "b".repeat(64)), true);
  const listeningArchive = action("open").getByText("spotify-export.zip", { exact: true });
  for (const [destination, privateMessages] of [["Archivo 2025", true], ["Archivo 2026", false]]) {
    await action("new").click();
    await page.locator("#archive-provider").selectOption("x");
    await action("choose").click();
    await page.locator("#archive-account").fill("Cuenta de prueba");
    await page.locator("#archive-destination").fill(destination);
    if (destination === "Archivo 2025") {
      const calculate = page.getByRole("button", {name:"Calcular adjuntos"});
      const summary = page.locator("#archive-media-review-result");
      assert.equal(await calculate.isDisabled(), true);
      await page.locator("#archive-from").fill("2025-01-08");
      await calculate.press("Enter");
      await page.getByText("2 adjuntos en el intervalo · 4 KB", {exact:true}).waitFor();
      assert.equal(await page.locator("#archive-selection-summary").innerText(), "2 categorías · 4 archivos · 6 KB");
      assert.equal(await summary.evaluate(node => node === document.activeElement), true);
      const request = (await calls()).find(call => call.cmd === "personal_archive_review_media").args;
      assert.equal(request.from, "2025-01-08");
      assert.equal(request.to, null);
      assert.deepEqual(Object.keys(request).sort(), ["fingerprint", "from", "requestId", "to"]);
      for (const [width,height] of [[1180,890],[900,620]]) {
        await page.setViewportSize({width,height});
        await calculate.scrollIntoViewIfNeeded();
        const bounds = await summary.boundingBox();
        assert.ok(bounds && bounds.y >= 0 && bounds.y + bounds.height < height);
        await page.screenshot({path:`${directory}/dated-media-${width}.png`});
      }
      await page.locator("input[value='media']").uncheck();
      assert.equal(await summary.textContent(), "");
      assert.equal(await calculate.isDisabled(), true);
      await page.locator("input[value='media']").check();
      await calculate.click();
      await page.getByText("2 adjuntos en el intervalo · 4 KB", {exact:true}).waitFor();
      await page.locator("#archive-from").fill("");
      assert.equal(await summary.textContent(), "");
      assert.equal(await calculate.isDisabled(), true);
    }
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
  const attachmentsButton = page.getByRole("button", { name: "2 adjuntos" });
  await attachmentsButton.press("Enter");
  const attachmentsTitle = page.getByRole("heading", { name: "Adjuntos de la publicación" });
  await attachmentsTitle.waitFor();
  assert.ok(await attachmentsTitle.evaluate(node => node === document.activeElement));
  assert.deepEqual(await page.locator(".archive-attachment-name strong").allTextContents(), ["Lago de prueba.jpg", "Vídeo dañado.mp4"]);
  for (const [width, height] of [[1180, 890], [900, 620]]) {
    await page.setViewportSize({ width, height });
    await page.keyboard.press("Escape");
    await attachmentsButton.press("Enter");
    await attachmentsTitle.waitFor();
    assert.ok(await page.locator(".archive-attachments").evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    const attachmentBounds = await page.locator(".archive-attachment-list").boundingBox();
    assert.ok(attachmentBounds.y >= 0 && attachmentBounds.y + attachmentBounds.height <= height);
    await page.screenshot({ path: `${directory}/x-attachments-${width}.png` });
  }
  await action("copy").and(page.locator('[data-id="photo"]')).click();
  assert.equal((await calls()).filter(call => call.cmd === "personal_archive_copy_media").at(-1).args.category, "media");
  await page.getByRole("button", { name: "Volver a publicaciones" }).click();
  assert.ok(await attachmentsButton.evaluate(node => node === document.activeElement));
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

  await archivesTab.click();
  await action("new").click();
  await page.locator("#archive-provider").selectOption("instagram");
  await action("choose").click();
  await page.locator("#archive-account").fill("Cuenta de fotos");
  await page.locator("#archive-destination").fill("Álbum de prueba");
  await page.getByRole("button", { name: "Importar selección" }).click();
  const photo = action("media-preview").and(page.locator('[data-id="photo"]'));
  await photo.waitFor();
  const sound = page.locator(".archive-record").filter({ hasText: "Audio de prueba.mp3" });
  assert.equal(await sound.getByRole("button", { name: "Vista previa" }).count(), 0);
  assert.equal(await sound.getByRole("button", { name: "Guardar copia" }).count(), 1);
  await page.locator("#archive-query").fill("Lago");
  await page.getByRole("button", { name: "Buscar", exact: true }).click();
  await page.getByRole("checkbox", { name: "Lago de prueba.jpg" }).check();
  await photo.press("Enter");
  await page.getByText("Preparando vista previa…", { exact: true }).waitFor();
  await page.keyboard.press("Escape");
  assert.ok(await photo.evaluate(element => element === document.activeElement));
  await photo.press("Enter");
  const image = page.getByRole("img", { name: "Lago de prueba.jpg" });
  await image.waitFor();
  await page.waitForFunction(() => document.querySelector(".archive-media-stage img")?.naturalWidth === 560);
  for (const [width, height] of [[1180, 890], [900, 620]]) {
    await page.setViewportSize({ width, height });
    await page.keyboard.press("Escape");
    await photo.press("Enter");
    await page.waitForFunction(() => document.querySelector(".archive-media-stage img")?.naturalWidth === 560);
    assert.ok(await page.locator(".archive-media-preview").evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    const bounds = await image.boundingBox();
    assert.ok(bounds.y >= 0 && bounds.y + bounds.height <= height);
    await page.screenshot({ path: `${directory}/media-${width}.png` });
  }
  await action("copy").click();
  await page.getByText("Copia guardada.", { exact: true }).waitFor();
  await action("media-back").click();
  assert.equal(await page.locator("#archive-query").inputValue(), "Lago");
  assert.equal(await page.getByRole("checkbox", { name: "Lago de prueba.jpg" }).isChecked(), true);
  await page.locator("#archive-query").fill("Vídeo");
  await page.getByRole("button", { name: "Buscar", exact: true }).click();
  await action("media-preview").and(page.locator('[data-id="broken"]')).click();
  await page.getByText("Primer fotograma", { exact: true }).waitFor();
  await page.getByRole("alert").getByText("No se pudo mostrar la vista previa.", { exact: false }).waitFor();
  assert.equal(await page.locator(".archive-media-stage img").count(), 0);
  assert.ok((await calls()).some(call => call.cmd === "personal_archive_preview_cancel"));
  assert.deepEqual(errors, []);
  console.log("Verified dated attachment reviews, archived X threads and attachments, privacy, Spotify history, local playlist matching and cancellable media previews at 900 and 1180 px in Edge.");
} finally {
  await browser.close();
}
