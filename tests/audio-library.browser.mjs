// Uses real browser audio decoding with silent PCM and simulated desktop storage.
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, channel: "msedge" });
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = process.env.NABNAB_AUDIO_ARTIFACTS || "artifacts/audio-library";
await mkdir(directory, { recursive: true });
const page = await browser.newPage({ viewport: { width: 1180, height: 780 } });
const errors = [];
page.on("pageerror", error => errors.push(error.message));

const pcm = Buffer.alloc(44 + 8000 * 2 * 10);
pcm.write("RIFF"); pcm.writeUInt32LE(pcm.length - 8, 4); pcm.write("WAVEfmt ", 8);
pcm.writeUInt32LE(16, 16); pcm.writeUInt16LE(1, 20); pcm.writeUInt16LE(1, 22);
pcm.writeUInt32LE(8000, 24); pcm.writeUInt32LE(16000, 28); pcm.writeUInt16LE(2, 32);
pcm.writeUInt16LE(16, 34); pcm.write("data", 36); pcm.writeUInt32LE(pcm.length - 44, 40);
await page.route("http://nabmedia.localhost/**", async route => {
  const bytes = /^bytes=(\d+)-(\d*)$/.exec(route.request().headers().range ?? "");
  if (!bytes) return route.fulfill({ contentType: "audio/wav", body: pcm, headers: { "Access-Control-Allow-Origin": "*" } });
  const start = Number(bytes[1]);
  const end = bytes[2] ? Math.min(Number(bytes[2]), pcm.length - 1) : pcm.length - 1;
  await route.fulfill({ status: 206, contentType: "audio/wav", body: pcm.subarray(start, end + 1),
    headers: { "Access-Control-Allow-Origin": "*", "Accept-Ranges": "bytes", "Content-Range": `bytes ${start}-${end}/${pcm.length}` } });
});

