import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { openTool } from "./tool-navigation.mjs";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = process.env.NABNAB_TRANSLATION_ARTIFACTS || "artifacts/translation-warnings";
await mkdir(directory, { recursive: true });
const browser = await chromium.launch({ headless: true, channel: "msedge" });
const page = await browser.newPage({ viewport: { width: 1180, height: 780 } });
const errors = [];
page.on("pageerror", error => errors.push(error.message));
const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri = true;
  mockWindows("main");
  const settings = { folder: "C:/Fixture", language: "es", reducedMotion: true, reducedTransparency: true, sidebarExpanded: true, trayEnabled: false };
  const job = { id: "fixture", title: "Escenas de prueba", source: { kind: "captionTranslation" }, state: "completed", outputs: [], createdAt: 1, runId: 1 };
  const dub = { id: "fixture-dub", title: "Voz de prueba", state: "error", createdAt: 2, runId: 1, outputs: [],
    source: { kind: "dubVoice", selection: { draftId: job.id, revision: 2, settings: { voiceId: "fixture-voice", rate: 0, volume: 100 } } },
    error: { code: "dubbing_timing", technical: "DUB_TIMING 2, 14", message: "Some cues do not fit." } };
  const snapshot = { settings, jobs: [job, dub], library: [], collections: [], drafts: [], sources: [], rules: [], smartCollections: [], presets: [], audioPlaylists: [] };
  const calls = window.translationCalls = [];
  const original = Array.from({ length: 15 }, (_, index) => ({ identifier: "escena-" + (index + 1), start: index * 8, end: index * 8 + (index === 13 ? 0.8 : 5), text: index === 2 ? "<i>Aquí continúa</i> la escena." : "Segmento original " + (index + 1) }));
  const translated = original.map((cue, index) => "Cue " + (index + 1));
  translated[0] = "This line is deliberately wider than the suggested line width.";
  translated[13] = "Too many words for a short interval.";
  let draft = { id: "fixture", revision: 1, name: "Escenas de prueba", selection: { input: { libraryId: "source", trackId: "track" }, pair: "es-en", sourceHash: "a".repeat(64) }, original, translated, contextualBlocks: 2, edited: [] };
  mockIPC(async (command, args = {}) => {
    calls.push({ command, args: structuredClone(args) });
    if (command === "startup") return { ...snapshot, folder: settings.folder, enginesReady: true, engineError: null };
    if (command === "get_snapshot") return snapshot;
    if (command === "optional_components") return [{ id: "translate-es-en", installed: true, enabled: true, available: true }];
    if (command === "voice_status") return { enabled: true, available: true, voices: [{ id: "fixture-voice", name: "Fixture voice", language: "en-US", enabled: true }] };
    if (command === "translation_drafts") return [draft.id];
    if (command === "translation_read") return structuredClone(draft);
    if (command === "translation_edit") {
      for (const { index, text } of args.edits) draft.translated[index] = text;
      draft = { ...draft, revision: draft.revision + 1, edited: args.edits.map(edit => edit.index) };
      return structuredClone(draft);
    }
  }, { shouldMockEvents: true });
  await import("/src/main.ts");
