import { escapeHtml as h, localMediaUrl, type Copy, type LibraryItem } from "./contracts";
import { detailsFor } from "./library-features";
import type { AudioPlaylist, LocalTrack } from "./library-types";
import { createMiniPlayer, playerArtist } from "./mini-player";
import type { createSceneControls } from "./music-scene-controls";
import { audiobookResumePosition, isPlayable, playbackRates, readPlayback, resumePosition, savePlayback } from "./playback-preferences";
import { connectPlaybackSignal } from "./playback-signal";
import { createPlaybackStage, type StageState } from "./playback-stage";
import { durationLabel } from "./presentation";
import { createTrackWheel, type WheelAction, type WheelEntry, type WheelSession } from "./track-wheel";
import { icon, option } from "./views-common";

export function audioTitle(item: LibraryItem): string {
  return item.details?.audio?.title || item.name.replace(/\.[^.]+$/, "");
}

export function isAudiobook(item: LibraryItem): boolean {
  return Boolean(item.details?.audiobook) || /\.m4b$/i.test(item.name);
}

interface Hooks {
  item(id: string): LibraryItem | undefined;
  command<T>(name: string, args: Record<string, unknown>): Promise<T>;
  render(): void;
  playlists(): AudioPlaylist[];
  thumbnails(): void;
  windowVisible(): Promise<boolean>;
  refresh(): Promise<void>;
  fullscreen(enabled: boolean): Promise<void>;
  miniWindow?(action: "detach-audio" | "detach-video" | "restore" | "focus" | "drag" | "resize-audio" | "resize-video"): Promise<void>;
  desktopPlayback?(available: boolean): void;
  desktopWheel?(session: WheelSession | null): void;
  convert(id: string): void;
  timeChanged(id: string, time: number): void;
  showError(error: unknown): void;
  t: Copy;
}

