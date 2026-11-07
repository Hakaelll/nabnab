import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, readdir, writeFile, copyFile } from "node:fs/promises";
import path from "node:path";
import { createRequire } from "node:module";

const root = path.resolve(import.meta.dirname, "..");
const base = path.join(root, "artifacts/background");
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "native-run-"));
const output = path.join(data, "output");
const modelPath = path.join(data, "components/u2netp-309c8469.onnx");
const fixtureModel = process.env.NABNAB_TEST_BACKGROUND_MODEL;
await mkdir(output);
if (fixtureModel) {
  await mkdir(path.dirname(modelPath));
  await copyFile(fixtureModel, modelPath);
  await writeFile(path.join(data, "components/background.enabled"), "1");
}
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const endpoint = `http://127.0.0.1:${process.env.NABNAB_BACKGROUND_DEBUG_PORT || 9250}`;
let child, browser, page, log = "";
const errors = [];

async function wait(check, timeout = 60000) {
  const until = Date.now() + timeout;
  while (Date.now() < until) {
    const result = await check();
    if (result) return result;
    await new Promise(resolve => setTimeout(resolve, 150));
  }
  throw Error("Timed out waiting for the native background workflow.");
}

function rpc(command, args = {}) {
  return page.evaluate(async ({ command, args }) => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    return invoke(command, args);
  }, { command, args });
}

function pixels(url, channel) {
  return page.evaluate(async ({ url, channel }) => {
    const image = new Image(); image.src = url; await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = image.naturalWidth; canvas.height = image.naturalHeight;
    const context = canvas.getContext("2d"); context.drawImage(image, 0, 0);
    const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let empty = 0, full = 0, sum = 0;
    for (let offset = channel; offset < data.length; offset += 4) {
      if (data[offset] < 20) empty++;
      if (data[offset] > 235) full++;
      sum += data[offset];
    }
    return { width: canvas.width, height: canvas.height, empty, full, sum, corner: Array.from(data.slice(0, 4)) };
  }, { url, channel });
}

async function outputFor(index) {
  return wait(async () => {
    const snapshot = await rpc("get_snapshot");
    const job = snapshot.jobs.filter(item => item.source.kind === "background")[index];
    if (job?.state === "error") throw Error(JSON.stringify(job.error));
    return job?.state === "completed" && snapshot.library.find(item => item.path === job.outputs[0]);
  });
}

