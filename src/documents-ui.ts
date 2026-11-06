import { escapeHtml as h, type Copy, type LibraryItem, type Snapshot } from "./contracts";
import { action, option } from "./views-common";

interface Hooks { command:<T>(name:string,args?:Record<string,unknown>)=>Promise<T>; snapshot:()=>Snapshot; render:()=>void; refresh:()=>Promise<void>; t:Copy }
interface Page { page:number; pages:number; png:number[]; text:string; textAvailable:boolean }
interface Hit { page:number; excerpt:string }
interface Info { pages:number; copy:boolean; assemble:boolean; print:boolean }
let selected:string[]=[];
let operation="pages";
let ranges="1-N";
let rotation="90";
let dpi="144";
let working=false;
let readerId="";
let readerPage:Page|null=null;
let readerImage="";
let readerBusy=false;
let readerError="";
let readerGeneration=0;
let searchResults:Hit[]|null=null;

function input(id:string):string {const element=document.getElementById(id);return element instanceof HTMLInputElement||element instanceof HTMLSelectElement?element.value:"";}
function capture():void {operation=input("document-operation")||operation;ranges=input("document-pages")||ranges;rotation=input("document-rotation")||rotation;dpi=input("document-dpi")||dpi;}
function pngUrl(bytes:number[]):string {let binary="";for(let offset=0;offset<bytes.length;offset+=16384)binary+=String.fromCharCode(...bytes.slice(offset,offset+16384));return "data:image/png;base64,"+btoa(binary);}

export function documentToolsPanel(snapshot:Snapshot,t:Copy):string{
  const pdfs=snapshot.library.filter(item=>item.name.toLowerCase().endsWith(".pdf"));
  selected=selected.filter(id=>pdfs.some(item=>item.id===id));
  return `<section class="tool-section document-tools"><h2>${t("Documentos PDF", "PDF documents")}</h2><p class="field-note">${t("Usa el componente opcional MuPDF. Los originales se conservan. Importa PDF o EPUB en la biblioteca para leerlos; estas operaciones trabajan con PDF.", "Uses the optional MuPDF component. Originals are preserved. Import PDF or EPUB into the library to read them; these operations accept PDF.")}</p>${action("import-files",t("Importar documentos", "Import documents"),"add","secondary")}<form id="document-tool-form"><div class="tool-options"><label>${t("Operación", "Operation")}<select id="document-operation">${option("merge",t("Unir PDF en el orden elegido", "Merge PDFs in selected order"),operation)}${option("pages",t("Seleccionar, separar o reordenar páginas", "Select, split or reorder pages"),operation)}${option("rotate",t("Rotar páginas en una copia", "Rotate pages in a copy"),operation)}${option("text",t("Extraer texto disponible", "Extract available text"),operation)}${option("image",t("Exportar una página a PNG", "Export one page as PNG"),operation)}</select></label><label>${t("Páginas · 1-3,5,N", "Pages · 1-3,5,N")}<input id="document-pages" required maxlength="4096" value="${h(ranges)}"></label><label>${t("Rotación", "Rotation")}<select id="document-rotation">${[90,180,270].map(angle=>option(String(angle),angle+"°",rotation)).join("")}</select></label><label>${t("PNG · DPI", "PNG · DPI")}<input id="document-dpi" type="number" min="72" max="300" required value="${h(dpi)}"></label></div><p class="field-note">${t("N es la última página. 5-1 invierte el orden. Para separar una página, selecciona solo esa página. PNG acepta una página por tarea. Texto no hace OCR y falla si no encuentra texto.", "N is the last page. 5-1 reverses order. To split out a page, select that page only. PNG accepts one page per job. Text extraction is not OCR and reports missing text.")}</p><div class="library-picker"><ul>${pdfs.slice(0,200).map(item=>`<li><label><input type="checkbox" data-document-input="${h(item.id)}" ${selected.includes(item.id)?"checked":""}><span>${h(item.name)}</span></label></li>`).join("")}</ul></div>${selected.length?`<ol class="document-order">${selected.map((id,index)=>`<li><span>${h(pdfs.find(item=>item.id===id)?.name??id)}</span><div class="button-group">${action("document-input-up",t("Subir", "Move up"),undefined,"quiet",`data-id="${h(id)}" ${index===0?"disabled":""}`)}${action("document-input-down",t("Bajar", "Move down"),undefined,"quiet",`data-id="${h(id)}" ${index===selected.length-1?"disabled":""}`)}</div></li>`).join("")}</ol>`:`<p class="field-note">${t("Selecciona PDF de la biblioteca.", "Select PDFs from the library.")}</p>`}<button type="submit" class="button primary" ${working||!selected.length?'disabled':''}>${working?t("Preparando…", "Preparing…"):t("Añadir a la cola", "Add to queue")}</button></form></section>`;
}

