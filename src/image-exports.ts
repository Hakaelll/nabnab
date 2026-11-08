import { escapeHtml as h, type Copy, type Job, type JobSource, type Snapshot } from "./contracts";
import { bytesLabel } from "./presentation";
import { action, icon } from "./views-common";

type ImageExportSource = Extract<JobSource, { kind: "upscale" | "background" | "sticker" }>;

export function imageExports(snapshot: Snapshot, kind: ImageExportSource["kind"], t: Copy): string {
  const jobs = snapshot.jobs.filter((job): job is Job & { source: ImageExportSource } => job.source.kind === kind);
  const latest = jobs.at(-1);
  if (!latest) return "";
  const batch = latest.groupId ? jobs.filter(job => job.groupId === latest.groupId) : [latest];
  const files = batch.flatMap(job => {
    if (job.state !== "completed" || job.outputs.length !== 1) return [];
    const item = snapshot.library.find(item => !item.missing && item.path === job.outputs[0]
      && item.parentId === job.source.selection.libraryId);
    return item ? [item] : [];
  });
  if (!files.length) return "";
  const share = t("Enviar a dispositivo", "Send to device");
  return `<section class="asset-review asset-exports" aria-labelledby="asset-exports-title">
    <div class="section-heading"><h3 id="asset-exports-title">${t("Última exportación", "Latest export")}</h3>
      ${batch.length > 1 ? `<span>${t(`${files.length} de ${batch.length} disponibles`, `${files.length} of ${batch.length} available`)}</span>` : ""}</div>
    <ul class="asset-export-files">${files.map(item => {
      const id = h(item.id);
      const dimensions = item.width && item.height ? `${item.width} × ${item.height} px · ` : "";
      return `<li><div class="asset-export-file">${icon(item.kind === "video" ? "video" : "image")}
        <span><strong>${h(item.name)}</strong><small>${dimensions}${bytesLabel(item.size)}</small></span></div>
        ${action("share-file", share, "arrow", "secondary",
          `id="asset-export-share-${id}" data-id="${id}" aria-label="${h(`${share} · ${item.name}`)}"`)}
      </li>`;
    }).join("")}</ul>
  </section>`;
}
