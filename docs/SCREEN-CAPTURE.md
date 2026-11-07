# Screen capture

Capture is an explicit desktop action under Tools → Capture. Enabling the component does not start recording. Screen capture, audio capture and replay are unavailable to the local MCP/CLI automation interface.

Select a monitor or window, request a preview, choose recording settings and start. A monitor may be cropped to a region entirely inside that monitor; region coordinates are relative to the selected monitor. The preview expires after two minutes. Start and screenshot revalidate the source rather than accepting a window handle or path supplied by the frontend. The destination is the authorized output folder captured when recording starts, and remains attached to replay snapshots and recovery journals.

Microphone and system audio are independent options, both initially off. System audio records the complete mix played through the selected Windows output endpoint, including sound from other applications. It does not isolate the selected window. With both options selected, the MKV plays a normalized system-plus-microphone mix by default and retains the two independently titled original tracks for editing. One selected source produces one audio track; both off produces video only.

## Backend and limits

Window capture uses the official Windows Graphics Capture HWND interop API on Windows 10 version 1903 or later and Windows 11. A free-threaded D3D11 frame pool reads the selected window's composed pixels, including accelerated WebView2 content. The application retains the system capture border, uses no global picker or desktop fallback, and crops each readback to the selected window's client area. A background MTA thread copies bounded BGRA frames to the existing managed FFmpeg encoder through stdin; preview and screenshot use that same graphics backend. Static windows repeat the last captured frame at the selected frame rate. The input channel holds one frame, and excessive encoder backpressure stops with an explicit partial result.

Explicit monitor/region capture retains the managed FFmpeg 9.0.2 `gdigrab` input. No separate screen-recorder executable or global installation is required. Frame rates are 15 or 30 fps, with standard/high OpenH264 quality, two-second keyframe segments and Matroska storage. Odd source dimensions receive at most one padding pixel for H.264 compatibility. Screenshots preserve the selected pixel dimensions in PNG. Each source/region is limited to 24 million pixels; graphics/readback allocations and the raw-frame channel are bounded independently of replay duration.

Protected content may appear blank; the application does not bypass capture protection. Verify the preview. Capture is SDR BGRA; this pipeline does not preserve HDR range. A window changing size, closing, becoming minimized or being replaced stops capture with an interrupted result. Display size/origin changes also stop it. Previewed windows are bound using a private Windows property removed when the window is destroyed, and checked with process identity, creation time, class and dimensions to reject handle reuse. Unsupported graphics capture fails explicitly rather than falling back to GDI for an accelerated window.

The application enumerates WASAPI render/capture endpoints through the MIT-licensed `wasapi` 0.26.0 crate and uses Windows shared-mode loopback for system sound. It does not depend on a vendor's “Stereo Mix” device. Native packet reads handle silent/null buffers, validate sizes, release each COM buffer on error, and use the packet's QPC timestamp. The first window frame's actual WGC `SystemRelativeTime` QPC timestamp—or gdigrab's microsecond PTS for an explicitly selected monitor—and a native QPC/epoch anchor align audio with video during finalization. Starting two processes at approximately the same time is not used as synchronization. Audio discontinuity, invalid timestamps or a disconnected selected device produce an interrupted result.

Recording duration is limited to 1–14,400 seconds and disk budget to 64–4,096 MiB. The backend reserves space for a source-sized frame and encoder buffering, validates that the budget can accommodate that reserve, counts video, both audio tracks, journals and currently open files, and polls limits every 100 ms. A configured duration/disk stop is a normal bounded stop; source/device/process failure is partial. Disk exhaustion can still interrupt a write and is reported as such.

## Replay and recovery

Replay is opt-in for 30, 60 or 120 seconds. Video and both selected audio tracks use two-second owned segments in the same bounded ring. Old video/audio segments are deleted as the window advances. The indicator reports completed available seconds. Saving snapshots only completed video segments, so the currently open final one to two seconds may be absent. Snapshots contain only media generated after activation. Selected files are copied into a new owned capture ID before a snapshot returns, allowing queue finalization to wait without depending on files the ring will delete.

The snapshot pins the completed segments while a blocking disk worker copies them, so the encoder can keep consuming frames. Stopping a healthy replay buffer discards its remaining temporary segments. Disabling capture and application exit stop the active recorder/buffer. An interrupted buffer retains available segments for an explicit partial recovery action. Capture does not start on application launch.

