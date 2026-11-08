import { durationLabel } from "./presentation";
import { escapeHtml as h, type Copy, type JobState } from "./contracts";

const paths = {
  document: '<path d="M5 3h9l5 5v13H5V3Z"/><path d="M14 3v6h5M8 13h8M8 17h8"/>',
  file: '<path d="M5 3h9l5 5v13H5V3Z"/><path d="M14 3v6h5M8 13h8M8 17h8"/>',
  home: '<path d="m3 10 9-7 9 7M5 9v11h5v-6h4v6h5V9"/>',
  downloads: '<path d="M12 3v12m-4-4 4 4 4-4M4 17v4h16v-4"/>',
  library: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 9h18M9 9v11"/>',
  tools: '<path d="m14 5 5 5M3 21l7-7M15 3a6 6 0 0 0-5 8L3 18a2 2 0 0 0 3 3l7-7a6 6 0 0 0 8-5l-4 3-5-5 3-4Z"/>',
  settings: '<path d="m9 3-1 3-3 1-2 3 2 2v3l2 3h3l2 3 3-1 1-3 3-1 2-3-2-2V8l-2-3h-3l-2-2H9Z"/><circle cx="12" cy="12" r="3"/>',
  menu: '<path d="M4 6h16M4 12h16M4 18h16"/>',
  arrow: '<path d="M4 12h16m-6-6 6 6-6 6"/>',
  paste: '<rect x="6" y="5" width="14" height="16" rx="2"/><path d="M15 5V3H3v14h3"/>',
  add: '<path d="M12 4v16M4 12h16"/>',
  close: '<path d="m6 6 12 12M6 18 18 6"/>',
  folder: '<path d="M3 7V4h7l2 3h9v13H3V7Z"/>',
  lock: '<rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V7a4 4 0 1 1 8 0v3m-4 4v3"/>',
  check: '<path d="m4 12 5 5L20 6"/>',
  error: '<circle cx="12" cy="12" r="9"/><path d="M12 7v6m0 4v.01"/>',
  search: '<circle cx="10" cy="10" r="6"/><path d="m15 15 6 6"/>',
  retry: '<path d="M20 11a8 8 0 1 0-2 6M20 4v7h-7"/>',
  pause: '<path d="M8 4v16M16 4v16"/>',
  play: '<path d="m8 4 12 8-12 8V4Z"/>',
  previous: '<path d="M5 5v14m14-14-11 7 11 7V5Z"/>',
  next: '<path d="M19 5v14M5 5l11 7-11 7V5Z"/>',
  back: '<path d="M20 12H4m6-6-6 6 6 6"/>',
  seekBack: '<path d="M7 4 3 8l4 4M3 8h10a6 6 0 0 1 0 12h-2"/><path d="M9 12v4m-1-3 1-1m4 0c-2 0-2 4 0 4s2-4 0-4Z"/>',
  seekForward: '<path d="m17 4 4 4-4 4m4-4H11a6 6 0 0 0 0 12h2"/><path d="M9 12v4m-1-3 1-1m4 0c-2 0-2 4 0 4s2-4 0-4Z"/>',
  volume: '<path d="M3 9v6h4l5 4V5L7 9H3Zm13 0a4 4 0 0 1 0 6m3-8a7 7 0 0 1 0 10"/>',
  muted: '<path d="M3 9v6h4l5 4V5L7 9H3Zm13 1 5 5m0-5-5 5"/>',
  fullscreen: '<path d="M3 9V3h6m6 0h6v6m0 6v6h-6m-6 0H3v-6"/>',
  miniPlayer: '<rect x="3" y="5" width="18" height="14" rx="2"/><rect x="12" y="11" width="6" height="5" rx=".7"/>',
  trackWheel: '<rect x="4" y="8" width="16" height="8" rx="2"/><path d="M7 4h10M7 20h10m-9-8h.01M11 12h5"/>',
  chevronUp: '<path d="m6 15 6-6 6 6"/>',
  chevronDown: '<path d="m6 9 6 6 6-6"/>',
  fullscreenExit: '<path d="M9 3v6H3m18 0h-6V3m0 18v-6h6M3 15h6v6"/>',
  captions: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M7 11h4m2 0h4M7 15h6m2 0h2"/>',
  video: '<rect x="3" y="5" width="13" height="14" rx="2"/><path d="m16 10 5-3v10l-5-3"/>',
  audio: '<path d="M9 18V5l11-2v13M9 9l11-2"/><ellipse cx="6" cy="18" rx="3" ry="3"/><ellipse cx="17" cy="16" rx="3" ry="3"/>',
  wave: '<path d="M2 12h4l3-7 4 14 3-7h6"/>',
  heart: '<path d="M20.8 9.4c0 4.1-8.8 9.2-8.8 9.2S3.2 13.5 3.2 9.4A4.1 4.1 0 0 1 12 7.8a4.1 4.1 0 0 1 8.8 1.6Z"/>',
  image: '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8" cy="8" r="2"/><path d="m3 17 6-5 4 3 4-5 4 4"/>',
  grid: '<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>',
  list: '<path d="M9 5h12M9 12h12M9 19h12M3 5h1M3 12h1M3 19h1"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10v.01"/>',
  link: '<path d="m9 15 6-6M8 17l-1 1a4 4 0 0 1-6-6l5-5a4 4 0 0 1 6 0m0 10a4 4 0 0 0 6 0l5-5a4 4 0 0 0-6-6l-1 1"/>',
  scissors: '<circle cx="6" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="m8 8 12 13M8 16 20 3"/>',
  compress: '<path d="M8 3v5H3m5 0L2 2m14 1v5h5m-5 0 6-6M8 21v-5H3m5 0-6 6m14-1v-5h5m-5 0 6 6"/>',
  copy: '<rect x="8" y="8" width="13" height="13" rx="2"/><path d="M16 8V3H3v13h5"/>',
};
export type Icon = keyof typeof paths;
export function icon(name: Icon): string { return `<svg viewBox="0 0 24 24" fill="none" aria-hidden="true">${paths[name]}</svg>`; }
export function action(actionName: string, label: string, symbol?: Icon, style = "secondary", extra = ""): string {
  return `<button type="button" class="button ${style}" data-action="${actionName}" ${extra}>${symbol ? icon(symbol) : ""}<span>${h(label)}</span></button>`;
}
export function option(value: string, label: string, selected: string): string { return `<option value="${h(value)}" ${selected === value ? "selected" : ""}>${h(label)}</option>`; }
export function unknownDuration(seconds: number | null, t: Copy): string { return seconds === null ? t("Duración desconocida", "Duration unknown") : durationLabel(seconds); }
export function stateLabel(state: JobState, t: Copy): string {
  const labels: Record<JobState, [string, string]> = { pending: ["Pendiente", "Pending"], analyzing: ["Analizando", "Analyzing"], ready: ["Preparado", "Ready"], downloading: ["Descargando", "Downloading"], processing: ["Procesando", "Processing"], verifying: ["Verificando", "Verifying"], completed: ["Completado", "Completed"], paused: ["Pausado", "Paused"], cancelled: ["Cancelado", "Cancelled"], error: ["Error", "Error"] };
  return t(...labels[state]);
}
