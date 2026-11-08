import "./articles.css";
import { escapeHtml as h, type Copy, type LibraryItem, type Snapshot } from "./contracts";
import { action, icon } from "./views-common";

export type ArticleBlock =
  | { kind: "heading"; text: string; level: number }
  | { kind: "paragraph" | "quote" | "code" | "listItem"; text: string }
  | { kind: "image"; text: string; resourceId: string };
export interface ArticleDocument {
  schemaVersion: 1; mode: "reading" | "selection" | "page";
  title: string; author: string | null; url: string;
  publishedAt: string | null; archivedAt: string;
  blocks: ArticleBlock[];
  resources: { id: string; dataUrl: string; width: number; height: number }[];
  warnings: string[]; selectionContext: { prefix: string; suffix: string } | null;
}
export interface ArticleAnnotation {
  id: string; version: string; block: number; quote: string; prefix: string; suffix: string;
  note: string; createdAt: number;
}
export interface ArticleHooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  snapshot(): Snapshot; refresh(): Promise<void>; render(): void; t: Copy;
  notify(message: string, kind?: "info" | "success" | "error"): void;
  share?(input: { ids: string[]; text?: string; url?: string }): void;
}
interface Anchor { version: string; block: number; quote: string; prefix: string; suffix: string }
type ReaderState =
  | { kind: "idle" }
  | { kind: "loading"; id: string }
  | { kind: "error"; id: string; error: string }
  | { kind: "ready"; id: string; version: string; document: ArticleDocument };

const formats = [{ id: "html", label: "HTML" }, { id: "epub", label: "EPUB" }, { id: "md", label: "Markdown" }];
const quoteLimit = 4000;
function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") return error.message;
  return String(error);
}
function dateText(value: string, t: Copy): string {
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? date.toLocaleString(t("es-ES", "en-GB"), { dateStyle: "medium", timeStyle: "short" }) : value;
}
function pngResource(value: string): boolean {
  return value.length <= 16 * 1024 * 1024 && /^data:image\/png;base64,iVBORw0KGgo[A-Za-z0-9+/]*={0,2}$/.test(value);
}
function validQuote(value: string): boolean {
  return Boolean(value.trim()) && value.length <= quoteLimit && new TextEncoder().encode(value).length <= 8000 && ![...value].some(character => { const point = character.codePointAt(0) ?? 0; return point >= 0xd800 && point <= 0xdfff; });
}
function anchorPosition(text: string, anchor: Pick<ArticleAnnotation, "quote" | "prefix" | "suffix">): number | null {
  if (!anchor.quote) return null;
  const matches: number[] = [];
  for (let position = text.indexOf(anchor.quote); position !== -1; position = text.indexOf(anchor.quote, position + 1)) {
    const end = position + anchor.quote.length;
    if (text.slice(Math.max(0, position - anchor.prefix.length), position) === anchor.prefix && text.slice(end, end + anchor.suffix.length) === anchor.suffix) matches.push(position);
    if (matches.length > 1) return null;
  }
  return matches.length === 1 ? matches[0] ?? null : null;
}
function highlightedText(text: string, annotations: ArticleAnnotation[]): string {
  const ranges = annotations.flatMap(annotation => {
    const start = anchorPosition(text, annotation);
    return start === null ? [] : [{ start, end: start + annotation.quote.length, id: annotation.id }];
  });
  const boundaries = [...new Set([0, text.length, ...ranges.flatMap(range => [range.start, range.end])])].sort((a, b) => a - b);
  let rendered = "";
  for (let index = 1; index < boundaries.length; index++) {
    const start = boundaries[index - 1] ?? 0, end = boundaries[index] ?? text.length;
    const active = ranges.filter(range => range.start <= start && range.end >= end).map(range => range.id);
    const fragment = h(text.slice(start, end));
    rendered += active.length ? `<mark data-article-annotations="${h(active.join(" "))}">${fragment}</mark>` : fragment;
  }
  return rendered;
}