Recordings remain recoverable as independently decodable MKV segments while running. Each PCM WAV segment checkpoints its header, and the private journal records owned file names, timing, destination and partial status. `captures/` uses the application's private-directory Windows ACL helper. An exclusive active-session lease prevents recovery from rewriting a running session. After an unexpected exit, recovery marks its journal interrupted; it never resumes screen or microphone capture. Publication is marked only after the common queue/library transaction has succeeded.

Finalization joins selected segments, aligns and pads the independent audio tracks, creates a new MKV in queue staging and decodes the resulting video to check integrity. Failed, canceled or corrupt finalization is not marked complete. Originals remain until publication, permitting recovery/retry through the existing queue. A partial output is named `Recording-partial.mkv` and carries explicit interrupted metadata.

## Integration

The desktop host owns component enablement, authorized destination, preview cancellation, persistent indicator, explicit shortcuts, stop-on-exit and publication. `Controller` starts directly, without acquiring the conversion queue permit. `Source::Capture { captureId }` can only finalize an already captured owned ID.

Backend entry points are `Controller::{sources, preview, start, status, stop, screenshot, save_replay, shutdown}`, plus `recover`, `readjournal`, `mark_published` and `finalize`. `start` and `screenshot` receive the host-authorized destination internally; no IPC argument accepts an arbitrary filesystem path. Frontend results contain opaque source/session/capture IDs and a PNG preview data URL, not native handles or private capture paths.

## Verification

Focused tests cover option/path boundaries, journal recovery without activation, null silent audio packets, recoverable PCM WAV segments and ring deletion. Opt-in native tests use `NABNAB_CAPTURE_FFMPEG` pointing to a managed engine directory. Their generated-pixel/tone fixture checks real remuxing, a default mix plus two distinct original audio tracks, literal 500/1,000 ms offsets and sample values before/after each source starts. The Windows fixture creates its own synthetic window and targets only that HWND for preview, PNG, recording, replay snapshot resize/closure interruption and a complete 66-second replay retention run. It never captures a personal desktop or microphone. Real audio hardware/permission/disconnection behavior must be tested separately with explicit consent; synthetic tones do not establish hardware compatibility.

`tests/native-screen-capture.browser.mjs` launches a fresh isolated application profile, selects only that process's own HWND, and checks real IPC, queue/library publication, destination binding, global controls, actual OS shortcut release and partial recovery. Preview, PNG and a decoded video frame must contain varied UI pixels: dimensions and a successful MKV remux alone cannot pass the accelerated-window regression. The native library `<video>` must load real metadata with the measured dimensions/duration, or explicitly report unsupported media; the test never starts playback.

The [native Windows Graphics Capture run](verification/capture-native.json) passed all seven integrated checks. Its 1475 × 975 accelerated-window screenshot and decoded 1476 × 976 recording frame each contain at least 256 sampled colors; the corresponding PNGs were visually inspected and show the actual application UI. The native library player loaded the 4.266-second MKV with `readyState = 4` while paused. These results establish the window-capture path on this test machine, not audio hardware or every Windows/graphics-driver combination.

```powershell
$env:NABNAB_CAPTURE_FFMPEG = (Resolve-Path .cache/ffmpeg-build/bundle).Path
cd src-tauri
cargo test screen_capture:: -- --nocapture
cargo test native_capture_ -- --ignored --nocapture --test-threads=1
```

References: [Microsoft HWND capture interop and minimum version](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow), [Microsoft capture frame pool, QPC timing and graphics resources](https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture), [FFmpeg gdigrab implementation](https://github.com/FFmpeg/FFmpeg/blob/master/libavdevice/gdigrab.c), [FFmpeg input-device documentation](https://ffmpeg.org/ffmpeg-devices.html#gdigrab), [Microsoft WASAPI loopback](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording), [Microsoft GetBuffer timestamp and silent-buffer contract](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer), [wasapi 0.26.0](https://docs.rs/wasapi/0.26.0/wasapi/).

Window close callbacks may outlive the capture thread. The capture implementation keeps the loaded Windows module resident by the address of its activation entry point, while releasing each capture session and its frame pool. [Microsoft module lifetime reference](https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-getmodulehandleexw).

The final synthetic minute test waited for frames before timing 66 seconds of active replay. It retained 30 completed segments, produced a 59.999-second MKV measured with ffprobe, and cleared the live buffer on Stop. Resize, close and synthetic audio track checks also passed.
