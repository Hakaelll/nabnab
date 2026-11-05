import "./style.css";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { startGraphics } from "./graphics";
import { errorLabel, errorDetails, isCancelled } from "./presentation";
import { activeStates, cancellableStates, defaultToolOptions, initialSettings, sourceUrls, type BridgeStatus, type Copy, type DownloadView, type EngineStatus, type HomeState, type Job, type LibraryView, type Page, type SessionStatus, type Settings, type SettingsSection, type SettingsView, type Snapshot, type Startup, type ToolKind, type ToolOptions, type ToolsView, type Media } from "./contracts";
import { downloadsView, homeView, icon, jobInspector, libraryView, mediaInspector, settingsView, stateLabel, toolsView } from "./views";

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
const scrollPositions = new Map<Page, number>();
const thumbnailUrls = new Map<string,string>();
let thumbnailObserver: IntersectionObserver | null = null;
const thumbnailQueue: HTMLElement[] = [];
let thumbnailWorkers=0;
const home: HomeState = { input: "", media: null, loading: false, selectedUrls: new Set(), format: "mp4", quality: "", audioBitrate: "192", submitting: false };
const downloads: DownloadView = { query: "", filter: "all", selected: new Set(), detailId: null };
const library: LibraryView = { query: "", filter: "all", sort: "newest", layout: "grid", collection: "", offset: 0, selected: new Set(), detailId: null };
const tools: ToolsView = { kind: "convert", ids: new Set(), options: defaultToolOptions("convert"), picking: false };
const preferences: SettingsView = { section: "general", engines: null, sessions: null, bridge: null, pairing: null, cache: null, working: false };
const t: Copy = (spanish, english) => snapshot.settings.language === "es" ? spanish : english;

function required<T extends HTMLElement = HTMLElement>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`Missing interface element: ${selector}`);
  return element;
}
function inputValue(id: string): string { const element = document.getElementById(id); return element instanceof HTMLInputElement || element instanceof HTMLSelectElement ? element.value : ""; }
function checked(id: string): boolean { const element = document.getElementById(id); return element instanceof HTMLInputElement && element.checked; }
function isPage(value: string | undefined): value is Page { return value === "home" || value === "downloads" || value === "library" || value === "tools" || value === "settings"; }
function isTool(value: string | undefined): value is ToolKind { return value === "convert" || value === "extractAudio" || value === "trim" || value === "compress"; }
function isSettingsSection(value: string | undefined): value is SettingsSection { return value === "general" || value === "downloads" || value === "formats" || value === "engines" || value === "sessions" || value === "appearance" || value === "browser" || value === "about"; }
async function command<T>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktop) throw new Error(t("Esta acción requiere la aplicación de escritorio nabnab.", "This action requires the nabnab desktop app."));
  return invoke<T>(name, args);
}

required("#app").innerHTML = `<header class="titlebar" data-tauri-drag-region><span class="titlebar-context" data-tauri-drag-region></span><div class="window-actions">
  <button id="minimize" aria-label="Minimizar" title="Minimizar"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3 8h10"/></svg></button>
  <button id="maximize" aria-label="Maximizar" title="Maximizar"><svg class="maximize-icon" viewBox="0 0 16 16" aria-hidden="true"><rect x="3.5" y="3.5" width="9" height="9" rx=".5"/></svg><svg class="restore-icon" viewBox="0 0 16 16" aria-hidden="true"><path d="M5.5 5.5v-2h7v7h-2"/><rect x="3.5" y="5.5" width="7" height="7" rx=".5"/></svg></button>
  <button id="close" aria-label="Cerrar" title="Cerrar"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="m4 4 8 8m0-8-8 8"/></svg></button>
  </div></header><div class="app-shell"><nav class="sidebar" aria-label="Navegación principal"><button id="toggle-sidebar" class="sidebar-toggle icon-button" data-action="toggle-sidebar" title="Contraer navegación" aria-label="Contraer navegación">${icon("menu")}</button><div class="nav-primary"></div><div class="nav-secondary"></div><div class="sidebar-caption">${icon("info")}<span class="nav-label">Local · nabnab</span></div></nav>
  <main id="main"><div id="notification" class="notification" hidden><span id="notification-icon"></span><div><span id="notification-text"></span><details id="notification-details" hidden><summary>Detalles técnicos</summary><pre></pre></details></div><button class="icon-button" data-action="dismiss-message" aria-label="Cerrar aviso">${icon("close")}</button></div><section id="home-page" class="home-page"><canvas id="field" aria-hidden="true"></canvas><div id="home-content"></div></section><div class="content-layout"><section id="page-content" class="page-content" hidden></section><aside id="inspector" class="inspector" aria-label="Detalles" hidden></aside></div><button id="activity" class="activity" data-action="show-activity" hidden><span class="activity-dot"></span><span id="activity-text"></span>${icon("arrow")}</button></main></div><dialog id="detail-dialog" class="detail-dialog" aria-label="Detalles"></dialog><div id="drop-indicator" class="drop-indicator" hidden>${icon("add")}<span></span></div>`;
