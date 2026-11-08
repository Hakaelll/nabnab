import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { mkdir, mkdtemp, readFile, rename, stat, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const base = path.join(root, "artifacts/playback");
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "native-run-"));
const output = path.join(data, "output");
await mkdir(output);
const desktopGesture = process.env.NABNAB_PLAYBACK_DESKTOP_GESTURE === "1";
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
const subtitles = path.join(data, "Subtítulos de prueba.srt");
const styledAss = path.join(data, "Estilos ñ.ass");
const styledSsa = path.join(data, "Estilos clásicos.ssa");
const missing = path.join(data, "06 · Archivo desplazado.mp4");
const handout = path.join(data, "Apuntes de la lección.txt");
const audioSamples = [[".wav", "pcm_s16le"], [".m4a", "aac"], [".opus", "libopus"], [".flac", "flac"]]
  .map(([extension, codec], index) => ({ file: path.join(data, `Audio ${index + 1}${extension}`), codec }));

function encode(...args) {
  return execFileSync(ffmpeg, ["-hide_banner", "-v", "error", ...args], { windowsHide: true });
}
encode("-f", "lavfi", "-i", "testsrc2=size=360x640:rate=24:duration=22",
  "-f", "lavfi", "-i", "sine=frequency=440:duration=22", "-f", "lavfi", "-i", "sine=frequency=660:duration=22",
  "-map", "0:v:0", "-map", "1:a:0", "-map", "2:a:0", "-c:v", "libopenh264", "-c:a", "aac",
  "-metadata:s:a:0", "language=spa", "-metadata:s:a:0", "handler_name=Castellano",
  "-metadata:s:a:1", "language=eng", "-metadata:s:a:1", "handler_name=Original", "-shortest", video);
encode("-i", video, "-t", "2", "-c", "copy", missing);
encode("-f", "lavfi", "-i", "color=c=#354551:s=100x100", "-frames:v", "1", picture);
encode("-f", "lavfi", "-i", "aevalsrc=if(between(t\\,4\\,10)\\,0\\,sin(2*PI*82*t)*(0.14+0.30*pow(0.5+0.5*cos(2*PI*2*t)\\,8))+0.13*sin(2*PI*540*t)+0.08*sin(2*PI*4800*t)*(0.5+0.5*sin(2*PI*1.7*t))):d=50", "-i", picture,
  "-map", "0:a", "-map", "1:v", "-c:a", "libmp3lame", "-c:v", "copy", "-disposition:v", "attached_pic",
  "-metadata", "title=Señal para la cola", "-metadata", "artist=Laboratorio nabnab", music);
encode("-f", "lavfi", "-i", "testsrc2=size=720x248:rate=20:duration=6", "-f", "lavfi",
  "-i", "sine=frequency=550:duration=6", "-c:v", "libvpx-vp9", "-deadline", "realtime", "-c:a", "libopus", "-shortest", webm);
for (const { file, codec } of audioSamples) encode("-f", "lavfi", "-i", "aevalsrc=(0.1+0.5*pow(0.5+0.5*cos(2*PI*3.5*t)\\,6))*sin(2*PI*550*t):d=10", "-c:a", codec, file);
await writeFile(broken, "Archivo de prueba sin contenedor de vídeo.");
await writeFile(captions, "WEBVTT\n\n00:00:04.000 --> 00:00:09.000\nTexto local &lt;b&gt;seguro&lt;/b&gt;\n");
await writeFile(subtitles, "1\n00:00:04,000 --> 00:00:09,000\nSecond local track\n");
await writeFile(styledAss, [
  "[Script Info]", "ScriptType: v4.00+", "PlayResX: 360", "PlayResY: 640", "ScaledBorderAndShadow: yes", "",
  "[V4+ Styles]", "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding",
  "Style: Default,Arial,30,&H0000F5FF,&H000000FF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,3,0,2,10,10,10,1", "",
  "[Events]", "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
  "Dialogue: 0,0:00:04.00,0:00:08.00,Default,,0,0,0,,{\\an7\\move(20,150,165,150,0,3500)}Texto ASS",
  "Dialogue: 1,0:00:04.00,0:00:08.00,Default,,0,0,0,,{\\an7\\pos(40,310)\\1c&HFFFF00&}Otra capa",
].join("\n"));
await writeFile(styledSsa, [
  "[Script Info]", "ScriptType: v4.00", "PlayResX: 360", "PlayResY: 640", "",
  "[V4 Styles]", "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, TertiaryColour, BackColour, Bold, Italic, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, AlphaLevel, Encoding",
  "Style: Default,Arial,32,16711935,16711935,0,0,-1,0,1,3,0,2,10,10,10,0,1", "",
  "[Events]", "Format: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
  "Dialogue: Marked=0,0:00:04.00,0:00:08.00,Default,,0,0,0,,Texto SSA",
].join("\n"));
encode("-i", webm, "-i", styledAss, "-i", styledSsa,
  "-map", "0:v", "-map", "0:a", "-map", "1:s", "-map", "2:s", "-c", "copy",
  "-metadata:s:s:0", "title=Rótulos de prueba", "-metadata:s:s:0", "language=spa",
  "-metadata:s:s:1", "title=Diálogo clásico", "-metadata:s:s:1", "language=eng", mkv);
await writeFile(handout, "Material de estudio de prueba.\n");

