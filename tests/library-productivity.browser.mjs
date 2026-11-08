// Real frontend and media playback with explicit IPC fixtures. This does not verify native downloads.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = "artifacts/library-productivity";
await mkdir(directory, { recursive: true });

// A generated 180-second silent WAV lets Edge perform a real seek to 134.5 seconds.
const samples = 8_000 * 180;
const wav = Buffer.alloc(44 + samples * 2);
wav.write("RIFF", 0); wav.writeUInt32LE(wav.length - 8, 4); wav.write("WAVEfmt ", 8);
wav.writeUInt32LE(16, 16); wav.writeUInt16LE(1, 20); wav.writeUInt16LE(1, 22);
wav.writeUInt32LE(8_000, 24); wav.writeUInt32LE(16_000, 28); wav.writeUInt16LE(2, 32); wav.writeUInt16LE(16, 34);
wav.write("data", 36); wav.writeUInt32LE(samples * 2, 40);

const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri = true; mockWindows("main");
  const settings = {folder:"C:/Fixture output",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
  const track = {id:"fixture-es",language:"es",origin:"imported",path:"C:/Fixture media/lecture.es.srt",formattingLost:false,segments:[{start:30,end:34,text:"Preparación de la toma"},{start:134.5,end:139,text:"La camera transition conserva la referencia original."}]};
  const details = {contentId:"fixture:lecture",creator:"Fixture author",platform:"Fixture platform",sourceIds:[],chapters:[{title:"Preparación",start:0,end:134.5},{title:"Transición de cámara",start:134.5,end:150}],tracks:[track],playbackPosition:32,favorite:false,bookmarks:[],clip:null};
  const file = (id,name,kind,extra={}) => ({id,path:"C:/Fixture media/"+name,name,kind,size:2880044,duration:180,width:null,height:null,videoCodec:null,audioCodec:"pcm_s16le",container:"wav",addedAt:1_793_923_200_000,source:null,parentId:null,collections:["Fixture manual"],tags:["learning"],missing:false,details:{...structuredClone(details),...extra}});
  const state = {settings,jobs:[],library:[file("lecture","Fixture lecture.wav","audio"),file("variant","Fixture lecture audio.mp3","audio",{tracks:[],playbackPosition:0}),file("captions","Fixture subtitles.srt","subtitle",{playbackPosition:0}),file("image","Fixture image.png","image",{contentId:"fixture:image",tracks:[],chapters:[],playbackPosition:0})],collections:["Fixture manual"],smartCollections:[],sources:[],drafts:[],presets:[],rules:[]};
  window.libraryFixture = {state,calls:[],clipboard:[],unexpected:[]};
  const copy = value => structuredClone(value);
  Object.defineProperty(navigator,"clipboard",{configurable:true,value:{writeText:async text=>window.libraryFixture.clipboard.push(text)}});
  mockIPC(async (cmd,args={})=>{
    window.libraryFixture.calls.push({cmd,args:copy(args)});
    if(cmd==="startup")return {...copy(state),folder:settings.folder,enginesReady:true,engineError:null};
    if(cmd==="get_snapshot")return copy(state);
    if(cmd==="local_timezone")return "Europe/Madrid";
    if(cmd==="screen_capture_status")return null;
    if(cmd==="library_search"){
      if(args.query==="Lens marker fixture") await new Promise(resolve=>{window.libraryFixture.finishSearch=resolve;});
      return [];
    }
    if(cmd==="plugin:window|is_maximized")return false;
    if(cmd.startsWith("plugin:"))return null;
    const item=state.library.find(item=>item.id===args.id);
    if(cmd==="library_playback"){item.details.playbackPosition=args.position;return null;}
    if(cmd==="library_favorite"){item.details.favorite=args.favorite;return null;}
    if(cmd==="library_bookmark"){item.details.bookmarks=item.details.bookmarks.filter(bookmark=>bookmark.id!==args.bookmark.id);if(!args.remove)item.details.bookmarks.push(copy(args.bookmark));return null;}
    if(cmd==="smart_collection_save"){state.smartCollections=state.smartCollections.filter(collection=>collection.id!==args.collection.id);state.smartCollections.push(copy(args.collection));return null;}
    if(cmd==="smart_collection_remove"){state.smartCollections=state.smartCollections.filter(collection=>collection.id!==args.id);return null;}
    if(cmd==="subtitle_import"){const imported={...copy(track),id:"fixture-imported-en",language:args.language,segments:[{start:1,end:3,text:"Explicit imported subtitle fixture"}]};item.details.tracks.push(imported);return copy(imported);}
    if(cmd==="subtitle_export")return "C:/Fixture output/subtitles."+args.format;
    if(cmd==="subtitle_rebuild")return 1;
    if(cmd==="thumbnail")return "data:image/svg+xml,"+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90"><rect width="160" height="90" fill="#17191d"/><text x="12" y="50" fill="#b2b5bd" font-size="12">UI fixture</text></svg>');
    if(cmd==="queue_tool"){const jobs=args.ids.map(id=>({id:"fixture-job-"+state.jobs.length,runId:0,title:"Fixture chapter trim",createdAt:Date.now(),updatedAt:Date.now(),state:"pending",source:{kind:"transform",libraryId:id,options:copy(args.options)},folder:settings.folder,provider:"FFmpeg",thumbnail:null,progress:null,outputs:[],error:null,attempt:0,groupId:args.clientRequest}));state.jobs.push(...jobs);return copy(jobs);}
    window.libraryFixture.unexpected.push({cmd,args});throw new Error("Unhandled explicit library fixture IPC: "+cmd);
  },{shouldMockEvents:true});
  await import("/src/main.ts");
`;

const browser = await chromium.launch({ headless: true, ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }) });
const results = [];
try {
  for (const [width, height] of [[1180,780], [900,620]]) {
    const page = await browser.newPage({ viewport:{width,height} });
    const errors=[];
    page.on("pageerror",error=>errors.push(error.message));
    await page.route(base,async route=>{
      const response=await route.fetch();const html=await response.text();
      const script=/<script type="module" src="\/src\/main\.ts[^"]*"><\/script>/;
      assert.match(html,script,"The fixture must replace the actual frontend entry point");
      await route.fulfill({response,body:html.replace(script,'<script type="module">'+setup+'</script>')});
    });
    await page.route("http://nabmedia.localhost/**",async route=>{
      const request=route.request();const range=request.headers().range?.match(/^bytes=(\d+)-(\d*)$/);
      const start=range?Number(range[1]):0;const end=range&&range[2]?Math.min(Number(range[2]),wav.length-1):wav.length-1;
      await route.fulfill({status:range?206:200,headers:{"content-type":"audio/wav","accept-ranges":"bytes","access-control-allow-origin":"*",...(range?{"content-range":'bytes '+start+'-'+end+'/'+wav.length}:{}),"content-length":String(end-start+1)},body:wav.subarray(start,end+1)});
    });
    await page.goto(base);
    await page.waitForFunction(()=>window.libraryFixture?.calls.some(call=>call.cmd==="startup"));
    await page.locator('[data-page="library"]').click();
    assert.equal(await page.locator('.media-item').count(),4);
    await page.locator("#library-content-filter").selectOption("subtitle");
    assert.equal(await page.locator('.media-item').count(),1);
    assert.match(await page.locator('.media-item').innerText(),/Fixture subtitles\.srt/);
    await page.locator('[data-action="library-filter"][data-filter="all"]').click();

    await page.locator('#library-search').fill('camera transition');
    const hit=page.locator('[data-action="library-seek-result"][data-id="lecture"]').first();
    await hit.waitFor();
    assert.equal(await hit.getAttribute('data-time'),'134.5');
    await hit.click();
    await page.waitForFunction(()=>{const player=document.querySelector('[data-local-player="lecture"]');return player instanceof HTMLMediaElement&&player.readyState>=1&&Math.abs(player.currentTime-134.5)<.01;});
    await page.evaluate(()=>{window.libraryFixture.originalPlayer=document.querySelector('[data-local-player="lecture"]');});
    await page.locator('[data-action="library-inspector-tab"][data-tab="subtitles"]').click();
    assert.equal(await page.evaluate(()=>document.querySelector('[data-local-player="lecture"]')===window.libraryFixture.originalPlayer),true,'Tabs preserve the real media element');
    assert.match(await page.locator('.subtitle-cue.is-current').innerText(),/camera transition/);
    await page.locator('#subtitle-track-search').fill('camera');
    assert.equal(await page.locator('.subtitle-cue').count(),1);
    await page.locator('[data-action="library-copy-cue"]').click();
    assert.match(await page.evaluate(()=>window.libraryFixture.clipboard.at(-1)),/^2:14 La camera transition/);
    await page.screenshot({path:directory+'/fixture-synced-subtitles-'+width+'.png'});
    await page.locator('#subtitle-import-language').fill('en');
    await page.locator('[data-action="library-import-subtitles"]').click();
    await page.waitForFunction(()=>window.libraryFixture.state.library.find(item=>item.id==='lecture').details.tracks.length===2);
    await page.locator('#local-subtitle-track').selectOption('fixture-imported-en');
    await page.locator('#subtitle-track-search').fill('');
    assert.match(await page.locator('.subtitle-reader').innerText(),/Explicit imported subtitle fixture/);
    await page.locator('[data-action="library-export-subtitles"][data-format="txt"]').click();
    await page.waitForFunction(()=>window.libraryFixture.calls.some(call=>call.cmd==='subtitle_export'));
    assert.deepEqual(await page.evaluate(()=>window.libraryFixture.calls.find(call=>call.cmd==='subtitle_export').args),{id:'lecture',trackId:'fixture-imported-en',format:'txt'});
    await page.locator('#local-subtitle-track').selectOption('fixture-es');

    await page.locator('[data-action="library-inspector-tab"][data-tab="details"]').click();
    await page.locator('#bookmark-name').fill('Lens marker fixture');
    await page.locator('#bookmark-form button[type="submit"]').click();
    await page.waitForFunction(()=>window.libraryFixture.state.library.find(item=>item.id==='lecture').details.bookmarks.length===1);
    const bookmark=await page.evaluate(()=>window.libraryFixture.state.library.find(item=>item.id==='lecture').details.bookmarks[0]);
    assert.equal(bookmark.name,'Lens marker fixture');assert.equal(bookmark.time,134.5);
    await page.waitForFunction(()=>{const player=document.querySelector('[data-local-player="lecture"]');return player instanceof HTMLMediaElement&&Math.abs(player.currentTime-134.5)<.01;});
    await page.locator('[data-action="library-favorite"]').click();
    await page.waitForFunction(()=>window.libraryFixture.state.library.find(item=>item.id==='lecture').details.favorite);
    await page.locator('[data-action="library-inspector-tab"][data-tab="versions"]').click();
    assert.match(await page.locator('[data-inspector-panel="versions"]').innerText(),/Fixture lecture audio\.mp3/);
    await page.locator('[data-action="library-inspector-tab"][data-tab="chapters"]').click();
    await page.locator('[data-chapter-index="1"]').check();
    await page.locator('[data-action="library-chapters-to-tools"]').click();
    await page.waitForFunction(()=>window.libraryFixture.calls.some(call=>call.cmd==='queue_tool'));
    const trim=await page.evaluate(()=>window.libraryFixture.calls.find(call=>call.cmd==='queue_tool').args);
    assert.deepEqual(trim.ids,['lecture']);assert.deepEqual(trim.options,{kind:'trim',start:134.5,end:150,precise:false});
    await page.locator('[data-action="close-inspector"]').click();
    await page.locator('#library-search').fill('Lens marker fixture');
    assert.equal(await page.locator('.media-item').count(),1,'Bookmark names participate in metadata search');
    assert.equal(await page.locator('.subtitle-search-hit').count(),0,'Metadata matches are distinct from subtitle matches');

    await page.locator('[data-action="library-section"][data-section="collections"]').click();
    await page.locator('[data-action="smart-new"]').click();
    await page.locator('#smart-name').fill('Fixture references');
    await page.locator('[data-condition-value]').selectOption('audio');
    await page.locator('[data-action="smart-add-condition"]').click();
    await page.locator('[data-condition-kind]').nth(1).selectOption('subtitleLanguage');
    await page.locator('[data-condition-value]').nth(1).fill('ES');
    await page.waitForFunction(()=>typeof window.libraryFixture.finishSearch==="function");
    await page.evaluate(async ()=>{
      window.libraryFixture.finishSearch();
      await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
    });
    assert.equal(await page.locator('[data-condition-value]').nth(1).inputValue(),'ES');
    await page.locator('#smart-collection-form button[type="submit"]').click();
    await page.waitForFunction(()=>window.libraryFixture.state.smartCollections.length===1);
    assert.equal(await page.evaluate(()=>window.libraryFixture.state.library.length),4,'Smart collections do not copy files');
    await page.locator('[data-action="open-library-collection"][data-collection^="smart:"]').click();
    // A query survives switching collections; clear it explicitly to inspect membership.
    assert.equal(await page.locator('#library-search').inputValue(),'Lens marker fixture');
    await page.locator('#library-search').fill('');
    assert.equal(await page.locator('.media-item').count(),1,'All conditions include only the audio file with Spanish subtitles');
    await page.screenshot({path:directory+'/fixture-smart-collection-'+width+'.png'});
    await page.locator('[data-action="library-section"][data-section="collections"]').click();
    await page.locator('[data-action="smart-edit"]').click();
    await page.locator('#smart-mode').selectOption('any');
    await page.locator('#smart-collection-form button[type="submit"]').click();
    await page.waitForFunction(()=>window.libraryFixture.state.smartCollections[0].mode==='any');
    await page.locator('[data-action="open-library-collection"][data-collection^="smart:"]').click();
    assert.equal(await page.locator('.media-item').count(),3,'Any condition includes audio variants and standalone Spanish subtitles');
    const refreshFixtures = async change => {
      const close = page.locator('[data-action="close-inspector"]').first();
      if (await close.isVisible()) await close.click();
      await page.locator('#library-search').fill('');
      await page.locator('#collection-filter').selectOption('');
      await page.locator('[data-action="inspect-media"][data-id="lecture"]').click();
      await page.locator('[data-action="library-inspector-tab"][data-tab="details"]').click();
      const before = await page.evaluate(()=>window.libraryFixture.calls.filter(call=>call.cmd==='get_snapshot').length);
      await page.evaluate(change);
      await page.locator('[data-action="library-favorite"]').click();
      await page.waitForFunction(before=>window.libraryFixture.calls.filter(call=>call.cmd==='get_snapshot').length>before,before);
      await page.locator('[data-action="close-inspector"]').click();
    };
    const seekResult = async (query,id,time) => {
      const close = page.locator('[data-action="close-inspector"]').first();
      if (await close.isVisible()) await close.click();
      await page.locator('#library-search').fill(query);
      await page.locator('[data-action="library-seek-result"]').first().click();
      await page.waitForFunction(({id,time})=>{const player=document.querySelector('[data-local-player="'+id+'"]');return player instanceof HTMLMediaElement&&player.readyState>=1&&Math.abs(player.currentTime-time)<.01;},{id,time});
      if(await page.locator("#playback-stage").isVisible())await page.locator("#stage-back").click();
    };
    await refreshFixtures(()=>{
      const state=window.libraryFixture.state;const base=state.library.find(item=>item.id==='lecture');
      const clip={requestedStart:60,requestedEnd:90,actualStart:null,actualEnd:null};
      const make=(id,kind,clip,text)=>({...structuredClone(base),id,name:'Fixture '+id+'.wav',path:'C:/Fixture media/'+id+'.wav',kind,duration:clip?30:180,parentId:null,collections:[],tags:[],details:{...structuredClone(base.details),contentId:'fixture:timeline',clip,chapters:[],playbackPosition:0,favorite:false,bookmarks:[],tracks:kind==='subtitle'?[{id:id+'-es',language:'es',origin:'imported',path:null,formattingLost:false,segments:[{start:5,end:8,text}]}]:[]}});
      // The clipped video is deliberately first. The WAV stream still performs a real Edge seek.
      state.library.unshift(make('video-clip','video',clip),make('video-full','video',null),make('subtitle-full','subtitle',null,'Full timeline marker'),make('subtitle-clip','subtitle',clip,'Clipped timeline marker'));
    });
    await seekResult('Full timeline marker','video-full',5);
    assert.equal(await page.locator('[data-local-player="video-clip"]').count(),0,'A full-track cue at5 cannot open clip60–90');
    await seekResult('Clipped timeline marker','video-clip',5);
    assert.match(await page.locator('#notification-text').innerText(),/Salto aproximado/);
    await refreshFixtures(()=>{window.libraryFixture.state.library=window.libraryFixture.state.library.filter(item=>item.id!=='video-clip');});
    await seekResult('Clipped timeline marker','video-full',65);
    await refreshFixtures(()=>{
      const state=window.libraryFixture.state;
      const video=structuredClone(state.library.find(item=>item.id==='video-full'));
      video.id='video-attested';video.name='Fixture attested clip.wav';video.duration=30;video.details.clip={requestedStart:60,requestedEnd:90,actualStart:62,actualEnd:92};
      const subtitles=structuredClone(state.library.find(item=>item.id==='subtitle-clip'));
      subtitles.id='subtitle-attested';subtitles.details.clip={requestedStart:60,requestedEnd:90,actualStart:64,actualEnd:94};subtitles.details.tracks[0].segments=[{start:5,end:8,text:'Attested timeline marker'}];
      state.library.unshift(video,subtitles);
    });
    await seekResult('Attested timeline marker','video-attested',7);
    await refreshFixtures(()=>{window.libraryFixture.state.library=window.libraryFixture.state.library.filter(item=>!(item.kind==='video'&&item.details.contentId==='fixture:timeline'));});
    await page.locator('#library-search').fill('Clipped timeline marker');
    await page.locator('[data-action="library-seek-result"]').first().click();
    await page.locator('[data-seek-fallback]').waitFor({state:'visible'});
    assert.match(await page.locator('[data-seek-reference]').innerText(),/contenido original.*1:05/);
    assert.match(await page.locator('[data-seek-reference]').innerText(),/Referencia aproximada/);
    await page.locator('[data-action="library-copy-time"]').click();
    assert.equal(await page.evaluate(()=>window.libraryFixture.clipboard.at(-1)),'1:05');
    await page.screenshot({path:directory+'/fixture-clip-reference-'+width+'.png'});
    await page.locator('[data-action="close-inspector"]').click();
    const overflow=await page.evaluate(()=>({scroll:document.querySelector('#main').scrollWidth,width:document.querySelector('#main').clientWidth,document:document.documentElement.scrollWidth,viewport:innerWidth}));
    assert(overflow.scroll<=overflow.width+1&&overflow.document<=overflow.viewport+1,JSON.stringify(overflow));
    assert.deepEqual(errors,[]);assert.deepEqual(await page.evaluate(()=>window.libraryFixture.unexpected),[]);
    results.push({viewport:{width,height},status:'passed',checks:['subtitle file filter','local phrase and exact real playback seek at 134.5','synced text and clipboard timestamp','subtitle import and text export intent','bookmark and favorite persistence','bookmark metadata search distinct from subtitle hits','chapter trim delivery to existing queue','related variants','smart all/any membership without file copies','query retained across collection filtering','full-track cue ignores non-containing clip','clip cue prefers matching variant then maps into full media','attested cut starts map69 source seconds to7 local seconds','missing video offers original65-second copy reference and timing limitation','no horizontal overflow'],fixtures:true});
    await page.close();
  }
  await writeFile(directory+'/results.json',JSON.stringify({scope:'Real frontend and Edge media playback with explicit IPC metadata fixtures; native downloads and persistence verified separately.',results},null,2));
  console.log(JSON.stringify(results));
} finally {await browser.close();}
