import {createAutomation} from "./automation-ui";
import "./style.css";
import "./sections.css";
import "./capture-shell.css";
import {createScreenCapture,captureIsActive,type CaptureState} from "./screen-capture-ui";
import {createVoice} from "./voice-ui";
import {createImageAssets} from "./image-assets-ui";
import {createStickerTool} from "./stickers-ui";
import {createUpscaleTool} from "./upscale-ui";
import {createBackgroundTool} from "./background-ui";
import {createStemsTool} from "./stems-ui";
import {createTranslationTool} from "./translation-ui";
import {createFileOrganization} from "./file-organization-ui";
import {createPortableCollections} from "./portable-collections-ui";
import {createDeviceShare} from "./device-share-ui";
import {createArticles} from "./articles-ui";
import {createOcr} from "./ocr-ui";
import {createDocumentEdits} from "./document-edits-ui";
import {createFileSources} from "./file-sources-ui";
import {createPersonalServers} from "./personal-servers-ui";
import {createAudioPlayer} from "./audio-player";
import type { WheelAction } from "./track-wheel";
import {isPlayable} from "./playback-preferences";
import {createAudioLibrary} from "./audio-library-ui";
import {createPersonalArchives} from "./personal-archives-ui";
import {createAdvancedFilesController} from "./advanced-files-ui";
import {createPalettes,palettesPanel} from "./palettes-ui";
import {createMediaEdits,mediaEditsPanel} from "./media-edits-ui";
import {createCaptionEditor} from "./captions-editor-ui";
import {createSourceExportsController} from "./source-exports-ui";
import {createLiveController} from "./live-ui";
import {createTelegramController} from "./telegram-ui";
import {createDocuments,documentToolsPanel} from "./documents-ui";
import {createGalleryExports} from "./gallery-exports-ui";
import {browserCourseProvider, createCoursesController} from "./courses-ui";
import {createTranscriptionManager} from "./transcription-ui";
import {createTorrentsController} from "./torrents-ui";
import {createPlatformsController} from "./platforms-ui";
import {readExpandedToolOptions, toolDestinations, toolsCatalog} from "./views-tools";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { startGraphics,handleEmblemClick } from "./graphics";
import { errorLabel, errorDetails, isCancelled, searchKey } from "./presentation";
import { activeStates, cancellableStates, defaultToolOptions, initialSettings, sourceUrls, type BridgeStatus, type Copy, type DownloadView, type EngineStatus, type HomeState, type Job, type LibraryView, type Page, type SessionStatus, type Settings, type SettingsSection, type SettingsView, type ThumbnailCacheStatus, type Snapshot, type Startup, type ToolKind, type ToolOptions, type ToolsView, type Media } from "./contracts";
import { downloadsView, homeView, icon, jobInspector, libraryView, mediaInspector, settingsView, stateLabel, toolsView } from "./views";
import { needsPersonalCopyReview } from "./views-downloads";
import { filteredLibrary } from "./views-library";
import {createBatchController} from './batch-ui';
import {createSourcesController} from './sources-ui';
import {collectionIncludes, createLibraryFeatures} from './library-features';
import {createOrganizationController} from './organization-ui';
import {escapeHtml as h} from './contracts';
import {option} from './views-common';
import {readScheduling,schedulingFields} from './scheduling-types';
import type {ResolvedOptions} from './options';
import {createPalette} from './palette';

const desktop = isTauri();
let page: Page = "home";
let snapshot: Snapshot = { settings: { ...initialSettings }, jobs: [], library: [], collections: [] };
let enginesReady = false;
let generation = Date.now();
let analysisRequest = 0;
let analysisUrl = "";
let debounce: ReturnType<typeof setTimeout> | undefined;
let pendingRender: ReturnType<typeof setTimeout> | undefined;
let disposeGraphics: (() => void) | null = null;
let inspectorKey = "";
let compactHome = false;
let renderedPreviewRequest = 0;
let pointerInput = false;
const surfaceAnimations = new Map<HTMLElement, Animation>();
const scrollPositions = new Map<Page, number>();
const thumbnailUrls = new Map<string,string>();
let thumbnailObserver: IntersectionObserver | null = null;
const thumbnailQueue: HTMLElement[] = [];
let thumbnailWorkers=0;
const home: HomeState = { input: "", media: null, loading: false, selectedUrls: new Set(), format: "mp4", quality: "", audioBitrate: "192", subtitleLanguage: "", submitting: false };
const downloads: DownloadView = { query: "", filter: "all", selected: new Set(), detailId: null };
const library: LibraryView = { query: "", filter: "all", sort: "newest", layout: "grid", collection: "", offset: 0, selected: new Set(), detailId: null };
type LocalToolModule = ReturnType<typeof localToolGroups>[number]["items"][number][0];
let localToolModule: "media" | LocalToolModule = "media";
const tools: ToolsView = { kind: "convert", ids: new Set(), options: defaultToolOptions("convert"), picking: false };
const preferences: SettingsView = { section: "general", engines: null, sessions: null, bridge: null, pairing: null, cache: null, working: false };
const t: Copy = (spanish, english) => snapshot.settings.language === "es" ? spanish : english;
let librarySection:'files'|'collections'|'sources'|'later'|'references'|'audio'|'archives'='files';
let quickPreset='';
let toolSearch='';
let showToolCatalog=true;
let toolCategory="";
let toolCatalogGroup="video";
let quickRequest=0;
let quickLoading=false;
let libraryContentHits:import("./library-types").SearchHit[]=[];
let contentSearchGeneration=0;
let contentSearchTimer:ReturnType<typeof setTimeout>|undefined;
type QuickPrepared={url:string;title:string;contentId:string;options:ResolvedOptions;sourceId:null;groupId:null};
let quickPrepared:QuickPrepared|null=null;
let localTimezone='UTC';
const transcriptionManager=createTranscriptionManager({command,render:renderCurrent});
const torrents=createTorrentsController({command,refresh,notify,showError,t,render:renderCurrent,settings:()=>{preferences.section='engines';navigate('settings');void loadSettingsSection().catch(showError);},folder:()=>snapshot.settings.folder});
let platformModule:'catalog'|'torrent'|'courses'|'telegram'|'live'|'exports'|'files'|'fileSources'='catalog';
const mediaEdits=createMediaEdits({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t});
const palettes=createPalettes({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t});
const personalServers=createPersonalServers({
  command, refresh, render: renderCurrent, notify, t,
  folder: () => snapshot.settings.folder,
  chooseFolder: () => { void chooseFolder().then(renderCurrent).catch(showError); },
  openBrowser: () => { platformModule = "fileSources"; navigate("platforms"); fileSources.openPersonal(); },
  openReferences: () => { librarySection = "references"; navigate("library"); void personalServers.load(); },
  openDownload: jobId => {
    navigate("downloads");
    const selector = `[data-action="review-personal-copy"][data-id="${CSS.escape(jobId)}"]`;
    (document.querySelector<HTMLElement>(`#detail-dialog[open] ${selector}`) ?? document.querySelector<HTMLElement>(selector))?.focus();
  },
});
const fileSources=createFileSources({command,refresh,render:renderCurrent,notify,showError,t,renderPersonal:personalServers.renderBrowser,loadPersonal:personalServers.load,followSource:(url,kind)=>{librarySection="sources";sources.openAdd(url,kind);navigate("library");},folder:()=>snapshot.settings.folder,chooseFolder:()=>{void chooseFolder().then(renderCurrent).catch(showError);}});
const advancedFiles=createAdvancedFilesController({command,refresh,render:renderCurrent,notify,showError,t,settings:()=>{preferences.section='engines';navigate('settings');void loadSettingsSection().catch(showError);},folder:()=>snapshot.settings.folder});
const captionEditor=createCaptionEditor({command,snapshot:()=>snapshot,refresh,render:renderLibraryModules,notify,t});
const sourceExports=createSourceExportsController({command,refresh,render:renderCurrent,notify,showError,t});
const live=createLiveController({command,refresh,render:renderCurrent,notify,t});
const telegram=createTelegramController({command,refresh,render:renderCurrent,notify,showError,t,settings:()=>{preferences.section='engines';navigate('settings');void loadSettingsSection().catch(showError);},folder:()=>snapshot.settings.folder});
const articles=createArticles({command,snapshot:()=>snapshot,refresh,render:renderLibraryModules,notify,t,share:input=>deviceShare.open(input)});
const ocr=createOcr({command,snapshot:()=>snapshot,refresh,render:renderLibraryModules,notify,t,settings:()=>{preferences.section="engines";navigate("settings");void loadSettingsSection().catch(showError);}});
const imageAssets=createImageAssets({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t});
const stickerTool=createStickerTool({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t});
const upscaleTool=createUpscaleTool({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t,components:()=>{preferences.section='engines';navigate('settings');}});
const backgroundTool=createBackgroundTool({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t,components:()=>{preferences.section='engines';navigate('settings');}});
const stemsTool=createStemsTool({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t,components:()=>{preferences.section='engines';navigate('settings');},
  compare:(id,group)=>audioPlayer.compareTrack(id,group),playingId:()=>audioPlayer.current()?.id,pausePlayback:()=>audioPlayer.pause(),share:ids=>deviceShare.open({ids})});
const translationTool=createTranslationTool({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t,
  openTool:mode=>openToolDestination(()=>openLocalTool(mode),"subtitles"),
  components:()=>{preferences.section='engines';navigate('settings');}});
const fileOrganization=createFileOrganization({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t});
const portable=createPortableCollections({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t});
const deviceShare=createDeviceShare({command,t,item:id=>snapshot.library.find(item=>item.id===id),afterClose:()=>{renderInspector();libraryFeatures.bind();},package:ids=>{
  localToolModule='portable';navigate('tools');portable.open(ids,true);
}});
type CaptureStatus=CaptureState&{shortcutLabel?:string|null;shortcutReason?:string|null};
let captureStatus:CaptureStatus|null=null;
const screenCapture=createScreenCapture({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t,onStatus:showCaptureStatus,settings:()=>{preferences.section='engines';navigate('settings');void loadSettingsSection().catch(showError);},chooseFolder:()=>{void chooseFolder().then(renderCurrent).catch(showError);}});
const automation=createAutomation({command,render:renderCurrent,notify,t,desktop,copy:copyText,review:()=>{preferences.section="automation";navigate("settings");}});
const voice=createVoice({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t,settings:()=>{preferences.section='engines';navigate('settings');void loadSettingsSection().catch(showError);},chooseFolder:()=>{void chooseFolder().then(renderCurrent).catch(showError);}});
const documentEdits=createDocumentEdits({command,snapshot:()=>snapshot,refresh,render:renderCurrent,notify,t});
const documents=createDocuments({command,snapshot:()=>snapshot,refresh,render:renderLibraryModules,t});
const galleryExports=createGalleryExports({command,selected:()=>[...library.selected],refresh,notify,t});
const platforms=createPlatformsController({command,snapshot:()=>snapshot,refresh,notify,showError,render:renderCurrent,t,openLink:url=>{navigate('home');updateSourceInput(url);clearTimeout(debounce);void inspect(true).catch(showError);}});
const sources=createSourcesController({command,openPublicSource:url=>{platformModule="fileSources";navigate("platforms");void fileSources.open(url).catch(showError);},getSnapshot:()=>snapshot,refresh,notify,showError,t,render:renderCurrent});
const batches=createBatchController({command,snapshot:()=>snapshot,refresh,notify,showError,t,render:renderCurrent,onFollowSource:url=>{librarySection='sources';sources.openAdd(url);navigate('library');}});
const libraryFeatures=createLibraryFeatures({
  command, snapshot:()=>snapshot, refresh, notify, showError, t,
  render:()=>{renderCurrent();renderInspector(true);libraryFeatures.bind();},
  inspect:id=>{librarySection='files';library.detailId=id;navigate('library');},
  timeFor:id=>audioPlayer.timeFor(id),
  playAt:(id,time)=>{
    const item=snapshot.library.find(item=>item.id===id);
    if(!isPlayable(item))return false;
    if(item.kind==='video')closeInspector();
    else if(page==='library'&&library.detailId!==id){
      librarySection='files';library.detailId=id;renderCurrent();
    }
    return audioPlayer.playAt(id,time);
  },
});
let desktopWheelConfig = "";
let desktopWheelRevision = 0;
let desktopPlaybackAvailable: boolean | undefined;
const audioPlayer=createAudioPlayer({
  item: id => snapshot.library.find(item => item.id === id),
  command, render: renderCurrent, refresh, showError, t,
  fullscreen: enabled => getCurrentWindow().setFullscreen(enabled),
  miniWindow: desktop ? (action,onlyWhenMinimized) => command("playback_mini_window", {action,onlyWhenMinimized}) : undefined,
  desktopPlayback: desktop ? available => {
    if (available === desktopPlaybackAvailable) return;
    desktopPlaybackAvailable = available;
    void command("playback_tray_available", {available}).catch(showError);
  } : undefined,
  desktopWheel: desktop ? session => {
    const configuration = session && { ...session, language: snapshot.settings.language,
      reducedMotion: snapshot.settings.reducedMotion, reducedTransparency: snapshot.settings.reducedTransparency };
    const serialized = JSON.stringify(configuration);
    if (serialized === desktopWheelConfig) return;
    desktopWheelConfig = serialized;
    void command("quick_skip_session", { configuration, revision: ++desktopWheelRevision }).catch(showError);
  } : undefined,
  convert: id => setToolInput([id]),
  timeChanged: (id,time)=>{libraryFeatures.playbackChanged(id,time);audioLibrary.playbackChanged(id,time);},
  playlists: () => snapshot.audioPlaylists ?? [], thumbnails: observePlaybackCovers,
  windowVisible: async () => {
    if(!desktop)return true;
    const window=getCurrentWindow();
    const [minimized,visible]=await Promise.all([window.isMinimized(),window.isVisible()]);
    return minimized!==true&&visible!==false;
  },
});
const courses=createCoursesController({command,getSnapshot:()=>snapshot,refresh,render:renderCurrent,notify,showError,t,player:audioPlayer,openLibrary:id=>{librarySection='files';library.detailId=id;navigate('library');if(snapshot.library.some(i=>i.id===id&&i.kind==='document'))void documents.open(id);}});
const audioLibrary=createAudioLibrary({
  snapshot: () => snapshot, command, refresh, render: renderCurrent, notify, showError, t, player: audioPlayer,
  invalidateCover: id => { thumbnailUrls.delete(id); },
  inspect: id => { librarySection = "files"; library.detailId = id; navigate("library"); },
  importMedia: () => { void importMedia(false).catch(showError); },
});
const personalArchives=createPersonalArchives({
  command, render: renderCurrent, notify, t,
  prepareLinks: async links => { navigate("home"); await batches.prepare(links.join("\n")); },
  openPlaylist: preview => { librarySection = "audio"; navigate("library"); audioLibrary.openArchivePlaylist(preview); },
  returnToHeading: () => { required("#main").scrollTop = 0; document.getElementById("archive-title")?.focus({ preventScroll: true }); },
});
const organization=createOrganizationController({command,snapshot:()=>snapshot,refresh,render:renderCurrent,t});

