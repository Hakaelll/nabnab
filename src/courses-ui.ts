import { escapeHtml as h, type Copy, type LibraryItem, type Job } from "./contracts";
import type { createAudioPlayer } from "./audio-player";
import { isPlayable, resumePosition } from "./playback-preferences";
import { bytesLabel, durationLabel } from "./presentation";
import { action, icon, option } from "./views-common";
import "./courses.css";

const browserCoursePlatforms = {
  rocketseat: { name: "Rocketseat", host: "app.rocketseat.com.br", path: /^\/journey\/[^/]+/ },
  coursera: { name: "Coursera", host: "coursera.org", path: /^\/learn\/[^/]+/ },
  domestika: { name: "Domestika", host: "domestika.org", path: /^\/(?:[a-z]{2}\/)?courses\/\d+-[^/]+/ },
  skillshare: { name: "Skillshare", host: "skillshare.com", path: /^\/(?:[a-z]{2}\/)?classes\/[^/]+\/\d+(?:\/|$)/ },
};
type BrowserCourseProvider = keyof typeof browserCoursePlatforms;
type CourseProvider = "udemy" | "hotmart" | "kiwify" | BrowserCourseProvider;
const courseProviders: [CourseProvider, string][] = [
  ["udemy", "Udemy"], ["hotmart", "Hotmart"], ["kiwify", "Kiwify"],
  ["rocketseat", "Rocketseat"], ["coursera", "Coursera"],
  ["domestika", "Domestika"], ["skillshare", "Skillshare"],
];
const isBrowserProvider = (value: string): value is BrowserCourseProvider => Object.hasOwn(browserCoursePlatforms, value);
const courseProviderName = (id: string) => courseProviders.find(([value]) => value === id)?.[1] ?? id;
const skillshareResourceSection = "skillshare-resources";

export function browserCourseProvider(url: URL): BrowserCourseProvider | undefined {
  if (url.protocol !== "https:" || url.port || url.username || url.password) return;
  for (const [id] of courseProviders) {
    if (!isBrowserProvider(id)) continue;
    const { host, path } = browserCoursePlatforms[id];
    if ((url.hostname === host || id !== "rocketseat" && url.hostname === `www.${host}`) && path.test(url.pathname)) return id;
  }
}

export interface CourseAsset { id: string; title: string; kind: string; status: "local" | "downloadable" | "drm" | "unavailable"; restriction?: string | null; libraryId: string | null; urlRef: string | null; jobIds: string[] }
export interface CourseNote { id: string; text: string; time: number | null; bookmarkId: string | null }
export interface CourseLesson { id: string; title: string; learned: boolean; assets: CourseAsset[]; notes: CourseNote[] }
export interface CourseSection { id: string; title: string; lessons: CourseLesson[] }
export interface Course { id: string; title: string; provider: string; url: string | null; slug?: string | null; sections: CourseSection[]; createdAt: number }
interface ReviewFile { id: number; name: string; kind: string; size: number }
interface MaterialReview { id: number; title: string; provider: BrowserCourseProvider; groups: { title: string; files: ReviewFile[] }[] }
interface CourseHooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  getSnapshot(): { library: LibraryItem[]; jobs: Job[] };
  refresh(): Promise<void>;
  render(): void;
  notify(text: string, kind?: "info" | "success" | "error"): void;
  showError(error: unknown): void;
  openLibrary(id: string): void;
  player: Pick<ReturnType<typeof createAudioPlayer>, "play" | "playAt" | "current" | "timeFor">;
  t: Copy;
}

