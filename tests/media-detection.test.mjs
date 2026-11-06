import test from "node:test";
import assert from "node:assert/strict";
import vm from "node:vm";
import { normalizeMediaScan, mediaSummaries, scanMedia, selectedMediaRequest } from "../browser-extension/media.js";
import { makeRequest } from "../browser-extension/protocol.js";

const signed = "https://cdn.example.org/video.mp4?token=secret&signature=private";
const pageUrl = "https://example.org/watch?id=authorized";
function scan(items, extra = {}) {
  let next = 0;
  return normalizeMediaScan({ items, ...extra }, { id: () => `opaque-${++next}` });
}

test("detection groups loaded media and exposes format and known dimensions without signed URLs", () => {
  const result = scan([
    { url: signed, width: 1280, height: 720, size: null },
    { url: signed + "#t=3", size: 2048 },
    { url: "https://cdn.example.org/master.m3u8?token=secret" },
    { url: "https://cdn.example.org/manifest.mpd" },
    { url: "https://cdn.example.org/track.webm", mime: "audio/opus" },
    { url: "https://cdn.example.org/file", mime: "audio/mpeg; codecs=mp3" },
  ]);
  assert.deepEqual(mediaSummaries(result), {
    candidates: [
      { id: "opaque-1", host: "cdn.example.org", format: "MP4", width: 1280, height: 720, size: 2048, protected: false, manifest: false },
      { id: "opaque-2", host: "cdn.example.org", format: "HLS", width: null, height: null, size: null, protected: false, manifest: true },
      { id: "opaque-3", host: "cdn.example.org", format: "DASH", width: null, height: null, size: null, protected: false, manifest: true },
      { id: "opaque-4", host: "cdn.example.org", format: "Opus", width: null, height: null, size: null, protected: false, manifest: false },
      { id: "opaque-5", host: "cdn.example.org", format: "MP3", width: null, height: null, size: null, protected: false, manifest: false },
    ], blobs: 0, drmDetected: false,
  });
  const visible = JSON.stringify(mediaSummaries(result));
  for (const secret of ["token", "signature", "secret", "private", "video.mp4"]) assert.equal(visible.includes(secret), false);
  assert.equal(result.candidates[0].url, signed);
});

test("blob, credentials, unsupported URLs and unknown formats cannot become download candidates", () => {
  const result = scan([
    { url: "blob:https://example.org/local-only" },
    { url: "https://user:password@example.org/video.mp4" },
    { url: "file:///C:/private.mp4" },
    { url: "data:video/mp4,private" },
    { url: "javascript:alert(1)" },
    { url: "https://example.org/.constructor" },
    { url: "https://example.org/file", mime: "__proto__" },
    { url: "https://example.org/video.mp4", width: -4, size: Infinity },
  ]);
  assert.equal(result.blobs, 1);
  assert.deepEqual(result.candidates.map(({ format, width, size }) => ({ format, width, size })), [{ format: "MP4", width: null, size: null }]);
});

test("only a fresh selected opaque ID from the same active tab is sent to the existing preparer", () => {
  const result = scan([{ url: signed }]);
  const snapshot = { ...result, tabId: 42, pageUrl, createdAt: 1000 };
  const request = selectedMediaRequest(snapshot, { tabId: 42, pageUrl, ids: ["opaque-1", "opaque-1"], requestId: "media-request-123", now: 1001 });
  assert.deepEqual(makeRequest(request), { version: 1, requestId: "media-request-123", url: pageUrl, urls: [signed], intent: "prepare", profile: "source" });
  for (const overrides of [{ tabId: 43 }, { pageUrl: "https://other.example.org/" }, { now: 301001 }, { ids: ["other-id"] }, { ids: [] }]) {
    assert.throws(() => selectedMediaRequest(snapshot, { tabId: 42, pageUrl, ids: ["opaque-1"], requestId: "media-request-123", now: 1001, ...overrides }));
  }
});

test("DRM detected on an element survives duplicate grouping and blocks sending its resource", () => {
  const result = scan([{ url: signed }, { url: signed, protected: true }]);
  assert.equal(result.candidates.length, 1);
  assert.equal(result.drmDetected, true);
  assert.equal(mediaSummaries(result).candidates[0].protected, true);
  assert.throws(() => selectedMediaRequest({ ...result, tabId: 1, pageUrl, createdAt: 1000 }, { tabId: 1, pageUrl, ids: ["opaque-1"], now: 1001 }), /protegido/);
});

