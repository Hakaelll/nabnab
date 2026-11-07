import "./audio-library.css";
import { audioTitle, isAudiobook, type createAudioPlayer } from "./audio-player";
import { escapeHtml as h, type Copy, type LibraryItem, type Snapshot } from "./contracts";
import { detailsFor } from "./library-features";
import type { AudioPlaylist, AudioPlaylistEntry, AudioTags, Lyric } from "./library-types";
import { durationLabel } from "./presentation";
import { action, icon, option } from "./views-common";

type Category = "music" | "podcasts" | "books" | "playlists";
type TagField = Exclude<keyof AudioTags, "embeddedCover">;
type Column = "locator" | "id" | "title" | "artist" | "duration" | "version";
interface Candidate { id: string; title: string; artist: string | null; album: string | null; duration: number | null; versionMatches: boolean }
interface ImportedRow { entry: AudioPlaylistEntry; artist: string | null; duration: number | null; version: string | null; candidates: Candidate[] }
type ImportPreview =
  | { kind: "csv"; id: string; name: string; columns: string[]; sample: string[][]; count: number }
  | { kind: "list"; name: string; rows: ImportedRow[] };
const categories: Category[] = ["music", "podcasts", "books", "playlists"];
const tagFields: TagField[] = ["title", "artist", "albumArtist", "album", "genre", "disc", "track"];
const columns: Column[] = ["locator", "id", "title", "artist", "duration", "version"];
const columnAliases: Record<Column, string[]> = {
  locator: ["path", "filepath", "location", "url", "uri", "ruta"],
  id: ["id", "trackid", "identifier", "identificador"], title: ["title", "trackname", "track", "título", "titulo"],
  artist: ["artist", "artistname", "artista"], duration: ["duration", "durationms", "seconds", "duración", "duracion"],
  version: ["version", "versión", "recording", "edición"],
};
const speeds = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
const PAGE_SIZE = 60;

interface Hooks {
  snapshot(): Snapshot;
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  refresh(): Promise<void>;
  render(): void;
  inspect(id: string): void;
  invalidateCover(id: string): void;
  importMedia(): void;
  notify(message: string, kind?: "info" | "success" | "error"): void;
  showError(error: unknown): void;
  player: ReturnType<typeof createAudioPlayer>;
  t: Copy;
}

function albumKey(item: LibraryItem) {
  const tags = item.details?.audio;
  return JSON.stringify([tags?.albumArtist || tags?.artist || "", tags?.album || ""]);
}

function bookKey(item: LibraryItem) {
  return item.details?.audiobook?.bookId ?? item.id;
}

function numberOrder(value: number | null | undefined) { return value ?? Number.MAX_SAFE_INTEGER; }

function compareTracks(left: LibraryItem, right: LibraryItem) {
  const a = left.details?.audio, b = right.details?.audio;
  return speeds.compare(a?.albumArtist || a?.artist || "", b?.albumArtist || b?.artist || "")
    || speeds.compare(a?.album || "", b?.album || "")
    || (a?.disc ?? 1) - (b?.disc ?? 1)
    || numberOrder(a?.track) - numberOrder(b?.track)
    || speeds.compare(audioTitle(left), audioTitle(right))
    || left.id.localeCompare(right.id);
}

function compareChapters(left: LibraryItem, right: LibraryItem) {
  return speeds.compare(left.details?.audiobook?.bookTitle ?? audioTitle(left), right.details?.audiobook?.bookTitle ?? audioTitle(right))
    || numberOrder(left.details?.audiobook?.chapter) - numberOrder(right.details?.audiobook?.chapter)
    || compareTracks(left, right);
}

