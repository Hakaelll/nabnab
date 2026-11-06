import { escapeHtml as h, type Copy, type LibraryItem, type Snapshot } from "./contracts";
import { option } from "./views-common";

type Mode="segment"|"replace"|"shift"|"sync";
export type CaptionEditOptions={kind:"segment";index:number;start:number;end:number;text:string}|{kind:"replace";find:string;replace:string}|{kind:"shift";seconds:number}|{kind:"sync";sourceStart:number;targetStart:number;sourceEnd:number;targetEnd:number};
interface Hooks{command:<T>(name:string,args?:Record<string,unknown>)=>Promise<T>;snapshot:()=>Snapshot;refresh:()=>Promise<void>;render:()=>void;notify:(message:string)=>void;t:Copy}
let mode:Mode="segment";
let currentFile="";
let selectedTrack="";
let cueIndex=0;
let working=false;

function numberField(id:string,label:string,value:number,min?:number):string{return `<label>${label}<input id="${id}" type="number" step="0.001" ${min===undefined?'':`min="${min}"`} required value="${value}"></label>`;}
function value(id:string):string{const field=document.getElementById(id);return field instanceof HTMLInputElement||field instanceof HTMLSelectElement||field instanceof HTMLTextAreaElement?field.value:"";}
function numeric(id:string):number{const number=Number(value(id));if(!Number.isFinite(number))throw new Error("Enter a finite number of seconds.");return number;}

export function captionEditorPanel(item:LibraryItem,t:Copy):string{
  const tracks=item.details?.tracks??[];
  if(!tracks.length)return "";
  if(currentFile!==item.id){currentFile=item.id;selectedTrack=tracks[0].id;cueIndex=0;}
  const track=tracks.find(track=>track.id===selectedTrack)??tracks[0];selectedTrack=track.id;
  cueIndex=Math.max(0,Math.min(track.segments.length-1,cueIndex));const cue=track.segments[cueIndex];
  return `<details class="caption-editor"><summary>${t("Editar una copia de los subtítulos", "Edit a subtitle copy")}</summary><form id="caption-editor-form" data-library-id="${h(item.id)}" data-track-id="${h(track.id)}"><p class="field-note">${t("Crea un SRT nuevo en la cola. Conserva la pista original. Texto plano, hasta 100.000 segmentos y 2 MiB de texto; los tiempos deben seguir siendo positivos y crecientes dentro de cada segmento.", "Creates a new SRT through the queue. Preserves the original track. Plain text, up to 100,000 segments and 2 MiB of text; each segment must keep non-negative times with its end after its start.")}</p><label>${t("Pista que se editará", "Track to edit")}<select id="caption-editor-track">${tracks.map(track=>option(track.id,`${track.language} · ${track.segments.length} ${t("segmentos", "segments")}`,selectedTrack)).join("")}</select></label><label>${t("Operación", "Operation")}<select id="caption-editor-mode">${option("segment",t("Corregir un segmento", "Correct one segment"),mode)}${option("replace",t("Buscar y sustituir", "Find and replace"),mode)}${option("shift",t("Desplazar tiempos", "Shift timestamps"),mode)}${option("sync",t("Sincronizar con dos puntos", "Sync using two points"),mode)}</select></label><div class="tool-options">${mode==="segment"&&cue?`<label>${t("Número de segmento", "Segment number")}<input id="caption-editor-index" type="number" min="1" max="${track.segments.length}" step="1" required value="${cueIndex+1}"></label>${numberField("caption-editor-start",t("Inicio · segundos", "Start · seconds"),cue.start,0)}${numberField("caption-editor-end",t("Fin · segundos", "End · seconds"),cue.end,0)}<label class="full-width">${t("Texto corregido", "Corrected text")}<textarea id="caption-editor-text" rows="3" required maxlength="2097152">${h(cue.text)}</textarea></label>`:mode==="replace"?`<label>${t("Buscar literalmente", "Find literal text")}<input id="caption-editor-find" required maxlength="4096"></label><label>${t("Sustituir por", "Replace with")}<input id="caption-editor-replacement" maxlength="4096"></label><p class="field-note full-width">${t("Distingue mayúsculas y minúsculas. Cambia todas las coincidencias de la pista elegida.", "Case-sensitive. Changes all matches in the selected track.")}</p>`:mode==="shift"?`${numberField("caption-editor-offset",t("Desplazamiento · segundos", "Offset · seconds"),0)}<p class="field-note full-width">${t("Un valor negativo adelanta los subtítulos. Si un segmento quedaría antes de cero, se rechaza el cambio completo.", "A negative value moves subtitles earlier. If a segment would start before zero, the entire change is rejected.")}</p>`:mode==="sync"?`${numberField("caption-editor-source-start",t("Primer tiempo en subtítulos", "First subtitle time"),0,0)}${numberField("caption-editor-target-start",t("Primer tiempo correcto", "First correct time"),0,0)}${numberField("caption-editor-source-end",t("Segundo tiempo en subtítulos", "Second subtitle time"),60,0)}${numberField("caption-editor-target-end",t("Segundo tiempo correcto", "Second correct time"),60,0)}<p class="field-note full-width">${t("Calcula una corrección lineal de desplazamiento y velocidad para toda la pista. Ambos pares deben avanzar en el tiempo.", "Applies a linear offset and timing scale to the whole track. Both pairs must move forward in time.")}</p>`:""}</div><button type="submit" class="button secondary" ${working||!track.segments.length?'disabled':''}>${working?t("Preparando…", "Preparing…"):t("Crear copia en la cola", "Create copy in queue")}</button></form></details>`;
}

