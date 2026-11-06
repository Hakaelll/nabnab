# Browser extension and local bridge

`browser-extension/` is a Manifest V3 extension for Chrome and Edge. It sends the active tab, a context-menu link, an explicitly selected page batch or detected media candidates to the desktop. Destinations are Download, Prepare and Save for later. It does not download or convert media. There are no persistent content scripts, cookie permissions, navigation listeners or external-message entry points. `scripting` and `activeTab` inspect the current page only after the user selects Collect links or Detect media. No global page access is requested. Firefox has not been validated.

This implementation uses one authenticated loopback HTTP transport. The extension has no predetermined store ID and the existing installer has no Native Messaging host registration. Manual pairing permits an unpacked extension in either browser without inventing an authorized extension ID or installing another host. Native Messaging can replace this transport if packaging later supplies stable extension IDs and host registration. There is no second transport.

## Load and pair

1. Open `chrome://extensions` or `edge://extensions`, enable developer mode, and load `browser-extension/` as an unpacked extension.
2. Open the extension's settings and copy its 32-letter extension ID.
3. Open nabnab, then Ajustes > Navegador. Paste the ID and generate a pairing token.
4. Paste the token in the extension's settings and select `Guardar y comprobar conexión`.
5. Open a public media page, choose a profile from the popup and select `Enviar pestaña actual`. The link's context menu also offers video and audio actions.

nabnab must be running to receive links. The popup displays reception only after the native app returns job IDs for Download or a draft ID for Prepare and Save. A failed request can be retried with the same request ID. Pending requests remain in the browser's in-memory session storage until receipt or forgetting the pairing. Closing the browser discards this retry state.

The extension stores its bearer token in its own local extension storage, not sync storage. nabnab displays the token once and persists only its SHA-256 hash. Forgetting a token in the extension removes its copy. Revoking the client in nabnab invalidates that token, including copies; removing the last client closes the listener.

## Detect media in this tab

Select `Detectar medios en esta pestaña` in the popup to inspect the main frame's current video/audio/source elements and existing resource-timing entries. The scan is a one-time action. It adds no listeners, does not observe later browsing and does not inspect other tabs or cross-origin frames. The existing [`activeTab`](https://developer.chrome.com/docs/extensions/develop/concepts/activeTab) and [`scripting`](https://developer.chrome.com/docs/extensions/reference/api/scripting) permissions authorize this page inspection. The extension does not fetch media hosts, so it does not request permission to read those hosts, download manifests or forward cookies.

The popup lists MP4, WebM, supported audio suffixes/MIME types, HLS and DASH candidates when their URLs or declared MIME types identify them. Identical URLs are grouped. Dimensions come from the media element when available; size is the observed encoded resource size, which may describe a range or manifest rather than the final download. Missing values stay unknown. Query parameters are retained internally for authorized signed URLs but neither those URLs nor URL paths appear in the detected-media popup. The service worker stores raw candidates in browser session storage and gives the popup opaque IDs and display metadata. New scans replace the prior result; selections expire after five minutes or when the active tab or page URL changes. Closing the browser discards this state. `Descartar detección` removes it explicitly.

Select candidates and use `Preparar selección en nabnab`. The worker resolves the IDs and sends only the selected URLs, page URL and selected output profile through the existing paired loopback transport. It requests a preparation draft, so detection and sending do not immediately start a download. The authenticated desktop still validates source URLs, redirects and supported operations. A selection with long signed URLs may need to be sent in smaller groups because the existing bridge retains its 16 KiB body limit. No authentication from the page is copied to resource hosts.

A `blob:` source is reported as unavailable to the desktop. If the element already has EME `mediaKeys`, the popup marks its sources as protected and prevents sending them. Resource candidates on a page with detected protected playback are also conservatively blocked. This is a limited signal, not proof that every other source is unprotected. HLS/DASH remain candidates with compatibility and DRM explicitly unverified. A detected URL does not guarantee that nabnab supports saving it; this change does not add keys, EME bypass, covert capture or a manifest extractor.

Run `node --test tests/media-detection.test.mjs tests/browser-extension.test.mjs` to verify the scanner's inputs, grouping, secret-free summaries, scoped selections, DRM/blob handling, bridge limits and selected-ID transport through the service worker. These fixture checks do not certify live detection on a provider page. No new browser permissions or external dependencies were added.

## Native boundaries

The listener binds only `127.0.0.1:49587`, after an explicit pairing or a later startup with saved, consented clients. Without paired clients no listener starts. The application retains the historical configuration location and stores a separate versioned `browser-clients.json` containing extension IDs and token hashes. Updates use a checked temporary file and atomic rename.

