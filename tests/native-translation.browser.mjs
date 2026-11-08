import assert from "node:assert/strict";
import { openTool } from "./tool-navigation.mjs";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const base = path.join(root, "artifacts/translation");
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "native-run-"));
const output = path.join(data, "output");
await mkdir(output);
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const endpoint = `http://127.0.0.1:${process.env.NABNAB_TRANSLATION_DEBUG_PORT || 9252}`;
let child, browser, page, log = "";

async function wait(check, timeout = 60_000) {
  const until = Date.now() + timeout;
  while (Date.now() < until) {
    const result = await check();
    if (result) return result;
    await new Promise(resolve => setTimeout(resolve, 150));
  }
  throw Error("Timed out waiting for the native translation workflow.");
}

function rpc(command, args = {}) {
  return page.evaluate(async ({ command, args }) => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    return invoke(command, args);
  }, { command, args });
}

async function jobOf(kind, index = 0) {
  return wait(async () => {
    const snapshot = await rpc("get_snapshot");
    const job = snapshot.jobs.filter(item => item.source.kind === kind)[index];
    if (job?.state === "error") throw Error(JSON.stringify(job.error));
    return job?.state === "completed" && { job, snapshot };
  });
}

try {
  child = spawn(path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
    cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${new URL(endpoint).port}` },
  });
  for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => { log = (log + chunk).slice(-8000); });
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
  await page.locator('[data-page="tools"]').click();
  await openTool(page, "translation");
  await page.getByRole("button", { name: "Ver componentes" }).click();
  const directions = [
    ["translate-es-en", "Bergamot · ES → EN", 17_448_973],
    ["translate-en-es", "Bergamot · EN → ES", 14_318_591],
  ];
  for (const [id, name, bytes] of directions) {
    const status = (await rpc("optional_components")).find(item => item.id === id);
    assert.equal(status.installed, false);
    assert.equal(status.size, bytes);
    const component = page.locator(".component-row").filter({ hasText: name });
    await component.getByRole("button", { name: "Instalar", exact: true }).click();
    await wait(async () => {
      if (await component.getByRole("button", { name: "Desactivar", exact: true }).isVisible()) return true;
      const notification = page.locator("#notification:not([hidden])");
      if (await notification.count()) throw Error(await notification.textContent());
      return false;
    }, 120_000);
  }
  const runtime = path.join(data, "components/translation-bergamot-0-4-9");
  assert.deepEqual((await readdir(runtime)).sort(), ["en-es", "es-en", "runtime"]);
  assert.deepEqual((await readdir(path.join(runtime, "runtime"))).sort(), [
    "bergamot-translator-worker.js", "bergamot-translator-worker.wasm", "translator-worker.js", "vocab.esen.spm",
  ]);

  const input = path.join(data, "Pista original ñ.vtt");
  const original = [
    "WEBVTT", "", "saludo-1", "00:00:00.110 --> 00:00:01.650", '¡Hola, <i>mundo</i>!<img src="https://example.invalid/test" onerror="alert(1)">',
    "", "saludo-2", "00:00:01.700 --> 00:00:03.930", "¿Cómo estás hoy?",
    "", "viaje", "00:00:05.100 --> 00:00:07.010", "El viernes iré",
    "", "viaje-final", "00:00:07.040 --> 00:00:09.215", "a buscar a mi hermana.", "",
  ].join("\n");
  await writeFile(input, original);
  const imported = await rpc("import_paths", { paths: [input] });
  const item = imported.library.find(candidate => candidate.name === path.basename(input));
  const track = item.details.tracks[0];
  await page.locator('[data-page="tools"]').click();
  const tool = page.locator(".translation-tool");
  await tool.locator("#translation-source").selectOption(`${item.id}/${track.id}`);
  await tool.locator("#translation-pair").selectOption("es-en");
  await tool.getByRole("button", { name: "Revisar pista" }).click();
  await tool.locator(".translation-original-sample").waitFor();
  assert.equal(await tool.locator(".translation-original-sample img").count(), 0);
  await tool.getByRole("button", { name: "Traducir en local" }).click();
  const { job } = await jobOf("captionTranslation");
  assert.deepEqual(job.outputs, []);
  await tool.locator("#translation-cue-0").waitFor();
  const draft = await rpc("translation_read", { draftId: job.id });
  assert.deepEqual(draft.original.map(cue => [cue.identifier, cue.start, cue.end]), [
    ["saludo-1", 0.11, 1.65], ["saludo-2", 1.7, 3.93], ["viaje", 5.1, 7.01], ["viaje-final", 7.04, 9.215],
  ]);
  assert.ok(draft.contextualBlocks >= 1);
  assert.match(draft.translated[0], /Hello, <i>world<\/i>/i);
  assert.match(draft.translated[1], /How are you today/i);
  assert.match(draft.translated[2] + " " + draft.translated[3], /Friday.*sister/i);
  assert.ok(draft.original.every(cue => !cue.text.includes("onerror")));
  const edited = "Hello, <i>world</i>. This longer cue tests whether the reader has enough time.";
  await tool.locator("#translation-cue-0").fill(edited);
  await tool.locator("#translation-warning-0").waitFor({ state: "visible" });
  await tool.getByRole("button", { name: "Guardar edición" }).click();
  await wait(async () => (await rpc("translation_read", { draftId: job.id })).revision === 2);
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await tool.evaluate(element => element.scrollWidth <= element.clientWidth + 1), `Overflow at ${width}px`);
    const firstCue = tool.locator(".translation-cue").first();
    assert.equal(await firstCue.locator(".translation-columns").evaluate(element => getComputedStyle(element).gridTemplateColumns.split(" ").length), 2);
    await firstCue.screenshot({ path: path.join(base, `comparison-${width}.png`), animations: "disabled" });
  }
  await tool.locator("#translation-bilingual").check();
  await tool.getByRole("button", { name: "Exportar subtítulos" }).click();
  const vtt = await jobOf("captionTranslationExport");
  const bilingual = await readFile(vtt.job.outputs[0], "utf8");
  assert.match(bilingual, /saludo-1\n00:00:00.110 --> 00:00:01.650\n¡Hola, <i>mundo<\/i>!\nHello, <i>world<\/i>/);
  assert.ok(bilingual.includes(edited));
  assert.ok(!bilingual.includes("onerror"));
  const indexed = vtt.snapshot.library.find(candidate => candidate.path === vtt.job.outputs[0]);
  assert.equal(indexed.parentId, item.id);
  assert.equal(indexed.details.translation.draftId, job.id);
  assert.equal(indexed.details.tracks[0].origin, "machineTranslation");
  assert.equal(indexed.details.tracks[0].path, indexed.path);

  await tool.locator("#translation-format").selectOption("srt");
  await tool.locator("#translation-bilingual").uncheck();
  await tool.getByRole("button", { name: "Exportar subtítulos" }).click();
  const srt = await jobOf("captionTranslationExport", 1);
  const translated = await readFile(srt.job.outputs[0], "utf8");
  assert.ok(translated.startsWith(`1\n00:00:00,110 --> 00:00:01,650\n${edited}\n`));
  assert.ok(!translated.includes("¡Hola"));
  assert.equal(await readFile(input, "utf8"), original);
  assert.deepEqual(await readdir(path.join(data, "local-previews")), []);
  assert.deepEqual(await readdir(path.join(data, "components/local-previews")), []);

  const reverseInput = { libraryId: indexed.id, trackId: indexed.details.tracks[0].id };
  const reverseSource = await rpc("translation_source", { input: reverseInput });
  const [reverse] = await rpc("queue_translation", {
    selection: { input: reverseInput, pair: "en-es", sourceHash: reverseSource.sourceHash }, clientRequest: "translation-native-reverse",
  });
  await jobOf("captionTranslation", 1);
  const reversed = await rpc("translation_read", { draftId: reverse.id });
  assert.match(reversed.translated[0], /Hola/i);
  assert.equal(reversed.original[3].end, 9.215);

  await page.reload();
  await page.locator('[data-page="tools"]').click();
  await openTool(page, "translation");
  await tool.locator("#translation-draft").selectOption(job.id);
  await tool.locator("#translation-cue-0").waitFor();
  assert.equal(await tool.locator("#translation-cue-0").inputValue(), edited);
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ directions: 2, contextualBlocks: draft.contextualBlocks, preservedCues: draft.original.length,
    savedRevision: 2, exports: ["bilingual VTT", "translated SRT"], viewports: [1180, 900], tempFiles: 0 }));
} catch (error) {
  console.error(`Native exit ${child?.exitCode}\n${log}`);
  console.error(await page?.locator("#notification").textContent().catch(() => ""));
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
