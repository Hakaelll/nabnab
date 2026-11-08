import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const base = path.join(root, "artifacts/dubbing");
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "native-run-"));
const output = path.join(data, "output");
await mkdir(output);
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const ffmpeg = path.join(root, "src-tauri/engines/ffmpeg.exe");
const ffprobe = path.join(root, "src-tauri/engines/ffprobe.exe");
const video = path.join(data, "Vídeo original ñ.mp4");
const silent = path.join(data, "Vídeo sin audio.mp4");
const subtitle = path.join(data, "Subtítulos originales.vtt");
const draftId = "19999-1";
const port = Number(process.env.NABNAB_DUBBING_DEBUG_PORT || 9256);
const hash = bytes => createHash("sha256").update(bytes).digest("hex");

await writeFile(subtitle, "WEBVTT\n\nprimero\n00:00:00.500 --> 00:00:02.000\nHola.\n\nsegundo\n00:00:03.000 --> 00:00:05.500\nHasta luego.\n");
execFileSync(ffmpeg, ["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=12:d=6", "-f", "lavfi", "-i", "sine=frequency=290:d=6",
  "-i", subtitle, "-map", "0:v", "-map", "1:a", "-map", "2:s", "-c:v", "libopenh264", "-c:a", "aac", "-c:s", "mov_text", video], { windowsHide: true });
execFileSync(ffmpeg, ["-v", "error", "-i", video, "-map", "0:v:0", "-c:v", "copy", silent], { windowsHide: true });

let child, browser, page, launches = 0, log = "";
async function wait(check, timeout = 40_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const result = await check();
    if (result) return result;
    await new Promise(resolve => setTimeout(resolve, 125));
  }
  throw Error("Timed out waiting for native dubbing.");
}

function rpc(command, args = {}) {
  return page.evaluate(async ({ command, args }) => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    return invoke(command, args);
  }, { command, args });
}

