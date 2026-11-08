import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { openTool } from "./tool-navigation.mjs";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = process.env.NABNAB_UPSCALE_ARTIFACTS || "artifacts/upscale-comparison";
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
  const image = { id: "crop", path: "C:/Fixture/Paisaje.png", name: "Paisaje.png", kind: "image", width: 240, height: 180, size: 9000, addedAt: 1, missing: false, collections: [], tags: [] };
  const snapshot = { settings, jobs: [], library: [image], collections: [], drafts: [], sources: [], rules: [], smartCollections: [], presets: [], audioPlaylists: [] };
  const calls = window.upscaleCalls = [];
  function sample(soft) {
    const canvas = document.createElement("canvas");
    canvas.width = 480;
    canvas.height = 360;
    const ctx = canvas.getContext("2d");
    ctx.filter = soft ? "blur(3px)" : "none";
    ctx.fillStyle = "#e5bd96"; ctx.fillRect(0, 0, 480, 360);
    ctx.fillStyle = "#fdf1ca"; ctx.beginPath(); ctx.arc(275, 100, 48, 0, 2 * Math.PI); ctx.fill();
    for (const [x, y, color] of [[65, 156, "#947080"], [174, 130, "#715d80"], [360, 140, "#796078"], [440, 190, "#484463"]]) {
      ctx.fillStyle = color; ctx.beginPath(); ctx.moveTo(x - 194, 360); ctx.lineTo(x, y); ctx.lineTo(x + 204, 360); ctx.fill();
    }
    ctx.fillStyle = "#303750"; ctx.fillRect(0, 294, 480, 66);
    ctx.fillStyle = "#d59d76";
    for (let x = 14; x < 480; x += 29) { ctx.fillRect(x, 314, 15, 2); ctx.fillRect(x + 6, 333, 11, 2); }
    return Array.from(atob(canvas.toDataURL().split(",")[1]), char => char.charCodeAt(0));
  }
  mockIPC(async (command, args = {}) => {
    calls.push({ command, args: structuredClone(args) });
    if (command === "startup") return { ...snapshot, folder: settings.folder, enginesReady: true, engineError: null };
    if (command === "get_snapshot") return snapshot;
    if (command === "optional_components") return [{ id: "upscale", installed: true, enabled: true, available: true }];
    if (command === "upscale_preview") return { previewId: 17, before: sample(true), after: sample(false), cropWidth: 120, cropHeight: 90, cropX: 30, cropY: 20, dimensions: { width: 240, height: 180, outputWidth: 960, outputHeight: 720 }, peakRgbaBytes: 2937600 };
    if (command === "queue_upscale") return [];
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
  await openTool(page, "upscale");
  const tool = page.locator(".upscale-tool");
  await tool.getByRole("button", { name: "Elegir imágenes" }).click();
  await tool.getByRole("checkbox", { name: "Paisaje.png" }).check();
  await tool.getByRole("button", { name: "Cerrar lista" }).click();
  await tool.getByRole("button", { name: "Generar muestra" }).click();
  const pair = tool.locator(".upscale-pair");
  await pair.waitFor();
  assert.deepEqual(await pair.locator("img").evaluateAll(images => Promise.all(images.map(async img => {
    await img.decode();
    return [img.naturalWidth, img.naturalHeight];
  }))), [[480, 360], [480, 360]]);
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    await tool.locator(".upscale-review").screenshot({ path: path.join(directory, `pair-${width}.png`) });
  }
  const sideBySide = tool.getByRole("button", { name: "Lado a lado", exact: true });
  const wipeMode = tool.getByRole("button", { name: "Deslizador", exact: true });
  assert.equal(await sideBySide.getAttribute("aria-pressed"), "true");
  await wipeMode.press("Enter");
  assert.equal(await wipeMode.getAttribute("aria-pressed"), "true");
  const wipe = tool.locator(".upscale-wipe");
  const slider = wipe.getByRole("slider", { name: "Original visible" });
  assert.equal(await slider.inputValue(), "50");
  await slider.evaluate(control => { window.activeUpscaleSlider = control; });
  await slider.press("End");
  assert.equal(await slider.getAttribute("aria-valuetext"), "100% de original");
  await slider.press("Home");
  assert.equal(await slider.inputValue(), "0");
  await slider.press("ArrowRight");
  assert.equal(await slider.inputValue(), "1");
  assert.ok(await slider.evaluate(control => control === window.activeUpscaleSlider && control === document.activeElement));
  assert.equal(await wipe.evaluate(element => element.style.getPropertyValue("--split")), "1%");
  const range = await slider.boundingBox();
  await page.mouse.click(range.x + range.width * 0.58, range.y + range.height / 2);
  const split = Number(await slider.inputValue());
  assert.ok(split >= 55 && split <= 62, `Pointer selected ${split}%`);
  const clips = await wipe.locator("img").evaluateAll(images => images.map(img => getComputedStyle(img).clipPath));
  assert.match(clips[0], /inset\(/);
  assert.match(clips[1], new RegExp(`${split}%`));
  for (const [width, height] of [[900, 620], [1180, 780]]) {
    await page.setViewportSize({ width, height });
    const layers = await wipe.locator("img").evaluateAll(async images => Promise.all(images.map(async img => {
      await img.decode();
      return { size: [img.naturalWidth, img.naturalHeight], bounds: img.getBoundingClientRect().toJSON() };
    })));
    assert.deepEqual(layers[0], layers[1]);
    assert.deepEqual(layers[0].size, [480, 360]);
    assert.ok(await tool.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    assert.equal(await wipe.locator("label").evaluate(label => getComputedStyle(label).flexDirection), "row");
    await tool.locator(".upscale-review").screenshot({ path: path.join(directory, `slider-${width}.png`) });
  }
  await sideBySide.click();
  await pair.waitFor();
  await wipeMode.click();
  assert.equal(await slider.inputValue(), String(split));
  await tool.getByRole("button", { name: "Ampliar en la cola" }).click();
  const requests = await page.evaluate(() => window.upscaleCalls);
  assert.equal(requests.filter(call => call.command === "upscale_preview").length, 1);
  assert.equal(requests.find(call => call.command === "queue_upscale").args.previewId, 17);
  assert.deepEqual(errors, []);
  console.log("The same crop stays aligned, responds to pointer and keyboard input, and keeps its original queue approval.");
} finally {
  await browser.close();
}
