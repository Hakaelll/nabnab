// Rendered inspector behavior with local state fixtures; no provider or native engine is represented.
import assert from "node:assert/strict";
import {createRequire} from "node:module";
const {chromium}=createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE||"playwright");
const browser=await chromium.launch({headless:true,...(process.env.EDGE_EXECUTABLE?{executablePath:process.env.EDGE_EXECUTABLE}:{channel:"msedge"})});
const page=await browser.newPage();const errors=[];page.on("pageerror",e=>errors.push(e.message));
const base=process.env.NABNAB_UI_URL||"http://127.0.0.1:1420/";
await page.route(base,route=>route.fulfill({contentType:"text/html",body:"<!doctype html><html><body><main id=inspector-fixture></main></body></html>"}));
await page.goto(base);
try{
  await page.evaluate(async()=>{
    const {mediaInspector}=await import("/src/views-library.ts");
    const {createLibraryFeatures}=await import("/src/library-features.ts");
    const {emptyLibraryDetails}=await import("/src/library-types.ts");
    const kinds=["video","audio","subtitle","file","image","article","document"];
    const items=kinds.map(kind=>({id:kind,name:kind==="document"?"Fixture.pdf":kind==="file"?"Fixture.zip":"Fixture."+kind,kind,path:"C:/Fixture/"+kind,size:100,duration:kind==="video"||kind==="audio"?60:null,width:null,height:null,videoCodec:null,audioCodec:null,container:null,addedAt:1,source:null,parentId:null,collections:[],tags:[],missing:false,details:structuredClone(emptyLibraryDetails)}));
    const snapshot={settings:{folder:"C:/Fixture"},library:items,collections:[]};
    const fixture=window.inspectorContextFixture={current:items[0],show:null};
    const root=document.getElementById("inspector-fixture");
    let features;const render=()=>{root.innerHTML=mediaInspector(fixture.current,snapshot,es=>es);features.bind();};
    features=createLibraryFeatures({command:async()=>{},snapshot:()=>snapshot,refresh:async()=>{},render,notify:()=>{},showError:error=>{throw error;},inspect:()=>{}});
    fixture.show=kind=>{fixture.current=items.find(item=>item.kind===kind);render();};
    document.addEventListener("click",event=>{const button=event.target instanceof Element?event.target.closest("button[data-action]"):null;if(button instanceof HTMLButtonElement)void features.handleClick(button);});
    render();
  });
  const tabIds=()=>page.locator('[data-action="library-inspector-tab"]').evaluateAll(buttons=>buttons.map(button=>button.dataset.tab));
  assert.deepEqual(await tabIds(),["details","chapters","subtitles","versions"]);
  await page.locator('[data-tab="chapters"]').click();assert.equal(await page.locator('[data-inspector-panel="chapters"]').isVisible(),true);
  for(const kind of ["file","image","article","document"]){await page.evaluate(kind=>window.inspectorContextFixture.show(kind),kind);assert.deepEqual(await tabIds(),["details","versions"]);assert.equal(await page.locator('[data-inspector-panel="details"]').isVisible(),true);assert.equal(await page.locator('[data-tab="details"]').getAttribute("aria-pressed"),"true");assert.equal(await page.locator('[data-inspector-panel="chapters"]').count(),0);assert.equal(await page.locator('[data-inspector-panel="subtitles"]').count(),0);}
  await page.evaluate(()=>window.inspectorContextFixture.show("document"));assert.equal(await page.locator('[data-action="document-open"]').count(),1);
  await page.evaluate(()=>window.inspectorContextFixture.show("subtitle"));assert.deepEqual(await tabIds(),["details","subtitles","versions"]);await page.locator('[data-tab="subtitles"]').click();assert.equal(await page.locator('[data-inspector-panel="subtitles"]').isVisible(),true);
  await page.evaluate(()=>window.inspectorContextFixture.show("file"));assert.equal(await page.locator('[data-inspector-panel="details"]').isVisible(),true);
  await page.evaluate(()=>window.inspectorContextFixture.show("audio"));assert.deepEqual(await tabIds(),["details","chapters","subtitles","versions"]);
  await page.evaluate(()=>window.inspectorContextFixture.show("video"));assert.deepEqual(await tabIds(),["details","chapters","subtitles","versions"]);assert.equal(await page.locator("#bookmark-form").count(),1);
  assert.deepEqual(errors,[]);console.log("Contextual inspector tabs and stale-tab fallback passed for video, audio, subtitles, ZIP, image, article and PDF.");
}finally{await browser.close();}
