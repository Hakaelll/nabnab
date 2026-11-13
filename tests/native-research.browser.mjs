// Opt-in live verification. Vite and the Windows debug executable must be available.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = path.resolve(import.meta.dirname, "..");
const base = path.resolve(process.env.NABNAB_RESEARCH_ARTIFACTS ?? path.join(root, "artifacts/native-research"));
await mkdir(base, { recursive: true });
const data = await mkdtemp(path.join(base, "run-"));
const output = path.join(data, "output");
await mkdir(output);
const languageOnly = process.env.NABNAB_RESEARCH_LANGUAGE_ONLY === "1";
const chaptersOnly = process.env.NABNAB_RESEARCH_CHAPTER_FILTERS_ONLY === "1";
const communityOnly = process.env.NABNAB_RESEARCH_COMMUNITY_ONLY === "1";
const zenodoOnly = process.env.NABNAB_RESEARCH_ZENODO_ONLY === "1";
const arxivDetailsOnly = process.env.NABNAB_RESEARCH_ARXIV_DETAILS_ONLY === "1";
const allProviders = !languageOnly && !chaptersOnly && !communityOnly && !zenodoOnly && !arxivDetailsOnly;
const port = process.env.NABNAB_RESEARCH_DEBUG_PORT || 9259;
const endpoint = `http://127.0.0.1:${port}`;
const child = spawn(process.env.NABNAB_RESEARCH_TEST_EXE ?? path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
  cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
  env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
});
let browser, page, log = "";
let requestId = Date.now();
const results = [];
const errors = [];
for (const stream of [child.stdout, child.stderr]) stream.on("data", bytes => { log = (log + bytes).slice(-5000); });

async function wait(check, timeout = 25_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (child.exitCode !== null) throw Error(`Native app exited ${child.exitCode}`);
    const value = await check();
    if (value) return value;
    await new Promise(resolve => setTimeout(resolve, 150));
  }
  throw Error("Timed out waiting for the public source.");
}
const rpc = (name, args = {}) => page.evaluate(async ({ name, args }) => {
  const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
  return invoke(name, args);
}, { name, args });
const inspect = url => rpc("public_source_inspect", { url, cursor: null, requestId: ++requestId });
const getSnapshot = () => rpc("get_snapshot");

async function queue(source, asset, size, kind) {
  assert.ok(size > 0 && size <= 2 * 1024 * 1024);
  const jobs = await rpc("queue_public_files", { sourceUrl: source.sourceUrl, assetIds: [asset.id], collection: "Prueba de fuentes públicas", clientRequest: randomUUID() });
  assert.equal(jobs.length, 1);
  const { job, item } = await wait(async () => {
    const snapshot = await getSnapshot();
    const job = snapshot.jobs.find(value => value.id === jobs[0].id);
    if (job?.state === "error") throw Error(JSON.stringify(job.error));
    if (job?.state !== "completed") return null;
    return { job, item: snapshot.library.find(value => value.path === job.outputs[0]) };
  }, 80_000);
  assert.equal(job.outputs.length, 1);
  assert.equal(item.kind, kind);
  assert.equal(item.details.contentId, asset.id);
  assert.equal(item.details.sourceUrl, asset.sourceUrl);
  assert.equal(item.details.creator, asset.author);
  assert.equal(item.details.license, asset.license);
  const bytes = await readFile(item.path);
  assert.equal(bytes.length, size);
  if (asset.checksum) {
    const [algorithm, digest] = asset.checksum.split(":");
    assert.equal(createHash(algorithm).update(bytes).digest("hex"), digest);
  }
  return { item, bytes };
}

async function verify(provider, run) {
  try {
    const detail = await run();
    results.push(provider);
    console.log(JSON.stringify({ provider, ...detail }));
  } catch (error) {
    errors.push(error);
    const notices = await page?.locator(".file-source-error, #notification-text").allTextContents().catch(() => []);
    console.error(JSON.stringify({ provider, error: String(error), notices }));
  }
}

