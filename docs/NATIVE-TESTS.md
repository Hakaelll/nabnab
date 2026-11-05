# Native offline verification

Executed on Windows x64 on 5 November 2026 against the workspace engine bundle. Each command below exited with code 0 and reported one passed test, zero failures. These are real subprocess and output checks, with no mocked FFmpeg, ffprobe, yt-dlp or Deno.

The tests use generated local media and explicit loopback fixtures. No Internet platform download, user media file or actual session cookie was used. They ran with normal Windows process permissions. The restricted execution token blocks Windows atomic replacement and deliberately private session ACLs; those protections were not weakened to accommodate it.

## Engine identity

Version commands returned yt-dlp `2026.10.26`, Deno `2.9.8`, FFmpeg `9.0.2` and ffprobe `9.0.2`.

| Executable | Observed SHA-256 |
| --- | --- |
| `yt-dlp.exe` | `f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d` |
| `deno.exe` | `924397800533d3eb0ed0c889b20ad70e18b970d93acd8135bc043f425e528266` |
| `ffmpeg.exe` | `4ec10e7b5505d724e64493df9dcf91913275c304b8479de56e13e6f8cb07234a` |
| `ffprobe.exe` | `e50c77c6397f81d5da84b7060b58de5269eab0075bfb0948cc095bab98ee01b4` |

The Deno executable hash identifies the extracted binary. Its official ZIP has a different pinned hash in `scripts/engine-manifest.json`. FFmpeg build details and shared-library hashes are recorded in the prepared bundle's `ffmpeg-build.json` and `checksums.json`; these tests do not change its build or licensing.

## Observed results

| Test filter | Observed behavior |
| --- | --- |
| `tools::tests::real_engines_local_tools_preserve_originals_and_verify_outputs` | Seven real operations passed: stream-copy MKV, VP9/Opus WebM conversion, MP3 extraction, original AAC extraction, precise trim, keyframe trim and target-size compression. Outputs stayed in their job staging folders, verification events occurred, and the Unicode-named original remained byte-identical. Repeating an operation could not overwrite its existing result. |
| `media::tests::real_engines_probe_rejects_manifests_disguised_as_local_media` | HLS, DASH and concat text fixtures named `.mp4` were all rejected promptly. The loopback request tripwire received no connection. This verifies the local probe's protocol/container boundary, not remote yt-dlp manifest isolation. |
| `smoke::real_engines_offline_codec_conversion` | Real MP4-compatible, original-video, MP3, M4A and Opus results passed codec/container validation and complete decoding. A valid compatible result was reused, and MP3 cover art survived without another transcode. Reported fixture output bytes were MP4 248431, source 78570, MP3 49580, M4A 60386 and Opus 19600. These are fixture sizes, not compression promises. |
| `smoke::real_engines_720p_and_1080p_compatible_profiles` | Both exact 720p and 1080p selections produced H.264/AAC and did not add another transcode when the source was already compatible. |
| `smoke::real_engines_missing_dependencies_and_processing_failures` | A missing engine classified as a dependency error. Invalid audio failed verification while its input remained present. A real FFmpeg input failure classified as a processing error. |
| `smoke::real_engines_stop_on_http_429` | A local HTTP 429 response with `Retry-After: 120` classified as rate-limited. The fixture observed exactly one request, with no immediate engine retries. |

## Reproduce

Three additional real-engine groups passed during the direct-media and subtitle work:

| Test filter | Observed behavior |
| --- | --- |
| `tools::tests::real_engines_download_audio_profiles_preserve_input_and_recover_partial_results` | Direct-download MP3 bitrate choices, AAC/M4A, Opus and original audio produced the expected codecs and containers. A partial audio result was retained safely before retry; the source bytes remained unchanged. |
| `queue::direct_tests::real_engines_direct_http_profiles_publish_real_containers_without_losing_cached_input` | A synthetic QuickTime file served with a misleading audio MIME type produced six valid profile outputs in a shared destination. Processing aliases followed the probed container, raw completed input stayed intact, and a valid local receipt avoided another HTTP request. |
| `media::tests::real_engines_subtitles_survive_copy_and_compatible_video_conversion` | Synthetic Spanish captions survived compatible stream copying and VP9/Opus/SubRip conversion to H.264/AAC/MP4 timed text. A valid-looking cache containing English instead of Spanish was regenerated; extracted Spanish text and original input bytes matched literal expected values. |

These three groups also use local synthetic media. Their filters can be substituted in the commands below. The separate, explicitly networked YouTube and HTTP open-license checks are recorded in [PLATFORMS.md](PLATFORMS.md); they are not included in this offline report.

Prepare and verify the pinned Windows x64 engine bundle first. The default path is `src-tauri/engines`. Run from the repository root:

```powershell
cargo test --offline --manifest-path src-tauri/Cargo.toml tools::tests::real_engines_local_tools_preserve_originals_and_verify_outputs -- --ignored --nocapture --test-threads=1
cargo test --offline --manifest-path src-tauri/Cargo.toml media::tests::real_engines_probe_rejects_manifests_disguised_as_local_media -- --ignored --nocapture --test-threads=1
cargo test --offline --manifest-path src-tauri/Cargo.toml smoke::real_engines_offline_codec_conversion -- --ignored --nocapture --test-threads=1
cargo test --offline --manifest-path src-tauri/Cargo.toml smoke::real_engines_720p_and_1080p_compatible_profiles -- --ignored --nocapture --test-threads=1
cargo test --offline --manifest-path src-tauri/Cargo.toml smoke::real_engines_missing_dependencies_and_processing_failures -- --ignored --nocapture --test-threads=1
cargo test --offline --manifest-path src-tauri/Cargo.toml smoke::real_engines_stop_on_http_429 -- --ignored --nocapture --test-threads=1
```

`--offline` prevents Cargo from accessing its registry. These selected tests themselves do not need Internet access; other ignored tests have different network and content requirements and were not included in this run.

These checks do not certify a platform, an installed clean-machine workflow, Windows startup registration, tray interactions, browser delivery or installer upgrade/uninstall. Those require their own recorded checks. This report also makes no CPU, memory, start-time or throughput claim.