export function documentReaderPanel(item:LibraryItem,t:Copy):string{
  const active=readerId===item.id;
  const page=active?readerPage:null;
  const bookmarks=item.details?.bookmarks??[];
  return `<section class="document-reader" aria-label="${t("Lector local", "Local reader")}"><div class="button-group">${action("document-open",t("Abrir lector", "Open reader"),"library","secondary",`data-id="${h(item.id)}" ${readerBusy&&active?'disabled':''}`)}${page?action("document-previous",t("Anterior", "Previous"),undefined,"quiet",`data-id="${h(item.id)}" ${page.page<=1||readerBusy?'disabled':''}`):""}${page?`<span aria-live="polite">${t("Página", "Page")} ${page.page} / ${page.pages}</span>${action("document-next",t("Siguiente", "Next"),undefined,"quiet",`data-id="${h(item.id)}" ${page.page>=page.pages||readerBusy?'disabled':''}`)}${action("document-bookmark",t("Marcar página", "Bookmark page"),"add","quiet",`data-id="${h(item.id)}"`)}`:""}</div>${active&&readerBusy?`<p role="status">${t("Cargando página…", "Loading page…")}</p>`:""}${active&&readerError?`<p role="alert">${h(readerError)}</p>`:""}${page?`<figure><img class="document-page-image" src="${readerImage}" alt="${t("Página", "Page")} ${page.page}"></figure><details><summary>${t("Texto de esta página", "Text on this page")}</summary>${page.textAvailable?`<pre class="document-text">${h(page.text)}</pre>`:`<p class="field-note">${t("No hay texto extraíble o el documento no permite copiarlo. La lectura no aplica OCR.", "There is no extractable text, or copying is not permitted. The reader does not perform OCR.")}</p>`}</details><form id="document-search-form" data-id="${h(item.id)}"><label>${t("Buscar en el documento", "Search the document")}<input id="document-search" type="search" required minlength="2" maxlength="160"></label><button type="submit" class="button secondary" ${readerBusy?'disabled':''}>${t("Buscar", "Search")}</button></form>${searchResults===null?"":searchResults.length?`<ul class="document-search-results">${searchResults.map(hit=>`<li><button type="button" class="button quiet" data-action="document-go-page" data-id="${h(item.id)}" data-page="${hit.page}"><span>${t("Página", "Page")} ${hit.page}</span><span>${h(hit.excerpt)}</span></button></li>`).join("")}</ul>`:`<p>${t("No hay coincidencias en el texto disponible.", "No matches in the available text.")}</p>`}`:""}${bookmarks.length?`<div class="button-group">${bookmarks.map(bookmark=>action("document-go-page",`${bookmark.name} · ${Math.floor(bookmark.time)}`,undefined,"quiet",`data-id="${h(item.id)}" data-page="${Math.floor(bookmark.time)}"`)).join("")}</div>`:""}<p class="field-note">${t("El lector muestra una imagen y texto local. No ejecuta scripts ni abre enlaces del documento.", "The reader displays a local image and text. It does not execute scripts or open document links.")}</p></section>`;
}