function required<T extends HTMLElement = HTMLElement>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`Missing interface element: ${selector}`);
  return element;
}
function inputValue(id: string): string { const element = document.getElementById(id); return element instanceof HTMLInputElement || element instanceof HTMLSelectElement ? element.value : ""; }
function cancelSurfaceMotion(): void {
  for (const animation of surfaceAnimations.values()) animation.cancel();
  surfaceAnimations.clear();
}
function enterSurface(element: HTMLElement, axis: "x" | "y" = "y"): void {
  surfaceAnimations.get(element)?.cancel();
  if (!pointerInput || snapshot.settings.reducedMotion || matchMedia("(prefers-reduced-motion: reduce), (forced-colors: active)").matches) return;
  const animation = element.animate([
    { opacity: 0, transform: axis === "x" ? "translateX(12px)" : "translateY(6px)" },
    { opacity: 1, transform: "none" },
  ], { duration: 200, easing: "cubic-bezier(.23, 1, .32, 1)" });
  surfaceAnimations.set(element, animation);
  void animation.finished.catch(() => {}).then(() => { if (surfaceAnimations.get(element) === animation) surfaceAnimations.delete(element); });
}
function checked(id: string): boolean { const element = document.getElementById(id); return element instanceof HTMLInputElement && element.checked; }
function isLibraryFilter(value:string|undefined):value is LibraryView['filter']{return ['all','video','audio','image','subtitle','document','file','article','ocr','recording','narration','audiobook'].includes(value??'');}
function isPage(value: string | undefined): value is Page { return value === "platforms" || value === "home" || value === "downloads" || value === "library" || value === "tools" || value === "settings"; }
function isTool(value: string | undefined): value is ToolKind { return ["transcribe","animate","frame","contactSheet","rotate","crop","mute","audioProcess","imageConvert"].includes(value??"") || value === "convert" || value === "extractAudio" || value === "trim" || value === "compress"; }
function isSettingsSection(value: string | undefined): value is SettingsSection { return value === "automation" || value === "connections" || value === "general" || value === "downloads" || value === "formats" || value === 'organization' || value === "engines" || value === "sessions" || value === "appearance" || value === "browser" || value === "about"; }
async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktop) throw new Error(t("Esta acción requiere la aplicación de escritorio nabnab.", "This action requires the nabnab desktop app."));
  return invoke<T>(name, args);
}

required("#app").innerHTML = `<canvas id="field" aria-hidden="true"></canvas><header class="titlebar" data-tauri-drag-region><span class="titlebar-context" data-tauri-drag-region></span><button type="button" class="titlebar-search" data-action="open-palette" aria-keyshortcuts="Control+K">${icon("search")}<span></span><kbd aria-hidden="true">Ctrl K</kbd></button><div class="window-actions">
  <button id="minimize" aria-label="Minimizar" title="Minimizar"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8h10"/></svg></button>
  <button id="maximize" aria-label="Maximizar" title="Maximizar"><svg class="maximize-icon" viewBox="0 0 16 16" aria-hidden="true"><rect x="3.5" y="3.5" width="9" height="9" rx=".5"/></svg><svg class="restore-icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M5.5 5.5v-2h7v7h-2"/><rect x="3.5" y="5.5" width="7" height="7" rx=".5"/></svg></button>
  <button id="close" aria-label="Cerrar" title="Cerrar"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="m4 4 8 8m0-8-8 8"/></svg></button>
  </div></header><div class="app-shell"><nav class="sidebar" aria-label="Navegación principal"><div class="sidebar-header"><span class="sidebar-mark" aria-hidden="true"></span><button id="toggle-sidebar" class="sidebar-toggle icon-button" data-action="toggle-sidebar" title="Contraer navegación" aria-label="Contraer navegación">${icon("menu")}</button></div><div class="nav-primary"></div><div class="nav-secondary"></div></nav>
  <div class="work-area"><main id="main"><div id="notification" class="notification" hidden><span id="notification-icon"></span><div><span id="notification-text"></span><details id="notification-details" hidden><summary>Detalles técnicos</summary><pre></pre></details></div><button class="icon-button" data-action="dismiss-message" aria-label="Cerrar aviso">${icon("close")}</button></div><section id="home-page" class="home-page"><div id="home-content"></div></section><div class="content-layout"><section id="page-content" class="page-content" hidden></section><aside id="inspector" class="inspector" aria-label="Detalles" hidden></aside></div><button id="activity" class="activity" data-action="show-activity" hidden><span class="activity-dot"></span><span id="activity-text"></span>${icon("arrow")}</button></main><section class="audio-player" id="audio-player" hidden><div id="audio-transport"></div></section></div></div><section class="playback-stage" id="playback-stage" aria-label="Reproductor de vídeo" tabindex="-1" hidden><video id="audio-engine" crossorigin="anonymous" preload="metadata" playsinline></video><div class="stage-captions" hidden></div><div class="stage-feedback" aria-hidden="true" hidden></div><div class="stage-hud"></div></section><aside id="capture-indicator" class="capture-indicator" hidden aria-label="Controles de captura"><span class="capture-status-dot" aria-hidden="true"></span><div class="capture-status-copy"><strong id="capture-status-label" aria-live="polite"></strong><small id="capture-status-source"></small></div><time id="capture-status-time"></time><span id="capture-shortcut-label"></span><button class="button quiet" data-action="screen-capture-open">Captura</button><button class="button secondary" data-action="screen-capture-save-replay" hidden>Guardar</button><button class="button primary" data-action="screen-capture-stop">Detener</button></aside><dialog id="detail-dialog" class="detail-dialog" aria-label="Detalles"></dialog><div id="drop-indicator" class="drop-indicator" hidden>${icon("add")}<span></span></div>`;
const inspectorBody = document.createElement("div");
inspectorBody.className = "inspector-body";
const palette=createPalette({snapshot:()=>snapshot,t,showError,search:desktop?async query=>(await command<import("./library-types").SearchHit[]>("library_search",{query,collection:null,smartId:null})).filter(hit=>hit.kind==="article"||hit.kind==="ocr").map((hit,index)=>({id:`content:${hit.fileId}:${index}`,kind:hit.kind==="article"?t("Artículo","Article"):"OCR",label:hit.title,context:hit.excerpt,run:()=>openContentHit(hit,query)})):undefined,
  destinations(query) {
    const destinations = [...navigationLinks(), ["settings", t("Ajustes", "Settings"), "settings"] as const].map(([id, label, glyph]) => ({
      id: `page:${id}`, label, glyph, kind: t("Sección", "Section"), context: "", run: () => openPage(id),
    }));
    const libraryLabel = t("Biblioteca", "Library");
    const shortcuts = [
      { id: "view:audio", label: "Audio", glyph: "audio", context: libraryLabel,
        run: () => { librarySection = "audio"; openPage("library"); } },
      { id: "view:archives", label: t("Archivos personales", "Personal archives"), glyph: "file", context: libraryLabel,
        run: () => { librarySection = "archives"; openPage("library"); } },
      { id: "view:components", label: t("Componentes", "Components"), glyph: "settings", context: t("Ajustes", "Settings"),
        run: () => { preferences.section = "engines"; openPage("settings"); } },
    ] as const;
    const sections = [...shortcuts.map(item => ({ ...item, kind: t("Vista", "View") })), ...destinations];
    if (!query) return sections;
    const kind = t("Herramienta", "Tool");
    return [...sections,
      ...toolDestinations(t).map(({ id, label, group, glyph }) => ({ id: `tool:${id}`, label, glyph, kind, context: group,
        run: () => openToolDestination(() => openMediaTool(id)) })),
      ...localToolGroups().flatMap(({ title, items }) => items.map(([id, label, glyph]) => ({
        id: `tool:${id}`, label, glyph, kind, context: title, run: () => openToolDestination(() => openLocalTool(id)),
      }))),
    ];
  },
  prepare:()=>{navigate('home');required<HTMLTextAreaElement>('#url').focus();},
  addSource:()=>{librarySection='sources';navigate('library');sources.openAdd();},
  inspect:(id,time)=>{librarySection='files';library.detailId=id;navigate('library');if(time!==undefined)libraryFeatures.seek(id,time);},
  collection:name=>{librarySection='files';library.collection=name;navigate('library');},
  preset:async id=>{if(batches.current){navigate('home');await batches.applyPresetToSelection(id);}else{preferences.section='formats';navigate('settings');batches.editPreset(id);}},
  source:id=>{librarySection='sources';navigate('library');sources.open(id);},
});

function notify(text: string, kind: "info" | "success" | "error" = "info", detail = ""): void {
  const container = required("#notification");
  container.querySelector(".automation-pending-notice")?.remove();
  container.hidden = !text; container.dataset.kind = kind;
  container.setAttribute("role", kind === "error" ? "alert" : "status");
  required("#notification-text").textContent = text;
  required("#notification-icon").innerHTML = icon(kind === "error" ? "error" : kind === "success" ? "check" : "info");
  const details = required<HTMLDetailsElement>("#notification-details");
  details.hidden = !detail; details.open = false;
  required("#notification-details summary").textContent = t("Detalles técnicos", "Technical details");
  required("#notification-details pre").textContent = detail;
}
function showError(error: unknown): void {
  if (isCancelled(error)) return;
  const messages: Record<string, [string, string]> = {
    budget_exceeded: ["Se ha superado el límite aprobado. Prepara otro plan con el tamaño o el tiempo necesarios.", "The approved budget was exceeded. Prepare another plan with the required size or time."],
    rate_limited: ["El servidor está limitando las solicitudes. Espera antes de reintentar.", "The server is limiting requests. Wait before retrying."],
    authentication: ["El contenido requiere una sesión autorizada. Revisa Sesiones y privacidad.", "This content requires an authorized session. Review Sessions and privacy."],
    forbidden: ["El origen ha denegado el acceso. Revisa los requisitos de sesión.", "The source denied access. Review session requirements."],
    dependency: ["Falta un motor o no puede iniciarse. Revisa Motores en Ajustes.", "An engine is missing or cannot start. Review Engines in Settings."],
    javascript: ["No se pudo resolver la verificación de JavaScript. Revisa los motores.", "JavaScript verification failed. Review the engines."],
    format_unavailable: ["La calidad ya no está disponible. Analiza el enlace de nuevo.", "This quality is no longer available. Analyze the link again."],
    partial_operation: ["Algunos archivos no se renombraron. Abre Revisar y renombrar → Historial para continuar o deshacer.", "Some files were not renamed. Open Review and rename → History to resume or undo."],
    processing: ["No se pudo procesar o verificar el archivo. Revisa el diagnóstico.", "The file could not be processed or verified. Review the diagnostic."],
    subtitles: ["No se pudieron integrar los subtítulos. Elige otro idioma manual o descarga sin subtítulos.", "Subtitles could not be embedded. Choose another manual language or download without subtitles."],
    disk_full: ["No hay espacio suficiente. Libera espacio o elige otro destino.", "There is not enough disk space. Free space or choose another destination."],
    permission: ["No se puede escribir en el destino. Elige otra carpeta.", "The destination is not writable. Choose another folder."],
    file_exists: ["El archivo ya existe. Elige otro destino o cambia su nombre.", "The file already exists. Choose another destination or rename it."],
    network: ["No se pudo conectar con el origen. Revisa tu conexión y reintenta.", "Could not connect to the source. Check your connection and retry."],
    unavailable: ["El contenido no está disponible. Revisa el enlace.", "The content is unavailable. Check the link."],
  };
  const code = typeof error === "object" && error !== null && "code" in error && typeof error.code === "string" ? error.code : "";
  notify(messages[code] ? t(...messages[code]) : errorLabel(error), "error", errorDetails(error));
}
function preserveFocus(render: () => void): void {
  const active = document.activeElement;
  const id = active instanceof HTMLElement ? active.id : "";
  const focusKeys = ["index", "field", "selectJob", "selectMedia", "action", "id", "page", "filter", "kind", "layout", "section", "site", "sourceUrl", "toolInput"];
  const attributes = active instanceof HTMLElement ? Object.entries(active.dataset).filter(([key]) => focusKeys.includes(key)) : [];
  const wasInInspector = active instanceof HTMLElement && inspectorBody.contains(active);
  const selection = active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement ? [active.selectionStart, active.selectionEnd] : null;
  const scrollTop = required("#main").scrollTop;
  const detailsState=new Map([...document.querySelectorAll('details')].map(detail=>[detail.id||detail.className+'|'+detail.querySelector('summary')?.textContent,detail.open]));
  render();
  for(const detail of document.querySelectorAll('details')){const open=detailsState.get(detail.id||detail.className+'|'+detail.querySelector('summary')?.textContent);if(open!==undefined)detail.open=open;}
  const target = id ? document.getElementById(id) : attributes.length
    ? [...document.querySelectorAll<HTMLElement>("[data-select-job], [data-select-media], [data-action], [data-page], [data-source-url], [data-tool-input]")]
      .find(element => inspectorBody.contains(element) === wasInInspector && attributes.every(([key, value]) => element.dataset[key] === value))
    : null;
  const dialog = required<HTMLDialogElement>("#detail-dialog");
  if (target instanceof HTMLElement && !target.closest("[hidden], [inert]") && (!dialog.open || dialog.contains(target))) {
    target.focus({ preventScroll: true });
    if (selection && (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) && selection[0] !== null && selection[1] !== null && !["number", "checkbox"].includes(target.type)) {
      try { target.setSelectionRange(selection[0], selection[1]); } catch { /* Selection is unavailable for some native input types. */ }
    }
  }
  required("#main").scrollTop = scrollTop;
}
function navigationLinks(): [Page, string, Parameters<typeof icon>[0]][] {
  return [["home",t("Inicio", "Home"),"home"],["downloads",t("Descargas", "Downloads"),"downloads"],["library",t("Biblioteca", "Library"),"library"],["tools",t("Herramientas", "Tools"),"tools"],["platforms",t("Plataformas","Platforms"),"grid"]];
}

