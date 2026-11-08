import "./device-share.css";
import { escapeHtml as h, type Copy, type LibraryItem } from "./contracts";
import { bytesLabel, errorLabel } from "./presentation";
import { icon, option } from "./views-common";

type Mode = "files" | "link" | "text";
type Payload = { kind: "files"; ids: string[] } | { kind: "link"; url: string } | { kind: "text"; text: string };
interface Device { id: string; name: string }
interface Status { state: "ready" | "missing" | "offline" | "incompatible" | "unsupported"; devices: Device[]; share: boolean; shareText: boolean }
interface Preview { token: string; device: Device; entries: { name: string; bytes: number }[]; total: number }
interface Hooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  item(id: string): LibraryItem | undefined;
  package(ids: string[]): void;
  afterClose(): void;
  t: Copy;
}

const maxFiles = 50;

export function createDeviceShare(hooks: Hooks) {
  const { t } = hooks;
  let dialog: HTMLDialogElement | undefined;
  let status: Status | null = null, preview: Preview | null = null;
  let mode: Mode = "files", ids: string[] = [], link = "", text = "", label = "", deviceId = "";
  let busy: "discover" | "review" | "send" | null = null, generation = 0, error = "";
  let receipt: "handedOff" | "unconfirmed" | null = null;

  const supported = () => mode === "text" ? status?.shareText : status?.share;
  function selection(): Payload | null {
    if (mode === "files") return ids.length > 0 && ids.length <= maxFiles ? { kind: "files", ids } : null;
    if (mode === "link") return link.trim() ? { kind: "link", url: link.trim() } : null;
    return text.trim() ? { kind: "text", text } : null;
  }

  function discard() {
    if (preview) void hooks.command("device_share_discard", { token: preview.token }).catch(() => {});
    preview = null;
  }

  function draw() {
    if (!dialog) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement.id : "";
    const devices = status?.devices ?? [];
    const deviceLabel = (device: Device) => devices.some(other => other.id !== device.id && other.name === device.name)
      ? `${device.name} · ${device.id.slice(-8)}` : device.name;
    const files = ids.map(hooks.item).filter(item => item !== undefined);
    const help = status?.state === "missing" ? t("Instala KDE Connect por separado o selecciona el ejecutable de una instalación existente.", "Install KDE Connect separately or select the executable from an existing installation.")
      : status?.state === "offline" ? t("Abre KDE Connect en Windows y actualiza los dispositivos.", "Open KDE Connect in Windows and refresh the devices.")
      : status?.state === "incompatible" ? t("Esta instalación no expone los comandos de envío de KDE Connect.", "This installation does not expose the KDE Connect share commands.")
      : status?.state === "unsupported" ? t("La integración de nabnab está disponible en Windows.", "Nabnab’s integration is available on Windows.")
      : !devices.length && status ? t("Empareja un dispositivo desde KDE Connect y comprueba que esté accesible.", "Pair a device in KDE Connect and make sure it is reachable.") : "";
    const button = (id: string, caption: string, style = "quiet", disabled = false) => `<button type="button" id="share-${id}" class="button ${style}" data-share-action="${id}" ${disabled ? "disabled" : ""}>${h(caption)}</button>`;
    dialog.innerHTML = `<header class="device-share-head"><div><p class="eyebrow">KDE Connect</p><h2 id="device-share-title">${t("Enviar a dispositivo", "Send to device")}</h2>${label ? `<p class="field-note">${h(label)}</p>` : ""}</div>
      <button type="button" id="share-close" class="icon-button" data-share-action="close" aria-label="${t("Cerrar", "Close")}" ${busy === "send" ? "disabled" : ""}>${icon("close")}</button></header>
      ${receipt ? `<section class="device-share-result" role="status"><strong>${receipt === "handedOff" ? t("Entregado a KDE Connect", "Handed off to KDE Connect") : t("Entrega a KDE Connect sin confirmar", "KDE Connect handoff unconfirmed")}</strong>
        <p>${receipt === "handedOff" ? t("Consulta KDE Connect para seguir la transferencia y confirmar la recepción.", "Check KDE Connect for transfer progress and receipt.") : t("Comprueba KDE Connect y el dispositivo antes de volver a enviar.", "Check KDE Connect and the device before sending again.")}</p></section>` : preview ? `<section class="device-share-review">
        <div><span class="field-note">${t("Dispositivo", "Device")}</span><strong>${h(deviceLabel(preview.device))}</strong></div>
        <div><span class="field-note">${preview.entries.length} ${t("elemento(s)", "item(s)")}</span><strong>${h(bytesLabel(preview.total))}</strong></div>
        <ul aria-label="${t("Contenido a enviar", "Content to send")}">${preview.entries.map(item => `<li><span>${h(item.name)}</span><small>${h(bytesLabel(item.bytes))}</small></li>`).join("")}</ul>
        <div class="device-share-footer">${button("back", t("Editar envío", "Edit share"), "quiet", Boolean(busy))}${button("send", busy ? t("Entregando a KDE Connect…", "Handing off to KDE Connect…") : t("Confirmar envío", "Confirm share"), "primary", Boolean(busy))}</div></section>` : `<section class="device-share-fields">
        <div class="filter-tabs" role="group" aria-label="${t("Qué quieres enviar", "What to share")}">${([['files', t("Archivos", "Files")], ['link', t("Enlace público", "Public link")], ['text', t("Texto", "Text")]] as const).map(([kind, caption]) => `<button type="button" id="share-${kind}" data-share-action="${kind}" aria-pressed="${mode === kind}" ${busy ? "disabled" : ""}>${h(caption)}</button>`).join("")}</div>
        ${mode === "files" ? `<div class="device-share-files">${files.length ? `<strong>${files.length} ${t("archivo(s) seleccionado(s)", "selected file(s)")}</strong><ul>${files.slice(0, 5).map(file => `<li>${h(file.name)}</li>`).join("")}</ul>${files.length > 5 ? `<p class="field-note">+${files.length - 5} ${t("más", "more")}</p>` : ""}` : `<p class="field-note">${t("Selecciona archivos en Biblioteca o abre un resultado de Descargas.", "Select files in Library or open an output in Downloads.")}</p>`}
          ${ids.length > maxFiles ? `<p class="field-note">${t("Para más de 50 archivos, prepara una colección portátil.", "For more than 50 files, prepare a portable collection.")}</p>` : ""}
          ${ids.length > 1 ? `${button("zip", t("Preparar ZIP de la selección", "Prepare selection ZIP"), "secondary", Boolean(busy))}<p class="field-note">${t("Podrás revisar el envío en Colección portátil cuando el ZIP esté listo.", "You can review the share in Portable collection once the ZIP is ready.")}</p>` : ""}</div>` : mode === "link"
          ? `<label for="share-url">${t("Enlace HTTP o HTTPS", "HTTP or HTTPS link")}<input id="share-url" type="url" maxlength="2048" value="${h(link)}" ${busy ? "disabled" : ""}></label><p class="field-note">${t("Para enlaces privados o con firma, elige el archivo local en Archivos.", "For private or signed links, choose the local file under Files.")}</p>`
          : `<label for="share-text-value">${t("Texto para el dispositivo", "Text for the device")}<textarea id="share-text-value" rows="4" maxlength="8192" ${busy ? "disabled" : ""}>${h(text)}</textarea></label>`}
        <label for="share-device">${t("Dispositivo emparejado", "Paired device")}<select id="share-device" ${busy || !devices.length ? "disabled" : ""}>${option("", t("Elige un dispositivo", "Choose a device"), deviceId)}${devices.map(device => option(device.id, deviceLabel(device), deviceId)).join("")}</select></label>
        ${help ? `<p class="field-note" role="status">${h(help)}</p>` : ""}
        ${status?.state === "ready" && !supported() ? `<p class="field-note">${t("Esta instalación de KDE Connect no admite este tipo de envío.", "This KDE Connect installation does not support this share type.")}</p>` : ""}
        ${busy === "discover" ? `<p class="field-note" role="status">${t("Consultando KDE Connect…", "Checking KDE Connect…")}</p>` : ""}
        <div class="device-share-footer">${button("refresh", t("Actualizar dispositivos", "Refresh devices"), "quiet", Boolean(busy))}
          ${status && ["missing", "offline", "incompatible"].includes(status.state) ? button("choose", t("Seleccionar ejecutable", "Choose executable"), "secondary", Boolean(busy)) : ""}
          ${button("review", busy === "review" ? t("Preparando revisión…", "Preparing review…") : t("Revisar envío", "Review share"), "primary", Boolean(busy || !deviceId || !supported() || !selection()))}</div></section>`}
      ${error ? `<p class="notice error device-share-error" role="alert">${h(error)}</p>` : ""}`;
    if (previous) dialog.querySelector<HTMLElement>(`#${CSS.escape(previous)}`)?.focus({ preventScroll: true });
  }

  async function perform(kind: NonNullable<typeof busy>, work: (revision: number) => Promise<void>) {
    if (busy) return;
    const revision = ++generation;
    busy = kind; error = ""; draw();
    try { await work(revision); }
    catch (cause) { if (revision === generation) error = errorLabel(cause); }
    finally { if (revision === generation) { busy = null; draw(); } }
  }

  function refresh(choose = false) {
    return perform("discover", async revision => {
      if (choose) await hooks.command("device_share_choose_cli");
      const next = await hooks.command<Status>("device_share_status");
      if (revision !== generation) return;
      status = next;
      if (!next.devices.some(device => device.id === deviceId)) deviceId = "";
    });
  }

  async function click(name: string) {
    if (name === "close" && busy !== "send") { dialog?.close(); return; }
    if (busy) return;
    if (name === "refresh" || name === "choose") { await refresh(name === "choose"); return; }
    if (name === "zip") { const current = ids.slice(); dialog?.close(); hooks.package(current); return; }
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
      const token = preview.token;
      await perform("send", async () => {
        preview = null;
        receipt = await hooks.command<typeof receipt>("device_share_send", { token });
      });
      dialog?.querySelector<HTMLElement>("#share-close")?.focus({ preventScroll: true });
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
        generation++; busy = null; discard(); text = link = "";
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
        else if (element.id === "share-text-value") text = element.value;
        else if (element.id === "share-device") deviceId = element.value;
        const review = dialog?.querySelector<HTMLButtonElement>("#share-review");
        if (review) review.disabled = Boolean(busy || !deviceId || !supported() || !selection());
      });
      document.body.append(dialog);
    }
    if (dialog.open) return;
    ids = [...new Set(input.ids ?? [])]; label = input.label ?? ""; link = input.url ?? ""; text = input.text ?? "";
    mode = input.text !== undefined ? "text" : input.url !== undefined || !ids.length ? "link" : "files";
    deviceId = ""; receipt = null; error = ""; status = null; busy = null; discard();
    draw(); dialog.showModal();
    void refresh();
  }

  return { open, isOpen: () => dialog?.open ?? false };
}
