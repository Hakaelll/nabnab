import { escapeHtml as h, type Copy, type Snapshot } from "./contracts";
import { action, option } from "./views-common";

interface Hooks { command:<T>(name:string,args?:Record<string,unknown>)=>Promise<T>;snapshot:()=>Snapshot;render:()=>void;refresh:()=>Promise<void>;notify:(message:string)=>void;t:Copy }
let selected=new Set<string>();
let colorCount=8;
let format:"hex"|"css"="css";
let working=false;
let previewId="";
let previewColors:string[]=[];
let previewLoading=false;
let previewError="";
let previewGeneration=0;
function value(id:string):string{const input=document.getElementById(id);return input instanceof HTMLInputElement||input instanceof HTMLSelectElement?input.value:"";}
function capture():void{const count=Number(value("palette-count"));if(Number.isInteger(count)&&count>=2&&count<=16)colorCount=count;const next=value("palette-format");if(next==="hex"||next==="css")format=next;}
function css(colors:string[]):string{return ":root {\n"+colors.map((color,index)=>`  --palette-${index+1}: ${color};`).join("\n")+"\n}\n";}

export function palettesPanel(snapshot:Snapshot,t:Copy):string{
  const images=snapshot.library.filter(item=>item.kind==="image");selected=new Set([...selected].filter(id=>images.some(item=>item.id===id)));
  const palettes=snapshot.library.filter(item=>/^palette-\d+\.(css|txt)$/i.test(item.name));if(!palettes.some(item=>item.id===previewId)){const next=palettes[0]?.id??"";if(next!==previewId){++previewGeneration;previewLoading=false;}previewId=next;previewColors=[];}
  return `<details class="tool-section palette-tools"><summary>${t("Extraer paleta de imágenes", "Extract image palette")}</summary><p class="field-note">${t("Extrae colores representativos de una imagen o una colección mediante muestreo local. Ignora píxeles transparentes y usa el primer fotograma de las animaciones. Conserva los originales.", "Extracts representative colors from an image or collection using local sampling. Ignores transparent pixels and uses the first frame of animations. Preserves originals.")}</p><form id="palette-form"><div class="tool-options"><label>${t("Máximo de colores", "Maximum colors")}<input id="palette-count" type="number" min="2" max="16" step="1" required value="${colorCount}"></label><label>${t("Formato de salida", "Output format")}<select id="palette-format">${option("css",t("Variables CSS", "CSS variables"),format)}${option("hex",t("Lista HEX · TXT", "HEX list · TXT"),format)}</select></label></div><p class="field-note">${selected.size} ${t("imágenes seleccionadas", "selected images")}</p><div class="library-picker"><ul>${images.slice(0,200).map(item=>`<li><label><input type="checkbox" data-palette-input="${h(item.id)}" ${selected.has(item.id)?'checked':''} ${working||selected.size>=200&&!selected.has(item.id)?'disabled':''}><span>${h(item.name)}</span></label></li>`).join("")}</ul></div><p class="field-note">${t("Hasta 200 imágenes de 256 MiB cada una. Se muestrean hasta 128 × 128 píxeles por imagen. Una imagen de color uniforme puede producir menos colores que el máximo elegido.", "Up to 200 images of 256 MiB each. Samples up to 128 × 128 pixels per image. A solid-color image may produce fewer colors than the selected maximum.")}</p><div class="destination-row"><span>${h(snapshot.settings.folder||t("Elige un destino", "Choose a destination"))}</span>${action("choose-folder",t("Cambiar destino", "Change destination"),"folder","quiet")}</div><button type="submit" class="button primary" ${working||!selected.size?'disabled':''}>${working?t("Preparando…", "Preparing…"):t("Extraer en la cola", "Extract through queue")}</button></form>${palettes.length?`<section class="palette-preview"><label>${t("Previsualizar una paleta guardada", "Preview a saved palette")}<select id="palette-preview-file">${palettes.map(item=>option(item.id,item.name,previewId)).join("")}</select></label>${action("palette-preview",previewLoading?t("Leyendo archivo…", "Reading file…"):t("Mostrar colores del archivo", "Show colors from file"),"image","secondary",previewLoading?'disabled':'')}${previewError?`<p role="alert">${h(previewError)}</p>`:""}${previewColors.length?`<ul class="button-group" aria-label="${t("Colores extraídos", "Extracted colors")}">${previewColors.map(color=>`<li style="display:flex;align-items:center;gap:8px;padding:6px"><span aria-hidden="true" style="display:inline-block;width:36px;height:36px;border:1px solid var(--border);border-radius:8px;background-color:${h(color)}"></span><code>${h(color)}</code></li>`).join("")}</ul><div class="button-group">${action("palette-copy-hex",t("Copiar HEX", "Copy HEX"),"copy","quiet")}${action("palette-copy-css",t("Copiar CSS", "Copy CSS"),"copy","quiet")}</div>`:`<p class="field-note">${t("Los colores se muestran al leer un resultado real de la biblioteca.", "Colors appear after reading an actual result from the library.")}</p>`}</section>`:`<p class="field-note">${t("Al terminar la tarea, la paleta estará en la biblioteca y podrás previsualizar sus colores aquí.", "When the job finishes, its palette appears in the library and its colors can be previewed here.")}</p>`}</details>`;
}

export function createPalettes(hooks:Hooks){
  function change(target:HTMLInputElement|HTMLSelectElement):boolean{
    if(target instanceof HTMLInputElement&&target.dataset.paletteInput){capture();if(target.checked)selected.add(target.dataset.paletteInput);else selected.delete(target.dataset.paletteInput);hooks.render();return true;}
    if(target.id==="palette-preview-file"){previewId=target.value;previewColors=[];previewError="";previewLoading=false;++previewGeneration;hooks.render();return true;}
    return false;
  }
  async function click(name:string):Promise<boolean>{
    if(name==="palette-preview"){
      if(previewLoading||!previewId)return true;const generation=++previewGeneration;previewLoading=true;previewError="";hooks.render();
      try{const colors=await hooks.command<string[]>("palette_preview",{id:previewId});if(generation===previewGeneration)previewColors=colors.filter(color=>/^#[0-9a-f]{6}$/i.test(color));}
      catch(error){if(generation===previewGeneration)previewError=error instanceof Error?error.message:String(error);}
      finally{if(generation===previewGeneration){previewLoading=false;hooks.render();}}return true;
    }
    if(name==="palette-copy-hex"||name==="palette-copy-css"){if(previewColors.length){await navigator.clipboard.writeText(name==="palette-copy-css"?css(previewColors):previewColors.join("\n"));hooks.notify(hooks.t("Paleta copiada.","Palette copied."));}return true;}
    return false;
  }
  async function submit(form:HTMLFormElement):Promise<boolean>{
    if(form.id!=="palette-form")return false;if(working)return true;capture();const ids=[...selected];if(!ids.length)return true;
    working=true;hooks.render();try{await hooks.command("queue_palette",{ids,colors:colorCount,format,clientRequest:crypto.randomUUID()});await hooks.refresh();hooks.notify(hooks.t("Extracción de paleta añadida a la cola.","Palette extraction added to the queue."));}finally{working=false;hooks.render();}return true;
  }
  return {change,click,submit};
}
