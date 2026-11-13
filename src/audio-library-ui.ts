import "./audio-library.css";
import { audioTitle, type createAudioPlayer } from "./audio-player";
import { escapeHtml as h, type Copy, type LibraryItem, type Snapshot } from "./contracts";
import { detailsFor } from "./library-features";
import type { AudioPlaylist, AudioPlaylistEntry, AudioTags, Chapter, LibraryDetails, Lyric } from "./library-types";
import { librivoxLanguageLabel } from "./librivox-language";
import { audiobookChapterComplete, audiobookResumePosition, chapterAt, isAudiobook, isPlayable } from "./playback-preferences";
import { durationLabel, errorLabel, searchKey } from "./presentation";
import { action, icon, option, type Icon } from "./views-common";

type Category = "music" | "podcasts" | "books" | "playlists";
type TagField = Exclude<keyof AudioTags, "embeddedCover">;
type Column = "locator" | "id" | "title" | "artist" | "duration" | "version";
interface Candidate { id: string; title: string; artist: string | null; album: string | null; duration: number | null; versionMatches: boolean }
interface ImportedRow { entry: AudioPlaylistEntry; artist: string | null; duration: number | null; version: string | null; candidates: Candidate[] }
interface QueueEntry { position: number; item: LibraryItem | undefined }
type Lyrics =
  | { id: string; kind: "loading" }
  | { id: string; kind: "error"; message: string }
  | { id: string; kind: "ready"; cues: Lyric[]; scrollTop: number };
export type ImportPreview =
  | { kind: "csv"; id: string; name: string; columns: string[]; sample: string[][]; count: number }
  | { kind: "list"; name: string; rows: ImportedRow[]; listId?: string };
const categories: Category[] = ["music", "podcasts", "books", "playlists"];
const categoryIcons: Record<Category, Icon> = { music: "audio", podcasts: "wave", books: "library", playlists: "list" };
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
const SHELF_SIZE = 3;
const CHAPTER_PAGE_SIZE = 30;

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

function librivoxEdition(key: string) {
  return /^librivox:([1-9]\d{0,19})$/.exec(key)?.[1];
}

function editionAvailability(items: readonly LibraryItem[]) {
  const indexed = new Set<string>();
  const saved = new Set<string>();
  const counts = new Set<number>();
  for (const item of items) {
    const chapter = item.details?.audiobook;
    if (!chapter) continue;
    const key = item.details?.contentId ?? String(chapter.chapter);
    indexed.add(key);
    if (isPlayable(item)) saved.add(key);
    if (chapter.editionFiles && Number.isSafeInteger(chapter.editionFiles)) counts.add(chapter.editionFiles);
  }
  const total = counts.size === 1 ? counts.values().next().value : undefined;
  return { saved: saved.size, missing: indexed.size - saved.size, total: total && total >= indexed.size ? total : undefined };
}

function adjacentChapters(group: readonly LibraryItem[], current: LibraryItem): LibraryItem[] {
  const number = current.details?.audiobook?.chapter;
  if (number === undefined) return [current];
  const available = new Map([[number, current]]);
  for (const item of group) {
    const chapter = item.details?.audiobook?.chapter;
    if (chapter !== undefined && !available.has(chapter) && isPlayable(item)) available.set(chapter, item);
  }
  let first = number;
  while (available.has(first - 1)) first--;
  const adjacent: LibraryItem[] = [];
  for (let chapter = first; ; chapter++) {
    const item = available.get(chapter);
    if (!item) break;
    adjacent.push(item);
  }
  return adjacent;
}

