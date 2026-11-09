import assert from "node:assert/strict";
import { openTool } from "./tool-navigation.mjs";
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFile, mkdir, mkdtemp, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { createRequire } from "node:module";

const root = path.resolve(import.meta.dirname, "..");
const base = path.resolve(process.env.NABNAB_STICKER_ARTIFACTS ?? path.join(root, "artifacts/stickers"));
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "native-run-"));
const output = path.join(data, "output");
await mkdir(output);
const original = path.join(data, "Original del recorte.png");
const cutout = path.join(output, "Recorte transparente.png");
const solid = path.join(output, "Fondo sólido.png");
const missing = path.join(output, "Recorte ausente.png");
const ffmpeg = path.join(root, "src-tauri/engines/ffmpeg.exe");
for (const [filter, file] of [
  ["color=c=0x344056:s=480x400", original],
  ["color=c=black@0:s=480x400,format=rgba,drawbox=x=105:y=85:w=270:h=230:color=0x8bd6c5@1:t=fill:replace=1", cutout],
]) execFileSync(ffmpeg, ["-nostdin", "-v", "error", "-f", "lavfi", "-i", filter, "-frames:v", "1", file], { windowsHide: true });
await copyFile(original, solid);
const originalBytes = await readFile(original), cutoutBytes = await readFile(cutout);
const item = async (id, file, parentId = null) => ({ id, parentId, name: path.basename(file), path: file, kind: "image",
  size: (await stat(file).catch(() => null))?.size ?? 0, width: 480, height: 400, addedAt: 1, collections: [], tags: [] });
const inputHash = createHash("sha256").update(originalBytes).digest("hex");
const published = (id, file, fill) => ({ id, runId: 1, clientRequest: id, title: path.basename(file), createdAt: 1, updatedAt: 1,
  state: "completed", source: { kind: "background", selection: { libraryId: "cutout-original", inputHash, options: { fill, format: "png", edge: 0, softness: 1 } } },
  folder: output, provider: "Local", outputs: [file], attempt: 1, groupId: "cutout-fixture-batch" });
