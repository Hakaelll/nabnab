// This function is injected once into the main frame; keep it independent of module state.
export function scanMedia() {
  const items = [];
  const nodes = [...document.querySelectorAll("video,audio")].slice(0, 100);
  const pageHasDrm = nodes.some(node => Boolean(node.mediaKeys));
  for (const node of nodes) {
    const details = { width: node.videoWidth || null, height: node.videoHeight || null, protected: Boolean(node.mediaKeys), origin: "element" };
    if (node.currentSrc || node.src) items.push({ ...details, url: node.currentSrc || node.src, mime: node.getAttribute("type") || "" });
    for (const source of [...node.querySelectorAll("source[src]")].slice(0, 20)) {
      items.push({ ...details, url: source.src, mime: source.type || "" });
    }
    if (items.length >= 200) break;
  }
  for (const entry of performance.getEntriesByType("resource").slice(-3000)) {
    if (items.length >= 400) break;
    if (/\.(?:mp4|webm|m4a|mp3|ogg|oga|opus|wav|aac|flac|m3u8|mpd)(?:[?#]|$)/i.test(entry.name)) {
      items.push({ url: entry.name, mime: "", width: null, height: null, size: entry.encodedBodySize > 0 ? entry.encodedBodySize : null, protected: pageHasDrm, origin: "resource" });
    }
  }
  return { pageUrl: location.href, pageHasDrm, items: items.slice(0, 400) };
}

const extensions = { mp4: "MP4", webm: "WebM", m4a: "M4A", mp3: "MP3", ogg: "Ogg", oga: "Ogg", opus: "Opus", wav: "WAV", aac: "AAC", flac: "FLAC", m3u8: "HLS", mpd: "DASH" };
const mimeFormats = { "video/mp4": "MP4", "video/webm": "WebM", "audio/mp4": "M4A", "audio/mpeg": "MP3", "audio/ogg": "Ogg", "audio/opus": "Opus", "audio/wav": "WAV", "audio/aac": "AAC", "audio/flac": "FLAC", "application/vnd.apple.mpegurl": "HLS", "application/x-mpegurl": "HLS", "application/dash+xml": "DASH" };
const positive = value => typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;

export function normalizeMediaScan(raw, { id = () => crypto.randomUUID() } = {}) {
  if (!raw || typeof raw !== "object" || !Array.isArray(raw.items)) throw new Error("La pestaña no ha devuelto una lista de medios válida.");
  const found = new Map();
  let blobs = 0;
  for (const item of raw.items.slice(0, 400)) {
    if (!item || typeof item.url !== "string" || item.url.length > 8192 || /[\u0000-\u001f\u007f]/.test(item.url)) continue;
    if (item.url.startsWith("blob:")) { blobs++; continue; }
    let url;
    try { url = new URL(item.url); } catch { continue; }
    if (!["http:", "https:"].includes(url.protocol) || !url.hostname || url.username || url.password) continue;
    const extension = url.pathname.match(/\.([a-z0-9]+)$/i)?.[1]?.toLowerCase();
    const mime = typeof item.mime === "string" ? item.mime.split(";")[0].trim().toLowerCase() : "";
    const format = (Object.hasOwn(mimeFormats, mime) ? mimeFormats[mime] : null) || (Object.hasOwn(extensions, extension) ? extensions[extension] : null);
    if (!format) continue;
    url.hash = "";
    const key = url.href;
    const existing = found.get(key);
    if (existing) {
      existing.width ??= positive(item.width); existing.height ??= positive(item.height); existing.size ??= positive(item.size);
      existing.protected ||= item.protected === true;
      continue;
    }
    if (found.size >= 200) break;
    found.set(key, { id: id(), url: key, host: url.hostname, format, width: positive(item.width), height: positive(item.height), size: positive(item.size), protected: item.protected === true, manifest: format === "HLS" || format === "DASH" });
  }
  return { candidates: [...found.values()], blobs, drmDetected: raw.pageHasDrm === true || [...found.values()].some(item => item.protected) };
}

export function mediaSummaries(scan) {
  return { candidates: scan.candidates.map(({ id, host, format, width, height, size, protected: protectedMedia, manifest }) => ({ id, host, format, width, height, size, protected: protectedMedia, manifest })), blobs: scan.blobs, drmDetected: scan.drmDetected };
}

export function selectedMediaRequest(snapshot, { tabId, pageUrl, ids, profile = "source", presetId, requestId, now = Date.now() }) {
  if (!snapshot || snapshot.tabId !== tabId || snapshot.pageUrl !== pageUrl || !Number.isFinite(snapshot.createdAt) || now - snapshot.createdAt > 5 * 60 * 1000 || now < snapshot.createdAt) {
    throw new Error("La detección ha caducado o la pestaña ha cambiado. Vuelve a detectar los medios.");
  }
  if (!Array.isArray(ids) || !ids.length || ids.length > 200 || ids.some(id => typeof id !== "string")) throw new Error("Selecciona entre 1 y 200 medios detectados.");
  const urls = [...new Set(ids)].map(id => {
    const candidate = snapshot.candidates.find(item => item.id === id);
    if (!candidate || candidate.protected) throw new Error("La selección contiene un medio protegido o que ya no está disponible.");
    return candidate.url;
  });
  const request = { url: pageUrl, urls, intent: "prepare", profile, requestId, ...(presetId ? { presetId } : {}) };
  if (new TextEncoder().encode(JSON.stringify(request)).length > 15_500) throw new Error("La selección contiene enlaces largos. Envía menos medios cada vez.");
  return request;
}
