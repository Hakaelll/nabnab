// Render the real home UI with local IPC fixtures and a deterministic animation clock.
import assert from "node:assert/strict";
import {createRequire} from "node:module";
import {execFileSync} from "node:child_process";
import {mkdir,writeFile} from "node:fs/promises";
const require=createRequire(import.meta.url);
const {chromium}=require(process.env.PLAYWRIGHT_MODULE||"playwright");
const browser=await chromium.launch({headless:true,channel:"msedge"});
const directory="artifacts/home-motion";await mkdir(directory,{recursive:true});
const base=process.env.NABNAB_UI_URL||"http://127.0.0.1:1420/";
const before=process.argv.includes("--before"),cpu=process.argv.includes("--cpu"),checks=[];
const label=before?"before":cpu?"after-cpu":"after";
try{
  for(const [width,height] of [[1180,780],[900,620]]){
    const page=await browser.newPage({viewport:{width,height}}),errors=[];page.on("pageerror",e=>errors.push(e.message));
    if(before){const ts=require("typescript");for(const file of ["graphics.ts","field.ts"]){const source=execFileSync("git",["show",`011d9bc:src/${file}`],{encoding:"utf8"});const compiled=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ESNext}}).outputText.replace(/from "\.\/([^"\n]+)"/g,(_,name)=>`from "/src/${name.endsWith(".json")?name+"?import":name+".ts"}"`);await page.route(`**/src/${file}*`,route=>route.fulfill({contentType:"application/javascript",body:compiled}));}}
    if(cpu)await page.addInitScript(()=>{const original=HTMLCanvasElement.prototype.getContext;HTMLCanvasElement.prototype.getContext=function(kind,...options){if(kind==="webgl2")return null;return original.call(this,kind,...options);};});
    await page.addInitScript(()=>{let clock=0,next=0;const callbacks=new Map();window.requestAnimationFrame=callback=>{callbacks.set(++next,callback);return next;};window.cancelAnimationFrame=id=>callbacks.delete(id);window.stepMotion=(frames=1)=>{for(let i=0;i<frames;i++){clock+=1000/60;const pending=[...callbacks.values()];callbacks.clear();for(const callback of pending)callback(clock);}return callbacks.size;};});
    const setup=`import {mockIPC,mockWindows} from '/node_modules/@tauri-apps/api/mocks.js';globalThis.isTauri=true;mockWindows('main');
      const settings={folder:'C:/Downloads/nabnab',language:'es',networkLimit:3,processingLimit:1,reducedMotion:false,reducedTransparency:false,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
      const state={settings,jobs:[],library:[],collections:[],drafts:[],sources:[],rules:[],presets:[],smartCollections:[]};
      mockIPC(async(name,args)=>{if(name==='startup')return {...state,folder:settings.folder,enginesReady:true,engineError:null};if(name==='get_snapshot')return state;if(name==='local_timezone')return 'UTC';if(name==='save_settings'){Object.assign(settings,args.settings);return state;}if(name==='optional_components'||name==='sessions_status'||name==='ocr_models')return [];if(name==='ocr_status')return{installed:false,enabled:false};},{shouldMockEvents:true});await import('/src/main.ts');`;
    await page.route(base,async route=>{const response=await route.fetch();const html=(await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/,`<script type="module">${setup}</script>`);await route.fulfill({response,body:html});});
    await page.goto(base);await page.locator('#url').waitFor();await page.evaluate(()=>document.fonts.ready);
    await page.screenshot({path:`${directory}/home-${label}-${width}x${height}.png`,animations:"disabled"});
    const initial=await page.locator('#field').evaluate(canvas=>canvas.toDataURL());await page.evaluate(()=>window.stepMotion(180));const moved=await page.locator('#field').evaluate(canvas=>canvas.toDataURL());assert.notEqual(moved,initial,"Ambient current must actually change its pixels.");
    await page.screenshot({path:`${directory}/home-${label}-moving-${width}x${height}.png`,animations:"disabled"});
    await page.locator('[data-page="tools"]').click();await page.locator('.tool-workspace').waitFor();assert.equal(await page.evaluate(()=>window.stepMotion(3)),0,"Leaving Home must release the animation loop.");
    await page.emulateMedia({reducedMotion:"reduce"});await page.locator('[data-page="home"]').click();await page.locator('#url').waitFor();const quiet=await page.locator('#field').evaluate(canvas=>canvas.toDataURL());assert.equal(await page.evaluate(()=>window.stepMotion(180)),0,"Reduced motion must remain static.");assert.equal(await page.locator('#field').evaluate(canvas=>canvas.toDataURL()),quiet);
    await page.locator('#url').focus();await page.keyboard.insertText('https://example.org/selected-file');assert.equal(await page.locator('#url').inputValue(),'https://example.org/selected-file');assert.deepEqual(errors,[]);
    checks.push({viewport:[width,height],actualPixelMotion:true,leavingHomeStopsLoop:true,reducedMotionStatic:true,keyboardInputPreserved:true});await page.close();
  }
  if(!before)await writeFile(`${directory}/verification${cpu?"-cpu":""}.json`,JSON.stringify({scope:"Real rendered UI and graphics with local IPC fixtures; deterministic animation clock",checks},null,2));
  console.log(JSON.stringify({before,checks},null,2));
}finally{await browser.close();}
