import "./personal-archives.css";
import type { ImportPreview } from "./audio-library-ui";
import { escapeHtml as h, type Copy } from "./contracts";
import { bytesLabel, isCancelled } from "./presentation";
import { action, icon, option, type Icon } from "./views-common";

const providers = {
  instagram: "Instagram", facebook: "Facebook", x: "X", spotify: "Spotify", linkedin: "LinkedIn",
};
type Provider = keyof typeof providers;
type Category = "posts" | "saved" | "media" | "likes" | "bookmarks" | "listening" | "playlists" | "contacts" | "messages";
const categoryIcons: Record<Category, Icon> = {
  posts: "document", saved: "folder", media: "image", likes: "heart", bookmarks: "link",
  listening: "wave", playlists: "list", contacts: "lock", messages: "lock",
};
const categoryKeys = Object.keys(categoryIcons) as Category[];
type Availability = "absent" | "unsupported" | "empty" | "available" | "requiresSelection";
type Format = "json" | "csv" | "html" | "media" | "xAssignment" | "unsupported";
interface CategoryPreview {
  category: Category;
  sensitive: boolean;
  availability: Availability;
  files: number;
  bytes: number;
  records: number | null;
  start: number | null;
  end: number | null;
  formats: Format[];
}
interface Preview { source: string; fingerprint: string; bytes: number; categories: CategoryPreview[] }
interface Archive {
  id: string;
  provider: Provider;
  account: string;
  destination: string;
  source: string;
  fingerprint: string;
  categories: Partial<Record<Category, number>>;
  formats: Partial<Record<Category, Format[]>>;
  mediaBytes: number;
  pendingRemoval: boolean;
}
interface Row {
  id: string;
  source: string;
  title: string;
  text: string;
  time: number | null;
  link: string | null;
  author: string | null;
  threadParent: string | null;
  playedMs: number | null;
  trackCount: number;
  media: string | null;
}
interface RecordPage {
  rows: Row[];
  total: number;
  start: number | null;
  end: number | null;
  playedMs: number | null;
  completions: number | null;
  completionKnown: number;
  ended: number | null;
  artists: [string, number][];
}
interface Hooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  render(): void;
  notify(message: string, kind: "success" | "info"): void;
  prepareLinks(links: string[]): Promise<void>;
  openPlaylist(preview: ImportPreview): void;
  returnToHeading(): void;
  t: Copy;
}

const PAGE = 50;
const privateCategory = (category: Category) => category === "contacts" || category === "messages";
const selectable = (item: CategoryPreview) => item.availability === "available" || item.availability === "requiresSelection";
const formatName = (format: Format) => format === "xAssignment" ? "X · JS/JSON" : format.toUpperCase();

