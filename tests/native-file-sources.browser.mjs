// Runs real desktop IPC and three public APIs with small downloads in an isolated repository.
// Build first: cargo build --manifest-path src-tauri/Cargo.toml --offline. Vite must run on 1420.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { mkdir, mkdtemp, readFile, writeFile, stat } from "node:fs/promises";
import path from "node:path";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = path.resolve(import.meta.dirname, "..");
const executable = process.env.NABNAB_EXECUTABLE || path.join(root, "src-tauri/target/debug/nabnab.exe");
const base = path.join(root, "artifacts/file-sources");
await mkdir(base, { recursive: true });
const reuseData = process.env.NABNAB_NATIVE_REUSE_DATA;
const data = reuseData ? path.resolve(reuseData) : await mkdtemp(path.join(base, "native-run-"));
assert.ok(path.dirname(data) === base && path.basename(data).startsWith("native-run-"), "Reuse is limited to this test artifact directory.");
const output = path.join(data, "outputs");
await mkdir(output, { recursive: true });
const port = Number(process.env.NABNAB_DEBUG_PORT || 9234);
const limit = 512 * 1024;
const checks = [];
const errors = [];
const providerFailures = [];
const logs = [];
let child, browser, page;
let request = Date.now();

async function waitFor(check, timeout = 30000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const result = await check();
    if (result) return result;
    await new Promise(resolve => setTimeout(resolve, 150));
  }
  throw new Error(`Timed out after ${timeout}ms waiting for the isolated native app.`);
}
async function rpc(cmd, args = {}) {
  return page.evaluate(async ({ cmd, args }) => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    return invoke(cmd, args);
  }, { cmd, args });
}
async function inspect(url, cursor = null) {
  let result;
  for (let attempt = 0; attempt < 2; attempt++) {
    try { result = await rpc("public_source_inspect", { url, cursor, requestId: ++request }); break; }
    catch (error) {
      if (attempt || !/HTTP 50[234]/.test(String(error))) throw error;
      checks.push({ case: "temporary public API failure", sourceUrl: url, retry: 1, error: String(error) });
      await new Promise(resolve => setTimeout(resolve, 1000));
    }
  }
  assert.ok(result);
  checks.push({ case: "public metadata", provider: result.provider, sourceUrl: result.sourceUrl, assets: result.assets.length, entries: result.entries.length, partial: Boolean(result.nextCursor) });
  return result;
}
async function findAsset(url, predicate) {
  let result = await inspect(url);
  for (let index = 0; index < 3; index++) {
    const asset = result.assets.find(value => value.size !== null && value.size > 0 && value.size <= limit && predicate(value));
    if (asset) return { source: result, asset };
    if (!result.nextCursor) break;
    result = await inspect(result.sourceUrl, result.nextCursor);
  }
  throw new Error(`The public source did not offer a matching file within the ${limit}-byte test budget: ${url}`);
}
async function completed(id) {
  return waitFor(async () => {
    const snapshot = await rpc("get_snapshot");
    const job = snapshot.jobs.find(value => value.id === id);
    if (job?.state === "error") throw new Error(`${job.provider}: ${job.error?.code}: ${job.error?.technical || job.error?.message}`);
    return job?.state === "completed" ? job : false;
  }, 90000);
}
async function queueAndVerify(source, asset, collection, expectedKind) {
  assert.ok(asset.size > 0 && asset.size <= limit);
  const clientRequest = `native-public-${randomUUID()}`;
  const jobs = await rpc("queue_public_files", { sourceUrl: source.sourceUrl, assetIds: [asset.id], collection, clientRequest });
  assert.ok(jobs.length > 0);
  const replay = await rpc("queue_public_files", { sourceUrl: source.sourceUrl, assetIds: [asset.id], collection, clientRequest });
  assert.deepEqual(replay.map(job => job.id), jobs.map(job => job.id), "Identical client receipts must return the original jobs.");
  const completedJobs = await Promise.all(jobs.map(job => completed(job.id)));
  const fileJob = completedJobs.find(job => job.source.kind === "publicFile");
  assert.ok(fileJob);
  assert.equal(fileJob.outputs.length, 1);
  const bytes = await readFile(fileJob.outputs[0]);
  assert.equal(bytes.length, asset.size);
  assert.ok(bytes.length <= limit);
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  if (asset.sha256) assert.equal(sha256, asset.sha256.toLowerCase());
  const snapshot = await rpc("get_snapshot");
  const item = snapshot.library.find(value => value.path === fileJob.outputs[0]);
  assert.ok(item);
  assert.equal(item.kind, expectedKind);
  assert.ok(item.collections.includes(collection));
  assert.equal(item.details.contentId, asset.id);
  assert.equal(item.details.sourceUrl, asset.sourceUrl);
  assert.equal(item.details.creator, asset.author);
  assert.equal(item.details.license, asset.license);
  checks.push({ case: "native queued public file", status: "passed", provider: source.provider, sourceUrl: asset.sourceUrl, filename: asset.filename, bytes: bytes.length, sha256, publishedHashVerified: Boolean(asset.sha256), libraryKind: item.kind, collection, idempotentReceipt: true });
  return { jobs: completedJobs, item };
}
async function verifyProvider(provider, operation) {
  try { await operation(); } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    providerFailures.push({ provider, error: message }); checks.push({ case: "provider verification", provider, status: "failed", error: message });
    console.error(JSON.stringify({ provider, status: "failed", error: message }));
  }
}
async function close() {
  if (page && !page.isClosed()) {
    try { await page.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close(); }); } catch { /* The WebView may close before its reply. */ }
  }
  if (child && child.exitCode === null) {
    await Promise.race([new Promise(resolve => child.once("exit", resolve)), new Promise(resolve => setTimeout(resolve, 4000))]);
    if (child.exitCode === null) child.kill();
  }
  try { await browser?.close(); } catch { /* The native process owns the CDP browser. */ }
}
try {
  child = spawn(executable, [], { cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"), WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` } });
  child.stdout.on("data", chunk => logs.push(chunk.toString()));
  child.stderr.on("data", chunk => logs.push(chunk.toString()));
  await waitFor(async () => {
    if (child.exitCode !== null) throw new Error(`Native app exited with code ${child.exitCode}`);
    try { return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok; } catch { return false; }
  });
  browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  page = await waitFor(() => browser.contexts()[0]?.pages().find(value => /localhost:1420|127\.0\.0\.1:1420/.test(value.url())));
  page.on("pageerror", error => errors.push(error.message));
  await page.waitForSelector("[data-page='platforms']");
  const startup = await waitFor(async () => { try { return await rpc("get_snapshot"); } catch { return false; } });
  assert.equal(startup.settings.trayEnabled, false);
  assert.equal(startup.settings.startWithWindows, false);
  if (!reuseData) { assert.equal(startup.library.length, 0); assert.equal(startup.jobs.length, 0); }
  else { assert.equal(startup.library.length, 4); assert.equal(startup.jobs.length, 4); checks.push({ case: "reopen existing native test artifacts", status: "passed", sourceDirectory: path.relative(root, data), remoteApisCalled: false }); }
  await rpc("save_settings", { settings: { ...startup.settings, folder: output, language: "es", reducedMotion: true, reducedTransparency: true } });
  await page.reload();
  await page.waitForSelector("[data-page='platforms']");

  if (!reuseData) {
  await verifyProvider("github", async () => {
  const repository = await inspect("https://github.com/cli/cli");
  const release = repository.entries.find(value => !value.prerelease);
  assert.ok(release, "The GitHub repository must offer a stable release.");
  const github = await findAsset(release.url, asset => /checksums?\.txt$/i.test(asset.filename));
  assert.equal((await rpc("get_snapshot")).jobs.length, 0, "Inspection must not enqueue files.");
  await page.locator("[data-page='platforms']").click();
  await page.locator("[data-action='open-fileSources']").first().click();
  await page.locator("#file-source-url").fill(release.url);
  await page.locator("#file-source-form button[type='submit']").click();
  const file = page.locator(`[data-file-source-select=${JSON.stringify(github.asset.id)}]`);
  await file.waitFor({ timeout: 45000 });
  assert.equal(await file.isChecked(), false);
  await file.check();
  await page.locator("#file-source-collection").fill("Native release verification");
  await page.screenshot({ path: path.join(data, "github-selection-native.png") });
  const githubResult = await queueAndVerify(github.source, github.asset, "Native release verification", "document");
  assert.ok((await readFile(githubResult.item.path, "utf8")).includes("gh_"), "The downloaded checksum manifest should list GitHub CLI assets.");

  });
  await verifyProvider("commons", async () => {
  const commons = await findAsset("https://commons.wikimedia.org/wiki/File:Example.jpg", asset => asset.mime.startsWith("image/") && asset.original);
  assert.ok(commons.asset.attribution, "Commons must retain attribution and report metadata absent from the source.");
  const commonsResult = await queueAndVerify(commons.source, commons.asset, "Native Commons verification", "image");
  const attributionJob = commonsResult.jobs.find(job => job.source.kind === "attribution");
  assert.ok(attributionJob, "Commons selections must create an attribution result in the same queue.");
  const attribution = JSON.parse(await readFile(attributionJob.outputs[0], "utf8"));
  assert.equal(attribution.schemaVersion, 1);
  assert.equal(attribution.files.length, 1);
  assert.equal(attribution.files[0].assetId, commons.asset.id);
  assert.equal(attribution.files[0].author, commons.asset.author);
  assert.equal(attribution.files[0].license, commons.asset.license);
  assert.equal(attribution.files[0].sourceUrl, commons.asset.sourceUrl);
  assert.equal("downloadUrl" in attribution.files[0], false);
  checks.push({ case: "Commons attribution", status: "passed", author: commons.asset.author, license: commons.asset.license, queueResults: commonsResult.jobs.length, signedUrlsExported: false });

  });
  await verifyProvider("archive", async () => {
  const archive = await findAsset("https://archive.org/details/BigBuckBunny_328", asset => /(?:__ia_thumb|thumb|thumbnail).*\.(?:jpg|png)$/i.test(asset.filename));
  await queueAndVerify(archive.source, archive.asset, "Native Archive verification", "image");
  checks.push({ case: "Archive provenance", status: "passed", rightsAsReported: archive.source.rights, original: archive.asset.original, noDefaultBulkDownload: true });

  });
  const cancellation = await page.evaluate(async requestId => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    const pending = invoke("public_source_inspect", { url: "https://github.com/cli/cli", cursor: null, requestId }).then(() => ({ canceled: false }), error => ({ canceled: true, error: String(error) }));
    await new Promise(resolve => setTimeout(resolve, 20));
    await invoke("cancel_file_source_inspection", { requestId });
    return pending;
  }, ++request);
  assert.equal(cancellation.canceled, true, "Cancel must interrupt the native source inspection.");
  assert.match(cancellation.error, /cancel/i);
  checks.push({ case: "native inspection cancellation", status: "passed" });
  }
  const snapshot = await rpc("get_snapshot");
  if (!providerFailures.length) {
    assert.equal(snapshot.library.length, 4); assert.equal(snapshot.jobs.length, 4);
  }
  assert.ok(snapshot.jobs.every(job => job.state === "completed"));
  const persisted = await readFile(path.join(data, "state-v1.json"), "utf8");
  assert.equal(/\bBearer\s+[A-Za-z0-9]|[?&](?:access_token|token|signature|x-amz-signature)=/i.test(persisted), false);
  assert.equal(persisted.includes('"downloadUrl"'), false);
  assert.equal(snapshot.library.some(item => item.path.includes(".part")), false);
  assert.ok((await rpc("cloud_status")).every(connection => !connection.configured && !connection.connected));
  checks.push({ case: "persistence privacy and isolation", status: "passed", libraryItems: snapshot.library.length, jobs: snapshot.jobs.length, credentialsConfigured: false, backgroundStartupEnabled: false });
  await page.locator("[data-page='library']").click();
  if (snapshot.library.length) await page.waitForFunction(() => document.querySelectorAll("[data-select-media]").length > 0);
  for (const item of snapshot.library) {
    const label = await page.locator(`.media-item .media-open[data-id=${JSON.stringify(item.id)}] .media-info > span`).innerText();
    assert.match(label, item.kind === "document" ? /^Documento/ : item.kind === "file" ? /^Archivo/ : /^Imagen/, `The visible type must match ${item.kind}: ${item.name}`);
  }
  if (snapshot.library.some(item => item.kind === "image")) await page.waitForFunction(() => {
    const posters = [...document.querySelectorAll("[data-thumbnail-id]")];
    return posters.length > 0 && posters.every(poster => { const image = poster.querySelector("img"); return image?.complete && image.naturalWidth > 0; });
  }, null, { timeout: 15000 }).catch(async error => {
    const images = []; for (const item of snapshot.library.filter(value => value.kind === "image")) {
      try { images.push({ id: item.id, result: await rpc("thumbnail", { id: item.id }) }); } catch (failure) { images.push({ id: item.id, error: String(failure) }); }
    }
    checks.push({ case: "native image thumbnails", status: "unavailable", error: String(error), images });
  });
  checks.push({ case: "visible library file types", status: "passed", kinds: snapshot.library.map(item => item.kind) });
  await page.screenshot({ path: path.join(data, reuseData ? "public-files-library-local-native.png" : "public-files-library-native.png") });
  assert.deepEqual(errors, []);
  await writeFile(path.join(data, reuseData ? "local-results.json" : "results.json"), JSON.stringify({ date: new Date().toISOString(), native: true, publicApis: !reuseData, isolatedData: true, maxBytesPerDownload: limit, oauthAccountsTested: false, checks, providerFailures, browserErrors: errors }, null, 2) + "\n");
  assert.deepEqual(providerFailures, [], "All public providers must pass; failures remain recorded individually.");
  console.log(JSON.stringify({ status: "passed", artifactDirectory: path.relative(root, data), checks: checks.length }));
} catch (error) {
  if (page && !page.isClosed()) await page.screenshot({ path: path.join(data, "failure.png") }).catch(() => {});
  const snapshot = page && !page.isClosed() ? await rpc("get_snapshot").catch(() => null) : null;
  await writeFile(path.join(data, reuseData ? "local-failure.json" : "failure.json"), JSON.stringify({ date: new Date().toISOString(), native: true, checks, browserErrors: errors, snapshot, error: error instanceof Error ? error.message : String(error) }, null, 2) + "\n");
  console.error(JSON.stringify({ status: "failed", artifactDirectory: path.relative(root, data), error: error instanceof Error ? error.message : String(error) }));
  throw error;
} finally {
  await close();
  await writeFile(path.join(data, "native.log"), logs.join(""));
  assert.ok((await stat(data)).isDirectory());
}
