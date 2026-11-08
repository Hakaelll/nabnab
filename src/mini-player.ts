import "./mini-player.css";
import { escapeHtml as h, type Copy, type LibraryItem } from "./contracts";
import { durationLabel } from "./presentation";
import { icon } from "./views-common";

export function playerButton(id: string, label: string, symbol: Parameters<typeof icon>[0], extra = "", caption = "") {
  return `<button id="${id}" type="button" class="stage-button" aria-label="${h(label)}" title="${h(label)}" ${extra}>${icon(symbol)}${caption ? `<span>${h(caption)}</span>` : ""}</button>`;
}

export function playerArtwork(item: LibraryItem) {
  const video = item.kind === "video";
  const thumbnail = video || item.details?.audioCover || item.details?.audio?.embeddedCover;
  return `<span class="audio-cover" aria-hidden="true"${thumbnail ? ` data-thumbnail-id="${h(item.id)}"` : ""}>${icon(video ? "video" : "audio")}</span>`;
}

export function playerArtist(item: LibraryItem) {
  return item.details?.audio?.artist ?? item.details?.creator ?? "";
}

interface Hooks {
  t: Copy;
  title(item: LibraryItem): string;
  seek(time: number): void;
  toggle(): void;
  mute(): void;
  move(direction: -1 | 1): void;
  expand(): void;
  dock(): void;
  wheel(button: HTMLButtonElement): void;
  thumbnails(): void;
}

export function createMiniPlayer(host: HTMLElement, media: HTMLVideoElement, hooks: Hooks) {
  const { t } = hooks;
  const view = document.createElement("div");
  view.className = "mini-player";
  view.hidden = true;
  view.innerHTML = `
    <div class="mini-header"><div class="mini-identity"><span class="mini-art"></span><div>
      <strong id="mini-title"></strong><small id="mini-artist" hidden></small>
    </div></div>
    <div class="mini-window">
      ${playerButton("mini-expand", t("Ampliar reproductor", "Expand player"), "fullscreen")}
      ${playerButton("mini-dock", t("Volver a la barra de reproducción", "Return to playback bar"), "close")}
    </div></div>
    <div class="mini-controls">
      ${playerButton("mini-previous", t("Pista anterior", "Previous track"), "previous", 'data-mini-audio')}
      ${playerButton("mini-toggle", t("Reproducir", "Play"), "play")}
      ${playerButton("mini-next", t("Pista siguiente", "Next track"), "next", 'data-mini-audio')}
      ${playerButton("mini-mute", t("Silenciar", "Mute"), "volume")}
      ${playerButton("mini-wheel", t("Salto rápido · Alt + rueda", "Quick skip · Alt + scroll"), "trackWheel", 'data-mini-audio aria-controls="track-wheel" aria-expanded="false"', t("Salto rápido", "Quick skip"))}
    </div>
    <div class="mini-timeline">
      <span id="mini-elapsed">0:00</span>
      <input id="mini-seek" type="range" min="0" max="0" step="0.1" aria-label="${t("Posición de reproducción", "Playback position")}" disabled>
      <span id="mini-duration">0:00</span>
    </div>
    <p class="mini-issue" role="alert" hidden></p>`;
  host.append(view);

  const title = view.querySelector<HTMLElement>("#mini-title")!;
  const artist = view.querySelector<HTMLElement>("#mini-artist")!;
  const art = view.querySelector<HTMLElement>(".mini-art")!;
  const error = view.querySelector<HTMLElement>(".mini-issue")!;
  const seek = view.querySelector<HTMLInputElement>("#mini-seek")!;
  const elapsed = view.querySelector<HTMLElement>("#mini-elapsed")!;
  const end = view.querySelector<HTMLElement>("#mini-duration")!;
  const play = view.querySelector<HTMLButtonElement>("#mini-toggle")!;
  const mute = view.querySelector<HTMLButtonElement>("#mini-mute")!;
  const wheel = view.querySelector<HTMLButtonElement>("#mini-wheel")!;
  let artwork = "";

  function buttonState(button: HTMLButtonElement, label: string, symbol: Parameters<typeof icon>[0]) {
    if (button.title === label) return;
    button.title = label;
    button.setAttribute("aria-label", label);
    button.innerHTML = icon(symbol);
  }

  function update() {
    if (view.hidden) return;
    const duration = Number.isFinite(media.duration) ? media.duration : 0;
    const position = duration ? Math.min(media.currentTime, duration) : 0;
    const elapsedLabel = durationLabel(position);
    elapsed.textContent = elapsedLabel;
    end.textContent = durationLabel(duration);
    seek.max = String(duration);
    seek.value = String(position);
    seek.disabled = duration <= 0;
    seek.style.setProperty("--played", `${duration ? 100 * position / duration : 0}%`);
    seek.setAttribute("aria-valuetext", elapsedLabel);
    buttonState(play, media.paused ? t("Reproducir", "Play") : t("Pausar", "Pause"), media.paused ? "play" : "pause");
    const silent = media.muted || media.volume === 0;
    buttonState(mute, silent ? t("Activar sonido", "Unmute") : t("Silenciar", "Mute"), silent ? "muted" : "volume");
  }

  function sync(item: LibraryItem, issue: string) {
    if (view.hidden) return;
    host.dataset.mini = item.kind;
    host.setAttribute("aria-label", item.kind === "audio" ? t("Minirreproductor de audio", "Audio mini player") : t("Minirreproductor de vídeo", "Video mini player"));
    title.textContent = title.title = hooks.title(item);
    artist.textContent = playerArtist(item);
    artist.hidden = !artist.textContent;
    const nextArt = item.kind === "audio" ? playerArtwork(item) : "";
    if (artwork !== nextArt) {
      artwork = nextArt;
      art.innerHTML = artwork;
      hooks.thumbnails();
    }
    error.textContent = issue;
    error.hidden = !issue;
    update();
  }

  play.addEventListener("click", hooks.toggle);
  mute.addEventListener("click", hooks.mute);
  seek.addEventListener("input", () => hooks.seek(seek.valueAsNumber));
  wheel.addEventListener("click", () => hooks.wheel(wheel));
  view.querySelector("#mini-previous")!.addEventListener("click", () => hooks.move(-1));
  view.querySelector("#mini-next")!.addEventListener("click", () => hooks.move(1));
  view.querySelector("#mini-expand")!.addEventListener("click", hooks.expand);
  view.querySelector("#mini-dock")!.addEventListener("click", hooks.dock);
  media.addEventListener("click", () => { if (host.dataset.mini === "video") hooks.toggle(); });

  return {
    active: () => !view.hidden,
    contains: (target: Node) => view.contains(target),
    show(item: LibraryItem, issue: string) {
      host.hidden = view.hidden = false;
      sync(item, issue);
      play.focus({ preventScroll: true });
    },
    hide() {
      if (view.hidden) return;
      host.hidden = view.hidden = true;
      delete host.dataset.mini;
    },
    sync, update,
  };
}
