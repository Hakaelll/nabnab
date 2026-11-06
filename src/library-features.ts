import { escapeHtml as h, type Copy, type LibraryItem, type Snapshot } from "./contracts";
import { durationLabel } from "./presentation";
import { action, option } from "./views-common";
import { emptyLibraryDetails, type Bookmark, type LibraryDetails, type SmartCollection, type SmartCondition, type LocalTrack } from "./library-types";

export function detailsFor(item: LibraryItem): LibraryDetails { return item.details ?? emptyLibraryDetails; }
interface ContentTiming {
  start: number; end: number | null; requestedStart: number; requestedEnd: number | null;
  clipped: boolean; approximate: boolean;
}
function contentTiming(item: LibraryItem, snapshot: Snapshot, visited = new Set<string>()): ContentTiming | null {
  if (visited.has(item.id) || visited.size >= 32) return null;
  visited.add(item.id);
  const parent = item.parentId ? snapshot.library.find(parent => parent.id === item.parentId) : undefined;
  if (item.parentId && !parent) return null;
  const base: ContentTiming | null = parent ? contentTiming(parent, snapshot, visited) : { start: 0, end: null, requestedStart: 0, requestedEnd: null, clipped: false, approximate: false };
  if (!base) return null;
  const clip = detailsFor(item).clip;
  if (clip) {
    if (![clip.requestedStart, clip.requestedEnd].every(Number.isFinite) || clip.requestedStart < 0 || clip.requestedEnd <= clip.requestedStart) return null;
    const attestedStart = clip.actualStart !== null && Number.isFinite(clip.actualStart) && clip.actualStart >= 0;
    const attestedEnd = clip.actualEnd !== null && Number.isFinite(clip.actualEnd) && clip.actualEnd > (clip.actualStart ?? clip.requestedStart);
    const start = base.start + (attestedStart ? clip.actualStart ?? clip.requestedStart : clip.requestedStart);
    const end = attestedEnd ? base.start + (clip.actualEnd ?? clip.requestedEnd) : start + (attestedStart && item.duration !== null ? item.duration : clip.requestedEnd - clip.requestedStart);
    return { start, end, requestedStart: base.requestedStart + clip.requestedStart, requestedEnd: base.requestedStart + clip.requestedEnd, clipped: true, approximate: base.approximate || !attestedStart };
  }
  return { ...base, end: item.duration !== null ? base.start + item.duration : base.end };
}
export function subtitlePlaybackTarget(item: LibraryItem, time: number, snapshot: Snapshot): { item: LibraryItem; time: number; sourceTime: number; approximate: boolean } | null {
  if (!Number.isFinite(time) || time < 0) return null;
  const contentId = detailsFor(item).contentId;
  const timing = contentTiming(item, snapshot);
  if (!contentId || !timing) return null;
  const sourceTime = timing.start + time;
  if (timing.end !== null && sourceTime >= timing.end) return null;
  const candidates = snapshot.library.flatMap(candidate => {
    if (candidate.kind !== "video" || candidate.missing || detailsFor(candidate).contentId !== contentId) return [];
    const candidateTiming = contentTiming(candidate, snapshot);
    if (!candidateTiming || sourceTime < candidateTiming.start || (candidateTiming.end !== null && sourceTime >= candidateTiming.end)) return [];
    const sameInterval = candidateTiming.clipped === timing.clipped && Math.abs(candidateTiming.start - timing.start) < .001 && (candidateTiming.end === timing.end || (candidateTiming.end !== null && timing.end !== null && Math.abs(candidateTiming.end - timing.end) < .001));
    const matching = candidateTiming.clipped === timing.clipped && Math.abs(candidateTiming.requestedStart - timing.requestedStart) < .001 && (candidateTiming.requestedEnd === timing.requestedEnd || (candidateTiming.requestedEnd !== null && timing.requestedEnd !== null && Math.abs(candidateTiming.requestedEnd - timing.requestedEnd) < .001));
    return [{ item: candidate, time: sourceTime - candidateTiming.start, sourceTime, approximate: timing.approximate || candidateTiming.approximate, estimated: candidateTiming.approximate, rank: sameInterval ? 0 : matching ? 1 : candidateTiming.clipped ? 3 : 2 }];
  });
  candidates.sort((a,b) => a.rank - b.rank || Number(a.estimated) - Number(b.estimated));
  return candidates[0] ?? null;
}
export function matchesSmartCollection(item: LibraryItem, collection: SmartCollection): boolean {
  const details = detailsFor(item);
  const compare = (condition: SmartCondition): boolean => {
    switch (condition.kind) {
      case "kind": return item.kind === condition.value;
      case "source": return details.sourceIds.includes(condition.value) || item.source?.toLowerCase() === condition.value.toLowerCase();
      case "platform": return details.platform?.toLowerCase() === condition.value.toLowerCase();
      case "tag": return item.tags.some(tag => tag.toLowerCase() === condition.value.toLowerCase());
      case "downloadedAfter": return item.addedAt >= condition.value;
      case "downloadedBefore": return item.addedAt < condition.value;
      case "minDuration": return item.duration !== null && item.duration >= condition.value;
      case "maxDuration": return item.duration !== null && item.duration <= condition.value;
      case "minHeight": return item.height !== null && item.height >= condition.value;
      case "maxHeight": return item.height !== null && item.height <= condition.value;
      case "subtitleLanguage": return details.tracks.some(track => track.language.toLowerCase() === condition.value.toLowerCase());
      case "hasSubtitles": return (details.tracks.length > 0) === condition.value;
      case "favorite": return details.favorite === condition.value;
    }
  };
  return collection.conditions.length > 0 && (collection.mode === "all" ? collection.conditions.every(compare) : collection.conditions.some(compare));
}