let child, browser, page, nativeHandle, log = "";
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
  child = spawn(process.env.NABNAB_PLAYBACK_TEST_EXE ?? path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
    cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
    env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port} --mute-audio` },
  });
  for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => { log = (log + chunk).slice(-6000); });
  browser = await wait(async () => {
    if (child.exitCode !== null) throw Error(`Native app exited ${child.exitCode}`);
    try {
      const response = await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(400) });
      if (response.ok) return await chromium.connectOverCDP(endpoint, { timeout: 2000 });
    } catch { /* WebView2 can recycle its debugging endpoint during restart. */ }
    return null;
  });
  page = await wait(() => browser.contexts()[0]?.pages().find(tab => /:1420\/$/.test(tab.url())));
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
  if (child) await wait(async () => {
    try { return !(await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(250) })).ok; }
    catch { return true; }
  }, 6000).catch(() => {});
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
const mainMinimized = () => page.evaluate(async () => {
  const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js");
  return getCurrentWindow().isMinimized();
});
const clip = file => JSON.parse(execFileSync(ffprobe, ["-v", "error", "-show_streams", "-of", "json", file], { windowsHide: true })).streams;

const seekVideo = position => stage().locator("#stage-seek").evaluate((slider, position) => {
  slider.value = String(position);
  slider.dispatchEvent(new Event("input", { bubbles: true }));
}, position);

function captionPixels(canvas) {
  return canvas.evaluate(canvas => {
    const result = { yellow: 0, cyan: 0, magenta: 0, left: canvas.width };
    if (canvas.hidden) return result;
    const { data } = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height);
    for (let offset = 0; offset < data.length; offset += 4) {
      const [red, green, blue, alpha] = data.subarray(offset, offset + 4);
      if (alpha < 200) continue;
      if (red > 190 && green > 190 && blue < 80) {
        result.yellow++;
        result.left = Math.min(result.left, (offset / 4) % canvas.width);
      }
      if (red < 80 && green > 190 && blue > 190) result.cyan++;
      if (red > 190 && green < 80 && blue > 190) result.magenta++;
    }
    return result;
  });
}

async function verifyStyledCaptions(engine) {
  const canvas = stage().locator(".stage-styled-captions");
  const select = page.locator("#stage-caption-track");
  const load = async file => {
    await page.evaluate(file => { window.nabnabStyledFixture = file; }, file);
    await clickStage("import-styled");
    await select.locator('option[value="styled"]').filter({ hasText: path.basename(file) }).waitFor({ state: "attached" });
    assert.equal(await select.inputValue(), "styled");
  };
  const pixels = () => captionPixels(canvas);
  await clickStage("toggle");
  await wait(async () => (await playing()).paused);
  await seekVideo(5);
  await load(styledAss);
  const first = await wait(async () => {
    const painted = await pixels();
    return painted.yellow > 100 && painted.cyan > 100 && painted;
  });
  assert.equal(await stage().locator(".stage-captions").isVisible(), false);
  await seekVideo(6.5);
  await wait(async () => (await pixels()).left > first.left + 20 && (await pixels()).yellow > 100);
  const box = await canvas.boundingBox();
  assert.ok(Math.abs(box.width / box.height - 360 / 640) < 0.01);
  await page.screenshot({ path: path.join(base, "video-ass.png"), animations: "disabled" });
  const offset = page.locator("#stage-caption-offset");
  await offset.fill("3");
  await offset.press("Tab");
  await canvas.waitFor({ state: "hidden" });
  await offset.fill("0");
  await offset.press("Tab");
  await canvas.waitFor({ state: "visible" });
  await load(styledSsa);
  await wait(async () => (await pixels()).magenta > 100);
  await select.selectOption({ label: "es · Local" });
  await canvas.waitFor({ state: "hidden" });
  await seekVideo(5.25);
  assert.equal(await engine.evaluate(video => video === document.getElementById("audio-engine")), true);
  await clickStage("toggle");
  console.log("Native libass preserved overlapping colors and moving ASS text, rendered SSA and respected the subtitle offset without replacing the video engine.");
}

async function verifyEmbeddedCaptions(engine, video) {
  const before = await playing();
  assert.equal(before.error, undefined, "The WebView must play the Matroska sample");
  await clickStage("toggle");
  await wait(async () => (await playing()).paused);
  await clickStage("captions");
  const canvas = stage().locator(".stage-styled-captions");
  const select = page.locator("#stage-caption-track");
  const tracks = select.locator("#stage-embedded-options option");
  await tracks.first().waitFor({ state: "attached" });
  assert.deepEqual(await tracks.allTextContents(), ["Rótulos de prueba · spa · ASS", "Diálogo clásico · eng · ASS"]);
  await assert.rejects(rpc("styled_caption_embedded", { id: "6d9df8bf-c66b-424a-b965-1adbe40bb411", video, index: 0 }),
    /not an embedded ASS\/SSA subtitle/);
  await seekVideo(4.5);
  await select.selectOption({ label: "Rótulos de prueba · spa · ASS" });
  await stage().locator("#stage-caption-origin").getByText("Integrada").waitFor();
  const first = await wait(async () => {
    const frame = await captionPixels(canvas);
    return frame.yellow > 60 && frame.cyan > 60 && frame;
  });
  await seekVideo(5.75);
  await wait(async () => {
    const frame = await captionPixels(canvas);
    return frame.yellow > 60 && frame.cyan > 60 && frame.left > first.left + 5;
  });
  await page.screenshot({ path: path.join(base, "video-embedded.png"), animations: "disabled" });
  await select.selectOption({ label: "Diálogo clásico · eng · ASS" });
  await wait(async () => {
    const frame = await captionPixels(canvas);
    return frame.magenta > 50 && frame.yellow === 0 && frame.cyan === 0;
  });
  const after = await playing();
  assert.equal(after.src, before.src);
  assert.ok(Math.abs(after.time - 5.75) < 0.05);
  assert.equal(after.paused, true);
  assert.equal(await engine.evaluate(element => element === document.getElementById("audio-engine")), true);
  await select.selectOption("");
  await canvas.waitFor({ state: "hidden" });
  console.log("Native Matroska captions retained language, colors, layers and motion while switching styles in the same paused video.");
}

async function verifyAudioTracks(engine, videoId) {
  const source = await readFile(video);
  const before = await playing();
  await play(videoId);
  await clickStage("resume");
  await wait(async () => {
    const current = await playing();
    return current.id === videoId && !current.paused && current.time > 10;
  });
  await clickStage("toggle");
  await clickStage("options");
  const select = stage().locator("#stage-audio-track");
  await select.locator("option").filter({ hasText: "Original · inglés" }).waitFor({ state: "attached" });
  const time = (await playing()).time;
  const frequency = async expected => {
    let actual;
    try {
      await wait(async () => {
        actual = await page.evaluate(() => ({
          hz: window.playbackFrequency?.(),
          decoded: document.querySelector("video").webkitAudioDecodedByteCount,
          audio: document.querySelector("video").audioTracks?.length,
        }));
        return Math.abs(actual.hz - expected) < 12;
      }, 4500);
    } catch (cause) {
      throw Error(`Expected audio at ${expected} Hz, observed ${JSON.stringify(actual)}`, { cause });
    }
  };
  await select.focus();
  await select.selectOption({ label: "Original · inglés" });
  await wait(async () => await select.isEnabled() && /\?audio=3$/.test((await playing()).src));
  assert.ok(Math.abs((await playing()).time - time) < 0.08);
  assert.equal((await playing()).paused, true);
  assert.equal(await select.evaluate(element => element === document.activeElement), true);
  await page.screenshot({ path: path.join(base, "video-audio-tracks.png"), animations: "disabled" });
  await clickStage("toggle");
  await frequency(660);
  await select.selectOption({ label: "Predeterminada del archivo" });
  await wait(async () => await select.isEnabled() && !/\?audio=/.test((await playing()).src) && !(await playing()).paused);
  await frequency(440);
  const after = await playing();
  assert.equal(after.rate, before.rate);
  assert.equal(after.volume, before.volume);
  assert.equal(after.width, 360);
  assert.equal(await engine.evaluate(element => element === document.getElementById("audio-engine")), true);
  assert.deepEqual(await readFile(video), source);
  await clickStage("back");
  console.log("WebView2 decoded both audio tracks at their original frequencies and restored the same position and player.");
}

async function seedPlaybackFixtures(videoId, audioId, handoutId, bookIds) {
  const stateFile = path.join(data, "state-v1.json");
  const state = JSON.parse(await readFile(stateFile, "utf8"));
  const asset = id => {
    const item = state.library.find(candidate => candidate.id === id);
    return { id: `asset-${id}`, title: item.name, kind: item.kind, status: "local", libraryId: id, urlRef: null, jobIds: [] };
  };
  const prior = { id: "note-fixture", text: "Nota anterior", time: 7, bookmarkId: "course-note-fixture" };
  state.library.find(item => item.id === handoutId).details.bookmarks.push({ id: prior.bookmarkId, name: prior.text, time: prior.time });
  state.courses.push({ id: "local-fixture", title: "Taller de composición", provider: "local", url: null, createdAt: 1,
    sections: [{ id: "section-fixture", title: "01 · Fundamentos", lessons: [
      { id: "lesson-video", title: "Encuadre y proporción", learned: false, assets: [asset(handoutId), asset(videoId)], notes: [prior] },
      { id: "lesson-audio", title: "Ejercicio de escucha", learned: false, assets: [asset(audioId)], notes: [] },
    ] }],
  });
  for (const [index, id] of bookIds.entries()) {
    state.library.find(item => item.id === id).details.audiobook = {
      bookId: "fixture-audiobook", bookTitle: "Lectura de prueba", chapter: index + 1, reader: null, language: "Spanish",
    };
  }
  await writeFile(stateFile, JSON.stringify(state));
}

async function verifyBookContinuation(ids) {
  const openBooks = async () => {
    await page.locator('button[data-page="library"]').click();
    await page.locator('[data-action="library-section"][data-section="audio"]').click();
    await page.getByRole("button", { name: "Audiolibros", exact: true }).click();
  };
  await openBooks();
  await page.locator(`[data-action="audio-play"][data-id="${ids[0]}"]`).click();
  const chapter = await wait(async () => {
    const sample = await playing();
    return sample.id === ids[0] && !sample.paused && sample.duration > 0 && sample;
  });
  await page.locator("#audio-sleep").selectOption("chapter");
  await page.locator("#audio-engine").evaluate(media => { media.currentTime = media.duration - 0.2; });
  await wait(async () => (await playing()).paused);
  const saved = await wait(async () => {
    const { library } = await rpc("get_snapshot");
    const item = library.find(item => item.id === ids[0]);
    return item.details.playbackPosition >= chapter.duration - 0.05 && item;
  });
  assert.ok(saved.details.lastPlayedAt);
  assert.equal((await playing()).id, ids[0]);
  await shutdown();
  await launch();
  await openBooks();
  await page.getByRole("button", { name: /Continuar · Lectura de prueba · Capítulo 2/ }).click();
  await wait(async () => (await playing()).id === ids[1] && !(await playing()).paused);
  assert.equal(await page.locator(".audio-next li").count(), 1);
  console.log("Native audiobook continuation retained the completed chapter across restart and opened the next chapter.");
}

async function verifyCoursePlayback(videoId, musicId, audioId, handoutId) {
  await page.locator('[data-page="library"]').click();
  await play(musicId);
  await wait(async () => (await playing()).id === musicId && !(await playing()).paused);
  const engine = await page.locator("#audio-engine").elementHandle();
  const button = name => page.getByRole("button", { name, exact: true });
  await page.locator('[data-page="platforms"]').click();
  await page.locator('.platform-shortcuts [data-action="open-courses"]').click();
  await page.getByRole("button", { name: /Taller de composición/ }).click();
  assert.equal(await page.locator("audio, video").count(), 1);
  assert.equal(await button("Añadir a la cola").count(), 0);
  assert.equal((await playing()).id, musicId);
  assert.equal(await button("Usar tiempo actual").isEnabled(), false);
  await page.screenshot({ path: path.join(base, "course-video-1180x780.png"), animations: "disabled" });
  await button("Reproducir lección").press("Enter");
  await stage().locator("#stage-resume").waitFor();
  assert.equal(await engine.evaluate(node => node === document.getElementById("audio-engine")), true);
  assert.equal((await playing()).id, videoId);
  await clickStage("resume");
  await wait(async () => (await playing()).time > 10 && !(await playing()).paused);
  await clickStage("back");
  await button("Usar tiempo actual").click();
  const timestamp = Number(await page.locator("#course-note-time").inputValue());
  assert.ok(timestamp >= 10 && timestamp < 22);
  await page.locator("#course-note-text").fill("Volver al encuadre vertical");
  await button("Guardar nota").click();
  const added = page.locator(".course-note").filter({ hasText: "Volver al encuadre vertical" });
  await added.waitFor();
  const snapshot = await rpc("get_snapshot");
  const videoBookmark = snapshot.library.find(item => item.id === videoId).details.bookmarks.find(bookmark => bookmark.name === "Volver al encuadre vertical");
  assert.equal(videoBookmark?.time, timestamp);
  assert.ok(!snapshot.library.find(item => item.id === handoutId).details.bookmarks.some(bookmark => bookmark.id === videoBookmark.id));
  await page.locator(".course-note").filter({ hasText: "Nota anterior" }).getByRole("button", { name: "Eliminar nota" }).click();
  await wait(async () => !(await rpc("get_snapshot")).library.find(item => item.id === handoutId).details.bookmarks.some(bookmark => bookmark.id === "course-note-fixture"));
  await added.getByRole("button", { name: `${timestamp.toFixed(1)} s` }).click();
  await stage().waitFor();
  await wait(async () => (await playing()).paused && Math.abs((await playing()).time - timestamp) < 0.05);
  await clickStage("back");
  await button("Ejercicio de escucha").click();
  await button("Reproducir lección").click();
  await wait(async () => (await playing()).id === audioId && !(await playing()).paused);
  assert.equal(await stage().isVisible(), false);
  await page.locator("#audio-seek").evaluate(range => {
    range.value = "5";
    range.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await page.locator("#audio-toggle").click();
  await wait(async () => (await rpc("get_snapshot")).library.find(item => item.id === audioId).details.playbackPosition >= 4.95);
  await shutdown();
  await launch();
  await wait(async () => (await playing()).id === audioId && (await playing()).time >= 4.95);
  assert.equal((await playing()).paused, true);
  console.log("Course lessons use the shared engine, timed notes target the video, and short audio resumes after restart.");
}

async function downloadDuringPlayback(itemId) {
  const { job, observations } = await page.evaluate(async () => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    const { listen } = await import("/node_modules/@tauri-apps/api/event.js");
    const media = document.getElementById("audio-engine");
    const clientRequest = crypto.randomUUID();
    const observations = [];
    const { promise, resolve, reject } = Promise.withResolvers();
    const timer = setTimeout(() => reject(Error("Sample download did not reach a terminal state.")), 30_000);
    const unlisten = await listen("job-update", ({ payload: job }) => {
      if (job.clientRequest !== clientRequest) return;
      observations.push({ state: job.state, id: media.dataset.localPlayer, time: media.currentTime,
        paused: media.paused, bytes: job.progress?.downloaded ?? 0 });
      if (["completed", "error", "cancelled"].includes(job.state)) resolve(job);
    });
    try {
      await invoke("queue_download", {
        urls: ["https://developer.mozilla.org/shared-assets/videos/flower.mp4"], titles: ["MDN video sample"],
        format: "source", height: null, audioBitrate: null, subtitleLanguage: null, clientRequest,
      });
      return { job: await promise, observations };
    } finally {
      clearTimeout(timer);
      unlisten();
    }
  });
  assert.equal(job.state, "completed", JSON.stringify(job.error));
  const active = observations.filter(sample => sample.state === "downloading");
  assert.ok(active.some(sample => sample.bytes > 0), JSON.stringify(observations));
  assert.ok(observations.every(sample => sample.id === itemId && !sample.paused), JSON.stringify(observations));
  const elapsed = observations.at(-1).time - active[0].time;
  assert.ok(elapsed > 0, JSON.stringify(observations));
  const { size } = await stat(job.outputs[0]);
  assert.ok(size > 0 && size <= 4 * 1024 * 1024);
  assert.ok(clip(job.outputs[0]).some(stream => stream.codec_type === "video"));
  console.log(JSON.stringify({ downloadedDuringPlayback: size, elapsed }));
}

function windowCommand(operation, width = 0, height = 0) {
  const command = operation === "resize"
    ? `[Nabnab.PlaybackTestWindow]::SetWindowPos($window, [IntPtr]0, 0, 0, ${width}, ${height}, 0x4016)`
    : `[Nabnab.PlaybackTestWindow]::PostMessageW($window, 0x112, [IntPtr]${operation === "minimize" ? "0xf020" : "0xf120"}, [IntPtr]0)`;
  nativeHandle = Number(execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `
    Add-Type -Name PlaybackTestWindow -Namespace Nabnab -MemberDefinition @'
    [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool PostMessageW(System.IntPtr handle, uint message, System.IntPtr command, System.IntPtr unused);
    [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool SetWindowPos(System.IntPtr handle, System.IntPtr after, int x, int y, int width, int height, uint flags);
    [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern System.IntPtr SetThreadDpiAwarenessContext(System.IntPtr awareness);
'@
    [void][Nabnab.PlaybackTestWindow]::SetThreadDpiAwarenessContext([IntPtr](-4))
    $window = ${nativeHandle ? `[IntPtr]${nativeHandle}` : `(Get-Process -Id ${child.pid} -ErrorAction Stop).MainWindowHandle`}
    if (!$window -or !(${command})) { exit 1 }
    $window.ToInt64()
  `], { windowsHide: true, encoding: "utf8" }).trim());
  assert.ok(Number.isSafeInteger(nativeHandle) && nativeHandle > 0);
}

async function verifyDesktopWheel(original, selectedTracks) {
  const overlay = await wait(() => browser.contexts()[0]?.pages().find(tab => tab.url().includes("/quick-skip.html")));
  overlay.on("pageerror", error => { log += `\nQuick Skip: ${error.message}`; });
  const other = spawn("powershell.exe", ["-NoProfile", "-NonInteractive", "-STA", "-Command", `
    Add-Type -AssemblyName System.Windows.Forms
    $work = [Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    $form = [Windows.Forms.Form]::new()
    $form.Text = 'Nabnab playback test window'
    $form.StartPosition = 'Manual'
    $form.SetBounds($work.Right - 450, $work.Top + 180, 420, 280)
    $form.Add_Shown({ $form.Activate(); [Console]::WriteLine('ready') })
    [Windows.Forms.Application]::Run($form)
  `], { windowsHide: true, stdio: ["ignore", "pipe", "pipe"] });
  let ready = false;
  other.stdout.on("data", bytes => { if (String(bytes).includes("ready")) ready = true; });
  other.stderr.on("data", bytes => { log += String(bytes); });
  function desktop(operation, delta = 0) {
    return execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `
      Add-Type -Name DesktopWheelInput -Namespace Nabnab -MemberDefinition @'
      [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();
      [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint process);
      [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
      [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, System.UIntPtr extra);
      [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern void mouse_event(uint flags, int x, int y, int data, System.UIntPtr extra);
      [System.Runtime.InteropServices.DllImport("user32.dll")] public static extern System.IntPtr SetThreadDpiAwarenessContext(System.IntPtr awareness);
'@
      $api = [Nabnab.DesktopWheelInput]
      [uint32]$owner = 0
      [void]$api::GetWindowThreadProcessId($api::GetForegroundWindow(), [ref]$owner)
      $owner
      if ('${operation}' -eq 'release') { $api::keybd_event(0xA4, 0, 2, [UIntPtr]::Zero); exit }
      if ($owner -ne ${other.pid}) { exit }
      if ('${operation}' -eq 'open') {
        [void]$api::SetThreadDpiAwarenessContext([IntPtr](-4))
        Add-Type -AssemblyName System.Windows.Forms
        $work = [Windows.Forms.Screen]::PrimaryScreen.WorkingArea
        [void]$api::SetCursorPos($work.Right - 90, $work.Top + 300)
        $api::keybd_event(0xA4, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 35
      }
      if (${delta} -ne 0) { $api::mouse_event(0x800, 0, 0, ${delta}, [UIntPtr]::Zero) }
      if ('${operation}' -eq 'cancel') {
        $api::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero)
        $api::keybd_event(0x1B, 0, 2, [UIntPtr]::Zero)
      }
    `], { windowsHide: true, encoding: "utf8" }).trim();
  }

  const wheel = overlay.locator("#track-wheel");
  const central = wheel.locator('[data-offset="0"]');
  const visible = () => page.evaluate(async () => {
    const { Window } = await import("/node_modules/@tauri-apps/api/window.js");
    return new Window("quick-skip", { skip: true }).isVisible();
  });
  try {
    await wait(() => ready);
    await wait(() => desktop("status") === String(other.pid), 7000);
    const before = (await playing()).id;
    const expected = selectedTracks.find(item => item.name === path.basename(music)).id;
    const queue = await page.evaluate(() => JSON.parse(localStorage.getItem("nabnab.playback.v1")).queue);
    const coverPosition = queue.order.findIndex(index => queue.tracks[index] === expected);
    const distance = coverPosition - queue.cursor;
    assert.ok(coverPosition >= 0 && distance !== 0);
    assert.equal(desktop("open", -120 * Math.sign(distance)), String(other.pid));
    await central.waitFor();
    assert.equal(await wheel.getAttribute("data-side"), "left");
    if (Math.abs(distance) > 1) assert.equal(desktop("step", -120 * (distance - Math.sign(distance))), String(other.pid));
    await wait(async () => (await central.getAttribute("data-position")) === String(coverPosition));
    assert.equal((await playing()).id, before);
    assert.equal(await mainMinimized(), true);
    assert.equal(await visible(), true);
    await central.locator(".audio-cover img").waitFor();
    await overlay.screenshot({ path: path.join(output, "quick-skip-desktop.png"), omitBackground: true });
    assert.equal(desktop("release"), String(other.pid));
    await wheel.waitFor({ state: "hidden" });
    await wait(async () => !await visible());
    await wait(async () => (await playing()).id === expected);
    assert.equal(await mainMinimized(), true);
    assert.equal(await original.evaluate(element => element === document.getElementById("audio-engine")), true);
    assert.equal(desktop("open", 120 * Math.sign(distance)), String(other.pid));
    await central.waitFor();
    assert.equal(desktop("cancel"), String(other.pid));
    await wheel.waitFor({ state: "hidden" });
    desktop("release");
    assert.equal((await playing()).id, expected);
    await page.locator("#audio-close").evaluate(button => button.click());
    await wait(() => page.evaluate(() => JSON.parse(localStorage.getItem("nabnab.playback.v1")).queue === null));
    assert.equal(desktop("open", -120), String(other.pid));
    assert.equal(desktop("release"), String(other.pid));
    assert.equal(await visible(), false);
    assert.equal(await mainMinimized(), true);
    console.log("Desktop Alt + wheel committed and canceled while minimized and stayed closed after stopping playback.");
  } finally {
    desktop("release");
    other.kill();
  }
}

async function av1Fixture() {
  const result = await page.evaluate(async () => {
    if (typeof VideoEncoder === "undefined") return null;
    const width = 256, height = 144, count = 36, rate = 24;
    const config = { codec: "av01.0.04M.08", width, height, bitrate: 160000, framerate: rate, hardwareAcceleration: "prefer-software" };
    if (!(await VideoEncoder.isConfigSupported(config)).supported) return null;
    const canvas = new OffscreenCanvas(width, height);
    const context = canvas.getContext("2d");
    const frames = [];
    let failure = "";
    const encoder = new VideoEncoder({
      output(chunk) {
        const data = new Uint8Array(chunk.byteLength);
        chunk.copyTo(data);
        frames.push({ timestamp: chunk.timestamp, data: Array.from(data) });
      },
      error(error) { failure = error.message; },
    });
    encoder.configure(config);
    for (let index = 0; index < count; index++) {
      context.fillStyle = "#16213a"; context.fillRect(0, 0, width, height);
      context.fillStyle = "#71d4cd"; context.fillRect(index * 5, 40, 42, 60);
      const frame = new VideoFrame(canvas, { timestamp: Math.round(index * 1e6 / rate) });
      encoder.encode(frame, { keyFrame: index === 0 }); frame.close();
    }
    await encoder.flush(); encoder.close();
    if (failure || !frames.length) throw Error(failure || "AV1 encoder returned no frames");
    return { width, height, rate, frames };
  }).catch(error => { console.log(JSON.stringify({ av1Fixture: "unavailable", detail: error.message })); return null; });
  if (!result) return null;
  const ivf = path.join(data, "fixture-av1.ivf");
  const mp4 = path.join(data, "07 · AV1.mp4");
  const header = Buffer.alloc(32);
  header.write("DKIF"); header.writeUInt16LE(32, 6); header.write("AV01", 8);
  header.writeUInt16LE(result.width, 12); header.writeUInt16LE(result.height, 14);
  header.writeUInt32LE(result.rate, 16); header.writeUInt32LE(1, 20); header.writeUInt32LE(result.frames.length, 24);
  const chunks = result.frames.map(({ timestamp, data }) => {
    const frame = Buffer.alloc(12 + data.length);
    frame.writeUInt32LE(data.length); frame.writeBigUInt64LE(BigInt(Math.round(timestamp * result.rate / 1e6)), 4);
    Buffer.from(data).copy(frame, 12);
    return frame;
  });
  await writeFile(ivf, Buffer.concat([header, ...chunks]));
  encode("-i", ivf, "-c", "copy", mp4);
  assert.equal(clip(mp4)[0].codec_name, "av1");
  return mp4;
}

async function renderPreviews() {
  const output = path.join(base, "previews");
  await mkdir(output, { recursive: true });
  const names = await page.evaluate(async () => {
    const { scenes, createMusicVisualizer } = await import("/src/music-visualizer.ts");
    const canvas = document.createElement("canvas");
    canvas.id = "native-scene-preview";
    Object.assign(canvas.style, { position: "fixed", top: "0", left: "0", width: "384px", height: "216px", zIndex: "1000" });
    document.body.append(canvas);
    const waveform = Float32Array.from({ length: 48 }, (_, i) => Math.sin(i * 0.3) * 0.48);
    const input = { bass: 0.56, mids: 0.42, highs: 0.25, level: 0.73, attack: 0.09, stereo: 0.14, waveform };
    const visualizer = createMusicVisualizer(canvas, () => input, () => false, failed => { canvas.dataset.failed = String(failed); }, () => {});
    visualizer.show();
    visualizer.playing(true);
    window.renderScenePreview = (scene, color) => {
      visualizer.playing(false);
      visualizer.update({ scene, color, intensity: 0.82, quality: "high" });
      visualizer.reset();
      visualizer.playing(true);
    };
    return scenes.map(([name]) => name);
  });
  for (const [index, name] of names.entries()) {
    for (const color of [false, true]) {
      await page.evaluate(({ index, color }) => window.renderScenePreview(index, color), { index, color });
      await page.waitForFunction(() => document.querySelector("#native-scene-preview")?.dataset.failed === "false");
      await page.waitForTimeout(400);
      const preview = await page.locator("#native-scene-preview").screenshot({ animations: "disabled", scale: "css" });
      execFileSync(ffmpeg, ["-hide_banner", "-v", "error", "-y", "-f", "image2pipe", "-i", "pipe:0", "-frames:v", "1", "-c:v", "libwebp", "-quality", "84",
        path.join(output, `${name}${color ? "-color" : ""}.webp`)], { windowsHide: true, input: preview });
    }
  }
  console.log(`Rendered ${names.length * 2} native WebGL previews to ${output}`);
}

try {
  const initial = await launch();
  if (process.env.NABNAB_RENDER_SCENES === "1") {
    await renderPreviews();
    await shutdown();
    process.exit(0);
  }
  const errors = [];
  const unexpectedNetwork = [];
  await page.route(/^https?:\/\/(?!(?:localhost|127\.0\.0\.1|nabmedia\.localhost|ipc\.localhost|tauri\.localhost)(?=[:/]))/, route => {
    unexpectedNetwork.push(new URL(route.request().url()).origin);
    return route.abort("internetdisconnected");
  });
  await page.route(/\/src\/styled-captions\.ts(?:\?.*)?$/, async route => {
    const response = await route.fetch();
    const script = await response.text();
    assert.ok(script.includes("path: null"));
    // Provide the selected file in place of the system file picker. Rendering still runs through native IPC.
    await route.fulfill({ response, body: script.replace("path: null", "path: window.nabnabStyledFixture") });
  });
  page.on("pageerror", error => errors.push(error.message));
  const av1 = await av1Fixture();
  await page.addInitScript(() => {
    const createSource = AudioContext.prototype.createMediaElementSource;
    AudioContext.prototype.createMediaElementSource = function (element) {
      const source = createSource.call(this, element);
      const analyser = this.createAnalyser();
      analyser.fftSize = 8192;
      analyser.smoothingTimeConstant = 0;
      const silence = new GainNode(this, { gain: 0 });
      source.connect(analyser).connect(silence).connect(this.destination);
      const levels = new Float32Array(analyser.frequencyBinCount);
      window.playbackFrequency = () => {
        analyser.getFloatFrequencyData(levels);
        let peak = 1;
        for (let index = 2; index < levels.length; index++) if (levels[index] > levels[peak]) peak = index;
        return levels[peak] > -85 ? peak * analyser.context.sampleRate / analyser.fftSize : 0;
      };
      return source;
    };
    const locations = new WeakMap();
    const proto = WebGL2RenderingContext.prototype;
    const location = proto.getUniformLocation, uniform4 = proto.uniform4f, uniform1 = proto.uniform1f, uniformId = proto.uniform1i, draw = proto.drawArrays;
    window.sceneProbe = { frames: 0, signal: [], blend: 1, scene: 0, timestamps: [] };
    proto.getUniformLocation = function (program, name) {
      const value = location.call(this, program, name);
      if (value) locations.set(value, name);
      return value;
    };
    proto.uniform4f = function (uniform, ...values) {
      if (this.canvas.closest(".music-surface") && locations.get(uniform) === "signal") window.sceneProbe.signal = values;
      return uniform4.call(this, uniform, ...values);
    };
    proto.uniform1f = function (uniform, value) {
      if (this.canvas.closest(".music-surface") && locations.get(uniform) === "blend") window.sceneProbe.blend = value;
      return uniform1.call(this, uniform, value);
    };
    proto.uniform1i = function (uniform, value) {
      if (this.canvas.closest(".music-surface") && locations.get(uniform) === "scene") window.sceneProbe.scene = value;
      return uniformId.call(this, uniform, value);
    };
    proto.drawArrays = function (...args) {
      if (this.canvas.closest(".music-surface")) {
        window.sceneProbe.frames++;
        window.sceneProbe.timestamps.push(performance.now());
        if (window.sceneProbe.timestamps.length > 240) window.sceneProbe.timestamps.shift();
      }
      return draw.call(this, ...args);
    };
  });
  await rpc("save_settings", { settings: { ...initial.settings, folder: output, language: "es", reducedMotion: false, trayEnabled: false, closeToTray: false } });
  const imported = await rpc("import_paths", { paths: [video, music, webm, mkv, broken, missing, handout, ...audioSamples.map(sample => sample.file), ...(av1 ? [av1] : [])] });
  const item = file => imported.library.find(candidate => candidate.name === path.basename(file));
  const videoId = item(video).id, musicId = item(music).id;
  assert.equal(item(music).kind, "audio");
  assert.equal(item(music).details.audio.embeddedCover, true);
  assert.equal(item(video).videoCodec, "h264");
  assert.equal(item(video).audioCodec, "aac");
  await rpc("subtitle_import", { id: videoId, language: "es", path: captions });
  await rpc("subtitle_import", { id: videoId, language: "en", path: subtitles });
  await rpc("library_playback", { id: videoId, position: 5.25 });
  await page.reload();
  await page.locator('[data-page="library"]').click();
  await page.locator(`.media-details[data-id="${videoId}"]`).click();
  await page.locator('[data-action="close-inspector"]:visible').click();
  await page.locator(`.media-open[data-id="${videoId}"]`).dblclick();
  await stage().locator("#stage-resume").waitFor();
  const original = await page.locator("#audio-engine").elementHandle();
  assert.equal((await playing()).paused, true);
  await clickStage("resume");
  await wait(async () => { const current = await playing(); return !current.paused && current.time >= 5.25; });
  assert.deepEqual([(await playing()).width, (await playing()).height], [360, 640]);
  assert.equal(await page.locator("audio, video").count(), 1);
  await clickStage("mini");
  await page.locator('#playback-stage[data-mini="video"] > video').waitFor();
  assert.equal(await page.locator(".app-shell").evaluate(shell => shell.inert), false);
  const miniTime = (await playing()).time;
  await wait(async () => (await playing()).time > miniTime + 0.25);
  await stage().screenshot({ path: path.join(output, "mini-video.png"), animations: "disabled" });
  await page.locator("#mini-float").click();
  await wait(async () => await mainMinimized() && await page.evaluate(() => innerWidth <= 400));
  const floatedVideoTime = (await playing()).time;
  await wait(async () => (await playing()).time > floatedVideoTime + 0.2);
  assert.equal(await stage().locator("video").isVisible(), true);
  assert.equal(await original.evaluate(element => element === document.getElementById("audio-engine")), true);
  await stage().screenshot({ path: path.join(output, "mini-video-detached.png"), animations: "disabled" });
  await page.locator("#mini-expand").click();
  await wait(async () => !await mainMinimized() && await page.evaluate(() => innerWidth >= 900));
  assert.equal(await original.evaluate(element => element === document.getElementById("audio-engine")), true);
  await seekVideo(5.5);
  await clickStage("captions");
  await page.locator("#stage-caption-track").selectOption({ label: "es · Local" });
  assert.match(await stage().locator(".stage-captions").textContent(), /Texto local/);
  assert.equal(await stage().locator(".stage-captions b").count(), 0);
  await page.locator("#stage-caption-track").selectOption({ label: "en · Local" });
  assert.match(await stage().locator(".stage-captions").textContent(), /Second local track/);
  await verifyStyledCaptions(original);
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
    for (const position of ["2", "18", "4", "17", "10"]) {
      range.value = position;
      range.dispatchEvent(new Event("input", { bubbles: true }));
    }
  });
  await wait(() => seek.evaluate(() => {
    const media = document.getElementById("audio-engine");
    return !media.seeking && Math.abs(media.currentTime - 10) < 0.05;
  }));
  assert.equal((await playing()).paused, true);
  await seek.hover({ position: { x: 200, y: 12 } });
  const hoverFrame = stage().locator("#stage-hover img");
  await wait(async () => hoverFrame.evaluate(image => !image.hidden && image.naturalHeight > 0));
  assert.match(await hoverFrame.getAttribute("src"), /^data:image\/jpeg;base64,/);
  await page.screenshot({ path: path.join(base, "video-seek.png"), animations: "disabled" });
  console.log(JSON.stringify({ audioTracksInMp4: clip(video).filter(stream => stream.codec_type === "audio").map(stream => stream.tags.language),
    audioTracksExposedByWebView: await page.locator("#audio-engine").evaluate(media => media.audioTracks?.length ?? 0) }));

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
  await stage().focus();
  await page.keyboard.press("ArrowUp");
  assert.equal((await playing()).volume, 0.41);
  assert.equal(await stage().locator(".stage-feedback").innerText(), "41%");
  await page.keyboard.press("ArrowDown");
  assert.equal((await playing()).volume, 0.36);
  const volume = stage().locator("#stage-volume");
  await volume.press("ArrowUp");
  assert.equal((await playing()).volume, 0.37);
  await volume.press("ArrowDown");
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
  await page.locator("#audio-toggle").press("ArrowDown");
  assert.equal((await playing()).volume, 0.31);
  await page.locator("#audio-toggle").press("ArrowUp");
  assert.equal((await playing()).volume, 0.36);
  await page.locator("#audio-open").click();
  await stage().waitFor();
  assert.equal(await original.evaluate(element => element === document.getElementById("audio-engine")), true);
  const reopened = await playing();
  assert.equal(reopened.src, before.src);
  assert.equal(reopened.time, before.time);
  assert.equal(reopened.paused, true);
  const scale = await page.evaluate(() => devicePixelRatio);
  const originalWidth = await page.evaluate(() => innerWidth);
  const target = [900, 620];
  const requested = target.map(size => Math.round(size * scale));
  windowCommand("resize", ...requested);
  const actual = await wait(() => page.evaluate(initial => innerWidth !== initial ? [innerWidth, innerHeight] : null, originalWidth), 8000);
  if (actual.some((size, index) => Math.abs(size - target[index]) > 2)) {
    windowCommand("resize", ...requested.map((size, index) => size + Math.round((target[index] - actual[index]) * scale)));
  }
  await wait(() => page.evaluate(() => Math.abs(innerWidth - 900) <= 3 && Math.abs(innerHeight - 620) <= 3), 8000)
    .catch(async error => { throw Error(`Native resize returned ${await page.evaluate(() => [innerWidth, innerHeight, devicePixelRatio])}`, { cause: error }); });
  assert.ok(await stage().locator(".stage-toolbar").evaluate(bar => bar.scrollWidth <= bar.clientWidth + 1));
  await page.screenshot({ path: path.join(base, "video-900.png"), animations: "disabled" });
  await clickStage("captions");
  assert.equal(await stage().locator('#stage-caption-track option[value="styled"]').count(), 0);
  assert.ok(await stage().locator("#stage-panel-captions").evaluate(panel => {
    const bounds = panel.getBoundingClientRect();
    const contrast = panel.querySelector(".stage-caption-contrast");
    return bounds.left >= 0 && bounds.top >= 0 && bounds.right <= innerWidth && bounds.bottom <= innerHeight
      && panel.scrollWidth <= panel.clientWidth + 1 && getComputedStyle(contrast).flexDirection === "row";
  }));
  await clickStage("captions");
  await clickStage("back");
  await page.locator("#audio-close").click();
  await page.locator('[data-action="library-layout"][data-layout="list"]').click();
  const musicRow = page.locator(`.media-list .media-open[data-id="${musicId}"]`);
  await musicRow.click();
  assert.equal(await page.locator("#audio-player").isVisible(), false);
  await musicRow.dblclick();
  await wait(async () => { const current = await playing(); return current.id === musicId && current.time > 0; });
  await page.locator("#audio-player").waitFor();
  assert.ok(await page.locator("#audio-player").evaluate(bar => bar.scrollWidth <= bar.clientWidth + 1));
  await page.screenshot({ path: path.join(base, "compact-900.png"), animations: "disabled" });
  await page.locator('[data-page="home"]').click();
  await page.locator("#url").waitFor();
  assert.ok(await page.locator(".work-area").evaluate(area => {
    const main = area.querySelector("#main"), bar = area.querySelector("#audio-player");
    const frame = area.getBoundingClientRect(), controls = bar.getBoundingClientRect();
    return main.getBoundingClientRect().bottom === controls.top && controls.bottom === frame.bottom
      && controls.left === frame.left && getComputedStyle(bar).borderTopWidth === "0px";
  }));
  await page.screenshot({ path: path.join(base, "compact-home-900.png"), animations: "disabled" });
  await page.locator('[data-page="library"]').click();
  await page.locator('[data-action="library-layout"][data-layout="grid"]').click();
  assert.equal(await original.evaluate(element => element === document.getElementById("audio-engine")), true);

  await page.locator(".audio-visualize").click();
  await stage().waitFor();
  assert.equal(await stage().getAttribute("data-kind"), "music");
  assert.equal(await page.locator("audio, video").count(), 1);
  const graphics = () => page.evaluate(() => window.sceneProbe);
  await wait(async () => (await graphics()).signal[3] > 0.25);
  await wait(async () => page.locator(".music-identity .audio-cover img").count());
  assert.deepEqual(await page.locator(".music-identity p").allTextContents(), ["Laboratorio nabnab"]);
  await page.mouse.move(220, 150);
  assert.ok(await stage().locator(".stage-toolbar").evaluate(bar => {
    const toggle = bar.querySelector("#stage-toggle").getBoundingClientRect();
    return bar.scrollWidth <= bar.clientWidth + 1 && Math.abs(toggle.x + toggle.width / 2 - innerWidth / 2) < 2;
  }));
  await page.screenshot({ path: path.join(base, "music-900.png"), animations: "disabled" });
  await clickStage("scenes");
  assert.ok(await stage().locator("#stage-panel-scenes").evaluate(panel => {
    const bounds = panel.getBoundingClientRect();
    const timeline = document.querySelector(".stage-timeline").getBoundingClientRect();
    return bounds.top >= 0 && bounds.right <= innerWidth && bounds.bottom < timeline.top
      && panel.scrollWidth <= panel.clientWidth + 1 && getComputedStyle(panel).overflowY === "auto";
  }));
  assert.equal(await page.locator(".music-scene-pick").count(), 13);
  await wait(async () => page.locator(".music-scene-pick img").evaluateAll(images => images.every(image => image.complete && image.naturalWidth > 0)));
  await page.locator("#scene-favorite-4").click();
  assert.equal(await page.locator("#scene-favorite-4").getAttribute("aria-pressed"), "true");
  await page.locator("#scene-pick-4").click();
  await wait(async () => { const state = await graphics(); return state.blend > 0 && state.blend < 0.98; });
  await page.locator("#scene-color").selectOption("color");
  await wait(async () => (await graphics()).blend === 1);
  await page.screenshot({ path: path.join(base, "music-scenes-900.png"), animations: "disabled" });
  const pixels = () => page.locator(".music-surface canvas").evaluate(canvas => {
    const gl = canvas.getContext("webgl2");
    const data = new Uint8Array(canvas.width * canvas.height * 4);
    gl.readPixels(0, 0, canvas.width, canvas.height, gl.RGBA, gl.UNSIGNED_BYTE, data);
    let hash = 2166136261, peak = 0;
    for (let offset = 0; offset < data.length; offset += 68) {
      const value = data[offset] + data[offset + 1] + data[offset + 2];
      peak = Math.max(peak, value);
      hash = Math.imul(hash ^ value, 16777619);
    }
    return { hash, peak };
  });
  const signatures = new Set();
  for (const [index, name] of [[8, "moire"], [9, "phosphor"], [10, "cassette"], [11, "nacre"], [12, "vector"]]) {
    await stage().locator("#stage-seek").evaluate(range => { range.value = "12"; range.dispatchEvent(new Event("input", { bubbles: true })); });
    await page.locator(`#scene-pick-${index}`).click();
    await wait(async () => { const state = await graphics(); return state.scene === index && state.blend === 1 && state.signal[3] > .25; });
    const first = await pixels();
    assert.ok(first.peak > 120);
    await wait(async () => (await pixels()).hash !== first.hash);
    signatures.add(first.hash);
    await page.keyboard.press("Escape");
    await page.screenshot({ path: path.join(base, `music-${name}-900.png`), animations: "disabled" });
    await clickStage("scenes");
  }
  assert.equal(signatures.size, 5);
  await page.locator("#scene-pick-4").click();
  async function measure(quality) {
    await page.locator("#scene-quality").selectOption(quality);
    await wait(async () => (await graphics()).blend === 1);
    const from = await page.evaluate(() => performance.now());
    await page.waitForTimeout(2300);
    const samples = (await graphics()).timestamps.filter(time => time >= from);
    return Math.round(1000 * (samples.length - 1) / (samples.at(-1) - samples[0]));
  }
  const autoFps = await measure("auto");
  const lowFps = await measure("low");
  await page.locator("#scene-quality").selectOption("off");
  const stillFrames = (await graphics()).frames;
  await page.waitForTimeout(250);
  assert.equal((await graphics()).frames, stillFrames);
  await page.locator("#scene-quality").selectOption("low");
  await page.keyboard.press("Escape");
  await stage().locator("#stage-seek").evaluate(range => { range.value = "5"; range.dispatchEvent(new Event("input", { bubbles: true })); });
  await wait(async () => { const { time, paused } = await playing(); return !paused && time > 5 && time < 10 && (await graphics()).signal[3] < 0.1; }, 8000);
  await stage().locator("#stage-volume").evaluate(range => { range.value = "0"; range.dispatchEvent(new Event("input", { bubbles: true })); });
  await stage().locator("#stage-seek").evaluate(range => { range.value = "12"; range.dispatchEvent(new Event("input", { bubbles: true })); });
  await wait(async () => (await graphics()).signal[3] > 0.25);
  assert.equal((await playing()).volume, 1);
  assert.equal(await stage().locator("#stage-volume").inputValue(), "0");
  await stage().locator("#stage-volume").evaluate(range => { range.value = "0.36"; range.dispatchEvent(new Event("input", { bubbles: true })); });
  await clickStage("queue");
  assert.equal(await stage().locator("#stage-panel-queue [aria-current]").textContent(), "Señal para la cola");
  await page.keyboard.press("Escape");
  await clickStage("immerse");
  assert.equal(await stage().getAttribute("data-controls"), "hidden");
  await page.keyboard.press("Tab");
  assert.equal(await stage().getAttribute("data-controls"), "visible");
  const beforeCompact = await playing();
  await clickStage("back");
  await stage().waitFor({ state: "hidden" });
  const rendered = (await graphics()).frames;
  await wait(async () => (await playing()).time > beforeCompact.time + 0.4);
  assert.equal((await graphics()).frames, rendered);
  assert.equal((await playing()).src, beforeCompact.src);
  await page.locator(".audio-visualize").click();
  await wait(async () => (await graphics()).frames > rendered + 3);
  await clickStage("scenes");
  assert.equal(await page.locator("#scene-pick-4").getAttribute("aria-pressed"), "true");
  await page.keyboard.press("Escape");
  const isMinimized = () => page.evaluate(async () => {
    const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js");
    return getCurrentWindow().isMinimized();
  });
  windowCommand("minimize");
  try {
    await wait(isMinimized, 8000);
    await page.waitForTimeout(650);
    const minimizedFrames = (await graphics()).frames;
    const minimizedTime = (await playing()).time;
    await wait(async () => (await playing()).time > minimizedTime + 0.5);
    assert.equal((await graphics()).frames, minimizedFrames);
  } finally { windowCommand("restore"); }
  await wait(async () => !(await isMinimized()), 8000);
  const restoredFrames = (await graphics()).frames;
  await wait(async () => (await graphics()).frames > restoredFrames + 2);
  const result = await page.locator(".music-surface canvas").evaluate(canvas => {
    const gl = canvas.getContext("webgl2");
    const debug = gl.getExtension("WEBGL_debug_renderer_info");
    return { width: canvas.width, height: canvas.height, renderer: debug ? gl.getParameter(debug.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
      glError: gl.getError() };
  });
  assert.equal(result.glError, 0);
  const lossTime = (await playing()).time;
  await page.locator(".music-surface canvas").evaluate(canvas => {
    window.sceneLossExtension = canvas.getContext("webgl2").getExtension("WEBGL_lose_context");
    window.sceneLossExtension.loseContext();
  });
  await page.locator('.music-surface[data-fallback="true"]').waitFor();
  await wait(async () => (await playing()).time > lossTime + 0.4);
  await page.evaluate(() => window.sceneLossExtension.restoreContext());
  await page.locator('.music-surface[data-fallback="false"]').waitFor();
  await clickStage("back");
  console.log(JSON.stringify({ music: true, mutedSignal: true, silence: true, pausedGraphicsInCompact: true,
    pausedGraphicsWhileMinimized: true, contextLossKeepsAudio: true, autoFps, lowFps, graphics: result }));

  for (const { file } of audioSamples) {
    await play(item(file).id);
    await wait(async () => { const current = await playing(); return current.id === item(file).id && current.time > 0.15; });
    if (path.extname(file) === ".wav") {
      await page.locator(".audio-visualize").click();
      assert.equal(await stage().locator(".music-identity h1").textContent(), "Audio 1");
      assert.equal(await stage().locator(".music-identity :is(img, p)").count(), 0);
      await wait(async () => { const { signal } = await graphics(); return signal[1] > 0.09 && signal[0] < 0.045 && signal[3] > 0.23; });
      await clickStage("back");
    }
    console.log(JSON.stringify({ container: path.extname(file), codecs: clip(file).map(stream => stream.codec_name), outcome: "audio" }));
  }

  const selectedTracks = [item(music), ...audioSamples.map(({ file }) => item(file))];
  for (const track of selectedTracks) await page.locator(`[data-select-media="${track.id}"]`).check();
  await page.locator('[data-action="play-selection"]').click();
  const first = (await playing()).id;
  await page.locator("#audio-mini").click();
  await page.locator('#playback-stage[data-mini="audio"]').waitFor();
  await page.locator("#mini-wheel").click();
  const wheel = page.locator("#track-wheel");
  const selection = wheel.locator('.track-wheel-card[data-offset="0"]');
  assert.equal(await selection.getAttribute("aria-current"), "true");
  await page.keyboard.press("ArrowDown");
  assert.equal(await selection.getAttribute("data-position"), "1");
  await page.keyboard.press("Escape");
  assert.equal((await playing()).id, first);
  assert.equal(await page.locator("#mini-wheel").evaluate(button => button === document.activeElement), true);
  await page.mouse.move(440, 240);
  await page.keyboard.down("Alt");
  await page.mouse.wheel(0, 120);
  await selection.waitFor();
  assert.equal(await selection.getAttribute("data-position"), "1");
  await page.mouse.wheel(0, 120);
  await wait(async () => (await selection.getAttribute("data-position")) === "2");
  assert.equal((await playing()).id, first);
  const title = await selection.locator("strong").textContent();
  const chosen = selectedTracks.find(track => (track.details?.audio?.title || path.parse(track.name).name) === title);
  assert.ok(chosen);
  const bounds = await wheel.boundingBox();
  assert.ok(bounds.x > 400 && bounds.y >= 0 && bounds.height > 420);
  await wait(async () => {
    const artwork = await selection.locator(".audio-cover").boundingBox();
    return artwork && artwork.width > 128 && artwork.height > 128;
  });
  assert.equal(await wheel.evaluate(node => getComputedStyle(node).backgroundColor), "rgba(0, 0, 0, 0)");
  await wheel.screenshot({ path: path.join(output, "quick-skip.png"), animations: "disabled" });
  await page.keyboard.up("Alt");
  await wheel.waitFor({ state: "hidden" });
  await wait(async () => { const current = await playing(); return current.id === chosen.id && !current.paused; });
  assert.equal(await original.evaluate(element => element === document.getElementById("audio-engine")), true);
  await wait(() => page.locator(".mini-monitor canvas").evaluate(canvas => {
    const { data } = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height / 2 - 3);
    return data.some((value, index) => index % 4 === 3 && value > 0);
  }));
  await stage().screenshot({ path: path.join(output, "mini-audio.png"), animations: "disabled" });
  if (desktopGesture) windowCommand("minimize");
  else await page.locator("#minimize").click();
  await wait(async () => await mainMinimized() && await page.evaluate(() => innerWidth <= 400));
  await page.locator("#mini-wheel").click();
  await selection.waitFor();
  await page.keyboard.press("ArrowUp");
  const detachedTitle = await selection.locator("strong").textContent();
  await page.keyboard.press("Enter");
  await wait(async () => (await page.locator("#mini-title").textContent()) === detachedTitle && !(await playing()).paused);
  await page.mouse.move(72, 87);
  await page.keyboard.down("Alt");
  await page.mouse.wheel(0, 120);
  await selection.waitFor();
  assert.equal(await selection.locator("strong").textContent(), title);
  await page.keyboard.up("Alt");
  await wait(async () => (await playing()).id === chosen.id && !(await playing()).paused);
  const miniVolume = page.locator("#mini-volume");
  await miniVolume.fill("0.65");
  await wait(() => page.evaluate(() => JSON.parse(localStorage.getItem("nabnab.playback.v1")).preferences.volume === .65));
  await stage().screenshot({ path: path.join(output, "mini-audio-detached.png"), animations: "disabled" });
  await page.locator("#mini-dock").click();
  await wait(async () => !await mainMinimized());
  await stage().waitFor({ state: "hidden" });
  assert.equal(await page.locator("#audio-mini").evaluate(button => button === document.activeElement), true);
  await page.locator('[data-action="clear-library-selection"]').click();
  console.log("Both floating mini players preserved the media element with the main window minimized. Alt + wheel worked in the main and detached windows.");

  if (desktopGesture) {
    if (!(await playing()).paused) await page.locator("#audio-toggle").click();
    windowCommand("minimize");
    await wait(mainMinimized);
    try { await verifyDesktopWheel(original, selectedTracks); }
    finally { windowCommand("restore"); }
  }

  await play(musicId);
  await wait(async () => { const current = await playing(); return current.id === musicId && !current.paused && current.time > 0.15; });
  const continuous = await playing();
  for (let iteration = 0; iteration < 3; iteration++) {
    await page.locator(".audio-visualize").click();
    assert.equal(await page.locator("audio, video").count(), 1);
    await clickStage("back");
    await stage().waitFor({ state: "hidden" });
    const current = await playing();
    assert.equal(current.src, continuous.src);
    assert.equal(current.paused, false);
    assert.ok(current.time >= continuous.time);
  }
  if (process.env.NABNAB_PLAYBACK_LIVE_DOWNLOAD === "1") await downloadDuringPlayback(musicId);
  await verifyAudioTracks(original, videoId);

  for (const file of [webm, mkv, ...(av1 ? [av1] : [])]) {
    await play(item(file).id);
    await wait(async () => { const current = await playing(); return current.error || current.width > 0; });
    const current = await playing();
    if (file === webm) assert.deepEqual([current.width, current.height], [720, 248]);
    if (file === mkv) await verifyEmbeddedCaptions(original, item(mkv).id);
    if (current.error) await stage().locator(".stage-error").waitFor();
    console.log(JSON.stringify({ container: path.extname(file), codecs: clip(file).map(stream => stream.codec_name),
      outcome: current.error ? `media-error-${current.error}` : `${current.width}x${current.height}` }));
    await clickStage("back");
  }
  await play(item(broken).id);
  await stage().locator(".stage-error").waitFor();
  await stage().getByRole("button", { name: "Convertir una copia" }).waitFor();
  await clickStage("back");
  await page.locator("#library-sort").selectOption("name");
  const webmId = item(webm).id, brokenId = item(broken).id;
  for (const id of [webmId, brokenId]) await page.locator(`[data-select-media="${id}"]`).check();
  await original.evaluate((media, id) => {
    const observer = new MutationObserver(() => {
      if (media.dataset.localPlayer !== id) return;
      observer.disconnect();
      window.loadingReplacement = media.readyState;
      document.getElementById("playback-stage").dispatchEvent(new KeyboardEvent("keydown", {
        key: "ArrowLeft", ctrlKey: true, bubbles: true,
      }));
    });
    observer.observe(media, { attributes: true, attributeFilter: ["data-local-player"] });
  }, brokenId);
  await play(brokenId);
  await wait(async () => { const current = await playing(); return current.id === webmId && current.width === 720 && current.time > 0.2; });
  assert.equal(await page.evaluate(() => window.loadingReplacement), 0);
  assert.equal(await stage().locator(".stage-error").isVisible(), false);
  await clickStage("back");
  for (const id of [webmId, brokenId]) await page.locator(`[data-select-media="${id}"]`).uncheck();
  await rename(missing, `${missing}.moved`);
  await play(item(missing).id);
  await stage().getByText("El archivo no está disponible.", { exact: false }).waitFor();
  await clickStage("back");
  await play(videoId);
  await stage().locator("#stage-resume").waitFor();
  await clickStage("resume");
  await wait(async () => (await playing()).time > 10 && !(await playing()).paused);
  await clickStage("back");
  await wait(async () => (await rpc("get_snapshot")).library.find(candidate => candidate.id === videoId).details.playbackPosition >= 10);
  assert.deepEqual(errors, []);
  assert.deepEqual(unexpectedNetwork, []);
  await shutdown();
  const audioLessonId = item(audioSamples[0].file).id;
  const handoutId = item(handout).id;
  const bookIds = audioSamples.slice(1).map(sample => item(sample.file).id);
  await seedPlaybackFixtures(videoId, audioLessonId, handoutId, bookIds);
  await launch();
  await page.locator("#audio-player").waitFor();
  await wait(async () => (await playing()).time >= 10);
  assert.equal((await playing()).paused, true);
  assert.equal((await playing()).id, videoId);
  assert.equal((await playing()).volume, 0.65);
  console.log(JSON.stringify({ sharedElement: true, compact: true, video: true, hidControlsAfterMouseClick: true, captions: true,
    fullscreen: true, restoredOnRestart: true, playlistAudioCover: true, viewports: [1180, 900], rapidSeek: true,
    replacedLoadingSource: true, compactRoundTrips: 3, externalWebViewRequests: unexpectedNetwork.length }));
  await verifyCoursePlayback(videoId, musicId, audioLessonId, handoutId);
  await verifyBookContinuation(bookIds);
} catch (error) {
  console.error(log);
  console.error(await page?.locator("#playback-stage").innerText().catch(() => ""));
  console.error(await page?.locator("#notification").textContent().catch(() => ""));
  throw error;
} finally {
  await shutdown();
}
