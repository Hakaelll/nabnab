import type { LibraryItem } from "./contracts";

export const playbackRates = [0.5, 0.75, 1, 1.25, 1.5, 1.75, 2];
export type Repeat = "off" | "all" | "one";

export interface PlaybackPreferences {
  volume: number;
  muted: boolean;
  rate: number;
  repeat: Repeat;
  shuffled: boolean;
  autoplay: boolean;
  resumeMusic: boolean;
  fill: boolean;
  captionSize: "small" | "normal" | "large";
  captionContrast: boolean;
}

export interface PlaybackQueue {
  tracks: string[];
  order: number[];
  cursor: number;
  resumeAudioId?: string;
}

const key = "nabnab.playback.v1";
const defaults: PlaybackPreferences = {
  volume: 1, muted: false, rate: 1, repeat: "off", shuffled: false, autoplay: true,
  resumeMusic: false, fill: false, captionSize: "normal", captionContrast: true,
};

export function isPlayable(item: LibraryItem | undefined): item is LibraryItem & { kind: "audio" | "video" } {
  return Boolean(item && !item.missing && (item.kind === "audio" || item.kind === "video"));
}

export function audiobookChapterComplete(position: number, duration?: number | null): boolean {
  return Number.isFinite(position) && typeof duration === "number" && duration > 0 && position >= duration;
}

export function audiobookResumePosition(position: number, duration?: number | null): number {
  return Number.isFinite(position) && position > 0 && !audiobookChapterComplete(position, duration) ? position : 0;
}

export function resumePosition(position: number, duration?: number | null): number {
  if (!Number.isFinite(position) || position < 3) return 0;
  if (duration && Number.isFinite(duration)) {
    const tail = Math.max(2, Math.min(15, duration * 0.03));
    if (position >= duration - tail) return 0;
  }
  return position;
}

export function readPlayback(): { preferences: PlaybackPreferences; queue: PlaybackQueue | null } {
  const preferences = { ...defaults };
  try {
    const saved = JSON.parse(localStorage.getItem(key) ?? "null");
    if (!saved || typeof saved !== "object") return { preferences, queue: null };
    const value = saved.preferences;
    if (value && typeof value === "object") {
      if (Number.isFinite(value.volume) && value.volume >= 0 && value.volume <= 1) preferences.volume = value.volume;
      if (playbackRates.includes(value.rate)) preferences.rate = value.rate;
      if (value.repeat === "all" || value.repeat === "one") preferences.repeat = value.repeat;
      if (value.captionSize === "small" || value.captionSize === "large") preferences.captionSize = value.captionSize;
      for (const flag of ["muted", "shuffled", "autoplay", "resumeMusic", "fill", "captionContrast"] as const) {
        if (typeof value[flag] === "boolean") preferences[flag] = value[flag];
      }
    }
    const queue = saved.queue;
    if (!queue || !Array.isArray(queue.tracks) || queue.tracks.length > 2000 || !Array.isArray(queue.order)
      || queue.order.length !== queue.tracks.length || !Number.isInteger(queue.cursor)
      || queue.cursor < 0 || queue.cursor >= queue.order.length
      || !queue.tracks.every((id: unknown) => typeof id === "string" && /^[a-z0-9-]{1,100}$/i.test(id))
      || new Set(queue.order).size !== queue.order.length
      || !queue.order.every((index: number) => Number.isInteger(index) && index >= 0 && index < queue.tracks.length)) {
      return { preferences, queue: null };
    }
    const resumeAudioId = typeof queue.resumeAudioId === "string" && queue.tracks.includes(queue.resumeAudioId) ? queue.resumeAudioId : undefined;
    return { preferences, queue: { tracks: queue.tracks, order: queue.order, cursor: queue.cursor, resumeAudioId } };
  } catch { return { preferences, queue: null }; }
}

export function savePlayback(preferences: PlaybackPreferences, queue: PlaybackQueue | null): void {
  try { localStorage.setItem(key, JSON.stringify({ preferences, queue })); } catch { /* Playback remains available with restricted storage. */ }
}