try {
  browser = await wait(async () => {
    try {
      if ((await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(400) })).ok) return await chromium.connectOverCDP(endpoint);
    } catch { /* WebView2 is still starting. */ }
    return null;
  });
  page = await wait(() => browser.contexts()[0]?.pages().find(tab => /:1420/.test(tab.url())));
  const initial = await wait(async () => { try { return await getSnapshot(); } catch { return null; } });
  assert.equal(initial.jobs.length, 0);
  await rpc("save_settings", { settings: { ...initial.settings, language: "es", folder: output, reducedMotion: true, trayEnabled: false, closeToTray: false } });
  await page.reload();

  if (communityOnly) await verify("Zenodo community", async () => {
    const url = "https://zenodo.org/communities/cfa";
    const title = "Harvard-Smithsonian Center for Astrophysics";
    const stableUrl = "https://zenodo.org/communities/e4064eef-ce1b-4e34-b801-9f9570159e58";
    await page.locator("#url").fill(url);
    await page.locator("#load").click();
    const communityHeading = page.getByRole("heading", { name: title, exact: true });
    await communityHeading.waitFor({ timeout: 60_000 });
    const records = page.locator(".file-source-entry");
    const ids = await records.evaluateAll(entries => entries.map(entry => entry.dataset.id));
    assert.ok(ids.length > 0 && ids.length <= 20);
    assert.equal(new Set(ids).size, ids.length);
    assert.ok(ids.every(id => /^zenodo:\d+$/.test(id)));
    for (const [width, height] of [[1180, 780], [900, 620]]) {
      await page.setViewportSize({ width, height });
      await communityHeading.scrollIntoViewIfNeeded();
      assert.ok(await page.locator(".file-sources").evaluate(node => node.scrollWidth <= node.clientWidth + 1));
      await page.screenshot({ path: path.join(data, `zenodo-community-${width}.png`) });
    }
    const first = records.first();
    await first.press("Enter");
    const back = page.getByRole("button", { name: "Volver a la comunidad", exact: true });
    await back.waitFor();
    await page.getByText(/^Registro \d+|^Versión .*Registro \d+/).first().waitFor({ timeout: 60_000 });
    assert.equal(await page.locator("[data-file-source-select]:checked").count(), 0);
    await back.press("Enter");
    assert.equal(await first.evaluate(element => element === document.activeElement), true);
    await page.getByRole("button", { name: "Seguir comunidad", exact: true }).click();
    await page.getByRole("button", { name: "Añadir y explorar", exact: true }).click();
    const saved = await wait(async () => (await getSnapshot()).sources.find(source => source.url === stableUrl && source.lastCheck), 90_000);
    assert.equal(saved.settings.automatic, false);
    assert.equal(saved.settings.intervalHours, null);
    assert.equal(saved.items.length, ids.length);
    assert.ok(saved.items.every(item => item.state === "review" && !item.jobIds.length));
    const same = await rpc("add_source", { url: stableUrl, settings: saved.settings, kind: "publicSearch", publicFeed: false });
    assert.equal(same.id, saved.id);
    assert.equal((await getSnapshot()).sources.length, 1);
    assert.equal((await getSnapshot()).jobs.length, 0);
    return { records: ids.length, stableUrl, queued: 0 };
  });

  if (languageOnly || allProviders) await verify("LibriVox language", async () => {
    await page.locator('[data-page="platforms"]').click();
    await page.locator('[data-action="open-fileSources"][data-id="librivox"]').click();
    await page.getByRole("combobox", { name: "Idioma", exact: true }).selectOption({ label: "Español" });
    await page.locator("#file-source-catalog-form button[type=submit]").click();
    const entries = page.locator('[data-action="file-source-entry"]');
    await entries.first().waitFor({ timeout: 45_000 });
    const count = await entries.count();
    assert.ok(count > 0 && count <= 25);
    const ids = await entries.evaluateAll(buttons => buttons.map(button => button.dataset.id));
    assert.equal(new Set(ids).size, count);
    await page.locator("#file-source-catalog-form").scrollIntoViewIfNeeded();
    await page.screenshot({ path: path.join(data, "librivox-language.png") });
    const more = page.getByRole("button", { name: "Cargar más", exact: true });
    assert.equal(await more.getAttribute("data-cursor"), "2");
    await more.click();
    await page.locator('[data-action="file-source-more"][data-cursor="3"]').waitFor({ timeout: 45_000 });
    const total = await entries.count();
    assert.ok(total >= count && total <= 50);
    const allIds = await entries.evaluateAll(buttons => buttons.map(button => button.dataset.id));
    assert.equal(new Set(allIds).size, total);
    const first = await entries.first().getAttribute("data-id");
    await entries.first().press("Enter");
    await page.locator("[data-file-source-select]").first().waitFor({ timeout: 60_000 });
    await page.getByRole("button", { name: "Volver a resultados" }).click();
    assert.equal(await entries.count(), total);
    assert.equal(await entries.first().evaluate(button => button === document.activeElement), true);
    await page.getByRole("button", { name: "Seguir idioma" }).click();
    await page.locator("#source-exploration-limit").fill("25");
    await page.locator("#source-editor button[type=submit]").click();
    const followed = await wait(async () => {
      const { sources } = await getSnapshot();
      return sources.find(source => source.url === "https://librivox.org/search/?language_id=5" && source.items.length > 0);
    }, 55_000);
    assert.equal(followed.settings.intervalHours, null);
    assert.equal(followed.settings.automatic, false);
    assert.equal(followed.settings.pageSize, 25);
    assert.equal((await getSnapshot()).jobs.length, 0);
    return { firstPage: count, loaded: total, first, followed: followed.items.length };
  });

  if (chaptersOnly) await verify("LibriVox chapter filters", async () => {
    const editionUrl = "https://librivox.org/mini-multilingual-bedtime-stories-2-by-aesop/";
    const source = await inspect(editionUrl);
    const chapter = source.assets.find(asset => asset.audiobook?.chapter === 3);
    assert.equal(chapter.audiobook.language, "Spanish");
    assert.equal(chapter.audiobook.reader, "Alexandria Adair");
    assert.ok(chapter.size > 0 && chapter.size <= 2 * 1024 * 1024);
    await page.locator('[data-page="platforms"]').click();
    await page.locator('[data-action="open-fileSources"][data-id="librivox"]').click();
    await page.locator("#file-source-url").fill(editionUrl);
    await page.locator("#file-source-form button[type=submit]").click();
    const language = page.getByRole("combobox", { name: "Idioma del capítulo" });
    const narration = page.getByRole("combobox", { name: "Lectura" });
    await language.selectOption({ label: "Español" });
    await narration.selectOption(chapter.audiobook.reader);
    const checkboxes = page.locator("[data-file-source-select]");
    assert.deepEqual(await checkboxes.evaluateAll(boxes => boxes.map(box => box.dataset.fileSourceSelect)), [chapter.id]);
    await page.locator('[data-action="file-source-select-chapters"]').click();
    assert.equal(await checkboxes.first().isChecked(), true);
    await language.selectOption({ label: "Inglés" });
    assert.equal(await page.locator("[data-file-source-select]:checked").count(), 0);
    const review = page.getByRole("button", { name: "Solo seleccionados", exact: true });
    await review.press("Enter");
    assert.equal(await review.getAttribute("aria-pressed"), "true");
    assert.equal(await language.inputValue(), "English");
    assert.equal(await language.isDisabled(), true);
    assert.deepEqual(await checkboxes.evaluateAll(boxes => boxes.map(box => box.dataset.fileSourceSelect)), [chapter.id]);
    assert.equal(await checkboxes.first().isChecked(), true);
    assert.equal(await page.locator(".file-source-selection-title strong").textContent(), "1 seleccionado");
    await page.locator(".file-source-result-heading").evaluate(heading => heading.scrollIntoView({ block: "start" }));
    await page.locator(".file-source-results").screenshot({ path: path.join(data, "librivox-chapter-review.png") });
    await page.locator('[data-action="file-source-queue"]').click();
    const result = await wait(async () => {
      const snapshot = await getSnapshot();
      const job = snapshot.jobs[0];
      if (job?.state === "error") throw Error(job.error?.message ?? "The selected chapter could not be downloaded.");
      if (job?.state !== "completed") return null;
      return { jobs: snapshot.jobs, item: snapshot.library.find(item => item.path === job.outputs[0]) };
    }, 75_000);
    assert.equal(result.jobs.length, 1);
    assert.equal(result.item.kind, "audio");
    assert.deepEqual(result.item.details.audiobook, chapter.audiobook);
    assert.ok(chapter.audiobook.editionFiles >= 3);
    assert.equal((await readFile(result.item.path)).length, chapter.size);
    const openShelf = page.getByRole("button", { name: "Abrir en Estantería", exact: true });
    await openShelf.waitFor();
    assert.equal(await language.inputValue(), "English");
    const chapterCopies = page.getByRole("combobox", { name: "Copia local" });
    await chapterCopies.selectOption("saved");
    await language.selectOption({ label: "Español" });
    assert.deepEqual(await checkboxes.evaluateAll(boxes => boxes.map(box => box.dataset.fileSourceSelect)), [chapter.id]);
    await chapterCopies.selectOption("missing");
    assert.equal(await checkboxes.count(), 0);
    assert.equal(await openShelf.getAttribute("data-id"), chapter.audiobook.bookId);
    await openShelf.press("Enter");
    await page.waitForFunction(() => document.activeElement?.id === "audio-shelf-title");
    assert.equal(await page.locator("#audio-shelf-title").textContent(), chapter.audiobook.bookTitle);
    const edition = page.getByRole("region", { name: chapter.audiobook.bookTitle, exact: true });
    assert.match(await edition.innerText(), new RegExp(`1 de ${chapter.audiobook.editionFiles} disponibles`));
    assert.equal(await edition.getByRole("button", { name: "Ver edición en LibriVox" }).getAttribute("data-id"), chapter.audiobook.bookId.split(":")[1]);
    assert.deepEqual(await page.locator(".audio-row .audio-track-number").allTextContents(), ["Cap. 3"]);
    assert.equal(await page.locator("html.has-audio-player").count(), 0);
    assert.equal((await getSnapshot()).jobs.length, 1);
    return { chapter: chapter.audiobook.chapter, language: chapter.audiobook.language, reader: chapter.audiobook.reader, bytes: chapter.size, openedInBookshelf: true };
  });

  if (arxivDetailsOnly) await verify("arXiv version details", async () => {
    const article = "https://arxiv.org/abs/2506.08399";
    await page.getByRole("button", { name: "Inicio", exact: true }).click();
    await page.locator("#url").fill(article);
    await page.locator("#load").click();
    const versionTwo = page.getByRole("checkbox", { name: /^Versión 2 · / });
    await versionTwo.waitFor({ timeout: 60_000 });
    await versionTwo.check();
    const open = page.getByRole("button", { name: "Ficha de la versión 1", exact: true });
    await open.press("Enter");
    const details = page.getByRole("region", { name: "Ficha de la versión 1" });
    await details.waitFor({ timeout: 60_000 });
    assert.match(await details.innerText(), /arxiv:2506\.08399v1.*Jiachen Ma/);
    assert.equal(await details.evaluate(node => node === document.activeElement), true);
    await details.getByRole("button", { name: "Cerrar ficha" }).press("Enter");
    assert.equal(await open.evaluate(node => node === document.activeElement), true);
    assert.ok(await versionTwo.isChecked());
    assert.equal((await getSnapshot()).jobs.length, 0);
    await assert.rejects(rpc("queue_public_files", {
      sourceUrl: article + "v1", assetIds: ["arxiv:2506.08399v1"], collection: "", clientRequest: randomUUID(),
    }), /Inspect the source again/);
    return { exactVersion: "2506.08399v1", queueUnchanged: true };
  });

  if (allProviders) await verify("arXiv", async () => {
    const paper = "2506.08399";
    const source = await inspect(`https://arxiv.org/abs/${paper}`);
    assert.deepEqual(source.assets.map(asset => asset.version), ["v2", "v1"]);
    const saved = [];
    for (const version of ["v1", "v2"]) {
      const asset = source.assets.find(value => value.version === version);
      const headers = await fetch(`https://arxiv.org/pdf/${paper}${version}`, { method: "HEAD", signal: AbortSignal.timeout(15_000) });
      assert.ok(headers.ok);
      assert.match(headers.headers.get("content-type"), /^application\/pdf\b/i);
      const result = await queue(source, asset, Number(headers.headers.get("content-length")), "document");
      assert.equal(result.bytes.subarray(0, 5).toString(), "%PDF-");
      saved.push(result);
    }
    assert.notEqual(saved[0].item.path, saved[1].item.path);
    assert.notDeepEqual(saved[0].bytes, saved[1].bytes);
    assert.deepEqual(await readFile(saved[0].item.path), saved[0].bytes);
    return { versions: saved.map(value => value.item.name), bytes: saved.map(value => value.bytes.length) };
  });

  if (zenodoOnly || allProviders) await verify("Zenodo", async () => {
    const family = await inspect("14634896");
    assert.equal(family.zenodo.recordId, "14634897");
    assert.equal(family.zenodo.familyId, "14634896");
    const concept = await inspect("10.5281/zenodo.14634896");
    assert.equal(concept.zenodo.recordId, family.zenodo.recordId);
    const history = await inspect(family.zenodo.versionsUrl);
    assert.deepEqual(history.assets, []);
    const record = history.entries.find(entry => entry.id === "zenodo:14634897");
    assert.ok(record);
    const source = await inspect(record.url);
    const asset = source.assets.find(value => value.filename === "Fig_3b_left.csv");
    assert.equal(asset.size, 450);
    assert.equal(asset.checksum, "md5:e9cdc5528564d20d0516ef90a9e3a980");
    const { item } = await queue(source, asset, asset.size, "document");
    assert.equal(item.name, "Fig_3b_left.csv");
    const jobCount = (await getSnapshot()).jobs.length;
    await page.getByRole("button", { name: "Inicio", exact: true }).click();
    await page.locator("#url").fill("10.5281/zenodo.14634896");
    await page.locator("#load").click();
    const showFolder = page.getByRole("button", { name: "Mostrar carpeta de Fig_3b_left.csv", exact: true });
    await showFolder.waitFor({ timeout: 60_000 });
    assert.equal(await showFolder.getAttribute("data-id"), item.id);
    assert.match(await page.locator(".file-source-saved-copy").innerText(), /En Biblioteca · 450 B/);
    assert.equal(await page.locator(".file-source-saved-copy").count(), 1);
    const copyFilter = page.getByRole("combobox", { name: "Copia local" });
    await copyFilter.selectOption("saved");
    assert.equal(await page.locator("[data-file-source-select]").count(), 1);
    assert.equal(await page.getByRole("button", { name: "Marcar archivos sin copia" }).isDisabled(), true);
    const verifyCopy = page.getByRole("button", { name: "Verificar copia de Fig_3b_left.csv", exact: true });
    const result = page.locator(".file-source-saved-copy").getByRole("status");
    await verifyCopy.press("Enter");
    await result.getByText("El checksum coincide.", { exact: true }).waitFor();
    const original = await readFile(item.path);
    const edited = Buffer.from(original);
    edited[0] ^= 1;
    try {
      await writeFile(item.path, edited);
      await verifyCopy.press("Enter");
      await result.getByText("El checksum es distinto.", { exact: true }).waitFor();
      assert.deepEqual(await readFile(item.path), edited);
    } finally {
      await writeFile(item.path, original);
    }
    await copyFilter.selectOption("missing");
    assert.equal(await showFolder.count(), 0);
    assert.equal(await page.locator(`[data-file-source-select="${asset.id}"]`).count(), 0);
    assert.equal((await getSnapshot()).jobs.length, jobCount);
    return { family: family.zenodo.familyId, record: source.zenodo.recordId, filename: item.name, bytes: item.size, checksum: asset.checksum, savedCopyViaConceptDoi: true };
  });

  if (allProviders) await verify("LibriVox", async () => {
    const source = await inspect("https://librivox.org/rss/353");
    const chapters = source.assets.map(asset => asset.audiobook?.chapter);
    assert.ok(chapters.length > 2);
    assert.deepEqual(chapters, [...chapters].sort((a, b) => a - b));
    const assets = source.assets.filter(asset => asset.audiobook?.chapter > 0 && asset.size > 0 && asset.size <= 1024 * 1024).slice(0, 2);
    assert.equal(assets.length, 2);
    for (const asset of [...assets].reverse()) {
      const { item } = await queue(source, asset, asset.size, "audio");
      assert.deepEqual(item.details.audiobook, asset.audiobook);
    }
    await page.locator('[data-page="library"]').click();
    await page.locator('[data-action="library-section"][data-section="audio"]').click();
    await page.getByRole("button", { name: "Audiolibros", exact: true }).click();
    const shown = await page.locator(".audio-row .audio-track-number").allTextContents();
    assert.deepEqual(shown, assets.map(asset => `Cap. ${asset.audiobook.chapter}`));
    return { chapters: assets.map(asset => asset.audiobook.chapter), bytes: assets.map(asset => asset.size) };
  });
  const expected = communityOnly ? ["Zenodo community"] : chaptersOnly ? ["LibriVox chapter filters"] : languageOnly ? ["LibriVox language"] : zenodoOnly ? ["Zenodo"]
    : arxivDetailsOnly ? ["arXiv version details"]
    : ["LibriVox language", "arXiv", "Zenodo", "LibriVox"];
  assert.deepEqual(results, expected, errors.map(String).join("\n"));
} catch (error) {
  console.error(log);
  throw error;
} finally {
  try { await page?.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close(); }); } catch {}
  if (child.exitCode === null) {
    const exited = new Promise(resolve => child.once("exit", resolve));
    await Promise.race([exited, new Promise(resolve => setTimeout(resolve, 4000))]);
    if (child.exitCode === null) { child.kill(); await exited; }
  }
  await browser?.close().catch(() => {});
}
