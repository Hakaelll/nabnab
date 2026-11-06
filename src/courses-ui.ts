import { escapeHtml as h, localMediaUrl, type Copy, type LibraryItem, type Job } from "./contracts";
import { action, icon, option } from "./views-common";
import "./courses.css";

export interface CourseAsset { id: string; title: string; kind: string; status: "local" | "downloadable" | "drm" | "unavailable"; restriction?: string | null; libraryId: string | null; urlRef: string | null; jobIds: string[] }
export interface CourseNote { id: string; text: string; time: number | null; bookmarkId: string | null }
export interface CourseLesson { id: string; title: string; learned: boolean; assets: CourseAsset[]; notes: CourseNote[] }
export interface CourseSection { id: string; title: string; lessons: CourseLesson[] }
export interface Course { id: string; title: string; provider: string; url: string | null; slug?: string | null; sections: CourseSection[]; createdAt: number }
interface CourseHooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  getSnapshot(): { library: LibraryItem[]; jobs: Job[] };
  refresh(): Promise<void>;
  render(): void;
  notify(text: string, kind?: "info" | "success" | "error"): void;
  showError(error: unknown): void;
  openLibrary(id: string): void;
  t: Copy;
}

export function createCoursesController(hooks: CourseHooks) {
  const { t } = hooks;
  let courses: Course[] = [];
  let courseId: string | null = null;
  let lessonId: string | null = null;
  let input = "";
  let provider: "udemy" | "hotmart" | "kiwify" = "udemy";
  let productId = "";
  let slug = "";
  let limit = 100;
  let busy = false;
  let query = "";
  let noteText = "";
  let noteTime = "";
  const positions = new Map<string, number>();
  const selected = new Set<string>();
  const current = () => courses.find(course => course.id === courseId);
  const currentLesson = () => current()?.sections.flatMap(section => section.lessons).find(lesson => lesson.id === lessonId);
  const local = (asset: CourseAsset) => asset.libraryId ? hooks.getSnapshot().library.find(item => item.id === asset.libraryId && !item.missing) : undefined;
  const active = (asset: CourseAsset) => asset.jobIds.some(id => hooks.getSnapshot().jobs.some(job => job.id === id && ["pending", "paused", "analyzing", "ready", "downloading", "processing", "verifying"].includes(job.state)));
  const available = (asset: CourseAsset) => asset.status === "downloadable" && !local(asset) && !active(asset);
  const status = (asset: CourseAsset) => local(asset) ? t("Guardado", "Saved") : active(asset) ? t("En cola", "Queued") : asset.status === "drm" ? "DRM" : asset.status === "downloadable" ? t("Descarga autorizada disponible", "Authorized download available") : asset.status === "local" ? t("Archivo ausente", "Missing file") : t("Sin descarga autorizada", "No authorized download");

  async function reload(): Promise<void> { courses = await hooks.command<Course[]>("list_courses"); }
  async function perform(work: () => Promise<void>): Promise<void> {
    if (busy) return;
    await savePlayback();
    busy = true; hooks.render();
    try { await work(); await hooks.refresh(); await reload(); }
    catch (error) { hooks.showError(error); }
    finally { busy = false; hooks.render(); }
  }
  async function open(url = "", initialProvider?:string): Promise<void> { if(initialProvider==="udemy"||initialProvider==="hotmart"||initialProvider==="kiwify")provider=initialProvider; input = url; if (url.includes("hotmart.com")) provider = "hotmart"; else if (url.includes("kiwify.com")) provider = "kiwify"; await reload(); hooks.render(); }

  function render(): string {
    const oldPlayer = document.getElementById("course-player");
    if (oldPlayer instanceof HTMLMediaElement && oldPlayer.dataset.coursePlayer && Number.isFinite(oldPlayer.currentTime)) positions.set(oldPlayer.dataset.coursePlayer, oldPlayer.currentTime);
    const course = current();
    if (!course) return `<div class="page-heading"><div><h1>${t("Cursos", "Courses")}</h1><p>${t("Descarga materiales autorizados y estudia tus archivos locales.", "Download authorized materials and study your local files.")}</p></div>${action("course-import", t("Importar carpeta", "Import folder"), "folder", "primary", busy ? "disabled" : "")}</div>
      <form id="course-open-online" class="settings-form" aria-busy="${busy}"><h2>${t("Abrir un curso con acceso autorizado", "Open an authorized course")}</h2><label>${t("Plataforma", "Platform")}<select id="course-provider">${option("udemy", "Udemy", provider)}${option("hotmart", "Hotmart", provider)}${option("kiwify", "Kiwify", provider)}</select></label><label>${t("Enlace del curso", "Course URL")}<input id="course-url" type="url" required value="${h(input)}" placeholder="https://…" autocomplete="off"></label>${provider !== "udemy" ? `<div class="form-grid"><label>${t("ID numérico del curso o producto", "Numeric course or product ID")}<input id="course-product-id" inputmode="numeric" required value="${h(productId)}"></label>${provider === "hotmart" ? `<label>${t("Identificador del club, slug", "Club identifier, slug")}<input id="course-slug" required value="${h(slug)}"></label>` : ""}</div>` : ""}<div class="form-grid"><label>${t("Máximo de lecciones a explorar", "Lesson exploration limit")}<input id="course-limit" type="number" min="1" max="1000" value="${limit}"></label><div class="button-group"><button type="submit" class="button secondary" ${busy ? "disabled" : ""}>${busy ? t("Explorando…", "Exploring…") : t("Explorar con mi sesión", "Explore with my session")}</button></div></div><p class="field-note">${provider === "udemy" ? t("Importa tu sesión en Ajustes.", "Import your session in Settings.") : t("Conecta un token de alumno de esta plataforma. El ID y el slug se encuentran en el enlace o los datos del curso.", "Connect a student token for this platform. The ID and slug are in the course link or course data.")} ${t("Solo se ofrecen archivos con una ruta de descarga autorizada. Reproducir una lección no implica permiso para guardarla.", "Only materials with an authorized download route are offered. Playback access does not imply permission to save a lesson.")}</p></form>
      ${provider !== "udemy" ? `<form id="course-token" class="settings-form"><h2>${t("Conexión de alumno", "Student connection")} · ${h(provider)}</h2><label>${t("Token de acceso autorizado", "Authorized access token")}<input id="course-token-secret" type="password" required autocomplete="off"></label><p class="field-note">${t("Se protege localmente. No se publica ni se transfiere a otra plataforma. La conexión no descarga contenido por sí sola.", "Stored protected locally. It is not published or sent to another platform. Connecting does not download content.")}</p><div class="button-group"><button type="submit" class="button secondary" ${busy ? "disabled" : ""}>${t("Guardar conexión", "Save connection")}</button>${action("course-token-remove", t("Eliminar conexión", "Remove connection"), undefined, "quiet", busy ? "disabled" : "")}</div></form>` : ""}
      ${courses.length ? `<div class="source-list">${courses.map(course => `<article class="source-row"><button class="media-open" data-action="course-open" data-id="${h(course.id)}"><span class="media-info"><strong>${h(course.title)}</strong><span>${h(course.provider === "local" ? t("Carpeta local", "Local folder") : course.provider)} · ${course.sections.flatMap(section => section.lessons).length} ${t("lecciones", "lessons")}</span></span>${icon("arrow")}</button></article>`).join("")}</div>` : `<div class="empty-state">${icon("library")}<h2>${t("Tu primer curso", "Your first course")}</h2><p>${t("Importa una carpeta con clases legítimamente guardadas o abre un curso al que tengas acceso.", "Import a folder of legitimately saved lessons or open a course you can access.")}</p></div>`}`;
    const lesson = currentLesson();
    const lessons = course.sections.flatMap(section => section.lessons);
    const playerAsset = lesson?.assets.find(asset => { const item = local(asset); return item?.kind === "video" || item?.kind === "audio"; });
    const item = playerAsset ? local(playerAsset) : undefined;
    const queueable = course.sections.flatMap(section => section.lessons).flatMap(lesson => lesson.assets).filter(asset => selected.has(asset.id) && available(asset));
    return `<div class="page-heading"><div>${action("course-back", t("Todos los cursos", "All courses"), undefined, "quiet")}<h1>${h(course.title)}</h1><p>${h(course.provider === "local" ? t("Archivos locales", "Local files") : course.provider)} · ${lessons.length} ${t("lecciones", "lessons")}</p></div>${course.url ? action("course-refresh", t("Actualizar materiales", "Refresh materials"), "retry", "quiet", busy ? "disabled" : "") : ""}</div>
      <div class="selection-toolbar"><span>${queueable.length} ${t("materiales seleccionados", "materials selected")}</span>${action("course-queue", t("Añadir a la cola", "Add to queue"), "add", "primary", busy || !queueable.length ? "disabled" : "")}${action("course-clear", t("Quitar selección", "Clear selection"), undefined, "quiet")}</div>
      <div class="course-layout"><nav aria-label="${t("Lecciones del curso", "Course lessons")}" class="course-tree"><label class="search-control">${icon("search")}<input id="course-search" type="search" value="${h(query)}" placeholder="${t("Buscar lección", "Find lesson")}" aria-label="${t("Buscar lección", "Find lesson")}"></label>${course.sections.map(section => `<details ${section.lessons.some(lesson => lesson.id === lessonId) || !lessonId ? "open" : ""}><summary>${h(section.title)}</summary>${action("course-select-section", t("Seleccionar materiales disponibles", "Select available materials"), undefined, "quiet", `data-id="${h(section.id)}"`)}<ul>${section.lessons.filter(lesson => lesson.title.toLocaleLowerCase().includes(query.toLocaleLowerCase())).map(lesson => `<li><button type="button" class="course-lesson" data-action="course-lesson" data-id="${h(lesson.id)}" ${lesson.id === lessonId ? 'aria-current="true"' : ""}><span>${h(lesson.title)}</span>${lesson.learned ? `<small>${t("Aprendida", "Learned")}</small>` : ""}</button></li>`).join("")}</ul></details>`).join("")}</nav><section class="course-study">${lesson ? `<div class="page-heading"><h2>${h(lesson.title)}</h2>${action("course-learned", lesson.learned ? t("Marcar pendiente de aprender", "Mark as not learned") : t("Marcar aprendida", "Mark as learned"), "check", "quiet", `aria-pressed="${lesson.learned}" ${busy ? "disabled" : ""}`)}</div>
      ${item ? `<div class="media-player"><${item.kind === "audio" ? "audio" : "video"} id="course-player" controls preload="metadata" data-course-player="${h(item.id)}" src="${h(localMediaUrl(item.id))}"></${item.kind === "audio" ? "audio" : "video"}></div>` : ""}
      <h3>${t("Materiales", "Materials")}</h3><div class="source-items">${lesson.assets.map(asset => { const item = local(asset); return `<article class="source-item"><label><input type="checkbox" data-course-asset="${h(asset.id)}" ${selected.has(asset.id) ? "checked" : ""} ${available(asset) ? "" : "disabled"} aria-label="${h(t("Seleccionar ", "Select ") + asset.title)}"></label><div class="source-item-main"><strong>${h(asset.title)}</strong><span>${h(asset.kind)} · ${h(status(asset))}</span>${asset.restriction ? `<small>${h(asset.restriction)}</small>` : ""}</div>${item ? action("course-open-file", t("Ver en biblioteca", "View in library"), undefined, "quiet", `data-id="${h(item.id)}"`) : ""}</article>`; }).join("")}</div>
      <h3>${t("Notas de esta lección", "Lesson notes")}</h3><form id="course-note" class="settings-form"><label>${t("Nota", "Note")}<textarea id="course-note-text" rows="3" maxlength="4000" required>${h(noteText)}</textarea></label><div class="form-grid"><label>${t("Tiempo en segundos, opcional", "Time in seconds, optional")}<input id="course-note-time" type="number" min="0" step="0.1" value="${h(noteTime)}"></label><div class="button-group">${item ? action("course-note-now", t("Usar tiempo actual", "Use current time"), undefined, "quiet") : ""}<button class="button secondary" type="submit" ${busy ? "disabled" : ""}>${t("Guardar nota", "Save note")}</button></div></div></form>
      ${lesson.notes.map(note => `<article class="course-note"><p>${h(note.text)}</p><div class="button-group">${note.time === null ? "" : action("course-note-seek", `${note.time.toFixed(1)} s`, "play", "quiet", `data-time="${note.time}"`)}${action("course-note-remove", t("Eliminar nota", "Remove note"), "close", "quiet", `data-id="${h(note.id)}" ${busy ? "disabled" : ""}`)}</div></article>`).join("")}` : `<div class="empty-state"><h2>${t("Elige una lección", "Choose a lesson")}</h2><p>${t("Guardado y aprendido son estados independientes.", "Saved and learned are separate states.")}</p></div>`}</section></div>`;
  }

  function handleInput(element: HTMLElement): boolean {
    if (element instanceof HTMLInputElement && element.dataset.courseAsset) { if (element.checked) selected.add(element.dataset.courseAsset); else selected.delete(element.dataset.courseAsset); hooks.render(); return true; }
    if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement || element instanceof HTMLSelectElement)) return false;
    switch (element.id) {
      case "course-provider": if (element.value === "udemy" || element.value === "hotmart" || element.value === "kiwify") provider = element.value; hooks.render(); return true;
      case "course-product-id": productId = element.value; return true;
      case "course-slug": slug = element.value; return true;
      case "course-url": input = element.value; return true;
      case "course-limit": limit = Number(element.value); return true;
      case "course-search": query = element.value; hooks.render(); return true;
      case "course-note-text": noteText = element.value; return true;
      case "course-note-time": noteTime = element.value; return true;
      default: return false;
    }
  }
  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    if (form.id === "course-open-online") { await perform(async () => { const course = provider === "udemy" ? await hooks.command<Course>("inspect_course", { url: input, limit }) : await hooks.command<Course>("inspect_member_course", { provider, url: input, productId, slug, limit }); courseId = course.id; lessonId = course.sections[0]?.lessons[0]?.id ?? null; }); return true; }
    if (form.id === "course-token") { const field = document.getElementById("course-token-secret"); if (field instanceof HTMLInputElement) { const token = field.value; field.value = ""; await perform(async () => { await hooks.command("save_course_token", { provider, token, remove: false }); hooks.notify(t("Conexión guardada. Explora un curso para comprobar el acceso.", "Connection saved. Explore a course to check access."), "success"); }); } return true; }
    if (form.id === "course-note" && courseId && lessonId) { await perform(async () => { await hooks.command("save_course_note", { courseId, lessonId, text: noteText, time: noteTime.trim() ? Number(noteTime) : null, noteId: null, remove: false }); noteText = ""; noteTime = ""; }); return true; }
    return false;
  }
  async function handleClick(name: string, element: HTMLElement): Promise<boolean> {
    if (!name.startsWith("course-")) return false;
    const course = current(), lesson = currentLesson();
    switch (name) {
      case "course-token-remove": await perform(async () => { await hooks.command("save_course_token", { provider, token: "", remove: true }); hooks.notify(t("Conexión eliminada. Los archivos se conservan.", "Connection removed. Local files are kept."), "success"); }); break;
      case "course-import": await perform(async () => { const course = await hooks.command<Course>("import_local_course"); courseId = course.id; lessonId = course.sections[0]?.lessons[0]?.id ?? null; hooks.notify(t("Curso local importado.", "Local course imported."), "success"); }); break;
      case "course-open": await savePlayback(); courseId = element.dataset.id ?? null; lessonId = current()?.sections[0]?.lessons[0]?.id ?? null; break;
      case "course-back": await savePlayback(); courseId = null; lessonId = null; selected.clear(); break;
      case "course-lesson": await savePlayback(); lessonId = element.dataset.id ?? null; noteText = ""; noteTime = ""; break;
      case "course-clear": selected.clear(); break;
      case "course-select-section": if (course) for (const asset of course.sections.find(section => section.id === element.dataset.id)?.lessons.flatMap(lesson => lesson.assets) ?? []) if (available(asset)) selected.add(asset.id); break;
      case "course-queue": if (course) await perform(async () => { await hooks.command("queue_course_assets", { courseId: course.id, assetIds: [...selected], clientRequest: crypto.randomUUID() }); selected.clear(); hooks.notify(t("Materiales añadidos a la cola.", "Materials added to queue."), "success"); }); break;
      case "course-refresh": if (course?.url) await perform(async () => { if (course.provider === "udemy") await hooks.command("inspect_course", { url: course.url, limit }); else await hooks.command("inspect_member_course", { provider: course.provider, url: course.url, productId: course.id.replace(/^(hotmart|kiwify)-/, ""), slug: course.slug ?? "", limit }); }); break;
      case "course-learned": if (course && lesson) await perform(async () => { await hooks.command("course_learned", { courseId: course.id, lessonId: lesson.id, learned: !lesson.learned }); }); break;
      case "course-open-file": if (element.dataset.id) hooks.openLibrary(element.dataset.id); return true;
      case "course-note-now": { const player = document.getElementById("course-player"); if (player instanceof HTMLMediaElement) noteTime = player.currentTime.toFixed(1); break; }
      case "course-note-seek": { const player = document.getElementById("course-player"); const time = Number(element.dataset.time); if (player instanceof HTMLMediaElement && Number.isFinite(time)) player.currentTime = time; return true; }
      case "course-note-remove": if (course && lesson) await perform(async () => { await hooks.command("save_course_note", { courseId: course.id, lessonId: lesson.id, text: "", time: null, noteId: element.dataset.id, remove: true }); }); break;
      default: return false;
    }
    hooks.render(); return true;
  }
  async function savePlayback(): Promise<void> { const player = document.getElementById("course-player"); if (player instanceof HTMLMediaElement && player.dataset.coursePlayer && Number.isFinite(player.currentTime)) await hooks.command("library_playback", { id: player.dataset.coursePlayer, position: player.currentTime }); }
  function bind(): void {
    const player = document.getElementById("course-player"); if (!(player instanceof HTMLMediaElement) || !player.dataset.coursePlayer) return;
    const item = hooks.getSnapshot().library.find(item => item.id === player.dataset.coursePlayer);
    const position = positions.get(player.dataset.coursePlayer) ?? item?.details?.playbackPosition ?? 0;
    player.addEventListener("loadedmetadata", () => { if (position > 0 && Number.isFinite(player.duration)) player.currentTime = Math.min(position, player.duration); }, { once: true });
    let last = performance.now();
    player.addEventListener("timeupdate", () => { if (performance.now() - last > 5000) { last = performance.now(); void savePlayback().catch(hooks.showError); } });
    player.addEventListener("pause", () => { void savePlayback().catch(hooks.showError); });
  }
  return { open, render, handleInput, handleSubmit, handleClick, bind, savePlayback };
}
