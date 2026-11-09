import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { createRequire } from "node:module";
import { openTool } from "./tool-navigation.mjs";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = process.env.NABNAB_STEMS_ARTIFACTS || "artifacts/stems-waveforms";
await mkdir(directory, { recursive: true });
const browser = await chromium.launch({ headless: true, channel: "msedge", args: ["--mute-audio"] });
const page = await browser.newPage({ viewport: { width: 900, height: 620 } });
const errors = [];
page.on("pageerror", error => errors.push(error.message));

const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri = true;
  mockWindows("main");
  const settings = { folder: "C:/Fixture", language: "es", reducedMotion: true, reducedTransparency: true, sidebarExpanded: true, trayEnabled: false };
  const snapshot = { settings, jobs: [], library: [
    { id: "mix", name: "Mezcla de prueba.wav", kind: "audio", path: "C:/Fixture/Mezcla de prueba.wav", duration: 30 },
    { id: "unknown", name: "Mezcla sin duración.wav", kind: "audio", path: "C:/Fixture/Mezcla sin duración.wav", duration: null },
  ], collections: [], drafts: [], sources: [], rules: [], smartCollections: [], presets: [], audioPlaylists: [] };
  const calls = window.stemsCalls = [];
  function wav(track) {
    const rate = 16000, length = rate * 4 * 4;
    const bytes = new ArrayBuffer(44 + length);
    const data = new DataView(bytes);
    const text = (offset, value) => [...value].forEach((char, index) => data.setUint8(offset + index, char.charCodeAt(0)));
    text(0, "RIFF"); data.setUint32(4, 36 + length, true); text(8, "WAVE");
    text(12, "fmt "); data.setUint32(16, 16, true); data.setUint16(20, 1, true); data.setUint16(22, 2, true);
    data.setUint32(24, rate, true); data.setUint32(28, rate * 4, true); data.setUint16(32, 4, true); data.setUint16(34, 16, true);
    text(36, "data"); data.setUint32(40, length, true);
    for (let index = 0; index < rate * 4; index++) {
      const t = index / rate;
      const voice = 12000 * Math.sin(t * 1820) * (Math.max(0, Math.sin(t * 9 - 0.4)) ** 2);
      const backing = 3800 * Math.sin(t * 810) * (0.2 + 0.8 * Math.max(0, Math.cos(t * 13)) ** 4);
      data.setInt16(44 + index * 4, track === 1 ? 0 : backing, true);
      data.setInt16(46 + index * 4, track === 0 ? voice + backing : track === 1 ? voice : backing, true);
    }
    return bytes;
  }
  mockIPC(async (command, args = {}) => {
    calls.push({ command, args: structuredClone(args) });
    if (command === "startup") return { ...snapshot, enginesReady: true };
    if (command === "get_snapshot") return snapshot;
    if (command === "optional_components") return [{ id: "stems", installed: true, enabled: true, available: true }];
    if (command === "stems_preview") return { previewId: 22, seconds: 4, channels: 2, originalChannels: 2 };
    if (command === "stems_sample") return wav(args.track);
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
  await openTool(page, "stems");
  const tool = page.locator(".stems-tool");
  const start = tool.getByRole("spinbutton", { name: "Inicio (segundos)", exact: true });
  const timeline = tool.getByRole("slider", { name: "Inicio en la línea de tiempo", exact: true });
  await tool.locator("#stems-source").selectOption("unknown");
  assert.equal(await timeline.count(), 0);
  assert.equal(await start.isEnabled(), true);
  await tool.locator("#stems-source").selectOption("mix");
  await timeline.waitFor({ timeout: 2500 });
  await timeline.press("End");
  assert.equal(await start.inputValue(), "29.9");
  assert.equal(await timeline.getAttribute("aria-valuetext"), "De 0:29,9 a 0:30");
  await start.fill("12.3");
  assert.equal(await timeline.inputValue(), "12.3");
  await timeline.press("ArrowRight");
  assert.equal(await start.inputValue(), "12.4");
  assert.equal(await timeline.getAttribute("aria-valuetext"), "De 0:12,4 a 0:20,4");
  await timeline.scrollIntoViewIfNeeded();
  const { x, y, width, height } = await timeline.boundingBox();
  await page.mouse.move(x + width * 0.42, y + height / 2);
  await page.mouse.down();
  await page.mouse.move(x + width * 0.75, y + height / 2, { steps: 8 });
  await page.mouse.up();
  assert.ok(Number(await start.inputValue()) > 20);
  await start.fill("12.4");
  for (const [width, height] of [[900, 620], [1180, 780]]) {
    await page.setViewportSize({ width, height });
    await tool.locator(".asset-review").evaluate(section => section.scrollIntoView({ block: "start" }));
    assert.ok(await tool.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    await page.screenshot({ path: `${directory}/stems-window-${width}.png` });
  }
  await page.setViewportSize({ width: 900, height: 620 });
  await tool.getByRole("button", { name: "Generar muestra" }).click();
  const review = tool.locator(".stems-review");
  const position = review.getByRole("slider", { name: "Posición en la muestra" });
  await position.fill("1.5");
  const vocal = review.getByRole("button", { name: "Voz", exact: true });
  await vocal.press("Enter");
  await review.scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${directory}/sample-900.png` });
  const waveforms = review.locator(".stems-wave path");
  await waveforms.first().waitFor({ timeout: 3000 });
  assert.equal(await waveforms.count(), 3);
  const [original, voice, instrumental] = await waveforms.evaluateAll(paths => paths.map(path => path.getTotalLength()));
  assert.ok(original > voice && voice > instrumental && instrumental > 0);
  assert.equal(await vocal.getAttribute("aria-pressed"), "true");
  assert.equal(await position.inputValue(), "1.5");
  const heads = review.locator(".stems-playhead");
  const headPositions = () => heads.evaluateAll(nodes => nodes.map(node => {
    const { left, width } = node.parentElement.getBoundingClientRect();
    return (node.getBoundingClientRect().left - left) / width;
  }));
  for (const value of await headPositions()) assert.ok(Math.abs(value - 0.375) < 0.01);
  await position.press("ArrowRight");
  assert.equal(await position.inputValue(), "1.51");
  await review.getByRole("button", { name: "Reproducir" }).click();
  await page.waitForFunction(() => Number(document.getElementById("stems-position").value) > 1.7);
  const before = Number(await position.inputValue());
  const backing = review.getByRole("button", { name: "Instrumental", exact: true });
  await backing.press("Enter");
  assert.equal(await backing.getAttribute("aria-pressed"), "true");
  assert.ok(Number(await position.inputValue()) >= before);
  await review.getByRole("button", { name: "Pausar" }).click();
  for (const [width, height] of [[900, 620], [1180, 780]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await tool.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    await review.scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${directory}/review-${width}.png` });
  }
  await timeline.press("ArrowLeft");
  await review.waitFor({ state: "detached" });
  assert.equal(await tool.getByRole("button", { name: "Exportar pistas" }).isDisabled(), true);
  assert.equal(await start.inputValue(), "12.3");
  await tool.getByRole("button", { name: "Generar muestra" }).click();
  await review.waitFor();
  await tool.getByRole("button", { name: "Exportar pistas" }).click();
  const calls = await page.evaluate(() => window.stemsCalls);
  assert.deepEqual(calls.filter(call => call.command === "stems_preview").map(call => call.args.start), [12.4, 12.3]);
  assert.ok(calls.some(call => call.command === "stems_release" && call.args.previewId === 22));
  assert.equal(calls.find(call => call.command === "queue_stems").args.previewId, 22);
  assert.deepEqual(errors, []);
  console.log("Three aligned waveforms reflect both channels and share the sample position through seeking and track switches.");
} finally {
  await browser.close();
}
