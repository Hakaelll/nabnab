# Third-party notices

nabnab launches the following separate engine executables. They are not linked into its Rust binary.

Optional OCR uses Tesseract 5.5.4 and Leptonica 1.87.0 from pinned Cygwin packages, plus their separate runtime dependencies. Package versions, SHA-512 checksums, license labels and source-package links are recorded in `src-tauri/src/ocr/packages.json`. The installer retains package license files and excludes unrelated system utilities. English and Spanish use the pinned Apache-2.0 `tessdata_fast` models documented in `docs/OCR.md`; they are downloaded only after consent. These binaries and models are not included in the default application bundle.

- **yt-dlp 2026.10.26**: yt-dlp's source is under the Unlicense. The official PyInstaller Windows executable includes GPLv3+ code and is distributed under GPLv3+ as a combined work. See licenses/yt-dlp.txt, licenses/yt-dlp-third-party.txt and licenses/GPL-3.0.txt.
- **Deno 2.9.8**: MIT, with additional third-party components documented in its upstream source. See licenses/deno.txt.
- **FFmpeg 9.0.2**: LGPL 2.1 or later, built from official source with the pinned Microsoft vcpkg recipes in scripts/ffmpeg. FFmpeg and FFprobe share replaceable DLLs; GPL and nonfree features are disabled. See licenses/ffmpeg-lgpl.txt and engines/licenses/ffmpeg.txt. Dependency notices, including LAME, Opus, OpenH264, libvpx, dav1d, zlib, libwebp and libass when enabled by the recipe, are copied into engines/licenses/.
- **Tauri, Vite, TypeScript and Rust dependencies**: license metadata is available in their published packages and the committed dependency lockfiles.

Exact corresponding upstream source and build-material locations are listed in licenses/SOURCES.md. All licenses in that directory accompany the installer. The separate engines and shared libraries can be replaced by compatible versions in the installed engines directory; nabnab does not modify or restrict their replacement.

The customized NSIS installer template derives from Tauri CLI 2.12.4 under the MIT license. Its complete notice and pinned source revision are preserved in src-tauri/installer.nsi.

The supplied emblem and application logo were provided by the project owner. No license to those assets is implied.

# Optional expansion components

gallery-dl 1.31.10 is an optional standalone component from https://github.com/mikf/gallery-dl, licensed under GPL-2.0. aria2 1.37.0 is optional, from https://github.com/aria2/aria2, licensed under GPL-2.0-or-later with its bundled third-party notices. whisper.cpp 1.7.6 and its multilingual GGML models are optional, from https://github.com/ggml-org/whisper.cpp and https://huggingface.co/ggerganov/whisper.cpp, licensed under MIT. No optional executable or model is downloaded at startup. Packages have fixed sources, sizes and SHA-256 values in the native component manager. Archive installations retain included license files.

The Rust RSS parser uses quick-xml under MIT and the local Telegram QR generator uses qrcodegen under MIT. Course API behavior was studied from OpenSelena/omniget as a reference; no source code, assets or credentials were copied.

MuPDF 1.28.5 is an optional standalone component from https://mupdf.com/releases, distributed under AGPL-3.0-or-later; its archive license files are retained. TDLib 1.8.69 is optional, provided by the ForNeVeR tdlib.native.win-x64 NuGet package, with its OpenSSL and zlib DLLs under BSL-1.0, Apache-2.0 and Zlib. It is downloaded only by an explicit component installation. See licenses/tdlib-native.txt for notices and source/build materials, and docs/TELEGRAM.md for runtime requirements. The package installs native DLLs without a .NET runtime.

