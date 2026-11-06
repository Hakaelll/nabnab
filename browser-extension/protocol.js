const base = "http://127.0.0.1:49587";
const profiles = new Set(["mp4", "mp3", "source", "audio"]);

export function makeRequest({ url, profile, requestId, intent, urls, presetId }) {
  if (typeof url !== "string" || url.length > 8192) throw new Error("El enlace no es válido.");
  let parsed;
  try { parsed = new URL(url); } catch { throw new Error("Abre una página con un enlace HTTP o HTTPS."); }
  if (!["http:", "https:"].includes(parsed.protocol) || !parsed.hostname || parsed.username || parsed.password) {
    throw new Error("Solo se pueden enviar enlaces HTTP o HTTPS sin credenciales.");
  }
  if (!profiles.has(profile)) throw new Error("El perfil no es válido.");
  if (typeof requestId !== "string" || !/^[a-zA-Z0-9_.-]{8,128}$/.test(requestId)) throw new Error("El identificador de solicitud no es válido.");
  const request={ version: 1, requestId, url: parsed.href, profile };
  if(intent!==undefined){if(!['download','prepare','save'].includes(intent))throw new Error('El destino no es válido.');request.intent=intent;}
  if(urls!==undefined){if(!Array.isArray(urls)||!urls.length||urls.length>200||intent==='download')throw new Error('Envía una selección de hasta 200 enlaces al preparador.');request.urls=[...new Set(urls.map(value=>makeRequest({url:value,profile,requestId}).url))];}
  if(presetId!==undefined&&presetId!==null){if(typeof presetId!=='string'||!/^[a-zA-Z0-9_.:-]{1,192}$/.test(presetId))throw new Error('Elige un ajuste local válido.');request.presetId=presetId;}
  return request;
}

export function validToken(token) {
  return typeof token === "string" && /^[a-f0-9]{64}$/.test(token);
}

export function createClient({ token, fetcher = globalThis.fetch }) {
  if (!validToken(token)) throw new Error("Empareja la extensión en sus ajustes.");
  async function call(path, body) {
    let response;
    try {
      response = await fetcher(base + path, {
        method: "POST",
        mode: "cors", credentials: "omit", cache: "no-store", redirect: "error",
        headers: { Authorization: "Bearer " + token, "Content-Type": "application/json" },
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(10000),
      });
    } catch { throw new Error("Abre nabnab y comprueba el emparejamiento en Ajustes > Navegador."); }
    if (response.status === 401 || response.status === 403) throw new Error("nabnab ha rechazado el emparejamiento. Genera un token nuevo en la aplicación.");
    let payload;
    try { payload = await response.json(); } catch { throw new Error("La respuesta de nabnab no es válida."); }
    if (!response.ok) throw new Error(typeof payload?.error === "string" ? payload.error.slice(0, 500) : "nabnab no ha aceptado la solicitud.");
    return payload;
  }
  return {
    async health() {
      const response = await call("/health", { version: 1 });
      if (response?.version !== 1 || response.product !== "nabnab" || response.connected !== true) throw new Error("La versión del enlace con nabnab no es compatible.");
      return true;
    },
    async enqueue(request) {
      const outgoing = makeRequest(request);
      const response = await call("/enqueue", outgoing);
      const draft=outgoing.intent==='prepare'||outgoing.intent==='save';
      if (response?.version !== 1 || response.requestId !== outgoing.requestId || !Array.isArray(response.acceptedJobIds)
        || (draft ? response.intent!==outgoing.intent||response.acceptedJobIds.length!==0||typeof response.acceptedDraftId!=='string'||!response.acceptedDraftId.length||response.acceptedDraftId.length>192 : !response.acceptedJobIds.length || !response.acceptedJobIds.every(id => typeof id === "string" && id.length > 0 && id.length <= 128))) {
        throw new Error("nabnab no ha confirmado la recepción. Reintenta la misma solicitud.");
      }
      return response;
    },
    async presets(){
      const response=await call('/presets',{version:1});
      if(response?.version!==1||!Array.isArray(response.presets)||response.presets.length>1000||!response.presets.every(preset=>preset&&typeof preset.id==='string'&&/^[a-zA-Z0-9_.:-]{1,192}$/.test(preset.id)&&typeof preset.name==='string'&&preset.name.length<=120&&Number.isInteger(preset.revision)&&preset.revision>0))throw new Error('La lista de ajustes recibida no es válida.');
      return response.presets.map(({id,name,revision})=>({id,name,revision}));
    },
  };
}