try {
  child = spawn(path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
    cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${new URL(endpoint).port}` },
  });
  for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => { log = (log + chunk).slice(-6000); });
  await wait(async () => {
    if (child.exitCode !== null) throw Error(`Native app exited ${child.exitCode}`);
    try { return (await fetch(`${endpoint}/json/version`)).ok; } catch { return false; }
  });
  browser = await chromium.connectOverCDP(endpoint);
  page = await wait(() => browser.contexts()[0]?.pages().find(tab => /:1420/.test(tab.url())));
  page.on("pageerror", error => errors.push(error.message));
  const initial = await wait(async () => { try { return await rpc("get_snapshot"); } catch { return null; } });
  await rpc("save_settings", { settings: { ...initial.settings, folder: output, language: "es", reducedMotion: true, trayEnabled: false, closeToTray: false } });
  const status = async () => (await rpc("optional_components")).find(item => item.id === "background");
  assert.equal((await status()).installed, Boolean(fixtureModel));
  await page.locator('[data-page="tools"]').click();
  await page.locator("#local-tool-background").click();
  if (!fixtureModel) await page.getByRole("button", { name: "Ver componentes" }).click();
  else {
    await page.locator('[data-page="settings"]').click();
    await page.locator('[data-action="settings-section"][data-section="engines"]').click();
  }
  const component = page.locator(".component-row").filter({ hasText: "U²-Net small · Windows ML" });
  if (!fixtureModel) await component.getByRole("button", { name: "Instalar", exact: true }).click();
  await component.getByRole("button", { name: "Desactivar", exact: true }).waitFor({ timeout: 120000 });
  assert.equal((await status()).installedSize, 4574861);
  await component.getByRole("button", { name: "Desactivar", exact: true }).click();
  await component.getByRole("button", { name: "Activar", exact: true }).waitFor();
  assert.equal((await status()).installed, true);
  await component.getByRole("button", { name: "Activar", exact: true }).click();
  await component.getByRole("button", { name: "Desactivar", exact: true }).waitFor();

  const fixture = Buffer.from(await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    canvas.width = 321; canvas.height = 241;
    const ctx = canvas.getContext("2d");
    const background = ctx.createLinearGradient(0, 0, 321, 241);
    background.addColorStop(0, "#eae2d8"); background.addColorStop(1, "#aeb2b2");
    ctx.fillStyle = background; ctx.fillRect(0, 0, 321, 241);
    ctx.fillStyle = "#8c888341"; ctx.beginPath(); ctx.ellipse(164, 192, 102, 20, 0, 0, Math.PI * 2); ctx.fill();
    const ceramic = ctx.createLinearGradient(70, 0, 224, 0);
    ceramic.addColorStop(0, "#942a48"); ceramic.addColorStop(0.45, "#ed6880"); ceramic.addColorStop(1, "#7c2543");
    ctx.strokeStyle = "#a73853"; ctx.lineWidth = 18;
    ctx.beginPath(); ctx.ellipse(216, 123, 38, 38, 0, -Math.PI * 0.6, Math.PI * 0.6); ctx.stroke();
    ctx.fillStyle = ceramic; ctx.beginPath(); ctx.moveTo(75, 68); ctx.lineTo(84, 170);
    ctx.bezierCurveTo(89, 207, 190, 208, 207, 171); ctx.lineTo(218, 68); ctx.closePath(); ctx.fill();
    ctx.fillStyle = "#f894a3"; ctx.beginPath(); ctx.ellipse(146, 68, 72, 23, 0, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = "#6b3440"; ctx.beginPath(); ctx.ellipse(146, 68, 61, 16, 0, 0, Math.PI * 2); ctx.fill();
    ctx.fillStyle = "#a97152"; ctx.beginPath(); ctx.ellipse(146, 74, 55, 8, 0, 0, Math.PI * 2); ctx.fill();
    return canvas.toDataURL("image/png").split(",")[1];
  }), "base64");
  const source = path.join(data, "Taza sintética.png");
  await writeFile(source, fixture);
  const imported = await rpc("import_paths", { paths: [source] });
  const libraryId = imported.library.find(item => item.name === "Taza sintética.png").id;
  await page.locator('[data-page="tools"]').click();
  const tool = page.locator(".background-tool");
  await tool.getByRole("button", { name: "Elegir imagen", exact: true }).click();
  await tool.getByRole("radio", { name: "Taza sintética.png" }).click();
  const generate = tool.getByRole("button", { name: "Generar muestra" });
  const submit = tool.getByRole("button", { name: "Exportar en la cola" });
  const images = tool.locator(".background-pair img");
  await generate.click();
  await images.first().waitFor({ timeout: 45000 });
  const baseline = await pixels(await images.nth(1).getAttribute("src"), 0);
  assert.ok(baseline.empty > 3000 && baseline.full > 3000);
  await page.locator("#background-edge").press("ArrowLeft");
  assert.equal(await submit.isDisabled(), true);
  await generate.click();
  await images.first().waitFor();
  const contracted = await pixels(await images.nth(1).getAttribute("src"), 0);
  assert.ok(contracted.sum < baseline.sum, "Contracting the edge reduces total mask coverage.");
  await page.locator("#background-surface").selectOption("light");
  assert.equal(await tool.locator(".background-pair [data-surface]").getAttribute("data-surface"), "light");
  await page.locator("#background-surface").selectOption("grid");
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await tool.evaluate(element => element.scrollWidth <= element.clientWidth + 1), `Overflow at ${width}px`);
    await tool.locator(".background-review").screenshot({ path: path.join(base, `preview-${width}.png`), animations: "disabled" });
  }
  await submit.click();
  const png = await outputFor(0);
  assert.equal(png.parentId, libraryId);
  const pngPixels = await pixels("data:image/png;base64," + (await readFile(png.path)).toString("base64"), 3);
  assert.deepEqual([pngPixels.width, pngPixels.height], [321, 241]);
  assert.ok(pngPixels.empty > 3000 && pngPixels.full > 3000);

  await page.locator("#background-format").selectOption("webp");
  await page.locator("#background-fill").selectOption("solid");
  await generate.click();
  await images.first().waitFor();
  await submit.click();
  const webp = await outputFor(1);
  assert.equal(webp.parentId, libraryId);
  assert.match(webp.name, /\.webp$/i);
  const webpPixels = await pixels("data:image/webp;base64," + (await readFile(webp.path)).toString("base64"), 3);
  assert.deepEqual([webpPixels.width, webpPixels.height], [321, 241]);
  assert.equal(webpPixels.full, 321 * 241);
  assert.ok(webpPixels.corner.slice(0, 3).every(value => value >= 235));
  assert.deepEqual(await readFile(source), fixture);
  assert.deepEqual(await readdir(path.join(data, "local-previews")), []);
  assert.deepEqual(errors, []);
  console.log("Verified real Windows ML mask, edge adjustment, matching native previews, transparent PNG and solid WebP.");
} catch (error) {
  if (log || child?.exitCode != null) console.error(`Native exit ${child?.exitCode}\n${log}`);
  throw error;
} finally {
  try { await page?.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close(); }); } catch {}
  if (child?.exitCode === null) {
    await Promise.race([new Promise(resolve => child.once("exit", resolve)), new Promise(resolve => setTimeout(resolve, 4000))]);
    if (child.exitCode === null) child.kill();
  }
  try { await browser?.close(); } catch {}
}