export function collectionIncludes(item: LibraryItem, selection: string, snapshot: Snapshot): boolean {
  if (!selection) return true;
  if (selection === "__favorites") return detailsFor(item).favorite;
  if (selection === "__continue") return detailsFor(item).playbackPosition > 3 && (item.duration === null || detailsFor(item).playbackPosition < item.duration - 3);
  if (selection.startsWith("smart:")) {
    const smart = snapshot.smartCollections?.find(collection => `smart:${collection.id}` === selection);
    return smart !== undefined && matchesSmartCollection(item, smart);
  }
  return item.collections.includes(selection);
}
export function metadataText(item: LibraryItem): string {
  const details = detailsFor(item);
  return `${item.name} ${details.creator ?? ""} ${item.tags.join(" ")} ${item.collections.join(" ")} ${details.bookmarks.map(b => b.name).join(" ")}`;
}
export function hasLibraryMatch(item: LibraryItem, query: string): boolean {
  const needle = query.trim().toLowerCase();
  return !needle || metadataText(item).toLowerCase().includes(needle) || detailsFor(item).tracks.some(track => track.segments.some(segment => segment.text.toLowerCase().includes(needle)));
}
export function subtitleSearchResults(items: LibraryItem[], query: string, t: Copy): string {
  const needle = query.trim().toLowerCase();
  if (!needle) return "";
  let count = 0;
  const results: string[] = [];
  for (const item of items) for (const track of detailsFor(item).tracks) for (const segment of track.segments) {
    if (count >= 50) break;
    if (!segment.text.toLowerCase().includes(needle)) continue;
    count += 1;
    results.push(`<button type="button" class="subtitle-search-hit media-open" data-action="library-seek-result" data-id="${h(item.id)}" data-time="${segment.start}"><span><strong>${h(item.name)}</strong><small>${t("Subtítulos", "Subtitles")} · ${h(track.language)} · ${durationLabel(segment.start)}</small><span>${h(segment.text.slice(0, 220))}</span></span></button>`);
  }
  return results.length ? `<section class="subtitle-search-results" aria-label="${t("Coincidencias en subtítulos", "Subtitle matches")}"><h2>${t("En los subtítulos", "In subtitles")}</h2>${results.join("")}${count === 50 ? `<p class="field-note">${t("Se muestran las primeras 50 coincidencias. Acota la búsqueda.", "Showing the first 50 matches. Refine your search.")}</p>` : ""}</section>` : "";
}

