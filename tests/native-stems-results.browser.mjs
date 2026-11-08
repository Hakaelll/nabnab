// Checks playback of completed FLAC fixtures without downloading a model.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(import.meta.dirname, "..");
const base = path.join(root, "artifacts/stems");
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "results-run-"));
const output = path.join(data, "output");
await mkdir(output);
const original = path.join(data, "Mezcla de prueba.mp4");
const vocals = path.join(output, "Vocals.flac");
const instrumental = path.join(output, "Instrumental.flac");
const ffmpeg = path.join(root, "src-tauri/engines/ffmpeg.exe");
const encode = (...args) => execFileSync(ffmpeg, ["-hide_banner", "-loglevel", "error", ...args], { windowsHide: true });
encode("-f", "lavfi", "-i", "color=c=black:s=160x90:r=12:d=21", "-f", "lavfi", "-i", "sine=frequency=480:duration=21",
  "-c:v", "libopenh264", "-c:a", "aac", "-shortest", original);
for (const [file, frequency] of [[vocals, 320], [instrumental, 660]]) {
  encode("-f", "lavfi", "-i", `sine=frequency=${frequency}:duration=21`, "-c:a", "flac", file);
}
execFileSync("rustc", ["--edition=2021", "-C", "debuginfo=0", path.join(root, "tests/kdeconnect-cli-fixture.rs"), "-o", path.join(data, "kdeconnect-cli.exe")], { windowsHide: true });

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
let child, browser, page, launches = 0;
const port = Number(process.env.NABNAB_STEMS_RESULTS_DEBUG_PORT || 9380);

async function wait(check) {
  const deadline = Date.now() + 25_000;
  while (Date.now() < deadline) {
    if (child?.exitCode !== null) throw Error(`Native app exited ${child?.exitCode}.`);
    const value = await check();
    if (value) return value;
    await new Promise(resolve => setTimeout(resolve, 120));
  }
  throw Error("Timed out waiting for the stems result.");
}

const rpc = (command, args = {}) => page.evaluate(async ({ command, args }) => {
  const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
  return invoke(command, args);
}, { command, args });