function updateNavigation(): void {
  document.body.dataset.page=page;
  required('#app').classList.toggle('is-home',page==='home');
  document.documentElement.lang = snapshot.settings.language;
  document.documentElement.classList.toggle("reduced-motion", snapshot.settings.reducedMotion);
  document.documentElement.classList.toggle("reduced-transparency", snapshot.settings.reducedTransparency);
  document.documentElement.classList.toggle("sidebar-collapsed", !snapshot.settings.sidebarExpanded);
  required(".sidebar").setAttribute("aria-label", t("Navegación principal", "Main navigation"));
  required(".titlebar-search span").textContent=t("Buscar en nabnab", "Search nabnab");
  required(".nav-primary").innerHTML = navigationLinks().map(([key,label,symbol])=>`<button class="nav-item" data-page="${key}" aria-current="${page===key?'page':'false'}" aria-label="${label}" title="${label}">${icon(symbol)}<span class="nav-label">${label}</span>${key==='downloads'&&snapshot.jobs.some(job=>activeStates.includes(job.state))?'<span class="nav-work-dot" aria-hidden="true"></span>':''}</button>`).join("");
  const settingsLabel=t("Ajustes", "Settings");
  required(".nav-secondary").innerHTML=`<button class="nav-item" data-page="settings" aria-current="${page==='settings'?'page':'false'}" aria-label="${settingsLabel}" title="${settingsLabel}">${icon("settings")}<span class="nav-label">${settingsLabel}</span></button>`;
  const toggleLabel=snapshot.settings.sidebarExpanded?t("Contraer navegación", "Collapse navigation"):t("Expandir navegación", "Expand navigation");
  required("#toggle-sidebar").setAttribute("aria-label",toggleLabel);required("#toggle-sidebar").title=toggleLabel;
  for (const [id,label] of [["minimize",t("Minimizar", "Minimize")],["close",t("Cerrar", "Close")]]) { required(`#${id}`).setAttribute("aria-label",label); required(`#${id}`).title=label; }
  const maximize=required('#maximize');const maximizeLabel=maximize.dataset.maximized==='true'?t("Restaurar", "Restore"):t("Maximizar", "Maximize");maximize.setAttribute('aria-label',maximizeLabel);maximize.title=maximizeLabel;
  required("#notification [data-action='dismiss-message']").setAttribute("aria-label",t("Cerrar aviso", "Dismiss notice"));
  required("#drop-indicator span").textContent=t("Suelta para importar", "Drop to import");
}
function graphics(): void {
  const visible=page==='home'&&!document.documentElement.classList.contains('has-playback-stage');
  if(visible&&disposeGraphics&&!snapshot.settings.reducedMotion)return;
  disposeGraphics?.(); disposeGraphics=null;
  if (!visible) return;
  disposeGraphics=startGraphics(required<HTMLCanvasElement>("#field"),required<HTMLCanvasElement>("#emblem"));
  if(snapshot.settings.reducedMotion){disposeGraphics();disposeGraphics=null;}
}
function sizeSourceInput(): void {
  const input=document.getElementById('url');if(!(input instanceof HTMLTextAreaElement))return;
  input.style.height='0px';const height=Math.min(126,Math.max(52,input.scrollHeight));input.style.height=`${height}px`;input.style.overflowY=input.scrollHeight>126?'auto':'hidden';
}
function updateHomeLayout(): void {
  if(home.media||sourceUrls(home.input).urls.length>1)compactHome=true;
  else if(!home.input.trim())compactHome=false;
  required('#home-page').classList.toggle('has-media',compactHome);
}
function bindPreviewImage(): void {
  const image=document.querySelector<HTMLImageElement>('.preview-image');if(!image)return;
  const frame=image.closest<HTMLElement>('.preview-media');if(!frame)return;
  const size=()=>{if(!image.naturalWidth||!image.naturalHeight)return;frame.style.setProperty('--preview-ratio',String(image.naturalWidth/image.naturalHeight));frame.dataset.orientation=image.naturalHeight>image.naturalWidth?'portrait':'landscape';};
  image.addEventListener('load',size);image.addEventListener('error',()=>{const heading=frame.closest<HTMLElement>('.preview-heading');if(!heading)return;const fallback=document.createElement('div');fallback.className='preview-placeholder';fallback.innerHTML=icon(home.media?.hasVideo?'video':'audio');frame.replaceWith(fallback);heading.classList.remove('with-thumbnail');heading.classList.add('is-fallback');});
  if(image.complete)size();
}
function renderHome(): void {
  preserveFocus(()=>{const emblem=document.getElementById('emblem');const status=document.getElementById('source-status');updateHomeLayout();required("#home-content").innerHTML=homeView(home,snapshot,enginesReady,desktop,t);const replacement=document.getElementById('emblem');if(emblem instanceof HTMLCanvasElement&&replacement)replacement.replaceWith(emblem);const nextStatus=document.getElementById('source-status');if(status&&nextStatus){const text=nextStatus.textContent;nextStatus.replaceWith(status);status.textContent=text;}sizeSourceInput();bindPreviewImage();});graphics();
  required('#home-page').classList.toggle('has-batch',batches.active);
  if(batches.active){document.querySelector('.source-preview')?.remove();document.querySelector('.home-workspace > .inline-error')?.remove();required('.home-workspace').insertAdjacentHTML('beforeend',batches.render());}
  else if(home.media&&!home.media.entries.length){
    const preview=document.querySelector('.source-preview');
    preview?.insertAdjacentHTML('beforeend',`<div class="quick-preset"><label>${t('Ajuste guardado','Saved preset')}<select id="quick-preset">${option('',t('Opciones actuales','Current options'),quickPreset)}${(snapshot.presets??[]).map(p=>option(p.id,p.name,quickPreset)).join('')}</select></label><button type="button" class="button quiet" data-action="prepare-single">${t('Fragmentos y opciones','Clips and options')}</button><button type="button" class="button quiet" data-action="save-single-later">${t('Guardar para después','Save for later')}</button>${quickPrepared?`<p class="field-note">${h(quickPrepared.options.values.format.toUpperCase())} · ${quickPrepared.options.values.additionalAudio?t('También audio · ','Also audio · '):''}${h(quickPrepared.options.values.relativeFolder||snapshot.settings.folder)} · ${t('Tamaño desconocido. Las salidas adicionales son operaciones relacionadas.','Size unknown. Additional outputs are related operations.')}</p>`:''}</div>`);
  }
  const quickButton=document.getElementById('download');if(quickButton instanceof HTMLButtonElement&&quickLoading)quickButton.disabled=true;
  const previewRequest=home.media?.requestId??0;
  if(previewRequest&&previewRequest!==renderedPreviewRequest){const preview=document.querySelector<HTMLElement>('.source-preview');if(preview)enterSurface(preview);}
  renderedPreviewRequest=previewRequest;
}
function renderInspector(force=false): void {
  if(deviceShare.isOpen())return;
  const job=page==='downloads'?snapshot.jobs.find(item=>item.id===downloads.detailId):null;
  const item=page==='library'&&librarySection==='files'?snapshot.library.find(item=>item.id===library.detailId):null;
  const key=job?`job:${job.id}:${job.runId}:${job.updatedAt}:${job.state}`:item?`media:${item.id}:${item.path}:${item.collections.join(',')}:${item.tags.join(',')}`:"";
  const aside=required("#inspector");const dialog=required<HTMLDialogElement>("#detail-dialog");const opening=!inspectorKey&&!!key;
  if(!key){aside.hidden=true;if(dialog.open)dialog.close();inspectorKey="";required(".content-layout").classList.remove("has-inspector");return;}
  if(force||key!==inspectorKey){inspectorBody.innerHTML=job?jobInspector(job,snapshot,t):item?mediaInspector(item,snapshot,t):"";inspectorKey=key;
    if(item?.kind==='article')inspectorBody.insertAdjacentHTML("beforeend",articles.renderInspector(item));
    if(item?.details?.ocrRef)inspectorBody.insertAdjacentHTML("beforeend",ocr.renderInspector(item));
    if(item&&(item.kind==='article'||item.details?.ocrRef||item.kind==='document'&&/\.(pdf|epub|txt|md)$/i.test(item.name)))inspectorBody.insertAdjacentHTML('beforeend',`<section class="local-narration-action"><button class="button secondary" data-action="voice-from-library" data-id="${h(item.id)}">${icon('audio')}${t('Preparar narración local','Prepare local narration')}</button></section>`);
    if(job&&['pending','paused'].includes(job.state))inspectorBody.insertAdjacentHTML('beforeend',`<form id="job-scheduling" class="settings-form" data-id="${h(job.id)}"><h3>${t('Prioridad y programación','Priority and schedule')}</h3>${schedulingFields('job',job.priority??'normal',job.schedule??{kind:'immediate'},localTimezone,t)}<button type="submit" class="button secondary">${t('Guardar programación','Save schedule')}</button><div class="button-group"><button type="button" class="button quiet" data-action="job-next" data-id="${h(job.id)}">${t('Ejecutar a continuación','Run next')}</button><button type="button" class="button quiet" data-action="job-up" data-id="${h(job.id)}">${t('Subir','Move up')}</button><button type="button" class="button quiet" data-action="job-down" data-id="${h(job.id)}">${t('Bajar','Move down')}</button></div></form>`);
  }
  if(window.innerWidth>=1120){if(dialog.open)dialog.close();aside.hidden=false;if(inspectorBody.parentElement!==aside)aside.append(inspectorBody);required(".content-layout").classList.add("has-inspector");}
  else{aside.hidden=true;required(".content-layout").classList.remove("has-inspector");if(inspectorBody.parentElement!==dialog)dialog.append(inspectorBody);if(!dialog.open)dialog.showModal();}
  articles.bind();
  if(opening)enterSurface(window.innerWidth>=1120?aside:dialog,"x");
}
function renderLibraryModules():void{preserveFocus(()=>{renderCurrent();renderInspector(true);libraryFeatures.bind();});}
function localToolGroups() {
  return [
    {key:'documents',title:t('Documentos','Documents'),items:[['documents',t('Operaciones PDF','PDF operations'),'document'],['pdfEdits',t('Editar PDF','Edit PDF'),'document'],['ocr',t('Reconocer texto · OCR','Recognize text · OCR'),'document']]},
    {key:'image',title:t('Imagen y composición','Image and composition'),items:[['upscale',t('Ampliar imagen','Upscale image'),'image'],['background',t('Eliminar fondo','Remove background'),'image'],['imageAssets',t('Capturas, iconos y atlas','Screenshots, icons and atlas'),'image'],['stickers','Stickers','image'],['palettes',t('Paleta de imagen','Image palette'),'image'],['composition',t('Componer medios','Compose media'),'video']]},
    {key:'files',title:t('Archivos y colecciones','Files and collections'),items:[['fileOrganization',t('Revisar y renombrar','Review and rename'),'library'],['portable',t('Colección portátil','Portable collection'),'folder']]},
    {key:'capture',title:t('Captura','Capture'),items:[['screenCapture',t('Pantalla y repetición','Screen and replay'),'video']]},
    {key:'audio',title:t('Audio y voz','Audio and voice'),items:[['stems',t('Separar pistas','Separate stems'),'audio'],['voice',t('Narración local','Local narration'),'audio']]},
    {key:'subtitles',title:t('Subtítulos','Subtitles'),items:[['translation',t('Traducir subtítulos','Translate subtitles'),'document'],['dubbing',t('Crear doblaje','Create dubbing'),'audio']]}
  ] as const;
}
function renderCurrent(): void {
  const showingDubbing=page==='tools'&&localToolModule==='dubbing'&&!showToolCatalog;
  if(!showingDubbing)translationTool.leave();
  const directoryScroll=document.querySelector<HTMLElement>('.tool-directory')?.scrollTop??0;
  preserveFocus(()=>{
    updateNavigation();required("#home-page").hidden=page!=="home";required("#page-content").hidden=page==='home';
    if(page==='home')renderHome();
    else {disposeGraphics?.();disposeGraphics=null;required("#page-content").innerHTML=page==='downloads'?downloadsView(snapshot,downloads,t):page==='library'?libraryView(snapshot,library,t):page==='tools'?toolsView(snapshot,tools,t,showToolCatalog):page==='platforms'?(platformModule==='torrent'?torrents.render():platformModule==='courses'?courses.render():platformModule==='telegram'?telegram.render():platformModule==='live'?live.render():platformModule==='exports'?sourceExports.render():platformModule==='files'?advancedFiles.render():platformModule==='fileSources'?fileSources.render():platforms.render()):settingsView(snapshot,preferences,t,desktop);}
    if(page==='platforms'&&platformModule!=='catalog')required('#page-content').insertAdjacentHTML('afterbegin',`<button type="button" class="button quiet" data-action="platform-back">${t('← Plataformas','← Platforms')}</button>`);
    if(page==='library'){
      const container=required('#page-content');
      const navigation=`<nav class="library-navigation filter-tabs" aria-label="${t('Vistas de Biblioteca','Library views')}">${[['files',t('Archivos','Files')],['audio',t('Audio','Audio')],['collections',t('Colecciones','Collections')],['sources',t('Fuentes','Sources')],['later',t('Para después','For later')],['references',t('Referencias','References')],['archives',t('Archivos personales','Personal archives')]].map(([section,label])=>`<button data-action="library-section" data-section="${section}" aria-current="${librarySection===section?'page':'false'}">${label}</button>`).join('')}</nav>`;
      if(librarySection!=='files')container.innerHTML=librarySection==='sources'?sources.render():librarySection==='later'?batches.renderLater():librarySection==='references'?personalServers.renderReferences():librarySection==='audio'?audioLibrary.render():librarySection==='archives'?personalArchives.render():libraryFeatures.collectionsView();
      container.insertAdjacentHTML('afterbegin',navigation);
    }
    if(page==='settings'&&(preferences.section==='formats'||preferences.section==='organization'))required('.settings-content').innerHTML=`<h2 id="settings-title">${preferences.section==='formats'?t('Ajustes reutilizables','Reusable presets'):t('Organización','Organization')}</h2>${preferences.section==='formats'?batches.renderPresets():organization.render()}`;
    if(page==='tools'){
      const directory=required('.tool-directory');
      const moduleNav=localToolGroups().map(group=>`<section class="tool-group local-tool-group" data-tool-category="${group.key}"><h2>${group.title}</h2><div class="tool-navigation">${group.items.map(([id,label,glyph])=>`<button type="button" id="local-tool-${id}" data-action="tool-module" data-id="${id}" data-tool-name="${h((group.title+' '+label+(id==='voice'?' tts speech voz':id==='screenCapture'?' grabar recording screenshot':'')).toLocaleLowerCase())}" aria-pressed="${localToolModule===id}">${icon(glyph)}<span>${label}</span></button>`).join('')}</div></section>`).join('');
      directory.querySelector('.tool-search')?.insertAdjacentHTML('afterend',moduleNav);
      if(localToolModule!=='media'){
        for(const button of directory.querySelectorAll('[data-action="tool-kind"]'))button.setAttribute('aria-pressed','false');
        required('.tool-workspace').innerHTML=localToolModule==='documents'?documentToolsPanel(snapshot,t):localToolModule==='pdfEdits'?documentEdits.render():localToolModule==='ocr'?ocr.renderTools():localToolModule==='composition'?mediaEditsPanel(snapshot,t):localToolModule==='imageAssets'?imageAssets.render():localToolModule==='stickers'?stickerTool.render():localToolModule==='upscale'?upscaleTool.render():localToolModule==='background'?backgroundTool.render():localToolModule==='stems'?stemsTool.render():localToolModule==='translation'||localToolModule==='dubbing'?translationTool.render(localToolModule):localToolModule==='fileOrganization'?fileOrganization.renderTools():localToolModule==='portable'?portable.render():localToolModule==='screenCapture'?screenCapture.render():localToolModule==='voice'?voice.render():palettesPanel(snapshot,t);
      }
      if(showToolCatalog){
        const studio=required('.tool-studio');
        studio.insertAdjacentHTML('beforebegin',toolsCatalog(directory,t));
        const search=directory.querySelector('.tool-search');
        if(search)required('.tool-catalog').prepend(search);
        studio.hidden=true;
      }
    }
    if(page==='tools'){
      const search=document.getElementById('tool-search');if(search instanceof HTMLInputElement)search.value=toolSearch;
      filterToolDirectory();
      if(!showToolCatalog){
        const title=toolWorkspaceHeading();
        if(title){title.id||="tool-workspace-heading";if(title.tagName==="H2")title.tabIndex=-1;}
      }
      const directory=document.querySelector<HTMLElement>('.tool-directory');
      if(directory){
        directory.scrollTop=directoryScroll;
        const selected=directory.querySelector<HTMLElement>('[aria-pressed="true"]');
        if(!showToolCatalog&&selected?.getClientRects().length){
          const bounds=directory.getBoundingClientRect(),item=selected.getBoundingClientRect();
          const top=directory.querySelector('.tool-search')?.getBoundingClientRect().bottom??bounds.top;
          const bottom=Math.min(bounds.bottom,required('#main').getBoundingClientRect().bottom);
          if(item.bottom>bottom)directory.scrollTop+=item.bottom-bottom+8;
          else if(item.top<top)directory.scrollTop-=top-item.top+8;
        }
      }
    }
    if(page==='settings'&&preferences.section==='engines')required('.settings-content').insertAdjacentHTML('beforeend',platforms.renderComponents());
    if(page==='settings'&&preferences.section==='automation')required('.settings-content').insertAdjacentHTML('beforeend',automation.render());
    if(page==='settings'&&preferences.section==='connections')required('.settings-content').insertAdjacentHTML('beforeend',fileSources.renderConnections()+personalServers.renderConnections());
    renderInspector();libraryFeatures.bind();if(page==='platforms'&&platformModule==='catalog')platforms.bind();updateActivity();observeThumbnails();screenCapture.bind();voice.bind();if(showingDubbing)translationTool.bind();
  });
  audioLibrary.bindLyrics(page === "library" && librarySection === "audio");
}
function observeThumbnails(): void {
  thumbnailObserver?.disconnect();
  for(let index=thumbnailQueue.length-1;index>=0;index--)if(!thumbnailQueue[index].isConnected)thumbnailQueue.splice(index,1);
  if((page!=='library'&&page!=='tools'&&page!=='downloads')||!desktop)return;
  thumbnailObserver=new IntersectionObserver(entries=>{
    for(const entry of entries)if(entry.isIntersecting&&entry.target instanceof HTMLElement){thumbnailObserver?.unobserve(entry.target);enqueueThumbnail(entry.target);}
    void pumpThumbnails();
  },{root:required('#main'),rootMargin:'100px'});
  for(const element of document.querySelectorAll<HTMLElement>('#main [data-thumbnail-id]:not([data-thumbnail-queued])'))thumbnailObserver.observe(element);
}
function enqueueThumbnail(element: HTMLElement): void {
  if(element.dataset.thumbnailQueued)return;
  element.dataset.thumbnailQueued='true';
  thumbnailQueue.push(element);
}
function observePlaybackCovers(): void {
  if(!desktop)return;
  for(const element of document.querySelectorAll<HTMLElement>('#audio-player:not([hidden]) [data-thumbnail-id], #playback-stage:not([hidden]) [data-thumbnail-id], #track-wheel:not([hidden]) [data-thumbnail-id]'))enqueueThumbnail(element);
  void pumpThumbnails();
}
async function pumpThumbnails(): Promise<void> {
  while(thumbnailWorkers<2&&thumbnailQueue.length){const element=thumbnailQueue.shift();if(!element||!element.isConnected)continue;const id=element.dataset.thumbnailId;if(!id)continue;thumbnailWorkers++;
    void(async()=>{try{const url=thumbnailUrls.get(id)??await command<string>('thumbnail',{id});if(thumbnailUrls.size>=120)thumbnailUrls.delete(thumbnailUrls.keys().next().value??'');thumbnailUrls.set(id,url);if(!element.isConnected)return;const image=document.createElement('img');image.alt='';image.decoding='async';image.onload=()=>{if(element.isConnected)element.replaceChildren(image);};image.src=url;}catch{/* Missing or unsupported thumbnails retain their file-type icon. */}finally{thumbnailWorkers--;void pumpThumbnails();}})();
  }
}
function filterToolDirectory():void{
  const query=searchKey(toolSearch.trim());
  const words=query.split(/\s+/);
  for(const item of document.querySelectorAll<HTMLElement>('[data-tool-name]')){
    const name=searchKey(item.dataset.toolName??"");
    item.hidden=!words.every(word=>name.includes(word));
  }
  for(const group of document.querySelectorAll<HTMLElement>('.tool-group')){
    group.hidden=Boolean(toolCategory&&!query&&group.dataset.toolCategory!==toolCategory)
      || ![...group.querySelectorAll<HTMLElement>('[data-tool-name]')].some(item=>!item.hidden);
  }
  const available=new Set([...document.querySelectorAll<HTMLElement>('[data-tool-category]:not([hidden])')].map(group=>group.dataset.toolCategory));
  const cards=[...document.querySelectorAll<HTMLButtonElement>('.tool-catalog-item')];
  const active=available.has(toolCatalogGroup)?toolCatalogGroup:cards.find(card=>available.has(card.dataset.id))?.dataset.id;
  for(const card of cards){
    card.hidden=!available.has(card.dataset.id);
    card.setAttribute("aria-pressed",String(card.dataset.id===active));
  }
  for(const panel of document.querySelectorAll<HTMLElement>('[data-catalog-panel]'))panel.hidden=panel.dataset.catalogPanel!==active;
  for(const button of document.querySelectorAll<HTMLButtonElement>('[data-catalog-name]')){
    const row=button.closest('li');
    const name=searchKey(button.dataset.catalogName??"");
    if(row)row.hidden=!words.every(word=>name.includes(word));
  }
  const empty=document.getElementById('tool-catalog-empty');
  if(empty)empty.hidden=cards.some(card=>!card.hidden);
}
function showCaptureStatus(value:CaptureStatus):void{
  captureStatus=value;
  const indicator=required('#capture-indicator');
  const active=captureIsActive(value);indicator.hidden=!active;indicator.dataset.mode=value.phase;
  if(!active)return;
  const label=value.phase==='replay'?t('Búfer de repetición activo','Replay buffer active'):value.phase==='starting'?t('Iniciando captura…','Starting capture…'):value.phase==='stopping'?t('Finalizando captura…','Stopping capture…'):t('Grabando pantalla','Recording screen');
  const target=required('#capture-status-label');if(target.textContent!==label)target.textContent=label;
  required('#capture-status-source').textContent=value.sourceLabel??'';
  const seconds=Math.max(0,Math.floor(value.elapsedMs/1000));required('#capture-status-time').textContent=String(Math.floor(seconds/60)).padStart(2,'0')+':'+String(seconds%60).padStart(2,'0');
  required('#capture-shortcut-label').textContent=value.phase==='replay'?value.shortcutLabel??'':'';
  const open=indicator.querySelector<HTMLButtonElement>('[data-action="screen-capture-open"]');if(open)open.textContent=t('Captura','Capture');
  const save=indicator.querySelector<HTMLButtonElement>('[data-action="screen-capture-save-replay"]');if(save){save.hidden=value.phase!=='replay';save.disabled=value.availableSeconds<=0;save.textContent=t('Guardar repetición','Save replay');}
  const stop=indicator.querySelector<HTMLButtonElement>('[data-action="screen-capture-stop"]');if(stop){stop.disabled=value.phase==='stopping';stop.textContent=t('Detener','Stop');}
}
function updateActivity(): void {
  if(captureStatus)showCaptureStatus(captureStatus);
  const working=snapshot.jobs.filter(job=>activeStates.includes(job.state));const pending=snapshot.jobs.filter(job=>job.state==='pending');
  required("#activity").hidden=page==='downloads'||working.length+pending.length===0;
  required("#activity-text").textContent=working.length?`${working.length} ${t("en curso", "in progress")} · ${stateLabel(working[0].state,t)}`:`${pending.length} ${t("pendientes", "pending")}`;
}
function applySnapshot(next: Snapshot): void {
  snapshot=next;
  audioPlayer.syncDesktopWheel();
  for(const id of downloads.selected)if(!snapshot.jobs.some(job=>job.id===id))downloads.selected.delete(id);
  for(const id of library.selected)if(!snapshot.library.some(item=>item.id===id))library.selected.delete(id);
  for(const id of tools.ids)if(!snapshot.library.some(item=>item.id===id))tools.ids.delete(id);
  renderCurrent();
}
async function refresh(): Promise<void> { applySnapshot(await command<Snapshot>("get_snapshot")); }
function navigate(next: Page): void {
  if(next!=='tools'){voice.deactivate();stemsTool.leave();}
  else if(localToolModule!=='stems')stemsTool.leave();
  if(next!=='library'||librarySection!=='archives')personalArchives.leave();
  const changed=page!==next;cancelSurfaceMotion();
  if(page==='tools'){try{tools.options=readToolOptions();}catch{/* Keep the last valid options while a draft is incomplete. */}}
  if(next==='tools'&&(localToolModule!=='media'||tools.ids.size)){showToolCatalog=false;toolCategory="";}
  scrollPositions.set(page,required("#main").scrollTop);page=next;inspectorKey="";
  renderCurrent();required("#main").scrollTop=scrollPositions.get(next)??0;
  if(changed&&next==='tools')void mediaEdits.load();
  if(next==='tools')void loadLocalTool().catch(showError);
  if(changed)enterSurface(required(next==='home'?'#home-content':'#page-content'));
  if(next==='settings')void loadSettingsSection().catch(showError);
  if(next==='library'&&librarySection==='archives')void personalArchives.load();
}
function showPlatformsCatalog():void{
  platformModule='catalog';platforms.showCatalog();navigate('platforms');
  required('#main').scrollTop=0;required<HTMLInputElement>('#platform-search').focus({preventScroll:true});
}
function openPage(destination: Page) {
  if (destination === "platforms") showPlatformsCatalog();
  else {
    navigate(destination);
    document.querySelector<HTMLButtonElement>(`.nav-item[data-page="${destination}"]`)?.focus({ preventScroll: true });
  }
}
async function loadLocalTool() {
  switch (localToolModule) {
    case "upscale": await upscaleTool.load(); break;
    case "background": await backgroundTool.load(); break;
    case "stems": await stemsTool.load(); break;
    case "translation": case "dubbing": await translationTool.load(localToolModule); break;
    case "voice": await voice.open(); break;
    case "screenCapture": await screenCapture.open(); break;
  }
}
async function openLocalTool(module: LocalToolModule) {
  if (module !== "voice") voice.deactivate();
  if (module !== "stems") stemsTool.leave();
  showToolCatalog = false;
  localToolModule = module;
  if (page !== "tools") { navigate("tools"); return; }
  renderCurrent();
  await loadLocalTool();
}
async function openMediaTool(kind: ToolKind) {
  voice.deactivate();
  stemsTool.leave();
  localToolModule = "media";
  showToolCatalog = false;
  tools.kind = kind;
  tools.options = defaultToolOptions(kind);
  if (page === "tools") renderCurrent();
  else navigate("tools");
  if (kind === "transcribe") await transcriptionManager.load();
}
async function openToolDestination(open: () => Promise<void>, category = "") {
  toolCategory = category;
  toolSearch = "";
  const loading = open();
  required("#main").scrollTop = 0;
  toolWorkspaceHeading()?.focus({ preventScroll: true });
  await loading;
}
function toolWorkspaceHeading() {
  return document.querySelector<HTMLElement>(".tool-workspace :is(h2, summary)");
}
function scheduleJobsRender(): void {
  updateActivity();if(page!=='downloads'||pendingRender)return;
  pendingRender=setTimeout(()=>{pendingRender=undefined;if(page==='downloads')renderCurrent();},100);
}
function invalidateAnalysis(): void {
  quickPreset='';quickPrepared=null;quickLoading=false;++quickRequest;
  clearTimeout(debounce);++generation;
  if(analysisRequest)void command("cancel_job",{kind:"metadata",requestId:analysisRequest}).catch(()=>{});
  analysisRequest=0;analysisUrl="";home.loading=false;home.media=null;home.subtitleLanguage="";
}
function updateSourceInput(value: string, render=true): void {
  if(/^magnet:\?/i.test(value.trim())){invalidateAnalysis();home.input=value;platformModule='torrent';navigate('platforms');torrents.openMagnet(value.trim());return;}
  invalidateAnalysis();home.input=value;home.quality="";const parsed=sourceUrls(value);
  home.selectedUrls=new Set(parsed.urls.length>1?parsed.urls:[]);
  if(render)renderHome();
  else {updateHomeLayout();sizeSourceInput();required("#source-form").classList.remove("is-loading");required("#source-form").setAttribute("aria-busy","false");required("#source-status").textContent="";const load=document.getElementById("load");if(load instanceof HTMLButtonElement)load.disabled=!desktop;document.querySelector(".source-preview")?.remove();}
  if(desktop&&enginesReady&&!parsed.invalid&&parsed.urls.length===1)debounce=setTimeout(()=>void inspect(),500);
  else if(parsed.urls.length>1&&!render)renderHome();
}
async function inspect(skipGallery=false): Promise<void> {
  clearTimeout(debounce);const parsed=sourceUrls(home.input);
  const inputUrl=parsed.urls[0]?new URL(parsed.urls[0]):null;
  const sourceIdentifier = home.input.trim();
  if (/^(?:arxiv:\s*)?(?:\d{4}\.\d{4,5}|[a-z][a-z.-]*\/\d{7})(?:v[1-9]\d*)?$/i.test(sourceIdentifier)
      || /^(?:doi:\s*)?10\.5281\/zenodo\.\d+$|^zenodo:\s*\d+$/i.test(sourceIdentifier)) {
    platformModule='fileSources'; navigate('platforms'); await fileSources.open(sourceIdentifier); return;
  }
  if(inputUrl&&parsed.urls.length===1){const host=inputUrl.hostname;
    if(['github.com','archive.org','commons.wikimedia.org','arxiv.org','www.arxiv.org','export.arxiv.org','zenodo.org','www.zenodo.org','librivox.org','www.librivox.org','doi.org','dx.doi.org','drive.google.com','docs.google.com','1drv.ms','onedrive.live.com','dropbox.com','www.dropbox.com'].includes(host)||host.endsWith('.sharepoint.com')){platformModule='fileSources';navigate('platforms');await fileSources.open(inputUrl.href);return;}
    if(['t.me','telegram.me'].includes(host)){platformModule='telegram';navigate('platforms');telegram.openLink(inputUrl.href);return;}
    if(browserCourseProvider(inputUrl)||['udemy.com','hotmart.com','kiwify.com'].some(domain=>host===domain||host.endsWith('.'+domain))){platformModule='courses';navigate('platforms');await courses.open(inputUrl.href);return;}
    if(/\.(?:zip|7z|rar|iso|pdf|epub|torrent|exe|msi)$/i.test(inputUrl.pathname)){platformModule='files';navigate('platforms');advancedFiles.openUrl(inputUrl.href);return;}
  }
  if(batches.active||parsed.urls.length>1||parsed.invalid||inputUrl&&(/youtube\.com$/.test(inputUrl.hostname))&&(inputUrl.pathname.startsWith('/playlist')||inputUrl.pathname.startsWith('/@')||inputUrl.pathname.startsWith('/channel/'))){await batches.prepare(home.input);return;}
  if(parsed.invalid||parsed.urls.length===0){notify(t("Pega uno o varios enlaces HTTP o HTTPS completos.", "Paste one or more complete HTTP or HTTPS links."),"error");return;}
  if(parsed.truncated){notify(t("Divide el lote en grupos de hasta 100 enlaces.", "Split the batch into groups of up to 100 links."),"error");return;}
  if(!skipGallery&&parsed.urls.length===1&&platforms.openGallery(parsed.urls[0])){platformModule='catalog';navigate('platforms');return;}
  if(parsed.urls.length>1){home.selectedUrls=new Set(parsed.urls);renderHome();return;}
  if(home.loading&&analysisUrl===parsed.urls[0])return;
  if(!enginesReady){notify(t("Prepara los motores en Ajustes para analizar enlaces.", "Prepare engines in Settings to analyze links."));return;}
  invalidateAnalysis();const requestId=++generation;analysisRequest=requestId;analysisUrl=parsed.urls[0];home.loading=true;notify("");renderHome();
  try{const result=await command<Media>("inspect_media",{url:parsed.urls[0],requestId});if(requestId!==generation)return;if(result.entries.length){await batches.prepare(home.input);return;}if(!result.hasAudio&&!result.hasVideo&&!result.capabilities.images){platformModule='files';navigate('platforms');advancedFiles.openUrl(parsed.urls[0]);return;}home.media=result;home.selectedUrls=new Set(result.entries.map(entry=>entry.url));home.format=result.capabilities.images?"source":result.entries.length||result.hasVideo?"mp4":"audio";}
  catch(error){if(requestId===generation)showError(error);}
  finally{if(requestId===generation){home.loading=false;analysisRequest=0;analysisUrl="";renderHome();}}
}
async function queueDownload(): Promise<void> {
  if(home.submitting||quickLoading)return;
  if(quickPrepared){const prepared=quickPrepared;if(sourceUrls(home.input).urls[0]!==prepared.url||!home.media)return;home.submitting=true;renderHome();try{await command('queue_prepared',{items:[prepared],request:crypto.randomUUID()});quickPrepared=null;quickPreset='';invalidateAnalysis();home.input='';await refresh();notify(t('Añadido a Descargas.','Added to Downloads.'),'success');}finally{home.submitting=false;renderHome();}return;}
  if(home.submitting)return;const parsed=sourceUrls(home.input);const batch=Boolean(home.media?.entries.length)||parsed.urls.length>1;
  const urls=batch?[...home.selectedUrls]:parsed.urls;
  if(!urls.length||parsed.invalid||parsed.truncated||(!batch&&!home.media))return;
  const titles=urls.map(url=>home.media?.entries.find(entry=>entry.url===url)?.title??(urls.length===1?home.media?.title??"":""));
  home.submitting=true;renderHome();
  try{const jobs=await command<Job[]>("queue_download",{urls,titles,format:home.format,height:home.quality?Number(home.quality):null,audioBitrate:home.format==='mp3'?Number(home.audioBitrate):null,subtitleLanguage:!batch&&(home.format==='mp4'||home.format==='source')?(home.subtitleLanguage||null):null,clientRequest:crypto.randomUUID()});const ids=new Set(jobs.map(job=>job.id));snapshot.jobs=[...snapshot.jobs.filter(job=>!ids.has(job.id)),...jobs];invalidateAnalysis();home.input="";home.selectedUrls.clear();notify(t("Añadido a Descargas. Ya puedes pegar otro enlace.", "Added to Downloads. You can paste another link now."),"success");}
  catch(error){showError(error);}finally{home.submitting=false;renderHome();updateActivity();required<HTMLTextAreaElement>("#url").focus({preventScroll:true});}
}
async function openContentHit(hit:import('./library-types').SearchHit,query:string):Promise<void>{
  librarySection='files';library.detailId=hit.fileId;navigate('library');
  if(hit.kind==='article'){
    await articles.open(hit.fileId);
    const block=document.querySelector<HTMLElement>(`#article-dialog [data-article-block="${hit.block}"]`);
    if(block){block.dataset.searchMatch='true';block.focus({preventScroll:true});block.scrollIntoView({block:'center'});}
  }else if(hit.kind==='ocr')await ocr.open(hit.fileId,{page:hit.page,query});
  else if(hit.kind==='subtitle')libraryFeatures.seek(hit.fileId,hit.time);
}
async function saveSettings(settings: Settings): Promise<void> { applySnapshot(await command<Snapshot>("save_settings",{settings})); }
async function chooseFolder(): Promise<void> {const folder=await command<string|null>("choose_folder");if(folder){snapshot.settings.folder=folder;await refresh();notify(t("Carpeta de destino actualizada.", "Destination folder updated."),"success");}}
async function importMedia(folders: boolean, intoTools=false): Promise<void> {
  const previous=new Set(snapshot.library.map(item=>item.id));const next=await command<Snapshot>("import_media",{folders});
  if(intoTools){for(const item of next.library)if(!previous.has(item.id)&&(tools.kind==='imageConvert'?item.kind==='image':item.kind!=='subtitle'))tools.ids.add(item.id);tools.picking=tools.ids.size===0;}
  applySnapshot(next);if(next.library.length>previous.size)notify(t("Importado sin copiar ni modificar los originales.", "Imported without copying or modifying originals."),"success");
}
async function importUrlFile(path?: string): Promise<void> {const urls=await command<string[]>("import_url_file",path?{path}:undefined);if(urls.length){if(page!=='home')navigate('home');updateSourceInput(urls.join("\n"));}}
async function actOnJobs(ids: string[], action: "cancel"|"retry"|"pause"): Promise<void> {if(ids.length)applySnapshot(await command<Snapshot>("job_action",{ids,action}));}
async function loadSettingsSection(): Promise<void> {
  if(!desktop)return;const section=preferences.section;
  if(section==='engines'){await platforms.loadComponents();preferences.engines=await command<EngineStatus[]>("engine_status");enginesReady=preferences.engines.length>=4&&preferences.engines.every(engine=>engine.status==='installed');}
  else if(section==='automation')await automation.open();
  else if(section==='connections')await Promise.all([fileSources.loadConnections(),personalServers.load()]);
  else if(section==='sessions'){
    const [sessions,cache]=await Promise.all([
      command<SessionStatus[]>("sessions_status"),
      command<ThumbnailCacheStatus>("cache_status").catch(()=>"unavailable" as const),
    ]);
    preferences.sessions=sessions;
    preferences.cache=cache;
  }
  else if(section==='browser')preferences.bridge=await command<BridgeStatus>("bridge_status");
  if(page==='settings'&&section===preferences.section)renderCurrent();
}
async function engineOperation(mode: "repair"|"check"|"install"|"rollback"): Promise<void> {
  if(preferences.working)return;preferences.working=true;renderCurrent();
  try{if(mode==='repair')await command("repair_engines");else{const result=await command<unknown>("engine_update",{mode});if(result&&typeof result==='object'&&'message' in result&&typeof result.message==='string')notify(result.message);else notify(mode==='check'?t("Comprobación terminada. La instalación se solicita por separado.", "Check completed. Installation is requested separately."):t("Operación de motores terminada.", "Engine operation completed."),"success");}await loadSettingsSection();}
  finally{preferences.working=false;if(page==='settings')renderCurrent();}
}
function readToolOptions(): ToolOptions {
  switch(tools.kind){
    case 'convert':{const container=inputValue('tool-container');const videoCodec=inputValue('tool-video-codec');const audioCodec=inputValue('tool-audio-codec');if((container!=='mp4'&&container!=='mkv'&&container!=='webm')||(videoCodec!=='copy'&&videoCodec!=='h264'&&videoCodec!=='vp9')||(audioCodec!=='copy'&&audioCodec!=='aac'&&audioCodec!=='opus'&&audioCodec!=='mp3'))throw new Error(t("Selecciona un formato válido.", "Choose a valid format."));return{kind:'convert',container,videoCodec,audioCodec};}
    case 'extractAudio':{const format=inputValue('tool-audio-format');if(format!=='original'&&format!=='mp3'&&format!=='m4a')throw new Error(t("Selecciona un audio válido.", "Choose a valid audio format."));return{kind:'extractAudio',format};}
    case 'trim':{const start=Number(inputValue('tool-start'));const end=Number(inputValue('tool-end'));if(!Number.isFinite(start)||!Number.isFinite(end)||start<0||end<=start)throw new Error(t("El fin debe estar después del inicio.", "End must be after start."));return{kind:'trim',start,end,precise:checked('tool-precise')};}
    case 'compress':{const quality=inputValue('tool-compression');const size=inputValue('tool-target');const targetMb=size?Number(size):null;if(quality!=='balanced'&&quality!=='small'&&quality!=='high')throw new Error(t("Selecciona un perfil válido.", "Choose a valid profile."));if(targetMb!==null&&(!Number.isFinite(targetMb)||targetMb<1))throw new Error(t("El tamaño objetivo debe ser de al menos 1 MB.", "Target size must be at least 1 MB."));return{kind:'compress',quality,targetMb};}
    default:return readExpandedToolOptions(tools.kind,inputValue,checked);
  }
}
async function queueTool(): Promise<void> {
  tools.options=readToolOptions();if(!tools.ids.size)return;
  const jobs=await command<Job[]>("queue_tool",{ids:[...tools.ids],options:tools.options,clientRequest:crypto.randomUUID()});const ids=new Set(jobs.map(job=>job.id));snapshot.jobs=[...snapshot.jobs.filter(job=>!ids.has(job.id)),...jobs];downloads.filter='all';navigate('downloads');notify(t("Operación añadida a la cola. Los originales se conservan.", "Operation added to the queue. Originals are preserved."),"success");
}
function closeInspector(): void {
  const id=page==='downloads'?downloads.detailId:library.detailId;
  downloads.detailId=null;library.detailId=null;renderInspector();
  const source=[...document.querySelectorAll<HTMLButtonElement>('[data-action="inspect-job"], [data-action="inspect-media"]')].find(button=>button.dataset.id===id);
  source?.focus({preventScroll:true});
}
function focusLibraryPage(): void {
  required('#main').scrollTop=0;
  document.querySelector<HTMLButtonElement>('.media-open')?.focus({preventScroll:true});
}
async function copyText(text: string): Promise<void> {await navigator.clipboard.writeText(text);notify(t("Copiado al portapapeles.", "Copied to clipboard."),"success");}
function setToolInput(ids: string[]): void {
  localToolModule='media';if(ids.some(id=>snapshot.library.some(item=>item.id===id&&item.kind==='image'))){tools.kind='imageConvert';tools.options=defaultToolOptions('imageConvert');}tools.ids=new Set(ids.filter(id=>snapshot.library.some(item=>item.id===id&&['video','audio','image'].includes(item.kind))));tools.picking=false;navigate('tools');}

