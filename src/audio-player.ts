import { escapeHtml as h, localMediaUrl, type Copy, type LibraryItem } from "./contracts";
import { detailsFor } from "./library-features";
import { durationLabel } from "./presentation";
import { icon } from "./views-common";

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
  open(): void;
  showError(error: unknown): void;
  t: Copy;
}

type Repeat = "off" | "all" | "one";

export function createAudioPlayer(hooks: Hooks) {
  const { t } = hooks;
  let media: HTMLAudioElement | null = null;
  let host: HTMLElement | null = null;
  let transport: HTMLElement | null = null;
  let tracks: string[] = [];
  let order: number[] = [];
  let cursor = -1;
  let repeat: Repeat = "off";
  let shuffled = false;
  let rate = 1;
  let sleep = "off";
  let sleepUntil = 0;
  let stoppedBySleep = false;
  let stopAt = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let lastSaved = 0;
  let sourceRevision = 0;
  let issue = "";

  const currentId = () => tracks[order[cursor] ?? -1] ?? "";
  const current = () => hooks.item(currentId());
  const nextEntries = () => order.slice(cursor + 1).map(position => ({ position, id: tracks[position] }));

  function persist() {
    const item = current();
    if (!media || !item || media.readyState === 0 || !Number.isFinite(media.currentTime) || media.currentTime <= 0) return;
    const position = media.currentTime;
    item.details = { ...detailsFor(item), playbackPosition: position, lastPlayedAt: Date.now() };
    lastSaved = Date.now();
    void hooks.command("library_playback", { id: item.id, position }).catch(hooks.showError);
  }

  function seek(time: number) {
    if (!media || !Number.isFinite(media.duration) || !Number.isFinite(time)) return;
    media.currentTime = Math.max(0, Math.min(time, media.duration));
    updateTime();
  }

  function updateTime() {
    if (!media || !transport) return;
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
      input.setAttribute("aria-valuetext", durationLabel(media.currentTime));
    }
    if (duration > 0 && "mediaSession" in navigator) {
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

  function render() {
    if (!host || !transport || !media) return;
    const item = current();
    host.hidden = !item;
    document.documentElement.classList.toggle("has-audio-player", Boolean(item));
    if (!item) { transport.replaceChildren(); return; }
    const book = isAudiobook(item);
    const active = transport.contains(document.activeElement) && document.activeElement instanceof HTMLElement
      ? document.activeElement.id : "";
    const title = audioTitle(item);
    const subtitle = [item.details?.audio?.artist ?? item.details?.creator, item.details?.audiobook?.bookTitle ?? item.details?.audio?.album].filter(Boolean).join(" · ");
    const button = (id: string, text: string, symbol?: Parameters<typeof icon>[0], pressed?: boolean) =>
      `<button id="audio-${id}" class="${symbol ? "icon-button" : "button quiet"}" type="button" data-action="audio-player-${id}" aria-label="${h(text)}" title="${h(text)}"${pressed === undefined ? "" : ` aria-pressed="${pressed}"`}>${symbol ? icon(symbol) : h(text)}</button>`;
    transport.innerHTML = `
      <div class="audio-playing"><span class="audio-cover" aria-hidden="true"${item.details?.audioCover || item.details?.audio?.embeddedCover ? ` data-thumbnail-id="${h(item.id)}"` : ""}>${icon("audio")}</span>
        <button id="audio-open" class="audio-open" type="button" data-action="audio-player-open" title="${h(title)}"><strong>${h(title)}</strong><small>${h(subtitle || t("Biblioteca de audio", "Audio library"))}</small></button></div>
      <div class="audio-timeline"><div class="audio-transport-controls">
        ${book ? button("back", t("Retroceder 15 segundos", "Back 15 seconds")) : button("previous", t("Anterior", "Previous"))}
        ${button("toggle", t("Reproducir", "Play"), "play")}
        ${button("next", t("Siguiente", "Next"))}
      </div><div class="audio-progress"><span data-audio-elapsed>0:00</span><input type="range" id="audio-seek" min="0" max="0" step="0.1" value="0" aria-label="${t("Posición de reproducción", "Playback position")}" disabled><span data-audio-duration>0:00</span></div></div>
      <div class="audio-playback-options">${book ? `
        <select id="audio-rate" aria-label="${t("Velocidad", "Playback speed")}">${[0.75, 1, 1.25, 1.5, 1.75, 2].map(speed => `<option value="${speed}" ${speed === rate ? "selected" : ""}>${speed}×</option>`).join("")}</select>
        <select id="audio-sleep" aria-label="${t("Temporizador de descanso", "Sleep timer")}">${[["off", t("Descanso", "Sleep timer")], ["10", "10 min"], ["20", "20 min"], ["30", "30 min"], ["45", "45 min"], ["60", "60 min"], ["chapter", t("Fin del capítulo", "End of chapter")]].map(([value, name]) => `<option value="${value}" ${value === sleep ? "selected" : ""}>${name}</option>`).join("")}</select>
      ` : `${button("shuffle", t("Aleatorio", "Shuffle"), undefined, shuffled)}${button("repeat", repeat === "all" ? t("Repetir todo", "Repeat all") : repeat === "one" ? t("Repetir pista", "Repeat track") : t("Repetición", "Repeat"), undefined, repeat !== "off")}`}
      ${button("close", t("Cerrar reproductor", "Close player"), "close")}</div>
      ${issue ? `<p class="audio-player-issue" role="alert">${h(issue)}</p>` : ""}`;
    host.setAttribute("aria-label", t("Reproductor de audio", "Audio player"));
    updatePlaying();
    updateTime();
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
    if (!media) return;
    const revision = sourceRevision;
    try { await media.play(); }
    catch {
      if (revision !== sourceRevision) return;
      issue = t("No se pudo reproducir este archivo. Revisa el formato o su ubicación.", "Could not play this file. Check its format or location.");
      render();
    }
  }

  function activate(position: number, autoplay: boolean, resume: boolean) {
    const id = tracks[order[position] ?? -1];
    const item = id ? hooks.item(id) : undefined;
    if (!media || !item || item.missing) return;
    persist();
    media.pause();
    if (sleep === "chapter" || !isAudiobook(item) && sleep !== "off") {
      clearTimer();
      sleep = "off";
    }
    cursor = position;
    stoppedBySleep = false;
    issue = "";
    const revision = ++sourceRevision;
    const progress = resume ? detailsFor(item).playbackPosition : 0;
    media.src = localMediaUrl(item.id);
    media.playbackRate = isAudiobook(item) ? rate : 1;
    if (progress > 0) media.addEventListener("loadedmetadata", () => {
      if (revision === sourceRevision && progress < (media?.duration ?? 0) - 2) seek(progress);
    }, { once: true });
    lastSaved = Date.now();
    mediaSession(item);
    render();
    hooks.render();
    if (autoplay) void playMedia();
  }

  function reorder(keep: number) {
    order = tracks.map((_, index) => index);
    if (!shuffled) return order.indexOf(keep);
    order = order.filter(index => index !== keep);
    for (let index = order.length - 1; index > 0; index--) {
      const other = Math.floor(Math.random() * (index + 1));
      [order[index], order[other]] = [order[other], order[index]];
    }
    order.unshift(keep);
    return 0;
  }

  function play(ids: string[], start: number) {
    const id = ids[start];
    const item = id ? hooks.item(id) : undefined;
    if (!item || item.missing) return;
    const index = ids.slice(0, start).filter(value => hooks.item(value) && !hooks.item(value)?.missing).length;
    persist();
    tracks = ids.filter(value => hooks.item(value) && !hooks.item(value)?.missing);
    cursor = -1;
    if (isAudiobook(item)) shuffled = false;
    activate(reorder(index), true, true);
  }

  function add(id: string) {
    if (!hooks.item(id) || hooks.item(id)?.missing) return;
    tracks.push(id);
    order.push(tracks.length - 1);
    if (cursor < 0) activate(0, false, true);
    else hooks.render();
  }

  function remove(position: number) {
    const playing = order[cursor];
    if (position === playing || !order.includes(position)) return;
    tracks.splice(position, 1);
    order = order.filter(index => index !== position).map(index => index > position ? index - 1 : index);
    cursor = order.indexOf(playing > position ? playing - 1 : playing);
    hooks.render();
  }

  function move(direction: 1 | -1, manual: boolean) {
    if (!media || cursor < 0) return;
    if (direction === -1 && media.currentTime > 3) { seek(0); return; }
    const item = current();
    const book = item ? isAudiobook(item) : false;
    if (!manual && !book && repeat === "one") { seek(0); void playMedia(); return; }
    let position = cursor + direction;
    if (!book && repeat === "all" && order.length) position = (position + order.length) % order.length;
    for (; position >= 0 && position < order.length; position += direction) {
      const item = hooks.item(tracks[order[position]]);
      if (item && !item.missing) { activate(position, true, false); return; }
    }
    updatePlaying();
  }

  function close() {
    persist();
    media?.pause();
    ++sourceRevision;
    tracks = [];
    order = [];
    cursor = -1;
    clearTimer();
    sleep = "off";
    media?.removeAttribute("src");
    media?.load();
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
    if (!host || !(element instanceof HTMLAudioElement)) return;
    media = element;
    transport = document.getElementById("audio-transport");
    const playerHost = host;
    new ResizeObserver(() => {
      document.documentElement.style.setProperty("--audio-player-height", `${playerHost.offsetHeight}px`);
    }).observe(playerHost);
    element.addEventListener("timeupdate", () => {
      updateTime();
      if (sleep === "chapter" && stopAt > 0 && element.currentTime >= stopAt || sleepUntil && Date.now() >= sleepUntil) sleepNow();
      if (!element.paused && Date.now() - lastSaved > 15_000) persist();
    });
    element.addEventListener("loadedmetadata", updateTime);
    element.addEventListener("play", () => {
      stoppedBySleep = false;
      updatePlaying();
      for (const other of document.querySelectorAll<HTMLMediaElement>("[data-local-player]")) other.pause();
    });
    element.addEventListener("pause", () => { persist(); updatePlaying(); });
    element.addEventListener("ended", () => {
      persist();
      if (sleep === "chapter") sleepNow();
      else if (!stoppedBySleep) move(1, false);
    });
    document.addEventListener("play", event => {
      if (event.target instanceof HTMLMediaElement && event.target !== element && event.target.hasAttribute("data-local-player")) element.pause();
    }, true);
  }

  function input(element: HTMLElement): boolean {
    if (element instanceof HTMLInputElement && element.id === "audio-seek") {
      seek(Number(element.value)); return true;
    }
    if (element instanceof HTMLSelectElement && element.id === "audio-rate") {
      rate = Number(element.value);
      if (media) media.playbackRate = rate;
      return true;
    }
    if (element instanceof HTMLSelectElement && element.id === "audio-sleep") {
      if (element.value === "off" || element.value === "chapter" || ["10", "20", "30", "45", "60"].includes(element.value)) setSleep(element.value);
      return true;
    }
    return false;
  }

  function click(action: string): boolean {
    switch (action) {
      case "audio-player-toggle": if (media?.paused) void playMedia(); else media?.pause(); break;
      case "audio-player-next": move(1, true); break;
      case "audio-player-previous": move(-1, true); break;
      case "audio-player-back": seek((media?.currentTime ?? 0) - 15); break;
      case "audio-player-shuffle": shuffled = !shuffled; cursor = reorder(order[cursor]); render(); hooks.render(); break;
      case "audio-player-repeat": repeat = repeat === "off" ? "all" : repeat === "all" ? "one" : "off"; render(); break;
      case "audio-player-open": hooks.open(); break;
      case "audio-player-close": close(); break;
      default: return false;
    }
    return true;
  }

  return { mount, render, play, add, remove, input, click, current, nextEntries, seek, persist,
    time: () => media?.currentTime ?? 0 };
}
