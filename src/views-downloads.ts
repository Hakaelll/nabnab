import { bytesLabel } from "./presentation";
import { activeStates, cancellableStates, escapeHtml as h, type Copy, type DownloadView, type Job, type Snapshot } from "./contracts";
import { action, icon, stateLabel } from "./views-common";
import { toolName } from "./views-tools";
import {priorityLabel,scheduleLabel} from './scheduling-types';

function jobActions(job: Job, t: Copy): string {
  const id = `data-id="${h(job.id)}"`;
  return `${job.state === "pending" ? action("pause-job", t("Pausar pendiente", "Pause pending"), "pause", "icon-button", id + ` title="${t("Pausar pendiente", "Pause pending")}"`) : ""}
    ${cancellableStates.includes(job.state) ? action("cancel-job", t("Cancelar", "Cancel"), "close", "icon-button", id + ` title="${t("Cancelar", "Cancel")}"`) : ["error", "cancelled", "paused"].includes(job.state) ? action("retry-job", job.state === "paused" ? t("Reanudar", "Resume") : t("Reintentar", "Retry"), "retry", "icon-button", id + ` title="${t("Reintentar", "Retry")}"`) : ""}`;
}
export function filteredJobs(snapshot: Snapshot, view: DownloadView): Job[] {
  const query = view.query.toLocaleLowerCase();
  return snapshot.jobs.filter(job => {
    const match = view.filter === "all" || (view.filter === "activity" && activeStates.includes(job.state)) || (view.filter === "pending" && ["pending", "paused"].includes(job.state)) || (view.filter === "completed" && job.state === "completed") || (view.filter === "error" && ["error", "cancelled"].includes(job.state));
    return match && `${job.title} ${job.provider}`.toLocaleLowerCase().includes(query);
  }).sort((a,b) => {
    const category=(job:Job)=>activeStates.includes(job.state)?0:job.state==='pending'?1:2;
    const difference=category(a)-category(b);if(difference)return difference;
    if(a.state==='pending'&&b.state==='pending'){
      const rank=(job:Job)=>(job.priority==='high'?4:job.priority==='low'?0:2)+Math.floor(Math.max(0,Date.now()-job.createdAt)/(15*60_000));
      return Number(Boolean(b.runNext))-Number(Boolean(a.runNext))||rank(b)-rank(a)||(a.manualOrder??0)-(b.manualOrder??0)||a.createdAt-b.createdAt;
    }
    return b.createdAt-a.createdAt;
  });
}
export function downloadsView(snapshot: Snapshot, view: DownloadView, t: Copy): string {
  const jobs = filteredJobs(snapshot, view);
  const groups=new Map<string,Job[]>();for(const job of jobs){const key=job.groupId??job.id;const group=groups.get(key)??[];group.push(job);groups.set(key,group);}
  const ordered=[...groups.values()].flat();
  const filters: [DownloadView["filter"],string][] = [["all",t("Todas", "All")],["activity",t("Actividad", "Activity")],["pending",t("Pendientes", "Pending")],["completed",t("Completadas", "Completed")],["error",t("Errores", "Errors")]];
  return `<div class="page-heading"><div><h1>${t("Descargas", "Downloads")}</h1><p>${t("Descargas y operaciones locales en una misma cola.", "Downloads and local operations in one queue.")}</p></div>${action("go-home", t("Añadir enlace", "Add link"), "add", "secondary")}</div>
    <div class="view-toolbar"><div class="filter-tabs" role="group" aria-label="${t("Estado de los trabajos", "Job status")}">${filters.map(([filter,label]) => `<button data-action="job-filter" data-filter="${filter}" aria-pressed="${view.filter === filter}">${label}${filter === "all" ? `<span>${snapshot.jobs.length}</span>` : ""}</button>`).join("")}</div><label class="search-control">${icon("search")}<input id="job-search" type="search" placeholder="${t("Buscar trabajos", "Search jobs")}" aria-label="${t("Buscar trabajos", "Search jobs")}" value="${h(view.query)}"></label></div>
    ${view.selected.size ? `<div class="selection-toolbar"><span>${view.selected.size} ${t("seleccionados", "selected")}</span>${action("cancel-selected", t("Cancelar compatibles", "Cancel eligible"), "close", "quiet")}${action("retry-selected", t("Reintentar errores", "Retry errors"), "retry", "quiet")}${action("clear-job-selection",t("Quitar selección", "Clear selection"),undefined,"quiet")}</div>` : ""}
    ${jobs.length ? `<ul class="job-list" aria-label="${t("Cola de trabajos", "Job queue")}">${ordered.slice(0,200).map((job,index) => {
      const transferring = job.state === "downloading";
      const percent = transferring ? job.progress?.percent : null;
      const selected = view.detailId === job.id;
      const size = transferring ? job.progress?.total : job.state === "completed" ? null : job.progress?.total;
      const members=job.groupId?snapshot.jobs.filter(item=>item.groupId===job.groupId):[];
      const finished=members.filter(item=>item.state==='completed').length;const errors=members.filter(item=>item.state==='error'||item.state==='cancelled').length;
      const source=snapshot.sources?.find(source=>source.id===job.sourceId);
      const heading=job.groupId&&(index===0||ordered[index-1].groupId!==job.groupId)?`<li class="job-group-summary">${h(source?.name??t('Lote de operaciones','Operations batch'))} · ${finished}/${members.length} ${t('salidas terminadas','outputs complete')}${finished&&errors?` · ${t('Éxito parcial. Puedes reintentar las salidas fallidas.','Partial success. Failed outputs can be retried.')}`:errors?` · ${errors} ${t('con incidencias','with issues')}`:''}</li>`:'';
      return `${heading}<li class="job-row ${selected ? "is-selected" : ""}" data-state="${job.state}"><input type="checkbox" data-select-job="${h(job.id)}" aria-label="${h(t("Seleccionar ", "Select ") + job.title)}" ${view.selected.has(job.id) ? "checked" : ""}>
        <div class="row-thumbnail">${job.thumbnail ? `<img src="${h(job.thumbnail)}" alt="" loading="lazy" referrerpolicy="no-referrer">` : icon(job.source.kind === "transform" ? "tools" : "video")}</div>
        <button class="row-content" data-action="inspect-job" data-id="${h(job.id)}" aria-pressed="${selected}"><strong>${h(job.title)}</strong><span>${h(job.provider)} · ${job.source.kind === "transform" ? t("Archivo local", "Local file") : h(new URL(job.source.url).hostname)}${job.priority&&job.priority!=='normal'?` · ${priorityLabel(job.priority,t)}`:''}${job.schedule&&job.schedule.kind!=='immediate'?` · ${h(scheduleLabel(job.schedule,t))}`:''}${job.options?.values.subtitleMode==='only'?` · ${t('Solo subtítulos','Subtitles only')}`:''}</span></button>
        <div class="job-state"><span class="state-label state-${job.state}">${job.state === "completed" ? icon("check") : job.state === "error" ? icon("error") : ""}${stateLabel(job.state,t)}</span>${transferring ? `<div class="progress-rail ${percent === null ? "indeterminate" : ""}" role="progressbar" aria-label="${t("Descarga", "Download")}" aria-valuemin="0" aria-valuemax="100" ${percent !== null && percent !== undefined ? `aria-valuenow="${percent.toFixed(1)}"` : ""}><span style="width:${percent ?? 35}%"></span></div>` : ""}</div>
        <div class="job-metrics">${percent !== null && percent !== undefined ? `<strong>${percent.toFixed(1)}%</strong>` : ""}<small>${transferring && job.progress?.speed ? bytesLabel(job.progress.speed) + "/s" : size ? bytesLabel(size) : job.state === "completed" ? `${job.outputs.length} ${t("archivo(s)", "file(s)")}` : ""}</small></div><div class="row-actions">${jobActions(job,t)}</div></li>`;
    }).join("")}</ul>${jobs.length>200 ? `<p class="field-note">${t("Se muestran los 200 trabajos más recientes. Usa la búsqueda para acotar.", "Showing the 200 most recent jobs. Use search to narrow the list.")}</p>` : ""}` : `<div class="empty-state">${icon(view.filter === "error" ? "check" : "downloads")}<h2>${view.query ? t("No hay coincidencias", "No matches") : view.filter === "error" ? t("Sin errores en esta vista", "No errors in this view") : t("Tu cola está despejada", "Your queue is clear")}</h2><p>${view.query ? t("Prueba con otro título u origen.", "Try another title or source.") : t("Pega un enlace en Inicio o envía un archivo desde Herramientas.", "Paste a link in Home or send a file from Tools.")}</p></div>`}`;
}

