import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { copyFile, mkdir, mkdtemp, readdir, stat, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = path.resolve(import.meta.dirname, "..");
const artifacts = process.env.NABNAB_ARCHIVE_ARTIFACTS || path.join(root, "artifacts/native-personal-archives");
await mkdir(artifacts, { recursive: true });
const data = await mkdtemp(path.join(artifacts, "run-"));
const id = "a".repeat(64), generation = `${id}-00000001`;
const instagram = "c".repeat(64), instagramGeneration = `${instagram}-00000001`;
const managed = path.join(data, "personal-archive-media", id);
const secrets = path.join(data, "module-secrets");
const records = path.join(data, "personal-archive-data");
await Promise.all([mkdir(managed, { recursive: true }), mkdir(secrets), mkdir(records)]);
const digest = value => createHash("sha256").update(value).digest("hex");
const row = (source) => ({
  id: digest(source), source, title: path.posix.basename(source), text: "", time: null, link: null,
  author: null, threadParent: null, playedMs: null, reachedEnd: null, completed: null, tracks: [], media: source,
});
const [photo, video, invalid, privatePhoto] = [
  "data/tweets_media/100-Paisaje.png", "data/tweets_media/100-Escena.mp4",
  "data/tweets_media/999-Lista.mp4", "data/direct_messages_media/100-Foto privada.png",
].map(row);
const post = (id, title, replyTo = null) => ({ ...row("data/tweets.js"), id: digest(id), title, text: title,
  time: Date.UTC(2025, 0, Number(id) - 99), threadParent: replyTo, media: null });
const first = post("100", "Salida al lago"), reply = post("101", "La segunda publicación", "100");
const instagramMedia = [row("media/posts/202501/123-Lago.png"), row("media/posts/202501/Paseo.mp4")];
const instagramPost = {...post("300", "Paseo por la sierra"), source: "your_instagram_activity/media/posts_1.json",
  attachmentIds: instagramMedia.map(item => item.id)};
const file = item => path.join(managed, digest(item.source) + path.extname(item.source));
const ffmpeg = path.join(root, "src-tauri/engines/ffmpeg.exe");
function encode(args) {
  execFileSync(ffmpeg, ["-hide_banner", "-v", "error", ...args], { windowsHide: true });
}
encode(["-f", "lavfi", "-i", "smptebars=size=1080x720", "-frames:v", "1", file(photo)]);
encode(["-f", "lavfi", "-i", "testsrc2=size=360x640:duration=1", "-c:v", "libopenh264", file(video)]);
encode(["-f", "lavfi", "-i", "color=orange:size=120x80", "-frames:v", "1", file(privatePhoto)]);
await writeFile(file(invalid), "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nhttps://example.invalid/private.ts\n#EXT-X-ENDLIST\n");
const bytes = await Promise.all([photo, video, invalid, privatePhoto].map(async item => (await stat(file(item))).size));
const instagramDirectory = path.join(data, "personal-archive-media", instagram);
await mkdir(instagramDirectory, { recursive: true });
for (const [index, item] of instagramMedia.entries()) {
  await copyFile(file([photo,video][index]), path.join(instagramDirectory, item.id + path.extname(item.source)));
}
async function protect(directory, name, header, purpose, value) {
  const sealed = execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `
    Add-Type -AssemblyName System.Security
    $bytes = [Convert]::FromBase64String($env:NABNAB_ARCHIVE_FIXTURE)
    $entropy = [Text.Encoding]::UTF8.GetBytes('nabnab-cookie-v1:${purpose}')
    $sealed = [Security.Cryptography.ProtectedData]::Protect($bytes, $entropy, [Security.Cryptography.DataProtectionScope]::CurrentUser)
    [Console]::OpenStandardOutput().Write($sealed, 0, $sealed.Length)
  `], { windowsHide: true, env: { ...process.env, NABNAB_ARCHIVE_FIXTURE: Buffer.from(JSON.stringify(value)).toString("base64") } });
  await writeFile(path.join(directory, `${name}.bin`), Buffer.concat([Buffer.from(`${header}\n`), sealed]));
}
await protect(secrets, "personal-archive-index", "nabnab-module-secret-v1", "nabnab-module:personal-archive-index", [{
  id, generation: 1, provider: "x", account: "Cuenta de prueba", destination: "Álbum de 2025", source: "x-export.zip",
  fingerprint: "b".repeat(64), categories: { posts: 2, media: 3, messages: 1 }, formats: { posts: ["xAssignment"], media: ["media"], messages: ["media"] },
  mediaBytes: bytes.reduce((total, size) => total + size), pendingRemoval: false,
}, {
  id:instagram, generation:1, provider:"instagram", account:"Archivo de montaña", destination:"Viajes", source:"instagram-export.zip",
  fingerprint:"d".repeat(64), categories:{posts:1,media:2}, formats:{posts:["json"],media:["media"]},
  mediaBytes:bytes[0]+bytes[1], pendingRemoval:false,
}]);
await protect(records, generation, "nabnab-personal-archive-v1", `nabnab-personal-archive:${generation}`, {
  posts: [first, reply], media: [photo, video, invalid], messages: [privatePhoto],
});
await protect(records, instagramGeneration, "nabnab-personal-archive-v1", `nabnab-personal-archive:${instagramGeneration}`, {
  posts:[instagramPost], media:[...instagramMedia].reverse(),
});

