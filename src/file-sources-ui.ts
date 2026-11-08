import "./file-sources.css";
import { escapeHtml as h, type Copy } from "./contracts";
import type { AudiobookChapter } from "./library-types";
import { action, icon, option } from "./views-common";

type CloudProvider = "drive" | "onedrive" | "dropbox";
type SourceTab = "public" | "personal" | CloudProvider;
type CatalogSearch = "arxiv" | "librivox";
export interface PublicAsset {
  id: string; sourceUrl: string; filename: string; mime: string; size: number | null;
  title: string; version: string | null; original: boolean; author: string | null;
  license: string | null; attribution: string | null; sha256: string | null;
  platform: string | null; architecture: string | null;
  publishedAt?: string | null;
  checksum?: string | null;
  audiobook?: AudiobookChapter | null;
}
interface PublicEntry { id: string; title: string; url: string; prerelease: boolean; publishedAt: string | null }
interface PublicPage {
  provider: keyof typeof publicNames; sourceUrl: string; title: string;
  description: string | null; rights: string | null; entries: PublicEntry[];
  assets: PublicAsset[]; nextCursor: string | null; warnings: string[];
}
export interface CloudSelection { provider: CloudProvider; accountId: string; id: string; exportMime: string | null; version: string | null }
interface CloudEntry {
  selection: CloudSelection; name: string; kind: "file" | "document" | "folder" | "shortcut";
  mime: string; size: number | null; location: string;
  exports: { mime: string; extension: string; label: string }[]; downloadable: boolean;
}
interface CloudPage { entries: CloudEntry[]; nextCursor: string | null; partial: boolean; warnings: string[] }
interface ConnectionStatus {
  provider: CloudProvider; configured: boolean; connected: boolean; accountId: string | null;
  scope: string; redirectUri: string;
}
interface FileSourceHooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  refresh(): Promise<void>; render(): void; t: Copy;
  renderPersonal(): string; loadPersonal(): Promise<void>;
  notify(message: string, kind?: "info" | "success" | "error"): void;
  showError(error: unknown): void; folder(): string; chooseFolder?(): void; followSource?(url: string, kind: "github" | "publicSearch"): void;
}
const providers: { id: CloudProvider; name: string }[] = [
  { id: "drive", name: "Google Drive" }, { id: "onedrive", name: "OneDrive" }, { id: "dropbox", name: "Dropbox" },
];
function isCatalogSearch(page: PublicPage | null): boolean {
  return Boolean(page && (page.provider === "arxiv" || page.provider === "librivox")
    && page.sourceUrl.startsWith(`https://${page.provider}.org/search/?`));
}
const publicNames = {
  github: "GitHub Releases", archive: "Internet Archive", commons: "Wikimedia Commons",
  arxiv: "arXiv", zenodo: "Zenodo", librivox: "LibriVox",
};
const searches: CatalogSearch[] = ["arxiv", "librivox"];
function isCloud(value: string): value is CloudProvider { return providers.some(provider => provider.id === value); }
function cloudName(provider: CloudProvider): string { return providers.find(value => value.id === provider)?.name ?? provider; }
function fileSize(value: number | null, t: Copy): string {
  if (value === null) return t("Tamaño desconocido", "Size unknown");
  if (value < 1024) return `${value} B`;
  if (value < 1024 ** 2) return `${(value / 1024).toFixed(1)} KiB`;
  if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MiB`;
  return `${(value / 1024 ** 3).toFixed(2)} GiB`;
}
function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") return error.message;
  return "The source could not be opened.";
}

export function createFileSources(hooks: FileSourceHooks) {
  const { t } = hooks;
  let tab: SourceTab = "public";
  let url = "";
  let searchMode: CatalogSearch | null = null;
  let searchQuery = "";
  let arxivField = "title";
  let language = "";
  let publicPage: PublicPage | null = null;
  let catalogReturn: { page: PublicPage; entryId: string; mode: CatalogSearch | null } | null = null;
  let cloudPage: CloudPage | null = null;
  let connections: ConnectionStatus[] | null = null;
  let connectionEditor: CloudProvider | null = null;
  let collection = "";
  let filter: "all" | "original" | "derivative" = "all";
  let reader = "";
  let query = "";
  let busy: "inspect" | "queue" | "connect" | "configure" | "revoke" | null = null;
  let error = "";
  let activeRequest = 0;
  let requestCounter = Date.now();
  let folder: string | null = null;
  let sharedUrl: string | null = null;
  let trail: { id: string | null; name: string }[] = [];
  const publicSelected = new Map<string, PublicAsset>();
  const cloudSelected = new Map<string, CloudEntry>();
  const exports = new Map<string, string>();
  function clearSelection() { publicSelected.clear(); cloudSelected.clear(); exports.clear(); }
  function status(provider: CloudProvider) { return connections?.find(value => value.provider === provider); }
  async function loadConnections() {
    error = "";
    try { connections = await hooks.command<ConnectionStatus[]>("cloud_status"); }
    catch (failure) { error = errorMessage(failure); }
    hooks.render();
  }
  async function perform(kind: NonNullable<typeof busy>, operation: (requestId: number) => Promise<void>) {
    if (busy) return;
    const requestId = ++requestCounter;
    activeRequest = requestId; busy = kind; error = ""; hooks.render();
    try { await operation(requestId); }
    catch (failure) { if (activeRequest === requestId) error = errorMessage(failure); }
    finally { if (activeRequest === requestId) { busy = null; hooks.render(); } }
  }
  async function inspectPublic(cursor: string | null = null) {
    const input = publicPage && cursor ? publicPage.sourceUrl : url.trim();
    if (!cursor && !busy) { publicPage = null; publicSelected.clear(); }
    await perform("inspect", async requestId => {
      const page = await hooks.command<PublicPage>("public_source_inspect", { url: input, cursor, requestId });
      if (activeRequest !== requestId) return;
      if (cursor && publicPage?.sourceUrl === page.sourceUrl) {
        const known = new Set(publicPage.assets.map(value => value.id));
        const knownEntries = new Set(publicPage.entries.map(value => value.id));
        publicPage = { ...page, assets: [...publicPage.assets, ...page.assets.filter(value => !known.has(value.id))], entries: [...publicPage.entries, ...page.entries.filter(value => !knownEntries.has(value.id))] };
      } else { publicPage = page; publicSelected.clear(); query = ""; filter = "all"; reader = ""; }
      url = page.sourceUrl;
    });
  }
  async function browseCloud(cursor: string | null = null) {
    if (tab === "public") return;
    const provider = tab;
    if (!cursor && !busy) cloudPage = null;
    await perform("inspect", async requestId => {
      const page = await hooks.command<CloudPage>("cloud_browse", { provider, folder, cursor, sharedUrl, requestId });
      if (activeRequest !== requestId) return;
      if (cursor && cloudPage) {
        const known = new Set(cloudPage.entries.map(value => value.selection.id));
        cloudPage = { ...page, entries: [...cloudPage.entries, ...page.entries.filter(value => !known.has(value.selection.id))] };
      } else { cloudPage = page; query = ""; }
    });
  }
  function warningList(warnings: string[]): string {
    return warnings.length ? `<ul class="file-source-warnings">${warnings.map(warning => `<li>${h(warning)}</li>`).join("")}</ul>` : "";
  }
  function catalogSearch(): string {
    const disabled = busy ? "disabled" : "";
    return `<div class="file-source-catalog"><div class="button-group" role="group" aria-label="${t("Búsqueda en catálogos", "Catalog search")}">
      ${searches.map(id => `<button class="button secondary" type="button" data-action="file-source-search-mode" data-id="${id}" aria-expanded="${searchMode === id}" aria-controls="file-source-catalog-form" ${disabled}>${icon("search")}${publicNames[id]}</button>`).join("")}
    </div>${searchMode ? `<form id="file-source-catalog-form" class="file-source-catalog-form">
      ${searchMode === "arxiv" ? `<label>${t("Buscar por", "Search by")}<select id="file-source-arxiv-field" ${disabled}>${option("title", t("Título", "Title"), arxivField)}${option("author", t("Autor", "Author"), arxivField)}${option("category", t("Categoría", "Category"), arxivField)}</select></label>` : ""}
      <label>${searchMode === "arxiv" ? t("Consulta", "Query") : t("Título del libro", "Book title")}<input id="file-source-catalog-query" value="${h(searchQuery)}" required maxlength="160" ${disabled}></label>
      ${searchMode === "librivox" ? `<label>${t("Idioma, opcional", "Language, optional")}<input id="file-source-language" value="${h(language)}" maxlength="50" placeholder="Spanish" ${disabled}></label>` : ""}
      <button type="submit" class="button primary" ${disabled}>${t("Buscar", "Search")}</button>
    </form>` : `<div id="file-source-catalog-form" hidden></div>`}</div>`;
  }
  function matchesPublicFilter(asset: PublicAsset): boolean {
    return (filter === "all" || asset.original === (filter === "original"))
      && (!reader || asset.audiobook?.reader === reader)
      && `${asset.filename} ${asset.title} ${asset.mime} ${asset.version ?? ""} ${asset.audiobook?.reader ?? ""} ${asset.audiobook?.language ?? ""}`.toLowerCase().includes(query.toLowerCase());
  }
  function readerFilter(assets: PublicAsset[]): string {
    const credits = [...new Set(assets.map(asset => asset.audiobook?.reader ?? "").filter(Boolean))];
    if (credits.length < 2) return "";
    return `<select id="file-source-reader" aria-label="${t("Lectura", "Narration")}">
      ${option("", t("Todas las lecturas", "All narrations"), reader)}
      ${credits.map(credit => option(credit, credit, reader)).join("")}
    </select>`;
  }
  function publicDescription(): string {
    if (!publicPage?.description) return "";
    const paragraph = `<p class="file-source-description">${h(publicPage.description)}</p>`;
    if (isCatalogSearch(publicPage)) return paragraph;
    const summary = publicPage.provider === "arxiv" ? t("Resumen del artículo", "Article abstract")
      : publicPage.provider === "librivox" ? t("Detalles de la edición", "Edition details") : "";
    return summary ? `<details class="file-source-abstract"><summary>${summary}</summary>${paragraph}</details>` : paragraph;
  }
  function publicRows(): string {
    if (!publicPage || !publicPage.assets.length && publicPage.entries.length) return "";
    const { provider, rights } = publicPage;
    const research = provider === "arxiv" || provider === "librivox";
    const visible = publicPage.assets.filter(matchesPublicFilter);
    const group = provider === "arxiv" ? t("Versiones del PDF", "PDF versions") : provider === "librivox" ? t("Capítulos", "Chapters") : t("Archivos disponibles", "Available files");
    const sectionHeading = research ? `<div class="file-source-list-heading"><h3>${group}</h3>${provider === "librivox" && visible.length ? action("file-source-select-chapters", t("Seleccionar capítulos visibles", "Select visible chapters"), "list", "quiet", busy || publicSelected.size >= 199 ? "disabled" : "") : ""}</div>` : "";
    return `${sectionHeading}<div class="file-source-list${research ? " file-source-research-list" : ""}" role="group" aria-label="${group}">${visible.map(asset => {
      const chapter = asset.audiobook;
      const version = provider === "arxiv" ? asset.version?.match(/^v(\d+)$/)?.[1] : undefined;
      const title = chapter ? asset.title : version ? `${t("Versión", "Version")} ${version}` : asset.filename;
      const name = chapter ? `${t("Capítulo", "Chapter")} ${chapter.chapter} · ${title}` : version ? `${title} · ${asset.filename}` : "";
      const metadata = [version ? asset.filename : chapter ? null : asset.mime, chapter || version ? null : asset.version, asset.publishedAt, chapter?.reader, chapter?.language, asset.platform, asset.architecture].filter(Boolean).join(" · ");
      const attribution = [asset.author, research && rights === asset.license ? null : asset.license].filter(Boolean).join(" · ");
      const checked = publicSelected.has(asset.id);
      return `<label class="file-source-row ${checked ? "is-selected" : ""}">
        <input type="checkbox" data-file-source-select="${h(asset.id)}" ${name ? `aria-label="${h(name)}"` : ""} ${checked ? "checked" : ""} ${busy || publicSelected.size >= 199 && !checked ? "disabled" : ""}>
        <span class="file-source-file-icon" aria-hidden="true">${chapter ? String(chapter.chapter).padStart(2, "0") : icon(asset.mime.startsWith("image/") ? "image" : asset.mime.startsWith("audio/") ? "audio" : "file")}</span>
        <span class="file-source-row-main"><strong>${h(title)}</strong>${metadata ? `<small>${h(metadata)}</small>` : ""}${provider === "archive" ? `<small>${asset.original ? t("Original", "Original") : t("Derivado", "Derivative")}</small>` : ""}${attribution ? `<small>${h(attribution)}</small>` : ""}${asset.checksum || asset.sha256 ? `<small class="file-source-checksum" title="${h(asset.checksum ?? asset.sha256 ?? "")}">${h(asset.checksum ?? `sha256:${asset.sha256}`)}</small>` : ""}</span>
        <span class="file-source-size">${h(fileSize(asset.size, t))}</span></label>`;
    }).join("") || `<p class="file-source-empty">${t("No hay archivos con este filtro.", "No files match this filter.")}</p>`}</div>`;
  }
  function cloudRows(): string {
    if (!cloudPage) return "";
    const visible = cloudPage.entries.filter(entry => entry.name.toLowerCase().includes(query.toLowerCase()));
    return `<div class="file-source-list" role="group" aria-label="${t("Archivos de la cuenta", "Account files")}">${visible.map(entry => {
      const id = entry.selection.id;
      const isFolder = entry.kind === "folder";
      const exportMime = exports.get(id) ?? entry.selection.exportMime ?? entry.exports[0]?.mime ?? "";
      if (isFolder) return `<div class="file-source-row is-folder"><span></span><span class="file-source-file-icon">${icon("folder")}</span><button type="button" class="file-source-folder" data-action="file-source-folder" data-id="${h(id)}" ${busy ? "disabled" : ""}><strong>${h(entry.name)}</strong><small>${h(entry.location)} · ${t("Abrir carpeta", "Open folder")}</small></button>${action("file-source-folder-preview", t("Revisar lote", "Review batch"), "list", "quiet", `data-id="${h(id)}" ${busy ? "disabled" : ""}`)}</div>`;
      return `<div class="file-source-row ${cloudSelected.has(id) ? "is-selected" : ""}"><input type="checkbox" data-cloud-source-select="${h(id)}" aria-label="${h(t("Seleccionar ", "Select ") + entry.name)}" ${cloudSelected.has(id) ? "checked" : ""} ${busy || !entry.downloadable || cloudSelected.size >= 200 && !cloudSelected.has(id) ? "disabled" : ""}><span class="file-source-file-icon">${icon("file")}</span><div class="file-source-row-main"><strong>${h(entry.name)}</strong><small>${h(entry.mime)}${entry.location ? ` · ${h(entry.location)}` : ""}</small>${entry.exports.length ? `<label class="file-source-export">${t("Exportar como", "Export as")}<select data-cloud-source-export="${h(id)}" ${busy ? "disabled" : ""}>${entry.exports.map(format => option(format.mime, format.label, exportMime)).join("")}</select></label>` : `<small>${entry.kind === "document" ? t("Documento nativo sin exportación disponible", "Native document with no available export") : entry.kind === "shortcut" ? t("Acceso directo; no se explora automáticamente", "Shortcut; not explored automatically") : t("Archivo original", "Original file")}</small>`}${!entry.downloadable ? `<small>${t("Sin descarga autorizada disponible", "No authorized download available")}</small>` : ""}</div><span class="file-source-size">${h(fileSize(entry.size, t))}</span></div>`;
    }).join("") || `<p class="file-source-empty">${query ? t("No hay archivos con este filtro.", "No files match this filter.") : t("Esta ubicación no contiene archivos accesibles.", "This location has no accessible files.")}</p>`}</div>`;
  }
  function selectionFooter(): string {
    const selected = tab === "public" ? [...publicSelected.values()] : [...cloudSelected.values()];
    if (tab === "public" ? !publicPage?.assets.length : !cloudPage) return "";
    const knownSize = selected.reduce((sum, item) => sum + (item.size ?? 0), 0);
    const unknown = selected.some(item => item.size === null);
    const total = unknown && knownSize === 0 ? fileSize(null, t) : fileSize(knownSize, t) + (unknown ? ` + ${t("tamaño pendiente", "unknown size")}` : "");
    return `<div class="file-source-selection"><div class="file-source-selection-title"><strong>${selected.length} ${t("seleccionados", "selected")}</strong><span>${selected.length ? h(total) : t("Selecciona los archivos que quieras conservar", "Select the files you want to keep")}</span>${selected.length ? action("file-source-clear", t("Quitar selección", "Clear selection"), undefined, "quiet", busy ? "disabled" : "") : ""}</div><label>${t("Colección, opcional", "Collection, optional")}<input id="file-source-collection" value="${h(collection)}" maxlength="120" placeholder="${t("Sin colección", "No collection")}" ${busy ? "disabled" : ""}></label><div class="file-source-destination"><span>${t("Destino", "Destination")}</span><strong title="${h(hooks.folder())}">${h(hooks.folder())}</strong>${hooks.chooseFolder ? action("file-source-destination", t("Cambiar", "Change"), "folder", "quiet", busy ? "disabled" : "") : ""}</div>${action("file-source-queue", busy === "queue" ? t("Añadiendo…", "Adding…") : t("Añadir a Descargas", "Add to Downloads"), "downloads", "primary", busy || !selected.length ? "disabled" : "")}</div>`;
  }
  function connectionCard(provider: CloudProvider): string {
    const connection = status(provider);
    const editing = connectionEditor === provider;
    const description = !connection ? t("Estado sin comprobar", "Status not checked") : connection.connected ? t("Cuenta conectada", "Account connected") : connection.configured ? t("Aplicación configurada · autorización pendiente", "App configured · authorization needed") : t("Requiere una aplicación OAuth propia", "Your OAuth app is required");
    return `<section class="cloud-connection"><div class="cloud-connection-heading"><div><h3>${h(cloudName(provider))}</h3><p>${description}</p></div><span class="file-source-status">${connection?.connected ? t("Conectado", "Connected") : connection?.configured ? t("Sin autorizar", "Not authorized") : t("Sin configurar", "Not configured")}</span></div>${connection ? `<p class="field-note">${t("Permisos de lectura solicitados", "Requested read permissions")}: <span class="cloud-scope">${h(connection.scope)}</span></p>` : ""}<div class="button-group">${action("cloud-connect", t("Autorizar en el navegador", "Authorize in browser"), "link", "secondary", `data-provider="${provider}" ${busy || !connection?.configured ? "disabled" : ""}`)}${action("cloud-configure-open", editing ? t("Cerrar configuración", "Close configuration") : t("Configurar aplicación", "Configure app"), "settings", "quiet", `data-provider="${provider}" ${busy ? "disabled" : ""}`)}${connection?.connected ? action("cloud-revoke", t("Desconectar", "Disconnect"), undefined, "quiet", `data-provider="${provider}" ${busy ? "disabled" : ""}`) : ""}</div>${editing ? `<form id="cloud-configure-form" data-provider="${provider}" class="cloud-configure-form"><label>${t("Client ID de tu aplicación OAuth", "Your OAuth app client ID")}<input id="cloud-client-id" required maxlength="2048" autocomplete="off" spellcheck="false" ${busy ? "disabled" : ""}></label>${provider === "drive" ? `<label>${t("Metadato del cliente de escritorio, opcional", "Desktop client metadata, optional")}<input id="cloud-client-secret" type="password" maxlength="4096" autocomplete="off" ${busy ? "disabled" : ""}></label><p class="field-note">${t("Solo el client_secret del archivo de credenciales de una aplicación de escritorio de Google. Una app de escritorio no puede ocultar un secreto permanente.", "Only the client_secret from Google's desktop app credentials file. A desktop app cannot hide a permanent secret.")}</p>` : ""}<p class="field-note">${t("Registra una aplicación de escritorio para uso propio. No introduzcas contraseñas ni tokens de tu cuenta.", "Register a desktop app for your own use. Do not enter account passwords or tokens.")}</p><p class="field-note">${t("URI de redirección", "Redirect URI")}: <code>${h(connection?.redirectUri ?? "http://127.0.0.1:53687/callback")}</code></p><button class="button secondary" type="submit" ${busy ? "disabled" : ""}>${t("Guardar configuración", "Save configuration")}</button></form>` : ""}</section>`;
  }
  function renderConnections(): string {
    return `<section class="cloud-connections"><div class="page-heading"><div><h2>${t("Conexiones de archivos", "File connections")}</h2><p>${t("Autorización con el proveedor y credenciales protegidas para este usuario de Windows.", "Provider authorization and credentials protected for this Windows user.")}</p></div>${action("cloud-status", t("Actualizar estado", "Refresh status"), "retry", "quiet", busy ? "disabled" : "")}</div><p class="field-note">${t("Estas conexiones solo leen y descargan. El proveedor mostrará el alcance real de la cuenta antes de autorizar. Los documentos nativos se exportan en su plataforma; las herramientas posteriores funcionan localmente.", "These connections only read and download. The provider shows the actual account scope before authorization. Native documents are exported by their platform; subsequent tools run locally.")}</p>${error ? `<p class="file-source-error" role="alert">${h(error)}</p>` : ""}${providers.map(provider => connectionCard(provider.id)).join("")}${busy === "connect" ? `<div class="file-source-progress" role="status"><span>${t("Esperando autorización en el navegador…", "Waiting for browser authorization…")}</span>${action("file-source-cancel", t("Cancelar", "Cancel"), "close", "quiet")}</div>` : ""}</section>`;
  }
  function render(): string {
    const isPublic = tab === "public";
    const header = `<section class="file-sources"><div class="page-heading"><div><h1>${t("Archivos y documentos", "Files and documents")}</h1><p>${t("Explora el origen, elige archivos y consérvalos en tu biblioteca.", "Browse the source, choose files and keep them in your library.")}</p></div></div><nav class="file-source-tabs" aria-label="${t("Origen de archivos", "File source")}">${[{ id: "public", name: t("Fuentes públicas", "Public sources") }, ...providers, { id: "personal", name: t("Servidores personales", "Personal servers") }].map(provider => `<button type="button" data-action="file-source-tab" data-tab="${provider.id}" ${tab === provider.id ? 'aria-current="page"' : ""} ${busy ? "disabled" : ""}>${h(provider.name)}</button>`).join("")}</nav>`;
    if (tab === "personal") return `${header}${hooks.renderPersonal()}</section>`;
    const provider: CloudProvider = tab === "public" ? "drive" : tab;
    const isSearch = isCatalogSearch(publicPage);
    const hasPage = isPublic ? Boolean(publicPage) : Boolean(cloudPage);
    const cursor = isPublic ? publicPage?.nextCursor : cloudPage?.nextCursor;
    return `${header}
      ${isPublic ? catalogReturn ? "" : `<form id="file-source-form" class="file-source-form"><label for="file-source-url">${t("Enlace, identificador o DOI", "Link, identifier or DOI")}</label><div><input id="file-source-url" type="text" required value="${h(url)}" maxlength="8192" placeholder="https://… · arXiv:2401.00001 · 10.5281/zenodo.1234" autocomplete="off" spellcheck="false" ${busy ? "disabled" : ""}><button type="submit" class="button secondary" ${busy ? "disabled" : ""}>${icon("search")}${t("Explorar", "Browse")}</button></div><p class="field-note">GitHub Releases · Internet Archive · Wikimedia Commons · arXiv · Zenodo · LibriVox</p></form>${catalogSearch()}` : `<div class="file-source-cloud-toolbar"><div><strong>${h(cloudName(provider))}</strong><p class="field-note">${t("Solo los archivos a los que tu cuenta tiene acceso.", "Only files your account can access.")}</p></div>${action("file-source-cloud-root", t("Mis archivos", "My files"), "folder", "secondary", busy || !status(provider)?.connected ? "disabled" : "")}${action("cloud-configure-open", t("Conexión", "Connection"), "settings", "quiet", `data-provider="${tab}" ${busy ? "disabled" : ""}`)}</div><form id="cloud-shared-form" class="file-source-form"><label for="cloud-shared-url">${t("Enlace compartido, opcional", "Shared link, optional")}</label><div><input id="cloud-shared-url" type="url" value="${h(sharedUrl ?? "")}" maxlength="8192" placeholder="https://…" autocomplete="off" spellcheck="false" ${busy ? "disabled" : ""}><button class="button secondary" type="submit" ${busy || !status(provider)?.connected ? "disabled" : ""}>${t("Abrir enlace", "Open link")}</button></div></form>${connectionEditor === tab || !status(provider)?.connected ? connectionCard(provider) : ""}`}
      ${isPublic && catalogReturn ? `<div class="file-source-return">${action("file-source-back-results", t("Volver a resultados", "Back to results"), "back", "quiet", busy ? "disabled" : "")}</div>` : ""}
      ${error ? `<div class="file-source-error" role="alert">${icon("error")}<span>${h(error)}</span></div>` : ""}${busy === "inspect" || busy === "connect" ? `<div class="file-source-progress" role="status"><span>${busy === "connect" ? t("Esperando autorización en el navegador…", "Waiting for browser authorization…") : t("Consultando archivos del origen…", "Reading source files…")}</span>${action("file-source-cancel", t("Cancelar", "Cancel"), "close", "quiet")}</div>` : ""}
      ${hasPage ? `<div class="file-source-results"><div class="file-source-result-heading"><div><span class="file-source-eyebrow">${isPublic ? h(publicPage ? publicNames[publicPage.provider] : "") : h(cloudName(provider))}</span><h2${isPublic && catalogReturn ? ' tabindex="-1"' : ""}>${h(isPublic ? publicPage?.title ?? "" : trail.at(-1)?.name ?? t("Mis archivos", "My files"))}</h2></div>${isPublic && (publicPage?.provider === "github" || isSearch) && hooks.followSource ? action("file-source-follow", isSearch ? t("Seguir búsqueda", "Follow search") : t("Seguir versiones", "Follow releases"), "add", "quiet", busy ? "disabled" : "") : ""}${!isPublic && trail.length ? action("file-source-parent", t("Volver", "Back"), undefined, "quiet", busy ? "disabled" : "") : ""}</div>${isPublic ? publicDescription() : ""}${isPublic && publicPage?.rights ? `<p class="field-note">${t("Derechos indicados por el origen", "Rights stated by the source")}: ${h(publicPage.rights)}</p>` : ""}${warningList(isPublic ? publicPage?.warnings ?? [] : cloudPage?.warnings ?? [])}
        ${isPublic && publicPage?.entries.length ? `<div class="file-source-entries">${publicPage.entries.map(entry => `<button type="button" class="file-source-entry" data-action="file-source-entry" data-id="${h(entry.id)}" ${busy ? "disabled" : ""}><span><strong>${h(entry.title)}</strong><small>${h(entry.publishedAt ?? "")}</small></span>${publicPage?.provider === "github" ? `<span class="file-source-status">${entry.prerelease ? "Prerelease" : t("Estable", "Stable")}</span>` : ""}${icon("arrow")}</button>`).join("")}</div>` : ""}
        ${(isPublic ? publicPage?.assets.length : cloudPage?.entries.length) ? `<div class="file-source-filter"><label>${icon("search")}<input id="file-source-query" type="search" value="${h(query)}" placeholder="${t("Filtrar archivos cargados", "Filter loaded files")}" aria-label="${t("Filtrar archivos cargados", "Filter loaded files")}"></label>${isPublic && publicPage?.provider === "archive" ? `<select id="file-source-kind" aria-label="${t("Tipo de archivo", "File type")}">${option("all", t("Originales y derivados", "Originals and derivatives"), filter)}${option("original", t("Solo originales", "Originals only"), filter)}${option("derivative", t("Solo derivados", "Derivatives only"), filter)}</select>` : ""}${isPublic && publicPage?.provider === "librivox" ? readerFilter(publicPage.assets) : ""}<span>${isPublic ? publicPage?.assets.length ?? 0 : cloudPage?.entries.length ?? 0} ${t("cargados", "loaded")}</span></div>` : ""}${isPublic ? publicRows() : cloudRows()}
        ${cursor ? `<div class="file-source-pagination"><p>${t("Solo se ha cargado una parte del origen. La selección incluye únicamente archivos revisados.", "Only part of this source has loaded. Selection includes reviewed files only.")}</p>${action("file-source-more", t("Cargar más", "Load more"), "add", "secondary", busy ? "disabled" : "")}</div>` : !isPublic && cloudPage?.partial ? `<p class="field-note">${t("El origen informa de resultados parciales. Revisa los avisos antes de descargar.", "The source reports partial results. Review warnings before downloading.")}</p>` : ""}</div>` : !busy && isPublic && !catalogReturn ? `<div class="file-source-intro">${icon("folder")}<h2>${t("Elige qué conservar", "Choose what to keep")}</h2><p>${t("Consulta versiones y archivos antes de descargar. No se selecciona ningún formato automáticamente.", "Review versions and files before downloading. No format is selected automatically.")}</p><div><span>GitHub</span><small>${t("Versiones y assets publicados", "Versions and published assets")}</small><span>Internet Archive</span><small>${t("Originales, derivados y procedencia", "Originals, derivatives and provenance")}</small><span>Commons</span><small>${t("Imágenes y atribución por archivo", "Images and attribution per file")}</small></div></div>` : ""}${selectionFooter()}<p class="file-source-footnote">${t("Descargar conserva una copia local. No ejecuta archivos, instala programas ni modifica el origen.", "Downloading keeps a local copy. It does not run files, install software or modify the source.")}</p></section>`;
  }
  function handleInput(element: HTMLElement): boolean {
    if (element instanceof HTMLInputElement) {
      if (element.id === "file-source-catalog-query") { searchQuery = element.value; return true; }
      if (element.id === "file-source-language") { language = element.value; return true; }
      if (element.id === "file-source-url") { url = element.value; return true; }
      if (element.id === "file-source-collection") { collection = element.value; return true; }
      if (element.id === "file-source-query") { query = element.value; hooks.render(); return true; }
      if (element.id === "cloud-shared-url") { sharedUrl = element.value || null; return true; }
      const publicId = element.dataset.fileSourceSelect;
      if (publicId) {
        const asset = publicPage?.assets.find(value => value.id === publicId);
        if (asset) { if (element.checked && publicSelected.size < 199) publicSelected.set(publicId, asset); else publicSelected.delete(publicId); }
        hooks.render(); return true;
      }
      const cloudId = element.dataset.cloudSourceSelect;
      if (cloudId) {
        const entry = cloudPage?.entries.find(value => value.selection.id === cloudId);
        if (entry?.downloadable) { if (element.checked && cloudSelected.size < 200) cloudSelected.set(cloudId, entry); else cloudSelected.delete(cloudId); }
        hooks.render(); return true;
      }
    }
    if (element instanceof HTMLSelectElement) {
      if (element.id === "file-source-arxiv-field") { arxivField = element.value; return true; }
      if (element.id === "file-source-kind" && (element.value === "all" || element.value === "original" || element.value === "derivative")) { filter = element.value; hooks.render(); return true; }
      if (element.id === "file-source-reader") { reader = element.value; hooks.render(); return true; }
      if (element.dataset.cloudSourceExport) { exports.set(element.dataset.cloudSourceExport, element.value); return true; }
    }
    return false;
  }
  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    if (form.id === "file-source-form") {
      if (!busy && form.reportValidity()) { catalogReturn = null; searchMode = null; await inspectPublic(); }
      return true;
    }
    if (form.id === "file-source-catalog-form") {
      if (!busy && searchMode && form.reportValidity() && searchQuery.trim()) {
        const search = new URL(searchMode === "arxiv" ? "https://arxiv.org/search/" : "https://librivox.org/search/");
        if (searchMode === "arxiv") {
          search.searchParams.set("field", arxivField);
          search.searchParams.set("query", searchQuery.trim());
        } else {
          search.searchParams.set("title", searchQuery.trim());
          if (language.trim()) search.searchParams.set("language", language.trim());
        }
        catalogReturn = null;
        url = search.href;
        await inspectPublic();
      }
      return true;
    }
    if (form.id === "cloud-shared-form") { if (form.reportValidity() && sharedUrl) { folder = null; trail = []; clearSelection(); await browseCloud(); } return true; }
    if (form.id !== "cloud-configure-form") return false;
    const provider = form.dataset.provider;
    const input = form.querySelector<HTMLInputElement>("#cloud-client-id");
    const secret = form.querySelector<HTMLInputElement>("#cloud-client-secret");
    if (!provider || !isCloud(provider) || !input || !form.reportValidity()) return true;
    const clientId = input.value.trim(); const clientSecret = secret?.value || null;
    await perform("configure", async () => {
      await hooks.command("cloud_configure", { provider, clientId, clientSecret });
      connectionEditor = null; await loadConnections();
      hooks.notify(t("Aplicación configurada. Autoriza la cuenta en el navegador para continuar.", "App configured. Authorize your account in the browser to continue."), "success");
    });
    return true;
  }
  async function handleClick(name: string, element: HTMLElement): Promise<boolean> {
    switch (name) {
      case "file-source-tab": {
        const value = element.dataset.tab;
        if (value === "personal") { openPersonal(); await hooks.loadPersonal(); break; }
        if (busy || value !== "public" && (!value || !isCloud(value))) break;
        tab = value; error = ""; cloudPage = null; publicPage = null; catalogReturn = null; url = ""; folder = null; sharedUrl = null; trail = []; clearSelection(); hooks.render();
        if (value !== "public") { await loadConnections(); if (status(value)?.connected) await browseCloud(); }
      } break;
      case "file-source-entry": {
        const entry = publicPage?.entries.find(value => value.id === element.dataset.id);
        if (entry && !busy) {
          if (publicPage && isCatalogSearch(publicPage)) catalogReturn = { page: publicPage, entryId: entry.id, mode: searchMode };
          url = entry.url;
          searchMode = null;
          await inspectPublic();
          if (catalogReturn) {
            const target = document.querySelector<HTMLElement>(".file-source-result-heading h2[tabindex='-1']")
              ?? document.querySelector<HTMLElement>('[data-action="file-source-back-results"]');
            target?.focus();
          }
        }
      } break;
      case "file-source-back-results": {
        if (!catalogReturn || busy) break;
        const { page, entryId, mode } = catalogReturn;
        catalogReturn = null;
        publicPage = page;
        url = page.sourceUrl;
        searchMode = mode;
        error = "";
        query = "";
        filter = "all";
        reader = "";
        publicSelected.clear();
        hooks.render();
        document.querySelector<HTMLElement>(`[data-action="file-source-entry"][data-id="${CSS.escape(entryId)}"]`)?.focus();
      } break;
      case "file-source-folder": {
        const entry = cloudPage?.entries.find(value => value.selection.id === element.dataset.id && value.kind === "folder");
        if (entry && !busy) { trail.push({ id: folder, name: entry.name }); folder = entry.selection.id; await browseCloud(); }
      } break;
      case "file-source-folder-preview": {
        const entry = cloudPage?.entries.find(value => value.selection.id === element.dataset.id && value.kind === "folder");
        if (entry) await perform("inspect", async requestId => {
          const page = await hooks.command<CloudPage>("cloud_preview_folder", { selection: entry.selection, requestId });
          if (activeRequest !== requestId) return;
          trail.push({ id: folder, name: entry.name }); folder = entry.selection.id; cloudPage = page; query = "";
        });
      } break;
      case "file-source-parent": if (!busy) { folder = trail.pop()?.id ?? null; await browseCloud(); } break;
      case "file-source-cloud-root": if (!busy) { folder = null; sharedUrl = null; trail = []; await browseCloud(); } break;
      case "file-source-more": if (tab === "public") { if (publicPage?.nextCursor) await inspectPublic(publicPage.nextCursor); } else if (cloudPage?.nextCursor) await browseCloud(cloudPage.nextCursor); break;
      case "file-source-clear": if (!busy) { clearSelection(); hooks.render(); } break;
      case "file-source-follow": if (!busy && publicPage) hooks.followSource?.(publicPage.sourceUrl, isCatalogSearch(publicPage) ? "publicSearch" : "github"); break;
      case "file-source-destination": hooks.chooseFolder?.(); break;
      case "file-source-cancel": {
        if (busy !== "inspect" && busy !== "connect") break;
        const requestId = activeRequest; activeRequest = ++requestCounter; busy = null;
        try { await hooks.command("cancel_file_source_inspection", { requestId }); } catch (failure) { hooks.showError(failure); }
        hooks.render();
      } break;
      case "file-source-queue": {
        if (busy) break;
        const sourceUrl = publicPage?.sourceUrl;
        const assetIds = [...publicSelected.keys()];
        const selections = [...cloudSelected.values()].map(entry => ({ ...entry.selection, exportMime: entry.exports.length ? exports.get(entry.selection.id) ?? entry.selection.exportMime ?? entry.exports[0]?.mime ?? null : null }));
        if (tab === "public" ? !sourceUrl || !assetIds.length : !selections.length) break;
        await perform("queue", async () => {
          await hooks.command(tab === "public" ? "queue_public_files" : "queue_cloud_files", tab === "public" ? { sourceUrl, assetIds, collection: collection.trim(), clientRequest: crypto.randomUUID() } : { selections, collection: collection.trim(), clientRequest: crypto.randomUUID() });
          clearSelection(); await hooks.refresh(); hooks.notify(t("Selección añadida a Descargas.", "Selection added to Downloads."), "success");
        });
      } break;
      case "cloud-status": await loadConnections(); break;
      case "cloud-configure-open": {
        const provider = element.dataset.provider;
        if (provider && isCloud(provider) && !busy) { connectionEditor = connectionEditor === provider ? null : provider; hooks.render(); }
      } break;
      case "cloud-connect": {
        const provider = element.dataset.provider;
        if (provider && isCloud(provider) && status(provider)?.configured) await perform("connect", async requestId => {
          await hooks.command("cloud_connect", { provider, requestId });
          if (activeRequest !== requestId) return;
          await loadConnections(); hooks.notify(t("Cuenta autorizada. Ya puedes explorar sus archivos.", "Account authorized. You can now browse its files."), "success");
        });
      } break;
      case "file-source-select-chapters": {
        if (publicPage?.provider === "librivox") {
          for (const asset of publicPage.assets) {
            if (publicSelected.size >= 199) break;
            if (matchesPublicFilter(asset)) publicSelected.set(asset.id, asset);
          }
          hooks.render();
          document.querySelector<HTMLElement>('[data-action="file-source-queue"]')?.focus();
        }
      } break;
      case "file-source-search-mode": {
        const id = element.dataset.id;
        if (id === "arxiv" || id === "librivox") {
          searchMode = searchMode === id ? null : id;
          searchQuery = "";
          hooks.render();
          if (searchMode) document.getElementById("file-source-catalog-query")?.focus();
          else document.querySelector<HTMLElement>(`[data-action="file-source-search-mode"][data-id="${id}"]`)?.focus();
        }
      } break;
      case "cloud-revoke": {
        const provider = element.dataset.provider;
        if (provider && isCloud(provider)) await perform("revoke", async () => {
          await hooks.command("cloud_revoke", { provider });
          if (tab === provider) { cloudPage = null; cloudSelected.clear(); }
          await loadConnections(); hooks.notify(t("Conexión local revocada. Los archivos se conservan.", "Local connection revoked. Files are kept."), "success");
        });
      } break;
      default: return false;
    }
    return true;
  }
  function openPersonal() { tab = "personal"; hooks.render(); }
  function openCatalog(id: CatalogSearch | "zenodo") {
    if (busy) return;
    tab = "public";
    publicPage = null;
    catalogReturn = null;
    clearSelection();
    error = "";
    url = "";
    searchQuery = "";
    searchMode = id === "zenodo" ? null : id;
    hooks.render();
    document.getElementById(id === "zenodo" ? "file-source-url" : "file-source-catalog-query")?.focus();
  }
  async function open(input?: string) {
    if (busy) return;
    error = "";
    if (!input) { hooks.render(); await loadConnections(); return; }
    let host = "";
    try { host = new URL(input).hostname.toLowerCase(); } catch { /* The provider returns the actionable URL error. */ }
    const cloud: CloudProvider | null = host === "drive.google.com" || host === "docs.google.com" ? "drive" : host === "1drv.ms" || host === "onedrive.live.com" || host.endsWith(".sharepoint.com") ? "onedrive" : host === "dropbox.com" || host.endsWith(".dropbox.com") ? "dropbox" : null;
    clearSelection(); publicPage = null; catalogReturn = null; cloudPage = null; query = ""; folder = null; trail = [];
    if (cloud) { tab = cloud; sharedUrl = input; await loadConnections(); if (status(cloud)?.connected) await browseCloud(); else hooks.render(); }
    else { tab = "public"; url = input; searchMode = null; sharedUrl = null; await inspectPublic(); }
  }
  return { render, handleClick, handleSubmit, handleInput, open, openCatalog, openPersonal, renderConnections, loadConnections };
}
