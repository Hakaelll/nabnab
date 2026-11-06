import { escapeHtml as h, type Copy } from "./contracts";
import { action, icon, option } from "./views-common";

export interface TorrentFile { index: number; path: string; size: number }
export interface TorrentManifest { id: string; name: string; files: TorrentFile[]; totalSize: number; private: boolean }
export interface TorrentSelection { reference: string; manifest: TorrentManifest }
export type Seeding = { kind: "stop" } | { kind: "minutes"; minutes: number } | { kind: "ratio"; ratio: number };
export interface TorrentOptions { selected: number[]; downloadLimitKib: number; uploadLimitKib: number; seeding: Seeding }
interface TorrentStatus { enabled: boolean; installed: boolean }
interface TorrentHooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  t: Copy;
  render(): void;
  refresh(): Promise<void>;
  notify(message: string, kind?: "info" | "success" | "error"): void;
  showError(error: unknown): void;
  settings(): void;
  folder(): string;
}

const bytes = (value: number) => value < 1024 ? `${value} B` : value < 1024 ** 2 ? `${(value / 1024).toFixed(1)} KiB` : value < 1024 ** 3 ? `${(value / 1024 ** 2).toFixed(1)} MiB` : `${(value / 1024 ** 3).toFixed(2)} GiB`;

