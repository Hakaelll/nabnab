import "./ocr.css";
import {escapeHtml as h, type Copy, type LibraryItem, type Snapshot} from "./contracts";
import {action, icon, option} from "./views-common";

interface Region {x:number;y:number;width:number;height:number}
interface Word extends Region {text:string;confidence:number}
interface OcrPage {page:number;width:number;height:number;text:string;origin:"original"|"ocr";words:Word[];region:Region|null;corrected:boolean}
interface OcrDocument {schemaVersion:number;sourceFingerprint:string;languages:string[];pages:OcrPage[]}
interface Hit {page:number;excerpt:string;origin:"original"|"ocr";boxes:Region[]}
interface Model {id:string;name:string;bytes:number;license:string;source:string;installed:boolean}
interface Status {installed:boolean;enabled:boolean}
interface Hooks {command<T>(name:string,args?:Record<string,unknown>):Promise<T>;snapshot():Snapshot;refresh():Promise<void>;render():void;notify(text:string,kind?:"info"|"success"|"error"):void;t:Copy;settings():void}
type Output="text"|"searchablePdf";
interface Options {languages:string[];pages:string;dpi:number;region:Region|null;output:Output;acknowledgeStructureChanges:boolean}

function hasIndex(item:LibraryItem):boolean {return Boolean(item.details&&"ocrRef" in item.details&&item.details.ocrRef);}
function png(bytes:number[]):string {let binary="";for(let offset=0;offset<bytes.length;offset+=16384)binary+=String.fromCharCode(...bytes.slice(offset,offset+16384));return "data:image/png;base64,"+btoa(binary);}

