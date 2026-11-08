import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, stat, unlink, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(import.meta.dirname, "..");
const base = path.join(root, "artifacts/device-share");
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "native-run-"));
const output = path.join(data, "output");
await mkdir(output);
execFileSync("rustc", ["--edition=2021", path.join(root, "tests/kdeconnect-cli-fixture.rs"), "-o", path.join(data, "kdeconnect-cli.exe")], { windowsHide: true });
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const endpoint = `http://127.0.0.1:${process.env.NABNAB_DEVICE_SHARE_DEBUG_PORT || 9262}`;
const file = path.join(data, "Informe ñ.txt");
const second = path.join(data, "Ficha 02.txt");
const logFile = path.join(data, "cli-arguments.jsonl");
await writeFile(file, "Contenido local.\n");
await writeFile(second, "Otro contenido.\n");

let child, browser, page, log = "";
async function wait(check, timeout = 35_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const value = await check();
    if (value) return value;
    await new Promise(resolve => setTimeout(resolve, 120));
  }
  throw Error("Timed out waiting for native device sharing.");
}

function rpc(command, args = {}) {
  return page.evaluate(async ({ command, args }) => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    return invoke(command, args);
  }, { command, args });
}

async function calls() {
  return (await readFile(logFile, "utf8").catch(error => { if (error.code === "ENOENT") return ""; throw error; }))
    .trim().split("\n").filter(Boolean).map(line => JSON.parse(line));
}

const modal = () => page.locator(".device-share[open]");
const inShare = id => modal().locator(`#share-${id}`);
const device = "paired-device-123";
async function choose() {
  await modal().waitFor();
  await inShare("device").locator(`option[value="${device}"]`).waitFor({ state: "attached" });
  await inShare("device").selectOption(device);
}
async function closeShare() { await inShare("close").click(); await modal().waitFor({ state: "hidden" }); }