export function createCoursesController(hooks: CourseHooks) {
  const { t, player } = hooks;
  const lessonCount = (count: number) => `${count} ${count === 1 ? t("lección", "lesson") : t("lecciones", "lessons")}`;
  let courses: Course[] = [];
  let courseId: string | null = null;
  let lessonId: string | null = null;
  let input = "";
  let provider: CourseProvider = "udemy";
  let browserTitle = "";
  let resourceLinks = "";
  let resourceLessonId = "";
  let productId = "";
  let slug = "";
  let limit = 100;
  let busy = false;
  let query = "";
  let noteText = "";
  let noteTime = "";
  let review: { plan: MaterialReview; files: Set<number>; trigger: string; lessonTitle?: string } | null = null;
  const selected = new Set<string>();
  const current = () => courses.find(course => course.id === courseId);
  const currentLesson = () => current()?.sections.flatMap(section => section.lessons).find(lesson => lesson.id === lessonId);
  const local = (asset: CourseAsset) => {
    const { library, jobs } = hooks.getSnapshot();
    const indexed = asset.libraryId ? library.find(item => item.id === asset.libraryId && !item.missing) : undefined;
    if (indexed || !asset.jobIds.length) return indexed;
    const outputs = jobs.filter(job => job.state === "completed" && asset.jobIds.includes(job.id)).flatMap(job => job.outputs);
    return library.find(item => !item.missing && outputs.includes(item.path));
  };
  const lessonMedia = (lesson?: CourseLesson) => lesson?.assets.map(local).find(isPlayable);
  const active = (asset: CourseAsset) => asset.jobIds.some(id => hooks.getSnapshot().jobs.some(job => job.id === id && ["pending", "paused", "analyzing", "ready", "downloading", "processing", "verifying"].includes(job.state)));
  const failed = (asset: CourseAsset) => asset.jobIds.some(id => hooks.getSnapshot().jobs.some(job => job.id === id && job.state === "error"));
  const available = (asset: CourseAsset) => asset.status === "downloadable" && !local(asset) && !active(asset);
  function status(asset: CourseAsset) {
    if (local(asset)) return t("Guardado", "Saved");
    if (active(asset)) return t("En cola", "Queued");
    if (asset.status === "drm") return "DRM";
    if (asset.status === "downloadable") return failed(asset) ? t("Descarga fallida", "Download failed") : t("Descarga autorizada disponible", "Authorized download available");
    return asset.status === "local" ? t("Archivo ausente", "Missing file") : t("Sin descarga autorizada", "No authorized download");
  }
  function materialKind(kind: string, localStudy: boolean): string {
    if (!localStudy) return kind;
    if (kind === "video") return t("Lección de vídeo", "Video lesson");
    if (kind === "audio") return t("Lección de audio", "Audio lesson");
    if (kind === "subtitle") return t("Subtítulos", "Subtitles");
    return t("Adjunto", "Attachment");
  }

  async function reload(): Promise<void> { courses = await hooks.command<Course[]>("list_courses"); }
  async function perform(work: () => Promise<void | false>): Promise<void> {
    if (busy) return;
    busy = true; hooks.render();
    try { if (await work() !== false) { await hooks.refresh(); await reload(); } }
    catch (error) { hooks.showError(error); }
    finally { busy = false; hooks.render(); }
  }

  function focusCourseHeading(): void {
    const heading = document.getElementById("course-heading");
    const main = heading?.closest("main");
    if (main) main.scrollTop = 0;
    heading?.focus({ preventScroll: true });
  }

  async function open(url = "", initialProvider?: string): Promise<void> {
    if (review) {
      const reviewId = review.plan.id;
      review = null;
      await hooks.command("discard_course_materials", { reviewId }).catch(hooks.showError);
    }
    const parsed = URL.canParse(url) ? new URL(url) : null;
    const routed = parsed ? browserCourseProvider(parsed) ??
      courseProviders.find(([id]) => parsed.hostname === `${id}.com` || parsed.hostname.endsWith(`.${id}.com`))?.[0] : undefined;
    const next = routed ?? courseProviders.find(([id]) => id === initialProvider)?.[0];
    if (next || url) {
      provider = next ?? "udemy";
      courseId = null;
      lessonId = null;
      browserTitle = "";
      resourceLinks = "";
      resourceLessonId = "";
      selected.clear();
    }
    input = url;
    await reload();
    hooks.render();
    if (next || url) focusCourseHeading();
  }

  function browserGuidance(source: BrowserCourseProvider): string {
    switch (source) {
      case "rocketseat": return t("Abre la ruta de la formación. Importa únicamente archivos que Rocketseat o el autor te haya autorizado a guardar.", "Open the training journey. Import only files Rocketseat or the author has authorized you to save.");
      case "coursera": return t("Abre el curso con tu sesión y usa las opciones de descarga que el autor haya habilitado en cada lección.", "Open the course in your session and use the download options the author enabled for each lesson.");
      case "domestika": return t("Guarda los adjuntos de Recursos adicionales desde la web. El vídeo sin conexión se abre en la app de Domestika.", "Save attachments from Additional Resources on the website. Offline video plays in the Domestika app.");
      case "skillshare": return t("Ve a Proyectos y recursos y usa Descargar recursos. El vídeo sin conexión se abre en la app de Skillshare.", "Go to Projects & Resources and use Download Resources. Offline video plays in the Skillshare app.");
    }
  }

  async function reviewMaterials(source: BrowserCourseProvider, url: string, title: string, folder = false, lesson?: CourseLesson, droppedPaths?: string[]) {
    const trigger = lesson ? "course-add-attachments" : folder ? "course-add-folder" : courseId ? "course-add-materials" : "course-browser-import";
    await perform(async () => {
      const plan = await hooks.command<MaterialReview | null>("preview_course_materials", { provider: source, url, title, folder, lessonId: lesson?.id ?? null, droppedPaths });
      if (plan) review = { plan, files: new Set(), trigger, lessonTitle: lesson?.title };
      return false;
    });
    if (review) focusCourseHeading();
    else document.getElementById(trigger)?.focus({ preventScroll: true });
  }

  function supportsDrop(): boolean {
    const course = current();
    return course ? Boolean(course.url && isBrowserProvider(course.provider)) : isBrowserProvider(provider);
  }

  async function receiveDrop(paths: string[]): Promise<void> {
    if (!paths.length || busy || review) return;
    const course = current();
    if (course?.url && isBrowserProvider(course.provider)) {
      await reviewMaterials(course.provider, course.url, course.title, false, undefined, paths);
    } else if (isBrowserProvider(provider) && document.querySelector<HTMLFormElement>("#course-browser-form")?.reportValidity()) {
      await reviewMaterials(provider, input, browserTitle, false, undefined, paths);
    }
  }

  async function finishReview(commit: boolean) {
    if (!review || busy) return;
    const { plan, files, trigger } = review;
    if (commit && !files.size) return;
    let imported = false;
    await perform(async () => {
      review = null;
      if (!commit) {
        await hooks.command("discard_course_materials", { reviewId: plan.id });
        return false;
      }
      const course = await hooks.command<Course>("import_course_materials", { reviewId: plan.id, ids: [...files] });
      if (courseId !== course.id) {
        lessonId = course.sections[0]?.lessons[0]?.id ?? null;
        selected.clear();
      }
      courseId = course.id;
      imported = true;
      hooks.notify(trigger === "course-add-attachments"
        ? t("Adjuntos añadidos a la lección.", "Attachments added to the lesson.")
        : t("Materiales añadidos al curso.", "Materials added to the course."), "success");
    });
    if (imported && trigger === "course-add-attachments") document.getElementById("course-materials-heading")?.focus();
    else if (imported) focusCourseHeading();
    else document.getElementById(trigger)?.focus({ preventScroll: true });
  }

  function fileCount(count: number) {
    return `${count} ${count === 1 ? t("archivo", "file") : t("archivos", "files")}`;
  }

  function reviewView(plan: MaterialReview, files: Set<number>, lessonTitle?: string) {
    const disabled = busy ? "disabled" : "";
    const total = plan.groups.flatMap(group => group.files).filter(file => files.has(file.id)).reduce((sum, file) => sum + file.size, 0);
    return `<section class="course-review" aria-busy="${busy}">
      <div class="page-heading"><div><h1 id="course-heading" tabindex="-1">${t("Revisar materiales", "Review materials")}</h1>
        <p>${h(plan.title)} · ${h(courseProviderName(plan.provider))}</p></div></div>
      ${lessonTitle === undefined ? "" : `<p class="course-review-note">${t("Adjuntos para", "Attachments for")} <strong>${h(lessonTitle)}</strong></p>`}
      <p class="course-review-note">${t("Marca los archivos que quieras añadir. Se conservarán en su ubicación actual.", "Select the files to add. They will stay in their current locations.")}</p>
      <div class="course-review-groups">${plan.groups.map((group, index) => {
        const count = group.files.filter(file => files.has(file.id)).length;
        const all = count === group.files.length;
        return `<details id="course-review-group-${plan.id}-${index}" ${index === 0 ? "open" : ""}>
          <summary><strong>${h(group.title)}</strong><small>${count} / ${fileCount(group.files.length)}</small></summary>
          <div class="course-review-list">
            ${group.files.length > 1 ? action("course-review-unit", all ? t("Quitar selección del grupo", "Clear group selection") : t("Seleccionar grupo", "Select group"), undefined, "quiet", `id="course-review-unit-${index}" data-index="${index}" ${disabled}`) : ""}
            ${group.files.map(file => `<label class="course-review-file">
              <input id="course-review-file-${file.id}" type="checkbox" data-course-review-file="${file.id}" ${files.has(file.id) ? "checked" : ""} ${disabled}>
              <span>${h(file.name)}<small>${materialKind(file.kind, true)}</small></span><small>${bytesLabel(file.size)}</small>
            </label>`).join("")}
          </div>
        </details>`;
      }).join("")}</div>
      <footer class="course-review-footer">
        <output>${files.size ? `${fileCount(files.size)} · ${bytesLabel(total)}` : t("Elige al menos un archivo", "Select at least one file")}</output>
        <div class="button-group">${action("course-review-cancel", t("Volver", "Back"), undefined, "quiet", disabled)}
          ${action("course-review-confirm", lessonTitle === undefined ? t("Añadir al curso", "Add to course") : t("Añadir a la lección", "Add to lesson"), "add", "primary", busy || !files.size ? "disabled" : "")}</div>
      </footer>
    </section>`;
  }

  function browserForm(source: BrowserCourseProvider, platformField: string, linkField: string): string {
    return `<form id="course-browser-form" class="settings-form" aria-busy="${busy}">
      <h2>${t("Guardar materiales del curso", "Keep course materials")}</h2>
      <div class="course-browser-steps">
        <section>
          <h3>${t("1. Abre el curso", "1. Open the course")}</h3>
          ${platformField}${linkField}
          <p class="field-note">${h(browserGuidance(source))}</p>
          ${action("course-browser-open", t("Abrir curso en el navegador", "Open course in browser"), "arrow", "secondary", busy ? "disabled" : "")}
        </section>
        <section>
          <h3>${t("2. Añade tus archivos", "2. Add your files")}</h3>
          <label>${t("Nombre del curso", "Course name")}<input id="course-browser-title" maxlength="240" required value="${h(browserTitle)}" autocomplete="off"></label>
          <p class="field-note">${t("Elige o arrastra hasta 200 archivos o una carpeta con sus unidades. Podrás revisarlos antes de añadirlos.", "Choose or drop up to 200 files or a folder with its units. You can review them before adding them.")}</p>
          <div class="button-group">
            <button type="submit" id="course-browser-import" class="button primary" ${busy ? "disabled" : ""}>${busy ? t("Preparando…", "Preparing…") : t("Elegir archivos guardados", "Choose saved files")}</button>
            ${action("course-add-folder", t("Elegir carpeta del curso", "Choose course folder"), "folder", "secondary", `id="course-add-folder" ${busy ? "disabled" : ""}`)}
          </div>
          ${source === "skillshare" ? skillshareEntry() : ""}
        </section>
      </div>
    </form>`;
  }

  function skillshareEntry(course?: Course) {
    const sections = course?.sections.filter(section => section.id !== skillshareResourceSection && section.lessons.length) ?? [];
    return `<details id="course-skillshare-resources" class="course-resource-entry">
      <summary>${t("Descargar adjuntos de Skillshare", "Download Skillshare attachments")}</summary>
      <p class="field-note">${t("Copia los enlaces de los archivos que el docente ofrece en Descargar recursos. Pega uno por línea para revisarlos antes de añadirlos a Descargas.", "Copy file links the teacher offers under Download Resources. Paste one per line to review them before adding them to Downloads.")}</p>
      ${sections.length ? `<div class="course-resource-target"><label for="course-resource-lesson">${t("Guardar en", "Save in")}</label>
        <select id="course-resource-lesson" ${busy ? "disabled" : ""}>
          ${option("", t("Recursos generales del curso", "General course resources"), resourceLessonId)}
          ${sections.map(section => `<optgroup label="${h(section.title)}">${section.lessons.map(lesson => option(lesson.id, lesson.title, resourceLessonId)).join("")}</optgroup>`).join("")}
        </select></div>` : ""}
      <label for="course-resource-links">${t("Enlaces a adjuntos", "Attachment links")}</label>
      <textarea id="course-resource-links" rows="3" maxlength="61440" spellcheck="false" ${busy ? "disabled" : ""}>${h(resourceLinks)}</textarea>
      ${action("course-review-resources", t("Revisar enlaces", "Review links"), "document", "secondary", busy ? "disabled" : "")}
    </details>`;
  }

  function lessonPlayback(item: LibraryItem): string {
    const video = item.kind === "video";
    const thumbnail = video || item.details?.audioCover || item.details?.audio?.embeddedCover;
    const position = resumePosition(item.details?.playbackPosition ?? 0, item.duration);
    const timing = [item.duration != null ? durationLabel(item.duration) : "",
      position ? `${t("Guardado en", "Saved at")} ${durationLabel(position)}` : ""].filter(Boolean).join(" · ");
    return `<div class="course-playback">
      <span class="course-playback-art" aria-hidden="true"${thumbnail ? ` data-thumbnail-id="${h(item.id)}"` : ""}>${icon(video ? "video" : "audio")}</span>
      <div class="course-playback-copy"><span>${video ? t("Vídeo de la lección", "Lesson video") : t("Audio de la lección", "Lesson audio")}</span><strong>${h(item.name)}</strong>${timing ? `<small>${h(timing)}</small>` : ""}</div>
      ${action("course-play", t("Reproducir lección", "Play lesson"), "play", "primary")}
    </div>`;
  }

  function render(): string {
    if (review) return reviewView(review.plan, review.files, review.lessonTitle);
    const course = current();
    const platformField = `<label><span id="course-platform-label">${t("Plataforma", "Platform")}</span><select id="course-provider" aria-labelledby="course-platform-label">${courseProviders.map(([id, name]) => option(id, name, provider)).join("")}</select></label>`;
    const linkField = `<label>${t("Enlace del curso", "Course URL")}<input id="course-url" type="url" required value="${h(input)}" placeholder="https://…" autocomplete="off"></label>`;
    if (!course) return `<div class="page-heading"><div><h1 id="course-heading" tabindex="-1">${t("Cursos", "Courses")}</h1><p>${t("Descarga materiales autorizados y estudia tus archivos locales.", "Download authorized materials and study your local files.")}</p></div>${action("course-import", t("Importar curso local", "Import local course"), "folder", isBrowserProvider(provider) ? "secondary" : "primary", busy ? "disabled" : "")}</div>
      ${isBrowserProvider(provider) ? browserForm(provider, platformField, linkField) : `<form id="course-open-online" class="settings-form" aria-busy="${busy}"><h2>${t("Abrir un curso con acceso autorizado", "Open an authorized course")}</h2>${platformField}${linkField}${provider !== "udemy" ? `<div class="form-grid"><label>${t("ID numérico del curso o producto", "Numeric course or product ID")}<input id="course-product-id" inputmode="numeric" required value="${h(productId)}"></label>${provider === "hotmart" ? `<label>${t("Identificador del club, slug", "Club identifier, slug")}<input id="course-slug" required value="${h(slug)}"></label>` : ""}</div>` : ""}<div class="form-grid"><label>${t("Máximo de lecciones a explorar", "Lesson exploration limit")}<input id="course-limit" type="number" min="1" max="1000" value="${limit}"></label><div class="button-group"><button type="submit" class="button secondary" ${busy ? "disabled" : ""}>${busy ? t("Explorando…", "Exploring…") : t("Explorar con mi sesión", "Explore with my session")}</button></div></div><p class="field-note">${provider === "udemy" ? t("Importa tu sesión en Ajustes.", "Import your session in Settings.") : t("Conecta un token de alumno de esta plataforma. El ID y el slug se encuentran en el enlace o los datos del curso.", "Connect a student token for this platform. The ID and slug are in the course link or course data.")} ${t("Solo se ofrecen archivos con una ruta de descarga autorizada. Reproducir una lección no implica permiso para guardarla.", "Only materials with an authorized download route are offered. Playback access does not imply permission to save a lesson.")}</p></form>`}
      ${provider === "hotmart" || provider === "kiwify" ? `<form id="course-token" class="settings-form"><h2>${t("Conexión de alumno", "Student connection")} · ${h(provider)}</h2><label>${t("Token de acceso autorizado", "Authorized access token")}<input id="course-token-secret" type="password" required autocomplete="off"></label><p class="field-note">${t("Se protege localmente. No se publica ni se transfiere a otra plataforma. La conexión no descarga contenido por sí sola.", "Stored protected locally. It is not published or sent to another platform. Connecting does not download content.")}</p><div class="button-group"><button type="submit" class="button secondary" ${busy ? "disabled" : ""}>${t("Guardar conexión", "Save connection")}</button>${action("course-token-remove", t("Eliminar conexión", "Remove connection"), undefined, "quiet", busy ? "disabled" : "")}</div></form>` : ""}
      ${courses.length ? `<div class="source-list">${courses.map(course => `<article class="source-row"><button class="media-open" data-action="course-open" data-id="${h(course.id)}"><span class="media-info"><strong>${h(course.title)}</strong><span>${h(course.provider === "local" ? t("Carpeta local", "Local folder") : courseProviderName(course.provider))} · ${lessonCount(course.sections.flatMap(section => section.lessons).length)}</span></span>${icon("arrow")}</button></article>`).join("")}</div>` : isBrowserProvider(provider) ? "" : `<div class="empty-state">${icon("library")}<h2>${t("Tu primer curso", "Your first course")}</h2><p>${t("Importa una carpeta con clases legítimamente guardadas o abre un curso al que tengas acceso.", "Import a folder of legitimately saved lessons or open a course you can access.")}</p></div>`}`;
    const browserAssisted = isBrowserProvider(course.provider);
    const localStudy = browserAssisted || course.provider === "local";
    const lesson = currentLesson();
    const lessons = course.sections.flatMap(section => section.lessons);
    const resources = course.sections.find(section => section.id === skillshareResourceSection);
    const resourceCount = course.provider === "skillshare"
      ? lessons.flatMap(lesson => lesson.assets).filter(asset => asset.status === "downloadable" && asset.kind === "attachment").length
      : 0;
    const lessonTotal = lessons.length - (resources?.lessons.length ?? 0);
    const courseCounts = [lessonTotal || !resourceCount ? lessonCount(lessonTotal) : "", resourceCount ? `${resourceCount} ${resourceCount === 1 ? t("adjunto", "attachment") : t("adjuntos", "attachments")}` : ""].filter(Boolean).join(" · ");
    const resourceLesson = resources?.lessons.some(candidate => candidate.id === lessonId);
    const showQueue = !localStudy || lessons.some(lesson => lesson.assets.some(asset => asset.status === "downloadable"));
    const item = lessonMedia(lesson);
    const queueable = course.sections.flatMap(section => section.lessons).flatMap(lesson => lesson.assets).filter(asset => selected.has(asset.id) && available(asset));
    return `<div class="page-heading"><div>${action("course-back", t("Todos los cursos", "All courses"), undefined, "quiet")}<h1 id="course-heading" tabindex="-1">${h(course.title)}</h1><p>${h(course.provider === "local" ? t("Archivos locales", "Local files") : courseProviderName(course.provider))} · ${courseCounts}</p></div>${browserAssisted ? `<div class="button-group">${action("course-browser-open", t("Abrir curso en el navegador", "Open course in browser"), "arrow", "quiet", busy ? "disabled" : "")}${action("course-add-folder", t("Añadir carpeta", "Add folder"), "folder", "secondary", `id="course-add-folder" ${busy ? "disabled" : ""}`)}${action("course-add-materials", t("Añadir archivos guardados", "Add saved files"), "add", "primary", `id="course-add-materials" ${busy ? "disabled" : ""}`)}</div>` : course.url ? action("course-refresh", t("Actualizar materiales", "Refresh materials"), "retry", "quiet", busy ? "disabled" : "") : ""}</div>
      ${course.provider === "skillshare" ? skillshareEntry(course) : ""}
      ${!showQueue ? "" : `<div class="selection-toolbar"><span>${queueable.length === 1 ? t("1 material seleccionado", "1 material selected") : `${queueable.length} ${t("materiales seleccionados", "materials selected")}`}</span>${action("course-queue", t("Añadir a la cola", "Add to queue"), "add", "primary", busy || !queueable.length ? "disabled" : "")}${action("course-clear", t("Quitar selección", "Clear selection"), undefined, "quiet")}</div>`}
      <div class="course-layout${browserAssisted ? " course-layout--browser" : ""}"><nav aria-label="${resourceCount ? t("Lecciones y recursos", "Lessons and resources") : t("Lecciones del curso", "Course lessons")}" class="course-tree"><label class="search-control">${icon("search")}<input id="course-search" type="search" value="${h(query)}" placeholder="${t("Buscar lección", "Find lesson")}" aria-label="${t("Buscar lección", "Find lesson")}"></label>${course.sections.map(section => `<details ${section.lessons.some(lesson => lesson.id === lessonId) || !lessonId ? "open" : ""}><summary>${h(section.id === "skillshare-resources" ? t("Recursos de Skillshare", "Skillshare resources") : section.title)}</summary>${localStudy && !section.lessons.some(lesson => lesson.assets.some(available)) ? "" : action("course-select-section", t("Seleccionar materiales disponibles", "Select available materials"), undefined, "quiet", `data-id="${h(section.id)}"`)}<ul>${section.lessons.filter(lesson => lesson.title.toLocaleLowerCase().includes(query.toLocaleLowerCase())).map(lesson => `<li><button type="button" class="course-lesson" data-action="course-lesson" data-id="${h(lesson.id)}" ${lesson.id === lessonId ? 'aria-current="true"' : ""}><span>${h(lesson.title)}</span>${lesson.learned ? `<small>${t("Aprendida", "Learned")}</small>` : ""}</button></li>`).join("")}</ul></details>`).join("")}</nav><section class="course-study">${lesson ? `<div class="page-heading"><h2>${h(lesson.title)}</h2>${action("course-learned", resourceLesson ? (lesson.learned ? t("Marcar pendiente", "Mark as pending") : t("Marcar revisado", "Mark as reviewed")) : (lesson.learned ? t("Marcar pendiente de aprender", "Mark as not learned") : t("Marcar aprendida", "Mark as learned")), "check", "quiet", `aria-pressed="${lesson.learned}" ${busy ? "disabled" : ""}`)}</div>
      ${item ? lessonPlayback(item) : ""}
      <div class="course-materials-heading"><h3 id="course-materials-heading" tabindex="-1">${t("Materiales", "Materials")}</h3>${browserAssisted && !resourceLesson ? action("course-add-attachments", t("Añadir adjuntos", "Add attachments"), "add", "quiet", `id="course-add-attachments" ${busy ? "disabled" : ""}`) : ""}</div><div class="source-items">${lesson.assets.map(asset => { const item = local(asset); return `<article class="source-item">${localStudy && asset.status !== "downloadable" ? "" : `<label><input type="checkbox" data-course-asset="${h(asset.id)}" ${selected.has(asset.id) ? "checked" : ""} ${available(asset) ? "" : "disabled"} aria-label="${h(t("Seleccionar ", "Select ") + asset.title)}"></label>`}<div class="source-item-main"><strong>${h(asset.title)}</strong><span>${h(materialKind(asset.kind, localStudy))} · ${h(status(asset))}</span>${asset.restriction ? `<small>${h(asset.restriction)}</small>` : ""}</div>${item ? action("course-open-file", t("Ver en biblioteca", "View in library"), undefined, "quiet", `data-id="${h(item.id)}"`) : ""}</article>`; }).join("")}</div>
      <h3>${resourceLesson ? t("Notas de este adjunto", "Attachment notes") : t("Notas de esta lección", "Lesson notes")}</h3><form id="course-note" class="settings-form"><label>${t("Nota", "Note")}<textarea id="course-note-text" rows="3" maxlength="4000" required>${h(noteText)}</textarea></label><div class="form-grid"><label>${t("Tiempo en segundos, opcional", "Time in seconds, optional")}<input id="course-note-time" type="number" min="0" step="0.1" value="${h(noteTime)}"></label><div class="button-group">${item ? action("course-note-now", t("Usar tiempo actual", "Use current time"), undefined, "quiet", player.current()?.id === item.id ? "" : "disabled") : ""}<button class="button secondary" type="submit" ${busy ? "disabled" : ""}>${t("Guardar nota", "Save note")}</button></div></div></form>
      ${lesson.notes.map(note => `<article class="course-note"><p>${h(note.text)}</p><div class="button-group">${note.time === null ? "" : item ? action("course-note-seek", `${note.time.toFixed(1)} s`, "play", "quiet", `data-time="${note.time}"`) : `<span class="field-note">${note.time.toFixed(1)} s</span>`}${action("course-note-remove", t("Eliminar nota", "Remove note"), "close", "quiet", `data-id="${h(note.id)}" ${busy ? "disabled" : ""}`)}</div></article>`).join("")}` : `<div class="empty-state"><h2>${t("Elige una lección", "Choose a lesson")}</h2><p>${t("Guardado y aprendido son estados independientes.", "Saved and learned are separate states.")}</p></div>`}</section></div>`;
  }

  function handleInput(element: HTMLElement): boolean {
    if (review && element instanceof HTMLInputElement && element.dataset.courseReviewFile !== undefined) {
      const id = Number(element.dataset.courseReviewFile);
      if (element.checked) review.files.add(id);
      else review.files.delete(id);
      hooks.render();
      return true;
    }
    if (element instanceof HTMLInputElement && element.dataset.courseAsset) { if (element.checked) selected.add(element.dataset.courseAsset); else selected.delete(element.dataset.courseAsset); hooks.render(); return true; }
    if (!(element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement || element instanceof HTMLSelectElement)) return false;
    switch (element.id) {
      case "course-provider": {
        const next = courseProviders.find(([id]) => id === element.value)?.[0];
        if (next && next !== provider) { provider = next; resourceLinks = ""; resourceLessonId = ""; }
        hooks.render(); return true;
      }
      case "course-browser-title": browserTitle = element.value; return true;
      case "course-resource-links": resourceLinks = element.value; return true;
      case "course-resource-lesson": resourceLessonId = element.value; return true;
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
    if (form.id === "course-browser-form") {
      if (isBrowserProvider(provider) && form.reportValidity()) await reviewMaterials(provider, input, browserTitle);
      return true;
    }
    if (form.id === "course-open-online") { await perform(async () => { const course = provider === "udemy" ? await hooks.command<Course>("inspect_course", { url: input, limit }) : await hooks.command<Course>("inspect_member_course", { provider, url: input, productId, slug, limit }); courseId = course.id; lessonId = course.sections[0]?.lessons[0]?.id ?? null; }); return true; }
    if (form.id === "course-token") { const field = document.getElementById("course-token-secret"); if (field instanceof HTMLInputElement) { const token = field.value; field.value = ""; await perform(async () => { await hooks.command("save_course_token", { provider, token, remove: false }); hooks.notify(t("Conexión guardada. Explora un curso para comprobar el acceso.", "Connection saved. Explore a course to check access."), "success"); }); } return true; }
    if (form.id === "course-note" && courseId && lessonId) { await perform(async () => { await hooks.command("save_course_note", { courseId, lessonId, text: noteText, time: noteTime.trim() ? Number(noteTime) : null, noteId: null, remove: false }); noteText = ""; noteTime = ""; }); return true; }
    return false;
  }
  async function handleClick(name: string, element: HTMLElement): Promise<boolean> {
    if (!name.startsWith("course-")) return false;
    const course = current(), lesson = currentLesson();
    switch (name) {
      case "course-review-resources": {
        if ((course?.provider ?? provider) !== "skillshare" || !course && !element.closest("form")?.reportValidity()) return true;
        const links = resourceLinks.split(/\r?\n/).map(line => line.trim()).filter(Boolean);
        if (!links.length || links.length > 30) {
          hooks.notify(t("Pega entre 1 y 30 enlaces a adjuntos.", "Paste between 1 and 30 attachment links."), "error");
          document.getElementById("course-resource-links")?.focus();
          return true;
        }
        let opened = false;
        const target = course ? resourceLessonId || null : null;
        await perform(async () => {
          const next = await hooks.command<Course>("add_skillshare_resources", { url: course?.url ?? input, title: course?.title ?? browserTitle, links, lessonId: target });
          if (courseId !== next.id) selected.clear();
          courseId = next.id;
          lessonId = target ?? next.sections.find(section => section.id === skillshareResourceSection)?.lessons[0]?.id ?? null;
          resourceLinks = "";
          resourceLessonId = "";
          opened = true;
        });
        if (opened) {
          const entry = document.getElementById("course-skillshare-resources");
          if (entry instanceof HTMLDetailsElement) entry.open = false;
          if (target) document.getElementById("course-materials-heading")?.focus();
          else focusCourseHeading();
        }
        return true;
      }
      case "course-browser-open": {
        const source = course?.provider ?? provider;
        const url = course?.url ?? input;
        if (isBrowserProvider(source)) {
          try { await hooks.command("open_course_site", { provider: source, url }); }
          catch (error) { hooks.showError(error); }
        }
        return true;
      }
      case "course-add-materials":
        if (course?.url && isBrowserProvider(course.provider)) await reviewMaterials(course.provider, course.url, course.title);
        return true;
      case "course-add-attachments":
        if (lesson && course?.url && isBrowserProvider(course.provider)) await reviewMaterials(course.provider, course.url, course.title, false, lesson);
        return true;
      case "course-add-folder":
        if (course?.url && isBrowserProvider(course.provider)) await reviewMaterials(course.provider, course.url, course.title, true);
        else if (isBrowserProvider(provider) && element.closest("form")?.reportValidity()) await reviewMaterials(provider, input, browserTitle, true);
        return true;
      case "course-review-unit": {
        if (!review || busy) return true;
        const unit = review.plan.groups[Number(element.dataset.index)];
        if (!unit) return true;
        const { files } = review;
        const all = unit.files.every(file => files.has(file.id));
        for (const file of unit.files) {
          if (all) files.delete(file.id);
          else files.add(file.id);
        }
        hooks.render();
        return true;
      }
      case "course-review-cancel":
      case "course-review-confirm":
        await finishReview(name === "course-review-confirm");
        return true;
      case "course-token-remove": await perform(async () => { await hooks.command("save_course_token", { provider, token: "", remove: true }); hooks.notify(t("Conexión eliminada. Los archivos se conservan.", "Connection removed. Local files are kept."), "success"); }); break;
      case "course-import": await perform(async () => { const course = await hooks.command<Course>("import_local_course"); courseId = course.id; lessonId = course.sections[0]?.lessons[0]?.id ?? null; hooks.notify(t("Curso local importado.", "Local course imported."), "success"); }); break;
      case "course-open": courseId = element.dataset.id ?? null; lessonId = current()?.sections[0]?.lessons[0]?.id ?? null; resourceLinks = ""; resourceLessonId = ""; break;
      case "course-back": courseId = null; lessonId = null; resourceLinks = ""; resourceLessonId = ""; selected.clear(); break;
      case "course-lesson": lessonId = element.dataset.id ?? null; noteText = ""; noteTime = ""; break;
      case "course-clear": selected.clear(); break;
      case "course-select-section": if (course) for (const asset of course.sections.find(section => section.id === element.dataset.id)?.lessons.flatMap(lesson => lesson.assets) ?? []) if (available(asset)) selected.add(asset.id); break;
      case "course-queue": if (course) await perform(async () => { await hooks.command("queue_course_assets", { courseId: course.id, assetIds: [...selected], clientRequest: crypto.randomUUID() }); selected.clear(); hooks.notify(t("Materiales añadidos a la cola.", "Materials added to queue."), "success"); }); break;
      case "course-refresh": if (course?.url) await perform(async () => { if (course.provider === "udemy") await hooks.command("inspect_course", { url: course.url, limit }); else await hooks.command("inspect_member_course", { provider: course.provider, url: course.url, productId: course.id.replace(/^(hotmart|kiwify)-/, ""), slug: course.slug ?? "", limit }); }); break;
      case "course-learned": if (course && lesson) await perform(async () => { await hooks.command("course_learned", { courseId: course.id, lessonId: lesson.id, learned: !lesson.learned }); }); break;
      case "course-open-file": if (element.dataset.id) hooks.openLibrary(element.dataset.id); return true;
      case "course-play": {
        const item = lessonMedia(lesson);
        if (item) player.play([item.id], 0, true);
        return true;
      }
      case "course-note-now": {
        const item = lessonMedia(lesson);
        const time = item ? player.timeFor(item.id) : undefined;
        if (time !== undefined) noteTime = time.toFixed(1);
        break;
      }
      case "course-note-seek": {
        const item = lessonMedia(lesson);
        const time = Number(element.dataset.time);
        if (item && Number.isFinite(time)) player.playAt(item.id, time, true);
        return true;
      }
      case "course-note-remove": if (course && lesson) await perform(async () => { await hooks.command("save_course_note", { courseId: course.id, lessonId: lesson.id, text: "", time: null, noteId: element.dataset.id, remove: true }); }); break;
      default: return false;
    }
    hooks.render(); return true;
  }
  function observeJob(id: string) {
    if (current()?.sections.some(section => section.lessons.some(lesson => lesson.assets.some(asset => asset.jobIds.includes(id))))) hooks.render();
  }
  return { open, render, handleInput, handleSubmit, handleClick, observeJob, supportsDrop, receiveDrop };
}
