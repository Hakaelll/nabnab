document.querySelector("#extension-id").value = chrome.runtime.id;
const message = document.querySelector("#message");
const token = document.querySelector("#token");
document.querySelector("#extension-id").addEventListener("click", event => event.target.select());
document.querySelector("#pair-form").addEventListener("submit", async event => {
  event.preventDefault();
  const button = event.submitter;
  button.disabled = true;
  try {
    const response = await chrome.runtime.sendMessage({ kind: "savePairing", token: token.value.trim() });
    token.value = "";
    if (!response?.ok) throw new Error(response?.error || "No se ha podido conectar.");
    message.textContent = "Conectado a nabnab. Ya puedes enviar pestañas y enlaces.";
  } catch (error) { message.textContent = error.message; }
  finally { button.disabled = false; }
});
document.querySelector("#forget").addEventListener("click", async () => {
  const response = await chrome.runtime.sendMessage({ kind: "forget" });
  token.value = "";
  message.textContent = response?.ok ? "Token eliminado de este navegador. Revoca el cliente en nabnab para invalidarlo." : "No se ha podido eliminar el token.";
});