async function launch() {
  const endpoint = `http://127.0.0.1:${port + launches++}`;
  child = spawn(path.join(root, "src-tauri/target/debug/nabnab.exe"), [], { cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, `webview-${launches}`),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${new URL(endpoint).port} --mute-audio` } });
  for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => { log = (log + chunk).slice(-5000); });
  browser = await wait(async () => {
    if (child.exitCode !== null) throw Error(`Native app exited ${child.exitCode}.`);
    try {
      const response = await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(400) });
      if (response.ok) return await chromium.connectOverCDP(endpoint, { timeout: 2000 });
    } catch { /* The WebView is still starting. */ }
    return null;
  });
  page = await wait(() => browser.contexts()[0]?.pages().find(tab => /:1420/.test(tab.url())));
  return wait(async () => { try { return await rpc("get_snapshot"); } catch { return null; } });
}

async function close() {
  try { await page?.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close(); }); } catch {}
  if (child?.exitCode === null) {
    const done = new Promise(resolve => child.once("exit", resolve));
    await Promise.race([done, new Promise(resolve => setTimeout(resolve, 5000))]);
    if (child.exitCode === null) { child.kill(); await done; }
  }
  await browser?.close().catch(() => {});
  page = browser = child = null;
}

async function jobOf(kind, index, state) {
  return wait(async () => {
    const snapshot = await rpc("get_snapshot");
    const job = snapshot.jobs.filter(job => job.source.kind === kind)[index];
    if (job?.state === "error" && state !== "error") throw Error(JSON.stringify(job.error));
    return job?.state === state && job;
  });
}

const probe = file => JSON.parse(execFileSync(ffprobe, ["-v", "error", "-show_streams", "-show_format", "-of", "json", file], { windowsHide: true })).streams;
function rms(bytes, from, to) {
  let sum = 0;
  const start = Math.round(from * 22050), end = Math.round(to * 22050);
  for (let index = start; index < end; index++) sum += (bytes.readInt16LE(44 + index * 2) / 32768) ** 2;
  return Math.sqrt(sum / (end - start));
}

try {
  const initial = await launch();
  await rpc("save_settings", { settings: { ...initial.settings, folder: output, language: "es", reducedMotion: true, trayEnabled: false, closeToTray: false } });
  await rpc("enable_component", { id: "voice", enabled: true });
  const imported = await rpc("import_paths", { paths: [video, silent, subtitle] });
  const item = file => imported.library.find(candidate => candidate.name === path.basename(file));
  const input = { libraryId: item(subtitle).id, trackId: item(subtitle).details.tracks[0].id };
  const { sourceHash, cues } = await rpc("translation_source", { input });
  await close();

  // Seed a completed translation in the isolated repository so this test needs no model download.
  const repository = path.join(data, "state-v1.json");
  const fixture = JSON.parse(await readFile(repository, "utf8"));
  const selection = { input, sourceHash, pair: "es-en" };
  fixture.jobs.push({ id: draftId, runId: 1, clientRequest: "fixture-translation", title: "Traducción de prueba", createdAt: Date.now(), updatedAt: Date.now(), state: "completed",
    source: { kind: "captionTranslation", selection }, folder: output, provider: "Bergamot", thumbnail: null, progress: null, outputs: [], error: null, attempt: 1,
    groupId: null, runNext: false, associations: [] });
  await writeFile(repository, JSON.stringify(fixture));
  const drafts = path.join(data, "subtitle-translations");
  await mkdir(drafts, { recursive: true });
  await writeFile(path.join(drafts, `${draftId}.json`), JSON.stringify({ id: draftId, revision: 1, name: "Subtítulos originales", selection, original: cues,
    translated: ["This sentence is deliberately far too long to fit inside the first short subtitle interval even after speeding it up moderately.", "See you soon."],
    contextualBlocks: 1, edited: [] }));

  const sourceBytes = await readFile(video);
  await launch();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.locator('[data-page="tools"]').click();
  await page.locator('#local-tool-translation').click();
  await page.locator("#translation-draft").selectOption(draftId);
  await page.locator("#dub-open").click();
  await page.locator("#dub-voice").waitFor();
  await page.locator('[data-action="dub-sample"]').click();
  const sample = page.locator("#dub-sample-slot audio");
  await wait(async () => await sample.count() && sample.evaluate(audio => audio.readyState >= 2 && audio.duration > 0));
  await page.locator("#dub-queue-voice").click();
  const tooLong = await jobOf("dubVoice", 0, "error");
  assert.equal(tooLong.error.code, "dubbing_timing");
  assert.deepEqual(tooLong.outputs, []);
  const caches = path.join(data, "voice-cache");
  const cachedBefore = (await readdir(caches)).filter(name => name.endsWith(".wav")).sort();
  assert.equal(cachedBefore.length, 2);
  await page.locator('[data-action="dub-edit-cue"]').click();
  assert.equal(await page.locator("#translation-cue-0").evaluate(node => node === document.activeElement), true);
  await page.locator("#translation-cue-0").fill("Hello.");
  assert.equal(await page.locator("#dub-queue-voice").isDisabled(), true);
  await page.locator("#translation-save").click();
  await wait(async () => (await rpc("translation_read", { draftId })).revision === 2);
  await page.locator("#dub-queue-voice").click();
  const wav = await jobOf("dubVoice", 1, "completed");
  const cachedAfter = (await readdir(caches)).filter(name => name.endsWith(".wav")).sort();
  assert.equal(cachedAfter.length, 3);
  assert.ok(cachedBefore.every(name => cachedAfter.includes(name)));
  const pcm = await readFile(wav.outputs[0]);
  assert.equal(pcm.subarray(0, 4).toString(), "RIFF");
  assert.equal(pcm.readUInt32LE(40), 5.5 * 22050 * 2);
  assert.equal(rms(pcm, 0, 0.49), 0);
  assert.ok(rms(pcm, 0.5, 2) > 0.01);
  assert.equal(rms(pcm, 2, 2.99), 0);
  assert.ok(rms(pcm, 3, 5.5) > 0.01);
  await page.getByRole("button", { name: "Escuchar WAV sincronizado" }).click();
  await wait(() => page.locator("#audio-engine").evaluate(media => media.currentTime > 0));
  await page.locator("#audio-close").click();

  await page.locator("#dub-video").selectOption(item(video).id);
  await page.locator('[data-action="dub-queue-video"]').click();
  const alternate = await jobOf("dubVideo", 0, "completed");
  const audioTracks = probe(alternate.outputs[0]).filter(stream => stream.codec_type === "audio");
  assert.equal(audioTracks.length, 2);
  assert.equal(audioTracks[1].tags.language, "eng");
  assert.equal(probe(alternate.outputs[0]).find(stream => stream.codec_type === "subtitle").codec_name, "subrip");
  await page.locator("#dub-video").selectOption(item(silent).id);
  await page.locator("#dub-mode").selectOption("mix");
  await page.locator('[data-action="dub-queue-video"]').click();
  const failed = await jobOf("dubVideo", 1, "error");
  assert.equal(failed.error.code, "dubbing_no_original");
  assert.equal(failed.source.options.voiceJobId, wav.id);
  assert.deepEqual(failed.outputs, []);
  const bytes = await readFile(wav.outputs[0]);
  assert.equal(hash(bytes), hash(pcm));
  await page.getByRole("button", { name: "Reintentar copia" }).click();
  await wait(async () => {
    const jobs = (await rpc("get_snapshot")).jobs;
    return jobs.find(job => job.id === failed.id && job.runId > failed.runId && job.state === "error");
  });
  await page.locator("#dub-video").selectOption(item(video).id);
  await page.locator('[data-action="dub-queue-video"]').click();
  const mixed = await jobOf("dubVideo", 2, "completed");
  assert.equal(probe(mixed.outputs[0]).filter(stream => stream.codec_type === "audio").length, 1);
  assert.equal(mixed.source.options.voiceJobId, wav.id);
  const tone = execFileSync(ffmpeg, ["-v", "error", "-ss", "2.4", "-i", mixed.outputs[0], "-t", "0.25", "-vn", "-ac", "1", "-ar", "22050", "-f", "s16le", "pipe:1"], { windowsHide: true });
  assert.ok(tone.some(value => value !== 0));
  await page.locator("#dub-mode").selectOption("replace");
  await page.locator('[data-action="dub-queue-video"]').click();
  const replaced = await jobOf("dubVideo", 3, "completed");
  assert.equal(probe(replaced.outputs[0]).filter(stream => stream.codec_type === "audio").length, 1);
  const speechGap = execFileSync(ffmpeg, ["-v", "error", "-ss", "2.4", "-i", replaced.outputs[0], "-t", "0.25", "-vn", "-ac", "1", "-ar", "22050", "-f", "s16le", "pipe:1"], { windowsHide: true });
  assert.ok(speechGap.every(value => value === 0));
  assert.equal(hash(await readFile(video)), hash(sourceBytes));
  assert.deepEqual(errors, []);

  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    const section = page.locator(".dub-expanded");
    await section.evaluate(node => node.scrollIntoView({ block: "start" }));
    assert.ok(await section.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    for (const selector of [".dub-export .field-note", "#translation-export .path"]) {
      await page.locator(selector).evaluate(node => { node.textContent = "C:\\Pruebas\\nabnab\\dubbing\\output"; });
    }
    await page.screenshot({ path: path.join(base, `dubbing-${width}.png`), animations: "disabled" });
  }
  console.log(JSON.stringify({ draftFixture: true, sapiSample: true, timingIssue: tooLong.error.technical, reusedCues: 1, synchronizedSeconds: 5.5,
    alternateTracks: 2, failedMixKeptWav: true, mixedTracks: 1, unchangedOriginal: true, viewports: [1180, 900] }));
} catch (error) {
  console.error(log);
  await page?.screenshot({ path: path.join(base, "failure.png"), animations: "disabled" }).catch(() => {});
  throw error;
} finally {
  await close();
}
