import "./stems.css";
import { escapeHtml as h, type Copy, type Snapshot } from "./contracts";
import { bytesLabel, durationLabel, isCancelled } from "./presentation";
import { action, icon, option } from "./views-common";

interface Component { id: string; installed: boolean; enabled: boolean; available: boolean; size: number; installedSize?: number }
interface Preview { previewId: number; seconds: number; channels: number; originalChannels: number }
interface Hooks {
  snapshot(): Snapshot;
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  render(): void;
  refresh(): Promise<void>;
  notify(message: string): void;
  components(): void;
  compare(id: string, group: readonly string[]): void;
  playingId(): string | undefined;
  pausePlayback(): void;
  share(ids: string[]): void;
  t: Copy;
}

export function createStemsTool(hooks: Hooks) {
  const { t } = hooks;
  let component: Component | null = null;
  let libraryId = "", query = "", start = 0;
  let preview: Preview | null = null;
  let busy = false, error = "", requestId = 0, revision = 0;
  let context: AudioContext | null = null;
  let buffers: AudioBuffer[] = [], gains: GainNode[] = [], nodes: AudioBufferSourceNode[] = [];
  let chosen = 0, position = 0, began = 0, frame = 0;
  const labels = () => [t("Original", "Original"), t("Voz", "Vocals"), t("Instrumental", "Instrumental")];
  const sources = () => hooks.snapshot().library.filter(item => !item.missing && (item.kind === "audio" || item.kind === "video"));
  const selected = () => sources().find(item => item.id === libraryId);
  const ready = () => component?.installed && component.enabled && component.available;
  const disabled = () => busy ? "disabled" : "";

  function publishedSet() {
    if (!libraryId) return;
    const { jobs, library } = hooks.snapshot();
    const job = jobs.filter(job => job.source.kind === "stems" && job.source.selection.libraryId === libraryId && job.state === "completed")
      .sort((a, b) => b.createdAt - a.createdAt)[0];
    if (!job) return;
    const files = (["Vocals", "Instrumental"] as const).flatMap(role => {
      const item = library.find(item => !item.missing && item.kind === "audio" && item.parentId === libraryId
        && item.details?.stems?.setId === job.id && item.details.stems.role === role && job.outputs.includes(item.path));
      return item ? [{ role, item }] : [];
    });
    return files;
  }

  function resultGroup() {
    const source = selected();
    return [...(source ? [source.id] : []), ...(publishedSet()?.map(({ item }) => item.id) ?? [])];
  }

  function results() {
    const files = publishedSet();
    if (!files?.length) return "";
    const current = hooks.playingId();
    const listen = (id: string, label: string) => action("stems-compare", label, "audio", current === id ? "secondary" : "quiet",
      `id="stems-listen-${h(id)}" data-id="${h(id)}" aria-pressed="${current === id}"`);
    const source = selected();
    const [original, vocals, instrumental] = labels();
    return `<section class="asset-review stems-output" aria-labelledby="stems-output-title">
      <div class="section-heading"><h3 id="stems-output-title">${t("Pistas completas", "Full-length stems")}</h3><span>FLAC · ${bytesLabel(files.reduce((total, { item }) => total + item.size, 0))}</span></div>
      <div class="button-group stems-tracks" role="group" aria-label="${t("Comparar pistas completas", "Compare full-length stems")}">
        ${source ? listen(source.id, original) : ""}${files.map(({ role, item }) => listen(item.id, role === "Vocals" ? vocals : instrumental)).join("")}
      </div>
      <p class="field-note">${t("Cambia de pista en el mismo punto. Usa la barra inferior para pausar o buscar.", "Switch tracks at the same position. Use the player below to pause or seek.")}</p>
      ${files.length === 1 ? `<p class="field-note">${t("Solo queda una pista disponible de esta exportación.", "One stem from this export is available.")}</p>` : ""}
      <div class="button-group stems-tracks">${action("stems-share", files.length === 2 ? t("Enviar las dos pistas", "Send both stems")
        : t("Enviar pista disponible", "Send available stem"), "arrow", "secondary")}</div>
    </section>`;
  }

  async function load() {
    try { component = (await hooks.command<Component[]>("optional_components")).find(entry => entry.id === "stems") ?? null; }
    catch (cause) { error = String(cause); }
    hooks.render();
  }

  function clock() {
    if (!context || !nodes.length) return position;
    return Math.min(preview?.seconds ?? 0, Math.max(0, context.currentTime - began));
  }

  function drawTime() {
    const time = clock();
    const range = document.getElementById("stems-position");
    if (range instanceof HTMLInputElement) range.value = String(time);
    const label = document.getElementById("stems-position-label");
    if (label) label.textContent = durationLabel(time);
    if (nodes.length) frame = requestAnimationFrame(drawTime);
  }

  function stop() {
    position = clock();
    cancelAnimationFrame(frame);
    for (const node of nodes) {
      node.onended = null;
      node.stop();
      node.disconnect();
    }
    for (const gain of gains) gain.disconnect();
    nodes = []; gains = [];
  }

  function play() {
    if (!context || !preview || !buffers.length) return;
    if (position >= preview.seconds - 0.01) position = 0;
    const audio = context;
    const when = audio.currentTime + 0.02;
    began = when - position;
    gains = buffers.map((_, index) => {
      const gain = audio.createGain();
      gain.gain.value = index === chosen ? 1 : 0;
      gain.connect(audio.destination);
      return gain;
    });
    nodes = buffers.map((buffer, index) => {
      const node = audio.createBufferSource();
      node.buffer = buffer;
      node.connect(gains[index]);
      node.start(when, position);
      return node;
    });
    nodes[0].onended = () => { stop(); hooks.render(); };
    drawTime();
  }

  function discard() {
    stop();
    if (preview) void hooks.command("stems_release", { previewId: preview.previewId }).catch(() => {});
    if (context) void context.close();
    preview = null; context = null; buffers = []; position = 0; chosen = 0;
  }

  function leave() {
    revision++;
    if (requestId) void hooks.command("cancel_file_source_inspection", { requestId }).catch(() => {});
    busy = false; requestId = 0; error = "";
    discard();
  }

  function trackButtons() {
    return labels().map((label, index) => action("stems-track", label, "audio", chosen === index ? "secondary" : "quiet",
      `data-index="${index}" aria-pressed="${chosen === index}"`)).join("");
  }

  function review() {
    if (!preview) return `<p class="field-note stems-empty">${t("Genera una muestra de hasta 8 segundos para comparar la mezcla y las pistas.", "Generate a sample of up to 8 seconds to compare the mix and the stems.")}</p>`;
    return `<div class="stems-review">
      <div class="button-group stems-tracks" role="group" aria-label="${t("Pista de escucha", "Track to listen to")}">${trackButtons()}</div>
      <div class="stems-transport">
        ${action("stems-play", nodes.length ? t("Pausar", "Pause") : t("Reproducir", "Play"), nodes.length ? "pause" : "play", "secondary", 'id="stems-play"')}
        <label class="stems-timeline"><span>${t("Posición en la muestra", "Sample position")}</span>
          <input id="stems-position" type="range" min="0" max="${preview.seconds}" step="0.01" value="${clock()}"></label>
        <span class="stems-time"><span id="stems-position-label">${durationLabel(clock())}</span> / ${durationLabel(preview.seconds)}</span>
      </div>
      <p class="field-note">${t("Las pistas comparten la misma posición. Puedes cambiar de pista mientras escuchas.", "Tracks share one position. Switch between them while listening.")}</p>
      ${preview.originalChannels > preview.channels ? `<p class="field-note">${t("La entrada multicanal se mezcla a estéreo.", "The multichannel input is downmixed to stereo.")}</p>` : ""}
    </div>`;
  }

  function render() {
    const matches = sources().filter(item => item.id === libraryId || item.name.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
    const mib = (bytes: number) => `${new Intl.NumberFormat(t("es", "en"), { maximumFractionDigits: 1 }).format(bytes / 1048576)} MiB`;
    const status = ready() ? "" : `<div class="upscale-component"><div><strong>Spleeter · sherpa-onnx</strong>
      <p class="field-note">${t("Instala y activa el componente en Ajustes. El audio se procesa en CPU en este equipo.", "Install and enable the component in Settings. Audio is processed on this device’s CPU.")}</p>
      ${component ? `<small>${mib(component.size)} ${t("de descarga", "download")} · ${mib(component.installedSize ?? 0)} ${t("en disco", "on disk")}</small>` : ""}</div>
      ${action("stems-component", t("Ver componentes", "View components"), "settings", "secondary")}</div>`;
    return `<section class="tool-section image-assets-panel stems-tool">
      <header class="asset-panel-heading"><span>${icon("audio")}</span><div><h2>${t("Separar pistas", "Separate stems")}</h2>
        <p>${t("Escucha voz e instrumental por separado antes de exportar.", "Listen to vocals and instrumental separately before exporting.")}</p></div></header>
      ${status}<form id="stems-form">
        <fieldset class="stems-fields" ${disabled()}><legend>${t("Entrada", "Input")}</legend>
          <label for="stems-query">${t("Buscar en Biblioteca", "Find in Library")}
            <input id="stems-query" type="search" value="${h(query)}"></label>
          <label for="stems-source">${t("Audio o vídeo", "Audio or video")}<select id="stems-source">
            ${option("", t("Selecciona un archivo", "Select a file"), libraryId)}${matches.map(item => option(item.id, item.name, libraryId)).join("")}
          </select></label>
          ${action("import-files", t("Importar", "Import"), "add", "quiet")}
        </fieldset>
        <section class="asset-review" aria-labelledby="stems-review-title">
          <div class="section-heading"><h3 id="stems-review-title">${t("Muestra sincronizada", "Synchronized sample")}</h3></div>
          <div class="stems-sample-controls">
            <label for="stems-start">${t("Inicio (segundos)", "Start (seconds)")}<input id="stems-start" type="number" min="0" max="1800" step="0.1" required value="${Number.isFinite(start) ? start : ""}" ${disabled()}></label>
            ${action("stems-preview", t("Generar muestra", "Generate sample"), "audio", "secondary", !ready() || !selected() || busy ? "disabled" : "")}
            ${requestId ? action("stems-cancel", t("Cancelar muestra", "Cancel sample"), "close", "quiet") : ""}
          </div>
          ${review()}
          ${busy ? `<p role="status" class="field-note">${requestId ? t("Separando la muestra en este equipo…", "Separating the sample on this device…") : t("Añadiendo a la cola…", "Adding to the queue…")}</p>` : ""}
          ${error ? `<p role="alert" class="asset-error">${h(error)}</p>` : ""}
        </section>
        <div class="asset-destination"><span>${t("Destino", "Destination")}<span class="path">${h(hooks.snapshot().settings.folder)}</span></span>
          ${action("choose-folder", t("Cambiar carpeta", "Change folder"), "folder", "quiet", disabled())}</div>
        <div class="asset-submit"><p class="field-note">${t("Dos pistas FLAC vinculadas al original · hasta 30 min por archivo. La separación es estimada y puede dejar filtraciones o artefactos.", "Two FLAC stems linked to the original · up to 30 min per file. Separation is estimated and may leave bleed or artifacts.")}</p>
          <button type="submit" class="button primary" ${!preview || busy || !ready() || !selected() ? "disabled" : ""}>${t("Exportar pistas", "Export stems")}</button></div>
      </form>${results()}
    </section>`;
  }

  async function run(task: (revision: number) => Promise<void>) {
    if (busy) return;
    const active = ++revision;
    busy = true; error = ""; hooks.render();
    try { await task(active); }
    catch (cause) {
      if (revision === active && !isCancelled(cause)) error = String(cause).slice(0, 500);
    } finally {
      if (revision === active) { busy = false; requestId = 0; hooks.render(); }
    }
  }

  async function click(kind: string, button: HTMLElement) {
    switch (kind) {
      case "stems-component": hooks.components(); return true;
      case "stems-preview": {
        if (!ready() || !selected()) return true;
        const field = document.getElementById("stems-start");
        if (!(field instanceof HTMLInputElement) || !field.reportValidity()) return true;
        await run(async active => {
          discard(); requestId = Date.now() * 1000 + 719;
          const id = requestId;
          hooks.render();
          const prepared = await hooks.command<Preview>("stems_preview", { libraryId, start, requestId: id });
          const audio = new AudioContext({ sampleRate: 44100 });
          try {
            const samples = await Promise.all(labels().map(async (_, track) =>
              audio.decodeAudioData(await hooks.command<ArrayBuffer>("stems_sample", { previewId: prepared.previewId, track }))));
            if (revision !== active) return;
            context = audio; buffers = samples; preview = prepared;
          } finally {
            if (context !== audio) {
              void audio.close();
              void hooks.command("stems_release", { previewId: prepared.previewId }).catch(() => {});
            }
          }
        });
        return true;
      }
      case "stems-cancel":
        if (requestId) await hooks.command("cancel_file_source_inspection", { requestId });
        return true;
      case "stems-track":
        chosen = Number(button.dataset.index);
        for (const [index, gain] of gains.entries()) gain.gain.value = index === chosen ? 1 : 0;
        hooks.render();
        document.querySelector<HTMLElement>(`[data-action="stems-track"][data-index="${chosen}"]`)?.focus({ preventScroll: true });
        return true;
      case "stems-play":
        if (nodes.length) stop();
        else if (context) {
          const current = hooks.playingId();
          if (current && resultGroup().includes(current)) hooks.pausePlayback();
          await context.resume(); play();
        }
        hooks.render();
        document.getElementById("stems-play")?.focus({ preventScroll: true });
        return true;
      case "stems-compare": {
        const id = button.dataset.id;
        const group = resultGroup();
        if (id && group.includes(id)) {
          const stoppedSample = nodes.length > 0;
          stop();
          hooks.compare(id, group);
          if (stoppedSample) hooks.render();
          document.getElementById(`stems-listen-${id}`)?.focus({ preventScroll: true });
        }
        return true;
      }
      case "stems-share": {
        const ids = publishedSet()?.map(({ item }) => item.id);
        if (ids?.length) hooks.share(ids);
        return true;
      }
      default: return false;
    }
  }

  function input(target: HTMLElement) {
    if (target.id === "stems-query" && target instanceof HTMLInputElement) {
      query = target.value; hooks.render(); return true;
    }
    if (target.id === "stems-position" && target instanceof HTMLInputElement) {
      const playing = nodes.length > 0;
      stop(); position = Number(target.value);
      if (playing) play();
      else drawTime();
      return true;
    }
    if (target.id === "stems-start" && target instanceof HTMLInputElement) {
      start = target.valueAsNumber;
      if (!preview && !error) return true;
    } else if (target.id === "stems-source" && target instanceof HTMLSelectElement) {
      libraryId = target.value; start = 0;
    } else return false;
    discard(); error = ""; hooks.render();
    return true;
  }

  async function submit(form: HTMLFormElement) {
    if (form.id !== "stems-form") return false;
    if (preview && selected() && ready()) {
      const previewId = preview.previewId;
      await run(async () => {
        await hooks.command("queue_stems", { previewId, clientRequest: crypto.randomUUID() });
        discard(); await hooks.refresh();
        hooks.notify(t("Separación añadida a Descargas.", "Separation added to Downloads."));
      });
    }
    return true;
  }

  return { render, load, leave, click, input, submit };
}