await writeFile(path.join(data, "state-v1.json"), JSON.stringify({ schemaVersion: 2, settings: { folder: output },
  library: [await item("cutout-original", original), await item("cutout-transparent", cutout, "cutout-original"), await item("cutout-solid", solid, "cutout-original"), await item("cutout-missing", missing, "cutout-original")],
  jobs: [published("cutout-job", cutout, { kind: "transparent" }), published("cutout-solid-job", solid, { kind: "solid", rgb: [52, 64, 86] }), published("cutout-missing-job", missing, { kind: "transparent" })],
}));
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
  child = spawn(process.env.NABNAB_STICKER_TEST_EXE ?? path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
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
  const clip = path.join(data, "Clip de prueba.mp4");
  execFileSync(path.join(root, "src-tauri/engines/ffmpeg.exe"), ["-v", "error", "-f", "lavfi", "-i",
    "testsrc2=size=320x180:rate=12:duration=2", "-c:v", "libopenh264", clip], { windowsHide: true });
  const clipBytes = await readFile(clip);
  const imported = await rpc("import_paths", { paths: [source, clip] });
  const libraryId = imported.library.find(item => item.name === "Estrella de prueba.png").id;
  const clipId = imported.library.find(item => item.name === "Clip de prueba.mp4").id;
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

  await module.getByRole("button", { name: "Elegir archivo" }).click();
  await module.getByRole("searchbox").fill("Clip");
  await module.getByRole("radio", { name: "Clip de prueba.mp4" }).click();
  await page.locator("#sticker-profile").selectOption("whatsappAnimated");
  await page.locator("#sticker-length").fill("1");
  await page.locator("#sticker-length").press("Tab");
  await module.getByRole("button", { name: "Generar muestra" }).click();
  const play = module.getByRole("button", { name: "Reproducir animación", exact: true });
  await play.waitFor();
  await module.locator(".sticker-review").scrollIntoViewIfNeeded();
  await page.screenshot({ path: path.join(base, "animated-sticker-stopped-900.png"), animations: "disabled" });
  await image.waitFor({ timeout: 2500 });
  const still = await image.getAttribute("src");
  assert.match(still, /^data:image\/png;base64,/);
  assert.deepEqual(await image.evaluate(async element => {
    await element.decode();
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 512;
    const ctx = canvas.getContext("2d");
    ctx.drawImage(element, 0, 0);
    return [element.naturalWidth, ctx.getImageData(0, 0, 1, 1).data[3], ctx.getImageData(256, 256, 1, 1).data[3]];
  }), [512, 0, 255]);
  const surface = page.getByLabel("Inspeccionar sobre", { exact: true });
  await surface.focus();
  await surface.selectOption("light");
  assert.equal(await module.locator(".sticker-preview").getAttribute("data-surface"), "light");
  assert.equal(await image.getAttribute("src"), still);
  assert.equal(await surface.evaluate(control => control === document.activeElement), true);
  await play.press("Enter");
  const motion = await image.getAttribute("src");
  assert.match(motion, /^data:image\/webp;base64,/);
  await image.evaluate(element => { element.dataset.playbackMarker = "preserved"; });
  await surface.selectOption("dark");
  assert.equal(await image.getAttribute("data-playback-marker"), "preserved");
  assert.equal(await image.getAttribute("src"), motion);
  await module.getByRole("button", { name: "Detener animación", exact: true }).press("Enter");
  assert.equal(await image.getAttribute("src"), still);
  assert.equal(await module.locator(".sticker-preview").getAttribute("data-surface"), "dark");
  for (const [width,height] of [[1180,780],[900,620]]) {
    await page.setViewportSize({width,height});
    await module.locator(".sticker-review").evaluate(panel => panel.scrollIntoView({block:"start"}));
    assert.ok(await module.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    await page.screenshot({path:path.join(base,`animated-sticker-review-${width}.png`),animations:"disabled"});
  }
  await submit.click();
  const animation = await wait(async () => {
    const snapshot = await rpc("get_snapshot");
    const job = snapshot.jobs.find(item => item.source.kind === "sticker" && item.id !== completed.job.id && item.id !== stale.id);
    if (job?.state === "error") throw Error(JSON.stringify(job.error));
    return job?.state === "completed" && snapshot.library.find(item => item.path === job.outputs[0]);
  });
  assert.equal(animation.parentId, clipId);
  const animatedBytes = Buffer.from(motion.split(",")[1], "base64");
  assert.ok(animatedBytes.length <= 500000);
  assert.deepEqual(await readFile(animation.path), animatedBytes);
  assert.deepEqual(await readFile(clip), clipBytes);

  await module.getByRole("button", { name: "Generar muestra" }).click();
  await play.waitFor();
  await openTool(page, "background");
  const exports = page.getByRole("region", { name: "Última exportación", exact: true });
  const create = exports.getByRole("button", { name: "Crear sticker · Recorte transparente.png" });
  await create.waitFor();
  assert.equal(await exports.getByRole("button", { name: /^Crear sticker/ }).count(), 1);
  assert.equal(await exports.getByText("Fondo sólido.png", { exact: true }).count(), 1);
  assert.equal(await exports.getByText("Recorte ausente.png", { exact: true }).count(), 0);
  const beforeCutout = (await rpc("get_snapshot")).jobs.length;
  for (const [width, height] of [[900, 620], [1180, 780]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await exports.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    await exports.screenshot({ path: path.join(base, `cutout-handoff-${width}.png`), animations: "disabled" });
  }
  await create.press("Enter");
  assert.equal(await module.locator(".sticker-source > span").textContent(), path.basename(cutout));
  assert.equal(await module.getByRole("heading", { name: "Stickers" }).evaluate(heading => heading === document.activeElement), true);
  assert.equal(await page.locator("#sticker-profile").inputValue(), "whatsappImage");
  assert.equal(await image.count(), 0, "The prior animation preview must be invalidated");
  assert.equal(await submit.isDisabled(), true);
  assert.equal((await rpc("get_snapshot")).jobs.length, beforeCutout);

  await page.getByRole("button", { name: "Descargas", exact: true }).click();
  await page.locator('[data-filter="completed"]').click();
  await page.locator('[data-action="inspect-job"][data-id="cutout-job"]').click();
  await page.getByRole("button", { name: "Crear sticker · Recorte transparente.png" }).click();
  assert.equal(await module.locator(".sticker-source > span").textContent(), path.basename(cutout));
  await module.getByRole("button", { name: "Generar muestra" }).click();
  await image.waitFor();
  const previewBytes = Buffer.from((await image.getAttribute("src")).split(",")[1], "base64");
  assert.deepEqual(await image.evaluate(async element => {
    await element.decode();
    const canvas = document.createElement("canvas"); canvas.width = canvas.height = 512;
    const ctx = canvas.getContext("2d"); ctx.drawImage(element, 0, 0);
    return [ctx.getImageData(80, 256, 1, 1).data[3], ctx.getImageData(256, 256, 1, 1).data[3]];
  }), [0, 255]);
  await submit.click();
  const sticker = await wait(async () => {
    const snapshot = await rpc("get_snapshot");
    const job = snapshot.jobs.find(job => job.source.kind === "sticker" && job.source.selection.libraryId === "cutout-transparent");
    if (job?.state === "error") throw Error(JSON.stringify(job.error));
    return job?.state === "completed" && snapshot.library.find(item => item.path === job.outputs[0]);
  });
  assert.equal(sticker.parentId, "cutout-transparent");
  assert.deepEqual(await readFile(sticker.path), previewBytes);
  assert.deepEqual(await readFile(cutout), cutoutBytes);
  assert.deepEqual(await readFile(original), originalBytes);
  assert.deepEqual(errors, []);
  console.log("Verified still and animated WebP exports and the cutout handoff from Tools and Downloads with preserved transparency and source files.");
} finally {
  try { await page?.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close(); }); } catch {}
  if (child?.exitCode === null) {
    await Promise.race([new Promise(resolve => child.once("exit", resolve)), new Promise(resolve => setTimeout(resolve, 4000))]);
    if (child.exitCode === null) child.kill();
  }
  try { await browser?.close(); } catch {}
}
