# Direct HTTP media

nabnab uses `src-tauri/src/direct.rs` for generic HTTP/HTTPS audio and video files. Generic links are not sent to yt-dlp or a remote conversion service. Known platform downloads have their own engine path; this provider does not certify those platforms.

## Network boundary

Every request and redirect parses its URL, rejects credentials and local/reserved addresses, resolves all addresses, and pins those approved addresses with reqwest's `resolve_to_addrs`. The HTTP client disables system proxies and automatic redirects. It retains normal TLS hostname/certificate verification. HTTPS cannot redirect to plaintext HTTP. Five redirects and URLs of 8192 bytes are the limits.

DNS resolution has an eight-second deadline. Connections have a ten-second deadline and reads have a twenty-second deadline. A transfer also has a twenty-four-hour request deadline. Cancellation interrupts DNS/request waits and body reads.

These controls apply to this native HTTP provider. They are not a network sandbox for yt-dlp or arbitrary subprocesses.

## Inspection and supported inputs

Inspection sends `GET` with `Range: bytes=0-0`; it does not assume HEAD or Range support. A valid full `200` is accepted when the server ignores Range. A `206` must describe the requested byte. Inspection closes the response after checking headers and does not write files or start a download.

Supported response types include MP4, WebM, QuickTime, Matroska, AVI, MPEG/TS, MP3, M4A, Ogg/Opus, WAV, FLAC and AAC. `application/octet-stream` requires a recognized media filename extension. HTML, images, unknown response types and HLS/DASH/other playlist MIME types are rejected. Known manifest path extensions are rejected before a request.

Inspection infers audio/video capability from the response type. It does not invent codecs, duration, resolution or a verified platform status. Its title contains only `HTTP media` and the hostname. It never displays an opaque URL basename, signed query or credential-bearing path.

Response bytes are still untrusted. The transfer checks a bounded initial prefix before writing it, including split UTF-8 BOMs and leading whitespace. XML/HTML, M3U/PLS and concatenation signatures are rejected. Independently, local FFprobe/FFmpeg calls must restrict protocols and demuxers, excluding HLS, DASH and concatenation. Header and prefix checks alone cannot replace that restriction.

## Transfer and recovery

The caller supplies an existing, job-owned staging directory. The provider uses fixed filenames inside it and rejects symlinks/reparse points. It streams chunks without collecting the full body. The response and actual byte counter are limited to 128 GiB. Contradictory Content-Length/Content-Range values and compressed transfer encodings are rejected.

Progress contains observed bytes and elapsed transfer speed. Total and percentage remain absent when the server supplies no usable total. No ETA is invented. Events are limited to roughly four per second, with explicit initial and phase events.

Interrupted data remains in `.direct-source.part`. Resume requires a strong ETag, an unchanged original URL hash, `If-Range`, and a matching `206` range covering the remaining bytes. An ignored Range returning `200` restarts that owned partial from zero. An invalid range preserves the partial and returns an error. `416` triggers one fresh request.

ETags are scoped to a resource URI. Range and If-Range are not forwarded to a different redirect target, and redirected responses do not establish resumable state. Such transfers restart instead of appending bytes from a different resource.

`.direct-source.resume.json` contains only a URL hash and strong ETag. It contains no operational URL. After a complete transfer, a separate completion receipt records the URL hash, fixed temporary filename, byte count and SHA-256. A later processing retry verifies that receipt against the completed raw input and can reuse it offline. Modified or truncated input is rejected and preserved. A receipt written immediately before the final move also lets recovery finish that move after a crash.

The transfer flushes and hashes the completed input, then moves it without replacing an existing file. The raw input's extension initially follows the response header. It is not a codec guarantee. The queue probes the actual media, creates an appropriately named processing alias, and verifies the selected profile before publishing a final file. It retains the raw source until downstream work succeeds so retries can reuse its completion receipt. No original library file is overwritten.

Processing aliases include the stable job ID and the probed container extension, so unrelated direct jobs can share a destination without colliding. Before final publication the queue can reuse a byte-identical, reverified result with the same effective profile and destination. Publication and the completed library record share an exclusive decision interval; concurrent completions cannot both pass the reuse check before either is indexed. This is output deduplication, not a claim that all repeated requests avoid network traffic.

## Verification

Run the focused deterministic checks with:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml direct::tests -- --test-threads=1
```

The tests exercise private destinations, redirect rejection with a zero-request tripwire, Range GET inspection, secret-free preview/state, interrupted transfer and valid resume, ignored/wrong ranges, HTTP 413 and size limits, cancellation, unknown totals, no replacement, completed-input reuse and tampering, and split manifest prefixes. These transport fixtures are not valid media samples or platform certifications.

The loopback connector exists only under `cfg(test)` and accepts only its exact fixture hostname/port. Production has no private-network preference, environment override or UI bypass. The queue's real-FFmpeg fixture checks use the same native transfer API through this test-only connector, then verify actual containers and codecs. Live checks require explicitly authorized, openly licensed samples and are recorded separately from fixture results.
