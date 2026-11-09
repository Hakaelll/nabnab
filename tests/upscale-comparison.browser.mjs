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
  const image = { id: "crop", path: "C:/Fixture/Paisaje.png", name: "Paisaje.png", kind: "image", width: null, height: null, size: 9000, addedAt: 1, missing: false, collections: [], tags: [] };
  const portrait = { ...image, id: "portrait", name: "Retrato.png", path: "C:/Fixture/Retrato.png" };
  const batch = [
    { id: image.id, width: 240, height: 180, outputWidth: 960, outputHeight: 720, rgbaBytes: 2937600 },
    { id: portrait.id, width: 360, height: 540, outputWidth: 1440, outputHeight: 2160, rgbaBytes: 13219200 },
  ];
  const snapshot = { settings, jobs: [], library: [image, portrait], collections: [], drafts: [], sources: [], rules: [], smartCollections: [], presets: [], audioPlaylists: [] };
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
    if (command === "image_assets_inspect") return { images: [{ id: image.id, name: image.name, width: 240, height: 180 }] };
    if (command === "thumbnail" && args.stored) return "data:image/png;base64," + btoa(String.fromCharCode(...sample(false)));
    if (command === "upscale_inspect") return batch;
    if (command === "upscale_preview") return { previewId: 17, before: sample(true), after: sample(false), cropWidth: 120, cropHeight: 90, cropX: 30, cropY: 20, batch };
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
  await tool.getByRole("checkbox", { name: "Retrato.png" }).check();
  await tool.getByRole("button", { name: "Cerrar lista" }).click();
  await page.setViewportSize({ width: 900, height: 620 });
  const review = tool.locator(".upscale-review");
  await review.evaluate(node => node.scrollIntoView({ block: "start" }));
  await page.screenshot({ path: path.join(directory, "crop-overview-before.png") });
  const batchReview = tool.getByRole("region", { name: "Dimensiones del lote" });
  await batchReview.getByRole("button", { name: "Revisar dimensiones" }).press("Enter", { timeout: 3500 });
  await batchReview.getByText("1440 × 2160 px", { exact: false }).waitFor();
  assert.equal(await batchReview.getByRole("heading").evaluate(heading => heading === document.activeElement), true);
  assert.deepEqual(await batchReview.locator("li strong").allTextContents(), ["Paisaje.png", "Retrato.png"]);
  assert.match(await batchReview.innerText(), /12,6 MiB/);
  assert.equal(await tool.getByRole("button", { name: "Ampliar en la cola" }).isDisabled(), true);
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await batchReview.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await batchReview.screenshot({ path: path.join(directory, `batch-dimensions-${width}.png`) });
  }
  const map = tool.getByRole("button", { name: "Colocar recorte en la imagen" });
  await map.waitFor({ timeout: 4500 });
  await map.locator("img").evaluate(img => img.decode());
  const horizontal = tool.getByRole("slider", { name: /Posición horizontal/ });
  const vertical = tool.getByRole("slider", { name: /Posición vertical/ });
  await map.click({ position: { x: 8, y: 8 } });
  assert.equal(await horizontal.inputValue(), "0");
  assert.equal(await vertical.inputValue(), "0");
  await horizontal.press("ArrowRight");
  assert.equal(await horizontal.inputValue(), "1");
  assert.equal(await batchReview.locator("li").count(), 2);
  assert.equal(await horizontal.evaluate(control => control === document.activeElement), true);
  await map.press("Enter");
  assert.equal(await horizontal.inputValue(), "50");
  assert.equal(await vertical.inputValue(), "50");
  const overview = await map.boundingBox();
  await map.click({ position: { x: overview.width - 8, y: overview.height - 8 } });
  assert.equal(await horizontal.inputValue(), "100");
  assert.equal(await vertical.inputValue(), "100");
  const crop = await map.locator(".upscale-crop-window").boundingBox();
  const repositioned = await map.boundingBox();
  assert.ok(Math.abs(crop.x + crop.width - repositioned.x - repositioned.width) < 2);
  assert.ok(Math.abs(crop.y + crop.height - repositioned.y - repositioned.height) < 2);
  for (const [width, height] of [[900, 620], [1180, 780]]) {
    await page.setViewportSize({ width, height });
    await review.evaluate(node => node.scrollIntoView({ block: "start" }));
    assert.ok(await review.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await page.screenshot({ path: path.join(directory, `crop-overview-${width}.png`) });
  }
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
  assert.deepEqual(requests.find(call => call.command === "upscale_inspect").args.ids, ["crop", "portrait"]);
  assert.deepEqual(requests.find(call => call.command === "upscale_preview").args.anchor, {x:100,y:100});
  assert.equal(requests.find(call => call.command === "queue_upscale").args.previewId, 17);
  assert.deepEqual(errors, []);
  console.log("The same crop stays aligned, responds to pointer and keyboard input, and keeps its original queue approval.");
} finally {
  await browser.close();
}