export function jobInspector(job: Job, snapshot: Snapshot, t: Copy): string {
  const outputs = snapshot.library.filter(item=>job.outputs.includes(item.path));
  const progress=job.progress;
  return `<div class="inspector-heading"><span class="eyebrow">${t("Detalles del trabajo", "Job details")}</span>${action("close-inspector",t("Cerrar detalles", "Close details"),"close","icon-button")}</div><h2>${h(job.title)}</h2><p class="state-label state-${job.state}">${stateLabel(job.state,t)}</p><dl class="metadata-list"><div><dt>${t("Origen", "Source")}</dt><dd>${h(job.provider)}</dd></div><div><dt>${t("Perfil", "Profile")}</dt><dd>${h(job.source.kind==='download'?job.source.format:toolName(job.source.options.kind,t))}</dd></div>${job.source.kind==='download'&&job.source.subtitleLanguage?`<div><dt>${t("Subtítulos", "Subtitles")}</dt><dd>${h(job.source.subtitleLanguage)}</dd></div>`:""}<div><dt>${t("Destino", "Destination")}</dt><dd class="path">${h(job.folder)}</dd></div><div><dt>${t("Creado", "Created")}</dt><dd>${h(new Date(job.createdAt).toLocaleString(snapshot.settings.language))}</dd></div><div><dt>${t("Intento", "Attempt")}</dt><dd>${job.attempt}</dd></div>${progress?.downloaded ? `<div><dt>${t("Transferido", "Transferred")}</dt><dd>${bytesLabel(progress.downloaded)}${progress.total?' / '+bytesLabel(progress.total):''}</dd></div>`:""}</dl>
    ${job.error?`<div class="notice error">${icon("error")}<div><strong>${h(job.error.message)}</strong>${job.error.retryAfter?`<p>${t("Espera al menos", "Wait at least")} ${job.error.retryAfter} s.</p>`:""}</div></div><details class="diagnostic"><summary>${t("Diagnóstico técnico", "Technical diagnostic")}</summary><pre>${h(job.error.technical)}</pre>${action("copy-job-error",t("Copiar diagnóstico", "Copy diagnostic"),"copy","quiet",`data-id="${h(job.id)}"`)}</details>`:""}
    ${outputs.length?`<h3>${t("Resultados", "Outputs")}</h3><ul class="output-files">${outputs.map(item=>`<li><span>${h(item.name)}</span>${action("inspect-media",t("Ver archivo", "View file"),"arrow","icon-button",`data-id="${h(item.id)}"`)}</li>`).join("")}</ul>`:job.outputs.length?`<h3>${t("Resultados", "Outputs")}</h3>${job.outputs.map(file=>`<p class="path">${h(file)}</p>`).join("")}`:""}
    <div class="inspector-actions">${jobActions(job,t)}</div>`;
}
