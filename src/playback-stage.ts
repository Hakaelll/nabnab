import "./playback-stage.css";
import { escapeHtml as h, type Copy, type LibraryItem } from "./contracts";
import { durationLabel } from "./presentation";
import { createPlaybackPreview } from "./playback-preview";
import { playbackRates, type PlaybackPreferences } from "./playback-preferences";
import { createStyledCaptions } from "./styled-captions";
import { icon, option, type Icon } from "./views-common";

interface AudioTrack { label: string; language: string; enabled: boolean }
interface EmbeddedCaption { index: number; codec: "ASS" | "SSA"; title: string | null; language: string | null }
type MediaTracks = HTMLVideoElement & { audioTracks?: EventTarget & ArrayLike<AudioTrack> };
type Panel = "options" | "captions" | "scenes" | "queue" | "track" | null;
export type StageState = "loading" | "paused" | "playing" | "seeking" | "ended" | "error";

interface Hooks {
  seek(time: number): void;
  toggle(): void;
  back(): void;
  resume(fromStart: boolean): void;
  mute(): void;
  volume(value: number): void;
  sound(): { muted: boolean; volume: number };
  rate(value: number): void;
  preference(key: "fill" | "captionContrast" | "autoplay", value: boolean): void;
  captionSize(value: PlaybackPreferences["captionSize"]): void;
  fullscreen(): void;
  importCaption(language: string): Promise<void>;
  convert(id: string): void;
  external(id: string): void;
  move(direction: 1 | -1): void;
  musicTitle(item: LibraryItem): string;
  queue(): string;
  trackInfo(item: LibraryItem): string;
  sceneControls(): void;
  command<T>(name: string, args: Record<string, unknown>): Promise<T>;
  t: Copy;
}

const hideDelay = 2750;

function button(action: string, label: string, symbol?: Icon, attributes = ""): string {
  return `<button type="button" class="stage-button" data-stage-action="${action}" aria-label="${h(label)}" title="${h(label)}" ${attributes}>${symbol ? icon(symbol) : h(label)}</button>`;
}

function setButton(control: HTMLButtonElement | null, label: string, symbol: Icon) {
  if (!control || control.getAttribute("aria-label") === label) return;
  control.setAttribute("aria-label", label);
  control.title = label;
  control.innerHTML = icon(symbol);
}

function captionAt(segments: readonly { start: number; end: number; text: string }[], time: number): string {
  let left = 0;
  let right = segments.length;
  while (left < right) {
    const middle = Math.floor((left + right) / 2);
    if (segments[middle].start <= time) left = middle + 1;
    else right = middle;
  }
  const cue = segments[left - 1];
  return cue && time < cue.end ? cue.text : "";
}

