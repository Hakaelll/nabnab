import { escapeHtml as h, type Copy, type Job, type OutputFormat, type Snapshot } from "./contracts";
import { batchCandidates, batchCounts, batchVisible, type Draft, type DraftItem } from "./batch-types";
import { defaultWorkOptions, type Preset, type SubtitleMode, type WorkOptions } from "./options";
import { readScheduling, schedulingFields } from "./scheduling-types";
import { durationLabel } from "./presentation";
import { action, icon, option } from "./views-common";

type ProductSnapshot = Snapshot & { drafts?: Draft[]; presets?: Preset[] };
interface BatchHooks {
  command: <T>(name: string, args?: Record<string, unknown>) => Promise<T>;
  snapshot: () => ProductSnapshot;
  refresh: () => Promise<void>;
  notify: (text: string, kind?: "info" | "success" | "error") => void;
  showError: (error: unknown) => void;
  t: Copy;
  render: () => void;
  onFollowSource?: (url: string) => void;
}

function format(value: string): OutputFormat {
  switch (value) { case "mp3": case "m4a": case "opus": case "source": case "audio": return value; default: return "mp4"; }
}
function subtitleMode(value: string): SubtitleMode {
  switch (value) { case "sidecar": case "embed": case "only": return value; default: return "none"; }
}
function optionName(name:string,t:Copy):string {
  if(name.startsWith('tag:'))return `${t('Etiqueta','Tag')} ${name.slice(4)}`;
  const labels:Record<string,[string,string]>={format:['Perfil','Profile'],height:['Altura','Height'],audioBitrate:['Bitrate de audio','Audio bitrate'],additionalAudio:['Audio adicional','Additional audio'],subtitleLanguages:['Idiomas de subtítulos','Subtitle languages'],subtitleMode:['Salida de subtítulos','Subtitle output'],automaticSubtitles:['Pistas automáticas','Automatic tracks'],relativeFolder:['Carpeta','Folder'],filenameTemplate:['Nombre de archivo','Filename'],collection:['Colección','Collection'],tags:['Etiquetas','Tags'],clip:['Fragmento','Clip'],priority:['Prioridad','Priority'],schedule:['Programación','Schedule']};
  const label=labels[name];return label?t(...label):name;
}

export function renderWorkOptions(prefix: string, values: WorkOptions, timezone: string, t: Copy, languages: readonly string[] = []): string {
  const field = (suffix: string) => `${h(prefix)}-${suffix}`;
  return `<div class="work-options">
    <fieldset><legend>${t("Contenido", "Content")}</legend><label>${t("Perfil multimedia", "Media profile")}<select id="${field("format")}">${[["mp4",t("Vídeo compatible", "Compatible video")],["source",t("Vídeo original", "Original video")],["audio",t("Audio original", "Original audio")],["mp3","MP3"],["m4a","M4A"],["opus","Opus"]].map(([value,label])=>option(value,label,values.format)).join("")}</select></label>
    <label>${t("Altura elegida, px", "Selected height, px")}<input id="${field("height")}" type="number" min="1" max="16384" placeholder="${t("Mejor disponible", "Best available")}" value="${values.height ?? ""}"></label>
    <label>${t("Crear también audio", "Also create audio")}<select id="${field("additional-audio")}">${option("",t("Sin salida adicional", "No additional output"),values.additionalAudio??"")}${[["audio",t("Audio original", "Original audio")],["mp3","MP3"],["m4a","M4A"],["opus","Opus"]].map(([value,label])=>option(value,label,values.additionalAudio??"")).join("")}</select></label></fieldset>
    <fieldset><legend>${t("Salida", "Output")}</legend><label>${t("Subtítulos", "Subtitles")}<select id="${field("subtitle-mode")}">${[["none",t("Sin subtítulos", "No subtitles")],["sidecar",t("Archivos junto al vídeo", "Files beside video")],["embed",t("Dentro del contenedor", "Inside container")],["only",t("Solo subtítulos", "Subtitles only")]].map(([value,label])=>option(value,label,values.subtitleMode)).join("")}</select></label>
    <label>${t("Idiomas, separados por coma", "Languages, separated by commas")}<input id="${field("languages")}" value="${h(values.subtitleLanguages.join(", "))}" placeholder="es, en"></label>
    ${languages.length ? `<small>${t("Pistas disponibles", "Available tracks")}: ${h(languages.join(", "))}</small>` : `<small>${t("Se comprobarán las pistas disponibles de cada origen. No se generan subtítulos.", "Each source's available tracks will be checked. Subtitles are not generated.")}</small>`}
    <label class="check-label"><input id="${field("automatic")}" type="checkbox" ${values.automaticSubtitles?"checked":""}>${t("Usar pistas automáticas del proveedor", "Use provider automatic tracks")}</label>
    <div class="form-row"><label>${t("Inicio del fragmento, s", "Clip start, s")}<input id="${field("clip-start")}" type="number" min="0" step="0.001" value="${values.clip?.start??""}"></label><label>${t("Fin, s", "End, s")}<input id="${field("clip-end")}" type="number" min="0" step="0.001" value="${values.clip?.end??""}"></label></div>
    <label class="check-label"><input id="${field("precise")}" type="checkbox" ${values.clip?.precise?"checked":""}>${t("Corte preciso, puede recodificar", "Precise cut, may re-encode")}</label>
    <small>${t("El corte rápido puede ajustarse a fotogramas clave. Un fragmento puede requerir descargar más contenido o el archivo completo.", "Fast cuts may align to keyframes. A clip may require more transfer or the full file.")}</small></fieldset>
    <fieldset><legend>${t("Organización", "Organization")}</legend><label>${t("Carpeta relativa", "Relative folder")}<input id="${field("folder")}" value="${h(values.relativeFolder)}" placeholder="Tutoriales/{creador}"></label><label>${t("Nombre de archivo", "Filename")}<input id="${field("filename")}" value="${h(values.filenameTemplate)}"></label><small>{plataforma}, {creador}, {lista}, {fecha}, {titulo}, {id}</small><label>${t("Colección", "Collection")}<input id="${field("collection")}" value="${h(values.collection??"")}"></label><label>${t("Etiquetas, separadas por coma", "Tags, separated by commas")}<input id="${field("tags")}" value="${h(values.tags.join(", "))}"></label></fieldset>
    <fieldset><legend>${t("Programación", "Scheduling")}</legend>${schedulingFields(prefix,values.priority,values.schedule,timezone,t)}</fieldset></div>`;
}

