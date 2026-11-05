# Browser extension and local bridge

`browser-extension/` is a Manifest V3 extension for Chrome and Edge. It sends the active tab or a link chosen from the context menu to the desktop queue. It does not extract, download or convert media. There are no content scripts, cookie permissions, navigation listeners or external-message entry points. Firefox has not been validated.

This implementation uses one authenticated loopback HTTP transport. The extension has no predetermined store ID and the existing installer has no Native Messaging host registration. Manual pairing permits an unpacked extension in either browser without inventing an authorized extension ID or installing another host. Native Messaging can replace this transport if packaging later supplies stable extension IDs and host registration. There is no second transport.

## Load and pair

1. Open `chrome://extensions` or `edge://extensions`, enable developer mode, and load `browser-extension/` as an unpacked extension.
2. Open the extension's settings and copy its 32-letter extension ID.
3. Open nabnab, then Ajustes > Navegador. Paste the ID and generate a pairing token.
4. Paste the token in the extension's settings and select `Guardar y comprobar conexión`.
5. Open a public media page, choose a profile from the popup and select `Enviar pestaña actual`. The link's context menu also offers video and audio actions.

nabnab must be running to receive links. The popup confirms an authenticated connection and displays reception only after the native app returns job IDs. A failed request can be retried with the same request ID. Pending requests remain in the browser's in-memory session storage until receipt or forgetting the pairing. Closing the browser discards this retry state.

The extension stores its bearer token in its own local extension storage, not sync storage. nabnab displays the token once and persists only its SHA-256 hash. Forgetting a token in the extension removes its copy. Revoking the client in nabnab invalidates that token, including copies; removing the last client closes the listener.

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

The extension rejects a receipt for another request, an unsupported version or an empty list of jobs. Tokens appear only in the Authorization header, never in URLs, job records or receipts. Errors returned by the native queue pass through the diagnostic sanitizer.

## Verification and limits

Run `node --test tests/browser-extension.test.mjs` for extension protocol behavior, stable retry IDs, receipt validation and the packaged permission boundary. Native `browser::tests` exercise host/origin/token rejection, bounded HTTP parsing, message validation, exact CORS and hash-only storage. These tests do not substitute for loading the extension and pairing it in a real browser.

On 5 November 2026, `tests/native-extension.browser.mjs` passed six checks in an isolated Edge 156.0.4314.58 profile with the real native application. It loaded the unpacked service worker, paired through the options UI, sent the active tab after a browser-action permission grant, verified receipt deduplication, rejected an invalid token and confirmed that revocation blocked the extension. The public sample page used a local HTML fixture; this checks extension permissions and transport, not a platform download. No token was logged. The final rerun is `artifacts/native-extension/run-oiBFMQ/results.json`, with a sanitized copy in [verification/native-extension.json](verification/native-extension.json).

The final native run exposed an intermittent HTTP 400 when a client connected before its first request bytes were ready. Accepted sockets now explicitly use blocking I/O with two-second read/write timeouts inside the existing bounded worker; the listener remains nonblocking. A delayed TCP client reproduces the old failure and passes after the fix. Host, Origin, bearer authentication, payload limits and the five-second parser deadline are unchanged. The native app's eleven-check workflow and the six real Edge checks both passed after this correction.

The actual Windows context-menu click remains unverified. The computer-use controller refused that input because it could not confidently identify the current browser URL. No alternate native input method bypassed that refusal. Chrome and Firefox were not certified by this run.

The extension performs no session import. A platform requiring login is handled by the desktop application's explicit session workflow. Sending a link does not authorize bypassing DRM or access controls.
