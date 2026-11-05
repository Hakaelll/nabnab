export type Language = "es" | "en";
export type Page = "home" | "downloads" | "library" | "tools" | "settings";
export type OutputFormat = "mp4" | "mp3" | "m4a" | "opus" | "source" | "audio";
export type JobState = "pending" | "analyzing" | "ready" | "downloading" | "processing" | "verifying" | "completed" | "paused" | "cancelled" | "error";
export interface Settings { folder: string; language: Language; networkLimit: number; processingLimit: number; reducedMotion: boolean; reducedTransparency: boolean; sidebarExpanded: boolean; trayEnabled: boolean; closeToTray: boolean; startWithWindows: boolean }
export interface Failure { code: string; message: string; technical: string; retryable: boolean; retryAfter: number | null }
export interface Progress { phase: string; percent: number | null; speed: number | null; downloaded: number | null; total: number | null }
export type ToolOptions =
  | { kind: "convert"; container: "mp4" | "mkv" | "webm"; videoCodec: "copy" | "h264" | "vp9"; audioCodec: "copy" | "aac" | "opus" | "mp3" }
  | { kind: "extractAudio"; format: "original" | "mp3" | "m4a" }
  | { kind: "trim"; start: number; end: number; precise: boolean }
  | { kind: "compress"; targetMb: number | null; quality: "balanced" | "small" | "high" };
export type ToolKind = ToolOptions["kind"];
export type JobSource = { kind: "download"; url: string; format: OutputFormat; height: number | null; audioBitrate: number | null; subtitleLanguage?: string | null }
  | { kind: "transform"; libraryId: string; options: ToolOptions };
export interface Job {
  id: string; runId: number; title: string; createdAt: number; updatedAt: number; state: JobState;
  source: JobSource; folder: string; provider: string; thumbnail: string | null;
  progress: Progress | null; outputs: string[]; error: Failure | null; attempt: number; groupId: string | null;
}
export interface LibraryItem {
  id: string; path: string; name: string; kind: "video" | "audio" | "image"; size: number; duration: number | null;
  width: number | null; height: number | null; videoCodec: string | null; audioCodec: string | null;
  container: string | null; addedAt: number; source: string | null; parentId: string | null; collections: string[]; tags: string[]; missing?: boolean;
}
export interface Snapshot { settings: Settings; jobs: Job[]; library: LibraryItem[]; collections: string[] }
export interface Startup extends Snapshot { folder: string; enginesReady: boolean; engineError: string | null }
export interface Capabilities { video: boolean; audio: boolean; images: boolean; batches: boolean; subtitles: boolean; verified: boolean }
export interface MediaEntry { id: string; title: string; url: string; thumbnail: string | null; duration: number | null }
export interface Media { requestId: number; title: string; duration: number | null; thumbnail: string | null; uploader: string | null; heights: number[]; hasAudio: boolean; hasVideo: boolean; entries: MediaEntry[]; subtitleLanguages?: string[]; capabilities: Capabilities }
export interface EngineStatus { name: string; version: string | null; path: string | null; size: number; status: "installed" | "missing" | "broken"; source: string }
export interface SessionStatus { site: string }
export interface BridgeStatus { enabled: boolean; clients: string[]; port: number }
export type Copy = (spanish: string, english: string) => string;
export interface HomeState {
  input: string; media: Media | null; loading: boolean; selectedUrls: Set<string>;
  format: OutputFormat; quality: string; audioBitrate: string; subtitleLanguage: string; submitting: boolean;
}
export interface DownloadView { query: string; filter: "activity" | "pending" | "completed" | "error" | "all"; selected: Set<string>; detailId: string | null }
export interface LibraryView { query: string; filter: "all" | "video" | "audio" | "image"; sort: "newest" | "name" | "duration" | "size"; layout: "grid" | "list"; collection: string; offset: number; selected: Set<string>; detailId: string | null }
export interface ToolsView { kind: ToolKind; ids: Set<string>; options: ToolOptions; picking: boolean }
export type SettingsSection = "general" | "downloads" | "formats" | "engines" | "sessions" | "appearance" | "browser" | "about";
export interface SettingsView { section: SettingsSection; engines: EngineStatus[] | null; sessions: SessionStatus[] | null; bridge: BridgeStatus | null; pairing: { token: string; port: number } | null; cache: { bytes: number; files: number } | null; working: boolean }

export const initialSettings: Settings = { folder: "", language: "es", networkLimit: 3, processingLimit: 1, reducedMotion: false, reducedTransparency: false, sidebarExpanded: true, trayEnabled: false, closeToTray: false, startWithWindows: false };
export const activeStates: readonly JobState[] = ["analyzing", "ready", "downloading", "processing", "verifying"];
export const cancellableStates: readonly JobState[] = ["pending", "paused", ...activeStates];
export function escapeHtml(value: string | number): string {
  return String(value).replace(/[&<>"']/g, char => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char] ?? char);
}
export function sourceUrls(value: string): { urls: string[]; invalid: boolean; truncated: boolean } {
  const inputs = value.trim().split(/\s+/).filter(Boolean);
  const urls: string[] = [];
  let invalid = false;
  for (const input of inputs) {
    try {
      const parsed = new URL(input);
      if (!["http:", "https:"].includes(parsed.protocol) || !parsed.hostname || parsed.username || parsed.password) { invalid = true; continue; }
      if (!urls.includes(parsed.href)) urls.push(parsed.href);
    } catch { invalid = true; }
  }
  return { urls: urls.slice(0, 100), invalid, truncated: urls.length > 100 };
}
export function localMediaUrl(id: string): string { return `http://nabmedia.localhost/${encodeURIComponent(id)}`; }
export function defaultToolOptions(kind: ToolKind): ToolOptions {
  switch (kind) {
    case "convert": return { kind, container: "mp4", videoCodec: "h264", audioCodec: "aac" };
    case "extractAudio": return { kind, format: "original" };
    case "trim": return { kind, start: 0, end: 10, precise: false };
    case "compress": return { kind, targetMb: null, quality: "balanced" };
  }
}
