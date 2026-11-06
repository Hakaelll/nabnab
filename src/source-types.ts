export type SourceStatus = "ready" | "checking" | "paused" | "requiresSession" | "error";
export type SourceItemState = "observed" | "review" | "queued" | "downloaded" | "ignored" | "unavailable";
export interface SourceSettings {
  presetId: string | null;
  relativeFolder: string;
  intervalHours: 1 | 6 | 24 | null;
  automatic: boolean;
  automaticLimit: number;
  explorationLimit: number;
  pageSize: number;
  history: "fromNow" | "reviewHistory";
}
export interface SourceItem {
  contentId: string; title: string; creator: string; url: string; urlRef?: string | null; size?: number | null; mediaType?: string; thumbnail: string | null; duration: number | null;
  publishedAt: number | null; discoveredAt: number; state: SourceItemState;
  discovery: "history" | "published" | "added" | "ambiguous";
  reason: string | null; jobIds: string[];
}
export interface FollowedSource {
  id: string; name: string; url: string; kind: "channel" | "playlist" | "podcast"; urlRef?: string | null; publicFeedUrl?: string | null; status: SourceStatus;
  createdAt: number; lastCheck: number | null; nextCheck: number | null; error: string | null; failures: number;
  settings: SourceSettings;
  baseline: { complete: boolean; inspected: number; total: number | null; cursor: number; hasMore: boolean };
  items: SourceItem[];
}
export const defaultSourceSettings = (): SourceSettings => ({ presetId: null, relativeFolder: "", intervalHours: null, automatic: false, automaticLimit: 20, explorationLimit: 200, pageSize: 50, history: "fromNow" });
