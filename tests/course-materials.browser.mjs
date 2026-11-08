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
const directory = "artifacts/course-materials";
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
    if (cmd === "import_course_materials") {
      if (fixture.cancel) return null;
      state.library = [{id:"material-pdf",name:"02 Preparar.pdf",path:"C:/Fixture/01 Fundamentos/02 Preparar.pdf",kind:"document",size:1024,missing:false,collections:[],tags:[]}];
      const asset = {id:"asset-pdf",title:"02 Preparar.pdf",kind:"document",status:"local",libraryId:"material-pdf",urlRef:null,jobIds:[]};
      const lesson = {id:"lesson-1",title:"02 Preparar",learned:true,assets:[asset],notes:[{id:"note-1",text:"Revisar la plantilla",time:null,bookmarkId:null}]};
      const course = {id:"course-fixture",provider:args.provider,title:args.title,url:args.url,sections:[{id:"section-1",title:"01 Fundamentos",lessons:[lesson]}],createdAt:1};
      if (args.folder) {
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
  await page.waitForFunction(() => window.courseFixture.calls.some(call => call.cmd === "import_course_materials" && call.args.folder));
  await chooseFolder.waitFor();
  assert.equal(await chooseFolder.evaluate(button => button === document.activeElement), true);
  await page.screenshot({ path: `${directory}/form-1180x780.png` });

  await page.setViewportSize({ width: 900, height: 620 });
  assert.ok(await page.locator("#course-browser-form").evaluate(form => form.scrollWidth <= form.clientWidth + 1));
  await page.evaluate(() => { window.courseFixture.cancel = false; });
  await choose.focus();
  await page.keyboard.press("Enter");
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
  await page.getByText("02 Extras", { exact: true }).waitFor();
  const imports = await page.evaluate(() => window.courseFixture.calls.filter(call => call.cmd === "import_course_materials").map(call => call.args.folder));
  assert.deepEqual(imports, [false, true, false, true]);
  assert.equal(await page.getByText("Revisar la plantilla", { exact: true }).count(), 1);
  await page.getByText("Materiales añadidos al curso.", { exact: true }).waitFor();
  await page.locator("[data-action='dismiss-message']").click();
  await page.screenshot({ path: `${directory}/study-900x620.png` });
  await page.locator("[data-page='home']").click();
  await page.locator("#url").fill("https://www.coursera.org/learn/apis/home");
  await page.locator("#course-browser-form").waitFor();
  assert.equal(await page.getByLabel("Plataforma", { exact: true }).inputValue(), "coursera");
  assert.deepEqual(errors, []);
  console.log("Four browser-assisted course choices, file and folder cancellation, keyboard import, compact study, and incremental folder import passed.");
} finally {
  await browser.close();
}
