import "./translation.css";
import { escapeHtml as h, type Copy, type Job, type Snapshot } from "./contracts";
import { createDubbing } from "./dubbing-ui";
import { publishedTranslationFile, translationOutput } from "./translation-output";
import { action, icon, option, stateLabel } from "./views-common";

type Pair = "es-en" | "en-es";
type Mode = "translation" | "dubbing";
interface Input { libraryId: string; trackId: string }
interface Cue { identifier: string | null; start: number; end: number; text: string }
interface Preview { sourceHash: string; count: number; cues: Cue[] }
interface Draft {
  id: string;
  revision: number;
  name: string;
  selection: { input: Input; pair: Pair; sourceHash: string };
  original: Cue[];
  translated: string[];
  contextualBlocks: number;
  edited: number[];
}
interface Component { id: string; installed: boolean; enabled: boolean; available: boolean }
interface Hooks {
  snapshot(): Snapshot;
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  render(): void;
  refresh(): Promise<void>;
  notify(message: string): void;
  components(): void;
  openTool(mode: Mode): Promise<void>;
  t: Copy;
}

const directions: Pair[] = ["es-en", "en-es"];
const pageSize = 12;
const lineLimit = 42;
const readingLimit = 20;
function styled(text: string) {
  return h(text).replace(/&lt;(\/?(?:b|i|u))&gt;/g, "<$1>").replace(/&amp;(lt|gt|amp);/g, "&$1;");
}

function timing(time: number) {
  const ms = Math.round(time * 1000);
  const h = Math.floor(ms / 3600000), m = Math.floor(ms / 60000) % 60;
  const s = Math.floor(ms / 1000) % 60;
  const two = (value: number) => String(value).padStart(2, "0");
  return `${two(h)}:${two(m)}:${two(s)}.${String(ms % 1000).padStart(3, "0")}`;
}

