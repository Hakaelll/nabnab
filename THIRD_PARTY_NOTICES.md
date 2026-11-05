# Third-party notices

nabnab launches the following separate engine executables. They are not linked into its Rust binary.

- **yt-dlp 2026.10.26**: yt-dlp's source is under the Unlicense. The official PyInstaller Windows executable includes GPLv3+ code and is distributed under GPLv3+ as a combined work. See licenses/yt-dlp.txt, licenses/yt-dlp-third-party.txt and licenses/GPL-3.0.txt.
- **Deno 2.9.8**: MIT, with additional third-party components documented in its upstream source. See licenses/deno.txt.
- **FFmpeg 9.0.2**: LGPL 2.1 or later, built from official source with the pinned Microsoft vcpkg recipes in scripts/ffmpeg. FFmpeg and FFprobe share replaceable DLLs; GPL and nonfree features are disabled. See licenses/ffmpeg-lgpl.txt and engines/licenses/ffmpeg.txt. The bundled LAME, Opus, OpenH264, libvpx, dav1d and zlib license texts are also in engines/licenses/.
- **Tauri, Vite, TypeScript and Rust dependencies**: license metadata is available in their published packages and the committed dependency lockfiles.

Exact corresponding upstream source and build-material locations are listed in licenses/SOURCES.md. All licenses in that directory accompany the installer. The separate engines and shared libraries can be replaced by compatible versions in the installed engines directory; nabnab does not modify or restrict their replacement.

The customized NSIS installer template derives from Tauri CLI 2.12.4 under the MIT license. Its complete notice and pinned source revision are preserved in src-tauri/installer.nsi.

The supplied emblem and application logo were provided by the project owner. No license to those assets is implied.