Every health or enqueue request must have:

- Exact `Host: 127.0.0.1:49587`.
- Exact `Origin: chrome-extension://<paired-32-letter-id>`.
- `Authorization: Bearer <64-hex-character-token>` matching that client.

There is no discovery endpoint, pairing endpoint or HTTP token-delivery endpoint. CORS grants only an exact paired origin. Preflight requests cannot contain an authorization token in normal browser CORS, so they disclose no data and only allow GET/POST with Authorization and Content-Type for an already paired origin. They also permit the browser's private-network preflight for that origin. Unpaired sites, `null` origins, origin suffixes, other loopback host spellings and rebinding hosts are rejected.

HTTP parsing rejects duplicate headers, transfer encoding, unexpected extra body bytes and invalid content lengths. Limits are 8 KiB of headers, 16 KiB of JSON and 8 KiB per URL. Socket reads/writes time out after two seconds; reads also have a five-second overall deadline. Requests are handled serially in one dedicated thread, so clients cannot create an unbounded thread pool. Responses disable caching and MIME sniffing.

## Protocol version 1

The extension uses `POST /health` with `Content-Type: application/json` and body `{"version":1}`. Edge omits `Origin` from extension GET requests; POST supplies the actual extension origin, which the native server still validates exactly. An authenticated GET with an explicit valid Origin and no body remains accepted for native diagnostics. Both return:

```json
{"version":1,"product":"nabnab","connected":true}
```

`POST /enqueue` requires exactly `Content-Type: application/json`:

```json
{"version":1,"requestId":"stable-request-id","url":"https://www.youtube.com/watch?v=public","profile":"mp4"}
```

Profiles are `mp4`, `mp3`, `source` and `audio`. Requests reject unsupported versions, unknown fields, malformed IDs and URLs that fail the native public-HTTP boundary. The server assigns `client_id` from the authenticated extension; JSON cannot supply a trusted client ID. The application namespaces request IDs by that client and deduplicates reception before scheduling work.

An accepted request returns:

```json
{"version":1,"requestId":"stable-request-id","acceptedJobIds":["native-job-id"]}
```

Optional fields extend the same version 1 protocol: `intent` is `download`, `prepare` or `save`; `presetId` references a local preset; `urls` contains at most 200 selected links and is accepted only for preparation or saving. The 16 KiB request-body limit also applies. Paths, commands and unrecognized fields remain rejected. `POST /presets` with `{"version":1}` returns only local IDs, names and revisions, through the same authentication and origin checks.

Prepare and Save return `acceptedDraftId`, the matching `intent` and an empty `acceptedJobIds` array. Repeating the request returns the same saved draft. Download requires at least one job ID. The extension rejects receipts with a different request, version, intent or output kind. Tokens appear only in the Authorization header, never in URLs, job records or receipts.

See [local productivity](PRODUCTIVITY.md) for the new flows, screenshots and validation performed on 6 November 2026.

## Verification and limits

Run `node --test tests/browser-extension.test.mjs` for extension protocol behavior, stable retry IDs, receipt validation and the packaged permission boundary. Native `browser::tests` exercise host/origin/token rejection, bounded HTTP parsing, message validation, exact CORS and hash-only storage. These tests do not substitute for loading the extension and pairing it in a real browser.

On 5 November 2026, `tests/native-extension.browser.mjs` passed six checks in an isolated Edge 156.0.4314.58 profile with the real native application. It loaded the unpacked service worker, paired through the options UI, sent the active tab after a browser-action permission grant, verified receipt deduplication, rejected an invalid token and confirmed that revocation blocked the extension. The public sample page used a local HTML fixture; this checks extension permissions and transport, not a platform download. No token was logged. The final rerun is `artifacts/native-extension/run-oiBFMQ/results.json`, with a sanitized copy in [verification/native-extension.json](verification/native-extension.json).

The final native run exposed an intermittent HTTP 400 when a client connected before its first request bytes were ready. Accepted sockets now explicitly use blocking I/O with two-second read/write timeouts inside the existing bounded worker; the listener remains nonblocking. A delayed TCP client reproduces the old failure and passes after the fix. Host, Origin, bearer authentication, payload limits and the five-second parser deadline are unchanged. The native app's eleven-check workflow and the six real Edge checks both passed after this correction.

The actual Windows context-menu click remains unverified. The computer-use controller refused that input because it could not confidently identify the current browser URL. No alternate native input method bypassed that refusal. Chrome and Firefox were not certified by this run.

The extension performs no session import. A platform requiring login is handled by the desktop application's explicit session workflow. Sending a link does not authorize bypassing DRM or access controls.
