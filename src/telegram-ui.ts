import { escapeHtml as h, type Copy } from "./contracts";
import { action, icon, option } from "./views-common";

type Authorization = { kind: "disconnected" | "initializing" | "phone" | "code" | "ready" | "closing" | "closed" } | { kind: "password"; hint: string } | { kind: "qr"; svg: string } | { kind: "unsupported"; state: string };
interface Account { id: string; authorization: Authorization; version?: string | null }
interface Status { installed: boolean; accounts: Account[] }
interface Chat { id: string; title: string; protected: boolean }
export interface TelegramAttachment { accountId: string; chatId: string; messageId: string; fileId: number; id: string; title: string; filename: string; kind: string; size: number | null; date: number; canSave: boolean; reason: string | null }
interface MediaPage { items: TelegramAttachment[]; nextMessageId: string; inspected: number }
interface TelegramHooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  t: Copy;
  render(): void;
  refresh(): Promise<void>;
  notify(message: string, kind?: "info" | "success" | "error"): void;
  showError(error: unknown): void;
  folder(): string;
  settings(): void;
}

const sizeLabel = (size: number | null, t: Copy) => size === null ? t("Tamaño desconocido", "Size unknown") : size < 1024 ** 2 ? `${(size / 1024).toFixed(1)} KiB` : `${(size / 1024 ** 2).toFixed(1)} MiB`;