const inspectorBody = document.createElement("div");
inspectorBody.className = "inspector-body";

function notify(text: string, kind: "info" | "success" | "error" = "info", detail = ""): void {
  const container = required("#notification");
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
    rate_limited: ["El servidor está limitando las solicitudes. Espera antes de reintentar.", "The server is limiting requests. Wait before retrying."],
    authentication: ["El contenido requiere una sesión autorizada. Revisa Sesiones y privacidad.", "This content requires an authorized session. Review Sessions and privacy."],
    forbidden: ["El origen ha denegado el acceso. Revisa los requisitos de sesión.", "The source denied access. Review session requirements."],
    dependency: ["Falta un motor o no puede iniciarse. Revisa Motores en Ajustes.", "An engine is missing or cannot start. Review Engines in Settings."],
    javascript: ["No se pudo resolver la verificación de JavaScript. Revisa los motores.", "JavaScript verification failed. Review the engines."],
    format_unavailable: ["La calidad ya no está disponible. Analiza el enlace de nuevo.", "This quality is no longer available. Analyze the link again."],
    processing: ["No se pudo procesar o verificar el archivo. Revisa el diagnóstico.", "The file could not be processed or verified. Review the diagnostic."],
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
  const selection = active instanceof HTMLInputElement || active instanceof HTMLTextAreaElement ? [active.selectionStart, active.selectionEnd] : null;
  const scrollTop = required("#main").scrollTop;
  render();
  const target = id ? document.getElementById(id) : null;
  if (target instanceof HTMLElement) {
    target.focus({ preventScroll: true });
    if (selection && (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) && selection[0] !== null && selection[1] !== null && !["number", "checkbox"].includes(target.type)) {
      try { target.setSelectionRange(selection[0], selection[1]); } catch { /* Selection is unavailable for some native input types. */ }
    }
  }
  required("#main").scrollTop = scrollTop;
}
function updateNavigation(): void {
  document.documentElement.lang = snapshot.settings.language;
  document.documentElement.classList.toggle("reduced-motion", snapshot.settings.reducedMotion);
  document.documentElement.classList.toggle("sidebar-collapsed", !snapshot.settings.sidebarExpanded);
  required(".sidebar").setAttribute("aria-label", t("Navegación principal", "Main navigation"));
  const names: [Page,string,Parameters<typeof icon>[0]][] = [["home",t("Inicio", "Home"),"home"],["downloads",t("Descargas", "Downloads"),"downloads"],["library",t("Biblioteca", "Library"),"library"],["tools",t("Herramientas", "Tools"),"tools"]];
  required(".nav-primary").innerHTML = names.map(([key,label,symbol])=>`<button class="nav-item" data-page="${key}" aria-current="${page===key?'page':'false'}" aria-label="${label}" title="${label}">${icon(symbol)}<span class="nav-label">${label}</span>${key==='downloads'&&snapshot.jobs.some(job=>activeStates.includes(job.state))?'<span class="nav-work-dot" aria-hidden="true"></span>':''}</button>`).join("");
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
  disposeGraphics?.(); disposeGraphics=null;
  if (page!=="home") return;
  disposeGraphics=startGraphics(required<HTMLCanvasElement>("#field"),required<HTMLCanvasElement>("#emblem"));
  if(snapshot.settings.reducedMotion){disposeGraphics();disposeGraphics=null;}
}
function renderHome(): void { preserveFocus(()=>{required("#home-content").innerHTML=homeView(home,snapshot,enginesReady,desktop,t);}); graphics(); }
function renderInspector(force=false): void {
  const job=page==='downloads'?snapshot.jobs.find(item=>item.id===downloads.detailId):null;
  const item=page==='library'?snapshot.library.find(item=>item.id===library.detailId):null;
  const key=job?`job:${job.id}:${job.runId}:${job.updatedAt}:${job.state}`:item?`media:${item.id}:${item.path}:${item.collections.join(',')}:${item.tags.join(',')}`:"";
  const aside=required("#inspector");const dialog=required<HTMLDialogElement>("#detail-dialog");
  if(!key){aside.hidden=true;if(dialog.open)dialog.close();inspectorKey="";required(".content-layout").classList.remove("has-inspector");return;}
  if(force||key!==inspectorKey){inspectorBody.innerHTML=job?jobInspector(job,snapshot,t):item?mediaInspector(item,snapshot,t):"";inspectorKey=key;}
  if(window.innerWidth>=1120){if(dialog.open)dialog.close();aside.hidden=false;aside.append(inspectorBody);required(".content-layout").classList.add("has-inspector");}
  else{aside.hidden=true;required(".content-layout").classList.remove("has-inspector");dialog.append(inspectorBody);if(!dialog.open)dialog.showModal();}
}
function renderCurrent(): void {
  updateNavigation();required("#home-page").hidden=page!=="home";required("#page-content").hidden=page==='home';
  if(page==='home')renderHome();
  else {disposeGraphics?.();disposeGraphics=null;preserveFocus(()=>{required("#page-content").innerHTML=page==='downloads'?downloadsView(snapshot,downloads,t):page==='library'?libraryView(snapshot,library,t):page==='tools'?toolsView(snapshot,tools,t):settingsView(snapshot,preferences,t,desktop);});}
  renderInspector();updateActivity();observeThumbnails();
}
function observeThumbnails(): void {
  thumbnailObserver?.disconnect();thumbnailQueue.length=0;
  if(page!=='library'||!desktop)return;
  thumbnailObserver=new IntersectionObserver(entries=>{
    for(const entry of entries)if(entry.isIntersecting&&entry.target instanceof HTMLElement){thumbnailObserver?.unobserve(entry.target);thumbnailQueue.push(entry.target);}
    void pumpThumbnails();
  },{root:required('#main'),rootMargin:'100px'});
  for(const element of document.querySelectorAll<HTMLElement>('[data-thumbnail-id]'))thumbnailObserver.observe(element);
}
async function pumpThumbnails(): Promise<void> {
  while(thumbnailWorkers<2&&thumbnailQueue.length){const element=thumbnailQueue.shift();if(!element||!element.isConnected)continue;const id=element.dataset.thumbnailId;if(!id)continue;thumbnailWorkers++;
    void(async()=>{try{const url=thumbnailUrls.get(id)??await command<string>('thumbnail',{id});if(thumbnailUrls.size>=120)thumbnailUrls.delete(thumbnailUrls.keys().next().value??'');thumbnailUrls.set(id,url);if(!element.isConnected)return;const image=document.createElement('img');image.alt='';image.loading='lazy';image.onload=()=>{if(element.isConnected)element.replaceChildren(image);};image.src=url;}catch{/* Missing or unsupported thumbnails retain their file-type icon. */}finally{thumbnailWorkers--;void pumpThumbnails();}})();
  }
}
function updateActivity(): void {
  const working=snapshot.jobs.filter(job=>activeStates.includes(job.state));const pending=snapshot.jobs.filter(job=>job.state==='pending');
  required("#activity").hidden=page==='downloads'||working.length+pending.length===0;
  required("#activity-text").textContent=working.length?`${working.length} ${t("en curso", "in progress")} · ${stateLabel(working[0].state,t)}`:`${pending.length} ${t("pendientes", "pending")}`;
}
function applySnapshot(next: Snapshot): void {
  snapshot=next;
  for(const id of downloads.selected)if(!snapshot.jobs.some(job=>job.id===id))downloads.selected.delete(id);
  for(const id of library.selected)if(!snapshot.library.some(item=>item.id===id))library.selected.delete(id);
  for(const id of tools.ids)if(!snapshot.library.some(item=>item.id===id))tools.ids.delete(id);
  renderCurrent();
}
async function refresh(): Promise<void> { applySnapshot(await command<Snapshot>("get_snapshot")); }
function navigate(next: Page): void {
  if(page==='tools'){try{tools.options=readToolOptions();}catch{/* Keep the last valid options while a draft is incomplete. */}}
  scrollPositions.set(page,required("#main").scrollTop);page=next;inspectorKey="";
  renderCurrent();required("#main").scrollTop=scrollPositions.get(next)??0;
  if(next==='settings')void loadSettingsSection().catch(showError);
}
function scheduleJobsRender(): void {
  updateActivity();if(page!=='downloads'||pendingRender)return;
  pendingRender=setTimeout(()=>{pendingRender=undefined;if(page==='downloads')renderCurrent();},100);
}
function invalidateAnalysis(): void {
  clearTimeout(debounce);++generation;
  if(analysisRequest)void command("cancel_job",{kind:"metadata",requestId:analysisRequest}).catch(()=>{});
  analysisRequest=0;analysisUrl="";home.loading=false;home.media=null;
}
function updateSourceInput(value: string, render=true): void {
  invalidateAnalysis();home.input=value;home.quality="";const parsed=sourceUrls(value);
  home.selectedUrls=new Set(parsed.urls.length>1?parsed.urls:[]);
  if(render)renderHome();
  else {required("#source-form").classList.remove("is-loading");required("#source-form").setAttribute("aria-busy","false");required("#source-status").textContent="";const load=document.getElementById("load");if(load instanceof HTMLButtonElement)load.disabled=!desktop;document.querySelector(".source-preview")?.remove();}
  if(desktop&&enginesReady&&!parsed.invalid&&parsed.urls.length===1)debounce=setTimeout(()=>void inspect(),500);
  else if(parsed.urls.length>1&&!render)renderHome();
}
async function inspect(): Promise<void> {
  clearTimeout(debounce);const parsed=sourceUrls(home.input);
  if(parsed.invalid||parsed.urls.length===0){notify(t("Pega uno o varios enlaces HTTP o HTTPS completos.", "Paste one or more complete HTTP or HTTPS links."),"error");return;}
  if(parsed.truncated){notify(t("Divide el lote en grupos de hasta 100 enlaces.", "Split the batch into groups of up to 100 links."),"error");return;}
  if(parsed.urls.length>1){home.selectedUrls=new Set(parsed.urls);renderHome();return;}
  if(home.loading&&analysisUrl===parsed.urls[0])return;
  if(!enginesReady){notify(t("Prepara los motores en Ajustes para analizar enlaces.", "Prepare engines in Settings to analyze links."));return;}
  invalidateAnalysis();const requestId=++generation;analysisRequest=requestId;analysisUrl=parsed.urls[0];home.loading=true;notify("");renderHome();
  try{const result=await command<Media>("inspect_media",{url:parsed.urls[0],requestId});if(requestId!==generation)return;home.media=result;home.selectedUrls=new Set(result.entries.map(entry=>entry.url));home.format=result.entries.length||result.hasVideo?"mp4":"audio";}
  catch(error){if(requestId===generation)showError(error);}
  finally{if(requestId===generation){home.loading=false;analysisRequest=0;analysisUrl="";renderHome();}}
}
async function queueDownload(): Promise<void> {
  if(home.submitting)return;const parsed=sourceUrls(home.input);const batch=Boolean(home.media?.entries.length)||parsed.urls.length>1;
  const urls=batch?[...home.selectedUrls]:parsed.urls;
  if(!urls.length||parsed.invalid||parsed.truncated||(!batch&&!home.media))return;
  const titles=urls.map(url=>home.media?.entries.find(entry=>entry.url===url)?.title??(urls.length===1?home.media?.title??"":""));
  home.submitting=true;renderHome();
  try{const jobs=await command<Job[]>("queue_download",{urls,titles,format:home.format,height:home.quality?Number(home.quality):null,audioBitrate:home.format==='mp3'?Number(home.audioBitrate):null,clientRequest:crypto.randomUUID()});const ids=new Set(jobs.map(job=>job.id));snapshot.jobs=[...snapshot.jobs.filter(job=>!ids.has(job.id)),...jobs];invalidateAnalysis();home.input="";home.selectedUrls.clear();notify(t("Añadido a Descargas. Ya puedes pegar otro enlace.", "Added to Downloads. You can paste another link now."),"success");}
  catch(error){showError(error);}finally{home.submitting=false;renderHome();updateActivity();required<HTMLTextAreaElement>("#url").focus({preventScroll:true});}
}
async function saveSettings(settings: Settings): Promise<void> { applySnapshot(await command<Snapshot>("save_settings",{settings})); }
async function chooseFolder(): Promise<void> {const folder=await command<string|null>("choose_folder");if(folder){snapshot.settings.folder=folder;await refresh();notify(t("Carpeta de destino actualizada.", "Destination folder updated."),"success");}}
async function importMedia(folders: boolean, intoTools=false): Promise<void> {
  const previous=new Set(snapshot.library.map(item=>item.id));const next=await command<Snapshot>("import_media",{folders});
  if(intoTools){for(const item of next.library)if(!previous.has(item.id)&&item.kind!=='image')tools.ids.add(item.id);tools.picking=tools.ids.size===0;}
  applySnapshot(next);if(next.library.length>previous.size)notify(t("Importado sin copiar ni modificar los originales.", "Imported without copying or modifying originals."),"success");
}
async function importUrlFile(path?: string): Promise<void> {const urls=await command<string[]>("import_url_file",path?{path}:undefined);if(urls.length){if(page!=='home')navigate('home');updateSourceInput(urls.join("\n"));}}
async function actOnJobs(ids: string[], action: "cancel"|"retry"|"pause"): Promise<void> {if(ids.length)applySnapshot(await command<Snapshot>("job_action",{ids,action}));}
async function loadSettingsSection(): Promise<void> {
  if(!desktop)return;const section=preferences.section;
  if(section==='engines'){preferences.engines=await command<EngineStatus[]>("engine_status");enginesReady=preferences.engines.length>=4&&preferences.engines.every(engine=>engine.status==='installed');}
  else if(section==='sessions'){preferences.sessions=await command<SessionStatus[]>("sessions_status");preferences.cache=await command<{bytes:number;files:number}>("cache_status");}
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
async function copyText(text: string): Promise<void> {await navigator.clipboard.writeText(text);notify(t("Copiado al portapapeles.", "Copied to clipboard."),"success");}
function setToolInput(ids: string[]): void {tools.ids=new Set(ids.filter(id=>snapshot.library.some(item=>item.id===id&&item.kind!=='image')));tools.picking=false;navigate('tools');}

document.addEventListener("click",event=>{
  const target=event.target instanceof Element?event.target.closest<HTMLButtonElement>('button'):null;
  if(!target||target.disabled)return;const destination=target.dataset.page;
  if(isPage(destination)){navigate(destination);[...document.querySelectorAll<HTMLButtonElement>('.nav-item')].find(button=>button.dataset.page===destination)?.focus({preventScroll:true});return;}const action=target.dataset.action;
  if(action)void handleAction(action,target).catch(showError);
});
async function handleAction(action: string, button: HTMLButtonElement): Promise<void> {
  const id=button.dataset.id??"";
  switch(action){
    case 'toggle-sidebar':{const settings={...snapshot.settings,sidebarExpanded:!snapshot.settings.sidebarExpanded};if(desktop)await saveSettings(settings);else{snapshot.settings=settings;renderCurrent();}break;}
    case 'dismiss-message':notify("");break;
    case 'go-home':navigate('home');required<HTMLTextAreaElement>('#url').focus();break;
    case 'show-activity':downloads.filter='all';navigate('downloads');break;
    case 'paste':{const value=await navigator.clipboard.readText();if(value.trim()){updateSourceInput(value);required<HTMLTextAreaElement>('#url').focus();}break;}
    case 'url-file':await importUrlFile();break;
    case 'queue-download':await queueDownload();break;
    case 'choose-folder':await chooseFolder();break;
    case 'prepare-engines':preferences.section='engines';navigate('settings');break;
    case 'job-filter':{const value=button.dataset.filter;if(value==='all'||value==='activity'||value==='pending'||value==='completed'||value==='error'){downloads.filter=value;renderCurrent();}break;}
    case 'inspect-job':downloads.detailId=id;renderCurrent();break;
    case 'cancel-job':await actOnJobs([id],'cancel');break;
    case 'pause-job':await actOnJobs([id],'pause');break;
    case 'retry-job':await actOnJobs([id],'retry');break;
    case 'cancel-selected':await actOnJobs(snapshot.jobs.filter(job=>downloads.selected.has(job.id)&&cancellableStates.includes(job.state)).map(job=>job.id),'cancel');break;
    case 'retry-selected':await actOnJobs(snapshot.jobs.filter(job=>downloads.selected.has(job.id)&&['error','cancelled'].includes(job.state)).map(job=>job.id),'retry');break;
    case 'clear-job-selection':downloads.selected.clear();renderCurrent();break;
    case 'copy-job-error':{const job=snapshot.jobs.find(job=>job.id===id);if(job?.error)await copyText(job.error.technical);break;}
    case 'library-filter':{const value=button.dataset.filter;if(value==='all'||value==='video'||value==='audio'||value==='image'){library.filter=value;library.offset=0;renderCurrent();}break;}
    case 'library-layout':if(button.dataset.layout==='grid'||button.dataset.layout==='list'){library.layout=button.dataset.layout;renderCurrent();}break;
    case 'library-next':library.offset+=60;renderCurrent();required('#main').scrollTop=0;break;
    case 'library-previous':library.offset=Math.max(0,library.offset-60);renderCurrent();required('#main').scrollTop=0;break;
    case 'import-files':await importMedia(false);break;
    case 'import-folder':await importMedia(true);break;
    case 'inspect-media':library.detailId=id;if(page!=='library')navigate('library');else renderCurrent();break;
    case 'close-inspector':closeInspector();break;
    case 'open-media':await command('open_media',{id,folder:false});break;
    case 'reveal-media':await command('open_media',{id,folder:true});break;
    case 'copy-path':{const item=snapshot.library.find(item=>item.id===id);if(item)await copyText(item.path);break;}
    case 'locate-media':thumbnailUrls.delete(id);applySnapshot(await command<Snapshot>('locate_media',{id}));renderInspector(true);break;
    case 'remove-media':applySnapshot(await command<Snapshot>('remove_library',{id}));notify(t("Quitado de la biblioteca. El archivo se conserva en el disco.", "Removed from library. The file is kept on disk."),'success');break;
    case 'tool-media':setToolInput([id]);break;
    case 'tools-selected':setToolInput([...library.selected]);break;
    case 'clear-library-selection':library.selected.clear();renderCurrent();break;
    case 'tool-kind':{const kind=button.dataset.kind;if(isTool(kind)){tools.kind=kind;tools.options=defaultToolOptions(kind);renderCurrent();}break;}
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
    case 'clear-cache':await command('clear_cache');preferences.cache=await command<{bytes:number;files:number}>('cache_status');renderCurrent();notify(t("Caché de miniaturas revisada. Los archivos multimedia y las sesiones se conservan.", "Thumbnail cache reviewed. Media files and sessions are preserved."),'success');break;
    case 'revoke-session':await command('revoke_session',{site:button.dataset.site??""});await loadSettingsSection();notify(t("Sesión eliminada.", "Session removed."),'success');break;
    case 'copy-pairing':if(preferences.pairing)await copyText(preferences.pairing.token);break;
    case 'revoke-bridge':await command('bridge_revoke',{extensionId:id});preferences.pairing=null;await loadSettingsSection();break;
  }
}
document.addEventListener('input',event=>{
  const target=event.target;if(!(target instanceof HTMLInputElement||target instanceof HTMLTextAreaElement))return;
  if(target.id==='url')updateSourceInput(target.value,false);
  else if(target.id==='job-search'){downloads.query=target.value;renderCurrent();}
  else if(target.id==='library-search'){library.query=target.value;library.offset=0;renderCurrent();}
  else if(target.id==='tool-picker-search'){const query=target.value.toLocaleLowerCase();for(const item of document.querySelectorAll<HTMLElement>('[data-picker-name]'))item.hidden=!item.dataset.pickerName?.includes(query);}
});
document.addEventListener('change',event=>{
  const target=event.target;
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
    if(target.id==='output-format'&&(value==='mp4'||value==='source'||value==='mp3'||value==='m4a'||value==='audio'||value==='opus')){home.format=value;home.quality="";renderHome();}
    else if(target.id==='quality')home.quality=value;
    else if(target.id==='audio-bitrate')home.audioBitrate=value;
    else if(target.id==='collection-filter'){library.collection=value;library.offset=0;renderCurrent();}
    else if(target.id==='library-sort'&&(value==='newest'||value==='name'||value==='duration'||value==='size')){library.sort=value;library.offset=0;renderCurrent();}
  }
});
document.addEventListener('submit',event=>{const form=event.target;if(form instanceof HTMLFormElement){event.preventDefault();void submitForm(form).catch(showError);}});
async function submitForm(form: HTMLFormElement): Promise<void> {
  switch(form.id){
    case 'source-form':await inspect();break;
    case 'tool-form':{const button=form.querySelector<HTMLButtonElement>('button[type=submit]');if(button?.disabled)return;if(button)button.disabled=true;try{await queueTool();}finally{if(button)button.disabled=false;}break;}
    case 'general-settings':{const language=inputValue('language');if(language==='es'||language==='en')await saveSettings({...snapshot.settings,language,trayEnabled:checked('tray-enabled'),closeToTray:checked('tray-enabled')&&checked('close-to-tray'),startWithWindows:checked('start-with-windows')});notify(t("Ajustes guardados.", "Settings saved."),'success');break;}
    case 'download-settings':await saveSettings({...snapshot.settings,networkLimit:Number(inputValue('network-limit')),processingLimit:Number(inputValue('processing-limit'))});notify(t("Ajustes guardados.", "Settings saved."),'success');break;
    case 'appearance-settings':await saveSettings({...snapshot.settings,reducedMotion:checked('reduced-motion'),sidebarExpanded:checked('expanded-sidebar')});notify(t("Ajustes guardados.", "Settings saved."),'success');break;
    case 'organize-media':{const split=(value:string)=>[...new Set(value.split(',').map(value=>value.trim()).filter(Boolean))].slice(0,50);applySnapshot(await command<Snapshot>('update_library',{id:form.dataset.id??"",collections:split(inputValue('media-collections')),tags:split(inputValue('media-tags'))}));notify(t("Organización guardada.", "Organization saved."),'success');break;}
    case 'session-form':await command('import_session',{site:inputValue('session-site')});await loadSettingsSection();notify(t("Sesión importada con protección de Windows.", "Session imported with Windows protection."),'success');break;
    case 'bridge-form':preferences.pairing=await command<{token:string;port:number}>('bridge_pair',{extensionId:inputValue('extension-id')});await loadSettingsSection();break;
  }
}
document.addEventListener('keydown',event=>{
  if(event.target instanceof HTMLTextAreaElement&&event.target.id==='url'&&event.key==='Enter'&&!event.shiftKey){event.preventDefault();void inspect().catch(showError);return;}
  if(event.key==='Escape'&&!required<HTMLDialogElement>('#detail-dialog').open){closeInspector();return;}
  if(!desktop||!event.ctrlKey||event.altKey||event.shiftKey)return;
  if(event.key.toLowerCase()==='l'){event.preventDefault();navigate('home');required<HTMLTextAreaElement>('#url').focus();return;}
  if(event.target instanceof HTMLInputElement||event.target instanceof HTMLTextAreaElement||event.target instanceof HTMLSelectElement)return;
  const pages:Record<string,Page>={'1':'home','2':'downloads','3':'library','4':'tools',',':'settings'};const target=pages[event.key];if(target){event.preventDefault();navigate(target);}
});
required<HTMLDialogElement>('#detail-dialog').addEventListener('cancel',event=>{event.preventDefault();closeInspector();});
required<HTMLDialogElement>('#detail-dialog').addEventListener('click',event=>{if(event.target===event.currentTarget)closeInspector();});
document.addEventListener('error',event=>{const target=event.target;if(target instanceof HTMLElement&&target.dataset.localPlayer){const fallback=target.closest('.media-player')?.querySelector<HTMLElement>('#playback-error');if(fallback)fallback.hidden=false;}else if(target instanceof HTMLImageElement&&target.classList.contains('preview-image'))target.hidden=true;},true);
window.addEventListener('resize',()=>renderInspector());

