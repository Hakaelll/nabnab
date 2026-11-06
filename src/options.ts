import type { OutputFormat } from './contracts';
import type { Priority, Schedule } from './scheduling-types';

export type SubtitleMode = 'none' | 'sidecar' | 'embed' | 'only';
export interface Clip { start: number; end: number; precise: boolean }
export interface WorkOptions {
  format: OutputFormat; height: number | null; audioBitrate: number | null;
  additionalAudio: OutputFormat | null; subtitleLanguages: string[]; subtitleMode: SubtitleMode; automaticSubtitles: boolean;
  relativeFolder: string; filenameTemplate: string; collection: string | null; tags: string[]; clip: Clip | null;
  priority: Priority; schedule: Schedule;
}
export interface Preset { id: string; name: string; revision: number; options: WorkOptions }
export interface OptionContext {url:string;title:string;creator:string;list:string;contentId:string;platform:string;contentType:string;tags:string[]}
export interface ResolvedOptions { version: number; presetId: string | null; presetRevision: number | null; values: WorkOptions; origins: Record<string, string>; context?:OptionContext }
export interface RuleConditions { platform: string; source: string; contentType: string; title: string; tag: string }
export interface Rule { id: string; name: string; enabled: boolean; conditions: RuleConditions; presetId: string | null; relativeFolder: string | null; collection: string | null; tags: string[] }
export const defaultWorkOptions = (): WorkOptions => ({format:'mp4',height:null,audioBitrate:null,additionalAudio:null,subtitleLanguages:[],subtitleMode:'none',automaticSubtitles:false,relativeFolder:'',filenameTemplate:'{titulo} [{id}]',collection:null,tags:[],clip:null,priority:'normal',schedule:{kind:'immediate'}});