`;
await page.route(base, async route => {
  const response = await route.fetch();
  const html = (await response.text()).replace(/<script\s+type="module"\s+src="\/src\/main\.ts[^"]*"><\/script>/, `<script type="module">${setup}</script>`);
  await route.fulfill({ response, body: html });
});

try {
  await page.goto(base);
  await page.locator('[data-page="tools"]').click();
  await openTool(page, "translation");
  const tool = page.locator(".translation-tool");
  await tool.locator("#translation-draft").selectOption("fixture");
  const firstHint = tool.locator("#translation-warning-0");
  await firstHint.waitFor();
  const count = tool.locator("#translation-warning-count");
  const next = tool.getByRole("button", { name: "Siguiente aviso", exact: true });
  const first = tool.locator("#translation-cue-0");
  const second = tool.locator("#translation-cue-13");
  await page.setViewportSize({ width: 900, height: 620 });
  await tool.locator("#translation-title").evaluate(node => node.scrollIntoView({ block: "start" }));
  await page.screenshot({ path: path.join(directory, "editor-before-900.png") });
  const search = tool.getByRole("searchbox", { name: "Buscar segmentos", exact: true });
  await search.waitFor({ timeout: 4500 });
  await search.pressSequentially("aqui continua");
  assert.equal(await search.evaluate(input => input === document.activeElement), true);
  assert.deepEqual(await tool.locator(".translation-cues textarea").evaluateAll(fields => fields.map(field => field.id)), ["translation-cue-2"]);
  await search.fill("escena-14");
  assert.equal(await second.isVisible(), true);
  await search.fill("00:01:44");
  assert.equal(await second.isVisible(), true);
  await search.fill("segmento original");
  assert.equal(await tool.locator(".translation-pagination > span").innerText(), "1–12 / 14");
  await tool.getByRole("button", { name: "Siguiente", exact: true }).click();
  assert.deepEqual(await tool.locator(".translation-cues textarea").evaluateAll(fields => fields.map(field => field.id)), ["translation-cue-13", "translation-cue-14"]);
  assert.equal(await tool.locator(".translation-pagination > span").innerText(), "13–14 / 14");
  await search.fill("sin coincidencias");
  await tool.getByText("Ningún segmento coincide con la búsqueda.", { exact: true }).waitFor();
  assert.equal(await tool.locator(".translation-cues textarea").count(), 0);
  await search.fill("00:01:44");
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    await tool.locator("#translation-title").evaluate(node => node.scrollIntoView({ block: "start" }));
    assert.ok(await tool.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    await page.screenshot({ path: path.join(directory, `editor-search-${width}.png`) });
  }
  await search.fill("");
  assert.equal(await count.innerText(), "2 avisos de longitud");
  await next.press("Enter");
  assert.ok(await first.evaluate(input => input === document.activeElement));
  assert.match(await firstHint.innerText(), /caracteres en la línea más larga/);
  assert.match(await firstHint.innerText(), /Divide la línea/);
  await page.setViewportSize({ width: 900, height: 620 });
  await firstHint.scrollIntoViewIfNeeded();
  await page.screenshot({ path: path.join(directory, "line-warning-900.png") });
  await next.press("Enter");
  assert.ok(await second.evaluate(input => input === document.activeElement));
  const secondHint = tool.locator("#translation-warning-13");
  assert.match(await secondHint.innerText(), /caracteres por segundo/);
  assert.match(await secondHint.innerText(), /Acorta el texto/);
  assert.equal(await tool.locator(".translation-pagination > span").innerText(), "13–15 / 15");
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    await secondHint.scrollIntoViewIfNeeded();
    assert.ok(await tool.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    await page.screenshot({ path: path.join(directory, `reading-warning-${width}.png`) });
  }
  await next.press("Enter");
  assert.ok(await first.evaluate(input => input === document.activeElement));
  await first.fill("A short line.");
  assert.equal(await count.innerText(), "1 aviso de longitud");
  assert.equal(await firstHint.isHidden(), true);
  await next.press("Enter");
  assert.ok(await second.evaluate(input => input === document.activeElement));
  await second.fill("Brief.");
  assert.equal(await count.innerText(), "0 avisos de longitud");
  assert.equal(await next.isDisabled(), true);
  await search.fill("a short line");
  assert.equal(await first.inputValue(), "A short line.");
  await search.fill("brief");
  assert.equal(await second.inputValue(), "Brief.");
  await tool.getByRole("button", { name: "Guardar edición", exact: true }).click();
  const calls = await page.evaluate(() => window.translationCalls);
  assert.deepEqual(calls.find(call => call.command === "translation_edit").args, {
    draftId: "fixture", revision: 1, edits: [{ index: 0, text: "A short line." }, { index: 13, text: "Brief." }],
  });
  await tool.getByRole("button", { name: "Crear doblaje", exact: true }).press("Enter");
  const dubbing = page.locator(".dubbing-tool");
  const editSecond = dubbing.getByRole("button", { name: "Editar segmento 14", exact: true });
  await dubbing.locator(".dub-result").waitFor();
  await dubbing.locator(".dub-result").scrollIntoViewIfNeeded();
  await page.screenshot({ path: path.join(directory, "dub-timing-900.png") });
  await editSecond.press("Enter", { timeout: 2500 });
  await page.waitForFunction(() => document.activeElement?.id === "translation-cue-13");
  assert.equal(await tool.locator(".translation-pagination > span").innerText(), "13–15 / 15");
  const dubNext = tool.getByRole("button", { name: "Siguiente aviso de doblaje", exact: true });
  await second.fill("See you.");
  await search.fill("See you.");
  assert.equal(await second.inputValue(), "See you.");
  await dubNext.press("Enter");
  assert.equal(await search.inputValue(), "");
  const other = tool.locator("#translation-cue-1");
  assert.ok(await other.evaluate(input => input === document.activeElement));
  await other.fill("Hello.");
  await dubNext.press("Enter");
  assert.ok(await second.evaluate(input => input === document.activeElement));
  assert.equal(await second.inputValue(), "See you.");
  await tool.getByText("No cabía en la última síntesis.", { exact: true }).waitFor();
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    await second.scrollIntoViewIfNeeded();
    assert.ok(await tool.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    assert.ok(await dubNext.evaluate(button => {
      const area = document.getElementById("main").getBoundingClientRect();
      const { top, bottom } = button.getBoundingClientRect();
      return top >= area.top && bottom <= area.bottom;
    }));
    await page.screenshot({ path: path.join(directory, `dub-timing-editor-${width}.png`) });
  }
  await tool.getByRole("button", { name: "Guardar edición", exact: true }).press("Enter");
  await dubNext.waitFor({ state: "detached" });
  await tool.getByRole("button", { name: "Crear doblaje", exact: true }).press("Enter");
  await dubbing.locator("#dub-queue-voice").waitFor();
  assert.equal(await editSecond.count(), 0);
  assert.deepEqual(await page.evaluate(() => window.translationCalls.filter(call => call.command === "translation_edit").at(-1).args), {
    draftId: "fixture", revision: 2, edits: [{ index: 13, text: "See you." }, { index: 1, text: "Hello." }],
  });
  assert.deepEqual(errors, []);
  console.log("Length and dubbing warnings focus cues across pages and keep edits until save.");
} finally {
  await browser.close();
}
