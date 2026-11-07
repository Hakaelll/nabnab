import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFile, mkdir, mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const base = path.join(root, "artifacts/stems");
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "native-run-"));
const output = path.join(data, "output");
const fixtureRuntime = process.env.NABNAB_TEST_STEMS_RUNTIME;
await mkdir(output);
if (fixtureRuntime) {
  const runtime = path.join(data, "components/stems-sherpa-2-0-1");
  await mkdir(runtime, { recursive: true });
  for (const file of ["sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll", "vocals.fp16.onnx", "accompaniment.fp16.onnx"]) {
    await copyFile(path.join(fixtureRuntime, file), path.join(runtime, file));
  }
  await writeFile(path.join(data, "components/stems.enabled"), "1");
}

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const endpoint = `http://127.0.0.1:${process.env.NABNAB_STEMS_DEBUG_PORT || 9251}`;
const engines = path.join(root, "src-tauri/engines");
const hash = bytes => createHash("sha256").update(bytes).digest("hex");
let child, browser, page, log = "";

async function wait(check, timeout = 60_000) {
  const until = Date.now() + timeout;
  while (Date.now() < until) {
    const result = await check();
    if (result) return result;
    await new Promise(resolve => setTimeout(resolve, 120));
  }
  throw Error("Timed out waiting for the native stems workflow.");
}

function rpc(command, args = {}) {
  return page.evaluate(async ({ command, args }) => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    return invoke(command, args);
  }, { command, args });
}

function ffmpeg(...args) {
  return execFileSync(path.join(engines, "ffmpeg.exe"), ["-hide_banner", "-loglevel", "error", ...args], { windowsHide: true, maxBuffer: 4 * 1024 * 1024 });
}

