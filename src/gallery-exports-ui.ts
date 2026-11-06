import { type Copy } from "./contracts";
import { action } from "./views-common";

interface Hooks {
  command:<T>(name:string,args?:Record<string,unknown>)=>Promise<T>;
  selected:()=>string[];
  refresh:()=>Promise<void>;
  notify:(message:string)=>void;
  t:Copy;
}

export function galleryExportActions(t:Copy):string{
  return `${action("gallery-export-html",t("Galería HTML", "HTML gallery"),"image","quiet")}${action("gallery-export-contact",t("Hoja de contacto", "Contact sheet"),"grid","quiet")}`;
}

export function createGalleryExports(hooks:Hooks){
  let working=false;
  async function click(name:string):Promise<boolean>{
    if(name!=="gallery-export-html"&&name!=="gallery-export-contact")return false;
    if(working)return true;
    const ids=hooks.selected();
    if(!ids.length)return true;
    working=true;
    try{
      await hooks.command("queue_gallery_export",{ids,kind:name==="gallery-export-html"?"html":"contactSheet",clientRequest:crypto.randomUUID()});
      await hooks.refresh();
      hooks.notify(hooks.t("Exportación añadida a la cola. Los originales se conservan.","Export added to the queue. Originals are preserved."));
    }finally{working=false;}
    return true;
  }
  return {click};
}