const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri=true; mockWindows("main");
  const copy = value => structuredClone(value);
  const settings={folder:"C:/fixture",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
  const tags=(title,disc,track,artist="Sonia")=>({title,disc,track,artist,albumArtist:"Conjunto",album:"Sesiones",genre:"Jazz",embeddedCover:false});
  const item=(id,name,audio,extra={})=>({id,path:"C:/fixture/"+name,name,kind:"audio",size:160044,duration:10,width:null,height:null,videoCodec:null,audioCodec:"pcm_s16le",container:"wav",addedAt:1,source:null,parentId:null,collections:[],tags:[],missing:false,
    details:{contentId:null,creator:null,platform:null,sourceIds:[],chapters:[],tracks:[],playbackPosition:0,favorite:false,bookmarks:[],clip:null,audio,...extra}});
  const chapters=(number)=>({audiobook:{bookId:"librivox:1",bookTitle:"Libro de prueba",chapter:number,reader:"Lector de prueba",language:"Spanish"}});
  const collection=(title,album)=>({...tags(title,1,1,"Zeta"),albumArtist:"Zeta",album,embeddedCover:album!=="Nocturno"});
  const state={settings,jobs:[],library:[
    item("last","Third.wav",tags("Tercera pista",2,1)),
    item("second","Second.mp3",tags("Segunda pista",1,10,"Rafael")),
    item("first","First.mp3",tags("Primera pista",1,2)),
    item("archive","Directo.wav",collection("En directo","Archivo")),
    item("notebook","Improvisación.wav",collection("Improvisación","Cuaderno")),
    item("night","Toma nocturna.wav",collection("Toma nocturna","Nocturno")),
    item("chapter3","Capítulo 3.wav",null,chapters(3)),
    item("chapter2","Capítulo 2.wav",null,chapters(2)),
    item("chapter1","Capítulo 1.wav",null,chapters(1)),
    item("other-book","Prólogo.wav",null,{audiobook:{...chapters(1).audiobook,bookId:"librivox:2",bookTitle:"Otro libro"},playbackPosition:9,lastPlayedAt:2}),
    item("episode","Episodio.wav",null,{platform:"Podcast RSS"}),
  ],collections:[],drafts:[],sources:[],rules:[],smartCollections:[],presets:[],audioPlaylists:[]};
  const calls=[];
  window.audioFixture={state,calls};
  mockIPC(async (cmd,args={})=>{
    calls.push({cmd,args:copy(args)});
    if(cmd==="plugin:window|is_maximized")return false;
    if(cmd==="startup")return {...copy(state),folder:settings.folder,enginesReady:true,engineError:null};
    if(cmd==="get_snapshot")return copy(state);
    if(cmd==="public_source_inspect")return {provider:"librivox",sourceUrl:args.url,title:"Libro de prueba en LibriVox",description:null,rights:null,entries:[],assets:[],nextCursor:null,warnings:[]};
    if(cmd==="local_timezone")return "Romance Standard Time";
    if(cmd==="thumbnail"){
      const design=args.id==="archive"?'<circle cx="50" cy="50" r="34" fill="none" stroke="#e5e5e7" stroke-width="5"/><circle cx="50" cy="50" r="8" fill="#e5e5e7"/>':'<path d="M20 20h60v60H20zM20 46h60M48 20v60" stroke="#b2b5bd" stroke-width="4" fill="none"/>';
      return "data:image/svg+xml,"+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><path fill="#25282e" d="M0 0h100v100H0z"/>'+design+'</svg>');
    }
    if(cmd==="library_playback"){const details=state.library.find(item=>item.id===args.id).details;details.playbackPosition=args.position;details.lastPlayedAt=Date.now();return;}
    if(cmd==="audio_tags_edit"){for(const item of state.library.filter(item=>args.ids.includes(item.id)))Object.assign(item.details.audio,args.patch);return;}
    if(cmd==="audio_playlist_save"){const list={id:args.id??"fixture-playlist-"+state.audioPlaylists.length,name:args.name,entries:args.entries};state.audioPlaylists=state.audioPlaylists.filter(entry=>entry.id!==list.id).concat(list);return copy(state.audioPlaylists);}
    if(cmd==="audio_playlist_rematch"){
      const list=state.audioPlaylists.find(item=>item.id===args.listId);
      return {kind:"list",listId:list.id,name:list.name,rows:list.entries.map(entry=>({
        entry,artist:entry.kind==="missing"?"Sonia":null,duration:entry.kind==="missing"?12:null,version:null,
        candidates:entry.kind==="missing"?[{id:"first",title:"Primera pista",artist:"Sonia",album:"Álbum de prueba",duration:12,versionMatches:false}]:[],
      }))};
    }
    if(cmd==="audio_playlist_choose")return {kind:"csv",id:"fixture-csv",name:"Lista exportada",columns:["Track name","Artist","Duration (ms)","Version"],sample:[["Versión buscada","Sonia","10000","En directo"]],count:3};
    if(cmd==="audio_playlist_map")return {kind:"list",name:"Lista exportada",rows:[
      {entry:{kind:"remote",url:"https://example.org/audio",title:"Versión buscada"},artist:"Sonia",duration:10,version:"En directo",candidates:[
        {id:"first",title:"Versión buscada",artist:"Sonia",album:"Sesiones",duration:10,versionMatches:false},
        {id:"last",title:"Versión buscada",artist:"Sonia",album:"Sesiones",duration:10,versionMatches:true}]},
      {entry:{kind:"missing",locator:"missing.mp3",title:"Ausente"},artist:null,duration:null,version:null,candidates:[]},
      {entry:{kind:"remote",url:"https://example.org/remote",title:"Referencia remota"},artist:null,duration:null,version:null,candidates:[]},
    ]};
    if(cmd==="audio_lyrics")return state.library.find(item=>item.id===args.id)?.details.lyrics ??
      (args.id==="second"?[{time:0,text:"Letra de la segunda pista"}]:
      [{time:1,text:"Una letra local"},...Array.from({length:16},(_,i)=>({time:1.5+i*.45,text:"Verso de prueba "+(i+2)}))]);
    if(cmd==="audio_lyrics_choose"){
      const lines=[{time:2,text:"Letra elegida"},{time:2,text:"Segunda voz"}];
      state.library.find(item=>item.id===args.id).details.lyrics=lines;
      return lines;
    }
    if(cmd==="audio_cover_choose"){state.library.find(item=>item.id===args.id).details.audioCover=args.remove?null:"C:/fixture/cover.png";return true;}
    if(cmd==="audio_tags_queue_copies")return [];
  }, {shouldMockEvents:true});
  await import("/src/main.ts");
`;
await page.route(base, async route => {
  const response = await route.fetch();
  const html = (await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/, `<script type="module">${setup}</script>`);
  await route.fulfill({ response, body: html });
});
const section = name => page.locator(`[data-action='library-section'][data-section='${name}']`);
const category = name => page.locator(`[data-action='audio-category'][data-id='${name}']`);

try {
  await page.goto(base);
  await page.locator("#url").waitFor();
  await page.locator("[data-page='library']").click();
  await section("audio").click();
  await page.getByRole("heading", { name:"Audio", exact:true }).waitFor();
  const trackTitles = () => page.locator(".audio-track strong").allTextContents();
  const shelfTitles = () => page.locator(".audio-shelf-item strong").allTextContents();
  assert.deepEqual(await trackTitles(), ["Primera pista", "Segunda pista", "Tercera pista", "En directo", "Improvisación", "Toma nocturna"]);
  assert.deepEqual(await shelfTitles(), ["Sesiones", "Archivo", "Cuaderno"]);
  await page.getByRole("checkbox", {name:"Seleccionar Primera pista",exact:true}).check();
  await page.getByRole("button", {name:"Grupos siguientes"}).press("Enter");
  await category("podcasts").press("Enter");
  assert.equal(await page.locator(".audio-selection").count(), 0, "Music selection leaked into Podcasts");
  await page.locator("#audio-query").fill("Episodio");
  await page.getByRole("checkbox", {name:"Seleccionar Episodio",exact:true}).check();
  await page.getByRole("button", {name:"Editar etiquetas",exact:true}).click();
  await page.locator('[data-action="audio-tag-enable"][data-field="genre"]').check();
  await page.locator("#audio-tag-genre").fill("Entrevista");
  await category("music").press("Enter");
  assert.equal(await page.locator("#audio-query").inputValue(), "");
  assert.equal(await page.locator("#audio-tag-form").count(), 0);
  assert.equal(await page.getByRole("checkbox", {name:"Seleccionar Primera pista",exact:true}).isChecked(), true);
  assert.deepEqual(await shelfTitles(), ["Nocturno"]);
  await page.locator("#audio-query").fill("Toma nocturna");
  assert.equal(await page.getByText("1 fuera de esta página", {exact:true}).count(), 1);
  await category("podcasts").press("Enter");
  assert.equal(await page.locator("#audio-query").inputValue(), "Episodio");
  const episode = page.getByRole("checkbox", {name:"Seleccionar Episodio",exact:true});
  assert.equal(await episode.isChecked(), true);
  assert.equal(await page.locator("#audio-tag-genre").inputValue(), "Entrevista");
  await episode.uncheck();
  assert.equal(await page.locator("#audio-tag-form").count(), 0);
  await page.locator("#audio-query").clear();
  await category("music").press("Enter");
  assert.equal(await page.locator("#audio-query").inputValue(), "Toma nocturna");
  assert.equal(await page.getByText("1 fuera de esta página", {exact:true}).count(), 1);
  for (const [width,height] of [[900,620],[1180,780]]) {
    await page.setViewportSize({width,height});
    const toolbar = page.locator(".audio-selection");
    await toolbar.scrollIntoViewIfNeeded();
    assert.ok(await toolbar.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await page.screenshot({path:`${directory}/audio-view-selection-${width}x${height}.png`,animations:"disabled"});
  }
  await page.getByRole("button", {name:"Quitar selección",exact:true}).click();
  await page.locator("#audio-query").clear();
  await page.getByRole("button", {name:"Grupos siguientes"}).press("Enter");
  assert.deepEqual(await shelfTitles(), ["Nocturno"]);
  await page.getByRole("button", {name:"Nocturno Zeta 1 pista",exact:true}).press("Enter");
  assert.deepEqual(await trackTitles(), ["Toma nocturna"]);
  assert.equal(await page.evaluate(() => document.activeElement?.dataset.action), "audio-shelf-clear");
  await page.getByRole("button", {name:"Todos los álbumes"}).press("Enter");
  assert.equal(await page.evaluate(() => document.activeElement?.querySelector("strong")?.textContent), "Nocturno");
  await page.getByRole("button", {name:"Grupos anteriores"}).press("Enter");
  await page.evaluate(async () => {
    const { state } = window.audioFixture;
    const missing = structuredClone(state.library.find(item => item.id === "second"));
    missing.id = "album-missing";
    missing.missing = true;
    missing.details.audio.title = "Pista ausente";
    state.library.push(missing);
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    await emit("library-update");
  });
  await page.getByText("Pista ausente", { exact: true }).waitFor();
  await page.locator(".audio-shelf-item").filter({ hasText: "Sesiones" }).press("Enter");
  const album = page.getByRole("region", { name: "Sesiones" });
  await page.locator("#audio-query").fill("Segunda pista");
  assert.deepEqual(await trackTitles(), ["Segunda pista"]);
  assert.match(await album.innerText(), /Conjunto[\s\S]*3 pistas · 0:30 · 1 ausente/);
  const playAlbum = album.getByRole("button", { name: "Reproducir álbum" });
  await playAlbum.press("Enter");
  await page.waitForFunction(() => { const media=document.getElementById("audio-engine"); return media.dataset.localPlayer==="first"&&!media.paused; });
  assert.deepEqual(await page.locator(".audio-next li strong").allTextContents(), ["Segunda pista", "Tercera pista"]);
  assert.equal(await playAlbum.evaluate(button => button === document.activeElement), true);
  for (const [width, height] of [[900, 620], [1180, 780]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await album.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await album.screenshot({ path: `${directory}/album-${width}.png`, animations: "disabled" });
  }
  await page.getByRole("button", {name:"Todos los álbumes"}).press("Enter");
  await page.locator("#audio-query").clear();
  await page.locator("[data-action='audio-play'][data-id='archive']").click();
  await page.waitForFunction(() => { const media=document.getElementById("audio-engine"); return media.dataset.localPlayer==="archive"&&!media.paused; });
  await page.locator(".audio-shelf-item").filter({ hasText: "Sesiones" }).press("Enter");
  await page.locator("#audio-query").fill("sin resultados");
  assert.deepEqual(await trackTitles(), []);
  await album.getByRole("button", { name: "Álbum a continuación" }).press("Enter");
  assert.deepEqual(await page.locator(".audio-next li strong").allTextContents(), ["Primera pista", "Segunda pista", "Tercera pista", "Improvisación", "Toma nocturna"]);
  assert.equal(await page.locator("#audio-engine").evaluate(media => media.dataset.localPlayer === "archive" && !media.paused), true);
  await page.locator("#audio-close").click();
  await page.getByRole("button", {name:"Todos los álbumes"}).press("Enter");
  await page.locator("#audio-query").clear();
  await page.evaluate(async () => {
    const { state } = window.audioFixture;
    state.library = state.library.filter(item => item.id !== "album-missing");
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    await emit("library-update");
  });
  await page.getByText("Pista ausente", { exact: true }).waitFor({ state: "hidden" });
  await page.locator("#audio-artist").selectOption(JSON.stringify("Rafael"));
  assert.deepEqual(await page.locator(".audio-track strong").allTextContents(), ["Segunda pista"]);
  await page.locator("#audio-artist").selectOption("");
  await page.locator("[data-action='audio-play'][data-id='first']").click();
  await page.waitForFunction(() => { const audio=document.getElementById("audio-engine"); return audio.readyState>=2 && !audio.paused && audio.currentTime>0; });
  await page.getByRole("checkbox", { name:"Seleccionar Tercera pista", exact:true }).check();
  await page.getByRole("checkbox", { name:"Seleccionar Segunda pista", exact:true }).check();
  await page.locator(".audio-selection").scrollIntoViewIfNeeded();
  {
    const engine = await page.locator("#audio-engine").elementHandle();
    const next = page.getByRole("button", { name:"Reproducir a continuación", exact:true });
    assert.equal(await next.count(), 1, "The audio selection must offer to play tracks next.");
    await next.press("Enter");
    assert.equal(await next.evaluate(button => button === document.activeElement), true);
    assert.deepEqual(await page.locator(".audio-next li strong").allTextContents(),
      ["Tercera pista", "Segunda pista", "En directo", "Improvisación", "Toma nocturna"]);
    assert.equal(await engine.evaluate(media => media === document.getElementById("audio-engine")
      && media.dataset.localPlayer === "first" && !media.paused), true);
    assert.match(await page.locator(".audio-next .is-next").innerText(), /Siguiente\s*Tercera pista/);
    assert.match(await page.locator(".audio-selection-heading").innerText(), /Siguiente · Tercera pista/);
    await page.locator(".audio-selection").scrollIntoViewIfNeeded();
    await page.screenshot({ path:`${directory}/audio-next-after.png`, animations:"disabled" });
    await page.setViewportSize({ width:900, height:620 });
    assert.ok(await page.locator(".audio-selection").evaluate(panel => panel.scrollWidth <= panel.clientWidth + 1));
    await page.screenshot({ path:`${directory}/audio-next-900.png`, animations:"disabled" });
    await page.setViewportSize({ width:1180, height:780 });
    await page.reload();
    await page.locator("button[data-page='library']").click();
    await section("audio").click();
    assert.deepEqual(await page.locator(".audio-next li strong").allTextContents(),
      ["Tercera pista", "Segunda pista", "En directo", "Improvisación", "Toma nocturna"]);
    await page.locator("#audio-next").click();
    await page.waitForFunction(() => document.getElementById("audio-engine").dataset.localPlayer === "last");
    await page.locator("#audio-close").click();
    await page.getByRole("checkbox", { name:"Seleccionar Primera pista", exact:true }).check();
    await page.getByRole("checkbox", { name:"Seleccionar Segunda pista", exact:true }).check();
    await next.press("Enter");
    await page.waitForFunction(() => {
      const media = document.getElementById("audio-engine");
      return media.dataset.localPlayer === "first" && media.paused;
    });
    assert.deepEqual(await page.locator(".audio-next li strong").allTextContents(), ["Segunda pista"]);
    await page.locator("[data-action='audio-clear']").click();
  }
  await page.locator("[data-action='audio-play'][data-id='first']").click();
  await page.waitForFunction(() => !document.getElementById("audio-engine").paused);
  await page.locator("[data-action='audio-open-lyrics']").click();
  await page.getByText("Una letra local", {exact:true}).waitFor();
  await page.locator("#audio-toggle").click();
  const lyricMedia = page.locator("#audio-engine");
  const lyricList = page.locator("#audio-lyric-lines");
  const currentLyric = lyricList.locator('[aria-current="true"]');
  const follow = page.getByRole("button", {name:"Seguir letra",exact:true});
  await follow.press("Space");
  assert.equal(await follow.getAttribute("aria-pressed"), "false");
  await lyricMedia.evaluate(media => { media.currentTime = 9; });
  await currentLyric.getByText("Verso de prueba 17", {exact:true}).waitFor();
  assert.equal(await lyricList.evaluate(list => list.scrollTop), 0);
  await follow.press("Space");
  assert.equal(await follow.getAttribute("aria-pressed"), "true");
  assert.ok(await lyricList.evaluate(list => list.scrollTop > 0));
  for (const [width,height] of [[900,620],[1180,780]]) {
    await page.setViewportSize({width,height});
    await page.locator("#audio-lyrics").scrollIntoViewIfNeeded();
    assert.ok(await lyricList.evaluate(list => list.scrollWidth <= list.clientWidth + 1));
    await page.screenshot({path:`${directory}/audio-lyrics-${width}x${height}.png`,animations:"disabled"});
  }
  await lyricList.getByRole("button", {name:/Una letra local/}).press("Enter");
  await currentLyric.getByText("Una letra local", {exact:true}).waitFor();
  assert.equal(await lyricMedia.evaluate(media => media.currentTime), 1);
  await page.locator("[data-action='audio-choose-lyrics']").click();
  await page.getByText("Letra elegida", {exact:false}).waitFor();
  assert.equal(await lyricList.getByRole("button").count(), 1);
  assert.match(await lyricList.innerText(), /Letra elegida\s+Segunda voz/);
  await page.locator("#audio-next").click();
  await page.getByText("Letra de la segunda pista", {exact:true}).waitFor();
  await page.locator("[data-action='audio-play'][data-id='first']").click();
  await lyricList.getByText("Letra elegida", {exact:false}).waitFor();
  await page.waitForFunction(() => !document.getElementById("audio-engine").paused);
  await page.locator("[data-action='audio-open-lyrics']").click();
  for (const [width,height] of [[1180,780],[900,620]]) {
    await page.setViewportSize({width,height});
    await page.locator("#main").evaluate(main => { main.scrollTop = 0; });
    await page.locator(".audio-shelf-cover img").first().waitFor();
    const player = await page.locator("#audio-player").boundingBox();
    assert(player && player.y + player.height <= height + 1 && player.width + player.x <= width + 1);
    const overflow = await page.locator("#audio-transport").evaluate(element => element.scrollWidth > element.clientWidth + 1);
    assert.equal(overflow, false, `Player overflow at ${width}`);
    assert.ok(await page.locator(".audio-shelf").evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    await page.screenshot({path:`${directory}/audio-${width}x${height}.png`});
  }
  const audioEngine = await page.locator("#audio-engine").elementHandle();
  await page.locator("[data-page='downloads']").click();
  assert.equal(await page.locator("#audio-player").isVisible(), true);
  assert.equal(await audioEngine.evaluate(el => el === document.getElementById("audio-engine") && !el.paused), true);
  await page.locator("#audio-next").click();
  await page.waitForFunction(() => document.getElementById("audio-engine").src.endsWith("/second"));
  await page.locator("#audio-open").click();
  await page.locator('#playback-stage[data-kind="music"]').waitFor();
  assert.equal(await audioEngine.evaluate(el => el === document.getElementById("audio-engine") && !el.paused), true);
  await page.locator("#stage-back").click();
  await page.locator("#audio-player").waitFor();
  await page.locator("[data-page='library']").click();
  await section("audio").waitFor();

  await page.getByRole("checkbox", {name:"Seleccionar Primera pista",exact:true}).check();
  await page.getByRole("checkbox", {name:"Seleccionar Segunda pista",exact:true}).check();
  await page.locator("[data-action='audio-edit-tags']").click();
  await page.locator("[data-action='audio-tag-enable'][data-field='genre']").check();
  await page.locator("#audio-tag-genre").fill("Fusion");
  await page.getByRole("button", {name:"Previsualizar cambios",exact:true}).click();
  assert.match(await page.locator(".audio-tag-preview").innerText(), /Jazz → Fusion/);
  assert.equal(await page.evaluate(() => window.audioFixture.calls.some(call=>call.cmd==="audio_tags_edit")), false);
  await page.getByRole("button", {name:"Guardar etiquetas",exact:true}).click();
  await page.waitForFunction(() => window.audioFixture.state.library.filter(item=>["first","second"].includes(item.id)).every(item=>item.details.audio.genre==="Fusion"));
  await page.getByRole("button", {name:"Elegir portada",exact:true}).click();
  await page.getByRole("button", {name:"Quitar portada local",exact:true}).waitFor();
  await page.getByRole("button", {name:"Quitar portada local",exact:true}).click();
  await page.getByRole("button", {name:"Quitar portada local",exact:true}).waitFor({state:"detached"});
  await page.locator("[data-action='audio-edit-tags']").click();
  await page.locator("[data-action='audio-tag-enable'][data-field='album']").check();
  await page.locator("#audio-tag-album").fill("Álbum para la copia");
  await page.getByRole("button", {name:"Previsualizar cambios",exact:true}).click();
  await page.setViewportSize({width:1180,height:780});
  await page.locator("#audio-tag-form").scrollIntoViewIfNeeded();
  await page.screenshot({path:`${directory}/audio-tags-preview.png`});
  await page.getByRole("button", {name:"Crear copias con etiquetas",exact:true}).click();
  const queued = await page.evaluate(() => window.audioFixture.calls.find(call=>call.cmd==="audio_tags_queue_copies").args);
  assert.deepEqual(queued.ids,["first","second"]);
  assert.deepEqual(queued.patch,{album:"Álbum para la copia"});
  assert.equal(await page.evaluate(() => window.audioFixture.state.library.find(item=>item.id==="first").details.audio.album),"Sesiones");
  await page.getByRole("button", {name:"Crear lista",exact:true}).click();
  await page.locator("#audio-playlist-name").fill("Para escuchar");
  await page.locator("[data-action='audio-list-down'][data-index='0']").click();
  await page.getByRole("button", {name:"Guardar lista",exact:true}).click();
  await page.getByRole("heading", {name:"Para escuchar",exact:true}).waitFor();
  assert.deepEqual(await page.locator(".audio-list-entries strong").allTextContents(), ["Segunda pista", "Primera pista"]);
  await page.getByRole("button", {name:"Importar lista",exact:true}).click();
  await page.locator("#audio-csv-duration").selectOption("2");
  await page.locator("#audio-csv-ms").check();
  await page.getByRole("button", {name:"Buscar coincidencias locales",exact:true}).click();
  const comparison = page.locator("#audio-import-match-0");
  await comparison.waitFor();
  await page.setViewportSize({ width: 900, height: 620 });
  assert.equal(await comparison.getAttribute("open"), null);
  await comparison.locator("summary").press("Enter");
  const originalReference = comparison.getByRole("radio", { name: "Conservar referencia original" });
  assert.equal(await originalReference.isChecked(), true);
  assert.equal(await comparison.getByRole("radio").count(), 3);
  assert.equal(await comparison.getByText("C:/fixture/First.mp3", { exact: true }).isVisible(), true);
  const localRecording = comparison.getByRole("radio", { name: /C:\/fixture\/Third\.wav/ });
  await localRecording.press("Space");
  assert.equal(await localRecording.isChecked(), true);
  assert.ok(await localRecording.evaluate(input => input === document.activeElement));
  await originalReference.press("Space");
  assert.equal(await originalReference.isChecked(), true);
  await localRecording.press("Space");
  assert.ok(await comparison.evaluate(panel => panel.scrollWidth <= panel.clientWidth + 1));
  const [first, second, transport] = await comparison.evaluate(panel => {
    const options = [...panel.querySelectorAll(".audio-match-candidates label")].map(option => option.getBoundingClientRect().toJSON());
    return [...options, document.querySelector("#audio-player").getBoundingClientRect().toJSON()];
  });
  await page.screenshot({path:`${directory}/playlist-matching-900.png`});
  assert.equal(first.top, second.top);
  assert.ok(Math.max(first.bottom, second.bottom) <= transport.top, `Candidate rows end at ${Math.max(first.bottom, second.bottom)} and the player begins at ${transport.top}`);
  await page.setViewportSize({width:1180,height:780});
  await page.locator("#main").evaluate(main => { main.scrollTop = 0; });
  await page.screenshot({path:`${directory}/playlist-matching.png`});
  await page.getByRole("button", {name:"Preparar lista",exact:true}).click();
  await page.getByRole("button", {name:"Guardar lista",exact:true}).click();
  await page.getByRole("heading", {name:"Lista exportada",exact:true}).waitFor();
  const imported = await page.evaluate(() => window.audioFixture.state.audioPlaylists.find(list=>list.name==="Lista exportada"));
  assert.deepEqual(imported.entries,[{kind:"local",id:"last",title:"Versión buscada"},{kind:"missing",locator:"missing.mp3",title:"Ausente"},{kind:"remote",url:"https://example.org/remote",title:"Referencia remota"}]);
  const mapping = await page.evaluate(() => window.audioFixture.calls.find(call=>call.cmd==="audio_playlist_map").args.mapping);
  assert.equal(mapping.duration,2);
  assert.equal(mapping.milliseconds,true);

  const reviewReferences = page.getByRole("button", { name: "Revisar referencias" });
  const closeReview = page.getByRole("button", { name: "Cerrar revisión" });
  const prepareChanges = page.getByRole("button", { name: "Preparar cambios" });
  const missingRecording = page.locator("#audio-import-match-1");
  const readSavedPlaylist = () => page.evaluate(() => window.audioFixture.state.audioPlaylists.find(list=>list.name==="Lista exportada"));
  await page.setViewportSize({width:900,height:620});
  await reviewReferences.scrollIntoViewIfNeeded();
  await page.screenshot({path:`${directory}/saved-references-before-900.png`});
  await reviewReferences.press("Enter");
  await missingRecording.waitFor();
  assert.equal(await page.locator("#audio-import-heading").evaluate(heading => heading === document.activeElement), true);
  assert.deepEqual(await page.locator(".audio-import-rows li[value]").evaluateAll(rows => rows.map(row => row.value)), [2,3]);
  await page.getByText("Sin coincidencias locales", {exact:true}).waitFor();
  assert.equal(await prepareChanges.isDisabled(), true);
  await missingRecording.locator("summary").press("Enter");
  assert.equal(await missingRecording.getByRole("radio", {name:"Conservar referencia original"}).isChecked(), true);
  const replacement = missingRecording.getByRole("radio", {name:/Primera pista/});
  await replacement.press("Space");
  assert.deepEqual(await readSavedPlaylist(), imported);
  for (const [width,height] of [[1180,780],[900,620]]) {
    await page.setViewportSize({width,height});
    await missingRecording.scrollIntoViewIfNeeded();
    await prepareChanges.evaluate(button => button.scrollIntoView({block:"center"}));
    assert.ok(await missingRecording.evaluate(panel => panel.scrollWidth <= panel.clientWidth + 1));
    assert.ok(await prepareChanges.evaluate(button => button.getBoundingClientRect().bottom <= document.querySelector("#audio-player").getBoundingClientRect().top));
    await page.screenshot({path:`${directory}/saved-references-review-${width}.png`});
  }
  await closeReview.press("Enter");
  assert.equal(await reviewReferences.evaluate(button => button === document.activeElement), true);
  assert.deepEqual(await readSavedPlaylist(), imported);
  await reviewReferences.press("Enter");
  await missingRecording.locator("summary").press("Enter");
  await replacement.press("Space");
  await prepareChanges.press("Enter");
  assert.deepEqual(await readSavedPlaylist(), imported);
  await page.getByRole("button", {name:"Guardar lista",exact:true}).press("Enter");
  await page.waitForFunction(id => window.audioFixture.state.audioPlaylists.find(list=>list.id===id).entries[1].id==="first", imported.id);
  const rematched = await readSavedPlaylist();
  assert.equal(rematched.id, imported.id);
  assert.deepEqual(rematched.entries, [imported.entries[0], {kind:"local",id:"first",title:"Primera pista"}, imported.entries[2]]);

  await category("podcasts").click();
  assert.deepEqual(await page.locator(".audio-track strong").allTextContents(), ["Episodio"]);
  await category("books").click();
  assert.equal(await page.locator(".audio-selection").count(), 0);
  await category("music").click();
  assert.equal(await page.getByRole("checkbox", {name:"Seleccionar Primera pista",exact:true}).isChecked(), true);
  assert.equal(await page.getByRole("checkbox", {name:"Seleccionar Segunda pista",exact:true}).isChecked(), true);
  await category("books").click();
  assert.deepEqual(await trackTitles(), ["Capítulo 1", "Capítulo 2", "Capítulo 3", "Prólogo"]);
  assert.deepEqual(await shelfTitles(), ["Libro de prueba", "Otro libro"]);
  const partial = page.getByRole("button", {name:/Continuar · Otro libro · Capítulo 1 · 0:09/});
  await partial.press("Enter");
  await page.waitForFunction(() => { const audio = document.getElementById("audio-engine"); return audio.src.endsWith("/other-book") && !audio.paused && audio.currentTime >= 9; });
  await page.locator("#audio-toggle").click();
  await page.getByRole("button", {name:"Todos los libros",exact:true}).press("Enter");
  await page.locator("[data-action='audio-play'][data-id='chapter1']").click();
  await page.waitForFunction(() => document.getElementById("audio-engine").readyState>=2);
  await page.locator("#audio-rate").selectOption("1.5");
  assert.equal(await page.locator("#audio-engine").evaluate(el=>el.playbackRate),1.5);
  await page.locator("#audio-sleep").selectOption("chapter");
  await page.locator("#audio-engine").evaluate(el=>{el.currentTime=9.8;});
  await page.waitForFunction(() => document.getElementById("audio-engine").paused);
  assert.match(await page.locator("#audio-engine").getAttribute("src"), /\/chapter1$/);
  assert.equal(await page.locator("#audio-sleep").inputValue(),"off");
  assert.equal(await page.evaluate(() => window.audioFixture.state.library.find(item => item.id === "chapter1").details.playbackPosition), 10);
  const resume = page.getByRole("button", {name:/Continuar · Libro de prueba · Capítulo 2/});
  await resume.waitFor();
  const notice = page.getByRole("button", {name:"Cerrar aviso",exact:true});
  if (await notice.isVisible()) await notice.click();
  await page.locator("#main").evaluate(main => { main.scrollTop = 0; });
  await page.screenshot({path:`${directory}/audio-books.png`});
  await page.locator("#audio-query").fill("Capítulo 2");
  assert.deepEqual(await trackTitles(), ["Capítulo 2"]);
  await resume.press("Enter");
  await page.waitForFunction(() => document.getElementById("audio-engine").src.endsWith("/chapter2") && !document.getElementById("audio-engine").paused);
  assert.deepEqual(await trackTitles(), ["Capítulo 1", "Capítulo 2", "Capítulo 3"]);
  assert.deepEqual(await page.locator(".audio-next li strong").allTextContents(), ["Capítulo 3"]);
  await page.evaluate(async () => {
    const { state } = window.audioFixture;
    const third = state.library.find(item => item.id === "chapter3");
    third.missing = true;
    const fourth = structuredClone(third);
    Object.assign(fourth, { id:"chapter4", name:"Capítulo 4.wav", path:"C:/fixture/Capítulo 4.wav", missing:false });
    fourth.details.audiobook.chapter = 4;
    state.library.push(fourth);
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    await emit("library-update");
  });
  await page.locator("[data-action='audio-play'][data-id='chapter4']").waitFor();
  await page.locator("[data-action='audio-play'][data-id='chapter2']").click();
  assert.deepEqual(await page.locator(".audio-next li strong").allTextContents(), []);
  await page.locator("#audio-engine").evaluate(el => { el.currentTime=9.8; });
  const missingChapter = page.getByText("Falta el capítulo 3 en Biblioteca.", { exact: true });
  await missingChapter.waitFor();
  assert.equal(await page.locator("#audio-engine").evaluate(el => el.paused && el.src.endsWith("/chapter2")), true);
  assert.equal(await page.getByRole("button", { name:/Continuar · Libro de prueba · Capítulo 4/ }).count(), 0);
  for (const [width,height] of [[1180,780],[900,620]]) {
    await page.setViewportSize({width,height});
    await missingChapter.scrollIntoViewIfNeeded();
    assert.ok(await page.locator(".audio-library").evaluate(el => el.scrollWidth <= el.clientWidth + 1));
    await page.screenshot({path:`${directory}/audiobook-gap-${width}.png`});
  }
  await page.getByRole("button", { name:"Ver edición en LibriVox", exact:true }).press("Enter");
  const edition = page.getByRole("heading", { name:"Libro de prueba en LibriVox", exact:true });
  assert.ok(await edition.evaluate(heading => heading === document.activeElement));
  assert.equal(await page.evaluate(() => window.audioFixture.calls.filter(call => call.cmd === "public_source_inspect").at(-1).args.url), "https://librivox.org/api/feed/audiobooks/id/1");
  assert.equal(await page.evaluate(() => window.audioFixture.calls.filter(call => call.cmd === "queue_public_files").length), 0);
  await page.getByRole("button", { name:"Biblioteca", exact:true }).click();
  await page.locator("[data-action='audio-play'][data-id='chapter4']").click();
  await page.waitForFunction(() => document.getElementById("audio-engine").src.endsWith("/chapter4") && !document.getElementById("audio-engine").paused);
  await page.evaluate(async () => {
    const { state } = window.audioFixture;
    const book = structuredClone(state.library.find(item => item.id === "chapter1"));
    Object.assign(book, {id:"many-chapters", name:"Voces del valle.m4b", path:"C:/fixture/Voces del valle.m4b", duration:33});
    Object.assign(book.details, {audiobook:null, playbackPosition:0, chapters:Array.from({length:33}, (_, i) => ({title:`Parte ${i + 1}`,start:i,end:i + 1}))});
    state.library.push(book);
    const { emit } = await import("/node_modules/@tauri-apps/api/event.js");
    await emit("library-update");
  });
  await page.locator('#audio-book option[value="many-chapters"]').waitFor({state:"attached"});
  await page.locator("#audio-book").selectOption("many-chapters");
  const chapterIndex = page.getByRole("region", {name:"Capítulos del archivo"});
  assert.equal(await chapterIndex.locator("li").count(), 30);
  await chapterIndex.getByRole("button", {name:"Siguientes",exact:true}).press("Enter");
  assert.deepEqual(await chapterIndex.locator("strong").allTextContents(), ["Parte 31", "Parte 32", "Parte 33"]);
  assert.equal(await chapterIndex.getByRole("heading").evaluate(node => node === document.activeElement), true);
  assert.deepEqual(errors, []);
  console.log("Audio shelves, playback, playlist import and saved reference review, and audiobook controls passed.");
} finally { await browser.close(); }
