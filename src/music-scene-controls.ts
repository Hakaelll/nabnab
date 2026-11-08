import { type Copy } from "./contracts";
import { createMusicVisualizer, scenes, type VisualSettings } from "./music-visualizer";
import type { PlaybackSignal } from "./playback-signal";
import { icon, option } from "./views-common";

interface SceneSettings extends VisualSettings {
  favorites: number[];
  automatic: boolean;
  locked: boolean;
  shuffled: boolean;
  interval: number;
}

const key = "nabnab.music-scenes.v1";
const sceneIndex = (value: unknown): value is number => typeof value === "number" && Number.isInteger(value) && value >= 0 && value < scenes.length;

function read(): SceneSettings {
  const defaults: SceneSettings = { scene: 0, color: false, intensity: 0.78, quality: "auto", favorites: [], automatic: false, locked: false, shuffled: false, interval: 45 };
  try {
    const saved = JSON.parse(localStorage.getItem(key) ?? "null");
    if (!saved || typeof saved !== "object") return defaults;
    for (const flag of ["color", "automatic", "locked", "shuffled"] as const) if (typeof saved[flag] === "boolean") defaults[flag] = saved[flag];
    if (sceneIndex(saved.scene)) defaults.scene = saved.scene;
    if (saved.quality === "high" || saved.quality === "low" || saved.quality === "off") defaults.quality = saved.quality;
    if (Number.isFinite(saved.intensity)) defaults.intensity = Math.min(1, Math.max(0.15, saved.intensity));
    if (Number.isFinite(saved.interval)) defaults.interval = Math.round(Math.min(180, Math.max(15, saved.interval)));
    if (Array.isArray(saved.favorites)) defaults.favorites = [...new Set<number>(saved.favorites.filter(sceneIndex))];
  } catch { /* Local defaults remain available. */ }
  return defaults;
}

const preview = (scene: number, color: boolean) => `${import.meta.env.BASE_URL}playback/${scenes[scene][0]}${color ? "-color" : ""}.webp`;

