import "./upscale.css";
import { escapeHtml as h, type Copy, type LibraryItem, type Snapshot } from "./contracts";
import { imageExports } from "./image-exports";
import { isCancelled } from "./presentation";
import { action, icon, option } from "./views-common";

type Model = "photo" | "illustration";
interface Options { model: Model; scale: number; tile: number }
interface Component { id: string; installed: boolean; enabled: boolean; available: boolean; size: number; installedSize?: number }
interface ImageSize { width: number; height: number }
interface BatchImage extends ImageSize { id: string; outputWidth: number; outputHeight: number; rgbaBytes: number }
type Operation = "inspect" | "preview" | "queue";
interface Preview {
  previewId: number;
  before: number[];
  after: number[];
  cropWidth: number;
  cropHeight: number;
  cropX: number;
  cropY: number;
  batch: BatchImage[];
}
type Overview = { id: string; kind: "loading" | "unavailable" } | ({ id: string; kind: "ready"; url: string } & ImageSize);
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
const SAMPLE_SIDE = 160;

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
  let overview: Overview | null = null;
  let batch: BatchImage[] = [];
  let preview: (Omit<Preview, "before" | "after" | "batch"> & { before: string; after: string }) | null = null;
  let comparisonMode: "pair" | "wipe" = "pair";
  let split = 50;
  let busy: Operation | null = null;
  let error: { operation: Operation; message: string } | null = null;
  let requestId = 0;
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
    loadOverview(true);
    try { component = (await hooks.command<Component[]>("optional_components")).find(entry => entry.id === "upscale") ?? null; }
    catch (cause) { error = { operation: "preview", message: cause instanceof Error ? cause.message : String(cause) }; }
    hooks.render();
  }

  function invalidate() { preview = null; split = 50; error = null; }

  function loadOverview(refresh = false) {
    if (!sampleId) { overview = null; return; }
    if (overview?.id === sampleId && (!refresh || overview.kind === "loading")) return;
    const pending: Overview = { id: sampleId, kind: "loading" };
    overview = pending;
    const requestId = Date.now() * 1000 + 714;
    void hooks.command<{ images: ImageSize[] }>("image_assets_inspect", { ids: [pending.id], stitch: false, requestId }).then(async ({ images }) => {
      if (overview !== pending) return;
      const size = images[0];
      if (!size) throw new Error("Missing image dimensions.");
      const url = await hooks.command<string>("thumbnail", { id: pending.id, stored: true });
      if (overview !== pending) return;
      overview = { id: pending.id, kind: "ready", url, ...size };
      hooks.render();
    }).catch(() => {
      if (overview !== pending) return;
      overview = { id: pending.id, kind: "unavailable" };
      hooks.render();
    });
  }

  function cropSelector() {
    const sample = selected().find(item => item.id === sampleId);
    const map = overview?.id === sampleId ? overview : null;
    const size = map?.kind === "ready" ? map : sample;
    const width = size?.width ?? 0, height = size?.height ?? 0;
    const frameWidth = Math.min(width, SAMPLE_SIDE), frameHeight = Math.min(height, SAMPLE_SIDE);
    const x = Math.floor((width - frameWidth) * anchor.x / 100), y = Math.floor((height - frameHeight) * anchor.y / 100);
    const controlsDisabled = busy || !valid();
    const horizontal = t("Posición horizontal", "Horizontal position"), vertical = t("Posición vertical", "Vertical position");
    const canPlace = width > SAMPLE_SIDE || height > SAMPLE_SIDE;
    return `<div class="upscale-crop-selector">
      ${map?.kind === "ready" && width && height ? `<div class="upscale-crop-overview">
        <button type="button" id="upscale-crop-map" data-action="upscale-place-crop" class="upscale-crop-map"
          aria-label="${t("Colocar recorte en la imagen", "Place crop on the image")}" aria-describedby="upscale-crop-help"
          ${controlsDisabled || !canPlace ? "disabled" : ""}
          style="--overview-ratio:${width / height};--crop-left:${100 * x / width}%;--crop-top:${100 * y / height}%;--crop-width:${100 * frameWidth / width}%;--crop-height:${100 * frameHeight / height}%">
          <img src="${h(map.url)}" alt="" decoding="async"><span class="upscale-crop-window" aria-hidden="true"></span>
        </button>
        <p id="upscale-crop-help" class="field-note">${canPlace
          ? t("Haz clic para situar el recorte. Intro lo centra.", "Click to place the crop. Enter centers it.")
          : t("La muestra cubre toda la imagen.", "The preview covers the whole image.")}</p>
      </div>` : ""}
      <div class="upscale-fields upscale-anchors">
        <label for="upscale-x">${horizontal} · ${anchor.x}%<input id="upscale-x" aria-label="${horizontal}" type="range" min="0" max="100" step="1" value="${anchor.x}"
          ${controlsDisabled || width && width <= SAMPLE_SIDE ? "disabled" : ""}></label>
        <label for="upscale-y">${vertical} · ${anchor.y}%<input id="upscale-y" aria-label="${vertical}" type="range" min="0" max="100" step="1" value="${anchor.y}"
          ${controlsDisabled || height && height <= SAMPLE_SIDE ? "disabled" : ""}></label>
        ${width && height ? `<p role="status" class="field-note upscale-crop-coordinates">${frameWidth} × ${frameHeight} px · x=${x}, y=${y}</p>` : ""}
        ${map?.kind === "unavailable" ? `<p class="field-note">${t("Miniatura no disponible. Usa los controles de posición.", "Thumbnail unavailable. Use the position controls.")}</p>` : ""}
      </div>
    </div>`;
  }

  async function run(operation: Operation, task: () => Promise<void>) {
    if (busy) return;
    busy = operation;
    error = null;
    hooks.render();
    try { await task(); }
    catch (cause) {
      if (!isCancelled(cause)) error = { operation, message: (cause instanceof Error ? cause.message : String(cause)).slice(0, 500) };
    }
    finally { busy = null; requestId = 0; hooks.render(); }
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
      <p class="field-note">${t("La GPU procesa una imagen por turno. Usa bloques de 64 px para reducir el uso de memoria.", "The GPU processes one image at a time. Use 64 px tiles to reduce memory use.")}</p>
    </fieldset>`;
  }

  function batchReview(current: readonly LibraryItem[]) {
    const peak = Math.max(0, ...batch.map(image => image.rgbaBytes));
    return `<section class="upscale-batch" aria-labelledby="upscale-batch-title">
      <div class="section-heading"><h3 id="upscale-batch-title" tabindex="-1">${t("Dimensiones del lote", "Batch dimensions")}</h3>
        <div class="button-group">
          ${action("upscale-inspect", t("Revisar dimensiones", "Review dimensions"), "list", "secondary", !valid() || busy ? "disabled" : "")}
          ${busy === "inspect" && requestId ? action("upscale-cancel", t("Cancelar revisión", "Cancel review"), "close", "quiet") : ""}
        </div></div>
      ${batch.length ? `<ul aria-label="${t("Imágenes del lote", "Batch images")}">${batch.map(image => `<li>
        <strong>${h(current.find(item => item.id === image.id)?.name ?? "")}</strong>
        <span>${image.width} × ${image.height} → <b>${image.outputWidth} × ${image.outputHeight} px</b></span>
        <small>${mib(image.rgbaBytes)}</small></li>`).join("")}</ul>
        <p class="field-note">${t(`Pico de memoria de píxeles · ${mib(peak)} por imagen`, `Peak pixel memory · ${mib(peak)} per image`)}</p>
        <p class="field-note">${t("Estimación de píxeles RGBA de entrada y salida. El modelo y el controlador usan memoria adicional.", "Estimate for input and output RGBA pixels. The model and driver use extra memory.")}</p>`
        : busy !== "inspect" && error?.operation !== "inspect" ? `<p class="field-note">${t("Comprueba el tamaño de salida y la memoria de píxeles antes de generar la muestra.", "Check output dimensions and pixel memory before generating the preview.")}</p>` : ""}
      ${busy === "inspect" ? `<p role="status" class="field-note">${t("Revisando las imágenes…", "Reviewing images…")}</p>` : ""}
      ${error?.operation === "inspect" ? `<p role="alert" class="asset-error" id="upscale-batch-error" tabindex="-1">${h(error.message)}</p>` : ""}
    </section>`;
  }

  function comparison() {
    const d = batch.find(image => image.id === sampleId);
    if (!preview || !d) return `<div class="upscale-empty">${t("Genera una muestra para comparar los detalles del mismo recorte.", "Generate a preview to compare details in the same crop.")}</div>`;
    const original = t("Original · bicúbico", "Original · bicubic");
    const processed = `Real-ESRGAN ×${settings.scale}`;
    const before = `<img alt="${t("Recorte original ampliado para comparar", "Original crop enlarged for comparison")}" src="${preview.before}">`;
    const after = `<img alt="${t("El mismo recorte procesado con Real-ESRGAN", "The same crop processed with Real-ESRGAN")}" src="${preview.after}">`;
    return `<div class="upscale-comparison-modes" role="group" aria-label="${t("Vista de comparación", "Comparison view")}">
      ${action("upscale-compare-pair", t("Lado a lado", "Side by side"), undefined, "quiet", `aria-pressed="${comparisonMode === "pair"}" ${disabled()}`)}
      ${action("upscale-compare-wipe", t("Deslizador", "Slider"), undefined, "quiet", `aria-pressed="${comparisonMode === "wipe"}" ${disabled()}`)}
    </div>${comparisonMode === "pair" ? `<div class="upscale-pair">
      <figure><figcaption>${original}</figcaption><div class="asset-preview-image">${before}</div></figure>
      <figure><figcaption>${processed}</figcaption><div class="asset-preview-image">${after}</div></figure>
    </div>` : `<figure class="upscale-wipe" style="--split:${split}%;--crop-ratio:${preview.cropWidth / preview.cropHeight}">
      <figcaption><span>${original}</span><span>${processed}</span></figcaption>
      <div class="asset-preview-image upscale-wipe-frame">${before}${after}<span class="upscale-wipe-divider" aria-hidden="true"></span></div>
      <label for="upscale-split">${t("Original visible", "Visible original")} <span class="upscale-split-value" aria-hidden="true">${split}%</span></label>
      <input id="upscale-split" type="range" min="0" max="100" value="${split}" aria-valuetext="${t(`${split}% de original`, `${split}% original`)}" ${disabled()}>
    </figure>`}<p class="field-note">${t("Recorte", "Crop")} ${preview.cropWidth} × ${preview.cropHeight} px · x=${preview.cropX}, y=${preview.cropY} · ${t("Imagen completa", "Full image")} ${d.width} × ${d.height} → ${d.outputWidth} × ${d.outputHeight} px</p>`;
  }

  function render() {
    const current = selected();
    if (current.length !== ids.length && !busy) {
      ids = current.map(item => item.id);
      batch = [];
      if (!ids.includes(sampleId)) sampleId = ids[0] ?? "";
      loadOverview();
      invalidate();
    }
    const status = ready() ? "" : `<div class="upscale-component"><div><strong>Real-ESRGAN · Vulkan</strong>
      <p class="field-note">${t("Instala y activa el componente local en Ajustes. Requiere una GPU compatible con Vulkan.", "Install and enable the local component in Settings. Requires a Vulkan-compatible GPU.")}</p>
      ${component?.installedSize ? `<small>${t("Descarga", "Download")} ${mib(component.size)} · ${t("En disco tras instalar", "On disk after installation")} ${mib(component.installedSize)}</small>` : ""}</div>
      ${action("upscale-component", t("Ver componentes", "View components"), "settings", "secondary")}</div>`;
    return `<section class="tool-section image-assets-panel upscale-tool">
      <header class="asset-panel-heading"><span>${icon("image")}</span><div><h2>${t("Ampliar imagen", "Upscale image")}</h2>
        <p>${t("Real-ESRGAN estima detalles y guarda una copia PNG ampliada.", "Real-ESRGAN estimates detail and saves an enlarged PNG copy.")}</p></div></header>
      ${status}<form id="upscale-form">${sources()}${options()}${ids.length ? batchReview(current) : ""}
        <section class="asset-review upscale-review" aria-labelledby="upscale-preview-title">
          <div class="section-heading"><h3 id="upscale-preview-title">${t("Muestra del recorte", "Crop preview")}</h3>
            <div class="button-group">${action("upscale-preview", t("Generar muestra", "Generate preview"), "image", "secondary", !ready() || !valid() || busy ? "disabled" : "")}
            ${busy === "preview" && requestId ? action("upscale-cancel", t("Cancelar muestra", "Cancel preview"), "close", "quiet") : ""}</div></div>
          ${ids.length ? cropSelector() : ""}${comparison()}
          ${busy && busy !== "inspect" ? `<p role="status" class="field-note">${busy === "preview" ? t("Procesando la muestra en este equipo…", "Processing the preview on this device…") : t("Añadiendo a la cola…", "Adding to the queue…")}</p>` : ""}
          ${error && error.operation !== "inspect" ? `<p role="alert" class="asset-error">${h(error.message)}</p>` : ""}
        </section>
        <div class="asset-destination"><span>${t("Destino", "Destination")}<span class="path">${h(hooks.snapshot().settings.folder)}</span></span>
          ${action("choose-folder", t("Cambiar carpeta", "Change folder"), "folder", "quiet", disabled())}</div>
        <div class="asset-submit"><p class="field-note">${t("Se amplían las imágenes completas. Las copias aparecen en Biblioteca.", "Whole images are enlarged. Copies appear in Library.")}</p>
          <button class="button primary" type="submit" ${!preview || busy || picking || !ready() || !valid() ? "disabled" : ""}>${t("Ampliar en la cola", "Queue upscale")}</button></div>
      </form>${imageExports(hooks.snapshot(), "upscale", t)}</section>`;
  }

  async function click(kind: string, event?: MouseEvent) {
    switch (kind) {
      case "upscale-component": hooks.components(); return true;
      case "upscale-place-crop": {
        const map = document.getElementById("upscale-crop-map");
        if (busy || !map || overview?.kind !== "ready" || overview.id !== sampleId) return true;
        const { width, height } = overview;
        const bounds = map.getBoundingClientRect();
        const position = (fraction: number, pixels: number) => pixels <= SAMPLE_SIDE ? 50
          : Math.round(Math.min(1, Math.max(0, (fraction * pixels - SAMPLE_SIDE / 2) / (pixels - SAMPLE_SIDE))) * 100);
        anchor = event?.detail ? {
          x: position((event.clientX - bounds.left) / bounds.width, width),
          y: position((event.clientY - bounds.top) / bounds.height, height),
        } : { x: 50, y: 50 };
        invalidate(); hooks.render();
        document.getElementById("upscale-crop-map")?.focus({ preventScroll: true });
        return true;
      }
      case "upscale-compare-pair":
      case "upscale-compare-wipe":
        comparisonMode = kind === "upscale-compare-pair" ? "pair" : "wipe";
        hooks.render();
        return true;
      case "upscale-picker":
        picking = !picking; hooks.render();
        if (picking) document.getElementById("upscale-query")?.focus();
        return true;
      case "upscale-inspect":
        if (!valid() || busy) return true;
        await run("inspect", async () => {
          invalidate();
          batch = [];
          requestId = Date.now() * 1000 + 705;
          hooks.render();
          batch = await hooks.command<BatchImage[]>("upscale_inspect", { ids, options: settings, requestId });
        });
        (document.getElementById("upscale-batch-error") ?? document.getElementById("upscale-batch-title"))?.focus({ preventScroll: true });
        return true;
      case "upscale-preview":
        if (ready() && valid()) await run("preview", async () => {
          invalidate();
          requestId = Date.now() * 1000 + 704;
          hooks.render();
          const { before, after, batch: dimensions, ...details } = await hooks.command<Preview>("upscale_preview", { ids, sampleId, options: settings, anchor, requestId });
          batch = dimensions;
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
    if (target instanceof HTMLInputElement && target.id === "upscale-split") {
      split = target.valueAsNumber;
      const wipe = target.closest<HTMLElement>(".upscale-wipe");
      wipe?.style.setProperty("--split", `${split}%`);
      const value = wipe?.querySelector(".upscale-split-value");
      if (value) value.textContent = `${split}%`;
      target.setAttribute("aria-valuetext", t(`${split}% de original`, `${split}% original`));
      return true;
    }
    if (target instanceof HTMLInputElement && target.dataset.upscaleId) {
      const id = target.dataset.upscaleId;
      if (target.checked && ids.length < MAX_BATCH) ids = [...ids, id];
      if (!target.checked) ids = ids.filter(value => value !== id);
      batch = [];
      if (!ids.includes(sampleId)) sampleId = ids[0] ?? "";
    } else switch (target.id) {
      case "upscale-model":
        settings = { ...settings, model: target.value as Model, scale: target.value === "photo" ? 4 : settings.scale };
        batch = [];
        break;
      case "upscale-scale": settings = { ...settings, scale: Number(target.value) }; batch = []; break;
      case "upscale-tile": settings = { ...settings, tile: Number(target.value) }; break;
      case "upscale-sample": sampleId = target.value; break;
      case "upscale-x": anchor = { ...anchor, x: Number(target.value) }; break;
      case "upscale-y": anchor = { ...anchor, y: Number(target.value) }; break;
      default: return false;
    }
    loadOverview();
    invalidate(); hooks.render(); return true;
  }

  async function submit(form: HTMLFormElement) {
    if (form.id !== "upscale-form") return false;
    if (preview && ready() && valid() && !picking) {
      const previewId = preview.previewId;
      await run("queue", async () => {
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
