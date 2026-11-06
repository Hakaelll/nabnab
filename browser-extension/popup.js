const connection = document.querySelector("#connection");
const message = document.querySelector("#message");
const send = document.querySelector("#send");
const retry = document.querySelector("#retry");
let pendingRequest;
async function call(value) {
  const response = await chrome.runtime.sendMessage(value);
  if (!response?.ok) throw new Error(response?.error || "La extensión no ha recibido respuesta.");
  return response.result;
}
document.querySelector("#settings").addEventListener("click", () => chrome.runtime.openOptionsPage());
async function update() {
  try { await call({ kind: "health" }); connection.textContent = "Conectado a nabnab"; }
  catch (error) { connection.textContent = error.message; }
  try { retry.hidden = !(await call({ kind: "pending" })).pending; } catch { retry.hidden = true; }
  try{const presets=await call({kind:'presets'});const group=document.createElement('optgroup');group.label='Ajustes de nabnab';for(const preset of presets){const option=document.createElement('option');option.value='preset:'+preset.id;option.textContent=preset.name+' · v'+preset.revision;group.append(option);}document.querySelector('#profile').append(group);}catch{/* Older desktop versions keep their supported format profiles. */}
}
async function submit(value) {
  send.disabled = true; retry.disabled = true;
  message.textContent = "Esperando confirmación de nabnab…";
  try {
    const receipt = await call(value);
    message.textContent = receipt.acceptedDraftId ? receipt.intent==='save'?'Guardado en Biblioteca > Para después. No se inició ninguna descarga.':'Recibido en el preparador de nabnab. Revisa los enlaces antes de añadirlos a la cola.' : receipt.acceptedJobIds.length === 1 ? "Recibido en nabnab. Consulta Descargas en la aplicación." : "Recibido en nabnab. Consulta los trabajos en Descargas.";
    pendingRequest = undefined;
    retry.hidden = true;
    return true;
  } catch (error) {
    message.textContent = error.message;
    retry.hidden = false;
    return false;
  } finally { send.disabled = false; retry.disabled = false; }
}
send.addEventListener("click", async () => {
  try{
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab?.url) { message.textContent = "No se puede leer la dirección de esta pestaña."; return; }
  const selected=document.querySelector('#profile').value;
  const presetId=selected.startsWith('preset:')?selected.slice(7):null;
  const profile=presetId?'mp4':selected;
  const intent=document.querySelector('#intent').value;
  const urls=[...document.querySelectorAll('#candidate-links input:checked')].map(input=>input.value);
  if(urls.length&&intent==='download'){message.textContent='Elige Preparar en nabnab o Guardar para después para enviar una selección.';return;}
  const request={url:tab.url,profile,intent,...(presetId?{presetId}:{}),...(urls.length?{urls}:{})};
  if (!pendingRequest || JSON.stringify({...pendingRequest,requestId:undefined})!==JSON.stringify(request)) pendingRequest = { ...request, requestId: crypto.randomUUID() };
  await submit({ kind: "enqueue", request: pendingRequest });
  }catch(error){message.textContent=error.message;}
});
document.querySelector('#collect').addEventListener('click',async()=>{
  const button=document.querySelector('#collect');button.disabled=true;
  try{
    const candidates=await call({kind:'collect'});const list=document.querySelector('#candidate-links');list.replaceChildren();
    for(const candidate of candidates){const row=document.createElement('li');const label=document.createElement('label');const input=document.createElement('input');input.type='checkbox';input.value=candidate.url;const text=document.createElement('span');text.textContent=candidate.title||candidate.url;text.title=candidate.url;label.append(input,text);row.append(label);list.append(row);}
    document.querySelector('#candidates').hidden=false;document.querySelector('#intent').value='prepare';
    message.textContent=candidates.length?candidates.length+' enlaces candidatos. Selecciona cuáles enviar.':'No se encontraron enlaces HTTP en esta página.';
  }catch(error){message.textContent=error.message;}finally{button.disabled=false;}
});
document.querySelector('#select-visible').addEventListener('change',event=>{for(const input of document.querySelectorAll('#candidate-links input'))input.checked=event.target.checked;});
retry.addEventListener("click", () => { void submit({ kind: "retry" }); });

const detectMedia = document.querySelector("#detect-media");
const sendMedia = document.querySelector("#send-media");
const mediaCandidates = document.querySelector("#media-candidates");
const mediaStatus = document.querySelector("#media-status");
let pendingMedia;
const mediaSize = value => value === null ? "Tamaño desconocido" : value < 1024 ** 2 ? `${Math.round(value / 1024)} KiB observados` : `${(value / 1024 ** 2).toFixed(1)} MiB observados`;
detectMedia.addEventListener("click", async () => {
  detectMedia.disabled = true; sendMedia.disabled = true;
  mediaCandidates.replaceChildren(); document.querySelector("#detected-media").hidden = false;
  mediaStatus.textContent = "Inspeccionando los medios ya cargados en esta pestaña…";
  try {
    const result = await call({ kind: "detectMedia" });
    for (const candidate of result.candidates) {
      const row = document.createElement("li"), label = document.createElement("label"), input = document.createElement("input"), text = document.createElement("span"), details = document.createElement("small");
      input.type = "checkbox"; input.value = candidate.id; input.disabled = candidate.protected;
      text.textContent = `${candidate.format}${candidate.width && candidate.height ? ` · ${candidate.width} × ${candidate.height}` : ""} · ${candidate.host}`;
      details.textContent = candidate.protected ? "DRM detectado. No se puede enviar." : candidate.manifest ? "Manifiesto. Compatibilidad y DRM pendientes de comprobar." : mediaSize(candidate.size);
      text.append(details); label.append(input, text); row.append(label); mediaCandidates.append(row);
    }
    mediaStatus.textContent = `${result.candidates.length} candidatos. ${result.blobs ? `${result.blobs} fuentes blob no se pueden descargar desde el escritorio. ` : ""}${result.drmDetected ? "Se detectó reproducción protegida. " : ""}La detección no garantiza una descarga. Las direcciones firmadas se ocultan.`;
  } catch (error) { mediaStatus.textContent = error.message; }
  finally { detectMedia.disabled = false; }
});
mediaCandidates.addEventListener("change", () => { sendMedia.disabled = !mediaCandidates.querySelector("input:checked"); });
sendMedia.addEventListener("click", async () => {
  const ids = [...mediaCandidates.querySelectorAll("input:checked")].map(input => input.value);
  if (!ids.length) return;
  const selected = document.querySelector("#profile").value;
  const presetId = selected.startsWith("preset:") ? selected.slice(7) : null;
  const request = { kind: "sendDetectedMedia", ids, profile: presetId ? "source" : selected, ...(presetId ? { presetId } : {}) };
  const key = JSON.stringify(request);
  if (!pendingMedia || pendingMedia.key !== key) pendingMedia = { key, requestId: crypto.randomUUID() };
  sendMedia.disabled = true;
  try { if (await submit({ ...request, requestId: pendingMedia.requestId })) pendingMedia = undefined; }
  finally { sendMedia.disabled = false; }
});
document.querySelector("#clear-media").addEventListener("click", async () => {
  await call({ kind: "clearDetectedMedia" }); mediaCandidates.replaceChildren(); document.querySelector("#detected-media").hidden = true; sendMedia.disabled = true;
});
void update();
