import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/native-course-materials");
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
  console.log("The native command rejected a local path without a preceding Windows drop.");
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
