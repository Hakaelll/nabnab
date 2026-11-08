// Controller and rendered form checks with IPC fixtures; native image proofs are Rust opt-in tests.
import assert from "node:assert/strict";
import {createRequire} from "node:module";
import {mkdir} from "node:fs/promises";
const {chromium}=createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE||"playwright");
const browser=await chromium.launch({headless:true,...(process.env.EDGE_EXECUTABLE?{executablePath:process.env.EDGE_EXECUTABLE}:{channel:"msedge"})});
const page=await browser.newPage({viewport:{width:1180,height:900}});
const errors=[];page.on("pageerror",e=>errors.push(e.message));
const base=process.env.NABNAB_UI_URL||"http://127.0.0.1:1420/";
await page.route(base,route=>route.fulfill({contentType:"text/html",body:"<!doctype html><html><head><meta charset=utf-8></head><body></body></html>"}));
await page.goto(base);
await page.evaluate(async()=>{
  const {createImageAssets}=await import("/src/image-assets-ui.ts");
  await import("/src/style.css");
  document.body.innerHTML='<main id="image-assets-fixture" style="max-width:1040px;margin:24px auto;height:850px;overflow:auto;padding:16px"></main>';
  const root=document.getElementById("image-assets-fixture");
  const library=[{id:"a",name:"Screenshot A.png",kind:"image"},{id:"b",name:"Screenshot B.png",kind:"image"},{id:"hostile",name:'<img src=x onerror=alert(1)>.png',kind:"image"}];
  const fixture=window.imageAssetsFixture={calls:[],notifications:[],pending:null,delay:false};
  const bytes=Array.from(Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aWQkAAAAASUVORK5CYII="),c=>c.charCodeAt(0)));
  const snapshot={settings:{folder:"C:/Downloads/nabnab-fixture"},library};
  let controller;
  const render=()=>{root.innerHTML=controller.render();};
  controller=createImageAssets({snapshot:()=>snapshot,render,refresh:async()=>{},notify:m=>fixture.notifications.push(m),t:es=>es,command:async(name,args)=>{
    fixture.calls.push({name,args:structuredClone(args)});
    if(name==="image_assets_inspect")return {images:args.ids.map(id=>({id,name:library.find(i=>i.id===id).name,width:64,height:80})),suggestions:args.stitch?[{index:1,cut:20,score:1,ambiguous:false}]:[],warnings:[]};
    if(name==="cancel_image_assets_preview"){fixture.pending?.();fixture.pending=null;return;}
    if(name==="image_assets_preview"){
      if(fixture.delay)return new Promise((_,reject)=>fixture.pending=()=>reject(new Error("Cancelled")));
      return {previewId:"preview-"+fixture.calls.length,png:bytes,width:134,height:100,files:[{name:args.options.kind==="stitch"?"joined-screenshots.png":args.options.kind==="icons"?"icon-32.png":args.options.kind==="split"?"frame-0000.png":"atlas.png",width:134,height:100}],warnings:[]};
    }
    if(name==="queue_image_assets")return [];
    throw new Error("Unexpected command "+name);
  }});
  document.addEventListener("input",e=>{if(e.target instanceof HTMLElement)controller.input(e.target);});
  document.addEventListener("change",e=>{if(e.target instanceof HTMLElement)controller.change(e.target);});
  document.addEventListener("click",e=>{const button=e.target instanceof Element?e.target.closest("button[data-action]"):null;if(button instanceof HTMLButtonElement)void controller.click(button.dataset.action,button);});
  document.addEventListener("submit",e=>{if(e.target instanceof HTMLFormElement){e.preventDefault();void controller.submit(e.target);}});
  render();
});
try{
  await page.locator("#asset-input-a").check();await page.locator("#asset-input-b").check();
  await page.getByRole("button",{name:"Buscar solapamientos",exact:true}).click();
  await page.getByRole("button",{name:"Probar 20 px",exact:true}).click();assert.equal(await page.locator("#asset-cut-1").inputValue(),"20");
  await page.getByRole("button",{name:"Previsualizar resultado",exact:true}).click();await page.getByRole("button",{name:"Crear en la cola",exact:true}).waitFor();
  await page.locator("#asset-cut-1").fill("10");await page.locator("#asset-cut-1").dispatchEvent("change");assert.equal(await page.getByRole("button",{name:"Crear en la cola",exact:true}).isDisabled(),true);
  await page.getByRole("button",{name:"Previsualizar resultado",exact:true}).click();await page.getByRole("button",{name:"Crear en la cola",exact:true}).click();
  let calls=await page.evaluate(()=>window.imageAssetsFixture.calls);let queued=calls.filter(c=>c.name==="queue_image_assets").at(-1);assert.deepEqual(queued.args.options,{kind:"stitch",cuts:[10]});assert.ok(queued.args.previewId);
  await page.getByRole("button",{name:"Paquete de iconos",exact:true}).click();await page.locator("#asset-background").selectOption("solid");await page.locator("#asset-color").fill("#112233");await page.locator("#asset-color").dispatchEvent("change");
  await page.getByRole("button",{name:"Previsualizar resultado",exact:true}).click();await page.getByRole("button",{name:"Crear en la cola",exact:true}).click();calls=await page.evaluate(()=>window.imageAssetsFixture.calls);queued=calls.filter(c=>c.name==="queue_image_assets").at(-1);assert.equal(queued.args.options.kind,"icons");assert.deepEqual(queued.args.options.background,{kind:"solid",color:"#112233"});assert.deepEqual(queued.args.ids,["a"]);
  await page.getByRole("button",{name:"Separar cuadrícula",exact:true}).click();await page.locator("#asset-columns").fill("2");await page.locator("#asset-columns").dispatchEvent("change");await page.locator("#asset-rows").fill("2");await page.locator("#asset-rows").dispatchEvent("change");await page.locator("#asset-order").selectOption("columnMajor");
  await page.getByRole("button",{name:"Previsualizar resultado",exact:true}).click();await page.getByRole("button",{name:"Crear en la cola",exact:true}).click();calls=await page.evaluate(()=>window.imageAssetsFixture.calls);queued=calls.filter(c=>c.name==="queue_image_assets").at(-1);assert.deepEqual(queued.args.options,{kind:"split",columns:2,rows:2,padding:0,order:"columnMajor"});
  await page.getByRole("button",{name:"Crear atlas",exact:true}).click();await page.locator("#asset-input-b").check();await page.locator('[data-action="asset-up"][data-id="b"]').click();
  await page.getByRole("button",{name:"Previsualizar resultado",exact:true}).click();await page.getByRole("button",{name:"Crear en la cola",exact:true}).click();calls=await page.evaluate(()=>window.imageAssetsFixture.calls);queued=calls.filter(c=>c.name==="queue_image_assets").at(-1);assert.deepEqual(queued.args.ids,["b","a"]);
  assert.equal(await page.locator('img[src="x"]').count(),0);assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
  await page.evaluate(()=>{window.imageAssetsFixture.delay=true;});await page.getByRole("button",{name:"Previsualizar resultado",exact:true}).click();await page.getByRole("button",{name:"Cancelar",exact:true}).click();await page.getByRole("alert").waitFor();assert.equal(await page.getByRole("button",{name:"Crear en la cola",exact:true}).isDisabled(),true);
  await mkdir("artifacts/image-assets",{recursive:true});await page.screenshot({path:"artifacts/image-assets/controller.png",fullPage:true});
  const published = await page.evaluate(async () => {
    const [{ createUpscaleTool }, { createBackgroundTool }] = await Promise.all([
      import("/src/upscale-ui.ts"), import("/src/background-ui.ts"),
    ]);
    const file = (id, parentId, missing = false) => ({
      id, parentId, name: `${id}.png`, path: `C:/out/${id}.png`, kind: "image", width: 512, height: 384, size: 9216, missing,
    });
    const job = (kind, groupId, state, libraryId, output) => ({
      source: { kind, selection: { libraryId } }, groupId, state, outputs: [`C:/out/${output}.png`],
    });
    const snapshot = {
      settings: { folder: "C:/out" },
      jobs: [
        job("upscale", "earlier", "completed", "a", "old-upscale"),
        job("upscale", "batch", "completed", "a", "latest-upscale"),
        job("upscale", "batch", "processing", "b", "still-running"),
        job("upscale", "batch", "completed", "c", "unrelated"),
        job("upscale", "batch", "completed", "d", "missing"),
        job("background", "cutout", "completed", "a", "latest-cutout"),
      ],
      library: [file("old-upscale", "a"), file("latest-upscale", "a"), file("still-running", "b"),
        file("unrelated", "x"), file("missing", "d", true), file("latest-cutout", "a")],
    };
    const hooks = { t: es => es, snapshot: () => snapshot };
    return [createUpscaleTool(hooks), createBackgroundTool(hooks)].map(controller => {
      const tree = new DOMParser().parseFromString(controller.render(), "text/html");
      const region = tree.querySelector(".asset-exports");
      return {
        files: Array.from(region.querySelectorAll('[data-action="share-file"]'), button => button.dataset.id),
        status: region.querySelector(".section-heading > span")?.textContent ?? null,
        size: region.querySelector("li small").textContent,
      };
    });
  });
  assert.deepEqual(published, [
    { files: ["latest-upscale"], status: "1 de 4 disponibles", size: "512 × 384 px · 9 KB" },
    { files: ["latest-cutout"], status: null, size: "512 × 384 px · 9 KB" },
  ]);
  assert.deepEqual(errors,[]);console.log("Image asset controller: options, stale previews, ordering, cancellation, escaping, layout, and exact published exports passed.");
}finally{await browser.close();}