export function createOcr(hooks:Hooks){
  const {t}=hooks;
  let status:Status|null=null,models:Model[]=[],loading=false,loaded=false,error="",busy=false;
  let selected="",pages="1-N",dpi=200,output:Output="text",languages=["spa"],crop=false;
  let region:Region={x:0,y:0,width:1000,height:1000},acknowledge=false;
  let currentId="",document:OcrDocument|null=null,pageNumber=1,edited="",query="",hits:Hit[]|null=null,boxes:Region[]=[];
  let preview="",previewError="",generation=0;
  const snapshot=()=>hooks.snapshot();
  const eligible=()=>snapshot().library.filter(item=>item.kind==="image"||item.kind==="document"&&item.name.toLowerCase().endsWith(".pdf"));
  const input=()=>eligible().find(item=>item.id===selected);
  const pdf=()=>Boolean(input()?.name.toLowerCase().endsWith(".pdf"));
  const currentPage=()=>document?.pages.find(page=>page.page===pageNumber);
  function sourceItem(id:string):LibraryItem|undefined{const item=snapshot().library.find(item=>item.id===id);return item?.parentId?snapshot().library.find(source=>source.id===item.parentId)??item:item;}
  async function load():Promise<void>{
    if(loading)return;loading=true;error="";
    try{[status,models]=await Promise.all([hooks.command<Status>("ocr_status"),hooks.command<Model[]>("ocr_models")]);loaded=true;}
    catch(reason){error=reason instanceof Error?reason.message:String(reason);loaded=true;}
    finally{loading=false;hooks.render();}
  }
  function renderTools():string{
    if(!loaded&&!loading)void load();
    const files=eligible();if(selected&&!files.some(item=>item.id===selected))selected="";
    const canRun=Boolean(selected&&status?.installed&&status.enabled&&languages.length&&languages.every(language=>models.some(model=>model.id===language&&model.installed)));
    return `<section class="tool-section ocr-tools"><div class="ocr-heading"><div><h2>${t("Reconocer texto", "Recognize text")}</h2><p>${t("Imágenes y PDF escaneados. Tesseract trabaja en este equipo.","Images and scanned PDFs. Tesseract runs on this computer.")}</p></div><span class="ocr-engine-status">${loading?t("Comprobando…","Checking…"):!status?.installed?t("Componente ausente","Component missing"):!status.enabled?t("OCR desactivado","OCR disabled"):t("Motor local disponible","Local engine available")}</span></div>
      ${error?`<p class="notice error" role="alert">${h(error)}</p>`:""}
      ${!status?.installed||!status.enabled?`<div class="ocr-component-notice">${icon("document")}<div><strong>${t("OCR es un componente opcional","OCR is optional")}</strong><p>${t("Tesseract 5.5.4 · 6,7 MB de descarga. Los idiomas se descargan por separado y requieren tu consentimiento.","Tesseract 5.5.4 · 6.7 MB download. Languages are downloaded separately with your consent.")}</p>${action("ocr-settings",t("Abrir componentes","Open components"),"settings","secondary")}</div></div>`:""}
      <div class="ocr-models" aria-label="${t("Idiomas OCR","OCR languages")}">${models.map(model=>`<div><span><strong>${h(model.name)}</strong><small>${(model.bytes/1048576).toFixed(1)} MB · ${h(model.license)}</small></span>${model.installed?`<span>${icon("check")}${t("Instalado","Installed")}</span>`:action("ocr-install-model",t("Descargar idioma","Download language"),"downloads","secondary",`data-id="${h(model.id)}" ${busy||!status?.enabled?"disabled":""}`)}</div>`).join("")}</div>
      <form id="ocr-form" aria-busy="${busy}"><label>${t("Entrada de Biblioteca","Library input")}<select id="ocr-input" required>${option("",t("Elige una imagen o PDF","Choose an image or PDF"),selected)}${files.slice(0,500).map(item=>option(item.id,item.name,selected)).join("")}</select></label>
      <fieldset class="ocr-languages"><legend>${t("Idioma del texto","Text language")}</legend>${[["spa","Español"],["eng","English"]].map(([id,name])=>`<label><input type="checkbox" id="ocr-language-${id}" data-ocr-language="${id}" ${languages.includes(id)?"checked":""}>${name}</label>`).join("")}</fieldset>
      <div class="form-grid"><label>${t("Resultado","Output")}<select id="ocr-output">${option("text",t("Texto para revisar y corregir","Text to review and correct"),output)}${pdf()?option("searchablePdf",t("Copia PDF buscable","Searchable PDF copy"),output):""}</select></label>${pdf()?`<label>${t("Páginas · 1-3,5,N","Pages · 1-3,5,N")}<input id="ocr-pages" value="${h(pages)}" required maxlength="4096"></label><label>${t("Resolución OCR","OCR resolution")}<select id="ocr-dpi">${[150,200,300].map(value=>option(String(value),`${value} DPI`,String(dpi))).join("")}</select></label>`:""}</div>
      ${selected&&!pdf()?`<label><input id="ocr-crop" type="checkbox" ${crop?"checked":""}>${t("Reconocer una región de la imagen","Recognize an image region")}</label>${crop?`<div class="ocr-region">${(["x","y","width","height"] satisfies (keyof Region)[]).map(key=>`<label>${({x:"X",y:"Y",width:t("Ancho","Width"),height:t("Alto","Height")})[key]}<input id="ocr-region-${key}" type="number" min="${key==="width"||key==="height"?1:0}" max="40000000" required value="${region[key]}"></label>`).join("")}</div><p class="field-note">${t("Coordenadas en píxeles del original, desde la esquina superior izquierda. Se comprueban antes de recortar. Las imágenes animadas usan el primer fotograma.","Coordinates are original-image pixels from its upper-left corner. They are checked before cropping. Animated images use their first frame.")}</p>`:""}`:""}
      ${output==="searchablePdf"?`<label class="ocr-consent"><input id="ocr-acknowledge" type="checkbox" required ${acknowledge?"checked":""}>${t("Entiendo que la copia puede cambiar estructura, formularios y firmas. Su validez no está garantizada. El original se conserva.","I understand that the copy may change structure, forms and signatures. Their validity is not guaranteed. The original is preserved.")}</label>`:""}
      <p class="field-note">${t("Hasta 200 páginas por trabajo. Las páginas con texto útil conservan su capa existente. El reconocimiento puede contener errores; no reconstruye tablas ni garantiza escritura manuscrita.","Up to 200 pages per job. Pages with useful text keep their existing text layer. Recognition can contain errors; it does not reconstruct tables or guarantee handwriting recognition.")}</p>
      <div class="ocr-destination"><span>${t("Destino","Destination")}</span><span class="path">${h(snapshot().settings.folder)}</span></div><div class="button-group"><button class="button primary" type="submit" ${busy||!canRun?"disabled":""}>${busy?t("Preparando…","Preparing…"):t("Reconocer en la cola","Recognize in queue")}</button>${action("ocr-refresh",t("Actualizar componentes","Refresh components"),"retry","quiet",loading?"disabled":"")}</div></form></section>`;
  }
  function renderInspector(item:LibraryItem):string{
    if(!hasIndex(item))return "";
    if(currentId!==item.id)return `<section class="ocr-review-entry">${action("ocr-open",t("Revisar texto OCR","Review OCR text"),"document","secondary",`data-id="${h(item.id)}"`)}<p class="field-note">${t("Texto local relacionado con esta versión. Puedes buscar, corregir el índice y guardar una copia de texto.","Local text linked to this version. Search, correct its index and save a text copy.")}</p></section>`;
    const page=currentPage();
    const overlay=page&&!page.region&&page.width&&page.height?boxes.map(box=>`<span style="left:${box.x/page.width*100}%;top:${box.y/page.height*100}%;width:${box.width/page.width*100}%;height:${box.height/page.height*100}%"></span>`).join(""):"";
    return `<section class="ocr-review" aria-label="${t("Revisión OCR","OCR review")}"><div class="ocr-heading"><h3>${t("Texto reconocido","Recognized text")}</h3>${action("ocr-open",t("Volver a cargar","Reload"),"retry","quiet",`data-id="${h(item.id)}" ${busy?"disabled":""}`)}</div>${error?`<p role="alert" class="notice error">${h(error)}</p>`:""}${busy?`<p role="status">${t("Cargando…","Loading…")}</p>`:""}
      ${document?.pages.length?`<label>${t("Página","Page")}<select id="ocr-review-page">${document.pages.map(page=>option(String(page.page),`${page.page} · ${page.origin==="original"?t("Texto original","Original text"):t("OCR","OCR")}${page.corrected?` · ${t("Corregido","Corrected")}`:""}`,String(pageNumber))).join("")}</select></label>`:`<p class="field-note">${t("No queda texto generado en este índice.","No generated text remains in this index.")}</p>`}
      ${preview?`<figure class="ocr-preview"><div class="ocr-preview-layer"><img src="${preview}" alt="${t("Página o imagen de origen","Source page or image")}">${overlay}</div></figure>`:""}${previewError?`<p class="field-note">${h(previewError)}</p>`:""}
      ${page?`<p class="field-note">${page.origin==="original"?t("Esta página ya tenía texto útil. Se conservó sin aplicar OCR.","This page already had useful text. It was preserved without OCR."):page.corrected?t("Índice corregido. Se retiraron las coordenadas anteriores para no señalar posiciones inexactas.","Corrected index. Previous coordinates were removed to avoid inaccurate highlighting."):page.text?t("Texto estimado por OCR. Revisa nombres, cifras y acentos.","Estimated OCR text. Review names, figures and accents."):t("No se detectó texto en esta página.","No text was detected on this page.")}${page.region?` ${t("Región del original","Original region")}: ${page.region.x},${page.region.y} · ${page.region.width}×${page.region.height} px.`:""}</p><form id="ocr-correction-form"><label>${t("Texto de esta página","Page text")}<textarea id="ocr-corrected-text" rows="10" maxlength="1000000" ${page.origin==="original"?"readonly":""}>${h(edited)}</textarea></label><div class="button-group">${page.origin==="ocr"?`<button type="submit" class="button secondary" ${busy?"disabled":""}>${t("Guardar corrección del índice","Save index correction")}</button>`:""}${action("ocr-copy",t("Copiar texto","Copy text"),"copy","quiet",!edited?"disabled":"")}</div></form>`:""}
      ${document?.pages.length?`<form id="ocr-search-form"><label>${t("Buscar en el texto local","Search local text")}<input id="ocr-search" type="search" required minlength="2" maxlength="160" value="${h(query)}"></label><button class="button secondary" type="submit" ${busy?"disabled":""}>${t("Buscar","Search")}</button></form>${hits===null?"":hits.length?`<ul class="ocr-hits">${hits.map(hit=>`<li><button class="button quiet" data-action="ocr-go-hit" data-page="${hit.page}"><span>${t("Página","Page")} ${hit.page} · ${hit.origin==="ocr"?"OCR":t("Original","Original")}</span><span>${h(hit.excerpt)}</span></button></li>`).join("")}</ul>`:`<p>${t("No hay coincidencias.","No matches.")}</p>`}<div class="button-group">${action("ocr-save-text",t("Guardar texto como resultado nuevo","Save text as a new result"),"downloads","secondary",busy?"disabled":"")}${action("ocr-remove-generated",t("Retirar índice OCR","Remove OCR index"),"close","quiet",busy||!document.pages.some(page=>page.origin==="ocr")?"disabled":"")}</div><p class="field-note">${t("Corregir o retirar cambia el índice local. No modifica el PDF, la imagen ni su capa de texto. Guarda el texto corregido como un resultado nuevo.","Correcting or removing changes the local index. It does not modify the PDF, image or its text layer. Save corrected text as a new result.")}</p>`:""}</section>`;
  }
  async function previewPage():Promise<void>{
    preview="";previewError="";const source=sourceItem(currentId),page=currentPage(),request=generation;
    if(!source||!page)return;
    try{
      const rendered=await hooks.command<{png:number[]}>("ocr_preview",{id:currentId,page:page.page});if(request===generation&&rendered.png.length<=8*1048576)preview=png(rendered.png);
    }catch(reason){if(request===generation)previewError=reason instanceof Error?reason.message:String(reason);}
  }
  async function perform(operation:()=>Promise<void>):Promise<void>{if(busy)return;busy=true;error="";hooks.render();try{await operation();}catch(reason){error=reason instanceof Error?reason.message:String(reason);}finally{busy=false;hooks.render();}}
  async function open(id:string,location?:{page:number;query:string}):Promise<void>{currentId=id;document=null;hits=null;boxes=[];generation++;await perform(async()=>{document=await hooks.command<OcrDocument>("ocr_get",{id});pageNumber=location?.page??document.pages[0]?.page??1;edited=currentPage()?.text??"";if(location){query=location.query;hits=await hooks.command<Hit[]>("ocr_search",{id,query});boxes=hits.find(hit=>hit.page===pageNumber)?.boxes??[];}await previewPage();});}
  async function click(name:string,button:HTMLButtonElement):Promise<boolean>{
    if(!name.startsWith("ocr-"))return false;
    switch(name){
      case "ocr-settings":hooks.settings();break;
      case "ocr-refresh":await load();break;
      case "ocr-install-model":{const id=button.dataset.id;if(id)await perform(async()=>{await hooks.command("install_ocr_model",{id});await load();});break;}
      case "ocr-open":if(button.dataset.id)await open(button.dataset.id);break;
      case "ocr-copy":await navigator.clipboard.writeText(edited);hooks.notify(t("Texto copiado.","Text copied."));break;
      case "ocr-save-text":await perform(async()=>{await hooks.command("ocr_save_text",{id:currentId,clientRequest:crypto.randomUUID()});await hooks.refresh();hooks.notify(t("La copia de texto está en Descargas.","The text copy is in Downloads."),"success");});break;
      case "ocr-remove-generated":await perform(async()=>{document=await hooks.command<OcrDocument>("ocr_remove_generated",{id:currentId});pageNumber=document.pages[0]?.page??1;edited=currentPage()?.text??"";hits=null;boxes=[];await hooks.refresh();await previewPage();});break;
      case "ocr-go-hit":{const number=Number(button.dataset.page),hit=hits?.find(hit=>hit.page===number);if(hit){pageNumber=number;edited=currentPage()?.text??"";boxes=hit.boxes;generation++;await previewPage();}break;}
      default:return false;
    }
    hooks.render();return true;
  }
  function change(target:HTMLInputElement|HTMLSelectElement|HTMLTextAreaElement):boolean{
    if(target instanceof HTMLInputElement&&target.dataset.ocrLanguage){const id=target.dataset.ocrLanguage;if(target.checked&&!languages.includes(id))languages.push(id);else if(!target.checked)languages=languages.filter(language=>language!==id);hooks.render();return true;}
    switch(target.id){
      case "ocr-input":selected=target.value;if(!pdf())output="text";hooks.render();return true;
      case "ocr-pages":pages=target.value;return true;
      case "ocr-dpi":dpi=Number(target.value);return true;
      case "ocr-output":output=target.value==="searchablePdf"?"searchablePdf":"text";hooks.render();return true;
      case "ocr-crop":if(target instanceof HTMLInputElement)crop=target.checked;hooks.render();return true;
      case "ocr-acknowledge":if(target instanceof HTMLInputElement)acknowledge=target.checked;return true;
      case "ocr-region-x":region.x=Number(target.value);return true;
      case "ocr-region-y":region.y=Number(target.value);return true;
      case "ocr-region-width":region.width=Number(target.value);return true;
      case "ocr-region-height":region.height=Number(target.value);return true;
      case "ocr-corrected-text":edited=target.value;return true;
      case "ocr-search":query=target.value;return true;
      case "ocr-review-page":pageNumber=Number(target.value);edited=currentPage()?.text??"";boxes=[];generation++;void previewPage().then(hooks.render);hooks.render();return true;
      default:return false;
    }
  }
  async function submit(form:HTMLFormElement):Promise<boolean>{
    if(form.id==="ocr-form"){
      const options:Options={languages:[...languages],pages:pdf()?pages:"1",dpi,region:!pdf()&&crop?{...region}:null,output,acknowledgeStructureChanges:acknowledge};
      await perform(async()=>{await hooks.command("queue_ocr",{id:selected,options,clientRequest:crypto.randomUUID()});await hooks.refresh();hooks.notify(t("OCR añadido a Descargas. El original se conserva.","OCR added to Downloads. The original is preserved."),"success");});return true;
    }
    if(form.id==="ocr-correction-form"){await perform(async()=>{document=await hooks.command<OcrDocument>("ocr_correct_page",{id:currentId,page:pageNumber,text:edited});edited=currentPage()?.text??"";boxes=[];hits=null;await hooks.refresh();hooks.notify(t("Índice corregido. El archivo original se conserva.","Index corrected. The original file is preserved."),"success");});return true;}
    if(form.id==="ocr-search-form"){await perform(async()=>{hits=await hooks.command<Hit[]>("ocr_search",{id:currentId,query});const hit=hits[0];if(hit){pageNumber=hit.page;edited=currentPage()?.text??"";boxes=hit.boxes;generation++;await previewPage();}});return true;}
    return false;
  }
  return {renderTools,renderInspector,open,click,change,submit,load};
}
