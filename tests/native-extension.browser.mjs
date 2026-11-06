// Loads the real unpacked extension and pairs it with an isolated native debug app.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, writeFile, rm, access } from "node:fs/promises";
import path from "node:path";
import os from "node:os";

const { chromium } = createRequire(import.meta.url)(process.env.PLAYWRIGHT_MODULE || "playwright");
const root = path.resolve(import.meta.dirname, "..");
const executable = process.env.NABNAB_EXECUTABLE || path.join(root, "src-tauri/target/debug/nabnab.exe");
const extension = path.join(root, "browser-extension");
const channel = process.env.NABNAB_BROWSER_CHANNEL || "msedge";
const port = Number(process.env.NABNAB_DEBUG_PORT || 9223);
const publicMedia = "https://www.w3schools.com/html/mov_bbb.mp4";
const contextLink = publicMedia + "?nabnab=link-menu";
await mkdir(path.join(root, "artifacts/native-extension"), { recursive: true });
const data = await mkdtemp(path.join(root, "artifacts/native-extension/run-"));
const profile = await mkdtemp(path.join(os.tmpdir(), "nabnab-extension-qa-"));
let child, nativeBrowser, nativePage, context, worker, extensionId, options;
const results = [];
const errors = [];
const bridgeRequests = [];
async function waitFor(check, timeout = 30000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await check();
    if (value) return value;
    await new Promise(resolve => setTimeout(resolve, 150));
  }
  throw new Error("Timed out waiting for the isolated extension test.");
}
async function rpc(cmd, args = {}) {
  return nativePage.evaluate(async ({ cmd, args }) => {
    const { invoke } = await import("/node_modules/@tauri-apps/api/core.js");
    return invoke(cmd, args);
  }, { cmd, args });
}
async function extensionCall(kind, extra = {}) {
  const result = await options.evaluate(async message => chrome.runtime.sendMessage(message), { kind, ...extra });
  if (!result?.ok) throw new Error(result?.error || "The extension did not acknowledge the request.");
  return result.result;
}
async function cancelJobs(ids) {
  if (ids.length) await rpc("job_action", { ids, action: "cancel" });
}
async function close() {
  if (worker) {
    try { await worker.evaluate(async () => { await chrome.storage.local.clear(); await chrome.storage.session.clear(); }); } catch {}
  }
  try { await context?.close(); } catch {}
  if (nativePage && !nativePage.isClosed()) {
    try { await nativePage.evaluate(async () => { const { getCurrentWindow } = await import("/node_modules/@tauri-apps/api/window.js"); await getCurrentWindow().close(); }); } catch {}
  }
  if (child && child.exitCode === null) {
    await Promise.race([new Promise(resolve => child.once("exit", resolve)), new Promise(resolve => setTimeout(resolve, 4000))]);
    if (child.exitCode === null) child.kill();
  }
  try { await nativeBrowser?.close(); } catch {}
  assert.equal(path.dirname(path.resolve(profile)), path.resolve(os.tmpdir()));
  assert.match(path.basename(profile), /^nabnab-extension-qa-/);
  for (let attempt = 0; attempt < 5; attempt++) {
    try { await rm(profile, { recursive: true, force: true }); return; }
    catch (error) { if (attempt === 4) throw error; await new Promise(resolve => setTimeout(resolve, 500)); }
  }
}

