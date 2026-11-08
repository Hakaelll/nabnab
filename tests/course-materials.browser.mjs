// IPC fixtures exercise navigation and study without launching a provider or using accounts.
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { createRequire } from "node:module";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({
  headless: true,
  ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }),
});
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = process.env.NABNAB_COURSE_ARTIFACTS || "artifacts/course-materials";
await mkdir(directory, { recursive: true });
const page = await browser.newPage({ viewport: { width: 1180, height: 780 } });
const errors = [];
page.on("pageerror", error => errors.push(error.message));
const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri = true;
  mockWindows("main");
  const settings = {folder:"C:/Downloads/nabnab-fixture",language:"es",networkLimit:3,processingLimit:1,reducedMotion:true,reducedTransparency:true,sidebarExpanded:true,trayEnabled:false,closeToTray:false,startWithWindows:false};
  const state = {settings,jobs:[],library:[],collections:[],drafts:[],sources:[],rules:[],smartCollections:[],presets:[]};
  const fixture = window.courseFixture = {calls:[],courses:[],cancel:true};
  mockIPC(async (cmd, args = {}) => {
    fixture.calls.push({cmd,args:structuredClone(args)});
    if (cmd === "startup") return {...structuredClone(state),folder:settings.folder,enginesReady:true,engineError:null};
    if (cmd === "get_snapshot") return structuredClone(state);
    if (cmd === "list_courses") return structuredClone(fixture.courses);
    if (cmd === "open_course_site") return;
    if (cmd === "add_skillshare_resources") {
      if (args.lessonId) {
        const course = fixture.courses.find(item => item.url === args.url);
        const lesson = course.sections.flatMap(section => section.lessons).find(item => item.id === args.lessonId);
        lesson.assets.push({id:"skillshare-lesson-resource",title:"Referencias.pdf",kind:"attachment",status:"downloadable",libraryId:null,urlRef:null,jobIds:[]});
        return structuredClone(course);
      }
      const attachments = ["Guía del proyecto.pdf", "Plantillas.zip"];
      const course = {id:"skillshare-fixture",provider:"skillshare",title:args.title,url:args.url,createdAt:1,
        sections:[{id:"skillshare-resources",title:"Recursos de Skillshare",lessons:attachments.map((name,index)=>({
          id:"resource-"+index,title:name,learned:false,notes:[],assets:[{id:"skillshare-resource-"+index,title:name,
            kind:"attachment",status:"downloadable",libraryId:null,urlRef:null,jobIds:[]}]}))},
          {id:"skillshare-unit-2",title:"02 Composición",lessons:[{id:"skillshare-lesson-2",title:"Organizar la página",
            learned:true,notes:[{id:"skillshare-note",text:"Revisar la cuadrícula",time:null,bookmarkId:null}],assets:[]}]}]};
      fixture.courses.push(course);
      return structuredClone(course);
    }
    if (cmd === "queue_course_assets") {
      const course = fixture.courses.find(course => course.id === args.courseId);
      const queued = course.sections.flatMap(section => section.lessons).flatMap(lesson => lesson.assets)
        .filter(asset => args.assetIds.includes(asset.id)).map(asset => {
          const id = "queued-" + asset.id;
          asset.jobIds.push(id);
          return {id,runId:1,updatedAt:1,state:"pending",outputs:[]};
        });
      state.jobs.push(...queued);
      return structuredClone(queued);
    }
    if (cmd === "preview_course_materials") {
      if (fixture.cancel) return null;
      const id = fixture.calls.length;
      fixture.plan = {id, provider:args.provider, title:args.title, url:args.url, folder:args.folder, lessonId:args.lessonId};
      if (args.lessonId) return {id, provider:args.provider, title:args.title, groups:[{title:"Descargas",
        files:[{id:3,name:"Material de referencia.pdf",kind:"document",size:4096},{id:4,name:"Recursos opcionales.zip",kind:"file",size:8192}]}]};
      if (fixture.many) return {id, provider:args.provider, title:args.title, groups:[{title:"02 Extras",
        files:Array.from({length:24}, (_, i) => ({id:i,name:"Apuntes de la unidad "+(i+1)+".txt",kind:"document",size:1024}))}]};
      return {id, provider:args.provider, title:args.title, groups:args.folder
        ? [{title:"02 Extras", files:[{id:1,name:"01 Repaso.pdf",kind:"document",size:1024},{id:2,name:"03 Guía opcional.txt",kind:"document",size:2048}]}]
        : [{title:"01 Fundamentos",files:[{id:0,name:"02 Preparar.pdf",kind:"document",size:1024}]}]};
    }
    if (cmd === "discard_course_materials") {
      if (fixture.plan?.id === args.reviewId) fixture.plan = null;
      return;
    }
    if (cmd === "import_course_materials") {
      const plan = fixture.plan;
      if (!plan || plan.id !== args.reviewId) throw Error("Fixture review expired.");
      fixture.plan = null;
      if (plan.lessonId) {
        const course = fixture.courses.find(course => course.url === plan.url);
        const lesson = course.sections.flatMap(section => section.lessons).find(lesson => lesson.id === plan.lessonId);
        if (args.ids.includes(3)) {
          const item = {...state.library[0],id:"material-reference",name:"Material de referencia.pdf",path:"C:/Downloads/Material de referencia.pdf"};
          state.library.push(item);
          lesson.assets.push({...lesson.assets[0],id:"asset-reference",title:item.name,libraryId:item.id});
        }
        return structuredClone(course);
      }
      state.library = [{id:"material-pdf",name:"02 Preparar.pdf",path:"C:/Fixture/01 Fundamentos/02 Preparar.pdf",kind:"document",size:1024,missing:false,collections:[],tags:[]}];
      const asset = {id:"asset-pdf",title:"02 Preparar.pdf",kind:"document",status:"local",libraryId:"material-pdf",urlRef:null,jobIds:[]};
      const lesson = {id:"lesson-1",title:"02 Preparar",learned:true,assets:[asset],notes:[{id:"note-1",text:"Revisar la plantilla",time:null,bookmarkId:null}]};
      const course = {id:"course-fixture",provider:plan.provider,title:plan.title,url:plan.url,sections:[{id:"section-1",title:"01 Fundamentos",lessons:[lesson]}],createdAt:1};
      if (plan.folder && args.ids.includes(1)) {
        state.library.push({...state.library[0],id:"material-extras",name:"01 Repaso.pdf",path:"C:/Fixture/02 Extras/01 Repaso.pdf"});
        course.sections.push({id:"section-2",title:"02 Extras",lessons:[
          {id:"lesson-2",title:"01 Repaso",learned:false,assets:[{...asset,id:"asset-extras",title:"01 Repaso.pdf",libraryId:"material-extras"}],notes:[]},
        ]});
      }
      fixture.courses = [course];
      return structuredClone(course);
    }
    if (cmd === "engine_status" || cmd === "sessions_status" || cmd === "optional_components") return [];
    if (cmd === "plugin:window|is_maximized") return false;
  }, {shouldMockEvents:true});
  await import("/src/main.ts");
