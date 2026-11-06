import { escapeHtml as h, type Copy, type Job } from "./contracts";
import { action, option } from "./views-common";

type Operation = "bilibiliDanmaku" | "youtubeThumbnail" | "youtubeDescription" | "youtubeMetadata" | "youtubeChapters" | "youtubePlaylist" | "redditPost" | "twitterThread";
const operations: { value: Operation; es: string; en: string }[] = [
  {value:"bilibiliDanmaku",es:"Bilibili · danmaku XML",en:"Bilibili · danmaku XML"},
  { value: "youtubeThumbnail", es: "YouTube · miniatura", en: "YouTube · thumbnail" },
  { value: "youtubeDescription", es: "YouTube · descripción", en: "YouTube · description" },
  { value: "youtubeMetadata", es: "YouTube · metadatos", en: "YouTube · metadata" },
  { value: "youtubeChapters", es: "YouTube · capítulos", en: "YouTube · chapters" },
  { value: "youtubePlaylist", es: "YouTube · datos de una lista", en: "YouTube · playlist metadata" },
  { value: "redditPost", es: "Reddit · publicación y comentarios", en: "Reddit · post and comments" },
  { value: "twitterThread", es: "X · conversación accesible", en: "X · accessible conversation" },
];
function formats(operation: Operation): readonly string[] {
  switch (operation) {
    case "bilibiliDanmaku":return ["xml"];
    case "youtubeThumbnail": return ["original"];
    case "youtubeDescription": return ["txt"];
    case "youtubeMetadata": case "youtubePlaylist": return ["json"];
    case "youtubeChapters": return ["json", "md"];
    default: return ["json", "md", "html"];
  }
}
interface Hooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  refresh(): Promise<void>;
  render(): void;
  notify(text: string, kind?: "info" | "success" | "error"): void;
  showError(error: unknown): void;
  t: Copy;
}
export function createSourceExportsController(hooks: Hooks) {
  const { t } = hooks;
  let operation: Operation = "youtubeMetadata";
  let format = "json";
  let url = "";
  let busy = false;
  function open(input = ""): void {
    url = input;
    if (input) {
      try { const host = new URL(input).hostname; if (host === "redd.it" || host === "reddit.com" || host.endsWith(".reddit.com")) operation = "redditPost"; else if (["x.com", "twitter.com"].some(domain => host === domain || host.endsWith(`.${domain}`))) operation = "twitterThread"; else operation = "youtubeMetadata"; }
      catch { operation = "youtubeMetadata"; }
    }
    format = formats(operation)[0];
  }
  function render(): string {
    const archive = operation === "redditPost" || operation === "twitterThread";
    return `<div class="page-heading"><div><h1>${t("Exportar información", "Export source information")}</h1><p>${t("Guarda texto y datos de una fuente sin descargar sus vídeos.", "Save source text and metadata without downloading videos.")}</p></div></div>
      <form id="source-export-form" class="settings-form" aria-busy="${busy}"><label>${t("Qué guardar", "What to save")}<select id="source-export-operation">${operations.map(item => option(item.value, t(item.es, item.en), operation)).join("")}</select></label><label>${t("Enlace de origen", "Source URL")}<input id="source-export-url" type="url" required autocomplete="off" placeholder="https://…" value="${h(url)}"></label><label>${t("Formato", "Format")}<select id="source-export-format">${formats(operation).map(value => option(value, value === "original" ? t("Imagen original", "Original image") : value === "html" ? t("HTML estático", "Static HTML") : value === "md" ? "Markdown" : value.toUpperCase(), format)).join("")}</select></label>
      <p class="field-note">${operation === "youtubePlaylist" ? t("Hasta 200 posiciones de la lista. El archivo indica el alcance y si quedan entradas fuera. No descarga los vídeos.", "Up to 200 playlist positions. The export records its scope and whether entries remain. It does not download videos.") : operation === "redditPost" ? t("Una petición con hasta 200 comentarios y profundidad limitada. Se indican los comentarios no recuperados. Los datos privados o borrados pueden faltar.", "One request with up to 200 comments and limited depth. Unfetched comments are identified. Private or deleted data may be absent.") : operation === "twitterThread" ? t("Hasta 200 publicaciones devueltas por la conversación accesible. Requiere Galerías y, normalmente, una sesión de X. No certifica un hilo completo.", "Up to 200 posts returned by the accessible conversation. Requires Galleries and usually an X session. It does not certify a complete thread.") : operation === "youtubeThumbnail" ? t("Conserva el formato de la miniatura accesible, hasta 2 MiB. No descarga el vídeo.", "Preserves the accessible thumbnail format, up to 2 MiB. It does not download the video.") : t("Usa los metadatos y capítulos disponibles en la fuente. Un resultado ausente se indica como error, no como un archivo vacío.", "Uses the metadata and chapters available from the source. Missing results produce an error instead of an empty file.")}</p>
      ${archive ? `<p class="field-note">${t("JSON conserva el texto y la estructura recuperados. Markdown y HTML no cargan imágenes remotas ni scripts de la plataforma. Las sesiones se gestionan en Ajustes.", "JSON preserves returned text and structure. Markdown and HTML do not load remote images or platform scripts. Sessions are managed in Settings.")}</p>` : ""}
      <div class="button-group"><button type="submit" class="button primary" ${busy ? "disabled" : ""}>${busy ? t("Preparando…", "Preparing…") : t("Añadir exportación a la cola", "Add export to queue")}</button>${action("source-export-clear", t("Vaciar enlace", "Clear link"), undefined, "quiet", busy ? "disabled" : "")}</div></form>`;
  }
  function handleInput(target: HTMLElement): boolean {
    if (target instanceof HTMLInputElement && target.id === "source-export-url") { url = target.value; return true; }
    if (target instanceof HTMLSelectElement && target.id === "source-export-operation") {
      const selected = operations.find(item => item.value === target.value);
      if (selected) { operation = selected.value; format = formats(operation)[0]; hooks.render(); }
      return true;
    }
    if (target instanceof HTMLSelectElement && target.id === "source-export-format") { if (formats(operation).includes(target.value)) format = target.value; return true; }
    return false;
  }
  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    if (form.id !== "source-export-form") return false;
    if (busy) return true;
    busy = true; hooks.render();
    try { const jobs = await hooks.command<Job[]>("queue_source_export", { url, operation, format, clientRequest: crypto.randomUUID() }); await hooks.refresh(); hooks.notify(jobs.length ? t("Exportación añadida a la cola.", "Export added to queue.") : t("La exportación ya está en cola.", "The export is already queued."), "success"); }
    catch (error) { hooks.showError(error); }
    finally { busy = false; hooks.render(); }
    return true;
  }
  function handleClick(name: string): boolean { if (name !== "source-export-clear") return false; url = ""; hooks.render(); return true; }
  return { open, render, handleInput, handleSubmit, handleClick };
}
