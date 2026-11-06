import { createClient, makeRequest, validToken } from "./protocol.js";
import { scanMedia, normalizeMediaScan, mediaSummaries, selectedMediaRequest } from "./media.js";

async function client() {
  const { pairingToken } = await chrome.storage.local.get("pairingToken");
  return createClient({ token: pairingToken });
}
async function send(request) {
  const outgoing = makeRequest(request);
  await chrome.storage.session.set({ pendingRequest: outgoing });
  const receipt = await (await client()).enqueue(outgoing);
  await chrome.storage.session.remove("pendingRequest");
  await chrome.action.setBadgeText({ text: "" });
  return receipt;
}
async function handle(message) {
  switch (message?.kind) {
    case "health":
      await (await client()).health();
      return { connected: true };
    case "enqueue":
      return send(message.request);
    case 'presets':return (await client()).presets();
    case "detectMedia": {
      const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
      if (!tab?.id || !tab.url || !/^https?:\/\//i.test(tab.url)) throw new Error("Abre una página HTTP o HTTPS y vuelve a pulsar la extensión.");
      await chrome.storage.session.remove("detectedMedia");
      let result;
      try { result = await chrome.scripting.executeScript({ target: { tabId: tab.id }, func: scanMedia }); }
      catch { throw new Error("El navegador no permite leer esta página. Pulsa la extensión desde la pestaña que quieres inspeccionar."); }
      const raw = result[0]?.result;
      if (raw?.pageUrl !== tab.url) throw new Error("La pestaña ha cambiado durante la detección. Vuelve a intentarlo.");
      const scan = normalizeMediaScan(raw);
      await chrome.storage.session.set({ detectedMedia: { ...scan, tabId: tab.id, pageUrl: tab.url, createdAt: Date.now() } });
      return mediaSummaries(scan);
    }
    case "sendDetectedMedia": {
      const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
      const { detectedMedia } = await chrome.storage.session.get("detectedMedia");
      const request = selectedMediaRequest(detectedMedia, { tabId: tab?.id, pageUrl: tab?.url, ids: message.ids, profile: message.profile, presetId: message.presetId, requestId: message.requestId });
      return send(request);
    }
    case "clearDetectedMedia":
      await chrome.storage.session.remove("detectedMedia");
      return { cleared: true };
    case 'collect':{
      const [tab]=await chrome.tabs.query({active:true,currentWindow:true});
      if(!tab?.id)throw new Error('No hay una página activa.');
      const result=await chrome.scripting.executeScript({target:{tabId:tab.id},func:()=>{
        const found=new Map();
        for(const anchor of document.querySelectorAll('a[href]')){
          try{const parsed=new URL(anchor.href);if(['http:','https:'].includes(parsed.protocol)&&!parsed.username&&!parsed.password&&!found.has(parsed.href))found.set(parsed.href,{url:parsed.href,title:(anchor.textContent||parsed.hostname).trim().slice(0,160)});}catch{}
          if(found.size>=200)break;
        }
        return [...found.values()];
      }});
      return result[0]?.result??[];
    }
    case "retry": {
      const { pendingRequest } = await chrome.storage.session.get("pendingRequest");
      if (!pendingRequest) throw new Error("No hay ninguna solicitud pendiente de confirmar.");
      return send(pendingRequest);
    }
    case "pending": {
      const { pendingRequest } = await chrome.storage.session.get("pendingRequest");
      return { pending: Boolean(pendingRequest) };
    }
    case "savePairing":
      if (!validToken(message.token)) throw new Error("Copia el token de 64 caracteres que muestra nabnab.");
      await chrome.storage.local.set({ pairingToken: message.token });
      await (await client()).health();
      return { connected: true };
    case "forget":
      await chrome.storage.local.remove("pairingToken");
      await chrome.storage.session.remove(["pendingRequest", "detectedMedia"]);
      return { connected: false };
    default:
      throw new Error("Solicitud de extensión desconocida.");
  }
}

chrome.runtime.onMessage.addListener((message, sender, respond) => {
  if (sender.id !== chrome.runtime.id) return false;
  handle(message).then(result => respond({ ok: true, result }), error => respond({ ok: false, error: error instanceof Error ? error.message : "La solicitud ha fallado." }));
  return true;
});

async function menus() {
  await chrome.contextMenus.removeAll();
  chrome.contextMenus.create({ id: "nabnab-video", title: "Enviar vídeo a nabnab", contexts: ["link", "page"] });
  chrome.contextMenus.create({ id: "nabnab-audio", title: "Enviar audio a nabnab", contexts: ["link", "page"] });
  chrome.contextMenus.create({ id: "nabnab-prepare", title: "Preparar enlace en nabnab", contexts: ["link", "page"] });
  chrome.contextMenus.create({ id: "nabnab-save", title: "Guardar para después en nabnab", contexts: ["link", "page"] });
}
chrome.runtime.onInstalled.addListener(() => { void menus(); });
chrome.runtime.onStartup.addListener(() => { void menus(); });
chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (!["nabnab-video", "nabnab-audio",'nabnab-prepare','nabnab-save'].includes(String(info.menuItemId))) return;
  const url = info.linkUrl || info.pageUrl || tab?.url;
  void send({ url, profile: info.menuItemId === "nabnab-audio" ? "mp3" : "mp4", requestId: crypto.randomUUID(),intent:info.menuItemId==='nabnab-prepare'?'prepare':info.menuItemId==='nabnab-save'?'save':'download' })
    .catch(async () => { await chrome.action.setBadgeText({ text: "!" }); await chrome.action.setBadgeBackgroundColor({ color: "#7e4b42" }); });
});