test("the injected scan reads current-frame DOM and existing performance entries without fetching", () => {
  const source = { src: "https://cdn.example.org/audio.mp3", type: "audio/mpeg" };
  const video = { currentSrc: signed, src: "", videoWidth: 1920, videoHeight: 1080, mediaKeys: {}, getAttribute: () => "video/mp4", querySelectorAll: selector => { assert.equal(selector, "source[src]"); return [source]; } };
  const raw = vm.runInNewContext(`(${scanMedia.toString()})()`, {
    document: { querySelectorAll: selector => { assert.equal(selector, "video,audio"); return [video]; } },
    performance: { getEntriesByType: type => { assert.equal(type, "resource"); return [{ name: "https://cdn.example.org/master.m3u8?token=hidden", encodedBodySize: 512 }, { name: "https://example.org/ordinary.js" }]; } },
    location: { href: pageUrl }, fetch: () => assert.fail("Detection must not fetch a resource"),
  });
  const result = scan(raw.items, raw);
  assert.deepEqual(result.candidates.map(({ format, protected: drm }) => ({ format, drm })), [{ format: "MP4", drm: true }, { format: "MP3", drm: true }, { format: "HLS", drm: true }]);
  assert.equal(result.candidates[2].size, 512);
});

test("detection bounds candidates and refuses a selection larger than the authenticated bridge body", () => {
  const result = scan(Array.from({ length: 600 }, (_, i) => ({ url: `https://cdn.example.org/${i}.mp4` })));
  assert.equal(result.candidates.length, 200);
  const long = scan([1, 2].map(i => ({ url: `https://cdn.example.org/${i}.mp4?token=${"x".repeat(7800)}` })));
  assert.throws(() => selectedMediaRequest({ ...long, tabId: 1, pageUrl, createdAt: 1000 }, { tabId: 1, pageUrl, ids: ["opaque-1", "opaque-2"], now: 1001 }), /largos/);
});

test("the service worker scans only after a request and sends selected media through the paired bridge", async () => {
  const previousChrome = globalThis.chrome, previousFetch = globalThis.fetch;
  const session = {}, calls = [];
  let listener;
  globalThis.chrome = {
    runtime: { id: "test-extension", onMessage: { addListener(value) { listener = value; } }, onInstalled: { addListener() {} }, onStartup: { addListener() {} } },
    contextMenus: { onClicked: { addListener() {} } },
    tabs: { async query() { calls.push("tab"); return [{ id: 42, url: pageUrl }]; } },
    scripting: { async executeScript({ target, func }) { assert.deepEqual(target, { tabId: 42 }); assert.equal(func.name, "scanMedia"); calls.push("scan"); return [{ result: { pageUrl, items: [{ url: signed, width: 1280, height: 720 }] } }]; } },
    storage: {
      local: { async get() { return { pairingToken: "a".repeat(64) }; } },
      session: { async get(key) { return { [key]: session[key] }; }, async set(values) { Object.assign(session, structuredClone(values)); }, async remove(keys) { for (const key of Array.isArray(keys) ? keys : [keys]) delete session[key]; } },
    },
    action: { async setBadgeText() {} },
  };
  globalThis.fetch = async (url, options) => {
    assert.equal(url, "http://127.0.0.1:49587/enqueue");
    assert.equal(options.credentials, "omit");
    assert.equal(options.headers.Authorization, "Bearer " + "a".repeat(64));
    const request = JSON.parse(options.body);
    assert.deepEqual(request.urls, [signed]); assert.equal(request.url, pageUrl); assert.equal(request.intent, "prepare");
    calls.push("paired-send");
    return new Response(JSON.stringify({ version: 1, requestId: request.requestId, acceptedJobIds: [], acceptedDraftId: "draft-media", intent: "prepare" }));
  };
  try {
    await import("../browser-extension/background.js");
    assert.deepEqual(calls, []);
    const dispatch = message => new Promise(resolve => listener(message, { id: "test-extension" }, resolve));
    const detected = await dispatch({ kind: "detectMedia" });
    assert.equal(detected.ok, true); assert.equal(detected.result.candidates.length, 1);
    assert.equal(JSON.stringify(detected).includes("secret"), false);
    assert.equal(session.detectedMedia.candidates[0].url, signed);
    const sent = await dispatch({ kind: "sendDetectedMedia", ids: [detected.result.candidates[0].id], profile: "source", requestId: "media-request-123" });
    assert.equal(sent.ok, true); assert.equal(sent.result.acceptedDraftId, "draft-media");
    assert.deepEqual(calls, ["tab", "scan", "tab", "paired-send"]);
    assert.equal(session.pendingRequest, undefined);
  } finally { globalThis.chrome = previousChrome; globalThis.fetch = previousFetch; }
});
