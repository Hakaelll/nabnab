import { createClient, makeRequest, validToken } from "./protocol.js";

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
      await chrome.storage.session.remove("pendingRequest");
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
}
chrome.runtime.onInstalled.addListener(() => { void menus(); });
chrome.runtime.onStartup.addListener(() => { void menus(); });
chrome.contextMenus.onClicked.addListener((info, tab) => {
  if (!["nabnab-video", "nabnab-audio"].includes(String(info.menuItemId))) return;
  const url = info.linkUrl || info.pageUrl || tab?.url;
  void send({ url, profile: info.menuItemId === "nabnab-audio" ? "mp3" : "mp4", requestId: crypto.randomUUID() })
    .catch(async () => { await chrome.action.setBadgeText({ text: "!" }); await chrome.action.setBadgeBackgroundColor({ color: "#7e4b42" }); });
});