export function createPersonalArchives(hooks: Hooks) {
  const { t } = hooks;
  let provider: Provider = "instagram";
  let archives: Archive[] = [];
  let preview: Preview | null = null;
  let archiveId = "";
  let category: Category | null = null;
  let result: RecordPage | null = null;
  let query = "", offset = 0, requestId = 0;
  let privateOpen = false, confirming = false, privateExport = false;
  let loaded = false, busy = false, visible = false, adding = false;
  let error = "";
  let draft = { account: "", destination: "", from: "", to: "" };
  const included = new Set<Category>();
  const selected = new Map<string, Row>();
  const name = (value: Category) => ({
    posts: t("Publicaciones", "Posts"), saved: t("Guardados", "Saved"), media: t("Multimedia", "Media"),
    likes: t("Me gusta", "Likes"), bookmarks: t("Marcadores", "Bookmarks"), listening: t("Escuchas", "Listening"),
    playlists: t("Listas", "Playlists"), contacts: t("Contactos", "Contacts"), messages: t("Mensajes", "Messages"),
  })[value];
  const availability = (value: Availability) => ({
    absent: t("No incluido", "Not included"), unsupported: t("Formato no compatible", "Unsupported format"),
    empty: t("Vacío", "Empty"), available: t("Disponible", "Available"),
    requiresSelection: t("Se lee al importar", "Read on import"),
  })[value];
  const current = () => archives.find(item => item.id === archiveId);
  const disabled = () => busy ? "disabled" : "";
  const number = (value: number, digits = 0) => new Intl.NumberFormat(t("es", "en"), { maximumFractionDigits: digits }).format(value);
  const day = (value: number) => new Intl.DateTimeFormat(t("es", "en"), { dateStyle: "medium", timeZone: "UTC" }).format(value);
  const dates = (start: number | null, end: number | null) => start === null || end === null ? "" : start === end ? day(start) : `${day(start)} – ${day(end)}`;

  function resetRecords(next: Category | null) {
    category = next;
    query = "";
    offset = 0;
    result = null;
    privateOpen = false;
    privateExport = false;
    selected.clear();
  }

  async function run(task: () => Promise<void>) {
    if (busy) return;
    error = "";
    busy = true;
    hooks.render();
    try { await task(); }
    catch (cause) { if (!isCancelled(cause)) error = (cause instanceof Error ? cause.message : String(cause)).slice(0, 500); }
    finally { busy = false; requestId = 0; hooks.render(); }
  }

  async function refreshList() {
    archives = await hooks.command<Archive[]>("personal_archive_list");
    loaded = true;
  }

  async function load() {
    visible = true;
    if (!loaded) await run(refreshList);
  }

  function leave() {
    visible = false;
    if (category && privateCategory(category)) resetRecords(category);
  }

  async function read() {
    if (!archiveId || !category || (privateCategory(category) && !privateOpen)) return;
    const page = await hooks.command<RecordPage>("personal_archive_records", {
      id: archiveId, category, consent: privateOpen, query, offset,
    });
    if (visible && (!category || !privateCategory(category) || privateOpen)) result = page;
  }

  async function choose() {
    await run(async () => {
      requestId = Date.now() * 1000 + 401;
      hooks.render();
      const file = await hooks.command<Preview | null>("personal_archive_choose", { provider, requestId });
      if (!file) return;
      preview = file;
      included.clear();
      for (const item of file.categories) if (!item.sensitive && selectable(item)) included.add(item.category);
    });
  }

  function categoryRow(item: CategoryPreview) {
    const info = [availability(item.availability)];
    if (item.files) info.push(`${number(item.files)} ${item.files === 1 ? t("archivo", "file") : t("archivos", "files")}`, bytesLabel(item.bytes));
    if (item.records !== null) info.push(`${number(item.records)} ${item.records === 1 ? t("registro", "record") : t("registros", "records")}`);
    const range = dates(item.start, item.end);
    if (range) info.push(range);
    if (item.formats.length && selectable(item)) info.push(item.formats.filter(format => format !== "unsupported").map(formatName).join(", "));
    return `<label class="archive-option">
      <input type="checkbox" name="archive-category" value="${item.category}" ${included.has(item.category) ? "checked" : ""} ${!selectable(item) || busy ? "disabled" : ""}>
      <span><strong>${name(item.category)}</strong><small>${h(info.join(" · "))}</small></span>
    </label>`;
  }

  function importForm() {
    return `<section class="archive-panel" aria-labelledby="archive-import-title">
      <div class="archive-heading"><h2 id="archive-import-title">${t("Importar exportación", "Import an export")}</h2>
        ${action("archive-close-form", t("Cerrar formulario", "Close form"), undefined, "secondary", disabled())}</div>
      <p>${t("Elige una exportación oficial guardada en este equipo.", "Choose an official export saved on this device.")}</p>
      <div class="archive-choose">
        <label for="archive-provider">${t("Proveedor", "Provider")}
          <select id="archive-provider" ${disabled()}>${Object.entries(providers).map(([id, label]) => option(id, label, provider)).join("")}</select>
        </label>
        ${action("archive-choose", preview ? t("Cambiar archivo", "Change file") : t("Elegir archivo", "Choose file"), "folder", preview ? "secondary" : "primary", disabled())}
      </div>
      ${preview ? `<form id="archive-import-form">
        <p class="archive-file"><strong>${h(preview.source)}</strong> <span>${bytesLabel(preview.bytes)}</span></p>
        <fieldset class="archive-categories"><legend>${t("Categorías", "Categories")}</legend>
          ${preview.categories.filter(item => !item.sensitive).map(categoryRow).join("")}
        </fieldset>
        ${preview.categories.some(item => item.sensitive) ? `<fieldset class="archive-categories archive-private"><legend>${t("Datos privados · selección independiente", "Private data · separate selection")}</legend>
          ${preview.categories.filter(item => item.sensitive).map(categoryRow).join("")}
        </fieldset>` : ""}
        <div class="archive-fields">
          <label for="archive-account">${t("Cuenta", "Account")}<input id="archive-account" name="account" maxlength="72" required value="${h(draft.account)}" ${disabled()}></label>
          <label for="archive-destination">${t("Carpeta lógica", "Archive group")}<input id="archive-destination" name="destination" maxlength="72" required value="${h(draft.destination)}" ${disabled()}></label>
          <label for="archive-from">${t("Desde (opcional)", "From (optional)")}<input id="archive-from" name="from" type="date" value="${h(draft.from)}" ${disabled()}></label>
          <label for="archive-to">${t("Hasta (opcional)", "To (optional)")}<input id="archive-to" name="to" type="date" value="${h(draft.to)}" ${disabled()}></label>
        </div>
        <p class="muted">${t("El intervalo usa UTC. Si lo aplicas, se omiten los registros y medios sin fecha.", "The date range uses UTC. Applying it skips undated records and media.")}</p>
        <button type="submit" class="button primary" ${disabled()}>${t("Importar selección", "Import selection")}</button>
      </form>` : ""}
    </section>`;
  }

  function listening(page: RecordPage) {
    return `<dl class="archive-metrics">
      <div><dt>${t("Eventos", "Events")}</dt><dd>${number(page.total)}</dd></div>
      <div><dt>${t("Minutos escuchados", "Minutes listened")}</dt><dd>${page.playedMs === null ? "—" : number(page.playedMs / 60_000, 1)}</dd></div>
      <div><dt>${t("Reproducciones completas", "Complete plays")}</dt><dd>${page.completions === null ? t("Sin datos", "Unavailable") : number(page.completions)}</dd></div>
      ${page.ended === null ? "" : `<div><dt>${t("Llegaron al final", "Reached the end")}</dt><dd>${number(page.ended)}</dd></div>`}
    </dl>
    ${page.completionKnown > 0 && page.completionKnown < page.total ? `<p class="muted">${t("Eventos con estado completo verificable", "Events with verifiable completion")} · ${number(page.completionKnown)} / ${number(page.total)}</p>` : ""}
    ${page.artists.length ? `<details class="archive-details"><summary>${t("Artistas por número de eventos", "Artists by event count")}</summary>
      <ul>${page.artists.map(([artist, count]) => `<li>${h(artist)} · ${number(count)}</li>`).join("")}</ul></details>` : ""}`;
  }

  function recordRow(row: Row) {
    const metadata = [row.author, row.time === null ? null : day(row.time), row.playedMs === null ? null : `${number(row.playedMs / 1000, 1)} s`].filter(Boolean).join(" · ");
    const id = `data-id="${h(row.id)}" ${disabled()}`;
    return `<li class="archive-record">
      <label class="archive-record-name"><input type="checkbox" name="archive-record" value="${h(row.id)}" ${selected.has(row.id) ? "checked" : ""} ${disabled()}>
        <span><strong>${h(row.title)}</strong>${metadata ? `<small>${h(metadata)}</small>` : ""}</span>
      </label>
      ${row.media ? action("archive-copy", t("Guardar copia", "Save copy"), undefined, "secondary", id) : ""}
      ${row.trackCount ? action("archive-match", t("Buscar en Audio", "Match in Audio"), undefined, "secondary", id) : ""}
      <details class="archive-details"><summary>${t("Ver registro", "View record")}</summary>
        ${row.text ? `<p class="archive-text">${h(row.text)}</p>` : ""}
        ${row.link ? `<p>${h(row.link)}</p>` : ""}
        ${row.threadParent ? `<p>${t("Responde a", "Replies to")} · ${h(row.threadParent)}</p>` : ""}
        <small>${t("Archivo de origen", "Source file")} · ${h(row.source)}</small>
      </details>
    </li>`;
  }

  function recordList() {
    if (!category) return "";
    if (privateCategory(category) && !privateOpen) return `<div class="archive-empty archive-locked">
      <span class="archive-mark">${icon("lock")}</span>
      <div><h3>${t("Categoría privada", "Private category")}</h3>
        <p>${t("Los registros permanecen ocultos hasta que abras esta categoría.", "Records stay hidden until you open this category.")}</p></div>
      ${action("archive-reveal", t("Mostrar categoría privada", "Show private category"), undefined, "secondary", disabled())}
    </div>`;
    const page = result;
    const linkCount = [...selected.values()].filter(row => row.link).length;
    const range = page ? dates(page.start, page.end) : "";
    return `<form id="archive-search-form" class="archive-search">
      <label for="archive-query">${t("Buscar en esta categoría", "Search this category")}
        <input id="archive-query" name="query" type="search" maxlength="120" value="${h(query)}" ${disabled()}>
      </label>
      <button type="submit" class="button secondary" ${disabled()}>${icon("search")}${t("Buscar", "Search")}</button>
    </form>
    ${page ? `${range ? `<p class="muted">${h(range)} · UTC</p>` : ""}
      ${category === "listening" ? listening(page) : ""}
      <div class="archive-actions">
        <span>${number(selected.size)} ${t("seleccionados", "selected")}</span>
        ${action("archive-export", t("Exportar CSV", "Export CSV"), undefined, "secondary", selected.size ? disabled() : "disabled")}
        ${privateCategory(category) ? "" : action("archive-links", t("Preparar enlaces", "Prepare links"), undefined, "secondary", linkCount && linkCount <= 100 ? disabled() : "disabled")}
      </div>
      ${privateExport ? `<div class="archive-confirm"><p>${t("El CSV guardará los registros privados seleccionados como texto legible.", "The CSV will store the selected private records as readable text.")}</p>
        ${action("archive-export-confirm", t("Guardar CSV privado", "Save private CSV"), undefined, "primary", disabled())}
        ${action("archive-cancel-confirm", t("Cancelar", "Cancel"), undefined, "secondary", disabled())}</div>` : ""}
      ${page.rows.length ? `<ul class="archive-records">${page.rows.map(recordRow).join("")}</ul>` : `<p>${t("No hay registros para esta búsqueda.", "No records match this search.")}</p>`}
      ${page.total > PAGE ? `<div class="archive-actions">
        ${action("archive-previous", t("Anterior", "Previous"), undefined, "secondary", offset > 0 ? disabled() : "disabled")}
        <span>${number(offset + 1)}–${number(Math.min(offset + PAGE, page.total))} / ${number(page.total)}</span>
        ${action("archive-next", t("Siguiente", "Next"), undefined, "secondary", offset + PAGE < page.total ? disabled() : "disabled")}
      </div>` : ""}` : ""}`;
  }

  function accountIndex() {
    const accounts = new Map<string, Archive[]>();
    for (const archive of archives) {
      const key = JSON.stringify([archive.provider, archive.account.toLowerCase()]);
      const group = accounts.get(key);
      if (group) group.push(archive);
      else accounts.set(key, [archive]);
    }
    return `<nav class="archive-index" aria-labelledby="archive-index-title">
      <h2 id="archive-index-title">${t("Cuentas", "Accounts")} <span>${number(accounts.size)}</span></h2>
      ${[...accounts.values()].map((group, index) => {
        const { account, provider } = group[0];
        return `<section class="archive-account-group" aria-labelledby="archive-account-${index}-title">
          <header><span class="archive-provider">${providers[provider]}</span><h3 id="archive-account-${index}-title">${h(account)}</h3></header>
          <ul>${group.map(item => `<li><button type="button" class="archive-package" data-action="archive-open" data-id="${h(item.id)}"
            aria-current="${item.id === archiveId ? "true" : "false"}" aria-disabled="${busy}">
            ${icon("document")}<span><strong>${h(item.source)}</strong><small>${h(item.destination)}</small></span>
          </button></li>`).join("")}</ul>
        </section>`;
      }).join("")}
    </nav>`;
  }

  function browse(archive: Archive) {
    const categories = categoryKeys.filter(key => (archive.categories[key] ?? 0) > 0);
    return `<section class="archive-browser" aria-labelledby="archive-current">
      <header class="archive-heading archive-browser-heading">
        <div><p class="archive-location">${icon("folder")}${h(archive.destination)}</p>
          <h2 id="archive-current">${h(archive.account)}</h2></div>
        <details class="archive-details archive-provenance" id="archive-source-${h(archive.id)}"><summary>${t("Procedencia", "Provenance")}</summary>
          <p>${h(archive.source)} · ${bytesLabel(archive.mediaBytes)} ${t("en copias", "in copies")}</p>
          <p>SHA-256 <code>${h(archive.fingerprint)}</code></p>
          ${categories.map(key => `<p>${name(key)} · ${h((archive.formats[key] ?? []).map(formatName).join(", "))}</p>`).join("")}
          ${action("archive-remove", archive.pendingRemoval ? t("Reintentar retirada", "Retry removal") : t("Retirar importación", "Remove import"), undefined, "secondary", disabled())}
        </details>
      </header>
      ${confirming ? `<div class="archive-confirm"><p>${t("Se borrarán el índice privado y las copias administradas de esta importación. Se conserva el archivo original.", "This removes the private index and managed copies for this import. The original export is preserved.")}</p>
        ${action("archive-remove-confirm", t("Retirar", "Remove"), undefined, "primary", disabled())}
        ${action("archive-cancel-confirm", t("Cancelar", "Cancel"), undefined, "secondary", disabled())}</div>` : ""}
      ${archive.pendingRemoval ? "" : `<nav class="archive-tabs" aria-label="${t("Categorías del archivo", "Archive categories")}">
        ${categories.map(key => `<button type="button" data-action="archive-category" data-id="${key}" aria-pressed="${category === key}" aria-disabled="${busy}">
          ${icon(categoryIcons[key])}<span>${name(key)}</span><small>${number(archive.categories[key] ?? 0)}</small></button>`).join("")}
      </nav>${recordList()}`}
    </section>`;
  }

  function render() {
    const archive = current();
    return `<div class="personal-archives">
      <header class="archive-heading archive-page-heading">
        <div class="archive-title-group"><span class="archive-mark">${icon("library")}</span><div>
          <h1 id="archive-title" tabindex="-1">${t("Archivos personales", "Personal archives")}</h1>
          <p>${icon("lock")}${t("Índice cifrado en este equipo", "Index encrypted on this device")}</p>
        </div></div>
        ${!adding ? action("archive-new", t("Añadir exportación", "Add export"), "add", "primary", disabled()) : ""}
      </header>
      ${busy ? `<div class="archive-actions"><span role="status">${t("Procesando archivo…", "Working on archive…")}</span>
        ${requestId ? action("archive-cancel", t("Cancelar", "Cancel")) : ""}</div>` : ""}
      ${error ? `<p class="archive-error" role="alert">${h(error)}</p>` : ""}
      ${!loaded && !busy && error ? action("archive-retry", t("Reintentar", "Retry")) : ""}
      ${adding ? importForm() : ""}
      ${!adding && archives.length ? `<div class="archive-workspace">
        ${accountIndex()}
        ${archive ? browse(archive) : `<div class="archive-empty"><span class="archive-mark">${icon("document")}</span>
          <h2>${t("Elige una exportación", "Choose an export")}</h2>
          <p>${t("Abre un paquete del índice para explorar sus categorías.", "Open a package from the index to browse its categories.")}</p></div>`}
      </div>` : ""}
      ${loaded && !adding && !archives.length ? `<section class="archive-panel archive-empty">
        <span class="archive-mark">${icon("folder")}</span>
        <h2>${t("Aún no hay exportaciones importadas.", "No exports have been imported yet.")}</h2>
        <p>${t("Añade un ZIP o un archivo de exportación que ya tengas en el equipo.", "Add a ZIP or an export file you already have on this device.")}</p>
      </section>` : ""}
    </div>`;
  }

  async function open(id: string) {
    archiveId = id;
    confirming = false;
    const archive = current();
    const categories = categoryKeys.filter(key => (archive?.categories[key] ?? 0) > 0);
    resetRecords(archive?.pendingRemoval ? null : categories.find(item => !privateCategory(item)) ?? categories[0] ?? null);
    await read();
  }

  async function exportSelected() {
    if (!category) return;
    await run(async () => {
      const saved = await hooks.command<boolean>("personal_archive_export", {
        id: archiveId, category, records: [...selected.keys()], consent: privateExport,
      });
      privateExport = false;
      if (saved) hooks.notify(t("Selección exportada.", "Selection exported."), "success");
    });
  }

  async function click(kind: string, button: HTMLElement) {
    if (!kind.startsWith("archive-")) return false;
    switch (kind) {
      case "archive-new":
        adding = true; error = ""; hooks.render(); document.getElementById("archive-provider")?.focus(); break;
      case "archive-choose": await choose(); break;
      case "archive-close-form":
        adding = false; preview = null; error = ""; hooks.render();
        document.querySelector<HTMLElement>('[data-action="archive-new"]')?.focus({ preventScroll: true }); break;
      case "archive-open": {
        const id = button.dataset.id;
        if (id && id !== archiveId && archives.some(item => item.id === id)) await run(() => open(id));
        break;
      }
      case "archive-retry": await load(); break;
      case "archive-cancel":
        if (requestId) await hooks.command("cancel_file_source_inspection", { requestId });
        break;
      case "archive-category": {
        const key = categoryKeys.find(item => item === button.dataset.id);
        if (key && key !== category) await run(async () => { resetRecords(key); await read(); });
        break;
      }
      case "archive-reveal":
        await run(async () => { privateOpen = true; await read(); });
        if (visible) document.getElementById("archive-query")?.focus({ preventScroll: true });
        break;
      case "archive-previous": await run(async () => { offset = Math.max(0, offset - PAGE); await read(); }); break;
      case "archive-next": await run(async () => { offset += PAGE; await read(); }); break;
      case "archive-remove": confirming = true; hooks.render(); break;
      case "archive-cancel-confirm": confirming = false; privateExport = false; hooks.render(); break;
      case "archive-remove-confirm": await run(async () => {
        const id = archiveId;
        resetRecords(null);
        confirming = false;
        try { await hooks.command("personal_archive_remove", { id }); }
        finally { await refreshList(); }
        archiveId = "";
        hooks.notify(t("Importación retirada.", "Import removed."), "success");
      }); break;
      case "archive-export":
        if (category && privateCategory(category)) { privateExport = true; hooks.render(); }
        else await exportSelected();
        break;
      case "archive-export-confirm": await exportSelected(); break;
      case "archive-links": await run(async () => {
        const records = [...selected.values()].filter(row => row.link).map(row => row.id);
        const links = await hooks.command<string[]>("personal_archive_links", { id: archiveId, category, records });
        if (links.length) await hooks.prepareLinks(links);
      }); break;
      case "archive-copy": await run(async () => {
        if (await hooks.command<boolean>("personal_archive_copy_media", {
          id: archiveId, category, record: button.dataset.id, consent: privateOpen,
        })) hooks.notify(t("Copia guardada.", "Copy saved."), "success");
      }); break;
      case "archive-match": await run(async () => {
        const match = await hooks.command<ImportPreview>("personal_archive_match", { id: archiveId, record: button.dataset.id });
        hooks.openPlaylist(match);
      }); break;
    }
    return true;
  }

  function input(target: HTMLElement) {
    if (!(target instanceof HTMLInputElement)) return false;
    if (target.form?.id === "archive-import-form" && Object.hasOwn(draft, target.name)) {
      draft[target.name as keyof typeof draft] = target.value;
      return true;
    }
    if (target.id === "archive-query") { query = target.value; return true; }
    return false;
  }

  async function change(target: HTMLElement) {
    if (target instanceof HTMLSelectElement && target.id === "archive-provider") {
      provider = target.value as Provider;
      preview = null;
      included.clear();
      error = "";
      hooks.render();
    } else if (target instanceof HTMLInputElement && target.name === "archive-category") {
      const key = target.value as Category;
      if (target.checked) included.add(key); else included.delete(key);
    } else if (target instanceof HTMLInputElement && target.name === "archive-record") {
      const row = result?.rows.find(item => item.id === target.value);
      if (row && target.checked && selected.size < 200) selected.set(row.id, row);
      if (!target.checked) selected.delete(target.value);
      privateExport = false;
      hooks.render();
      document.querySelector<HTMLInputElement>(`input[name="archive-record"][value="${CSS.escape(target.value)}"]`)?.focus({ preventScroll: true });
    } else return false;
    return true;
  }

  async function submit(form: HTMLFormElement) {
    if (form.id === "archive-search-form") {
      await run(async () => { offset = 0; await read(); });
    } else if (form.id === "archive-import-form" && preview) {
      const fingerprint = preview.fingerprint;
      if (!included.size) {
        error = t("Elige al menos una categoría.", "Choose at least one category.");
        hooks.render();
        return true;
      }
      await run(async () => {
        requestId = Date.now() * 1000 + 401;
        hooks.render();
        const { archive, skippedFiles } = await hooks.command<{ archive: Archive; skippedFiles: number }>("personal_archive_import", {
          requestId, selection: { ...draft, fingerprint, categories: [...included].filter(key => !privateCategory(key)),
            sensitive: [...included].filter(privateCategory) },
        });
        archives = [...archives.filter(item => item.id !== archive.id), archive];
        loaded = true;
        preview = null; adding = false; included.clear();
        draft = { account: "", destination: "", from: "", to: "" };
        await open(archive.id);
        if (skippedFiles) hooks.notify(t(`Exportación importada. Archivos incompatibles omitidos: ${number(skippedFiles)}.`, `Export imported. Unsupported files skipped: ${number(skippedFiles)}.`), "info");
        else hooks.notify(t("Exportación importada.", "Export imported."), "success");
        if (visible) hooks.returnToHeading();
      });
    } else return false;
    return true;
  }

  return { render, load, leave, click, input, change, submit };
}
