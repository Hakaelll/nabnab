import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { makeRequest, createClient, validToken } from "../browser-extension/protocol.js";

const token = "a".repeat(64);
const request = { requestId: "request-123456", url: "https://www.youtube.com/watch?v=authorized&list=keep", profile: "mp4" };

test("sending a selected link preserves access parameters and rejects executable or credential URLs", () => {
  assert.deepEqual(makeRequest(request), { version: 1, ...request });
  for (const url of ["file:///private.mp4", "javascript:alert(1)", "chrome://settings", "https://user:password@example.com/video", "not a URL", "https://example.com/" + "x".repeat(8192)]) {
    assert.throws(() => makeRequest({ ...request, url }));
  }
  assert.throws(() => makeRequest({ ...request, profile: "--exec" }));
  assert.throws(() => makeRequest({ ...request, requestId: "other:client" }));
});

test("retrying an unacknowledged request keeps its ID and sends the token only as authorization", async () => {
  const calls = [];
  const client = createClient({ token, fetcher: async (url, options) => {
    calls.push({ url, options });
    if (calls.length === 1) throw new Error("connection lost after native acceptance");
    return new Response(JSON.stringify({ version: 1, requestId: request.requestId, acceptedJobIds: ["job-123"] }), { status: 200 });
  } });
  await assert.rejects(() => client.enqueue(request));
  assert.deepEqual(await client.enqueue(request), { version: 1, requestId: request.requestId, acceptedJobIds: ["job-123"] });
  assert.equal(calls.length, 2);
  for (const { url, options } of calls) {
    assert.equal(url, "http://127.0.0.1:49587/enqueue");
    assert.equal(options.headers.Authorization, "Bearer " + token);
    assert.equal(options.credentials, "omit");
    assert.equal(options.redirect, "error");
    assert.equal(JSON.parse(options.body).requestId, request.requestId);
    assert.equal(JSON.parse(options.body).url, request.url);
    assert.equal(options.body.includes(token), false);
    assert.equal(url.includes(token), false);
  }
});

test("a client never reports reception without a matching native receipt", async () => {
  for (const receipt of [
    { version: 1, requestId: "different-request", acceptedJobIds: ["job-1"] },
    { version: 2, requestId: request.requestId, acceptedJobIds: ["job-1"] },
    { version: 1, requestId: request.requestId, acceptedJobIds: [] },
    { version: 1, requestId: request.requestId, acceptedJobIds: [null] },
  ]) {
    const client = createClient({ token, fetcher: async () => new Response(JSON.stringify(receipt), { status: 200 }) });
    await assert.rejects(() => client.enqueue(request), /confirmado/);
  }
});

test("connection state requires an authenticated compatible nabnab response", async () => {
  let calls = 0;
  const client = createClient({ token, fetcher: async (url, options) => {
    calls++;
    assert.equal(url, "http://127.0.0.1:49587/health");
    assert.equal(options.headers.Authorization, "Bearer " + token);
    assert.equal(options.method,"POST");
    assert.deepEqual(JSON.parse(options.body),{version:1});
    return new Response(JSON.stringify({ version: 1, product: "nabnab", connected: true }), { status: 200 });
  } });
  assert.equal(await client.health(), true);
  assert.equal(calls, 1);
  const unauthorized = createClient({ token, fetcher: async () => new Response("{}", { status: 401 }) });
  await assert.rejects(() => unauthorized.health(), /emparejamiento/);
  const incompatible = createClient({ token, fetcher: async () => new Response(JSON.stringify({ connected: true }), { status: 200 }) });
  await assert.rejects(() => incompatible.health(), /compatible/);
  assert.equal(validToken(token), true);
  assert.equal(validToken("token"), false);
  assert.throws(() => createClient({ token: "" }));
});

test("the extension package requests no global browsing or cookie access", async () => {
  const manifest = JSON.parse(await readFile(new URL("../browser-extension/manifest.json", import.meta.url), "utf8"));
  assert.deepEqual(new Set(manifest.permissions), new Set(["activeTab", "contextMenus", "storage", "scripting"]));
  assert.deepEqual(manifest.host_permissions, ["http://127.0.0.1:49587/*"]);
  assert.equal(manifest.content_scripts, undefined);
  assert.equal(manifest.externally_connectable, undefined);
  assert.equal(manifest.manifest_version, 3);
});

test('a selected page batch carries a preparation intent and needs a draft receipt without download jobs',async()=>{
  const selected={...request,intent:'prepare',urls:['https://www.youtube.com/watch?v=one&list=keep','https://vimeo.com/123'],presetId:'reference'};
  const outgoing=makeRequest(selected);
  assert.deepEqual(outgoing.urls,selected.urls);
  assert.equal(outgoing.intent,'prepare');assert.equal(outgoing.presetId,'reference');
  for(const receipt of [{version:1,requestId:request.requestId,intent:'prepare',acceptedJobIds:['unwanted-download'],acceptedDraftId:'draft-1'},{version:1,requestId:request.requestId,intent:'download',acceptedJobIds:[],acceptedDraftId:'draft-1'},{version:1,requestId:request.requestId,intent:'prepare',acceptedJobIds:[]}]){
    const client=createClient({token,fetcher:async()=>new Response(JSON.stringify(receipt),{status:200})});await assert.rejects(()=>client.enqueue(selected),/confirmado/);
  }
  const receipt={version:1,requestId:request.requestId,intent:'prepare',acceptedJobIds:[],acceptedDraftId:'draft-1'};
  const client=createClient({token,fetcher:async()=>new Response(JSON.stringify(receipt),{status:200})});assert.deepEqual(await client.enqueue(selected),receipt);
  assert.throws(()=>makeRequest({...selected,intent:'download'}));
  assert.throws(()=>makeRequest({...selected,presetId:'../arbitrary-command'}));
});

test('preset discovery exposes only local identifiers and labels',async()=>{
  const client=createClient({token,fetcher:async(url,options)=>{assert.equal(url,'http://127.0.0.1:49587/presets');assert.deepEqual(JSON.parse(options.body),{version:1});return new Response(JSON.stringify({version:1,presets:[{id:'audio',name:'Audio',revision:2,cookies:'must not be copied'}]}),{status:200});}});
  assert.deepEqual(await client.presets(),[{id:'audio',name:'Audio',revision:2}]);
});
