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
    const shell = () => page.evaluate(() => Object.fromEntries(
      ["#main", ".sidebar-mark", ".titlebar-search", ".nav-item[data-page='library']"].map(selector => {
        const box = document.querySelector(selector).getBoundingClientRect();
        return [selector, [box.x, box.y, box.width, box.height]];
      })
    ));
    const homeShell = await shell();
    assert.ok(homeShell[".sidebar-mark"][2] > 0 && homeShell[".titlebar-search"][2] > 0);
    assert.deepEqual(await page.locator("#emblem").evaluate(canvas => {
      const orbit = getComputedStyle(canvas.parentElement, "::after");
      return [canvas.clientWidth, canvas.clientHeight, orbit.content];
    }), [72, 72, "none"], "Home retains the original emblem and its size.");
    await page.screenshot({path:`${directory}/home-${label}-${width}x${height}.png`,animations:"disabled"});
    const initial=await page.locator('#field').evaluate(canvas=>canvas.toDataURL());await page.evaluate(()=>window.stepMotion(180));const moved=await page.locator('#field').evaluate(canvas=>canvas.toDataURL());assert.notEqual(moved,initial,"Ambient current must actually change its pixels.");
    await page.screenshot({path:`${directory}/home-${label}-moving-${width}x${height}.png`,animations:"disabled"});
    await page.locator('[data-page="tools"]').click();await page.locator('.tool-workspace').waitFor();assert.equal(await page.evaluate(()=>window.stepMotion(3)),0,"Leaving Home must release the animation loop.");
    assert.deepEqual(await shell(), homeShell, "Tools uses the same window and navigation geometry as Home.");
    assert.equal(await page.locator("#field").evaluate(canvas => canvas.toDataURL()), moved, "The field retains its last frame outside Home.");
    await page.locator('[data-page="library"]').click();
    await page.locator('.library-navigation').waitFor();
    assert.deepEqual(await shell(), homeShell, "Library uses the same window and navigation geometry as Home.");
    await page.screenshot({path:`${directory}/library-${label}-${width}x${height}.png`,animations:"disabled"});
    await page.locator('[data-page="platforms"]').click();
    assert.deepEqual(await page.locator("#main").evaluate(main => {
      return [main.scrollWidth <= main.clientWidth + 1, getComputedStyle(main).overflowX, getComputedStyle(main).overflowY];
    }), [true, "hidden", "auto"], "Platforms stays within the horizontal viewport and scrolls vertically.");
    await page.mouse.move(width - 60, height - 70);
    await page.mouse.wheel(700, 260);
    await page.waitForFunction(() => document.querySelector("#main").scrollTop > 0, undefined, { polling: 50 });
    assert.equal(await page.locator("#main").evaluate(main => main.scrollLeft), 0);
    await page.locator("#main").evaluate(main => { main.scrollTop = 0; });
    const browserMenu = selector => page.locator(selector).evaluate(element =>
      element.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 })));
    assert.equal(await browserMenu(".platform-row[data-id='github']"), false);
    assert.equal(await browserMenu("#platform-search"), true);
    await page.locator("#platform-search").fill("YouTube");
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Tab");
    assert.deepEqual(await page.locator("#platform-search").evaluate(input => {
      return [document.activeElement === input, getComputedStyle(input).outlineStyle, getComputedStyle(input.closest("label")).outlineStyle];
    }), [true, "none", "solid"], "The search control has one visible focus ring.");
    const logo = page.locator(".platform-row[data-id='youtube'] .platform-symbol img");
    await logo.waitFor();
    await page.waitForFunction(() => document.querySelector(".platform-row[data-id='youtube'] img")?.naturalWidth > 0, undefined, { polling: 50 });
    assert.match(await logo.getAttribute("src"), /^\/platform-logos\/youtube\.svg$/);
    await page.locator("#platform-search").fill("");
    await page.waitForFunction(() => document.querySelector(".platform-row[data-id='github'] img")?.naturalWidth > 0, undefined, { polling: 50 });
    await page.screenshot({path:`${directory}/platforms-${label}-${width}x${height}.png`,animations:"disabled"});
    await page.emulateMedia({reducedMotion:"reduce"});await page.locator('[data-page="home"]').click();await page.locator('#url').waitFor();const quiet=await page.locator('#field').evaluate(canvas=>canvas.toDataURL());assert.equal(await page.evaluate(()=>window.stepMotion(180)),0,"Reduced motion must remain static.");assert.equal(await page.locator('#field').evaluate(canvas=>canvas.toDataURL()),quiet);
    await page.locator('#url').focus();await page.keyboard.insertText('https://example.org/selected-file');assert.equal(await page.locator('#url').inputValue(),'https://example.org/selected-file');assert.deepEqual(errors,[]);
    checks.push({viewport:[width,height],actualPixelMotion:true,leavingHomeStopsLoop:true,reducedMotionStatic:true,keyboardInputPreserved:true,sameShell:true,localPlatformLogos:true});await page.close();
  }
  if(!before)await writeFile(`${directory}/verification${cpu?"-cpu":""}.json`,JSON.stringify({scope:"Real rendered UI and graphics with local IPC fixtures; deterministic animation clock",checks},null,2));
  console.log(JSON.stringify({before,checks},null,2));
}finally{await browser.close();}
