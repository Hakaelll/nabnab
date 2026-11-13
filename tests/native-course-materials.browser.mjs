import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.resolve(root, process.env.NABNAB_NATIVE_COURSE_ARTIFACTS || "artifacts/native-course-materials");
const port = process.env.NABNAB_COURSE_DEBUG_PORT || "9268";
const endpoint = `http://127.0.0.1:${port}`;
const title = "Curso de prueba de Windows";
const url = "https://www.domestika.org/es/courses/24-curso-de-prueba";
const occupied = await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(400) }).catch(() => null);
assert.ok(!occupied, `The course test port ${port} is already in use.`);
await mkdir(artifacts, { recursive: true });
const data = await mkdtemp(path.join(artifacts, "run-"));
const fixture = path.join(data, "02 Guía de prueba.txt");
await writeFile(fixture, "Material sintético para el curso de Windows.\n");

const child = spawn(process.env.NABNAB_COURSE_TEST_EXE || path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
  cwd: root,
  windowsHide: true,
  env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
  stdio: "ignore",
});
const exited = once(child, "exit");
void exited.catch(() => {});
let browser, page;

async function connect() {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (child.exitCode !== null) throw Error(`nabnab exited with ${child.exitCode}.`);
    const response = await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(400) }).catch(() => null);
    if (response?.ok) return chromium.connectOverCDP(endpoint, { timeout: 3000 });
    await delay(150);
  }
  throw Error("The isolated WebView2 endpoint did not start.");
}

const rpc = (command, args = {}) => page.evaluate(async ({ command, args }) => {
  const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
  return invoke(command, args);
}, { command, args });

