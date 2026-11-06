# Corresponding source and build materials

yt-dlp and Deno downloads are pinned, unmodified upstream release assets. Their exact URLs and SHA-256 values are recorded in scripts/engine-manifest.json and the installed engines/versions.json. FFmpeg is built from verified upstream sources as described below.

## yt-dlp 2026.10.26

- [Release and source tarball](https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.26)
- [Exact tagged source](https://github.com/yt-dlp/yt-dlp/tree/2026.10.26)
- [Windows executable build materials](https://github.com/yt-dlp/yt-dlp/tree/2026.10.26/bundle)
- [Build workflow and dependency versions](https://github.com/yt-dlp/yt-dlp/tree/2026.10.26/.github/workflows)
- [Dependency lockfile](https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/uv.lock)
- [Bundled third-party notices](https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/THIRD_PARTY_LICENSES.txt)

The upstream Windows binary is a GPLv3+ combined work; the source repository alone is Unlicense. Its notices identify the included components and their licenses.

## FFmpeg 9.0.2, compact LGPL shared build

- [Exact FFmpeg source](https://github.com/FFmpeg/FFmpeg/tree/n9.0.2)
- [Pinned vcpkg registry, source hashes and patches](https://github.com/microsoft/vcpkg/tree/d4c0a46cb8f63e5c9fa007d13baf1cb3e43bf7e3/ports)
- [FFmpeg port and applied patches](https://github.com/microsoft/vcpkg/tree/d4c0a46cb8f63e5c9fa007d13baf1cb3e43bf7e3/ports/ffmpeg)
- App build recipe: scripts/ffmpeg/vcpkg.json, scripts/ffmpeg/triplets/x64-ytnabber.cmake and scripts/build-ffmpeg.mjs in this release's source.

The LGPL 2.1+ shared build uses replaceable DLLs and the static Microsoft CRT. GPL/nonfree features, FFplay and external GPU/filter packages are not enabled; none are called by YTNabber's download or conversion profiles. Core decoders, containers, native device inputs including lavfi, HTTPS via Windows Schannel, MP3/AAC/Opus encoding and OpenH264 remain enabled. Run engines/ffmpeg.exe -buildconf to inspect the exact configuration. The installed engines/ffmpeg-build.json records the recipe, configuration and binary hashes; engines/checksums.json records every distributed engine resource.

The pinned registry also contains the exact source URLs, cryptographic hashes and build patches for LAME, Opus, OpenH264, libvpx, dav1d, zlib, libwebp and libass (including their transitive dependencies when enabled). Their target-package copyright files are distributed in engines/licenses/. All source/build materials above are available without charge. Build tools and source caches are not part of the installed application.

## Deno 2.9.8

- [Exact source and third-party material](https://github.com/denoland/deno/tree/v2.9.8)
- [Deno license](https://github.com/denoland/deno/blob/v2.9.8/LICENSE.md)
- [Releases](https://github.com/denoland/deno/releases/tag/v2.9.8)

## Application dependencies

package-lock.json and src-tauri/Cargo.lock record the exact JavaScript and Rust dependencies. Their source packages include individual license metadata. The repository's versioned source and Git history identify the build materials; publishing a GitHub release/tag is a separate distribution step.