const port = process.env.NABNAB_ARCHIVE_DEBUG_PORT || 9269;
const endpoint = `http://127.0.0.1:${port}`;
const child = spawn(process.env.NABNAB_ARCHIVE_TEST_EXE || path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
  cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
  env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
});
let browser, log = "";
for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => { log = (log + chunk).slice(-4000); });
async function wait(check) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (child.exitCode !== null) throw Error(`nabnab exited ${child.exitCode}.`);
    try { const result = await check(); if (result) return result; } catch { /* Startup may still be in progress. */ }
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  throw Error("Timed out waiting for the native archive fixture.");
}

try {
  browser = await wait(async () => (await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(400) })).ok
    && await chromium.connectOverCDP(endpoint, { timeout: 2000 }));
  const page = await wait(() => browser.contexts()[0]?.pages().find(tab => /:1420\/$/.test(tab.url())));
  const rpc = (command, args) => page.evaluate(async ({ command, args }) => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    return invoke(command, args);
  }, { command, args });
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.locator("[data-page='library']").click();
  await page.locator("[data-section='archives']").click();
  await page.getByRole("button", { name: /x-export.zip/ }).click();
  const attachmentButton = page.getByRole("button", { name: "2 adjuntos" });
  await attachmentButton.waitFor();
  assert.equal(await page.locator('[data-action="archive-attachments"]').count(), 1);
  await attachmentButton.press("Enter");
  const attachmentTitle = page.getByRole("heading", { name: "Adjuntos de la publicación" });
  await attachmentTitle.waitFor();
  assert.ok(await attachmentTitle.evaluate(node => node === document.activeElement));
  assert.deepEqual(await page.locator(".archive-attachment-name strong").allTextContents(), ["Escena.mp4", "Paisaje.png"]);
  await page.screenshot({ path: path.join(data, "x-post-attachments.png") });
  await page.keyboard.press("Escape");
  assert.ok(await attachmentButton.evaluate(button => button === document.activeElement));
  await page.locator(`[data-action="archive-thread"][data-id="${first.id}"]`).click();
  await attachmentButton.click();
  await page.getByRole("button", { name: "Volver al hilo" }).waitFor();
  const preview = item => page.locator(`[data-action="archive-media-preview"][data-id="${item.id}"]`);
  for (const [item, dimensions] of [[photo, [900, 600]], [video, [338, 600]]]) {
    await preview(item).click();
    const image = page.getByRole("img", { name: item.title.replace(/^100-/, "") });
    await image.waitFor();
    await page.waitForFunction(() => {
      const image = document.querySelector(".archive-media-stage img");
      const bounds = image?.getBoundingClientRect();
      return image?.naturalWidth > 0 && bounds.top >= 0 && bounds.bottom <= innerHeight;
    });
    assert.deepEqual(await image.evaluate(node => [node.naturalWidth, node.naturalHeight]), dimensions);
    await page.screenshot({ path: path.join(data, `${path.parse(item.title).name}-preview.png`) });
    await page.keyboard.press("Escape");
    assert.ok(await preview(item).evaluate(button => button === document.activeElement));
  }
  await page.getByRole("button", { name: "Volver al hilo" }).click();
  assert.ok(await attachmentButton.evaluate(button => button === document.activeElement));
  await assert.rejects(rpc("personal_archive_attachments", { id, post: digest("999"), offset: 0 }), /post is no longer/);
  await page.locator('[data-action="archive-category"][data-id="media"]').click();
  await preview(invalid).click();
  await page.getByRole("alert").filter({ hasText: "No se pudo mostrar la vista previa" }).waitFor();
  await page.getByRole("button", { name: "Volver a los registros" }).click();
  const request = (item, category) => ({ id, category, record: item.id, consent: false, request: randomUUID() });
  await assert.rejects(rpc("personal_archive_preview_media", request(privatePhoto, "messages")), /private category explicitly/);
  await assert.rejects(rpc("personal_archive_preview_media", request(privatePhoto, "media")), /included in this archive/);
  await page.locator('[data-action="archive-category"][data-id="messages"]').click();
  await page.getByRole("button", { name: "Mostrar categoría privada" }).click();
  await preview(privatePhoto).click();
  await page.getByRole("img", { name: privatePhoto.title }).waitFor();
  await page.locator("[data-section='files']").click();
  await page.locator("[data-section='archives']").click();
  await page.getByRole("button", { name: "Mostrar categoría privada" }).waitFor();
  assert.equal(await page.locator(".archive-media-stage img").count(), 0);
  assert.equal((await readdir(managed)).length, 4);
  await page.getByRole("button", { name: /instagram-export.zip/ }).click();
  const instagramAttachments = page.getByRole("button", {name:"2 adjuntos",exact:true});
  await instagramAttachments.press("Enter");
  await attachmentTitle.waitFor();
  assert.deepEqual(await page.locator(".archive-attachment-name strong").allTextContents(), ["123-Lago.png", "Paseo.mp4"]);
  for (const [width,height] of [[1180,780],[900,620]]) {
    await page.setViewportSize({width,height});
    const panel = page.locator(".archive-attachments");
    assert.ok(await panel.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await panel.scrollIntoViewIfNeeded();
    await page.screenshot({path:path.join(data, `instagram-attachments-${width}.png`),animations:"disabled"});
  }
  await preview(instagramMedia[0]).press("Enter");
  const instagramImage = page.getByRole("img", {name:"123-Lago.png"});
  await instagramImage.waitFor();
  await page.waitForFunction(() => document.querySelector(".archive-media-stage img")?.naturalWidth > 0);
  assert.deepEqual(await instagramImage.evaluate(node => [node.naturalWidth,node.naturalHeight]), [900,600]);
  await page.getByRole("button", {name:"Volver a adjuntos"}).click();
  assert.ok(await preview(instagramMedia[0]).evaluate(node => node === document.activeElement));
  await page.getByRole("button", {name:"Volver a publicaciones"}).click();
  assert.ok(await instagramAttachments.evaluate(node => node === document.activeElement));
  assert.deepEqual(errors, []);
  console.log(`Verified X and Instagram post attachments, native image and video previews, private reveal and unmatched media isolation in ${data}`);
} catch (error) {
  console.error(log);
  throw error;
} finally {
  await browser?.close().catch(() => {});
  if (child.exitCode === null) {
    const exited = new Promise(resolve => child.once("exit", resolve));
    child.kill();
    await exited;
  }
}
