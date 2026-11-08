import { escapeHtml as h, type Copy, type LibraryItem } from "./contracts";
import { playerArtist, playerArtwork, playerButton } from "./mini-player";

interface Entry {
  position: number;
  item: LibraryItem;
}

interface Hooks {
  t: Copy;
  enabled(): boolean;
  entries(): Entry[];
  cursor(): number;
  title(item: LibraryItem): string;
  play(entry: Entry): void;
  thumbnails(): void;
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
    <header><strong id="track-wheel-heading">${t("Salto rápido", "Quick skip")}</strong><kbd></kbd></header>
    <div class="track-wheel-drum"></div>
    <footer>
      ${playerButton("track-wheel-previous", t("Selección anterior", "Previous selection"), "chevronUp")}
      <span class="track-wheel-position"></span>
      ${playerButton("track-wheel-next", t("Selección siguiente", "Next selection"), "chevronDown")}
    </footer><span class="track-wheel-announcement sr-only" role="status"></span>`;
  document.body.append(wheel);

  const drum = wheel.querySelector<HTMLElement>(".track-wheel-drum")!;
  const position = wheel.querySelector<HTMLElement>(".track-wheel-position")!;
  const status = wheel.querySelector<HTMLElement>(".track-wheel-announcement")!;
  const shortcut = wheel.querySelector<HTMLElement>("kbd")!;
  const previous = wheel.querySelector<HTMLButtonElement>("#track-wheel-previous")!;
  const next = wheel.querySelector<HTMLButtonElement>("#track-wheel-next")!;
  const cards = new Map<number, HTMLButtonElement>();
  let entries: Entry[] = [];
  let selected = 0;
  let source = -1;
  let opener: HTMLButtonElement | undefined;
  let usingAlt = false;
  let delta = 0;
  let lastScroll = 0;

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
        const title = hooks.title(entry.item);
        const artist = playerArtist(entry.item);
        card.setAttribute("aria-label", [title, artist].filter(Boolean).join(" · "));
        card.innerHTML = `${playerArtwork(entry.item)}<span class="track-wheel-copy"><strong>${h(title)}</strong>${artist ? `<small>${h(artist)}</small>` : ""}</span>`;
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
    status.textContent = hooks.title(entries[selected].item);
    previous.disabled = selected === 0;
    next.disabled = selected === entries.length - 1;
    if (artworkAdded) hooks.thumbnails();
  }

  function dismiss(restoreFocus = false) {
    if (wheel.hidden) return;
    wheel.hidden = true;
    wheel.classList.remove("is-open");
    drum.replaceChildren();
    cards.clear();
    entries = [];
    opener?.setAttribute("aria-expanded", "false");
    if (restoreFocus) opener?.focus({ preventScroll: true });
    opener = undefined;
    usingAlt = false;
    delta = 0;
  }

  function commit(index: number) {
    const entry = entries[index];
    dismiss(true);
    if (entry && entry.position !== source) hooks.play(entry);
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
    const { width, height } = wheel.getBoundingClientRect();
    const gap = 12;
    const preferredX = button ? x - width : x + gap + width > innerWidth ? x - width - gap : x + gap;
    const preferredY = button ? y - height - gap : y - height / 2;
    wheel.style.left = `${Math.max(gap, Math.min(innerWidth - width - gap, preferredX))}px`;
    wheel.style.top = `${Math.max(gap, Math.min(innerHeight - height - gap, preferredY))}px`;
  }

  function show(x: number, y: number, button?: HTMLButtonElement) {
    entries = hooks.entries();
    source = hooks.cursor();
    selected = entries.findIndex(entry => entry.position === source);
    if (selected < 0) return false;
    opener = button;
    usingAlt = !button;
    opener?.setAttribute("aria-expanded", "true");
    shortcut.textContent = usingAlt ? t("Suelta Alt", "Release Alt") : t("↑ ↓ · Intro", "↑ ↓ · Enter");
    wheel.hidden = false;
    render();
    place(x, y, button);
    wheel.classList.add("is-open");
    if (button) wheel.focus({ preventScroll: true });
    return true;
  }

  previous.addEventListener("click", () => move(-1));
  next.addEventListener("click", () => move(1));
  document.addEventListener("wheel", event => {
    if (!event.altKey || event.ctrlKey || event.metaKey || !event.deltaY || !hooks.enabled()) return;
    event.preventDefault();
    if (wheel.hidden) {
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
    if (!wheel.hidden && usingAlt && event.key === "Alt") commit(selected);
  });
  document.addEventListener("keydown", event => {
    if (wheel.hidden) return;
    if (event.key === "Escape") dismiss(true);
    else if (event.key === "ArrowUp" || event.key === "ArrowDown") move(event.key === "ArrowUp" ? -1 : 1);
    else if (event.key === "Enter" && !(wheel.contains(document.activeElement) && document.activeElement instanceof HTMLButtonElement)) commit(selected);
    else return;
    event.preventDefault();
    event.stopPropagation();
  }, true);
  document.addEventListener("pointerdown", event => {
    if (event.target instanceof Node && !wheel.contains(event.target) && !opener?.contains(event.target)) dismiss();
  }, true);
  window.addEventListener("blur", () => dismiss());
  window.addEventListener("resize", () => dismiss(true));

  return {
    toggle(button: HTMLButtonElement) {
      if (!wheel.hidden) { dismiss(true); return; }
      if (!hooks.enabled()) return;
      const rect = button.getBoundingClientRect();
      show(rect.right, rect.top, button);
    },
    dismiss,
  };
}