try {
  child = spawn(executable, [], { cwd: root, windowsHide: true, stdio: "ignore", env: {
    ...process.env, NABNAB_TEST_DATA_DIR: data, WEBVIEW2_USER_DATA_FOLDER: path.join(data, "webview"),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  } });
  await waitFor(async () => {
    if (child.exitCode !== null) throw new Error(`Native app exited ${child.exitCode}`);
    try { return (await fetch(`http://127.0.0.1:${port}/json/version`)).ok; } catch { return false; }
  });
  nativeBrowser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
  nativePage = await waitFor(() => nativeBrowser.contexts()[0]?.pages().find(page => /localhost:1420|127\.0\.0\.1:1420/.test(page.url())));
  await nativePage.waitForSelector('[data-page="library"]');
  await waitFor(async () => { try { return (await rpc("get_snapshot")).settings.folder; } catch { return false; } });

  context = await chromium.launchPersistentContext(profile, {
    channel, headless: process.env.NABNAB_EXTENSION_HEADED !== "1", viewport: { width: 1180, height: 780 },
    ignoreDefaultArgs: ["--disable-extensions"],
    args: [`--disable-extensions-except=${extension}`, `--load-extension=${extension}`],
  });
  context.on("request", request => {
    if (request.url().startsWith("http://127.0.0.1:49587/")) {
      let sent;try{sent=request.postDataJSON();}catch{}
      void request.allHeaders().then(headers => bridgeRequests.push({ method: request.method(), path: new URL(request.url()).pathname, origin: headers.origin || null, authorizationPresent: Boolean(headers.authorization),
        requestId:typeof sent?.requestId==="string"?sent.requestId:null,urlIsTabFixture:sent?.url===publicMedia,urlIsLinkFixture:sent?.url===contextLink })).catch(()=>{});
    }
  });
  worker = await waitFor(() => context.serviceWorkers().find(item => item.url().endsWith("/background.js")), 15000);
  extensionId = new URL(worker.url()).hostname;
  assert.match(extensionId, /^[a-p]{32}$/);
  results.push({ case: "actual unpacked extension service worker", status: "passed", browser: channel, version: context.browser()?.version() || null });

  options = await context.newPage();
  options.on("pageerror", error => errors.push(error.message));
  await options.goto(`chrome-extension://${extensionId}/options.html`);
  assert.equal(await options.locator("#extension-id").inputValue(), extensionId);
  await options.screenshot({ path: path.join(data, "unpaired-options.png") });
  const pairing = await rpc("bridge_pair", { extensionId });
  // Keep the generated test token in memory; never print it or save browser profiles in artifacts.
  await options.locator("#token").evaluate((element, value) => { element.value = value; element.dispatchEvent(new Event("input", { bubbles: true })); }, pairing.token);
  await options.getByRole("button", { name: "Guardar y comprobar conexión" }).click();
  await options.getByText("Conectado a nabnab. Ya puedes enviar pestañas y enlaces.", { exact: true }).waitFor({ timeout: 15000 });
  assert.equal(await options.locator("#token").inputValue(), "");
  await options.screenshot({ path: path.join(data, "paired-options.png") });
  results.push({ case: "options UI pairing and real authenticated extension-origin CORS", status: "passed" });

  await context.route(publicMedia, route => route.fulfill({ contentType: "text/html", body: `<!doctype html><html lang="es"><title>nabnab extension QA media</title><body><h1>Authorized media link fixture</h1><a id="media-link" href="${contextLink}">Send this public media link</a></body></html>` }));
  const mediaPage = await context.newPage();
  await mediaPage.goto(publicMedia);
  const sourceTab = await worker.evaluate(async () => (await chrome.tabs.query({ active: true, currentWindow: true }))[0]);
  assert.ok(sourceTab?.id);
  assert.equal(sourceTab.url,undefined);
  const browserCdp=await context.browser().newBrowserCDPSession();
  const targets=await browserCdp.send("Target.getTargets",{filter:[{type:"tab",exclude:false}]});
  const tabTarget=targets.targetInfos.find(target=>target.url===publicMedia);
  assert.ok(tabTarget);
  await browserCdp.send("Extensions.triggerAction",{id:extensionId,targetId:tabTarget.targetId});
  const granted=await worker.evaluate(async()=> (await chrome.tabs.query({active:true,currentWindow:true}))[0]);
  assert.equal(granted.url,publicMedia);
  // Edge's toolbar popup is transient under headless CDP; drive its same document in an inactive tab after the actual action granted activeTab.
  await worker.evaluate(async url=>chrome.tabs.create({url,active:false}),`chrome-extension://${extensionId}/popup.html`);
  const popup = await waitFor(() => context.pages().find(page => page.url().endsWith("/popup.html")),10000);
  popup.on("pageerror", error => errors.push(error.message));
  await popup.getByText("Conectado a nabnab", { exact: true }).waitFor();
  await popup.setViewportSize({width:360,height:640});
  await popup.locator('option[value="preset:reference"]').waitFor({state:'attached'});
  const beforePreparation=(await rpc('get_snapshot')).jobs.length;
  await popup.locator('#collect').click();await popup.locator('#candidate-links input').first().waitFor();
  assert.equal(await popup.locator('#candidate-links input').count(),1);assert.equal(await popup.locator('#candidate-links input').first().inputValue(),contextLink);
  await popup.locator('#candidate-links input').first().check();await popup.locator('#profile').selectOption('preset:reference');
  await popup.locator('#send').click();await popup.getByText('Recibido en el preparador de nabnab. Revisa los enlaces antes de añadirlos a la cola.',{exact:true}).waitFor();
  const preparedState=await rpc('get_snapshot');assert.equal(preparedState.jobs.length,beforePreparation);assert.ok(preparedState.drafts.some(draft=>draft.presetId==='reference'&&draft.items.length===1));
  await nativePage.locator('.batch-workspace').waitFor();await popup.screenshot({path:path.join(data,'popup-collected-prepared.png')});
  await popup.locator('#intent').selectOption('save');await popup.locator('#send').click();await popup.getByText('Guardado en Biblioteca > Para después. No se inició ninguna descarga.',{exact:true}).waitFor();
  const savedState=await rpc('get_snapshot');assert.equal(savedState.jobs.length,beforePreparation);assert.ok(savedState.drafts.some(draft=>draft.savedForLater));
  results.push({case:'real activeTab link collection, local presets, preparation and save intents create drafts without download jobs',status:'passed'});
  await popup.locator('#candidate-links input').first().uncheck();await popup.locator('#intent').selectOption('download');await popup.locator('#profile').selectOption('mp4');
  const before = (await rpc("get_snapshot")).jobs.length;
  await popup.getByRole("button", { name: "Enviar pestaña actual", exact: true }).click();
  await popup.getByText("Recibido en nabnab. Consulta Descargas en la aplicación.", { exact: true }).waitFor({ timeout: 15000 });
  const newJobs = (await rpc("get_snapshot")).jobs.slice(before);
  assert.equal(newJobs.length, 1);
  assert.equal(newJobs[0].source.url, new URL(publicMedia).origin);
  assert.equal(typeof newJobs[0].source.urlRef,"string");
  assert.ok(bridgeRequests.some(request=>request.path==="/enqueue"&&request.urlIsTabFixture));
  await cancelJobs(newJobs.map(job => job.id));
  await popup.screenshot({ path: path.join(data, "popup-received.png") });
  results.push({ case: "actual popup sends active tab once and receives native job", status: "passed" });

  const replay = { requestId: "actual-extension-replay-1", url: publicMedia, profile: "mp3" };
  const receipt = await extensionCall("enqueue", { request: replay });
  const second = await extensionCall("enqueue", { request: replay });
  assert.deepEqual(second.acceptedJobIds, receipt.acceptedJobIds);
  assert.equal((await rpc("get_snapshot")).jobs.filter(job => job.clientRequest === `${extensionId}:${replay.requestId}`).length, 1);
  await cancelJobs(receipt.acceptedJobIds);
  results.push({ case: "actual service worker replay keeps one native job", status: "passed" });

  const unauthorizedStatus=await options.evaluate(async()=> (await fetch("http://127.0.0.1:49587/health",{method:"POST",headers:{Authorization:"Bearer "+"b".repeat(64),"Content-Type":"application/json"},body:JSON.stringify({version:1})})).status);
  assert.equal(unauthorizedStatus,401);
  assert.ok(bridgeRequests.some(request=>request.path==="/health"&&request.method==="POST"&&request.origin===`chrome-extension://${extensionId}`&&request.authorizationPresent));
  results.push({case:"actual extension Origin is browser-supplied and invalid token is rejected",status:"passed"});

  if(process.env.NABNAB_EXTENSION_CONTEXT_MENU==="1"){
    await popup.close();await mediaPage.bringToFront();
    const count=(await rpc("get_snapshot")).jobs.length;
    await writeFile(path.join(data,"context-menu-ready.json"),JSON.stringify({source:contextLink,title:"nabnab extension QA media",action:"Use the real browser link context menu to send video to nabnab."}));
    console.log(JSON.stringify({waitingFor:"native context-menu click",artifactDirectory:path.relative(root,data)}));
    const accepted=await waitFor(async()=>{const jobs=(await rpc("get_snapshot")).jobs.slice(count);return jobs.length?jobs:false;},180000);
    assert.equal(accepted.length,1);assert.equal(accepted[0].source.url,new URL(contextLink).origin);assert.equal(accepted[0].source.format,"mp4");
    assert.ok(bridgeRequests.some(request=>request.path==="/enqueue"&&request.urlIsLinkFixture));
    await cancelJobs(accepted.map(job=>job.id));
    await writeFile(path.join(data,"context-menu-result.json"),JSON.stringify({status:"passed",acceptedJobs:accepted.length,profile:"mp4",origin:"paired real extension"}));
    await waitFor(async()=>{try{await access(path.join(data,"context-menu-ui-observed"));return true;}catch{return false;}},180000);
    results.push({case:"native Edge link context-menu action receives one native job",status:"passed"});
  }

  await rpc("bridge_revoke", { extensionId });
  await assert.rejects(() => extensionCall("health"));
  results.push({ case: "revoked real extension cannot connect", status: "passed" });
  assert.equal(errors.length, 0);
  await writeFile(path.join(data, "results.json"), JSON.stringify({ date: new Date().toISOString(), native: true, browser: channel, isolatedData: true, extensionLoaded: true,
    browserPageFixture: "Public URL is rendered with a local HTML link fixture; this tests extension permissions/transport, not platform downloading.",
    popupMethod:"DevTools Extensions.triggerAction grants the real activeTab permission; the same popup document is driven in an inactive extension tab.", results, pending: process.env.NABNAB_EXTENSION_CONTEXT_MENU==="1"?[]:["native context-menu click"] }, null, 2));
  console.log(JSON.stringify({ status: "passed", artifactDirectory: path.relative(root, data), checks: results.length, pending: process.env.NABNAB_EXTENSION_CONTEXT_MENU==="1"?[]:["native context-menu click"] }));
} catch (error) {
  const message = error instanceof Error ? error.message : String(error);
  const uiMessage = options && !options.isClosed() ? await options.locator("#message").textContent().catch(() => null) : null;
  await writeFile(path.join(data, "failure.json"), JSON.stringify({ date: new Date().toISOString(), browser: channel, results, bridgeRequests, uiMessage, error: message.slice(0, 2000) }, null, 2));
  throw error;
} finally {
  await close();
}
