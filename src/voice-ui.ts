import "./voice.css";
import { activeStates, escapeHtml as h, type Copy, type Job, type Snapshot } from "./contracts";
import { action, icon, option } from "./views-common";

export type VoiceInput = { kind: "text"; title: string; text: string } | { kind: "library"; id: string } | { kind: "note"; id: string; annotationId: string };
export interface VoiceSettings { voiceId: string; rate: number; volume: number }
interface InstalledVoice { id: string; name: string; language: string; enabled: boolean; fingerprint: string }
interface Status { available: boolean; enabled: boolean; engineVersion: string; voices: InstalledVoice[]; note: string }
export interface VoiceSegment { index: number; text: string; startChars: number; endChars: number; cached: boolean }
export interface VoicePlan { schemaVersion: 1; id: string; createdAt: number; text: string; textHash: string; sourceVersion: string; origin: { title: string; libraryId: string | null; kind: "text" | "library" | "note" }; settings: VoiceSettings; engineFingerprint: string; segments: VoiceSegment[]; warnings: string[] }
interface PlanStatus { planId: string; textHash: string; sourceVersion: string; engineFingerprint: string; active: boolean; completed: number; total: number; duration: number; bytes: number; segments: {index:number;cached:boolean;duration:number|null}[] }
interface Sample { wav: number[]; duration: number; segmentText: string; truncated: boolean }
interface Hooks { command<T>(name: string, args?: Record<string, unknown>): Promise<T>; snapshot(): Snapshot; refresh(): Promise<void>; render(): void; t: Copy; notify(message: string, kind?: "info" | "success" | "error"): void; settings?(): void; chooseFolder?(): void }
function failure(error: unknown): string { if (typeof error === "string") return error; if (error instanceof Error) return error.message; if (typeof error === "object" && error !== null && "message" in error && typeof error.message === "string") return error.message; return String(error); }
function duration(seconds: number): string { const value = Math.max(0, Math.round(seconds)); return `${Math.floor(value / 60)}:${String(value % 60).padStart(2, "0")}`; }
function audioBlob(bytes: number[]): Blob {
  if (bytes.length < 44 || bytes.length > 32 * 1024 * 1024 || bytes.some(value => !Number.isInteger(value) || value < 0 || value > 255) || String.fromCharCode(...bytes.slice(0, 4)) !== "RIFF" || String.fromCharCode(...bytes.slice(8, 12)) !== "WAVE") throw Error("The local voice engine did not return a bounded WAV segment.");
  return new Blob([new Uint8Array(bytes)], { type: "audio/wav" });
}
export function createVoice(hooks: Hooks) {
  const { t } = hooks;
  let status: Status | null = null, plan: VoicePlan | null = null, progress: PlanStatus | null = null;
  let inputKind: VoiceInput["kind"] = "text", title = "", text = "", libraryId = "", annotationId = "";
  let language = "", voiceId = "", rate = 0, volume = 100;
  let busy: "status" | "prepare" | "sample" | "queue" | "control" | "segment" | null = null, error = "";
  let counter = Date.now(), currentRequest = 0, poll: ReturnType<typeof setTimeout> | null = null, polling = false, visible = false;
  let player: HTMLAudioElement | null = null, audioUrl = "", playing: { kind: "sample"; text: string } | { kind: "segment"; index: number } | null = null;
  let sample: Sample | null = null, samplePlanId = "", continuous = true, listening = false, waitingIndex: number | null = null;
  let playerMessage = "";
  let segmentOffset = 0;
  const jobs = (): Job | undefined => {
    const matching = plan ? hooks.snapshot().jobs.filter(job => job.source.kind === "voice" && job.source.planId === plan?.id) : [];
    return matching.find(job => job.state !== "completed" && job.state !== "cancelled") ?? matching[0];
  };
  const destinationJob = (): Job | undefined => { const job = jobs(); return job && job.state !== "completed" && job.state !== "cancelled" ? job : undefined; };
  const eligible = () => hooks.snapshot().library.filter(item => !item.missing && (item.kind === "article" || Boolean(item.details?.ocrRef) || item.kind === "document" && /\.(pdf|epub|txt|md)$/i.test(item.name)));
  const noteItems = () => hooks.snapshot().library.filter(item => !item.missing && item.details?.webArchiveRef && item.details.articleAnnotations?.some(annotation => annotation.note.trim() && annotation.version === item.details?.webArchiveRef));
  function selectedInput(): VoiceInput | null {
    if (inputKind === "text") return text.trim() ? { kind: "text", title: title.trim() || t("Texto narrado", "Narrated text"), text } : null;
    if (inputKind === "library") return eligible().some(item => item.id === libraryId) ? { kind: "library", id: libraryId } : null;
    return noteItems().find(item => item.id === libraryId)?.details?.articleAnnotations?.some(annotation => annotation.id === annotationId && annotation.note.trim() && annotation.version === noteItems().find(item => item.id === libraryId)?.details?.webArchiveRef) ? { kind: "note", id: libraryId, annotationId } : null;
  }
  function ready() { return Boolean(status?.available && status.enabled && status.voices.some(voice => voice.id === voiceId && voice.enabled)); }
  function generating() { const job = jobs(); return Boolean(progress?.active || job && activeStates.includes(job.state)); }
  function stopListening() { listening = false; waitingIndex = null; player?.pause(); }
  function invalidate() {
    if (!plan) return; stopListening(); plan = null; progress = null; segmentOffset = 0; sample = null; samplePlanId = ""; playing = null; playerMessage = "";
    if (player) { player.removeAttribute("src"); player.load(); } if (audioUrl) { URL.revokeObjectURL(audioUrl); audioUrl = ""; }
  }
  async function perform(kind: NonNullable<typeof busy>, operation: (requestId: number) => Promise<void>) {
    if (busy) return; const request = ++counter; currentRequest = request; busy = kind; error = ""; hooks.render();
    try { await operation(request); } catch (value) { if (request === currentRequest) error = failure(value); }
    finally { if (request === currentRequest) { busy = null; hooks.render(); schedule(); } }
  }
  async function refreshPlan() {
    if (!plan || polling) return; const planId = plan.id; polling = true;
    try { const result = await hooks.command<PlanStatus>("voice_plan_status", { planId }); if (plan?.id !== planId) return; if (result.textHash !== plan.textHash || result.sourceVersion !== plan.sourceVersion || result.engineFingerprint !== plan.engineFingerprint) throw Error(t("La voz o la versión del texto cambió. Prepara la narración de nuevo.", "The voice or text version changed. Prepare the narration again.")); progress = result; if (waitingIndex !== null && result.segments.some(segment => segment.index === waitingIndex && segment.cached)) { const next = waitingIndex; waitingIndex = null; await playSegment(next, false); } }
    catch (value) { error = failure(value); }
    finally { polling = false; }
  }
  function schedule() {
    if (poll || !visible || !plan || !generating()) return;
    poll = setTimeout(() => { poll = null; void refreshPlan().then(() => { if (visible) hooks.render(); schedule(); }); }, 750);
  }
  async function open(source?: VoiceInput) {
    visible = true;
    if (source) { invalidate(); inputKind = source.kind; if (source.kind === "text") { title = source.title; text = source.text; } else { libraryId = source.id; annotationId = source.kind === "note" ? source.annotationId : ""; } }
    await perform("status", async () => { status = await hooks.command<Status>("voice_status"); const available = status.voices.filter(voice => voice.enabled); if (!available.some(voice => voice.id === voiceId)) { const preferred = available.find(voice => voice.language.toLowerCase().startsWith(t("es", "en"))) ?? available[0]; voiceId = preferred?.id ?? ""; language = preferred?.language ?? ""; } await refreshPlan(); });
  }
  function renderProgress(): string {
    if (!plan) return "";
    const job = jobs(), cached = progress?.completed ?? plan.segments.filter(segment => segment.cached).length;
    const total = progress?.total ?? plan.segments.length;
    const paused = job?.state === "paused", active = generating();
    return `<section class="voice-plan"><div class="voice-plan-heading"><div><span class="voice-eyebrow">${t("Versión preparada", "Prepared version")}</span><h3>${h(plan.origin.title)}</h3></div><span>${cached} / ${total} ${t("segmentos guardados", "saved segments")}</span></div><p class="field-note">${t("El texto y la configuración de voz quedan vinculados a esta preparación. Cambiarlos requiere otra preparación.", "Text and voice settings are bound to this preparation. Changing them requires a new preparation.")}</p>${plan.warnings.length ? `<ul class="voice-warnings">${plan.warnings.map(warning => `<li>${h(warning)}</li>`).join("")}</ul>` : ""}<div class="voice-generation-controls"><div><strong>${active ? t("Generando voz local", "Generating local speech") : paused ? t("Generación pausada", "Generation paused") : cached === total ? t("Segmentos preparados", "Segments prepared") : t("Generación preparada", "Ready to generate")}</strong><small>${progress?.bytes ? `${(progress.bytes / 1024 ** 2).toFixed(1)} MiB / 3 GiB · ` : ""}${progress?.duration ? `${duration(progress.duration)} ${t("de audio generado", "of generated audio")}` : t("La duración se muestra cuando existen segmentos de audio.", "Duration is shown when audio segments exist.")}</small></div><div class="button-group">${active && job ? action("voice-pause-generation", t("Pausar generación", "Pause generation"), "pause", "secondary", busy ? "disabled" : "") : ""}${job && (active || paused) ? action("voice-cancel-generation", t("Cancelar generación", "Cancel generation"), "close", "quiet", busy ? "disabled" : "") : ""}${paused ? action("voice-resume-generation", t("Reanudar generación", "Resume generation"), "play", "primary", busy ? "disabled" : "") : !active ? action("voice-queue", cached > 0 && cached < total ? t("Continuar desde segmentos guardados", "Continue from saved segments") : cached === total ? t("Guardar WAV completo", "Save complete WAV") : t("Generar voz en Descargas", "Generate speech in Downloads"), "audio", "primary", busy || !ready() ? "disabled" : "") : ""}</div></div>${job?.state === "error" ? `<p class="voice-error" role="alert">${h(job.error?.message ?? t("La generación se interrumpió. Los segmentos válidos se conservan.", "Generation was interrupted. Valid segments are kept."))}</p>` : ""}<div class="voice-listening-heading"><h3>${t("Escucha por segmentos", "Listen by segment")}</h3><label class="voice-check"><input id="voice-continuous" type="checkbox" ${continuous ? "checked" : ""}>${t("Continuar con el siguiente segmento", "Continue to the next segment")}</label></div><p class="field-note">${t("Puedes escuchar los segmentos listos mientras se genera el resto. El resaltado corresponde al segmento que está sonando.", "You can listen to ready segments while the rest are generated. Highlighting follows the segment currently playing.")}</p><div class="voice-segments" tabindex="0" aria-label="${t("Texto de los segmentos preparados", "Prepared segment text")}">${plan.segments.slice(segmentOffset, segmentOffset + 60).map(segment => {
      const cached = progress?.segments.find(value => value.index === segment.index)?.cached ?? segment.cached;
      const measured = progress?.segments.find(value => value.index === segment.index)?.duration;
      return `<section class="voice-segment ${playing?.kind === "segment" && playing.index === segment.index ? "is-current" : ""}" data-voice-segment="${segment.index}"><div><span>${t("Segmento", "Segment")} ${segment.index + 1}${measured ? ` · ${duration(measured)}` : ""}</span>${action("voice-listen-segment", cached ? t("Escuchar segmento ", "Listen to segment ") + (segment.index + 1) : t("Audio pendiente", "Audio pending"), "play", "voice-segment-play icon-button", `id="voice-listen-${segment.index}" data-index="${segment.index}" ${!cached || busy ? "disabled" : ""}`)}</div><p>${h(segment.text)}</p></section>`;
    }).join("")}</div>${plan.segments.length > 60 ? `<div class="voice-segment-pagination"><span>${segmentOffset + 1}–${Math.min(segmentOffset + 60, plan.segments.length)} / ${plan.segments.length}</span><div class="button-group">${action("voice-segment-page", t("Anterior", "Previous"), undefined, "quiet", `id="voice-segment-previous" data-direction="previous" ${segmentOffset === 0 ? "disabled" : ""}`)}${action("voice-segment-page", t("Siguiente", "Next"), undefined, "quiet", `id="voice-segment-next" data-direction="next" ${segmentOffset + 60 >= plan.segments.length ? "disabled" : ""}`)}</div></div>` : ""}</section>`;
  }
  function render(): string {
    const active = generating(), boundDestination = destinationJob(), available = status?.voices.filter(voice => voice.enabled) ?? [];
    const voices = available.filter(voice => !language || voice.language === language), languages = [...new Set(available.map(voice => voice.language))];
    const library = inputKind === "note" ? noteItems() : eligible(), notes = noteItems().find(item => item.id === libraryId)?.details?.articleAnnotations?.filter(annotation => annotation.note.trim() && annotation.version === noteItems().find(item => item.id === libraryId)?.details?.webArchiveRef) ?? [];
    return `<section class="voice-workspace tool-section" aria-labelledby="voice-title"><div class="voice-workspace-heading"><div><span class="voice-eyebrow">${t("Texto a voz local", "Local text to speech")}</span><h2 id="voice-title">${t("Escucha lo que has guardado", "Listen to what you have saved")}</h2><p>${t("Convierte texto en voz con las voces instaladas en Windows. La síntesis y los archivos permanecen en este equipo.", "Turn text into speech with voices installed in Windows. Synthesis and files stay on this computer.")}</p></div>${action("voice-refresh-status", t("Actualizar voces", "Refresh voices"), "retry", "quiet", busy ? "disabled" : "")}</div>${!status ? `<p class="field-note" role="status">${t("Consultando las voces locales…", "Checking local voices…")}</p>` : !status.enabled || !status.available ? `<div class="voice-unavailable"><strong>${!status.enabled ? t("Activa Voz local para continuar", "Enable Local Voice to continue") : t("No hay voces locales disponibles", "No local voices are available")}</strong><p>${h(status.note)}</p>${hooks.settings ? action("voice-settings", t("Revisar componentes y voces", "Review components and voices"), "settings", "secondary") : ""}</div>` : `<p class="voice-engine-note">${t("Voces de Windows, disponibles sin conexión.", "Windows voices, available offline.")}</p><details class="voice-engine-details"><summary>${t("Información del motor", "Engine information")}</summary><p>${h(status.engineVersion)}</p><p>${h(status.note)}</p></details>`}${error ? `<p class="voice-error" role="alert">${h(error)}</p>` : ""}<form id="voice-prepare-form"><div class="voice-workbench"><div class="voice-input"><div class="voice-input-mode" role="group" aria-label="${t("Origen del texto", "Text source")}">${["text", "library", "note"].map(kind => `<button type="button" data-action="voice-source-kind" data-kind="${kind}" aria-pressed="${inputKind === kind}" ${busy || active ? "disabled" : ""}>${kind === "text" ? t("Texto", "Text") : kind === "library" ? t("Biblioteca", "Library") : t("Nota", "Note")}</button>`).join("")}</div>${plan ? `<div class="voice-prepared-source"><strong>${h(plan.origin.title)}</strong><p>${h(plan.text.slice(0, 240))}${plan.text.length > 240 ? "…" : ""}</p><span>${plan.segments.length} ${t("segmentos de texto preparados", "text segments prepared")}</span>${action("voice-edit-source", t("Editar fuente", "Edit source"), "document", "quiet", active || busy ? "disabled" : "")}</div>` : inputKind === "text" ? `<label for="voice-text-title">${t("Título", "Title")}</label><input id="voice-text-title" maxlength="200" value="${h(title)}" placeholder="${t("Mi narración", "My narration")}" ${busy || active ? "disabled" : ""}><label for="voice-text">${t("Texto que quieres escuchar", "Text to listen to")}</label><textarea id="voice-text" rows="10" maxlength="1000000" placeholder="${t("Pega un artículo, escribe una nota o prepara un texto…", "Paste an article, write a note or prepare some text…")}" ${busy || active ? "disabled" : ""}>${h(text)}</textarea><p class="field-note">${t("El texto no se envía a un servicio de voz remoto.", "Text is not sent to a remote speech service.")}</p>` : `<label for="voice-library-source">${inputKind === "note" ? t("Artículo que contiene la nota", "Article containing the note") : t("Artículo, documento o texto OCR", "Article, document or OCR text")}</label><select id="voice-library-source" ${busy || active ? "disabled" : ""}>${option("", t("Elige un archivo de Biblioteca", "Choose a Library item"), libraryId)}${library.map(item => option(item.id, item.name, libraryId)).join("")}</select>${!library.length ? `<p class="field-note">${inputKind === "note" ? t("Añade una nota a una captura local para escucharla aquí.", "Add a note to a local capture to listen to it here.") : t("Importa un artículo, PDF, EPUB, TXT o Markdown. Los elementos con texto OCR también pueden prepararse.", "Import an article, PDF, EPUB, TXT or Markdown. Items with OCR text can also be prepared.")}</p>` : ""}${inputKind === "note" && libraryId ? `<label for="voice-note-source">${t("Nota de esta versión", "Note from this version")}</label><select id="voice-note-source" ${busy || active ? "disabled" : ""}>${option("", t("Elige una nota", "Choose a note"), annotationId)}${notes.map(annotation => option(annotation.id, annotation.note.slice(0, 120), annotationId)).join("")}</select>` : ""}<p class="field-note">${t("Se extrae el texto al preparar la narración. Una captura distinta conserva su propia versión y sus notas.", "Text is extracted when preparing narration. A different capture keeps its own version and notes.")}</p>`}</div><fieldset class="voice-configuration"><legend>${t("Voz y salida", "Voice and output")}</legend><label>${t("Idioma disponible", "Available language")}<select id="voice-language" ${busy || active || !available.length ? "disabled" : ""}>${option("", t("Todos los idiomas", "All languages"), language)}${languages.map(value => option(value, value, language)).join("")}</select></label><label>${t("Voz instalada", "Installed voice")}<select id="voice-voice" ${busy || active || !voices.length ? "disabled" : ""}>${option("", t("Elige una voz local", "Choose a local voice"), voiceId)}${voices.map(voice => option(voice.id, voice.name, voiceId)).join("")}</select></label><label>${t("Ritmo", "Rate")}<div class="voice-range"><input id="voice-rate" type="range" min="-10" max="10" step="1" value="${rate}" ${busy || active ? "disabled" : ""}><output id="voice-rate-value" for="voice-rate">${rate === 0 ? t("Normal", "Normal") : `${rate > 0 ? "+" : ""}${rate}`}</output></div></label><label>${t("Volumen de la voz", "Voice volume")}<div class="voice-range"><input id="voice-volume" type="range" min="0" max="100" step="5" value="${volume}" ${busy || active ? "disabled" : ""}><output id="voice-volume-value" for="voice-volume">${volume}%</output></div></label><p class="field-note">${t("El ritmo es el ajuste de la voz instalada; no representa una multiplicación exacta de velocidad.", "Rate is the installed voice's setting; it is not an exact speed multiplier.")}</p><p class="field-note">${t("Hasta 1 millón de caracteres y 4096 segmentos. Límite de audio de 3 GiB; la duración y el tamaño reales se muestran al generar.", "Up to 1 million characters and 4096 segments. Audio limit is 3 GiB; actual duration and size are shown during generation.")}</p><div class="voice-destination"><span>${t("Destino del WAV", "WAV destination")}</span><strong>${h(boundDestination?.folder ?? hooks.snapshot().settings.folder)}</strong>${hooks.chooseFolder ? action("voice-destination", t("Cambiar", "Change"), "folder", "quiet", busy || active || boundDestination ? "disabled" : "") : ""}${boundDestination ? `<p class="field-note">${t("Este trabajo conserva este destino al reanudarse. Puedes elegir otra carpeta al crear una narración nueva.", "This job keeps this destination when resumed. Choose another folder when creating a new narration.")}</p>` : ""}</div><button type="submit" class="button primary" ${busy || active || !ready() || !selectedInput() ? "disabled" : ""}>${icon("document")}${busy === "prepare" ? t("Preparando texto…", "Preparing text…") : t("Preparar narración", "Prepare narration")}</button>${busy === "prepare" || busy === "sample" ? action("voice-cancel-request", t("Cancelar preparación", "Cancel preparation"), "close", "quiet") : ""}</fieldset></div></form>${plan ? `<section class="voice-sample"><div><strong>${t("Prueba la voz antes de generar todo", "Try the voice before generating everything")}</strong><p>${sample && samplePlanId === plan.id ? `${duration(sample.duration)} · ${sample.truncated ? t("Muestra breve", "Short sample") : t("Muestra del texto", "Text sample")}` : t("Escucha una muestra breve con esta voz y configuración.", "Listen to a short sample with this voice and settings.")}</p></div>${action("voice-sample", busy === "sample" ? t("Preparando muestra…", "Preparing sample…") : t("Escuchar muestra", "Listen to sample"), "play", "secondary", busy || !ready() ? "disabled" : "")}</section><section class="voice-player" aria-label="${t("Reproductor de voz local", "Local speech player")}"><div class="voice-player-heading"><span id="voice-player-label" role="status">${h(playerMessage || t("Elige una muestra o un segmento listo para escuchar.", "Choose a sample or a ready segment to listen."))}</span>${playing ? action("voice-stop-listening", t("Detener escucha", "Stop listening"), "pause", "quiet") : ""}</div><div id="voice-player-slot"></div>${playing?.kind === "sample" ? `<p class="voice-sample-text">${h(playing.text)}</p>` : ""}</section>` : ""}${renderProgress()}<p class="voice-boundary">${t("Las voces disponibles dependen de Windows. No se usa Edge TTS, una API remota ni clonación de voz. La salida inicial es WAV; puedes convertirla después con las herramientas de audio existentes.", "Available voices depend on Windows. No Edge TTS, remote API or voice cloning is used. Initial output is WAV; existing audio tools can convert it afterwards.")}</p></section>`;
  }
  function change(element: HTMLElement): boolean {
    if (element instanceof HTMLInputElement && element.id === "voice-continuous") { continuous = element.checked; if (!continuous) waitingIndex = null; return true; }
    if (busy || generating()) return false;
    if (element instanceof HTMLTextAreaElement && element.id === "voice-text") { const hadPlan = Boolean(plan); text = element.value; invalidate(); if (hadPlan) hooks.render(); else updatePrepareButton(); return true; }
    if (element instanceof HTMLInputElement) {
      if (element.id === "voice-text-title") { const hadPlan = Boolean(plan); title = element.value; invalidate(); if (hadPlan) hooks.render(); return true; }
      if (element.id === "voice-rate" || element.id === "voice-volume") { if (element.id === "voice-rate") rate = Number(element.value); else volume = Number(element.value); invalidate(); hooks.render(); return true; }
    }
    if (element instanceof HTMLSelectElement) {
      if (element.id === "voice-language") { language = element.value; voiceId = status?.voices.find(voice => voice.enabled && (!language || voice.language === language))?.id ?? ""; invalidate(); hooks.render(); return true; }
      if (element.id === "voice-voice") { voiceId = element.value; invalidate(); hooks.render(); return true; }
      if (element.id === "voice-library-source") { libraryId = element.value; annotationId = ""; invalidate(); hooks.render(); return true; }
      if (element.id === "voice-note-source") { annotationId = element.value; invalidate(); hooks.render(); return true; }
    }
    return false;
  }
  function updatePrepareButton() { const button = document.querySelector<HTMLButtonElement>("#voice-prepare-form button[type='submit']"); if (button) button.disabled = Boolean(busy) || generating() || !ready() || !selectedInput(); }
  async function submit(form: HTMLFormElement): Promise<boolean> {
    if (form.id !== "voice-prepare-form") return false; const source = selectedInput(); if (!source || !ready() || generating() || !form.reportValidity()) return true;
    const settings: VoiceSettings = { voiceId, rate, volume };
    await perform("prepare", async requestId => { const result = await hooks.command<VoicePlan>("voice_prepare", { source, settings, requestId }); if (requestId !== currentRequest) return; stopListening(); plan = result; sample = null; samplePlanId = ""; playing = null; await refreshPlan(); }); return true;
  }
  function ensurePlayer() {
    if (player) return player;
    player = document.createElement("audio"); player.controls = true; player.preload = "metadata"; player.id = "voice-audio"; player.setAttribute("aria-label", t("Audio de voz local", "Local speech audio"));
    for (const event of ["playing", "pause", "ended"]) player.addEventListener(event, syncPlayback);
    player.addEventListener("error", () => { playerMessage = t("El audio local no se pudo reproducir. Inténtalo otra vez.", "The local audio could not be played. Try again."); syncPlayback(); });
    player.addEventListener("ended", () => { if (!listening || !continuous || playing?.kind !== "segment" || !plan) return; const next = playing.index + 1; if (next >= plan.segments.length) { listening = false; return; } if (progress?.segments.some(segment => segment.index === next && segment.cached)) void playSegment(next, false).catch(value => { error = failure(value); hooks.render(); }); else if (generating()) { waitingIndex = next; playerMessage = t("Esperando el siguiente segmento…", "Waiting for the next segment…"); syncPlayback(); schedule(); } });
    return player;
  }
  function syncPlayback() {
    const speaking = Boolean(player && !player.paused && !player.ended && playing?.kind === "segment");
    for (const element of document.querySelectorAll<HTMLElement>("[data-voice-segment]")) { const current = playing?.kind === "segment" && Number(element.dataset.voiceSegment) === playing.index; element.classList.toggle("is-current", current); element.classList.toggle("is-speaking", speaking && current); if (speaking && current) element.setAttribute("aria-current", "true"); else element.removeAttribute("aria-current"); }
    const label = document.getElementById("voice-player-label"); if (label) label.textContent = speaking && playing?.kind === "segment" ? `${t("Escuchando segmento", "Listening to segment")} ${playing.index + 1} / ${plan?.segments.length ?? 0}` : playerMessage;
  }
  async function playBytes(wav: number[]) {
    if (!visible) return;
    const next = URL.createObjectURL(audioBlob(wav)); const audio = ensurePlayer(); const previous = audioUrl; audioUrl = next; audio.src = next; if (previous) URL.revokeObjectURL(previous);
    hooks.render(); try { await audio.play(); } catch { playerMessage = t("Audio listo. Pulsa Reproducir para escucharlo.", "Audio ready. Press Play to listen."); syncPlayback(); }
  }
  async function playSegment(index: number, userAction: boolean) {
    if (!plan || !plan.segments.some(segment => segment.index === index)) return;
    const planId = plan.id; const wav = await hooks.command<number[]>("voice_segment_audio", { planId, index }); if (plan?.id !== planId || !visible || !userAction && !listening) return;
    if (userAction) listening = true; waitingIndex = null; playing = { kind: "segment", index }; segmentOffset = Math.floor(index / 60) * 60; playerMessage = `${t("Segmento", "Segment")} ${index + 1}`; await playBytes(wav);
  }
  async function click(name: string, button: HTMLButtonElement): Promise<boolean> {
    switch (name) {
      case "voice-refresh-status": await open(); break;
      case "voice-edit-source": if (!generating() && !busy) { invalidate(); hooks.render(); } break;
      case "voice-segment-page": if (plan) { segmentOffset = Math.max(0, Math.min(Math.floor((plan.segments.length - 1) / 60) * 60, segmentOffset + (button.dataset.direction === "next" ? 60 : -60))); hooks.render(); } break;
      case "voice-settings": hooks.settings?.(); break;
      case "voice-destination": if (!destinationJob()) hooks.chooseFolder?.(); break;
      case "voice-source-kind": if (!busy && !generating() && (button.dataset.kind === "text" || button.dataset.kind === "library" || button.dataset.kind === "note")) { invalidate(); inputKind = button.dataset.kind; libraryId = ""; annotationId = ""; hooks.render(); } break;
      case "voice-sample": if (plan) { const planId = plan.id; await perform("sample", async requestId => { const result = await hooks.command<Sample>("voice_sample", { planId, requestId }); if (requestId !== currentRequest || plan?.id !== planId) return; audioBlob(result.wav); stopListening(); sample = result; samplePlanId = planId; playing = { kind: "sample", text: result.segmentText }; playerMessage = t("Muestra breve de esta voz", "Short sample of this voice"); await playBytes(result.wav); }); } break;
      case "voice-queue": if (plan && !generating()) { const planId = plan.id; await perform("queue", async () => { await hooks.command("queue_voice", { planId, clientRequest: crypto.randomUUID() }); await hooks.refresh(); await refreshPlan(); hooks.notify(t("Narración añadida a Descargas. Los segmentos listos pueden escucharse aquí.", "Narration added to Downloads. Ready segments can be played here."), "success"); }); } break;
      case "voice-listen-segment": { const index = Number(button.dataset.index); if (Number.isInteger(index)) await perform("segment", async () => { await playSegment(index, true); }); } break;
      case "voice-stop-listening": stopListening(); playerMessage = t("Escucha detenida. La generación conserva su estado en Descargas.", "Listening stopped. Generation keeps its Downloads state."); hooks.render(); break;
      case "voice-pause-generation": { const job = jobs(); if (job) await perform("control", async () => { await hooks.command("voice_pause_job", { id: job.id }); await hooks.refresh(); await refreshPlan(); }); } break;
      case "voice-resume-generation": { const job = jobs(); if (job) await perform("control", async () => { await hooks.command("job_action", { ids: [job.id], action: "retry" }); await hooks.refresh(); await refreshPlan(); }); } break;
      case "voice-cancel-generation": { const job = jobs(); if (job) await perform("control", async () => { stopListening(); await hooks.command("job_action", { ids: [job.id], action: "cancel" }); await hooks.refresh(); await refreshPlan(); }); } break;
      case "voice-cancel-request": if (busy === "prepare" || busy === "sample") { const requestId = currentRequest; currentRequest = ++counter; busy = null; await hooks.command("cancel_voice_request", { requestId }); hooks.render(); } break;
      default: return false;
    } return true;
  }
  function bind() {
    const slot = document.getElementById("voice-player-slot");
    if (slot && player && player.parentElement !== slot) slot.append(player);
    visible = Boolean(document.querySelector(".voice-workspace")); syncPlayback(); schedule();
  }
  function deactivate() { visible = false; if (poll) { clearTimeout(poll); poll = null; } stopListening(); }
  function dispose() { deactivate(); player?.removeAttribute("src"); player?.load(); if (audioUrl) URL.revokeObjectURL(audioUrl); audioUrl = ""; player = null; }
  return { render, open, click, change, submit, bind, deactivate, dispose, refreshPlan };
}