export function createPlaybackStage(host: HTMLElement, media: HTMLVideoElement, hooks: Hooks) {
  const { t } = hooks;
  const controls = host.querySelector<HTMLElement>(".stage-hud")!;
  const captions = host.querySelector<HTMLElement>(".stage-captions")!;
  const feedback = host.querySelector<HTMLElement>(".stage-feedback")!;
  const preview = createPlaybackPreview(hooks.command);
  let item: LibraryItem | undefined;
  let panel: Panel = null;
  let caption = "";
  let lastCaption = "";
  let offset = 0;
  let remaining = false;
  let keyboard = false;
  let dragging = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let feedbackTimer: ReturnType<typeof setTimeout> | undefined;
  let state: StageState = "loading";
  let active = false;
  let resumeAt = 0;
  let previousFocus: HTMLElement | null = null;
  let fullscreen = false;
  let importingCaption = false;
  let importingStyled: symbol | undefined;
  let styledSelection = "";
  let styledStatus = "";
  let embeddedCaptions: EmbeddedCaption[] | undefined;
  let embeddedRequest = "";
  let embeddedStatus = "";
  let captionSize: PlaybackPreferences["captionSize"] = "normal";
  let language = "und";

  const find = <T extends HTMLElement = HTMLElement>(selector: string) => controls.querySelector<T>(selector);
  const audioTracks = () => Array.from((media as MediaTracks).audioTracks ?? []);
  const nativeCaptions = () => Array.from(media.textTracks).filter(track => track.kind === "subtitles" || track.kind === "captions");
  const localCaptions = () => item?.details?.tracks ?? [];
  const styled = createStyledCaptions(host, media, hooks.command, () => {
    setStyledStatus(t("No se pudo dibujar la pista ASS/SSA.", "The ASS/SSA track could not be rendered."));
  });

  function setStyledStatus(message: string) {
    styledStatus = message;
    const status = find("#stage-styled-status");
    if (status) { status.textContent = message; status.hidden = !message; }
  }

  function closeStyled() {
    styled.close();
    if (embeddedRequest) void hooks.command("styled_caption_close", { id: embeddedRequest }).catch(() => {});
    embeddedRequest = "";
    embeddedCaptions = undefined;
    embeddedStatus = "";
    importingStyled = undefined;
    styledStatus = "";
    if (caption === styledSelection) caption = "";
    if (lastCaption === styledSelection) lastCaption = "";
    styledSelection = "";
  }

  function embeddedLabel(track: EmbeddedCaption) {
    return [track.title, track.language === "und" ? null : track.language, track.codec].filter(Boolean).join(" · ");
  }

  function syncEmbedded() {
    const select = find<HTMLSelectElement>("#stage-caption-track");
    if (select) {
      select.querySelector("#stage-embedded-options")?.remove();
      if (embeddedCaptions?.length) {
        const group = document.createElement("optgroup");
        group.id = "stage-embedded-options";
        group.label = t("Integradas · con estilos", "Embedded · styled");
        for (const track of embeddedCaptions) {
          group.append(new Option(embeddedLabel(track), `embedded-${track.index}`));
        }
        select.append(group);
      }
      select.value = caption;
    }
    const status = find("#stage-embedded-status");
    if (status) { status.textContent = embeddedStatus; status.hidden = !embeddedStatus; }
  }

  async function inspectEmbedded() {
    if (!item?.path.toLowerCase().endsWith(".mkv") || embeddedCaptions || embeddedRequest) return;
    const id = crypto.randomUUID();
    embeddedRequest = id;
    embeddedStatus = t("Buscando pistas integradas…", "Looking for embedded tracks…");
    syncEmbedded();
    try {
      const tracks = await hooks.command<EmbeddedCaption[]>("styled_caption_tracks", { id, video: item.id });
      if (embeddedRequest !== id) return;
      embeddedCaptions = tracks;
      embeddedStatus = "";
    } catch {
      if (embeddedRequest !== id) return;
      embeddedStatus = t("No se pudieron leer las pistas integradas.", "Could not read embedded tracks.");
    } finally {
      if (embeddedRequest === id) { embeddedRequest = ""; syncEmbedded(); }
    }
  }

  async function loadStyled(track?: EmbeddedCaption) {
    if (importingStyled) return;
    const videoId = item?.id;
    if (!videoId) return;
    const request = Symbol();
    importingStyled = request;
    const control = find<HTMLButtonElement>("[data-stage-action='import-styled']");
    const picker = find<HTMLSelectElement>("#stage-caption-track");
    const keepFocus = document.activeElement === picker;
    if (control) control.disabled = true;
    if (picker) picker.disabled = true;
    setStyledStatus(t("Abriendo pista ASS/SSA…", "Opening ASS/SSA track…"));
    try {
      const loaded = await styled.load(track
        ? { kind: "embedded", video: videoId, index: track.index, title: embeddedLabel(track) }
        : { kind: "file" });
      if (!loaded || importingStyled !== request) return;
      const select = find<HTMLSelectElement>("#stage-caption-track");
      select?.querySelector('option[value="styled"]')?.remove();
      const title = styled.title();
      if (!track && title) select?.add(new Option(title, "styled"), select.querySelector("optgroup"));
      styledSelection = track ? `embedded-${track.index}` : "styled";
      selectCaption(styledSelection);
      setStyledStatus("");
    } catch {
      if (importingStyled === request) setStyledStatus(track
        ? t("No se pudo abrir esta pista integrada. Elige otra pista.", "Could not open this embedded track. Choose another track.")
        : t("No se pudo abrir la pista. Elige un archivo ASS o SSA en UTF-8.", "Could not open the track. Choose a UTF-8 ASS or SSA file."));
    } finally {
      if (importingStyled === request) {
        importingStyled = undefined;
        const next = find<HTMLButtonElement>("[data-stage-action='import-styled']");
        if (next) next.disabled = false;
        const select = find<HTMLSelectElement>("#stage-caption-track");
        if (select) { select.disabled = false; select.value = caption; }
        if (keepFocus && panel === "captions" && document.activeElement === document.body) select?.focus({ preventScroll: true });
        if (styledStatus === t("Abriendo pista ASS/SSA…", "Opening ASS/SSA track…")) setStyledStatus("");
      }
    }
  }

  function schedule() {
    clearTimeout(timer);
    if (!active || state !== "playing" || panel || dragging || keyboard && controls.contains(document.activeElement)) return;
    timer = setTimeout(() => { host.dataset.controls = "hidden"; preview.leave(); }, hideDelay);
  }

  function reveal() {
    host.dataset.controls = "visible";
    schedule();
  }

  function closePanel(restoreFocus = false) {
    const previous = panel;
    panel = null;
    for (const node of controls.querySelectorAll<HTMLElement>("[data-stage-panel]")) node.hidden = true;
    for (const node of controls.querySelectorAll("[aria-controls^='stage-panel']")) node.setAttribute("aria-expanded", "false");
    if (restoreFocus && previous) find(`[data-stage-action="${previous}"]`)?.focus({ preventScroll: true });
    reveal();
  }

  function selectCaption(value: string) {
    caption = value;
    if (value) lastCaption = value;
    for (const [index, track] of nativeCaptions().entries()) track.mode = value === `native-${index}` ? "hidden" : "disabled";
    const select = find<HTMLSelectElement>("#stage-caption-track");
    if (select) select.value = value;
    const origin = find("#stage-caption-origin");
    if (origin) {
      origin.hidden = !value;
      origin.textContent = value.startsWith("embedded-") || value.startsWith("native-")
        ? t("Integrada", "Embedded") : t("Local", "Local");
    }
    find("[data-stage-action='captions']")?.setAttribute("data-selected", String(Boolean(value)));
    paintCaption();
  }

  function paintCaption() {
    if (!active) return;
    const styledSelected = Boolean(styledSelection) && caption === styledSelection;
    styled.update(styledSelected, offset, captionSize);
    if (styledSelected) { captions.hidden = true; return; }
    const time = media.currentTime - offset;
    const local = localCaptions().find(track => track.id === caption);
    const native = nativeCaptions()[Number(caption.replace("native-", ""))];
    let text = local ? captionAt(local.segments, time) : "";
    if (caption.startsWith("native-") && native) {
      const cues = offset ? native.cues : native.activeCues;
      text = Array.from(cues ?? []).filter(cue => time >= cue.startTime && time < cue.endTime)
        .map(cue => (cue as VTTCue).text ?? "").join("\n");
    }
    const safe = text.replace(/<[^>]*>/g, "").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
    if (captions.textContent !== safe) captions.textContent = safe;
    captions.hidden = !safe;
  }

  function setFullscreen(value: boolean) {
    fullscreen = value;
    setButton(find("[data-stage-action='fullscreen']"),
      value ? t("Salir de pantalla completa", "Exit full screen") : t("Pantalla completa", "Full screen"),
      value ? "fullscreenExit" : "fullscreen");
  }

  function update(next: StageState, issue: string) {
    const changed = state !== next;
    state = next;
    if (!active) return;
    host.dataset.state = state;
    if (changed && state !== "playing") reveal();
    else if (changed) schedule();
    const total = Number.isFinite(media.duration) ? media.duration : 0;
    const time = Math.min(media.currentTime, total);
    const seek = find<HTMLInputElement>("#stage-seek");
    if (seek) {
      seek.disabled = total <= 0 || state === "error" || resumeAt > 0;
      seek.max = String(total);
      if (!dragging) seek.value = String(time);
      seek.setAttribute("aria-valuetext", durationLabel(dragging ? Number(seek.value) : time));
      let buffered = 0;
      for (let index = 0; index < media.buffered.length; index++) {
        if (media.buffered.start(index) <= time + 0.25 && media.buffered.end(index) > time) buffered = media.buffered.end(index);
      }
      seek.style.setProperty("--played", `${total ? 100 * Number(seek.value) / total : 0}%`);
      seek.style.setProperty("--buffered", `${total ? 100 * buffered / total : 0}%`);
    }
    const elapsed = find("[data-stage-elapsed]");
    const duration = find("[data-stage-action='remaining']");
    if (elapsed) elapsed.textContent = durationLabel(time);
    if (duration) duration.textContent = remaining && total ? `−${durationLabel(total - time)}` : durationLabel(total);
    const status = find(".stage-status");
    const message = state === "loading" ? t("Preparando archivo…", "Preparing media…")
      : state === "seeking" ? t("Buscando…", "Seeking…") : state === "ended" ? t("Reproducción finalizada", "Playback ended") : "";
    if (status && status.textContent !== message) status.textContent = message;
    const alert = find(".stage-error");
    if (alert) alert.hidden = !issue;
    const error = find(".stage-error p");
    if (error && error.textContent !== issue) error.textContent = issue;
    setButton(find("[data-stage-action='toggle']"), media.paused ? t("Reproducir", "Play") : t("Pausar", "Pause"), media.paused ? "play" : "pause");
    const sound = hooks.sound();
    const silent = sound.muted || sound.volume === 0;
    setButton(find("[data-stage-action='mute']"), silent ? t("Activar sonido", "Unmute") : t("Silenciar", "Mute"), silent ? "muted" : "volume");
    const volume = find<HTMLInputElement>("#stage-volume");
    if (volume && document.activeElement !== volume) volume.value = String(sound.volume);
    paintCaption();
  }

  function render(current: LibraryItem, preferences: PlaybackPreferences, progress: number, issue: string) {
    preview.leave();
    const fresh = item?.id !== current.id;
    item = current;
    const music = current.kind === "audio";
    const title = music ? hooks.musicTitle(current) : current.name.replace(/\.[^.]+$/, "");
    host.dataset.kind = music ? "music" : "video";
    resumeAt = progress;
    if (fresh) { closeStyled(); caption = ""; lastCaption = ""; offset = 0; language = "und"; panel = null; }
    const chapterOptions = (item.details?.chapters ?? []).map(chapter => option(String(chapter.start),
      `${durationLabel(chapter.start)} · ${chapter.title}`, ""));
    const captionOptions = localCaptions().map(track => option(track.id,
      `${track.language} · ${track.origin === "machineTranslation" ? t("Traducción automática", "Machine translation") : t("Local", "Local")}`, caption));
    const styledTitle = styledSelection === "styled" ? styled.title() : undefined;
    const audio = audioTracks();
    const chosen = document.activeElement instanceof HTMLElement ? document.activeElement.id : "";
    const elapsed = '<span data-stage-elapsed>0:00</span>';
    const duration = button("remaining", t("Alternar duración y tiempo restante", "Toggle duration and remaining time"));
    const volume = `<div class="stage-volume">${button("mute", t("Silenciar", "Mute"), "volume", 'id="stage-mute"')}
      <input id="stage-volume" type="range" min="0" max="1" step="0.02" aria-label="${t("Volumen", "Volume")}" value="${preferences.volume}"></div>`;
    const toggle = button("toggle", t("Reproducir", "Play"), "play", 'id="stage-toggle"');
    const fullscreenButton = button("fullscreen", t("Pantalla completa", "Full screen"), "fullscreen", 'id="stage-fullscreen"');
    controls.innerHTML = `
      <header class="stage-top">
        ${button("back", t("Atrás", "Back"), "back", 'id="stage-back"')}
        ${music ? `<div class="music-identity"><span class="audio-cover" aria-hidden="true"${current.details?.audioCover || current.details?.audio?.embeddedCover ? ` data-thumbnail-id="${h(current.id)}"` : ""}>${icon("audio")}</span><div>
          <h1 title="${h(title)}">${h(title)}</h1>${current.details?.audio?.artist ? `<p>${h(current.details.audio.artist)}</p>` : ""}</div></div>` : `<h1 title="${h(current.name)}">${h(title)}</h1>`}
        <div class="stage-top-actions">${music ? button("track", t("Información de la pista", "Track information"), "info", 'id="music-track" aria-expanded="false" aria-controls="stage-panel-track"') : ""}
        ${button("options", t("Opciones de reproducción", "Playback options"), "settings", 'id="stage-options" aria-expanded="false" aria-controls="stage-panel-options"')}</div>
      </header>
      ${music ? `<section class="stage-panel stage-options music-track" id="stage-panel-track" data-stage-panel="track" aria-label="${t("Información de la pista", "Track information")}" hidden></section>
        <section class="stage-panel music-queue" id="stage-panel-queue" data-stage-panel="queue" aria-label="${t("Cola de reproducción", "Playback queue")}" hidden></section>
        <section class="stage-panel music-scenes" id="stage-panel-scenes" data-stage-panel="scenes" aria-label="${t("Escenas", "Scenes")}" hidden><div id="music-scene-controls"></div></section>` : ""}
      <div class="stage-panel stage-options" id="stage-panel-options" data-stage-panel="options" hidden>
        <label for="stage-rate">${t("Velocidad", "Speed")}<select id="stage-rate">${playbackRates.map(rate => option(String(rate), `${rate}×`, String(preferences.rate))).join("")}</select></label>
        ${music ? `<label class="toggle"><input type="checkbox" id="audio-resume-music" ${preferences.resumeMusic ? "checked" : ""}>${t("Reanudar pistas de música", "Resume music tracks")}</label>` : `<label for="stage-fit">${t("Imagen", "Picture")}<select id="stage-fit">${option("contain", t("Ajustar", "Fit"), preferences.fill ? "cover" : "contain")}${option("cover", t("Rellenar · recorta la imagen", "Fill · crops the image"), preferences.fill ? "cover" : "contain")}</select></label>`}
        ${audio.length > 1 ? `<label for="stage-audio-track">${t("Pista de audio", "Audio track")}<select id="stage-audio-track">${audio.map((track, index) => option(String(index), [track.label, track.language].filter(Boolean).join(" · ") || `${t("Pista", "Track")} ${index + 1}`, String(audio.findIndex(track => track.enabled)))).join("")}</select></label>` : ""}
        ${chapterOptions.length ? `<label for="stage-chapter">${t("Capítulos", "Chapters")}<select id="stage-chapter">${option("", t("Ir a un capítulo", "Go to a chapter"), "")}${chapterOptions.join("")}</select></label>` : ""}
        <label class="toggle"><input type="checkbox" id="stage-autoplay" ${preferences.autoplay ? "checked" : ""}>${t("Reproducir el siguiente archivo", "Play next file")}</label>
      </div>
      ${progress ? `<section class="stage-prompt" aria-label="${t("Posición guardada", "Saved position")}">
        <strong>${t("Posición guardada", "Saved position")} · ${durationLabel(progress)}</strong><div class="button-group">
        ${button("resume", t("Continuar", "Continue"), undefined, 'id="stage-resume"')}
        ${button("restart", t("Desde el principio", "From the beginning"))}</div></section>` : ""}
      <section class="stage-error" role="alert" hidden><p></p><div class="button-group">
        ${button("convert", t("Convertir una copia", "Convert a copy"))}
        ${button("external", t("Abrir con el sistema", "Open with system"))}</div></section>
      <footer class="stage-bottom">
        <div class="stage-status" role="status"></div>
        <div class="stage-timeline">${music ? elapsed : ""}
          <div class="stage-scrub"><output id="stage-hover" aria-hidden="true" hidden><img alt="" hidden><span></span></output><input id="stage-seek" type="range" min="0" max="0" step="0.01" value="0" aria-label="${t("Posición de reproducción", "Playback position")}" disabled></div>
          ${music ? duration : ""}
        </div>
        <div class="stage-toolbar">
          ${music ? `${volume}<div class="music-transport">
            ${button("previous", t("Pista anterior", "Previous track"), "previous")}${toggle}${button("next", t("Pista siguiente", "Next track"), "next")}
          </div><div class="music-tools">
            <button type="button" class="stage-button music-scenes-trigger" data-stage-action="scenes" id="music-scenes" aria-expanded="false" aria-controls="stage-panel-scenes" aria-label="${t("Escenas", "Scenes")}" title="${t("Escenas", "Scenes")}">${icon("wave")}<span>${t("Escenas", "Scenes")}</span></button>
            ${button("queue", t("Cola de reproducción", "Playback queue"), "list", 'id="music-queue" aria-expanded="false" aria-controls="stage-panel-queue"')}
            ${button("immerse", t("Ocultar controles", "Hide controls"), "image")}${fullscreenButton}
          </div>` : `${toggle}
            ${button("back-ten", t("Retroceder 10 segundos", "Back 10 seconds"), "seekBack")}
            ${button("forward-ten", t("Avanzar 10 segundos", "Forward 10 seconds"), "seekForward")}
            ${volume}<div class="stage-clock">${elapsed}<span aria-hidden="true">/</span>${duration}</div>
            <span class="stage-spacer"></span>
            ${button("captions", t("Subtítulos", "Subtitles"), "captions", 'id="stage-caption-button" aria-expanded="false" aria-controls="stage-panel-captions"')}
            ${fullscreenButton}`}
        </div>
        ${music ? "" : `<div class="stage-panel stage-caption-panel" id="stage-panel-captions" data-stage-panel="captions" role="group" aria-label="${t("Subtítulos", "Subtitles")}" hidden>
          <div class="stage-caption-heading"><span>${icon("captions")}</span><strong>${t("Subtítulos", "Subtitles")}</strong>
            <small id="stage-caption-origin" hidden></small></div>
          <label for="stage-caption-track">${t("Pista", "Track")}<select id="stage-caption-track" ${importingStyled ? "disabled" : ""}>${option("", t("Desactivados", "Off"), caption)}
            ${captionOptions.join("")}${styledTitle ? option("styled", styledTitle, caption) : ""}${nativeCaptions().map((track, index) => option(`native-${index}`, [track.label, track.language].filter(Boolean).join(" · ") || `${t("Integrada", "Embedded")} ${index + 1}`, caption)).join("")}</select></label>
          <p class="stage-caption-status" id="stage-embedded-status" role="status" hidden></p>
          <p class="stage-caption-status" id="stage-styled-status" role="status" ${styledStatus ? "" : "hidden"}>${h(styledStatus)}</p>
          <div class="stage-caption-grid"><label for="stage-caption-size">${t("Tamaño", "Size")}<select id="stage-caption-size">${["small", "normal", "large"].map((size, index) => option(size,
            [t("Pequeño", "Small"), "Normal", t("Grande", "Large")][index], preferences.captionSize)).join("")}</select></label>
          <label for="stage-caption-offset">${t("Desfase (s)", "Offset (s)")}<input id="stage-caption-offset" type="number" step="0.25" min="-120" max="120" value="${offset}"></label></div>
          <label class="stage-caption-contrast"><input id="stage-caption-contrast" type="checkbox" ${preferences.captionContrast ? "checked" : ""}>${t("Contraste reforzado", "Enhanced contrast")}</label>
          <div class="stage-caption-files"><strong>${t("Añadir desde el equipo", "Add from this device")}</strong>
            <div class="stage-caption-import"><label for="stage-caption-language">${t("Idioma SRT / VTT", "SRT / VTT language")}<input id="stage-caption-language" value="${h(language)}" maxlength="24"></label>
              ${button("import-caption", t("Cargar SRT o VTT", "Load SRT or VTT"), undefined, importingCaption ? "disabled" : "")}</div>
            ${button("import-styled", t("Cargar ASS o SSA con estilos", "Load ASS or SSA with styling"), undefined, importingStyled ? "disabled" : "")}
          </div>
        </div>`}
      </footer>`;
    host.dataset.fit = preferences.fill ? "cover" : "contain";
    captionSize = preferences.captionSize;
    host.dataset.captionSize = preferences.captionSize;
    host.dataset.captionContrast = String(preferences.captionContrast);
    syncEmbedded();
    selectCaption(caption);
    if (music) { syncMusic(); hooks.sceneControls(); }
    setFullscreen(fullscreen);
    if (panel) showPanel(panel, false);
    if (chosen) find(`#${CSS.escape(chosen)}`)?.focus({ preventScroll: true });
    update(state, issue);
  }

  function showPanel(next: Exclude<Panel, null>, focus: boolean) {
    panel = next;
    for (const node of controls.querySelectorAll<HTMLElement>("[data-stage-panel]")) node.hidden = node.dataset.stagePanel !== panel;
    for (const node of controls.querySelectorAll<HTMLElement>("[aria-controls^='stage-panel']")) node.setAttribute("aria-expanded", String(node.dataset.stageAction === panel));
    if (focus) find(`[data-stage-panel="${panel}"] :is(select, button:not(:disabled), input:not(:disabled))`)?.focus({ preventScroll: true });
    if (next === "captions") void inspectEmbedded();
    reveal();
  }

  function show(current: LibraryItem, preferences: PlaybackPreferences, progress: number, issue: string) {
    if (!active) previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const opening = !active;
    active = true;
    host.hidden = false;
    host.setAttribute("aria-label", current.kind === "audio" ? t("Reproductor de música", "Music player") : t("Reproductor de vídeo", "Video player"));
    document.documentElement.classList.add("has-playback-stage");
    for (const node of document.querySelectorAll<HTMLElement>(".app-shell, .titlebar")) node.inert = true;
    render(current, preferences, progress, issue);
    reveal();
    if (opening) host.focus({ preventScroll: true });
  }

  function hide() {
    if (!active) return;
    active = false;
    clearTimeout(timer);
    clearTimeout(feedbackTimer);
    preview.leave();
    closeStyled();
    feedback.hidden = true;
    panel = null; keyboard = false; dragging = false;
    host.hidden = true;
    document.documentElement.classList.remove("has-playback-stage");
    for (const node of document.querySelectorAll<HTMLElement>(".app-shell, .titlebar")) node.inert = false;
    if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
  }

  function toggleOnSurface() {
    if (resumeAt || state === "error") return;
    const playing = media.paused;
    hooks.toggle();
    feedback.innerHTML = icon(playing ? "play" : "pause");
    feedback.hidden = false;
    clearTimeout(feedbackTimer);
    feedbackTimer = setTimeout(() => { feedback.hidden = true; }, 420);
    reveal();
  }

  function toggleCaptions() {
    selectCaption(caption ? "" : lastCaption || localCaptions()[0]?.id || styledSelection || (nativeCaptions().length ? "native-0" : ""));
  }

  function syncMusic() {
    if (!active || item?.kind !== "audio") return;
    for (const [selector, markup] of [["#stage-panel-queue", hooks.queue()], ["#stage-panel-track", hooks.trackInfo(item)]] as const) {
      const target = find(selector);
      if (!target) continue;
      const focus = target.contains(document.activeElement) && document.activeElement instanceof HTMLElement ? document.activeElement.id : "";
      const scroll = target.scrollTop;
      target.innerHTML = markup;
      target.scrollTop = scroll;
      if (focus) target.querySelector<HTMLElement>(`#${CSS.escape(focus)}`)?.focus({ preventScroll: true });
    }
  }

  function key(event: KeyboardEvent): boolean {
    if (!active) return false;
    keyboard = true;
    reveal();
    if (event.key === "Escape") {
      if (panel) closePanel(true);
      else if (fullscreen) hooks.fullscreen();
      else hooks.back();
      return true;
    }
    if (event.target instanceof HTMLElement && event.target.closest("input, select, textarea, [contenteditable='true']")) return false;
    const lower = event.key.toLowerCase();
    if (event.altKey || event.metaKey || event.shiftKey) return false;
    if (event.ctrlKey) {
      if (lower === "arrowleft" || lower === "arrowright") { hooks.move(lower === "arrowleft" ? -1 : 1); return true; }
      return false;
    }
    if (lower === " " && event.target instanceof HTMLButtonElement) return false;
    switch (lower) {
      case " ": case "k": toggleOnSurface(); break;
      case "arrowleft": case "j": hooks.seek(media.currentTime - (lower === "j" ? 10 : 5)); break;
      case "arrowright": case "l": hooks.seek(media.currentTime + (lower === "l" ? 10 : 5)); break;
      case "m": hooks.mute(); break;
      case "f": hooks.fullscreen(); break;
      case "c": if (item?.kind !== "video") return false; toggleCaptions(); break;
      default: return false;
    }
    return true;
  }

  host.addEventListener("pointermove", event => {
    if (event.movementX || event.movementY || host.dataset.controls === "hidden") reveal();
    const seek = find<HTMLInputElement>("#stage-seek");
    const hover = find("#stage-hover");
    if (!seek || !hover) return;
    hover.hidden = event.target !== seek || !Number.isFinite(media.duration) || seek.disabled;
    if (!hover.hidden) {
      const bounds = seek.getBoundingClientRect();
      const ratio = Math.max(0, Math.min(1, (event.clientX - bounds.left) / bounds.width));
      const time = Math.min(ratio * media.duration, media.duration - 0.01);
      const label = hover.querySelector("span");
      const image = hover.querySelector("img");
      if (label) label.textContent = durationLabel(time);
      const margin = Math.min(83, bounds.width / 2);
      hover.style.left = `${Math.min(bounds.width - margin, Math.max(margin, ratio * bounds.width))}px`;
      if (image && item?.kind === "video" && !dragging) preview.hover(image, item.id, time);
    } else preview.leave();
  });
  host.addEventListener("pointerleave", () => {
    const hover = find("#stage-hover");
    if (hover) hover.hidden = true;
    preview.leave();
  });
  host.addEventListener("pointerdown", event => {
    keyboard = false;
    dragging = event.target instanceof HTMLInputElement && event.target.id === "stage-seek";
    reveal();
  });
  const release = () => { if (dragging) { dragging = false; reveal(); } };
  window.addEventListener("pointerup", release);
  window.addEventListener("pointercancel", release);
  host.addEventListener("focusin", reveal);
  host.addEventListener("focusout", () => queueMicrotask(schedule));
  host.addEventListener("click", event => {
    const target = event.target;
    if (!(target instanceof Element)) return;
    const action = target.closest<HTMLButtonElement>("[data-stage-action]")?.dataset.stageAction;
    if (!action) {
      if (target === media || target === host || target.closest(".music-surface")) { if (panel) closePanel(); else toggleOnSurface(); }
      return;
    }
    switch (action) {
      case "back": hooks.back(); break;
      case "toggle": if (resumeAt) hooks.resume(false); else hooks.toggle(); break;
      case "back-ten": hooks.seek(media.currentTime - 10); break;
      case "forward-ten": hooks.seek(media.currentTime + 10); break;
      case "previous": case "next": hooks.move(action === "previous" ? -1 : 1); break;
      case "mute": hooks.mute(); break;
      case "fullscreen": hooks.fullscreen(); break;
      case "remaining": remaining = !remaining; update(state, find(".stage-error p")?.textContent ?? ""); break;
      case "options": case "captions": case "scenes": case "queue": case "track":
        if (panel === action) closePanel(); else showPanel(action, keyboard); break;
      case "immerse":
        closePanel(); clearTimeout(timer); host.focus({ preventScroll: true }); host.dataset.controls = "hidden"; return;
      case "resume": hooks.resume(false); break;
      case "restart": hooks.resume(true); break;
      case "convert": if (item) hooks.convert(item.id); break;
      case "external": if (item) hooks.external(item.id); break;
      case "import-caption":
        if (!importingCaption) {
          importingCaption = true;
          void hooks.importCaption(language).finally(() => { importingCaption = false; });
        }
        break;
      case "import-styled": void loadStyled(); break;
    }
    reveal();
  });
  host.addEventListener("input", event => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement)) return;
    if (target.id === "stage-seek") hooks.seek(Number(target.value));
    if (target.id === "stage-volume") hooks.volume(Number(target.value));
    if (target.id === "stage-caption-language") language = target.value;
  });
  host.addEventListener("change", event => {
    const target = event.target;
    if (!(target instanceof HTMLInputElement || target instanceof HTMLSelectElement)) return;
    switch (target.id) {
      case "stage-rate": hooks.rate(Number(target.value)); break;
      case "stage-fit": hooks.preference("fill", target.value === "cover"); host.dataset.fit = target.value; paintCaption(); break;
      case "stage-caption-track": {
        const track = embeddedCaptions?.find(candidate => target.value === `embedded-${candidate.index}`);
        if (track && target.value !== styledSelection) void loadStyled(track);
        else selectCaption(target.value);
        break;
      }
      case "stage-caption-size":
        if (target.value === "small" || target.value === "normal" || target.value === "large") {
          captionSize = target.value; hooks.captionSize(captionSize); host.dataset.captionSize = captionSize; paintCaption();
        }
        break;
      case "stage-caption-offset":
        if (!(target instanceof HTMLInputElement)) break;
        offset = Number.isFinite(target.valueAsNumber) ? Math.max(-120, Math.min(120, target.valueAsNumber)) : 0;
        target.value = String(offset); paintCaption(); break;
      case "stage-caption-contrast":
        hooks.preference("captionContrast", target instanceof HTMLInputElement && target.checked);
        host.dataset.captionContrast = String(target instanceof HTMLInputElement && target.checked); break;
      case "stage-audio-track": audioTracks().forEach((track, index) => { track.enabled = index === Number(target.value); }); break;
      case "stage-chapter": if (target.value) { hooks.seek(Number(target.value)); closePanel(); } break;
      case "stage-autoplay": hooks.preference("autoplay", target instanceof HTMLInputElement && target.checked); break;
    }
  });

  return { show, render, hide, update, key, setFullscreen, selectCaption, syncMusic, clearPreview: preview.clear };
}
