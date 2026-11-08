import { escapeHtml as h, type Copy, type Job } from "./contracts";
import type { Preset } from "./options";
import { action, icon, option, unknownDuration } from "./views-common";
import { defaultSourceSettings, type FollowedSource, type SourceItem, type SourceItemState, type SourceKind, type SourceSettings, type SourceStatus } from "./source-types";

interface SourceSnapshot { sources?: FollowedSource[]; presets?: Preset[]; jobs: Job[] }
interface SourceHooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  getSnapshot(): SourceSnapshot;
  refresh(): Promise<void>;
  notify(text: string, kind?: "info" | "success" | "error"): void;
  showError(error: unknown): void;
  t: Copy;
  render(): void;
  openPublicSource?(url: string): void;
}
type EditorKind = "youtube" | Exclude<SourceKind, "channel" | "playlist">;
interface Editor { id: string | null; name: string; url: string; settings: SourceSettings; paused: boolean; kind: EditorKind; publicFeed: boolean }
const fileReview = (kind: EditorKind | SourceKind) => kind === "github" || kind === "publicSearch";
const catalogName = (url: string) => url.startsWith("https://arxiv.org/") ? "arXiv"
  : url.startsWith("https://zenodo.org/") ? "Zenodo" : "LibriVox";
const manualSearchSettings = (settings: SourceSettings): SourceSettings => ({
  ...settings, intervalHours: null, automatic: false, presetId: null, relativeFolder: "",
  pageSize: 20, explorationLimit: 200, history: "reviewHistory",
});

