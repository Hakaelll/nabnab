import { escapeHtml as h, type Copy } from "./contracts";
import { playerButton } from "./mini-player";
import { icon } from "./views-common";

export interface WheelEntry {
  position: number;
  id: string;
  title: string;
  artist: string;
  artwork: boolean;
}

export interface WheelSession {
  entries: WheelEntry[];
  cursor: number;
  playing: boolean;
}

export type WheelAction =
  | { type: "toggle"; source: WheelEntry }
  | { type: "play"; source: WheelEntry; target: WheelEntry };

interface Hooks {
  t: Copy;
  enabled(): boolean;
  entries(): WheelEntry[];
  cursor(): number;
  play(entry: WheelEntry): void;
  toggle(): void;
  playing(): boolean;
  thumbnails(): void;
  compact?(): boolean;
  closed?(): void;
}

export function createTrackWheel(hooks: Hooks) {
  const { t } = hooks;
  const wheel = document.createElement("section");
  wheel.id = "track-wheel";
  wheel.className = "track-wheel";
  wheel.tabIndex = -1;
  wheel.hidden = true;
  wheel.setAttribute("aria-labelledby", "track-wheel-heading");
  wheel.innerHTML = `
    <div class="track-wheel-drum"></div>
    <aside class="track-wheel-tools">
      <span id="track-wheel-heading">${icon("audio")}${t("Salto rápido", "Quick skip")}</span>
      <div class="track-wheel-transport">
        ${playerButton("track-wheel-previous", t("Selección anterior", "Previous selection"), "chevronUp")}
        ${playerButton("track-wheel-toggle", t("Pausar", "Pause"), "pause")}
        ${playerButton("track-wheel-next", t("Selección siguiente", "Next selection"), "chevronDown")}
      </div>
      <div class="track-wheel-hint"><span class="track-wheel-position"></span><kbd></kbd></div>
      <button type="button" class="track-wheel-back" hidden></button>
    </aside>${playerButton("track-wheel-close", t("Cerrar salto rápido", "Close Quick Skip"), "close")}
    <span class="track-wheel-announcement sr-only" role="status"></span>`;
  document.body.append(wheel);

  const drum = wheel.querySelector<HTMLElement>(".track-wheel-drum")!;
  const position = wheel.querySelector<HTMLElement>(".track-wheel-position")!;
  const status = wheel.querySelector<HTMLElement>(".track-wheel-announcement")!;
  const shortcut = wheel.querySelector<HTMLElement>("kbd")!;
  const previous = wheel.querySelector<HTMLButtonElement>("#track-wheel-previous")!;
  const next = wheel.querySelector<HTMLButtonElement>("#track-wheel-next")!;
  const playPause = wheel.querySelector<HTMLButtonElement>("#track-wheel-toggle")!;
  const close = wheel.querySelector<HTMLButtonElement>("#track-wheel-close")!;
  const back = wheel.querySelector<HTMLButtonElement>(".track-wheel-back")!;
  const cards = new Map<number, HTMLButtonElement>();
  let entries: WheelEntry[] = [];
  let selected = 0;
  let source = -1;
  let opener: HTMLButtonElement | undefined;
  let usingAlt = false;
  let delta = 0;
  let lastScroll = 0;
  let opened = false;
  let motion: Animation | undefined;
  let offsetX = 0;
  let offsetY = 0;
  let returnTo: { from: WheelEntry; to: WheelEntry } | undefined;

  function animate(opening: boolean) {
    const reduced = document.documentElement.classList.contains("reduced-motion") || matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (reduced || !usingAlt && opening) return;
    const current = getComputedStyle(wheel);
    const full = { opacity: 1, transform: "none" };
    const folded = { opacity: 0, transform: `translate(${offsetX}px, ${offsetY}px) scale(.9)` };
    const initial = motion ? { opacity: current.opacity, transform: current.transform } : opening ? folded : full;
    motion?.cancel();
    motion = wheel.animate([initial, opening ? full : folded], {
      duration: opening ? 180 : 125, easing: "cubic-bezier(.23,1,.32,1)", fill: "both",
    });
    return motion;
  }

  function update() {
    if (!opened) return;
    const playing = hooks.playing();
    const label = playing ? t("Pausar", "Pause") : t("Reproducir", "Play");
    if (playPause.title === label) return;
    playPause.title = label;
    playPause.setAttribute("aria-label", label);
    playPause.innerHTML = icon(playing ? "pause" : "play");
  }

  function render() {
    const start = Math.max(0, selected - 3);
    const end = Math.min(entries.length, selected + 4);
    let artworkAdded = false;
    for (const [index, card] of cards) {
      if (index >= start && index < end) continue;
      if (card === document.activeElement) wheel.focus({ preventScroll: true });
      card.remove();
      cards.delete(index);
    }
    for (let index = start; index < end; index++) {
      const entry = entries[index];
      let card = cards.get(index);
      if (!card) {
        card = document.createElement("button");
        card.type = "button";
        card.className = "track-wheel-card";
        card.dataset.position = String(entry.position);
        const { title, artist } = entry;
        card.setAttribute("aria-label", [title, artist].filter(Boolean).join(" · "));
        card.title = title;
        card.innerHTML = `<span class="audio-cover" aria-hidden="true"${entry.artwork ? ` data-thumbnail-id="${h(entry.id)}"` : ""}>${icon("record")}</span><span class="track-wheel-copy"><strong>${h(title)}</strong>${artist ? `<small>${h(artist)}</small>` : ""}</span>`;
        card.addEventListener("click", () => commit(index));
        const following = [...cards.keys()].filter(key => key > index).sort((a, b) => a - b)[0];
        drum.insertBefore(card, cards.get(following) ?? null);
        cards.set(index, card);
        artworkAdded = true;
      }
      const offset = index - selected;
      card.dataset.offset = String(offset);
      card.style.zIndex = String(4 - Math.abs(offset));
      card.tabIndex = offset === 0 ? 0 : -1;
      card.setAttribute("aria-pressed", String(offset === 0));
      card.setAttribute("aria-hidden", String(Math.abs(offset) > 2));
      if (entry.position === source) card.setAttribute("aria-current", "true");
    }
    position.textContent = `${selected + 1} / ${entries.length}`;
    status.textContent = `${entries[selected].title} · ${t(`${selected + 1} de ${entries.length}`, `${selected + 1} of ${entries.length}`)}`;
    previous.disabled = selected === 0;
    next.disabled = selected === entries.length - 1;
    update();
    if (artworkAdded) hooks.thumbnails();
  }

  function dismiss(restoreFocus = false) {
    if (!opened) return;
    opened = false;
    wheel.inert = true;
    wheel.classList.remove("is-open");
    const exit = usingAlt ? animate(false) : undefined;
    opener?.setAttribute("aria-expanded", "false");
    if (restoreFocus) opener?.focus({ preventScroll: true });
    opener = undefined;
    usingAlt = false;
    delta = 0;
    const finish = () => {
      if (opened) return;
      wheel.hidden = true;
      motion?.cancel();
      motion = undefined;
      drum.replaceChildren();
      cards.clear();
      entries = [];
      hooks.closed?.();
    };
    if (exit) void exit.finished.then(finish, () => {});
    else finish();
  }

  function commit(index: number) {
    if (!opened) return;
    const entry = entries[index];
    const from = entries.find(candidate => candidate.position === source);
    if (entry && from && entry.position !== source) {
      returnTo = { from, to: entry };
      hooks.play(entry);
    }
    dismiss(true);
  }

  function move(direction: number) {
    const index = Math.max(0, Math.min(entries.length - 1, selected + direction));
    if (index === selected) return;
    const focus = drum.contains(document.activeElement);
    selected = index;
    render();
    if (focus) cards.get(selected)?.focus({ preventScroll: true });
  }

  function place(x: number, y: number, button?: HTMLButtonElement) {
    if (wheel.classList.contains("is-compact")) {
      wheel.dataset.side = "left";
      wheel.style.left = wheel.style.top = "0px";
      wheel.style.transformOrigin = "center";
      offsetX = offsetY = 0;
      return;
    }
    const { width, height } = wheel.getBoundingClientRect();
    const margin = 8;
    const gap = 26;
    const leftSide = x + gap + width > innerWidth - margin;
    wheel.dataset.side = leftSide ? "left" : "right";
    const preferredX = leftSide ? x - width - gap : x + gap;
    const preferredY = y - height * (button ? .66 : .57);
    const left = Math.max(margin, Math.min(innerWidth - width - margin, preferredX));
    const top = Math.max(margin, Math.min(innerHeight - height - margin, preferredY));
    wheel.style.left = `${left}px`;
    wheel.style.top = `${top}px`;
    const center = drum.offsetLeft + drum.offsetWidth / 2;
    wheel.style.transformOrigin = `${x - left}px ${y - top}px`;
    offsetX = Math.max(-32, Math.min(32, x - left - center));
    offsetY = Math.max(-26, Math.min(26, y - top - height / 2));
  }

  function show(x: number, y: number, button?: HTMLButtonElement, steps = 0) {
    entries = hooks.entries();
    source = hooks.cursor();
    selected = entries.findIndex(entry => entry.position === source);
    if (selected < 0) return false;
    opener = button;
    usingAlt = !button;
    opened = true;
    wheel.classList.toggle("is-compact", hooks.compact?.() === true);
    drum.replaceChildren();
    cards.clear();
    opener?.setAttribute("aria-expanded", "true");
    wheel.querySelector("#track-wheel-heading")!.innerHTML = `${icon("audio")}${t("Salto rápido", "Quick skip")}`;
    for (const [control, label] of [
      [previous, t("Selección anterior", "Previous selection")],
      [next, t("Selección siguiente", "Next selection")],
      [close, t("Cerrar salto rápido", "Close Quick Skip")],
    ] as const) {
      control.title = label;
      control.setAttribute("aria-label", label);
    }
    shortcut.textContent = usingAlt ? t("Suelta Alt", "Release Alt") : t("↑ ↓ · Intro", "↑ ↓ · Enter");
    const old = returnTo;
    const canReturn = old && entries[selected].id === old.to.id
      && entries.some(entry => entry.position === old.from.position && entry.id === old.from.id);
    back.hidden = !canReturn;
    if (canReturn) {
      back.textContent = t("Volver a la pista", "Back to track");
      back.title = old.from.title;
      back.setAttribute("aria-label", `${back.textContent} · ${back.title}`);
    }
    motion?.cancel();
    motion = undefined;
    wheel.hidden = false;
    wheel.inert = false;
    selected = Math.max(0, Math.min(entries.length - 1, selected + steps));
    render();
    place(x, y, button);
    wheel.classList.add("is-open");
    const entrance = animate(true);
    if (entrance) void entrance.finished.then(() => {
      if (motion === entrance) { entrance.cancel(); motion = undefined; }
    }, () => {});
    if (button) wheel.focus({ preventScroll: true });
    return true;
  }

  previous.addEventListener("click", () => move(-1));
  next.addEventListener("click", () => move(1));
  playPause.addEventListener("click", hooks.toggle);
  close.addEventListener("click", () => dismiss(true));
  back.addEventListener("click", () => {
    const target = returnTo?.from;
    if (target) commit(entries.findIndex(entry => entry.position === target.position && entry.id === target.id));
  });
  document.addEventListener("wheel", event => {
    if (!event.altKey || event.ctrlKey || event.metaKey || !event.deltaY || !hooks.enabled()) return;
    event.preventDefault();
    if (!opened) {
      if (!show(event.clientX, event.clientY)) return;
      move(Math.sign(event.deltaY));
      lastScroll = event.timeStamp;
      return;
    }
    if (event.timeStamp - lastScroll > 150 || Math.sign(delta) !== Math.sign(event.deltaY)) delta = 0;
    lastScroll = event.timeStamp;
    delta += event.deltaMode ? Math.sign(event.deltaY) * 48 : event.deltaY;
    if (Math.abs(delta) >= 48) {
      move(Math.sign(delta));
      delta = 0;
    }
  }, { capture: true, passive: false });
  window.addEventListener("keyup", event => {
    if (opened && usingAlt && event.key === "Alt") commit(selected);
  });
  document.addEventListener("keydown", event => {
    if (!opened) return;
    if (event.key === "Escape") dismiss(true);
    else if (event.key === "ArrowUp" || event.key === "ArrowDown") move(event.key === "ArrowUp" ? -1 : 1);
    else if (event.key === " " && !(wheel.contains(document.activeElement) && document.activeElement instanceof HTMLButtonElement)) hooks.toggle();
    else if (event.key === "Enter" && !(wheel.contains(document.activeElement) && document.activeElement instanceof HTMLButtonElement)) commit(selected);
    else return;
    event.preventDefault();
    event.stopPropagation();
  }, true);
  document.addEventListener("pointerdown", event => {
    if (event.target instanceof Node && !wheel.contains(event.target) && !opener?.contains(event.target)) dismiss();
  }, true);
  if (!hooks.closed) {
    window.addEventListener("blur", () => dismiss());
    window.addEventListener("resize", () => dismiss(true));
  }

  return {
    toggle(button: HTMLButtonElement) {
      if (opened) { dismiss(true); return; }
      if (!hooks.enabled()) return;
      const rect = button.getBoundingClientRect();
      show(rect.right, rect.top, button);
    },
    show, move, commit: () => commit(selected), dismiss, update,
  };
}
