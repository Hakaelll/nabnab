import { escapeHtml as h, type Copy } from "./contracts";
import { action } from "./views-common";

interface AuthStatus { origin: string; configured: boolean }
interface AuthHooks { command<T>(name: string,args?: Record<string,unknown>): Promise<T>; t: Copy; render(): void; notify(message: string,kind?: "info"|"success"|"error"): void; showError(error: unknown): void }
export function createFederatedAuthController(hooks: AuthHooks) {
  const { t } = hooks;
  let origin = "";
  let status: AuthStatus | null = null;
  let consent = false;
  let busy = false;
  async function perform(operation: () => Promise<void>) { if (busy) return; busy = true; hooks.render(); try { await operation(); } catch (error) { hooks.showError(error); } finally { busy = false; hooks.render(); } }
  function render(): string {
    return `<div class="page-heading"><div><h1>${t("Acceso a Mastodon", "Mastodon access")}</h1><p>${t("Usa una autorización de tu cuenta para una instancia concreta.", "Use your account authorization for one specific instance.")}</p></div></div><form id="federated-auth-form" class="settings-form" aria-busy="${busy}"><label>${t("Origen HTTPS de la instancia", "Instance HTTPS origin")}<input id="federated-auth-origin" type="url" value="${h(origin)}" required maxlength="2048" placeholder="https://social.example.org" autocomplete="off" ${busy ? "disabled" : ""}></label>${action("federated-auth-check",t("Comprobar token local","Check local token"),undefined,"secondary",busy ? "disabled" : "")}${status ? `<p role="status">${status.configured ? t("Hay un token protegido guardado para este origen. Sus permisos se comprobarán al abrir una publicación.","A protected token is stored for this origin. Its permissions will be checked when opening a post.") : t("No hay un token guardado. Las publicaciones públicas pueden explorarse sin él.","No token is stored. Public posts can be inspected without one.")}</p>` : ""}<label>${t("Token OAuth ya autorizado", "Already-authorized OAuth token")}<input id="federated-auth-token" type="password" required maxlength="4096" autocomplete="off" spellcheck="false" ${busy ? "disabled" : ""}></label><p class="field-note">${t("En tu instancia, abre Preferencias → Desarrollo y configura una aplicación autorizada con el permiso read:statuses. Copia su token de acceso de usuario. nabnab no registra una aplicación ni inicia sesión por ti.", "In your instance, open Preferences → Development and configure an authorized application with the read:statuses permission. Copy its user access token. nabnab does not register an application or log in for you.")}</p><label><input id="federated-auth-consent" type="checkbox" required ${consent ? "checked" : ""}>${t("Autorizar el uso de este token solo con este origen HTTPS.","Allow this token to be used only with this HTTPS origin.")}</label><p class="field-note">${t("Se guarda protegido para este usuario de Windows. No se muestra después de guardarlo ni se envía a otras instancias o a los hosts de medios. La API seguirá aplicando los permisos de tu cuenta.","It is stored protected for this Windows user. It is not displayed after saving or sent to other instances or media hosts. The API continues to enforce your account permissions.")}</p><div class="button-group"><button type="submit" class="button primary" ${busy || !consent ? "disabled" : ""}>${t("Guardar token para esta instancia","Save token for this instance")}</button>${action("federated-auth-remove",t("Eliminar token local","Remove local token"),"close","quiet danger",busy || !status?.configured ? "disabled" : "")}</div><p class="field-note">${t("Eliminarlo de nabnab conserva tus archivos. Para revocar también su autorización en el servidor, usa las aplicaciones autorizadas de tu instancia.","Removing it from nabnab keeps your files. To revoke its server authorization too, use your instance's authorized applications.")}</p></form>`;
  }
  function handleInput(element: HTMLElement): boolean {
    if (!(element instanceof HTMLInputElement)) return false;
    if (element.id === "federated-auth-origin") { origin = element.value; status = null; consent = false; hooks.render(); return true; }
    if (element.id === "federated-auth-consent") { consent = element.checked; const button = document.querySelector<HTMLButtonElement>("#federated-auth-form button[type=submit]"); if (button) button.disabled = busy || !consent; return true; }
    return false;
  }
  async function handleClick(name: string): Promise<boolean> {
    switch (name) {
      case "federated-auth-check": await perform(async () => { status = await hooks.command<AuthStatus>("federated_auth_status", { origin }); origin = status.origin; }); break;
      case "federated-auth-remove": if (status?.configured) await perform(async () => { status = await hooks.command<AuthStatus>("set_federated_token", { origin, token: "", remove: true, consent: true }); hooks.notify(t("Token local eliminado. Los archivos se conservan.","Local token removed. Files are kept."),"success"); }); break;
      default: return false;
    }
    return true;
  }
  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    if (form.id !== "federated-auth-form") return false;
    const input = document.getElementById("federated-auth-token");
    if (!(input instanceof HTMLInputElement) || !input.value || !consent) return true;
    const token = input.value;
    await perform(async () => { status = await hooks.command<AuthStatus>("set_federated_token", { origin, token, remove: false, consent }); origin = status.origin; hooks.notify(t("Token protegido guardado para esta instancia.","Protected token saved for this instance."),"success"); });
    return true;
  }
  return { render, handleInput, handleClick, handleSubmit, async open(input: string) { try { origin = new URL(input).origin; } catch { origin = input; } consent = false; status = null; hooks.render(); if (origin) await perform(async () => { status = await hooks.command<AuthStatus>("federated_auth_status", { origin }); }); } };
}