function chapterResume(details: LibraryDetails, chapter: Chapter) {
  const saved = details.chapterPositions?.find(point => point.start === chapter.start && point.end === chapter.end);
  const position = saved?.position ?? details.playbackPosition;
  return position > chapter.start && position < chapter.end ? position : chapter.start;
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

interface TrackView {
  query: string;
  offset: number;
  shelfOffset: number;
  selected: Set<string>;
  tags: { patch: Partial<Record<TagField, string>>; preview: boolean } | null;
}

function createTrackView(): TrackView {
  return { query: "", offset: 0, shelfOffset: 0, selected: new Set(), tags: null };
}

export function createAudioLibrary(hooks: Hooks) {
  const { t, player } = hooks;
  let category: Category = "music";
  const views = { music: createTrackView(), podcasts: createTrackView(), books: createTrackView(), playlists: createTrackView() };
  let artist = "", album = "", genre = "", book = "";
  let chapterOffset = 0;
  let editing: { id: string | null; name: string; entries: AudioPlaylistEntry[] } | null = null;
  let activeList = "";
  let addTo = "";
  let busy = false;
  let request = 0;
  let lyricOpen = false;
  let lyricFollow = true;
  let choosingLyrics = false;
  let lyrics: Lyrics | null = null;
  let lyricPanelElement: HTMLElement | null = null;
  let lyricList: HTMLOListElement | null = null;
  let lyricIndex = -1;
  let imported: ImportPreview | null = null;
  let mapping: Partial<Record<Column, number>> = {};
  let milliseconds = false;
  let importPage = 0;
  const resolutions = new Map<number, string>();
  const matchQueries = new Map<number, string>();
  let audition: { index: number; id: string; title: string } | null = null;

  function stopAudition() {
    if (!audition) return;
    const active = player.comparison() && player.current()?.id === audition.id;
    audition = null;
    if (active) player.restoreComparison();
  }

  function resetMatchReview() {
    stopAudition();
    resolutions.clear();
    matchQueries.clear();
    importPage = 0;
  }

  const items = () => hooks.snapshot().library.filter(item => item.kind === "audio");
  const itemFor = (id: string) => hooks.snapshot().library.find(item => item.id === id && item.kind === "audio");
  const isLocalAvailable = (entry: AudioPlaylistEntry) => {
    const item = entry.kind === "local" ? itemFor(entry.id) : undefined;
    return item !== undefined && !item.missing;
  };
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
    const needle = searchKey(views[category].query.trim());
    const results = byCategory.filter(item => {
      const meta = item.details?.audio;
      if (category === "music" && (artist && artist !== JSON.stringify(meta?.artist ?? "")
        || album && album !== albumKey(item) || genre && genre !== JSON.stringify(meta?.genre ?? ""))) return false;
      if (category === "books" && book && bookKey(item) !== book) return false;
      const chapter = item.details?.audiobook;
      return !needle || searchKey([audioTitle(item), item.name, meta?.artist, meta?.albumArtist, meta?.album, meta?.genre,
        chapter?.bookTitle, chapter?.reader, chapter?.language, chapter?.language && librivoxLanguageLabel(chapter.language, t)]
        .filter(Boolean).join(" ")).includes(needle);
    });
    results.sort(category === "books" ? compareChapters : category === "podcasts" ? (a, b) => b.addedAt - a.addedAt : compareTracks);
    return { byCategory, results };
  }

  function filterSelect(id: string, label: string, values: Map<string, string>, chosen: string) {
    const entries = [...values].sort((a, b) => speeds.compare(a[1], b[1]));
    return `<label>${h(label)}<select id="audio-${id}">${option("", t("Todos", "All"), chosen)}${entries.map(([key, value]) => option(key, value, chosen)).join("")}</select></label>`;
  }

  function filters(all: LibraryItem[]) {
    const { query } = views[category];
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
    const placeholder = category === "books" ? t("Título, lector o idioma", "Title, reader or language") : t("Título o artista", "Title or artist");
    return `<div class="audio-filters"><label>${t("Buscar audio", "Search audio")}<input id="audio-query" type="search" value="${h(query)}" placeholder="${placeholder}"></label>
      ${category === "music" ? filterSelect("artist", tagName("artist"), artists, artist) + filterSelect("album", tagName("album"), albums, album) + filterSelect("genre", tagName("genre"), genres, genre)
        : category === "books" ? filterSelect("book", t("Libro", "Book"), books, book) : ""}</div>`;
  }

  function cover(item: LibraryItem, kind: "album" | "book") {
    const hasCover = !item.missing && (item.details?.audioCover || item.details?.audio?.embeddedCover);
    return `<span class="audio-shelf-cover audio-shelf-cover--${kind}" ${hasCover ? `data-thumbnail-id="${h(item.id)}"` : ""} aria-hidden="true">${icon(kind === "album" ? "audio" : "library")}</span>`;
  }

  function albumTracks(all: LibraryItem[]) {
    return category === "music" && album ? all.filter(item => albumKey(item) === album).sort(compareTracks) : [];
  }

  function albumDetails(tracks: LibraryItem[]) {
    const first = tracks[0];
    if (!first?.details?.audio?.album) return "";
    const available = tracks.filter(isPlayable);
    const missing = tracks.length - available.length;
    const art = available.find(item => item.details?.audioCover || item.details?.audio?.embeddedCover) ?? first;
    const current = player.current();
    const canQueue = !(current && isAudiobook(current)) && available.some(item => item.id !== current?.id);
    const length = available.length && available.every(item => item.duration !== null)
      ? durationLabel(available.reduce((sum, item) => sum + (item.duration ?? 0), 0)) : "";
    const summary = [t(`${available.length} ${available.length === 1 ? "pista" : "pistas"}`, `${available.length} ${available.length === 1 ? "track" : "tracks"}`),
      length, missing ? t(`${missing} ${missing === 1 ? "ausente" : "ausentes"}`, `${missing} missing`) : ""].filter(Boolean).join(" · ");
    const artist = first.details.audio.albumArtist || first.details.audio.artist;
    return `<section class="audio-album" aria-labelledby="audio-album-title">
      <div class="audio-album-identity">${cover(art, "album")}<div><h2 id="audio-album-title">${h(first.details.audio.album)}</h2>
        ${artist ? `<p>${h(artist)}</p>` : ""}<p class="field-note">${h(summary)}</p></div></div>
      <div class="button-group">${action("audio-play-album", t("Reproducir álbum", "Play album"), "play", "primary", !available.length ? "disabled" : "")}
        ${action("audio-queue-album", t("Álbum a continuación", "Queue album next"), "next", "secondary", !canQueue ? "disabled" : "")}</div>
    </section>`;
  }

  function editionCount({ saved, total }: ReturnType<typeof editionAvailability>) {
    return total ? t(`${saved} de ${total} disponibles`, `${saved} of ${total} available`)
      : t(`${saved} ${saved === 1 ? "disponible" : "disponibles"}`, `${saved} available`);
  }

  function editionDetails(items: LibraryItem[]) {
    const first = items[0];
    const edition = first?.details?.audiobook;
    const source = edition && librivoxEdition(edition.bookId);
    if (!edition || !source) return "";
    const readers = new Set(items.map(item => item.details?.audiobook?.reader).filter(Boolean));
    const reader = readers.size > 1 ? t("Varias lecturas", "Multiple narrations") : readers.values().next().value;
    const art = items.find(item => !item.missing && (item.details?.audioCover || item.details?.audio?.embeddedCover)) ?? first;
    const availability = editionAvailability(items);
    const { missing } = availability;
    const status = [editionCount(availability), missing ? t(`${missing} ${missing === 1 ? "archivo ausente" : "archivos ausentes"}`,
      `${missing} missing ${missing === 1 ? "file" : "files"}`) : ""].filter(Boolean).join(" · ");
    return `<section class="audio-album audio-edition" aria-labelledby="audio-shelf-title">
      <div class="audio-album-identity">${cover(art, "book")}<div><p class="audio-edition-source">LibriVox</p>
        <h2 id="audio-shelf-title" tabindex="-1">${h(edition.bookTitle)}</h2>
        ${reader ? `<p>${h(reader)}</p>` : ""}<p class="audio-edition-count">${h(status)}</p></div></div>
      ${action("open-audiobook-catalog", t("Ver edición en LibriVox", "View LibriVox edition"), "link", "secondary", `data-id="${source}"`)}</section>`;
  }

  function shelf(items: LibraryItem[], all: LibraryItem[]) {
    if (category !== "music" && category !== "books") return "";
    const music = category === "music";
    const chosen = music ? album : book;
    const groupKey = music ? albumKey : bookKey;
    const titleFor = (item: LibraryItem) => music ? item.details?.audio?.album || t("Sin álbum", "No album")
      : item.details?.audiobook?.bookTitle || audioTitle(item);
    if (chosen) {
      const selectedBook = music ? [] : all.filter(item => bookKey(item) === chosen).sort(compareChapters);
      const detail = music ? albumDetails(albumTracks(all)) : editionDetails(selectedBook);
      const first = music ? items[0] : selectedBook[0];
      return `<div class="audio-shelf-trail">${action("audio-shelf-clear", music ? t("Todos los álbumes", "All albums") : t("Todos los libros", "All books"), "back", "quiet")}${!detail && first ? `<h2 id="audio-shelf-title" tabindex="-1">${h(titleFor(first))}</h2>` : ""}</div>${detail}`;
    }

    const groups = new Map<string, LibraryItem[]>();
    for (const item of items) {
      if (music && !item.details?.audio?.album) continue;
      const key = groupKey(item);
      const group = groups.get(key);
      if (group) group.push(item); else groups.set(key, [item]);
    }
    if (!groups.size) return "";
    const editions = new Map<string, LibraryItem[]>();
    if (!music) {
      for (const item of all) {
        if (!item.details?.audiobook) continue;
        const key = bookKey(item);
        const edition = editions.get(key);
        if (edition) edition.push(item); else editions.set(key, [item]);
      }
    }
    const label = music ? t("Álbumes", "Albums") : t("Estantería", "Bookshelf");
    const entries = [...groups.entries()];
    const view = views[category];
    view.shelfOffset = Math.min(view.shelfOffset, Math.floor((entries.length - 1) / SHELF_SIZE) * SHELF_SIZE);
    const { shelfOffset } = view;
    return `<section class="audio-shelf" aria-label="${label}"><div class="audio-shelf-heading"><h2>${label}</h2>
      ${entries.length > SHELF_SIZE ? `<div class="audio-shelf-navigation"><span>${shelfOffset + 1}–${Math.min(shelfOffset + SHELF_SIZE, entries.length)} / ${entries.length}</span>
        ${action("audio-shelf-back", t("Grupos anteriores", "Previous groups"), "back", "icon-button", `aria-label="${t("Grupos anteriores", "Previous groups")}" ${shelfOffset ? "" : "disabled"}`)}
        ${action("audio-shelf-next", t("Grupos siguientes", "Next groups"), "arrow", "icon-button", `aria-label="${t("Grupos siguientes", "Next groups")}" ${shelfOffset + SHELF_SIZE >= entries.length ? "disabled" : ""}`)}</div>` : ""}</div>
      <ul class="audio-shelf-items">${entries.slice(shelfOffset, shelfOffset + SHELF_SIZE).map(([key, group]) => {
        const item = group[0];
        const art = group.find(candidate => !candidate.missing && (candidate.details?.audioCover || candidate.details?.audio?.embeddedCover)) ?? item;
        const edition = editions.get(key);
        const subtitle = music ? item.details?.audio?.albumArtist || item.details?.audio?.artist : edition ? "LibriVox" : "";
        const chapters = !music && !item.details?.audiobook ? item.details?.chapters.length ?? 0 : 0;
        const total = chapters || group.length;
        const count = music ? (total === 1 ? t("pista", "track") : t("pistas", "tracks"))
          : chapters || item.details?.audiobook ? (total === 1 ? t("capítulo", "chapter") : t("capítulos", "chapters"))
          : (total === 1 ? t("archivo", "file") : t("archivos", "files"));
        return `<li><button type="button" class="audio-shelf-item" data-action="audio-shelf-open" data-id="${h(key)}">
          ${cover(art, music ? "album" : "book")}<span class="audio-shelf-copy"><strong>${h(titleFor(item))}</strong>${subtitle ? `<span>${h(subtitle)}</span>` : ""}<small>${edition ? editionCount(editionAvailability(edition)) : `${total} ${count}`}</small></span>
          ${icon("arrow")}</button></li>`;
      }).join("")}</ul></section>`;
  }

  function continueBook(items: LibraryItem[]) {
    if (category !== "books") return "";
    const chapters = items.filter(item => !book || bookKey(item) === book).sort(compareChapters);
    const books = new Map<string, LibraryItem[]>();
    for (const chapter of chapters) {
      const key = bookKey(chapter);
      const group = books.get(key);
      if (group) group.push(chapter); else books.set(key, [chapter]);
    }
    const recent = chapters.filter(item => !item.missing && (item.details?.playbackPosition ?? 0) > 0)
      .sort((left, right) => (right.details?.lastPlayedAt ?? 0) - (left.details?.lastPlayedAt ?? 0));
    const reviewed = new Set<string>();
    for (const last of recent) {
      const key = bookKey(last);
      if (reviewed.has(key)) continue;
      reviewed.add(key);
      const group = books.get(key) ?? [];
      const completed = audiobookChapterComplete(last.details?.playbackPosition ?? 0, last.duration);
      const adjacent = completed ? adjacentChapters(group, last) : [last];
      const next = completed
        ? adjacent.slice(adjacent.indexOf(last) + 1).find(item => !audiobookChapterComplete(item.details?.playbackPosition ?? 0, item.duration))
        : last;
      if (!next) {
        const end = adjacent.at(-1)?.details?.audiobook?.chapter;
        if (end === undefined) continue;
        let later = Infinity, known = Infinity;
        for (const item of group) {
          const chapter = item.details?.audiobook?.chapter;
          if (chapter === undefined || chapter <= end) continue;
          known = Math.min(known, chapter);
          if (!item.missing) later = Math.min(later, chapter);
        }
        const missingThrough = Number.isFinite(later) ? later - 1 : known;
        if (!Number.isFinite(missingThrough)) continue;
        const missing = missingThrough === end + 1
          ? t(`Falta el capítulo ${end + 1} en Biblioteca.`, `Chapter ${end + 1} is missing from Library.`)
          : t(`Faltan los capítulos ${end + 1}–${missingThrough} en Biblioteca.`, `Chapters ${end + 1}–${missingThrough} are missing from Library.`);
        if (book) return `<p class="audio-edition-gap" role="status">${h(missing)}</p>`;
        const source = librivoxEdition(key);
        return `<section class="audio-continue" aria-label="${t("Audiolibro incompleto", "Incomplete audiobook")}">
          ${cover(last, "book")}<div class="audio-continue-copy"><span>${t("Audiolibro incompleto", "Incomplete audiobook")}</span>
          <h2>${h(last.details?.audiobook?.bookTitle ?? audioTitle(last))}</h2><small>${h(missing)}</small></div>
          ${source ? action("open-audiobook-catalog", t("Ver edición en LibriVox", "View LibriVox edition"), "link", "secondary", `data-id="${source}"`) : ""}</section>`;
      }
      const chapter = next.details?.audiobook;
      const title = chapter?.bookTitle ?? audioTitle(next);
      const position = audiobookResumePosition(next.details?.playbackPosition ?? 0, next.duration);
      const embedded = chapter ? undefined : chapterAt(detailsFor(next).chapters, position);
      const label = chapter ? `${t("Capítulo", "Chapter")} ${chapter.chapter}` : embedded?.title;
      const elapsed = position - (embedded?.start ?? 0);
      const detail = [label, elapsed ? durationLabel(elapsed) : t("Desde el principio", "From the start")].filter(Boolean).join(" · ");
      return `<section class="audio-continue" aria-label="${t("Continuar audiolibro", "Continue audiobook")}">
        ${cover(next, "book")}<div class="audio-continue-copy"><span>${t("Continuar audiolibro", "Continue audiobook")}</span><h2>${h(title)}</h2><small>${h(detail)}</small></div>
        ${action("audio-continue-book", t("Continuar", "Continue"), "play", "primary", `data-id="${h(next.id)}" aria-label="${h(t("Continuar · ", "Continue · ") + title + " · " + detail)}"`)}</section>`;
    }
    return "";
  }

  function embeddedChapters(item: LibraryItem | undefined) {
    if (!item || item.missing) return "";
    const details = detailsFor(item);
    const chapters = details.chapters;
    if (!chapters.length) return "";
    chapterOffset = Math.min(chapterOffset, Math.floor((chapters.length - 1) / CHAPTER_PAGE_SIZE) * CHAPTER_PAGE_SIZE);
    const end = Math.min(chapterOffset + CHAPTER_PAGE_SIZE, chapters.length);
    return `<section class="audio-book-chapters" aria-labelledby="audio-book-chapters-title">
      <header><h2 id="audio-book-chapters-title" tabindex="-1">${t("Capítulos del archivo", "File chapters")}</h2><span>${chapters.length}</span></header>
      <ol start="${chapterOffset + 1}">${chapters.slice(chapterOffset, end).map((chapter, index) => {
        const number = chapterOffset + index;
        const elapsed = chapterResume(details, chapter) - chapter.start;
        const caption = elapsed ? t("Retomar", "Resume") : t("Reproducir", "Play");
        const label = elapsed ? `${t("Desde", "From")} ${durationLabel(elapsed)}` : durationLabel(chapter.end - chapter.start);
        return `<li><span class="audio-book-chapter-number" aria-hidden="true">${number + 1}</span>
          <div class="audio-book-chapter-copy"><strong>${h(chapter.title)}</strong><small>${label}</small></div>
          ${action("audio-play-chapter", caption, "play", "quiet", `id="audio-chapter-${number}" data-id="${h(item.id)}" data-index="${number}" aria-label="${h(`${caption} · ${chapter.title}`)}"`)}</li>`;
      }).join("")}</ol>
      ${chapters.length > CHAPTER_PAGE_SIZE ? `<div class="library-pagination"><span>${chapterOffset + 1}–${end} / ${chapters.length}</span>
        ${action("audio-chapters-back", t("Anteriores", "Previous"), undefined, "quiet", chapterOffset ? "" : "disabled")}
        ${action("audio-chapters-next", t("Siguientes", "Next"), undefined, "quiet", end < chapters.length ? "" : "disabled")}</div>` : ""}
    </section>`;
  }

  function row(item: LibraryItem) {
    const { selected } = views[category];
    const tags = item.details?.audio;
    const chapter = item.details?.audiobook;
    const title = audioTitle(item);
    const description = chapter ? [book ? null : chapter.bookTitle, chapter.reader, chapter.language && librivoxLanguageLabel(chapter.language, t)].filter(Boolean).join(" · ")
      : [tags?.artist ?? item.details?.creator, tags?.album, tags?.genre].filter(Boolean).join(" · ");
    const sequence = chapter ? `${t("Cap.", "Ch.")} ${chapter.chapter}`
      : tags?.track ? `${tags.disc ? `${tags.disc}.` : ""}${tags.track}` : "";
    const attrs = `data-id="${h(item.id)}" ${item.missing ? "disabled" : ""}`;
    return `<li class="audio-row${chapter ? " is-chapter" : ""}${player.current()?.id === item.id ? " is-playing" : ""}">
      <input type="checkbox" data-action="audio-select" data-id="${h(item.id)}" aria-label="${h(t("Seleccionar ", "Select ") + title)}" ${selected.has(item.id) ? "checked" : ""} ${busy || !selected.has(item.id) && selected.size >= 200 ? "disabled" : ""}>
      <span class="audio-track-number">${h(sequence)}</span><span class="audio-cover"${item.details?.audioCover || tags?.embeddedCover ? ` data-thumbnail-id="${h(item.id)}"` : ""} aria-hidden="true">${icon("audio")}</span>
      <button class="audio-track" data-action="audio-play" ${attrs}><strong>${h(title)}</strong><small>${h(item.missing ? t("Archivo no encontrado", "File not found") : description || noMetadata())}</small></button>
      <span class="audio-track-duration">${item.duration ? durationLabel(item.duration) : "—"}</span>
      ${action("audio-add-queue", t("Añadir a la cola de audio", "Add to audio queue"), "add", "icon-button", attrs)}
      ${action("audio-inspect", t("Detalles del archivo", "File details"), "info", "icon-button", `data-id="${h(item.id)}"`)}</li>`;
  }

  function editTags() {
    const { tags, selected } = views[category];
    if (!tags) return "";
    const { patch, preview: tagPreview } = tags;
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

  function selection(next: LibraryItem | undefined, visible: LibraryItem[]) {
    const { selected } = views[category];
    if (!selected.size) return "";
    const hidden = selected.size - visible.filter(item => selected.has(item.id)).length;
    const pending = [...selected].filter(id => !itemFor(id)?.details?.audio && !itemFor(id)?.missing).length;
    const playlists = lists();
    const current = player.current();
    const canQueueNext = [...selected].some(id => {
      const item = itemFor(id);
      return id !== current?.id && isPlayable(item) && !isAudiobook(item);
    });
    return `<div class="audio-selection"><div class="audio-selection-heading"><div class="audio-selection-scope"><strong>${selected.size} ${t("en selección", "selected")}</strong>
      ${hidden ? `<span>${hidden} ${t("fuera de esta página", "outside this page")}</span>` : ""}</div>
      ${next ? `<span>${t("Siguiente", "Next")} · <b>${h(audioTitle(next))}</b></span>` : ""}</div><div class="button-group">
      ${category !== "books" && !(current && isAudiobook(current))
        ? action("audio-queue-next", t("Reproducir a continuación", "Play next"), "next", "secondary", busy || !canQueueNext ? "disabled" : "") : ""}
      ${action("audio-edit-tags", t("Editar etiquetas", "Edit tags"), "settings", "quiet", disabled())}
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
    const rows = entries.map(entry => ({ entry, item: entry.kind === "local" ? itemFor(entry.id) : undefined }));
    const available = rows.flatMap(({ item }) => isPlayable(item) ? [item] : []);
    const remote = entries.filter(entry => entry.kind === "remote").length;
    const missing = entries.length - available.length - remote;
    const current = player.current();
    const canQueue = !(current && isAudiobook(current)) && available.some(item => item.id !== current?.id && !isAudiobook(item));
    const length = available.length && available.every(item => item.duration !== null)
      ? durationLabel(available.reduce((sum, item) => sum + (item.duration ?? 0), 0)) : "";
    const summary = [
      t(`${available.length} ${available.length === 1 ? "pista disponible" : "pistas disponibles"}`,
        `${available.length} available ${available.length === 1 ? "track" : "tracks"}`),
      length,
      remote ? t(`${remote} ${remote === 1 ? "remota" : "remotas"}`, `${remote} remote`) : "",
      missing ? t(`${missing} ${missing === 1 ? "ausente" : "ausentes"}`, `${missing} missing`) : "",
    ].filter(Boolean).join(" · ");
    return `<div class="audio-playlists"><div class="audio-filters"><label>${t("Lista guardada", "Saved playlist")}<select id="audio-active-list" ${disabled()}>${option("", t("Elige lista", "Choose playlist"), activeList)}${lists().map(item => option(item.id, item.name, activeList)).join("")}</select></label>
      ${action("audio-create-list", t("Nueva lista", "New playlist"), "add", "secondary", disabled())}
      ${action("audio-import-list", t("Importar lista", "Import playlist"), "file", "secondary", disabled())}
      ${list && !editing ? action("audio-edit-list", t("Editar lista", "Edit playlist"), "settings", "quiet", disabled()) : ""}</div>
      ${editing ? `<form id="audio-playlist-form" class="audio-editor"><label>${t("Nombre de la lista", "Playlist name")}<input id="audio-playlist-name" value="${h(name)}" required maxlength="100" ${disabled()}></label></form>` : list ? `<section class="audio-playlist-heading" aria-labelledby="audio-playlist-title">
        <div><h2 id="audio-playlist-title">${h(list.name)}</h2><p class="field-note">${h(summary)}</p></div>
        <div class="button-group">
          ${action("audio-list-play-all", t("Reproducir lista", "Play playlist"), "play", "primary", !available.length ? "disabled" : disabled())}
          ${action("audio-list-queue-next", t("Lista a continuación", "Queue playlist next"), "next", "secondary", !canQueue ? "disabled" : disabled())}
          ${remote || missing ? action("audio-rematch-list", t("Revisar referencias", "Review references"), "search", "quiet", disabled()) : ""}
        </div></section>` : ""}
      ${rows.length ? `<ol class="audio-rows audio-list-entries">${rows.map(({ entry, item }, index) => {
        const playable = isPlayable(item);
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
    const rematching = preview.kind === "list" && preview.listId !== undefined;
    const instruction = preview.kind === "csv" ? t("Confirma qué contiene cada columna.", "Confirm what each column contains.")
      : rematching ? t("Elige una grabación para sustituir una referencia en esta lista.", "Choose a recording to replace a reference in this playlist.")
        : t("Compara las grabaciones locales.", "Compare local recordings.");
    const header = `<div class="page-heading"><div><h2 id="audio-import-heading" tabindex="-1">${h(preview.name)}</h2><p>${instruction}</p></div>${action("audio-cancel-import", rematching ? t("Cerrar revisión", "Close review") : t("Cerrar importación", "Close import"), "close", "quiet", disabled())}</div>`;
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
    const localFiles = new Map(items().filter(item => !item.missing).map(item => [item.id, item]));
    const reviewRows = preview.rows.map((row, index) => ({row, index, local: row.entry.kind === "local" && localFiles.has(row.entry.id)}))
      .filter(({local}) => !rematching || !local);
    const unresolved = reviewRows.filter(({index, local}) => !local && !localFiles.has(resolutions.get(index) ?? "")).length;
    const count = preview.rows.length === 1 ? t("1 entrada", "1 entry") : t(`${preview.rows.length} entradas`, `${preview.rows.length} entries`);
    const pending = unresolved === 1 ? t("1 referencia remota o ausente", "1 remote or missing reference")
      : t(`${unresolved} referencias remotas o ausentes`, `${unresolved} remote or missing references`);
    const playing = audition && player.comparison() && player.current()?.id === audition.id ? audition : null;
    return `<section class="audio-editor audio-import-editor">${header}<p class="field-note" role="status">${count} · ${pending}</p>
      ${playing ? `<div class="audio-audition" role="group" aria-label="${t("Escucha de comparación", "Comparison playback")}">
        <div>${icon("audio")}<span>${t("Escuchando para comparar", "Listening to compare")}<strong>${h(playing.title)}</strong></span></div>
        ${action("audio-end-audition", t("Terminar escucha", "End preview"), "close", "quiet")}</div>` : ""}
      ${reviewRows.length ? "" : `<p class="field-note">${t("Todas las grabaciones de esta lista están disponibles.", "All recordings in this playlist are available.")}</p>`}
      <ol class="audio-rows audio-import-rows" start="${start + 1}">${reviewRows.slice(start, start + 40).map(({row, index, local}) => {
        const selected = resolutions.get(index);
        const chosenFile = selected ? localFiles.get(selected) : undefined;
        const original = chosenFile ? t("Copia local elegida", "Local copy chosen")
          : selected ? t("La copia elegida ya no está disponible", "Chosen copy is no longer available")
          : local ? t("Archivo local", "Local file") : row.entry.kind === "remote" ? t("URL remota", "Remote URL") : t("Archivo ausente", "Missing file");
        const metadata = [row.artist, row.version, row.duration === null ? "" : durationLabel(row.duration), original].filter(Boolean).join(" · ");
        const suggestions = row.candidates.filter(candidate => localFiles.has(candidate.id));
        const query = matchQueries.get(index) ?? "";
        const terms = searchKey(query).trim().split(/\s+/).filter(Boolean);
        const fromLibrary = (item: LibraryItem): Candidate => ({
          id: item.id, title: audioTitle(item), artist: item.details?.audio?.artist ?? item.details?.creator ?? null,
          album: item.details?.audio?.album ?? null, duration: item.duration, versionMatches: false,
        });
        const matches: Candidate[] = [];
        if (terms.length) {
          for (const item of localFiles.values()) {
            const candidate = fromLibrary(item);
            const text = searchKey([candidate.title, candidate.artist, candidate.album, item.name].join(" "));
            if (terms.every(term => text.includes(term))) matches.push(candidate);
            if (matches.length === 9) break;
          }
        }
        const candidates = terms.length ? matches.slice(0, 8) : suggestions;
        if (chosenFile && !candidates.some(candidate => candidate.id === chosenFile.id)) {
          candidates.unshift(suggestions.find(candidate => candidate.id === chosenFile.id) ?? fromLibrary(chosenFile));
        }
        const summary = suggestions.length === 1 ? t("Revisar grabación local", "Review local recording")
          : suggestions.length ? `${suggestions.length} ${t("grabaciones locales para comparar", "local recordings to compare")}`
          : t("Buscar en Biblioteca", "Search Library");
        const radio = (id: string) => `<input type="radio" id="audio-match-${index}-${id ? `file-${h(id)}` : "original"}" name="audio-match-${index}" data-audio-match="${index}" value="${h(id)}" ${id === (selected ?? "") ? "checked" : ""} ${disabled()}>`;
        const searchLabel = t("Buscar por título, artista, álbum o archivo", "Search by title, artist, album, or filename");
        return `<li class="audio-playlist-entry" value="${index + 1}"><span>${index + 1}</span><div><strong id="audio-import-title-${index}">${h(row.entry.title)}</strong><small>${h(metadata)}</small></div>
          ${!local ? `<details class="audio-match" id="audio-import-match-${index}"><summary id="audio-match-summary-${index}" aria-describedby="audio-import-title-${index}">${summary}</summary>
            <fieldset><legend class="sr-only">${h(t("Grabaciones locales para ", "Local recordings for ") + row.entry.title)}</legend>
              <label class="audio-match-option">${radio("")}<span>${t("Conservar referencia original", "Keep original reference")}</span></label>
              <label class="search-control audio-match-search">${icon("search")}<input type="search" id="audio-match-search-${index}" data-match-query="${index}"
                value="${h(query)}" placeholder="${searchLabel}" aria-label="${searchLabel}" maxlength="180" ${disabled()}></label>
              <div class="audio-match-candidates">${candidates.map(candidate => {
                const details = [candidate.artist, candidate.album, candidate.duration === null ? "" : durationLabel(candidate.duration), candidate.versionMatches ? t("Coincide el texto de la versión", "Version text matches") : ""].filter(Boolean).join(" · ");
                const filePath = localFiles.get(candidate.id)?.path;
                const active = playing?.id === candidate.id && playing.index === index;
                const listen = active ? t("Terminar escucha", "End preview") : t("Escuchar", "Listen");
                return `<div class="audio-match-candidate"><label class="audio-match-option">${radio(candidate.id)}<span><strong>${h(candidate.title)}</strong>${details ? `<small>${h(details)}</small>` : ""}${filePath ? `<small class="audio-match-path" dir="auto">${h(filePath)}</small>` : ""}</span></label>
                  ${action("audio-audition", listen, active ? "close" : "play", active ? "secondary" : "quiet",
                    `data-id="${h(candidate.id)}" data-index="${index}" aria-label="${h(`${listen} · ${candidate.title}${filePath ? ` · ${filePath}` : ""}`)}" ${disabled()}`)}</div>`;
              }).join("")}</div>
              ${terms.length && !matches.length ? `<p class="field-note" role="status">${t("No hay grabaciones disponibles con esa búsqueda.", "No available recordings match this search.")}</p>` : ""}
              ${matches.length > 8 ? `<p class="field-note" role="status">${t("Acota la búsqueda para ver otras grabaciones.", "Narrow the search to see other recordings.")}</p>` : ""}
            </fieldset></details>` : ""}
          ${!local && !suggestions.length && !selected ? `<small class="field-note">${t("Sin coincidencias locales", "No local matches")}</small>` : ""}</li>`;
      }).join("")}</ol>
      ${reviewRows.length > 40 ? `<div class="library-pagination"><span>${start + 1}–${Math.min(reviewRows.length, start + 40)} / ${reviewRows.length}</span>
        ${action("audio-import-back", t("Anterior", "Previous"), undefined, "quiet", importPage ? disabled() : "disabled")}
        ${action("audio-import-next", t("Siguiente", "Next"), undefined, "quiet", start + 40 < reviewRows.length ? disabled() : "disabled")}</div>` : ""}
      ${action("audio-import-review", rematching ? t("Preparar cambios", "Prepare changes") : t("Preparar lista", "Prepare playlist"), "check", "primary", rematching && !resolutions.size ? "disabled" : disabled())}</section>`;
  }

  function context(upcoming: QueueEntry[], next: QueueEntry | undefined, remaining: number) {
    const item = player.current();
    return `<aside class="audio-context" aria-label="${t("Reproducción actual", "Current playback")}"><h2>${t("A continuación", "Up next")}</h2>
      ${upcoming.length ? `<ol class="audio-next">${upcoming.map(entry => {
        const title = entry.item ? audioTitle(entry.item) : t("Archivo ausente", "Missing file");
        const isNext = entry === next;
        const remove = t("Quitar de la cola", "Remove from queue");
        return `<li class="${isNext ? "is-next" : ""}"><div class="audio-next-copy">
          ${isNext ? `<small>${t("Siguiente", "Next")}</small>` : ""}<strong title="${h(title)}">${h(title)}</strong></div>
          ${action("audio-remove-queue", remove, "close", "icon-button",
            `data-index="${entry.position}" aria-label="${h(`${remove} · ${title}`)}"`)}</li>`;
      }).join("")}</ol>
        ${remaining ? `<small>+${remaining}</small>` : ""}` : `<p class="field-note">${t("Las pistas siguientes aparecerán aquí.", "Upcoming tracks appear here.")}</p>`}
      ${item ? `<div class="audio-current"><h2>${h(audioTitle(item))}</h2>
        ${isAudiobook(item) ? `<div class="button-group">${action("audio-add-bookmark", t("Marcar posición", "Bookmark position"), "add", "secondary", disabled())}</div>
        ${category === "books" && book === bookKey(item) ? "" : detailsFor(item).chapters.map(chapter => action("audio-seek-chapter", `${durationLabel(chapter.start)} · ${chapter.title}`, "play", "quiet", `data-time="${chapter.start}"`)).join("")}
        ${detailsFor(item).bookmarks.map(bookmark => action("audio-seek-chapter", `${durationLabel(bookmark.time)} · ${bookmark.name}`, "play", "quiet", `data-time="${bookmark.time}"`)).join("")}` : ""}
        ${item.kind === "audio" ? `<div class="button-group">${action("audio-open-lyrics", lyricOpen ? t("Cerrar letras", "Close lyrics") : t("Letras locales", "Local lyrics"), "list", "quiet", `aria-expanded="${lyricOpen}" ${disabled()}`)}</div>
          ${lyricOpen ? lyricPanel(item.id) : ""}` : ""}
        <div class="button-group">${action("audio-cover-choose", t("Elegir portada", "Choose cover"), "image", "quiet", disabled())}
        ${item.details?.audioCover ? action("audio-cover-remove", t("Quitar portada local", "Remove local cover"), undefined, "quiet", disabled()) : ""}</div></div>` : ""}</aside>`;
  }

  function lyricPanel(id: string) {
    const state = lyrics?.id === id ? lyrics : null;
    const loading = !state || state.kind === "loading";
    const cues = state?.kind === "ready" ? state.cues : [];
    const message = loading ? t("Leyendo el LRC de esta pista…", "Reading this track’s LRC…")
      : state.kind === "error" ? state.message
      : t("No hay letras locales asociadas a esta pista.", "No local lyrics are linked to this track.");
    return `<section id="audio-lyrics" class="audio-lyrics" data-track-id="${h(id)}" aria-labelledby="audio-lyrics-title" aria-busy="${loading || choosingLyrics}">
      <div class="audio-lyrics-heading"><h3 id="audio-lyrics-title">${t("Letra sincronizada", "Timed lyrics")}</h3>
      ${cues.length ? action("audio-lyrics-follow", t("Seguir letra", "Follow lyrics"), undefined, "quiet", `aria-pressed="${lyricFollow}"`) : ""}</div>
      ${cues.length ? `<ol id="audio-lyric-lines" aria-labelledby="audio-lyrics-title">${cues.map((cue, index) => `<li><button type="button" class="audio-lyric-line" data-action="audio-seek-chapter" data-id="${h(id)}" data-index="${index}" data-time="${cue.time}"><span class="audio-lyric-time">${durationLabel(cue.time)}</span><span>${h(cue.text)}</span></button></li>`).join("")}</ol>`
        : `<p class="field-note" role="status">${h(message)}</p>`}
      ${action("audio-choose-lyrics", t("Asociar archivo LRC", "Link LRC file"), "file", "quiet", `id="audio-lyric-file" ${choosingLyrics ? "disabled" : ""}`)}</section>`;
  }

  function render() {
    const { byCategory, results } = filterTracks(items());
    const queue = player.nextEntries();
    const library = hooks.snapshot().library;
    const upcoming = queue.slice(0, 16).map(({ position, id }) => ({ position, item: library.find(item => item.id === id) }));
    const next = upcoming.find(entry => isPlayable(entry.item));
    const chapterFile = category === "books" && book ? byCategory.find(item => bookKey(item) === book && !item.details?.audiobook) : undefined;
    const view = views[category];
    if (view.selected.size) {
      const available = new Set(byCategory.map(item => item.id));
      for (const id of view.selected) {
        if (available.has(id)) continue;
        view.selected.delete(id);
        if (view.tags) view.tags.preview = false;
      }
      if (!view.selected.size) view.tags = null;
    }
    view.offset = Math.min(view.offset, Math.max(0, Math.floor((results.length - 1) / PAGE_SIZE) * PAGE_SIZE));
    const { offset } = view;
    const visible = results.slice(offset, offset + PAGE_SIZE);
    return `<section class="audio-library"><div class="page-heading"><div><h1>${t("Audio", "Audio")}</h1>
      <p>${t("Música, podcasts y audiolibros de tu biblioteca.", "Music, podcasts and audiobooks from your library.")}</p></div>
      ${action("audio-import-media", t("Importar audio", "Import audio"), "add", "secondary", disabled())}</div>
      <nav class="audio-category-nav" aria-label="${t("Tipo de audio", "Audio type")}">${categories.map(value => `<button type="button" data-action="audio-category" data-id="${value}" ${value === category ? 'aria-current="page"' : ""}>${icon(categoryIcons[value])}${categoryName(value)}</button>`).join("")}</nav>
      <div class="audio-layout"><div class="audio-content">${category === "playlists" ? imported ? importView(imported) : playlistView() : `${continueBook(byCategory)}${shelf(results, byCategory)}${filters(byCategory)}
        ${embeddedChapters(chapterFile)}
        ${selection(next?.item, visible)}${editTags()}${request ? `<p role="status" class="field-note">${t("Leyendo etiquetas locales…", "Reading local tags…")} ${action("audio-cancel-read", t("Cancelar", "Cancel"), undefined, "quiet")}</p>` : ""}
        ${visible.length ? `<ol class="audio-rows">${visible.map(row).join("")}</ol>`
          : `<div class="empty-state">${icon("audio")}<h2>${t("Sin audio en esta vista", "No audio in this view")}</h2><p>${t("Importa archivos o cambia los filtros.", "Import files or change the filters.")}</p></div>`}
        ${results.length > PAGE_SIZE ? `<div class="library-pagination"><span>${offset + 1}–${Math.min(offset + PAGE_SIZE, results.length)} / ${results.length}</span>
          ${action("audio-page-back", t("Anterior", "Previous"), undefined, "quiet", offset === 0 ? "disabled" : "")}${action("audio-page-next", t("Siguiente", "Next"), undefined, "quiet", offset + PAGE_SIZE >= results.length ? "disabled" : "")}</div>` : ""}`}</div>${context(upcoming, next, queue.length - upcoming.length)}</div></section>`;
  }

  async function saveList(id: string | null, name: string, entries: AudioPlaylistEntry[]) {
    const saved = await hooks.command<AudioPlaylist[]>("audio_playlist_save", { id, name, entries });
    activeList = id ?? saved.at(-1)?.id ?? "";
    editing = null;
    await hooks.refresh();
  }

  function setLyrics(id: string, lines: Lyric[]) {
    const cues: Lyric[] = [];
    for (const line of lines) {
      const previous = cues.at(-1);
      if (previous?.time === line.time) previous.text += "\n" + line.text;
      else cues.push({ ...line });
    }
    lyrics = { id, kind: "ready", cues, scrollTop: 0 };
  }

  async function loadLyrics(id: string) {
    const pending: Lyrics = { id, kind: "loading" };
    lyrics = pending;
    try {
      const lines = await hooks.command<Lyric[]>("audio_lyrics", { id });
      if (lyrics !== pending) return;
      setLyrics(id, lines);
    } catch (error) {
      if (lyrics !== pending) return;
      lyrics = { id, kind: "error", message: errorLabel(error) };
    }
    if (lyricPanelElement?.dataset.trackId === id) hooks.render();
  }

  async function chooseLyrics() {
    const item = player.current();
    if (item?.kind !== "audio" || choosingLyrics) return;
    choosingLyrics = true;
    hooks.render();
    try {
      const lines = await hooks.command<Lyric[] | null>("audio_lyrics_choose", { id: item.id });
      if (lines && lyricOpen && player.current()?.id === item.id) setLyrics(item.id, lines);
    } catch (error) { hooks.showError(error); }
    finally {
      choosingLyrics = false;
      if (lyricPanelElement) {
        hooks.render();
        if (lyricPanelElement?.dataset.trackId === item.id) document.getElementById("audio-lyric-file")?.focus({ preventScroll: true });
      }
    }
  }

  function playbackChanged(id: string, time: number) {
    const list = lyricList;
    if (!list || lyrics?.kind !== "ready" || lyrics.id !== id) return;
    const { cues } = lyrics;
    let low = 0, high = cues.length;
    while (low < high) {
      const middle = (low + high) >>> 1;
      if (cues[middle].time <= time) low = middle + 1;
      else high = middle;
    }
    const next = low - 1;
    if (next === lyricIndex) return;
    list.children[lyricIndex]?.firstElementChild?.removeAttribute("aria-current");
    lyricIndex = next;
    const row = list.children[next];
    row?.firstElementChild?.setAttribute("aria-current", "true");
    if (!row || !lyricFollow || list.contains(document.activeElement)) return;
    const bounds = list.getBoundingClientRect(), line = row.getBoundingClientRect();
    if (line.top < bounds.top) list.scrollTop += line.top - bounds.top;
    else if (line.bottom > bounds.bottom) list.scrollTop += Math.min(line.top - bounds.top, line.bottom - bounds.bottom);
    lyrics.scrollTop = list.scrollTop;
  }

  function bindLyrics(visible: boolean) {
    if (!visible) stopAudition();
    lyricPanelElement = visible ? document.getElementById("audio-lyrics") : null;
    lyricList = lyricPanelElement?.querySelector("ol") ?? null;
    lyricIndex = -1;
    const id = lyricPanelElement?.dataset.trackId;
    if (!id) return;
    if (lyrics?.id !== id) { void loadLyrics(id); return; }
    const list = lyricList, state = lyrics;
    if (!list || state.kind !== "ready") return;
    list.scrollTop = state.scrollTop;
    list.addEventListener("scroll", () => { state.scrollTop = list.scrollTop; });
    playbackChanged(id, player.timeFor(id) ?? 0);
  }

  function playTrack(id: string) {
    const { byCategory, results } = filterTracks(items());
    const item = results.find(item => item.id === id);
    if (!isPlayable(item)) return;
    const queue = category === "books" ? adjacentChapters(byCategory.filter(candidate => bookKey(candidate) === bookKey(item)), item) : results;
    const ids = queue.map(item => item.id);
    player.play(ids, ids.indexOf(id));
  }

  async function click(name: string, button: HTMLElement): Promise<boolean> {
    const id = button.dataset.id ?? "";
    const index = Number(button.dataset.index);
    const list = active();
    const view = views[category];
    const { selected } = view;
    switch (name) {
      case "audio-import-media": hooks.importMedia(); break;
      case "audio-category":
        if (id === "music" || id === "books" || id === "podcasts" || id === "playlists") {
          if (id !== "playlists") stopAudition();
          category = id; hooks.render();
        } break;
      case "audio-play": playTrack(id); break;
      case "audio-play-album":
      case "audio-queue-album": {
        const tracks = albumTracks(filterTracks(items()).byCategory).filter(isPlayable).map(item => item.id);
        if (!tracks.length) break;
        if (name === "audio-play-album") {
          const restart = player.current()?.id === tracks[0];
          player.play(tracks, 0);
          if (restart) player.seek(0);
        } else {
          const count = player.playNext(tracks);
          if (count) hooks.notify(t("Álbum colocado a continuación.", "Album queued next."));
        }
      } break;
      case "audio-play-chapter": {
        const item = itemFor(id);
        if (!isPlayable(item) || !isAudiobook(item) || book !== bookKey(item)) break;
        const details = detailsFor(item);
        const chapter = details.chapters[index];
        if (!chapter) break;
        player.playFrom(id, chapterResume(details, chapter));
        document.getElementById(`audio-chapter-${index}`)?.focus({ preventScroll: true });
      } break;
      case "audio-chapters-back":
      case "audio-chapters-next": {
        chapterOffset = Math.max(0, chapterOffset + (name === "audio-chapters-next" ? CHAPTER_PAGE_SIZE : -CHAPTER_PAGE_SIZE));
        hooks.render();
        (document.querySelector<HTMLElement>(`[data-action="${name}"]:not(:disabled)`) ?? document.getElementById("audio-book-chapters-title"))?.focus();
      } break;
      case "audio-continue-book": {
        const item = itemFor(id);
        if (category !== "books" || !item || !isAudiobook(item)) break;
        book = bookKey(item); view.query = ""; view.offset = 0; chapterOffset = 0;
        playTrack(id);
        document.querySelector<HTMLElement>(".audio-continue [data-action='audio-continue-book']")?.focus({ preventScroll: true });
      } break;
      case "audio-shelf-open":
        if (category === "music") album = id;
        else if (category === "books") { book = id; chapterOffset = 0; }
        view.offset = 0; hooks.render();
        document.querySelector<HTMLElement>(".audio-shelf-trail button")?.focus({ preventScroll: true });
        break;
      case "audio-shelf-clear": {
        const key = category === "books" ? book : album;
        if (category === "music") album = "";
        else if (category === "books") book = "";
        view.offset = 0; hooks.render();
        document.querySelector<HTMLElement>(`.audio-shelf-item[data-id="${CSS.escape(key)}"]`)?.focus({ preventScroll: true });
      } break;
      case "audio-shelf-back":
      case "audio-shelf-next": {
        view.shelfOffset = Math.max(0, view.shelfOffset + (name === "audio-shelf-next" ? SHELF_SIZE : -SHELF_SIZE));
        hooks.render();
        const direction = document.querySelector<HTMLButtonElement>(`[data-action="${name}"]:not(:disabled)`);
        (direction ?? document.querySelector<HTMLElement>(".audio-shelf-item"))?.focus({ preventScroll: true });
      } break;
      case "audio-inspect": hooks.inspect(id); break;
      case "audio-add-queue": player.add(id); break;
      case "audio-queue-next": {
        const count = player.playNext(selected);
        if (count) hooks.notify(count === 1 ? t("Pista colocada a continuación.", "Track queued next.")
          : t(`${count} pistas colocadas a continuación.`, `${count} tracks queued next.`));
      } break;
      case "audio-remove-queue": player.remove(index); break;
      case "audio-page-back": view.offset = Math.max(0, view.offset - PAGE_SIZE); hooks.render(); break;
      case "audio-page-next": view.offset += PAGE_SIZE; hooks.render(); break;
      case "audio-clear": selected.clear(); view.tags = null; hooks.render(); break;
      case "audio-edit-tags": view.tags = { patch: {}, preview: false }; hooks.render(); break;
      case "audio-cancel-tags": view.tags = null; hooks.render(); break;
      case "audio-tag-copies": {
        const tags = view.tags;
        if (!tags?.preview) break;
        await run(async () => {
          await hooks.command("audio_tags_queue_copies", { ids: [...selected], patch: tags.patch, clientRequest: crypto.randomUUID() });
          view.tags = null;
          await hooks.refresh();
          hooks.notify(t("Copias añadidas a Descargas.", "Copies added to Downloads."), "success");
        });
      } break;
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
        resetMatchReview();
        if (preview.kind === "csv") {
          const labels = preview.columns.map(name => name.trim().toLocaleLowerCase().replace(/[ _-]/g, ""));
          for (const field of columns) {
            const index = labels.findIndex(value => columnAliases[field].includes(value));
            if (index >= 0) mapping[field] = index;
          }
          milliseconds = mapping.duration !== undefined && labels[mapping.duration] === "durationms";
        }
      }); break;
      case "audio-rematch-list": if (list && !editing) {
        await run(async () => {
          const preview = await hooks.command<ImportPreview>("audio_playlist_rematch", { listId: list.id });
          await hooks.refresh();
          imported = preview;
          resetMatchReview();
        });
        document.getElementById("audio-import-heading")?.focus();
      } break;
      case "audio-cancel-import": {
        const saved = imported?.kind === "list" && imported.listId !== undefined;
        imported = null; resetMatchReview(); hooks.render();
        if (saved) document.querySelector<HTMLElement>('[data-action="audio-rematch-list"]')?.focus();
      } break;
      case "audio-audition": {
        const row = imported?.kind === "list" ? imported.rows[index] : undefined;
        const item = itemFor(id);
        if (!row || !isPlayable(item) || busy) break;
        if (audition?.id === id && audition.index === index && player.current()?.id === id && player.comparison()) {
          stopAudition();
          hooks.render();
        } else {
          audition = { index, id, title: audioTitle(item) };
          player.auditionTrack(id);
        }
        break;
      }
      case "audio-end-audition":
        stopAudition();
        hooks.render();
        document.getElementById("audio-import-heading")?.focus();
        break;
      case "audio-import-back": importPage = Math.max(0, importPage - 1); hooks.render(); break;
      case "audio-import-next": importPage++; hooks.render(); break;
      case "audio-import-review": if (imported?.kind === "list") {
        const preview = imported;
        if ([...resolutions.values()].some(id => {
          const item = itemFor(id);
          return !isPlayable(item);
        })) {
          hooks.notify(t("Una grabación elegida ya no está disponible.", "A chosen recording is no longer available."), "error");
          break;
        }
        const entries = preview.rows.map<AudioPlaylistEntry>((row, index) => {
          const chosen = resolutions.get(index);
          if (!chosen) return row.entry;
          const { title, hints } = row.entry;
          return { kind: "local", id: chosen, title, ...(hints ? { hints } : {}) };
        });
        editing = { id: preview.listId ?? null, name: preview.name, entries };
        imported = null; resetMatchReview();
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
      case "audio-list-play-all":
      case "audio-list-queue-next": if (list && !editing) {
        const ids = list.entries.flatMap(entry => entry.kind === "local" && isLocalAvailable(entry) ? [entry.id] : []);
        if (!ids.length) break;
        if (name === "audio-list-play-all") {
          const restart = player.current()?.id === ids[0];
          lyricOpen = false;
          player.play(ids, 0);
          if (restart) player.seek(0);
        } else if (player.playNext(ids)) {
          hooks.notify(t("Lista colocada a continuación.", "Playlist queued next."));
        }
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
        lyricOpen = !lyricOpen; lyrics = null; hooks.render(); break;
      case "audio-lyrics-follow":
        lyricFollow = !lyricFollow; hooks.render(); break;
      case "audio-choose-lyrics": await chooseLyrics(); break;
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
    const view = views[category];
    const { selected, tags } = view;
    if (element instanceof HTMLInputElement) {
      if (element.dataset.matchQuery !== undefined) {
        matchQueries.set(Number(element.dataset.matchQuery), element.value);
        hooks.render();
        return true;
      }
      if (element.dataset.audioMatch !== undefined) {
        if (element.checked) {
          const index = Number(element.dataset.audioMatch);
          if (element.value) resolutions.set(index, element.value); else resolutions.delete(index);
          hooks.render();
        }
        return true;
      }
      if (element.id === "audio-csv-ms") { milliseconds = element.checked; return true; }
      if (element.id === "audio-query") { view.query = element.value; view.offset = 0; view.shelfOffset = 0; hooks.render(); return true; }
      if (element.id === "audio-playlist-name" && editing) { editing.name = element.value; return true; }
      if (element.dataset.action === "audio-select" && element.dataset.id) {
        if (!element.checked) selected.delete(element.dataset.id);
        else if (selected.size < 200) selected.add(element.dataset.id);
        if (!selected.size) view.tags = null;
        else if (tags) tags.preview = false;
        hooks.render(); return true;
      }
      const field = tagFields.find(field => element.dataset.field === field || element.id === `audio-tag-${field}`);
      if (field && tags) {
        if (element.dataset.action === "audio-tag-enable") {
          if (element.checked) tags.patch[field] = ""; else delete tags.patch[field];
          tags.preview = false; hooks.render();
        } else {
          tags.patch[field] = element.value;
          if (tags.preview) { tags.preview = false; hooks.render(); }
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
    switch (element.id) {
      case "audio-artist": artist = element.value; break;
      case "audio-album": album = element.value; break;
      case "audio-genre": genre = element.value; break;
      case "audio-book": book = element.value; chapterOffset = 0; break;
      case "audio-active-list": activeList = element.value; editing = null; break;
      case "audio-add-list": addTo = element.value; break;
      default: return false;
    }
    view.offset = 0;
    view.shelfOffset = 0;
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
    const view = views[category];
    const { tags, selected } = view;
    if (!tags || !form.reportValidity()) return true;
    if (!tags.preview) { tags.preview = true; hooks.render(); return true; }
    await run(async () => {
      await hooks.command("audio_tags_edit", { ids: [...selected], patch: tags.patch });
      view.tags = null;
      await hooks.refresh();
    });
    return true;
  }

  function openArchivePlaylist(preview: ImportPreview) {
    category = "playlists";
    imported = preview;
    editing = null;
    resetMatchReview();
    hooks.render();
  }

  function selectBook(id: string): boolean {
    if (!id || !items().some(item => !item.missing && isAudiobook(item) && bookKey(item) === id)) return false;
    category = "books";
    book = id;
    views.books.query = "";
    views.books.offset = 0;
    chapterOffset = 0;
    return true;
  }

  return { render, click, input, submit, openArchivePlaylist, selectBook, bindLyrics, playbackChanged };
}