`;
await page.route(base, async route => {
  const response = await route.fetch();
  const html = (await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/, `<script type="module">${setup}</script>`);
  await route.fulfill({ response, body: html });
});

try {
  await page.goto(base);
  await page.locator("[data-page='platforms']").click();
  await page.locator("[data-action='open-courses'][data-id='domestika']").click();
  await page.getByRole("heading", { name: "Guardar materiales del curso" }).waitFor();
  for (const [provider, guidance] of [
    ["rocketseat", /ruta de la formación/],
    ["coursera", /opciones de descarga/],
    ["skillshare", /Proyectos y recursos/],
    ["domestika", /Recursos adicionales/],
  ]) {
    await page.getByLabel("Plataforma", { exact: true }).selectOption(provider);
    assert.match(await page.locator("#course-browser-form").innerText(), guidance);
    assert.equal(await page.locator("#course-token").count(), 0);
  }

  const url = "https://www.domestika.org/es/courses/24-dibujo";
  await page.getByLabel("Enlace del curso").fill(url);
  await page.getByRole("button", { name: "Abrir curso en el navegador" }).click();
  assert.deepEqual(await page.evaluate(() => window.courseFixture.calls.find(call => call.cmd === "open_course_site")?.args), {
    provider: "domestika", url,
  });
  await page.getByLabel("Nombre del curso").fill("Dibujo y composición");
  const choose = page.getByRole("button", { name: "Elegir archivos guardados" });
  await choose.click();
  await choose.waitFor();
  assert.equal(await page.locator(".course-layout").count(), 0, "Cancel leaves the import form open");
  const chooseFolder = page.getByRole("button", { name: "Elegir carpeta del curso" });
  await chooseFolder.click();
  await page.waitForFunction(() => window.courseFixture.calls.some(call => call.cmd === "preview_course_materials" && call.args.folder));
  await chooseFolder.waitFor();
  assert.equal(await chooseFolder.evaluate(button => button === document.activeElement), true);
  await page.screenshot({ path: `${directory}/form-1180x780.png` });

  await page.setViewportSize({ width: 900, height: 620 });
  assert.ok(await page.locator("#course-browser-form").evaluate(form => form.scrollWidth <= form.clientWidth + 1));
  await page.evaluate(() => { window.courseFixture.cancel = false; });
  await choose.focus();
  await page.keyboard.press("Enter");
  const heading = page.getByRole("heading", { name: "Revisar materiales" });
  await heading.waitFor();
  assert.equal(await heading.evaluate(node => node === document.activeElement), true);
  const add = page.getByRole("button", { name: "Añadir al curso" });
  assert.equal(await add.isDisabled(), true);
  await page.getByRole("button", { name: "Volver", exact: true }).click();
  assert.equal(await choose.evaluate(node => node === document.activeElement), true);
  assert.equal(await page.evaluate(() => window.courseFixture.calls.some(call => call.cmd === "import_course_materials")), false);
  await choose.click();
  const chosen = page.getByRole("checkbox", { name: /^02 Preparar.pdf/ });
  await chosen.press("Space");
  assert.equal(await chosen.evaluate(node => node === document.activeElement), true);
  assert.match(await page.locator(".course-review-footer output").innerText(), /1 archivo/);
  await add.click();
  await page.getByRole("heading", { name: "Dibujo y composición" }).waitFor();
  assert.equal(await page.getByText("Revisar la plantilla", { exact: true }).count(), 1);
  assert.equal(await page.getByText("Adjunto · Guardado", { exact: true }).count(), 1);
  assert.equal(await page.getByRole("button", { name: "Añadir a la cola" }).count(), 0);
  assert.equal(await page.getByRole("button", { name: "Ver en biblioteca" }).count(), 1);
  const overflow = await page.locator(".course-layout").evaluate(element => element.scrollWidth - element.clientWidth);
  assert.ok(overflow <= 1, `Compact study view overflows horizontally by ${overflow}px`);
  await page.getByText("Materiales añadidos al curso.", { exact: true }).waitFor();
  await page.locator("[data-action='dismiss-message']").click();
  await page.getByRole("button", { name: "Añadir carpeta", exact: true }).click();
  await heading.waitFor();
  assert.equal(await add.isDisabled(), true);
  await page.getByRole("button", { name: "Seleccionar grupo" }).click();
  const optional = page.getByRole("checkbox", { name: /^03 Guía opcional/ });
  assert.equal(await optional.isChecked(), true);
  await optional.uncheck();
  for (const [width, height] of [[900, 620], [1180, 780]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await page.locator(".course-review").evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await page.screenshot({ path: `${directory}/review-${width}x${height}.png` });
  }
  await page.setViewportSize({ width: 900, height: 620 });
  await add.click();
  await page.getByText("02 Extras", { exact: true }).waitFor();
  const imports = await page.evaluate(() => window.courseFixture.calls.filter(call => call.cmd === "import_course_materials").map(call => call.args.ids));
  assert.deepEqual(imports, [[0], [1]]);
  assert.equal(await page.getByText("Revisar la plantilla", { exact: true }).count(), 1);
  await page.getByText("Materiales añadidos al curso.", { exact: true }).waitFor();
  await page.locator("[data-action='dismiss-message']").click();
  await page.screenshot({ path: `${directory}/study-900x620.png` });
  const attach = page.getByRole("button", {name:"Añadir adjuntos",exact:true});
  await attach.press("Enter");
  await heading.waitFor();
  assert.match(await page.locator(".course-review").innerText(), /Adjuntos para 02 Preparar/);
  await page.getByRole("button", {name:"Volver",exact:true}).click();
  assert.equal(await attach.evaluate(node => node === document.activeElement), true);
  await attach.press("Enter");
  await page.getByRole("checkbox", {name:/^Material de referencia.pdf/}).check();
  assert.ok(await page.locator(".course-review").evaluate(node => node.scrollWidth <= node.clientWidth + 1));
  await page.screenshot({path:`${directory}/attachment-review-900x620.png`,animations:"disabled"});
  await page.getByRole("button", {name:"Añadir a la lección",exact:true}).click();
  await page.getByText("Material de referencia.pdf", {exact:true}).waitFor();
  await page.waitForFunction(() => document.activeElement.id === "course-materials-heading");
  const attached = await page.evaluate(() => {
    const {calls,courses} = window.courseFixture;
    return {target:calls.filter(call => call.cmd === "preview_course_materials").at(-1).args.lessonId,
      selected:calls.filter(call => call.cmd === "import_course_materials").at(-1).args.ids,
      lessons:courses[0].sections.map(section => section.lessons.map(lesson => lesson.id))};
  });
  assert.deepEqual(attached,{target:"lesson-1",selected:[3],lessons:[["lesson-1"],["lesson-2"]]});
  assert.equal(await page.getByText("Revisar la plantilla", {exact:true}).count(), 1);
  assert.equal(await page.getByText("Recursos opcionales.zip", {exact:true}).count(), 0);
  await page.getByText("Adjuntos añadidos a la lección.", {exact:true}).waitFor();
  await page.locator("[data-action='dismiss-message']").click();
  for (const [width,height] of [[900,620],[1180,780]]) {
    await page.setViewportSize({width,height});
    assert.ok(await page.locator(".course-materials-heading").evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await page.locator("#main").evaluate(node => { node.scrollTop = 0; });
    await page.screenshot({path:`${directory}/attachment-study-${width}x${height}.png`,animations:"disabled"});
  }
  await page.setViewportSize({width:900,height:620});
  await page.evaluate(() => { window.courseFixture.many = true; });
  await page.getByRole("button", { name: "Añadir carpeta", exact: true }).click();
  await heading.waitFor();
  const reviewFooter = page.locator(".course-review-footer");
  assert.ok(await reviewFooter.evaluate(node => {
    const main = node.closest("main");
    return main.scrollHeight > main.clientHeight && node.getBoundingClientRect().bottom <= main.getBoundingClientRect().bottom + 1;
  }));
  const ratio = await page.locator(".course-review-file span small").first().evaluate(label => {
    const linear = channel => channel <= .04045 ? channel / 12.92 : ((channel + .055) / 1.055) ** 2.4;
    const luminance = color => {
      const values = color.match(/[\d.]+/g).slice(0, 3).map(value => linear(Number(value) / 255));
      return .2126 * values[0] + .7152 * values[1] + .0722 * values[2];
    };
    return (luminance(getComputedStyle(label).color) + .05) /
      (luminance(getComputedStyle(label.closest("details")).backgroundColor) + .05);
  });
  assert.ok(ratio >= 4.5, `Course review file metadata contrast was ${ratio.toFixed(2)}:1`);
  await page.getByRole("button", { name: "Volver", exact: true }).click();
  await page.locator("[data-page='home']").click();
  await page.locator("#url").fill("https://www.coursera.org/learn/apis/home");
  await page.locator("#course-browser-form").waitFor();
  assert.equal(await page.getByLabel("Plataforma", { exact: true }).inputValue(), "coursera");
  await page.getByLabel("Plataforma", { exact: true }).selectOption("skillshare");
  const skillshare = "https://www.skillshare.com/classes/Example-Design/123456";
  const resources = ["https://static.skillshare.com/uploads/attachment/12345678/90abcdef/Guide.pdf",
    "https://static.skillshare.com/uploads/attachment/12345679/90abcded/Templates.zip"];
  await page.getByLabel("Enlace del curso").fill(skillshare);
  await page.getByLabel("Nombre del curso").fill("Diseño de un cuaderno");
  await page.locator("#course-skillshare-resources summary").press("Enter");
  await page.getByLabel("Enlaces a adjuntos").fill(resources.join("\n"));
  await page.getByRole("button", {name:"Revisar enlaces"}).click();
  await page.getByRole("heading", {name:"Diseño de un cuaderno"}).waitFor();
  assert.equal(await page.getByText("Skillshare · 1 lección · 2 adjuntos").count(), 1);
  const reviewCall = await page.evaluate(() => window.courseFixture.calls.find(call => call.cmd === "add_skillshare_resources").args);
  assert.deepEqual(reviewCall, {url:skillshare,title:"Diseño de un cuaderno",links:resources,lessonId:null});
  assert.equal(await page.evaluate(() => window.courseFixture.calls.some(call => call.cmd === "queue_course_assets")), false);
  const queue = page.getByRole("button", {name:"Añadir a la cola"});
  assert.equal(await queue.isDisabled(), true);
  await page.getByRole("checkbox", {name:"Seleccionar Guía del proyecto.pdf"}).press("Space");
  assert.equal(await page.locator("#course-skillshare-resources").evaluate(entry => entry.open), false);
  for (const [width,height] of [[900,620],[1180,780]]) {
    await page.setViewportSize({width,height});
    assert.ok(await page.locator(".course-layout").evaluate(node => {
      const tree = node.querySelector(".course-tree"), panel = node.querySelector(".course-study");
      return node.scrollWidth <= node.clientWidth + 1 && tree.scrollWidth <= tree.clientWidth + 1
        && tree.getBoundingClientRect().right <= panel.getBoundingClientRect().left;
    }));
    await page.locator("#main").evaluate(node => { node.scrollTop = 0; });
    await page.screenshot({path:`${directory}/skillshare-${width}x${height}.png`,animations:"disabled"});
  }
  await queue.click();
  await page.getByText("Adjunto · En cola", {exact:true}).waitFor();
  const sent = await page.evaluate(() => window.courseFixture.calls.find(call => call.cmd === "queue_course_assets").args);
  assert.deepEqual(sent.assetIds, ["skillshare-resource-0"]);
  await page.evaluate(async () => {
    const {emit} = await import("/node_modules/@tauri-apps/api/event.js");
    await emit("job-update", {id:"queued-skillshare-resource-0",runId:1,updatedAt:2,state:"error",outputs:[]});
  });
  await page.getByText("Adjunto · Descarga fallida", {exact:true}).waitFor();
  assert.equal(await page.getByRole("checkbox", {name:"Seleccionar Guía del proyecto.pdf"}).isEnabled(), true);
  await page.getByRole("button", {name:"Plantillas.zip",exact:true}).click();
  assert.equal(await page.getByRole("checkbox", {name:"Seleccionar Plantillas.zip"}).isChecked(), false);
  const lessonResource = "https://static.skillshare.com/uploads/attachment/12345680/90abcded/Referencias.pdf";
  await page.locator("#course-skillshare-resources summary").press("Enter");
  const destination = page.getByLabel("Guardar en", {exact:true});
  assert.equal(await destination.inputValue(), "");
  assert.deepEqual(await destination.locator("optgroup option").allTextContents(), ["Organizar la página"]);
  await destination.selectOption({label:"Organizar la página"});
  await page.getByLabel("Enlaces a adjuntos").fill(lessonResource);
  for (const [width,height] of [[900,620],[1180,780]]) {
    await page.setViewportSize({width,height});
    const entry = page.locator("#course-skillshare-resources");
    await entry.scrollIntoViewIfNeeded();
    assert.ok(await entry.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await page.screenshot({path:`${directory}/skillshare-destination-${width}x${height}.png`,animations:"disabled"});
  }
  await page.getByRole("button", {name:"Revisar enlaces"}).press("Enter");
  const material = page.getByRole("checkbox", {name:"Seleccionar Referencias.pdf"});
  await material.waitFor();
  assert.equal(await page.getByRole("heading", {name:"Organizar la página"}).count(), 1);
  assert.equal(await page.getByText("Skillshare · 1 lección · 3 adjuntos", {exact:true}).count(), 1);
  assert.equal(await page.getByText("Revisar la cuadrícula", {exact:true}).count(), 1);
  assert.ok(await page.getByRole("heading", {name:"Materiales",exact:true}).evaluate(node => node === document.activeElement));
  const lessonCall = await page.evaluate(() => window.courseFixture.calls.filter(call => call.cmd === "add_skillshare_resources").at(-1).args);
  assert.deepEqual(lessonCall, {url:skillshare,title:"Diseño de un cuaderno",links:[lessonResource],lessonId:"skillshare-lesson-2"});
  assert.equal(await page.evaluate(() => window.courseFixture.calls.filter(call => call.cmd === "queue_course_assets").length), 1);
  await material.press("Space");
  await queue.click();
  await page.getByText("Adjunto · En cola", {exact:true}).waitFor();
  const queuedLesson = await page.evaluate(() => window.courseFixture.calls.filter(call => call.cmd === "queue_course_assets").at(-1).args);
  assert.deepEqual(queuedLesson.assetIds, ["skillshare-lesson-resource"]);
  assert.deepEqual(errors, []);
  console.log("Browser-assisted import and selective Skillshare queueing for course and lesson attachments passed at two widths.");
} finally {
  await browser.close();
}
