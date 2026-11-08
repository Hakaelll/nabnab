import { escapeHtml as h, type Copy, type Job, type LibraryItem } from "./contracts";
import { action } from "./views-common";

export function publishedTranslationFile(job: Job | undefined, library: LibraryItem[]): LibraryItem | undefined {
  if (job?.state !== "completed" || job.outputs.length !== 1) return;
  return library.find(item => !item.missing && item.path === job.outputs[0]);
}

export function translationOutput(item: LibraryItem, label: string, t: Copy, playLabel?: string): string {
  const id = h(item.id);
  const share = t("Enviar a dispositivo", "Send to device");
  return `<div class="translation-output"><div><p class="field-note">${h(label)}</p><strong>${h(item.name)}</strong></div>
    <div class="button-group">${playLabel ? action("play-media", playLabel, "play", "secondary", `data-id="${id}"`) : ""}
      ${action("share-file", share, "arrow", "secondary", `id="translation-share-${id}" data-id="${id}" aria-label="${h(`${share} · ${item.name}`)}"`)}</div></div>`;
}
