import "./upscale.css";
import { escapeHtml as h, type Copy, type Snapshot } from "./contracts";
import { isCancelled } from "./presentation";
import { action, icon, option } from "./views-common";

type Model = "photo" | "illustration";
interface Options { model: Model; scale: number; tile: number }
interface Component { id: string; installed: boolean; enabled: boolean; available: boolean; size: number; installedSize?: number }
interface Preview {
  previewId: number;
  before: number[];
  after: number[];
  cropWidth: number;
  cropHeight: number;
  cropX: number;
  cropY: number;
  dimensions: { width: number; height: number; outputWidth: number; outputHeight: number };
  peakRgbaBytes: number;
}
interface Hooks {
  snapshot(): Snapshot;
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  render(): void;
  refresh(): Promise<void>;
  notify(message: string): void;
  components(): void;
  t: Copy;
}

const MAX_BATCH = 12;

function pngUrl(data: number[]) {
  let binary = "";
  for (let offset = 0; offset < data.length; offset += 16384) binary += String.fromCharCode(...data.slice(offset, offset + 16384));
  return "data:image/png;base64," + btoa(binary);
}

export function createUpscaleTool(hooks: Hooks) {
  const { t } = hooks;
  let component: Component | null = null;
  let ids: string[] = [], query = "", sampleId = "", picking = false;
  let settings: Options = { model: "photo", scale: 4, tile: 64 };
  let anchor = { x: 50, y: 50 };
  let preview: (Omit<Preview, "before" | "after"> & { before: string; after: string }) | null = null;
  let busy = false, requestId = 0, error = "";
  const ready = () => component?.installed && component.enabled && component.available;
  const disabled = () => busy ? "disabled" : "";
  const images = () => hooks.snapshot().library.filter(item => item.kind === "image" && !item.missing && /\.(png|jpe?g|webp)$/i.test(item.name));
  const selected = () => {
    const available = images();
    return ids.flatMap(id => { const item = available.find(image => image.id === id); return item ? [item] : []; });
  };
  const valid = () => Boolean(ids.length && ids.length === selected().length);
  const mib = (value: number) => `${new Intl.NumberFormat(t("es", "en"), { maximumFractionDigits: 1 }).format(value / 1048576)} MiB`;

  async function load() {
    try { component = (await hooks.command<Component[]>("optional_components")).find(entry => entry.id === "upscale") ?? null; }
    catch (cause) { error = cause instanceof Error ? cause.message : String(cause); }
    hooks.render();
  }

  function invalidate() { preview = null; error = ""; }

  async function run(task: () => Promise<void>) {
    if (busy) return;
    busy = true;
    error = "";
    hooks.render();
    try { await task(); }
    catch (cause) { if (!isCancelled(cause)) error = (cause instanceof Error ? cause.message : String(cause)).slice(0, 500); }
    finally { busy = false; requestId = 0; hooks.render(); }
  }

  function sources() {
    const choices = images().filter(item => item.name.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
    const current = selected();
    return `<section class="upscale-inputs" aria-labelledby="upscale-input-title">
      <div class="section-heading"><h3 id="upscale-input-title">${t("Entrada", "Input")}</h3><div class="button-group">
        ${action("import-files", t("Importar", "Import"), "add", "quiet", disabled())}
        ${action("upscale-picker", picking ? t("Cerrar lista", "Close list") : t("Elegir imágenes", "Choose images"), "folder", "secondary", `aria-expanded="${picking}" aria-controls="upscale-picker" ${disabled()}`)}
      </div></div>
      <p class="field-note">${current.length} / ${MAX_BATCH} ${t("imágenes seleccionadas", "images selected")} · PNG, JPEG, WebP</p>
      <div id="upscale-picker" ${picking ? "" : "hidden"}><label for="upscale-query">${t("Buscar en Biblioteca", "Find in Library")}
        <input id="upscale-query" type="search" value="${h(query)}" ${disabled()}>
      </label>${choices.length ? `<div class="library-picker asset-picker"><ul>${choices.slice(0, 200).map(item => `<li><label>
        <input type="checkbox" id="upscale-input-${h(item.id)}" data-upscale-id="${h(item.id)}" ${ids.includes(item.id) ? "checked" : ""} ${busy || !ids.includes(item.id) && ids.length >= MAX_BATCH ? "disabled" : ""}>
        <span title="${h(item.name)}">${h(item.name)}</span></label></li>`).join("")}</ul></div>`
        : `<p class="field-note">${t("Importa una imagen o cambia la búsqueda.", "Import an image or change the search.")}</p>`}
      ${choices.length > 200 ? `<p class="field-note">${t("Se muestran 200 imágenes. Acota la búsqueda.", "Showing 200 images. Narrow the search.")}</p>` : ""}</div>
      ${current.length ? `<label for="upscale-sample">${t("Imagen para la muestra", "Image for the preview")}<select id="upscale-sample" ${disabled()}>
        ${current.map(item => option(item.id, item.name, sampleId)).join("")}</select></label>` : ""}
    </section>`;
  }

  function options() {
    const scales = settings.model === "photo" ? [4] : [2, 3, 4];
    return `<fieldset class="asset-settings upscale-options" ${disabled()}><legend>${t("Modelo y memoria", "Model and memory")}</legend>
      <div class="upscale-fields"><label for="upscale-model">${t("Modelo", "Model")}<select id="upscale-model">
        ${option("photo", t("Fotografía · x4plus", "Photo · x4plus"), settings.model)}
        ${option("illustration", t("Ilustración · AnimeVideo v3", "Illustration · AnimeVideo v3"), settings.model)}
      </select></label><label for="upscale-scale">${t("Factor", "Scale")}<select id="upscale-scale">${scales.map(scale => option(String(scale), `×${scale}`, String(settings.scale))).join("")}</select></label>
      <label for="upscale-tile">${t("Tamaño del bloque", "Tile size")}<select id="upscale-tile">
        ${option("64", t("64 px · menor uso de VRAM", "64 px · lower VRAM use"), String(settings.tile))}
        ${option("128", "128 px", String(settings.tile))}
      </select></label></div>
      <p class="field-note">${preview
        ? t(`Hasta ${mib(preview.peakRgbaBytes)} en píxeles RGBA de entrada y salida por imagen del lote. La memoria adicional del modelo depende de la GPU.`,
          `Up to ${mib(preview.peakRgbaBytes)} of input and output RGBA pixels per image in this batch. Additional model memory depends on the GPU.`)
        : t("La muestra calcula la memoria de los píxeles del lote. La GPU procesa una imagen por turno y usa bloques para limitar la memoria.", "The preview calculates pixel memory for the batch. The GPU processes one image at a time and uses tiles to limit memory.")}</p>
    </fieldset>`;
  }

  function comparison() {
    if (!preview) return `<div class="upscale-empty">${t("Genera una muestra para comparar los detalles del mismo recorte.", "Generate a preview to compare details in the same crop.")}</div>`;
    const d = preview.dimensions;
    return `<div class="upscale-pair">
      <figure><figcaption>${t("Original · bicúbico", "Original · bicubic")}</figcaption>
        <div class="asset-preview-image"><img alt="${t("Recorte original ampliado para comparar", "Original crop enlarged for comparison")}" src="${preview.before}"></div></figure>
      <figure><figcaption>Real-ESRGAN ×${settings.scale}</figcaption>
        <div class="asset-preview-image"><img alt="${t("El mismo recorte procesado con Real-ESRGAN", "The same crop processed with Real-ESRGAN")}" src="${preview.after}"></div></figure>
    </div><p class="field-note">${t("Recorte", "Crop")} ${preview.cropWidth} × ${preview.cropHeight} px · x=${preview.cropX}, y=${preview.cropY} · ${t("Imagen completa", "Full image")} ${d.width} × ${d.height} → ${d.outputWidth} × ${d.outputHeight} px</p>`;
  }

  function render() {
    const current = selected();
    if (current.length !== ids.length && !busy) {
      ids = current.map(item => item.id);
      if (!ids.includes(sampleId)) sampleId = ids[0] ?? "";
      invalidate();
    }
    const status = ready() ? "" : `<div class="upscale-component"><div><strong>Real-ESRGAN · Vulkan</strong>
      <p class="field-note">${t("Instala y activa el componente local en Ajustes. Requiere una GPU compatible con Vulkan.", "Install and enable the local component in Settings. Requires a Vulkan-compatible GPU.")}</p>
      ${component?.installedSize ? `<small>${t("Descarga", "Download")} ${mib(component.size)} · ${t("En disco tras instalar", "On disk after installation")} ${mib(component.installedSize)}</small>` : ""}</div>
      ${action("upscale-component", t("Ver componentes", "View components"), "settings", "secondary")}</div>`;
    return `<section class="tool-section image-assets-panel upscale-tool">
      <header class="asset-panel-heading"><span>${icon("image")}</span><div><h2>${t("Ampliar imagen", "Upscale image")}</h2>
        <p>${t("Real-ESRGAN estima detalles y guarda una copia PNG ampliada.", "Real-ESRGAN estimates detail and saves an enlarged PNG copy.")}</p></div></header>
      ${status}<form id="upscale-form">${sources()}${options()}
        <section class="asset-review upscale-review" aria-labelledby="upscale-preview-title">
          <div class="section-heading"><h3 id="upscale-preview-title">${t("Muestra del recorte", "Crop preview")}</h3>
            <div class="button-group">${action("upscale-preview", t("Generar muestra", "Generate preview"), "image", "secondary", !ready() || !valid() || busy ? "disabled" : "")}
            ${requestId ? action("upscale-cancel", t("Cancelar muestra", "Cancel preview"), "close", "quiet") : ""}</div></div>
          ${comparison()}<div class="upscale-fields upscale-anchors">
            <label for="upscale-x">${t("Posición horizontal", "Horizontal position")} · ${anchor.x}%<input id="upscale-x" type="range" min="0" max="100" step="1" value="${anchor.x}" ${busy || !valid() ? "disabled" : ""}></label>
            <label for="upscale-y">${t("Posición vertical", "Vertical position")} · ${anchor.y}%<input id="upscale-y" type="range" min="0" max="100" step="1" value="${anchor.y}" ${busy || !valid() ? "disabled" : ""}></label>
          </div>
          ${busy ? `<p role="status" class="field-note">${requestId ? t("Procesando la muestra en este equipo…", "Processing the preview on this device…") : t("Añadiendo a la cola…", "Adding to the queue…")}</p>` : ""}
          ${error ? `<p role="alert" class="asset-error">${h(error)}</p>` : ""}
        </section>
        <div class="asset-destination"><span>${t("Destino", "Destination")}<span class="path">${h(hooks.snapshot().settings.folder)}</span></span>
          ${action("choose-folder", t("Cambiar carpeta", "Change folder"), "folder", "quiet", disabled())}</div>
        <div class="asset-submit"><p class="field-note">${t("Se amplían las imágenes completas. Las copias aparecen en Biblioteca.", "Whole images are enlarged. Copies appear in Library.")}</p>
          <button class="button primary" type="submit" ${!preview || busy || picking || !ready() || !valid() ? "disabled" : ""}>${t("Ampliar en la cola", "Queue upscale")}</button></div>
      </form></section>`;
  }

  async function click(kind: string) {
    switch (kind) {
      case "upscale-component": hooks.components(); return true;
      case "upscale-picker":
        picking = !picking; hooks.render();
        if (picking) document.getElementById("upscale-query")?.focus();
        return true;
      case "upscale-preview":
        if (ready() && valid()) await run(async () => {
          invalidate();
          requestId = Date.now() * 1000 + 704;
          hooks.render();
          const { before, after, ...details } = await hooks.command<Preview>("upscale_preview", { ids, sampleId, options: settings, anchor, requestId });
          preview = { ...details, before: pngUrl(before), after: pngUrl(after) };
        });
        return true;
      case "upscale-cancel":
        if (requestId) await hooks.command("cancel_file_source_inspection", { requestId });
        return true;
      default: return false;
    }
  }

  function input(target: HTMLElement) {
    if (!(target instanceof HTMLInputElement || target instanceof HTMLSelectElement)) return false;
    if (target.id === "upscale-query") { query = target.value; hooks.render(); return true; }
    if (target instanceof HTMLInputElement && target.dataset.upscaleId) {
      const id = target.dataset.upscaleId;
      if (target.checked && ids.length < MAX_BATCH) ids = [...ids, id];
      if (!target.checked) ids = ids.filter(value => value !== id);
      if (!ids.includes(sampleId)) sampleId = ids[0] ?? "";
    } else switch (target.id) {
      case "upscale-model":
        settings = { ...settings, model: target.value as Model, scale: target.value === "photo" ? 4 : settings.scale };
        break;
      case "upscale-scale": settings = { ...settings, scale: Number(target.value) }; break;
      case "upscale-tile": settings = { ...settings, tile: Number(target.value) }; break;
      case "upscale-sample": sampleId = target.value; break;
      case "upscale-x": anchor = { ...anchor, x: Number(target.value) }; break;
      case "upscale-y": anchor = { ...anchor, y: Number(target.value) }; break;
      default: return false;
    }
    invalidate(); hooks.render(); return true;
  }

  async function submit(form: HTMLFormElement) {
    if (form.id !== "upscale-form") return false;
    if (preview && ready() && valid() && !picking) {
      const previewId = preview.previewId;
      await run(async () => {
        await hooks.command("queue_upscale", { ids, options: settings, previewId, clientRequest: crypto.randomUUID() });
        invalidate();
        await hooks.refresh();
        hooks.notify(t("Lote de ampliación añadido a Descargas.", "Upscale batch added to Downloads."));
      });
    }
    return true;
  }

  return { render, load, click, input, submit };
}
