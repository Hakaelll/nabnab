# nabnab

nabnab is a Windows desktop app for downloading, organizing, playing, and editing media locally. It uses Tauri 2, Rust, and TypeScript.

[Download nabnab 0.2.0 for Windows x64](https://github.com/Hakaelll/nabnab/releases/tag/v0.2.0). The unsigned installer requires WebView2. An upgrade from ytnabber keeps the existing installation folder and application data. The features described below reflect the current source on `main`. See the release notes for the contents of the published installer. [Release notes](docs/releases/0.2.0.md) · [Migration guide](docs/MIGRATION.md)

![The nabnab Home screen](docs/home-motion-after.png)

## Get started

1. Paste a link in **Home**, or import a list of links from a `.txt` file. Analysis prepares the available choices.
2. Choose what to save and select a download profile. Click **Download** or **Add to queue**.
3. Follow the work in **Downloads**. Verified outputs appear in **Library**.
4. Import files or folders to index them in place. From **Library**, open a file, play it, find it on disk, or organize it into collections.
5. Open **Tools** to convert, trim, or process a file. Tool jobs use the same queue and keep the source file.

After a restart, interrupted jobs are paused for review. Choose **Retry** to run one again. nabnab creates a separate output if a file already exists. Removing a Library entry leaves the file on disk. [Download profiles and formats](docs/PLATFORMS.md) · [Local tools](docs/TOOLS.md)

## Sources and courses

- **Research and audiobooks.** Browse versioned PDFs on arXiv, choose files from a Zenodo record or DOI, and select chapters from LibriVox editions. Open saved arXiv copies by exact version. Zenodo shows the combined size of the files you select when their sizes are known. Follow searches or version histories for manual review. [Research and audiobooks](docs/RESEARCH-SOURCES.md)
- **Media and subscriptions.** Review individual gallery and social attachments, follow podcast RSS feeds, and import or export OPML subscriptions. The BitTorrent workflow offers file selection and requires explicit P2P consent. Telegram attachment selection requires the optional TDLib component and an authorized account. [Sources and tools](docs/EXPANSION.md) · [Podcasts](docs/PODCASTS.md)
- **Personal servers.** Browse an authorized Nextcloud or WebDAV root, use SFTP with a verified host fingerprint, or select a mounted network folder in Windows. Index a reference for later or queue a local copy. [Personal servers](docs/PERSONAL-SERVERS.md)
- **Courses.** Study local lessons with notes, progress, and playback position. Udemy, Hotmart, and Kiwify use separate authorized connections. For Rocketseat, Coursera, and Domestika, save authorized materials on the provider's site and import them. Skillshare also lets you review supported direct attachment links and add selected resources to a course or lesson. [Courses and provider limits](docs/COURSES.md)

## Library and playback

- **Audio.** Browse music by artist, album, or genre, then switch to podcasts or audiobooks. Manage a separate playback queue and edit playlists. Import M3U, M3U8, PLS, or CSV and review ambiguous matches against local tracks. Open local LRC lyrics. When listening to an audiobook, resume chapters, rewind a short interval, or set a sleep timer. [Audio Library](docs/AUDIO-LIBRARY.md)
- **Mini player.** Keep audio or video available while navigating the app. On Windows, use **Ctrl + Shift + M** to open the floating mini player. **Quick Skip** lets you preview the audio queue from the desktop while nabnab is minimized. [Playback and shortcuts](docs/playback.md)
- **Personal archives.** Import an official Instagram, Facebook, X, Spotify, or LinkedIn export. Browse it by account, category, and date. Contacts and messages require separate selection and reveal. Spotify listening history reports events, listening time, and complete plays separately. [Personal archives](docs/PERSONAL-ARCHIVES.md)

The Home character field and translucent navigation are part of the desktop interface. **Settings → Appearance** offers reduced motion and reduced transparency. [Home and the compact sidebar](docs/nabnab-sidebar.png) · [Library](docs/nabnab-library.png) · [Design reference](docs/design-reference.md)

## Local tools and device sharing

- **Images.** Compare the same crop before and after upscaling, inspect a cutout and its mask at the same zoom, or export a sticker for Telegram or WhatsApp. [Upscaling](docs/IMAGE-UPSCALE.md) · [Background removal](docs/REMOVE-BACKGROUND.md) · [Stickers](docs/STICKERS.md)
- **Audio and subtitles.** Compare synchronized waveforms for the original audio, vocals, and instrumental. Translate subtitles locally, edit the draft by segment, and export a translated or bilingual track. Use installed Windows voices to create a synchronized WAV or add it to a video copy. [Stem separation](docs/STEMS.md) · [Translation and dubbing](docs/TRANSLATION.md)
- **Send to device.** Review files, a portable ZIP, a public link, or selected text before handing the request to KDE Connect. Use KDE Connect to follow the transfer and verify receipt. [KDE Connect setup and status](docs/DEVICE-SHARE.md) · [Portable collections](docs/PORTABLE-COLLECTIONS.md)

Open **Settings → Components** to check requirements, download size, installed size, and licenses before installing an optional component. Once installed, local processing runs on your computer. Under **Settings → Sessions and privacy**, you can inspect and clear the bounded thumbnail cache. [Engines and privacy](docs/NETWORK.md)

## Run from source

On Windows, use Node.js 24, the Rust MSVC toolchain, Visual Studio C++ Build Tools with a Windows SDK, and WebView2. Engine preparation uses vcpkg to build FFmpeg.

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

## Browser integration and verification

Pair the Chrome or Edge extension to send a chosen link or tab to the desktop app. You can revoke the pairing in Settings. For a provider that requires an authorized session, import a site-specific Netscape cookie file. nabnab protects stored session data with Windows DPAPI. [Browser extension](docs/BROWSER.md) · [Network and session handling](docs/NETWORK.md)

The verification guides distinguish mocked interface tests, Windows tests with local fixtures, and live provider checks. Course accounts still require authorized validation. KDE Connect tests cover the CLI handoff, and receipt on a physical phone remains to be verified. [Verification record](docs/NABNAB-VERIFICATION.md) · [Platform matrix](docs/PLATFORMS.md) · [Research provider checks](docs/RESEARCH-SOURCES.md)

The Windows application ID `com.ytnabber.desktop` preserves compatibility with earlier installations. [Architecture](docs/IMPLEMENTATION.md) · [Third-party licenses and notices](THIRD_PARTY_NOTICES.md)
