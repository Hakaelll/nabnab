// Real Tauri and DPAPI with synthetic mounted-folder and HTTPS WebDAV fixtures.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rename, stat, utimes, writeFile } from "node:fs/promises";
import { createServer } from "node:https";
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
  throw Error("Timed out waiting for the personal-server fixture.");
}
const rpc = (name, args = {}) => page.evaluate(async ({ name, args }) => {
  const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
  return invoke(name, args);
}, { name, args });
const button = name => page.getByRole("button", { name, exact: typeof name === "string" });
const comparison = () => page.locator(".personal-reference-review");
async function selectReference() { await button(/Cuaderno ñ\.txt.*Remota/).click(); }
async function job(index, serverId = id) {
  return wait(async () => {
    const { jobs } = await rpc("get_snapshot");
    const pending = jobs.filter(job => job.source.kind === "personalFile" && job.source.selection.serverId === serverId)[index];
    return pending && ["completed", "error"].includes(pending.state) && pending;
  });
}

async function verifyWebdav(openssl) {
  const run = (...args) => execFileSync(openssl, args, { cwd: data, windowsHide: true, stdio: "pipe" });
  run("req", "-x509", "-newkey", "rsa:2048", "-noenc", "-sha256", "-days", "2", "-subj", "/CN=nabnab test CA",
    "-addext", "basicConstraints=critical,CA:TRUE", "-addext", "keyUsage=critical,keyCertSign,cRLSign", "-keyout", "root.key", "-out", "root.pem");
  run("req", "-new", "-newkey", "rsa:2048", "-noenc", "-subj", "/CN=localhost", "-keyout", "server.key", "-out", "server.csr");
  await writeFile(path.join(data, "server.ext"), "subjectAltName=DNS:localhost\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n");
  run("x509", "-req", "-in", "server.csr", "-CA", "root.pem", "-CAkey", "root.key", "-CAcreateserial", "-days", "2", "-sha256", "-extfile", "server.ext", "-out", "server.pem");
  const read = name => readFile(path.join(data, name));
  const [rootCertificate, key, cert] = await Promise.all([read("root.pem"), read("server.key"), read("server.pem")]);
  const pem = rootCertificate.toString().trim();
  const folder = "/remote.php/dav/files/reader/";
  const subfolder = `${folder}Cuadernos/`;
  const remoteFile = `${subfolder}Informe%20%C3%B1.txt`;
  const modified = "Wed, 04 Nov 2026 10:00:00 GMT";
  const username = "reader";
  const password = "local-fixture-only";
  const auth = `Basic ${Buffer.from(`${username}:${password}`).toString("base64")}`;
  let version = 1, redirect = false, outside = false, redirected = 0;
  const requests = [];
  const trap = createServer({ key, cert }, (_req, res) => { redirected++; res.writeHead(403).end(); });
  const server = createServer({ key, cert }, (req, res) => {
    req.resume();
    requests.push({ path: req.url, method: req.method, auth: req.headers.authorization === auth, validator: req.headers["if-match"] });
    if (req.headers.authorization !== auth) return res.writeHead(401).end();
    if (redirect) return res.writeHead(302, { Location: `https://localhost:${trap.address().port}${folder}` }).end();
    const content = Buffer.from(version === 1 ? original : replacement);
    const etag = `"dav-v${version}"`;
    if (req.method === "GET" && req.url === remoteFile) {
      if (req.headers["if-match"] !== etag) return res.writeHead(412).end();
      return res.writeHead(200, { "Content-Length": content.length, "Content-Type": "text/plain", "Last-Modified": modified, ETag: etag }).end(content);
    }
    const resource = (href, properties) => `<d:response><d:href>${href}</d:href><d:propstat><d:prop>${properties}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>`;
    const collection = href => resource(href, "<d:resourcetype><d:collection/></d:resourcetype>");
    const file = () => resource(remoteFile, `<d:resourcetype/><d:getcontentlength>${content.length}</d:getcontentlength><d:getlastmodified>${modified}</d:getlastmodified><d:getetag>${etag}</d:getetag><d:getcontenttype>text/plain</d:getcontenttype>`);
    if (req.method !== "PROPFIND") return res.writeHead(405).end();
    const depth = req.headers.depth;
    const entries = req.url === folder && depth === "1" ? collection(folder) + collection(outside ? "/outside/" : subfolder)
      : req.url === subfolder && depth === "1" ? collection(subfolder) + file()
      : req.url === remoteFile && depth === "0" ? file() : "";
    if (!entries) return res.writeHead(404).end();
    res.writeHead(207, { "Content-Type": "application/xml" }).end(`<d:multistatus xmlns:d="DAV:">${entries}</d:multistatus>`);
  });
  const listen = server => new Promise((resolve, reject) => { server.once("error", reject); server.listen(0, "127.0.0.1", resolve); });
  const close = server => new Promise(resolve => { server.close(resolve); server.closeAllConnections(); });

  try {
    await Promise.all([listen(server), listen(trap)]);
    const port = server.address().port;
    const root = `https://localhost:${port}${folder}`;
    const settings = async () => {
      await button("Ajustes").click();
      await page.locator('[data-action="settings-section"][data-section="connections"]').click();
    };
    const connect = async (name, root, certificatePem) => {
      await settings();
      await button("Nextcloud / WebDAV").click();
      await page.locator("#personal-name").fill(name);
      await page.locator("#personal-root").fill(root);
      await page.locator("#personal-username").fill(username);
      await page.locator("#personal-password").fill(password);
      if (certificatePem) {
        await page.locator("#personal-webdav-trust summary").press("Enter");
        await page.getByRole("textbox", { name: "Certificado PEM" }).fill(certificatePem);
      }
      await button("Guardar conexión").click();
      await button(new RegExp(`${name}.*Nextcloud`)).click();
    };
    const alert = () => page.locator(".personal-servers [role=alert]");
    const serverIdFor = async name => (await rpc("personal_servers_status")).servers.find(server => server.name === name).id;
    await assert.rejects(rpc("personal_server_webdav", {
      name: "Clave rechazada", root, username, password, certificatePem: "-----BEGIN PRIVATE KEY-----\nZml4dHVyZQ==\n-----END PRIVATE KEY-----",
    }), error => String(error).includes("valid PEM certificate"));
    await connect("Sin certificado", root, null);
    await alert().filter({ hasText: "HTTPS certificate" }).waitFor();
    assert.equal(requests.length, 0);
    const untrustedId = await serverIdFor("Sin certificado");
    await settings();
    await page.locator(`[data-action="personal-remove"][data-id="${untrustedId}"]`).click();
    await page.locator(`[data-action="personal-remove"][data-id="${untrustedId}"]`).waitFor({ state: "detached" });

    await connect("Nombre distinto", `https://127.0.0.1:${port}${folder}`, pem);
    await alert().filter({ hasText: "HTTPS certificate" }).waitFor();
    assert.equal(requests.length, 0);
    await connect("WebDAV de prueba", root, pem);
    await button("Cuadernos").click();
    const webdavId = await serverIdFor("WebDAV de prueba");
    const view = JSON.stringify(await rpc("personal_servers_status"));
    assert.ok(!view.includes(password) && !view.includes("BEGIN CERTIFICATE"));
    await assert.rejects(rpc("personal_server_webdav", { name: "WebDAV de prueba", root, username, password, certificatePem: null }),
      error => String(error).includes("trust settings changed"));
    await page.getByRole("checkbox", { name: /Informe ñ\.txt/ }).check();
    await button("Indexar referencia").click();
    await button("Referencias guardadas").click();
    const select = () => button(/Informe ñ\.txt.*Remota/).click();
    await select();
    await button("Obtener copia local").click();
    const first = await job(0, webdavId);
    assert.equal(first.state, "completed", JSON.stringify(first.error));
    assert.equal(await readFile(first.outputs[0], "utf8"), original);
    version = 2;
    await select();
    await button("Obtener copia local").click();
    const stale = await job(1, webdavId);
    assert.equal(stale.state, "error");
    assert.match(JSON.stringify(stale.error), /WebDAV file changed/);
    assert.deepEqual(stale.outputs, []);
    await select();
    await button("Revisar en servidor").click();
    await comparison().getByRole("status").filter({ hasText: "El identificador de versión es distinto." }).waitFor();
    await button("Actualizar referencia").click();
    await comparison().getByRole("status").filter({ hasText: "Los datos visibles coinciden" }).waitFor();
    await button("Volver a referencias").click();
    await select();
    await button("Obtener copia local").click();
    const updated = await job(2, webdavId);
    assert.equal(updated.state, "completed", JSON.stringify(updated.error));
    assert.equal(updated.source.selection.stamp.size, first.source.selection.stamp.size);
    assert.equal(updated.source.selection.stamp.modified, first.source.selection.stamp.modified);
    assert.notEqual(updated.source.selection.stamp.etag, first.source.selection.stamp.etag);
    assert.notEqual(updated.outputs[0], first.outputs[0]);
    assert.equal(await readFile(updated.outputs[0], "utf8"), replacement);
    assert.equal(await readFile(first.outputs[0], "utf8"), original);
    assert.deepEqual(requests.filter(request => request.method === "GET").map(request => request.validator), ['"dav-v1"', '"dav-v2"']);
    assert.ok(requests.every(request => request.auth && request.path.startsWith(folder)));

    const browse = requestId => rpc("personal_server_browse", { serverId: webdavId, path: [], requestId });
    outside = true;
    await assert.rejects(browse(1), error => String(error).includes("outside the authorized root"));
    redirect = true;
    await assert.rejects(browse(2), error => String(error).includes("redirected the request"));
    assert.equal(redirected, 0);
    console.log("Native WebDAV verified private TLS, hostname checks, scope, redirect blocking and two distinct file versions.");
  } finally {
    await Promise.all([close(server), close(trap)]);
  }
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
  if (process.env.NABNAB_TEST_OPENSSL) await verifyWebdav(process.env.NABNAB_TEST_OPENSSL);
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
