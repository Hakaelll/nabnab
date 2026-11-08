const maxFrameBytes = 48 * 1024;
const maxCachedFrames = 28;

export function createPlaybackPreview(command: <T>(name: string, args: Record<string, unknown>) => Promise<T>) {
  const cache = new Map<string, string>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let request = "";
  let image: HTMLImageElement | null = null;
  let key = "";

  function leave() {
    clearTimeout(timer);
    if (request) void command("playback_preview_cancel", { request }).catch(() => {});
    request = "";
    key = "";
    if (image) { image.hidden = true; image.removeAttribute("src"); }
    image = null;
  }

  function hover(target: HTMLImageElement, id: string, seconds: number) {
    const time = Math.floor(seconds / 3) * 3;
    const next = `${id}:${time}`;
    if (key === next && image === target) return;
    leave();
    key = next;
    image = target;
    const cached = cache.get(key);
    if (cached) {
      cache.delete(key); cache.set(key, cached);
      target.src = cached; target.hidden = false;
      return;
    }
    timer = setTimeout(() => {
      const token = crypto.randomUUID();
      request = token;
      void command<number[]>("playback_preview_frame", { id, time, request: token }).then(bytes => {
        if (request !== token || !target.isConnected || !Array.isArray(bytes) || !bytes.length || bytes.length > maxFrameBytes) return;
        const url = `data:image/jpeg;base64,${btoa(String.fromCharCode(...bytes))}`;
        if (cache.size >= maxCachedFrames) cache.delete(cache.keys().next().value ?? "");
        cache.set(next, url);
        target.src = url;
        target.hidden = false;
      }).catch(() => { /* Keep the seek time visible when a frame cannot be decoded. */ })
        .finally(() => { if (request === token) request = ""; });
    }, 260);
  }

  return { hover, leave, clear() { leave(); cache.clear(); } };
}
