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
}
async function submit(value) {
  send.disabled = true; retry.disabled = true;
  message.textContent = "Enviando a la cola…";
  try {
    const receipt = await call(value);
    message.textContent = receipt.acceptedJobIds.length === 1 ? "Recibido en nabnab. Consulta Descargas en la aplicación." : "Recibido en nabnab. Consulta los trabajos en Descargas.";
    pendingRequest = undefined;
    retry.hidden = true;
  } catch (error) {
    message.textContent = error.message;
    retry.hidden = false;
  } finally { send.disabled = false; retry.disabled = false; }
}
send.addEventListener("click", async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab?.url) { message.textContent = "No se puede leer la dirección de esta pestaña."; return; }
  const profile = document.querySelector("#profile").value;
  if (!pendingRequest || pendingRequest.url !== tab.url || pendingRequest.profile !== profile) pendingRequest = { url: tab.url, profile, requestId: crypto.randomUUID() };
  await submit({ kind: "enqueue", request: pendingRequest });
});
retry.addEventListener("click", () => { void submit({ kind: "retry" }); });
void update();