async function launch() {
  const debug = port + launches++;
  const endpoint = `http://127.0.0.1:${debug}`;
  child = spawn(path.join(root, "src-tauri/target/debug/nabnab.exe"), [], { cwd: root, windowsHide: true, stdio: "ignore",
    env: { ...process.env, NABNAB_TEST_DATA_DIR: data, Path: `${data}${path.delimiter}${process.env.Path ?? process.env.PATH ?? ""}`,
      WEBVIEW2_USER_DATA_FOLDER: path.join(data, `webview-${launches}`), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debug} --mute-audio` } });
  await wait(async () => { try { return (await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(400) })).ok; } catch { return false; } });
  browser = await chromium.connectOverCDP(endpoint);
  page = await wait(() => browser.contexts()[0]?.pages().find(tab => /:1420/.test(tab.url())));
  return wait(async () => { try { return await rpc("get_snapshot"); } catch { return null; } });
}

async function close() {
  try { await page?.evaluate(async () => (await import("/node_modules/@tauri-apps/api/window.js")).getCurrentWindow().close()); } catch {}
  if (child?.exitCode === null) {
    const done = new Promise(resolve => child.once("exit", resolve));
    await Promise.race([done, new Promise(resolve => setTimeout(resolve, 4000))]);
    if (child.exitCode === null) { child.kill(); await done; }
  }
  await browser?.close().catch(() => {});
  page = browser = child = null;
}

try {
  const initial = await launch();
  await rpc("save_settings", { settings: { ...initial.settings, folder: output, language: "es", reducedMotion: true, trayEnabled: false, closeToTray: false } });
  await rpc("import_paths", { paths: [original, vocals, instrumental] });
  await close();
  const repository = path.join(data, "state-v1.json");
  const fixture = JSON.parse(await readFile(repository, "utf8"));
  const source = fixture.library.find(item => item.name === path.basename(original));
  const files = ["Vocals", "Instrumental"].map(role => {
    const item = fixture.library.find(item => item.name === `${role}.flac`);
    item.parentId = source.id;
    item.details.stems = { setId: "19999-1", role, model: "Spleeter" };
    return item;
  });
  fixture.jobs.push({ id: "19999-1", runId: 1, clientRequest: "fixture-stems-results", title: "Pistas de prueba", createdAt: 19999, updatedAt: 19999,
    state: "completed", source: { kind: "stems", selection: { libraryId: source.id, inputHash: createHash("sha256").update(await readFile(original)).digest("hex") } },
    folder: output, provider: "Spleeter", thumbnail: null, progress: null, outputs: files.map(item => item.path), error: null, attempt: 1, groupId: null, associations: [] });
  await writeFile(repository, JSON.stringify(fixture));
  await launch();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.locator('[data-page="tools"]').click();
  await page.locator("#local-tool-stems").click();
  await page.locator("#stems-source").selectOption(source.id);
  const panel = page.locator(".stems-output");
  const baseline = process.argv.includes("--baseline");
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    await page.locator("#stems-form .asset-submit").evaluate(node => node.scrollIntoView({ block: "start" }));
    assert.equal(await panel.count(), baseline ? 0 : 1);
    if (!baseline) assert.ok(await panel.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await page.screenshot({ path: path.join(data, `stems-${width}.png`), animations: "disabled" });
  }
  if (!baseline) {
    const media = page.locator("#audio-engine");
    const engine = await media.elementHandle();
    const listen = label => panel.getByRole("button", { name: label, exact: true });
    const at = (id, time, paused) => media.evaluate((node, { id, time, paused }) => node.dataset.localPlayer === id
      && node.readyState >= 2 && !node.seeking && node.paused === paused && Math.abs(node.currentTime - time) < (paused ? 0.2 : 1), { id, time, paused });
    await listen("Original").press("Enter");
    await wait(() => at(source.id, 0.5, false));
    assert.equal(await page.locator("#playback-stage").isVisible(), false);
    const seek = page.locator("#audio-seek");
    await seek.fill("10");
    await wait(() => at(source.id, 10, false));
    await listen("Voz").press("Enter");
    await wait(() => at(files[0].id, 10.5, false));
    await page.locator("#audio-toggle").click();
    await seek.fill("14");
    await listen("Instrumental").press("Enter");
    await wait(() => at(files[1].id, 14, true));
    assert.equal(await listen("Instrumental").getAttribute("aria-pressed"), "true");
    assert.equal(await listen("Instrumental").evaluate(button => button === document.activeElement), true);
    await listen("Original").press("Enter");
    await wait(() => at(source.id, 14, true));
    assert.equal(await engine.evaluate(node => node === document.getElementById("audio-engine")), true);
    assert.equal(await media.evaluate(node => Math.round(node.duration)), 21);
    await panel.scrollIntoViewIfNeeded();
    const box = await panel.boundingBox();
    await page.screenshot({ path: path.join(data, "stems-results.png"), animations: "disabled",
      clip: { x: box.x - 10, y: box.y, width: box.width + 20, height: box.height } });

    await panel.getByRole("button", { name: "Enviar las dos pistas" }).press("Enter");
    const dialog = page.locator(".device-share[open]");
    await dialog.locator('#share-device option[value="paired-device-123"]').waitFor({ state: "attached" });
    await dialog.locator("#share-device").selectOption("paired-device-123");
    await dialog.locator("#share-review").click();
    await dialog.locator("#share-send").waitFor();
    assert.deepEqual(await dialog.locator(".device-share-review li > span").allTextContents(), ["Vocals.flac", "Instrumental.flac"]);
    const receipt = path.join(data, "cli-arguments.jsonl");
    await assert.rejects(readFile(receipt), { code: "ENOENT" });
    await dialog.locator("#share-send").click();
    await dialog.getByText("Entregado a KDE Connect", { exact: true }).waitFor();
    const args = JSON.parse((await readFile(receipt, "utf8")).trim());
    assert.equal(args[0], "--device=paired-device-123");
    assert.equal(args[2], "--");
    const paths = [args[1].replace(/^--share=/, ""), ...args.slice(3)].map(url => path.toNamespacedPath(fileURLToPath(url)));
    assert.deepEqual(paths, files.map(item => path.toNamespacedPath(item.path)));
  }
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ baseline, output: data, comparison: !baseline, fileShare: !baseline }));
} finally {
  await close();
}
