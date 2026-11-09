import assert from "node:assert/strict";
import { openTool } from "./tool-navigation.mjs";
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, readdir, realpath, rm, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(import.meta.dirname, "..");
const base = path.resolve(process.env.NABNAB_DUBBING_ARTIFACTS ?? path.join(root, "artifacts/dubbing"));
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "native-run-"));
const output = path.join(data, "output");
await mkdir(output);
execFileSync("rustc", ["--edition=2021", path.join(root, "tests/kdeconnect-cli-fixture.rs"), "-o", path.join(data, "kdeconnect-cli.exe")], { windowsHide: true });
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const ffmpeg = path.join(root, "src-tauri/engines/ffmpeg.exe");
const ffprobe = path.join(root, "src-tauri/engines/ffprobe.exe");
const video = path.join(data, "Vídeo original ñ.mp4");
const silent = path.join(data, "Vídeo sin audio.mp4");
const instrumental = path.join(output, "Instrumental de prueba.flac");
const vocals = path.join(output, "Voz separada de prueba.flac");
const subtitle = path.join(data, "Subtítulos originales.vtt");
const draftId = "19999-1";
const port = Number(process.env.NABNAB_DUBBING_DEBUG_PORT || 9256);
const hash = bytes => createHash("sha256").update(bytes).digest("hex");

await writeFile(subtitle, "WEBVTT\n\nprimero\n00:00:00.500 --> 00:00:02.000\nHola.\n\nsegundo\n00:00:03.000 --> 00:00:05.500\nNos vemos pronto.\n\ntercero\n00:00:06.000 --> 00:00:08.500\nHasta luego.\n");
execFileSync(ffmpeg, ["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=12:d=9", "-itsoffset", "0.25", "-f", "lavfi", "-i", "sine=frequency=290:d=8.75",
  "-f", "lavfi", "-i", "sine=frequency=710:d=9", "-i", subtitle, "-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3:s",
  "-metadata:s:a:0", "language=spa", "-metadata:s:a:0", "handler_name=Primera mezcla",
  "-metadata:s:a:1", "language=eng", "-metadata:s:a:1", "handler_name=Segunda mezcla",
  "-c:v", "libopenh264", "-c:a", "aac", "-c:s", "mov_text", video], { windowsHide: true });