try {
  child = spawn(path.join(root, "src-tauri/target/debug/nabnab.exe"), [], { cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, Path: `${data}${path.delimiter}${process.env.Path ?? process.env.PATH ?? ""}`, NABNAB_TEST_DATA_DIR: data,
      WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${new URL(endpoint).port}` } });
  for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => { log = (log + chunk).slice(-5000); });
  browser = await wait(async () => {
    if (child.exitCode !== null) throw Error(`Native app exited ${child.exitCode}.`);
    try { if ((await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(400) })).ok) return await chromium.connectOverCDP(endpoint, { timeout: 2000 }); }
    catch { /* WebView2 is still starting. */ }
    return null;
  });
  page = await wait(() => browser.contexts()[0]?.pages().find(tab => /:1420/.test(tab.url())));
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  const initial = await wait(async () => { try { return await rpc("get_snapshot"); } catch { return null; } });
  await rpc("save_settings", { settings: { ...initial.settings, folder: output, language: "es", reducedMotion: true, trayEnabled: false, closeToTray: false } });
  const imported = await rpc("import_paths", { paths: [file, second] });
  const ids = imported.library.filter(item => [path.basename(file), path.basename(second)].includes(item.name)).map(item => item.id);
  assert.equal(ids.length, 2);
  for (const id of ids) await rpc("update_library", { id, collections: ["Lecturas"], tags: [] });
  await page.reload();
  await page.locator('[data-page="library"]').click();
  await page.locator(`[data-action="inspect-media"][data-id="${ids[0]}"]`).click();
  await page.locator(`[data-action="share-file"][data-id="${ids[0]}"]`).click();
  await choose();
  assert.equal(await inShare("device").locator("option").count(), 2);
  await inShare("review").click();
  await inShare("send").waitFor();
  assert.match(await modal().innerText(), /Teléfono de prueba ñ/);
  assert.deepEqual(await calls(), []);
  const selected = imported.library.find(item => item.id === ids[0]);
  await writeFile(selected.path, "Un cambio de tamaño después de revisar.\n");
  await inShare("send").click();
  await modal().getByRole("alert").waitFor();
  assert.match(await modal().getByRole("alert").textContent(), /changed/i);
  assert.deepEqual(await calls(), []);
  await inShare("review").click();
  await inShare("send").waitFor();
  await modal().screenshot({ path: path.join(base, "review-1180.png"), animations: "disabled" });
  await page.setViewportSize({ width: 900, height: 620 });
  assert.ok(await modal().evaluate(element => element.scrollWidth <= element.clientWidth + 1));
  await modal().screenshot({ path: path.join(base, "review-900.png"), animations: "disabled" });
  await inShare("send").click();
  await modal().getByText("Entregado a KDE Connect", { exact: true }).waitFor();
  const sent = await calls();
  assert.equal(sent.length, 1);
  assert.equal(sent[0][0], `--device=${device}`);
  assert.match(sent[0][1], /^--share=file:\/\/\//);
  assert.match(decodeURIComponent(sent[0][1]), new RegExp(selected.name.replaceAll(".", "\\.") + "$"));
  await closeShare();
  await page.locator('[data-action="close-inspector"]').click();

  await page.locator('[data-action="share-open"]').click();
  await choose();
  await inShare("url").fill("https://example.org/download?signature=secret");
  await inShare("review").click();
  await modal().getByRole("alert").waitFor();
  assert.equal((await calls()).length, 1);
  await inShare("url").fill("https://www.youtube.com/watch?v=abcdefghijk");
  await inShare("review").click();
  await inShare("send").click();
  await modal().getByText("Entregado a KDE Connect", { exact: true }).waitFor();
  assert.equal((await calls()).at(-1)[1], "--share=https://www.youtube.com/watch?v=abcdefghijk");
  await closeShare();

  const text = "--send-clipboard\nTexto ñ explícito";
  await assert.rejects(rpc("device_share_prepare", { deviceId: device, selection: { kind: "text", text: "URL: https://example.org/file?token=secret" } }), /public link/i);
  const preview = await rpc("device_share_prepare", { deviceId: device, selection: { kind: "text", text } });
  assert.equal(preview.total, Buffer.byteLength(text));
  await writeFile(path.join(data, "cli-warning"), "1");
  assert.equal(await rpc("device_share_send", { token: preview.token }), "unconfirmed");
  await assert.rejects(rpc("device_share_send", { token: preview.token }), /Review the selection/);
  const afterText = await calls();
  assert.equal(afterText.length, 3);
  assert.deepEqual(afterText.at(-1), [`--device=${device}`, `--share-text=${text}`]);
  await unlink(path.join(data, "cli-warning"));

  const stale = await rpc("device_share_prepare", { deviceId: device, selection: { kind: "files", ids } });
  await writeFile(path.join(data, "unreachable"), "1");
  await assert.rejects(rpc("device_share_send", { token: stale.token }), /no longer reachable/);
  assert.equal((await calls()).length, 3);
  assert.deepEqual((await rpc("device_share_status")).devices, []);
  await unlink(path.join(data, "unreachable"));

  await page.locator('[data-action="library-section"][data-section="collections"]').click();
  await page.locator('[data-action="share-collection"][data-collection="Lecturas"]').click();
  await modal().getByText("Lecturas", { exact: true }).waitFor();
  await choose();
  await inShare("review").click();
  await inShare("send").click();
  await modal().getByText("Entregado a KDE Connect", { exact: true }).waitFor();
  const multi = (await calls()).at(-1);
  assert.equal(multi.length, 4);
  assert.equal(multi[2], "--");
  assert.match(multi[3], /^file:\/\/\//);
  await closeShare();
  await page.locator('[data-action="share-collection"][data-collection="Lecturas"]').click();
  await inShare("zip").waitFor();
  await wait(() => inShare("zip").isEnabled());
  await inShare("zip").click();
  const portable = page.locator("#portable-export-form");
  await portable.waitFor();
  assert.equal(await portable.locator("[data-portable-item]:checked").count(), 2);
  for (const option of ["metadata", "notes", "ocr"]) assert.equal(await portable.locator(`#portable-${option}`).isChecked(), false);
  await portable.getByRole("button", { name: "Revisar tamaño y duplicados" }).click();
  await page.locator('[data-action="portable-export-queue"]').click();
  const zip = await wait(async () => {
    const job = (await rpc("get_snapshot")).jobs.find(job => job.source.kind === "portableExport");
    if (job?.state === "error") throw Error(JSON.stringify(job.error));
    return job?.state === "completed" && job;
  });
  const result = page.getByRole("region", { name: "Último ZIP de esta sesión" });
  const zipName = path.basename(zip.outputs[0]);
  const zipInfo = await stat(zip.outputs[0]);
  assert.ok(zipInfo.size > 0);
  const reviewZip = result.getByRole("button", { name: "Revisar envío del ZIP" });
  await reviewZip.waitFor();
  assert.match(await result.innerText(), /ZIP disponible para revisar/);
  await page.locator("#main").evaluate(main => { main.scrollTop = 0; });
  await page.screenshot({ path: path.join(base, "portable-share-900.png"), animations: "disabled" });
  const beforeZip = await calls();
  await reviewZip.click();
  await choose();
  await inShare("review").click();
  await inShare("send").waitFor();
  const review = modal().locator(".device-share-review");
  assert.deepEqual(await review.locator("li > span").allTextContents(), [zipName]);
  const sizeLabel = zipInfo.size < 1024 ? `${zipInfo.size} B` : `${(zipInfo.size / 1024).toFixed(0)} KB`;
  assert.equal(await review.locator("li > small").innerText(), sizeLabel);
  assert.deepEqual(await calls(), beforeZip);
  await inShare("send").click();
  await modal().getByText("Entregado a KDE Connect", { exact: true }).waitFor();
  const final = await calls();
  assert.equal(final.length, beforeZip.length + 1);
  assert.equal(final.at(-1)[0], `--device=${device}`);
  assert.ok(decodeURIComponent(final.at(-1)[1]).endsWith(`/${zipName}`));
  await closeShare();

  const image = path.join(data, "Estrella ñ de prueba.png");
  execFileSync(path.join(root, "src-tauri/engines/ffmpeg.exe"), [
    "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=s=160x120", "-frames:v", "1", image,
  ], { windowsHide: true });
  const source = (await rpc("import_paths", { paths: [image] })).library.find(item => item.name === path.basename(image));
  assert.ok(source);
  await page.locator("#local-tool-stickers").click();
  await page.getByRole("button", { name: "Elegir archivo", exact: true }).click();
  await page.getByRole("radio", { name: source.name, exact: true }).click();
  await page.getByRole("button", { name: "Generar muestra", exact: true }).click();
  await page.getByRole("button", { name: "Exportar en la cola", exact: true }).click();
  const sticker = await wait(async () => {
    const snapshot = await rpc("get_snapshot");
    const job = snapshot.jobs.find(job => job.source.kind === "sticker");
    if (job?.state === "error") throw Error(JSON.stringify(job.error));
    return job?.state === "completed" && snapshot.library.find(item => item.path === job.outputs[0]);
  });
  const exports = page.getByRole("region", { name: "Última exportación", exact: true });
  await exports.waitFor({ state: "attached", timeout: 5000 });
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    await page.locator("#main").evaluate(main => { main.scrollTop = main.scrollHeight; });
    await page.screenshot({ path: path.join(data, `sticker-export-${width}.png`), animations: "disabled" });
  }
  assert.ok(await exports.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
  const sendSticker = exports.getByRole("button", { name: `Enviar a dispositivo · ${sticker.name}`, exact: true });
  await sendSticker.press("Enter");
  await choose();
  await inShare("review").click();
  await inShare("send").waitFor();
  assert.deepEqual(await modal().locator(".device-share-review li > span").allTextContents(), [sticker.name]);
  assert.deepEqual(await calls(), final);
  await inShare("send").click();
  await modal().getByText("Entregado a KDE Connect", { exact: true }).waitFor();
  const stickerCall = (await calls()).at(-1);
  assert.equal(stickerCall.length, 2);
  assert.equal(path.toNamespacedPath(fileURLToPath(stickerCall[1].replace(/^--share=/, ""))), path.toNamespacedPath(sticker.path));
  await closeShare();
  await page.reload();
  await page.locator('[data-page="tools"]').click();
  await page.locator("#local-tool-stickers").click();
  await sendSticker.waitFor();
  await exports.scrollIntoViewIfNeeded();
  const box = await exports.boundingBox();
  await page.screenshot({ path: path.join(data, "sticker-export.png"), animations: "disabled",
    clip: { x: box.x - 10, y: box.y, width: box.width + 20, height: box.height } });
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ nativeCliFixture: true, unicodeFile: true, staleFileRejected: true, signedLinkRejected: true,
    publicLink: true, textOneArgument: true, ambiguousHandoff: true, replayRejected: true, offlineDeviceRejected: true, collection: true, realPortableZip: true, shareFromExportResult: true, shareFromRealSticker: true, outputSurvivesReload: true, output: data }));
} catch (error) {
  console.error(log);
  await page?.screenshot({ path: path.join(base, "failure.png"), animations: "disabled" }).catch(() => {});
  throw error;
} finally {
  try { await page?.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close(); }); } catch {}
  if (child?.exitCode === null) {
    const done = new Promise(resolve => child.once("exit", resolve));
    await Promise.race([done, new Promise(resolve => setTimeout(resolve, 4000))]);
    if (child.exitCode === null) { child.kill(); await done; }
  }
  await browser?.close().catch(() => {});
}
