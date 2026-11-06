import { escapeHtml as h, type Copy, type Snapshot } from "./contracts";
import { action, option } from "./views-common";

interface Hooks{command:<T>(name:string,args?:Record<string,unknown>)=>Promise<T>;snapshot:()=>Snapshot;refresh:()=>Promise<void>;render:()=>void;notify:(message:string)=>void;t:Copy}
type Kind="join"|"replaceAudio"|"burnSubtitles";
type Options={kind:"join";mode:"copy"|"encode";width:number;height:number;fps:number}|{kind:"replaceAudio"}|{kind:"burnSubtitles";trackId:string};
let kind:Kind="join";
let joinMode:"copy"|"encode"="copy";
let selected:string[]=[];
let videoId="";
let audioId="";
let width=1280,height=720,fps=30;
let working=false;
let burnAvailable:boolean|null=null;
function value(id:string):string{const element=document.getElementById(id);return element instanceof HTMLSelectElement||element instanceof HTMLInputElement?element.value:"";}
function capture():void{const video=value("media-edit-video"),audio=value("media-edit-audio");if(video)videoId=video;if(audio)audioId=audio;if(joinMode==="encode"){width=Number(value("media-edit-width"))||width;height=Number(value("media-edit-height"))||height;fps=Number(value("media-edit-fps"))||fps;}}

export function mediaEditsPanel(snapshot:Snapshot,t:Copy):string{
  const videos=snapshot.library.filter(item=>item.kind==="video"),audio=snapshot.library.filter(item=>item.kind==="audio"||item.kind==="video");
  selected=selected.filter(id=>videos.some(item=>item.id===id));if(!videos.some(item=>item.id===videoId))videoId=videos[0]?.id??"";if(!audio.some(item=>item.id===audioId))audioId=audio.find(item=>item.id!==videoId)?.id??audio[0]?.id??"";
  const tracks=videos.find(item=>item.id===videoId)?.details?.tracks??[];
  return `<details class="tool-section media-edit-tools"><summary>${t("Unir clips, sustituir audio y subtítulos", "Join clips, replace audio and subtitles")}</summary><p class="field-note">${t("Crea una copia en la cola. Conserva todos los originales.", "Creates a copy through the queue. Preserves all originals.")}</p><form id="media-edit-form"><label>${t("Operación", "Operation")}<select id="media-edit-kind">${option("join",t("Unir clips en orden", "Join clips in order"),kind)}${option("replaceAudio",t("Sustituir audio de un vídeo", "Replace video audio"),kind)}<option value="burnSubtitles" ${kind==="burnSubtitles"?'selected':''} ${burnAvailable?'':'disabled'}>${t("Incrustar subtítulos en la imagen", "Burn subtitles into video")}${burnAvailable?'':t(" · requiere filtro libass", " · requires libass filter")}</option></select></label>${kind==="join"?`<label>${t("Método", "Method")}<select id="media-edit-join-mode">${option("copy",t("Copiar streams compatibles", "Copy compatible streams"),joinMode)}${option("encode",t("Recodificar y normalizar", "Re-encode and normalize"),joinMode)}</select></label>${joinMode==="encode"?`<div class="tool-options"><label>${t("Ancho", "Width")}<input id="media-edit-width" type="number" min="16" max="8192" step="2" required value="${width}"></label><label>${t("Alto", "Height")}<input id="media-edit-height" type="number" min="16" max="8192" step="2" required value="${height}"></label><label>FPS<input id="media-edit-fps" type="number" min="1" max="120" required value="${fps}"></label></div>`:""}<p class="field-note">${joinMode==="copy"?t("Copiar conserva los streams y exige códecs, resolución, FPS y audio compatibles. El resultado es MKV.", "Copying preserves streams and requires compatible codecs, resolution, FPS and audio. Output is MKV."):t("Recodifica H.264/AAC. Ajusta la imagen a las dimensiones y FPS elegidos, con márgenes para conservar proporciones. Normaliza audio a estéreo de 48 kHz y añade silencio donde falte audio. El resultado es MKV.", "Re-encodes H.264/AAC. Fits video to the selected dimensions and FPS, padding to keep proportions. Normalizes audio to stereo at 48 kHz and adds silence where audio is missing. Output is MKV.")}</p><div class="library-picker"><ul>${videos.slice(0,200).map(item=>`<li><label><input type="checkbox" data-media-edit-input="${h(item.id)}" ${selected.includes(item.id)?'checked':''}><span>${h(item.name)}</span></label></li>`).join("")}</ul></div><ol class="document-order">${selected.map((id,index)=>`<li><span>${h(videos.find(item=>item.id===id)?.name??id)}</span><div class="button-group">${action("media-edit-up",t("Subir", "Move up"),undefined,"quiet",`data-id="${h(id)}" ${index===0?'disabled':''}`)}${action("media-edit-down",t("Bajar", "Move down"),undefined,"quiet",`data-id="${h(id)}" ${index===selected.length-1?'disabled':''}`)}</div></li>`).join("")}</ol>`:`<label>${t("Vídeo original", "Original video")}<select id="media-edit-video" required>${videos.map(item=>option(item.id,item.name,videoId)).join("")}</select></label>${kind==="replaceAudio"?`<label>${t("Nueva fuente de audio", "New audio source")}<select id="media-edit-audio" required>${audio.map(item=>option(item.id,item.name,audioId)).join("")}</select></label><p class="field-note">${t("Copia el stream de vídeo a MKV y codifica el audio como AAC. El audio más corto se completa con silencio y el más largo se recorta a la duración del vídeo.", "Copies the video stream into MKV and encodes audio as AAC. Shorter audio is padded with silence; longer audio is trimmed to video duration.")}</p>`:`<label>${t("Pista local", "Local track")}<select id="media-edit-track" required>${tracks.map(track=>option(track.id,`${track.language} · ${track.origin}`,tracks[0]?.id??"")).join("")}</select></label><p class="field-note">${t("Recodifica el vídeo como H.264/AAC a MP4, con los subtítulos visibles en la imagen. Este cambio no se puede deshacer en la copia.", "Re-encodes video as H.264/AAC into MP4, with subtitles visible in the image. This change cannot be undone in the copy.")}</p>`}`}${burnAvailable===false?`<p class="field-note">${t("El componente FFmpeg instalado no incluye el filtro libass. Puedes unir clips y sustituir audio; quemar subtítulos necesita un componente con ese filtro.", "The installed FFmpeg component has no libass filter. Clip joining and audio replacement are available; burning subtitles requires a component with that filter.")}</p>`:""}<button type="submit" class="button primary" ${working||kind==="join"&&selected.length<2||kind!=="join"&&!videoId||kind==="burnSubtitles"&&(!burnAvailable||!tracks.length)?'disabled':''}>${working?t("Preparando…", "Preparing…"):t("Añadir a la cola", "Add to queue")}</button></form></details>`;
}

