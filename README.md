# nabnab

nabnab is a Windows desktop app for saving media and documents, organizing a local library, and playing or editing your files. Downloads and local processing share a queue, so you can leave a job running while you browse your library.

The Windows x64 installer for version 0.2.0 is unsigned and requires WebView2. An upgrade from ytnabber keeps the existing installation folder and application data. The features below describe the source on `main`. [Download 0.2.0](https://github.com/Hakaelll/nabnab/releases/tag/v0.2.0) · [Release notes](docs/releases/0.2.0.md) · [Migration guide](docs/MIGRATION.md)

![The nabnab Home screen](docs/home-motion-after.png)

## Get started

1. Paste a link in **Home** or import a list of links from a `.txt` file. Choose a download profile after nabnab analyzes the link.
2. Click **Download** or **Add to queue**. Follow the job in **Downloads**. Verified outputs appear in **Library**.
3. To add files you already have, open **Library** and import files or folders in place. Double-click a media file to open it.
4. Open **Tools** to convert, trim, or process a file. The queue writes a new output and keeps the source file.

After a restart, interrupted jobs are paused for review. Choose **Retry** to run one again. nabnab creates a separate output if a file already exists. Removing a Library entry leaves the file on disk. [Download profiles and formats](docs/PLATFORMS.md) · [Local tools](docs/TOOLS.md)

## Sources and courses

- **Research.** Open arXiv PDFs by exact version, or review the files and sizes in a Zenodo record. Paste a public Zenodo community link to browse recent records and follow the community in **Library → Sources** for manual review. [Research sources](docs/RESEARCH-SOURCES.md)
- **Audiobooks.** Browse LibriVox editions and review the chapters you want before downloading. [LibriVox workflow](docs/RESEARCH-SOURCES.md) · [Chapter selection](docs/librivox-chapter-review.png)
- **Media and subscriptions.** Review individual gallery and social attachments, follow podcast RSS feeds, and import or export OPML subscriptions. The BitTorrent workflow offers file selection and requires explicit P2P consent. Telegram attachment selection requires the optional TDLib component and an authorized account. [Sources and tools](docs/EXPANSION.md) · [Podcasts](docs/PODCASTS.md)
- **Personal servers.** Browse Nextcloud, WebDAV, SFTP, or a network folder that Windows can access. Index a reference or queue a local copy. If a file changes on the server, compare the saved version with the current one before updating the reference or copying again. [Personal servers](docs/PERSONAL-SERVERS.md)
- **Courses.** Study local lessons with notes, progress, and playback position. Udemy, Hotmart, and Kiwify use separate authorized connections. With a Coursera Business organization token, you can import an outline and attach files saved from the site. Rocketseat has separate entries for classroom and learning path links, with access to the original page in your browser. For Rocketseat and Domestika, import files you saved through their websites. Skillshare can also review supported direct attachment links. [Courses and provider limits](docs/COURSES.md)

## Library and playback

- **Audio.** Browse music by artist, album, or genre, then switch to podcasts or audiobooks. Import M3U, M3U8, PLS, or CSV playlists and match their entries against local tracks. Revisit unresolved entries after adding more music. Open local LRC lyrics, or use chapter resume and a sleep timer for audiobooks. [Audio Library](docs/AUDIO-LIBRARY.md)
- **Playback.** Switch between the compact bar, full player, and mini player with the same queue and position. The music view has scenes that respond to the sound, including Phosphor and Cassette. Use **↑** and **↓** to adjust volume while the player has focus. [Playback guide](docs/playback.md) · [Phosphor scene](docs/playback/phosphor-full.png)
- **Desktop shortcuts.** With a file loaded, press **Ctrl + Shift + M** to open the floating audio or video player. For audio, hold **Alt** and scroll to browse the queue in **Quick Skip**. Release **Alt** to play the selected track, or press **Esc** to cancel. Both shortcuts work while the main window is minimized. [Audio mini player](docs/playback/mini-audio.png) · [Video mini player](docs/playback/mini-video.png) · [Quick Skip](docs/playback/quick-skip-desktop.png)
- **Personal archives.** Browse official Instagram, Facebook, X, Spotify, and LinkedIn exports by account, category, and date. Before you import a date range, review public record counts and the size of linked attachments. Contacts and messages require explicit selection. Spotify listening history reports events, listening time, and complete plays separately. [Personal archives](docs/PERSONAL-ARCHIVES.md) · [Date range review](docs/personal-archive-date-review.png)

The animated character field on Home cycles through eleven patterns. The sidebar uses the same translucent material across sections. In **Settings → Appearance**, you can reduce motion or transparency. [Home and the compact sidebar](docs/nabnab-sidebar.png) · [Library](docs/nabnab-library.png) · [Design reference](docs/design-reference.md)

## Local tools and device sharing

- **Images and stickers.** Compare the same crop before and after upscaling, or inspect a cutout and its mask at the same zoom. Play an animated WhatsApp sticker preview, return to its first frame, and check its edges on light, dark, or checkerboard backgrounds. [Upscaling](docs/IMAGE-UPSCALE.md) · [Background removal](docs/REMOVE-BACKGROUND.md) · [Sticker export](docs/STICKERS.md)
- **Audio and subtitles.** Compare synchronized waveforms for the original audio, vocals, and instrumental. Translate subtitles locally, edit the draft by segment, and export a translated or bilingual track. [Stem separation](docs/STEMS.md) · [Subtitle translation](docs/TRANSLATION.md)
- **Dubbing.** Use installed Windows voices to create a synchronized WAV. Export an MKV with an alternate voice track, replace the audio, or mix the voice with a selected original track or a previously exported instrumental from the same video. [Dubbing guide](docs/TRANSLATION.md#doblaje-local) · [Instrumental selector](docs/dubbing-instrumental.png)
- **Documents and captures.** Search scanned pages after local OCR, or read a saved PDF or EPUB and add bookmarks. Capture a monitor, window, or region, with separate controls for microphone and system audio. [Local documents](docs/DOCUMENTS.md) · [Screen capture](docs/SCREEN-CAPTURE.md)
- **Send to device.** Review files, a portable ZIP, a public link, or selected text before handing the request to KDE Connect. If a file is missing from disk, remove it from the share while keeping its Library entry. The text field shows how much of the 8 KiB limit you have used. Use KDE Connect to follow the transfer and verify receipt. [KDE Connect setup and status](docs/DEVICE-SHARE.md) · [Portable collections](docs/PORTABLE-COLLECTIONS.md)

Open **Settings → Components** to check download size, installed size, requirements, and licenses before installing an optional component. Local processing runs on your computer. To inspect or clear the thumbnail cache, open **Settings → Sessions and privacy**. [Engines and privacy](docs/NETWORK.md)

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

## Verification and compatibility

The verification guides distinguish mocked interface tests, Windows tests with local fixtures, and live provider checks. Course accounts still require authorized validation. KDE Connect tests cover the CLI handoff, and receipt on a physical phone remains to be verified. [Verification record](docs/NABNAB-VERIFICATION.md) · [Platform matrix](docs/PLATFORMS.md) · [Research provider checks](docs/RESEARCH-SOURCES.md)

The Windows application ID `com.ytnabber.desktop` preserves compatibility with earlier installations. [Architecture](docs/IMPLEMENTATION.md) · [Third-party licenses and notices](THIRD_PARTY_NOTICES.md)