export function inspectorTabs(t: Copy): string {
  return `<div class="filter-tabs inspector-tabs" role="group" aria-label="${t("Contenido del inspector", "Inspector content")}">${[["details", t("Detalles", "Details")], ["chapters", t("Capítulos", "Chapters")], ["subtitles", t("Subtítulos", "Subtitles")], ["versions", t("Versiones", "Versions")]].map(([id, label]) => `<button type="button" data-action="library-inspector-tab" data-tab="${id}" aria-pressed="${id === "details"}">${label}</button>`).join("")}</div>`;
}
export function advancedInspector(item: LibraryItem, snapshot: Snapshot, t: Copy): string {
  const details = detailsFor(item);
  const variants = snapshot.library.filter(other => other.id !== item.id && ((details.contentId && detailsFor(other).contentId === details.contentId) || other.parentId === item.id || item.parentId === other.id || (item.parentId && other.parentId === item.parentId)));
  return `<section data-inspector-panel="chapters" hidden>${details.chapters.length ? `<ul class="chapter-list">${details.chapters.map((chapter, index) => `<li><label><input type="checkbox" data-chapter-index="${index}" data-id="${h(item.id)}">${h(chapter.title)}</label><button type="button" class="button quiet" data-action="library-seek" data-id="${h(item.id)}" data-time="${chapter.start}">${durationLabel(chapter.start)} – ${durationLabel(chapter.end)}</button></li>`).join("")}</ul>${action("library-chapters-to-tools", t("Crear los capítulos seleccionados", "Create selected chapters"), "scissors", "secondary", `data-id="${h(item.id)}"`)}<p class="field-note">${t("Se crean recortes independientes con FFmpeg en la cola. El original se conserva.", "Independent FFmpeg trims run in the queue. The original is kept.")}</p>` : `<p class="field-note">${t("Este archivo no contiene capítulos conocidos.", "This file has no known chapters.")}</p>`}</section>
    <section data-inspector-panel="subtitles" hidden><label class="compact-label">${t("Idioma al importar", "Import language")}<input id="subtitle-import-language" value="es" maxlength="24" aria-label="${t("Código de idioma", "Language code")}"></label>${action("library-import-subtitles", t("Importar SRT o VTT", "Import SRT or VTT"), "add", "secondary", `data-id="${h(item.id)}"`)}<p class="field-note">${t("Se conservan los tiempos. La vista de texto y las exportaciones pueden perder estilos y posición.", "Timing is kept. The text view and exports may lose styling and positioning.")}</p>${details.tracks.length ? `<label class="compact-label">${t("Pista local", "Local track")}<select id="local-subtitle-track" data-id="${h(item.id)}">${details.tracks.map(track => option(track.id, `${track.language} · ${originLabel(track.origin, t)}`, details.tracks[0]?.id ?? "")).join("")}</select></label><label class="search-control"><input id="subtitle-track-search" type="search" placeholder="${t("Buscar en esta pista", "Search this track")}" aria-label="${t("Buscar en esta pista", "Search this track")}"></label><div class="button-group">${["txt", "srt", "vtt"].map(format => action("library-export-subtitles", t("Exportar ", "Export ") + format.toUpperCase(), undefined, "quiet", `data-id="${h(item.id)}" data-format="${format}"`)).join("")}${action("library-rebuild-subtitles", t("Reconstruir índice", "Rebuild index"), "retry", "quiet", `data-id="${h(item.id)}"`)}</div><div class="subtitle-reader" data-subtitle-reader="${h(item.id)}"></div>` : `<p class="field-note">${t("No hay pistas locales. Importa un SRT o VTT, o descarga una pista disponible desde el enlace original.", "There are no local tracks. Import SRT or VTT, or download an available track from the original link.")}</p>`}</section>
    <section data-inspector-panel="versions" hidden>${details.clip ? `<p class="field-note">${t("Fragmento solicitado", "Requested fragment")} ${durationLabel(details.clip.requestedStart)} – ${durationLabel(details.clip.requestedEnd)}. ${details.clip.actualStart === null ? t("El motor no informa del intervalo real exacto.", "The engine does not report the exact actual interval.") : `${t("Intervalo real", "Actual interval")} ${durationLabel(details.clip.actualStart)} – ${durationLabel(details.clip.actualEnd)}`}</p>` : ""}${variants.length ? variants.map(variant => `<button type="button" class="media-open" data-action="inspect-media" data-id="${h(variant.id)}"><span><strong>${h(variant.name)}</strong><small>${h(variant.kind)} · ${h(variant.path)}</small></span></button>`).join("") : `<p class="field-note">${t("No hay otras versiones conocidas de este contenido.", "There are no other known versions of this content.")}</p>`}</section>`;
}
export function bookmarksPanel(item: LibraryItem, t: Copy): string {
  const details = detailsFor(item);
  return `<section class="bookmark-panel"><div class="button-group">${action("library-favorite", details.favorite ? t("Quitar favorito", "Unfavorite") : t("Favorito", "Favorite"), undefined, "quiet", `data-id="${h(item.id)}" aria-pressed="${details.favorite}"`)}${details.playbackPosition > 3 ? action("library-seek", t("Continuar en ", "Continue at ") + durationLabel(details.playbackPosition), "play", "quiet", `data-id="${h(item.id)}" data-time="${details.playbackPosition}"`) : ""}</div><form id="bookmark-form" data-id="${h(item.id)}" class="organize-form"><label>${t("Nombre del marcador", "Bookmark name")}<input id="bookmark-name" required maxlength="160" placeholder="${t("Marca el momento actual", "Mark the current moment")}"></label><button class="button secondary" type="submit">${t("Añadir marcador", "Add bookmark")}</button></form>${details.bookmarks.map(bookmark => `<div class="button-group"><button type="button" class="button quiet" data-action="library-seek" data-id="${h(item.id)}" data-time="${bookmark.time}">${h(bookmark.name)} · ${durationLabel(bookmark.time)}</button>${action("library-remove-bookmark", t("Quitar marcador", "Remove bookmark"), "close", "icon-button", `data-id="${h(item.id)}" data-bookmark="${h(bookmark.id)}"`)}</div>`).join("")}</section>`;
}
export function playbackFallback(t: Copy): string {
  return `<div class="field-note" data-seek-fallback hidden><p data-seek-reference></p><button type="button" class="button quiet" data-action="library-copy-time" data-time="0">${t("Copiar tiempo", "Copy time")}</button></div>`;
}
function originLabel(origin: LocalTrack["origin"], t: Copy): string {
  return origin === "creator" ? t("Del creador", "Creator") : origin === "automatic" ? t("Automáticos", "Automatic") : t("Importados", "Imported");
}