execFileSync(ffmpeg, ["-v", "error", "-i", video, "-map", "0:v:0", "-c:v", "copy", silent], { windowsHide: true });
for (const [file, frequency] of [[instrumental, 540], [vocals, 610]]) {
  execFileSync(ffmpeg, ["-v", "error", "-f", "lavfi", "-i", `sine=frequency=${frequency}:duration=8.75`, "-c:a", "flac", file], { windowsHide: true });
}

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
  child = spawn(process.env.NABNAB_DUBBING_TEST_EXE ?? path.join(root, "src-tauri/target/debug/nabnab.exe"), [], { cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, Path: `${data}${path.delimiter}${process.env.Path ?? process.env.PATH ?? ""}`,
      NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, `webview-${launches}`),
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

async function openDubbingFromPalette() {
  await page.keyboard.press("Control+k");
  await page.locator("#palette-query").fill("crear doblaje");
  await page.getByRole("option", { name: /Crear doblaje/ }).waitFor({ timeout: 5000 });
  await page.keyboard.press("Enter");
  await page.getByRole("heading", { name: "Crear doblaje", exact: true }).waitFor();
}

const probe = file => JSON.parse(execFileSync(ffprobe, ["-v", "error", "-show_streams", "-show_format", "-of", "json", file], { windowsHide: true })).streams;
const gapSamples = (file, start = "2.4", length = "0.25") => execFileSync(ffmpeg, ["-v", "error", "-ss", start, "-i", file, "-t", length, "-vn", "-ac", "1", "-ar", "22050", "-f", "s16le", "pipe:1"], { windowsHide: true });
function frequencyStrength(samples, hz) {
  let real = 0, imaginary = 0;
  for (let offset = 0; offset + 1 < samples.length; offset += 2) {
    const phase = Math.PI * hz * offset / 22050;
    const sample = samples.readInt16LE(offset);
    real += Math.cos(phase) * sample;
    imaginary += Math.sin(phase) * sample;
  }
  return Math.hypot(real, imaginary);
}
async function shares() {
  const log = await readFile(path.join(data, "cli-arguments.jsonl"), "utf8").catch(error => {
    if (error.code === "ENOENT") return "";
    throw error;
  });
  return log.split("\n").filter(line => line.trim()).map(line => JSON.parse(line));
}

async function shareOutput(job) {
  const outputPath = job.outputs[0];
  const name = path.basename(outputPath);
  const trigger = page.getByRole("button", { name: `Enviar a dispositivo · ${name}`, exact: true });
  const previous = await shares();
  await trigger.press("Enter");
  const modal = page.locator(".device-share[open]");
  await modal.locator('#share-device option[value="paired-device-123"]').waitFor({ state: "attached" });
  await modal.locator("#share-device").selectOption("paired-device-123");
  await modal.locator("#share-review").click();
  await modal.locator("#share-send").waitFor();
  assert.deepEqual(await modal.locator(".device-share-review li > span").allTextContents(), [name]);
  assert.deepEqual(await shares(), previous);
  if (name.endsWith(".mkv")) await modal.screenshot({ path: path.join(data, "dubbing-share.png"), animations: "disabled" });
  await modal.locator("#share-send").click();
  await modal.getByText("Entregado a KDE Connect", { exact: true }).waitFor();
  const submitted = await shares();
  assert.equal(submitted.length, previous.length + 1);
  const [device, file] = submitted.at(-1);
  assert.equal(device, "--device=paired-device-123");
  assert.ok(file.startsWith("--share=file:"));
  assert.equal(path.toNamespacedPath(fileURLToPath(file.slice("--share=".length))), path.toNamespacedPath(outputPath));
  await modal.locator("#share-close").click();
  await modal.waitFor({ state: "hidden" });
  assert.equal(await trigger.evaluate(button => button === document.activeElement), true);
}

function rms(bytes, from, to) {
  let sum = 0;
  const start = Math.round(from * 22050), end = Math.round(to * 22050);
  for (let index = start; index < end; index++) sum += (bytes.readInt16LE(44 + index * 2) / 32768) ** 2;
  return Math.sqrt(sum / (end - start));
}

try {
  const initial = await launch();
  await rpc("save_settings", { settings: { ...initial.settings, folder: output, language: "es", reducedMotion: true, trayEnabled: false, closeToTray: false } });
  await page.locator('[data-page="tools"]').click();
  await openTool(page, "dubbing");
  await page.locator(".dubbing-empty").waitFor();
  await page.locator('.dubbing-empty [data-action="translation-edit-draft"]').click();
  await page.locator("#translation-setup").waitFor();
  await rpc("enable_component", { id: "voice", enabled: true });
  const imported = await rpc("import_paths", { paths: [video, silent, subtitle, instrumental, vocals] });
  const item = file => imported.library.find(candidate => candidate.name === path.basename(file));
  const input = { libraryId: item(subtitle).id, trackId: item(subtitle).details.tracks[0].id };
  const { sourceHash, cues } = await rpc("translation_source", { input });
  await close();

  // Seed a completed translation in the isolated repository so this test needs no model download.
  const repository = path.join(data, "state-v1.json");
  const fixture = JSON.parse(await readFile(repository, "utf8"));
  const sourceBytes = await readFile(video);
  const instrumentalBytes = await readFile(instrumental);
  const stemsId = "19998-1";
  const stems = [[instrumental, "Instrumental"], [vocals, "Vocals"]].map(([file, role]) => {
    const stem = fixture.library.find(candidate => candidate.id === item(file).id);
    stem.parentId = item(video).id;
    stem.details.stems = { setId: stemsId, role, model: "Spleeter" };
    return stem.path;
  });
  fixture.jobs.push({ id: stemsId, runId: 1, clientRequest: "fixture-stems", title: "Separación de prueba", createdAt: 19998, updatedAt: 19998, state: "completed",
    source: { kind: "stems", selection: { libraryId: item(video).id, inputHash: hash(sourceBytes) } },
    folder: output, provider: "Spleeter", thumbnail: null, progress: null, outputs: stems, error: null, attempt: 1,
    groupId: null, runNext: false, associations: [] });
  const selection = { input, sourceHash, pair: "es-en" };
  fixture.jobs.push({ id: draftId, runId: 1, clientRequest: "fixture-translation", title: "Traducción de prueba", createdAt: Date.now(), updatedAt: Date.now(), state: "completed",
    source: { kind: "captionTranslation", selection }, folder: output, provider: "Bergamot", thumbnail: null, progress: null, outputs: [], error: null, attempt: 1,
    groupId: null, runNext: false, associations: [] });
  await writeFile(repository, JSON.stringify(fixture));
  const drafts = path.join(data, "subtitle-translations");
  await mkdir(drafts, { recursive: true });
  await writeFile(path.join(drafts, `${draftId}.json`), JSON.stringify({ id: draftId, revision: 1, name: "Subtítulos originales", selection, original: cues,
    translated: ["This sentence is deliberately far too long to fit inside the first short subtitle interval even after speeding it up moderately.", "See you soon.",
      "The final sentence is deliberately long as well, so the test can check how a second timing warning opens its own line in the editor."],
    contextualBlocks: 1, edited: [] }));

  await launch();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await openDubbingFromPalette();
  const picker = page.locator("#translation-draft");
  await picker.focus();
  await picker.selectOption(draftId);
  await page.locator("#dub-voice").waitFor();
  assert.equal(await picker.evaluate(element => element === document.activeElement), true);
  await page.locator('[data-action="dub-sample"]').click();
  const sample = page.locator("#dub-sample-slot audio");
  await wait(async () => await sample.count() && sample.evaluate(audio => audio.readyState >= 2 && audio.duration > 0));
  await page.locator("#dub-queue-voice").click();
  const tooLong = await jobOf("dubVoice", 0, "error");
  assert.equal(tooLong.error.code, "dubbing_timing");
  assert.equal(tooLong.error.technical, "DUB_TIMING 1, 3");
  assert.deepEqual(tooLong.outputs, []);
  const caches = path.join(data, "voice-cache");
  const cachedBefore = (await readdir(caches)).filter(name => name.endsWith(".wav")).sort();
  assert.equal(cachedBefore.length, 3);
  await page.getByRole("button", { name: "Editar segmento 3", exact: true }).press("Enter");
  await page.waitForFunction(() => document.activeElement?.id === "translation-cue-2");
  await page.locator("#translation-cue-2").fill("Goodbye.");
  await page.getByRole("button", { name: "Siguiente aviso de doblaje", exact: true }).press("Enter");
  assert.equal(await page.locator("#translation-cue-0").evaluate(node => node === document.activeElement), true);
  await page.locator("#translation-cue-0").fill("Hello.");
  await openDubbingFromPalette();
  await page.getByText("Guarda la edición para generar la voz con esos cambios.").waitFor();
  assert.equal(await page.locator("#translation-draft").isDisabled(), true);
  assert.equal(await page.locator("#dub-queue-voice").isDisabled(), true);
  await page.getByRole("button", { name: "Editar traducción", exact: true }).click();
  const cueSearch = page.getByRole("searchbox", { name: "Buscar segmentos", exact: true });
  await cueSearch.fill("tercero");
  assert.equal(await page.locator("#translation-cue-0").count(), 0);
  assert.equal(await page.locator("#translation-cue-2").inputValue(), "Goodbye.");
  await cueSearch.fill("00:00:00.500");
  assert.equal(await page.locator("#translation-cue-0").inputValue(), "Hello.");
  assert.equal(await page.locator("#translation-cue-2").count(), 0);
  await page.getByRole("button", { name: "Siguiente aviso de doblaje", exact: true }).press("Enter");
  await page.waitForFunction(() => document.activeElement?.id === "translation-cue-2");
  await cueSearch.fill("00:00:00.500");
  await page.locator("#translation-save").click();
  await wait(async () => (await rpc("translation_read", { draftId })).revision === 2);
  const reviewed = await rpc("translation_read", { draftId });
  assert.deepEqual(reviewed.original, cues);
  assert.deepEqual(reviewed.translated, ["Hello.", "See you soon.", "Goodbye."]);
  await page.getByRole("button", { name: "Volver a la muestra del segmento 3", exact: true }).press("Enter");
  assert.equal(await page.locator("#translation-draft").inputValue(), draftId);
  assert.equal(await page.locator("#dub-cue").inputValue(), "3");
  await page.waitForFunction(() => document.activeElement?.id === "dub-play-sample");
  await page.locator("#dub-play-sample").press("Enter");
  await page.locator(".dub-sample").getByText("Goodbye.", { exact: true }).waitFor();
  await wait(async () => await sample.count() && sample.evaluate(audio => audio.readyState >= 2 && audio.duration > 0));
  await page.locator("#dub-queue-voice").click();
  const wav = await jobOf("dubVoice", 1, "completed");
  const cachedAfter = (await readdir(caches)).filter(name => name.endsWith(".wav")).sort();
  assert.equal(cachedAfter.length, 5);
  assert.ok(cachedBefore.every(name => cachedAfter.includes(name)));
  const pcm = await readFile(wav.outputs[0]);
  assert.equal(pcm.subarray(0, 4).toString(), "RIFF");
  assert.equal(pcm.readUInt32LE(40), 8.5 * 22050 * 2);
  assert.equal(rms(pcm, 0, 0.49), 0);
  assert.ok(rms(pcm, 0.5, 2) > 0.01);
  assert.equal(rms(pcm, 2, 2.99), 0);
  assert.ok(rms(pcm, 3, 5.5) > 0.01);
  assert.equal(rms(pcm, 5.5, 5.99), 0);
  assert.ok(rms(pcm, 6, 8.5) > 0.01);
  await page.getByRole("button", { name: "Escuchar WAV", exact: true }).click();
  await wait(() => page.locator("#audio-engine").evaluate(media => media.currentTime > 0));
  await page.locator("#audio-close").click();

  await page.locator("#dub-video").selectOption(item(video).id);
  await page.locator('[data-action="dub-queue-video"]').click();
  const alternate = await jobOf("dubVideo", 0, "completed");
  const audioTracks = probe(alternate.outputs[0]).filter(stream => stream.codec_type === "audio");
  assert.equal(audioTracks.length, 3);
  assert.equal(audioTracks[2].tags.language, "eng");
  assert.equal(probe(alternate.outputs[0]).find(stream => stream.codec_type === "subtitle").codec_name, "subrip");
  await page.locator("#dub-video").selectOption(item(silent).id);
  await page.locator("#dub-mode").selectOption("mix");
  const reviewAudio = page.getByRole("button", { name: "Revisar pistas", exact: true });
  await reviewAudio.click({ timeout: 5000 });
  await page.getByText("Este vídeo no tiene pistas de audio.", { exact: true }).waitFor();
  assert.equal(await page.locator('[data-action="dub-queue-video"]').isDisabled(), true);
  await rpc("queue_dub_video", { options: { voiceJobId: wav.id, videoId: item(silent).id, mode: "mix" }, clientRequest: "legacy-silent-mix" });
  const failed = await jobOf("dubVideo", 1, "error");
  assert.equal(failed.error.code, "dubbing_no_original");
  assert.equal(failed.source.options.voiceJobId, wav.id);
  assert.deepEqual(failed.outputs, []);
  assert.equal(await page.locator(".dub-expanded .translation-output").filter({ hasText: "Copia de vídeo" }).count(), 0);
  const bytes = await readFile(wav.outputs[0]);
  assert.equal(hash(bytes), hash(pcm));
  await rpc("job_action", { ids: [failed.id], action: "retry" });
  await wait(async () => {
    const jobs = (await rpc("get_snapshot")).jobs;
    return jobs.find(job => job.id === failed.id && job.runId > failed.runId && job.state === "error");
  });
  await page.locator("#dub-video").selectOption(item(video).id);
  await reviewAudio.press("Enter");
  const track = page.getByLabel("Pista original", { exact: true });
  await track.waitFor();
  assert.equal(await track.inputValue(), "");
  assert.equal(await track.evaluate(element => document.activeElement === element), true);
  assert.equal(await page.locator('[data-action="dub-queue-video"]').isDisabled(), true);
  const secondTrack = track.getByRole("option", { name: /Segunda mezcla.*eng/ });
  const originalAudioId = await secondTrack.getAttribute("value");
  await track.selectOption(originalAudioId);
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    const options = page.locator("#dub-video-controls");
    await options.evaluate(element => element.scrollIntoView({ block: "center" }));
    assert.ok(await options.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await page.screenshot({ path: path.join(data, `dubbing-mix-${width}.png`), animations: "disabled" });
  }
  await page.locator('[data-action="dub-queue-video"]').click();
  const mixed = await jobOf("dubVideo", 2, "completed");
  assert.equal(probe(mixed.outputs[0]).filter(stream => stream.codec_type === "audio").length, 1);
  assert.equal(mixed.source.options.voiceJobId, wav.id);
  assert.equal(mixed.source.options.originalAudioId, originalAudioId);
  const tone = gapSamples(mixed.outputs[0]);
  assert.ok(frequencyStrength(tone, 710) > 10 * frequencyStrength(tone, 290));
  await page.locator("#dub-mode").selectOption("replace");
  await page.locator('[data-action="dub-queue-video"]').click();
  const replaced = await jobOf("dubVideo", 3, "completed");
  assert.equal(probe(replaced.outputs[0]).filter(stream => stream.codec_type === "audio").length, 1);
  const speechGap = gapSamples(replaced.outputs[0]);
  assert.ok(speechGap.every(value => value === 0));
  await rpc("queue_dub_video", { options: { voiceJobId: wav.id, videoId: item(video).id, mode: "mix", originalAudioId: "e".repeat(64) },
    clientRequest: "changed-dubbing-track" });
  const changedTrack = await jobOf("dubVideo", 4, "error");
  assert.equal(changedTrack.error.code, "dubbing_audio_changed");
  assert.deepEqual(changedTrack.outputs, []);
  assert.equal(hash(await readFile(wav.outputs[0])), hash(pcm));
  await page.locator("#dub-mode").selectOption("instrumental", { timeout: 5000 });
  const background = page.getByLabel("Instrumental de Biblioteca", { exact: true });
  await background.waitFor();
  assert.equal(await background.getByRole("option", { name: /Voz separada/ }).count(), 0);
  const options = { voiceJobId: wav.id, videoId: item(video).id, mode: "instrumental", instrumentalId: item(instrumental).id };
  for (const invalid of [{ ...options, instrumentalId: item(vocals).id }, { ...options, videoId: item(silent).id }]) {
    await assert.rejects(rpc("queue_dub_video", { options: invalid, clientRequest: "foreign-stems" }), /instrumental/i);
  }
  await page.locator("#dub-video").selectOption(item(silent).id);
  await page.getByRole("button", { name: "Abrir Separar pistas", exact: true }).waitFor();
  assert.equal(await page.locator('[data-action="dub-queue-video"]').isDisabled(), true);
  await page.locator("#dub-video").selectOption(item(video).id);
  await background.selectOption(item(instrumental).id);
  await page.getByRole("button", { name: "Escuchar instrumental", exact: true }).click();
  await wait(() => page.locator("#audio-engine").evaluate((media, id) => media.dataset.localPlayer === id && media.currentTime > 0, item(instrumental).id));
  await page.locator("#audio-close").click();
  assert.equal(await background.inputValue(), item(instrumental).id);
  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    const panel = page.locator("#dub-video-controls");
    await panel.evaluate(element => element.scrollIntoView({ block: "center" }));
    assert.ok(await panel.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await page.screenshot({ path: path.join(data, `dubbing-instrumental-${width}.png`), animations: "disabled" });
  }
  await page.locator('[data-action="dub-queue-video"]').click();
  const stemMix = await jobOf("dubVideo", 5, "completed");
  assert.deepEqual(stemMix.source.options, options);
  assert.match(probe(stemMix.outputs[0]).find(stream => stream.codec_type === "audio").tags.title, /separated instrumental/);
  assert.ok(gapSamples(stemMix.outputs[0], "0.04", "0.10").every(byte => byte === 0));
  assert.ok(frequencyStrength(gapSamples(stemMix.outputs[0], "0.32", "0.10"), 540) > 100000);
  const instrumentalTone = gapSamples(stemMix.outputs[0]);
  for (const frequency of [290, 610, 710]) {
    assert.ok(frequencyStrength(instrumentalTone, 540) > 10 * frequencyStrength(instrumentalTone, frequency));
  }
  await writeFile(video, Buffer.concat([sourceBytes, Buffer.from("changed video fixture")]));
  try {
    await rpc("queue_dub_video", { options, clientRequest: "stale-stems" });
    const staleStem = await jobOf("dubVideo", 6, "error");
    assert.equal(staleStem.error.code, "dubbing_instrumental_changed");
    assert.deepEqual(staleStem.outputs, []);
  } finally {
    await writeFile(video, sourceBytes);
  }
  assert.equal(hash(await readFile(wav.outputs[0])), hash(pcm));
  assert.deepEqual(await readFile(instrumental), instrumentalBytes);
  assert.equal(hash(await readFile(video)), hash(sourceBytes));
  await page.locator("#dub-mode").selectOption("replace");
  await shareOutput(wav);
  await shareOutput(replaced);
  await page.getByRole("button", { name: "Editar traducción", exact: true }).click();
  assert.equal(await cueSearch.inputValue(), "00:00:00.500");
  assert.equal(await page.locator(".translation-cues textarea").count(), 1);
  await page.locator("#translation-export-button").click();
  const exported = await jobOf("captionTranslationExport", 0, "completed");
  assert.equal(await readFile(exported.outputs[0], "utf8"),
    "WEBVTT\n\nprimero\n00:00:00.500 --> 00:00:02.000\nHello.\n\nsegundo\n00:00:03.000 --> 00:00:05.500\nSee you soon.\n\ntercero\n00:00:06.000 --> 00:00:08.500\nGoodbye.\n\n");
  await shareOutput(exported);
  assert.deepEqual(errors, []);
  await page.locator("#dub-open").click();
  await page.locator("#dub-voice:enabled").waitFor();

  for (const [width, height] of [[1180, 780], [900, 620]]) {
    await page.setViewportSize({ width, height });
    const section = page.locator(".dubbing-tool");
    await section.evaluate(node => node.scrollIntoView({ block: "start" }));
    assert.ok(await section.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await page.locator(".dub-export .field-note").evaluate(node => { node.textContent = "C:\\Pruebas\\nabnab\\dubbing\\output"; });
    await page.screenshot({ path: path.join(data, `dubbing-${width}.png`), animations: "disabled" });
    if (width === 1180) await section.screenshot({ path: path.join(data, "dubbing-results.png"), animations: "disabled" });
  }
  console.log(JSON.stringify({ draftFixture: true, catalogAndPalette: true, editsPreservedBetweenTools: true, sapiSample: true, returnedToEditedCue: 3, timingIssue: tooLong.error.technical, reusedCues: 1, synchronizedSeconds: 8.5,
    alternateTracks: 3, failedMixKeptWav: true, selectedMixHz: 710, separatedInstrumentalHz: 540, staleInstrumentalRejected: true,
    mixedTracks: 1, unchangedOriginal: true, shares: 3, viewports: [1180, 900] }));
} catch (error) {
  console.error(log);
  await page?.screenshot({ path: path.join(data, "failure.png"), animations: "disabled" }).catch(() => {});
  throw error;
} finally {
  await close();
  for (let launch = 1; launch <= launches; launch++) {
    const profile = path.join(data, `webview-${launch}`);
    try {
      assert.equal(path.relative(profile, await realpath(profile)), "", "Refusing to remove a redirected WebView2 profile.");
      await rm(profile, { recursive: true, force: true, maxRetries: 8, retryDelay: 150 });
    } catch (error) {
      if (error.code !== "ENOENT") {
        console.error(`Could not clean up the WebView2 test profile at ${profile}:`, error);
        process.exitCode = 1;
      }
    }
  }
}