document.addEventListener("pointerdown",()=>{pointerInput=true;document.documentElement.dataset.input="pointer";},{passive:true});
document.addEventListener("keydown",()=>{pointerInput=false;document.documentElement.dataset.input="keyboard";cancelSurfaceMotion();},true);
document.addEventListener("contextmenu", event => {
  const target = event.target;
  const textInput = target instanceof HTMLInputElement && ["text", "search", "url", "email", "tel", "password", "number"].includes(target.type);
  if (textInput || target instanceof HTMLTextAreaElement || target instanceof HTMLElement && target.isContentEditable) return;
  event.preventDefault();
});
matchMedia("(prefers-reduced-motion: reduce)").addEventListener("change",()=>cancelSurfaceMotion());
document.addEventListener("click",event=>{
  const target=event.target instanceof Element?event.target.closest<HTMLButtonElement>('button'):null;
  if(!target||target.disabled)return;const destination=target.dataset.page;
  if(isPage(destination)){openPage(destination);return;}const action=target.dataset.action;
  if(event.detail > 0 && target.hasAttribute("data-open-on-double-click")) return;
  if(action)void handleAction(action,target).catch(showError);
});
document.addEventListener("dblclick", event => {
  const file = event.target instanceof Element ? event.target.closest<HTMLButtonElement>("[data-open-on-double-click]") : null;
  if (file && !file.disabled) void handleAction("play-media", file).catch(showError);
});
async function handleAction(action: string, button: HTMLButtonElement): Promise<void> {
  const id=button.dataset.id??"";
  if(action==='open-palette'){palette.open();return;}
  if(action==='tool-catalog-open'){
    showToolCatalog=true;toolCategory="";toolSearch='';renderCurrent();required('#main').scrollTop=0;
    document.getElementById('tool-search')?.focus();return;
  }
  if(action==='tool-catalog-group'){
    toolCatalogGroup=id;
    filterToolDirectory();
    return;
  }
  if(action==='share-open'){deviceShare.open();return;}
  if(action==='share-file'){deviceShare.open({ids:[id]});return;}
  if(action==='share-selected'){deviceShare.open({ids:[...library.selected]});return;}
  if(action==='share-collection'){
    const collection=button.dataset.collection??'';
    if(collection)deviceShare.open({ids:snapshot.library.filter(item=>collectionIncludes(item,collection,snapshot)).map(item=>item.id),label:button.dataset.label});
    return;
  }
  if(action==='share-job'){
    const job=snapshot.jobs.find(item=>item.id===id&&item.state==='completed');
    if(job)deviceShare.open({ids:snapshot.library.filter(item=>job.outputs.includes(item.path)).map(item=>item.id),label:job.title});
    return;
  }
  if(action==='batch-open'||action==='batch-new')navigate('home');
  if(action==='emblem-activate'){const color=handleEmblemClick();if(color.toggled)renderHome();return;}
  if(action==='open-fileSources'){platformModule='fileSources';navigate('platforms');if(id==='personal'){fileSources.openPersonal();await personalServers.load();return;}if(id==='arxiv'||id==='zenodo'||id==='librivox'){fileSources.openCatalog(id);return;}await fileSources.open();if(id==='drive'||id==='onedrive'||id==='dropbox'){const tabButton=document.createElement('button');tabButton.dataset.tab=id;await fileSources.handleClick('file-source-tab',tabButton);}return;}
  if(action==='open-files'){platformModule='files';navigate('platforms');await advancedFiles.load();return;}
  if(action==='tool-module'){
    const module=localToolGroups().flatMap(group=>group.items.map(([id])=>id)).find(id=>id===button.dataset.id);
    if(module){
      if(button.dataset.catalogGroup)await openToolDestination(()=>openLocalTool(module),button.dataset.catalogGroup);
      else await openLocalTool(module);
    }
    return;
  }
  if(action==='translation-from-library'){
    const select=document.getElementById('local-subtitle-track');
    if(select instanceof HTMLSelectElement){translationTool.chooseTrack(id,select.value);localToolModule='translation';navigate('tools');}
    return;
  }
  if(action==='translation-open-draft'){localToolModule='translation';navigate('tools');await translationTool.openSaved(id);return;}
  if(action==='organize-selected'){localToolModule='fileOrganization';navigate('tools');fileOrganization.open([...library.selected]);return;}
  if(action==='portable-selected'){localToolModule='portable';navigate('tools');portable.open([...library.selected]);return;}
  if(action==='screen-capture-open'){toolSearch='';voice.deactivate();localToolModule='screenCapture';navigate('tools');await screenCapture.open();return;}
  if(action==='voice-from-library'){toolSearch='';await articles.click('article-close',button);localToolModule='voice';navigate('tools');await voice.open({kind:'library',id});return;}
  if(action==='tool-audio-preview'){tools.options=readToolOptions();const libraryId=[...tools.ids][0];if(libraryId){await command('queue_audio_preview',{libraryId,options:tools.options,clientRequest:crypto.randomUUID()});await refresh();notify(t('Escucha previa añadida a Descargas. El resultado aparecerá en Biblioteca.','Audio preview added to Downloads. The result will appear in Library.'));}return;}
  if(action==='open-source-exports'){platformModule='exports';navigate('platforms');sourceExports.open();return;}
  if(action==='open-live'){platformModule='live';navigate('platforms');return;}
  if(action==='open-telegram'){platformModule='telegram';navigate('platforms');await telegram.load();return;}
  if(action==='stop-recording'){await command('stop_recording',{id});notify(t('Finalizando la grabación…','Finalizing recording…'));return;}
  if(action==='open-courses'){platformModule='courses';navigate('platforms');await courses.open('',id);renderCurrent();return;}
  if(action==='open-podcasts'){librarySection='sources';navigate('library');sources.openAdd('', 'podcast');return;}
  if(action==='open-torrents'||action==='open-torrent'){platformModule='torrent';navigate('platforms');await torrents.load();renderCurrent();return;}
  if(action==='platform-back'){showPlatformsCatalog();return;}
  if(await automation.click(action,button)||await screenCapture.click(action,button)||await voice.click(action,button))return;
  if(await imageAssets.click(action,button)||await stickerTool.click(action)||await upscaleTool.click(action)||await backgroundTool.click(action)||await stemsTool.click(action,button)||await translationTool.click(action)||await fileOrganization.click(action,button)||await portable.click(action,button))return;
  if(await articles.click(action,button)||await ocr.click(action,button)||await documentEdits.click(action))return;
  if(audioPlayer.click(action,button)||await audioLibrary.click(action,button)||await personalArchives.click(action,button))return;
  if(await personalServers.handleClick(action,button)||await fileSources.handleClick(action,button))return;
  if(await palettes.click(action)||await advancedFiles.handleClick(action)||await mediaEdits.click(action,button)||await sourceExports.handleClick(action)||await galleryExports.click(action)||await documents.click(action,button)||await telegram.handleClick(action)||await courses.handleClick(action,button)||await transcriptionManager.click(action,button)||await torrents.handleClick(action)||await platforms.handleClick(action,button)||await batches.handleClick(action,button)||await sources.handleClick(action,button)||await organization.handleClick(action,button)||await libraryFeatures.handleClick(button))return;
  switch(action){
    case 'open-library-collection':librarySection='files';library.collection=button.dataset.collection??'';library.offset=0;renderCurrent();break;
    case 'job-next':case 'job-up':case 'job-down':{const job=snapshot.jobs.find(j=>j.id===id);if(job)applySnapshot(await command<Snapshot>('configure_jobs',{ids:[id],priority:job.priority??'normal',schedule:job.schedule??{kind:'immediate'},orderAction:action.slice(4)}));break;}
    case 'library-section':{const section=button.dataset.section;if(section==='files'||section==='collections'||section==='sources'||section==='later'||section==='references'||section==='audio'||section==='archives'){if(section!=='archives')personalArchives.leave();librarySection=section;renderCurrent();if(section==='references')await personalServers.load();if(section==='archives')await personalArchives.load();}break;}
    case 'prepare-single':await batches.prepare(home.input,quickPreset||null);break;
    case 'save-single-later':await batches.prepare(home.input,quickPreset||null,true);break;
    case 'toggle-sidebar':{const settings={...snapshot.settings,sidebarExpanded:!snapshot.settings.sidebarExpanded};if(desktop)await saveSettings(settings);else{snapshot.settings=settings;renderCurrent();}break;}
    case 'dismiss-message':notify("");break;
    case 'go-home':navigate('home');required<HTMLTextAreaElement>('#url').focus();break;
    case 'show-activity':downloads.filter='all';navigate('downloads');break;
    case 'url-file':await importUrlFile();break;
    case 'queue-download':await queueDownload();break;
    case 'choose-folder':await chooseFolder();break;
    case 'prepare-engines':preferences.section='engines';navigate('settings');break;
    case 'job-filter':{const value=button.dataset.filter;if(value==='all'||value==='activity'||value==='pending'||value==='completed'||value==='error'){downloads.filter=value;renderCurrent();}break;}
    case 'inspect-job':
      downloads.detailId=id;
      if(page==='downloads')renderCurrent();
      else {downloads.filter='all';downloads.query='';navigate('downloads');}
      break;
    case 'cancel-job':await actOnJobs([id],'cancel');break;
    case 'pause-job':await actOnJobs([id],'pause');break;
    case 'retry-job':await actOnJobs([id],'retry');break;
    case 'review-personal-copy': {
      const job = snapshot.jobs.find(job => job.id === id);
      if (job && needsPersonalCopyReview(job)) await personalServers.reviewCopy(job.id, job.source.selection);
    } break;
    case 'cancel-selected':await actOnJobs(snapshot.jobs.filter(job=>downloads.selected.has(job.id)&&cancellableStates.includes(job.state)).map(job=>job.id),'cancel');break;
    case 'retry-selected':await actOnJobs(snapshot.jobs.filter(job=>downloads.selected.has(job.id)&&['error','cancelled'].includes(job.state)&&!needsPersonalCopyReview(job)).map(job=>job.id),'retry');break;
    case 'clear-job-selection':downloads.selected.clear();renderCurrent();break;
    case 'copy-job-error':{const job=snapshot.jobs.find(job=>job.id===id);if(job?.error)await copyText(job.error.technical);break;}
    case 'library-filter':{const value=button.dataset.filter;if(isLibraryFilter(value)){library.filter=value;library.offset=0;renderCurrent();}break;}
    case 'library-layout':if(button.dataset.layout==='grid'||button.dataset.layout==='list'){library.layout=button.dataset.layout;renderCurrent();}break;
    case 'library-next':library.offset+=60;renderCurrent();focusLibraryPage();break;
    case 'library-previous':library.offset=Math.max(0,library.offset-60);renderCurrent();focusLibraryPage();break;
    case 'import-files':await importMedia(false);break;
    case 'import-folder':await importMedia(true);break;
    case 'inspect-media':library.detailId=id;if(page!=='library')navigate('library');else renderCurrent();if(snapshot.library.some(item=>item.id===id&&item.kind==='document'))await documents.open(id);const contentHit=libraryContentHits.find(hit=>hit.fileId===id&&(hit.kind==='article'||hit.kind==='ocr'));if(library.query&&contentHit)await openContentHit(contentHit,library.query);else{if(snapshot.library.some(item=>item.id===id&&item.kind==='article'))await articles.open(id);if(snapshot.library.some(item=>item.id===id&&item.details?.ocrRef))await ocr.open(id);}break;
    case 'close-inspector':closeInspector();break;
    case 'open-media':await command('open_media',{id,folder:false});break;
    case 'play-media':case 'play-selection':case 'play-job':{
      let items = snapshot.library.filter(item=>item.id===id);
      if(action==='play-job'){
        const job=snapshot.jobs.find(candidate=>candidate.id===id);
        items=(job?.outputs??[]).flatMap(path=>snapshot.library.find(item=>item.path===path)??[]);
      } else {
        const fromSelection=action==='play-selection'||library.selected.has(id);
        if(page==='library'&&(fromSelection||library.collection)){
          items=filteredLibrary(snapshot,library).filter(item=>!fromSelection||library.selected.has(item.id));
        }
      }
      const ids=items.filter(isPlayable).map(item=>item.id);
      if(!ids.length)break;
      closeInspector();
      const position=ids.indexOf(id);
      audioPlayer.play(ids,position<0?0:position);
      break;
    }
    case 'reveal-media':await command('open_media',{id,folder:true});break;
    case 'copy-path':{const item=snapshot.library.find(item=>item.id===id);if(item)await copyText(item.path);break;}
    case 'locate-media':thumbnailUrls.delete(id);applySnapshot(await command<Snapshot>('locate_media',{id}));renderInspector(true);break;
    case 'remove-media':applySnapshot(await command<Snapshot>('remove_library',{id}));notify(t("Quitado de la biblioteca. El archivo se conserva en el disco.", "Removed from library. The file is kept on disk."),'success');break;
    case 'tool-media':setToolInput([id]);break;
    case 'tools-selected':setToolInput([...library.selected]);break;
    case 'clear-library-selection':library.selected.clear();renderCurrent();break;
    case 'tool-kind':{
      const kind=button.dataset.kind;
      if(isTool(kind)){
        if(button.dataset.catalogGroup)await openToolDestination(()=>openMediaTool(kind),button.dataset.catalogGroup);
        else await openMediaTool(kind);
      }
      break;
    }
    case 'pick-tool-files':tools.options=readToolOptions();await importMedia(false,true);break;
    case 'pick-tool-library':tools.options=readToolOptions();tools.picking=!tools.picking;renderCurrent();break;
    case 'finish-tool-picker':tools.picking=false;renderCurrent();break;
    case 'remove-tool-input':tools.options=readToolOptions();tools.ids.delete(id);renderCurrent();break;
    case 'trim-start-here':case 'trim-end-here':{const preview=document.getElementById('trim-preview');if(preview instanceof HTMLVideoElement){const options=readToolOptions();if(options.kind==='trim'){const value=Number(preview.currentTime.toFixed(2));if(action==='trim-start-here')options.start=value;else options.end=value;tools.options=options;const input=document.getElementById(action==='trim-start-here'?'tool-start':'tool-end');if(input instanceof HTMLInputElement)input.value=String(value);}}break;}
    case 'settings-section':{const section=button.dataset.section;if(isSettingsSection(section)){preferences.section=section;renderCurrent();await loadSettingsSection();}break;}
    case 'repair-engines':await engineOperation('repair');break;
    case 'check-engine-update':await engineOperation('check');break;
    case 'install-engine-update':await engineOperation('install');break;
    case 'rollback-engine-update':await engineOperation('rollback');break;
    case 'rollback-engines':await command('rollback_engines');await loadSettingsSection();notify(t("Conjunto anterior de motores restaurado.", "Previous engine set restored."),'success');break;
    case 'export-diagnostics':{const path=await command<string|null>('export_diagnostics');if(path)notify(t("Diagnóstico sanitizado exportado.", "Sanitized diagnostic exported."),'success');break;}
    case 'clear-cache':{
      required<HTMLDialogElement>("#thumbnail-cache-confirm").showModal();
      break;
    }
    case 'confirm-clear-cache':{
      required<HTMLDialogElement>("#thumbnail-cache-confirm").close();
      preferences.cache=await command<ThumbnailCacheStatus>("clear_cache");
      if(page==="settings"&&preferences.section==="sessions"){
        renderCurrent();
        required("#thumbnail-cache-heading").focus();
      }
      notify(t("Caché de miniaturas vaciada.", "Thumbnail cache cleared."),"success");
      break;
    }
    case 'revoke-session':await command('revoke_session',{site:button.dataset.site??""});await loadSettingsSection();notify(t("Sesión eliminada.", "Session removed."),'success');break;
    case 'copy-pairing':if(preferences.pairing)await copyText(preferences.pairing.token);break;
    case 'revoke-bridge':await command('bridge_revoke',{extensionId:id});preferences.pairing=null;await loadSettingsSection();break;
  }
}
document.addEventListener('input',event=>{
  if(event.target instanceof HTMLElement&&event.target.id==='sticker-query'&&stickerTool.input(event.target))return;
  if(event.target instanceof HTMLElement&&event.target.id==='upscale-query'&&upscaleTool.input(event.target))return;
  if(event.target instanceof HTMLElement&&event.target.id==='background-query'&&backgroundTool.input(event.target))return;
  if(event.target instanceof HTMLElement&&['stems-query','stems-start','stems-position'].includes(event.target.id)&&stemsTool.input(event.target))return;
  if(event.target instanceof HTMLElement&&translationTool.input(event.target))return;
  if(event.target instanceof HTMLElement&&personalArchives.input(event.target))return;
  if(event.target instanceof HTMLElement&&(automation.change(event.target)||screenCapture.change(event.target)||voice.change(event.target)||imageAssets.input(event.target)||fileOrganization.input(event.target)||(event.target.id==='portable-query'&&portable.change(event.target))||audioPlayer.input(event.target)||audioLibrary.input(event.target)||personalServers.handleInput(event.target)||fileSources.handleInput(event.target)||advancedFiles.handleInput(event.target)||sourceExports.handleInput(event.target)||live.input(event.target)||telegram.handleInput(event.target)||courses.handleInput(event.target)||torrents.handleInput(event.target)||platforms.handleInput(event.target)||batches.handleInput(event.target)||sources.handleInput(event.target)||libraryFeatures.handleInput(event.target)))return;
  const target=event.target;if(!(target instanceof HTMLInputElement||target instanceof HTMLTextAreaElement))return;
  if(target.id==='url')updateSourceInput(target.value,false);
  else if(target.id==='job-search'){downloads.query=target.value;renderCurrent();}
  else if(target.id==='library-search'){library.query=target.value;library.offset=0;library.contentMatches=undefined;libraryContentHits=[];renderCurrent();clearTimeout(contentSearchTimer);const request=++contentSearchGeneration;if(library.query.trim().length>=2)contentSearchTimer=setTimeout(()=>{void command<import('./library-types').SearchHit[]>('library_search',{query:library.query,collection:library.collection&&!library.collection.startsWith('smart:')&&!library.collection.startsWith('__')?library.collection:null,smartId:library.collection.startsWith('smart:')?library.collection.slice(6):null}).then(hits=>{if(request!==contentSearchGeneration)return;libraryContentHits=hits;library.contentMatches=new Set(hits.map(hit=>hit.fileId));if(page==='library'&&librarySection==='files')renderCurrent();}).catch(showError);},250);}
  else if(target.id==='tool-search'){toolSearch=target.value;filterToolDirectory();}
  else if(target.id==='tool-picker-search'){const query=target.value.toLocaleLowerCase();for(const item of document.querySelectorAll<HTMLElement>('[data-picker-name]'))item.hidden=!item.dataset.pickerName?.includes(query);}
});
document.addEventListener('change',event=>{
  const target=event.target;
  if(target instanceof HTMLElement&&target.id!=='sticker-query'&&stickerTool.input(target))return;
  if(target instanceof HTMLElement&&target.id!=='upscale-query'&&upscaleTool.input(target))return;
  if(target instanceof HTMLElement&&target.id!=='background-query'&&backgroundTool.input(target))return;
  if(target instanceof HTMLElement&&target.id==='stems-source'&&stemsTool.input(target))return;
  if(target instanceof HTMLElement&&(target.id.startsWith('dub-')||['translation-source','translation-pair','translation-format','translation-bilingual','translation-draft'].includes(target.id))){void translationTool.change(target).catch(showError);return;}
  if(target instanceof HTMLElement&&target.closest('.personal-archives')){void personalArchives.change(target).catch(showError);return;}
  if((target instanceof HTMLInputElement||target instanceof HTMLSelectElement||target instanceof HTMLTextAreaElement)&&(articles.change(target)||ocr.change(target)))return;
  if(target instanceof HTMLSelectElement&&target.id==='library-content-filter'){const value=target.value||'all';if(isLibraryFilter(value)){library.filter=value;library.offset=0;renderCurrent();}return;}
  if(target instanceof HTMLElement&&(automation.change(target)||screenCapture.change(target)||voice.change(target)||imageAssets.change(target)||portable.change(target)||documentEdits.change(target)))return;
  if(target instanceof HTMLElement){if((target instanceof HTMLInputElement||target instanceof HTMLSelectElement)&&(palettes.change(target)||mediaEdits.change(target)||captionEditor.change(target)||documents.change(target)))return;
  if(audioPlayer.input(target)||audioLibrary.input(target)||personalServers.handleInput(target)||fileSources.handleInput(target)||advancedFiles.handleInput(target)||sourceExports.handleInput(target)||telegram.handleInput(target)||live.input(target)||courses.handleInput(target)||torrents.handleInput(target)||platforms.handleInput(target)||sources.handleInput(target)||libraryFeatures.handleChange(target))return;void batches.handleChange(target).catch(showError);}
  if(target instanceof HTMLSelectElement&&target.id==='job-schedule-kind'){for(const field of target.closest('form')?.querySelectorAll<HTMLElement>('[data-schedule-fields]')??[])field.hidden=field.dataset.scheduleFields!==target.value;return;}
  if(target instanceof HTMLInputElement){
    const job=target.dataset.selectJob;const media=target.dataset.selectMedia;const tool=target.dataset.toolInput;const url=target.dataset.sourceUrl;
    if(job){target.checked?downloads.selected.add(job):downloads.selected.delete(job);renderCurrent();}
    else if(media){target.checked?library.selected.add(media):library.selected.delete(media);renderCurrent();}
    else if(tool){target.checked?tools.ids.add(tool):tools.ids.delete(tool);}
    else if(url){target.checked?home.selectedUrls.add(url):home.selectedUrls.delete(url);renderHome();}
    else if(target.id==='select-all-sources'){const urls=home.media?.entries.length?home.media.entries.map(entry=>entry.url):sourceUrls(home.input).urls;home.selectedUrls=new Set(target.checked?urls:[]);renderHome();}
    else if(target.id==='tool-precise'){try{tools.options=readToolOptions();renderCurrent();}catch(error){showError(error);}}
  }
  if(target instanceof HTMLSelectElement){
    const value=target.value;
    if(target.id==='quick-preset'){quickPreset=value;quickPrepared=null;const request=++quickRequest;const url=sourceUrls(home.input).urls[0];const media=home.media;quickLoading=Boolean(value&&media);renderHome();void(async()=>{try{const prepared=value&&media?await command<QuickPrepared>('prepare_download',{url,title:media.title,creator:media.uploader??'',presetId:value}):null;if(request!==quickRequest||url!==sourceUrls(home.input).urls[0]||quickPreset!==value)return;quickPrepared=prepared;if(prepared){home.format=prepared.options.values.format;home.quality=String(prepared.options.values.height??'');}}finally{if(request===quickRequest){quickLoading=false;renderHome();}}})().catch(showError);return;}
    if(target.id==='output-format'&&(value==='mp4'||value==='source'||value==='mp3'||value==='m4a'||value==='audio'||value==='opus')){home.format=value;home.quality="";if(quickPrepared){const options=quickPrepared.options.values;options.format=value;options.height=null;options.audioBitrate=value==='mp3'?192:null;if(value!=='mp4'&&value!=='source'){options.additionalAudio=null;options.subtitleMode='none';options.subtitleLanguages=[];}quickPrepared.options.origins.format=t('Cambio manual','Manual change');}renderHome();}
    else if(target.id==='quality'){home.quality=value;if(quickPrepared)quickPrepared.options.values.height=value?Number(value):null;}
    else if(target.id==='audio-bitrate'){home.audioBitrate=value;if(quickPrepared)quickPrepared.options.values.audioBitrate=Number(value);}
    else if(target.id==='subtitles'){home.subtitleLanguage=value;if(quickPrepared){quickPrepared.options.values.subtitleLanguages=value?[value]:[];quickPrepared.options.values.subtitleMode=value?'embed':'none';quickPrepared.options.values.automaticSubtitles=false;}}
    else if(target.id==='collection-filter'){library.collection=value;library.offset=0;renderCurrent();}
    else if(target.id==='library-sort'&&(value==='newest'||value==='name'||value==='duration'||value==='size')){library.sort=value;library.offset=0;renderCurrent();}
  }
});
document.addEventListener('submit',event=>{const form=event.target;if(form instanceof HTMLFormElement&&form.method!=="dialog"){event.preventDefault();void submitForm(form).catch(showError);}});
async function submitForm(form: HTMLFormElement): Promise<void> {
  if(await automation.submit(form)||await screenCapture.submit(form)||await voice.submit(form))return;
  if(await imageAssets.submit(form)||await stickerTool.submit(form)||await upscaleTool.submit(form)||await backgroundTool.submit(form)||await stemsTool.submit(form)||await translationTool.submit(form)||await fileOrganization.submit(form)||await portable.submit(form))return;
  if(await articles.submit(form)||await ocr.submit(form)||await documentEdits.submit(form))return;
  if(await audioLibrary.submit(form)||await personalArchives.submit(form))return;
  if(await personalServers.handleSubmit(form)||await fileSources.handleSubmit(form))return;
  if(await sources.handleSubmit(form)||await organization.handleSubmit(form)||await libraryFeatures.handleSubmit(form))return;
  if(await palettes.submit(form)||await advancedFiles.handleSubmit(form)||await mediaEdits.submit(form)||await captionEditor.submit(form)||await sourceExports.handleSubmit(form)||await documents.submit(form)||await live.submit(form)||await telegram.handleSubmit(form)||await courses.handleSubmit(form)||await torrents.handleSubmit(form)||await platforms.handleSubmit(form))return;
  switch(form.id){
    case 'job-scheduling':{const values=readScheduling('job',localTimezone);applySnapshot(await command<Snapshot>('configure_jobs',{ids:[form.dataset.id??''],...values,orderAction:null}));break;}
    case 'source-form':await inspect();break;
    case 'tool-form':{const button=form.querySelector<HTMLButtonElement>('button[type=submit]');if(button?.disabled)return;if(button)button.disabled=true;try{await queueTool();}finally{if(button)button.disabled=false;}break;}
    case 'general-settings':{const language=inputValue('language');if(language==='es'||language==='en')await saveSettings({...snapshot.settings,language,trayEnabled:checked('tray-enabled'),closeToTray:checked('tray-enabled')&&checked('close-to-tray'),startWithWindows:checked('start-with-windows')});notify(t("Ajustes guardados.", "Settings saved."),'success');break;}
    case 'download-settings':await saveSettings({...snapshot.settings,networkLimit:Number(inputValue('network-limit')),processingLimit:Number(inputValue('processing-limit'))});notify(t("Ajustes guardados.", "Settings saved."),'success');break;
    case 'appearance-settings':cancelSurfaceMotion();await saveSettings({...snapshot.settings,reducedMotion:checked('reduced-motion'),reducedTransparency:checked('reduced-transparency'),sidebarExpanded:checked('expanded-sidebar')});notify(t("Ajustes guardados.", "Settings saved."),'success');break;
    case 'organize-media':{const split=(value:string)=>[...new Set(value.split(',').map(value=>value.trim()).filter(Boolean))].slice(0,50);applySnapshot(await command<Snapshot>('update_library',{id:form.dataset.id??"",collections:split(inputValue('media-collections')),tags:split(inputValue('media-tags'))}));notify(t("Organización guardada.", "Organization saved."),'success');break;}
    case 'session-form':{const site=inputValue('session-site');await command('import_session',{site});await loadSettingsSection();const saved=preferences.sessions?.some(session=>session.site===site);notify(saved?t("Sesión disponible con protección de Windows.", "Session available with Windows protection."):t("No se ha añadido ninguna sesión.", "No session was added."),saved?'success':'info');break;}
    case 'bridge-form':preferences.pairing=await command<{token:string;port:number}>('bridge_pair',{extensionId:inputValue('extension-id')});await loadSettingsSection();break;
  }
}
document.addEventListener('keydown',event=>{
  if(deviceShare.isOpen())return;
  if(palette.isOpen){
    if(event.ctrlKey&&!event.altKey&&!event.shiftKey&&event.key.toLowerCase()==='k'){event.preventDefault();palette.open();}
    return;
  }
  if(audioPlayer.key(event)){event.preventDefault();return;}
  if(audioPlayer.isExpanded()||audioPlayer.isDetached())return;
  if(event.ctrlKey&&!event.altKey&&!event.shiftKey&&event.key.toLowerCase()==='k'){event.preventDefault();palette.open();return;}
  if(event.key==='Escape'&&document.querySelector('dialog[open]:not(#detail-dialog)'))return;
  if(personalArchives.key(event)){event.preventDefault();return;}
  if(event.target instanceof HTMLTextAreaElement&&event.target.id==='url'&&event.key==='Enter'&&!event.shiftKey){event.preventDefault();void inspect().catch(showError);return;}
  if(event.key==='Escape'&&!required<HTMLDialogElement>('#detail-dialog').open){closeInspector();return;}
  if(!desktop||!event.ctrlKey||event.altKey||event.shiftKey)return;
  if(event.key.toLowerCase()==='l'){event.preventDefault();navigate('home');required<HTMLTextAreaElement>('#url').focus();return;}
  if(event.target instanceof HTMLInputElement||event.target instanceof HTMLTextAreaElement||event.target instanceof HTMLSelectElement)return;
  const pages:Record<string,Page>={'1':'home','2':'downloads','3':'library','4':'tools',',':'settings'};const target=pages[event.key];if(target){event.preventDefault();navigate(target);}
});
required<HTMLDialogElement>('#detail-dialog').addEventListener('cancel',event=>{event.preventDefault();closeInspector();});
required<HTMLDialogElement>('#detail-dialog').addEventListener('click',event=>{if(event.target===event.currentTarget)closeInspector();});
document.addEventListener('error',event=>{const target=event.target;if(target instanceof HTMLImageElement&&target.classList.contains('preview-image'))target.hidden=true;},true);
window.addEventListener('resize',()=>preserveFocus(()=>renderInspector()));