function probe(file) {
  return JSON.parse(execFileSync(path.join(engines, "ffprobe.exe"), [
    "-v", "error", "-select_streams", "a:0", "-show_streams", "-of", "json", file,
  ], { windowsHide: true })).streams[0];
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
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  const initial = await wait(async () => { try { return await rpc("get_snapshot"); } catch { return null; } });
  await rpc("save_settings", { settings: { ...initial.settings, folder: output, language: "es", reducedMotion: true, trayEnabled: false, closeToTray: false } });
  const status = async () => (await rpc("optional_components")).find(item => item.id === "stems");
  assert.equal((await status()).installed, Boolean(fixtureRuntime));
  await page.locator('[data-page="tools"]').click();
  await page.locator("#local-tool-stems").click();
  if (!fixtureRuntime) {
    await page.getByRole("button", { name: "Ver componentes" }).click();
    const row = page.locator(".component-row").filter({ hasText: "Spleeter · sherpa-onnx" });
    await row.getByRole("button", { name: "Instalar", exact: true }).click();
    await row.getByRole("button", { name: "Desactivar", exact: true }).waitFor({ timeout: 120_000 });
  }
  assert.equal((await status()).size, 41_142_144);
  assert.equal((await status()).installedSize, 56_406_521);
  assert.equal((await status()).enabled, true);
  assert.deepEqual(await readdir(path.join(data, "components/stems-sherpa-2-0-1")), [
    "accompaniment.fp16.onnx", "onnxruntime.dll", "onnxruntime_providers_shared.dll",
    "sherpa-onnx-c-api.dll", "vocals.fp16.onnx",
  ]);

  const source = path.join(data, "Mezcla de prueba ñ.wav");
  ffmpeg("-f", "lavfi", "-i", "aevalsrc=0.82*sin(2*PI*440*t)+0.16*sin(2*PI*997*t):s=44100:d=21.03",
    "-c:a", "pcm_s16le", source);
  const originalHash = hash(await readFile(source));
  const imported = await rpc("import_paths", { paths: [source] });
  const libraryId = imported.library.find(item => item.name === path.basename(source)).id;
  await page.locator('[data-page="tools"]').click();
  const tool = page.locator(".stems-tool");
  await tool.locator("#stems-source").selectOption(libraryId);
  await tool.locator("#stems-start").fill("2.5");
  const generate = tool.getByRole("button", { name: "Generar muestra" });
  const submit = tool.getByRole("button", { name: "Exportar pistas" });
  assert.equal(await submit.isDisabled(), true);
  await generate.click();
  await tool.locator(".stems-review").waitFor();
  const position = tool.locator("#stems-position");
  assert.equal(Number(await position.getAttribute("max")), 8);
  await tool.getByRole("button", { name: "Reproducir" }).click();
  await wait(async () => Number(await position.inputValue()) > 0.2);
  const beforeSwitch = Number(await position.inputValue());
  await tool.getByRole("button", { name: "Voz", exact: true }).click();
  assert.equal(await tool.getByRole("button", { name: "Voz", exact: true }).getAttribute("aria-pressed"), "true");
  assert.ok(Number(await position.inputValue()) >= beforeSwitch);
  await tool.getByRole("button", { name: "Pausar" }).click();
  await position.focus();
  await position.press("ArrowRight");
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await tool.evaluate(element => element.scrollWidth <= element.clientWidth + 1), `Overflow at ${width}px`);
    await tool.locator(".asset-review").screenshot({ path: path.join(base, `sample-${width}.png`), animations: "disabled" });
  }
  await submit.click();
  const completed = await wait(async () => {
    const snapshot = await rpc("get_snapshot");
    const job = snapshot.jobs.find(item => item.source.kind === "stems");
    if (job?.state === "error") throw Error(JSON.stringify(job.error));
    return job?.state === "completed" && { job, items: snapshot.library.filter(item => item.parentId === libraryId) };
  });
  assert.equal(completed.items.length, 2);
  assert.equal(completed.job.outputs.length, 2);
  const set = path.dirname(completed.job.outputs[0]);
  assert.deepEqual(await readdir(set), ["Instrumental.flac", "Vocals.flac"]);
  const inputStream = probe(source);
  const stems = [];
  for (const item of completed.items) {
    assert.equal(item.details.stems.setId, completed.job.id);
    assert.equal(item.collections[0], completed.items[0].collections[0]);
    assert.equal(path.dirname(item.path), set);
    const audio = probe(item.path);
    assert.equal(audio.channels, 1);
    assert.equal(audio.sample_rate, "44100");
    assert.equal(audio.duration_ts, inputStream.duration_ts);
    const samples = ffmpeg("-i", item.path, "-f", "s16le", "-");
    let peak = 0;
    for (let i = 0; i < samples.length; i += 2) peak = Math.max(peak, Math.abs(samples.readInt16LE(i)));
    assert.ok(peak > 0 && peak < 32_767);
    stems.push({ role: item.details.stems.role, frames: audio.duration_ts, peak, sha256: hash(samples) });
  }
  assert.notEqual(stems[0].sha256, stems[1].sha256);
  assert.equal(hash(await readFile(source)), originalHash);
  const cancelledSource = path.join(data, "Cancelación estéreo.wav");
  ffmpeg("-f", "lavfi", "-i", "sine=frequency=550:sample_rate=44100:duration=45", "-ac", "2",
    "-c:a", "pcm_s16le", cancelledSource);
  const importedStereo = await rpc("import_paths", { paths: [cancelledSource] });
  const stereoId = importedStereo.library.find(item => item.name === path.basename(cancelledSource)).id;
  const sample = await rpc("stems_preview", { libraryId: stereoId, start: 40, requestId: Date.now() });
  assert.equal(sample.channels, 2);
  assert.equal(sample.seconds, 5);
  const [cancellable] = await rpc("queue_stems", { previewId: sample.previewId, clientRequest: "cancel-native-separation" });
  await rpc("stems_release", { previewId: sample.previewId });
  const jobState = async () => (await rpc("get_snapshot")).jobs.find(job => job.id === cancellable.id).state;
  await wait(async () => (await jobState()) === "processing");
  await rpc("job_action", { ids: [cancellable.id], action: "cancel" });
  await wait(async () => (await jobState()) === "cancelled" && !(await readdir(output)).some(name => name.startsWith(".nabnab-")));
  await wait(async () => (await readdir(path.join(data, "local-previews"))).length === 0);
  assert.deepEqual(await readdir(output), [path.basename(set)]);
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ installed: !fixtureRuntime, synchronized: true, cancelled: true, viewports: [1180, 900], stems }));
} catch (error) {
  console.error(`Native exit ${child?.exitCode}\n${log}`);
  console.error(await page?.locator("#page-content").innerText().catch(() => ""));
  throw error;
} finally {
  try { await page?.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close(); }); } catch {}
  if (child?.exitCode === null) {
    await Promise.race([new Promise(resolve => child.once("exit", resolve)), new Promise(resolve => setTimeout(resolve, 4000))]);
    if (child.exitCode === null) child.kill();
  }
  await browser?.close().catch(() => {});
}
