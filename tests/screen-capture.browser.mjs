// Exercises rendered controls with IPC fixtures, not live Windows capture or audio devices.
import assert from "node:assert/strict";
import { openTool } from "./tool-navigation.mjs";
import { createRequire } from "node:module";
import { mkdir, writeFile } from "node:fs/promises";
import { routeFixture } from "./capture-voice-fixture.mjs";
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }) });
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/", directory = "artifacts/screen-capture"; await mkdir(directory, { recursive: true }); const results = [];
try {
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    const page = await browser.newPage({ viewport: { width, height } }); const errors = [], remote = []; page.on("pageerror", error => errors.push(error.message)); page.on("request", request => { if (!request.url().startsWith(base) && !/^(data:|blob:)/.test(request.url())) remote.push(request.url()); }); await routeFixture(page, base);
    await page.goto(base); await page.locator("[data-page='tools']").click(); await openTool(page, "screenCapture");
    await page.locator("#screen-capture-source").waitFor();
    assert.equal(await page.locator("#screen-capture-microphone").isChecked(), false); assert.equal(await page.locator("#screen-capture-system-audio").isChecked(), false);
    assert.equal(await page.evaluate(() => window.localMediaFixture.calls.some(call => call.cmd === "screen_capture_start" || call.cmd === "screen_capture_preview")), false, "Opening the workspace must not capture pixels or start recording.");
    assert.equal(await page.locator("#screen-capture-form button[type='submit']").isDisabled(), true);
    await page.locator("#screen-capture-source").selectOption("monitor-1"); assert.equal(await page.locator("#screen-capture-region-enabled").isDisabled(), true);
    await page.locator("[data-action='screen-capture-preview']").click(); await page.locator(".capture-preview-ready").waitFor();
    assert.equal(await page.locator(".capture-preview-frame img").evaluate(image => image.naturalWidth > 0), true);
    await page.locator("#screen-capture-region-enabled").check(); assert.equal(await page.locator("#screen-capture-form button[type='submit']").isDisabled(), true);
    for (const [key, value] of [["x", "100"], ["y", "80"], ["width", "320"], ["height", "180"]]) await page.locator(`#screen-capture-region-${key}`).fill(value);
    await page.locator("[data-action='screen-capture-preview']").click(); await page.locator(".capture-preview-ready").waitFor();
    const previews = await page.evaluate(() => window.localMediaFixture.calls.filter(call => call.cmd === "screen_capture_preview").map(call => call.args));
    assert.deepEqual(previews.at(-1).region, { x: 100, y: 80, width: 320, height: 180 });
    await page.locator("[data-action='screen-capture-screenshot']").click(); assert.equal(await page.evaluate(() => window.localMediaFixture.calls.filter(call => call.cmd === "screen_capture_start").length), 0, "A screenshot does not start video or audio capture.");
    await page.locator("#screen-capture-microphone").check(); await page.locator("#screen-capture-microphone-device").selectOption("mic-1"); await page.locator("#screen-capture-system-audio").check(); await page.locator("#screen-capture-system-device").selectOption("system-1");
    assert.match(await page.locator(".capture-audio-controls").innerText(), /también otras ventanas/);
    await page.locator(".capture-preview-stage").scrollIntoViewIfNeeded(); await page.screenshot({ path: `${directory}/source-region-${width}x${height}-fixture.png` });
    await page.locator("#screen-capture-form button[type='submit']").click(); await page.locator(".screen-capture-session.is-active").waitFor();
    const started = await page.evaluate(() => window.localMediaFixture.calls.find(call => call.cmd === "screen_capture_start").args);
    assert.equal(started.previewId, "preview-1"); assert.equal(started.options.systemAudioId, "system-1"); assert.equal(started.options.microphoneId, "mic-1"); assert.equal(started.options.durationSeconds, 1800); assert.equal(started.options.diskLimitMiB, 1024);
    assert.equal(await page.locator("#screen-capture-source").isDisabled(), true);
    await page.locator("[data-page='home']").click();
    const globalStop = page.locator("#capture-indicator button[data-action='screen-capture-stop']"); await globalStop.waitFor(); await globalStop.click();
    await page.locator("[data-page='tools']").click(); await openTool(page, "screenCapture"); await page.locator("[data-action='screen-capture-mode'][data-mode='replay']").click();
    await page.locator("#screen-capture-microphone").uncheck(); await page.locator("#screen-capture-system-audio").uncheck();
    await page.locator("#screen-capture-replay-seconds").selectOption("60"); await page.locator("[data-action='screen-capture-preview']").click(); await page.locator(".capture-preview-ready").waitFor(); await page.locator("#screen-capture-form button[type='submit']").click();
    await page.locator(".screen-capture-session [data-action='screen-capture-save-replay']").waitFor(); assert.match(await page.locator(".screen-capture-session").innerText(), /Tiempo realmente disponible/); assert.match(await page.locator(".screen-capture-session").innerText(), /2 s/);
    await page.locator(".screen-capture-session [data-action='screen-capture-save-replay']").click(); await page.locator(".screen-capture-session").scrollIntoViewIfNeeded(); await page.screenshot({ path: `${directory}/replay-active-${width}x${height}-fixture.png` });
    await page.evaluate(() => { window.localMediaFixture.sourceClosed = true; }); await page.locator("[data-action='screen-capture-stop']").first().click(); await page.getByText("Captura interrumpida", { exact: true }).waitFor(); assert.match(await page.locator(".screen-capture-session").innerText(), /resultado puede ser parcial/);
    await page.locator(".capture-recovery summary").click(); await page.locator("[data-action='screen-capture-recover']").click(); assert.equal(await page.evaluate(() => window.localMediaFixture.calls.filter(call => call.cmd === "queue_screen_capture_recovery").length), 1);
    await page.evaluate(() => { window.localMediaFixture.previewSlow = true; }); await page.locator("[data-action='screen-capture-preview']").click(); await page.locator("[data-action='screen-capture-cancel-preview']").click(); assert.equal(await page.locator(".capture-preview-ready").count(), 0);
    await page.evaluate(() => { window.localMediaFixture.previewError = true; }); await page.locator("[data-action='screen-capture-preview']").click(); await page.getByRole("alert").filter({ hasText: "selected window is no longer available" }).waitFor(); assert.equal(await page.locator("#screen-capture-form button[type='submit']").isDisabled(), true);
    await page.evaluate(() => { window.localMediaFixture.captureAvailable = false; }); await page.locator("[data-action='screen-capture-refresh']").click(); await page.locator(".capture-unavailable").waitFor(); assert.equal(await page.locator("#screen-capture-source").isDisabled(), true); assert.deepEqual(remote, []);
    const dimensions = await page.locator(".screen-capture-workspace").evaluate(element => ({ width: element.clientWidth, scroll: element.scrollWidth, overflow: [...element.querySelectorAll("*")].filter(child => child.getBoundingClientRect().right > element.getBoundingClientRect().right + 1).map(child => ({ tag: child.tagName, class: child.className, width: child.clientWidth, scroll: child.scrollWidth, right: child.getBoundingClientRect().right })) })); assert.ok(dimensions.scroll <= dimensions.width + 1, JSON.stringify(dimensions)); assert.deepEqual(errors, []);
    results.push({ viewport: { width, height }, status: "passed", checks: ["No capture or audio on open; preview before start", "Real PNG fixture and monitor region payload", "Separate microphone and system audio opt-in", "Persistent stop after navigation", "Replay saves only available completed footage", "Interrupted-state recovery and canceled-preview invalidation", "No horizontal overflow"] }); await page.close();
  }
  await writeFile(`${directory}/verification.json`, JSON.stringify({ evidence: "Rendered frontend with IPC fixtures; no live screen or device access certified.", results }, null, 2) + "\n"); console.log(JSON.stringify(results));
} finally { await browser.close(); }