export function createSceneControls(host: HTMLElement, t: Copy, signal: (dt: number, playing: boolean) => PlaybackSignal | undefined) {
  const surface = document.createElement("div");
  surface.className = "music-surface";
  surface.setAttribute("aria-hidden", "true");
  const still = document.createElement("img");
  still.alt = "";
  const canvas = document.createElement("canvas");
  surface.append(still, canvas);
  host.prepend(surface);
  const settings = read();
  let seconds = 0;
  let visible = false;
  let showFavorites = false;
  let graphUnavailable = false;
  let graphicsUnavailable = false;
  const label = (index: number) => t(scenes[index][1], scenes[index][2]);
  const panel = () => host.querySelector<HTMLElement>("#music-scene-controls");
  const visualizer = createMusicVisualizer(canvas, signal, () => document.documentElement.classList.contains("reduced-motion"), failed => {
    if (failed === graphicsUnavailable) return;
    graphicsUnavailable = failed;
    surface.dataset.fallback = String(failed);
    render();
  }, elapsed => {
    if (!settings.automatic || settings.locked) return;
    seconds += elapsed;
    if (seconds >= settings.interval) move(1, settings.shuffled);
  });

  function apply() {
    try { localStorage.setItem(key, JSON.stringify(settings)); } catch { /* Scene settings still apply for this session. */ }
    updateSurface();
    render();
  }

  function updateSurface() {
    still.src = preview(settings.scene, settings.color);
    surface.dataset.static = String(settings.quality === "off");
    visualizer.update(settings);
    if (visible && settings.quality !== "off") visualizer.show();
    else visualizer.hide();
  }

  function move(direction: -1 | 1, random = false) {
    const distance = random ? 1 + Math.floor(Math.random() * (scenes.length - 1)) : direction;
    settings.scene = (settings.scene + scenes.length + distance) % scenes.length;
    seconds = 0;
    apply();
  }

  function render() {
    const root = panel();
    if (!root) return;
    const focused = root.contains(document.activeElement) ? document.activeElement as HTMLElement : null;
    const position = root.scrollTop;
    const indices = showFavorites ? settings.favorites : scenes.map((_, index) => index);
    const checkbox = (id: string, text: string, checked: boolean) => `<label class="toggle"><input id="${id}" type="checkbox" ${checked ? "checked" : ""}>${text}</label>`;
    root.innerHTML = `<header class="music-panel-head"><div><h2>${t("Escenas", "Scenes")}</h2><span>${label(settings.scene)}</span></div>
      <div class="button-group"><button type="button" class="button quiet" id="scene-filter" aria-pressed="${showFavorites}">${t("Favoritas", "Favorites")}</button>
      <button type="button" id="scene-previous" class="stage-button" aria-label="${t("Escena anterior", "Previous scene")}">${icon("previous")}</button>
      <button type="button" id="scene-next" class="stage-button" aria-label="${t("Escena siguiente", "Next scene")}">${icon("next")}</button></div></header>
      ${graphUnavailable || graphicsUnavailable ? `<p class="field-note" role="status">${graphicsUnavailable ? t("Vista fija. Los gráficos no están disponibles.", "Still preview. Graphics are unavailable.") : t("La señal de audio no está disponible para la escena.", "The scene cannot access the audio signal.")}</p>` : ""}
      <div class="music-scene-grid">${indices.map(index => `<div class="music-scene-tile" data-selected="${index === settings.scene}">
        <button id="scene-pick-${index}" class="music-scene-pick" data-scene="${index}" aria-pressed="${index === settings.scene}" type="button"><img src="${preview(index, settings.color)}" alt="" decoding="async"><span>${label(index)}</span></button>
        <button id="scene-favorite-${index}" class="stage-button music-scene-favorite" data-scene-favorite="${index}" type="button" aria-pressed="${settings.favorites.includes(index)}" aria-label="${t("Favorita", "Favorite")} · ${label(index)}">${icon("heart")}</button></div>`).join("") || `<p class="field-note">${t("Aún no hay escenas favoritas.", "No favorite scenes yet.")}</p>`}</div>
      <div class="music-scene-options">
        <label for="scene-color">${t("Tratamiento", "Palette")}<select id="scene-color">${option("sober", t("Sobrio", "Muted"), settings.color ? "color" : "sober")}${option("color", "Color", settings.color ? "color" : "sober")}</select></label>
        <label for="scene-quality">${t("Calidad gráfica", "Graphics quality")}<select id="scene-quality">${option("auto", t("Automática", "Automatic"), settings.quality)}${option("high", t("Alta", "High"), settings.quality)}${option("low", t("Ahorro", "Low power"), settings.quality)}${option("off", t("Imagen fija", "Still image"), settings.quality)}</select></label>
        <label for="scene-intensity">${t("Intensidad", "Intensity")}<input id="scene-intensity" type="range" min="0.15" max="1" step="0.01" value="${settings.intensity}"></label>
        <label for="scene-interval">${t("Intervalo (s)", "Interval (s)")}<input id="scene-interval" type="number" min="15" max="180" value="${settings.interval}"></label>
      </div><div class="music-scene-rotation">
        ${checkbox("scene-auto", t("Cambio automático", "Auto change"), settings.automatic)}
        ${checkbox("scene-shuffle", t("Orden aleatorio", "Shuffle scenes"), settings.shuffled)}
        ${checkbox("scene-lock", t("Fijar escena actual", "Lock current scene"), settings.locked)}
      </div>`;
    root.scrollTop = position;
    if (focused?.id) (root.querySelector<HTMLElement>(`#${CSS.escape(focused.id)}`) ?? root.querySelector<HTMLElement>("#scene-filter"))?.focus({ preventScroll: true });
  }

  host.addEventListener("click", event => {
    if (!(event.target instanceof Element)) return;
    const control = event.target.closest<HTMLButtonElement>("#music-scene-controls button");
    if (!control) return;
    if (control.id === "scene-previous" || control.id === "scene-next") { move(control.id === "scene-next" ? 1 : -1); return; }
    if (control.id === "scene-filter") { showFavorites = !showFavorites; render(); return; }
    const pick = Number(control.dataset.scene), favorite = Number(control.dataset.sceneFavorite);
    if (sceneIndex(pick)) { settings.scene = pick; seconds = 0; }
    else if (sceneIndex(favorite)) settings.favorites = settings.favorites.includes(favorite) ? settings.favorites.filter(index => index !== favorite) : [...settings.favorites, favorite];
    else return;
    apply();
  });
  host.addEventListener("change", event => {
    const control = event.target;
    if (!(control instanceof HTMLInputElement || control instanceof HTMLSelectElement) || !panel()?.contains(control)) return;
    switch (control.id) {
      case "scene-color": settings.color = control.value === "color"; break;
      case "scene-quality": settings.quality = control.value === "high" || control.value === "low" || control.value === "off" ? control.value : "auto"; break;
      case "scene-intensity": settings.intensity = Number(control.value); break;
      case "scene-interval": settings.interval = Math.round(Math.min(180, Math.max(15, Number(control.value)))); seconds = 0; break;
      case "scene-auto": settings.automatic = control instanceof HTMLInputElement && control.checked; seconds = 0; break;
      case "scene-shuffle": settings.shuffled = control instanceof HTMLInputElement && control.checked; break;
      case "scene-lock": settings.locked = control instanceof HTMLInputElement && control.checked; seconds = 0; break;
      default: return;
    }
    apply();
  });

  return {
    show() { visible = true; updateSurface(); render(); },
    hide() { visible = false; visualizer.hide(); },
    release() { visible = false; visualizer.release(); },
    playing: visualizer.playing,
    visibility: visualizer.visibility,
    reset: visualizer.reset,
    render,
    unavailable() { graphUnavailable = true; render(); },
  };
}