export function readWorkOptions(prefix: string, base: WorkOptions, timezone: string, root: ParentNode = document): WorkOptions {
  const input = (suffix: string) => root.querySelector<HTMLInputElement | HTMLSelectElement>(`#${CSS.escape(prefix)}-${suffix}`);
  const value = (suffix: string) => input(suffix)?.value ?? "";
  const checked = (suffix: string) => { const element = input(suffix); return element instanceof HTMLInputElement && element.checked; };
  const start = value("clip-start"), end = value("clip-end");
  if ((!!start !== !!end) || (start && (!Number.isFinite(Number(start)) || !Number.isFinite(Number(end)) || Number(start)<0 || Number(end)<=Number(start)))) throw new Error("Choose both a valid clip start and a later end.");
  const output = format(value("format"));
  const audio = value("additional-audio");
  const subtitles = subtitleMode(value("subtitle-mode"));
  const height = value("height") ? Number(value("height")) : null;
  const video = output === "mp4" || output === "source";
  return {...base,format:output,height:video?height:null,audioBitrate:output==="mp3"?(base.audioBitrate??192):null,
    additionalAudio:video&&audio&&subtitles!=="only"?format(audio):null,subtitleMode:subtitles,
    subtitleLanguages:[...new Set(value("languages").split(",").map(item=>item.trim()).filter(Boolean))],automaticSubtitles:checked("automatic"),
    relativeFolder:value("folder"),filenameTemplate:value("filename"),collection:value("collection").trim()||null,
    tags:[...new Set(value("tags").split(",").map(tag=>tag.trim()).filter(Boolean))],
    clip:start&&end?{start:Number(start),end:Number(end),precise:checked("precise")}:null,...readScheduling(prefix,timezone,root)};
}

function stateLabel(state: DraftItem["state"], t: Copy): string {
  const labels = {pending:["Pendiente de analizar","Awaiting analysis"],ready:["Preparado","Ready"],available:["Ya disponible","Already available"],active:["Trabajo activo","Active job"],authentication:["Requiere sesión","Session required"],unsupported:["No compatible","Unsupported"]} satisfies Record<DraftItem["state"], [string,string]>;
  return t(...labels[state]);
}