function droppingCourseMaterials(): boolean {
  return page === 'platforms' && platformModule === 'courses' && courses.supportsDrop();
}

async function receivePaths(paths: string[]): Promise<void> {
  required('#drop-indicator').hidden=true;
  if (droppingCourseMaterials()) { await courses.receiveDrop(paths); return; }
  const textFiles=page==='home'?paths.filter(path=>path.toLowerCase().endsWith('.txt')):[];const mediaFiles=paths.filter(path=>!textFiles.includes(path));
  for(const path of textFiles)await importUrlFile(path);
  if(mediaFiles.length){
    const previous=new Set(snapshot.library.map(item=>item.id));
    snapshot=await command<Snapshot>('import_paths',{paths:mediaFiles});
    const imported=snapshot.library.filter(item=>!previous.has(item.id));
    if(page==='tools'){
      for(const item of imported)if(item.kind!=='image')tools.ids.add(item.id);
      navigate('tools');
    }else{
      librarySection='files';library.detailId=imported[0]?.id??null;navigate('library');
    }
    notify(t("Archivo importado. El original no se ha modificado.", "File imported. The original was not modified."),'success');
  }
}
document.addEventListener('dragover',event=>{event.preventDefault();required('#drop-indicator').hidden=false;});
document.addEventListener('dragleave',event=>{if(event.relatedTarget===null)required('#drop-indicator').hidden=true;});
document.addEventListener('drop',event=>{event.preventDefault();required('#drop-indicator').hidden=true;void(async()=>{const file=event.dataTransfer?.files[0];if(file){if(file.name.toLowerCase().endsWith('.txt')){if(file.size>1024*1024)throw new Error(t("El archivo de enlaces supera el límite de 1 MB.", "The links file exceeds the 1 MB limit."));navigate('home');updateSourceInput(await file.text());}else notify(t("Arrastra archivos desde el explorador a la aplicación de escritorio.", "Drop files from Explorer into the desktop application."));return;}const value=event.dataTransfer?.getData('text/uri-list')||event.dataTransfer?.getData('text/plain');if(value){navigate('home');updateSourceInput(value);}})().catch(showError);});

