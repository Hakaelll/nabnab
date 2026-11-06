import {escapeHtml as h,type Copy,type Snapshot} from './contracts';
import {detailsFor,metadataText} from './library-features';
import {durationLabel} from './presentation';

interface Result {id:string;kind:string;label:string;context:string;run:()=>void|Promise<void>}
interface Hooks {snapshot:()=>Snapshot;t:Copy;prepare:()=>void;addSource:()=>void;inspect:(id:string,time?:number)=>void;collection:(name:string)=>void;preset:(id:string)=>void|Promise<void>;source:(id:string)=>void;showError:(error:unknown)=>void}
export function createPalette(hooks:Hooks){
  const dialog=document.createElement('dialog');dialog.id='action-palette';dialog.className='action-palette';dialog.setAttribute('aria-label',hooks.t('Acciones y búsqueda local','Actions and local search'));document.body.append(dialog);
  let previous:Element|null=null;let selected=0;let results:Result[]=[];
  function find(query:string):Result[]{
    const t=hooks.t;const snapshot=hooks.snapshot();const needle=query.trim().toLowerCase();
    const actions:Result[]=[{id:'prepare',kind:t('Acción','Action'),label:t('Preparar lote','Prepare batch'),context:t('Inicio · pegar enlaces o importar .txt','Home · paste links or import .txt'),run:hooks.prepare},{id:'source-add',kind:t('Acción','Action'),label:t('Añadir fuente','Add source'),context:'YouTube',run:hooks.addSource}];
    for(const preset of snapshot.presets??[])actions.push({id:`preset:${preset.id}`,kind:t('Ajuste','Preset'),label:preset.name,context:t('Aplicar a la selección del lote o editar ajuste','Apply to batch selection or edit preset'),run:()=>hooks.preset(preset.id)});
    for(const name of snapshot.collections)actions.push({id:`collection:${name}`,kind:t('Colección','Collection'),label:name,context:t('Colección manual','Manual collection'),run:()=>hooks.collection(name)});
    for(const collection of snapshot.smartCollections??[])actions.push({id:`smart:${collection.id}`,kind:t('Colección','Collection'),label:collection.name,context:t('Colección inteligente','Smart collection'),run:()=>hooks.collection(`smart:${collection.id}`)});
    for(const source of snapshot.sources??[])actions.push({id:`source:${source.id}`,kind:t('Fuente','Source'),label:source.name,context:source.url,run:()=>hooks.source(source.id)});
    const matches=actions.filter(result=>!needle||`${result.label} ${result.context}`.toLowerCase().includes(needle));
    for(const file of snapshot.library){
      if(!needle||metadataText(file).toLowerCase().includes(needle))matches.push({id:`file:${file.id}`,kind:t('Archivo','File'),label:file.name,context:file.missing?t('No disponible en disco','Unavailable on disk'):file.path,run:()=>hooks.inspect(file.id)});
      if(needle)for(const track of detailsFor(file).tracks)for(const [index,segment] of track.segments.entries())if(segment.text.toLowerCase().includes(needle)){matches.push({id:`cue:${file.id}:${track.id}:${index}`,kind:t('Subtítulo','Subtitle'),label:segment.text,context:`${file.name} · ${durationLabel(segment.start)} · ${track.language}`,run:()=>hooks.inspect(file.id,segment.start)});if(matches.length>=80)return matches;}
      if(matches.length>=80)break;
    }
    return matches.slice(0,80);
  }
  function update():void{
    const input=dialog.querySelector<HTMLInputElement>('#palette-query');results=find(input?.value??'');selected=Math.max(0,Math.min(selected,results.length-1));
    const list=dialog.querySelector('#palette-results');if(!list)return;
    list.innerHTML=results.length?results.map((result,index)=>`<li id="palette-option-${index}" role="option" aria-selected="${index===selected}" data-palette-index="${index}"><span class="palette-kind">${h(result.kind)}</span><strong>${h(result.label)}</strong><small>${h(result.context)}</small></li>`).join(''):`<li class="palette-empty">${hooks.t('No hay coincidencias locales. Prueba con otro título o acción.','No local matches. Try another title or action.')}</li>`;
    if(results.length)input?.setAttribute('aria-activedescendant',`palette-option-${selected}`);else input?.removeAttribute('aria-activedescendant');
    dialog.querySelector('#palette-status')?.replaceChildren(document.createTextNode(`${results.length} ${hooks.t('resultados','results')}`));
    dialog.querySelector<HTMLElement>(`#palette-option-${selected}`)?.scrollIntoView({block:'nearest'});
  }
  function close():void{dialog.close();if(previous instanceof HTMLElement&&previous.isConnected)previous.focus({preventScroll:true});}
  function open():void{if(dialog.open)return;previous=document.activeElement;selected=0;dialog.innerHTML=`<label for="palette-query">${hooks.t('Acciones y búsqueda local','Actions and local search')}</label><input id="palette-query" type="search" role="combobox" aria-autocomplete="list" aria-expanded="true" aria-controls="palette-results" autocomplete="off" placeholder="${hooks.t('Buscar acciones, archivos, colecciones, ajustes y fuentes','Search actions, files, collections, presets and sources')}"><ul id="palette-results" role="listbox"></ul><div class="palette-footer"><span id="palette-status" role="status"></span><span>↑ ↓ · Enter · Esc</span></div>`;dialog.showModal();update();dialog.querySelector<HTMLInputElement>('#palette-query')?.focus();}
  function execute():void{const result=results[selected];if(!result)return;close();Promise.resolve().then(result.run).catch(hooks.showError);}
  dialog.addEventListener('input',()=>{selected=0;update();});
  dialog.addEventListener('click',event=>{if(event.target===dialog){close();return;}if(!(event.target instanceof HTMLElement))return;const row=event.target.closest<HTMLElement>('[data-palette-index]');if(row){selected=Number(row.dataset.paletteIndex);execute();}});
  dialog.addEventListener('keydown',event=>{if(event.key==='ArrowDown'||event.key==='ArrowUp'){event.preventDefault();selected=(selected+(event.key==='ArrowDown'?1:-1)+Math.max(1,results.length))%Math.max(1,results.length);update();}else if(event.key==='Enter'){event.preventDefault();execute();}});
  dialog.addEventListener('cancel',event=>{event.preventDefault();close();});
  return {open,close,get isOpen(){return dialog.open;}};
}
