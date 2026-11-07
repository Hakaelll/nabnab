import "./background.css";
import { escapeHtml as h, type Copy, type Snapshot } from "./contracts";
import { isCancelled } from "./presentation";
import { action, icon, option } from "./views-common";

type Format = "png" | "webp";
type Surface = "grid" | "light" | "dark";
type Fill = { kind: "transparent" } | { kind: "solid"; rgb: number[] };
interface Component { id: string; installed: boolean; enabled: boolean; available: boolean; size: number }
interface Preview { previewId: number; dimensions: { width: number; height: number }; result: number[]; mask: number[] }
interface Hooks {
  snapshot(): Snapshot;
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  render(): void;
  refresh(): Promise<void>;
  notify(message: string): void;
  components(): void;
  t: Copy;
}

function pngUrl(data: number[]) {
  let binary = "";
  for (let offset = 0; offset < data.length; offset += 16384) binary += String.fromCharCode(...data.slice(offset, offset + 16384));
  return "data:image/png;base64," + btoa(binary);
}

export function createBackgroundTool(hooks: Hooks) {
  const { t } = hooks;
  let component: Component | null = null;
  let libraryId = "", query = "", picking = false;
  let edge = 0, softness = 1, format: Format = "png", transparent = true, color = "#f2f2f3";
  let surface: Surface = "grid";
  let preview: (Omit<Preview, "result" | "mask"> & { result: string; mask: string }) | null = null;
  let busy = false, requestId = 0, error = "";
  const images = () => hooks.snapshot().library.filter(item => item.kind === "image" && !item.missing && /\.(png|jpe?g|webp)$/i.test(item.name));
  const selected = () => images().find(item => item.id === libraryId);
  const ready = () => component?.installed && component.enabled && component.available;
  const disabled = () => busy ? "disabled" : "";

  async function load() {
    try { component = (await hooks.command<Component[]>("optional_components")).find(entry => entry.id === "background") ?? null; }
    catch (cause) { error = cause instanceof Error ? cause.message : String(cause); }
    hooks.render();
  }

  function options() {
    const fill: Fill = transparent ? { kind: "transparent" }
      : { kind: "solid", rgb: [1, 3, 5].map(offset => Number.parseInt(color.slice(offset, offset + 2), 16)) };
    return { format, fill, edge, softness };
  }

  async function run(task: () => Promise<void>) {
    if (busy) return;
    busy = true; error = ""; hooks.render();
    try { await task(); }
    catch (cause) { if (!isCancelled(cause)) error = (cause instanceof Error ? cause.message : String(cause)).slice(0, 500); }
    finally { busy = false; requestId = 0; hooks.render(); }
  }

  function sources() {
    const matches = images().filter(item => item.name.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
    return `<section class="background-inputs">
      <div class="section-heading"><h3>${t("Entrada", "Input")}</h3><div class="button-group">
        ${action("import-files", t("Importar", "Import"), "add", "quiet", disabled())}
        ${action("background-picker", picking ? t("Cerrar lista", "Close list") : t("Elegir imagen", "Choose image"), "folder", "secondary", `aria-expanded="${picking}" aria-controls="background-picker" ${disabled()}`)}
      </div></div><p class="background-source">${h(selected()?.name ?? t("Elige un PNG, JPEG o WebP de Biblioteca.", "Choose a PNG, JPEG or WebP from Library."))}</p>
      <div id="background-picker" ${picking ? "" : "hidden"}><label for="background-query">${t("Buscar en Biblioteca", "Find in Library")}
        <input id="background-query" type="search" value="${h(query)}" ${disabled()}>
      </label>${matches.length ? `<div class="library-picker asset-picker"><ul>${matches.slice(0, 200).map(item => `<li><label>
        <input type="radio" name="background-input" value="${h(item.id)}" ${item.id === libraryId ? "checked" : ""} ${disabled()}>
        <span title="${h(item.name)}">${h(item.name)}</span></label></li>`).join("")}</ul></div>`
        : `<p class="field-note">${t("Importa una imagen o cambia la búsqueda.", "Import an image or change the search.")}</p>`}
      ${matches.length > 200 ? `<p class="field-note">${t("Se muestran 200 imágenes. Acota la búsqueda.", "Showing 200 images. Narrow the search.")}</p>` : ""}</div>
    </section>`;
  }

  function settings() {
    return `<fieldset class="asset-settings background-settings" ${disabled()}><legend>${t("Bordes y salida", "Edges and output")}</legend>
      <div class="background-fields">
        <label for="background-edge">${t("Contraer / expandir", "Contract / expand")} · ${edge > 0 ? "+" : ""}${edge} px
          <input type="range" id="background-edge" min="-8" max="8" step="1" value="${edge}"></label>
        <label for="background-softness">${t("Suavizado", "Smoothing")} · ${softness}
          <input type="range" id="background-softness" min="0" max="4" step="1" value="${softness}"></label>
      </div>
      <div class="background-fields">
        <label for="background-fill">${t("Fondo de salida", "Output background")}<select id="background-fill">
          ${option("transparent", t("Transparente", "Transparent"), transparent ? "transparent" : "solid")}
          ${option("solid", t("Color sólido", "Solid color"), transparent ? "transparent" : "solid")}</select></label>
        ${transparent ? "" : `<label for="background-color">${t("Color", "Color")}<input id="background-color" type="color" value="${h(color)}"></label>`}
        <label for="background-format">${t("Formato", "Format")}<select id="background-format">
          ${option("png", "PNG", format)}${option("webp", t("WebP sin pérdida", "Lossless WebP"), format)}</select></label>
      </div>
      <p class="field-note">${t("Ajusta el borde si quedan halos o se pierden detalles finos del sujeto.", "Adjust the edge if a halo remains or fine subject detail is lost.")}</p>
    </fieldset>`;
  }

  function comparison() {
    if (!preview) return `<p class="background-empty">${t("Genera una muestra para revisar el recorte y su máscara.", "Generate a preview to review the cutout and its mask.")}</p>`;
    return `<div class="background-pair">
      <figure><figcaption>${t("Resultado", "Result")}</figcaption><div class="asset-preview-image" data-surface="${surface}">
        <img src="${preview.result}" alt="${t("Sujeto con el fondo de salida elegido", "Subject with the selected output background")}"></div></figure>
      <figure><figcaption>${t("Máscara", "Mask")}</figcaption><div class="asset-preview-image">
        <img src="${preview.mask}" alt="${t("Máscara de opacidad: blanco conserva, negro recorta", "Opacity mask: white keeps, black removes")}"></div></figure>
    </div>
    <div class="background-review-controls"><span class="field-note">${preview.dimensions.width} × ${preview.dimensions.height} px</span>
      ${transparent ? `<label for="background-surface">${t("Inspeccionar sobre", "Inspect on")}<select id="background-surface">
        ${option("grid", t("Cuadrícula", "Checkerboard"), surface)}${option("light", t("Claro", "Light"), surface)}${option("dark", t("Oscuro", "Dark"), surface)}
      </select></label>` : ""}</div>`;
  }

  function render() {
    const status = ready() ? "" : `<div class="upscale-component"><div><strong>U²-Net small · Windows ML</strong>
      <p class="field-note">${t("Instala y activa el modelo desde Ajustes. La segmentación se ejecuta en CPU.", "Install and enable the model in Settings. Segmentation runs on the CPU.")}</p>
      ${component ? `<small>${new Intl.NumberFormat(t("es", "en"), { maximumFractionDigits: 1 }).format(component.size / 1048576)} MiB · ${t("modelo local", "local model")}</small>` : ""}</div>
      ${action("background-component", t("Ver componentes", "View components"), "settings", "secondary")}</div>`;
    return `<section class="tool-section image-assets-panel background-tool">
      <header class="asset-panel-heading"><span>${icon("image")}</span><div><h2>${t("Eliminar fondo", "Remove background")}</h2>
        <p>${t("Separa el sujeto y revisa el borde antes de exportar una copia.", "Isolate the subject and review its edge before exporting a copy.")}</p></div></header>
      ${status}<form id="background-form">${sources()}${settings()}
        <section class="asset-review background-review" aria-labelledby="background-preview-title">
          <div class="section-heading"><h3 id="background-preview-title">${t("Muestra", "Preview")}</h3><div class="button-group">
            ${action("background-preview", t("Generar muestra", "Generate preview"), "image", "secondary", !ready() || !selected() || busy ? "disabled" : "")}
            ${requestId ? action("background-cancel", t("Cancelar muestra", "Cancel preview"), "close", "quiet") : ""}</div></div>
          ${comparison()}
          ${busy ? `<p role="status" class="field-note">${requestId ? t("Separando el sujeto en este equipo…", "Isolating the subject on this device…") : t("Añadiendo a la cola…", "Adding to the queue…")}</p>` : ""}
          ${error ? `<p role="alert" class="asset-error">${h(error)}</p>` : ""}
        </section>
        <div class="asset-destination"><span>${t("Destino", "Destination")}<span class="path">${h(hooks.snapshot().settings.folder)}</span></span>
          ${action("choose-folder", t("Cambiar carpeta", "Change folder"), "folder", "quiet", disabled())}</div>
        <div class="asset-submit"><p class="field-note">${t("La copia aparece en Biblioteca vinculada al original.", "The copy appears in Library linked to the original.")}</p>
          <button type="submit" class="button primary" ${!preview || busy || picking || !ready() || !selected() ? "disabled" : ""}>${t("Exportar en la cola", "Queue export")}</button></div>
      </form></section>`;
  }

  async function click(kind: string) {
    switch (kind) {
      case "background-component": hooks.components(); return true;
      case "background-picker":
        picking = !picking; hooks.render();
        if (picking) document.getElementById("background-query")?.focus();
        return true;
      case "background-preview":
        if (ready() && selected()) await run(async () => {
          preview = null; requestId = Date.now() * 1000 + 705; hooks.render();
          const { result, mask, ...details } = await hooks.command<Preview>("background_preview", { libraryId, options: options(), requestId });
          preview = { ...details, result: pngUrl(result), mask: pngUrl(mask) };
        });
        return true;
      case "background-cancel":
        if (requestId) await hooks.command("cancel_file_source_inspection", { requestId });
        return true;
      default: return false;
    }
  }

  function input(target: HTMLElement) {
    if (!(target instanceof HTMLInputElement || target instanceof HTMLSelectElement)) return false;
    if (target.id === "background-query") { query = target.value; hooks.render(); return true; }
    if (target.id === "background-surface") { surface = target.value as Surface; hooks.render(); return true; }
    if (target.name === "background-input") { libraryId = target.value; picking = false; }
    else switch (target.id) {
      case "background-edge": edge = Number(target.value); break;
      case "background-softness": softness = Number(target.value); break;
      case "background-fill": transparent = target.value === "transparent"; break;
      case "background-color": color = target.value; break;
      case "background-format": format = target.value as Format; break;
      default: return false;
    }
    preview = null; error = ""; hooks.render();
    if (target.name === "background-input") document.getElementById("background-edge")?.focus({ preventScroll: true });
    return true;
  }

  async function submit(form: HTMLFormElement) {
    if (form.id !== "background-form") return false;
    if (preview && ready() && selected() && !picking) {
      const previewId = preview.previewId;
      await run(async () => {
        await hooks.command("queue_background", { libraryId, options: options(), previewId, clientRequest: crypto.randomUUID() });
        preview = null; await hooks.refresh();
        hooks.notify(t("Recorte añadido a Descargas.", "Cutout added to Downloads."));
      });
    }
    return true;
  }

  return { render, load, click, input, submit };
}
