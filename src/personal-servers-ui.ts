import { escapeHtml as h, type Copy } from "./contracts";
import { bytesLabel } from "./presentation";
import { action, icon } from "./views-common";

type Kind = "webdav" | "sftp" | "nas";
const kinds: Kind[] = ["webdav", "sftp", "nas"];
interface Stamp { size: number | null; modified: string | null; etag: string | null }
export interface PersonalSelection { serverId: string; path: string[]; stamp: Stamp }
interface Entry { path: string[]; directory: boolean; stamp: Stamp; mime: string | null }
interface Server { id: string; name: string; kind: Kind; location: string }
interface Reference { id: string; serverName: string; selection: PersonalSelection }
interface ServerStatus { servers: Server[]; references: Reference[] }
interface Challenge { challenge: number; fingerprint: string }

interface Hooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  render(): void;
  refresh(): Promise<void>;
  notify(message: string, kind?: "info" | "success" | "error"): void;
  folder(): string;
  chooseFolder(): void;
  openBrowser(): void;
  openReferences(): void;
  t: Copy;
}

export function createPersonalServers(hooks: Hooks) {
  const { t } = hooks;
  let status: ServerStatus | null = null;
  let editor: Kind | null = null;
  let draft = { name: "", root: "", username: "", host: "", port: "22", identity: "" };
  let trust: Challenge | null = null;
  let serverId = "";
  let path: string[] = [];
  let entries: Entry[] | null = null;
  let selected = new Set<number>();
  let referenceId = "";
  let reviewing: Reference | null = null;
  let error = "";
  let busy: "read" | "probe" | "save" | "copy" | null = null;
  let sequence = 0;
  let requestId = 0;

  const nameFor = (kind: Kind) => kind === "webdav" ? "Nextcloud / WebDAV"
    : kind === "sftp" ? "SFTP" : t("Carpeta de red / NAS", "Network folder / NAS");
  const activeServer = () => status?.servers.find(server => server.id === serverId);
  const activeReference = () => status?.references.find(reference => reference.id === referenceId);
  const disabled = () => busy ? "disabled" : "";
  const label = (value: string[]) => value.join("/");
  const reviewedIndex = () => {
    if (!reviewing || !entries) return -1;
    const target = label(reviewing.selection.path);
    return entries.findIndex(entry => !entry.directory && label(entry.path) === target);
  };
  const cleanDraft = () => {
    draft = { name: "", root: "", username: "", host: "", port: "22", identity: "" };
    editor = null;
    trust = null;
  };

  async function run(kind: NonNullable<typeof busy>, task: () => Promise<void>) {
    if (busy) return;
    busy = kind;
    requestId = Date.now() * 1000 + ++sequence % 1000;
    error = "";
    hooks.render();
    try { await task(); }
    catch (failure) {
      error = typeof failure === "string" ? failure : failure instanceof Error ? failure.message
        : t("No se pudo completar la operación.", "Could not complete the operation.");
    } finally { busy = null; hooks.render(); }
  }

  async function load() {
    await run("save", async () => { status = await hooks.command<ServerStatus>("personal_servers_status"); });
  }

  async function browse(id: string, folder: string[], reference: Reference | null = null) {
    if (busy) return;
    serverId = id;
    path = folder;
    reviewing = reference;
    entries = null;
    selected.clear();
    await run("read", async () => {
      entries = await hooks.command<Entry[]>("personal_server_browse", { serverId: id, path: folder, requestId });
      const index = reviewedIndex();
      if (index >= 0) selected.add(index);
    });
  }

  function input(id: string, text: string, value: string, attributes = "") {
    return `<label>${text}<input id="personal-${id}" value="${h(value)}" ${attributes} ${disabled()}></label>`;
  }

  function setup() {
    return `<div class="button-group" role="group" aria-label="${t("Tipo de conexión", "Connection type")}">
      ${kinds.map(kind => action("personal-add", nameFor(kind), "add", "secondary",
        `data-kind="${kind}" aria-pressed="${editor === kind}" ${disabled()}`)).join("")}
    </div>${editor ? `<form id="personal-configure" class="personal-server-form">
      ${input("name", t("Nombre", "Name"), draft.name, 'required maxlength="100"')}
      ${editor === "webdav" ? `
        ${input("root", t("URL HTTPS de la raíz WebDAV", "HTTPS URL of the WebDAV root"), draft.root, 'type="url" required placeholder="https://…"')}
        ${input("username", t("Usuario", "Username"), draft.username, 'required autocomplete="off"')}
        <label>${t("Contraseña de aplicación", "App password")}<input id="personal-password" type="password" required autocomplete="new-password" ${disabled()}></label>
      ` : editor === "sftp" ? `
        ${input("host", t("Servidor", "Host"), draft.host, 'required spellcheck="false"')}
        ${input("port", t("Puerto", "Port"), draft.port, 'type="number" min="1" max="65535" required')}
        ${input("username", t("Usuario", "Username"), draft.username, 'required autocomplete="off"')}
        ${input("root", t("Carpeta remota absoluta", "Absolute remote folder"), draft.root, 'required placeholder="/media"')}
        <div class="personal-key"><span class="path">${h(draft.identity || t("Elige una clave privada local de OpenSSH", "Choose a local OpenSSH private key"))}</span>
        ${action("personal-key", t("Elegir clave", "Choose key"), "file", "secondary", disabled())}</div>
      ` : `<p class="field-note">${t("Elige una carpeta montada o una ruta UNC a la que Windows ya tenga acceso.", "Choose a mounted folder or UNC path that Windows can already access.")}</p>`}
      ${trust && editor === "sftp" ? `<div class="personal-trust" role="status"><strong>${t("Huella del servidor", "Server fingerprint")}</strong><code>${h(trust.fingerprint)}</code>
        <p class="field-note">${t("Confirma que coincide con la huella de tu servidor.", "Confirm that it matches your server’s fingerprint.")}</p>
        ${action("personal-trust", t("Confiar y guardar", "Trust and save"), "check", "primary", disabled())}</div>`
        : `<button class="button primary" type="submit" ${busy || editor === "sftp" && !draft.identity ? "disabled" : ""}>${editor === "nas" ? t("Elegir carpeta", "Choose folder") : editor === "sftp" ? t("Comprobar huella", "Check fingerprint") : t("Guardar conexión", "Save connection")}</button>`}
    </form>` : ""}`;
  }

  function feedback() {
    return `${error ? `<div class="file-source-error" role="alert">${icon("error")}<span>${h(error)}</span></div>` : ""}
      ${busy ? `<div class="file-source-progress" role="status"><span>${busy === "probe" ? t("Consultando la huella…", "Reading the fingerprint…") : busy === "read" ? t("Consultando la carpeta…", "Reading the folder…") : t("Guardando…", "Saving…")}</span>${busy === "read" || busy === "probe" ? action("personal-cancel", t("Cancelar", "Cancel"), "close", "quiet") : ""}</div>` : ""}`;
  }

  function serverList(settings: boolean) {
    return `<div class="file-source-entries">${(status?.servers ?? []).map(server => `
      <div class="personal-server-row ${server.id === serverId ? "is-selected" : ""}">
        <button class="file-source-entry" data-action="personal-browse" data-id="${h(server.id)}" ${disabled()}>
          <span><strong>${h(server.name)}</strong><small>${h(nameFor(server.kind))} · ${h(server.location)}</small></span>${icon("arrow")}
        </button>
        ${settings ? action("personal-remove", t("Desconectar", "Disconnect"), undefined, "quiet", `data-id="${h(server.id)}" ${disabled()}`) : ""}
      </div>`).join("")}</div>`;
  }

  function destination() {
    return `<div class="file-source-destination"><span>${t("Destino local", "Local destination")}</span><strong title="${h(hooks.folder())}">${h(hooks.folder())}</strong>${action("personal-destination", t("Cambiar", "Change"), "folder", "quiet", disabled())}</div>`;
  }

  function review() {
    if (!selected.size || !entries) return "";
    const sizes = [...selected].map(index => entries?.[index]?.stamp.size ?? null);
    const total = sizes.reduce<number>((sum, size) => sum + (size ?? 0), 0);
    const updating = selected.size === 1 && selected.has(reviewedIndex());
    return `<div class="file-source-selection personal-review"><div class="file-source-selection-title"><strong>${selected.size} ${selected.size === 1 ? t("archivo seleccionado", "file selected") : t("archivos seleccionados", "files selected")}</strong>
      <span>${bytesLabel(total)}${sizes.includes(null) ? ` + ${t("tamaño desconocido", "unknown size")}` : ""}</span></div>
      ${destination()}<div class="button-group">${action("personal-index", updating ? t("Actualizar referencia", "Update reference") : t("Indexar referencia", "Index reference"), "link", "secondary", disabled())}${action("personal-copy", t("Copiar a mi biblioteca", "Copy to my library"), "downloads", "primary", disabled())}</div>
      <p class="field-note">${updating
        ? t("Copiar guarda el archivo actual. Actualizar guarda sus metadatos en la referencia.", "Copy saves the current file. Update saves its metadata to the reference.")
        : t("Las referencias ocupan solo espacio de índice y requieren conexión al servidor.", "References only occupy index space and require a server connection.")}</p></div>`;
  }

  function referenceReview(server: Server) {
    if (!reviewing) return "";
    const stored = reviewing.selection.stamp;
    const current = entries?.[reviewedIndex()]?.stamp;
    const different = current && (stored.size !== current.size || stored.modified !== current.modified || stored.etag !== current.etag);
    const changedVersion = current?.etag && stored.etag && current.etag !== stored.etag;
    const message = busy === "read" ? t("Consultando la carpeta…", "Reading the folder…")
      : !entries ? t("Actualiza la carpeta para volver a consultar este archivo.", "Refresh the folder to check this file again.")
      : !current ? t("El archivo ya no aparece en esta carpeta.", "The file is no longer listed in this folder.")
      : changedVersion ? t("El identificador de versión es distinto.", "The version identifier differs.")
      : different ? t("Los datos del archivo en el servidor cambiaron.", "The server reports different file metadata.")
      : t("Los datos visibles coinciden con la referencia.", "The visible metadata matches the reference.");
    const describe = (stamp: Stamp) => {
      const size = stamp.size === null ? t("Tamaño desconocido", "Unknown size") : bytesLabel(stamp.size);
      const time = stamp.modified === null ? NaN : server.kind === "sftp" ? Number(stamp.modified) * 1000 : Date.parse(stamp.modified);
      const date = Number.isFinite(time) ? new Intl.DateTimeFormat(t("es", "en"), { dateStyle: "medium", timeStyle: "short" }).format(time) : "";
      return h(date ? `${size} · ${date}` : size);
    };
    return `<section class="personal-reference-review" aria-labelledby="personal-reference-review-title">
      <header><div><span class="file-source-eyebrow">${t("Revisión de referencia", "Reference review")}</span>
        <h3 id="personal-reference-review-title" tabindex="-1">${h(reviewing.selection.path.at(-1) ?? "")}</h3></div>
        ${action("personal-references", t("Volver a referencias", "Back to references"), "link", "quiet", disabled())}</header>
      <p role="status">${message}</p>
      ${current ? `<dl><div><dt>${t("Referencia guardada", "Saved reference")}</dt><dd>${describe(stored)}</dd></div>
        <div><dt>${t("Archivo en el servidor", "File on server")}</dt><dd>${describe(current)}</dd></div></dl>` : ""}
    </section>`;
  }

  function renderBrowser() {
    const server = activeServer();
    return `<section class="personal-servers">${reviewing ? "" : `<div class="page-heading"><div><h2>${t("Servidores personales", "Personal servers")}</h2>
      <p>${t("Explora una raíz autorizada. Guarda referencias remotas o copia archivos a Biblioteca.", "Browse an authorized root. Save remote references or copy files to Library.")}</p></div>
      ${action("personal-references", t("Referencias guardadas", "Saved references"), "link", "quiet", disabled())}</div>
      ${setup()}${serverList(false)}`}${feedback()}
      ${server ? `<div class="file-source-results"><div class="file-source-result-heading"><div><span class="file-source-eyebrow">${t("Origen remoto", "Remote source")}</span><h2>${h(server.name)}</h2><p class="path">${h(server.location)}</p>${path.length ? `<p class="field-note">${t("Carpeta", "Folder")} · ${h(label(path))}</p>` : ""}</div>
        <div class="button-group">${path.length ? action("personal-parent", t("Subir carpeta", "Parent folder"), undefined, "quiet", disabled()) : ""}${action("personal-reload", t("Actualizar", "Refresh"), "retry", "quiet", disabled())}</div></div>
        ${referenceReview(server)}
        ${entries?.length ? `<div class="file-source-list" role="group" aria-label="${t("Archivos del servidor", "Server files")}">${entries.map((entry, index) => {
          const name = h(entry.path.at(-1) ?? "");
          return entry.directory ? `<button class="file-source-entry" data-action="personal-folder" data-index="${index}" ${disabled()}><span>${icon("folder")} ${name}</span>${icon("arrow")}</button>`
            : `<label class="file-source-row ${selected.has(index) ? "is-selected" : ""}"><input id="personal-file-${index}" type="checkbox" data-personal-file="${index}" ${selected.has(index) ? "checked" : ""} ${disabled()}>
              <span class="file-source-file-icon">${icon("file")}</span><span class="file-source-row-main"><strong>${name}</strong>${entry.mime ? `<small>${h(entry.mime)}</small>` : ""}</span>
              <span class="file-source-size">${entry.stamp.size === null ? t("Tamaño desconocido", "Unknown size") : bytesLabel(entry.stamp.size)}</span></label>`;
        }).join("")}</div>` : entries ? `<p class="file-source-empty">${t("Esta carpeta no contiene archivos visibles.", "This folder has no visible files.")}</p>` : ""}
        ${review()}</div>` : ""}</section>`;
  }

  function renderConnections() {
    return `<section class="cloud-connections personal-servers"><div class="page-heading"><div><h2>${t("Servidores personales", "Personal servers")}</h2>
      <p>${t("Configura raíces de solo lectura y accede con tus credenciales.", "Configure read-only roots and connect with your credentials.")}</p></div></div>
      ${setup()}${feedback()}${serverList(true)}</section>`;
  }

  function renderReferences() {
    const chosen = activeReference();
    const connected = (ref: Reference) => status?.servers.some(server => server.id === ref.selection.serverId);
    return `<section class="file-sources personal-servers"><div class="page-heading"><div><h1>${t("Referencias remotas", "Remote references")}</h1>
      <p>${t("Las referencias apuntan al servidor. Para usar un archivo sin conexión, guarda una copia local.", "References point to the server. To use a file offline, save a local copy.")}</p></div>${action("personal-open", t("Abrir servidores", "Open servers"), "folder", "secondary", disabled())}</div>
      ${feedback()}${status?.references.length ? `<div class="file-source-entries">${status.references.map(reference => `
        <div class="personal-server-row ${reference.id === referenceId ? "is-selected" : ""}">
        <button class="file-source-entry" data-action="personal-reference" data-id="${h(reference.id)}" aria-pressed="${reference.id === referenceId}" ${disabled()}>
          <span><strong>${h(reference.selection.path.at(-1) ?? "")}</strong><small>${h(reference.serverName)} · ${h(label(reference.selection.path))} · ${connected(reference) ? t("Remota", "Remote") : t("Servidor desconectado", "Server disconnected")}</small></span>${icon("link")}</button>
        ${action("personal-remove-reference", t("Quitar referencia", "Remove reference"), undefined, "quiet", `data-id="${h(reference.id)}" ${disabled()}`)}</div>`).join("")}</div>`
        : !busy ? `<p class="file-source-empty">${t("Aún no hay referencias guardadas.", "There are no saved references yet.")}</p>` : ""}
      ${chosen ? `<div class="file-source-selection personal-review"><div class="file-source-selection-title"><strong>${h(chosen.selection.path.at(-1) ?? "")}</strong><span>${chosen.selection.stamp.size === null ? t("Tamaño desconocido", "Unknown size") : bytesLabel(chosen.selection.stamp.size)}</span></div>
        ${destination()}<div class="button-group">
          ${action("personal-review-reference", t("Revisar en servidor", "Review on server"), "folder", "secondary", busy || !connected(chosen) ? "disabled" : "")}
          ${action("personal-copy-reference", t("Obtener copia local", "Get local copy"), "downloads", "primary", busy || !connected(chosen) ? "disabled" : "")}
        </div></div>` : ""}</section>`;
  }

  function handleInput(element: HTMLElement): boolean {
    if (!(element instanceof HTMLInputElement) || !element.id.startsWith("personal-")) return false;
    const key = element.id.slice("personal-".length);
    if (key === "name" || key === "root" || key === "username" || key === "host" || key === "port") {
      draft[key] = element.value;
      if (key !== "name" && trust) { trust = null; hooks.render(); }
      return true;
    }
    if (element.dataset.personalFile !== undefined) {
      const index = Number(element.dataset.personalFile);
      if (entries?.[index] && !entries[index].directory) {
        if (element.checked && selected.size < 200) selected.add(index); else selected.delete(index);
        hooks.render();
      }
      return true;
    }
    return key === "password";
  }

  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    if (form.id !== "personal-configure") return false;
    if (busy || !editor || !form.reportValidity()) return true;
    if (editor === "webdav") {
      const password = form.querySelector<HTMLInputElement>("#personal-password")?.value ?? "";
      await run("save", async () => {
        status = await hooks.command<ServerStatus>("personal_server_webdav", { name: draft.name, root: draft.root, username: draft.username, password });
        cleanDraft();
      });
    } else if (editor === "nas") {
      await run("save", async () => {
        const result = await hooks.command<ServerStatus | null>("personal_server_nas", { name: draft.name });
        if (result) { status = result; cleanDraft(); }
      });
    } else if (draft.identity && !trust) {
      await run("probe", async () => {
        trust = await hooks.command<Challenge>("personal_server_probe_sftp", {
          host: draft.host, port: Number(draft.port), username: draft.username, root: draft.root, identity: draft.identity, requestId,
        });
      });
    }
    return true;
  }

  async function handleClick(name: string, element: HTMLElement) {
    if (!name.startsWith("personal-")) return false;
    if (name === "personal-cancel") {
      if (busy === "probe" || busy === "read") await hooks.command("cancel_file_source_inspection", { requestId });
      return true;
    }
    if (busy) return true;
    const id = element.dataset.id ?? "";
    const paths = [...selected].map(index => entries?.[index]?.path).filter(value => value !== undefined);
    switch (name) {
      case "personal-open": reviewing = null; hooks.openBrowser(); await load(); break;
      case "personal-references": hooks.openReferences(); break;
      case "personal-destination": hooks.chooseFolder(); break;
      case "personal-add": {
        const kind = element.dataset.kind;
        if (kind === "webdav" || kind === "sftp" || kind === "nas") {
          const previous = editor;
          cleanDraft();
          if (previous !== kind) editor = kind;
          error = "";
          hooks.render();
          document.getElementById("personal-name")?.focus();
        }
      } break;
      case "personal-browse":
        if (status?.servers.some(server => server.id === id)) {
          hooks.openBrowser();
          await browse(id, []);
        }
        break;
      case "personal-parent": if (serverId) await browse(serverId, path.slice(0, -1)); break;
      case "personal-reload": if (serverId) await browse(serverId, path, reviewing); break;
      case "personal-folder": {
        const entry = entries?.[Number(element.dataset.index)];
        if (serverId && entry?.directory) await browse(serverId, entry.path);
      } break;
      case "personal-key": await run("save", async () => {
        const key = await hooks.command<string | null>("personal_server_choose_key");
        if (key) { draft.identity = key; trust = null; }
      }); break;
      case "personal-trust": {
        const confirmed = trust;
        if (confirmed) await run("save", async () => {
          trust = null;
          status = await hooks.command<ServerStatus>("personal_server_trust_sftp", { name: draft.name, ...confirmed });
          cleanDraft();
        });
      } break;
      case "personal-reference": referenceId = id; hooks.render(); break;
      case "personal-remove":
      case "personal-remove-reference": await run("save", async () => {
        status = await hooks.command<ServerStatus>("personal_server_remove", { id, reference: name === "personal-remove-reference" });
        if (id === referenceId) referenceId = "";
        if (id === serverId) { serverId = ""; entries = null; path = []; reviewing = null; selected.clear(); }
      }); break;
      case "personal-review-reference": {
        const reference = activeReference();
        if (!reference || !status?.servers.some(server => server.id === reference.selection.serverId)) break;
        hooks.openBrowser();
        await browse(reference.selection.serverId, reference.selection.path.slice(0, -1), reference);
        document.getElementById("personal-reference-review-title")?.focus();
      } break;
      case "personal-index": if (paths.length) await run("save", async () => {
        status = await hooks.command<ServerStatus>("personal_server_index", { serverId, paths });
        if (reviewing) reviewing = status.references.find(ref => ref.id === reviewing?.id) ?? null;
        selected.clear();
        hooks.notify(t("Referencias guardadas en Biblioteca.", "References saved in Library."), "success");
      }); break;
      case "personal-copy":
      case "personal-copy-reference": {
        const reference = activeReference();
        if (name === "personal-copy" ? paths.length > 0 : reference) await run("copy", async () => {
          await hooks.command("personal_server_copy", { serverId: name === "personal-copy" ? serverId : null,
            paths: name === "personal-copy" ? paths : null, referenceId: name === "personal-copy-reference" ? reference?.id : null,
            clientRequest: crypto.randomUUID() });
          selected.clear();
          referenceId = "";
          await hooks.refresh();
          hooks.notify(t("Copia añadida a Descargas.", "Copy added to Downloads."), "success");
        });
      } break;
      default: return false;
    }
    return true;
  }

  return { load, renderBrowser, renderConnections, renderReferences, handleInput, handleSubmit, handleClick };
}