export function createTorrentsController(hooks: TorrentHooks) {
  const { t } = hooks;
  let status: TorrentStatus | null = null;
  let current: TorrentSelection | null = null;
  let magnet = "";
  let busy = false;
  let metadataLoading = false;
  let consent = false;
  let query = "";
  let downloadLimitKib = 0;
  let uploadLimitKib = 128;
  let seedingKind: Seeding["kind"] = "stop";
  let seedingValue = 60;
  const selected = new Set<number>();

  async function perform(operation: () => Promise<void>) {
    if (busy) return;
    busy = true; hooks.render();
    try { await operation(); } catch (error) { hooks.showError(error); }
    finally { busy = false; hooks.render(); }
  }
  async function load() {
    try { status = await hooks.command<TorrentStatus>("torrent_status"); } catch (error) { hooks.showError(error); }
    hooks.render();
  }
  function setSelection(value: TorrentSelection) {
    current = value; selected.clear(); query = "";
  }
  function tree(files: TorrentFile[], prefix = ""): string {
    const folders = new Map<string, TorrentFile[]>();
    const leaves: TorrentFile[] = [];
    for (const file of files) {
      const relative = file.path.slice(prefix.length);
      const slash = relative.indexOf("/");
      if (slash === -1) leaves.push(file);
      else {
        const folder = prefix + relative.slice(0, slash + 1);
        const children = folders.get(folder) ?? []; children.push(file); folders.set(folder, children);
      }
    }
    return [...folders].map(([folder, children]) => `<details open class="torrent-folder"><summary><label><input type="checkbox" data-torrent-folder="${h(folder)}" ${children.every(file => selected.has(file.index)) ? "checked" : ""} ${busy ? "disabled" : ""}>${h(folder.slice(prefix.length, -1))}</label><span class="field-note">${children.length} · ${bytes(children.reduce((sum, file) => sum + file.size, 0))}</span></summary>${tree(children, folder)}</details>`).join("")
      + leaves.map(file => `<label class="source-item"><input type="checkbox" data-torrent-file="${file.index}" ${selected.has(file.index) ? "checked" : ""} ${busy ? "disabled" : ""}><span class="source-item-main"><strong>${h(file.path.slice(prefix.length))}</strong><small>${bytes(file.size)}</small></span></label>`).join("");
  }
  function render(): string {
    const ready = Boolean(status?.enabled && status.installed);
    const selectedFiles = current?.manifest.files.filter(file => selected.has(file.index)) ?? [];
    const visible = current?.manifest.files.filter(file => file.path.toLocaleLowerCase().includes(query.toLocaleLowerCase())) ?? [];
    return `<div class="page-heading"><div><h1>${t("Archivos y BitTorrent", "Files and BitTorrent")}</h1><p>${t("Elige qué guardar antes de descargar.", "Choose what to save before downloading.")}</p></div>${action("torrent-settings", t("Componentes", "Components"), "settings", "quiet")}</div>
      ${status === null ? `<p role="status">${t("Comprobando el componente…", "Checking the component…")}</p>` : !status.installed ? `<div class="notice">${icon("info")}<div><strong>${t("Se necesita aria2", "aria2 is required")}</strong><p>${t("Instala el componente opcional Archivos/BitTorrent en Ajustes. No se instala ni se activa al abrir esta vista.", "Install the optional Files/BitTorrent component in Settings. Opening this view does not install or activate it.")}</p></div></div>` : ""}
      ${status && !status.enabled ? `<section class="settings-form"><h2>${t("Activar intercambio con peers", "Enable peer exchange")}</h2><p>${t("BitTorrent conecta con otros equipos y puede subir partes mientras descarga. Usarlo desde nabnab no proporciona anonimato. El seeding posterior se detiene al terminar por defecto.", "BitTorrent connects to other computers and may upload pieces while downloading. Using it in nabnab does not provide anonymity. Post-download seeding stops on completion by default.")}</p><label><input id="torrent-consent" type="checkbox" ${consent ? "checked" : ""}>${t("Entiendo y autorizo las conexiones P2P y la subida durante las descargas.", "I understand and allow P2P connections and uploads during downloads.")}</label>${action("torrent-enable", t("Activar BitTorrent", "Enable BitTorrent"), undefined, "primary", !consent || !status.installed || busy ? "disabled" : "")}</section>` : ""}
      <form id="torrent-magnet-form" class="settings-form" aria-busy="${metadataLoading}"><label>${t("Enlace magnet v1", "V1 magnet link")}<input id="torrent-magnet" value="${h(magnet)}" placeholder="magnet:?xt=urn:btih:…" maxlength="16384" spellcheck="false" ${busy ? "disabled" : ""}></label><div class="button-group"><button type="submit" class="button secondary" ${!ready || busy || !magnet.trim() ? "disabled" : ""}>${metadataLoading ? t("Recuperando metadatos…", "Retrieving metadata…") : t("Obtener lista de archivos", "Retrieve file list")}</button>${action("torrent-open", t("Abrir archivo .torrent", "Open .torrent file"), "folder", "secondary", busy ? "disabled" : "")}${metadataLoading ? action("torrent-cancel-inspection", t("Detener búsqueda de metadatos", "Stop metadata lookup"), "close", "quiet") : ""}</div>${metadataLoading ? `<p role="status">${t("Conectando con peers para obtener la lista. Los archivos de contenido aún no se descargan.", "Connecting to peers to retrieve the list. Content files are not being downloaded yet.")}</p>` : ""}<p class="field-note">${t("Abrir un .torrent local no conecta a la red. Obtener metadatos de un magnet sí conecta con peers y puede tardar hasta dos minutos.", "Opening a local .torrent does not connect to the network. Retrieving magnet metadata connects to peers and may take up to two minutes.")}</p></form>
      ${current ? `<section aria-label="${t("Selección de archivos", "File selection")}"><div class="page-heading"><div><h2>${h(current.manifest.name)}</h2><p>${current.manifest.files.length} ${t("archivos", "files")} · ${bytes(current.manifest.totalSize)}${current.manifest.private ? ` · ${t("Torrent privado", "Private torrent")}` : ""}</p></div></div><label class="search-control">${icon("search")}<input id="torrent-search" type="search" value="${h(query)}" placeholder="${t("Filtrar archivos", "Filter files")}" aria-label="${t("Filtrar archivos", "Filter files")}"></label><div class="selection-toolbar">${action("torrent-select-visible", t("Seleccionar visibles", "Select visible"), undefined, "quiet", busy ? "disabled" : "")}${action("torrent-clear", t("Quitar selección", "Clear selection"), undefined, "quiet", busy ? "disabled" : "")}<span>${selectedFiles.length} ${t("seleccionados", "selected")} · ${bytes(selectedFiles.reduce((sum, file) => sum + file.size, 0))}</span></div><div class="torrent-tree">${visible.length ? tree(visible.slice(0, 2000)) : `<p>${t("No hay archivos con este filtro.", "No files match this filter.")}</p>`}</div>${visible.length > 2000 ? `<p class="field-note">${t("Se muestran los primeros 2000 resultados. Filtra para acotar la lista.", "Showing the first 2000 results. Filter to narrow the list.")}</p>` : ""}<p class="field-note">${t("La selección parcial puede transferir piezas compartidas con archivos no elegidos. Los temporales se conservan para reanudar y no se ejecuta ningún archivo.", "Partial selections may transfer shared pieces belonging to unselected files. Temporary data is kept for resume and no files are executed.")}</p><div class="settings-form"><div class="form-grid"><label>${t("Límite de bajada, KiB/s", "Download limit, KiB/s")}<input id="torrent-download-limit" type="number" min="0" max="1048576" value="${downloadLimitKib}"></label><label>${t("Límite de subida, KiB/s", "Upload limit, KiB/s")}<input id="torrent-upload-limit" type="number" min="0" max="1048576" value="${uploadLimitKib}"></label></div><p class="field-note">${t("0 significa sin límite. Detener el seeding posterior no elimina la subida durante la descarga.", "0 means unlimited. Stopping post-download seeding does not prevent uploads during downloads.")}</p><label>${t("Al terminar la descarga", "After download completes")}<select id="torrent-seeding">${option("stop", t("Detener seeding", "Stop seeding"), seedingKind)}${option("minutes", t("Sembrar durante un tiempo", "Seed for a duration"), seedingKind)}${option("ratio", t("Sembrar hasta un ratio", "Seed to a ratio"), seedingKind)}</select></label>${seedingKind === "stop" ? "" : `<label>${seedingKind === "minutes" ? t("Minutos de seeding", "Seeding minutes") : t("Ratio de subida/descarga", "Upload/download ratio")}<input id="torrent-seeding-value" type="number" min="${seedingKind === "minutes" ? 1 : 0.1}" max="${seedingKind === "minutes" ? 1440 : 20}" step="${seedingKind === "minutes" ? 1 : 0.1}" value="${seedingValue}"></label><p class="field-note">${t("El seeding por ratio tiene un límite adicional de 24 horas.", "Ratio-based seeding also has a 24-hour limit.")}</p>`}<p class="field-note path">${t("Destino", "Destination")}: ${h(hooks.folder())}</p>${action("torrent-queue", t("Añadir selección a Descargas", "Add selection to Downloads"), "add", "primary", !ready || busy || !selectedFiles.length ? "disabled" : "")}</div></section>` : ""}`;
  }
  function handleInput(element: HTMLElement): boolean {
    if (element instanceof HTMLInputElement) {
      if (element.id === "torrent-magnet") { magnet = element.value; hooks.render(); return true; }
      if (element.id === "torrent-search") { query = element.value; hooks.render(); return true; }
      if (element.id === "torrent-consent") { consent = element.checked; hooks.render(); return true; }
      if (element.id === "torrent-download-limit") { downloadLimitKib = element.valueAsNumber; return true; }
      if (element.id === "torrent-upload-limit") { uploadLimitKib = element.valueAsNumber; return true; }
      if (element.id === "torrent-seeding-value") { seedingValue = element.valueAsNumber; return true; }
      if (element.dataset.torrentFile) { const id = Number(element.dataset.torrentFile); if (element.checked) selected.add(id); else selected.delete(id); hooks.render(); return true; }
      if (element.dataset.torrentFolder) { for (const file of current?.manifest.files.filter(file => file.path.startsWith(element.dataset.torrentFolder ?? "") && file.path.toLocaleLowerCase().includes(query.toLocaleLowerCase())) ?? []) { if (element.checked) selected.add(file.index); else selected.delete(file.index); } hooks.render(); return true; }
    }
    if (element instanceof HTMLSelectElement && element.id === "torrent-seeding") {
      if (element.value === "stop" || element.value === "minutes" || element.value === "ratio") { seedingKind = element.value; seedingValue = element.value === "ratio" ? 1 : 60; hooks.render(); }
      return true;
    }
    return false;
  }
  async function handleClick(name: string): Promise<boolean> {
    switch (name) {
      case "torrent-settings": hooks.settings(); break;
      case "torrent-cancel-inspection": await hooks.command("cancel_torrent_inspection"); break;
      case "torrent-enable": if (consent) await perform(async () => { status = await hooks.command<TorrentStatus>("set_torrent_enabled", { enabled: true }); }); break;
      case "torrent-open": await perform(async () => { const value = await hooks.command<TorrentSelection | null>("open_torrent"); if (value) setSelection(value); }); break;
      case "torrent-clear": selected.clear(); hooks.render(); break;
      case "torrent-select-visible": for (const file of current?.manifest.files.filter(file => file.path.toLocaleLowerCase().includes(query.toLocaleLowerCase())) ?? []) selected.add(file.index); hooks.render(); break;
      case "torrent-queue": if (current) {
        const selection = current;
        const seeding: Seeding = seedingKind === "stop" ? { kind: "stop" } : seedingKind === "minutes" ? { kind: "minutes", minutes: seedingValue } : { kind: "ratio", ratio: seedingValue };
        const options: TorrentOptions = { selected: [...selected], downloadLimitKib, uploadLimitKib, seeding };
        await perform(async () => { await hooks.command("queue_torrent", { reference: selection.reference, manifestId: selection.manifest.id, options, clientRequest: crypto.randomUUID() }); await hooks.refresh(); hooks.notify(t("Selección añadida a Descargas.", "Selection added to Downloads."), "success"); });
      } break;
      default: return false;
    }
    return true;
  }
  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    if (form.id !== "torrent-magnet-form") return false;
    if (!status?.enabled || !status.installed || !magnet.trim()) return true;
    metadataLoading = true;
    try { await perform(async () => { setSelection(await hooks.command<TorrentSelection>("inspect_torrent_magnet", { magnet })); }); }
    finally { metadataLoading = false; hooks.render(); }
    return true;
  }
  return { render, load, handleInput, handleClick, handleSubmit, openMagnet(value: string) { magnet = value; hooks.render(); void load(); } };
}