export function createMediaEdits(hooks:Hooks){
  async function load():Promise<void>{try{const result=await hooks.command<{burnSubtitles:boolean}>("media_edit_capabilities");burnAvailable=result.burnSubtitles;}catch{burnAvailable=false;}hooks.render();}
  function change(target:HTMLInputElement|HTMLSelectElement):boolean{
    if(target.dataset.mediaEditInput){capture();const id=target.dataset.mediaEditInput;if(target instanceof HTMLInputElement){if(target.checked&&!selected.includes(id))selected.push(id);else if(!target.checked)selected=selected.filter(value=>value!==id);}hooks.render();return true;}
    if(target.id==="media-edit-kind"){capture();const next=target.value;if(next==="join"||next==="replaceAudio"||next==="burnSubtitles")kind=next;hooks.render();return true;}
    if(target.id==="media-edit-join-mode"){capture();const next=target.value;if(next==="copy"||next==="encode")joinMode=next;hooks.render();return true;}
    if(target.id==="media-edit-video"){capture();hooks.render();return true;}
    return false;
  }
  async function click(name:string,button:HTMLButtonElement):Promise<boolean>{if(name!=="media-edit-up"&&name!=="media-edit-down")return false;capture();const from=selected.indexOf(button.dataset.id??""),to=from+(name==="media-edit-up"?-1:1);if(from>=0&&to>=0&&to<selected.length){[selected[from],selected[to]]=[selected[to],selected[from]];hooks.render();}return true;}
  async function submit(form:HTMLFormElement):Promise<boolean>{
    if(form.id!=="media-edit-form")return false;if(working)return true;capture();
    let options:Options;let ids:string[];
    if(kind==="join"){options={kind,mode:joinMode,width,height,fps};ids=[...selected];}
    else if(kind==="replaceAudio"){options={kind};ids=[videoId,audioId];}
    else{if(!burnAvailable)throw new Error(hooks.t("Falta el filtro de subtítulos del componente.","The component has no subtitle filter."));options={kind,trackId:value("media-edit-track")};ids=[videoId];}
    working=true;hooks.render();try{await hooks.command("queue_media_edit",{ids,options,clientRequest:crypto.randomUUID()});await hooks.refresh();hooks.notify(hooks.t("Edición añadida a la cola.","Edit added to the queue."));}finally{working=false;hooks.render();}return true;
  }
  return {load,change,click,submit};
}