export function createSourcesController(hooks: SourceHooks) {
  let detailId: string | null = null;
  let editor: Editor | null = null;
  let query = "";
  let itemFilter: SourceItemState | "all" = "all";
  let itemOffset = 0;
  let busy = false;
  const selected = new Set<string>();
  const { t } = hooks;
  const all = () => hooks.getSnapshot().sources ?? [];
  const presets = () => hooks.getSnapshot().presets ?? [];
  const statusLabel = (status: SourceStatus) => ({ ready: t("Disponible", "Ready"), checking: t("Comprobando", "Checking"), paused: t("Incorporaciones pausadas", "Future additions paused"), requiresSession: t("Requiere sesión", "Requires session"), error: t("Error", "Error") })[status];
  const itemLabel = (state: SourceItemState) => ({ observed: t("Historial observado", "Observed history"), review: t("Pendiente de revisar", "For review"), queued: t("En cola", "Queued"), downloaded: t("Descargado", "Downloaded"), ignored: t("Ignorado", "Ignored"), unavailable: t("No disponible", "Unavailable") })[state];
  const reasonLabel = (reason: string): string => {
    switch (reason) {
      case "Incomplete baseline or missing publication date. Review this item before downloading.": return t("La exploración es parcial o falta la fecha de publicación. Revisa este elemento antes de descargar.", reason);
      case "Initial history is never downloaded automatically. Select an item to queue it.": return t("El historial inicial requiere selección. Elige el contenido que quieras añadir a la cola.", reason);
      case "Open a catalog result to review the available files.": return t("Abre un resultado del catálogo para revisar los archivos disponibles.", reason);
      case "Release history is observed only. Open a release to choose its files.": return t("El historial de versiones queda observado. Abre una versión para elegir sus archivos.", reason);
      case "This older channel item may belong to history not previously inspected.": return t("Este contenido antiguo puede pertenecer al historial aún sin inspeccionar.", reason);
      case "The automatic check limit was reached. This item remains for review.": return t("Se alcanzó el límite automático de esta comprobación. Este elemento queda para revisión.", reason);
      case "A related output did not finish. Review its jobs in Downloads.": return t("Una salida relacionada no terminó. Revisa sus trabajos en Descargas.", reason);
      case "The source reports this item as unavailable.": return t("La fuente indica que este elemento no está disponible.", reason);
      case "Live, upcoming or protected content is not available for automatic download.": return t("El contenido en directo, futuro o protegido no admite descarga automática.", reason);
      default: return reason;
    }
  };
  const effectiveState = (item: SourceItem): SourceItemState => {
    if (!item.jobIds.length) return item.state;
    const linked = item.jobIds.map(id => hooks.getSnapshot().jobs.find(job => job.id === id));
    if (linked.every(job => job?.state === "completed")) return "downloaded";
    if (linked.some(job => job && ["pending", "paused", "analyzing", "ready", "downloading", "processing", "verifying"].includes(job.state))) return "queued";
    return "review";
  };
  const pending = (source: FollowedSource) => source.items.filter(item => effectiveState(item) === "review").length;
  const visibleItems = (source: FollowedSource) => source.items.filter(item => (itemFilter === "all" || effectiveState(item) === itemFilter) && `${item.title} ${item.creator ?? ""}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  const validSelection = (source: FollowedSource) => source.items.filter(item => selected.has(item.contentId) && !["queued", "downloaded", "unavailable"].includes(effectiveState(item)));

  function editFields(value: Editor): string {
    const settings = value.settings;
    const publicSearch = value.kind === "publicSearch";
    return `<form id="source-editor" class="settings-form source-editor" aria-busy="${busy}"><h2>${value.id ? t("Configurar fuente", "Configure source") : t("Añadir fuente", "Add source")}</h2>
      ${value.id ? `<label>${t("Nombre", "Name")}<input id="source-name" value="${h(value.name)}" maxlength="240"></label>` : `<label>${t("Tipo de fuente", "Source type")}<select id="source-kind">${option("youtube", "YouTube", value.kind)}${option("podcast", t("Podcast RSS", "RSS podcast"), value.kind)}${option("gallery", t("Galería o tablero", "Gallery or board"), value.kind)}${option("github", "GitHub Releases", value.kind)}${option("publicSearch", t("Catálogo de investigación", "Research catalog"), value.kind)}</select></label><label>${publicSearch ? t("Enlace de búsqueda o historial", "Search or history URL") : t("Enlace de canal, lista, feed, galería o repositorio", "Channel, playlist, feed, gallery or repository URL")}<input id="source-url" type="url" required value="${h(value.url)}" placeholder="https://…"></label>${value.kind === "podcast" ? `<label><input id="source-public-feed" type="checkbox" ${value.publicFeed ? "checked" : ""}>${t("Este feed es público y puede incluirse en exportaciones OPML", "This feed is public and can be included in OPML exports")}</label><p class="field-note">${t("Sin marcar, el enlace se protege y se excluye de exportaciones. No compartas enlaces de suscripción privados.", "When unchecked, the link is protected and excluded from exports. Do not share private subscription URLs.")}</p>` : ""}`}
      ${fileReview(value.kind) ? "" : `<div class="form-grid"><label>${t("Ajuste para futuros trabajos", "Preset for future jobs")}<select id="source-preset">${option("", t("Valores generales y reglas", "General defaults and rules"), settings.presetId ?? "")}${presets().map(preset => option(preset.id, `${preset.name} · v${preset.revision}`, settings.presetId ?? "")).join("")}</select></label><label>${t("Carpeta relativa", "Relative folder")}<input id="source-folder" value="${h(settings.relativeFolder)}" placeholder="Tutoriales/{creador}"></label></div>`}
      ${value.id ? "" : `<fieldset><legend>${t("Qué contenido considerar", "Which content to consider")}</legend><label><input type="radio" name="source-history" value="fromNow" ${settings.history === "fromNow" ? "checked" : ""}>${t("Desde ahora. Conservar el historial como observado", "From now. Record existing history as observed")}</label><label><input type="radio" name="source-history" value="reviewHistory" ${settings.history === "reviewHistory" ? "checked" : ""}>${t("Revisar una selección inicial del historial", "Review an initial history selection")}</label></fieldset>`}
      ${publicSearch ? `<p class="field-note">${t("Comprobaciones manuales. Abre un resultado para elegir archivos. Puedes seguir búsquedas e historiales desde Plataformas.", "Manual checks. Open a result to choose files. You can follow searches and version histories from Platforms.")}</p>` : `<label>${t("Comprobaciones", "Checks")}<select id="source-interval">${[["", t("Solo manuales", "Manual only")], ["1", t("Cada hora", "Every hour")], ["6", t("Cada 6 horas", "Every 6 hours")], ["24", t("Cada 24 horas", "Every 24 hours")]].map(([value, label]) => option(value, label, String(settings.intervalHours ?? ""))).join("")}</select></label>`}
      ${publicSearch ? "" : value.kind === "github" ? `<p class="field-note">${t("Las versiones nuevas quedan para revisión. Abre una versión y elige sus archivos antes de descargar. No se ejecuta ni instala software.", "New releases await review. Open a release and choose its files before downloading. Software is never run or installed.")}</p>` : `<fieldset><legend>${t("Qué hacer con novedades", "What to do with new items")}</legend><label><input type="checkbox" id="source-automatic" ${settings.automatic ? "checked" : ""}>${t("Autorizar descarga automática de novedades identificadas", "Allow automatic downloads of identified new items")}</label><p class="field-note">${t("Sin marcar, todo queda para revisión. El historial y los elementos ambiguos siempre requieren selección. La descarga automática requiere un ajuste guardado.", "When unchecked, all new items await review. History and ambiguous items always require selection. Automatic downloads require a saved preset.")}</p><label>${t("Máximo automático por comprobación", "Automatic limit per check")}<input id="source-auto-limit" type="number" min="1" max="1000" value="${settings.automaticLimit}"></label><p class="field-note">${t("El exceso queda pendiente de revisión. El límite inicial es 20.", "Excess items await review. The initial limit is 20.")}</p></fieldset>`}
      ${publicSearch ? `<label>${t("Límite de posiciones por búsqueda", "Position limit per search")}<input id="source-exploration-limit" type="number" min="20" max="200" step="20" value="${settings.explorationLimit}"></label><p class="field-note">${t("Se consultan bloques de 20. Los filtros pueden reducir las coincidencias visibles.", "Each page checks 20 positions. Filters can reduce the number of matches.")}</p>` : `<details><summary>${t("Alcance de exploración", "Exploration scope")}</summary><div class="form-grid"><label>${t("Elementos por bloque", "Items per page")}<input id="source-page-size" type="number" min="1" max="${value.kind === "github" ? 50 : 1000}" value="${settings.pageSize}"></label><label>${t("Límite antes de ampliar", "Limit before extending")}<input id="source-exploration-limit" type="number" min="1" max="${value.kind === "github" ? 5000 : 100000}" value="${settings.explorationLimit}"></label></div></details>`}
      ${value.kind === "gallery" ? `<p class="field-note">${t("Requiere Galerías activado. La exploración conserva los originales y envía los archivos seleccionados a una colección. Las fechas ausentes o sin zona horaria requieren revisión manual.", "Requires Galleries enabled. Selected files keep their original format and go to a collection. Missing dates or dates without a timezone require manual review.")}</p>` : ""}
      ${value.id ? `<label><input id="source-paused" type="checkbox" ${value.paused ? "checked" : ""}>${t("Pausar futuras comprobaciones e incorporaciones", "Pause future checks and additions")}</label>` : ""}
      <p class="field-note">${t("Funciona mientras nabnab está abierto, también en bandeja. No comprueba ni despierta el equipo con la app cerrada. La primera exploración no descarga archivos.", "Works while nabnab is open, including in the tray. A closed app cannot check or wake the computer. Initial exploration downloads no files.")}</p>
      <div class="button-group"><button type="submit" class="button primary" ${busy ? "disabled" : ""}>${busy ? t("Explorando…", "Exploring…") : value.id ? t("Guardar fuente", "Save source") : t("Añadir y explorar", "Add and explore")}</button>${action("source-edit-cancel", t("Cancelar", "Cancel"), undefined, "quiet", busy ? "disabled" : "")}</div></form>`;
  }

  function render(): string {
    const source = all().find(candidate => candidate.id === detailId);
    if (editor) return editFields(editor);
    if (source) {
      const reviewFiles = fileReview(source.kind);
      const matching = visibleItems(source), selectedItems = validSelection(source);
      itemOffset = Math.min(itemOffset, Math.max(0, Math.floor((matching.length - 1) / 100) * 100));
      const visible = matching.slice(itemOffset, itemOffset + 100);
      const checked = source.lastCheck ? new Date(source.lastCheck).toLocaleString() : t("Sin comprobar", "Not checked");
      return `<div class="page-heading"><div><button class="button quiet" data-action="source-back">${t("Todas las fuentes", "All sources")}</button><h1>${h(source.name)}</h1><p>${h(statusLabel(source.status))} · ${h(checked)}</p></div><div class="button-group">${action("source-check", t("Comprobar novedades", "Check for new items"), "retry", "secondary", `data-id="${h(source.id)}" ${busy || source.status === "paused" || source.status === "checking" ? "disabled" : ""}`)}${action("source-edit", t("Configurar", "Configure"), "settings", "quiet", `data-id="${h(source.id)}"`)}${source.kind === "podcast" ? action("source-export-m3u", t("Exportar M3U", "Export M3U"), undefined, "quiet", busy ? "disabled" : "") : ""}</div></div>
        <p class="field-note path">${h(source.url)}</p><p class="field-note">${source.settings.automatic ? `${t("Descarga automática autorizada", "Automatic downloads allowed")} · ${source.settings.automaticLimit} ${t("como máximo por comprobación", "maximum per check")}` : t("Novedades para revisión manual", "New items await manual review")} · ${source.settings.intervalHours ? `${source.settings.intervalHours} h` : t("Comprobaciones manuales", "Manual checks")} · ${t("Requiere nabnab abierto", "Requires nabnab open")}</p>
        ${source.error ? `<div class="notice error">${icon("error")}<div><strong>${t("La última comprobación requiere atención", "The last check needs attention")}</strong><p>${h(source.error)}</p></div></div>` : ""}
        <div class="view-toolbar"><label class="search-control">${icon("search")}<input id="source-search" type="search" aria-label="${t("Buscar en esta fuente", "Search this source")}" placeholder="${t("Buscar en esta fuente", "Search this source")}" value="${h(query)}"></label><label class="compact-label">${t("Estado", "State")}<select id="source-item-filter">${option("all", t("Todos", "All"), itemFilter)}${(["review", "observed", "queued", "downloaded", "ignored", "unavailable"] satisfies SourceItemState[]).map(state => option(state, itemLabel(state), itemFilter)).join("")}</select></label></div>
        <div class="library-controls"><span class="result-count">${source.baseline.inspected} ${t("posiciones inspeccionadas", "positions inspected")}${source.baseline.total === null ? "" : ` / ${source.baseline.total}`} · ${source.items.length} ${t("elementos conocidos", "known items")} · ${source.baseline.complete ? t("línea base completa", "complete baseline") : t("exploración parcial", "partial exploration")}</span></div>
        ${source.kind === "publicSearch" ? `<p class="field-note">${t("Abre un resultado para revisar las versiones o capítulos disponibles y seleccionar archivos.", "Open a result to review versions or chapters and choose files.")}</p>` : source.kind === "github" ? `<p class="field-note">${t("Abre una versión para seleccionar archivos. El seguimiento no descarga ni instala software automáticamente.", "Open a release to select files. Following never downloads or installs software automatically.")}</p>` : `<div class="selection-toolbar">${action("source-select-visible", t("Seleccionar visibles", "Select visible"), undefined, "quiet")}${action("source-clear-selection", t("Quitar selección", "Clear selection"), undefined, "quiet")}<span>${selectedItems.length} ${t("para añadir", "to add")}</span>${action("source-queue-selected", t("Añadir selección a la cola", "Add selection to queue"), "add", "primary", !selectedItems.length || busy ? "disabled" : "")}${action("source-ignore-selected", t("Ignorar selección", "Ignore selection"), undefined, "quiet", !selectedItems.length || busy ? "disabled" : "")}</div>`}
        ${source.status === "checking" ? `<p role="status">${t("Comprobando la fuente. Puedes seguir utilizando nabnab.", "Checking the source. You can keep using nabnab.")}</p>` : ""}
        ${visible.length ? `<div class="source-items">${visible.map(item => { const state = effectiveState(item); if (reviewFiles) return `<article class="source-item media-item"><div class="source-item-main"><strong>${h(item.title)}</strong><span>${h(itemLabel(state))} · ${source.kind === "github" ? (item.mediaType === "github-prerelease" ? t("Previa", "Prerelease") : t("Estable", "Stable")) : h(item.contentId.startsWith("arxiv:") ? item.contentId.slice(6) : item.creator)}${item.publishedAt ? ` · ${h(new Date(item.publishedAt).toLocaleDateString())}` : ""}</span>${item.reason ? `<small>${h(reasonLabel(item.reason))}</small>` : ""}</div>${action("source-release-files", t("Elegir archivos", "Choose files"), "arrow", "secondary", `data-id="${h(item.contentId)}" ${!hooks.openPublicSource ? "disabled" : ""}`)}</article>`; return `<article class="source-item media-item"><label class="source-item-check"><input type="checkbox" data-source-item="${h(item.contentId)}" aria-label="${h(t("Seleccionar ", "Select ") + item.title)}" ${selected.has(item.contentId) ? "checked" : ""} ${["queued", "downloaded", "unavailable"].includes(state) ? "disabled" : ""}></label><div class="source-item-main"><strong>${h(item.title)}</strong><span>${h(itemLabel(state))} · ${source.kind === "gallery" ? h(item.mediaType === "video" ? t("Vídeo", "Video") : item.mediaType === "animation" ? t("Animación o WebP", "Animation or WebP") : t("Imagen", "Image")) : unknownDuration(item.duration, t)} · ${item.discovery === "published" ? t("Publicación nueva", "New publication") : item.discovery === "added" ? t("Nueva incorporación a la lista", "New playlist addition") : item.discovery === "history" ? t("Historial", "History") : t("Requiere revisión", "Requires review")}</span>${item.reason ? `<small>${h(reasonLabel(item.reason))}</small>` : ""}</div></article>`; }).join("")}</div>` : `<div class="empty-state">${icon("link")}<h2>${source.lastCheck ? t("Sin elementos en esta vista", "No items in this view") : t("Explora esta fuente", "Explore this source")}</h2><p>${t("Comprueba la fuente o cambia los filtros. Los elementos descubiertos no son archivos locales.", "Check the source or change the filters. Discovered items are not local files.")}</p></div>`}
        ${matching.length > 100 ? `<div class="library-pagination"><span>${itemOffset + 1}–${Math.min(matching.length, itemOffset + 100)} / ${matching.length}</span><div class="button-group">${action("source-items-previous", t("Anterior", "Previous"), undefined, "quiet", itemOffset === 0 ? "disabled" : "")}${action("source-items-next", t("Siguiente", "Next"), undefined, "quiet", itemOffset + 100 >= matching.length ? "disabled" : "")}</div></div>` : ""}
        ${source.baseline.hasMore ? `<div class="library-pagination"><p>${source.baseline.cursor >= source.settings.explorationLimit ? (source.kind === "publicSearch" && source.settings.explorationLimit === 200 ? t("Se alcanzó el límite de 200 posiciones. Puedes acotar la búsqueda en Plataformas.", "The 200-position limit was reached. You can narrow the search in Platforms.") : t("Alcance inicial alcanzado. Amplía el límite en Configurar para continuar.", "Initial scope reached. Increase the limit in Configure to continue.")) : t("Queda contenido sin inspeccionar. Una página parcial no representa toda la fuente.", "Some content has not been inspected. A partial page is not the whole source.")}</p>${action("source-continue", `${t("Cargar otro bloque", "Load another page")} · ${source.settings.pageSize}`, undefined, "secondary", busy || source.baseline.cursor >= source.settings.explorationLimit ? "disabled" : "")}</div>` : ""}
        <details class="inspector-secondary"><summary>${t("Eliminar seguimiento", "Remove source following")}</summary><p>${t("Cancela futuras comprobaciones y conserva todos los archivos locales.", "Stops future checks and keeps all local files.")}</p><label><input type="checkbox" id="source-remove-pending">${t("Cancelar también sus trabajos pendientes y pausados", "Also cancel its pending and paused jobs")}</label>${action("source-remove", t("Eliminar fuente y conservar archivos", "Remove source and keep files"), "close", "quiet danger", busy ? "disabled" : "")}</details>`;
    }
    return `<div class="page-heading"><div><h1>${t("Fuentes", "Sources")}</h1><p>${t("Sigue canales, listas, podcasts, galerías, versiones de software y búsquedas públicas.", "Follow channels, playlists, podcasts, galleries, software releases and public searches.")}</p></div><div class="button-group">${action("source-import-opml", t("Importar OPML", "Import OPML"), undefined, "quiet", busy ? "disabled" : "")}${action("source-export-opml", t("Exportar OPML", "Export OPML"), undefined, "quiet", busy ? "disabled" : "")}${action("source-add", t("Añadir fuente", "Add source"), "add", "primary")}</div></div><p class="field-note">${t("Solo se comprueban con nabnab abierto. La descarga automática necesita un permiso separado por fuente.", "Checks only run while nabnab is open. Automatic downloads need separate permission for each source.")}</p>
      ${all().length ? `<div class="source-list">${all().map(source => `<article class="source-row"><button class="media-open" data-action="source-open" data-id="${h(source.id)}"><span class="media-info"><strong>${h(source.name)}</strong><span>${source.kind === "publicSearch" ? catalogName(source.url) : source.kind === "github" ? "GitHub Releases" : source.kind === "gallery" ? t("Galería", "Gallery") : source.kind === "podcast" ? t("Podcast RSS", "RSS podcast") : source.kind === "playlist" ? t("Lista", "Playlist") : t("Canal", "Channel")} · ${h(statusLabel(source.status))} · ${pending(source)} ${t("pendientes de revisión", "awaiting review")}</span><small>${source.lastCheck ? new Date(source.lastCheck).toLocaleString() : t("Sin comprobar", "Not checked")} · ${fileReview(source.kind) ? t("Selección de archivos", "File selection") : h(presets().find(preset => preset.id === source.settings.presetId)?.name ?? t("Reglas y valores generales", "Rules and defaults"))}${source.settings.relativeFolder ? ` · ${h(source.settings.relativeFolder)}` : ""}</small></span>${icon("arrow")}</button></article>`).join("")}</div>` : `<div class="empty-state">${icon("link")}<h2>${t("Añade tu primera fuente", "Add your first source")}</h2><p>${t("Pega un canal, lista de YouTube, feed RSS, galería o repositorio GitHub. Revisa una exploración inicial sin descargar todo el historial.", "Paste a YouTube channel, playlist, RSS feed, gallery or GitHub repository. Review an initial exploration without downloading all its history.")}</p>${action("source-add", t("Añadir fuente", "Add source"), "add")}</div>`}`;
  }

  function resetScroll(): void { document.getElementById("main")?.scrollTo({ top: 0, behavior: "instant" }); }
  function openAdd(url = "", kind: EditorKind = "youtube"): void {
    const defaults = defaultSourceSettings();
    editor = { id: null, name: "", url, settings: kind === "publicSearch" ? manualSearchSettings(defaults) : defaults, paused: false, kind, publicFeed: false };
    detailId = null; selected.clear(); hooks.render(); resetScroll();
    document.getElementById("source-url")?.focus({ preventScroll: true });
  }
  function handleInput(element: HTMLElement): boolean {
    if (element instanceof HTMLInputElement && element.id === "source-search") { query = element.value; itemOffset = 0; hooks.render(); return true; }
    if (element instanceof HTMLSelectElement && element.id === "source-item-filter") { const value = element.value; if (["all", "observed", "review", "queued", "downloaded", "ignored", "unavailable"].includes(value)) { switch (value) { case "observed": case "review": case "queued": case "downloaded": case "ignored": case "unavailable": itemFilter = value; break; default: itemFilter = "all"; } } itemOffset = 0; hooks.render(); return true; }
    if (element instanceof HTMLInputElement && element.dataset.sourceItem) { if (element.checked) selected.add(element.dataset.sourceItem); else selected.delete(element.dataset.sourceItem); hooks.render(); return true; }
    if (!editor || !(element instanceof HTMLInputElement || element instanceof HTMLSelectElement)) return false;
    const settings = editor.settings;
    switch (element.id) {
      case "source-kind": {
        switch (element.value) {
          case "podcast": case "gallery": case "github": case "publicSearch": editor.kind = element.value; break;
          default: editor.kind = "youtube";
        }
        if (editor.kind === "publicSearch") editor.settings = manualSearchSettings(settings);
        if (editor.kind === "github") {
          settings.automatic = false;
          settings.pageSize = Math.min(50, settings.pageSize);
          settings.explorationLimit = Math.min(5000, settings.explorationLimit);
        }
        hooks.render(); return true;
      }
      case "source-public-feed": if (element instanceof HTMLInputElement) editor.publicFeed = element.checked; return true;
      case "source-url": editor.url = element.value; return true;
      case "source-name": editor.name = element.value; return true;
      case "source-preset": settings.presetId = element.value || null; return true;
      case "source-folder": settings.relativeFolder = element.value; return true;
      case "source-interval": settings.intervalHours = element.value === "1" ? 1 : element.value === "6" ? 6 : element.value === "24" ? 24 : null; return true;
      case "source-automatic": if (element instanceof HTMLInputElement) settings.automatic = element.checked; return true;
      case "source-auto-limit": settings.automaticLimit = Number(element.value); return true;
      case "source-page-size": settings.pageSize = Number(element.value); return true;
      case "source-exploration-limit": settings.explorationLimit = Number(element.value); return true;
      case "source-paused": if (element instanceof HTMLInputElement) editor.paused = element.checked; return true;
      default: if (element instanceof HTMLInputElement && element.name === "source-history") { settings.history = element.value === "reviewHistory" ? "reviewHistory" : "fromNow"; return true; } return false;
    }
  }
  async function perform(operation: () => Promise<unknown>): Promise<void> {
    if (busy) return;
    busy = true; hooks.render();
    try { await operation(); await hooks.refresh(); } catch (error) { hooks.showError(error); }
    finally { busy = false; hooks.render(); }
  }
  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    if (form.id !== "source-editor" || !editor) return false;
    const value = structuredClone(editor);
    await perform(async () => {
      const source = value.id
        ? await hooks.command<FollowedSource>("save_source", { id: value.id, name: value.name, settings: value.settings, paused: value.paused })
        : await hooks.command<FollowedSource>("add_source", { url: value.url, settings: value.settings, kind: value.kind === "youtube" ? null : value.kind, publicFeed: value.publicFeed });
      detailId = source.id; editor = null; selected.clear();
      hooks.notify(source.error ? t("Fuente guardada. Revisa el error de la exploración.", "Source saved. Review the exploration error.") : t("Fuente guardada. El historial no se ha descargado.", "Source saved. History has not been downloaded."), source.error ? "info" : "success");
    });
    if (!editor) { resetScroll(); document.querySelector<HTMLElement>("[data-action='source-edit']")?.focus({ preventScroll: true }); }
    return true;
  }
  async function handleClick(actionName: string, element: HTMLElement): Promise<boolean> {
    if (!actionName.startsWith("source-")) return false;
    const source = all().find(candidate => candidate.id === detailId);
    switch (actionName) {
      case "source-add": openAdd(); break;
      case "source-import-opml": await perform(async () => { const count = await hooks.command<number>("import_podcast_opml"); hooks.notify(count + " " + t("suscripciones importadas. Comprueba cada fuente para explorar episodios.", "subscriptions imported. Check each source to explore episodes."), "success"); }); break;
      case "source-export-opml": await perform(async () => { const result = await hooks.command<{ exported: number; omitted: number }>("export_podcast_opml"); hooks.notify(result.exported + " " + t("exportadas", "exported") + " · " + result.omitted + " " + t("privadas excluidas", "private feeds omitted"), "success"); }); break;
      case "source-export-m3u": if (source) await perform(async () => { await hooks.command("export_podcast_m3u", { id: source.id }); hooks.notify(t("Lista local exportada.", "Local playlist exported."), "success"); }); break;
      case "source-back": detailId = null; selected.clear(); query = ""; break;
      case "source-release-files": if (source && fileReview(source.kind)) { const result = source.items.find(item => item.contentId === element.dataset.id); if (result) hooks.openPublicSource?.(result.url); } break;
      case "source-open": detailId = element.dataset.id ?? null; selected.clear(); query = ""; itemFilter = "all"; itemOffset = 0; break;
      case "source-edit": if (source) editor = { id: source.id, name: source.name, url: source.url, settings: structuredClone(source.settings), paused: source.status === "paused", kind: source.kind === "channel" || source.kind === "playlist" ? "youtube" : source.kind, publicFeed: Boolean(source.publicFeedUrl) }; break;
      case "source-edit-cancel": editor = null; break;
      case "source-select-visible": if (source) for (const item of visibleItems(source).slice(itemOffset, itemOffset + 100)) if (!["queued", "downloaded", "unavailable"].includes(effectiveState(item))) selected.add(item.contentId); break;
      case "source-items-previous": itemOffset = Math.max(0, itemOffset - 100); break;
      case "source-items-next": itemOffset += 100; break;
      case "source-clear-selection": selected.clear(); break;
      case "source-check": case "source-continue": if (source) await perform(() => hooks.command("check_source", { id: source.id, continuation: actionName === "source-continue" })); break;
      case "source-queue-selected": case "source-ignore-selected": if (source) {
        const contentIds = validSelection(source).map(item => item.contentId);
        await perform(async () => { await hooks.command("source_item_action", { id: source.id, contentIds, action: actionName === "source-queue-selected" ? "queue" : "ignore" }); selected.clear(); hooks.notify(actionName === "source-queue-selected" ? t("Selección añadida a la cola.", "Selection added to queue.") : t("Selección ignorada.", "Selection ignored."), "success"); });
      } break;
      case "source-remove": if (source) {
        const checkbox = document.getElementById("source-remove-pending");
        const cancelPending = checkbox instanceof HTMLInputElement && checkbox.checked;
        await perform(async () => { await hooks.command("remove_source", { id: source.id, cancelPending }); detailId = null; selected.clear(); hooks.notify(t("Seguimiento eliminado. Los archivos se conservan.", "Source following removed. Files are kept."), "success"); });
      } break;
      default: return false;
    }
    hooks.render();
    if (["source-open", "source-back", "source-edit", "source-edit-cancel"].includes(actionName)) resetScroll();
    if (actionName === "source-edit") document.getElementById("source-name")?.focus({ preventScroll: true });
    return true;
  }
  return { render, openAdd, handleClick, handleInput, handleSubmit, open(id: string) { detailId = id; editor = null; selected.clear(); hooks.render(); } };
}
