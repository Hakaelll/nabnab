import { escapeHtml as h, type Copy, type Job, type JobSource, type LibraryItem, type Snapshot } from "./contracts";
import { bytesLabel } from "./presentation";
import { action, icon } from "./views-common";

type ImageExportSource = Extract<JobSource, { kind: "upscale" | "background" | "sticker" }>;

export function publishedImageOutput(job: Job, library: LibraryItem[]): LibraryItem | undefined {
  const { source } = job;
  if (source.kind !== "upscale" && source.kind !== "background" && source.kind !== "sticker") return;
  if (job.state !== "completed" || job.outputs.length !== 1) return;
  return library.find(item => !item.missing && item.path === job.outputs[0] && item.parentId === source.selection.libraryId);
}

export function createStickerAction(job: Job, item: LibraryItem | undefined, t: Copy): string {
  if (job.source.kind !== "background" || job.source.selection.options.fill?.kind !== "transparent" || item?.kind !== "image") return "";
  const label = t("Crear sticker", "Create sticker");
  return action("sticker-from-cutout", label, "image", "secondary", `data-id="${h(item.id)}" aria-label="${h(`${label} · ${item.name}`)}"`);
}

export function imageExports(snapshot: Snapshot, kind: ImageExportSource["kind"], t: Copy): string {
  const jobs = snapshot.jobs.filter((job): job is Job & { source: ImageExportSource } => job.source.kind === kind);
  const latest = jobs.at(-1);
  if (!latest) return "";
  const batch = latest.groupId ? jobs.filter(job => job.groupId === latest.groupId) : [latest];
  const files = batch.flatMap(job => {
    const item = publishedImageOutput(job, snapshot.library);
    return item ? [{ job, item }] : [];
  });
  if (!files.length) return "";
  const share = t("Enviar a dispositivo", "Send to device");
  return `<section class="asset-review asset-exports" aria-labelledby="asset-exports-title">
    <div class="section-heading"><h3 id="asset-exports-title">${t("Última exportación", "Latest export")}</h3>
      ${batch.length > 1 ? `<span>${t(`${files.length} de ${batch.length} disponibles`, `${files.length} of ${batch.length} available`)}</span>` : ""}</div>
    <ul class="asset-export-files">${files.map(({ job, item }) => {
      const id = h(item.id);
      const dimensions = item.width && item.height ? `${item.width} × ${item.height} px · ` : "";
      const sticker = createStickerAction(job, item, t);
      return `<li><div class="asset-export-file">${icon(item.kind === "video" ? "video" : "image")}
        <span><strong>${h(item.name)}</strong><small>${dimensions}${bytesLabel(item.size)}</small></span></div>
        <div class="button-group">${sticker}${action("share-file", share, "arrow", sticker ? "quiet" : "secondary",
          `id="asset-export-share-${id}" data-id="${id}" aria-label="${h(`${share} · ${item.name}`)}"`)}</div>
      </li>`;
    }).join("")}</ul>
  </section>`;
}