async function receivePaths(paths: string[]): Promise<void> {
  required('#drop-indicator').hidden=true;
  const textFiles=paths.filter(path=>path.toLowerCase().endsWith('.txt'));const mediaFiles=paths.filter(path=>!path.toLowerCase().endsWith('.txt'));
  for(const path of textFiles)await importUrlFile(path);
  if(mediaFiles.length){const previous=new Set(snapshot.library.map(item=>item.id));const next=await command<Snapshot>('import_paths',{paths:mediaFiles});for(const item of next.library)if(!previous.has(item.id)&&item.kind!=='image')tools.ids.add(item.id);snapshot=next;navigate(tools.ids.size?'tools':'library');notify(t("Archivo importado. El original no se ha modificado.", "File imported. The original was not modified."),'success');}
}
document.addEventListener('dragover',event=>{event.preventDefault();required('#drop-indicator').hidden=false;});
document.addEventListener('dragleave',event=>{if(event.relatedTarget===null)required('#drop-indicator').hidden=true;});
document.addEventListener('drop',event=>{event.preventDefault();required('#drop-indicator').hidden=true;void(async()=>{const file=event.dataTransfer?.files[0];if(file){if(file.name.toLowerCase().endsWith('.txt')){if(file.size>1024*1024)throw new Error(t("El archivo de enlaces supera el límite de 1 MB.", "The links file exceeds the 1 MB limit."));navigate('home');updateSourceInput(await file.text());}else notify(t("Arrastra archivos desde el explorador a la aplicación de escritorio.", "Drop files from Explorer into the desktop application."));return;}const value=event.dataTransfer?.getData('text/uri-list')||event.dataTransfer?.getData('text/plain');if(value){navigate('home');updateSourceInput(value);}})().catch(showError);});

