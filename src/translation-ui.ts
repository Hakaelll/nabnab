import "./translation.css";
import { escapeHtml as h, type Copy, type Job, type Snapshot } from "./contracts";
import { createDubbing } from "./dubbing-ui";
import { action, icon, option, stateLabel } from "./views-common";

type Pair = "es-en" | "en-es";
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
  t: Copy;
}

const directions: Pair[] = ["es-en", "en-es"];
const pageSize = 12;
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
  return Math.max(...counts) > 42 || counts.reduce((total, length) => total + length, 0) / (cue.end - cue.start) > 20;
}

export function createTranslationTool(hooks: Hooks) {
  const { t } = hooks;
  let components: Component[] | null = null;
  let pair: Pair | "" = "";
  let sourceKey = "", query = "", workingId = "";
  let sourcePreview: Preview | null = null;
  let draft: Draft | null = null;
  let draftIds: string[] = [];
  let busy = false, error = "", confirmRemoval = false;
  let offset = 0;
  let format: "srt" | "vtt" = "vtt", bilingual = false;
  const edits = new Map<number, string>();
  const warnings = new Set<number>();
  const dubbing = createDubbing({ ...hooks, hasEdits: () => edits.size > 0,
    draft: () => draft ? { id: draft.id, revision: draft.revision, pair: draft.selection.pair, sourceId: draft.selection.input.libraryId, count: draft.original.length } : null,
    focusCue: index => {
      offset = Math.floor(index / pageSize) * pageSize;
      hooks.render();
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
  const warning = () => t("Puede costar leerlo a tiempo. Prueba a acortar el texto o repartirlo en líneas.", "This cue may be hard to read in time. Try shortening it or splitting it into lines.");

  async function load() {
    try {
      [components, draftIds] = await Promise.all([
        hooks.command<Component[]>("optional_components"), hooks.command<string[]>("translation_drafts"),
      ]);
      const choices = installed();
      if (!pair || !choices.includes(pair)) pair = choices[0] ?? "";
      if (draft && !draftIds.includes(draft.id) && !edits.size) draft = null;
      const pending = hooks.snapshot().jobs.find(job => job.source.kind === "captionTranslation"
        && !["completed", "cancelled", "error"].includes(job.state));
      if (!workingId && pending) workingId = pending.id;
      const current = working();
      if (current) observe(current);
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
    dubbing.stopSample();
    draft = result; edits.clear(); offset = 0; confirmRemoval = false;
    updateWarnings(result);
    if (!draftIds.includes(id)) draftIds = [id, ...draftIds];
  }

  function updateWarnings(current: Draft) {
    warnings.clear();
    current.original.forEach((cue, index) => {
      if (lengthWarning(current.translated[index], cue)) warnings.add(index);
    });
  }

  function observe(job: Job) {
    dubbing.observe(job);
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
    const rows = current.original.slice(offset, offset + pageSize).map((cue, visibleIndex) => {
      const index = offset + visibleIndex;
      const target = edits.get(index) ?? current.translated[index];
      return `<li class="translation-cue">
        <div class="translation-time"><span>${h(cue.identifier ?? String(index + 1))}</span>
          <time>${timing(cue.start)} – ${timing(cue.end)}</time></div>
        <div class="translation-columns">
          <div class="translation-original"><span class="field-note">${t("Original", "Original")}</span>
            <p>${styled(cue.text)}</p></div>
          <label for="translation-cue-${index}"><span class="field-note">${t("Traducción", "Translation")}</span>
            <textarea id="translation-cue-${index}" data-translation-index="${index}" aria-label="${t("Traducción del segmento", "Translation of cue")} ${index + 1}"
              aria-describedby="translation-warning-${index}" rows="2" maxlength="16000" ${busy ? "disabled" : ""}>${h(target)}</textarea>
            <small id="translation-warning-${index}" class="translation-warning" ${warnings.has(index) ? "" : "hidden"}>${warning()}</small>
          </label>
        </div></li>`;
    }).join("");
    return `<section class="asset-review translation-review" aria-labelledby="translation-title">
      <div class="section-heading"><div><h3 id="translation-title">${t("Comparar y editar", "Compare and edit")}</h3>
        <p class="field-note">${h(current.name)} · ${directionLabel(current.selection.pair)} · ${t("Traducción automática", "Machine translation")}</p></div>
        ${action("translation-save", t("Guardar edición", "Save edits"), "check", "secondary", `id="translation-save" ${busy || !edits.size ? "disabled" : ""}`)}
      </div>
      <p class="field-note">${current.contextualBlocks
        ? t(`Se usó contexto en ${current.contextualBlocks} bloques.`, `Context used in ${current.contextualBlocks} blocks.`)
        : t("Los segmentos se tradujeron por separado.", "Cues were translated individually.")}
        <span id="translation-warning-count">${warnings.size} ${t("avisos de longitud", "length warnings")}</span></p>
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

  function exportForm() {
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
    </form>`;
  }

  function render() {
    const drafts = hooks.snapshot().jobs.filter(job => draftIds.includes(job.id));
    return `<section class="tool-section image-assets-panel translation-tool">
      <header class="asset-panel-heading"><span>${icon("document")}</span><div><h2>${t("Traducir subtítulos", "Translate subtitles")}</h2>
        <p>${t("Traduce una pista local y revisa cada segmento junto al original.", "Translate a local track and review each cue alongside the original.")}</p></div></header>
      ${setup()}
      ${drafts.length ? `<label class="translation-saved" for="translation-draft">${t("Borradores guardados", "Saved drafts")}
        <select id="translation-draft" ${busy || edits.size ? "disabled" : ""}>${option("", t("Selecciona un borrador", "Select a draft"), draft?.id ?? "")}
        ${drafts.map(job => option(job.id, job.title, draft?.id ?? "")).join("")}</select></label>` : ""}
      ${error ? `<p role="alert" class="asset-error">${h(error)}</p>` : ""}
      ${draft ? comparison(draft) + exportForm() + dubbing.render() : ""}
    </section>`;
  }

  async function click(kind: string) {
    if (kind.startsWith("dub-")) return dubbing.click(kind);
    switch (kind) {
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
      case "translation-prev": offset = Math.max(0, offset - pageSize); hooks.render(); return true;
      case "translation-next": offset += pageSize; hooks.render(); return true;
      case "translation-confirm-remove": confirmRemoval = true; hooks.render(); return true;
      case "translation-keep": confirmRemoval = false; hooks.render(); return true;
      case "translation-remove":
        if (draft && confirmRemoval) await run(async () => {
          const id = draft?.id;
          await hooks.command("translation_remove", { draftId: id });
          draftIds = draftIds.filter(entry => entry !== id); draft = null; edits.clear(); confirmRemoval = false;
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
    if (element.value === draft.translated[index]) edits.delete(index);
    else edits.set(index, element.value);
    dubbing.updateEdits();
    if (lengthWarning(element.value, draft.original[index])) warnings.add(index);
    else warnings.delete(index);
    const hint = document.getElementById(`translation-warning-${index}`);
    if (hint) hint.hidden = !warnings.has(index);
    for (const id of ["translation-save", "translation-export-button", "translation-draft", "translation-source"]) {
      const control = document.getElementById(id);
      if (control instanceof HTMLButtonElement || control instanceof HTMLSelectElement) {
        control.disabled = id === "translation-save" ? edits.size === 0 : edits.size > 0;
      }
    }
    const warningCount = document.getElementById("translation-warning-count");
    if (warningCount) warningCount.textContent = `${warnings.size} ${t("avisos de longitud", "length warnings")}`;
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
        case "translation-draft":
          if (!edits.size) {
            if (element.value) await run(() => open(element.value));
            else draft = null;
          }
          break;
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
        if (current) observe(current);
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
