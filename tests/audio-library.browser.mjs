// Uses real browser audio decoding with silent PCM and simulated desktop storage.
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, channel: "msedge" });
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = "artifacts/audio-library";
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
  if (!bytes) return route.fulfill({ contentType: "audio/wav", body: pcm });
  const start = Number(bytes[1]);
  const end = bytes[2] ? Math.min(Number(bytes[2]), pcm.length - 1) : pcm.length - 1;
  await route.fulfill({ status: 206, contentType: "audio/wav", body: pcm.subarray(start, end + 1),
    headers: { "Accept-Ranges": "bytes", "Content-Range": `bytes ${start}-${end}/${pcm.length}` } });
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
  const state={settings,jobs:[],library:[
    item("last","Third.wav",tags("Tercera pista",2,1)),
    item("second","Second.mp3",tags("Segunda pista",1,10,"Rafael")),
    item("first","First.mp3",tags("Primera pista",1,2)),
    item("chapter2","Capítulo 2.wav",null,chapters(2)),
    item("chapter1","Capítulo 1.wav",null,chapters(1)),
    item("episode","Episodio.wav",null,{platform:"Podcast RSS"}),
  ],collections:[],drafts:[],sources:[],rules:[],smartCollections:[],presets:[],audioPlaylists:[]};
  const calls=[];
  window.audioFixture={state,calls};
  mockIPC(async (cmd,args={})=>{
    calls.push({cmd,args:copy(args)});
    if(cmd==="plugin:window|is_maximized")return false;
    if(cmd==="startup")return {...copy(state),folder:settings.folder,enginesReady:true,engineError:null};
    if(cmd==="get_snapshot")return copy(state);
    if(cmd==="local_timezone")return "Romance Standard Time";
    if(cmd==="library_playback"){const details=state.library.find(item=>item.id===args.id).details;details.playbackPosition=args.position;details.lastPlayedAt=Date.now();return;}
    if(cmd==="audio_tags_edit"){for(const item of state.library.filter(item=>args.ids.includes(item.id)))Object.assign(item.details.audio,args.patch);return;}
    if(cmd==="audio_playlist_save"){const list={id:args.id??"fixture-playlist-"+state.audioPlaylists.length,name:args.name,entries:args.entries};state.audioPlaylists=state.audioPlaylists.filter(entry=>entry.id!==list.id).concat(list);return copy(state.audioPlaylists);}
    if(cmd==="audio_playlist_choose")return {kind:"csv",id:"fixture-csv",name:"Lista exportada",columns:["Track name","Artist","Duration (ms)","Version"],sample:[["Versión buscada","Sonia","10000","En directo"]],count:2};
    if(cmd==="audio_playlist_map")return {kind:"list",name:"Lista exportada",rows:[
      {entry:{kind:"remote",url:"https://example.org/audio",title:"Versión buscada"},artist:"Sonia",duration:10,version:"En directo",candidates:[
        {id:"first",title:"Primera pista",artist:"Sonia",album:"Sesiones",duration:10,versionMatches:false},
        {id:"last",title:"Tercera pista",artist:"Sonia",album:"Sesiones",duration:10,versionMatches:true}]},
      {entry:{kind:"missing",locator:"missing.mp3",title:"Ausente"},artist:null,duration:null,version:null,candidates:[]},
    ]};
    if(cmd==="audio_lyrics")return [{time:1,text:"Una letra local"}];
    if(cmd==="audio_lyrics_choose")return [{time:2,text:"Letra elegida"}];
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
  assert.deepEqual(await page.locator(".audio-track strong").allTextContents(), ["Primera pista", "Segunda pista", "Tercera pista"]);
  await page.locator("#audio-artist").selectOption(JSON.stringify("Rafael"));
  assert.deepEqual(await page.locator(".audio-track strong").allTextContents(), ["Segunda pista"]);
  await page.locator("#audio-artist").selectOption("");
  await page.locator("[data-action='audio-play'][data-id='first']").click();
  await page.waitForFunction(() => { const audio=document.getElementById("audio-engine"); return audio.readyState>=2 && !audio.paused && audio.currentTime>0; });
  await page.locator("[data-action='audio-open-lyrics']").click();
  await page.getByText("Una letra local", {exact:true}).waitFor();
  await page.locator("[data-action='audio-choose-lyrics']").click();
  await page.getByText("Letra elegida", {exact:true}).waitFor();
  await page.locator("[data-action='audio-open-lyrics']").click();
  for (const [width,height] of [[1180,780],[900,620]]) {
    await page.setViewportSize({width,height});
    const player = await page.locator("#audio-player").boundingBox();
    assert(player && player.y + player.height <= height + 1 && player.width + player.x <= width + 1);
    const overflow = await page.locator("#audio-transport").evaluate(element => element.scrollWidth > element.clientWidth + 1);
    assert.equal(overflow, false, `Player overflow at ${width}`);
    await page.screenshot({path:`${directory}/audio-${width}x${height}.png`});
  }
  const audioEngine = await page.locator("#audio-engine").elementHandle();
  await page.locator("[data-page='downloads']").click();
  assert.equal(await page.locator("#audio-player").isVisible(), true);
  assert.equal(await audioEngine.evaluate(el => el === document.getElementById("audio-engine") && !el.paused), true);
  await page.locator("#audio-next").click();
  await page.waitForFunction(() => document.getElementById("audio-engine").src.endsWith("/second"));
  await page.locator("#audio-open").click();
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
  await page.locator("#audio-import-match-0").waitFor();
  assert.equal(await page.locator("#audio-import-match-0").inputValue(),"");
  assert.equal(await page.locator("#audio-import-match-0 option").count(),3);
  await page.locator("#audio-import-match-0").selectOption("last");
  await page.setViewportSize({width:1180,height:780});
  await page.screenshot({path:`${directory}/playlist-matching.png`});
  await page.getByRole("button", {name:"Preparar lista",exact:true}).click();
  await page.getByRole("button", {name:"Guardar lista",exact:true}).click();
  await page.getByRole("heading", {name:"Lista exportada",exact:true}).waitFor();
  const imported = await page.evaluate(() => window.audioFixture.state.audioPlaylists.find(list=>list.name==="Lista exportada"));
  assert.deepEqual(imported.entries,[{kind:"local",id:"last",title:"Tercera pista"},{kind:"missing",locator:"missing.mp3",title:"Ausente"}]);
  const mapping = await page.evaluate(() => window.audioFixture.calls.find(call=>call.cmd==="audio_playlist_map").args.mapping);
  assert.equal(mapping.duration,2);
  assert.equal(mapping.milliseconds,true);

  await category("podcasts").click();
  assert.deepEqual(await page.locator(".audio-track strong").allTextContents(), ["Episodio"]);
  await category("books").click();
  assert.deepEqual(await page.locator(".audio-track strong").allTextContents(), ["Capítulo 1", "Capítulo 2"]);
  await page.locator("[data-action='audio-play'][data-id='chapter1']").click();
  await page.waitForFunction(() => document.getElementById("audio-engine").readyState>=2);
  await page.locator("#audio-rate").selectOption("1.5");
  assert.equal(await page.locator("#audio-engine").evaluate(el=>el.playbackRate),1.5);
  await page.locator("#audio-sleep").selectOption("chapter");
  await page.locator("#audio-engine").evaluate(el=>{el.currentTime=9.8;});
  await page.waitForFunction(() => document.getElementById("audio-engine").paused);
  assert.match(await page.locator("#audio-engine").getAttribute("src"), /\/chapter1$/);
  assert.equal(await page.locator("#audio-sleep").inputValue(),"off");
  assert.equal(await page.evaluate(() => window.audioFixture.calls.some(call=>call.cmd==="library_playback" && call.args.id==="chapter1")), true);
  assert.deepEqual(errors, []);
  console.log("Audio grouping, persistent real browser playback, metadata preview, ordered playlists, local lyrics and audiobook sleep timer passed.");
} finally { await browser.close(); }