try {
  browser = await connect();
  for (let attempt = 0; attempt < 100 && !page; attempt++) {
    page = browser.contexts()[0]?.pages().find(tab => /^http:\/\/(?:localhost|127\.0\.0\.1):1420\//.test(tab.url()));
    if (!page) await delay(150);
  }
  assert.ok(page, "Run the Vite development server on port 1420.");
  let initial;
  for (let attempt = 0; attempt < 100 && !initial; attempt++) {
    initial = await rpc("get_snapshot").catch(() => null);
    if (!initial) await delay(150);
  }
  assert.equal(initial?.library.length, 0, "The isolated library must start empty.");

  await assert.rejects(
    rpc("preview_course_materials", { provider: "domestika", url, title, folder: false, lessonId: null, droppedPaths: [fixture] }),
    /Drop the course files from Explorer again/,
  );
  assert.equal((await rpc("get_snapshot")).library.length, 0);
  assert.deepEqual(await rpc("list_courses"), []);

  const outline = "# Unidad práctica\nPreparar el material\nCrear una muestra\n\n# Cierre\nRevisar el proyecto";
  const request = { provider: "domestika", url, title, text: outline, save: false };
  for (const text of ["Sin unidad", "# Unidad\n# Otra\nUna lección", "# Unidad\nLección\nlección", "# Unidad\nhttps://example.org/?token=fixture"]) {
    await assert.rejects(rpc("course_manual_outline", { ...request, text }), /line|lesson/i);
  }
  await page.getByRole("button", { name: "Plataformas", exact: true }).click();
  await page.locator("[data-action='open-courses'][data-id='domestika']").click();
  await page.getByLabel("Enlace del curso").fill(url);
  await page.getByLabel("Nombre del curso").fill(title);
  await page.locator("#course-manual-outline > summary").press("Enter");
  const field = page.getByLabel("Unidades y lecciones");
  await field.fill(outline);
  await page.getByRole("button", { name: "Revisar temario" }).press("Enter");
  const heading = page.getByRole("heading", { name: "Revisar temario" });
  await heading.waitFor();
  assert.equal(await heading.evaluate(node => node === document.activeElement), true);
  assert.deepEqual(await page.locator(".course-outline-review ol li").allTextContents(), ["Preparar el material", "Crear una muestra", "Revisar el proyecto"]);
  assert.deepEqual(await rpc("list_courses"), [], "Reviewing an outline does not save it");
  await page.getByRole("button", { name: "Editar temario" }).press("Enter");
  assert.equal(await field.inputValue(), outline);
  assert.equal(await field.evaluate(node => node === document.activeElement), true);
  await page.getByRole("button", { name: "Revisar temario" }).click();
  for (const [width, height] of [[900, 620], [1180, 780]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await page.locator(".course-outline-review").evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    if (process.env.NABNAB_NATIVE_COURSE_ARTIFACTS) await page.screenshot({ path: path.join(artifacts, `outline-${width}.png`), animations: "disabled" });
  }
  await page.getByRole("button", { name: "Guardar temario" }).press("Enter");
  await page.getByRole("heading", { name: title }).waitFor();
  const [course] = await rpc("list_courses");
  assert.equal(course.sections[0].title, "Unidad práctica");
  const closingUnit = page.locator(".course-tree details").filter({ hasText: "Cierre" });
  assert.equal(await closingUnit.evaluate(element => element.open), false);
  const search = page.getByRole("searchbox", { name: "Buscar lección o recurso", exact: true });
  await search.fill("revisár");
  await page.getByRole("button", { name: "Revisar el proyecto", exact: true }).press("Enter");
  const resultTitle = page.getByRole("heading", { name: "Revisar el proyecto", exact: true });
  assert.ok(await resultTitle.evaluate(element => element === document.activeElement));
  await search.fill("");
  assert.equal(await closingUnit.evaluate(element => element.open), true);
  const lessonId = course.sections[0].lessons[0].id;
  await assert.rejects(rpc("preview_course_materials", {
    provider: "domestika", url, title, folder: false, lessonId, playback: true, droppedPaths: [fixture],
  }), /valid destination/);
  await rpc("course_learned", { courseId: course.id, lessonId, learned: true });
  await rpc("save_course_note", { courseId: course.id, lessonId, text: "Nota de estudio", time: null, noteId: null, remove: false });
  const updated = await rpc("course_manual_outline", { ...request, save: true, text: "# Unidad práctica\nCrear una muestra\nPreparar el material" });
  assert.equal(updated.id, course.id);
  assert.equal(updated.createdAt, course.createdAt);
  assert.deepEqual(updated.sections.map(section => section.title), ["Unidad práctica", "Cierre"]);
  assert.equal(updated.sections[0].lessons[1].id, lessonId);
  assert.equal(updated.sections[0].lessons[1].learned, true);
  assert.equal(updated.sections[0].lessons[1].notes[0].text, "Nota de estudio");

  const skillshare = { provider: "skillshare", url: "https://www.skillshare.com/en/classes/Fixture/123", title: "Clase de prueba", text: "# Práctica\nPreparar" };
  const skillCourse = await rpc("course_manual_outline", { ...skillshare, save: true });
  const skillLesson = skillCourse.sections[0].lessons[0].id;
  await rpc("add_skillshare_resources", { url: skillshare.url, title: skillshare.title, links: ["https://static.skillshare.com/uploads/attachment/1234/abcdef12/Guide.pdf"], lessonId: skillLesson });
  const skillUpdated = await rpc("course_manual_outline", { ...skillshare, text: "# Práctica\nRevisar\nPreparar", save: true });
  assert.equal(skillUpdated.sections[0].lessons[1].assets[0].title, "Guide.pdf");
  const rocket = await rpc("course_manual_outline", { ...request, provider: "rocketseat", url: "https://app.rocketseat.com.br/classroom/git?token=fixture", save: true });
  assert.equal(rocket.url, "https://app.rocketseat.com.br/classroom/git");
  const snapshot = await rpc("get_snapshot");
  assert.deepEqual(snapshot.library, []);
  assert.deepEqual(snapshot.jobs, []);
  const savedData = await readFile(path.join(data, "state-v1.json"), "utf8");
  assert.ok(!savedData.includes("token=fixture"));
  const persisted = JSON.parse(savedData);
  assert.equal(persisted.courses.length, 3);
  console.log("Native course outlines passed for Domestika, Skillshare and Rocketseat. Search, keyboard focus, review, order, notes, progress and safe file selection were verified.");
} finally {
  try {
    await page?.evaluate(async () => {
      const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js");
      await getCurrentWindow().close();
    });
  } catch {}
  if (child.exitCode === null && !await Promise.race([exited.then(() => true), delay(3000, false)])) {
    child.kill();
    await exited;
  }
  await browser?.close().catch(() => {});
  assert.equal(path.relative(data, await realpath(data)), "", "Refusing to remove redirected test data.");
  await rm(data, { recursive: true, force: true, maxRetries: 8, retryDelay: 150 });
}
