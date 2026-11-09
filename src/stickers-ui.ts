import "./stickers.css";
import { escapeHtml as h, type Copy, type Snapshot } from "./contracts";
import { imageExports } from "./image-exports";
import { isCancelled } from "./presentation";
import { action, icon, option } from "./views-common";

type Profile = "telegramImage" | "telegramVideo" | "whatsappImage" | "whatsappAnimated";
type Surface = "grid" | "light" | "dark";
const profiles: Record<Profile, { limit: number; duration: number | null; format: string }> = {
  telegramImage: { limit: 512, duration: null, format: "WebP" },
  telegramVideo: { limit: 256, duration: 3, format: "WebM · VP9" },
  whatsappImage: { limit: 100, duration: null, format: "WebP" },
  whatsappAnimated: { limit: 500, duration: 10, format: "WebP" },
};
interface Options {
  profile: Profile;
  crop: { horizontal: number; vertical: number } | null;
  outline: number;
  startMs: number;
  durationMs: number | null;
}
interface Preview {
  previewId: number;
  data: number[];
  width: number;
  height: number;
  durationMs: number | null;
  frameRate: number | null;
  bytes: number;
  limit: number;
  mime: "video/webm" | "image/webp";
}
interface Hooks {
  snapshot(): Snapshot;
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  render(): void;
  refresh(): Promise<void>;
  notify(message: string): void;
  t: Copy;
}

const inputTypes = /\.(png|jpe?g|webp|bmp|gif|mp4|m4v|mov|mkv|webm)$/i;
const motionTypes = /\.(gif|mp4|m4v|mov|mkv|webm)$/i;

function webpUrl(bytes: number[]) {
  let binary = "";
  for (let offset = 0; offset < bytes.length; offset += 16384) binary += String.fromCharCode(...bytes.slice(offset, offset + 16384));
  return "data:image/webp;base64," + btoa(binary);
}

async function firstFrame(bytes: number[]) {
  const bitmap = await createImageBitmap(new Blob([new Uint8Array(bytes)], { type: "image/webp" }));
  try {
    const canvas = document.createElement("canvas");
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const context = canvas.getContext("2d");
    if (!context) return "";
    context.drawImage(bitmap, 0, 0);
    return canvas.toDataURL("image/png");
  } finally {
    bitmap.close();
  }
}