function lengthWarning(text: string, cue: Cue) {
  const lines = text.replace(/<\/?(?:b|i|u)>/gi, "").replace(/&(lt|gt|amp|quot|#39);/g, "_").split("\n");
  const counts = lines.map(line => Array.from(line).length);
  const longest = Math.max(...counts);
  const rate = counts.reduce((total, length) => total + length, 0) / (cue.end - cue.start);
  return longest > lineLimit || rate > readingLimit ? { longest, rate } : null;
}

export function createTranslationTool(hooks: Hooks) {
  const { t } = hooks;
  let components: Component[] | null = null;
  let pair: Pair | "" = "";
  let sourceKey = "", query = "", workingId = "";
  let sourcePreview: Preview | null = null;
  let draft: Draft | null = null;
  let draftIds: string[] | null = null;
  let busy = false, error = "", confirmRemoval = false;
  let offset = 0;
  let warningCursor: number | null = null;
  let format: "srt" | "vtt" = "vtt", bilingual = false;
  const edits = new Map<number, string>();
  const warnings = new Set<number>();
  const dubbing = createDubbing({ ...hooks, hasEdits: () => edits.size > 0,
    draft: () => draft ? { id: draft.id, revision: draft.revision, pair: draft.selection.pair, sourceId: draft.selection.input.libraryId, count: draft.original.length } : null,
    cueTiming: index => {
      const cue = draft?.original[index];
      return cue ? `${timing(cue.start)} – ${timing(cue.end)}` : "";
    },
    focusCue: async index => {
      offset = Math.floor(index / pageSize) * pageSize;
      warningCursor = index;
      await hooks.openTool("translation");
      requestAnimationFrame(() => document.getElementById(`translation-cue-${index}`)?.focus());
    },
  });

  const directionLabel = (value: Pair) => value === "es-en"
    ? t("Español → Inglés", "Spanish → English") : t("Inglés → Español", "English → Spanish");
  const available = () => directions.filter(value => components?.some(component => component.id === `translate-${value}` && component.available));
  const installed = () => available().filter(value => components?.some(component => component.id === `translate-${value}` && component.installed && component.enabled));
  const tracks = () => hooks.snapshot().library.flatMap(item => item.missing ? [] : (item.details?.tracks ?? []).map(track => ({
    key: `${item.id}/${track.id}`,
    input: { libraryId: item.id, trackId: track.id },
    label: `${item.name} · ${track.language}`,
    language: track.language,
  })));
  const source = () => tracks().find(track => track.key === sourceKey);
  const working = () => hooks.snapshot().jobs.find(job => job.id === workingId);
  const active = () => {
    const job = working();
    return Boolean(job && !["completed", "cancelled", "error"].includes(job.state));
  };
  const warningCountLabel = () => `${warnings.size} ${warnings.size === 1 ? t("aviso de longitud", "length warning") : t("avisos de longitud", "length warnings")}`;

  function warning(details: NonNullable<ReturnType<typeof lengthWarning>>) {
    const reasons: string[] = [];
    if (details.longest > lineLimit) reasons.push(t(`${details.longest} caracteres en la línea más larga (referencia ${lineLimit})`,
      `${details.longest} characters in the longest line (guideline ${lineLimit})`));
    if (details.rate > readingLimit) {
      const rate = (Math.ceil(details.rate * 10) / 10).toLocaleString(t("es", "en"));
      reasons.push(t(`${rate} caracteres por segundo (referencia ${readingLimit})`, `${rate} characters per second (guideline ${readingLimit})`));
    }
    const remedy = details.rate > readingLimit ? t("Acorta el texto.", "Shorten the text.")
      : t("Divide la línea o acórtala.", "Split or shorten the line.");
    return `${reasons.join(" · ")}. ${remedy}`;
  }

  async function load(mode: Mode) {
    try {
      const listing = hooks.command<string[]>("translation_drafts");
      if (mode === "translation") {
        [draftIds, components] = await Promise.all([listing, hooks.command<Component[]>("optional_components")]);
        const choices = installed();
        if (!pair || !choices.includes(pair)) pair = choices[0] ?? "";
        const pending = hooks.snapshot().jobs.find(job => job.source.kind === "captionTranslation"
          && !["completed", "cancelled", "error"].includes(job.state));
        if (!workingId && pending) workingId = pending.id;
        const current = working();
        if (current?.state === "completed" && draft) workingId = "";
        else if (current) observe(current, mode);
      } else {
        [draftIds] = await Promise.all([listing, dubbing.load()]);
      }
      if (draft && !draftIds.includes(draft.id) && !edits.size) clearDraft();
    } catch (cause) { error = String(cause); }
    hooks.render();
  }

  async function run(task: () => Promise<void>) {
    if (busy) return;
    busy = true; error = ""; hooks.render();
    try { await task(); }
    catch (cause) { error = String(cause).slice(0, 600); }
    finally { busy = false; hooks.render(); }
  }

  async function open(id: string) {
    const result = await hooks.command<Draft>("translation_read", { draftId: id });
    dubbing.resetDraft();
    draft = result; edits.clear(); offset = 0; warningCursor = null; confirmRemoval = false;
    updateWarnings(result);
    if (!draftIds?.includes(id)) draftIds = [id, ...(draftIds ?? [])];
  }

  function clearDraft() {
    dubbing.resetDraft();
    draft = null; edits.clear(); warnings.clear(); warningCursor = null; confirmRemoval = false;
  }

  function updateWarnings(current: Draft) {
    warnings.clear();
    current.original.forEach((cue, index) => {
      if (lengthWarning(current.translated[index], cue)) warnings.add(index);
    });
  }

  function observe(job: Job, mode: Mode) {
    if (mode === "dubbing") {
      dubbing.observe(job);
      if (job.source.kind === "captionTranslation" && job.state === "completed" && draftIds && !draftIds.includes(job.id)) {
        draftIds = [job.id, ...draftIds]; hooks.render();
      }
      return;
    }
    if (job.id !== workingId) return;
    if (job.state === "completed" && !edits.size && !busy) {
      workingId = "";
      void run(async () => {
        await open(job.id);
        hooks.notify(t("Traducción lista para revisar.", "Translation ready to review."));
      });
    } else if (job.state === "error" && job.error) {
      error = job.error.message;
      hooks.render();
    } else {
      hooks.render();
    }
  }

  function setup() {
    const selected = source();
    const eligible = tracks().filter(track => track.key === sourceKey || track.label.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
    const matches = eligible.slice(0, 200);
    if (selected && !matches.some(track => track.key === selected.key)) matches.unshift(selected);
    const choices = installed();
    const disabled = busy || active() || edits.size > 0;
    const directionsAvailable = available().map(directionLabel).join(" · ");
    const job = working();
    return `<form id="translation-setup" class="translation-setup">
      <fieldset class="translation-fields" ${disabled ? "disabled" : ""}><legend>${t("Entrada y par de idiomas", "Input and language pair")}</legend>
        <label for="translation-search">${t("Buscar en Biblioteca", "Find in Library")}<input id="translation-search" type="search" value="${h(query)}"></label>
        <label for="translation-source">${t("Pista local", "Local track")}<select id="translation-source">
          ${option("", t("Selecciona una pista", "Select a track"), sourceKey)}${matches.map(track => option(track.key, track.label, sourceKey)).join("")}
        </select></label>
        <label for="translation-pair">${t("Dirección instalada", "Installed direction")}<select id="translation-pair" ${choices.length ? "" : "disabled"}>
          ${choices.length ? choices.map(value => option(value, directionLabel(value), pair)).join("")
            : option("", t("Instala un par de idiomas", "Install a language pair"), "")}
        </select></label>
        <div class="button-group">${action("import-files", t("Importar SRT o VTT", "Import SRT or VTT"), "add", "quiet")}
        ${action("translation-inspect", t("Revisar pista", "Review track"), "document", "secondary", !selected ? "disabled" : "")}</div>
      </fieldset>
      ${eligible.length > 200 ? `<p class="field-note">${t("Acota la búsqueda para ver más pistas.", "Narrow the search to see more tracks.")}</p>` : ""}
      ${components && choices.length < available().length ? `<div class="upscale-component"><div><strong>Bergamot</strong>
        <p class="field-note">${h(directionsAvailable)} · ${t("Modelos opcionales. La traducción se ejecuta en este equipo.", "Optional models. Translation runs on this device.")}</p></div>
        ${action("translation-components", t("Ver componentes", "View components"), "settings", "secondary")}</div>` : ""}
      ${components && !directionsAvailable ? `<p class="field-note">${t("Bergamot requiere Windows x64 y los motores de nabnab preparados.", "Bergamot requires Windows x64 and the prepared nabnab engines.")}</p>` : ""}
      ${sourcePreview ? `<div class="translation-original-sample"><strong>${t("Muestra del original", "Original sample")} · ${sourcePreview.count} ${t("segmentos", "cues")}</strong>
        <ol>${sourcePreview.cues.map(cue => `<li><time>${timing(cue.start)}</time><span>${styled(cue.text)}</span></li>`).join("")}</ol></div>` : ""}
      <div class="asset-submit"><p class="field-note">${job && job.state !== "completed"
        ? `<span role="status">${stateLabel(job.state, t)}</span>` : t("El resultado se guarda como borrador editable con los tiempos del original.", "The result is saved as an editable draft with the source timing.")}</p>
        <div class="button-group">${active() ? action("translation-cancel", t("Cancelar traducción", "Cancel translation"), "close", "quiet") : ""}
        <button type="submit" class="button primary" ${!pair || !selected || !sourcePreview || disabled ? "disabled" : ""}>${t("Traducir en local", "Translate locally")}</button></div></div>
    </form>`;
  }

  function comparison(current: Draft) {
    const dubIssues = new Set(dubbing.timingIssues());
    const rows = current.original.slice(offset, offset + pageSize).map((cue, visibleIndex) => {
      const index = offset + visibleIndex;
      const target = edits.get(index) ?? current.translated[index];
      const details = warnings.has(index) ? lengthWarning(target, cue) : null;
      const dubIssue = dubIssues.has(index);
      return `<li class="translation-cue">
        <div class="translation-time"><span>${h(cue.identifier ?? String(index + 1))}</span>
          <time>${timing(cue.start)} – ${timing(cue.end)}</time></div>
        <div class="translation-columns">
          <div class="translation-original"><span class="field-note">${t("Original", "Original")}</span>
            <p>${styled(cue.text)}</p></div>
          <label for="translation-cue-${index}"><span class="field-note">${t("Traducción", "Translation")}</span>
            <textarea id="translation-cue-${index}" data-translation-index="${index}" aria-label="${t("Traducción del segmento", "Translation of cue")} ${index + 1}"
              aria-describedby="translation-warning-${index}${dubIssue ? ` translation-dub-warning-${index}` : ""}" rows="2" maxlength="16000" ${busy ? "disabled" : ""}>${h(target)}</textarea>
            <small id="translation-warning-${index}" class="translation-warning" ${details ? "" : "hidden"}>${details ? h(warning(details)) : ""}</small>
            ${dubIssue ? `<small id="translation-dub-warning-${index}" class="translation-warning">${t("No cabía en la última síntesis.", "Did not fit in the last synthesis.")}</small>` : ""}
          </label>
        </div></li>`;
    }).join("");
    return `<section class="asset-review translation-review" aria-labelledby="translation-title">
      <div class="section-heading"><div><h3 id="translation-title">${t("Comparar y editar", "Compare and edit")}</h3>
        <p class="field-note">${h(current.name)} · ${directionLabel(current.selection.pair)} · ${t("Traducción automática", "Machine translation")}</p></div>
        <div class="button-group translation-review-actions">
          ${action("translation-save", t("Guardar edición", "Save edits"), "check", "secondary", `id="translation-save" ${busy || !edits.size ? "disabled" : ""}`)}
          ${action("dub-open", t("Crear doblaje", "Create dubbing"), "audio", "quiet", `id="dub-open" ${busy || edits.size ? "disabled" : ""}`)}
        </div>
      </div>
      <div class="translation-review-status"><p class="field-note">${current.contextualBlocks
        ? t(`Se usó contexto en ${current.contextualBlocks} bloques.`, `Context used in ${current.contextualBlocks} blocks.`)
        : t("Los segmentos se tradujeron por separado.", "Cues were translated individually.")}</p>
        <div class="translation-warning-nav"><span id="translation-warning-count" role="status">${warningCountLabel()}</span>
          ${action("translation-next-warning", t("Siguiente aviso", "Next warning"), "arrow", "quiet", `id="translation-next-warning" ${busy || !warnings.size ? "disabled" : ""}`)}</div></div>
      ${dubIssues.size ? `<div class="translation-warning-nav translation-dub-nav"><span>${dubIssues.size === 1 ? t("1 aviso de doblaje", "1 dubbing warning") : t(`${dubIssues.size} avisos de doblaje`, `${dubIssues.size} dubbing warnings`)}</span>
        ${action("translation-next-dub-warning", t("Siguiente aviso de doblaje", "Next dubbing warning"), "arrow", "quiet", busy ? "disabled" : "")}</div>` : ""}
      <ol class="translation-cues">${rows}</ol>
      ${current.original.length > pageSize ? `<div class="translation-pagination">
        ${action("translation-prev", t("Anterior", "Previous"), undefined, "quiet", offset === 0 ? "disabled" : "")}
        <span>${offset + 1}–${Math.min(offset + pageSize, current.original.length)} / ${current.original.length}</span>
        ${action("translation-next", t("Siguiente", "Next"), undefined, "quiet", offset + pageSize >= current.original.length ? "disabled" : "")}
      </div>` : ""}
      ${confirmRemoval ? `<div class="translation-confirm"><p>${t("¿Eliminar este borrador y sus ediciones?", "Delete this draft and its edits?")}</p>
        <div class="button-group">${action("translation-keep", t("Conservar", "Keep"), undefined, "quiet")}
        ${action("translation-remove", t("Eliminar borrador", "Delete draft"), "close", "secondary")}</div></div>` : ""}
    </section>`;
  }

  function exportForm(current: Draft) {
    const snapshot = hooks.snapshot();
    const exported = snapshot.jobs.filter(job => job.source.kind === "captionTranslationExport"
      && job.source.options.draftId === current.id && job.source.options.revision === current.revision
      && job.source.options.format === format && job.source.options.bilingual === bilingual)
      .sort((left, right) => right.createdAt - left.createdAt)
      .map(job => publishedTranslationFile(job, snapshot.library)).find(Boolean);
    return `<form id="translation-export">
      <div class="translation-fields">
        <label for="translation-format">${t("Formato", "Format")}<select id="translation-format">
          ${option("vtt", "VTT", format)}${option("srt", "SRT", format)}</select></label>
        <label class="toggle"><input id="translation-bilingual" type="checkbox" ${bilingual ? "checked" : ""}>
          ${t("Original y traducción", "Original and translation")}</label>
      </div>
      ${format === "srt" ? `<p class="field-note">${t("SRT numera los segmentos. Elige VTT para conservar identificadores con letras.", "SRT numbers cues. Choose VTT to keep text identifiers.")}</p>` : ""}
      <div class="asset-destination"><span>${t("Destino", "Destination")}<span class="path">${h(hooks.snapshot().settings.folder)}</span></span>
        ${action("choose-folder", t("Cambiar carpeta", "Change folder"), "folder", "quiet", busy ? "disabled" : "")}</div>
      <div class="asset-submit">
        ${action("translation-confirm-remove", t("Retirar borrador", "Remove draft"), undefined, "quiet", busy || edits.size ? "disabled" : "")}
        <button id="translation-export-button" class="button primary" type="submit" ${busy || edits.size ? "disabled" : ""}>${t("Exportar subtítulos", "Export subtitles")}</button>
      </div>
    </form>${exported ? translationOutput(exported, t("Última exportación", "Latest export"), t) : ""}`;
  }

  function draftPicker(drafts: Job[], label: string) {
    return `<label class="translation-saved" for="translation-draft">${label}
      <select id="translation-draft" ${busy || edits.size ? "disabled" : ""}>${option("", t("Selecciona un borrador", "Select a draft"), draft?.id ?? "")}
      ${drafts.map(job => option(job.id, job.title, draft?.id ?? "")).join("")}</select></label>`;
  }

  function render(mode: Mode) {
    const drafts = hooks.snapshot().jobs.filter(job => draftIds?.includes(job.id));
    const problem = error ? `<p role="alert" class="asset-error">${h(error)}</p>` : "";
    if (mode === "dubbing") {
      const edit = action("translation-edit-draft", t("Editar traducción", "Edit translation"), "document", "quiet");
      const status = draftIds === null ? (error ? "" : `<p class="field-note" role="status">${t("Buscando borradores…", "Finding drafts…")}</p>`)
        : drafts.length ? "" : `<div class="dubbing-empty"><p>${t("Traduce una pista local para crear el primer borrador.", "Translate a local track to create your first draft.")}</p>
          ${action("translation-edit-draft", t("Traducir subtítulos", "Translate subtitles"), "document", "secondary")}</div>`;
      return `<section class="tool-section image-assets-panel translation-tool dubbing-tool">
        <header class="asset-panel-heading"><span>${icon("audio")}</span><div><h2>${t("Crear doblaje", "Create dubbing")}</h2>
          <p>${t("Genera una voz local con los tiempos de la traducción y guárdala como audio o en una copia del vídeo.", "Generate a local voice using the translation timing and save it as audio or in a video copy.")}</p></div></header>
        ${drafts.length ? `<div class="dubbing-source">${draftPicker(drafts, t("Traducción guardada", "Saved translation"))}${draft ? edit : ""}</div>` : ""}
        ${status}${problem}
        ${draft ? `<p class="field-note dubbing-source-detail">${h(directionLabel(draft.selection.pair))} · ${draft.original.length} ${draft.original.length === 1 ? t("segmento", "cue") : t("segmentos", "cues")}</p>
          ${edits.size ? `<p class="dubbing-unsaved" role="status">${t("Guarda la edición para generar la voz con esos cambios.", "Save the edits to generate the voice with those changes.")}</p>` : ""}
          ${dubbing.render()}` : ""}
      </section>`;
    }
    return `<section class="tool-section image-assets-panel translation-tool">
      <header class="asset-panel-heading"><span>${icon("document")}</span><div><h2>${t("Traducir subtítulos", "Translate subtitles")}</h2>
        <p>${t("Traduce una pista local y revisa cada segmento junto al original.", "Translate a local track and review each cue alongside the original.")}</p></div></header>
      ${setup()}
      ${drafts.length ? draftPicker(drafts, t("Borradores guardados", "Saved drafts")) : ""}
      ${problem}
      ${draft ? comparison(draft) + exportForm(draft) : ""}
    </section>`;
  }

  async function click(kind: string, button: HTMLElement) {
    if (kind === "dub-open") {
      if (!busy && !edits.size) await hooks.openTool("dubbing");
      return true;
    }
    if (kind.startsWith("dub-")) return dubbing.click(kind, button);
    switch (kind) {
      case "translation-edit-draft": await hooks.openTool("translation"); return true;
      case "translation-components": hooks.components(); return true;
      case "translation-inspect": {
        const item = source();
        if (item) await run(async () => { sourcePreview = await hooks.command<Preview>("translation_source", { input: item.input }); });
        return true;
      }
      case "translation-save":
        if (draft && edits.size) await run(async () => {
          draft = await hooks.command<Draft>("translation_edit", {
            draftId: draft?.id, revision: draft?.revision, edits: Array.from(edits, ([index, text]) => ({ index, text })),
          });
          edits.clear(); updateWarnings(draft);
          hooks.notify(t("Cambios guardados en el borrador.", "Changes saved to the draft."));
        });
        return true;
      case "translation-cancel":
        if (workingId) await run(async () => {
          await hooks.command("job_action", { ids: [workingId], action: "cancel" });
          await hooks.refresh();
        });
        return true;
      case "translation-prev": offset = Math.max(0, offset - pageSize); warningCursor = null; hooks.render(); return true;
      case "translation-next": offset += pageSize; warningCursor = null; hooks.render(); return true;
      case "translation-next-warning":
      case "translation-next-dub-warning": {
        const ordered = (kind === "translation-next-warning" ? [...warnings] : dubbing.timingIssues()).sort((a, b) => a - b);
        if (busy || !ordered.length) return true;
        const index = ordered.find(index => index > (warningCursor ?? offset - 1)) ?? ordered[0];
        warningCursor = index;
        offset = Math.floor(index / pageSize) * pageSize;
        hooks.render();
        document.getElementById(`translation-cue-${index}`)?.focus();
        return true;
      }
      case "translation-confirm-remove": confirmRemoval = true; hooks.render(); return true;
      case "translation-keep": confirmRemoval = false; hooks.render(); return true;
      case "translation-remove":
        if (draft && confirmRemoval) await run(async () => {
          const id = draft?.id;
          await hooks.command("translation_remove", { draftId: id });
          draftIds = draftIds?.filter(entry => entry !== id) ?? []; clearDraft();
        });
        return true;
      default: return false;
    }
  }

  function input(element: HTMLElement) {
    if (element.id === "translation-search" && element instanceof HTMLInputElement) {
      query = element.value; hooks.render(); return true;
    }
    if (!(element instanceof HTMLTextAreaElement) || element.dataset.translationIndex === undefined || !draft) return false;
    const index = Number(element.dataset.translationIndex);
    warningCursor = index;
    if (element.value === draft.translated[index]) edits.delete(index);
    else edits.set(index, element.value);
    const details = lengthWarning(element.value, draft.original[index]);
    if (details) warnings.add(index);
    else warnings.delete(index);
    const hint = document.getElementById(`translation-warning-${index}`);
    if (hint) {
      hint.hidden = !details;
      hint.textContent = details ? warning(details) : "";
    }
    for (const id of ["translation-save", "translation-export-button", "translation-draft", "translation-source", "dub-open"]) {
      const control = document.getElementById(id);
      if (control instanceof HTMLButtonElement || control instanceof HTMLSelectElement) {
        control.disabled = id === "translation-save" ? edits.size === 0 : edits.size > 0;
      }
    }
    const warningCount = document.getElementById("translation-warning-count");
    const label = warningCountLabel();
    if (warningCount && warningCount.textContent !== label) warningCount.textContent = label;
    const nextWarning = document.getElementById("translation-next-warning");
    if (nextWarning instanceof HTMLButtonElement) nextWarning.disabled = busy || !warnings.size;
    return true;
  }

  async function change(element: HTMLElement) {
    if (dubbing.change(element)) return true;
    if (element instanceof HTMLSelectElement) {
      switch (element.id) {
        case "translation-source": {
          sourceKey = element.value; sourcePreview = null;
          const language = source()?.language.split("-")[0];
          const suggested = installed().find(value => value.startsWith(language + "-"));
          if (suggested) pair = suggested;
          break;
        }
        case "translation-pair": pair = element.value as Pair; break;
        case "translation-format": format = element.value as "srt" | "vtt"; break;
        case "translation-draft": {
          const focused = document.activeElement === element;
          if (!edits.size) {
            if (element.value) await run(() => open(element.value));
            else clearDraft();
          }
          if (focused && document.activeElement === document.body) {
            document.getElementById(element.id)?.focus({ preventScroll: true });
          }
          break;
        }
        default: return false;
      }
    } else if (element instanceof HTMLInputElement && element.id === "translation-bilingual") {
      bilingual = element.checked;
    } else return false;
    hooks.render();
    return true;
  }

  async function submit(form: HTMLFormElement) {
    if (form.id === "translation-setup") {
      const item = source();
      if (item && pair && sourcePreview && !active() && !edits.size) {
        const selection = { input: item.input, pair, sourceHash: sourcePreview.sourceHash };
        await run(async () => {
          const jobs = await hooks.command<Job[]>("queue_translation", { selection, clientRequest: crypto.randomUUID() });
          workingId = jobs[0].id;
          await hooks.refresh();
        });
        const current = working();
        if (current) observe(current, "translation");
      }
      return true;
    }
    if (form.id !== "translation-export") return false;
    if (draft && !edits.size) {
      const options = { draftId: draft.id, revision: draft.revision, format, bilingual };
      await run(async () => {
        await hooks.command("queue_translation_export", { options, clientRequest: crypto.randomUUID() });
        await hooks.refresh();
        hooks.notify(t("Exportación añadida a Descargas.", "Export added to Downloads."));
      });
    }
    return true;
  }

  function chooseTrack(libraryId: string, trackId: string) {
    if (edits.size) return;
    sourceKey = `${libraryId}/${trackId}`; sourcePreview = null;
  }

  async function openSaved(id: string) {
    if (edits.size) return;
    await run(() => open(id));
  }

  return { render, load, click, input, change, submit, observe, chooseTrack, openSaved, bind: dubbing.bind, leave: dubbing.stopSample };
}
