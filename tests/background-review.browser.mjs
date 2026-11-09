import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { createRequire } from "node:module";
import { openTool } from "./tool-navigation.mjs";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/";
const directory = process.env.NABNAB_BACKGROUND_ARTIFACTS || "artifacts/background-review";
await mkdir(directory, { recursive: true });
const browser = await chromium.launch({ headless: true, channel: "msedge" });
const page = await browser.newPage({ viewport: { width: 900, height: 620 } });
const errors = [];
page.on("pageerror", error => errors.push(error.message));
const setup = `
  import { mockIPC, mockWindows } from "/node_modules/@tauri-apps/api/mocks.js";
  globalThis.isTauri = true;
  mockWindows("main");
  const settings = { folder: "C:/Fixture", language: "es", reducedMotion: true, reducedTransparency: true, sidebarExpanded: true, trayEnabled: false };
  const snapshot = { settings, jobs: [], library: [{ id: "source", name: "Hojas.png", kind: "image", path: "C:/Fixture/Hojas.png" }], collections: [], drafts: [], sources: [], rules: [], smartCollections: [], presets: [], audioPlaylists: [] };
  const calls = window.backgroundCalls = [];
  function sample(mask) {
    const canvas = document.createElement("canvas");
    canvas.width = 640; canvas.height = 420;
    const ctx = canvas.getContext("2d");
    if (mask) { ctx.fillStyle = "#000"; ctx.fillRect(0, 0, canvas.width, canvas.height); }
    ctx.translate(336, 210);
    ctx.lineCap = "round";
    ctx.strokeStyle = mask ? "#fff" : "#617053";
    ctx.lineWidth = 6; ctx.beginPath(); ctx.moveTo(-24, 158); ctx.quadraticCurveTo(-4, 58, 18, -164); ctx.stroke();
    for (let index = 0; index < 8; index++) {
      const side = index % 2 ? -1 : 1;
      ctx.save(); ctx.translate(8 - index * 3, 120 - index * 33); ctx.scale(side, 1);
      ctx.fillStyle = mask ? "#fff" : ["#697b62", "#9ea791", "#566550"][index % 3];
      ctx.beginPath(); ctx.moveTo(0, 0); ctx.bezierCurveTo(30, -61, 95, -58, 132, -47); ctx.bezierCurveTo(110, -20, 60, 10, 0, 0); ctx.fill();
      ctx.restore();
    }
    return [...atob(canvas.toDataURL("image/png").split(",")[1])].map(value => value.charCodeAt(0));
  }
  mockIPC(async (command, args = {}) => {
    calls.push({ command, args: structuredClone(args) });
    if (command === "startup") return { ...snapshot, enginesReady: true };
    if (command === "get_snapshot") return snapshot;
    if (command === "optional_components") return [{ id: "background", installed: true, enabled: true, available: true }];
    if (command === "background_preview") return { previewId: 42, dimensions: { width: 640, height: 420 }, result: sample(false), mask: sample(true) };
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
  await openTool(page, "background");
  const tool = page.locator(".background-tool");
  await tool.getByRole("button", { name: "Elegir imagen" }).click();
  await tool.getByRole("radio", { name: "Hojas.png" }).click();
  await tool.getByRole("button", { name: "Generar muestra" }).click();
  const pair = tool.locator(".background-pair");
  await pair.locator("img").first().evaluate(image => image.decode());
  await tool.locator(".background-review").scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${directory}/fit-900.png` });
  const zoom = tool.getByRole("button", { name: "Ampliar muestra 2×" });
  await zoom.press("Space", { timeout: 3000 });
  assert.equal(await zoom.getAttribute("aria-pressed"), "true");
  const result = tool.getByRole("region", { name: "Resultado ampliado" });
  const mask = tool.getByRole("region", { name: "Máscara ampliada" });
  await zoom.press("Tab");
  assert.equal(await result.evaluate(element => element === document.activeElement), true);
  const position = frame => frame.evaluate(element => [element.scrollLeft, element.scrollTop]);
  assert.deepEqual(await position(result), await position(mask));
  const center = await position(result);
  assert.ok(center.every(value => value > 0));
  await result.press("ArrowRight");
  await page.waitForFunction(({ x }) => {
    const [result, mask] = document.querySelectorAll(".background-pair .asset-preview-image");
    return result.scrollLeft > x && result.scrollLeft === mask.scrollLeft;
  }, { x: center[0] });
  await mask.press("ArrowUp");
  await page.waitForFunction(({ y }) => {
    const [result, mask] = document.querySelectorAll(".background-pair .asset-preview-image");
    return mask.scrollTop < y && result.scrollTop === mask.scrollTop;
  }, { y: center[1] });
  const panned = await position(result);
  await tool.locator("#background-surface").selectOption("light");
  assert.deepEqual(await position(result), panned);
  assert.deepEqual(await position(mask), panned);
  for (const [width, height] of [[900, 620], [1180, 780]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await tool.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    await tool.locator(".background-review").scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${directory}/zoom-${width}.png` });
  }
  await zoom.press("Space");
  assert.equal(await zoom.getAttribute("aria-pressed"), "false");
  assert.equal(await result.count(), 0);
  const overlay = tool.getByRole("button", { name: "Superponer máscara" });
  const frame = tool.locator("#background-result");
  const tint = frame.locator(".background-mask-overlay");
  for (const [width, height] of [[900, 620], [1180, 780]]) {
    await page.setViewportSize({ width, height });
    const pairedWidth = (await frame.boundingBox()).width;
    await overlay.press("Space", { timeout: 3000 });
    assert.equal(await overlay.getAttribute("aria-pressed"), "true");
    assert.ok(await overlay.evaluate(element => element === document.activeElement));
    assert.equal(await tool.locator("#background-mask").count(), 0);
    assert.ok((await frame.boundingBox()).width > pairedWidth * 1.8);
    assert.deepEqual(await tint.boundingBox(), await frame.locator("img").boundingBox());
    assert.ok(await tool.evaluate(element => element.scrollWidth <= element.clientWidth + 1));
    const png = (await frame.screenshot()).toString("base64");
    const [background, leaf] = await page.evaluate(async png => {
      const image = new Image();
      image.src = `data:image/png;base64,${png}`;
      await image.decode();
      const canvas = document.createElement("canvas");
      canvas.width = image.width;
      canvas.height = image.height;
      const ctx = canvas.getContext("2d");
      ctx.drawImage(image, 0, 0);
      return [[0.15, 0.15], [0.4, 0.19]].map(([x, y]) => {
        return [...ctx.getImageData(Math.floor(x * image.width), Math.floor(y * image.height), 1, 1).data];
      });
    }, png);
    assert.ok(background[0] > background[1] + 25, `Removed area has no pink tint: ${background}`);
    assert.ok(leaf[1] > leaf[0], `The mask changed the green subject: ${leaf}`);
    await tool.locator(".background-review").scrollIntoViewIfNeeded();
    await page.screenshot({ path: `${directory}/overlay-${width}.png` });
    await overlay.press("Space");
    assert.equal(await overlay.getAttribute("aria-pressed"), "false");
    assert.equal(await tool.locator("#background-mask").count(), 1);
  }
  await overlay.press("Space");
  await zoom.press("Space");
  const fraction = frame => frame.evaluate(element => element.scrollLeft / (element.scrollWidth - element.clientWidth));
  const beforePan = await fraction(result);
  await result.evaluate(element => element.addEventListener("scrollend", () => { element.dataset.panFinished = "true"; }, { once: true }));
  await result.press("ArrowLeft");
  await tool.locator('#background-result[data-pan-finished="true"]').waitFor({ timeout: 3000 });
  const overlayPan = await fraction(result);
  assert.ok(overlayPan < beforePan);
  await overlay.press("Space");
  const pairedPan = await fraction(result);
  assert.ok(Math.abs(pairedPan - overlayPan) < 0.005, `Pan changed from ${overlayPan} to ${pairedPan}`);
  assert.deepEqual(await position(result), await position(mask));
  await tool.getByRole("button", { name: "Exportar en la cola" }).click();
  const calls = await page.evaluate(() => window.backgroundCalls);
  assert.equal(calls.filter(call => call.command === "background_preview").length, 1);
  assert.deepEqual(calls.find(call => call.command === "queue_background").args.options, {
    format: "png", fill: { kind: "transparent" }, edge: 0, softness: 1,
  });
  assert.equal(calls.find(call => call.command === "queue_background").args.previewId, 42);
  assert.deepEqual(errors, []);
  console.log("Cutout and mask stay aligned in paired and overlay views without changing export settings.");
} finally {
  await browser.close();
}
