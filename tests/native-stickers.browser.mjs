import assert from "node:assert/strict";
import { openTool } from "./tool-navigation.mjs";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { createRequire } from "node:module";

const root = path.resolve(import.meta.dirname, "..");
const base = path.join(root, "artifacts/stickers");
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "native-run-"));
const output = path.join(data, "output");
await mkdir(output);
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const port = Number(process.env.NABNAB_STICKER_DEBUG_PORT || 9248);
const endpoint = `http://127.0.0.1:${port}`;
let child, browser, page;
const errors = [];

async function wait(check, timeout = 60000) {
  const until = Date.now() + timeout;
  while (Date.now() < until) {
    const result = await check();
    if (result) return result;
    await new Promise(resolve => setTimeout(resolve, 150));
  }
  throw Error("Timed out waiting for the native sticker workflow.");
}

function rpc(command, args = {}) {
  return page.evaluate(async ({ command, args }) => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    return invoke(command, args);
  }, { command, args });
}

try {
  child = spawn(path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
    cwd: root, windowsHide: true, stdio: "ignore",
    env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
  });
  await wait(async () => {
    if (child.exitCode !== null) throw Error(`Native app exited ${child.exitCode}`);
    try { return (await fetch(`${endpoint}/json/version`)).ok; } catch { return false; }
  });
  browser = await chromium.connectOverCDP(endpoint);
  page = await wait(() => browser.contexts()[0]?.pages().find(tab => /:1420/.test(tab.url())));
  page.on("pageerror", error => errors.push(error.message));
  const initial = await wait(async () => { try { return await rpc("get_snapshot"); } catch { return null; } });
  await rpc("save_settings", { settings: { ...initial.settings, folder: output, language: "es", reducedMotion: true, trayEnabled: false, closeToTray: false } });

  const fixture = Buffer.from(await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    canvas.width = 480;
    canvas.height = 400;
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#8bd6c5";
    ctx.beginPath();
    ctx.moveTo(236, 25);
    ctx.bezierCurveTo(256, 25, 273, 139, 307, 162);
    ctx.bezierCurveTo(331, 178, 411, 181, 410, 200);
    ctx.bezierCurveTo(409, 219, 333, 226, 306, 246);
    ctx.bezierCurveTo(268, 271, 255, 375, 236, 375);
    ctx.bezierCurveTo(216, 375, 202, 272, 167, 245);
    ctx.bezierCurveTo(144, 227, 62, 218, 62, 200);
    ctx.bezierCurveTo(62, 181, 144, 175, 167, 156);
    ctx.bezierCurveTo(201, 130, 217, 25, 236, 25);
    ctx.fill();
    ctx.fillStyle = "#1e4e4c";
    for (const x of [217, 256]) {
      ctx.beginPath(); ctx.ellipse(x, 187, 6, 11, 0, 0, Math.PI * 2); ctx.fill();
    }
    ctx.lineWidth = 6;
    ctx.strokeStyle = ctx.fillStyle;
    ctx.beginPath(); ctx.arc(236, 206, 13, 0.2, Math.PI - 0.2); ctx.stroke();
    return canvas.toDataURL("image/png").split(",")[1];
  }), "base64");
  const source = path.join(data, "Estrella de prueba.png");
  await writeFile(source, fixture);
  const imported = await rpc("import_paths", { paths: [source] });
  const libraryId = imported.library.find(item => item.name === "Estrella de prueba.png").id;
  const options = { profile: "whatsappImage", crop: null, outline: 8, startMs: 0, durationMs: null };
  const sample = await rpc("sticker_preview", { libraryId, options, requestId: 96401 });
  let rejected;
  try { await rpc("queue_sticker", { libraryId, options: { ...options, outline: 4 }, previewId: sample.previewId, clientRequest: "sticker-mismatched-test" }); }
  catch (error) { rejected = String(error); }
  assert.match(rejected ?? "", /preview/i);

  await page.locator('[data-page="tools"]').click();
  await openTool(page, "stickers");
  const module = page.locator(".sticker-tool");
  const submit = module.getByRole("button", { name: "Exportar en la cola" });
  assert.equal(await submit.isDisabled(), true);
  await module.getByRole("button", { name: "Elegir archivo" }).click();
  await module.getByRole("searchbox").fill("Estrella");
  await module.getByRole("radio", { name: "Estrella de prueba.png" }).click();
  assert.equal(await module.locator(".sticker-source > span").textContent(), "Estrella de prueba.png");
  await page.locator("#sticker-profile").selectOption("whatsappImage");
  await page.locator("#sticker-outline").selectOption("8");
  await module.getByRole("button", { name: "Generar muestra" }).click();
  const image = module.locator(".sticker-preview img");
  await image.waitFor();
  assert.equal(await image.evaluate(async element => { await element.decode(); return element.naturalWidth; }), 512);
  const reviewed = Buffer.from((await image.getAttribute("src")).split(",")[1], "base64");
  assert.ok(reviewed.length > 0 && reviewed.length <= 100000);
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    const fits = await module.evaluate(element => element.scrollWidth <= element.clientWidth + 1);
    assert.ok(fits, `Sticker form overflows at ${width}px`);
    await module.locator(".sticker-review").scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(base, `native-sticker-${width}.png`), animations: "disabled" });
    if (width === 1180) await module.locator(".sticker-workspace").screenshot({ path: path.join(base, "preview.png"), animations: "disabled" });
  }
  await submit.click();
  const completed = await wait(async () => {
    const snapshot = await rpc("get_snapshot");
    const job = snapshot.jobs.find(item => item.source.kind === "sticker");
    if (job?.state === "error") throw Error(JSON.stringify(job.error));
    return job?.state === "completed" && { job, output: snapshot.library.find(item => item.path === job.outputs[0]) };
  });
  assert.equal(completed.output.parentId, libraryId);
  assert.match(completed.output.name, /\.webp$/i);
  assert.deepEqual(await readFile(completed.output.path), reviewed);
  assert.deepEqual(await readFile(source), fixture);
  const checked = await rpc("sticker_preview", { libraryId, options, requestId: 96402 });
  const changed = Buffer.concat([fixture, Buffer.from("changed fixture")]);
  await writeFile(source, changed);
  const [stale] = await rpc("queue_sticker", { libraryId, options, previewId: checked.previewId, clientRequest: "sticker-changed-source-test" });
  const failed = await wait(async () => {
    const snapshot = await rpc("get_snapshot");
    const job = snapshot.jobs.find(item => item.id === stale.id);
    return ["error", "completed"].includes(job?.state) && job;
  });
  assert.equal(failed.state, "error");
  assert.match(failed.error.technical, /changed after preview/i);
  assert.deepEqual(failed.outputs, []);
  assert.deepEqual(await readFile(source), changed);
  assert.deepEqual(await readFile(completed.output.path), reviewed);
  assert.deepEqual(errors, []);
  console.log("Verified real Windows preview, changed options and source checks, queued output and original bytes.");
} finally {
  try { await page?.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close(); }); } catch {}
  if (child?.exitCode === null) {
    await Promise.race([new Promise(resolve => child.once("exit", resolve)), new Promise(resolve => setTimeout(resolve, 4000))]);
    if (child.exitCode === null) child.kill();
  }
  try { await browser?.close(); } catch {}
}
