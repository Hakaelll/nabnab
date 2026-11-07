import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const base = path.join(root, "artifacts/playback");
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "native-run-"));
const output = path.join(data, "output");
await mkdir(output);
const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const port = process.env.NABNAB_PLAYBACK_DEBUG_PORT || 9254;
const endpoint = `http://127.0.0.1:${port}`;
const ffmpeg = path.join(root, "src-tauri/engines/ffmpeg.exe");
const ffprobe = path.join(root, "src-tauri/engines/ffprobe.exe");
const video = path.join(data, "01 · Vídeo vertical ñ.mp4");
const music = path.join(data, "02 · Canción con portada.mp3");
const webm = path.join(data, "03 · Plano ancho.webm");
const mkv = path.join(data, "04 · Plano ancho.mkv");
const broken = path.join(data, "05 · Archivo dañado.mp4");
const picture = path.join(data, "cover.png");
const captions = path.join(data, "Subtítulos ñ.vtt");

function encode(...args) {
  return execFileSync(ffmpeg, ["-hide_banner", "-v", "error", ...args], { windowsHide: true });
}
encode("-f", "lavfi", "-i", "testsrc2=size=360x640:rate=24:duration=22",
  "-f", "lavfi", "-i", "sine=frequency=440:duration=22", "-c:v", "libopenh264", "-c:a", "aac", "-shortest", video);
encode("-f", "lavfi", "-i", "color=c=#354551:s=100x100", "-frames:v", "1", picture);
encode("-f", "lavfi", "-i", "sine=frequency=330:duration=16", "-i", picture,
  "-map", "0:a", "-map", "1:v", "-c:a", "libmp3lame", "-c:v", "copy", "-disposition:v", "attached_pic",
  "-metadata", "title=Señal para la cola", music);
encode("-f", "lavfi", "-i", "testsrc2=size=720x248:rate=20:duration=6", "-f", "lavfi",
  "-i", "sine=frequency=550:duration=6", "-c:v", "libvpx-vp9", "-deadline", "realtime", "-c:a", "libopus", "-shortest", webm);
encode("-i", webm, "-c", "copy", mkv);
await writeFile(broken, "Archivo de prueba sin contenedor de vídeo.");
await writeFile(captions, "WEBVTT\n\n00:00:04.000 --> 00:00:09.000\nTexto local &lt;b&gt;seguro&lt;/b&gt;\n");

let child, browser, page, log = "";
async function wait(check, timeout = 30_000) {
  const until = Date.now() + timeout;
  while (Date.now() < until) {
    const value = await check();
    if (value) return value;
    await new Promise(resolve => setTimeout(resolve, 90));
  }
  throw Error("Timed out waiting for playback.");
}

function rpc(command, args = {}) {
  return page.evaluate(async ({ command, args }) => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    return invoke(command, args);
  }, { command, args });
}

