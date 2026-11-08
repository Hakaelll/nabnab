// Real browser audio playback and UI with IPC fixtures; synthesis is tested natively elsewhere.
import assert from "node:assert/strict";
import { openTool } from "./tool-navigation.mjs";
import { createRequire } from "node:module";
import { mkdir, writeFile } from "node:fs/promises";
import { routeFixture } from "./capture-voice-fixture.mjs";
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, ...(process.env.EDGE_EXECUTABLE ? { executablePath: process.env.EDGE_EXECUTABLE } : { channel: "msedge" }) });
const base = process.env.NABNAB_UI_URL || "http://127.0.0.1:1420/", directory = "artifacts/voice"; await mkdir(directory, { recursive: true }); const results = [];
try {
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    const page = await browser.newPage({ viewport: { width, height } }); const errors = [], remote = []; page.on("pageerror", error => errors.push(error.message)); page.on("request", request => { if (!request.url().startsWith(base) && !/^(data:|blob:)/.test(request.url())) remote.push(request.url()); }); await routeFixture(page, base);
    await page.goto(base); await page.locator("[data-page='tools']").click(); await openTool(page, "voice"); await page.locator("#voice-voice option[value='es-1']").waitFor({ state: "attached" });
    assert.equal(await page.evaluate(() => window.localMediaFixture.calls.some(call => call.cmd === "voice_prepare" || call.cmd === "queue_voice" || call.cmd === "voice_sample")), false);
    await page.locator("#voice-text-title").fill("A local speech fixture"); await page.locator("#voice-text").fill('A local paragraph with <script>window.voiceXss=true</script>. A second paragraph stays in the same prepared version.');
    await page.locator("#voice-prepare-form button[type='submit']").click(); await page.locator(".voice-segments").waitFor(); assert.equal(await page.locator(".voice-segment script").count(), 0); assert.match(await page.locator(".voice-segments").innerText(), /<script>/);
    assert.equal(await page.locator("#voice-text").count(), 0, "Prepared input is summarized rather than duplicating long text in the DOM.");
    await page.locator("[data-action='voice-sample']").click(); await page.locator("#voice-audio").waitFor(); assert.match(await page.locator(".voice-sample-text").innerText(), /A local paragraph/);
    await page.locator("[data-action='voice-queue']").click(); await page.locator("[data-action='voice-pause-generation']").waitFor();
    await page.locator("[data-action='voice-listen-segment'][data-index='0']").click();
    await page.waitForFunction(() => { const audio = document.getElementById("voice-audio"); if (!audio || audio.paused) return false; window.persistedVoiceAudio = audio; return true; });
    await page.waitForFunction(() => document.getElementById("voice-audio") === window.persistedVoiceAudio && window.persistedVoiceAudio.currentTime > 1.1, null, { timeout: 10000 });
    assert.equal(await page.locator("[data-voice-segment='0']").getAttribute("aria-current"), "true");
    assert.equal(await page.locator("[data-voice-segment='1'] [data-action='voice-listen-segment']").isDisabled(), true);
    await page.locator(".voice-player").scrollIntoViewIfNeeded(); await page.screenshot({ path: `${directory}/segmented-playback-${width}x${height}-fixture.png` });
    await page.evaluate(() => { window.localMediaFixture.state.settings.folder = "C:/New default destination"; });
    await page.locator("[data-action='voice-pause-generation']").click(); await page.locator("[data-action='voice-resume-generation']").waitFor(); assert.equal(await page.evaluate(() => window.localMediaFixture.calls.filter(call => call.cmd === "voice_pause_job").length), 1);
    assert.equal(await page.locator(".voice-destination strong").innerText(), "C:/Fixture local output", "Paused narration keeps the destination bound when it was queued."); assert.equal(await page.locator("[data-action='voice-destination']").isDisabled(), true); assert.match(await page.locator(".voice-destination").innerText(), /trabajo conserva este destino/);
    await page.locator("[data-action='voice-resume-generation']").click(); assert.equal(await page.locator(".voice-destination strong").innerText(), "C:/Fixture local output"); assert.equal(await page.evaluate(() => window.localMediaFixture.state.jobs[0].folder), "C:/Fixture local output"); await page.locator("[data-action='voice-cancel-generation']").click(); await page.waitForFunction(() => window.localMediaFixture.state.jobs[0].state === "cancelled");
    assert.equal(await page.locator(".voice-destination strong").innerText(), "C:/New default destination"); assert.equal(await page.locator("[data-action='voice-destination']").isDisabled(), false);
    await page.locator("[data-action='voice-edit-source']").click(); await page.locator("#voice-language").selectOption("en-US"); assert.equal(await page.locator(".voice-plan").count(), 0); assert.equal(await page.locator("#voice-voice").inputValue(), "en-1");
    await page.locator("[data-action='voice-source-kind'][data-kind='note']").click(); await page.locator("#voice-library-source").selectOption("article"); await page.locator("#voice-note-source").selectOption("note-1"); await page.locator("#voice-prepare-form button[type='submit']").click(); await page.locator(".voice-plan").waitFor();
    const note = await page.evaluate(() => window.localMediaFixture.calls.filter(call => call.cmd === "voice_prepare").at(-1).args); assert.deepEqual(note.source, { kind: "note", id: "article", annotationId: "note-1" }); assert.equal(note.settings.voiceId, "en-1");
    await page.locator("[data-action='voice-edit-source']").click(); await page.locator("[data-action='voice-source-kind'][data-kind='text']").click(); await page.locator("#voice-text").fill("LONG_FIXTURE"); await page.locator("#voice-prepare-form button[type='submit']").click(); await page.locator(".voice-segment-pagination").waitFor(); assert.equal(await page.locator("[data-voice-segment]").count(), 60); await page.locator("[data-action='voice-segment-page'][data-direction='next']").click(); assert.equal(await page.locator("[data-voice-segment='60']").count(), 1);
    await page.locator("[data-action='voice-edit-source']").click(); await page.evaluate(() => { window.localMediaFixture.voiceSlow = true; }); await page.locator("#voice-prepare-form button[type='submit']").click(); await page.locator("[data-action='voice-cancel-request']").click(); assert.equal(await page.locator(".voice-plan").count(), 0);
    await page.evaluate(() => { window.localMediaFixture.voicePrepareError = true; }); await page.locator("#voice-prepare-form button[type='submit']").click(); await page.getByRole("alert").filter({ hasText: "installed voice is no longer available" }).waitFor();
    await page.evaluate(() => { window.localMediaFixture.voiceEnabled = false; }); await page.locator("[data-action='voice-refresh-status']").click(); await page.locator(".voice-unavailable").waitFor(); assert.equal(await page.locator("#voice-prepare-form button[type='submit']").isDisabled(), true); assert.deepEqual(remote, []);
    const dimensions = await page.locator(".voice-workspace").evaluate(element => ({ width: element.clientWidth, scroll: element.scrollWidth })); assert.ok(dimensions.scroll <= dimensions.width + 1); assert.deepEqual(errors, []);
    results.push({ viewport: { width, height }, status: "passed", checks: ["Explicit preparation and installed voice selection", "Sample and actual browser WAV playback", "Same AudioElement and uninterrupted time across progress rerenders", "Only the playing aligned segment is highlighted", "Generation pause/retry/cancel and immutable voice plan invalidation", "Queued output folder remains bound through pause and resume", "Note source IDs, bounded segment pagination and canceled preparation", "Escaped text and no horizontal overflow"] }); await page.close();
  }
  await writeFile(`${directory}/verification.json`, JSON.stringify({ evidence: "Browser audio playback with generated WAV and IPC fixtures; no real synthesis certified.", results }, null, 2) + "\n"); console.log(JSON.stringify(results));
} finally { await browser.close(); }
