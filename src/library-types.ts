export interface Chapter { title: string; start: number; end: number }
export type TrackOrigin = "creator" | "automatic" | "imported";
export interface Segment { start: number; end: number; text: string }
export interface RemoteSubtitleTrack { language: string; origin: TrackOrigin; formats: string[] }
export interface LocalTrack { id: string; language: string; origin: TrackOrigin; path: string | null; segments: Segment[]; formattingLost: boolean }
export interface Bookmark { id: string; name: string; time: number }
export interface ClipReference { requestedStart: number; requestedEnd: number; actualStart: number | null; actualEnd: number | null }
export interface LibraryDetails {
  contentId: string | null; creator: string | null; platform: string | null; sourceIds: string[];
  chapters: Chapter[]; tracks: LocalTrack[]; playbackPosition: number; favorite: boolean;
  bookmarks: Bookmark[]; clip: ClipReference | null;
}
export type SmartCondition =
  | { kind: "kind" | "source" | "platform" | "tag" | "subtitleLanguage"; value: string }
  | { kind: "downloadedAfter" | "downloadedBefore" | "minDuration" | "maxDuration" | "minHeight" | "maxHeight"; value: number }
  | { kind: "hasSubtitles" | "favorite"; value: boolean };
export interface SmartCollection { id: string; name: string; mode: "all" | "any"; conditions: SmartCondition[] }
export type SearchHit =
  | { kind: "metadata"; fileId: string; title: string; excerpt: string }
  | { kind: "subtitle"; fileId: string; title: string; trackId: string; language: string; time: number; excerpt: string };
export const emptyLibraryDetails: LibraryDetails = { contentId: null, creator: null, platform: null, sourceIds: [], chapters: [], tracks: [], playbackPosition: 0, favorite: false, bookmarks: [], clip: null };
