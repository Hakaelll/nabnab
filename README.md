# nabnab

nabnab is a Windows desktop app for downloading media and documents, organizing files where they already live, and playing or editing them locally. Downloads and local processing share a queue. Finished files appear in your library.

[Download the Windows x64 installer](https://github.com/Hakaelll/nabnab/releases/tag/v0.2.0) · [Run from source](#run-from-source) · [Browse supported platforms](docs/PLATFORMS.md)

![The nabnab Home screen](docs/home-motion-after.png)

## Get started

The unsigned 0.2.0 installer requires WebView2. Upgrading from ytnabber keeps your installation folder and app data. This README describes the current `main` branch. Check the release notes for the features bundled in the installer. [Release notes](docs/releases/0.2.0.md) · [Migration guide](docs/MIGRATION.md)

1. Paste a link in **Home** or import a list of links from a `.txt` file. Choose a download profile after nabnab analyzes the link.
2. Click **Download** or **Add to queue**. Follow the job in **Downloads**. Verified outputs appear in **Library**.
3. To add files you already have, open **Library** and import files or folders in place. Double-click a media file to open it.
4. Open **Tools** to convert, trim, or process a file. The queue writes a new output and keeps the source file.

After a restart, interrupted jobs are paused for review. Choose **Retry** to run a job again or **Resume** when a supported server copy has saved bytes. nabnab creates a separate output if a file already exists. Removing a Library entry leaves the file on disk. [Download profiles and formats](docs/PLATFORMS.md) · [Local tools](docs/TOOLS.md)

### Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| **Ctrl + K** | Open global search. |
| **↑** / **↓** | Adjust volume by five percentage points while the player has focus. |
| **Ctrl + Shift + M** | Open or focus the floating player with audio or video loaded, even while nabnab is minimized. |
| **Alt + mouse wheel** | Browse the audio queue in Quick Skip, even while nabnab is minimized. Release **Alt** to play the selected track or press **Esc** to cancel. |

The two desktop shortcuts are listed in **Settings → General → Desktop playback**. [Playback controls](docs/playback.md) · [Audio mini player](docs/playback/mini-audio.png) · [Video mini player](docs/playback/mini-video.png) · [Quick Skip](docs/playback/quick-skip-desktop.png)

## Sources and courses

- **Research.** Compare arXiv PDF versions and open each version's abstract, authors, and reported license. In Zenodo, use **Local copy** to find the files you still need. Choose **Verify copy** to check a saved file against its published checksum. Follow public Zenodo communities in **Library → Sources**. [Research sources](docs/RESEARCH-SOURCES.md) · [arXiv version details](docs/arxiv-version-details.png) · [Zenodo copy verification](docs/zenodo-copy-check.png)
- **Audiobooks.** Browse LibriVox editions, combine language and reader filters with local availability, and select the missing chapters in the results. [LibriVox workflow](docs/RESEARCH-SOURCES.md) · [Missing chapters](docs/librivox-missing-chapters.png)
- **Media and subscriptions.** Review individual gallery and social attachments, follow podcast RSS feeds, and import or export OPML subscriptions. The BitTorrent workflow offers file selection and requires explicit P2P consent. Telegram attachment selection requires the optional TDLib component and an authorized account. [Sources and tools](docs/EXPANSION.md) · [Podcasts](docs/PODCASTS.md)
- **Personal servers.** Browse Nextcloud, WebDAV, SFTP, or a network folder that Windows can access. Search saved references by file, folder, or server and find local copies of the exact saved version. In **Downloads**, pause supported WebDAV and mounted-folder copies, then resume or discard the saved bytes, including after a restart. Use **Review version** to compare a changed file with the version requested. [Personal servers](docs/PERSONAL-SERVERS.md) · [Recovered WebDAV copy](docs/personal-server-recovered.png)

### Study offline

Open **Platforms → Courses** to organize lessons and keep notes, progress, and playback position together. Create an outline for Rocketseat, Domestika, or Skillshare. In a lesson, use **Edit titles** to rename the lesson or its unit while keeping its study history, or choose **Link audio or video** to attach a saved recording. Domestika resources can sit alongside the lessons in their unit. Skillshare can also review supported direct attachment links. [Edit titles](docs/course-outline-titles.png) · [Link a recording](docs/course-lesson-recording.png)

Search the course by unit, lesson, or attachment name, including units you have collapsed. When you open a different lesson, the course can offer to resume your most recent unfinished audio or video lesson from its saved position, even after a restart. If a file you moved is still indexed in Library, use **Locate file** from the lesson to recover it and keep your notes and saved position. [Course search](docs/course-search.png) · [Lesson resume](docs/course-resume.png) · [Moved material](docs/course-moved-material.png)

Udemy, Hotmart, and Kiwify use separate authorized connections. With a Coursera Business organization token, you can import modules, lessons, and material names. nabnab follows supported continuation links and shows **Partial outline** if it cannot reach the end. [Courses and provider limits](docs/COURSES.md) · [Coursera outline status](docs/course-coursera-pagination.png)

## Library and playback

- **Music.** Browse by artist, album, or genre. Play an album or playlist, or queue it next while the current song keeps playing. Import M3U, M3U8, PLS, or CSV playlists and match their entries to local tracks. If a recording has a different name, search Library from that entry and compare its duration and file path before choosing it. Open local LRC lyrics during playback. [Audio Library](docs/AUDIO-LIBRARY.md) · [Playlist controls](docs/audio-playlist-playback.png)
- **Audiobook playback.** Open a LibriVox book or local M4B from the Bookshelf. The LibriVox edition shows saved chapters against the catalog total when known, with search by reader and language. In the floating player, change speed, rewind 15 seconds, or set a sleep timer while nabnab is minimized. Playback stops at a missing chapter. [Audiobook guide](docs/AUDIO-LIBRARY.md) · [LibriVox edition in the Bookshelf](docs/audio-librivox-edition.png) · [Floating audiobook player](docs/playback/mini-audiobook.png)
- **Playback.** Switch between the compact bar, full player, and mini player with the same queue and position. Video chapter markers show titles alongside seek previews. The music view includes thirteen scenes that respond to the sound, including Phosphor and Cassette. Mark favorites, enable automatic rotation, or use a still image. [Playback guide](docs/playback.md) · [Video chapters](docs/playback/video-chapters.png) · [Phosphor scene](docs/playback/phosphor-full.png)
- **Personal archives.** Browse official Instagram, Facebook, X, Spotify, and LinkedIn exports by account, category, and date. Before importing a date range, review record counts and attachment sizes. In an imported archive, **Export contents** shows which public categories the source package contained, including empty, missing, or unsupported categories. Contacts and messages require explicit selection. [Personal archives](docs/PERSONAL-ARCHIVES.md) · [Export contents](docs/personal-archive-coverage.png)

The animated character field on Home cycles through eleven patterns. The translucent sidebar expands to show labels on hover or keyboard focus, while the workspace stays in place. In **Settings → Appearance**, you can reduce motion or transparency. [Home and the compact sidebar](docs/nabnab-sidebar.png) · [Design reference](docs/design-reference.md)

## Local tools and device sharing

- **Images and stickers.** Review source and output dimensions for a batch before loading the optional upscaling model. The review identifies oversized files and estimates peak pixel memory per image. In Remove Background, inspect the cutout and its mask at the same zoom. Open a finished transparent cutout directly in Stickers and preview it for Telegram or WhatsApp. [Upscaling](docs/IMAGE-UPSCALE.md) · [Batch dimension review](docs/upscale-batch-dimensions.png) · [Background removal](docs/REMOVE-BACKGROUND.md) · [Sticker export](docs/STICKERS.md)
- **Audio stems.** Choose up to eight seconds to preview by dragging on the timeline or moving its handle with the arrow keys. Compare the original audio, vocals, and instrumental at the same position on synchronized waveforms. After comparing a full export, return to the same track and position in your playback queue. [Stem separation](docs/STEMS.md) · [Sample timeline](docs/stems-sample-window.png)
- **Subtitles.** Translate subtitles locally. Find a draft segment by text, cue ID, or timestamp and edit it in place. Export the complete translated or bilingual track while a search is active. [Subtitle translation](docs/TRANSLATION.md) · [Draft search](docs/translation-cue-search.png)
- **Dubbing.** Preview a translated cue with an installed Windows voice. Hear the same timing adjustment as in the synchronized WAV and edit cues that exceed the available time. Create an MKV with an alternate voice track, replacement audio, or a mix. If the video copy needs a different input, reuse the WAV for the next attempt. [Dubbing guide](docs/TRANSLATION.md#doblaje-local) · [Cue timing preview](docs/dubbing-sample-fit.png) · [Video copy recovery](docs/dubbing-copy-recovery.png)
- **Documents and captures.** Search scanned pages after local OCR, or read a saved PDF or EPUB and add bookmarks. Capture a monitor, window, or region, with separate controls for microphone and system audio. [Local documents](docs/DOCUMENTS.md) · [Screen capture](docs/SCREEN-CAPTURE.md)
- **Send to device.** Review files, a portable ZIP, a public link, or a selected passage from an archived article, PDF, or EPUB before handing the request to KDE Connect. nabnab reports whether the KDE Connect command accepted the request and keeps the details visible if the handoff is uncertain. Follow the transfer and confirm receipt in KDE Connect. [KDE Connect setup and status](docs/DEVICE-SHARE.md) · [Portable collections](docs/PORTABLE-COLLECTIONS.md)

### Storage and optional components

Open **Settings → Sessions and privacy → Temporary storage** to see how much space thumbnails and reusable narration or dubbing segments occupy. Clearing those caches frees space while keeping your Library entries and exported files. nabnab regenerates cached material as needed. [Local voice cache](docs/LOCAL-VOICE.md)

Open **Settings → Components** to review download size, disk space, and hardware requirements. Expand **Source and license** for the version, origin, and license. Disable a component to stop using it, or remove it to reclaim its space. nabnab also clears obsolete managed engine sets on startup and keeps the previous set available for rollback. For development builds, see [Run from source](#run-from-source) for the Cargo cleanup command. [Optional components in Settings](docs/settings-optional-components.png) · [Engines and privacy](docs/NETWORK.md)

## Browser extension and local automation

Pair the Chrome or Edge extension to send a link or save an article for offline reading. Review the captured content before saving it as HTML, EPUB, or Markdown. For a provider that requires an authorized session, you can import a site-specific Netscape cookie file. Windows DPAPI protects stored session data. [Browser extension](docs/BROWSER.md) · [Offline articles](docs/WEB-ARCHIVE.md) · [Network and session handling](docs/NETWORK.md)

The local CLI and MCP interface can submit requests to the same queue. Enable automation in Settings, authorize a client, and review each plan in nabnab before it can run. [Local automation](docs/LOCAL-AUTOMATION.md)

## Run from source

nabnab uses Tauri 2, Rust, and TypeScript. On Windows, install Node.js 24, the Rust MSVC toolchain, Visual Studio C++ Build Tools with a Windows SDK, and WebView2. Engine preparation uses vcpkg to build FFmpeg.

```powershell
npm ci
npm run engines:prepare
npm run desktop:dev
```

Engine preparation reuses bundles that match the manifest and integrity checks. Use `npm run engines:prepare -- --rebuild` to rebuild the bundle. Preparation clears its temporary downloads and build workspace after the run. [Engine preparation](docs/NETWORK.md)

```powershell
npm run check
npm test
npm run test:native
npm run desktop:build
```

The installer is written to `src-tauri/target/release/bundle/nsis/`. For frontend development, run `npm run dev` to serve the interface in a browser. Launch the desktop app for native operations. [Native test setup](docs/NATIVE-TESTS.md)

Development builds keep file and line numbers in Rust backtraces. If you need to inspect Rust types and variables in a debugger, run `$env:CARGO_PROFILE_DEV_DEBUG = '2'` in PowerShell before building. This creates larger debug files. [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#debug)

To see how much space you can reclaim, run the dry run first. Close the development app before cleaning. The next development build recreates `src-tauri/target/debug/`. Manage app caches separately in **Settings → Sessions and privacy**.

```powershell
cargo clean --dry-run --profile dev --manifest-path src-tauri/Cargo.toml
cargo clean --profile dev --manifest-path src-tauri/Cargo.toml
```

## Verification and compatibility

The verification guides distinguish mocked interface tests, Windows tests with local fixtures, and live provider checks. Course accounts still require authorized validation. KDE Connect tests cover the CLI handoff, and receipt on a physical phone remains to be verified. [Verification record](docs/NABNAB-VERIFICATION.md) · [Platform matrix](docs/PLATFORMS.md) · [Research provider checks](docs/RESEARCH-SOURCES.md)

The Windows application ID `com.ytnabber.desktop` preserves compatibility with earlier installations. [Architecture](docs/IMPLEMENTATION.md) · [Third-party licenses and notices](THIRD_PARTY_NOTICES.md)
