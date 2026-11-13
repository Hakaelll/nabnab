import { escapeHtml as h, type Copy, type LibraryItem, type Job } from "./contracts";
import type { createAudioPlayer } from "./audio-player";
import { isPlayable, resumePosition } from "./playback-preferences";
import { bytesLabel, durationLabel, searchKey } from "./presentation";
import { action, icon, option } from "./views-common";
import "./courses.css";

const browserCoursePlatforms = {
  rocketseat: { name: "Rocketseat", host: "app.rocketseat.com.br", path: /^\/(?:journey|classroom)\/[a-zA-Z0-9_-]{1,240}(?:\/|$)/ },
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
const hasManualOutline = (provider: string): boolean => provider === "rocketseat" || provider === "domestika" || provider === "skillshare";
const courseProviderName = (id: string) => courseProviders.find(([value]) => value === id)?.[1] ?? id;
const skillshareResourceSection = "skillshare-resources";
const unitResourceId = (unitId: string) => `unit-resources-${unitId}`;
const isUnitResource = (lesson: CourseLesson) => lesson.id.startsWith("unit-resources-");
const isManualUnit = (section: CourseSection) => section.id.startsWith("outline-section-");
const isManualLesson = (lesson: CourseLesson) => lesson.id.startsWith("outline-lesson-");
const studyLessonCount = (course: Course) => course.sections
  .filter(section => section.id !== skillshareResourceSection)
  .flatMap(section => section.lessons).filter(lesson => !isUnitResource(lesson)).length;

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
export interface Course { id: string; title: string; provider: string; url: string | null; slug?: string | null; sections: CourseSection[]; createdAt: number; outlineStatus?: { pages: number; partial: boolean } }
interface ReviewFile { id: number; name: string; kind: string; size: number }
interface MaterialReview { id: number; title: string; provider: BrowserCourseProvider; groups: { title: string; files: ReviewFile[] }[] }
interface MaterialTarget { kind: "lesson" | "unit" | "recording"; id: string; title: string }
type ResourceCheck = { kind: "supported" | "duplicate"; filename: string } | { kind: "unsupported" };
interface ResourceReview {
  url: string; title: string; lesson: Pick<CourseLesson, "id" | "title"> | null;
  links: string[]; checks: ResourceCheck[]; selected: Set<number>;
}
interface OutlineReview { course: Course; request: { provider: string; url: string; title: string; text: string } }
interface LessonResume { sectionId: string; lesson: CourseLesson; item: LibraryItem; position: number }
interface CourseHooks {
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  getSnapshot(): { library: LibraryItem[]; jobs: Job[] };
  refresh(): Promise<void>;
  render(): void;
  notify(text: string, kind?: "info" | "success" | "error"): void;
  showError(error: unknown): void;
  openLibrary(id: string): void;
  locateFile(id: string): Promise<void>;
  player: Pick<ReturnType<typeof createAudioPlayer>, "play" | "playAt" | "playFrom" | "current" | "timeFor">;
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
  let courseraIds = { organization: "", program: "", courseId: "" };
  let resourceLinks = "";
  let resourceLessonId = "";
  let productId = "";
  let slug = "";
  let limit = 100;
  let busy = false;
  let query = "";
  let expansionBeforeSearch: Map<string, boolean> | null = null;
  let noteText = "";
  let noteTime = "";
  let review: { plan: MaterialReview; files: Set<number>; trigger: string; target?: MaterialTarget } | null = null;
  let resourceReview: ResourceReview | null = null;
  let outlineReview: OutlineReview | null = null;
  let outlineText: string | null = null;
  let titleDraft: { courseId: string; lessonId: string; unit: string; lesson: string; error: string } | null = null;
  const selected = new Set<string>();
  const current = () => courses.find(course => course.id === courseId);
  const currentLesson = () => current()?.sections.flatMap(section => section.lessons).find(lesson => lesson.id === lessonId);
  const indexed = (asset: CourseAsset) => {
    const { library, jobs } = hooks.getSnapshot();
    const linked = asset.libraryId ? library.find(item => item.id === asset.libraryId) : undefined;
    if (linked && !linked.missing || !asset.jobIds.length) return linked;
    const outputs = jobs.filter(job => job.state === "completed" && asset.jobIds.includes(job.id)).flatMap(job => job.outputs);
    const copies = library.filter(item => outputs.includes(item.path));
    return copies.find(item => !item.missing) ?? linked ?? copies[0];
  };
  const local = (asset: CourseAsset) => {
    const item = indexed(asset);
    return item?.missing ? undefined : item;
  };
  function lessonMedia(lesson?: CourseLesson) {
    for (const asset of lesson?.assets ?? []) {
      const item = local(asset);
      if (isPlayable(item)) return item;
    }
  }
  const active = (asset: CourseAsset) => asset.jobIds.some(id => hooks.getSnapshot().jobs.some(job => job.id === id && ["pending", "paused", "analyzing", "ready", "downloading", "processing", "verifying"].includes(job.state)));
  const failed = (asset: CourseAsset) => asset.jobIds.some(id => hooks.getSnapshot().jobs.some(job => job.id === id && job.state === "error"));
  const available = (asset: CourseAsset) => asset.status === "downloadable" && !local(asset) && !active(asset);
  function status(asset: CourseAsset, item: LibraryItem | undefined) {
    if (item && !item.missing) return t("Guardado", "Saved");
    if (active(asset)) return t("En cola", "Queued");
    if (item?.missing) return t("Archivo ausente", "Missing file");
    if (asset.status === "drm") return "DRM";
    if (asset.status === "downloadable") return failed(asset) ? t("Descarga fallida", "Download failed") : t("Descarga autorizada disponible", "Authorized download available");
    if (asset.id.startsWith("coursera-item-")) return t("Consultar en Coursera", "View in Coursera");
    return asset.status === "local" ? t("Archivo ausente", "Missing file") : t("Sin descarga autorizada", "No authorized download");
  }
  function materialKind(kind: string, localStudy: boolean): string {
    if (!localStudy) return kind;
    if (kind === "video") return t("Lección de vídeo", "Video lesson");
    if (kind === "audio") return t("Lección de audio", "Audio lesson");
    if (kind === "subtitle") return t("Subtítulos", "Subtitles");
    if (kind === "reading") return t("Lectura", "Reading");
    if (kind === "quiz") return t("Cuestionario", "Quiz");
    if (kind === "assignment") return t("Actividad", "Assignment");
    if (kind === "course-item") return t("Material del curso", "Course material");
    return t("Adjunto", "Attachment");
  }

  function materialRow(asset: CourseAsset, localStudy: boolean): string {
    const item = indexed(asset);
    const titleId = `course-material-title-${asset.id}`;
    const fileAttributes = item ? `id="course-file-${h(asset.id)}" data-id="${h(item.id)}" aria-describedby="${h(titleId)}"` : "";
    return `<article class="source-item">
      ${localStudy && asset.status !== "downloadable" ? "" : `<label><input type="checkbox" data-course-asset="${h(asset.id)}"
        ${selected.has(asset.id) ? "checked" : ""} ${available(asset) ? "" : "disabled"} aria-label="${h(t("Seleccionar ", "Select ") + asset.title)}"></label>`}
      <div class="source-item-main"><strong id="${h(titleId)}">${h(asset.title)}</strong>
        <span>${h(materialKind(asset.kind, localStudy))} · ${h(status(asset, item))}</span>${asset.restriction ? `<small>${h(asset.restriction)}</small>` : ""}</div>
      ${item?.missing ? action("course-locate-file", t("Localizar archivo", "Locate file"), "folder", "secondary", `${fileAttributes} ${busy ? "disabled" : ""}`)
        : item ? action("course-open-file", t("Ver en biblioteca", "View in library"), undefined, "quiet", fileAttributes) : ""}
    </article>`;
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

  function clearSearch() {
    query = "";
    expansionBeforeSearch = null;
  }

  async function open(url = "", initialProvider?: string): Promise<void> {
    resourceReview = null;
    outlineReview = null;
    titleDraft = null;
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
      clearSearch();
      browserTitle = "";
      outlineText = null;
      courseraIds = { organization: "", program: "", courseId: "" };
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
      case "rocketseat": return t("Pega un enlace de formación o de aula. Abre Rocketseat con tu sesión para guardar materiales autorizados.", "Paste a journey or classroom link. Open Rocketseat in your browser session to save authorized materials.");
      case "coursera": return t("Abre el curso con tu sesión y usa las opciones de descarga que el autor haya habilitado en cada lección.", "Open the course in your session and use the download options the author enabled for each lesson.");
      case "domestika": return t("Guarda los adjuntos de Recursos adicionales desde la web. El vídeo sin conexión se abre en la app de Domestika.", "Save attachments from Additional Resources on the website. Offline video plays in the Domestika app.");
      case "skillshare": return t("Ve a Proyectos y recursos y usa Descargar recursos. El vídeo sin conexión se abre en la app de Skillshare.", "Go to Projects & Resources and use Download Resources. Offline video plays in the Skillshare app.");
    }
  }

  function focusReviewTrigger(trigger: string) {
    const button = document.getElementById(trigger);
    const unit = button?.closest("details");
    if (unit) unit.open = true;
    button?.focus({ preventScroll: true });
  }

  async function reviewMaterials(source: BrowserCourseProvider, url: string, title: string, folder = false, target?: MaterialTarget, droppedPaths?: string[]) {
    const trigger = target?.kind === "unit" ? `course-add-unit-${target.id}` : target?.kind === "recording" ? "course-link-recording" : target ? "course-add-attachments"
      : folder ? "course-add-folder" : courseId ? "course-add-materials" : "course-browser-import";
    await perform(async () => {
      const plan = await hooks.command<MaterialReview | null>("preview_course_materials", { provider: source, url, title, folder,
        lessonId: target && target.kind !== "unit" ? target.id : null, unitId: target?.kind === "unit" ? target.id : null,
        playback: target?.kind === "recording", droppedPaths });
      if (plan) review = { plan, files: new Set(), trigger, target };
      return false;
    });
    if (review) focusCourseHeading();
    else focusReviewTrigger(trigger);
  }

  function supportsDrop(): boolean {
    if (resourceReview) return false;
    const course = current();
    return course ? Boolean(course.url && isBrowserProvider(course.provider)) : isBrowserProvider(provider);
  }

  async function receiveDrop(paths: string[]): Promise<void> {
    if (!paths.length || busy || review || resourceReview || outlineReview) return;
    const course = current();
    if (course?.url && isBrowserProvider(course.provider)) {
      await reviewMaterials(course.provider, course.url, course.title, false, undefined, paths);
    } else if (isBrowserProvider(provider) && document.querySelector<HTMLFormElement>("#course-browser-form")?.reportValidity()) {
      await reviewMaterials(provider, input, browserTitle, false, undefined, paths);
    }
  }

  async function finishReview(commit: boolean) {
    if (!review || busy) return;
    const { plan, files, trigger, target } = review;
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
      if (target?.kind === "unit") {
        lessonId = unitResourceId(target.id);
        noteText = ""; noteTime = ""; query = "";
      }
      courseId = course.id;
      imported = true;
      hooks.notify(target?.kind === "unit" ? t("Recursos añadidos a la unidad.", "Resources added to the unit.")
        : target?.kind === "recording" ? t("Grabación vinculada a la lección.", "Recording linked to the lesson.")
        : target
        ? t("Adjuntos añadidos a la lección.", "Attachments added to the lesson.")
        : t("Materiales añadidos al curso.", "Materials added to the course."), "success");
    });
    if (imported && target) {
      if (target.kind === "unit") {
        const unit = document.getElementById(trigger)?.closest("details");
        if (unit) unit.open = true;
      }
      document.getElementById("course-materials-heading")?.focus();
    }
    else if (imported) focusCourseHeading();
    else focusReviewTrigger(trigger);
  }

  function fileCount(count: number) {
    return `${count} ${count === 1 ? t("archivo", "file") : t("archivos", "files")}`;
  }

  function reviewView(plan: MaterialReview, files: Set<number>, target?: MaterialTarget) {
    const disabled = busy ? "disabled" : "";
    const total = plan.groups.flatMap(group => group.files).filter(file => files.has(file.id)).reduce((sum, file) => sum + file.size, 0);
    return `<section class="course-review" aria-busy="${busy}">
      <div class="page-heading"><div><h1 id="course-heading" tabindex="-1">${t("Revisar materiales", "Review materials")}</h1>
        <p>${h(plan.title)} · ${h(courseProviderName(plan.provider))}</p></div></div>
      ${target ? `<p class="course-review-note">${target.kind === "unit" ? t("Recursos para la unidad", "Resources for the unit")
        : target.kind === "recording" ? t("Grabación para", "Recording for")
        : t("Adjuntos para", "Attachments for")} <strong>${h(target.title)}</strong></p>` : ""}
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
          ${action("course-review-confirm", target?.kind === "unit" ? t("Añadir a la unidad", "Add to unit")
            : target?.kind === "recording" ? t("Vincular a la lección", "Link to lesson")
            : target ? t("Añadir a la lección", "Add to lesson") : t("Añadir al curso", "Add to course"), "add", "primary", busy || !files.size ? "disabled" : "")}</div>
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
    </form>${source === "coursera" ? courseraForm() : outlineForm()}`;
  }

  function outlineForm(course?: Course): string {
    const saved = course?.sections.filter(isManualUnit)
      .map(section => [`# ${section.title}`, ...section.lessons.filter(isManualLesson).map(lesson => lesson.title)].join("\n")).join("\n\n") ?? "";
    return `<details id="course-manual-outline" class="course-resource-entry">
      <summary>${t("Temario manual", "Manual outline")}</summary>
      <form id="course-outline-form" class="settings-form" aria-busy="${busy}">
        <label for="course-outline-text">${t("Unidades y lecciones", "Units and lessons")}</label>
        <p id="course-outline-hint" class="field-note">${t("Escribe # antes de cada título de unidad y una lección por línea. Usa solo títulos.", "Start each unit title with # and put one lesson on each following line. Use titles only.")}</p>
        <textarea id="course-outline-text" name="outline" required rows="6" maxlength="65536" aria-describedby="course-outline-hint" placeholder="${t("# Unidad 1&#10;Introducción&#10;Primer ejercicio", "# Unit 1&#10;Introduction&#10;First exercise")}" ${busy ? "disabled" : ""}>${h(outlineText ?? saved)}</textarea>
        <div class="button-group"><button type="submit" class="button secondary" ${busy ? "disabled" : ""}>${t("Revisar temario", "Review outline")}</button></div>
      </form></details>`;
  }

  function outlineReviewView({ course }: OutlineReview): string {
    return `<section class="course-review course-outline-review">
      <div class="page-heading"><div><h1 id="course-heading" tabindex="-1">${t("Revisar temario", "Review outline")}</h1><p>${h(course.title)} · ${h(courseProviderName(course.provider))}</p></div></div>
      <p class="field-note">${t("Comprueba el orden antes de guardar. Las lecciones guardadas que falten en este temario se conservan.", "Check the order before saving. Saved lessons omitted from this outline are kept.")}</p>
      <div class="course-review-groups">${course.sections.map(section => `<details open><summary><strong>${h(section.title)}</strong><small>${lessonCount(section.lessons.length)}</small></summary><ol>${section.lessons.map(lesson => `<li>${h(lesson.title)}</li>`).join("")}</ol></details>`).join("")}</div>
      <footer class="course-review-footer"><output>${lessonCount(studyLessonCount(course))}</output><div class="button-group">
        ${action("course-outline-back", t("Editar temario", "Edit outline"), undefined, "quiet", busy ? "disabled" : "")}
        ${action("course-outline-save", t("Guardar temario", "Save outline"), "add", "primary", busy ? "disabled" : "")}</div></footer>
    </section>`;
  }

  function focusOutlineEditor(): void {
    const entry = document.querySelector<HTMLDetailsElement>("#course-manual-outline");
    if (entry) entry.open = true;
    document.getElementById("course-outline-text")?.focus();
  }

  function editableUnit(course: Course, lesson: CourseLesson) {
    if (!hasManualOutline(course.provider) || !isManualLesson(lesson)) return;
    return course.sections.find(section => isManualUnit(section) && section.lessons.some(item => item.id === lesson.id));
  }

  function lessonTitles(course: Course, lesson: CourseLesson, unit: CourseSection): string {
    const draft = titleDraft?.courseId === course.id && titleDraft.lessonId === lesson.id ? titleDraft : null;
    if (!draft) return `<div class="course-study-context"><span>${h(unit.title)}</span>${action("course-edit-titles", t("Editar títulos", "Edit titles"), undefined, "quiet", `id="course-edit-titles" ${busy ? "disabled" : ""}`)}</div>`;
    return `<form id="course-titles-form" class="settings-form" aria-label="${t("Editar títulos", "Edit titles")}" aria-busy="${busy}">
      <label>${t("Título de la unidad", "Unit title")}<input id="course-unit-title" required maxlength="240" value="${h(draft.unit)}" ${busy ? "disabled" : ""}></label>
      <label>${t("Título de la lección", "Lesson title")}<input id="course-lesson-title" required maxlength="240" value="${h(draft.lesson)}" ${busy ? "disabled" : ""}></label>
      ${draft.error ? `<p class="field-note" id="course-title-error" role="alert" tabindex="-1">${h(draft.error)}</p>` : ""}
      <div class="button-group">${action("course-cancel-titles", t("Cancelar", "Cancel"), undefined, "quiet", busy ? "disabled" : "")}
        <button type="submit" class="button secondary" ${busy ? "disabled" : ""}>${t("Guardar títulos", "Save titles")}</button></div></form>`;
  }

  function courseraForm(course?: Course): string {
    const idField = (name: string, caption: string, value = "") =>
      `<label>${caption}<input name="${name}" required maxlength="128" value="${h(value)}" autocomplete="off" spellcheck="false"></label>`;
    const status = course?.outlineStatus;
    const pages = status?.pages === 1 ? t("1 página consultada", "1 page fetched")
      : status ? t(`${status.pages} páginas consultadas`, `${status.pages} pages fetched`) : "";
    return `<details id="course-coursera-business" class="course-resource-entry">
      <summary>${t("Temario de Coursera Business", "Coursera Business outline")}${status ? `<small class="course-outline-status">${status.partial ? `${t("Temario parcial", "Partial outline")} · ` : ""}${pages}</small>` : ""}</summary>
      <form id="course-coursera-form" class="settings-form" aria-busy="${busy}">
        <p class="field-note">${t("Usa un token autorizado de Business, Campus o Government. La consulta guarda el temario y usa el token solo durante la solicitud.", "Use an authorized Business, Campus or Government token. This request saves the outline and uses the token only for the request.")}</p>
        ${status?.partial ? `<p class="field-note">${t("El temario guardado contiene las páginas que se pudieron consultar. Abre el curso en Coursera para ver el contenido restante.", "The saved outline contains the pages that could be fetched. Open the course in Coursera to see the remaining content.")}</p>` : ""}
        <fieldset ${busy ? "disabled" : ""}>
          <div class="form-grid">
            ${idField("organization", t("ID de organización", "Organization ID"), courseraIds.organization)}
            ${idField("program", t("ID de programa", "Program ID"), courseraIds.program)}
            ${idField("courseId", t("ID de curso en Business", "Business course ID"), course?.slug ?? courseraIds.courseId)}
            <label>${t("Token de acceso", "Access token")}<input name="token" type="password" required maxlength="16384" autocomplete="off"></label>
          </div>
          <div class="button-group"><button class="button secondary" type="submit">${busy ? t("Consultando…", "Fetching…") : course?.slug ? t("Actualizar temario", "Refresh outline") : t("Consultar temario", "Fetch outline")}</button></div>
        </fieldset>
      </form></details>`;
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

  function resourceReviewView(plan: ResourceReview) {
    const destination = plan.lesson?.title ?? t("Recursos generales del curso", "General course resources");
    return `<section class="course-review" aria-busy="${busy}">
      <div class="page-heading"><div><h1 id="course-heading" tabindex="-1">${t("Revisar adjuntos de Skillshare", "Review Skillshare attachments")}</h1>
        <p>${h(plan.title)}</p></div></div>
      <p class="course-review-note">${t("Destino", "Destination")} · <strong>${h(destination)}</strong></p>
      <p class="course-review-note">${t("Marca los adjuntos que quieras guardar en el curso. La disponibilidad se comprueba al descargar.", "Select the attachments to save to the course. Availability is checked at download time.")}</p>
      <ol class="course-resource-checks">${plan.checks.map((check, index) => `<li>
        <span class="course-resource-line" aria-hidden="true">${index + 1}</span>
        ${check.kind === "supported" ? `<label for="course-resource-check-${index}">
          <span>${h(check.filename)}</span><input id="course-resource-check-${index}" type="checkbox" data-resource-check="${index}"
            aria-label="${h(t("Incluir ", "Include ") + check.filename)}" ${plan.selected.has(index) ? "checked" : ""} ${busy ? "disabled" : ""}>
        </label>` : `<div class="course-resource-unavailable">
          <strong>${check.kind === "duplicate" ? t("Enlace duplicado", "Duplicate link") : t("Enlace no compatible", "Unsupported link")}</strong>
          <small>${check.kind === "duplicate" ? h(check.filename) : t("Usa un enlace directo de Descargar recursos.", "Use a direct link from Download Resources.")}</small>
        </div>`}
      </li>`).join("")}</ol>
      <footer class="course-review-footer">
        <output>${plan.selected.size ? fileCount(plan.selected.size) : t("Elige al menos un adjunto", "Select at least one attachment")}</output>
        <div class="button-group">${action("course-resource-back", t("Editar enlaces", "Edit links"), "back", "quiet", busy ? "disabled" : "")}
          ${action("course-resource-confirm", plan.lesson ? t("Añadir a la lección", "Add to lesson") : t("Añadir al curso", "Add to course"), "add", "primary", busy || !plan.selected.size ? "disabled" : "")}</div>
      </footer>
    </section>`;
  }

  function lessonArtwork(item: LibraryItem): string {
    const video = item.kind === "video";
    const thumbnail = video || item.details?.audioCover || item.details?.audio?.embeddedCover;
    return `<span class="course-playback-art" aria-hidden="true"${thumbnail ? ` data-thumbnail-id="${h(item.id)}"` : ""}>${icon(video ? "video" : "audio")}</span>`;
  }

  function lessonPlayback(item: LibraryItem): string {
    const position = resumePosition(item.details?.playbackPosition ?? 0, item.duration);
    const timing = [item.duration != null ? durationLabel(item.duration) : "",
      position ? `${t("Guardado en", "Saved at")} ${durationLabel(position)}` : ""].filter(Boolean).join(" · ");
    return `<div class="course-playback">
      ${lessonArtwork(item)}
      <div class="course-playback-copy"><span>${item.kind === "video" ? t("Vídeo de la lección", "Lesson video") : t("Audio de la lección", "Lesson audio")}</span><strong>${h(item.name)}</strong>${timing ? `<small>${h(timing)}</small>` : ""}</div>
      ${action("course-play", t("Reproducir lección", "Play lesson"), "play", "primary")}
    </div>`;
  }

  function resumableLesson(course: Course): LessonResume | undefined {
    let latest: LessonResume | undefined;
    for (const section of course.sections) {
      if (section.id === skillshareResourceSection) continue;
      for (const lesson of section.lessons) {
        if (lesson.learned || isUnitResource(lesson)) continue;
        const item = lessonMedia(lesson);
        const position = item && resumePosition(item.details?.playbackPosition ?? 0, item.duration);
        if (!item || !position) continue;
        if (!latest || (item.details?.lastPlayedAt ?? 0) > (latest.item.details?.lastPlayedAt ?? 0)) {
          latest = { sectionId: section.id, lesson, item, position };
        }
      }
    }
    return latest;
  }

  function courseResume(course: Course) {
    const next = resumableLesson(course);
    if (!next || next.lesson.id === lessonId) return "";
    return `<section class="course-playback course-resume" aria-label="${t("Retomar curso", "Resume course")}">
      ${lessonArtwork(next.item)}<div class="course-playback-copy"><span>${t("Retomar curso", "Resume course")}</span>
      <strong>${h(next.lesson.title)}</strong><small>${t("Guardado en", "Saved at")} ${durationLabel(next.position)}</small></div>
      ${action("course-resume", t("Retomar lección", "Resume lesson"), "play", "primary", busy ? "disabled" : "")}</section>`;
  }

  const unitResourcesTitle = () => t("Recursos de la unidad", "Unit resources");
  const lessonTitle = (lesson: CourseLesson) => isUnitResource(lesson) ? unitResourcesTitle() : lesson.title;
  const sectionTitle = (section: CourseSection) => section.id === skillshareResourceSection ? t("Recursos de Skillshare", "Skillshare resources") : section.title;

  function lessonRow(lesson: CourseLesson, material?: string) {
    const resource = isUnitResource(lesson);
    return `<li><button type="button" class="course-lesson${resource ? " course-unit-resources" : ""}" data-action="course-lesson" data-id="${h(lesson.id)}" ${lesson.id === lessonId ? 'aria-current="true"' : ""}>
      <span>${resource ? icon("folder") : ""}${h(lessonTitle(lesson))}</span>${material ? `<small>${h(material)}</small>` : ""}
      ${lesson.learned ? `<small>${resource ? t("Revisados", "Reviewed") : t("Aprendida", "Learned")}</small>` : ""}</button></li>`;
  }

  function renderCourseTree(course: Course, localStudy: boolean, hasResources: boolean) {
    const withResources = hasResources || course.provider === "domestika";
    const searchLabel = withResources ? t("Buscar lección o recurso", "Find lesson or resource") : t("Buscar lección o material", "Find lesson or material");
    const filter = searchKey(query.trim());
    const navigation = `<nav aria-label="${withResources ? t("Lecciones y recursos", "Lessons and resources") : t("Lecciones del curso", "Course lessons")}" class="course-tree">
      <label class="search-control">${icon("search")}<input id="course-search" type="search" value="${h(query)}" placeholder="${searchLabel}" aria-label="${searchLabel}"></label>`;
    if (filter) {
      let count = 0;
      const groups = course.sections.map(section => {
        const title = sectionTitle(section);
        const unitMatches = searchKey(title).includes(filter);
        const rows = section.lessons.flatMap(lesson => {
          if (unitMatches || searchKey(lessonTitle(lesson)).includes(filter)) return [lessonRow(lesson)];
          const match = lesson.assets.find(asset => searchKey(asset.title).includes(filter));
          return match ? [lessonRow(lesson, match.title)] : [];
        });
        count += rows.length;
        return rows.length ? `<section class="course-search-group"><h3>${h(title)}</h3><ul>${rows.join("")}</ul></section>` : "";
      }).join("");
      const summary = count === 0 ? t("No hay coincidencias en este curso.", "No matches in this course.")
        : count === 1 ? t("1 resultado", "1 result") : t(`${count} resultados`, `${count} results`);
      return `${navigation}<p class="field-note course-search-status" role="status">${h(summary)}</p>${groups}</nav>`;
    }
    return `${navigation}${course.sections.map(section => {
      const selectable = !localStudy || section.lessons.some(lesson => lesson.assets.some(available));
      const sectionElementId = `course-section-${section.id}`;
      const open = expansionBeforeSearch?.get(sectionElementId) ?? (section.lessons.some(lesson => lesson.id === lessonId) || !lessonId);
      return `<details id="${h(sectionElementId)}" ${open ? "open" : ""}>
        <summary>${h(sectionTitle(section))}</summary>
        ${selectable ? action("course-select-section", t("Seleccionar materiales disponibles", "Select available materials"), undefined, "quiet", `data-id="${h(section.id)}"`) : ""}
        <ul>${section.lessons.map(lesson => lessonRow(lesson)).join("")}</ul>
        ${course.provider === "domestika" && course.url ? action("course-add-unit-resources", t("Añadir recursos a la unidad", "Add resources to unit"), "add", "quiet",
          `id="course-add-unit-${h(section.id)}" data-id="${h(section.id)}" ${busy ? "disabled" : ""}`) : ""}
      </details>`;
    }).join("")}</nav>`;
  }

  function render(): string {
    if (review) return reviewView(review.plan, review.files, review.target);
    if (resourceReview) return resourceReviewView(resourceReview);
    if (outlineReview) return outlineReviewView(outlineReview);
    const course = current();
    const platformField = `<label><span id="course-platform-label">${t("Plataforma", "Platform")}</span><select id="course-provider" aria-labelledby="course-platform-label">${courseProviders.map(([id, name]) => option(id, name, provider)).join("")}</select></label>`;
    const linkField = `<label>${t("Enlace del curso", "Course URL")}<input id="course-url" type="url" required value="${h(input)}" placeholder="https://…" autocomplete="off"></label>`;
    if (!course) return `<div class="page-heading"><div><h1 id="course-heading" tabindex="-1">${t("Cursos", "Courses")}</h1><p>${t("Descarga materiales autorizados y estudia tus archivos locales.", "Download authorized materials and study your local files.")}</p></div>${action("course-import", t("Importar curso local", "Import local course"), "folder", isBrowserProvider(provider) ? "secondary" : "primary", busy ? "disabled" : "")}</div>
      ${isBrowserProvider(provider) ? browserForm(provider, platformField, linkField) : `<form id="course-open-online" class="settings-form" aria-busy="${busy}"><h2>${t("Abrir un curso con acceso autorizado", "Open an authorized course")}</h2>${platformField}${linkField}${provider !== "udemy" ? `<div class="form-grid"><label>${t("ID numérico del curso o producto", "Numeric course or product ID")}<input id="course-product-id" inputmode="numeric" required value="${h(productId)}"></label>${provider === "hotmart" ? `<label>${t("Identificador del club, slug", "Club identifier, slug")}<input id="course-slug" required value="${h(slug)}"></label>` : ""}</div>` : ""}<div class="form-grid"><label>${t("Máximo de lecciones a explorar", "Lesson exploration limit")}<input id="course-limit" type="number" min="1" max="1000" value="${limit}"></label><div class="button-group"><button type="submit" class="button secondary" ${busy ? "disabled" : ""}>${busy ? t("Explorando…", "Exploring…") : t("Explorar con mi sesión", "Explore with my session")}</button></div></div><p class="field-note">${provider === "udemy" ? t("Importa tu sesión en Ajustes.", "Import your session in Settings.") : t("Conecta un token de alumno de esta plataforma. El ID y el slug se encuentran en el enlace o los datos del curso.", "Connect a student token for this platform. The ID and slug are in the course link or course data.")} ${t("Solo se ofrecen archivos con una ruta de descarga autorizada. Reproducir una lección no implica permiso para guardarla.", "Only materials with an authorized download route are offered. Playback access does not imply permission to save a lesson.")}</p></form>`}
      ${provider === "hotmart" || provider === "kiwify" ? `<form id="course-token" class="settings-form"><h2>${t("Conexión de alumno", "Student connection")} · ${h(provider)}</h2><label>${t("Token de acceso autorizado", "Authorized access token")}<input id="course-token-secret" type="password" required autocomplete="off"></label><p class="field-note">${t("Se protege localmente. No se publica ni se transfiere a otra plataforma. La conexión no descarga contenido por sí sola.", "Stored protected locally. It is not published or sent to another platform. Connecting does not download content.")}</p><div class="button-group"><button type="submit" class="button secondary" ${busy ? "disabled" : ""}>${t("Guardar conexión", "Save connection")}</button>${action("course-token-remove", t("Eliminar conexión", "Remove connection"), undefined, "quiet", busy ? "disabled" : "")}</div></form>` : ""}
      ${courses.length ? `<div class="source-list">${courses.map(course => `<article class="source-row"><button class="media-open" data-action="course-open" data-id="${h(course.id)}"><span class="media-info"><strong>${h(course.title)}</strong><span>${h(course.provider === "local" ? t("Carpeta local", "Local folder") : courseProviderName(course.provider))} · ${lessonCount(studyLessonCount(course))}</span></span>${icon("arrow")}</button></article>`).join("")}</div>` : isBrowserProvider(provider) ? "" : `<div class="empty-state">${icon("library")}<h2>${t("Tu primer curso", "Your first course")}</h2><p>${t("Importa una carpeta con clases legítimamente guardadas o abre un curso al que tengas acceso.", "Import a folder of legitimately saved lessons or open a course you can access.")}</p></div>`}`;
    const browserAssisted = isBrowserProvider(course.provider);
    const localStudy = browserAssisted || course.provider === "local";
    const lesson = currentLesson();
    const lessons = course.sections.flatMap(section => section.lessons);
    const resources = course.sections.find(section => section.id === skillshareResourceSection);
    const resourceCount = course.provider === "skillshare"
      ? lessons.flatMap(lesson => lesson.assets).filter(asset => asset.status === "downloadable" && asset.kind === "attachment").length
      : 0;
    const lessonTotal = studyLessonCount(course);
    const courseCounts = [lessonTotal || !resourceCount ? lessonCount(lessonTotal) : "", resourceCount ? `${resourceCount} ${resourceCount === 1 ? t("adjunto", "attachment") : t("adjuntos", "attachments")}` : ""].filter(Boolean).join(" · ");
    const unitResource = lesson && isUnitResource(lesson);
    const resourceLesson = unitResource || resources?.lessons.some(candidate => candidate.id === lessonId);
    const showQueue = !localStudy || lessons.some(lesson => lesson.assets.some(asset => asset.status === "downloadable"));
    const item = lessonMedia(lesson);
    const manualUnit = lesson ? editableUnit(course, lesson) : undefined;
    const canLinkRecording = browserAssisted && !resourceLesson && !item && lesson && !lesson.assets.some(asset =>
      asset.status === "local" && (asset.kind === "audio" || asset.kind === "video"));
    const queueable = course.sections.flatMap(section => section.lessons).flatMap(lesson => lesson.assets).filter(asset => selected.has(asset.id) && available(asset));
    return `<div class="page-heading"><div>${action("course-back", t("Todos los cursos", "All courses"), undefined, "quiet")}<h1 id="course-heading" tabindex="-1">${h(course.title)}</h1><p>${h(course.provider === "local" ? t("Archivos locales", "Local files") : courseProviderName(course.provider))} · ${courseCounts}</p></div>${browserAssisted ? `<div class="button-group">${action("course-browser-open", t("Abrir curso en el navegador", "Open course in browser"), "arrow", "quiet", busy ? "disabled" : "")}${action("course-add-folder", t("Añadir carpeta", "Add folder"), "folder", "secondary", `id="course-add-folder" ${busy ? "disabled" : ""}`)}${action("course-add-materials", t("Añadir archivos guardados", "Add saved files"), "add", "primary", `id="course-add-materials" ${busy ? "disabled" : ""}`)}</div>` : course.url ? action("course-refresh", t("Actualizar materiales", "Refresh materials"), "retry", "quiet", busy ? "disabled" : "") : ""}</div>
      ${courseResume(course)}
      ${course.provider === "skillshare" ? skillshareEntry(course) : course.provider === "coursera" ? courseraForm(course) : ""}
      ${hasManualOutline(course.provider) ? outlineForm(course) : ""}
      ${!showQueue ? "" : `<div class="selection-toolbar"><span>${queueable.length === 1 ? t("1 material seleccionado", "1 material selected") : `${queueable.length} ${t("materiales seleccionados", "materials selected")}`}</span>${action("course-queue", t("Añadir a la cola", "Add to queue"), "add", "primary", busy || !queueable.length ? "disabled" : "")}${action("course-clear", t("Quitar selección", "Clear selection"), undefined, "quiet")}</div>`}
      <div class="course-layout${browserAssisted ? " course-layout--browser" : ""}">${renderCourseTree(course, localStudy, Boolean(resourceCount))}<section class="course-study">${lesson ? `<div class="page-heading"><h2 id="course-study-title" tabindex="-1">${h(lessonTitle(lesson))}</h2>${action("course-learned", resourceLesson ? (lesson.learned ? t("Marcar pendiente", "Mark as pending") : t("Marcar revisado", "Mark as reviewed")) : (lesson.learned ? t("Marcar pendiente de aprender", "Mark as not learned") : t("Marcar aprendida", "Mark as learned")), "check", "quiet", `aria-pressed="${lesson.learned}" ${busy ? "disabled" : ""}`)}</div>
      ${lesson && manualUnit ? lessonTitles(course, lesson, manualUnit) : ""}
      ${item ? lessonPlayback(item) : ""}
      <div class="course-materials-heading"><h3 id="course-materials-heading" tabindex="-1">${t("Materiales", "Materials")}</h3>${browserAssisted && !resourceLesson ? `<div class="button-group">
        ${action("course-add-attachments", t("Añadir adjuntos", "Add attachments"), "add", "quiet", `id="course-add-attachments" ${busy ? "disabled" : ""}`)}
        ${canLinkRecording ? action("course-link-recording", t("Vincular audio o vídeo", "Link audio or video"), "video", "secondary", `id="course-link-recording" ${busy ? "disabled" : ""}`) : ""}</div>` : ""}</div><div class="source-items">${lesson.assets.map(asset => materialRow(asset, localStudy)).join("")}</div>
      <h3>${unitResource ? t("Notas de la unidad", "Unit notes") : resourceLesson ? t("Notas de este adjunto", "Attachment notes") : t("Notas de esta lección", "Lesson notes")}</h3><form id="course-note" class="settings-form"><label>${t("Nota", "Note")}<textarea id="course-note-text" rows="3" maxlength="4000" required>${h(noteText)}</textarea></label><div class="form-grid"><label>${t("Tiempo en segundos, opcional", "Time in seconds, optional")}<input id="course-note-time" type="number" min="0" step="0.1" value="${h(noteTime)}"></label><div class="button-group">${item ? action("course-note-now", t("Usar tiempo actual", "Use current time"), undefined, "quiet", player.current()?.id === item.id ? "" : "disabled") : ""}<button class="button secondary" type="submit" ${busy ? "disabled" : ""}>${t("Guardar nota", "Save note")}</button></div></div></form>
      ${lesson.notes.map(note => `<article class="course-note"><p>${h(note.text)}</p><div class="button-group">${note.time === null ? "" : item ? action("course-note-seek", `${note.time.toFixed(1)} s`, "play", "quiet", `data-time="${note.time}"`) : `<span class="field-note">${note.time.toFixed(1)} s</span>`}${action("course-note-remove", t("Eliminar nota", "Remove note"), "close", "quiet", `data-id="${h(note.id)}" ${busy ? "disabled" : ""}`)}</div></article>`).join("")}` : `<div class="empty-state"><h2>${t("Elige una lección", "Choose a lesson")}</h2><p>${t("Guardado y aprendido son estados independientes.", "Saved and learned are separate states.")}</p></div>`}</section></div>`;
  }

  function handleInput(element: HTMLElement): boolean {
    if (resourceReview && element instanceof HTMLInputElement && element.dataset.resourceCheck !== undefined) {
      const index = Number(element.dataset.resourceCheck);
      if (resourceReview.checks[index]?.kind === "supported") {
        if (element.checked) resourceReview.selected.add(index);
        else resourceReview.selected.delete(index);
        hooks.render();
      }
      return true;
    }
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
        if (next && next !== provider) { provider = next; resourceLinks = ""; resourceLessonId = ""; outlineText = null; }
        hooks.render(); return true;
      }
      case "course-browser-title": browserTitle = element.value; return true;
      case "course-outline-text": outlineText = element.value; return true;
      case "course-unit-title":
      case "course-lesson-title":
        if (titleDraft) {
          titleDraft[element.id === "course-unit-title" ? "unit" : "lesson"] = element.value;
          titleDraft.error = "";
          document.getElementById("course-title-error")?.remove();
        }
        return true;
      case "course-resource-links": resourceLinks = element.value; return true;
      case "course-resource-lesson": resourceLessonId = element.value; return true;
      case "course-product-id": productId = element.value; return true;
      case "course-slug": slug = element.value; return true;
      case "course-url": input = element.value; return true;
      case "course-limit": limit = Number(element.value); return true;
      case "course-search": {
        const searching = Boolean(searchKey(element.value.trim()));
        if (searching && !expansionBeforeSearch) {
          expansionBeforeSearch = new Map([...document.querySelectorAll<HTMLDetailsElement>(".course-tree details")]
            .map(section => [section.id, section.open]));
        }
        query = element.value;
        hooks.render();
        if (!searching) expansionBeforeSearch = null;
        return true;
      }
      case "course-note-text": noteText = element.value; return true;
      case "course-note-time": noteTime = element.value; return true;
      default: return false;
    }
  }
  async function handleSubmit(form: HTMLFormElement): Promise<boolean> {
    if (form.id === "course-titles-form") {
      const draft = titleDraft;
      if (!draft || busy || !form.reportValidity()) return true;
      await perform(async () => {
        try {
          await hooks.command("rename_course_outline_titles", { courseId: draft.courseId, lessonId: draft.lessonId,
            unitTitle: draft.unit, lessonTitle: draft.lesson });
        } catch (error) {
          switch (error) {
            case "duplicate-course-unit-title": draft.error = t("Ya existe una unidad con ese título.", "A unit with that title already exists."); break;
            case "duplicate-course-lesson-title": draft.error = t("Ya existe una lección con ese título en la unidad.", "A lesson with that title already exists in this unit."); break;
            case "invalid-course-titles": draft.error = t("Escribe títulos de hasta 240 caracteres sin enlaces ni # inicial.", "Enter titles of up to 240 characters without links or a leading #."); break;
            default: hooks.showError(error); break;
          }
          return false;
        }
        titleDraft = null;
        outlineText = null;
      });
      const focus = titleDraft ? document.getElementById("course-title-error") ?? document.getElementById("course-lesson-title")
        : document.getElementById("course-study-title");
      focus?.focus();
      return true;
    }
    if (form.id === "course-outline-form") {
      const course = current();
      const source = course?.provider ?? provider;
      if (!hasManualOutline(source) || busy || !form.reportValidity()) return true;
      if (!course && !document.querySelector<HTMLFormElement>("#course-browser-form")?.reportValidity()) return true;
      const text = new FormData(form).get("outline");
      if (typeof text !== "string") return true;
      outlineText = text;
      const request = { provider: source, url: course?.url ?? input, title: course?.title ?? browserTitle, text };
      await perform(async () => {
        outlineReview = { course: await hooks.command<Course>("course_manual_outline", { ...request, save: false }), request };
        return false;
      });
      if (outlineReview) focusCourseHeading();
      else focusOutlineEditor();
      return true;
    }
    if (form.id === "course-browser-form") {
      if (isBrowserProvider(provider) && form.reportValidity()) await reviewMaterials(provider, input, browserTitle);
      return true;
    }
    if (form.id === "course-coursera-form") {
      const course = current();
      if (busy || (course?.provider ?? provider) !== "coursera") return true;
      if (!course && !document.querySelector<HTMLFormElement>("#course-browser-form")?.reportValidity()) return true;
      const data = new FormData(form);
      const token = form.querySelector<HTMLInputElement>('input[name="token"]');
      if (!token) return true;
      const accessToken = token.value;
      token.value = "";
      courseraIds = {
        organization: String(data.get("organization") ?? ""),
        program: String(data.get("program") ?? ""),
        courseId: String(data.get("courseId") ?? ""),
      };
      let saved = false;
      await perform(async () => {
        const result = await hooks.command<Course>("inspect_coursera_course", {
          url: course?.url ?? input, title: course?.title ?? browserTitle, ...courseraIds, token: accessToken,
        });
        courseId = result.id;
        lessonId = course?.id === courseId ? lessonId : result.sections[0]?.lessons[0]?.id ?? null;
        saved = true;
        hooks.notify(t("Temario guardado.", "Outline saved."), "success");
      });
      const entry = document.querySelector<HTMLDetailsElement>("#course-coursera-business");
      if (entry) entry.open = !saved;
      if (saved) focusCourseHeading();
      else entry?.querySelector<HTMLElement>('input[name="token"]')?.focus();
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
      case "course-edit-titles": {
        const unit = course && lesson && editableUnit(course, lesson);
        if (!course || !lesson || !unit || busy) return true;
        titleDraft = { courseId: course.id, lessonId: lesson.id, unit: unit.title, lesson: lesson.title, error: "" };
        hooks.render();
        document.getElementById("course-lesson-title")?.focus();
        return true;
      }
      case "course-cancel-titles":
        if (busy) return true;
        titleDraft = null;
        hooks.render();
        document.getElementById("course-edit-titles")?.focus();
        return true;
      case "course-outline-back":
        if (busy) return true;
        outlineReview = null;
        hooks.render();
        focusOutlineEditor();
        return true;
      case "course-outline-save": {
        const review = outlineReview;
        if (!review || busy) return true;
        await perform(async () => {
          const next = await hooks.command<Course>("course_manual_outline", { ...review.request, save: true });
          if (courseId !== next.id) { selected.clear(); lessonId = next.sections[0]?.lessons[0]?.id ?? null; }
          courseId = next.id;
          outlineReview = null;
          outlineText = null;
        });
        focusCourseHeading();
        return true;
      }
      case "course-review-resources": {
        if ((course?.provider ?? provider) !== "skillshare" || !course && !element.closest("form")?.reportValidity()) return true;
        const links = resourceLinks.split(/\r?\n/).map(line => line.trim()).filter(Boolean);
        if (!links.length || links.length > 30) {
          hooks.notify(t("Pega entre 1 y 30 enlaces a adjuntos.", "Paste between 1 and 30 attachment links."), "error");
          document.getElementById("course-resource-links")?.focus();
          return true;
        }
        const target = course?.sections.flatMap(section => section.lessons).find(lesson => lesson.id === resourceLessonId) ?? null;
        await perform(async () => {
          const checks = await hooks.command<ResourceCheck[]>("review_skillshare_resources", { links });
          resourceReview = { url: course?.url ?? input, title: course?.title ?? browserTitle, lesson: target, links, checks, selected: new Set() };
          return false;
        });
        if (resourceReview) focusCourseHeading();
        return true;
      }
      case "course-resource-back": {
        if (!resourceReview || busy) return true;
        resourceReview = null;
        hooks.render();
        const entry = document.getElementById("course-skillshare-resources");
        if (entry instanceof HTMLDetailsElement) entry.open = true;
        document.getElementById("course-resource-links")?.focus();
        return true;
      }
      case "course-resource-confirm": {
        const plan = resourceReview;
        if (!plan?.selected.size || busy) return true;
        const links = plan.links.filter((_, index) => plan.selected.has(index));
        const target = plan.lesson?.id ?? null;
        let opened = false;
        await perform(async () => {
          const next = await hooks.command<Course>("add_skillshare_resources", { url: plan.url, title: plan.title, links, lessonId: target });
          if (courseId !== next.id) selected.clear();
          courseId = next.id;
          lessonId = target ?? next.sections.find(section => section.id === skillshareResourceSection)?.lessons[0]?.id ?? null;
          resourceLinks = "";
          resourceLessonId = "";
          resourceReview = null;
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
        if (lesson && course?.url && isBrowserProvider(course.provider)) {
          await reviewMaterials(course.provider, course.url, course.title, false, { ...lesson, kind: "lesson" });
        }
        return true;
      case "course-link-recording":
        if (lesson && course?.url && isBrowserProvider(course.provider)) {
          await reviewMaterials(course.provider, course.url, course.title, false, { ...lesson, kind: "recording" });
        }
        return true;
      case "course-add-unit-resources": {
        const unit = course?.sections.find(section => section.id === element.dataset.id);
        if (unit && course?.url && course.provider === "domestika") {
          await reviewMaterials(course.provider, course.url, course.title, false, { ...unit, kind: "unit" });
        }
        return true;
      }
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
      case "course-open": courseId = element.dataset.id ?? null; lessonId = current()?.sections[0]?.lessons[0]?.id ?? null; clearSearch(); resourceLinks = ""; resourceLessonId = ""; outlineText = null; titleDraft = null; break;
      case "course-back": courseId = null; lessonId = null; clearSearch(); resourceLinks = ""; resourceLessonId = ""; outlineText = null; titleDraft = null; selected.clear(); break;
      case "course-lesson": {
        const next = element.dataset.id ?? null;
        if (next !== lessonId) { lessonId = next; noteText = ""; noteTime = ""; titleDraft = null; }
        if (!expansionBeforeSearch) break;
        const section = course?.sections.find(section => section.lessons.some(lesson => lesson.id === lessonId));
        if (section) expansionBeforeSearch.set(`course-section-${section.id}`, true);
        hooks.render();
        document.getElementById("course-study-title")?.focus();
        return true;
      }
      case "course-clear": selected.clear(); break;
      case "course-select-section": if (course) for (const asset of course.sections.find(section => section.id === element.dataset.id)?.lessons.flatMap(lesson => lesson.assets) ?? []) if (available(asset)) selected.add(asset.id); break;
      case "course-queue": if (course) await perform(async () => { await hooks.command("queue_course_assets", { courseId: course.id, assetIds: [...selected], clientRequest: crypto.randomUUID() }); selected.clear(); hooks.notify(t("Materiales añadidos a la cola.", "Materials added to queue."), "success"); }); break;
      case "course-refresh": if (course?.url) await perform(async () => { if (course.provider === "udemy") await hooks.command("inspect_course", { url: course.url, limit }); else await hooks.command("inspect_member_course", { provider: course.provider, url: course.url, productId: course.id.replace(/^(hotmart|kiwify)-/, ""), slug: course.slug ?? "", limit }); }); break;
      case "course-learned": if (course && lesson) await perform(async () => { await hooks.command("course_learned", { courseId: course.id, lessonId: lesson.id, learned: !lesson.learned }); }); break;
      case "course-open-file": if (element.dataset.id) hooks.openLibrary(element.dataset.id); return true;
      case "course-locate-file": {
        const id = element.dataset.id;
        if (!id || busy) return true;
        await perform(async () => {
          await hooks.locateFile(id);
          await reload();
          return false;
        });
        document.getElementById(element.id)?.focus({ preventScroll: true });
        return true;
      }
      case "course-resume": {
        const next = course && resumableLesson(course);
        if (!next || busy) return true;
        lessonId = next.lesson.id;
        noteText = "";
        noteTime = "";
        clearSearch();
        const section = document.getElementById(`course-section-${next.sectionId}`);
        if (section instanceof HTMLDetailsElement) section.open = true;
        player.playFrom(next.item.id, next.position, true);
        if (next.item.kind === "audio") document.getElementById("course-study-title")?.focus();
        return true;
      }
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
