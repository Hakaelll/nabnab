import { escapeHtml as h, type Copy } from "./contracts";
import { action, icon, option } from "./views-common";

interface FileProbe { filename: string; host: string; size: number | null; supportsRanges: boolean }
interface Component { id: string; installed: boolean; enabled: boolean }
interface AdvancedFileHooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  t: Copy;
  render(): void;
  refresh(): Promise<void>;
  notify(message: string, kind?: "info" | "success" | "error"): void;
  showError(error: unknown): void;
  folder(): string;
  settings(): void;
}
export function createAdvancedFilesController(hooks: AdvancedFileHooks) {
  const { t } = hooks;
  let url = "";
  let connections = 4;
  let checksum = "";
  let probe: FileProbe | null = null;
  let component: Component | null = null;
  let busy = false;
  async function perform(operation: () => Promise<void>) {
    if (busy) return;
    busy = true; hooks.render();
    try { await operation(); } catch (error) { hooks.showError(error); }
    finally { busy = false; hooks.render(); }
  }
  async function load() {
    try { component = (await hooks.command<Component[]>("optional_components")).find(value => value.id === "files") ?? null; }
    catch (error) { hooks.showError(error); }
    hooks.render();
  }
  function render(): string {
    const ready = Boolean(component?.installed && component.enabled);
    return `<div class="page-heading"><div><h1>${t("Descargar un archivo", "Download a file")}</h1><p>${t("Enlaces HTTP o HTTPS, reanudación y comprobación SHA-256.", "HTTP or HTTPS links, resume and SHA-256 checks.")}</p></div>${action("advanced-file-settings", t("Componentes", "Components"), "settings", "quiet")}</div>
      ${!ready ? `<div class="notice">${icon("info")}<div><strong>${t("Se necesita el componente Archivos", "The Files component is required")}</strong><p>${t("Instala y activa aria2 desde Ajustes → Componentes. La inspección del enlace está disponible sin instalar el motor.", "Install and enable aria2 in Settings → Components. Link inspection is available without installing the engine.")}</p></div></div>` : ""}
      <form id="advanced-file-form" class="settings-form" aria-busy="${busy}"><label>${t("Enlace del archivo", "File URL")}<input id="advanced-file-url" type="url" required maxlength="8192" autocomplete="off" spellcheck="false" value="${h(url)}" placeholder="https://…/archivo.zip" ${busy ? "disabled" : ""}></label><div class="form-grid"><label>${t("Conexiones máximas", "Maximum connections")}<select id="advanced-file-connections" ${busy ? "disabled" : ""}>${[1,2,4,8,16].map(value => option(String(value),String(value),String(connections))).join("")}</select></label><label>${t("SHA-256 esperado, opcional", "Expected SHA-256, optional")}<input id="advanced-file-checksum" pattern="[A-Fa-f0-9]{64}" maxlength="64" value="${h(checksum)}" autocomplete="off" spellcheck="false" ${busy ? "disabled" : ""}></label></div><p class="field-note">${t("Copia el hash de una fuente de confianza. nabnab no lo deduce del nombre ni de metadatos no verificados. Si no coincide, el archivo no se añade a la biblioteca.", "Copy the hash from a trusted source. nabnab does not infer it from the filename or unverified metadata. A mismatch prevents the file from being added to the library.")}</p><button type="submit" class="button secondary" ${busy ? "disabled" : ""}>${busy ? t("Comprobando…", "Checking…") : t("Inspeccionar enlace", "Inspect link")}</button>
      ${probe ? `<section class="source-item"><div class="source-item-main"><strong>${h(probe.filename)}</strong><span>${h(probe.host)} · ${probe.size === null ? t("Tamaño desconocido", "Size unknown") : `${(probe.size / 1024 ** 2).toFixed(1)} MiB`}</span><p>${probe.supportsRanges ? t("El servidor respondió a la petición de rango. Las conexiones se usan según tamaño y disponibilidad.", "The server answered the range request. Connections are used according to file size and availability.") : t("El servidor no confirmó rangos. Se usará una conexión; la reanudación puede no estar disponible.", "The server did not confirm ranges. One connection will be used; resume may be unavailable.")}</p></div></section>` : ""}<p class="field-note">${t("La reanudación exige rangos y una identidad de versión fiable o un checksum esperado. Si cambia la fuente, los parciales se conservan y se requiere un trabajo nuevo. Más conexiones no garantizan más velocidad.", "Resume requires ranges and a reliable version identity or an expected checksum. If the source changes, partial files are kept and a new job is required. More connections do not guarantee more speed.")}</p><p class="field-note path">${t("Destino", "Destination")}: ${h(hooks.folder())}</p>${action("advanced-file-queue", t("Añadir archivo a Descargas", "Add file to Downloads"), "add", "primary", !ready || busy || !probe ? "disabled" : "")}</form>`;
  }
  function handleInput(element: HTMLElement): boolean {
    if (element instanceof HTMLInputElement) {
      if (element.id === "advanced-file-url") { url = element.value; probe = null; hooks.render(); return true; }
      if (element.id === "advanced-file-checksum") { checksum = element.value; return true; }
    }
    if (element instanceof HTMLSelectElement && element.id === "advanced-file-connections") { connections = Number(element.value); return true; }
    return false;
  }
  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    if (form.id !== "advanced-file-form") return false;
    await perform(async () => { probe = await hooks.command<FileProbe>("inspect_advanced_file", { url }); });
    return true;
  }
  async function handleClick(name: string): Promise<boolean> {
    switch (name) {
      case "advanced-file-settings": hooks.settings(); break;
      case "advanced-file-queue": {
        const form = document.getElementById("advanced-file-form");
        if (!(form instanceof HTMLFormElement) || !form.reportValidity() || !probe) break;
        await perform(async () => { await hooks.command("queue_advanced_file", { url, connections, checksum: checksum.trim() || null, title: probe?.filename ?? "", clientRequest: crypto.randomUUID() }); await hooks.refresh(); hooks.notify(t("Archivo añadido a Descargas.", "File added to Downloads."), "success"); });
      } break;
      default: return false;
    }
    return true;
  }
  return { render, load, handleInput, handleSubmit, handleClick, openUrl(value: string) { url = value; probe = null; hooks.render(); void load(); } };
}