export function createTelegramController(hooks: TelegramHooks) {
  const { t } = hooks;
  let status: Status | null = null;
  let account = "main";
  let newAccount = false;
  let authorization: Authorization = { kind: "disconnected" };
  let chats: Chat[] = [];
  let chatId = "";
  let items: TelegramAttachment[] = [];
  let nextMessageId = "0";
  let inspected = 0;
  let busy = false;
  let kind = "all";
  let query = "";
  let pageSize = 50;
  let startDate = "";
  let endDate = "";
  let minSize = "";
  let maxSize = "";
  let messageLink = "";
  let collection = "";
  let removeConsent = false;
  const selected = new Set<string>();

  async function perform(operation: () => Promise<void>) {
    if (busy) return;
    busy = true; hooks.render();
    try { await operation(); } catch (error) { hooks.showError(error); }
    finally { busy = false; hooks.render(); }
  }
  function resetMedia() { chats = []; chatId = ""; items = []; nextMessageId = "0"; inspected = 0; selected.clear(); }
  function setAuthorization(value: Authorization) {
    authorization = value;
    const current = status?.accounts.find(value => value.id === account);
    if (current) current.authorization = value;
  }
  async function load() {
    try {
      status = await hooks.command<Status>("telegram_status");
      const current = status.accounts.find(value => value.id === account) ?? status.accounts[0];
      if (current) { account = current.id; authorization = current.authorization; }
    } catch (error) { hooks.showError(error); }
    hooks.render();
  }
  async function refreshAuthorization() {
    const value = await hooks.command<Account>("telegram_authorization", { account });
    setAuthorization(value.authorization);
  }
  async function media(append: boolean) {
    if (!chatId) return;
    const result = await hooks.command<MediaPage>("telegram_media", { query: {
      accountId: account, chatId, fromMessageId: append ? nextMessageId : "0", limit: pageSize, kind, query,
      startDate: startDate ? Math.floor(new Date(`${startDate}T00:00:00`).getTime() / 1000) : null,
      endDate: endDate ? Math.floor(new Date(`${endDate}T23:59:59`).getTime() / 1000) : null,
      minSize: minSize ? Math.round(Number(minSize) * 1024 ** 2) : null,
      maxSize: maxSize ? Math.round(Number(maxSize) * 1024 ** 2) : null,
    } });
    if (!append) { items = []; inspected = 0; selected.clear(); }
    const merged = new Map(items.map(item => [item.id, item]));
    for (const item of result.items) merged.set(item.id, item);
    items = [...merged.values()].slice(0, 1000); inspected += result.inspected; nextMessageId = result.nextMessageId;
  }
  function authFields(): string {
    const disabled = busy ? "disabled" : "";
    switch (authorization.kind) {
      case "phone": return `<form id="telegram-authorize" class="settings-form"><label>${t("Teléfono con prefijo internacional", "Phone with country code")}<input id="telegram-auth-value" type="tel" autocomplete="tel" required maxlength="24"></label><div class="button-group"><button type="submit" class="button primary" ${disabled}>${t("Solicitar código", "Request code")}</button>${action("telegram-qr", t("Usar QR", "Use QR"), undefined, "secondary", disabled)}</div></form>`;
      case "code": return `<form id="telegram-authorize" class="settings-form"><label>${t("Código de Telegram", "Telegram code")}<input id="telegram-auth-value" autocomplete="one-time-code" required maxlength="128"></label><p class="field-note">${t("Consulta el método de entrega indicado por Telegram. El código no se guarda.", "Check the delivery method indicated by Telegram. The code is not saved.")}</p><button type="submit" class="button primary" ${disabled}>${t("Comprobar código", "Verify code")}</button></form>`;
      case "password": return `<form id="telegram-authorize" class="settings-form"><label>${t("Contraseña de verificación en dos pasos", "Two-step verification password")}<input id="telegram-auth-value" type="password" autocomplete="current-password" required maxlength="1024"></label>${authorization.hint ? `<p class="field-note">${h(authorization.hint)}</p>` : ""}<button type="submit" class="button primary" ${disabled}>${t("Verificar contraseña", "Verify password")}</button></form>`;
      case "qr": return `<section class="settings-form"><h2>${t("Autorizar desde Telegram", "Authorize in Telegram")}</h2><p>${t("Abre Telegram en otro dispositivo y escanea este QR desde Ajustes → Dispositivos → Vincular dispositivo. Después comprueba la autorización aquí.", "Open Telegram on another device and scan this QR in Settings → Devices → Link device. Then check authorization here.")}</p><div class="telegram-qr">${authorization.svg}</div>${action("telegram-auth-refresh", t("Comprobar autorización", "Check authorization"), "retry", "primary", disabled)}</section>`;
      case "initializing": return `<p role="status">${t("Inicializando la sesión protegida. Comprueba el siguiente paso.", "Initializing the protected session. Check the next step.")}</p>${action("telegram-auth-refresh", t("Comprobar autorización", "Check authorization"), "retry", "secondary", disabled)}`;
      case "closing": return `<p role="status">${t("Telegram está cerrando la sesión. La conexión se conserva hasta que confirme el cierre.", "Telegram is closing the session. The connection is kept until closure is confirmed.")}</p>${action("telegram-auth-refresh", t("Comprobar cierre", "Check closure"), "retry", "secondary", disabled)}`;
      case "unsupported": return `<div class="notice error">${icon("info")}<div><strong>${t("Este paso de autorización requiere atención", "This authorization step needs attention")}</strong><p>${h(authorization.state)}</p><p>${t("No se crean cuentas, se compran suscripciones ni se resuelven pasos no implementados automáticamente.", "Accounts are not created, subscriptions are not purchased and unsupported steps are not completed automatically.")}</p></div></div>`;
      case "disconnected": case "closed": return `<p class="field-note">${t("Conecta la cuenta elegida para consultar sus medios accesibles.", "Connect the selected account to browse its accessible media.")}</p>`;
      case "ready": return `<p role="status">${t("Cuenta autorizada para navegar y seleccionar medios.", "Account authorized to browse and select media.")}</p>`;
    }
  }
  function render(): string {
    const configured = status?.accounts.some(value => value.id === account) && !newAccount;
    const ready = authorization.kind === "ready";
    const selectedItems = items.filter(item => selected.has(item.id) && item.canSave);
    const disabled = busy ? "disabled" : "";
    const currentChat = chats.find(chat => chat.id === chatId);
    return `<div class="page-heading"><div><h1>Telegram</h1><p>${t("Medios de los chats a los que tu cuenta tiene acceso.", "Media from chats accessible to your account.")}</p></div>${action("telegram-settings", t("Componentes", "Components"), "settings", "quiet")}</div>
      ${status && !status.installed ? `<div class="notice">${icon("info")}<div><strong>${t("Se necesita TDLib", "TDLib is required")}</strong><p>${t("No hay un paquete TDLib verificado incluido. La configuración de desarrollo se explica en docs/TELEGRAM.md. No se ha iniciado ninguna sesión.", "No verified TDLib package is bundled. Development configuration is described in docs/TELEGRAM.md. No session has been started.")}</p></div></div>` : ""}
      <section class="settings-form"><div class="form-grid"><label>${t("Cuenta local", "Local account")}<select id="telegram-account" ${disabled}>${(status?.accounts ?? []).map(value => option(value.id, value.id, newAccount ? "" : account)).join("")}${option("__new", t("Configurar otra cuenta", "Configure another account"), newAccount || !status?.accounts.length ? "__new" : account)}</select></label></div>${!configured ? `<form id="telegram-connect" class="settings-form"><label>${t("Nombre local de la conexión", "Local connection label")}<input id="telegram-new-account" pattern="[a-z0-9-]{1,32}" maxlength="32" value="${h(account)}" required></label><div class="form-grid"><label>api_id<input id="telegram-api-id" type="number" min="1" max="2147483647" required autocomplete="off"></label><label>api_hash<input id="telegram-api-hash" type="password" required minlength="32" maxlength="32" autocomplete="off"></label></div><p class="field-note">${t("Usa las credenciales legítimas de tu aplicación de my.telegram.org. Se protegen en este perfil de Windows y no sustituyen la autorización de tu cuenta.", "Use your legitimate application credentials from my.telegram.org. They are protected for this Windows profile and do not replace account authorization.")}</p><button type="submit" class="button primary" ${!status?.installed || busy ? "disabled" : ""}>${t("Guardar y conectar", "Save and connect")}</button></form>` : `<div class="button-group">${action("telegram-connect", t("Conectar cuenta", "Connect account"), undefined, "secondary", !status?.installed || busy || ready ? "disabled" : "")}${ready ? action("telegram-disconnect", t("Desconectar", "Disconnect"), undefined, "quiet", disabled) : ""}</div>`}${authFields()}</section>
      ${ready ? `<form id="telegram-message-link" class="settings-form"><label>${t("Enlace de mensaje accesible", "Accessible message link")}<input id="telegram-link" type="url" value="${h(messageLink)}" placeholder="https://t.me/…" required></label><button type="submit" class="button secondary" ${disabled}>${t("Abrir medios del mensaje", "Open message media")}</button></form><section class="settings-form"><div class="button-group">${action("telegram-load-chats", t("Cargar chats accesibles", "Load accessible chats"), "retry", "secondary", disabled)}<label class="compact-label">${t("Chat o canal", "Chat or channel")}<select id="telegram-chat" ${disabled}>${option("", t("Elige un chat", "Choose a chat"), chatId)}${chats.map(chat => option(chat.id, chat.title, chatId)).join("")}</select></label></div>${currentChat?.protected ? `<p class="field-note">${t("Este chat restringe guardar contenido. Sus adjuntos no se pueden añadir a la cola.", "This chat restricts saving content. Its attachments cannot be queued.")}</p>` : ""}<p class="field-note">${t("Se muestran como máximo 100 chats de la lista principal. No se envían mensajes, se unen grupos ni se marca el historial como leído.", "Up to 100 main-list chats are shown. No messages are sent, groups joined or history marked as read.")}</p></section>
      <form id="telegram-media-filter" class="settings-form"><div class="form-grid"><label>${t("Tipo de medio", "Media type")}<select id="telegram-kind">${[["all",t("Todos","All")],["photo",t("Fotos","Photos")],["video",t("Vídeos","Videos")],["audio",t("Audio","Audio")],["document",t("Documentos","Documents")]].map(([value,label]) => option(value,label,kind)).join("")}</select></label><label>${t("Búsqueda en el chat", "Search this chat")}<input id="telegram-query" type="search" maxlength="240" value="${h(query)}"></label></div><details><summary>${t("Rango y límite de exploración", "Exploration range and limit")}</summary><div class="form-grid"><label>${t("Desde", "From")}<input id="telegram-start" type="date" value="${h(startDate)}"></label><label>${t("Hasta", "Until")}<input id="telegram-end" type="date" value="${h(endDate)}"></label><label>${t("Tamaño mínimo, MiB", "Minimum size, MiB")}<input id="telegram-min" type="number" min="0" step="0.1" value="${h(minSize)}"></label><label>${t("Tamaño máximo, MiB", "Maximum size, MiB")}<input id="telegram-max" type="number" min="0" step="0.1" value="${h(maxSize)}"></label><label>${t("Mensajes por página", "Messages per page")}<select id="telegram-page-size">${[25,50,100].map(size => option(String(size),String(size),String(pageSize))).join("")}</select></label></div><p class="field-note">${t("Los rangos se aplican a cada página inspeccionada. Los tamaños desconocidos no coinciden con un filtro por tamaño. Carga otra página para continuar; no se extrae todo el historial automáticamente.", "Ranges apply to each inspected page. Unknown sizes do not match a size filter. Load another page to continue; history is not extracted automatically.")}</p></details><button type="submit" class="button secondary" ${busy || !chatId ? "disabled" : ""}>${t("Previsualizar medios del canal o chat", "Preview channel or chat media")}</button></form>
      <section><div class="selection-toolbar">${action("telegram-select", t("Seleccionar disponibles", "Select available"), undefined, "quiet", disabled)}${action("telegram-clear", t("Quitar selección", "Clear selection"), undefined, "quiet", disabled)}<span>${selectedItems.length} ${t("seleccionados", "selected")} · ${inspected} ${t("mensajes inspeccionados", "messages inspected")}</span></div><div class="source-items">${items.length ? items.map(item => `<article class="source-item"><label><input type="checkbox" data-telegram-item="${h(item.id)}" aria-label="${h(t("Seleccionar ","Select ")+item.title)}" ${selected.has(item.id) ? "checked" : ""} ${!item.canSave || busy ? "disabled" : ""}></label><div class="source-item-main"><strong>${h(item.title)}</strong><span>${h(item.filename)} · ${h(item.kind)} · ${sizeLabel(item.size,t)} · ${item.date ? h(new Date(item.date*1000).toLocaleDateString()) : t("Fecha desconocida","Date unknown")}</span>${!item.canSave ? `<small>${h(item.reason ?? t("Telegram no permite guardar este adjunto.","Telegram does not allow saving this attachment."))}</small>` : ""}</div></article>`).join("") : `<div class="empty-state">${icon("image")}<h2>${t("Sin medios en esta selección", "No media in this selection")}</h2><p>${t("Abre un mensaje o elige un chat y previsualiza una página. Una página vacía con filtros no representa todo el historial.", "Open a message or choose a chat and preview a page. An empty filtered page does not represent the whole history.")}</p></div>`}</div>${nextMessageId !== "0" ? action("telegram-more", t("Cargar otra página", "Load another page"), undefined, "secondary", busy || items.length >= 1000 ? "disabled" : "") : ""}<div class="settings-form"><label>${t("Colección opcional", "Optional collection")}<input id="telegram-collection" value="${h(collection)}" maxlength="120"></label><p class="field-note path">${t("Destino","Destination")}: ${h(hooks.folder())}</p><p class="field-note">${t("Máximo 200 adjuntos por lote. Los permisos se vuelven a comprobar antes de guardar cada archivo.", "Up to 200 attachments per batch. Permissions are checked again before each file is saved.")}</p>${action("telegram-queue", t("Añadir adjuntos a Descargas", "Add attachments to Downloads"), "add", "primary", busy || !selectedItems.length || selectedItems.length > 200 ? "disabled" : "")}</div></section>` : ""}
      ${configured ? `<details class="inspector-secondary"><summary>${t("Cerrar sesión y eliminar conexión", "Log out and remove connection")}</summary><p>${t("Cierra la sesión en Telegram y elimina su clave protegida. Los archivos de Biblioteca se conservan; Telegram puede eliminar sus datos y medios de caché. Una cuenta aún no autorizada se cierra localmente. Conecta la cuenta antes de eliminarla.", "Logs out of Telegram and removes its protected key. Library files are kept; Telegram may delete its own data and cached media. An account that is not yet authorized is closed locally. Connect the account before removing it.")}</p><label><input id="telegram-remove-consent" type="checkbox" ${removeConsent ? "checked" : ""}>${t("Cerrar sesión y eliminar esta conexión", "Log out and remove this connection")}</label>${action("telegram-remove", t("Eliminar conexión", "Remove connection"), "close", "quiet danger", !removeConsent || busy ? "disabled" : "")}</details>` : ""}`;
  }
  function handleInput(element: HTMLElement): boolean {
    if (element instanceof HTMLSelectElement) {
      switch (element.id) {
        case "telegram-account": newAccount = element.value === "__new"; account = newAccount ? "new-account" : element.value; authorization = status?.accounts.find(value => value.id === account)?.authorization ?? { kind: "disconnected" }; resetMedia(); hooks.render(); return true;
        case "telegram-chat": chatId = element.value; items = []; inspected = 0; nextMessageId = "0"; selected.clear(); hooks.render(); return true;
        case "telegram-kind": kind = element.value; return true;
        case "telegram-page-size": pageSize = Number(element.value); return true;
      }
    }
    if (element instanceof HTMLInputElement) {
      if (element.dataset.telegramItem) { if (element.checked) selected.add(element.dataset.telegramItem); else selected.delete(element.dataset.telegramItem); hooks.render(); return true; }
      switch (element.id) {
        case "telegram-query": query = element.value; return true;
        case "telegram-start": startDate = element.value; return true;
        case "telegram-end": endDate = element.value; return true;
        case "telegram-min": minSize = element.value; return true;
        case "telegram-max": maxSize = element.value; return true;
        case "telegram-link": messageLink = element.value; return true;
        case "telegram-collection": collection = element.value; return true;
        case "telegram-remove-consent": removeConsent = element.checked; hooks.render(); return true;
      }
    }
    return false;
  }
  async function handleClick(name: string): Promise<boolean> {
    switch (name) {
      case "telegram-settings": hooks.settings(); break;
      case "telegram-connect": await perform(async () => { const result = await hooks.command<Account>("telegram_connect", { account, apiId: null, apiHash: null }); setAuthorization(result.authorization); }); break;
      case "telegram-auth-refresh": await perform(refreshAuthorization); break;
      case "telegram-qr": await perform(async () => { const result = await hooks.command<Account>("telegram_authorize", { account, action: { kind: "qr" } }); setAuthorization(result.authorization); }); break;
      case "telegram-load-chats": await perform(async () => { chats = await hooks.command<Chat[]>("telegram_chats", { account, limit: 100 }); }); break;
      case "telegram-more": await perform(() => media(true)); break;
      case "telegram-select": for (const item of items.filter(item => item.canSave).slice(0,200)) selected.add(item.id); hooks.render(); break;
      case "telegram-clear": selected.clear(); hooks.render(); break;
      case "telegram-disconnect": await perform(async () => { await hooks.command("telegram_disconnect", { account, remove: false }); setAuthorization({ kind: "disconnected" }); resetMedia(); }); break;
      case "telegram-remove": if (removeConsent) await perform(async () => { await hooks.command("telegram_disconnect", { account, remove: true }); authorization = { kind: "disconnected" }; resetMedia(); removeConsent = false; await load(); }); break;
      case "telegram-queue": {
        const attachments = items.filter(item => item.canSave && selected.has(item.id));
        if (!attachments.length || attachments.length > 200) break;
        await perform(async () => { await hooks.command("queue_telegram", { attachments, collection: collection.trim(), clientRequest: crypto.randomUUID() }); await hooks.refresh(); selected.clear(); hooks.notify(t("Adjuntos añadidos a Descargas.", "Attachments added to Downloads."), "success"); });
      } break;
      default: return false;
    }
    return true;
  }
  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    const input = (id: string) => { const element = document.getElementById(id); if (!(element instanceof HTMLInputElement)) throw new Error(t("Falta un campo del formulario.", "A form field is missing.")); return element.value; };
    switch (form.id) {
      case "telegram-connect": {
        const args = { account: input("telegram-new-account"), apiId: Number(input("telegram-api-id")), apiHash: input("telegram-api-hash") };
        await perform(async () => { const result = await hooks.command<Account>("telegram_connect", args); account = result.id; authorization = result.authorization; newAccount = false; status = await hooks.command<Status>("telegram_status"); });
      } break;
      case "telegram-authorize": {
        if (!["phone","code","password"].includes(authorization.kind)) break;
        const kind = authorization.kind, value = input("telegram-auth-value");
        await perform(async () => { const result = await hooks.command<Account>("telegram_authorize", { account, action: { kind, value } }); setAuthorization(result.authorization); });
      } break;
      case "telegram-message-link": await perform(async () => { const result = await hooks.command<MediaPage>("telegram_message_link", { account, url: messageLink }); items = result.items; nextMessageId = "0"; inspected = result.inspected; selected.clear(); }); break;
      case "telegram-media-filter": await perform(() => media(false)); break;
      default: return false;
    }
    return true;
  }
  return { render, load, handleInput, handleClick, handleSubmit, openLink(url: string) { messageLink = url; hooks.render(); void load(); } };
}
