import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { cp, mkdir, mkdtemp, readdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { createRequire } from "node:module";

const root = path.resolve(import.meta.dirname, "..");
const base = path.join(root, "artifacts/upscale");
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "native-run-"));
const output = path.join(data, "output");
await mkdir(output);
const runtime = process.env.NABNAB_TEST_UPSCALE_RUNTIME;
if (runtime) {
  await mkdir(path.join(data, "components"));
  await cp(runtime, path.join(data, "components/upscale-2022-04-24"), { recursive: true });
  await writeFile(path.join(data, "components/upscale.enabled"), "1");
}
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const endpoint = `http://127.0.0.1:${process.env.NABNAB_UPSCALE_DEBUG_PORT || 9249}`;
let child, browser, page;
const errors = [];
let nativeLog = "";

async function wait(check, timeout = 60000) {
  const until = Date.now() + timeout;
  while (Date.now() < until) {
    const result = await check();
    if (result) return result;
    await new Promise(resolve => setTimeout(resolve, 150));
  }
  throw Error("Timed out waiting for the native upscale workflow.");
}

function rpc(command, args = {}) {
  return page.evaluate(async ({ command, args }) => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    return invoke(command, args);
  }, { command, args });
}

try {
  child = spawn(path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
    cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${new URL(endpoint).port}` },
  });
  child.stdout.on("data", chunk => { nativeLog = (nativeLog + chunk).slice(-6000); });
  child.stderr.on("data", chunk => { nativeLog = (nativeLog + chunk).slice(-6000); });
  await wait(async () => {
    if (child.exitCode !== null) throw Error(`Native app exited ${child.exitCode}`);
    try { return (await fetch(`${endpoint}/json/version`)).ok; } catch { return false; }
  });
  browser = await chromium.connectOverCDP(endpoint);
  page = await wait(() => browser.contexts()[0]?.pages().find(tab => /:1420/.test(tab.url())));
  page.on("pageerror", error => errors.push(error.message));
  const initial = await wait(async () => { try { return await rpc("get_snapshot"); } catch { return null; } });
  await rpc("save_settings", { settings: { ...initial.settings, folder: output, language: "es", reducedMotion: true, trayEnabled: false, closeToTray: false } });
  const status = async () => (await rpc("optional_components")).find(item => item.id === "upscale");
  assert.equal((await status()).installed, Boolean(runtime));
  await page.locator('[data-page="tools"]').click();
  await page.locator("#local-tool-upscale").click();
  if (!runtime) await page.getByRole("button", { name: "Ver componentes" }).click();
  else {
    await page.locator('[data-page="settings"]').click();
    await page.locator('[data-action="settings-section"][data-section="engines"]').click();
  }
  const component = page.locator(".component-row").filter({ hasText: "Real-ESRGAN · ncnn Vulkan" });
  if (!runtime) await component.getByRole("button", { name: "Instalar", exact: true }).click();
  await component.getByRole("button", { name: "Desactivar", exact: true }).waitFor({ timeout: 180000 });
  assert.equal((await status()).installedSize, 43636188);
  if (!runtime) assert.deepEqual(await readdir(path.join(data, "components/local-previews")), []);
  await component.getByRole("button", { name: "Desactivar", exact: true }).click();
  await component.getByRole("button", { name: "Activar", exact: true }).waitFor();
  assert.equal((await status()).installed, true);
  assert.equal((await status()).enabled, false);
  await component.getByRole("button", { name: "Activar", exact: true }).click();
  await component.getByRole("button", { name: "Desactivar", exact: true }).waitFor();

  const fixture = Buffer.from(await page.evaluate(() => {
    const canvas = document.createElement("canvas");
    canvas.width = 240; canvas.height = 150;
    const ctx = canvas.getContext("2d");
    ctx.fillStyle = "#e4be9b"; ctx.fillRect(0, 0, 240, 150);
    ctx.fillStyle = "#fbe3b3"; ctx.beginPath(); ctx.arc(133, 48, 21, 0, 2 * Math.PI); ctx.fill();
    const hills = [[35, 80, "#8c6373"], [83, 68, "#655374"], [176, 70, "#724c66"], [203, 91, "#34385a"]];
    for (const [x, y, color] of hills) {
      ctx.fillStyle = color; ctx.beginPath(); ctx.moveTo(x - 95, 150); ctx.lineTo(x, y); ctx.lineTo(x + 99, 150); ctx.fill();
    }
    ctx.fillStyle = "#2d344e"; ctx.fillRect(0, 127, 240, 23);
    ctx.fillStyle = "#c69175";
    for (let x = 11; x < 240; x += 21) { ctx.fillRect(x, 131, 5, 2); ctx.fillRect(x + 8, 139, 7, 1); }
    return canvas.toDataURL("image/png").split(",")[1];
  }), "base64");
  const paths = ["Horizonte sintético.png", "Copia del horizonte.png"].map(name => path.join(data, name));
  for (const file of paths) await writeFile(file, fixture);
  const imported = await rpc("import_paths", { paths });
  const ids = imported.library.filter(item => /horizonte/i.test(item.name)).map(item => item.id);
  assert.equal(ids.length, 2);
  await page.locator('[data-page="tools"]').click();
  const tool = page.locator(".upscale-tool");
  await tool.getByRole("button", { name: "Elegir imágenes" }).click();
  await tool.getByRole("checkbox", { name: "Horizonte sintético.png" }).check();
  await tool.getByRole("checkbox", { name: "Copia del horizonte.png" }).check();
  await tool.getByRole("button", { name: "Cerrar lista" }).click();
  assert.deepEqual(await page.locator("#upscale-scale option").allTextContents(), ["×4"]);
  await page.locator("#upscale-model").selectOption("illustration");
  await page.locator("#upscale-scale").selectOption("3");
  await tool.getByRole("button", { name: "Generar muestra" }).click();
  const pair = tool.locator(".upscale-pair");
  await pair.waitFor({ timeout: 45000 });
  assert.deepEqual(await pair.locator("img").evaluateAll(async images => Promise.all(images.map(async img => {
    await img.decode(); return [img.naturalWidth, img.naturalHeight];
  }))), [[480, 450], [480, 450]]);
  assert.notEqual(await pair.locator("img").nth(0).getAttribute("src"), await pair.locator("img").nth(1).getAttribute("src"));
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    const layout = await tool.evaluate(element => {
      const boundary = element.getBoundingClientRect().right;
      const overflow = [...element.querySelectorAll("*")].filter(child => child.getBoundingClientRect().right > boundary + 1)
        .map(child => [child.tagName, child.id, child.className]).slice(0, 12);
      return { width: element.clientWidth, content: element.scrollWidth, overflow };
    });
    assert.ok(layout.content <= layout.width + 1, `Overflow at ${width}px: ${JSON.stringify(layout)}`);
    await tool.locator(".upscale-review").scrollIntoViewIfNeeded();
    await tool.locator(".upscale-review").screenshot({ path: path.join(base, `preview-${width}.png`), animations: "disabled" });
  }
  await tool.getByRole("button", { name: "Ampliar en la cola" }).click();
  const completed = await wait(async () => {
    const snapshot = await rpc("get_snapshot");
    const jobs = snapshot.jobs.filter(item => item.source.kind === "upscale");
    const failed = jobs.find(job => job.state === "error");
    if (failed) throw Error(JSON.stringify(failed.error));
    if (jobs.length === 2 && jobs.every(job => job.state === "completed")) return jobs.map(job => snapshot.library.find(item => item.path === job.outputs[0]));
    return null;
  });
  assert.deepEqual(new Set(completed.map(item => item.parentId)), new Set(ids));
  for (const item of completed) assert.deepEqual([item.width, item.height], [720, 450]);
  for (const file of paths) assert.deepEqual(await readFile(file), fixture);
  assert.deepEqual(errors, []);
  console.log(`Verified Windows component ${runtime ? "with verified local files" : "install"}, disable and enable, matched crop and two queued Vulkan outputs.`);
} catch (error) {
  if (nativeLog || child?.exitCode != null) console.error(`Native exit ${child?.exitCode}\n${nativeLog}`);
  throw error;
} finally {
  try { await page?.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close(); }); } catch {}
  if (child?.exitCode === null) {
    await Promise.race([new Promise(resolve => child.once("exit", resolve)), new Promise(resolve => setTimeout(resolve, 4000))]);
    if (child.exitCode === null) child.kill();
  }
  try { await browser?.close(); } catch {}
}
