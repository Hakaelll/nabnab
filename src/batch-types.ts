import type { Media } from "./contracts";
import type { ResolvedOptions, WorkOptions } from "./options";

export type BatchItemState = "pending" | "ready" | "available" | "active" | "authentication" | "unsupported";
export interface ListProgress { loaded: number; complete: boolean; name: string }
export interface DraftItem {
  id: string; url: string; title: string; provider: string; creator: string; listName?: string; contentId: string;
  thumbnail: string | null; duration: number | null; state: BatchItemState; picked: boolean; excluded: boolean;
  options: WorkOptions | null; resolved: ResolvedOptions | null; destination: string | null; error: string | null;
  list: ListProgress | null; media: Media | null;
}
export interface Draft {
  id: string; name: string; createdAt: number; updatedAt: number; savedForLater: boolean;
  presetId: string | null; options: WorkOptions | null; items: DraftItem[]; explorationLimit: number;
}

export function batchCandidates(text: string): { url: string; valid: boolean }[] {
  const seen = new Set<string>();
  return text.trim().split(/\s+/).filter(Boolean).filter(url => {
    if (seen.has(url)) return false;
    seen.add(url); return true;
  }).map(url => {
    try {
      const parsed = new URL(url);
      return { url, valid: ["http:", "https:"].includes(parsed.protocol) && !!parsed.hostname && !parsed.username && !parsed.password };
    } catch { return { url, valid: false }; }
  });
}

export function batchVisible<T extends Pick<DraftItem, "title" | "url" | "provider" | "excluded">>(items: readonly T[], query: string): T[] {
  const needle = query.trim().toLocaleLowerCase();
  return items.filter(item => !item.excluded && (!needle || `${item.title} ${item.url} ${item.provider}`.toLocaleLowerCase().includes(needle)));
}

export function batchCounts(items: readonly Pick<DraftItem, "picked" | "excluded" | "state">[]): { selected: number; queueable: number; attention: number; excluded: number } {
  const selected = items.filter(item => item.picked && !item.excluded);
  return { selected: selected.length, queueable: selected.filter(item => item.state === "ready").length, attention: selected.filter(item => item.state !== "ready").length, excluded: items.filter(item => item.excluded).length };
}
