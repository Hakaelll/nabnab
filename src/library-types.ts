export interface Chapter { title: string; start: number; end: number }
export type TrackOrigin = "creator" | "automatic" | "imported";
export interface Segment { start: number; end: number; text: string }
export interface RemoteSubtitleTrack { language: string; origin: TrackOrigin; formats: string[] }
export interface LocalTrack { id: string; language: string; origin: TrackOrigin; path: string | null; segments: Segment[]; formattingLost: boolean }
export interface Bookmark { id: string; name: string; time: number }
export interface ClipReference { requestedStart: number; requestedEnd: number; actualStart: number | null; actualEnd: number | null }
export interface NarrationReference { planRef: string | null; textHash: string; sourceVersion: string; voiceName: string; language: string; engineFingerprint: string }
export interface RecordingReference { captureRef: string | null; sourceLabel: string; kind: string; durationMs: number; interrupted: boolean; reason: string | null }
export interface AudiobookChapter { bookId: string; bookTitle: string; chapter: number; reader: string | null; language: string | null }
export interface AudioTags {
  title: string | null; artist: string | null; albumArtist: string | null; album: string | null;
  genre: string | null; disc: number | null; track: number | null; embeddedCover: boolean;
}
export type AudioPlaylistEntry =
  | { kind: "local"; id: string; title: string }
  | { kind: "remote"; url: string; title: string }
  | { kind: "missing"; locator: string; title: string };
export interface AudioPlaylist { id: string; name: string; entries: AudioPlaylistEntry[] }
export interface Lyric { time: number; text: string }
export interface LibraryDetails {
  contentId: string | null; creator: string | null; platform: string | null; sourceIds: string[];sourceUrl?:string|null;license?:string|null;attribution?:string|null;webArchiveRef?:string|null;articleAnnotations?:import("./articles-ui").ArticleAnnotation[];ocrRef?:string|null;
  chapters: Chapter[]; tracks: LocalTrack[]; playbackPosition: number; favorite: boolean;
  bookmarks: Bookmark[]; clip: ClipReference | null;
  narration?: NarrationReference | null; recording?: RecordingReference | null;
  sourceVersion?: string | null; sourcePublishedAt?: string | null; sourceChecksum?: string | null;
  audiobook?: AudiobookChapter | null;
  audio?: AudioTags | null; lyrics?: Lyric[] | null; lastPlayedAt?: number | null;
}
export type SmartCondition =
  | { kind: "kind" | "source" | "platform" | "tag" | "subtitleLanguage"; value: string }
  | { kind: "downloadedAfter" | "downloadedBefore" | "minDuration" | "maxDuration" | "minHeight" | "maxHeight"; value: number }
  | { kind: "hasSubtitles" | "favorite"; value: boolean };
export interface SmartCollection { id: string; name: string; mode: "all" | "any"; conditions: SmartCondition[] }
export type SearchHit =
  | {kind:"article";fileId:string;title:string;block:number;excerpt:string}
  | {kind:"ocr";fileId:string;title:string;page:number;excerpt:string}
  | { kind: "metadata"; fileId: string; title: string; excerpt: string }
  | { kind: "subtitle"; fileId: string; title: string; trackId: string; language: string; time: number; excerpt: string };
export const emptyLibraryDetails: LibraryDetails = { contentId: null, creator: null, platform: null, sourceIds: [], chapters: [], tracks: [], playbackPosition: 0, favorite: false, bookmarks: [], clip: null };