audioPlayer.mount();
renderCurrent();
if(desktop){
  const nativeWindow=getCurrentWindow();
  required('#minimize').addEventListener('click',()=>{if(!audioPlayer.detachMini())void nativeWindow.minimize().catch(showError);});
  required('#maximize').addEventListener('click',()=>void nativeWindow.toggleMaximize().catch(showError));
  required('#close').addEventListener('click',()=>void Promise.resolve(audioPlayer.persist()).then(()=>nativeWindow.close()).catch(showError));
  const updateMaximize=async()=>{const maximized=await nativeWindow.isMaximized();const button=required('#maximize');button.dataset.maximized=String(maximized);const label=maximized?t("Restaurar", "Restore"):t("Maximizar", "Maximize");button.setAttribute('aria-label',label);button.title=label;};
  try{
    await listen<boolean>('playback-mini-restored',({payload})=>audioPlayer.nativeMiniRestored(payload),{target:{kind:"Webview",label:"main"}});
    await listen('playback-mini-open',audioPlayer.openFloatingMini,{target:{kind:"Webview",label:"main"}});
    await listen<WheelAction>('quick-skip-action',({payload})=>audioPlayer.wheelAction(payload),{target:{kind:"Webview",label:"main"}});
    await nativeWindow.onResized(async()=>{if(await nativeWindow.isMinimized())audioPlayer.detachMini(true);await updateMaximize();});
    await updateMaximize();
    await nativeWindow.onDragDropEvent(event=>{
      if(event.payload.type==='drop') { void receivePaths(event.payload.paths).catch(showError); return; }
      const courseDrop = droppingCourseMaterials();
      const indicator = required('#drop-indicator');
      required('#drop-indicator span').textContent = courseDrop ? t('Suelta para revisar materiales del curso', 'Drop to review course materials') : t('Suelta para importar', 'Drop to import');
      indicator.classList.toggle('is-course-drop', courseDrop);
      indicator.hidden = event.payload.type!=='enter' && event.payload.type!=='over';
    });
    await listen<CaptureStatus>('capture-update',({payload})=>{const previous=captureStatus?.phase;screenCapture.acceptStatus(payload);if(page==='tools'&&localToolModule==='screenCapture'&&(captureIsActive(payload)||previous!==payload.phase))renderCurrent();if(previous!==payload.phase&&(payload.phase==='interrupted'||payload.phase==='error')){notify(payload.readyCapture?t('La captura se interrumpió. Revisa el archivo parcial en Captura.','Capture was interrupted. Review the partial file in Capture.'):t('No se pudo continuar la captura. Revisa Captura.','Capture could not continue. Review Capture.'),'error',payload.reason??'');if(page==='tools'&&localToolModule==='screenCapture')void screenCapture.open().catch(showError);}});
    await listen('capture-saved',()=>{notify(t('Repetición enviada a Descargas.','Replay sent to Downloads.'),'success');void refresh().catch(showError);});
    await listen<string>('capture-error',({payload})=>showError(payload));
    await listen<Job>('job-update',({payload})=>{
      const current=snapshot.jobs.find(job=>job.id===payload.id);
      if(current&&(current.runId>payload.runId||(current.runId===payload.runId&&current.updatedAt>payload.updatedAt)))return;
      snapshot.jobs=current?snapshot.jobs.map(job=>job.id===payload.id?payload:job):[...snapshot.jobs,payload];
      if(page==="tools"&&(localToolModule==="translation"||localToolModule==="dubbing"))translationTool.observe(payload,localToolModule);
      if(page==="tools"&&localToolModule==="portable"&&current?.state!==payload.state&&portable.isExportJob(payload.id))renderCurrent();
      if(page==="platforms"&&platformModule==="courses"&&current?.state!==payload.state)courses.observeJob(payload.id);
      scheduleJobsRender();
    });
    await listen('library-update',()=>void refresh().catch(showError));
    await listen('sources-changed',()=>void refresh().catch(showError));
    await listen('automation-update',()=>void automation.onUpdate().then(pending=>{if(pending)required('#notification-text').insertAdjacentHTML('afterend',`<button type="button" class="button quiet automation-pending-notice" data-action="automation-open-settings">${t('Revisar solicitud','Review request')}</button>`);}).catch(showError));
    await listen('queue-changed',()=>void refresh().catch(showError));
    await listen('drafts-changed',()=>void refresh().catch(showError));
    await listen<string>('browser-draft',({payload})=>void(async()=>{await refresh();const draft=snapshot.drafts?.find(d=>d.id===payload);if(!draft)return;if(draft.savedForLater){notify(t('Enlaces guardados en Biblioteca > Para después.','Links saved in Library > For later.'),'success');}else{navigate('home');await batches.open(draft);}})().catch(showError));
    await listen<string>('app-warning',({payload})=>notify(payload));
    const state=await command<Startup>('startup');snapshot=state;enginesReady=state.enginesReady;void command<CaptureStatus>('screen_capture_status').then(value=>{if(value)screenCapture.acceptStatus(value);}).catch(()=>{});localTimezone=await command<string>('local_timezone').catch(()=> 'UTC');audioPlayer.restore();renderCurrent();if(state.engineError)notify(t("Algunos motores requieren preparación. Revisa Motores en Ajustes.", "Some engines require preparation. Review Engines in Settings."),'info',state.engineError);
  }catch(error){showError(error);}
}else{for(const button of document.querySelectorAll<HTMLButtonElement>('.window-actions button'))button.disabled=true;}
required<HTMLTextAreaElement>('#url').focus({preventScroll:true});
window.addEventListener('beforeunload',()=>{audioPlayer.persist();voice.dispose();disposeGraphics?.();clearTimeout(debounce);clearTimeout(pendingRender);});
