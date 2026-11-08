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
  const snapshot = { settings, jobs: [job], library: [], collections: [], drafts: [], sources: [], rules: [], smartCollections: [], presets: [], audioPlaylists: [] };
  const calls = window.translationCalls = [];
  const original = Array.from({ length: 15 }, (_, index) => ({ identifier: "escena-" + (index + 1), start: index * 8, end: index * 8 + (index === 13 ? 0.8 : 5), text: "Segmento original " + (index + 1) }));
  const translated = original.map((cue, index) => "Cue " + (index + 1));
  translated[0] = "This line is deliberately wider than the suggested line width.";
  translated[13] = "Too many words for a short interval.";
  let draft = { id: "fixture", revision: 1, name: "Escenas de prueba", selection: { input: { libraryId: "source", trackId: "track" }, pair: "es-en", sourceHash: "a".repeat(64) }, original, translated, contextualBlocks: 2, edited: [] };
  mockIPC(async (command, args = {}) => {
    calls.push({ command, args: structuredClone(args) });
    if (command === "startup") return { ...snapshot, folder: settings.folder, enginesReady: true, engineError: null };
    if (command === "get_snapshot") return snapshot;
    if (command === "optional_components") return [{ id: "translate-es-en", installed: true, enabled: true, available: true }];
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
  await tool.getByRole("button", { name: "Anterior", exact: true }).click();
  assert.equal(await first.inputValue(), "A short line.");
  await tool.getByRole("button", { name: "Guardar edición", exact: true }).click();
  const calls = await page.evaluate(() => window.translationCalls);
  assert.deepEqual(calls.find(call => call.command === "translation_edit").args, {
    draftId: "fixture", revision: 1, edits: [{ index: 0, text: "A short line." }, { index: 13, text: "Brief." }],
  });
  assert.deepEqual(errors, []);
  console.log("Length warnings distinguish line width and reading speed, focus cues across pages, and keep both edits until save.");
} finally {
  await browser.close();
}
