import "./palette.css";
import { escapeHtml as h, type Copy, type Snapshot } from "./contracts";
import { detailsFor, metadataText } from "./library-features";
import { durationLabel, searchKey } from "./presentation";
import { icon, type Icon } from "./views-common";

interface Result {
  id: string;
  kind: string;
  label: string;
  context: string;
  glyph?: Icon;
  run(): void | Promise<void>;
}

interface Hooks {
  snapshot(): Snapshot;
  t: Copy;
  destinations(query: string): Result[];
  search?(query: string): Promise<Result[]>;
  prepare(): void;
  addSource(): void;
  inspect(id: string, time?: number): void;
  collection(name: string): void;
  preset(id: string): void | Promise<void>;
  source(id: string): void;
  showError(error: unknown): void;
}

export function createPalette(hooks: Hooks) {
  const { t } = hooks;
  const dialog = document.createElement("dialog");
  dialog.id = "action-palette";
  dialog.className = "action-palette";
  dialog.setAttribute("aria-labelledby", "palette-label");
  document.body.append(dialog);
  let previous: HTMLElement | null = null;
  let input: HTMLInputElement;
  let list: HTMLUListElement;
  let status: HTMLElement;
  let clear: HTMLButtonElement;
  let results: Result[] = [];
  let selected = 0, generation = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  function find(query: string): Result[] {
    const snapshot = hooks.snapshot();
    const needle = searchKey(query);
    const words = needle.split(/\s+/);
    const matches = (text: string) => {
      const normalized = searchKey(text);
      return words.every(word => normalized.includes(word));
    };
    const actions: Result[] = [
      { id: "prepare", kind: t("Acción", "Action"), label: t("Preparar lote", "Prepare batch"),
        context: t("Inicio · enlaces y archivos .txt", "Home · links and .txt files"), glyph: "paste", run: hooks.prepare },
      { id: "source-add", kind: t("Acción", "Action"), label: t("Añadir fuente", "Add source"),
        context: t("Biblioteca · seguir una fuente", "Library · follow a source"), glyph: "add", run: hooks.addSource },
      ...hooks.destinations(query),
    ];
    for (const preset of snapshot.presets ?? []) actions.push({ id: `preset:${preset.id}`, kind: t("Ajuste", "Preset"),
      label: preset.name, context: t("Ajustes reutilizables", "Reusable presets"), glyph: "settings", run: () => hooks.preset(preset.id) });
    for (const name of snapshot.collections) actions.push({ id: `collection:${name}`, kind: t("Colección", "Collection"),
      label: name, context: t("Colección manual", "Manual collection"), glyph: "folder", run: () => hooks.collection(name) });
    for (const collection of snapshot.smartCollections ?? []) actions.push({ id: `smart:${collection.id}`, kind: t("Colección", "Collection"),
      label: collection.name, context: t("Colección inteligente", "Smart collection"), glyph: "folder", run: () => hooks.collection(`smart:${collection.id}`) });
    for (const source of snapshot.sources ?? []) actions.push({ id: `source:${source.id}`, kind: t("Fuente", "Source"),
      label: source.name, context: source.url, glyph: "link", run: () => hooks.source(source.id) });
    const found = actions.filter(result => matches(`${result.label} ${result.context}`)).slice(0, query ? 80 : 12);
    for (const file of snapshot.library) {
      if (found.length >= (query ? 80 : 16)) break;
      if (!query || matches(metadataText(file))) found.push({ id: `file:${file.id}`, kind: t("Archivo", "File"),
        label: file.name, context: file.missing ? t("No disponible en disco", "Unavailable on disk") : file.path,
        glyph: file.kind === "audio" || file.kind === "video" || file.kind === "image" ? file.kind : "file",
        run: () => hooks.inspect(file.id) });
      if (!query) continue;
      for (const track of detailsFor(file).tracks) {
        for (const [index, segment] of track.segments.entries()) {
          if (found.length >= 80) break;
          if (matches(segment.text)) found.push({ id: `cue:${file.id}:${track.id}:${index}`, kind: t("Subtítulo", "Subtitle"),
            label: segment.text, context: `${file.name} · ${durationLabel(segment.start)} · ${track.language}`,
            glyph: "captions", run: () => hooks.inspect(file.id, segment.start) });
        }
        if (found.length >= 80) break;
      }
    }
    const rank = (result: Result) => {
      const label = searchKey(result.label);
      return label === needle ? 3 : label.startsWith(needle) ? 2 : matches(label) ? 1 : 0;
    };
    return query ? found.sort((a, b) => rank(b) - rank(a)) : found;
  }

  function select(index: number, scroll = true) {
    const row = list.children[index];
    if (!results[index] || !row) {
      input.removeAttribute("aria-activedescendant");
      return;
    }
    list.children[selected]?.setAttribute("aria-selected", "false");
    selected = index;
    row.setAttribute("aria-selected", "true");
    input.setAttribute("aria-activedescendant", row.id);
    if (scroll) row.scrollIntoView({ block: "nearest" });
  }

  function show(next: Result[], message = "", keepSelection = false) {
    const active = keepSelection ? results[selected]?.id : undefined;
    results = next.slice(0, 80);
    selected = Math.max(0, results.findIndex(result => result.id === active));
    list.innerHTML = results.length ? results.map((result, index) =>
      `<li id="palette-option-${index}" role="option" aria-selected="false" data-palette-index="${index}">
        <span class="palette-glyph">${icon(result.glyph ?? "document")}</span>
        <span class="palette-copy"><strong>${h(result.label)}</strong>${result.context ? `<small>${h(result.context)}</small>` : ""}</span>
        <span class="palette-kind">${h(result.kind)}</span>
      </li>`).join("") : `<li class="palette-empty" role="presentation">${icon("search")}<span>${h(message ||
        t("Sin coincidencias. Prueba con otro nombre.", "No matches. Try another name."))}</span></li>`;
    status.textContent = message || t(`${results.length} ${results.length === 1 ? "resultado" : "resultados"}`,
      `${results.length} ${results.length === 1 ? "result" : "results"}`);
    select(selected);
  }

  function update() {
    clearTimeout(timer);
    const request = ++generation;
    const query = input.value.trim();
    clear.hidden = !input.value;
    const local = find(query);
    const search = hooks.search;
    if (!search || query.length < 2) { show(local); return; }
    show(local, t("Buscando en documentos…", "Searching documents…"));
    timer = setTimeout(() => {
      void search(query).then(found => {
        if (dialog.open && request === generation) show([...local, ...found], "", true);
      }).catch(() => {
        if (dialog.open && request === generation) show(local, t("No se pudo buscar en documentos.", "Could not search documents."), true);
      });
    }, 200);
  }

  function close() {
    if (!dialog.open) return;
    clearTimeout(timer);
    ++generation;
    results = [];
    dialog.close();
    if (previous?.isConnected) previous.focus({ preventScroll: true });
    previous = null;
  }

  function open() {
    if (dialog.open) { input.focus(); input.select(); return; }
    previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.innerHTML = `<div class="palette-field">
      <span class="source-emblem" aria-hidden="true"></span>
      <label id="palette-label" class="sr-only" for="palette-query">${t("Buscar en nabnab", "Search nabnab")}</label>
      <input id="palette-query" type="search" role="combobox" aria-autocomplete="list" aria-expanded="true"
        aria-controls="palette-results" autocomplete="off" placeholder="${t("Busca una herramienta, sección o archivo…", "Find a tool, section or file…")}">
      <button id="palette-clear" class="icon-button" type="button" aria-label="${t("Borrar búsqueda", "Clear search")}" hidden>${icon("close")}</button>
      <button id="palette-close" type="button" aria-label="${t("Cerrar búsqueda", "Close search")}"><kbd>Esc</kbd></button>
    </div>
    <ul id="palette-results" role="listbox" aria-label="${t("Resultados", "Results")}"></ul>
    <div class="palette-footer"><span id="palette-status" role="status"></span>
      <span aria-hidden="true"><kbd>↑</kbd> <kbd>↓</kbd> ${t("elegir", "select")} <kbd>↵</kbd> ${t("abrir", "open")}</span></div>`;
    input = dialog.querySelector<HTMLInputElement>("#palette-query")!;
    list = dialog.querySelector<HTMLUListElement>("#palette-results")!;
    status = dialog.querySelector<HTMLElement>("#palette-status")!;
    clear = dialog.querySelector<HTMLButtonElement>("#palette-clear")!;
    dialog.showModal();
    update();
    input.focus();
  }

  function execute(index: number) {
    const result = results[index];
    if (!result) return;
    close();
    Promise.resolve().then(() => result.run()).catch(hooks.showError);
  }

  const rowIndex = (target: EventTarget | null) => target instanceof Element
    ? Number(target.closest<HTMLElement>("[data-palette-index]")?.dataset.paletteIndex)
    : NaN;
  dialog.addEventListener("input", event => { if (event.target === input) update(); });
  dialog.addEventListener("pointermove", event => {
    if (event.pointerType === "touch") return;
    const index = rowIndex(event.target);
    if (Number.isInteger(index) && index !== selected) select(index, false);
  });
  dialog.addEventListener("click", event => {
    if (event.target === dialog || event.target instanceof Element && event.target.closest("#palette-close")) { close(); return; }
    if (event.target instanceof Element && event.target.closest("#palette-clear")) {
      input.value = "";
      update();
      input.focus();
      return;
    }
    execute(rowIndex(event.target));
  });
  dialog.addEventListener("keydown", event => {
    if (event.target !== input || event.isComposing) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (results.length) select((selected + (event.key === "ArrowDown" ? 1 : -1) + results.length) % results.length);
    } else if (event.key === "Enter") {
      event.preventDefault();
      execute(selected);
    }
  });
  dialog.addEventListener("cancel", event => { event.preventDefault(); close(); });
  return { open, get isOpen() { return dialog.open; } };
}