export function createStickerTool(hooks: Hooks) {
  const { t } = hooks;
  let libraryId = "", query = "", sourceOpen = false;
  let profile: Profile = "telegramImage", crop = false, horizontal = 50, vertical = 50, outline = 4;
  let start = "0", duration = "1.5", busy = false, requestId = 0, error = "", playAnimation = false;
  let preview: Omit<Preview, "data"> | null = null;
  let previewUrl = "", posterUrl = "";
  let surface: Surface = "grid";
  const items = () => hooks.snapshot().library.filter(item => !item.missing && inputTypes.test(item.name) && (item.kind === "image" || item.kind === "video"));
  const selected = () => items().find(item => item.id === libraryId);
  const motion = () => motionTypes.test(selected()?.name ?? "");
  const profileName = (key: Profile) => ({
    telegramImage: t("Telegram · imagen", "Telegram · image"),
    telegramVideo: t("Telegram · vídeo", "Telegram · video"),
    whatsappImage: t("WhatsApp · imagen", "WhatsApp · image"),
    whatsappAnimated: t("WhatsApp · animación", "WhatsApp · animation"),
  })[key];
  const disabled = () => busy ? "disabled" : "";
  const number = (value: number, digits = 0) => new Intl.NumberFormat(t("es", "en"), { maximumFractionDigits: digits }).format(value);

  function invalidate() {
    if (previewUrl.startsWith("blob:")) URL.revokeObjectURL(previewUrl);
    previewUrl = "";
    posterUrl = "";
    preview = null;
    playAnimation = false;
  }

  function options(): Options {
    return {
      profile, crop: crop ? { horizontal, vertical } : null, outline,
      startMs: motion() ? Math.round(Number(start) * 1000) : 0,
      durationMs: profiles[profile].duration === null ? null : Math.round(Number(duration) * 1000),
    };
  }

  async function run(task: () => Promise<void>) {
    if (busy) return;
    busy = true;
    error = "";
    hooks.render();
    try { await task(); }
    catch (cause) { if (!isCancelled(cause)) error = (cause instanceof Error ? cause.message : String(cause)).slice(0, 500); }
    finally { busy = false; requestId = 0; hooks.render(); }
  }

  function chooseInput() {
    const chosen = selected();
    const matches = items().filter(item => item.name.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
    return `<section class="asset-inputs">
      <div class="section-heading"><h3>${t("Entrada", "Input")}</h3>${action("import-files", t("Importar archivo", "Import file"), "add", "quiet", disabled())}</div>
      <div class="sticker-source">
        <span>${h(chosen?.name ?? t("Elige un archivo de Biblioteca", "Choose a file from Library"))}</span>
        ${action("sticker-source", sourceOpen ? t("Cerrar selección", "Close picker") : t("Elegir archivo", "Choose file"), "folder", "secondary", `aria-expanded="${sourceOpen}" aria-controls="sticker-picker" ${disabled()}`)}
      </div>
      <div id="sticker-picker" ${sourceOpen ? "" : "hidden"}>
        <label for="sticker-query">${t("Buscar en Biblioteca", "Find in Library")}<input type="search" id="sticker-query" value="${h(query)}" ${disabled()}></label>
        ${matches.length ? `<div class="library-picker asset-picker"><ul>${matches.slice(0, 200).map(item => `<li><label>
          <input type="radio" name="sticker-input" value="${h(item.id)}" ${item.id === libraryId ? "checked" : ""} ${disabled()}><span title="${h(item.name)}">${h(item.name)}</span></label></li>`).join("")}</ul></div>`
          : `<p class="field-note">${t("Importa una imagen, un GIF o un vídeo compatible.", "Import a compatible image, GIF or video.")}</p>`}
        ${matches.length > 200 ? `<p class="field-note">${t("Se muestran 200 archivos. Acota la búsqueda.", "Showing 200 files. Narrow the search.")}</p>` : ""}
      </div>
    </section>`;
  }

  function settings() {
    const limits = profiles[profile];
    return `<fieldset class="asset-settings sticker-options" ${disabled()}>
      <legend>${t("Opciones", "Options")}</legend>
      <label for="sticker-profile">${t("Perfil de exportación", "Export profile")}<select id="sticker-profile">
        ${(Object.keys(profiles) as Profile[]).map(key => {
          const label = `${profileName(key)} · ${profiles[key].format}`;
          return `<option value="${key}" ${key === profile ? "selected" : ""} ${profiles[key].duration !== null && !motion() ? "disabled" : ""}>${h(label)}</option>`;
        }).join("")}</select></label>
      <div class="sticker-fields">
        <label for="sticker-fit">${t("Encuadre", "Framing")}<select id="sticker-fit">
          ${option("contain", t("Conservar encuadre", "Keep framing"), crop ? "crop" : "contain")}${option("crop", t("Recorte cuadrado", "Square crop"), crop ? "crop" : "contain")}
        </select></label>
        <label for="sticker-outline">${t("Contorno blanco", "White outline")}<select id="sticker-outline">
          ${[0, 4, 8].map(px => option(String(px), px ? `${px} px` : t("Sin contorno", "No outline"), String(outline))).join("")}
        </select></label>
      </div>
      ${crop ? `<div class="sticker-fields">
        <label for="sticker-x">${t("Posición horizontal (%)", "Horizontal position (%)")}<input id="sticker-x" type="number" min="0" max="100" required value="${horizontal}"></label>
        <label for="sticker-y">${t("Posición vertical (%)", "Vertical position (%)")}<input id="sticker-y" type="number" min="0" max="100" required value="${vertical}"></label>
      </div>` : ""}
      ${motion() ? `<div class="sticker-fields">
        <label for="sticker-start">${limits.duration ? t("Inicio (s)", "Start (s)") : t("Fotograma en (s)", "Frame at (s)")}<input id="sticker-start" type="number" min="0" max="86400" step="0.01" required value="${h(start)}"></label>
        ${limits.duration ? `<label for="sticker-length">${t("Duración (s)", "Duration (s)")}<input id="sticker-length" type="number" min="0.2" max="${limits.duration}" step="0.01" required value="${h(duration)}"></label>` : ""}
      </div>` : ""}
      <p class="field-note">512 × 512 px · ${t("hasta", "up to")} ${limits.limit} KB${limits.duration ? ` · ${limits.duration} s · ${t("sin audio", "silent")}` : ""}</p>
    </fieldset>`;
  }

  function sample() {
    if (!preview) return `<div class="sticker-empty"><span>${icon("image")}</span><p>${t("Prepara una muestra para ver el encuadre, el contorno y el peso real.", "Generate a preview to check framing, outline and actual file size.")}</p></div>`;
    const animatedWebp = preview.mime === "image/webp" && preview.durationMs !== null;
    const resultLabel = t("Vista previa del sticker exportado", "Exported sticker preview");
    const image = animatedWebp && !playAnimation ? posterUrl : previewUrl;
    return `<div class="asset-preview-image sticker-preview" id="sticker-preview-stage" data-surface="${surface}">
      ${preview.mime === "video/webm" ? `<video src="${h(previewUrl)}" controls muted loop preload="metadata" aria-label="${resultLabel}"></video>`
        : image ? `<img src="${h(image)}" alt="${resultLabel}">`
          : `<p>${t("No se pudo mostrar el primer fotograma.", "The first frame could not be displayed.")}</p>`}
    </div>
    <div class="sticker-preview-controls">
      <div class="sticker-surface-field">
        <label for="sticker-surface">${t("Inspeccionar sobre", "Inspect on")}</label>
        <select id="sticker-surface">
          ${option("grid", t("Cuadrícula", "Checkerboard"), surface)}
          ${option("light", t("Fondo claro", "Light background"), surface)}
          ${option("dark", t("Fondo oscuro", "Dark background"), surface)}
        </select>
      </div>
      ${animatedWebp ? action("sticker-play", playAnimation ? t("Detener animación", "Stop animation") : t("Reproducir animación", "Play animation"), playAnimation ? "pause" : "play", "secondary", 'id="sticker-play" aria-controls="sticker-preview-stage"') : ""}
    </div>
    <p class="sticker-result">${preview.width} × ${preview.height} px · ${number(preview.bytes / 1000, 1)} / ${number(preview.limit / 1000)} KB${preview.durationMs !== null ? ` · ${number(preview.durationMs / 1000, 2)} s` : ""}</p>`;
  }

  function render() {
    return `<section class="tool-section image-assets-panel sticker-tool">
      <header class="asset-panel-heading"><span>${icon("image")}</span><div><h2>Stickers</h2>
        <p>${t("Recorta una imagen o un clip y exporta un sticker con borde transparente.", "Crop an image or clip and export a sticker with transparent padding.")}</p></div></header>
      <form id="sticker-form">
        <div class="sticker-workspace">
          <div class="sticker-controls">${chooseInput()}${settings()}</div>
          <section class="sticker-review" aria-labelledby="sticker-review-title">
            <h3 id="sticker-review-title">${t("Muestra", "Preview")}</h3>${sample()}
            <div class="button-group">${action("sticker-preview", t("Generar muestra", "Generate preview"), "image", "secondary", !selected() || busy ? "disabled" : "")}
            ${requestId ? action("sticker-cancel", t("Cancelar muestra", "Cancel preview"), "close", "quiet") : ""}</div>
            ${busy ? `<p class="field-note" role="status">${requestId ? t("Preparando el sticker con el motor local…", "Preparing the sticker with the local engine…") : t("Añadiendo a la cola…", "Adding to the queue…")}</p>` : ""}
            ${error ? `<p class="asset-error" role="alert">${h(error)}</p>` : ""}
          </section>
        </div>
        <div class="asset-destination"><span>${t("Destino", "Destination")}<span class="path">${h(hooks.snapshot().settings.folder || t("Elige una carpeta", "Choose a folder"))}</span></span>
          ${action("choose-folder", t("Cambiar carpeta", "Change folder"), "folder", "quiet", disabled())}</div>
        <div class="asset-submit">
          <p class="field-note">${t("El sticker exportado aparecerá en Biblioteca.", "The exported sticker will appear in Library.")}</p>
          <button type="submit" class="button primary" ${!selected() || !preview || busy || sourceOpen ? "disabled" : ""}>${t("Exportar en la cola", "Queue export")}</button>
        </div>
        <details class="sticker-instructions"><summary>${t("Cómo usar el archivo exportado", "How to use the exported file")}</summary><p>${profile.startsWith("telegram")
          ? t("En Telegram abre @Stickers, elige un paquete compatible y adjunta el archivo generado.", "Open @Stickers in Telegram, choose a compatible pack and attach the generated file.")
          : t("Para añadirlo como sticker de WhatsApp, importa el WebP en una aplicación de paquetes compatible y añade el paquete desde el móvil.", "To add it as a WhatsApp sticker, import the WebP into a compatible pack app and add the pack from your phone.")}</p></details>
      </form>${imageExports(hooks.snapshot(), "sticker", t)}
    </section>`;
  }

  async function click(kind: string) {
    switch (kind) {
      case "sticker-source":
        sourceOpen = !sourceOpen;
        hooks.render();
        if (sourceOpen) document.getElementById("sticker-query")?.focus();
        return true;
      case "sticker-preview": {
        const form = document.getElementById("sticker-form");
        if (!selected() || !(form instanceof HTMLFormElement) || !form.reportValidity()) return true;
        await run(async () => {
          invalidate();
          requestId = Date.now() * 1000 + 703;
          hooks.render();
          const { data, ...details } = await hooks.command<Preview>("sticker_preview", { libraryId, options: options(), requestId });
          previewUrl = details.mime === "video/webm"
            ? URL.createObjectURL(new Blob([new Uint8Array(data)], { type: details.mime }))
            : webpUrl(data);
          if (details.mime === "image/webp" && details.durationMs !== null) {
            posterUrl = await firstFrame(data).catch(() => "");
          }
          preview = details;
        });
        return true;
      }
      case "sticker-cancel":
        if (requestId) await hooks.command("cancel_file_source_inspection", { requestId });
        return true;
      case "sticker-play":
        playAnimation = !playAnimation;
        hooks.render();
        document.getElementById("sticker-play")?.focus({ preventScroll: true });
        return true;
      default: return false;
    }
  }

  function input(target: HTMLElement) {
    if (!(target instanceof HTMLInputElement || target instanceof HTMLSelectElement)) return false;
    if (target.id === "sticker-query") { query = target.value; hooks.render(); return true; }
    if (target.id === "sticker-surface") {
      const value = target.value;
      if (value === "grid" || value === "light" || value === "dark") {
        surface = value;
        document.getElementById("sticker-preview-stage")?.setAttribute("data-surface", surface);
      }
      return true;
    }
    if (target.name === "sticker-input") {
      libraryId = target.value; start = "0"; sourceOpen = false;
      if (!motion() && profiles[profile].duration !== null) profile = profile === "telegramVideo" ? "telegramImage" : "whatsappImage";
    } else switch (target.id) {
      case "sticker-profile":
        profile = target.value as Profile;
        duration = String(Math.min(Number(duration) || 1.5, profiles[profile].duration ?? 10));
        break;
      case "sticker-fit": crop = target.value === "crop"; break;
      case "sticker-outline": outline = Number(target.value); break;
      case "sticker-x": horizontal = Number(target.value); break;
      case "sticker-y": vertical = Number(target.value); break;
      case "sticker-start": start = target.value; break;
      case "sticker-length": duration = target.value; break;
      default: return false;
    }
    invalidate();
    error = "";
    hooks.render();
    if (target.name === "sticker-input") document.getElementById("sticker-profile")?.focus({ preventScroll: true });
    return true;
  }

  async function submit(form: HTMLFormElement) {
    if (form.id !== "sticker-form") return false;
    if (preview && selected() && !sourceOpen) {
      const previewId = preview.previewId;
      await run(async () => {
        await hooks.command("queue_sticker", { libraryId, options: options(), previewId, clientRequest: crypto.randomUUID() });
        invalidate();
        await hooks.refresh();
        hooks.notify(t("Sticker añadido a Descargas.", "Sticker added to Downloads."));
      });
    }
    return true;
  }
  return { render, click, input, submit };
}
