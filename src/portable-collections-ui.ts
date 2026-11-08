import "./portable-collections.css";
import { escapeHtml as h, type Copy, type Job, type Snapshot } from "./contracts";
import { action, icon, stateLabel } from "./views-common";

export interface PortableOptions { metadata: boolean; notes: boolean; ocr: boolean }
interface Entry { id: string; name: string; size: number; sha256: string; duplicateOf: string | null; article: boolean; ocr: boolean }
interface Preview { entries: Entry[]; bytes: number; uniqueBytes: number; duplicateBytes: number; warnings: string[] }
interface ImportPreview extends Preview { planId: string; expiresAt: number; target: string }
interface Hooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  snapshot(): Snapshot; refresh(): Promise<void>; render(): void; t: Copy;
  notify(message: string, kind?: "info" | "success" | "error"): void;
}
function bytes(value: number): string { if (value < 1024) return `${value} B`; if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KiB`; if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MiB`; return `${(value / 1024 ** 3).toFixed(2)} GiB`; }
function message(error: unknown): string { if (typeof error === "string") return error; if (error instanceof Error) return error.message; if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") return error.message; return String(error); }

export function createPortableCollections(hooks: Hooks) {
  const { t } = hooks;
  let mode: "export" | "import" = "export";
  let options: PortableOptions = { metadata: true, notes: true, ocr: false };
  const selected = new Set<string>();
  let query = "", collection = "", source = "", error = "";
  let exportPreview: Preview | null = null;
  let exportJobId: string | null = null;
  let importPreview: ImportPreview | null = null;
  let busy: "preview" | "queue" | "choose" | null = null;
  let counter = Date.now(), current = 0;
  function invalidate() { exportPreview = null; error = ""; }
  async function perform(kind: NonNullable<typeof busy>, operation: (requestId: number) => Promise<void>) {
    if (busy) return; const requestId = ++counter; current = requestId; busy = kind; error = ""; hooks.render();
    try { await operation(requestId); } catch (failure) { if (requestId === current) error = message(failure); }
    finally { if (requestId === current) { busy = null; hooks.render(); } }
  }
  function renderPreview(preview: Preview, importing: boolean): string {
    return `<section class="portable-preview" aria-label="${t("Revisión de la colección", "Collection review")}"><div class="portable-summary"><div><span>${t("Archivos revisados", "Reviewed files")}</span><strong>${preview.entries.length}</strong></div><div><span>${t("Contenido seleccionado", "Selected content")}</span><strong>${bytes(preview.bytes)}</strong></div><div><span>${importing ? t("Datos nuevos estimados", "Estimated new data") : t("Archivos únicos", "Unique files")}</span><strong>${bytes(preview.uniqueBytes)}</strong></div>${preview.duplicateBytes ? `<div><span>${t("Contenido repetido", "Repeated content")}</span><strong>${bytes(preview.duplicateBytes)}</strong></div>` : ""}</div><p class="field-note">${t("Los tamaños cuentan el contenido de los archivos. El manifiesto y los índices añaden espacio; los artículos importados se regeneran desde su captura local.", "Sizes count file content. The manifest and indexes add space; imported articles are regenerated from their local capture.")}</p><div class="portable-reviewed-files" tabindex="0"><table><thead><tr><th>${t("Archivo", "File")}</th><th>${t("Tamaño", "Size")}</th><th>${t("Resultado de la revisión", "Review result")}</th></tr></thead><tbody>${preview.entries.map(entry => `<tr><td><strong>${h(entry.name)}</strong>${entry.article || entry.ocr ? `<small>${[entry.article ? t("Artículo con versión local", "Article with local version") : "", entry.ocr ? t("Índice OCR", "OCR index") : ""].filter(Boolean).join(" · ")}</small>` : ""}<small>SHA-256 · ${h(entry.sha256.slice(0, 16))}…</small></td><td>${bytes(entry.size)}</td><td>${entry.duplicateOf ? `${t("Contenido idéntico a", "Identical content to")} ${h(entry.duplicateOf)}` : importing && entry.article ? t("Copia nueva; notas de su propia versión", "New copy; notes for its own version") : importing ? t("Añadir a la biblioteca", "Add to library") : t("Incluir en el ZIP", "Include in ZIP")}</td></tr>`).join("")}</tbody></table></div>${preview.warnings.length ? `<ul class="portable-warnings">${preview.warnings.map(warning => `<li>${h(warning)}</li>`).join("")}</ul>` : ""}</section>`;
  }
  function renderExportResult(snapshot: Snapshot): string {
    const job = snapshot.jobs.find(job => job.id === exportJobId && job.source.kind === "portableExport");
    if (!job) return "";
    const zip = job.state === "completed" ? snapshot.library.find(item =>
      !item.missing && item.kind === "file" && /\.zip$/i.test(item.path) && job.outputs.includes(item.path)) : undefined;
    const status = zip ? t("ZIP disponible para revisar", "ZIP ready for review")
      : job.state === "completed" ? t("El ZIP no está disponible en Biblioteca", "The ZIP is unavailable in Library")
      : stateLabel(job.state, t);
    return `<section class="portable-confirm portable-export-result" aria-labelledby="portable-export-heading">
      <div><h3 id="portable-export-heading" tabindex="-1">${t("Último ZIP de esta sesión", "Latest ZIP this session")}</h3>
      <span role="status">${status}</span>${zip ? `<strong>${h(zip.name)} · ${bytes(zip.size)}</strong>`
        : job.state === "error" && job.error ? `<small>${h(job.error.message)}</small>` : ""}</div>
      ${zip ? action("share-file", t("Revisar envío del ZIP", "Review ZIP share"), "arrow", "primary", `id="portable-share-zip" data-id="${h(zip.id)}"`)
        : action("inspect-job", t("Ver en Descargas", "View in Downloads"), "downloads", "secondary", `data-id="${h(job.id)}"`)}
    </section>`;
  }
  function render(): string {
    const snapshot = hooks.snapshot();
    for (const id of selected) if (!snapshot.library.some(item => item.id === id && !item.missing)) { selected.delete(id); exportPreview = null; }
    const visible = snapshot.library.filter(item => !item.missing && (!collection || item.collections.includes(collection)) && `${item.name} ${item.tags.join(" ")}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
    return `<section class="portable-workspace tool-section" aria-labelledby="portable-title"><div class="portable-heading"><div><h2 id="portable-title">${t("Colección portable", "Portable collection")}</h2><p>${t("Comparte archivos elegidos con sus referencias y recupera una colección en otro equipo.", "Share selected files with their references and restore a collection on another computer.")}</p></div><span class="portable-format">ZIP · SHA-256</span></div><nav class="portable-tabs" aria-label="${t("Operación de colección", "Collection operation")}">${["export", "import"].map(value => `<button type="button" data-action="portable-mode" data-mode="${value}" ${mode === value ? 'aria-current="page"' : ""} ${busy ? "disabled" : ""}>${value === "export" ? t("Exportar selección", "Export selection") : t("Importar colección", "Import collection")}</button>`).join("")}</nav>${mode === "export" ? renderExportResult(snapshot) : ""}${error ? `<p class="portable-error" role="alert">${h(error)}</p>` : ""}${busy === "preview" ? `<div class="portable-progress" role="status"><span>${t("Revisando archivos y comprobando contenido…", "Reviewing files and verifying their content…")}</span>${action("portable-cancel", t("Cancelar revisión", "Cancel review"), "close", "quiet")}</div>` : ""}
      ${mode === "export" ? `<form id="portable-export-form"><div class="portable-export-layout"><div><div class="portable-filters"><label class="search-control">${icon("search")}<input id="portable-query" type="search" value="${h(query)}" placeholder="${t("Filtrar archivos de la biblioteca", "Filter library files")}" aria-label="${t("Filtrar archivos de la biblioteca", "Filter library files")}" ${busy ? "disabled" : ""}></label><label>${t("Colección", "Collection")}<select id="portable-collection" ${busy ? "disabled" : ""}><option value="">${t("Todas", "All")}</option>${snapshot.collections.map(value => `<option value="${h(value)}" ${collection === value ? "selected" : ""}>${h(value)}</option>`).join("")}</select></label></div><div class="portable-selection-heading"><span>${selected.size} ${t("seleccionados", "selected")}</span>${action("portable-clear", t("Quitar selección", "Clear selection"), undefined, "quiet", busy || !selected.size ? "disabled" : "")}</div><div class="portable-file-picker" tabindex="0" role="group" aria-label="${t("Archivos elegidos de Biblioteca", "Selected library files")}">${visible.slice(0, 2000).map(item => `<label><input type="checkbox" id="portable-item-${h(item.id)}" data-portable-item="${h(item.id)}" ${selected.has(item.id) ? "checked" : ""} ${busy || selected.size >= 2000 && !selected.has(item.id) ? "disabled" : ""}><span><strong>${h(item.name)}</strong><small>${h(item.collections.join(" · "))}</small></span><small>${bytes(item.size)}</small></label>`).join("") || `<p class="field-note">${t("No hay archivos disponibles con este filtro.", "No available files match this filter.")}</p>`}</div>${visible.length > 2000 ? `<p class="field-note">${t("Se muestran 2000 resultados. Acota la colección o la búsqueda.", "Showing 2000 results. Narrow the collection or search.")}</p>` : ""}</div><fieldset class="portable-options"><legend>${t("Contenido del paquete", "Package contents")}</legend><p class="field-note">${t("Los archivos seleccionados siempre se incluyen. Los originales se conservan.", "Selected files are always included. Originals are kept.")}</p><label><input id="portable-metadata" type="checkbox" ${options.metadata ? "checked" : ""} ${busy ? "disabled" : ""}>${t("Colecciones, etiquetas y metadatos", "Collections, tags and metadata")}</label><label><input id="portable-notes" type="checkbox" ${options.notes ? "checked" : ""} ${busy ? "disabled" : ""}>${t("Notas, resaltados y marcadores", "Notes, highlights and bookmarks")}</label><label><input id="portable-ocr" type="checkbox" ${options.ocr ? "checked" : ""} ${busy ? "disabled" : ""}>${t("Incluir texto e índice OCR", "Include OCR text and index")}</label><p class="field-note">${t("Un artículo incluye su captura validada para poder leerse offline. El índice OCR puede contener texto privado del documento.", "An article includes its validated capture for offline reading. The OCR index may contain private document text.")}</p></fieldset></div><div class="portable-action-row"><span class="field-note">${t("Hasta 2000 elementos y 128 GiB de contenido. El ZIP conserva referencias relativas.", "Up to 2000 items and 128 GiB of content. The ZIP keeps relative references.")}</span><button type="submit" class="button secondary" ${busy || !selected.size ? "disabled" : ""}>${t("Revisar tamaño y duplicados", "Review size and duplicates")}</button></div></form>${exportPreview ? renderPreview(exportPreview, false) + `<div class="portable-confirm"><div><span>${t("Destino del ZIP", "ZIP destination")}</span><strong>${h(snapshot.settings.folder)}</strong></div>${action("portable-export-queue", t("Crear ZIP en Descargas", "Create ZIP in Downloads"), "downloads", "primary", busy ? "disabled" : "")}</div>` : ""}` : `<div class="portable-import-source"><h3>${t("Elige un ZIP portable o su carpeta extraída", "Choose a portable ZIP or its extracted folder")}</h3><p class="field-note">${t("La revisión comprueba el manifiesto, los hashes y los límites de expansión antes de añadir nada a Biblioteca.", "Review verifies the manifest, hashes and expansion limits before adding anything to Library.")}</p><div class="button-group">${action("portable-choose-zip", t("Elegir ZIP", "Choose ZIP"), "file", "secondary", busy ? "disabled" : "")}${action("portable-choose-folder", t("Elegir carpeta portable", "Choose portable folder"), "folder", "quiet", busy ? "disabled" : "")}</div>${source ? `<p class="portable-source-path">${h(source)}</p>${action("portable-import-preview", t("Revisar colección", "Review collection"), "search", "secondary", busy ? "disabled" : "")}` : ""}</div>${importPreview ? renderPreview(importPreview, true) + `<div class="portable-confirm"><div><span>${t("Destino autorizado", "Granted destination")}</span><strong>${h(importPreview.target)}</strong><small>${t("Se crea una carpeta nueva. Los archivos existentes no se sobrescriben.", "A new folder is created. Existing files are not overwritten.")}</small></div>${action("portable-import-queue", t("Importar elementos revisados", "Import reviewed items"), "add", "primary", busy ? "disabled" : "")}</div><p class="field-note">${t("La revisión caduca en 24 horas. Si cambia el origen o el destino, debes revisarla de nuevo.", "Review expires in 24 hours. If the source or destination changes, review it again.")}</p>` : ""}`}
      <p class="portable-boundary">${t("Se empaqueta esta selección. Las sesiones, cookies y tokens de nabnab quedan fuera; revisa también los propios archivos antes de compartirlos.", "This selection is packaged. nabnab sessions, cookies and tokens are excluded; also review the files themselves before sharing.")}</p></section>`;
  }
  function change(element: HTMLElement): boolean {
    if (element instanceof HTMLInputElement) {
      if (element.dataset.portableItem) { if (element.checked && selected.size < 2000) selected.add(element.dataset.portableItem); else selected.delete(element.dataset.portableItem); invalidate(); hooks.render(); return true; }
      if (element.id === "portable-query") { query = element.value; hooks.render(); return true; }
      if (element.id === "portable-metadata" || element.id === "portable-notes" || element.id === "portable-ocr") { options = { ...options, [element.id === "portable-metadata" ? "metadata" : element.id === "portable-notes" ? "notes" : "ocr"]: element.checked }; invalidate(); hooks.render(); return true; }
    }
    if (element instanceof HTMLSelectElement && element.id === "portable-collection") { collection = element.value; hooks.render(); return true; }
    return false;
  }
  async function submit(form: HTMLFormElement): Promise<boolean> {
    if (form.id !== "portable-export-form") return false;
    if (!busy) exportPreview = null;
    if (selected.size) await perform("preview", async requestId => { const preview = await hooks.command<Preview>("portable_preview_export", { ids: [...selected], options, requestId }); if (requestId === current) exportPreview = preview; });
    return true;
  }
  async function click(name: string, element: HTMLButtonElement): Promise<boolean> {
    switch (name) {
      case "portable-mode": if (!busy && (element.dataset.mode === "export" || element.dataset.mode === "import")) { mode = element.dataset.mode; error = ""; hooks.render(); } break;
      case "portable-clear": if (!busy) { selected.clear(); invalidate(); hooks.render(); } break;
      case "portable-cancel": if (busy === "preview") { const requestId = current; current = ++counter; busy = null; await hooks.command("cancel_portable_preview", { requestId }); hooks.render(); } break;
      case "portable-export-queue": {
        if (busy || !exportPreview || !selected.size) break;
        let queued = false;
        await perform("queue", async () => {
          const [job] = await hooks.command<Job[]>("queue_portable_export", { ids: [...selected], options, clientRequest: crypto.randomUUID() });
          if (!job) throw new Error(t("La cola no devolvió la tarea del ZIP.", "The queue did not return the ZIP job."));
          exportJobId = job.id;
          exportPreview = null;
          await hooks.refresh();
          queued = true;
        });
        if (queued) document.getElementById("portable-export-heading")?.focus();
        break;
      }
      case "portable-choose-zip": case "portable-choose-folder": await perform("choose", async () => { const path = await hooks.command<string | null>("portable_choose_import", { folder: name === "portable-choose-folder" }); if (path) { source = path; importPreview = null; } }); break;
      case "portable-import-preview": if (source) { if (!busy) importPreview = null; await perform("preview", async requestId => { const preview = await hooks.command<ImportPreview>("portable_preview_import", { path: source, requestId }); if (requestId === current) importPreview = preview; }); } break;
      case "portable-import-queue": if (importPreview) { const planId = importPreview.planId; await perform("queue", async () => { await hooks.command("queue_portable_import", { planId, clientRequest: crypto.randomUUID() }); await hooks.refresh(); hooks.notify(t("Importación añadida a Descargas. Los originales se conservan.", "Import added to Downloads. Originals are kept."), "success"); }); } break;
      default: return false;
    }
    return true;
  }
  function open(ids: string[] = [], forSharing = false) { mode = "export"; selected.clear(); if (forSharing) options = { metadata: false, notes: false, ocr: false }; const available = new Set(hooks.snapshot().library.filter(item => !item.missing).map(item => item.id)); for (const id of ids.slice(0, 2000)) if (available.has(id)) selected.add(id); invalidate(); hooks.render(); }
  return { render, open, click, change, submit, isExportJob: (id: string) => id === exportJobId };
}
