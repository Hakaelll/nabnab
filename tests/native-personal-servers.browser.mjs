// Real Tauri, DPAPI and the mounted-folder adapter with a synthetic local root.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rename, stat, utimes, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.join(root, "artifacts/native-personal-servers");
await mkdir(artifacts, { recursive: true });
const data = await mkdtemp(path.join(artifacts, "run-"));
const mount = path.join(data, "mount");
const offline = path.join(data, "offline");
const docs = path.join(mount, "Lecturas");
const output = path.join(data, "output");
const secrets = path.join(data, "module-secrets");
await Promise.all([mkdir(docs, { recursive: true }), mkdir(output), mkdir(secrets)]);
const file = "Cuaderno ñ.txt";
const remote = path.join(docs, file);
const original = "Edición A\n";
const replacement = "Edición B\n";
await writeFile(remote, original);

const location = path.toNamespacedPath(mount);
const id = createHash("sha256").update(JSON.stringify(["nas", location, ""])).digest("hex");
const vault = { connections: [{ id, name: "Archivo de prueba", config: { kind: "nas", config: location } }], references: [] };
const ciphertext = execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", `
  Add-Type -AssemblyName System.Security
  $bytes = [Convert]::FromBase64String($env:NABNAB_FIXTURE_VAULT)
  $entropy = [Text.Encoding]::UTF8.GetBytes('nabnab-cookie-v1:nabnab-module:personal-servers')
  $sealed = [Security.Cryptography.ProtectedData]::Protect($bytes, $entropy, [Security.Cryptography.DataProtectionScope]::CurrentUser)
  [Console]::OpenStandardOutput().Write($sealed, 0, $sealed.Length)
`], { windowsHide: true, env: { ...process.env, NABNAB_FIXTURE_VAULT: Buffer.from(JSON.stringify(vault)).toString("base64") } });
await writeFile(path.join(secrets, "personal-servers.bin"), Buffer.concat([Buffer.from("nabnab-module-secret-v1\n"), ciphertext]));

const port = process.env.NABNAB_PERSONAL_SERVER_DEBUG_PORT || 9258;
const endpoint = `http://127.0.0.1:${port}`;
const child = spawn(path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
  cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
  env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
});
let browser, page, log = "";
for (const stream of [child.stdout, child.stderr]) stream.on("data", chunk => { log = (log + chunk).slice(-5000); });

async function wait(check, timeout = 20_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (child.exitCode !== null) throw Error(`Native app exited ${child.exitCode}.`);
    const value = await check();
    if (value) return value;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw Error("Timed out waiting for the mounted-folder fixture.");
}
const rpc = (name, args = {}) => page.evaluate(async ({ name, args }) => {
  const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
  return invoke(name, args);
}, { name, args });
const button = name => page.getByRole("button", { name, exact: typeof name === "string" });
const comparison = () => page.locator(".personal-reference-review");
async function selectReference() { await button(/Cuaderno ñ\.txt.*Remota/).click(); }
async function job(index) {
  return wait(async () => {
    const { jobs } = await rpc("get_snapshot");
    const pending = jobs.filter(job => job.source.kind === "personalFile")[index];
    return pending && ["completed", "error"].includes(pending.state) && pending;
  });
}

try {
  browser = await wait(async () => {
    try {
      if ((await fetch(`${endpoint}/json/version`, { signal: AbortSignal.timeout(400) })).ok) {
        return await chromium.connectOverCDP(endpoint, { timeout: 2000 });
      }
    } catch { /* Wait for WebView2 to become available. */ }
    return null;
  });
  page = await wait(() => browser.contexts()[0]?.pages().find(tab => /:1420/.test(tab.url())));
  const initial = await wait(async () => { try { return await rpc("get_snapshot"); } catch { return null; } });
  await rpc("save_settings", { settings: { ...initial.settings, language: "es", folder: output, reducedMotion: true, trayEnabled: false, closeToTray: false } });
  await page.reload();
  await page.locator('[data-page="platforms"]').click();
  await button("Buscar en nabnab").click();
  await page.locator(".action-palette").waitFor();
  await page.keyboard.press("Escape");
  await page.locator('[data-action="open-fileSources"][data-id="personal"]').click();
  await button(/Archivo de prueba.*NAS/).click();
  await button("Lecturas").click();
  await page.getByRole("checkbox", { name: /Cuaderno ñ\.txt/ }).check();
  await button("Indexar referencia").click();
  await button("Referencias guardadas").click();
  await selectReference();
  await button("Obtener copia local").click();
  const first = await job(0);
  assert.equal(first.state, "completed", JSON.stringify(first.error));
  assert.equal(await readFile(first.outputs[0], "utf8"), original);

  await rename(mount, offline);
  await selectReference();
  await button("Revisar en servidor").click();
  await page.locator(".personal-servers [role=alert]").waitFor();
  assert.equal(await page.getByRole("checkbox", { name: /Cuaderno ñ\.txt/ }).count(), 0);
  const offlineFile = path.join(offline, "Lecturas", file);
  const previous = await stat(offlineFile);
  const newFile = path.join(offline, "Lecturas", "new.txt");
  await writeFile(newFile, replacement);
  await utimes(newFile, previous.atime, previous.mtime);
  await rename(newFile, offlineFile);
  await rename(offline, mount);
  await button("Actualizar").click();
  await comparison().getByRole("status").filter({ hasText: "El identificador de versión es distinto." }).waitFor();
  await comparison().screenshot({ path: path.join(artifacts, "reference-reconnected.png") });
  await button("Volver a referencias").click();
  await selectReference();
  await button("Obtener copia local").click();
  const stale = await job(1);
  assert.equal(stale.state, "error");
  assert.match(JSON.stringify(stale.error), /mounted file changed/i);
  assert.deepEqual(stale.outputs, []);

  await selectReference();
  await button("Revisar en servidor").click();
  await button("Actualizar referencia").click();
  await comparison().getByRole("status").filter({ hasText: "Los datos visibles coinciden" }).waitFor();
  await button("Volver a referencias").click();
  await selectReference();
  await button("Obtener copia local").click();
  const updated = await job(2);
  assert.equal(updated.state, "completed", JSON.stringify(updated.error));
  assert.equal(updated.source.selection.stamp.size, first.source.selection.stamp.size);
  assert.equal(updated.source.selection.stamp.modified, first.source.selection.stamp.modified);
  assert.notEqual(updated.source.selection.stamp.etag, first.source.selection.stamp.etag);
  assert.notEqual(updated.outputs[0], first.outputs[0]);
  assert.equal(await readFile(updated.outputs[0], "utf8"), replacement);
  assert.equal(await readFile(first.outputs[0], "utf8"), original);
  assert.equal(await readFile(remote, "utf8"), replacement);
  console.log("Native mounted-folder reconnection rejected the old version and preserved both verified copies.");
} catch (error) {
  console.error(log);
  console.error(await page?.locator("#page-content").innerText().catch(() => ""));
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
