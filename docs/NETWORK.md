# Network, engines and session policy

nabnab processes media on the user's computer. Downloading remote media and preparing missing engines require a network connection. The local library and installed FFmpeg tools do not require a service connection.

No telemetry, analytics, remote fonts, cloud AI, hosted extraction service, third-party download proxy or conversion service is added. The application does not monitor the clipboard, intercept browsing or import browser cookies automatically.

## Destinations

| Trigger | Destination | Purpose |
| --- | --- | --- |
| Explicit link analysis or queued download | The requested public platform and the resource/CDN hosts its extractor needs | Obtain metadata and authorized media through the local yt-dlp engine |
| Thumbnail preview | The HTTPS thumbnail URL supplied by the extractor | Display a preview with no referrer |
| Explicit yt-dlp update check/install | `api.github.com/repos/yt-dlp/yt-dlp/releases/latest`, official yt-dlp release assets at `github.com/yt-dlp/yt-dlp` | Existing stable engine update mechanism, separate from updating nabnab |
| Explicit engine repair | Pinned yt-dlp and Deno release assets at `github.com/yt-dlp/yt-dlp` and `github.com/denoland/deno` | Restore managed Windows x64 engine components |
| Official release redirects | `release-assets.githubusercontent.com` and `objects.githubusercontent.com` | GitHub's release asset delivery |
| Installer WebView preparation, when needed | Microsoft's configured WebView2 bootstrapper | Supply the existing Tauri webview runtime |

Engine repair has no startup download. The caller must obtain the user's consent before invoking it and prevent repair, rollback or updates while any job uses those engines. Local status checks only execute installed tools with version arguments. They do not contact a release service.

The URL boundary parses schemes, credentials, hosts and IP addresses. Initial DNS resolution rejects private and reserved destinations. yt-dlp performs its own later DNS, redirect and manifest-resource requests. The initial check is not an isolation boundary for every CDN or HLS/DASH resource. Do not describe it as a complete SSRF sandbox or claim arbitrary manifests are proven safe. Platform verification is recorded separately from these validation checks.

## Engine provenance and repair

The application continues to use the repository's pinned `scripts/engine-manifest.json`. Runtime repair accepts exactly the official Windows x64 yt-dlp and Deno URLs encoded by that manifest, with committed SHA-256 values. Content metadata cannot select a binary URL, command or engine component.

GitHub download connections use HTTPS, fixed permitted redirect hosts, at most five redirects, a timeout and a 200 MiB asset limit. Repair uses a direct connection without an environment-supplied proxy. Downloaded bytes must match their committed hash before execution or extraction. This trust begins with the application's packaged manifest and its acquisition/review, not with a hash fetched alongside an arbitrary binary.

FFmpeg and ffprobe remain the compiled build already distributed with nabnab. Repair verifies their packaged `checksums.json`, each required binary and shared resource before copying them to a fresh managed directory. The checksums are an integrity manifest belonging to the installed application. They are not an independent publisher signature. If the bundled FFmpeg files or verification data are missing or damaged, online repair stops with an instruction to repair the installer. It does not silently install a different FFmpeg distribution or require build tools on the user's machine.

Deno extraction uses Windows' `System32/tar.exe`, resolved by the Windows API rather than PATH. Before extraction, the complete archive entry list must contain one `deno.exe` entry. Absolute paths, traversal, duplicate entries and extra entries are rejected. The extracted executable must be a regular authorized file. Version smoke tests run before activation.

Managed sets live in `app_local_data_dir/managed-engines/set-<pid>-<nonce>`. Only a complete staged set is atomically activated. Its previous managed set, or the packaged set on the first repair, remains available for rollback. A failed repair leaves the active pointer unchanged and removes only its owned staging directory. A crash can leave an inactive set; it is never selected merely because its directory exists.

Production resolution uses the active managed set or packaged resources. Debug builds additionally recognize the repository's explicit `src-tauri/engines` directory. Installed builds do not depend on a developer's PATH or a hardcoded copy of an older application.

Status reports the real executable path, version response, executable bytes, source and `installed`, `missing` or `broken` state. A version check is not a complete media workflow test. Shared DLL, cache, application and installer sizes need a separate inventory; do not add executable byte counts and describe them as the whole installed size.

The FFmpeg build's licenses and provenance remain in `THIRD_PARTY_NOTICES.md`, `licenses/SOURCES.md`, and the packaged engine notices. There is no new app update endpoint, signing certificate or publisher identity.

## Optional site sessions

Session import is an explicit action for one site. The native dialog chooses a local UTF-8 Netscape HTTP cookie file of at most 2 MiB. The original export remains under the user's control. nabnab does not delete it, log its contents or write cookies to the job repository.

Every cookie domain must belong to the selected site's allowlist. A mixed export is rejected rather than importing unrelated sessions.

| Site ID | Permitted cookie domains, including subdomains |
| --- | --- |
| `youtube` | `youtube.com`, `youtube-nocookie.com`, `google.com` for YouTube's Google account session |
| `twitter` | `x.com`, `twitter.com` |
| `instagram` | `instagram.com` |
| `reddit` | `reddit.com`, `redd.it` |
| `tiktok` | `tiktok.com` |
| `vimeo` | `vimeo.com` |
| `twitch` | `twitch.tv` |
| `pinterest` | `pinterest.com`, `pin.it` |

On Windows, DPAPI protects each site's cookie bytes for the current Windows user. Site-specific entropy binds an encrypted file to its site ID. Encrypted session files live under `app_config_dir/sessions`, and the directory DACL grants access only to the current user with inheritance protected. There is no plaintext JSON fallback on other platforms.

For an engine operation, the app identifies the requested URL's provider using parsed hosts. It decrypts only that provider's imported session into a newly created file in `app_config_dir/session-temp`, with a private current-user DACL. The caller retains the `CookieFile` guard until the subprocess tree exits; dropping the guard removes that temporary file on success, error or cancellation. Decrypted in-memory buffers are cleared with volatile writes.

At startup, after acquiring the repository lock and before launching jobs, `sessions::cleanup` removes files matching the owned `cookie-*.txt` temporary format from that directory. It does not clean user exports, session records, downloaded media or arbitrary files. Call it only during that recovery interval, not while jobs are running.

Revocation removes the protected session for future operations. An engine that already loaded cookies may continue using them until that operation exits; cancel that operation to stop it. Cookies cannot bypass access restrictions or DRM, and authentication challenges can still fail.

Diagnostics must use the existing sanitizer and must never include cookies, tokens, authorization headers or signed URL parameters. No actual user cookies are required for the automated session tests; they use fictional local fixtures.
