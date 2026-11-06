# Local media tools

The four tools run through the desktop job queue. They accept an authorized local file or library item. They do not download input files, alter originals or invoke a shell. The application supplies a separate, owned staging directory inside the selected destination.

## Operations

| Operation | Output | Behavior |
| --- | --- | --- |
| Convert | MP4, MKV, WebM | Container and video/audio codecs are separate options. Copy preserves the first real video and audio tracks. MP4 accepts H.264 and AAC as the compatible preset. MKV can preserve source tracks. WebM accepts VP8/VP9/AV1 with Opus/Vorbis; encoding uses VP9 and Opus. |
| Extract audio | Original, MP3, M4A/AAC | Original copies the actual source codec into a suitable container. Opus remains `.opus`; MP3 conversion invokes the MP3 encoder. Matching MP3/AAC tracks are copied. |
| Trim | MP4/M4A for precise cuts; MKV or an audio container for fast cuts | Precise cuts decode and encode H.264/AAC, with output seeking after input decoding. Fast cuts copy streams and seek to available keyframes, so the boundary may differ from the requested time. |
| Compress | MP4 with H.264/AAC | Balanced, small and high profiles reduce the source bitrate. An optional target size derives a video bitrate after reserving audio and muxing overhead. It is an estimate, not a guaranteed output size. |
| Animation | GIF, optional animated WebP | Select start/end, width and FPS. GIF uses a generated palette. Up to five minutes, 1920 pixels wide and 60 FPS. WebP requires `libwebp_anim`, which is not present in the current engine bundle; the application reports this requirement instead of creating a fake result. |
| Capture frame | PNG | Select a time and output width. Accurate output seeking and one decoded frame. |
| Contact sheet | PNG | Samples the video timeline into a bounded grid of up to 10 × 10 thumbnails. |
| Rotate / crop frame | MP4 or PNG | Video is encoded as H.264/AAC. Still images become PNG. Crop coordinates are checked against the source dimensions and must have even width/height. |
| Remove audio | MKV | Copies the video stream without audio. No video re-encoding. |
| Process audio | M4A/AAC | Single-pass EBU R128 normalization with podcast −16 LUFS, broadcast −23 LUFS or music −14 LUFS profiles. Basic FFT denoising or silence removal are separate choices. Silence below −45 dB for at least 0.5 seconds is removed. These settings can remove quiet speech; audition the original and compare the resulting copy. |
| Convert images | PNG, JPEG, optional WebP | Batch jobs, proportional resize, quality and optional metadata removal. Static conversion rejects detected or uncertain animation. JPEG is rejected when transparency cannot be preserved. WebP requires `libwebp`, absent from the current bundle. |

Original extraction supports MP3, AAC, ALAC, Opus, Vorbis, FLAC and common little-endian PCM audio. Unsupported source codecs require an explicit MP3 or M4A choice. These operations use the first real video track and first audio track; subtitles, attachments and other tracks are omitted. Audio/video inputs require a valid positive duration. Still-image conversion, rotation and cropping do not require a duration.

Video encoding uses the pinned FFmpeg build's `libopenh264` and `libvpx-vp9`; audio uses `libmp3lame`, AAC and `libopus`. These encoders are checked by `scripts/build-ffmpeg.mjs`. The tools preserve resolution and frame rate unless the encoder rejects the source. No GPU acceleration is advertised. Copy is checked against the selected container before starting FFmpeg. Container support does not imply universal player support.

Lossy conversion does not improve the original audio. The compression target is expressed in MiB, uses a 5% muxing reserve, and reserves 128 kbps for an audio track. Duration, source complexity and codec behavior affect the actual output size. Targets requiring less than 100 kbps or more than 200 Mbps of video are rejected.

## File safety and execution

`src-tauri/src/tools.rs` parses tagged, camel-case `ToolOptions` at IPC. It rejects unknown fields, unsupported codecs and invalid ranges. Inputs must be existing absolute local paths. Network paths, URL inputs and direct symbolic links are rejected. Canonical Windows drive paths remain accepted.

Probing and conversion allow only `file,pipe` protocols and an explicit list of media demuxers. HLS, DASH and concat demuxers are excluded so a local file cannot become a network manifest or a list of unrelated local files. The executable is resolved from the managed engine directory. All arguments are passed as an argument vector to the existing process runner and its Windows Job Object.

FFmpeg uses `-nostdin` and `-n`. Output names use a sanitized source stem and operation suffix. Every output stays in the job's staging folder. Existing outputs are rejected. The queue is responsible for atomically publishing the verified output without replacing an existing file, indexing it in the library and cleaning only its own temporary files.

The existing runner does not expose structured FFmpeg processing percentages. Tools therefore report an indeterminate `processing` phase followed by `verifying`. They do not fabricate percentages, speed or completion. Cancellation uses the same watch channel and process-tree termination as downloads.

Before returning, the tool probes the result and verifies a positive size, the actual container and expected audio/video codecs. Precise cuts and full-length operations also check the duration within a small muxing tolerance. Still images have no duration requirement. Fast cuts and silence removal deliberately do not claim an exact duration. Completion is emitted by the queue only after final publication.

The new operations use the existing FFmpeg component and add no runtime dependency. The current bundle was inspected for GIF/PNG/JPEG encoders and the required filters. WebP remains an explicit encoder limitation. Clip joining, replacing audio, burning subtitles, palette export, transcription and PDF/EPUB tools are separate work and are not represented by these operation names.

## Verification

The `tools::tests` module covers the serialized IPC contract, copy/container incompatibility, Opus preservation versus MP3 encoding, trim bounds and modes, target bitrate calculation, output validation and filename/path safety. Run these through `cargo test --manifest-path src-tauri/Cargo.toml tools::tests` after the module is integrated. Real engine checks require the pinned engine bundle; unit fixtures alone do not certify runtime behavior.