export function createAudioPlayer(hooks: Hooks) {
  const { t } = hooks;
  const stored = readPlayback();
  const preferences = stored.preferences;
  let media: HTMLVideoElement | null = null;
  let host: HTMLElement | null = null;
  let transport: HTMLElement | null = null;
  let stage: ReturnType<typeof createPlaybackStage> | null = null;
  let mini: ReturnType<typeof createMiniPlayer> | null = null;
  let trackWheel: ReturnType<typeof createTrackWheel> | null = null;
  let scenes: ReturnType<typeof createSceneControls> | null = null;
  let sceneModule: Promise<void> | undefined;
  let visibilityTimer: ReturnType<typeof setTimeout> | undefined;
  let signal: ReturnType<typeof connectPlaybackSignal> | undefined;
  let signalUnavailable = false;
  let expanded = false;
  let fullscreen = false;
  let changingFullscreen = false;
  let detached = false;
  let changingWindow = false;
  let queueOpen = false;
  let tracks: string[] = [];
  let order: number[] = [];
  let cursor = -1;
  let resumeAudioId: string | undefined;
  let expectedSource = "";
  let audioSelection: number | null = null;
  let audioChange: { controller: AbortController; play: boolean } | undefined;
  let pendingSeek: number | null = null;
  let resumeChoice = 0;
  let mediaState: StageState = "loading";
  let sleep = "off";
  let sleepUntil = 0;
  let stoppedBySleep = false;
  let stopAt = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let lastSaved = 0;
  let sourceRevision = 0;
  let issue = "";
  let lastVolume = preferences.volume || 0.5;

  const currentId = () => tracks[order[cursor] ?? -1] ?? "";
  const current = () => hooks.item(currentId());
  const nextEntries = () => order.slice(cursor + 1).map(position => ({ position, id: tracks[position] }));
  const matchesSource = () => Boolean(media && expectedSource && media.currentSrc === expectedSource);
  const wheelEnabled = () => current()?.kind === "audio" && !document.querySelector("dialog[open]");
  const wheelPlaying = () => Boolean(media && !media.paused && !media.ended);
  function wheelEntries(): WheelEntry[] {
    return order.flatMap((track, position) => {
      const item = hooks.item(tracks[track]);
      if (!isPlayable(item)) return [];
      return [{
        position, id: item.id, title: audioTitle(item), artist: playerArtist(item),
        artwork: Boolean(item.kind === "video" || item.details?.audioCover || item.details?.audio?.embeddedCover),
      }];
    });
  }

  function syncDesktopWheel() {
    if (hooks.desktopWheel) hooks.desktopWheel(current()?.kind === "audio"
      ? { entries: wheelEntries(), cursor, playing: wheelPlaying() } : null);
  }

  function wheelAction(action: WheelAction) {
    if (!wheelEnabled() || action.source.position !== cursor || action.source.id !== currentId()) return;
    if (action.type === "toggle") toggleWheelPlayback();
    else if (tracks[order[action.target.position]] === action.target.id) activate(action.target.position, true, false);
  }

  function toggleWheelPlayback() {
    if (media?.paused) void playMedia();
    else media?.pause();
  }

  function saveSession() {
    savePlayback(preferences, cursor >= 0 ? { tracks, order, cursor, resumeAudioId } : null);
  }

  function showScenes() {
    if (scenes) { scenes.show(); monitorVisibility(); return; }
    sceneModule ??= import("./music-scene-controls").then(({ createSceneControls }) => {
      const root = document.getElementById("playback-stage");
      if (!root) return;
      scenes = createSceneControls(root, t, (dt, playing) => signal?.read(dt, playing));
      if (signalUnavailable) scenes.unavailable();
      if (expanded && current()?.kind === "audio") { scenes.show(); monitorVisibility(); updateTime(); }
    }).catch(() => { sceneModule = undefined; });
  }

  function monitorVisibility() {
    clearTimeout(visibilityTimer);
    if (!expanded || current()?.kind !== "audio") return;
    visibilityTimer = setTimeout(() => {
      void hooks.windowVisible().then(visible => {
        if (expanded && current()?.kind === "audio") scenes?.visibility(visible);
      }).catch(() => {}).finally(monitorVisibility);
    }, 420);
  }

  function prepareSignal() {
    if (!media) return;
    if (!signal && !signalUnavailable) {
      try { signal = connectPlaybackSignal(media); }
      catch { signalUnavailable = true; scenes?.unavailable(); }
    }
    if (signal) void signal.resume().catch(() => scenes?.unavailable());
  }

  function showStage() {
    const item = current();
    if (!isPlayable(item) || !stage || !media) return;
    trackWheel?.dismiss();
    mini?.hide();
    expanded = true;
    queueOpen = false;
    stage.show(item, preferences, resumeChoice, issue);
    if (item.kind === "audio") {
      prepareSignal();
      showScenes();
    } else { clearTimeout(visibilityTimer); scenes?.hide(); }
    render();
  }

  function hideStage() {
    expanded = false;
    clearTimeout(visibilityTimer);
    stage?.hide();
    scenes?.hide();
    if (fullscreen) void setFullscreen(false);
  }

  function showMini() {
    const item = current();
    if (!isPlayable(item) || !mini) return;
    trackWheel?.dismiss();
    hideStage();
    queueOpen = false;
    if (item.kind === "audio") prepareSignal();
    if (resumeChoice) {
      const position = resumeChoice;
      resumeChoice = 0;
      seek(position);
    }
    mini.show(item, issue);
    render();
    hooks.render();
  }

  function dockMini() {
    trackWheel?.dismiss();
    mini?.hide();
    render();
    hooks.render();
    document.getElementById("audio-mini")?.focus({ preventScroll: true });
  }

  function setDetached(value: boolean) {
    detached = value;
    document.documentElement.classList.toggle("has-detached-mini", value);
    mini?.detached(value);
  }

  async function restoreMini() {
    if (changingWindow) return false;
    if (!detached || !hooks.miniWindow) return true;
    changingWindow = true;
    try {
      await hooks.miniWindow("restore");
      setDetached(false);
      return true;
    } catch (error) { hooks.showError(error); return false; }
    finally { changingWindow = false; }
  }

  function detachMini(): boolean {
    const item = current();
    if (!mini?.active() || !isPlayable(item) || !hooks.miniWindow) return false;
    if (changingWindow || detached) return true;
    changingWindow = true;
    trackWheel?.dismiss();
    setDetached(true);
    void hooks.miniWindow(item.kind === "video" ? "detach-video" : "detach-audio").catch(error => {
      setDetached(false);
      hooks.showError(error);
    }).finally(() => { changingWindow = false; });
    return true;
  }

  function openFloatingMini() {
    if (changingWindow || !isPlayable(current())) return;
    if (detached) {
      void hooks.miniWindow?.("focus").catch(hooks.showError);
      return;
    }
    if (!mini?.active()) showMini();
    detachMini();
  }

  function backFromStage() {
    if (audioChange) audioChange.play = false;
    if (current()?.kind === "video") media?.pause();
    persist();
    hideStage();
    render();
    hooks.render();
    document.getElementById("audio-open")?.focus({ preventScroll: true });
  }

  async function setFullscreen(enabled: boolean) {
    if (changingFullscreen || fullscreen === enabled) return;
    changingFullscreen = true;
    try {
      await hooks.fullscreen(enabled);
      if (enabled && !expanded) await hooks.fullscreen(false);
      fullscreen = enabled && expanded;
      stage?.setFullscreen(fullscreen);
    } catch (error) { hooks.showError(error); }
    finally { changingFullscreen = false; }
  }

  function volume(value: number) {
    if (!media || !Number.isFinite(value)) return;
    preferences.volume = Math.max(0, Math.min(1, value));
    if (preferences.volume > 0) {
      lastVolume = preferences.volume;
      preferences.muted = false;
    }
    applyVolume();
    saveSession();
    updatePlaying();
  }

  function stepVolume(direction: 1 | -1) {
    volume((Math.round(preferences.volume * 100) + direction * 5) / 100);
  }

  function mute() {
    if (!media) return;
    if (preferences.volume === 0) { volume(lastVolume); return; }
    preferences.muted = !preferences.muted;
    applyVolume();
    saveSession();
    updatePlaying();
  }

  function applyVolume() {
    if (signal) signal.volume(preferences.muted ? 0 : preferences.volume);
    else if (media) { media.volume = preferences.volume; media.muted = preferences.muted; }
  }

  function persist() {
    const item = current();
    if (!media || !item || !matchesSource() || media.readyState === 0 || resumeChoice || pendingSeek !== null || !Number.isFinite(media.currentTime)) return;
    const position = !media.ended ? media.currentTime
      : isAudiobook(item) ? item.duration ?? media.currentTime : 0;
    const details = detailsFor(item);
    const playedAudio = item.kind === "audio" && media.currentTime > 0;
    if (details.playbackPosition === position && (!playedAudio || details.lastPlayedAt)) return;
    item.details = { ...details, playbackPosition: position, lastPlayedAt: playedAudio ? Date.now() : details.lastPlayedAt };
    lastSaved = Date.now();
    return hooks.command<void>("library_playback", { id: item.id, position }).catch(hooks.showError);
  }

  function seek(time: number) {
    if (!media || !expectedSource || !Number.isFinite(time)) return;
    if (!matchesSource() || media.readyState === 0 || !Number.isFinite(media.duration)) {
      pendingSeek = Math.max(0, time);
      return;
    }
    media.currentTime = Math.max(0, Math.min(time, media.duration));
    updateTime();
  }

  function updateTime() {
    if (!media || !transport) return;
    stage?.update(mediaState, issue);
    mini?.update();
    trackWheel?.update();
    scenes?.playing(mediaState === "playing" && !media.paused && !media.ended);
    const elapsed = transport.querySelector<HTMLElement>("[data-audio-elapsed]");
    const end = transport.querySelector<HTMLElement>("[data-audio-duration]");
    const input = transport.querySelector<HTMLInputElement>("#audio-seek");
    const duration = Number.isFinite(media.duration) ? media.duration : 0;
    if (elapsed) elapsed.textContent = durationLabel(media.currentTime);
    if (end) end.textContent = durationLabel(duration);
    if (input) {
      input.max = String(duration);
      input.value = String(media.currentTime);
      input.disabled = duration <= 0;
      input.style.setProperty("--played", `${duration ? Math.min(100, 100 * media.currentTime / duration) : 0}%`);
      input.setAttribute("aria-valuetext", durationLabel(media.currentTime));
    }
    if (matchesSource() && duration > 0 && "mediaSession" in navigator) {
      try {
        navigator.mediaSession.setPositionState({
          duration, position: Math.min(media.currentTime, duration), playbackRate: media.playbackRate,
        });
      } catch { /* The host may not expose system position controls. */ }
    }
  }

  function updatePlaying() {
    if (!media || !transport) return;
    const button = transport.querySelector<HTMLElement>("#audio-toggle");
    if (button) {
      const label = media.paused ? t("Reproducir", "Play") : t("Pausar", "Pause");
      button.setAttribute("aria-label", label);
      button.title = label;
      button.innerHTML = icon(media.paused ? "play" : "pause");
    }
    const muteButton = transport.querySelector<HTMLElement>("#audio-mute");
    if (muteButton) {
      const muted = preferences.muted || preferences.volume === 0;
      const label = muted ? t("Activar sonido", "Unmute") : t("Silenciar", "Mute");
      muteButton.setAttribute("aria-label", label);
      muteButton.title = label;
      muteButton.innerHTML = icon(muted ? "muted" : "volume");
    }
    const volumeInput = transport.querySelector<HTMLInputElement>("#audio-volume");
    if (volumeInput && document.activeElement !== volumeInput) volumeInput.value = String(preferences.volume);
    stage?.update(mediaState, issue);
    mini?.update();
    trackWheel?.update();
    syncDesktopWheel();
    if ("mediaSession" in navigator) navigator.mediaSession.playbackState = media.paused ? "paused" : "playing";
  }

  function clearTimer() {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    stopAt = 0;
    sleepUntil = 0;
  }

  function sleepNow() {
    clearTimer();
    sleep = "off";
    stoppedBySleep = true;
    media?.pause();
    render();
  }

  function chapterBoundary() {
    const item = current();
    if (!item || !media) return 0;
    const position = media.currentTime;
    return detailsFor(item).chapters.find(chapter => chapter.start <= position && chapter.end > position)?.end
      ?? (Number.isFinite(media.duration) ? media.duration : 0);
  }

  function setSleep(value: string) {
    clearTimer();
    sleep = value;
    if (value === "chapter") stopAt = chapterBoundary();
    else if (value !== "off") {
      sleepUntil = Date.now() + Number(value) * 60_000;
      timer = setTimeout(sleepNow, sleepUntil - Date.now());
    }
    render();
  }

  function queueMarkup(stageMode: boolean) {
    const prefix = stageMode ? "music" : "audio";
    const queueButton = (action: string, label: string, pressed: boolean) => `<button id="${prefix}-${action}" type="button" class="button quiet" data-action="audio-player-${action}" aria-pressed="${pressed}">${label}</button>`;
    return `<header><strong>${t("Cola de reproducción", "Playback queue")}</strong>
        <div class="button-group">${queueButton("shuffle", t("Aleatorio", "Shuffle"), preferences.shuffled)}
        ${queueButton("repeat", preferences.repeat === "all" ? t("Repetir todo", "Repeat all") : preferences.repeat === "one" ? t("Repetir pista", "Repeat track") : t("Repetición", "Repeat"), preferences.repeat !== "off")}</div></header>
      <ol>${order.slice(cursor, cursor + 30).map((index, offset) => {
        const queued = hooks.item(tracks[index]);
        if (!queued) return "";
        const entryButton = (direction: "up" | "down", text: string, disabled: boolean, symbol: string) => `<button id="${prefix}-queue-${index}-${direction}" type="button" class="icon-button" data-action="audio-player-queue-${direction}" data-position="${index}" aria-label="${text}" ${disabled ? "disabled" : ""}>${symbol}</button>`;
        return `<li><span ${offset === 0 ? 'aria-current="true"' : ""}>${h(audioTitle(queued))}</span>${offset > 0
          ? entryButton("up", t("Subir en la cola", "Move up in queue"), offset < 2, "↑") +
            entryButton("down", t("Bajar en la cola", "Move down in queue"), cursor + offset + 1 >= order.length, "↓") : ""}</li>`;
      }).join("")}</ol>
      ${order.length - cursor > 30 ? `<p class="field-note">${t("Se muestran los primeros 30 archivos.", "Showing the first 30 files.")}</p>` : ""}`;
  }

  function trackInfo(item: LibraryItem) {
    const tags = item.details?.audio;
    const lists = hooks.playlists().filter(list => !list.entries.some(entry => entry.kind === "local" && entry.id === item.id));
    return `<h2>${h(audioTitle(item))}</h2><dl>${[[t("Artista", "Artist"), tags?.artist], [t("Álbum", "Album"), tags?.album],
      [t("Género", "Genre"), tags?.genre], [t("Archivo", "File"), item.name]].filter(([, value]) => value).map(([name, value]) => `<div><dt>${name}</dt><dd>${h(value ?? "")}</dd></div>`).join("")}</dl>
      <button id="music-favorite" type="button" class="button quiet" data-action="audio-player-favorite" aria-pressed="${detailsFor(item).favorite}">${icon("heart")}<span>${detailsFor(item).favorite ? t("Quitar favorito", "Remove favorite") : t("Marcar como favorito", "Mark as favorite")}</span></button>
      ${lists.length ? `<div class="music-add-list"><label for="music-add-list">${t("Añadir a una lista", "Add to a playlist")}<select id="music-add-list">${lists.map(list => option(list.id, list.name, "")).join("")}</select></label>
        <button id="music-playlist-save" type="button" class="button secondary" data-action="audio-player-add-list">${t("Añadir", "Add")}</button></div>` : ""}`;
  }

  function render() {
    if (!host || !transport || !media) return;
    const item = current();
    const floating = mini?.active();
    hooks.desktopPlayback?.(isPlayable(item));
    host.hidden = !item || expanded || Boolean(floating);
    document.documentElement.classList.toggle("has-audio-player", !host.hidden);
    if (!item) { transport.replaceChildren(); syncDesktopWheel(); return; }
    mini?.sync(item, issue);
    const book = isAudiobook(item);
    const active = transport.contains(document.activeElement) && document.activeElement instanceof HTMLElement
      ? document.activeElement.id : "";
    const title = audioTitle(item);
    const subtitle = [item.details?.audio?.artist ?? item.details?.creator, item.details?.audiobook?.bookTitle ?? item.details?.audio?.album].filter(Boolean).join(" · ");
    const button = (id: string, text: string, symbol?: Parameters<typeof icon>[0], attributes = "") =>
      `<button id="audio-${id}" class="${symbol ? "icon-button" : "button quiet"}" type="button" data-action="audio-player-${id}" aria-label="${h(text)}" title="${h(text)}" ${attributes}>${symbol ? icon(symbol) : h(text)}</button>`;
    transport.innerHTML = `
      <button id="audio-open" class="audio-open" type="button" data-action="audio-player-open" title="${h(item.kind === "video" ? t("Abrir vídeo", "Open video") : t("Abrir visualización", "Open visualization"))}">
        <span class="audio-cover" aria-hidden="true"${item.details?.audioCover || item.details?.audio?.embeddedCover || item.kind === "video" ? ` data-thumbnail-id="${h(item.id)}"` : ""}>${icon(item.kind === "video" ? "video" : "audio")}</span>
        <span class="audio-playing-label"><strong>${h(title)}</strong>${subtitle ? `<small>${h(subtitle)}</small>` : ""}</span>
      </button>
      <div class="audio-timeline"><div class="audio-transport-controls">
        ${book ? button("back", t("Retroceder 15 segundos", "Back 15 seconds"), "seekBack") : button("previous", t("Anterior", "Previous"), "previous")}
        ${button("toggle", t("Reproducir", "Play"), "play")}
        ${button("next", t("Siguiente", "Next"), "next")}
      </div><div class="audio-progress"><span data-audio-elapsed>0:00</span><input type="range" id="audio-seek" min="0" max="0" step="0.1" value="0" aria-label="${t("Posición de reproducción", "Playback position")}" disabled><span data-audio-duration>0:00</span></div></div>
      <div class="audio-playback-options">${book ? `
        <select id="audio-rate" aria-label="${t("Velocidad", "Playback speed")}">${playbackRates.map(speed => `<option value="${speed}" ${speed === preferences.rate ? "selected" : ""}>${speed}×</option>`).join("")}</select>
        <select id="audio-sleep" aria-label="${t("Temporizador de descanso", "Sleep timer")}">${[["off", t("Descanso", "Sleep timer")], ["10", "10 min"], ["20", "20 min"], ["30", "30 min"], ["45", "45 min"], ["60", "60 min"], ["chapter", t("Fin del capítulo", "End of chapter")]].map(([value, name]) => `<option value="${value}" ${value === sleep ? "selected" : ""}>${name}</option>`).join("")}</select>
      ` : ""}
      <div class="audio-volume">${button("mute", t("Silenciar", "Mute"), "volume")}<input id="audio-volume" type="range" min="0" max="1" step="0.01" value="${preferences.volume}" aria-label="${t("Volumen", "Volume")}"></div>
      ${item.kind === "audio" ? `<button type="button" class="icon-button audio-visualize" data-action="audio-player-open" aria-label="${t("Abrir visualización", "Open visualization")}" title="${t("Abrir visualización", "Open visualization")}">${icon("wave")}</button>` : ""}
      ${button("mini", t("Abrir minirreproductor", "Open mini player"), "miniPlayer")}
      ${button("queue", t("Cola de reproducción", "Playback queue"), "list", `aria-expanded="${queueOpen}" aria-controls="audio-queue-panel"`)}
      ${button("close", t("Detener y cerrar", "Stop and close"), "close")}</div>
      ${issue && !expanded ? `<div class="audio-player-issue" role="alert"><span>${h(issue)}</span><div class="button-group">
        ${button("convert", t("Convertir una copia", "Convert a copy"))}${button("external", t("Abrir con el sistema", "Open with system"))}
      </div></div>` : ""}
      <section id="audio-queue-panel" class="audio-queue-panel" aria-label="${t("Cola de reproducción", "Playback queue")}" ${queueOpen ? "" : "hidden"}>
        ${queueMarkup(false)}
        <label class="toggle"><input id="audio-resume-music-compact" type="checkbox" ${preferences.resumeMusic ? "checked" : ""}>${t("Reanudar pistas de música", "Resume music tracks")}</label>
      </section>`;
    host.setAttribute("aria-label", t("Reproductor compacto", "Compact player"));
    updatePlaying();
    updateTime();
    stage?.syncMusic();
    hooks.thumbnails();
    if (active) document.getElementById(active)?.focus({ preventScroll: true });
  }

  function mediaSession(item: LibraryItem) {
    if (!("mediaSession" in navigator)) return;
    if (typeof MediaMetadata !== "undefined") {
      navigator.mediaSession.metadata = new MediaMetadata({
        title: audioTitle(item), artist: item.details?.audio?.artist ?? item.details?.creator ?? "",
        album: item.details?.audio?.album ?? item.details?.audiobook?.bookTitle ?? "",
      });
    }
    const actions: [MediaSessionAction, MediaSessionActionHandler][] = [
      ["play", () => { void playMedia(); }], ["pause", () => media?.pause()],
      ["nexttrack", () => move(1, true)], ["previoustrack", () => move(-1, true)],
      ["seekbackward", detail => seek((media?.currentTime ?? 0) - (detail.seekOffset ?? 15))],
      ["seekforward", detail => seek((media?.currentTime ?? 0) + (detail.seekOffset ?? 15))],
      ["seekto", detail => { if (detail.seekTime !== undefined) seek(detail.seekTime); }],
    ];
    for (const [action, handler] of actions) {
      try { navigator.mediaSession.setActionHandler(action, handler); }
      catch { /* WebView versions can expose different media controls. */ }
    }
  }

  async function playMedia() {
    if (!media || audioChange) return;
    if (signal) void signal.resume().catch(() => scenes?.unavailable());
    const revision = sourceRevision;
    try {
      if (media.ended) seek(0);
      await media.play();
    } catch (error) {
      if (revision !== sourceRevision || error instanceof DOMException && error.name === "AbortError") return;
      if (!issue) issue = playbackIssue(media.error);
      mediaState = "error";
      render();
      updateTime();
    }
  }

  function playbackIssue(error: MediaError | null): string {
    if (current()?.missing || error?.code === 2) return t("El archivo no está disponible. Revisa su ubicación en Biblioteca.", "The file is unavailable. Check its location in Library.");
    if (error?.code === 3) return t("El motor no pudo decodificar el archivo. Puede estar dañado o incompleto.", "The engine could not decode the file. It may be damaged or incomplete.");
    return t("El formato o los códecs de este archivo no se pueden reproducir en este equipo.", "This device cannot play the file's format or codecs.");
  }

  function verifyMissing() {
    const revision = sourceRevision;
    void hooks.refresh().then(() => {
      if (revision !== sourceRevision || !issue || !current()?.missing) return;
      issue = playbackIssue(media?.error ?? null);
      render(); updateTime();
    }).catch(() => {});
  }

  function resumeVideo(fromStart: boolean) {
    const position = fromStart ? 0 : resumeChoice;
    resumeChoice = 0;
    seek(position);
    const item = current();
    if (item && expanded) stage?.render(item, preferences, 0, issue);
    void playMedia();
  }

  function cancelAudioChange() {
    if (!audioChange) return;
    audioChange.controller.abort();
    audioChange = undefined;
    if (media) media.preload = "metadata";
  }

  async function switchAudio(track: number | null) {
    const element = media;
    const item = current();
    if (!element || item?.kind !== "video" || audioChange || !matchesSource()) return false;
    if (track === audioSelection) return true;
    persist();
    const change = { controller: new AbortController(), play: !element.paused && !resumeChoice };
    audioChange = change;
    const previous = expectedSource;
    const position = resumeChoice ? 0 : pendingSeek ?? element.currentTime;
    const original = localMediaUrl(item.id);
    const next = track === null ? original : `${original}?audio=${track}`;

    const load = (url: string) => new Promise<boolean>(resolve => {
      const listeners = new AbortController();
      const finish = (ready: boolean) => { listeners.abort(); clearTimeout(timeout); resolve(ready); };
      const timeout = setTimeout(() => finish(false), 8000);
      const check = () => {
        if (matchesSource() && element.readyState >= 2 && !element.seeking && pendingSeek === null) finish(true);
      };
      for (const event of ["loadeddata", "seeked", "canplay"]) {
        element.addEventListener(event, check, { signal: listeners.signal });
      }
      element.addEventListener("error", () => { if (element.error && element.src === url) finish(false); }, { signal: listeners.signal });
      change.controller.signal.addEventListener("abort", () => finish(false), { signal: listeners.signal });
      ++sourceRevision;
      element.pause();
      pendingSeek = position || null;
      expectedSource = url;
      issue = "";
      mediaState = "loading";
      element.preload = "auto";
      element.src = url;
      updateTime();
    });
    const switched = await load(next);
    if (audioChange !== change) return;
    if (switched) audioSelection = track;
    else if (!await load(previous) && audioChange === change) {
      issue = playbackIssue(element.error);
      mediaState = "error";
    }
    if (audioChange !== change) return;
    cancelAudioChange();
    element.playbackRate = preferences.rate;
    applyVolume();
    updateTime();
    if (change.play && (expanded || mini?.active()) && !issue) void playMedia();
    return switched;
  }

  function activate(position: number, autoplay: boolean, resume: boolean, openVideo = true) {
    const id = tracks[order[position] ?? -1];
    const item = id ? hooks.item(id) : undefined;
    if (!media || !isPlayable(item)) return;
    if (detached && item.kind !== current()?.kind) void hooks.miniWindow?.(item.kind === "video" ? "resize-video" : "resize-audio").catch(hooks.showError);
    trackWheel?.dismiss();
    cancelAudioChange();
    persist();
    media.pause();
    if (sleep === "chapter" || !isAudiobook(item) && sleep !== "off") {
      clearTimer();
      sleep = "off";
    }
    cursor = position;
    stoppedBySleep = false;
    issue = "";
    ++sourceRevision;
    const book = isAudiobook(item);
    const resumeAt = book ? audiobookResumePosition : resumePosition;
    const longAudio = book || item.details?.platform === "Podcast RSS" || (item.duration ?? 0) >= 30 * 60;
    const progress = resume && (item.kind === "video" || longAudio || item.id === resumeAudioId || preferences.resumeMusic)
      ? resumeAt(detailsFor(item).playbackPosition, item.duration) : 0;
    const floating = mini?.active();
    resumeChoice = item.kind === "video" && openVideo && autoplay && !floating ? progress : 0;
    pendingSeek = progress && !resumeChoice ? progress : null;
    mediaState = "loading";
    expectedSource = localMediaUrl(item.id);
    audioSelection = null;
    media.dataset.localPlayer = item.id;
    media.src = expectedSource;
    media.playbackRate = preferences.rate;
    lastSaved = Date.now();
    mediaSession(item);
    saveSession();
    signal?.reset(); scenes?.reset();
    if (floating && item.kind === "audio") prepareSignal();
    if (!floating) {
      if (item.kind === "video" && openVideo || item.kind === "audio" && expanded) showStage();
      else hideStage();
    }
    render();
    hooks.render();
    if (autoplay && !resumeChoice) void playMedia();
  }

  function reorder(keep: number) {
    order = tracks.map((_, index) => index);
    if (!preferences.shuffled) return order.indexOf(keep);
    order = order.filter(index => index !== keep);
    for (let index = order.length - 1; index > 0; index--) {
      const other = Math.floor(Math.random() * (index + 1));
      [order[index], order[other]] = [order[other], order[index]];
    }
    order.unshift(keep);
    return 0;
  }

  function play(ids: string[], start: number, resumeShortAudio = false) {
    const id = ids[start];
    const item = id ? hooks.item(id) : undefined;
    if (!isPlayable(item)) return;
    const index = ids.slice(0, start).filter(value => isPlayable(hooks.item(value))).length;
    if (index >= 2000) return;
    const same = currentId() === item.id && Boolean(expectedSource) && !issue;
    trackWheel?.dismiss();
    persist();
    tracks = ids.filter(value => isPlayable(hooks.item(value))).slice(0, 2000);
    resumeAudioId = resumeShortAudio && item.kind === "audio" ? item.id : undefined;
    cursor = -1;
    if (isAudiobook(item)) preferences.shuffled = false;
    const position = reorder(index);
    if (same) {
      cursor = position;
      if (!mini?.active() && (item.kind === "video" || expanded)) showStage();
      if (!resumeChoice && media?.paused) void playMedia();
      saveSession();
      render();
      hooks.render();
    } else activate(position, true, true);
  }

  function add(id: string) {
    if (!isPlayable(hooks.item(id)) || tracks.length >= 2000) return;
    trackWheel?.dismiss();
    tracks.push(id);
    order.push(tracks.length - 1);
    if (cursor < 0) { resumeAudioId = undefined; activate(0, false, true, false); }
    else { saveSession(); render(); hooks.render(); }
  }

  function remove(position: number) {
    const playing = order[cursor];
    if (position === playing || !order.includes(position)) return;
    trackWheel?.dismiss();
    tracks.splice(position, 1);
    order = order.filter(index => index !== position).map(index => index > position ? index - 1 : index);
    cursor = order.indexOf(playing > position ? playing - 1 : playing);
    saveSession();
    render();
    hooks.render();
  }

  function move(direction: 1 | -1, manual: boolean) {
    if (!media || cursor < 0) return;
    if (direction === -1 && media.currentTime > 3) { seek(0); return; }
    const item = current();
    const book = item ? isAudiobook(item) : false;
    if (!manual && !preferences.autoplay) return;
    if (!manual && !book && preferences.repeat === "one") { seek(0); void playMedia(); return; }
    let position = cursor + direction;
    for (let tried = 0; tried < order.length; tried++, position += direction) {
      if (!book && preferences.repeat === "all") position = (position + order.length) % order.length;
      if (position < 0 || position >= order.length) break;
      const item = hooks.item(tracks[order[position]]);
      if (isPlayable(item)) { activate(position, true, false); return; }
    }
    updatePlaying();
  }

  function close() {
    cancelAudioChange();
    persist();
    media?.pause();
    ++sourceRevision;
    expectedSource = "";
    audioSelection = null;
    pendingSeek = null;
    resumeChoice = 0;
    queueOpen = false;
    trackWheel?.dismiss();
    mini?.hide();
    hideStage();
    scenes?.release();
    stage?.clearPreview();
    tracks = [];
    order = [];
    cursor = -1;
    clearTimer();
    sleep = "off";
    media?.removeAttribute("src");
    media?.removeAttribute("data-local-player");
    media?.load();
    if (signal) void signal.suspend().catch(() => {});
    saveSession();
    if ("mediaSession" in navigator) {
      navigator.mediaSession.metadata = null;
      navigator.mediaSession.playbackState = "none";
      for (const action of ["play", "pause", "nexttrack", "previoustrack", "seekbackward", "seekforward", "seekto"] as const) {
        try { navigator.mediaSession.setActionHandler(action, null); } catch { /* Optional control. */ }
      }
    }
    render();
    hooks.render();
  }

  function mount() {
    host = document.getElementById("audio-player");
    const element = document.getElementById("audio-engine");
    const stageHost = document.getElementById("playback-stage");
    if (!host || !stageHost || !(element instanceof HTMLVideoElement)) return;
    media = element;
    element.volume = preferences.volume;
    element.muted = preferences.muted;
    transport = document.getElementById("audio-transport");
    stage = createPlaybackStage(stageHost, element, {
      t, seek, mute, volume, stepVolume, sound: () => preferences, back: backFromStage, mini: showMini, resume: resumeVideo, command: hooks.command,
      audioSelection: () => audioSelection, switchAudio,
      musicTitle: audioTitle, queue: () => queueMarkup(true), trackInfo, sceneControls: () => scenes?.render(),
      toggle: () => { if (element.paused) void playMedia(); else element.pause(); },
      rate: value => { preferences.rate = value; element.playbackRate = value; saveSession(); },
      preference: (key, value) => { preferences[key] = value; saveSession(); },
      captionSize: value => { preferences.captionSize = value; saveSession(); },
      fullscreen: () => { void setFullscreen(!fullscreen); },
      move: direction => move(direction, true),
      convert: id => { backFromStage(); hooks.convert(id); },
      external: openExternal,
      importCaption: async language => {
        const id = currentId();
        try {
          const track = await hooks.command<LocalTrack>("subtitle_import", { id, language, path: null });
          await hooks.refresh();
          const item = current();
          if (expanded && item?.id === id) {
            stage?.render(item, preferences, resumeChoice, issue);
            stage?.selectCaption(track.id);
          }
        } catch (error) {
          if (!String(error).includes("Cancelled")) hooks.showError(error);
        }
      },
    });
    trackWheel = createTrackWheel({
      t, thumbnails: hooks.thumbnails, toggle: toggleWheelPlayback,
      playing: wheelPlaying, enabled: wheelEnabled, entries: wheelEntries,
      cursor: () => cursor,
      play: ({ position, id }) => {
        if (tracks[order[position]] === id) activate(position, true, false);
      },
    });
    mini = createMiniPlayer(stageHost, element, {
      t, seek, mute, volume, title: audioTitle, thumbnails: hooks.thumbnails,
      sound: () => preferences,
      signal: dt => signal?.read(dt, mediaState === "playing"),
      toggle: () => { if (element.paused) void playMedia(); else element.pause(); },
      move: direction => move(direction, true),
      expand: () => { void restoreMini().then(restored => { if (restored) { showStage(); hooks.render(); } }); },
      dock: () => { void restoreMini().then(restored => { if (restored) dockMini(); }); },
      float: hooks.miniWindow ? () => { if (detached) void restoreMini(); else detachMini(); } : undefined,
      drag: () => { if (detached) void hooks.miniWindow?.("drag").catch(hooks.showError); },
      wheel: button => trackWheel?.toggle(button),
    });
    const playerHost = host;
    new ResizeObserver(() => {
      document.documentElement.style.setProperty("--audio-player-height", `${playerHost.offsetHeight}px`);
    }).observe(playerHost);
    element.addEventListener("timeupdate", () => {
      if (!matchesSource()) return;
      updateTime();
      hooks.timeChanged(currentId(), element.currentTime);
      if (sleep === "chapter" && stopAt > 0 && element.currentTime >= stopAt || sleepUntil && Date.now() >= sleepUntil) sleepNow();
      if (!element.paused && Date.now() - lastSaved > 15_000) persist();
    });
    element.addEventListener("loadedmetadata", () => {
      if (!matchesSource() || element.readyState < 1) return;
      if (pendingSeek !== null) {
        const position = pendingSeek;
        pendingSeek = null;
        seek(position);
      }
      if (resumeChoice && !resumePosition(resumeChoice, element.duration)) resumeChoice = 0;
      const item = current();
      if (item?.kind === "video" && expanded) stage?.render(item, preferences, resumeChoice, issue);
      if (element.paused && mediaState === "loading") mediaState = "paused";
      updateTime();
    });
    element.textTracks.addEventListener("addtrack", () => {
      const item = current();
      if (item?.kind === "video" && expanded && matchesSource()) stage?.render(item, preferences, resumeChoice, issue);
    });
    for (const event of ["waiting", "stalled", "seeking", "seeked", "playing", "canplay", "progress"] as const) {
      element.addEventListener(event, () => {
        if (!matchesSource()) return;
        if (event === "seeking") { mediaState = "seeking"; signal?.reset(); scenes?.reset(); }
        else if (event === "waiting" || event === "stalled") {
          if (!element.paused || element.readyState === 0) mediaState = "loading";
        } else if (event !== "progress" && !element.ended && !element.seeking) mediaState = element.paused ? "paused" : "playing";
        updateTime();
      });
    }
    element.addEventListener("play", () => {
      if (!matchesSource()) return;
      stoppedBySleep = false;
      if (element.readyState >= 3) mediaState = "playing";
      updatePlaying();
    });
    element.addEventListener("pause", () => {
      if (!matchesSource() || !element.paused) return;
      if (!element.ended && !issue) mediaState = element.readyState ? "paused" : "loading";
      persist(); updatePlaying();
    });
    element.addEventListener("ended", () => {
      if (!matchesSource() || !element.ended) return;
      const item = current();
      mediaState = "ended";
      persist();
      updateTime();
      if (sleep === "chapter") sleepNow();
      else if (!stoppedBySleep) move(1, false);
      if (item && isAudiobook(item) && currentId() === item.id) hooks.render();
    });
    element.addEventListener("error", () => {
      if (!expectedSource || element.src !== expectedSource || !element.error) return;
      if (audioChange) return;
      issue = playbackIssue(element.error);
      mediaState = "error";
      render(); updateTime();
      if (element.error.code === 4) verifyMissing();
    });
    document.addEventListener("play", event => {
      if (!(event.target instanceof HTMLMediaElement)) return;
      if (event.target === element) {
        for (const other of document.querySelectorAll<HTMLMediaElement>("audio,video")) if (other !== element && !other.paused) other.pause();
      } else element.pause();
    }, true);
  }

  function input(element: HTMLElement): boolean {
    if (element instanceof HTMLInputElement && element.id === "audio-seek") {
      seek(Number(element.value)); return true;
    }
    if (element instanceof HTMLInputElement && element.id === "audio-volume") {
      volume(Number(element.value)); return true;
    }
    if (element instanceof HTMLInputElement && (element.id === "audio-resume-music" || element.id === "audio-resume-music-compact")) {
      preferences.resumeMusic = element.checked; saveSession(); return true;
    }
    if (element instanceof HTMLSelectElement && element.id === "audio-rate") {
      preferences.rate = Number(element.value);
      if (media) media.playbackRate = preferences.rate;
      saveSession();
      return true;
    }
    if (element instanceof HTMLSelectElement && element.id === "audio-sleep") {
      if (element.value === "off" || element.value === "chapter" || ["10", "20", "30", "45", "60"].includes(element.value)) setSleep(element.value);
      return true;
    }
    return false;
  }

  function restore() {
    if (!stored.queue || current()) return;
    const saved = stored.queue;
    const remap = new Map<number, number>();
    tracks = saved.tracks.filter((id, index) => {
      if (!isPlayable(hooks.item(id))) return false;
      remap.set(index, remap.size);
      return true;
    });
    if (!tracks.length) { saveSession(); return; }
    resumeAudioId = saved.resumeAudioId;
    order = saved.order.flatMap(index => remap.has(index) ? [remap.get(index)!] : []);
    const previous = order.indexOf(remap.get(saved.order[saved.cursor]) ?? -1);
    activate(Math.max(0, previous), false, true, false);
  }

  function playAt(id: string, time: number, resumeShortAudio = false): boolean {
    const item = hooks.item(id);
    if (!isPlayable(item)) return false;
    const resumeId = resumeShortAudio && item.kind === "audio" ? id : undefined;
    if (id !== currentId() || issue) {
      persist();
      tracks = [id]; order = [0]; cursor = -1; resumeAudioId = resumeId;
      activate(0, false, false);
    } else if (item.kind === "video") showStage();
    if (resumeId && resumeId !== resumeAudioId) { resumeAudioId = resumeId; saveSession(); }
    if (resumeChoice) {
      resumeChoice = 0;
      stage?.render(item, preferences, 0, issue);
    }
    seek(time);
    return true;
  }

  function shuffleUpcoming() {
    trackWheel?.dismiss();
    preferences.shuffled = !preferences.shuffled;
    const history = order.slice(0, cursor + 1);
    const upcoming = order.slice(cursor + 1);
    if (preferences.shuffled) {
      for (let index = upcoming.length - 1; index > 0; index--) {
        const other = Math.floor(Math.random() * (index + 1));
        [upcoming[index], upcoming[other]] = [upcoming[other], upcoming[index]];
      }
    } else upcoming.sort((a, b) => a - b);
    order = [...history, ...upcoming];
    saveSession(); render(); hooks.render();
  }

  function key(event: KeyboardEvent): boolean {
    if (expanded) return stage?.key(event) ?? false;
    if (queueOpen && event.key === "Escape") {
      queueOpen = false; render(); document.getElementById("audio-queue")?.focus(); return true;
    }
    const target = event.target;
    const inMini = target instanceof HTMLElement && mini?.contains(target);
    if ((inMini || detached) && event.key === "Escape") { void restoreMini().then(restored => { if (restored) dockMini(); }); return true; }
    if ((event.key === "ArrowUp" || event.key === "ArrowDown")
      && !event.altKey && !event.metaKey && !event.shiftKey && !event.ctrlKey
      && target instanceof HTMLElement && (inMini || host && !host.hidden && host.contains(target))
      && !target.closest("input, select, textarea, [contenteditable]:not([contenteditable='false'])")) {
      stepVolume(event.key === "ArrowUp" ? 1 : -1);
      return true;
    }
    return false;
  }

  function openExternal(id: string) {
    media?.pause();
    void hooks.command("open_media", { id, folder: false }).catch(hooks.showError);
  }

  function click(action: string, control?: HTMLElement): boolean {
    switch (action) {
      case "audio-player-toggle":
        if (resumeChoice) resumeVideo(false);
        else if (media?.paused) void playMedia();
        else media?.pause();
        break;
      case "audio-player-next": move(1, true); break;
      case "audio-player-previous": move(-1, true); break;
      case "audio-player-back": seek((media?.currentTime ?? 0) - 15); break;
      case "audio-player-mute": mute(); break;
      case "audio-player-shuffle": shuffleUpcoming(); break;
      case "audio-player-repeat":
        preferences.repeat = preferences.repeat === "off" ? "all" : preferences.repeat === "all" ? "one" : "off";
        saveSession(); render(); break;
      case "audio-player-queue":
        queueOpen = !queueOpen;
        render();
        break;
      case "audio-player-queue-up": case "audio-player-queue-down": {
        const position = order.indexOf(Number(control?.dataset.position));
        const target = position + (action.endsWith("-up") ? -1 : 1);
        if (position > cursor && target > cursor && target < order.length) {
          trackWheel?.dismiss();
          [order[position], order[target]] = [order[target], order[position]];
          saveSession(); render(); hooks.render();
        }
        break;
      }
      case "audio-player-convert": if (currentId()) hooks.convert(currentId()); break;
      case "audio-player-external": if (currentId()) openExternal(currentId()); break;
      case "audio-player-favorite": {
        const item = current();
        if (item) void hooks.command("library_favorite", { id: item.id, favorite: !detailsFor(item).favorite }).then(hooks.refresh).then(() => stage?.syncMusic()).catch(hooks.showError);
        break;
      }
      case "audio-player-add-list": {
        const item = current();
        const choice = document.querySelector<HTMLSelectElement>("#music-add-list")?.value;
        const list = hooks.playlists().find(candidate => candidate.id === choice);
        if (item && list && control) {
          (control as HTMLButtonElement).disabled = true;
          const { id, name, entries } = list;
          void hooks.command("audio_playlist_save", { id, name, entries: [...entries, { kind: "local", id: item.id, title: audioTitle(item) }] })
            .then(hooks.refresh).then(() => stage?.syncMusic()).catch(hooks.showError).finally(() => { (control as HTMLButtonElement).disabled = false; });
        }
        break;
      }
      case "audio-player-open":
        showStage(); hooks.render();
        break;
      case "audio-player-mini": showMini(); break;
      case "audio-player-close": close(); break;
      default: return false;
    }
    return true;
  }

  return { mount, render, restore, play, playAt, add, remove, input, click, key, current, nextEntries, seek, persist, detachMini, openFloatingMini,
    nativeMiniRestored: () => setDetached(false), syncDesktopWheel, wheelAction, isExpanded: () => expanded, isDetached: () => detached,
    timeFor: (id: string) => id === currentId() && matchesSource() && media?.readyState ? media.currentTime : undefined,
    time: () => media?.currentTime ?? 0 };
}
