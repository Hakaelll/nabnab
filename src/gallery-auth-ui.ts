import { type Copy } from "./contracts";

interface AuthStatus { site: "pixiv"; configured: boolean }
interface Hooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  render(): void;
  notify(message: string): void;
  t: Copy;
}

export function createGalleryAuth(hooks: Hooks) {
  let configured = false;
  let busy = false;
  const { t } = hooks;
  async function load(): Promise<void> {
    const status = await hooks.command<AuthStatus>("gallery_auth_status");
    configured = status.configured;
    hooks.render();
  }
  function render(): string {
    return `<section class="settings-section"><h3>Pixiv</h3><p class="field-note">${t("Las obras de Pixiv requieren un refresh token propio. Las cookies del navegador no lo sustituyen. El token se protege en este equipo y solo se usa con el extractor oficial de Pixiv.", "Pixiv artworks require your own refresh token. Browser cookies cannot replace it. The token is protected on this computer and used only by the official Pixiv extractor.")}</p><p role="status">${configured ? t("Token guardado", "Token saved") : t("Sin token configurado", "No token configured")}</p><form id="gallery-auth-form" class="settings-form"><label>${t("Refresh token de Pixiv", "Pixiv refresh token")}<input id="gallery-auth-token" type="password" required maxlength="4096" autocomplete="off" spellcheck="false" ${busy ? "disabled" : ""}></label><div class="button-group"><button class="button secondary" type="submit" ${busy ? "disabled" : ""}>${t("Guardar token", "Save token")}</button><button class="button quiet danger" type="button" data-action="gallery-auth-remove" ${busy || !configured ? "disabled" : ""}>${t("Eliminar token", "Remove token")}</button></div></form></section>`;
  }
  async function save(token: string, remove: boolean): Promise<void> {
    if (busy) return;
    busy = true;
    hooks.render();
    try {
      const status = await hooks.command<AuthStatus>("set_gallery_token", { site: "pixiv", token, remove });
      configured = status.configured;
      hooks.notify(remove ? t("Token de Pixiv eliminado.", "Pixiv token removed.") : t("Token de Pixiv guardado.", "Pixiv token saved."));
    } finally { busy = false; hooks.render(); }
  }
  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    if (form.id !== "gallery-auth-form") return false;
    const field = form.querySelector<HTMLInputElement>("#gallery-auth-token");
    if (!field || busy) return true;
    const token = field.value;
    field.value = "";
    await save(token, false);
    return true;
  }
  async function handleClick(action: string): Promise<boolean> {
    if (action !== "gallery-auth-remove") return false;
    await save("", true);
    return true;
  }
  return { load, render, handleSubmit, handleClick };
}