export function createCaptionEditor(hooks:Hooks){
  function change(target:HTMLInputElement|HTMLSelectElement):boolean{
    if(target.id==="caption-editor-mode"){const next=target.value;if(next==="segment"||next==="replace"||next==="shift"||next==="sync"){mode=next;hooks.render();}return true;}
    if(target.id==="caption-editor-track"){selectedTrack=target.value;cueIndex=0;hooks.render();return true;}
    if(target.id==="caption-editor-index"){const next=Number(target.value);if(Number.isInteger(next)&&next>=1){cueIndex=next-1;hooks.render();}return true;}
    return false;
  }
  async function submit(form:HTMLFormElement):Promise<boolean>{
    if(form.id!=="caption-editor-form")return false;
    if(working)return true;
    const libraryId=form.dataset.libraryId??"",trackId=form.dataset.trackId??"";
    if(!hooks.snapshot().library.some(item=>item.id===libraryId&&item.details?.tracks.some(track=>track.id===trackId)))throw new Error(hooks.t("La pista ya no está disponible.", "The track is no longer available."));
    let options:CaptionEditOptions;
    switch(mode){
      case "segment":options={kind:mode,index:numeric("caption-editor-index")-1,start:numeric("caption-editor-start"),end:numeric("caption-editor-end"),text:value("caption-editor-text")};break;
      case "replace":options={kind:mode,find:value("caption-editor-find"),replace:value("caption-editor-replacement")};break;
      case "shift":options={kind:mode,seconds:numeric("caption-editor-offset")};break;
      case "sync":options={kind:mode,sourceStart:numeric("caption-editor-source-start"),targetStart:numeric("caption-editor-target-start"),sourceEnd:numeric("caption-editor-source-end"),targetEnd:numeric("caption-editor-target-end")};break;
    }
    working=true;hooks.render();
    try{await hooks.command("queue_caption_edit",{libraryId,trackId,options,clientRequest:crypto.randomUUID()});await hooks.refresh();hooks.notify(hooks.t("Copia de subtítulos añadida a la cola.", "Subtitle copy added to the queue."));}
    finally{working=false;hooks.render();}
    return true;
  }
  return {change,submit};
}