export function createBatchController(hooks: BatchHooks) {
  let draft: Draft | null = null;
  let query = "";
  let selectedItem: string | null = null;
  let generation = 0;
  let running = false;
  let timezone = "UTC";
  let presetEditor: string | null = null;
  const busy = new Set<string>();
  const chapterSelections = new Map<string, Set<number>>();
  type EditorValue = { kind: "value"; value: string } | { kind: "checked"; checked: boolean };
  const editors = new Map<string, { revision: number; fields: Map<string, EditorValue> }>();
  let editorRevision = 0;
  let editRevision = 0;
  let acknowledgedRevision = 0;
  let saving: Promise<void> = Promise.resolve();
  const presets = () => hooks.snapshot().presets ?? [];
  const valuesFor = (item?: DraftItem) => item?.options ?? draft?.options ?? item?.resolved?.values ?? presets().find(preset=>preset.id===draft?.presetId)?.options ?? defaultWorkOptions();
  const changed = () => hooks.render();
  const optionsKey = () => `${draft?.id}:options:${selectedItem??"common"}`;
  const metaKey = () => `${draft?.id}:meta`;
  const presetKey = () => `preset:${presetEditor??""}`;
  const controlKey = (control: HTMLInputElement | HTMLSelectElement) => control.id || `${control.getAttribute("name")??""}:${control.value}`;
  function rememberEditor(target: HTMLElement): boolean {
    if (!(target instanceof HTMLInputElement || target instanceof HTMLSelectElement)) return false;
    const field = target.id || target.getAttribute("name") || "";
    const key = field.startsWith("batch-options-") && draft ? optionsKey()
      : (field === "batch-name" || field === "batch-exploration-limit") && draft ? metaKey()
      : field.startsWith("preset-options-") || field === "preset-name" ? presetKey() : null;
    if (!key) return false;
    const state = editors.get(key) ?? { revision: 0, fields: new Map<string, EditorValue>() };
    state.fields.set(controlKey(target), target instanceof HTMLInputElement && target.type === "checkbox"
      ? { kind: "checked", checked: target.checked } : { kind: "value", value: target.value });
    state.revision = ++editorRevision;
    editors.set(key, state);
    return true;
  }
  function editorText(key: string, field: string, fallback: string | number): string {
    const stored = editors.get(key)?.fields.get(field);
    return stored?.kind === "value" ? stored.value : String(fallback);
  }
  function finishEditor(key: string, revision: number | undefined, fields?: string[]): void {
    const state = editors.get(key);
    if (!state || state.revision !== revision) return;
    if (fields) { for (const field of fields) state.fields.delete(field); if (!state.fields.size) editors.delete(key); }
    else editors.delete(key);
  }
  function renderEditor(key: string, prefix: string, values: WorkOptions, languages: readonly string[] = []): string {
    const markup = renderWorkOptions(prefix, values, timezone, hooks.t, languages);
    const state = editors.get(key);
    if (!state) return markup;
    const template = document.createElement("template"); template.innerHTML = markup;
    for (const control of template.content.querySelectorAll<HTMLInputElement | HTMLSelectElement>("input, select")) {
      const stored = state.fields.get(controlKey(control));
      if (!stored) continue;
      if (stored.kind === "checked") control.toggleAttribute("checked", stored.checked);
      else if (control instanceof HTMLSelectElement) for (const entry of control.options) entry.toggleAttribute("selected", entry.value === stored.value);
      else control.setAttribute("value", stored.value);
    }
    const schedule = template.content.querySelector<HTMLSelectElement>(`#${CSS.escape(prefix)}-schedule-kind`);
    for (const fields of template.content.querySelectorAll<HTMLElement>("[data-schedule-fields]")) fields.hidden = fields.dataset.scheduleFields !== schedule?.value;
    return template.innerHTML;
  }
  function accept(next: Draft, savedRevision?: number): void {
    if (draft?.id !== next.id) return;
    if (savedRevision !== undefined) acknowledgedRevision = Math.max(acknowledgedRevision, savedRevision);
    if (next.updatedAt < draft.updatedAt) return;
    if (acknowledgedRevision < editRevision) {
      // Inspection metadata may arrive while a newer selection or options save is still pending.
      next.name = draft.name; next.savedForLater = draft.savedForLater; next.presetId = draft.presetId;
      next.options = draft.options; next.explorationLimit = draft.explorationLimit;
      for (const incoming of next.items) {
        const local = draft.items.find(item => item.id === incoming.id);
        if (local) { incoming.picked = local.picked; incoming.excluded = local.excluded; incoming.options = local.options; }
      }
    }
    draft = next;
  }

  async function save(): Promise<void> {
    if(!draft)return;
    const input=structuredClone(draft);
    const revision = ++editRevision;
    saving=saving.catch(()=>{}).then(async()=>{ const next=await hooks.command<Draft>("batch_save",{draft:input});accept(next,revision); });
    await saving;
  }

  async function analyze(): Promise<void> {
    if(!draft||running)return;
    running=true; const pass=++generation;
    async function worker():Promise<void>{
      while(draft&&running&&generation===pass){
        const visible=new Set(batchVisible(draft.items,query).map(item=>item.id));
        const next=draft.items.filter(item=>item.state==="pending"&&!item.excluded&&!busy.has(item.id)).sort((a,b)=>Number(b.picked)-Number(a.picked)||Number(visible.has(b.id))-Number(visible.has(a.id)))[0];
        if(!next)break;
        const id=draft.id;busy.add(next.id);changed();
        try{const result=await hooks.command<Draft>("batch_inspect",{draftId:id,itemId:next.id});if(generation===pass)accept(result);}
        catch(error){if(generation===pass){hooks.showError(error);running=false;}}
        finally{busy.delete(next.id);if(generation===pass)changed();}
      }
    }
    await Promise.all([worker(),worker()]);if(generation===pass){running=false;changed();}
  }

  async function prepare(text:string, presetId:string|null=null, savedForLater=false):Promise<void>{
    if(!batchCandidates(text).length)return;
    if(draft)await close();
    draft=await hooks.command<Draft>("batch_create",{text,name:null,presetId});
    editRevision=0;acknowledgedRevision=0;editors.clear();chapterSelections.clear();
    query="";selectedItem=null;
    try{timezone=await hooks.command<string>("local_timezone");}catch{/* UTC remains an explicit fallback. */}
    if(savedForLater){draft.savedForLater=true;await save();await hooks.refresh();}
    changed();if(!savedForLater)void analyze();
  }

  async function open(value:Draft):Promise<void>{
    if(draft?.id===value.id)accept(structuredClone(value));
    else{if(draft)await close();draft=structuredClone(value);editRevision=0;acknowledgedRevision=0;}
    query="";selectedItem=null;editors.clear();changed();
    try{timezone=await hooks.command<string>("local_timezone");}catch{/* UTC remains an explicit fallback. */}
    changed();
  }

  async function close():Promise<void>{
    running=false;++generation;
    if(draft){await hooks.command<Draft>("batch_cancel",{draftId:draft.id});await save();}
    busy.clear();draft=null;editors.clear();chapterSelections.clear();changed();
  }

  async function applyPresetToSelection(id: string): Promise<void> {
    const preset = presets().find(preset => preset.id === id);
    if (!preset) throw new Error(hooks.t("El ajuste ya no existe.", "The preset no longer exists."));
    if (!draft) throw new Error(hooks.t("Prepara un lote antes de aplicar el ajuste.", "Prepare a batch before applying the preset."));
    for (const item of draft.items) if (item.picked && !item.excluded && !item.list && !["unsupported", "authentication"].includes(item.state)) {
      item.options = structuredClone(preset.options);
      editors.delete(`${draft.id}:options:${item.id}`);
    }
    await save(); changed();
  }

  function editPreset(id: string): void {
    if (!presets().some(preset => preset.id === id)) throw new Error(hooks.t("El ajuste ya no existe.", "The preset no longer exists."));
    presetEditor = id; changed();
  }

  function render():string{
    if(!draft)return "";
    const t=hooks.t;const items=batchVisible(draft.items,query);const counts=batchCounts(draft.items);
    const item=draft.items.find(entry=>entry.id===selectedItem);const values=valuesFor(item);
    const prepared=draft.items.filter(entry=>entry.picked&&!entry.excluded&&!entry.list&&entry.state==="ready");
    const outputLabels=(entry:DraftItem):string[]=>{const values=valuesFor(entry);return [...(values.subtitleMode==="only"?[]:[values.format.toUpperCase()]),...(values.additionalAudio?[values.additionalAudio.toUpperCase()]:[]),...(["sidecar","only"].includes(values.subtitleMode)?values.subtitleLanguages.map(language=>`${t("Subtítulos", "Subtitles")} ${language}`):[])];};
    return `<section class="batch-workspace source-preview" aria-label="${t("Preparador de lotes", "Batch preparation")}">
      <div class="section-heading"><div><h2>${h(draft.name)}</h2><p role="status">${draft.items.filter(entry=>!entry.list).length} ${t("elementos", "items")} · ${busy.size?`${busy.size} ${t("analizando", "analyzing")}`:t("Borrador local", "Local draft")}${draft.savedForLater?` · ${t("Guardado para después", "Saved for later")}`:""}</p></div>${action("batch-close",t("Cerrar", "Close"),"close","quiet")}</div>
      <div class="batch-toolbar"><label class="search-field">${icon("search")}<input id="batch-search" type="search" value="${h(query)}" placeholder="${t("Buscar en el lote", "Search this batch")}"></label><select id="batch-selection" aria-label="${t("Seleccionar elementos", "Select items")}"><option value="">${t("Seleccionar…", "Select…")}</option><option value="visible">${t("Resultados visibles", "Visible results")}</option><option value="all">${t("Todo el lote", "Entire batch")}</option><option value="ready">${t("Solo preparados", "Ready only")}</option><option value="none">${t("Ninguno", "None")}</option></select>${action("batch-exclude",t("Excluir seleccionados", "Exclude selected"),undefined,"quiet")}${running?action("batch-cancel",t("Cancelar análisis", "Cancel analysis"),"pause","quiet"):action("batch-analyze",t("Analizar pendientes", "Analyze pending"),"retry","quiet")}</div>
      <div class="batch-columns"><div class="batch-results"><ul class="batch-items">${items.map(entry=>`<li class="batch-item ${selectedItem===entry.id?"selected":""}"><label class="check-label"><input type="checkbox" data-batch-pick="${h(entry.id)}" aria-label="${h(t("Seleccionar", "Select")+" "+entry.title)}" ${entry.picked?"checked":""} ${entry.list?"disabled":""}></label>${entry.thumbnail?`<img src="${h(entry.thumbnail)}" loading="lazy" alt="" referrerpolicy="no-referrer">`:`<span class="batch-thumb" aria-hidden="true">${icon(entry.list?"list":valuesFor(entry).format==="mp4"||valuesFor(entry).format==="source"?"video":"audio")}</span>`}<button class="batch-item-title" type="button" data-action="batch-inspect" data-id="${h(entry.id)}"><strong>${h(entry.title)}</strong><small>${h(entry.provider)}${entry.duration!==null?` · ${durationLabel(entry.duration)}`:""} · ${h((entry.resolved?.values.format??valuesFor(entry).format).toUpperCase())}</small></button><span class="batch-item-state">${busy.has(entry.id)?t("Analizando…", "Analyzing…"):stateLabel(entry.state,t)}${entry.options?`<small>${t("Opciones propias", "Custom options")}</small>`:""}</span>${entry.list?`<div class="batch-list-progress"><span>${entry.list.loaded} ${t("cargados", "loaded")} · ${entry.list.complete?t("Exploración completa", "Exploration complete"):t("Lista parcial", "Partial list")}</span>${!entry.list.complete?action("batch-more",t("Cargar 50 más", "Load 50 more"),undefined,"quiet",`data-id="${h(entry.id)}" ${busy.has(entry.id)?"disabled":""}`):""}${entry.provider==="YouTube"?action("batch-follow",t("Seguir fuente", "Follow source"),undefined,"quiet",`data-id="${h(entry.id)}"`):""}</div>`:""}</li>`).join("")}</ul>${!items.length?`<p class="empty-state">${t("No hay resultados visibles. Cambia la búsqueda o restaura los excluidos.", "No visible results. Change the search or restore excluded items.")}</p>`:""}${counts.excluded?action("batch-restore",`${t("Restaurar excluidos", "Restore excluded")} · ${counts.excluded}`,undefined,"quiet"):""}</div>
      <details class="batch-options" open><summary>${item?t("Opciones del elemento", "Item options"):t("Opciones del lote", "Batch options")}</summary><div class="batch-options-body">${item?`<div class="section-heading"><h3>${h(item.title)}</h3>${action("batch-common",t("Volver al lote", "Back to batch"),undefined,"quiet")}</div>${item.options?action("batch-reset-item",t("Restablecer al ajuste del lote", "Reset to batch preset"),undefined,"quiet"):""}${item.error?`<p class="inline-error">${h(item.error)}</p>`:""}`:`<label>${t("Ajuste guardado", "Saved preset")}<select id="batch-preset">${option("",t("Valores generales y reglas", "General settings and rules"),draft.presetId??"")}${presets().map(preset=>option(preset.id,`${preset.name} · v${preset.revision}`,draft?.presetId??"")).join("")}</select></label>`}
      ${!item?`<label>${t("Nombre del borrador", "Draft name")}<input id="batch-name" maxlength="160" value="${h(editorText(metaKey(),"batch-name",draft.name))}"></label>`:action("batch-reanalyze",t("Volver a analizar", "Analyze again"),"retry","quiet",busy.has(item.id)?"disabled":"")}
      ${item?.media?.chapters?.length?`<details class="batch-chapters"><summary>${t("Capítulos disponibles", "Available chapters")} · ${item.media.chapters.length}</summary><ul>${item.media.chapters.map((chapter,index)=>`<li><label class="check-label"><input id="batch-chapter-${h(item.id)}-${index}" type="checkbox" data-batch-chapter="${index}" data-item-id="${h(item.id)}" ${chapterSelections.get(item.id)?.has(index)?"checked":""}><span><strong>${h(chapter.title)}</strong><small>${durationLabel(chapter.start)}–${durationLabel(chapter.end)}</small></span></label></li>`).join("")}</ul>${action("batch-chapters",t("Preparar capítulos seleccionados", "Prepare selected chapters"),"scissors","secondary",chapterSelections.get(item.id)?.size?"":"disabled")}<p class="field-note">${t("Usa las opciones aplicadas al elemento. El corte rápido puede ajustarse a fotogramas clave y requerir más transferencia o el archivo completo. Puedes elegir corte preciso en cada salida.", "Uses this item's applied options. Fast cuts may align to keyframes and require more transfer or the full file. You can choose precise cuts for each output.")}</p></details>`:item?.media?`<p class="field-note">${t("El proveedor no ha ofrecido capítulos. Puedes definir un fragmento manual.", "The provider did not offer chapters. You can define a manual clip.")}</p>`:""}
      ${renderEditor(optionsKey(),"batch-options",values,item?.media?.subtitleLanguages??[])}${action("batch-apply-options",t("Aplicar opciones", "Apply options"),"check")}
      ${item?.destination?`<div class="batch-summary"><strong>${t("Destino previsto", "Expected destination")}</strong><code>${h(item.destination)}</code><small>${t("Tamaño desconocido", "Size unknown")}</small>${item.resolved?`<small>${Object.entries(item.resolved.origins).map(([field,origin])=>`${h(optionName(field,t))}: ${h(origin)}`).join("<br>")}</small>`:""}</div>`:""}
      <label>${t("Límite de exploración antes de ampliar", "Exploration limit before expanding")}<input id="batch-exploration-limit" type="number" min="50" max="10000" step="50" value="${h(editorText(metaKey(),"batch-exploration-limit",draft.explorationLimit))}"></label>${action("batch-limit",t("Actualizar límite", "Update limit"),undefined,"quiet")}</div></details></div>
      ${prepared.length?`<details class="batch-plan"><summary>${t("Archivos y operaciones previstos", "Expected files and operations")} · ${prepared.reduce((total,entry)=>total+outputLabels(entry).length,0)}</summary><ul>${prepared.slice(0,50).map(entry=>`<li><strong>${h(entry.title)}</strong><span>${h(outputLabels(entry).join(" · "))}${valuesFor(entry).subtitleMode==="embed"?` · ${t("subtítulos integrados", "embedded subtitles")}`:""}</span><code>${h(entry.destination??hooks.snapshot().settings.folder)}</code></li>`).join("")}</ul>${prepared.length>50?`<p>${t("Se muestran los primeros 50. Inspecciona cada elemento para ver su destino.", "Showing the first 50. Inspect any item to see its destination.")}</p>`:""}<p class="field-note">${t("Se muestra el nombre base. La extensión final depende del formato real. Tamaño desconocido. Las salidas relacionadas se pueden reintentar por separado.", "The base filename is shown. The final extension depends on the actual format. Size unknown. Related outputs can be retried separately.")}</p></details>`:""}
      <footer class="batch-footer"><div><strong>${counts.queueable} ${t("elementos para añadir", "items to add")}</strong><small>${counts.attention} ${t("seleccionados requieren intervención", "selected items need attention")} · ${t("Tamaño desconocido", "Size unknown")}</small><small>${values.additionalAudio?t("Vídeo y audio serán operaciones relacionadas. Podrás reintentar cada salida por separado.", "Video and audio are related operations. Each output can be retried separately."):values.format==="source"||values.format==="audio"?t("Conserva los códecs disponibles.", "Keeps available codecs."):t("Puede convertir las pistas que lo requieran.", "May convert tracks when required.")}</small></div>${action("batch-save-later",t("Guardar para después", "Save for later"),"library")}${action("batch-enqueue",t("Añadir a la cola", "Add to queue"),"downloads","primary",!counts.queueable?"disabled":"")}</footer></section>`;
  }

  function renderLater():string{
    const t=hooks.t;const drafts=hooks.snapshot().drafts??[];
    return `<div class="section-heading"><div><h2>${t("Para después", "For later")}</h2><p>${t("Enlaces y borradores guardados. No son archivos descargados.", "Saved links and drafts. These are not downloaded files.")}</p></div>${action("batch-new",t("Preparar lote", "Prepare batch"),"add")}</div>${drafts.length?`<ul class="saved-batches">${drafts.map(value=>`<li><div><strong>${h(value.name)}</strong><small>${value.items.filter(item=>!item.excluded&&!item.list).length} ${t("enlaces", "links")} · ${new Date(value.updatedAt).toLocaleDateString()}${value.savedForLater?"":` · ${t("borrador", "draft")}`}</small></div>${action("batch-open",t("Abrir borrador", "Open draft"),undefined,"secondary",`data-id="${h(value.id)}"`)}${action("batch-delete",t("Quitar", "Remove"),undefined,"quiet",`data-id="${h(value.id)}"`)}</li>`).join("")}</ul>`:`<div class="empty-state">${icon("link")}<h3>${t("Guarda enlaces para otro momento", "Save links for another time")}</h3><p>${t("Prepara un lote en Inicio y elige Guardar para después.", "Prepare a batch in Home and choose Save for later.")}</p></div>`}`;
  }

  function renderPresets():string{
    const t=hooks.t;const preset=presets().find(value=>value.id===presetEditor);
    const values=preset?.options??defaultWorkOptions();
    return `<div class="section-heading"><div><h2>${t("Ajustes reutilizables", "Reusable presets")}</h2><p>${t("Cada trabajo conserva una copia de sus opciones. Editar un ajuste afecta a trabajos futuros.", "Each job keeps a copy of its options. Editing a preset affects future jobs.")}</p></div>${action("batch-preset-new",t("Nuevo ajuste", "New preset"),"add")}</div><div class="preset-rows">${presets().map(value=>`<div class="preset-row"><strong>${h(value.name)}</strong><span>${h(value.options.format.toUpperCase())} · v${value.revision}</span>${action("batch-preset-edit",t("Editar", "Edit"),undefined,"quiet",`data-id="${h(value.id)}"`)}${action("batch-preset-delete",t("Quitar", "Remove"),undefined,"quiet",`data-id="${h(value.id)}"`)}</div>`).join("")}</div>${presetEditor!==null?`<form id="preset-editor" class="preset-editor"><label>${t("Nombre", "Name")}<input id="preset-name" required maxlength="100" value="${h(editorText(presetKey(),"preset-name",preset?.name??""))}"></label>${renderEditor(presetKey(),"preset-options",values)}<div class="form-actions">${action("batch-preset-cancel",t("Cancelar", "Cancel"),undefined,"quiet")}${action("batch-preset-save",t("Guardar ajuste", "Save preset"),"check","primary")}</div></form>`:""}`;
  }

  async function handleClick(name:string,button:HTMLElement):Promise<boolean>{
    if(!name.startsWith("batch-"))return false;
    const id=button.dataset.id??"";const t=hooks.t;
    if(name==="batch-open"){const saved=hooks.snapshot().drafts?.find(value=>value.id===id);if(saved)await open(saved);return true;}
    if(name==="batch-delete"){await hooks.command("batch_remove",{id});if(draft?.id===id){running=false;++generation;busy.clear();draft=null;}await hooks.refresh();changed();return true;}
    if(name==="batch-new"){await close();return true;}
    if(name==="batch-preset-edit"){presetEditor=id;changed();return true;}
    if(name==="batch-preset-new"){presetEditor="";changed();return true;}
    if(name==="batch-preset-cancel"){editors.delete(presetKey());presetEditor=null;changed();return true;}
    if(name==="batch-preset-delete"){await hooks.command("delete_preset",{id});await hooks.refresh();changed();return true;}
    if(name==="batch-preset-save"){
      const existing=presets().find(value=>value.id===presetEditor);const input=document.getElementById("preset-name");
      const name=input instanceof HTMLInputElement?input.value.trim():"";
      if(!name)throw new Error(t("Pon un nombre al ajuste.", "Name the preset."));
      const preset:Preset={id:existing?.id??crypto.randomUUID(),name,revision:existing?.revision??1,options:readWorkOptions("preset-options",existing?.options??defaultWorkOptions(),timezone)};
      const key=presetKey(),revision=editors.get(key)?.revision;
      await hooks.command("save_preset",{preset});await hooks.refresh();finishEditor(key,revision);presetEditor=null;changed();return true;
    }
    if(!draft)return true;
    const item=draft.items.find(entry=>entry.id===selectedItem);
    switch(name){
      case "batch-close":await close();break;
      case "batch-inspect":selectedItem=id;changed();break;
      case "batch-common":selectedItem=null;changed();break;
      case "batch-reset-item":if(item){item.options=null;editors.delete(optionsKey());await save();changed();}break;
      case "batch-apply-options":{const key=optionsKey(),revision=editors.get(key)?.revision,meta=metaKey(),metaRevision=editors.get(meta)?.revision;const options=readWorkOptions("batch-options",valuesFor(item),timezone);if(item)item.options=options;else{draft.options=options;const name=document.getElementById("batch-name");if(name instanceof HTMLInputElement)draft.name=name.value;}await save();finishEditor(key,revision);if(!item)finishEditor(meta,metaRevision,["batch-name"]);changed();break;}
      case "batch-reanalyze":if(item){await save();busy.add(item.id);changed();try{accept(await hooks.command<Draft>("batch_inspect",{draftId:draft.id,itemId:item.id}));}finally{busy.delete(item.id);changed();}}break;
      case "batch-chapters":if(item){await save();accept(await hooks.command<Draft>("batch_chapters",{draftId:draft.id,itemId:item.id,indices:[...(chapterSelections.get(item.id)??[])]}));chapterSelections.delete(item.id);changed();}break;
      case "batch-exclude":for(const entry of draft.items)if(entry.picked)entry.excluded=true;await save();changed();break;
      case "batch-restore":for(const entry of draft.items)entry.excluded=false;await save();changed();break;
      case "batch-limit":{const key=metaKey(),revision=editors.get(key)?.revision;const input=document.getElementById("batch-exploration-limit");if(input instanceof HTMLInputElement)draft.explorationLimit=Number(input.value);await save();finishEditor(key,revision,["batch-exploration-limit"]);changed();break;}
      case "batch-analyze":await save();void analyze();break;
      case "batch-cancel":running=false;++generation;await hooks.command("batch_cancel",{draftId:draft.id});busy.clear();await save();changed();break;
      case "batch-more":{await save();busy.add(id);changed();try{accept(await hooks.command<Draft>("batch_inspect",{draftId:draft.id,itemId:id}));}finally{busy.delete(id);changed();}break;}
      case "batch-save-later":draft.savedForLater=true;await save();await hooks.refresh();hooks.notify(t("Borrador guardado. No se inició ninguna descarga.", "Draft saved. No download was started."),"success");changed();break;
      case "batch-enqueue":{await save();const draftId=draft.id;const jobs=await hooks.command<Job[]>("batch_enqueue",{draftId,request:crypto.randomUUID()});await hooks.refresh();const latest=hooks.snapshot().drafts?.find(value=>value.id===draftId);if(latest)accept(latest);hooks.notify(`${jobs.length} ${t("operaciones añadidas a la cola.", "operations added to the queue.")}`,"success");changed();break;}
      case "batch-follow":{const entry=draft.items.find(value=>value.id===id);if(entry)hooks.onFollowSource?.(entry.url);break;}
    }
    return true;
  }

  async function handleChange(target:HTMLElement):Promise<boolean>{
    if(target instanceof HTMLInputElement&&target.dataset.batchChapter!==undefined&&target.dataset.itemId){const id=target.dataset.itemId;const indices=chapterSelections.get(id)??new Set<number>();const index=Number(target.dataset.batchChapter);target.checked?indices.add(index):indices.delete(index);chapterSelections.set(id,indices);const button=target.closest(".batch-chapters")?.querySelector<HTMLButtonElement>("[data-action='batch-chapters']");if(button)button.disabled=indices.size===0;return true;}
    const editing=rememberEditor(target);
    if(target.id.endsWith("-schedule-kind")){
      const parent=target.closest("fieldset");for(const field of parent?.querySelectorAll<HTMLElement>("[data-schedule-fields]")??[])field.hidden=field.dataset.scheduleFields!==(target instanceof HTMLSelectElement?target.value:"");return target.id.startsWith("batch-")||target.id.startsWith("preset-");
    }
    if(editing)return true;
    if(!draft)return false;
    if(target instanceof HTMLInputElement&&target.dataset.batchPick){const item=draft.items.find(value=>value.id===target.dataset.batchPick);if(item)item.picked=target.checked;await save();changed();return true;}
    if(target.id==="batch-preset"&&target instanceof HTMLSelectElement){for(const key of editors.keys())if(key.startsWith(`${draft.id}:options:`))editors.delete(key);draft.presetId=target.value||null;draft.options=null;await save();changed();return true;}
    if(target.id==="batch-selection"&&target instanceof HTMLSelectElement){
      const visible=new Set(batchVisible(draft.items,query).map(item=>item.id));
      for(const item of draft.items){if(item.excluded||item.list)continue;switch(target.value){case "visible":item.picked=visible.has(item.id);break;case "all":item.picked=true;break;case "ready":item.picked=item.state==="ready";break;case "none":item.picked=false;break;}}
      await save();changed();return true;
    }
    return false;
  }

  function handleInput(target:HTMLElement):boolean{
    if(rememberEditor(target))return true;
    if(target.id==="batch-search"&&target instanceof HTMLInputElement){query=target.value;changed();return true;}return false;
  }

  return {get active(){return draft!==null;},get current(){return draft;},prepare,open,close,applyPresetToSelection,editPreset,render,renderLater,renderPresets,handleClick,handleChange,handleInput};
}