interface LibraryHooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  snapshot(): Snapshot; refresh(): Promise<void>; render(): void; t: Copy;
  notify(message: string): void; showError(error: unknown): void;
  inspect(id: string, time?: number): void;
}
const conditionLabels = (t: Copy): [SmartCondition["kind"], string][] => [["kind", t("Tipo", "Type")], ["source", t("Fuente", "Source")], ["platform", t("Plataforma", "Platform")], ["tag", t("Etiqueta", "Tag")], ["downloadedAfter", t("Descargado desde", "Downloaded after")], ["downloadedBefore", t("Descargado antes de", "Downloaded before")], ["minDuration", t("Duración mínima en segundos", "Minimum duration in seconds")], ["maxDuration", t("Duración máxima en segundos", "Maximum duration in seconds")], ["minHeight", t("Resolución mínima", "Minimum height")], ["maxHeight", t("Resolución máxima", "Maximum height")], ["subtitleLanguage", t("Idioma de subtítulos", "Subtitle language")], ["hasSubtitles", t("Tiene subtítulos", "Has subtitles")], ["favorite", t("Favorito", "Favorite")]];
export function createLibraryFeatures(hooks: LibraryHooks) {
  let inspectorTab = "details";
  let selectedTrack = "";
  let cueQuery = "";
  let cuePage = 0;
  let followPlayback = true;
  let lastReaderCheckAt = 0;
  let lastSavedAt = 0;
  let editSmart: SmartCollection | null = null;
  let seekTarget: { id: string; time: number } | null = null;
  let exporting = false;
  const t = hooks.t;
  function value(id: string): string { const input = document.getElementById(id); return input instanceof HTMLInputElement || input instanceof HTMLSelectElement ? input.value : ""; }
  function itemFor(id: string): LibraryItem | undefined { return hooks.snapshot().library.find(item => item.id === id); }
  function playerFor(id: string): HTMLMediaElement | null { return [...document.querySelectorAll<HTMLMediaElement>("video[data-local-player],audio[data-local-player]")].find(player => player.dataset.localPlayer === id) ?? null; }
  function seekFallback(time: number, original = false, approximate = false): void {
    hooks.notify(t("Tiempo para el reproductor externo: ", "Time for external player: ") + durationLabel(time));
    const fallback = document.querySelector<HTMLElement>("[data-seek-fallback]");
    if (fallback) {
      fallback.hidden = false;
      const note = fallback.querySelector<HTMLElement>("[data-seek-reference]");
      if (note) note.textContent = `${original ? t("Tiempo en el contenido original", "Time in the original content") : t("Tiempo en este archivo", "Time in this file")}: ${durationLabel(time)}. ${approximate ? t("Referencia aproximada: no se conoce el inicio real del corte.", "Approximate reference: the actual cut start is unknown.") : ""}`;
      const copy = fallback.querySelector<HTMLElement>("[data-action='library-copy-time']");
      if (copy) { copy.dataset.time = String(time); copy.textContent = `${t("Copiar tiempo", "Copy time")} ${durationLabel(time)}`; }
    }
  }
  function seek(id: string, time: number): void {
    const player = playerFor(id);
    if (!player) {
      const item = itemFor(id);
      if (item && (item.kind === "subtitle" || item.kind === "image")) {
        const snapshot = hooks.snapshot();
        const target = subtitlePlaybackTarget(item, time, snapshot);
        if (target) {
          seek(target.item.id, target.time);
          hooks.notify(target.approximate ? t("Salto aproximado: no se conoce el inicio real del corte. Los cortes rápidos pueden empezar en un fotograma cercano.", "Approximate seek: the actual cut start is unknown. Fast cuts may start on a nearby frame.") : "");
        } else {
          const timing = contentTiming(item, snapshot);
          const reference = timing ? timing.start + time : time;
          seekTarget = null;
          if (item.kind === "subtitle") inspectorTab = "subtitles";
          hooks.inspect(id);
          seekFallback(reference, timing !== null, timing?.approximate ?? false);
          hooks.notify(t("Ningún vídeo local cubre este momento. Copia el tiempo de referencia para usarlo en un reproductor externo.", "No local video covers this moment. Copy the reference time for an external player.") + " " + durationLabel(reference));
        }
        return;
      }
      seekTarget = { id, time }; hooks.inspect(id, time); return;
    }
    const apply = () => {
      if (player.error || !Number.isFinite(player.duration)) {
        seekFallback(time);
        return;
      }
      player.currentTime = Math.min(time, player.duration);
      seekTarget = null;
      cuePage = 0;
      followPlayback = true;
      renderReader(true);
    };
    if (player.error) seekFallback(time);
    else if (player.readyState >= 1) apply();
    else { player.addEventListener("loadedmetadata", apply, { once: true }); player.addEventListener("error", () => seekFallback(time), { once: true }); }
  }
  function renderReader(followCurrent = false): void {
    const reader = document.querySelector<HTMLElement>("[data-subtitle-reader]"); if (!reader) return;
    const item = itemFor(reader.dataset.subtitleReader ?? ""); if (!item) return;
    const tracks = detailsFor(item).tracks;
    const track = tracks.find(track => track.id === selectedTrack) ?? tracks[0]; if (!track) return;
    selectedTrack = track.id;
    const current = playerFor(item.id)?.currentTime ?? detailsFor(item).playbackPosition;
    const visible = track.segments.map((segment, index) => ({ segment, index })).filter(({ segment }) => !cueQuery || segment.text.toLowerCase().includes(cueQuery.toLowerCase()));
    if (followCurrent && !cueQuery) { const index = visible.findIndex(({ segment }) => current >= segment.start && current < segment.end); if (index >= 0) cuePage = Math.floor(index / 200); }
    cuePage = Math.min(cuePage, Math.max(0, Math.ceil(visible.length / 200) - 1));
    reader.dataset.cuePage = String(cuePage);
    reader.innerHTML = `${track.formattingLost ? `<p class="field-note">${t("La vista de texto simplifica el formato original.", "The text view simplifies the original formatting.")}</p>` : ""}${visible.slice(cuePage * 200, cuePage * 200 + 200).map(({ segment, index }) => {
      return `<div class="subtitle-cue ${current >= segment.start && current < segment.end ? "is-current" : ""}" data-cue-start="${segment.start}" data-cue-end="${segment.end}"><button type="button" class="button quiet" data-action="library-seek" data-id="${h(item.id)}" data-time="${segment.start}"><time>${durationLabel(segment.start)}</time><span>${h(segment.text)}</span></button>${action("library-copy-cue", t("Copiar con tiempo", "Copy with timestamp"), "copy", "icon-button", `data-id="${h(item.id)}" data-track="${h(track.id)}" data-cue-index="${index}"`)}</div>`;
    }).join("")}${visible.length > 200 ? `<div class="button-group">${action("library-cues-previous", t("Anterior", "Previous"), undefined, "quiet", cuePage === 0 ? "disabled" : "")}<span>${cuePage * 200 + 1}–${Math.min(visible.length, cuePage * 200 + 200)} / ${visible.length}</span>${action("library-cues-next", t("Siguiente", "Next"), undefined, "quiet", (cuePage + 1) * 200 >= visible.length ? "disabled" : "")}</div>` : !visible.length ? `<p class="field-note">${t("No hay coincidencias en esta pista.", "No matches in this track.")}</p>` : ""}`;
  }
  function bind(): void {
    document.querySelectorAll<HTMLElement>("[data-inspector-panel]").forEach(panel => { panel.hidden = panel.dataset.inspectorPanel !== inspectorTab; });
    document.querySelectorAll<HTMLElement>("[data-action='library-inspector-tab']").forEach(button => button.setAttribute("aria-pressed", String(button.dataset.tab === inspectorTab)));
    const select = document.getElementById("local-subtitle-track");
    if (select instanceof HTMLSelectElement && [...select.options].some(option => option.value === selectedTrack)) select.value = selectedTrack;
    const search = document.getElementById("subtitle-track-search"); if (search instanceof HTMLInputElement) search.value = cueQuery;
    renderReader(true);
    for (const player of document.querySelectorAll<HTMLMediaElement>("video[data-local-player],audio[data-local-player]")) {
      if (player.dataset.libraryBound) continue;
      player.dataset.libraryBound = "true";
      const id = player.dataset.localPlayer ?? "";
      const item = itemFor(id);
      const position = seekTarget?.id === id ? seekTarget.time : item ? detailsFor(item).playbackPosition : 0;
      if (position > 0) {
        player.addEventListener("loadedmetadata", () => { if (Number.isFinite(player.duration)) { player.currentTime = Math.min(position, player.duration); seekTarget = null; renderReader(true); } }, { once: true });
        player.addEventListener("error", () => seekFallback(position), { once: true });
      }
      const save = () => {
        if (!Number.isFinite(player.currentTime)) return;
        const item = itemFor(id); if (!item) return;
        const details = { ...detailsFor(item), playbackPosition: player.currentTime }; item.details = details;
        void hooks.command("library_playback", { id, position: player.currentTime }).catch(hooks.showError);
        lastSavedAt = Date.now();
      };
      player.addEventListener("timeupdate", () => {
        for (const cue of document.querySelectorAll<HTMLElement>("[data-cue-start]")) cue.classList.toggle("is-current", player.currentTime >= Number(cue.dataset.cueStart) && player.currentTime < Number(cue.dataset.cueEnd));
        if (followPlayback && !cueQuery && inspectorTab === "subtitles" && Date.now() - lastReaderCheckAt > 1000) {
          lastReaderCheckAt = Date.now();
          const currentItem = itemFor(id);
          const track = currentItem ? detailsFor(currentItem).tracks.find(track => track.id === selectedTrack) : undefined;
          const index = track?.segments.findIndex(segment => player.currentTime >= segment.start && player.currentTime < segment.end) ?? -1;
          if (index >= 0 && Math.floor(index / 200) !== cuePage) renderReader(true);
        }
        if (!player.paused && Date.now() - lastSavedAt > 15_000) save();
      });
      player.addEventListener("pause", save); player.addEventListener("ended", save);
    }
  }
  async function changed(): Promise<void> {
    for (const player of document.querySelectorAll<HTMLMediaElement>("video[data-local-player],audio[data-local-player]")) {
      const id = player.dataset.localPlayer;
      if (id && itemFor(id) && Number.isFinite(player.currentTime)) await hooks.command("library_playback", { id, position: player.currentTime });
    }
    await hooks.refresh(); hooks.render();
  }
  async function handleClick(target: HTMLElement): Promise<boolean> {
    const actionName = target.dataset.action ?? ""; const id = target.dataset.id ?? "";
    if (!actionName.startsWith("library-") && !actionName.startsWith("smart-")) return false;
    if (!new Set(["library-inspector-tab", "library-seek", "library-seek-result", "library-favorite", "library-remove-bookmark", "library-import-subtitles", "library-export-subtitles", "library-rebuild-subtitles", "library-copy-cue", "library-copy-time", "library-cues-previous", "library-cues-next", "library-chapters-to-tools", "smart-new", "smart-edit", "smart-remove", "smart-cancel", "smart-add-condition", "smart-remove-condition"]).has(actionName)) return false;
    try {
      const item = itemFor(id);
      if (actionName === "library-inspector-tab") { inspectorTab = target.dataset.tab ?? "details"; bind(); }
      else if (actionName === "library-seek" || actionName === "library-seek-result") { const time = Number(target.dataset.time); if (Number.isFinite(time) && time >= 0) seek(id, time); }
      else if (actionName === "library-copy-time") { const time = Number(target.dataset.time); if (Number.isFinite(time)) await navigator.clipboard.writeText(durationLabel(time)); }
      else if (actionName === "library-cues-previous" || actionName === "library-cues-next") { followPlayback = false; cuePage = Math.max(0, cuePage + (actionName === "library-cues-next" ? 1 : -1)); renderReader(); }
      else if (actionName === "library-favorite" && item) { await hooks.command("library_favorite", { id, favorite: !detailsFor(item).favorite }); await changed(); }
      else if (actionName === "library-remove-bookmark" && item) { const bookmark = detailsFor(item).bookmarks.find(bookmark => bookmark.id === target.dataset.bookmark); if (bookmark) { await hooks.command("library_bookmark", { id, bookmark, remove: true }); await changed(); } }
      else if (actionName === "library-import-subtitles") { await hooks.command("subtitle_import", { id, language: value("subtitle-import-language"), path: null }); await changed(); }
      else if (actionName === "library-export-subtitles" && !exporting) { exporting = true; try { await hooks.command("subtitle_export", { id, trackId: selectedTrack, format: target.dataset.format }); hooks.notify(t("Subtítulos exportados.", "Subtitles exported.")); } finally { exporting = false; } }
      else if (actionName === "library-rebuild-subtitles") { await hooks.command("subtitle_rebuild", { ids: [id] }); await changed(); }
      else if (actionName === "library-copy-cue" && item) { const cue = detailsFor(item).tracks.find(track => track.id === target.dataset.track)?.segments[Number(target.dataset.cueIndex)]; if (cue) { await navigator.clipboard.writeText(`${durationLabel(cue.start)} ${cue.text}`); hooks.notify(t("Fragmento copiado con su tiempo.", "Excerpt copied with timestamp.")); } }
      else if (actionName === "library-chapters-to-tools" && item) {
        const selected = [...document.querySelectorAll<HTMLInputElement>("[data-chapter-index]:checked")].filter(input => input.dataset.id === id).map(input => detailsFor(item).chapters[Number(input.dataset.chapterIndex)]).filter(chapter => chapter !== undefined);
        if (!selected.length) { hooks.notify(t("Selecciona uno o varios capítulos.", "Select one or more chapters.")); return true; }
        const request = `chapters-${Date.now()}`;
        for (const [index, chapter] of selected.entries()) await hooks.command("queue_tool", { ids: [id], options: { kind: "trim", start: chapter.start, end: chapter.end, precise: false }, clientRequest: `${request}-${index}` });
        await changed(); hooks.notify(t("Capítulos añadidos a la cola.", "Chapters added to queue."));
      }
      else if (actionName === "smart-new") { editSmart = { id: `smart-${Date.now()}`, name: "", mode: "all", conditions: [{ kind: "kind", value: "video" }] }; hooks.render(); }
      else if (actionName === "smart-edit") { const collection = hooks.snapshot().smartCollections?.find(c => c.id === id); if (collection) { editSmart = structuredClone(collection); hooks.render(); } }
      else if (actionName === "smart-remove") { await hooks.command("smart_collection_remove", { id }); await changed(); }
      else if (actionName === "smart-cancel") { editSmart = null; hooks.render(); }
      else if (actionName === "smart-add-condition" && editSmart) { readSmartFields(); if (editSmart.conditions.length < 24) editSmart.conditions.push({ kind: "tag", value: "" }); hooks.render(); }
      else if (actionName === "smart-remove-condition" && editSmart) { readSmartFields(); editSmart.conditions.splice(Number(target.dataset.index), 1); hooks.render(); }
    } catch (error) { hooks.showError(error); }
    return true;
  }
  function conditionValue(kind: string, value: string): SmartCondition | null {
    switch (kind) {
      case "kind": case "source": case "platform": case "tag": case "subtitleLanguage": return { kind, value };
      case "minDuration": case "maxDuration": case "minHeight": case "maxHeight": return { kind, value: Number(value) };
      case "downloadedAfter": case "downloadedBefore": return { kind, value: new Date(value).getTime() };
      case "hasSubtitles": case "favorite": return { kind, value: value === "true" };
      default: return null;
    }
  }
  function readSmartFields(): void {
    if (!editSmart) return;
    editSmart.name = value("smart-name"); editSmart.mode = value("smart-mode") === "any" ? "any" : "all";
    const conditions: SmartCondition[] = [];
    for (const row of document.querySelectorAll<HTMLElement>("[data-smart-condition]")) {
      const kind = row.querySelector<HTMLSelectElement>("[data-condition-kind]")?.value ?? "";
      const input = row.querySelector<HTMLInputElement | HTMLSelectElement>("[data-condition-value]");
      const condition = conditionValue(kind, input?.value ?? ""); if (condition) conditions.push(condition);
    }
    editSmart.conditions = conditions;
  }
  function handleInput(target: HTMLElement): boolean {
    if (target.id === "subtitle-track-search") { cueQuery = value(target.id); cuePage = 0; renderReader(); return true; }
    return false;
  }
  function handleChange(target: HTMLElement): boolean {
    if (target.id === "local-subtitle-track") { selectedTrack = value(target.id); cuePage = 0; followPlayback = true; renderReader(true); return true; }
    if ("conditionKind" in target.dataset && editSmart && target instanceof HTMLSelectElement) {
      readSmartFields();
      const kind = target.value;
      const date = kind === "downloadedAfter" || kind === "downloadedBefore";
      const numeric = ["minDuration", "maxDuration", "minHeight", "maxHeight"].includes(kind);
      const initial = kind === "kind" ? "video" : kind === "platform" ? "YouTube" : kind === "subtitleLanguage" ? "es" : kind === "hasSubtitles" || kind === "favorite" ? "true" : date ? new Date().toISOString().slice(0, 10) : numeric ? "0" : kind === "source" ? hooks.snapshot().sources?.[0]?.id ?? "" : "";
      const condition = conditionValue(kind, initial);
      const index = Number(target.closest<HTMLElement>("[data-smart-condition]")?.dataset.smartCondition);
      if (condition && Number.isInteger(index) && index >= 0 && index < editSmart.conditions.length) editSmart.conditions[index] = condition;
      hooks.render(); return true;
    }
    return false;
  }
  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    if (form.id !== "bookmark-form" && form.id !== "smart-collection-form") return false;
    try {
      if (form.id === "bookmark-form") {
        const id = form.dataset.id ?? ""; const item = itemFor(id);
        const position = playerFor(id)?.currentTime;
        if (!item || position === undefined || !Number.isFinite(position)) { hooks.notify(t("Abre el archivo en el reproductor interno para marcar el momento.", "Open the file in the internal player to mark the moment.")); return true; }
        const bookmark: Bookmark = { id: `bookmark-${Date.now()}`, name: value("bookmark-name"), time: position };
        await hooks.command("library_bookmark", { id, bookmark, remove: false }); await changed();
      } else if (editSmart) { readSmartFields(); await hooks.command("smart_collection_save", { collection: editSmart }); editSmart = null; await changed(); }
    } catch (error) { hooks.showError(error); }
    return true;
  }
  function collectionsView(): string {
    const snapshot = hooks.snapshot();
    const manual = snapshot.collections;
    const smart = snapshot.smartCollections ?? [];
    return `<div class="page-heading"><div><h1>${t("Colecciones", "Collections")}</h1><p>${t("Organiza referencias sin copiar los archivos.", "Organize references without copying files.")}</p></div>${action("smart-new", t("Crear colección inteligente", "Create smart collection"), "add", "primary")}</div>${editSmart ? renderSmartEditor() : ""}<div class="collection-list">${manual.map(name => `<button type="button" class="media-open" data-action="open-library-collection" data-collection="${h(name)}"><span><strong>${h(name)}</strong><small>${t("Manual", "Manual")} · ${snapshot.library.filter(item => item.collections.includes(name)).length} ${t("archivos", "files")}</small></span></button>`).join("")}${smart.map(collection => `<div class="collection-row"><button type="button" class="media-open" data-action="open-library-collection" data-collection="smart:${h(collection.id)}"><span><strong>${h(collection.name)}</strong><small>${t("Inteligente", "Smart")} · ${snapshot.library.filter(item => matchesSmartCollection(item, collection)).length} ${t("archivos", "files")}</small></span></button>${action("smart-edit", t("Editar condiciones", "Edit conditions"), undefined, "quiet", `data-id="${h(collection.id)}"`)}${action("smart-remove", t("Quitar colección", "Remove collection"), "close", "quiet", `data-id="${h(collection.id)}"`)}</div>`).join("")}</div>${!manual.length && !smart.length && !editSmart ? `<div class="empty-state"><h2>${t("Tus filtros también pueden ser una colección", "Your filters can be a collection")}</h2><p>${t("Crea una colección inteligente o asigna una colección manual desde los detalles de un archivo.", "Create a smart collection or assign a manual collection from a file's details.")}</p></div>` : ""}`;
  }
  function renderSmartEditor(): string {
    if (!editSmart) return "";
    const sourceOptions = new Map<string, string>((hooks.snapshot().sources ?? []).map(source => [source.id, source.name]));
    for (const item of hooks.snapshot().library) if (item.source) sourceOptions.set(item.source, item.source);
    return `<form id="smart-collection-form" class="organize-form smart-editor"><h2>${t("Condiciones de la colección", "Collection conditions")}</h2><label>${t("Nombre", "Name")}<input id="smart-name" value="${h(editSmart.name)}" required maxlength="160"></label><label>${t("Incluir archivos que cumplan", "Include files matching")}<select id="smart-mode">${option("all", t("Todas las condiciones", "All conditions"), editSmart.mode)}${option("any", t("Alguna condición", "Any condition"), editSmart.mode)}</select></label>${editSmart.conditions.map((condition, index) => {
      const numeric = ["minDuration", "maxDuration", "minHeight", "maxHeight"].includes(condition.kind);
      const date = condition.kind === "downloadedAfter" || condition.kind === "downloadedBefore";
      const boolean = condition.kind === "hasSubtitles" || condition.kind === "favorite";
      const dateValue = date && typeof condition.value === "number" && Number.isFinite(condition.value) ? new Date(condition.value).toISOString().slice(0, 10) : "";
      return `<div data-smart-condition="${index}" class="button-group"><select data-condition-kind aria-label="${t("Condición", "Condition")}">${conditionLabels(t).map(([kind, label]) => option(kind, label, condition.kind)).join("")}</select>${boolean ? `<select data-condition-value aria-label="${t("Valor", "Value")}">${option("true", t("Sí", "Yes"), String(condition.value))}${option("false", t("No", "No"), String(condition.value))}</select>` : condition.kind === "source" ? `<select data-condition-value required aria-label="${t("Fuente", "Source")}">${option("", t("Elige una fuente", "Choose a source"), String(condition.value))}${[...sourceOptions].map(([id, name]) => option(id, name, String(condition.value))).join("")}${typeof condition.value === "string" && condition.value && !sourceOptions.has(condition.value) ? option(condition.value, t("Fuente anterior", "Previous source"), condition.value) : ""}</select>` : condition.kind === "kind" ? `<select data-condition-value aria-label="${t("Tipo de archivo", "File type")}">${[["video", t("Vídeo", "Video")], ["audio", t("Audio", "Audio")], ["image", t("Imagen", "Image")], ["subtitle", t("Subtítulos", "Subtitles")]].map(([kind, label]) => option(kind, label, String(condition.value))).join("")}</select>` : `<input data-condition-value type="${date ? "date" : numeric ? "number" : "text"}" value="${h(date ? dateValue : String(condition.value))}" ${numeric ? 'min="0" step="any"' : ""} required aria-label="${t("Valor", "Value")}">`}${action("smart-remove-condition", t("Quitar condición", "Remove condition"), "close", "icon-button", `data-index="${index}"`)}</div>`;
    }).join("")}<div class="button-group">${action("smart-add-condition", t("Añadir condición", "Add condition"), "add", "quiet")}<button type="submit" class="button primary">${t("Guardar colección", "Save collection")}</button>${action("smart-cancel", t("Cancelar", "Cancel"), undefined, "quiet")}</div></form>`;
  }
  return { bind, handleClick, handleInput, handleChange, handleSubmit, collectionsView, seek };
}
