import "./device-share.css";
import { escapeHtml as h, type Copy, type LibraryItem } from "./contracts";
import { bytesLabel, errorLabel } from "./presentation";
import { icon, option } from "./views-common";

type Mode = "files" | "link" | "text";
type Payload = { kind: "files"; ids: string[] } | { kind: "link"; url: string } | { kind: "text"; text: string };
interface Device { id: string; name: string }
interface Status { state: "ready" | "missing" | "offline" | "incompatible" | "unsupported"; devices: Device[]; share: boolean; shareText: boolean }
interface Preview { token: string; device: Device; entries: { name: string; bytes: number }[]; total: number }
type UnconfirmedReason = { kind: "cliError" } | { kind: "noReply" } | { kind: "fileCount"; replies: number };
type Receipt = { state: "handedOff" } | { state: "unconfirmed"; reason: UnconfirmedReason };
type Review = Omit<Preview, "token">;
interface Hooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  item(id: string): LibraryItem | undefined;
  refreshLibrary(): Promise<void>;
  package(ids: string[]): void;
  afterClose(): void;
  t: Copy;
}

const maxFiles = 50;
const maxTextBytes = 8192;
const utf8 = new TextEncoder();

export function createDeviceShare(hooks: Hooks) {
  const { t } = hooks;
  let dialog: HTMLDialogElement | undefined;
  let status: Status | null = null, preview: Preview | null = null;
  let mode: Mode = "files", ids: string[] = [], link = "", text = "", label = "", deviceId = "";
  let sourceUrl: string | null = null;
  let busy: "discover" | "review" | "send" | null = null, generation = 0, error = "";
  let result: { receipt: Receipt; review: Review } | null = null;

  const supported = () => mode === "text" ? status?.shareText : status?.share;
  const textBytes = () => utf8.encode(text).byteLength;
  const textCount = (size: number) => `${size} / ${maxTextBytes} B`;
  const textLimitNote = (size: number) => size > maxTextBytes
    ? t("El texto supera 8 KiB. Acórtalo para continuar.", "The text exceeds 8 KiB. Shorten it to continue.") : "";
  const isAvailable = (id: string) => {
    const item = hooks.item(id);
    return item !== undefined && !item.missing;
  };
  function selection(): Payload | null {
    if (mode === "files") return ids.length > 0 && ids.length <= maxFiles && ids.every(isAvailable) ? { kind: "files", ids } : null;
    if (mode === "link") return link.trim() ? { kind: "link", url: link.trim() } : null;
    return text.trim() && textBytes() <= maxTextBytes ? { kind: "text", text } : null;
  }

  function discard() {
    if (preview) void hooks.command("device_share_discard", { token: preview.token }).catch(() => {});
    preview = null;
  }

  function textField() {
    const size = textBytes();
    return `<div class="device-share-text"><label for="share-text-value">${t("Texto para el dispositivo", "Text for the device")}
      <textarea id="share-text-value" rows="4" maxlength="${maxTextBytes}" aria-describedby="share-text-limit share-text-bytes"
        aria-invalid="${size > maxTextBytes}" ${busy ? "disabled" : ""}>${h(text)}</textarea></label>
      <div class="device-share-text-meta"><span id="share-text-limit" role="status">${textLimitNote(size)}</span>
        <output id="share-text-bytes" for="share-text-value" aria-live="off">${textCount(size)}</output></div></div>`;
  }

  function syncTextField(field: HTMLElement) {
    const size = textBytes();
    field.setAttribute("aria-invalid", String(size > maxTextBytes));
    const limit = dialog?.querySelector("#share-text-limit");
    const count = dialog?.querySelector("#share-text-bytes");
    const note = textLimitNote(size);
    if (limit && limit.textContent !== note) limit.textContent = note;
    if (count) count.textContent = textCount(size);
  }

  function receiptEvidence(receipt: Receipt, files: number): string {
    if (receipt.state === "handedOff") return "";
    switch (receipt.reason.kind) {
      case "cliError":
        return t("KDE Connect devolvió un error o un diagnóstico durante la entrega.", "KDE Connect returned an error or diagnostic during handoff.");
      case "noReply":
        return t("La respuesta de KDE Connect quedó vacía o no se pudo leer.", "The KDE Connect response was empty or unreadable.");
      case "fileCount": {
        const { replies } = receipt.reason;
        return t(
          `KDE Connect devolvió ${replies} ${replies === 1 ? "respuesta" : "respuestas"} para ${files} ${files === 1 ? "archivo" : "archivos"}.`,
          `KDE Connect returned ${replies} ${replies === 1 ? "response" : "responses"} for ${files} ${files === 1 ? "file" : "files"}.`,
        );
      }
    }
  }

  function draw() {
    if (!dialog) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement.id : "";
    const devices = status?.devices ?? [];
    const deviceLabel = (device: Device) => devices.some(other => other.id !== device.id && other.name === device.name)
      ? `${device.name} · ${device.id.slice(-8)}` : device.name;
    const reviewContent = (review: Review, showEntries: boolean) => `
      <div><span class="field-note">${t("Dispositivo", "Device")}</span><strong>${h(deviceLabel(review.device))}</strong></div>
      <div><span class="field-note">${review.entries.length} ${review.entries.length === 1 ? t("elemento", "item") : t("elementos", "items")}</span><strong>${h(bytesLabel(review.total))}</strong></div>
      ${showEntries ? `<ul aria-label="${t("Contenido revisado", "Reviewed content")}">${review.entries.map(item => `<li><span>${h(item.name)}</span><small>${h(bytesLabel(item.bytes))}</small></li>`).join("")}</ul>` : ""}`;
    const files = mode === "files" ? ids.map(hooks.item).filter(item => item !== undefined) : [];
    const available = files.filter(item => !item.missing);
    const missing = files.filter(item => item.missing);
    const removed = mode === "files" ? ids.length - files.length : 0;
    const unavailable = missing.length + removed;
    const help = status?.state === "missing" ? t("Instala KDE Connect por separado o selecciona el ejecutable de una instalación existente.", "Install KDE Connect separately or select the executable from an existing installation.")
      : status?.state === "offline" ? t("Abre KDE Connect en Windows y actualiza los dispositivos.", "Open KDE Connect in Windows and refresh the devices.")
      : status?.state === "incompatible" ? t("Esta instalación no expone los comandos de envío de KDE Connect.", "This installation does not expose the KDE Connect share commands.")
      : status?.state === "unsupported" ? t("La integración de nabnab está disponible en Windows.", "Nabnab’s integration is available on Windows.")
      : !devices.length && status ? t("Empareja un dispositivo desde KDE Connect y comprueba que esté accesible.", "Pair a device in KDE Connect and make sure it is reachable.") : "";
    const button = (id: string, caption: string, style = "quiet", disabled = false) => `<button type="button" id="share-${id}" class="button ${style}" data-share-action="${id}" ${disabled ? "disabled" : ""}>${h(caption)}</button>`;
    const evidence = result ? receiptEvidence(result.receipt, result.review.entries.length) : "";
    dialog.innerHTML = `<header class="device-share-head"><div><p class="eyebrow">KDE Connect</p><h2 id="device-share-title">${t("Enviar a dispositivo", "Send to device")}</h2>${label ? `<p class="field-note">${h(label)}</p>` : ""}</div>
      <button type="button" id="share-close" class="icon-button" data-share-action="close" aria-label="${t("Cerrar", "Close")}" ${busy === "send" ? "disabled" : ""}>${icon("close")}</button></header>
      ${result ? `<section class="device-share-result" aria-labelledby="share-result-status"><h3 id="share-result-status" tabindex="-1" aria-describedby="${evidence ? "share-result-evidence " : ""}share-result-note">${result.receipt.state === "handedOff" ? t("Entregado a KDE Connect", "Handed off to KDE Connect") : t("Entrega a KDE Connect sin confirmar", "KDE Connect handoff unconfirmed")}</h3>
        ${evidence ? `<p id="share-result-evidence">${h(evidence)}</p>` : ""}
        <p id="share-result-note">${result.receipt.state === "handedOff" ? t("Consulta KDE Connect para seguir la transferencia y confirmar la recepción.", "Check KDE Connect for transfer progress and receipt.") : t("Comprueba KDE Connect y el dispositivo antes de volver a enviar.", "Check KDE Connect and the device before sending again.")}</p>
        <div class="device-share-review">${reviewContent(result.review, mode === "files")}</div></section>` : preview ? `<section class="device-share-review">
        ${reviewContent(preview, true)}
        <div class="device-share-footer">${button("back", t("Editar envío", "Edit share"), "quiet", Boolean(busy))}${button("send", busy ? t("Entregando a KDE Connect…", "Handing off to KDE Connect…") : t("Confirmar envío", "Confirm share"), "primary", Boolean(busy))}</div></section>` : `<section class="device-share-fields">
        <div class="filter-tabs" role="group" aria-label="${t("Qué quieres enviar", "What to share")}">${([['files', t("Archivos", "Files")], ['link', t("Enlace público", "Public link")], ['text', t("Texto", "Text")]] as const).map(([kind, caption]) => `<button type="button" id="share-${kind}" data-share-action="${kind}" aria-pressed="${mode === kind}" ${busy ? "disabled" : ""}>${h(caption)}</button>`).join("")}</div>
        ${mode === "files" ? `<div class="device-share-files">
          ${available.length ? `<strong>${available.length} ${available.length === 1 ? t("archivo disponible", "available file") : t("archivos disponibles", "available files")}</strong>
            <ul>${available.slice(0, 5).map(file => `<li>${h(file.name)}</li>`).join("")}</ul>
            ${available.length > 5 ? `<p class="field-note">+${available.length - 5} ${t("más", "more")}</p>` : ""}` : ""}
          ${!ids.length ? `<p class="field-note">${t("Selecciona archivos en Biblioteca o abre un resultado de Descargas.", "Select files in Library or open an output in Downloads.")}</p>` : ""}
          ${unavailable ? `<div class="device-share-missing">
            <strong>${unavailable} ${unavailable === 1 ? t("archivo no disponible", "unavailable file") : t("archivos no disponibles", "unavailable files")}</strong>
            ${missing.length ? `<ul aria-label="${t("Archivos ausentes", "Missing files")}" ${missing.length > 4 ? 'tabindex="0"' : ""}>${missing.map(file => `<li>${h(file.name)}</li>`).join("")}</ul>` : ""}
            ${removed ? `<p class="field-note">${removed} ${removed === 1 ? t("ya no está en Biblioteca.", "is no longer in Library.") : t("ya no están en Biblioteca.", "are no longer in Library.")}</p>` : ""}
            ${button("exclude-missing", t("Quitar del envío", "Remove from share"), "secondary", Boolean(busy))}</div>` : ""}
          ${ids.length > maxFiles && !unavailable ? `<p class="field-note">${t("Para más de 50 archivos, prepara una colección portátil.", "For more than 50 files, prepare a portable collection.")}</p>` : ""}
          ${sourceUrl && ids.length === 1 ? `<div class="device-share-source"><span class="field-note">${t("Comparte la ficha pública sin adjuntar la copia.", "Share the public record without attaching the file.")}</span>
            ${button("source", t("Usar enlace de origen", "Use source link"), "secondary", Boolean(busy))}</div>` : ""}
          ${ids.length > 1 ? `<div class="device-share-package">${button("zip", t("Preparar ZIP de la selección", "Prepare selection ZIP"), "secondary", Boolean(busy || unavailable))}
            ${unavailable ? "" : `<p class="field-note">${t("Podrás revisar el envío en Colección portátil cuando el ZIP esté listo.", "You can review the share in Portable collection once the ZIP is ready.")}</p>`}</div>` : ""}</div>` : mode === "link"
          ? `<label for="share-url">${t("Enlace HTTP o HTTPS", "HTTP or HTTPS link")}<input id="share-url" type="url" maxlength="2048" value="${h(link)}" ${busy ? "disabled" : ""}></label><p class="field-note">${t("Para enlaces privados o con firma, elige el archivo local en Archivos.", "For private or signed links, choose the local file under Files.")}</p>`
          : textField()}
        <label for="share-device">${t("Dispositivo emparejado", "Paired device")}<select id="share-device" ${busy || !devices.length ? "disabled" : ""}>${option("", t("Elige un dispositivo", "Choose a device"), deviceId)}${devices.map(device => option(device.id, deviceLabel(device), deviceId)).join("")}</select></label>
        ${help ? `<p class="field-note" role="status">${h(help)}</p>` : ""}
        ${status?.state === "ready" && !supported() ? `<p class="field-note">${t("Esta instalación de KDE Connect no admite este tipo de envío.", "This KDE Connect installation does not support this share type.")}</p>` : ""}
        ${busy === "discover" ? `<p class="field-note" role="status">${t("Consultando KDE Connect…", "Checking KDE Connect…")}</p>` : ""}
        <div class="device-share-footer">${button("refresh", t("Actualizar dispositivos", "Refresh devices"), "quiet", Boolean(busy))}
          ${status && ["missing", "offline", "incompatible"].includes(status.state) ? button("choose", t("Seleccionar ejecutable", "Choose executable"), "secondary", Boolean(busy)) : ""}
          ${button("review", busy === "review" ? t("Preparando revisión…", "Preparing review…") : t("Revisar envío", "Review share"), "primary", Boolean(busy || !deviceId || !supported() || !selection()))}</div></section>`}
      ${error ? `<p class="notice error device-share-error" role="alert" tabindex="-1">${h(error)}</p>` : ""}`;
    if (previous) dialog.querySelector<HTMLElement>(`#${CSS.escape(previous)}`)?.focus({ preventScroll: true });
  }

  async function perform(kind: NonNullable<typeof busy>, work: (revision: number) => Promise<void>) {
    if (busy) return;
    const revision = ++generation;
    busy = kind; error = ""; draw();
    try { await work(revision); }
    catch (cause) {
      if (revision === generation) {
        error = errorLabel(cause);
        if (mode === "files" && kind !== "discover") await hooks.refreshLibrary().catch(() => {});
      }
    }
    finally { if (revision === generation) { busy = null; draw(); } }
  }

  function refresh(choose = false) {
    return perform("discover", async revision => {
      if (choose) await hooks.command("device_share_choose_cli");
      const id = ids.length === 1 ? ids[0] : undefined;
      const [next, suggested] = await Promise.all([
        hooks.command<Status>("device_share_status"),
        id && hooks.item(id)?.details?.sourceUrl
          ? hooks.command<string | null>("device_share_source_url", { id }).catch(() => null)
          : null,
      ]);
      if (revision !== generation) return;
      status = next;
      sourceUrl = suggested;
      if (!next.devices.some(device => device.id === deviceId)) deviceId = "";
    });
  }

  async function click(name: string) {
    if (name === "close" && busy !== "send") { dialog?.close(); return; }
    if (busy) return;
    if (name === "refresh" || name === "choose") { await refresh(name === "choose"); return; }
    if (name === "source" && sourceUrl) {
      mode = "link"; link = sourceUrl; error = ""; draw();
      dialog?.querySelector<HTMLInputElement>("#share-url")?.focus();
      return;
    }
    if (name === "zip" && ids.length > 1 && ids.every(isAvailable)) { const current = ids.slice(); dialog?.close(); hooks.package(current); return; }
    if (name === "exclude-missing") {
      ids = ids.filter(isAvailable); error = ""; draw();
      for (const id of ["review", "device", "files"]) {
        const field = dialog?.querySelector<HTMLElement>(`#share-${id}:enabled`);
        if (field) { field.focus(); break; }
      }
      return;
    }
    if (name === "files" || name === "link" || name === "text") { mode = name; error = ""; draw(); return; }
    if (name === "back") { discard(); error = ""; draw(); return; }
    if (name === "review") {
      const data = selection();
      if (!data || !supported() || !deviceId) return;
      await perform("review", async revision => {
        const result = await hooks.command<Preview>("device_share_prepare", { deviceId, selection: data });
        if (revision === generation) preview = result;
        else void hooks.command("device_share_discard", { token: result.token }).catch(() => {});
      });
      dialog?.querySelector<HTMLElement>(preview ? "#share-send" : ".device-share-error")?.focus({ preventScroll: true });
    } else if (name === "send" && preview) {
      const { token, ...review } = preview;
      await perform("send", async () => {
        preview = null;
        const receipt = await hooks.command<Receipt>("device_share_send", { token });
        result = { receipt, review };
      });
      dialog?.querySelector<HTMLElement>(result ? "#share-result-status" : ".device-share-error")?.focus();
    }
  }

  function open(input: { ids?: string[]; label?: string; text?: string; url?: string } = {}) {
    if (!dialog) {
      dialog = document.createElement("dialog");
      dialog.className = "device-share";
      dialog.setAttribute("aria-labelledby", "device-share-title");
      dialog.addEventListener("cancel", event => { if (busy === "send") event.preventDefault(); });
      dialog.addEventListener("close", () => {
        if (dialog?.open) return;
        generation++; busy = null; result = null; sourceUrl = null; discard(); text = link = "";
        hooks.afterClose();
      });
      dialog.addEventListener("click", event => {
        if (!(event.target instanceof Element)) return;
        const button = event.target.closest<HTMLButtonElement>("[data-share-action]");
        if (button && !button.disabled) void click(button.dataset.shareAction ?? "");
      });
      dialog.addEventListener("input", event => {
        const element = event.target;
        if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement || element instanceof HTMLSelectElement)) return;
        if (element.id === "share-url") link = element.value;
        else if (element.id === "share-text-value") { text = element.value; syncTextField(element); }
        else if (element.id === "share-device") deviceId = element.value;
        const review = dialog?.querySelector<HTMLButtonElement>("#share-review");
        if (review) review.disabled = Boolean(busy || !deviceId || !supported() || !selection());
      });
      document.body.append(dialog);
    }
    if (dialog.open) return;
    ids = [...new Set(input.ids ?? [])]; label = input.label ?? ""; link = input.url ?? ""; text = input.text ?? "";
    mode = input.text !== undefined ? "text" : input.url !== undefined || !ids.length ? "link" : "files";
    deviceId = ""; result = null; error = ""; status = null; sourceUrl = null; busy = null; discard();
    draw(); dialog.showModal();
    void refresh();
  }

  return { open, isOpen: () => dialog?.open ?? false };
}