export function createAudioLibrary(hooks: Hooks) {
  const { t, player } = hooks;
  let category: Category = "music";
  let artist = "", album = "", genre = "", book = "", query = "";
  let offset = 0;
  let selected = new Set<string>();
  let tagDraft: Partial<Record<TagField, string>> | null = null;
  let tagPreview = false;
  let editing: { id: string | null; name: string; entries: AudioPlaylistEntry[] } | null = null;
  let activeList = "";
  let addTo = "";
  let busy = false;
  let request = 0;
  let lyricOpen = false;
  let lyrics: { id: string; lines: Lyric[] } | null = null;
  let imported: ImportPreview | null = null;
  let mapping: Partial<Record<Column, number>> = {};
  let milliseconds = false;
  let importPage = 0;
  const resolutions = new Map<number, string>();

  const items = () => hooks.snapshot().library.filter(item => item.kind === "audio");
  const itemFor = (id: string) => hooks.snapshot().library.find(item => item.id === id && item.kind === "audio");
  const lists = () => hooks.snapshot().audioPlaylists ?? [];
  const active = () => lists().find(list => list.id === activeList);
  const tagName = (field: TagField) => ({
    title: t("Título", "Title"), artist: t("Artista", "Artist"), albumArtist: t("Artista del álbum", "Album artist"),
    album: t("Álbum", "Album"), genre: t("Género", "Genre"), disc: t("Disco", "Disc"), track: t("Pista", "Track"),
  })[field];
  const categoryName = (value: Category) => ({
    music: t("Música", "Music"), podcasts: "Podcasts", books: t("Audiolibros", "Audiobooks"), playlists: t("Listas", "Playlists"),
  })[value];
  const localEntry = (item: LibraryItem): AudioPlaylistEntry => ({ kind: "local", id: item.id, title: audioTitle(item).slice(0, 256) });
  const noMetadata = () => t("Sin etiqueta", "Untagged");
  const disabled = () => busy ? "disabled" : "";

  async function run(task: () => Promise<void>) {
    if (busy) return;
    busy = true;
    hooks.render();
    try { await task(); }
    catch (error) { hooks.showError(error); }
    finally { busy = false; hooks.render(); }
  }

  function filterTracks(all: LibraryItem[]) {
    const podcastIds = new Set(hooks.snapshot().sources?.filter(source => source.kind === "podcast").map(source => source.id));
    const isPodcast = (item: LibraryItem) => item.details?.sourceIds.some(id => podcastIds.has(id))
      || item.details?.platform === "Podcast RSS";
    const byCategory = all.filter(item => category === "books" ? isAudiobook(item)
      : category === "podcasts" ? !isAudiobook(item) && isPodcast(item)
      : !isAudiobook(item) && !isPodcast(item));
    const needle = query.trim().toLocaleLowerCase();
    const results = byCategory.filter(item => {
      const meta = item.details?.audio;
      if (category === "music" && (artist && artist !== JSON.stringify(meta?.artist ?? "")
        || album && album !== albumKey(item) || genre && genre !== JSON.stringify(meta?.genre ?? ""))) return false;
      if (category === "books" && book && bookKey(item) !== book) return false;
      return !needle || [audioTitle(item), item.name, meta?.artist, meta?.albumArtist, meta?.album, meta?.genre, item.details?.audiobook?.bookTitle]
        .filter(Boolean).join(" ").toLocaleLowerCase().includes(needle);
    });
    results.sort(category === "books" ? compareChapters : category === "podcasts" ? (a, b) => b.addedAt - a.addedAt : compareTracks);
    return { byCategory, results };
  }

  function filterSelect(id: string, label: string, values: Map<string, string>, chosen: string) {
    const entries = [...values].sort((a, b) => speeds.compare(a[1], b[1]));
    return `<label>${h(label)}<select id="audio-${id}">${option("", t("Todos", "All"), chosen)}${entries.map(([key, value]) => option(key, value, chosen)).join("")}</select></label>`;
  }

  function filters(all: LibraryItem[]) {
    const artists = new Map<string, string>();
    const albums = new Map<string, string>();
    const genres = new Map<string, string>();
    const books = new Map<string, string>();
    for (const item of all) {
      const meta = item.details?.audio;
      artists.set(JSON.stringify(meta?.artist ?? ""), meta?.artist ?? noMetadata());
      albums.set(albumKey(item), [meta?.album ?? noMetadata(), meta?.albumArtist ?? meta?.artist].filter(Boolean).join(" · "));
      genres.set(JSON.stringify(meta?.genre ?? ""), meta?.genre ?? noMetadata());
      books.set(bookKey(item), item.details?.audiobook?.bookTitle ?? audioTitle(item));
    }
    return `<div class="audio-filters"><label>${t("Buscar audio", "Search audio")}<input id="audio-query" type="search" value="${h(query)}" placeholder="${t("Título o artista", "Title or artist")}"></label>
      ${category === "music" ? filterSelect("artist", tagName("artist"), artists, artist) + filterSelect("album", tagName("album"), albums, album) + filterSelect("genre", tagName("genre"), genres, genre)
        : category === "books" ? filterSelect("book", t("Libro", "Book"), books, book) : ""}</div>`;
  }

  function row(item: LibraryItem) {
    const tags = item.details?.audio;
    const chapter = item.details?.audiobook;
    const title = audioTitle(item);
    const description = chapter ? [chapter.bookTitle, chapter.reader].filter(Boolean).join(" · ")
      : [tags?.artist ?? item.details?.creator, tags?.album, tags?.genre].filter(Boolean).join(" · ");
    const sequence = chapter ? `${t("Cap.", "Ch.")} ${chapter.chapter}`
      : tags?.track ? `${tags.disc ? `${tags.disc}.` : ""}${tags.track}` : "";
    const attrs = `data-id="${h(item.id)}" ${item.missing ? "disabled" : ""}`;
    return `<li class="audio-row ${player.current()?.id === item.id ? "is-playing" : ""}">
      <input type="checkbox" data-action="audio-select" data-id="${h(item.id)}" aria-label="${h(t("Seleccionar ", "Select ") + title)}" ${selected.has(item.id) ? "checked" : ""} ${busy || !selected.has(item.id) && selected.size >= 200 ? "disabled" : ""}>
      <span class="audio-track-number">${h(sequence)}</span><span class="audio-cover"${item.details?.audioCover || tags?.embeddedCover ? ` data-thumbnail-id="${h(item.id)}"` : ""} aria-hidden="true">${icon("audio")}</span>
      <button class="audio-track" data-action="audio-play" ${attrs}><strong>${h(title)}</strong><small>${h(item.missing ? t("Archivo no encontrado", "File not found") : description || noMetadata())}</small></button>
      <span class="audio-track-duration">${item.duration ? durationLabel(item.duration) : "—"}</span>
      ${action("audio-add-queue", t("Añadir a la cola de audio", "Add to audio queue"), "add", "icon-button", attrs)}
      ${action("audio-inspect", t("Detalles del archivo", "File details"), "info", "icon-button", `data-id="${h(item.id)}"`)}</li>`;
  }

  function editTags() {
    if (!tagDraft) return "";
    const patch = tagDraft;
    const changed = tagFields.filter(field => patch[field] !== undefined);
    const affected = [...selected].flatMap(id => { const item = itemFor(id); return item ? [item] : []; });
    return `<form id="audio-tag-form" class="audio-editor"><div class="page-heading"><div><h2>${t("Editar etiquetas de Biblioteca", "Edit library tags")}</h2>
      <p>${t("Marca los campos a cambiar. Un campo marcado y vacío borra ese valor.", "Select the fields to change. A selected empty field clears its value.")}</p></div>
      ${action("audio-cancel-tags", t("Cerrar", "Close"), "close", "quiet", disabled())}</div><div class="audio-tag-fields">${tagFields.map(field =>
        `<div><label><input type="checkbox" data-action="audio-tag-enable" data-field="${field}" ${patch[field] !== undefined ? "checked" : ""} ${disabled()}> ${tagName(field)}</label>
        <input id="audio-tag-${field}" aria-label="${h(tagName(field))}" ${field === "disc" || field === "track" ? 'type="number" min="1" max="65535"' : 'maxlength="256"'} value="${h(patch[field] ?? "")}" ${patch[field] === undefined || busy ? "disabled" : ""}></div>`).join("")}</div>
      <p class="field-note">${t("Se guardan en nabnab. Los archivos de audio se conservan.", "Changes are saved in nabnab. Audio files are preserved.")}</p>
      ${tagPreview ? `<div class="audio-tag-preview" role="region" aria-label="${t("Previsualización de etiquetas", "Tag preview")}"><strong>${affected.length} ${t("archivos", "files")}</strong>
        ${affected.slice(0, 4).map(item => `<div><span>${h(audioTitle(item))}</span><ul>${changed.map(field => `<li>${tagName(field)} · ${h(item.details?.audio?.[field] ?? "—")} → ${h(patch[field] || "—")}</li>`).join("")}</ul></div>`).join("")}</div>` : ""}
      <div class="button-group"><button type="submit" class="button primary" ${busy || !changed.length ? "disabled" : ""}>${tagPreview ? t("Guardar etiquetas", "Save tags") : t("Previsualizar cambios", "Preview changes")}</button>
      ${tagPreview ? action("audio-tag-copies", t("Crear copias con etiquetas", "Create tagged copies"), "copy", "secondary",
        busy || affected.some(item => item.missing || !/\.(mp3|m4a|m4b|flac|ogg|opus)$/i.test(item.name)) ? "disabled" : "") : ""}</div>
      ${tagPreview ? `<p class="field-note">${t("Las copias pasan por Descargas. Formatos admitidos: MP3, M4A, M4B, FLAC, OGG y Opus.", "Copies use Downloads. Supported formats: MP3, M4A, M4B, FLAC, OGG and Opus.")}</p>` : ""}</form>`;
  }

  function selection() {
    if (!selected.size) return "";
    const pending = [...selected].filter(id => !itemFor(id)?.details?.audio && !itemFor(id)?.missing).length;
    const playlists = lists();
    return `<div class="audio-selection"><strong>${selected.size} ${t("en selección", "selected")}</strong><div class="button-group">
      ${action("audio-edit-tags", t("Editar etiquetas", "Edit tags"), "settings", "secondary", disabled())}
      ${pending ? action("audio-read-tags", t("Leer etiquetas pendientes", "Read pending tags"), "file", "quiet", disabled()) : ""}
      ${action("audio-create-list", t("Crear lista", "Create playlist"), "add", "quiet", disabled())}
      ${action("audio-clear", t("Quitar selección", "Clear selection"), undefined, "quiet", disabled())}</div>
      ${playlists.length ? `<div class="audio-add-list"><label>${t("Añadir a una lista", "Add to a playlist")}<select id="audio-add-list">${option("", t("Elige lista", "Choose playlist"), addTo)}${playlists.map(list => option(list.id, list.name, addTo)).join("")}</select></label>
      ${action("audio-add-list", t("Añadir", "Add"), "add", "secondary", busy || !addTo ? "disabled" : "")}</div>` : ""}</div>`;
  }

  function playlistView() {
    const list = active();
    const entries = editing?.entries ?? list?.entries ?? [];
    const name = editing?.name ?? list?.name ?? "";
    return `<div class="audio-playlists"><div class="audio-filters"><label>${t("Lista guardada", "Saved playlist")}<select id="audio-active-list" ${disabled()}>${option("", t("Elige lista", "Choose playlist"), activeList)}${lists().map(item => option(item.id, item.name, activeList)).join("")}</select></label>
      ${action("audio-create-list", t("Nueva lista", "New playlist"), "add", "secondary", disabled())}
      ${action("audio-import-list", t("Importar lista", "Import playlist"), "file", "secondary", disabled())}
      ${list && !editing ? action("audio-edit-list", t("Editar lista", "Edit playlist"), "settings", "quiet", disabled()) : ""}</div>
      ${editing ? `<form id="audio-playlist-form" class="audio-editor"><label>${t("Nombre de la lista", "Playlist name")}<input id="audio-playlist-name" value="${h(name)}" required maxlength="100" ${disabled()}></label></form>` : list ? `<h2>${h(list.name)}</h2>` : ""}
      ${entries.length ? `<ol class="audio-rows audio-list-entries">${entries.map((entry, index) => {
        const item = entry.kind === "local" ? itemFor(entry.id) : undefined;
        const playable = item && !item.missing;
        const description = playable ? item?.details?.audio?.artist ?? t("Archivo local", "Local file")
          : entry.kind === "remote" ? t("URL remota", "Remote URL") : t("Archivo ausente", "Missing file");
        return `<li class="audio-playlist-entry"><span>${index + 1}</span><div><strong>${h(item ? audioTitle(item) : entry.title)}</strong><small>${h(description)}</small></div>
          ${editing ? `<div class="button-group">${action("audio-list-up", t("Subir", "Move up"), undefined, "quiet", `data-index="${index}" ${index === 0 ? "disabled" : disabled()}`)}${action("audio-list-down", t("Bajar", "Move down"), undefined, "quiet", `data-index="${index}" ${index + 1 === entries.length ? "disabled" : disabled()}`)}
            ${action("audio-list-delete-entry", t("Quitar de lista", "Remove from playlist"), "close", "icon-button", `data-index="${index}" ${disabled()}`)}</div>` : action("audio-list-play", t("Reproducir pista", "Play track"), "play", "icon-button", `data-index="${index}" ${playable ? "" : "disabled"}`)}</li>`;
      }).join("")}</ol>` : `<p class="field-note">${t("Selecciona pistas en Audio para añadirlas a una lista.", "Select tracks in Audio to add them to a playlist.")}</p>`}
      ${editing ? `<div class="button-group"><button class="button primary" type="submit" form="audio-playlist-form" ${disabled()}>${t("Guardar lista", "Save playlist")}</button>
        ${action("audio-cancel-list", t("Cancelar", "Cancel"), undefined, "quiet", disabled())}
        ${editing.id ? action("audio-remove-list", t("Eliminar lista", "Delete playlist"), undefined, "quiet", disabled()) : ""}</div>` : ""}</div>`;
  }

  function importView(preview: ImportPreview) {
    const header = `<div class="page-heading"><div><h2>${h(preview.name)}</h2><p>${t("Revisa el contenido antes de guardarlo como lista.", "Review the contents before saving as a playlist.")}</p></div>${action("audio-cancel-import", t("Cerrar importación", "Close import"), "close", "quiet", disabled())}</div>`;
    if (preview.kind === "csv") {
      const labels: Record<Column, string> = {
        locator: t("Ruta o URL", "Path or URL"), id: t("Identificador", "Identifier"), title: tagName("title"),
        artist: tagName("artist"), duration: t("Duración", "Duration"), version: t("Versión", "Version"),
      };
      return `<section class="audio-editor">${header}<p class="field-note">${preview.count} ${t("filas en el CSV", "CSV rows")}</p>
        <div class="audio-table-wrap"><table class="audio-import-table"><thead><tr>${preview.columns.map((name, index) => `<th scope="col">${h(name || t("Columna ", "Column ") + (index + 1))}</th>`).join("")}</tr></thead>
        <tbody>${preview.sample.map(row => `<tr>${preview.columns.map((_, index) => `<td>${h(row[index] ?? "")}</td>`).join("")}</tr>`).join("")}</tbody></table></div>
        <form id="audio-csv-mapping" class="audio-import-map"><div class="audio-filters">${columns.map(field => `<label>${labels[field]}<select id="audio-csv-${field}" ${disabled()}>${option("", t("Sin asignar", "Unassigned"), String(mapping[field] ?? ""))}${preview.columns.map((name, index) => option(String(index), name || String(index + 1), String(mapping[field] ?? ""))).join("")}</select></label>`).join("")}</div>
        <label class="audio-ms"><input type="checkbox" id="audio-csv-ms" ${milliseconds ? "checked" : ""} ${disabled()}> ${t("La duración numérica está en milisegundos", "Numeric duration is in milliseconds")}</label>
        <button class="button primary" type="submit" ${busy || mapping.id === undefined && mapping.title === undefined && mapping.locator === undefined ? "disabled" : ""}>${t("Buscar coincidencias locales", "Find local matches")}</button></form></section>`;
    }
    const start = importPage * 40;
    const unresolved = preview.rows.filter((row, index) => row.entry.kind !== "local" && !resolutions.has(index)).length;
    return `<section class="audio-editor">${header}<p class="field-note">${preview.rows.length} ${t("entradas", "entries")} · ${unresolved} ${t("referencias remotas o ausentes", "remote or missing references")}</p>
      <p class="field-note">${t("Elige una grabación para resolver una sugerencia. Las URLs quedan como referencias y no se descargan.", "Choose a recording to resolve a suggestion. URLs remain references and are never downloaded.")}</p>
      <ol class="audio-rows audio-import-rows" start="${start + 1}">${preview.rows.slice(start, start + 40).map((row, offset) => {
        const index = start + offset;
        const original = resolutions.has(index) ? t("Copia local elegida", "Local copy chosen")
          : row.entry.kind === "local" ? t("Archivo local", "Local file") : row.entry.kind === "remote" ? t("URL remota", "Remote URL") : t("Archivo ausente", "Missing file");
        const metadata = [row.artist, row.version, row.duration === null ? "" : durationLabel(row.duration), original].filter(Boolean).join(" · ");
        return `<li class="audio-playlist-entry"><span>${index + 1}</span><div><strong>${h(row.entry.title)}</strong><small>${h(metadata)}</small></div>
          ${row.candidates.length ? `<label class="audio-match">${t("Coincidencia local", "Local match")}<select id="audio-import-match-${index}" data-audio-match="${index}" ${disabled()}>
            ${option("", t("Conservar referencia original", "Keep original reference"), resolutions.get(index) ?? "")}
            ${row.candidates.map(candidate => option(candidate.id, [candidate.title, candidate.artist, candidate.album, candidate.duration === null ? "" : durationLabel(candidate.duration), candidate.versionMatches ? t("Versión indicada", "Reported version") : ""].filter(Boolean).join(" · "), resolutions.get(index) ?? "")).join("")}</select></label>` : ""}</li>`;
      }).join("")}</ol>
      ${preview.rows.length > 40 ? `<div class="library-pagination"><span>${start + 1}–${Math.min(preview.rows.length, start + 40)} / ${preview.rows.length}</span>
        ${action("audio-import-back", t("Anterior", "Previous"), undefined, "quiet", importPage ? disabled() : "disabled")}
        ${action("audio-import-next", t("Siguiente", "Next"), undefined, "quiet", start + 40 < preview.rows.length ? disabled() : "disabled")}</div>` : ""}
      ${action("audio-import-review", t("Preparar lista", "Prepare playlist"), "check", "primary", disabled())}</section>`;
  }

  function context() {
    const item = player.current();
    const queue = player.nextEntries();
    return `<aside class="audio-context" aria-label="${t("Reproducción actual", "Current playback")}"><h2>${t("A continuación", "Up next")}</h2>
      ${queue.length ? `<ol class="audio-next">${queue.slice(0, 16).map(entry => `<li><span>${h(itemFor(entry.id) ? audioTitle(itemFor(entry.id)!) : t("Archivo ausente", "Missing file"))}</span>
        ${action("audio-remove-queue", t("Quitar de la cola", "Remove from queue"), "close", "icon-button", `data-index="${entry.position}"`)}</li>`).join("")}</ol>
        ${queue.length > 16 ? `<small>+${queue.length - 16}</small>` : ""}` : `<p class="field-note">${t("Las pistas siguientes aparecerán aquí.", "Upcoming tracks appear here.")}</p>`}
      ${item ? `<div class="audio-current"><h2>${h(audioTitle(item))}</h2>
        ${isAudiobook(item) ? `<div class="button-group">${action("audio-add-bookmark", t("Marcar posición", "Bookmark position"), "add", "secondary", disabled())}</div>
        ${detailsFor(item).chapters.map(chapter => action("audio-seek-chapter", `${durationLabel(chapter.start)} · ${chapter.title}`, "play", "quiet", `data-time="${chapter.start}"`)).join("")}
        ${detailsFor(item).bookmarks.map(bookmark => action("audio-seek-chapter", `${durationLabel(bookmark.time)} · ${bookmark.name}`, "play", "quiet", `data-time="${bookmark.time}"`)).join("")}` : ""}
        <div class="button-group">${action("audio-open-lyrics", lyricOpen ? t("Cerrar letras", "Close lyrics") : t("Letras locales", "Local lyrics"), "list", "quiet", disabled())}</div>
        <div class="button-group">${action("audio-cover-choose", t("Elegir portada", "Choose cover"), "image", "quiet", disabled())}
        ${item.details?.audioCover ? action("audio-cover-remove", t("Quitar portada local", "Remove local cover"), undefined, "quiet", disabled()) : ""}</div>
        ${lyricOpen ? `<div class="audio-lyrics">${lyrics?.id === item.id && lyrics.lines.length ? `<ol>${lyrics.lines.map(line => `<li>${action("audio-seek-chapter", line.text, undefined, "quiet", `data-time="${line.time}"`)}</li>`).join("")}</ol>` : `<p class="field-note">${t("No hay letras locales asociadas a esta pista.", "No local lyrics are linked to this track.")}</p>`}
          ${action("audio-choose-lyrics", t("Asociar archivo LRC", "Link LRC file"), "file", "secondary", disabled())}</div>` : ""}</div>` : ""}</aside>`;
  }

  function render() {
    const { byCategory, results } = filterTracks(items());
    offset = Math.min(offset, Math.max(0, Math.floor((results.length - 1) / PAGE_SIZE) * PAGE_SIZE));
    const currentBook = category === "books" ? results.filter(item => !item.missing && (item.details?.playbackPosition ?? 0) > 0 && (item.duration === null || detailsFor(item).playbackPosition < item.duration - 2))
      .sort((left, right) => (right.details?.lastPlayedAt ?? 0) - (left.details?.lastPlayedAt ?? 0))[0] : undefined;
    return `<section class="audio-library"><div class="page-heading"><div><h1>${t("Audio", "Audio")}</h1>
      <p>${t("Música, podcasts y audiolibros de tu biblioteca.", "Music, podcasts and audiobooks from your library.")}</p></div>
      ${action("audio-import-media", t("Importar audio", "Import audio"), "add", "secondary", disabled())}</div>
      <nav class="file-source-tabs" aria-label="${t("Tipo de audio", "Audio type")}">${categories.map(value => `<button type="button" data-action="audio-category" data-id="${value}" ${value === category ? 'aria-current="page"' : ""}>${categoryName(value)}</button>`).join("")}</nav>
      <div class="audio-layout"><div class="audio-content">${category === "playlists" ? imported ? importView(imported) : playlistView() : `${filters(byCategory)}
        ${currentBook ? action("audio-play", t("Continuar · ", "Continue · ") + audioTitle(currentBook), "play", "secondary", `data-id="${h(currentBook.id)}"`) : ""}
        ${selection()}${editTags()}${request ? `<p role="status" class="field-note">${t("Leyendo etiquetas locales…", "Reading local tags…")} ${action("audio-cancel-read", t("Cancelar", "Cancel"), undefined, "quiet")}</p>` : ""}
        ${results.length ? `<ol class="audio-rows">${results.slice(offset, offset + PAGE_SIZE).map(row).join("")}</ol>`
          : `<div class="empty-state">${icon("audio")}<h2>${t("Sin audio en esta vista", "No audio in this view")}</h2><p>${t("Importa archivos o cambia los filtros.", "Import files or change the filters.")}</p></div>`}
        ${results.length > PAGE_SIZE ? `<div class="library-pagination"><span>${offset + 1}–${Math.min(offset + PAGE_SIZE, results.length)} / ${results.length}</span>
          ${action("audio-page-back", t("Anterior", "Previous"), undefined, "quiet", offset === 0 ? "disabled" : "")}${action("audio-page-next", t("Siguiente", "Next"), undefined, "quiet", offset + PAGE_SIZE >= results.length ? "disabled" : "")}</div>` : ""}`}</div>${context()}</div></section>`;
  }

  async function saveList(id: string | null, name: string, entries: AudioPlaylistEntry[]) {
    const saved = await hooks.command<AudioPlaylist[]>("audio_playlist_save", { id, name, entries });
    activeList = id ?? saved.at(-1)?.id ?? "";
    editing = null;
    await hooks.refresh();
  }

  async function readLyrics(choose: boolean) {
    const item = player.current();
    if (!item) return;
    await run(async () => {
      const lines = await hooks.command<Lyric[] | null>(choose ? "audio_lyrics_choose" : "audio_lyrics", { id: item.id });
      if (lines) lyrics = { id: item.id, lines };
    });
  }

  function playTrack(id: string) {
    const results = filterTracks(items()).results;
    const item = results.find(item => item.id === id);
    if (!item) return;
    const ids = (category === "books" ? results.filter(candidate => bookKey(candidate) === bookKey(item)) : results).map(item => item.id);
    lyricOpen = false;
    player.play(ids, ids.indexOf(id));
  }

  async function click(name: string, button: HTMLElement): Promise<boolean> {
    const id = button.dataset.id ?? "";
    const index = Number(button.dataset.index);
    const list = active();
    switch (name) {
      case "audio-import-media": hooks.importMedia(); break;
      case "audio-category":
        if (id === "music" || id === "books" || id === "podcasts" || id === "playlists") {
          category = id; offset = 0; hooks.render();
        } break;
      case "audio-play": playTrack(id); break;
      case "audio-inspect": hooks.inspect(id); break;
      case "audio-add-queue": player.add(id); break;
      case "audio-remove-queue": player.remove(index); break;
      case "audio-page-back": offset = Math.max(0, offset - PAGE_SIZE); hooks.render(); break;
      case "audio-page-next": offset += PAGE_SIZE; hooks.render(); break;
      case "audio-clear": selected.clear(); tagDraft = null; tagPreview = false; hooks.render(); break;
      case "audio-edit-tags": tagDraft = {}; tagPreview = false; hooks.render(); break;
      case "audio-cancel-tags": tagDraft = null; tagPreview = false; hooks.render(); break;
      case "audio-tag-copies": if (tagPreview && tagDraft) await run(async () => {
        await hooks.command("audio_tags_queue_copies", { ids: [...selected], patch: tagDraft, clientRequest: crypto.randomUUID() });
        tagDraft = null; tagPreview = false;
        await hooks.refresh();
        hooks.notify(t("Copias añadidas a Descargas.", "Copies added to Downloads."), "success");
      }); break;
      case "audio-read-tags": await run(async () => {
        const ids = [...selected].filter(id => !itemFor(id)?.details?.audio && !itemFor(id)?.missing).slice(0, 50);
        if (!ids.length) return;
        request = Date.now() * 1000 + 803;
        hooks.render();
        try {
          const report = await hooks.command<{ updated: number; unavailable: number }>("audio_tags_read", { ids, requestId: request });
          await hooks.refresh();
          hooks.notify(`${report.updated} ${t("etiquetas leídas", "tags read")}${report.unavailable ? ` · ${report.unavailable} ${t("no disponibles", "unavailable")}` : ""}`, "info");
        } finally { request = 0; }
      }); break;
      case "audio-cancel-read": if (request) await hooks.command("cancel_file_source_inspection", { requestId: request }); break;
      case "audio-create-list":
        editing = { id: null, name: "", entries: category === "playlists" ? [] : [...selected].flatMap(id => { const item = itemFor(id); return item ? [localEntry(item)] : []; }) };
        category = "playlists"; hooks.render(); document.getElementById("audio-playlist-name")?.focus(); break;
      case "audio-import-list": await run(async () => {
        const preview = await hooks.command<ImportPreview | null>("audio_playlist_choose");
        if (!preview) return;
        imported = preview;
        mapping = {};
        resolutions.clear();
        importPage = 0;
        if (preview.kind === "csv") {
          const labels = preview.columns.map(name => name.trim().toLocaleLowerCase().replace(/[ _-]/g, ""));
          for (const field of columns) {
            const index = labels.findIndex(value => columnAliases[field].includes(value));
            if (index >= 0) mapping[field] = index;
          }
          milliseconds = mapping.duration !== undefined && labels[mapping.duration] === "durationms";
        }
      }); break;
      case "audio-cancel-import": imported = null; resolutions.clear(); hooks.render(); break;
      case "audio-import-back": importPage = Math.max(0, importPage - 1); hooks.render(); break;
      case "audio-import-next": importPage++; hooks.render(); break;
      case "audio-import-review": if (imported?.kind === "list") {
        const preview = imported;
        const entries = preview.rows.map((row, index) => {
          const chosen = row.candidates.find(candidate => candidate.id === resolutions.get(index));
          return chosen ? { kind: "local" as const, id: chosen.id, title: chosen.title } : row.entry;
        });
        editing = { id: null, name: preview.name, entries };
        imported = null; resolutions.clear();
        hooks.render(); document.getElementById("audio-playlist-name")?.focus();
      } break;
      case "audio-edit-list": if (list) { editing = { id: list.id, name: list.name, entries: [...list.entries] }; hooks.render(); } break;
      case "audio-cancel-list": editing = null; hooks.render(); break;
      case "audio-remove-list": if (editing?.id) await run(async () => {
        await hooks.command("audio_playlist_remove", { id: editing?.id });
        activeList = ""; editing = null; await hooks.refresh();
      }); break;
      case "audio-list-up":
      case "audio-list-down":
      case "audio-list-delete-entry": {
        if (!editing || !Number.isInteger(index) || !editing.entries[index]) break;
        const entries = editing.entries;
        const destination = index + (name === "audio-list-up" ? -1 : 1);
        if (name === "audio-list-delete-entry") entries.splice(index, 1);
        else if (destination >= 0 && destination < entries.length) [entries[index], entries[destination]] = [entries[destination], entries[index]];
        hooks.render();
      } break;
      case "audio-list-play": if (list?.entries[index]?.kind === "local") {
        const ids = list.entries.flatMap(entry => entry.kind === "local" ? [entry.id] : []);
        const position = list.entries.slice(0, index).filter(entry => entry.kind === "local").length;
        lyricOpen = false;
        player.play(ids, position);
      } break;
      case "audio-add-list": {
        const target = lists().find(list => list.id === addTo);
        if (target) await run(async () => {
          const entries = [...target.entries, ...[...selected].flatMap(id => { const item = itemFor(id); return item ? [localEntry(item)] : []; })];
          await saveList(target.id, target.name, entries);
          hooks.notify(t("Pistas añadidas a la lista.", "Tracks added to playlist."), "success");
        });
      } break;
      case "audio-seek-chapter": player.seek(Number(button.dataset.time)); break;
      case "audio-add-bookmark": {
        const item = player.current();
        if (item) await run(async () => {
          const time = player.time();
          await hooks.command("library_bookmark", { id: item.id, bookmark: { id: crypto.randomUUID(), name: `${t("Posición", "Position")} ${durationLabel(time)}`, time }, remove: false });
          await hooks.refresh();
        });
      } break;
      case "audio-open-lyrics":
        lyricOpen = !lyricOpen; if (lyricOpen) await readLyrics(false); else hooks.render(); break;
      case "audio-choose-lyrics": await readLyrics(true); break;
      case "audio-cover-choose":
      case "audio-cover-remove": {
        const item = player.current();
        if (item) await run(async () => {
          const changed = await hooks.command<boolean>("audio_cover_choose", { id: item.id, remove: name === "audio-cover-remove" });
          if (changed) {
            hooks.invalidateCover(item.id);
            await hooks.refresh();
            player.render();
          }
        });
      } break;
      default: return false;
    }
    return true;
  }

  function input(element: HTMLElement): boolean {
    if (element instanceof HTMLInputElement) {
      if (element.id === "audio-csv-ms") { milliseconds = element.checked; return true; }
      if (element.id === "audio-query") { query = element.value; offset = 0; hooks.render(); return true; }
      if (element.id === "audio-playlist-name" && editing) { editing.name = element.value; return true; }
      if (element.dataset.action === "audio-select" && element.dataset.id) {
        if (element.checked && selected.size < 200) selected.add(element.dataset.id); else selected.delete(element.dataset.id);
        tagPreview = false; hooks.render(); return true;
      }
      const field = tagFields.find(field => element.dataset.field === field || element.id === `audio-tag-${field}`);
      if (field && tagDraft) {
        if (element.dataset.action === "audio-tag-enable") {
          if (element.checked) tagDraft[field] = ""; else delete tagDraft[field];
          tagPreview = false; hooks.render();
        } else {
          tagDraft[field] = element.value;
          if (tagPreview) { tagPreview = false; hooks.render(); }
        }
        return true;
      }
    }
    if (!(element instanceof HTMLSelectElement)) return false;
    const column = columns.find(field => element.id === `audio-csv-${field}`);
    if (column) {
      if (element.value === "") delete mapping[column]; else mapping[column] = Number(element.value);
      hooks.render(); return true;
    }
    if (element.dataset.audioMatch !== undefined) {
      const index = Number(element.dataset.audioMatch);
      if (element.value) resolutions.set(index, element.value); else resolutions.delete(index);
      hooks.render(); return true;
    }
    switch (element.id) {
      case "audio-artist": artist = element.value; break;
      case "audio-album": album = element.value; break;
      case "audio-genre": genre = element.value; break;
      case "audio-book": book = element.value; break;
      case "audio-active-list": activeList = element.value; editing = null; break;
      case "audio-add-list": addTo = element.value; break;
      default: return false;
    }
    offset = 0;
    hooks.render();
    return true;
  }

  async function submit(form: HTMLFormElement): Promise<boolean> {
    if (form.id === "audio-csv-mapping" && imported?.kind === "csv") {
      const id = imported.id;
      await run(async () => {
        imported = await hooks.command<ImportPreview>("audio_playlist_map", { id, mapping: { ...mapping, milliseconds } });
      });
      return true;
    }
    if (form.id === "audio-playlist-form" && editing) {
      if (form.reportValidity()) await run(async () => {
        const draft = editing;
        if (draft) await saveList(draft.id, draft.name, draft.entries);
      });
      return true;
    }
    if (form.id !== "audio-tag-form") return false;
    if (!tagDraft || !form.reportValidity()) return true;
    if (!tagPreview) { tagPreview = true; hooks.render(); return true; }
    await run(async () => {
      await hooks.command("audio_tags_edit", { ids: [...selected], patch: tagDraft });
      tagDraft = null; tagPreview = false;
      await hooks.refresh();
    });
    return true;
  }

  return { render, click, input, submit };
}
