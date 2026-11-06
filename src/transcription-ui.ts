import { bytesLabel } from "./presentation";
import { escapeHtml as h, type Copy } from "./contracts";
import { action } from "./views-common";

interface ModelStatus { id: string; bytes: number; multilingual: boolean; license: string; source: string; installed: boolean }
interface Hooks { command: <T>(name: string, args?: Record<string, unknown>) => Promise<T>; render: () => void }
let models: ModelStatus[] | null = null;
let working = "";
let failure = "";

export function transcriptionModelPanel(t: Copy): string {
  return `<section class="transcription-models full-width"><h3>${t("Modelos locales", "Local models")}</h3><p class="field-note">${t("Solo se descarga el modelo al pulsar su botón. Ambos modelos son multilingües, incluyen español y tienen licencia MIT. Procedencia: ggerganov/whisper.cpp en Hugging Face. Tiny necesita menos recursos; Small requiere más memoria. Instala y activa whisper.cpp en Ajustes → Componentes.", "A model downloads only when you press its button. Both models are multilingual, include Spanish and use the MIT license. Source: ggerganov/whisper.cpp on Hugging Face. Tiny needs fewer resources; Small needs more memory. Install and enable whisper.cpp in Settings → Components.")}</p>${failure?`<p class="field-note" role="alert">${h(failure)}</p>`:models?`<ul class="tool-inputs">${models.map(model=>`<li><span><strong>${h(model.id)} · ${bytesLabel(model.bytes)}</strong><small>${t("Multilingüe, español incluido", "Multilingual, includes Spanish")} · ${h(model.license)} · ${model.installed?t("Archivo presente; se verificará al usarlo", "File present; verified when used"):t("Sin descargar", "Not downloaded")}</small></span>${model.installed?`<button class="button quiet" data-action="transcription-remove-model" data-model="${h(model.id)}" ${working?"disabled":""}>${t("Eliminar modelo","Remove model")}</button>`:action("transcription-download-model",working===model.id?t("Descargando…", "Downloading…"):t("Descargar ", "Download ")+bytesLabel(model.bytes),"downloads","secondary",`data-model="${h(model.id)}" ${working?'disabled':''}`)}</li>`).join("")}</ul>`:`<p class="field-note" role="status">${t("Cargando modelos disponibles…", "Loading available models…")}</p>`}</section>`;
}

export function createTranscriptionManager(hooks: Hooks) {
  async function load(): Promise<void> {
    try { models=await hooks.command<ModelStatus[]>("transcription_models");failure=""; }
    catch(error){ failure=error instanceof Error?error.message:String(error); }
    hooks.render();
  }
  async function click(name: string, button: HTMLButtonElement): Promise<boolean> {
    if(name==="transcription-remove-model"){const id=button.dataset.model;if(!id||working)return true;working=id;hooks.render();try{await hooks.command("remove_transcription_model",{id});await load();}catch(error){failure=error instanceof Error?error.message:String(error);}finally{working="";hooks.render();}return true;}
    if(name!=="transcription-download-model")return false;
    const id=button.dataset.model;
    if(working||!id||!models?.some(model=>model.id===id&&!model.installed))return true;
    working=id;failure="";hooks.render();
    try { await hooks.command("install_transcription_model",{id});await load(); }
    catch(error){failure=error instanceof Error?error.message:String(error);}
    finally {working="";hooks.render();}
    return true;
  }
  return {load,click};
}