export function createDocuments(hooks:Hooks){
  async function page(id:string,number:number):Promise<void>{
    const generation=++readerGeneration;readerId=id;readerBusy=true;readerError="";hooks.render();
    try{const result=await hooks.command<Page>("document_page",{id,page:number});if(generation!==readerGeneration)return;readerPage=result;readerImage=pngUrl(result.png);await hooks.command("library_playback",{id,position:result.page});await hooks.refresh();}
    catch(error){if(generation===readerGeneration)readerError=error instanceof Error?error.message:String(error);}
    finally{if(generation===readerGeneration){readerBusy=false;hooks.render();}}
  }
  async function open(id:string):Promise<void>{
    const generation=++readerGeneration;
    readerId=id;readerPage=null;readerImage="";readerError="";searchResults=null;readerBusy=true;hooks.render();
    try{const info=await hooks.command<Info>("document_info",{id});if(generation!==readerGeneration)return;const saved=hooks.snapshot().library.find(item=>item.id===id)?.details?.playbackPosition??1;await page(id,Math.max(1,Math.min(info.pages,Math.floor(saved))));}
    catch(error){if(generation===readerGeneration){readerError=error instanceof Error?error.message:String(error);readerBusy=false;hooks.render();}}
  }
  function change(target:HTMLInputElement|HTMLSelectElement):boolean{
    if(!(target instanceof HTMLInputElement)||!target.dataset.documentInput)return false;
    capture();const id=target.dataset.documentInput;if(target.checked&&!selected.includes(id))selected.push(id);else if(!target.checked)selected=selected.filter(value=>value!==id);hooks.render();return true;
  }
  async function click(name:string,button:HTMLButtonElement):Promise<boolean>{
    if(!name.startsWith("document-"))return false;
    const id=button.dataset.id??"";
    if(name==="document-input-up"||name==="document-input-down"){capture();const from=selected.indexOf(id),to=from+(name==="document-input-up"?-1:1);if(from>=0&&to>=0&&to<selected.length){[selected[from],selected[to]]=[selected[to],selected[from]];hooks.render();}}
    else if(name==="document-open")await open(id);
    else if(name==="document-previous"&&readerPage)await page(id,readerPage.page-1);
    else if(name==="document-next"&&readerPage)await page(id,readerPage.page+1);
    else if(name==="document-go-page")await page(id,Number(button.dataset.page));
    else if(name==="document-bookmark"&&readerPage){await hooks.command("library_bookmark",{id,bookmark:{id:crypto.randomUUID(),name:hooks.t("Página ","Page ")+readerPage.page,time:readerPage.page},remove:false});await hooks.refresh();}
    return true;
  }
  async function submit(form:HTMLFormElement):Promise<boolean>{
    if(form.id==="document-tool-form"){
      if(working)return true;capture();if(!selected.length)return true;
      working=true;hooks.render();
      try{await hooks.command("queue_document",{ids:[...selected],options:{operation,pages:ranges,rotation:Number(rotation),dpi:Number(dpi)},clientRequest:crypto.randomUUID()});await hooks.refresh();}
      finally{working=false;hooks.render();}
      return true;
    }
    if(form.id==="document-search-form"){
      const id=form.dataset.id??"",query=input("document-search"),generation=readerGeneration;if(readerBusy)return true;
      readerBusy=true;readerError="";hooks.render();
      try{const result=await hooks.command<Hit[]>("document_search",{id,query});if(generation===readerGeneration)searchResults=result;}
      catch(error){if(generation===readerGeneration)readerError=error instanceof Error?error.message:String(error);}
      finally{if(generation===readerGeneration){readerBusy=false;hooks.render();}}
      return true;
    }
    return false;
  }
  return {open,click,change,submit};
}