renderCurrent();
if(desktop){
  const nativeWindow=getCurrentWindow();
  required('#minimize').addEventListener('click',()=>void nativeWindow.minimize().catch(showError));
  required('#maximize').addEventListener('click',()=>void nativeWindow.toggleMaximize().catch(showError));
  required('#close').addEventListener('click',()=>void nativeWindow.close().catch(showError));
  const updateMaximize=async()=>{const maximized=await nativeWindow.isMaximized();const button=required('#maximize');button.dataset.maximized=String(maximized);const label=maximized?t("Restaurar", "Restore"):t("Maximizar", "Maximize");button.setAttribute('aria-label',label);button.title=label;};
  try{
    await nativeWindow.onResized(updateMaximize);await updateMaximize();
    await nativeWindow.onDragDropEvent(event=>{if(event.payload.type==='drop')void receivePaths(event.payload.paths).catch(showError);else required('#drop-indicator').hidden=event.payload.type!=='enter'&&event.payload.type!=='over';});
    await listen<Job>('job-update',({payload})=>{const current=snapshot.jobs.find(job=>job.id===payload.id);if(current&&(current.runId>payload.runId||(current.runId===payload.runId&&current.updatedAt>payload.updatedAt)))return;snapshot.jobs=current?snapshot.jobs.map(job=>job.id===payload.id?payload:job):[...snapshot.jobs,payload];scheduleJobsRender();});
    await listen('library-update',()=>void refresh().catch(showError));
    await listen<string>('app-warning',({payload})=>notify(payload));
    const state=await command<Startup>('startup');snapshot=state;enginesReady=state.enginesReady;renderCurrent();if(state.engineError)notify(t("Algunos motores requieren preparación. Revisa Motores en Ajustes.", "Some engines require preparation. Review Engines in Settings."),'info',state.engineError);
  }catch(error){showError(error);}
}else{for(const button of document.querySelectorAll<HTMLButtonElement>('.window-actions button'))button.disabled=true;}
required<HTMLTextAreaElement>('#url').focus({preventScroll:true});
window.addEventListener('beforeunload',()=>{disposeGraphics?.();clearTimeout(debounce);clearTimeout(pendingRender);});