export function createArticles(hooks: ArticleHooks) {
  const { t } = hooks;
  let reader: ReaderState = { kind: "idle" };
  let requested = false;
  let generation = 0;
  let working = false;
  let error = "";
  let selection: Anchor | null = null;
  let shareSelection = "";
  let draft: Anchor | null = null;
  let note = "";
  let focusNote = false;
  let globalSelectionListener = false;
  const boundDialogs = new WeakSet<HTMLDialogElement>();
  let restoreId = "";

  function itemFor(id: string) { return hooks.snapshot().library.find(item => item.id === id); }
  function annotations(item: LibraryItem): ArticleAnnotation[] { return item.details?.articleAnnotations ?? []; }
  function renderBlocks(document: ArticleDocument, item: LibraryItem, version: string): string {
    return document.blocks.map((block, index) => {
      if (block.kind === "image") {
        const resource = document.resources.find(value => value.id === block.resourceId);
        if (!resource || !pngResource(resource.dataUrl)) return `<p class="article-image-unavailable">${icon("image")}${t("Imagen no conservada en esta captura", "Image not preserved in this capture")}${block.text ? ` · ${h(block.text)}` : ""}</p>`;
        return `<figure class="article-figure"><img src="${h(resource.dataUrl)}" alt="${h(block.text)}" loading="lazy" decoding="async">${block.text ? `<figcaption>${h(block.text)}</figcaption>` : ""}</figure>`;
      }
      const matching = annotations(item).filter(annotation => annotation.version === version && annotation.block === index);
      const text = highlightedText(block.text, matching);
      const attributes = `data-article-block="${index}" tabindex="-1"`;
      let content: string;
      switch (block.kind) {
        case "heading": { const level = Math.max(2, Math.min(6, Math.floor(block.level) || 2)); content = `<h${level} ${attributes}>${text}</h${level}>`; } break;
        case "paragraph": content = `<p ${attributes}>${text}</p>`; break;
        case "quote": content = `<blockquote ${attributes}>${text}</blockquote>`; break;
        case "code": content = `<pre ${attributes}><code>${text}</code></pre>`; break;
        case "listItem": content = `<p ${attributes}>${text}</p>`; break;
      }
      const annotate = action("article-annotate-block", t("Anotar bloque ", "Annotate block ") + (index + 1), "add", "article-block-note icon-button", `data-block="${index}" ${working || !validQuote(block.text) ? "disabled" : ""}`);
      if (block.kind === "listItem") return `${document.blocks[index - 1]?.kind === "listItem" ? "" : '<ul class="article-list">'}<li class="article-block-row">${content}${annotate}</li>${document.blocks[index + 1]?.kind === "listItem" ? "" : "</ul>"}`;
      return `<div class="article-block-row">${content}${annotate}</div>`;
    }).join("");
  }
  function renderNotes(item: LibraryItem, document: ArticleDocument, version: string): string {
    const saved = annotations(item);
    return `<section class="article-notes" aria-labelledby="article-notes-title"><div class="article-section-heading"><h3 id="article-notes-title">${t("Notas de la captura", "Capture notes")}</h3><span>${saved.length}</span></div>${saved.length ? `<ol>${saved.map(annotation => {
      const block = document.blocks[annotation.block];
      const matches = annotation.version === version && block && block.kind !== "image" && anchorPosition(block.text, annotation) !== null;
      return `<li data-article-note="${h(annotation.id)}"><blockquote>${h(annotation.quote)}</blockquote>${annotation.note ? `<p>${h(annotation.note)}</p>` : `<p class="field-note">${t("Resaltado sin nota", "Highlight without a note")}</p>`}${!matches ? `<p class="article-anchor-warning">${annotation.version !== version ? t("Pertenece a otra versión. No se resalta en este texto.", "Belongs to another version. It is not highlighted in this text.") : t("No se ha encontrado una coincidencia inequívoca. La nota se conserva sin moverla.", "No unambiguous match was found. The note is kept without moving it.")}</p>` : ""}<div class="button-group">${matches ? action("article-go-note", t("Ver en el texto", "Show in text"), "arrow", "quiet", `data-annotation="${h(annotation.id)}"`) : ""}${action("article-remove-note", t("Quitar nota", "Remove note"), "close", "quiet", `data-annotation="${h(annotation.id)}" ${working ? "disabled" : ""}`)}</div></li>`;
    }).join("")}</ol>` : `<p class="field-note">${t("Selecciona texto para resaltarlo o usa el botón de un bloque para añadir una nota.", "Select text to highlight it, or use a block's button to add a note.")}</p>`}<p class="field-note">${t("Las notas pertenecen a esta versión local. Una captura nueva no las traslada a otro texto.", "Notes belong to this local version. A new capture does not move them to another text.")}</p></section>`;
  }
  function renderBody(item: LibraryItem): string {
    if (reader.kind === "loading") return `<div class="article-reader-state" role="status">${t("Abriendo la captura local…", "Opening the local capture…")}</div>`;
    if (reader.kind === "error") return `<div class="article-reader-state"><p role="alert">${h(reader.error)}</p>${action("article-open", t("Reintentar", "Retry"), "retry", "secondary", `data-id="${h(item.id)}"`)}</div>`;
    if (reader.kind !== "ready" || reader.id !== item.id) return "";
    const { document, version } = reader;
    return `<header class="article-heading"><span class="article-mode">${document.mode === "selection" ? t("Selección guardada", "Saved selection") : document.mode === "page" ? t("Página estática", "Static page") : t("Copia de lectura", "Reading copy")}</span><h2 id="article-reader-title" tabindex="-1">${h(document.title)}</h2>${document.author ? `<p class="article-author">${h(document.author)}</p>` : ""}<p class="article-source-url">${h(document.url)}</p>${action("article-copy-source",t("Copiar vínculo al origen","Copy source link"),"copy","quiet")}${hooks.share ? action("article-share-source",t("Enviar enlace público","Share public link"),"arrow","quiet") : ""}<dl class="article-dates"><div><dt>${t("Archivado", "Archived")}</dt><dd>${h(dateText(document.archivedAt, t))}</dd></div>${document.publishedAt ? `<div><dt>${t("Publicado según el origen", "Published according to the source")}</dt><dd>${h(dateText(document.publishedAt, t))}</dd></div>` : ""}</dl></header>${document.warnings.length ? `<details class="article-capture-warnings"><summary>${t("Límites de esta captura", "Capture limitations")} · ${document.warnings.length}</summary><ul>${document.warnings.map(warning => `<li>${h(warning)}</li>`).join("")}</ul></details>` : ""}${document.selectionContext ? `<details class="article-selection-context"><summary>${t("Contexto de la selección", "Selection context")}</summary><dl><div><dt>${t("Antes de la selección", "Before the selection")}</dt><dd>${h(document.selectionContext.prefix)}</dd></div><div><dt>${t("Después de la selección", "After the selection")}</dt><dd>${h(document.selectionContext.suffix)}</dd></div></dl></details>` : ""}<article class="article-text" aria-label="${t("Texto de la versión local", "Local version text")}">${renderBlocks(document, item, version)}</article><div class="article-selection-tools"><span id="article-selection-status" role="status">${t("Selecciona texto de un solo bloque para resaltarlo.", "Select text within one block to highlight it.")}</span>${action("article-use-selection", t("Resaltar o anotar selección", "Highlight or annotate selection"), "add", "secondary", "disabled")}${hooks.share ? action("article-share-selection",t("Enviar texto seleccionado","Share selected text"),"arrow","quiet","disabled") : ""}</div>${draft ? `<form id="article-note-form" data-id="${h(item.id)}" class="article-note-form"><h3>${t("Añadir a esta versión", "Add to this version")}</h3><blockquote>${h(draft.quote)}</blockquote><label for="article-note">${t("Nota, opcional", "Note, optional")}</label><textarea id="article-note" rows="3" maxlength="4000" placeholder="${t("Escribe una nota sobre este fragmento", "Write a note about this passage")}" ${working ? "disabled" : ""}>${h(note)}</textarea><div class="button-group"><button type="submit" class="button primary" ${working ? "disabled" : ""}>${working ? t("Guardando…", "Saving…") : t("Guardar resaltado y nota", "Save highlight and note")}</button>${action("article-cancel-note", t("Cancelar", "Cancel"), undefined, "quiet", working ? "disabled" : "")}</div></form>` : ""}${error ? `<p class="article-reader-error" role="alert">${h(error)}</p>` : ""}${renderNotes(item, document, version)}`;
  }
  function renderInspector(item: LibraryItem): string {
    const active = reader.kind !== "idle" && reader.id === item.id;
    const title = active && reader.kind === "ready" ? reader.document.title : item.name;
    const source = active && reader.kind === "ready" ? reader.document.url : item.details?.sourceUrl ?? item.source;
    return `<section class="article-inspector" aria-label="${t("Artículo archivado", "Archived article")}"><span class="inspector-eyebrow">${t("Archivo web local", "Local web archive")}</span><h2>${h(title)}</h2>${source ? `<p class="article-source-url">${h(source)}</p>` : ""}<p class="field-note">${t("Lee la versión guardada sin scripts ni solicitudes al sitio de origen.", "Read the saved version without scripts or requests to the source site.")}</p>${action("article-open", t("Abrir lector local", "Open local reader"), "library", "primary", `data-id="${h(item.id)}"`)}<div class="article-export-actions"><span>${t("Exportar una copia", "Export a copy")}</span><div class="button-group">${formats.map(format => action("article-export", format.label, undefined, "secondary", `data-id="${h(item.id)}" data-format="${format.id}" ${working ? "disabled" : ""}`)).join("")}</div></div>${!requested && error ? `<p role="alert" class="article-reader-error">${h(error)}</p>` : ""}${active && requested ? `<dialog id="article-dialog" class="article-dialog" aria-label="${t("Lector de artículo local", "Local article reader")}"><div class="article-dialog-toolbar"><span>${t("Lectura offline", "Offline reading")}</span><div class="button-group">${formats.map(format => action("article-export", format.label, undefined, "quiet", `data-id="${h(item.id)}" data-format="${format.id}" ${working ? "disabled" : ""}`)).join("")}${action("article-close", t("Cerrar lector", "Close reader"), "close", "article-close icon-button")}</div></div><div class="article-reader-content">${renderBody(item)}</div></dialog>` : ""}</section>`;
  }
  function readSelection(): Anchor | null {
    if (reader.kind !== "ready") return null;
    const selected = window.getSelection();
    if (!selected || selected.isCollapsed || selected.rangeCount !== 1) return null;
    const range = selected.getRangeAt(0);
    const startElement = range.startContainer instanceof Element ? range.startContainer : range.startContainer.parentElement;
    const blockElement = startElement?.closest<HTMLElement>("[data-article-block]");
    if (!blockElement || !document.getElementById("article-dialog")?.contains(blockElement) || !blockElement.contains(range.endContainer)) return null;
    const blockIndex = Number(blockElement.dataset.articleBlock);
    const block = reader.document.blocks[blockIndex];
    if (!block || block.kind === "image") return null;
    const before = range.cloneRange(); before.selectNodeContents(blockElement); before.setEnd(range.startContainer, range.startOffset);
    const start = before.toString().length;
    const quote = range.toString();
    if (!validQuote(quote) || block.text.slice(start, start + quote.length) !== quote) return null;
    return { version: reader.version, block: blockIndex, quote, prefix: [...block.text.slice(0, start)].slice(-120).join(""), suffix: [...block.text.slice(start + quote.length)].slice(0, 120).join("") };
  }
  function updateSelection() {
    if (!requested || working) return;
    selection = readSelection();
    const selected = window.getSelection();
    const range = selected?.rangeCount === 1 ? selected.getRangeAt(0) : null;
    const article = document.querySelector("#article-dialog .article-text");
    const text = range && article?.contains(range.startContainer) && article.contains(range.endContainer) ? selected?.toString() ?? "" : "";
    shareSelection = text.trim() && new TextEncoder().encode(text).length <= 8192 ? text : "";
    const button = document.querySelector<HTMLButtonElement>("[data-action='article-use-selection']");
    if (button) button.disabled = !selection;
    const shareButton = document.querySelector<HTMLButtonElement>("[data-action='article-share-selection']");
    if (shareButton) shareButton.disabled = !shareSelection;
    const status = document.getElementById("article-selection-status");
    if (status) status.textContent = selection ? `${selection.quote.length} ${t("caracteres seleccionados", "characters selected")}` : t("Selecciona texto de un solo bloque para resaltarlo.", "Select text within one block to highlight it.");
  }
  async function open(id: string) {
    const item = itemFor(id);
    if (!item?.details?.webArchiveRef) return;
    requested = true; restoreId = id; selection = null; shareSelection = ""; draft = null; note = ""; error = "";
    if (reader.kind === "ready" && reader.id === id && reader.version === item.details.webArchiveRef) { hooks.render(); return; }
    const current = ++generation; reader = { kind: "loading", id }; hooks.render();
    try {
      const document = await hooks.command<ArticleDocument>("archive_document", { id });
      if (current !== generation) return;
      reader = { kind: "ready", id, version: item.details.webArchiveRef, document };
    } catch (failure) { if (current === generation) reader = { kind: "error", id, error: errorText(failure) }; }
    finally { if (current === generation) hooks.render(); }
  }
  function close() {
    requested = false; selection = null;
    if (reader.kind === "loading") { generation++; reader = { kind: "idle" }; }
    const dialog = document.getElementById("article-dialog");
    if (dialog instanceof HTMLDialogElement && dialog.open) dialog.close();
    document.querySelector<HTMLElement>(`[data-action='article-open'][data-id="${CSS.escape(restoreId)}"]`)?.focus({ preventScroll: true });
  }
  function change(element: HTMLElement): boolean {
    if (element instanceof HTMLTextAreaElement && element.id === "article-note") { note = element.value; return true; }
    return false;
  }
  async function perform(operation: () => Promise<void>) {
    if (working) return;
    working = true; error = ""; hooks.render();
    try { await operation(); } catch (failure) { error = errorText(failure); }
    finally { working = false; hooks.render(); }
  }
  async function click(name: string, button: HTMLButtonElement): Promise<boolean> {
    if (!name.startsWith("article-")) return false;
    switch (name) {
      case "article-open": await open(button.dataset.id ?? ""); break;
      case "article-close": close(); break;
      case "article-use-selection": {
        const anchor = readSelection() ?? selection;
        if (anchor && !working) { draft = anchor; note = ""; selection = null; focusNote = true; hooks.render(); }
      } break;
      case "article-share-selection": {
        if (shareSelection && reader.kind === "ready") hooks.share?.({ ids: [reader.id], text: shareSelection });
      } break;
      case "article-share-source": if (reader.kind === "ready") hooks.share?.({ ids: [reader.id], url: reader.document.url }); break;
      case "article-annotate-block": {
        if (reader.kind !== "ready" || working) break;
        const blockIndex = Number(button.dataset.block), block = reader.document.blocks[blockIndex];
        if (block && block.kind !== "image" && validQuote(block.text)) {
          draft = { version: reader.version, block: blockIndex, quote: block.text, prefix: "", suffix: "" }; note = ""; focusNote = true; hooks.render();
        }
      } break;
      case "article-cancel-note": draft = null; note = ""; hooks.render(); break;
      case "article-go-note": {
        const id = button.dataset.annotation;
        if (id) document.querySelector<HTMLElement>(`[data-article-annotations~="${CSS.escape(id)}"]`)?.closest<HTMLElement>("[data-article-block]")?.focus();
      } break;
      case "article-remove-note": {
        const id = button.dataset.annotation;
        if (id && reader.kind === "ready") { const libraryId = reader.id; await perform(async () => { await hooks.command<Snapshot>("archive_remove_annotation", { id: libraryId, annotationId: id }); await hooks.refresh(); }); }
      } break;
      case "article-copy-source": if(reader.kind==="ready"){await navigator.clipboard.writeText(reader.document.url);hooks.notify(t("Vínculo de origen copiado.","Source link copied."));}break;
      case "article-export": {
        const format = button.dataset.format;
        if (format !== "html" && format !== "epub" && format !== "md") break;
        const id = button.dataset.id ?? "";
        await perform(async () => { await hooks.command("queue_article_export", { id, format, clientRequest: crypto.randomUUID() }); await hooks.refresh(); hooks.notify(t("Copia añadida a Descargas. El artículo original se conserva.", "Copy added to Downloads. The original article is kept."), "success"); });
      } break;
    }
    return true;
  }
  async function submit(form: HTMLFormElement): Promise<boolean> {
    if (form.id !== "article-note-form") return false;
    if (!draft || reader.kind !== "ready" || working || !form.reportValidity()) return true;
    const id = reader.id, anchor = draft, savedNote = note;
    if (new TextEncoder().encode(savedNote).length > 8000) { error = t("La nota supera los 8000 bytes. Acórtala antes de guardarla.", "The note exceeds 8000 bytes. Shorten it before saving."); hooks.render(); return true; }
    if (anchor.version !== itemFor(id)?.details?.webArchiveRef) { error = t("La versión local cambió. Selecciona el fragmento de nuevo.", "The local version changed. Select the passage again."); hooks.render(); return true; }
    await perform(async () => {
      await hooks.command<Snapshot>("archive_add_annotation", { id, block: anchor.block, quote: anchor.quote, prefix: anchor.prefix, suffix: anchor.suffix, note: savedNote });
      draft = null; note = ""; await hooks.refresh(); hooks.notify(t("Nota guardada en esta versión local.", "Note saved to this local version."), "success");
    });
    return true;
  }
  function bind() {
    if (!globalSelectionListener) { document.addEventListener("selectionchange", updateSelection); globalSelectionListener = true; }
    const dialog = document.getElementById("article-dialog");
    if (!(dialog instanceof HTMLDialogElement) || !requested) return;
    if (!boundDialogs.has(dialog)) {
      dialog.addEventListener("pointerdown", event => {
        if (event.target instanceof Element && event.target.closest("[data-action='article-share-selection']") && shareSelection) event.preventDefault();
      });
      dialog.addEventListener("close", () => { requested = false; selection = null; if (reader.kind === "loading") { generation++; reader = { kind: "idle" }; } document.querySelector<HTMLElement>(`[data-action='article-open'][data-id="${CSS.escape(restoreId)}"]`)?.focus({ preventScroll: true }); });
      boundDialogs.add(dialog);
    }
    if (!dialog.open) { dialog.showModal(); if (!focusNote) dialog.querySelector<HTMLElement>("#article-reader-title")?.focus({ preventScroll: true }); }
    if (focusNote) {
      focusNote = false; const currentDraft = draft;
      queueMicrotask(() => { const currentDialog = document.getElementById("article-dialog"); if (draft === currentDraft && currentDialog instanceof HTMLDialogElement && currentDialog.open) currentDialog.querySelector<HTMLElement>("#article-note")?.focus(); });
    }
  }
  return { renderInspector, open, click, change, submit, bind };
}