async function launch() {
  child = spawn(path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
    cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --mute-audio` },
  });
  for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => { log = (log + chunk).slice(-6000); });
  await wait(async () => {
    if (child.exitCode !== null) throw Error(`Native app exited ${child.exitCode}`);
    try { return (await fetch(`${endpoint}/json/version`)).ok; } catch { return false; }
  });
  browser = await chromium.connectOverCDP(endpoint);
  page = await wait(() => browser.contexts()[0]?.pages().find(tab => /:1420/.test(tab.url())));
  return wait(async () => { try { return await rpc("get_snapshot"); } catch { return null; } });
}

async function shutdown() {
  try { await page?.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close(); }); } catch {}
  if (child?.exitCode === null) {
    await Promise.race([new Promise(resolve => child.once("exit", resolve)), new Promise(resolve => setTimeout(resolve, 4000))]);
    if (child.exitCode === null) {
      child.kill();
      await new Promise(resolve => child.once("exit", resolve));
    }
  }
  await browser?.close().catch(() => {});
  child = browser = page = null;
}

function playing() {
  return page.locator("#audio-engine").evaluate(media => ({
    time: media.currentTime, duration: media.duration, paused: media.paused, error: media.error?.code,
    width: media.videoWidth, height: media.videoHeight, src: media.currentSrc, muted: media.muted,
    volume: media.volume, rate: media.playbackRate, id: media.dataset.localPlayer, fit: getComputedStyle(media).objectFit,
  }));
}

const stage = () => page.locator("#playback-stage");
const play = id => page.locator(`.media-play[data-id="${id}"]`).click();
const clickStage = action => stage().locator(`[data-stage-action="${action}"]`).click();
const clip = file => JSON.parse(execFileSync(ffprobe, ["-v", "error", "-show_streams", "-of", "json", file], { windowsHide: true })).streams;

try {
  const initial = await launch();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await rpc("save_settings", { settings: { ...initial.settings, folder: output, language: "es", reducedMotion: true, trayEnabled: false, closeToTray: false } });
  const imported = await rpc("import_paths", { paths: [video, music, webm, mkv, broken] });
  const item = file => imported.library.find(candidate => candidate.name === path.basename(file));
  const videoId = item(video).id, musicId = item(music).id;
  assert.equal(item(music).kind, "audio");
  assert.equal(item(music).details.audio.embeddedCover, true);
  assert.equal(item(video).videoCodec, "h264");
  assert.equal(item(video).audioCodec, "aac");
  await rpc("subtitle_import", { id: videoId, language: "es", path: captions });
  await rpc("library_playback", { id: videoId, position: 5.25 });
  await page.reload();
  await page.locator('[data-page="library"]').click();
  await play(videoId);
  await stage().locator("#stage-resume").waitFor();
  const original = await page.locator("#audio-engine").elementHandle();
  assert.equal((await playing()).paused, true);
  await clickStage("resume");
  await wait(async () => { const current = await playing(); return !current.paused && current.time >= 5.25; });
  assert.deepEqual([(await playing()).width, (await playing()).height], [360, 640]);
  assert.equal(await page.locator("audio, video").count(), 1);
  await clickStage("captions");
  await page.locator("#stage-caption-track").selectOption({ label: "es · Local" });
  assert.match(await stage().locator(".stage-captions").textContent(), /Texto local/);
  assert.equal(await stage().locator(".stage-captions b").count(), 0);
  await clickStage("captions");
  await page.mouse.move(200, 230);
  await page.screenshot({ path: path.join(base, "video-controls.png"), animations: "disabled" });
  await wait(async () => (await stage().getAttribute("data-controls")) === "hidden", 5000);
  await page.screenshot({ path: path.join(base, "video-clean.png"), animations: "disabled" });
  await page.mouse.move(250, 270);
  assert.equal(await stage().getAttribute("data-controls"), "visible");
  await clickStage("toggle");
  await wait(async () => (await playing()).paused);
  await stage().focus();
  await page.keyboard.press("ArrowLeft");
  const seek = stage().locator("#stage-seek");
  await wait(async () => (await playing()).time < 6);
  await seek.evaluate(range => {
    range.value = "10";
    range.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await wait(async () => Math.abs((await playing()).time - 10) < 0.05);
  assert.equal((await playing()).paused, true);

  await clickStage("options");
  await page.locator("#stage-rate").selectOption("1.25");
  await page.locator("#stage-fit").selectOption("cover");
  assert.equal((await playing()).rate, 1.25);
  assert.equal((await playing()).fit, "cover");
  await page.locator("#stage-fit").selectOption("contain");
  await clickStage("options");
  await stage().locator("#stage-volume").evaluate(range => {
    range.value = "0.36";
    range.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await clickStage("mute");
  assert.equal((await playing()).muted, true);
  await clickStage("mute");
  assert.equal((await playing()).volume, 0.36);
  const geometry = await page.evaluate(() => [innerWidth, innerHeight]);
  const fullscreen = () => page.evaluate(async () => {
    const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js");
    return getCurrentWindow().isFullscreen();
  });
  await clickStage("fullscreen");
  await wait(fullscreen);
  await clickStage("fullscreen");
  await wait(async () => !(await fullscreen()));
  await wait(async () => JSON.stringify(await page.evaluate(() => [innerWidth, innerHeight])) === JSON.stringify(geometry));

  const before = await playing();
  await clickStage("back");
  await stage().waitFor({ state: "hidden" });
  await page.locator("#audio-open").click();
  await stage().waitFor();
  assert.equal(await original.evaluate(element => element === document.getElementById("audio-engine")), true);
  const reopened = await playing();
  assert.equal(reopened.src, before.src);
  assert.equal(reopened.time, before.time);
  assert.equal(reopened.paused, true);
  await page.setViewportSize({ width: 900, height: 620 });
  assert.ok(await stage().locator(".stage-toolbar").evaluate(bar => bar.scrollWidth <= bar.clientWidth + 1));
  await page.screenshot({ path: path.join(base, "video-900.png"), animations: "disabled" });
  await clickStage("back");
  await page.locator("#audio-close").click();
  await play(musicId);
  await wait(async () => { const current = await playing(); return current.id === musicId && current.time > 0; });
  await page.locator("#audio-player").waitFor();
  await page.screenshot({ path: path.join(base, "compact-900.png"), animations: "disabled" });
  assert.equal(await original.evaluate(element => element === document.getElementById("audio-engine")), true);

  for (const file of [webm, mkv]) {
    await play(item(file).id);
    await wait(async () => { const current = await playing(); return current.error || current.width > 0; });
    const current = await playing();
    if (file === webm) assert.deepEqual([current.width, current.height], [720, 248]);
    if (current.error) await stage().locator(".stage-error").waitFor();
    console.log(JSON.stringify({ container: path.extname(file), codecs: clip(file).map(stream => stream.codec_name),
      outcome: current.error ? `media-error-${current.error}` : `${current.width}x${current.height}` }));
    await clickStage("back");
  }
  await play(item(broken).id);
  await stage().locator(".stage-error").waitFor();
  await stage().getByRole("button", { name: "Convertir una copia" }).waitFor();
  await clickStage("back");
  await play(videoId);
  await stage().locator("#stage-resume").waitFor();
  await clickStage("resume");
  await wait(async () => (await playing()).time > 10 && !(await playing()).paused);
  await clickStage("back");
  await wait(async () => (await rpc("get_snapshot")).library.find(candidate => candidate.id === videoId).details.playbackPosition >= 10);
  assert.deepEqual(errors, []);
  await shutdown();
  await launch();
  await page.locator("#audio-player").waitFor();
  await wait(async () => (await playing()).time >= 10);
  assert.equal((await playing()).paused, true);
  assert.equal((await playing()).id, videoId);
  assert.equal((await playing()).volume, 0.36);
  console.log(JSON.stringify({ sharedElement: true, compact: true, video: true, hidControlsAfterMouseClick: true, captions: true,
    fullscreen: true, restoredOnRestart: true, playlistAudioCover: true, viewports: [1180, 900] }));
} catch (error) {
  console.error(log);
  console.error(await page?.locator("#playback-stage").innerText().catch(() => ""));
  console.error(await page?.locator("#notification").textContent().catch(() => ""));
  throw error;
} finally {
  await shutdown();
}
