import type { PlaybackPreferences } from "./playback-preferences";

type Command = <T>(name: string, args: Record<string, unknown>) => Promise<T>;
type StyledSource = { kind: "file" } | { kind: "embedded"; video: string; index: number; title: string };

export function createStyledCaptions(host: HTMLElement, video: HTMLVideoElement, command: Command, onError: () => void) {
  const canvas = document.createElement("canvas");
  canvas.className = "stage-styled-captions";
  canvas.setAttribute("aria-hidden", "true");
  canvas.hidden = true;
  video.after(canvas);
  const context = canvas.getContext("2d");
  let track: { id: string; title: string } | undefined;
  let revision = 0;
  let enabled = false;
  let pending = false;
  let failed = false;
  let loading = "";
  let frame = 0;
  let lastTick = 0;
  let offset = 0;
  let size = 1;
  let lastKey = "";
  let force = true;

  function clear() {
    canvas.hidden = true;
    lastKey = "";
    force = true;
  }

  function schedule() {
    if (frame || !enabled || failed || video.paused || document.hidden) return;
    frame = requestAnimationFrame(now => {
      frame = 0;
      if (now - lastTick < 75) { schedule(); return; }
      lastTick = now;
      draw();
    });
  }

  function draw() {
    if (!enabled || !track || failed || !context || document.hidden) return;
    schedule();
    const seconds = video.currentTime - offset;
    if (video.seeking || seconds < 0 || !video.videoWidth || !video.videoHeight) { clear(); return; }
    if (loading || pending) return;

    const sourceWidth = video.videoWidth, sourceHeight = video.videoHeight;
    const viewportWidth = host.clientWidth, viewportHeight = host.clientHeight;
    const scale = host.dataset.fit === "cover" ? Math.max : Math.min;
    const fit = scale(viewportWidth / sourceWidth, viewportHeight / sourceHeight);
    if (!fit) return;
    const displayWidth = sourceWidth * fit;
    const displayHeight = sourceHeight * fit;
    const resolution = Math.min(1, 960 / Math.max(displayWidth, displayHeight), Math.sqrt(480_000 / (displayWidth * displayHeight)));
    const width = Math.max(1, Math.floor(displayWidth * resolution));
    const height = Math.max(1, Math.floor(displayHeight * resolution));
    const safeBottom = viewportHeight - (host.dataset.controls === "hidden" ? 0 : 88);
    const lift = Math.max(0, Math.min(60, Math.ceil(100 * ((viewportHeight + displayHeight) / 2 - safeBottom) / displayHeight)));
    const time = video.paused ? Math.round(seconds * 1000) : Math.floor(seconds * 12) * 1000 / 12;
    const timestamp = Math.floor(time);
    const key = `${width}:${height}:${size}:${lift}:${timestamp}`;
    if (!force && lastKey === key) return;
    lastKey = key;
    const id = track.id;
    const requestRevision = revision;
    const geometry = { width, height, sourceWidth, sourceHeight, size, lift };
    const refresh = force;
    force = false;
    pending = true;

    void command<ArrayBuffer>("styled_caption_frame", { request: { id, time: timestamp, geometry, force: refresh } }).then(bytes => {
      if (revision !== requestRevision || !enabled || track?.id !== id || video.seeking || Math.abs(video.currentTime - offset - seconds) > 0.35) {
        force = true;
        return;
      }
      if (!(bytes instanceof ArrayBuffer)) throw Error("Invalid styled caption response.");
      canvas.style.width = `${displayWidth}px`;
      canvas.style.height = `${displayHeight}px`;
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }
      if (bytes.byteLength === 1) return;
      context.clearRect(0, 0, width, height);
      canvas.hidden = bytes.byteLength === 0;
      if (canvas.hidden) return;
      const data = new DataView(bytes);
      const x = data.getUint16(0, true), y = data.getUint16(2, true);
      const columns = data.getUint16(4, true), rows = data.getUint16(6, true);
      if (x + columns > width || y + rows > height || bytes.byteLength !== 8 + columns * rows * 4) throw Error("Invalid styled caption bitmap.");
      context.putImageData(new ImageData(new Uint8ClampedArray(bytes, 8), columns, rows), x, y);
    }).catch(() => {
      if (revision !== requestRevision || track?.id !== id) return;
      failed = true;
      clear();
      onError();
    }).finally(() => {
      pending = false;
      if (enabled && (force || video.paused)) draw();
    });
  }

  function update(visible: boolean, delay: number, scale: PlaybackPreferences["captionSize"]) {
    const nextSize = scale === "small" ? 0 : scale === "large" ? 2 : 1;
    if (enabled !== visible || delay !== offset || nextSize !== size) { ++revision; clear(); }
    enabled = visible;
    offset = delay;
    size = nextSize;
    if (!enabled) { cancelAnimationFrame(frame); frame = 0; return; }
    draw();
  }

  async function load(source: StyledSource) {
    if (!context) throw Error("The styled subtitle canvas is unavailable.");
    const id = crypto.randomUUID();
    loading = id;
    try {
      let title: string | null;
      if (source.kind === "embedded") {
        await command<void>("styled_caption_embedded", { id, video: source.video, index: source.index });
        title = source.title;
      } else {
        title = await command<string | null>("styled_caption_open", { id, path: null });
      }
      if (!title) return false;
      if (loading !== id) {
        await command<void>("styled_caption_close", { id });
        return false;
      }
      track = { id, title };
      failed = false;
      clear();
      return true;
    } finally { if (loading === id) loading = ""; }
  }

  function close() {
    ++revision;
    enabled = false;
    cancelAnimationFrame(frame);
    frame = 0;
    if (loading) void command<void>("styled_caption_close", { id: loading }).catch(() => {});
    loading = "";
    if (track) void command<void>("styled_caption_close", { id: track.id }).catch(() => {});
    track = undefined;
    failed = false;
    clear();
  }

  new ResizeObserver(draw).observe(host);
  document.addEventListener("visibilitychange", draw);
  return { load, close, update, title: () => track?.title };
}
