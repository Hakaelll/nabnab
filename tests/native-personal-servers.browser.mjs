// Real Tauri and DPAPI with synthetic mounted-folder, HTTPS WebDAV and SFTP fixtures.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { mkdir, mkdtemp, readdir, readFile, rename, stat, utimes, writeFile } from "node:fs/promises";
import { createServer } from "node:https";
import { createRequire } from "node:module";
import path from "node:path";
import { openSftpFixture } from "./native-sftp-fixture.mjs";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = path.resolve(import.meta.dirname, "..");
const artifacts = path.resolve(process.env.NABNAB_PERSONAL_SERVER_ARTIFACTS ?? path.join(root, "artifacts/native-personal-servers"));
await mkdir(artifacts, { recursive: true });
const data = await mkdtemp(path.join(artifacts, "run-"));
const sftpIdentity = path.join(data, "SFTP fixture key");
const mount = path.join(data, "mount");
const offline = path.join(data, "offline");
const docs = path.join(mount, "Lecturas");
const output = path.join(data, "output");
const secrets = path.join(data, "module-secrets");
const temporary = path.join(data, "tmp");
await Promise.all([mkdir(docs, { recursive: true }), mkdir(output), mkdir(secrets), mkdir(temporary)]);
const file = "Cuaderno ñ.txt";
const remote = path.join(docs, file);
const original = "Edición A\n" + "a".repeat(210_000);
const replacement = "Edición B\n" + "b".repeat(210_000);
await writeFile(remote, original);
await writeFile(path.join(docs, "Inventario.txt"), "Este archivo debe quedar fuera de la selección.\n");

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
const child = spawn(process.env.NABNAB_PERSONAL_SERVER_TEST_EXE || path.join(root, "src-tauri/target/debug/nabnab.exe"), [], {
  cwd: root, windowsHide: true, stdio: ["ignore", "pipe", "pipe"],
  env: { ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
    TEMP: temporary, TMP: temporary,
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
async function job(index, serverId = id, after = 0) {
  return wait(async () => {
    const { jobs } = await rpc("get_snapshot");
    const pending = jobs.filter(job => job.source.kind === "personalFile" && job.source.selection.serverId === serverId)[index];
    return pending && pending.runId > after && ["completed", "error"].includes(pending.state) && pending;
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
  let interrupt = false, ignoreRange = false, invalidRange = false;
  const fixtureContent = () => version <= 2 ? Buffer.from(version === 1 ? original : replacement) : Buffer.alloc(210_000, 48 + version);
  const requests = [];
  const trap = createServer({ key, cert }, (_req, res) => { redirected++; res.writeHead(403).end(); });
  const server = createServer({ key, cert }, (req, res) => {
    req.resume();
    requests.push({ path: req.url, method: req.method, auth: req.headers.authorization === auth, validator: req.headers["if-match"], range: req.headers.range });
    if (req.headers.authorization !== auth) return res.writeHead(401).end();
    if (redirect) return res.writeHead(302, { Location: `https://localhost:${trap.address().port}${folder}` }).end();
    const content = fixtureContent();
    const etag = `"dav-v${version}"`;
    if (req.method === "GET" && req.url === remoteFile) {
      if (req.headers["if-match"] !== etag) return res.writeHead(412).end();
      const headers = { "Content-Type": "text/plain", "Last-Modified": modified, ETag: etag };
      if (interrupt) {
        interrupt = false;
        res.writeHead(200, { ...headers, "Content-Length": content.length });
        res.write(content.subarray(0, 96_000));
        setTimeout(() => res.destroy(), 300);
        return;
      }
      const range = /^bytes=(\d+)-$/.exec(req.headers.range ?? "");
      if (range && !ignoreRange) {
        const start = Number(range[1]) + Number(invalidRange);
        return res.writeHead(206, { ...headers, "Content-Length": content.length - start,
          "Content-Range": `bytes ${start}-${content.length - 1}/${content.length}` }).end(content.subarray(start));
      }
      return res.writeHead(200, { ...headers, "Content-Length": content.length }).end(content);
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

    await page.locator('[data-page="downloads"]').click();
    const parts = async job => (await readdir(path.join(output, ".nabnab-" + job.id)).catch(error => {
      if (error.code === "ENOENT") return [];
      throw error;
    })).filter(name => name.startsWith(".personal-webdav-"));
    const row = job => page.locator(`.job-row:has([data-select-job="${job.id}"])`);
    let requestId = 30;
    const interrupted = async (nextVersion, index) => {
      version = nextVersion;
      interrupt = true;
      const entries = await rpc("personal_server_browse", { serverId: webdavId, path: ["Cuadernos"], requestId: ++requestId });
      const { path, stamp } = entries.find(entry => !entry.directory);
      await rpc("personal_server_copy", { selections: [{ serverId: webdavId, path, stamp }], referenceId: null, clientRequest: randomUUID() });
      const failed = await job(index, webdavId);
      assert.equal(failed.state, "error");
      assert.equal(failed.error.code, "webdav_interrupted", JSON.stringify(failed.error));
      assert.deepEqual(failed.outputs, []);
      assert.equal((await parts(failed)).length, 1);
      return failed;
    };
    const continueCopy = async (failed, index) => {
      await row(failed).getByRole("button", { name: "Reanudar", exact: true }).click();
      return job(index, webdavId, failed.runId);
    };
    const pausedCopy = await interrupted(3, 3);
    const [partialName] = await parts(pausedCopy);
    const received = (await stat(path.join(output, ".nabnab-" + pausedCopy.id, partialName))).size;
    assert.equal(received, 96_000);
    assert.equal(pausedCopy.progress.downloaded, received);
    await row(pausedCopy).getByText(/Copia interrumpida.*guardados/).waitFor();
    await row(pausedCopy).getByRole("button", { name: "Descartar parcial", exact: true }).waitFor();
    assert.ok(await row(pausedCopy).evaluate(node => node.scrollWidth <= node.clientWidth + 1));
    await row(pausedCopy).screenshot({ path: path.join(data, "webdav-resume.png") });
    const resumed = await continueCopy(pausedCopy, 3);
    assert.equal(resumed.state, "completed", JSON.stringify(resumed.error));
    assert.deepEqual(await readFile(resumed.outputs[0]), fixtureContent());
    assert.ok(requests.some(request => request.validator === '"dav-v3"' && request.range === `bytes=${received}-`));
    await wait(async () => (await parts(resumed)).length === 0);

    const changedCopy = await interrupted(4, 4);
    version = 5;
    const countBeforeChanged = requests.filter(request => request.method === "GET").length;
    const changed = await continueCopy(changedCopy, 4);
    assert.equal(changed.error.code, "webdav_changed");
    assert.deepEqual(changed.outputs, []);
    assert.deepEqual(await parts(changed), []);
    assert.equal(requests.filter(request => request.method === "GET").length, countBeforeChanged);

    const ignoredCopy = await interrupted(6, 5);
    ignoreRange = true;
    const restarted = await continueCopy(ignoredCopy, 5);
    assert.equal(restarted.state, "completed", JSON.stringify(restarted.error));
    assert.deepEqual(await readFile(restarted.outputs[0]), fixtureContent());
    ignoreRange = false;

    const malformedCopy = await interrupted(7, 6);
    invalidRange = true;
    const malformed = await continueCopy(malformedCopy, 6);
    assert.match(JSON.stringify(malformed.error), /incompatible byte range/);
    assert.deepEqual(malformed.outputs, []);
    assert.deepEqual(await parts(malformed), []);
    invalidRange = false;

    const discard = await interrupted(8, 7);
    await row(discard).getByRole("button", { name: "Descartar parcial", exact: true }).click();
    await wait(async () => (await rpc("get_snapshot")).jobs.find(job => job.id === discard.id)?.state === "cancelled");
    assert.deepEqual(await parts(discard), []);

    const browse = requestId => rpc("personal_server_browse", { serverId: webdavId, path: [], requestId });
    outside = true;
    await assert.rejects(browse(1), error => String(error).includes("outside the authorized root"));
    redirect = true;
    await assert.rejects(browse(2), error => String(error).includes("redirected the request"));
    assert.equal(redirected, 0);
    console.log("Native WebDAV verified private TLS, scope, redirect blocking, range continuation, changed and invalid ranges, full-response restart and partial discard.");
  } finally {
    await Promise.all([close(server), close(trap)]);
  }
}

async function verifySftp(module) {
  const server = await openSftpFixture(module, sftpIdentity);
  const savedName = "Servidor SFTP de prueba";
  try {
    await button("Ajustes").click();
    await page.locator('[data-action="settings-section"][data-section="connections"]').click();
    await page.locator('[data-action="personal-add"][data-kind="sftp"]').click();
    await page.locator("#personal-name").fill(savedName);
    await page.locator("#personal-host").fill(server.hostname);
    await page.locator("#personal-port").fill(String(server.port));
    await page.locator("#personal-username").fill(server.username);
    await page.locator("#personal-root").fill(server.root);
    await button("Elegir clave").click();
    await button("Comprobar huella").click();
    const trust = page.locator(".personal-trust");
    await trust.getByText(server.fingerprint, { exact: true }).waitFor();
    assert.equal(server.authenticationRequests, 0);
    assert.ok(!(await rpc("personal_servers_status")).servers.some(item => item.name === savedName));
    await trust.screenshot({ path: path.join(data, "sftp-fingerprint.png") });
    await button("Confiar y guardar").click();
    const serverButton = page.locator('[data-action="personal-browse"]').filter({ hasText: savedName });
    await serverButton.waitFor();
    const serverId = await serverButton.getAttribute("data-id");
    await serverButton.click();
    await button("Lecturas").waitFor();
    assert.equal(await button("Atajo").count(), 0);
    const browse = relative => rpc("personal_server_browse", { serverId, path: relative, requestId: Date.now() * 1000 + 310 });
    const activityBefore = server.activity.length;
    await assert.rejects(browse([".."]), /saved root/);
    assert.equal(server.activity.length, activityBefore);
    await assert.rejects(browse(["Atajo"]), /link, special file/);
    assert.ok(!server.activity.some(([verb, file]) => verb === "REALPATH" && file.endsWith("/Atajo")));
    await button("Lecturas").click();
    const selectedFile = page.getByRole("checkbox", { name: new RegExp(server.name) });
    await selectedFile.check();
    await page.locator(".personal-review").getByText("Destino local", { exact: true }).waitFor();
    await button("Indexar referencia").click();
    const saved = await wait(async () => (await rpc("personal_servers_status")).references.find(item => item.selection.serverId === serverId));
    assert.equal(saved.selection.stamp.size, server.data.length);
    assert.deepEqual(saved.selection.path, ["Lecturas", server.name]);
    assert.ok(!server.activity.some(([verb]) => verb === "OPEN"));
    assert.equal((await rpc("get_snapshot")).jobs.filter(item => item.source.selection?.serverId === serverId).length, 0);
    await button("Referencias guardadas").click();
    const savedReference = page.locator('[data-action="personal-reference"]').filter({ hasText: savedName });
    await savedReference.click();
    server.interruptNextCopy();
    await button("Obtener copia local").click();
    const interrupted = await job(0, serverId);
    assert.equal(interrupted.state, "error");
    assert.equal(interrupted.error?.code, "sftp_disconnected");
    assert.match(JSON.stringify(interrupted.error), /SFTP server closed the connection/);
    assert.deepEqual(interrupted.outputs, []);
    const staging = path.join(output, `.nabnab-${interrupted.id}`);
    assert.ok((await readdir(staging).catch(() => [])).every(name => !name.endsWith(".part")));

    const after = server.activity.length;
    await page.locator('[data-page="downloads"]').click();
    const row = page.locator(`.job-row:has([data-action="inspect-job"][data-id="${interrupted.id}"])`);
    await row.getByText("La conexión SFTP se cortó. Reintenta para copiar desde el principio.", { exact: true }).waitFor();
    const gate = Promise.withResolvers();
    server.holdAfterFirstChunk(gate.promise);
    try {
      await row.getByRole("button", { name: "Reintentar", exact: true }).click();
      const active = await wait(async () => {
        const item = (await rpc("get_snapshot")).jobs.find(job => job.id === interrupted.id);
        return item?.runId > interrupted.runId && item.state === "downloading" && item.progress?.downloaded > 0 && item;
      }, 10_000);
      assert.equal(active.progress.total, server.data.length);
      assert.ok(active.progress.downloaded < active.progress.total);
      const bar = row.getByRole("progressbar");
      const percent = (active.progress.downloaded / active.progress.total * 100).toFixed(1);
      await wait(async () => await bar.getAttribute("aria-valuenow") === percent, 5000);
      await row.getByText("128 KB / 259 KB", { exact: true }).waitFor();
      assert.equal(await row.getByRole("button", { name: "Reanudar", exact: true }).count(), 0);
      await row.screenshot({ path: path.join(data, "sftp-transfer.png") });
      await page.setViewportSize({ width: 900, height: 620 });
      assert.ok(await row.evaluate(node => node.scrollWidth <= node.clientWidth + 1));
      await row.screenshot({ path: path.join(data, "sftp-transfer-900.png") });
      await page.setViewportSize({ width: 1180, height: 780 });
      assert.equal(server.activity.slice(after).find(([verb]) => verb === "READ")[1], 0);
    } finally { gate.resolve(); }
    const completed = await job(0, serverId, interrupted.runId);
    assert.equal(completed.state, "completed", JSON.stringify(completed.error));
    const firstBytes = await readFile(completed.outputs[0]);
    assert.deepEqual(firstBytes, server.data);
    assert.ok(server.activity.filter(([verb]) => verb === "READ").length >= 3);
    assert.ok(server.activity.filter(([verb]) => verb === "OPEN").every(([, , flags]) => flags === 1));
    assert.ok(server.activity.every(([verb]) => !["EXEC", "SHELL"].includes(verb)));
    server.replace();
    await page.locator('[data-page="library"]').click();
    await savedReference.click();
    await button("Obtener copia local").click();
    const stale = await job(1, serverId);
    assert.equal(stale.state, "error");
    assert.equal(stale.error?.code, "sftp_changed");
    assert.match(JSON.stringify(stale.error), /SFTP file changed/);
    assert.deepEqual(stale.outputs, []);
    assert.deepEqual(await readFile(completed.outputs[0]), firstBytes);
    const oldFingerprint = server.fingerprint, count = server.authenticationRequests;
    await server.rotateHostKey();
    assert.notEqual(server.fingerprint, oldFingerprint);
    await assert.rejects(browse([]), /SFTP host key differs/);
    assert.equal(server.authenticationRequests, count);
    assert.ok((await readdir(temporary)).every(name => !name.startsWith("nabnab-known-host-")));
    console.log(`Native Windows SFTP verified key trust, root scope, live progress, retry from byte zero, ${firstBytes.length} copied bytes, stale version rejection and host key rotation.`);
  } finally { await server.close(); }
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
  if (process.env.NABNAB_TEST_SSH2_MODULE) {
    await page.route(/\/src\/personal-servers-ui\.ts(?:\?.*)?$/, async route => {
      const response = await route.fetch();
      const source = await response.text();
      const picker = 'await hooks.command("personal_server_choose_key")';
      assert.ok(source.includes(picker), "Could not inject the fixture's selected key path.");
      await route.fulfill({ response, body: source.replace(picker, JSON.stringify(sftpIdentity)) });
    });
  }
  await page.reload();
  await page.locator('[data-page="platforms"]').click();
  await button("Buscar en nabnab").click();
  await page.locator(".action-palette").waitFor();
  await page.keyboard.press("Escape");
  await page.locator('[data-action="open-fileSources"][data-id="personal"]').click();
  await button(/Archivo de prueba.*NAS/).click();
  await button("Lecturas").click();
  const query = page.getByRole("searchbox", { name: "Buscar en esta carpeta", exact: true });
  await query.fill("cuaderno n");
  const cuaderno = page.getByRole("checkbox", { name: /Cuaderno ñ\.txt/ });
  await cuaderno.check();
  await query.fill("inventario");
  assert.equal(await cuaderno.count(), 0);
  await button("Solo seleccionados").press("Enter");
  assert.equal(await cuaderno.isChecked(), true);
  assert.equal(await page.getByRole("checkbox", { name: /Inventario\.txt/ }).count(), 0);
  assert.equal((await rpc("get_snapshot")).jobs.length, 0);
  await button("Indexar referencia").click();
  const indexed = await wait(async () => (await rpc("personal_servers_status")).references[0]);
  assert.deepEqual(indexed.selection.path, ["Lecturas", file]);
  await button("Referencias guardadas").click();
  await selectReference();
  await button("Obtener copia local").click();
  const first = await job(0);
  assert.equal(first.state, "completed", JSON.stringify(first.error));
  assert.equal(await readFile(first.outputs[0], "utf8"), original);

  await rename(mount, offline);
  await selectReference();
  await button("Revisar en servidor").click();
  const unavailable = page.getByRole("region", { name: "Carpeta no disponible" });
  await unavailable.getByRole("alert").waitFor();
  const retryFolder = unavailable.getByRole("button", { name: "Volver a intentar" });
  await page.waitForFunction(() => document.activeElement?.id === "personal-reload");
  assert.equal((await rpc("get_snapshot")).jobs.length, 1);
  assert.equal(await page.getByRole("checkbox", { name: /Cuaderno ñ\.txt/ }).count(), 0);
  const offlineFile = path.join(offline, "Lecturas", file);
  const previous = await stat(offlineFile);
  const newFile = path.join(offline, "Lecturas", "new.txt");
  await writeFile(newFile, replacement);
  await utimes(newFile, previous.atime, previous.mtime);
  await rename(newFile, offlineFile);
  await rename(offline, mount);
  await retryFolder.press("Enter");
  await comparison().getByRole("status").filter({ hasText: "El identificador de versión es distinto." }).waitFor();
  const previousSelection = first.source.selection;
  const savedReference = (await rpc("personal_servers_status")).references[0];
  const newerSelection = { ...previousSelection, stamp: { ...previousSelection.stamp, etag: "different" } };
  await assert.rejects(rpc("personal_server_index", { selections: [previousSelection] }), /server listing changed/);
  await assert.rejects(rpc("personal_server_copy", { selections: [previousSelection], referenceId: null, clientRequest: randomUUID() }), /server listing changed/);
  await assert.rejects(rpc("personal_server_copy", { selections: [newerSelection], referenceId: savedReference.id, clientRequest: randomUUID() }), /saved reference changed/);
  assert.equal((await rpc("get_snapshot")).jobs.length, 1);
  assert.deepEqual((await rpc("personal_servers_status")).references[0].selection, previousSelection);
  await comparison().screenshot({ path: path.join(data, "reference-reconnected.png") });
  await button("Volver a referencias").click();
  await selectReference();
  await button("Obtener copia local").click();
  const stale = await job(1);
  assert.equal(stale.state, "error");
  assert.match(JSON.stringify(stale.error), /mounted file changed/i);
  assert.deepEqual(stale.outputs, []);
  await page.locator('[data-page="downloads"]').click();
  const staleRow = page.locator(`.job-row:has([data-action="inspect-job"][data-id="${stale.id}"])`);
  const reviewVersion = staleRow.getByRole("button", { name: "Revisar versión", exact: true });
  assert.equal(await staleRow.getByRole("button", { name: "Reintentar", exact: true }).count(), 0);
  await staleRow.getByRole("checkbox").check();
  assert.equal(await button("Reintentar errores").isDisabled(), true);
  await staleRow.getByRole("checkbox").uncheck();
  await staleRow.screenshot({ path: path.join(data, "changed-copy.png") });
  await reviewVersion.press("Enter");
  await comparison().getByRole("status").filter({ hasText: "El identificador de versión es distinto." }).waitFor();
  await page.waitForFunction(() => document.activeElement?.id === "personal-reference-review-title");
  assert.equal((await rpc("get_snapshot")).jobs.length, 2, "Reviewing must not enqueue another copy");
  assert.equal(await page.getByRole("button", { name: "Actualizar referencia", exact: true }).count(), 0);
  for (const [width, height] of [[900, 620], [1180, 780]]) {
    await page.setViewportSize({ width, height });
    assert.ok(await comparison().evaluate(panel => panel.scrollWidth <= panel.clientWidth + 1));
    await comparison().evaluate(panel => panel.scrollIntoView({ block: "center" }));
    await page.screenshot({ path: path.join(data, `changed-copy-review-${width}.png`) });
  }
  const copyVersion = comparison().getByRole("button", { name: "Copiar versión actual", exact: true });
  await copyVersion.waitFor({ timeout: 3000 });
  assert.match(await comparison().innerText(), /Destino local/);
  assert.equal(await page.getByRole("checkbox", { name: /Inventario\.txt/ }).count(), 0);
  assert.ok(await comparison().evaluate(panel => panel.getBoundingClientRect().bottom <= innerHeight));
  await comparison().screenshot({ path: path.join(data, "changed-copy-review.png"),
    mask: [comparison().locator(".file-source-destination strong")], maskColor: "#2b2e34" });
  await copyVersion.press("Enter");
  const updated = await job(2);
  assert.equal(updated.state, "completed", JSON.stringify(updated.error));
  assert.equal(updated.source.selection.stamp.size, first.source.selection.stamp.size);
  assert.equal(updated.source.selection.stamp.modified, first.source.selection.stamp.modified);
  assert.notEqual(updated.source.selection.stamp.etag, first.source.selection.stamp.etag);
  assert.notEqual(updated.outputs[0], first.outputs[0]);
  assert.equal(await readFile(updated.outputs[0], "utf8"), replacement);
  assert.equal(await readFile(first.outputs[0], "utf8"), original);
  assert.equal(await readFile(remote, "utf8"), replacement);
  await comparison().getByRole("status").filter({ hasText: "Copia añadida a Descargas." }).waitFor();
  assert.equal(await copyVersion.count(), 0);
  const saved = await rpc("personal_servers_status");
  assert.equal(saved.references[0].selection.stamp.etag, first.source.selection.stamp.etag);
  await button("Volver a Descargas").click();
  assert.equal(await reviewVersion.evaluate(control => document.activeElement === control), true);
  await page.locator('[data-page="library"]').click();

  await selectReference();
  await button("Revisar en servidor").click();
  await button("Actualizar referencia").click();
  await comparison().getByRole("status").filter({ hasText: "Los datos visibles coinciden" }).waitFor();
  await button("Volver a referencias").click();
  console.log("Native Downloads review rejected the old version, preserved both copies, and left the reference unchanged until explicitly updated.");

  await rename(mount, offline);
  await selectReference();
  await button("Obtener copia local").click();
  const interrupted = await job(3);
  assert.equal(interrupted.state, "error");
  const stage = path.join(output, ".nabnab-" + interrupted.id);
  const { serverId, path: relative, stamp: { size, modified, etag } } = interrupted.source.selection;
  const fingerprint = createHash("sha256").update(JSON.stringify({ serverId, path: relative, stamp: { size, modified, etag } })).digest("hex");
  const partial = path.join(stage, `.personal-nas-${fingerprint}.part`);
  const prefix = Buffer.from(replacement).subarray(0, 65_536);
  await writeFile(partial, prefix);
  await page.locator('[data-page="downloads"]').click();
  const copyRow = page.locator(`.job-row:has([data-action="inspect-job"][data-id="${interrupted.id}"])`);
  await copyRow.getByRole("button", { name: "Reintentar", exact: true }).click();
  const stillOffline = await job(3, id, interrupted.runId);
  assert.equal(stillOffline.error?.code, "mounted_interrupted");
  assert.equal(stillOffline.progress?.downloaded, prefix.length);
  assert.deepEqual(await readFile(partial), prefix);
  await copyRow.getByText(/Reconecta la carpeta.*64 KB guardados/).waitFor();
  await copyRow.getByText("Interrumpida", { exact: true }).waitFor();
  assert.equal(await copyRow.getByRole("button", { name: "Descartar parcial", exact: true }).count(), 1);
  assert.ok(await copyRow.evaluate(row => row.scrollWidth <= row.clientWidth + 1));
  await copyRow.screenshot({ path: path.join(data, "mounted-resume.png") });
  await rename(offline, mount);
  await copyRow.getByRole("button", { name: "Reanudar", exact: true }).click();
  const resumed = await job(3, id, stillOffline.runId);
  assert.equal(resumed.state, "completed", JSON.stringify(resumed.error));
  assert.equal(await readFile(resumed.outputs[0], "utf8"), replacement);
  await wait(async () => (await readdir(stage).catch(() => [])).every(name => !name.endsWith(".part")));
  console.log("Native mounted-folder retry retained the owned prefix offline and published the verified file after reconnecting.");
  await rpc("personal_server_remove", { id, reference: false });
  await reviewVersion.click();
  await comparison().getByText("La conexión ya no está configurada.", { exact: true }).waitFor();
  assert.equal(await button("Copiar versión actual").count(), 0);
  await button("Volver a Descargas").click();
  assert.equal((await rpc("get_snapshot")).jobs.length, 4);
  if (process.env.NABNAB_TEST_OPENSSL) await verifyWebdav(process.env.NABNAB_TEST_OPENSSL);
  if (process.env.NABNAB_TEST_SSH2_MODULE) await verifySftp(process.env.NABNAB_TEST_SSH2_MODULE);
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
