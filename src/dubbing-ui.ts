import { activeStates, escapeHtml as h, type Copy, type Job, type Snapshot } from "./contracts";
import { action, option, stateLabel } from "./views-common";
import { publishedTranslationFile, translationOutput } from "./translation-output";
import { audioBlob, type VoiceSample, type VoiceSettings, type VoiceStatus } from "./voice-ui";
import { isCancelled } from "./presentation";

export interface DubSelection { draftId: string; revision: number; settings: VoiceSettings }
export interface DubVideoOptions {
  voiceJobId: string;
  videoId: string;
  mode: "alternate" | "replace" | "mix" | "instrumental";
  originalAudioId?: string;
  instrumentalId?: string;
}
interface OriginalAudio { id: string; title: string | null; language: string | null; codec: string | null }
interface DraftContext { id: string; revision: number; pair: "es-en" | "en-es"; sourceId: string; count: number }
interface Hooks {
  draft(): DraftContext | null;
  hasEdits(): boolean;
  focusCue(index: number): Promise<void>;
  cueTiming(index: number): string;
  snapshot(): Snapshot;
  command<T>(name: string, args?: Record<string, unknown>): Promise<T>;
  render(): void;
  refresh(): Promise<void>;
  components(): void;
  t: Copy;
}

const pending = (job: Job | undefined) => job && (activeStates.includes(job.state) || job.state === "pending" || job.state === "paused");

export function createDubbing(hooks: Hooks) {
  const { t } = hooks;
  let status: VoiceStatus | null = null, busy = false, error = "";
  let voiceId = "", rate = 0, volume = 100, videoId = "", sampleCue = 0;
  let mode: DubVideoOptions["mode"] = "alternate";
  let audioReview: { videoId: string; tracks: OriginalAudio[] } | null = null;
  let originalAudioId = "";
  let instrumentalId = "";
  let player: HTMLAudioElement | null = null, sampleUrl = "", sampleText = "", requestId = 0;
  let observed = "";

  const voices = () => status?.voices.filter(voice => voice.enabled && voice.language.split("-")[0] === hooks.draft()?.pair.split("-")[1]) ?? [];
  const videos = () => hooks.snapshot().library.filter(item => item.kind === "video" && !item.missing);
  const videoChoice = () => videos().find(item => item.id === (videoId || hooks.draft()?.sourceId))?.id ?? "";
  const reviewedTracks = () => audioReview?.videoId === videoChoice() ? audioReview.tracks : undefined;
  const mixAudio = () => reviewedTracks()?.find(track => track.id === originalAudioId);

  function instrumentals() {
    const video = videoChoice();
    if (!video) return [];
    const { jobs, library } = hooks.snapshot();
    const sets = new Map(jobs.filter(job => job.source.kind === "stems"
      && job.source.selection.libraryId === video && job.state === "completed").map(job => [job.id, job.outputs]));
    return library.filter(item => {
      const stem = item.details?.stems;
      return !item.missing && item.kind === "audio" && item.parentId === video
        && stem?.role === "Instrumental" && sets.get(stem.setId)?.includes(item.path);
    });
  }

  const instrumentalChoice = () => instrumentals().find(item => item.id === instrumentalId);

  function selection(): DubSelection | null {
    const draft = hooks.draft();
    const voice = voices().find(value => value.id === voiceId) ?? voices()[0];
    return draft && voice && status?.enabled && status.available ? { draftId: draft.id, revision: draft.revision, settings: { voiceId: voice.id, rate, volume } } : null;
  }

  function matchingVoices() {
    const choice = selection();
    if (!choice) return [];
    return hooks.snapshot().jobs.filter(job => {
      if (job.source.kind !== "dubVoice") return false;
      const { draftId, revision, settings } = job.source.selection;
      return draftId === choice.draftId && revision === choice.revision && settings.voiceId === choice.settings.voiceId && settings.rate === rate && settings.volume === volume;
    }).sort((left, right) => right.createdAt - left.createdAt);
  }

  function completedVoice() {
    const { library } = hooks.snapshot();
    return matchingVoices().find(job => publishedTranslationFile(job, library));
  }

  function videoJob() {
    const wav = completedVoice();
    const track = mode === "mix" ? mixAudio() : undefined;
    const instrumental = mode === "instrumental" ? instrumentalChoice() : undefined;
    const first = reviewedTracks()?.[0];
    return hooks.snapshot().jobs.filter(job => {
      if (job.source.kind !== "dubVideo") return false;
      const options = job.source.options;
      return options.voiceJobId === wav?.id && options.videoId === videoChoice() && options.mode === mode
        && options.instrumentalId === instrumental?.id
        && (options.originalAudioId === track?.id || !options.originalAudioId && track?.id === first?.id
          && (job.state === "completed" || pending(job) && job.state !== "paused"));
    }).sort((left, right) => right.createdAt - left.createdAt)[0];
  }

  function timingIssues(job = matchingVoices()[0]): number[] {
    const error = job?.state === "error" ? job.error : null;
    if (error?.code !== "dubbing_timing" || !/^DUB_TIMING \d+(, \d+)*$/.test(error.technical)) return [];
    const count = hooks.draft()?.count ?? 0;
    const indices = error.technical.slice(11).split(", ").map(value => Number(value) - 1);
    return [...new Set(indices.filter(index => Number.isSafeInteger(index) && index >= 0 && index < count))];
  }

  function timingReview(issues: number[]): string {
    const heading = issues.length === 1 ? t("1 segmento no cabe", "1 cue does not fit")
      : t(`${issues.length} segmentos no caben`, `${issues.length} cues do not fit`);
    return `<section class="dub-result dub-timing" aria-labelledby="dub-timing-title">
      <div class="dub-timing-head"><h3 id="dub-timing-title">${heading}</h3><span class="field-note">WAV</span></div>
      <p class="field-note">${t("Abre un segmento para acortarlo en el editor.", "Open a cue to shorten it in the editor.")}</p>
      <ol>${issues.map(index => `<li>
        ${action("dub-edit-cue", `${t("Editar segmento", "Edit cue")} ${index + 1}`, "document", "quiet", `data-index="${index}" aria-describedby="dub-time-${index}" ${busy ? "disabled" : ""}`)}
        <time id="dub-time-${index}">${h(hooks.cueTiming(index))}</time>
      </li>`).join("")}</ol>
    </section>`;
  }

  function stopSample() {
    player?.pause(); player?.remove(); player = null;
    if (sampleUrl) URL.revokeObjectURL(sampleUrl);
    sampleUrl = ""; sampleText = "";
    if (requestId) {
      const id = requestId; requestId = 0;
      void hooks.command("cancel_voice_request", { requestId: id }).catch(() => {});
    }
  }

  function resetDraft() {
    stopSample();
    videoId = "";
    audioReview = null;
    originalAudioId = "";
    instrumentalId = "";
    sampleCue = 0;
    error = "";
  }

  function bind() {
    const slot = document.getElementById("dub-sample-slot");
    if (slot && player && player.parentElement !== slot) slot.append(player);
  }

  function originalAudioControls() {
    const tracks = reviewedTracks();
    const refresh = action("dub-review-audio", tracks ? t("Volver a revisar", "Review again") : t("Revisar pistas", "Review tracks"), "list", "quiet", 'id="dub-review-audio"');
    if (!tracks?.length) return `<div class="dub-mix-tracks">${refresh}<p class="field-note" role="status">${tracks
      ? t("Este vídeo no tiene pistas de audio.", "This video has no audio tracks.")
      : t("Revisa el vídeo para elegir el audio que se mezclará con la voz.", "Review the video to choose the audio to mix with the voice.")}</p></div>`;
    const labels = tracks.map((track, index) => [t(`Pista ${index + 1}`, `Track ${index + 1}`), track.title, track.language, track.codec?.toUpperCase()].filter(Boolean).join(" · "));
    return `<div class="dub-mix-tracks">
      <div class="dub-track-field"><label for="dub-original-audio">${t("Pista original", "Original track")}</label>
        <select id="dub-original-audio" title="${h(labels[tracks.findIndex(track => track.id === originalAudioId)] ?? "")}">
          ${tracks.length > 1 ? option("", t("Elige una pista de audio", "Choose an audio track"), originalAudioId) : ""}
          ${tracks.map((track, index) => option(track.id, labels[index], originalAudioId)).join("")}
        </select></div>${refresh}
    </div>`;
  }

  function instrumentalControls() {
    const tracks = instrumentals();
    const selected = tracks.find(item => item.id === instrumentalId);
    return `<div class="dub-mix-tracks">
      ${tracks.length ? `<div class="dub-track-field"><label for="dub-instrumental">${t("Instrumental de Biblioteca", "Library instrumental")}</label>
        <select id="dub-instrumental">
          ${option("", t("Elige una exportación instrumental", "Choose an instrumental export"), selected?.id ?? "")}
          ${tracks.map(item => option(item.id, item.name, selected?.id ?? "")).join("")}
        </select></div>
        ${selected ? action("play-media", t("Escuchar instrumental", "Play instrumental"), "play", "quiet", `data-id="${h(selected.id)}"`) : ""}
        <p class="field-note dub-instrumental-note">${t("Fondo al 20 % desde Separar pistas. La separación puede dejar restos de voz o artefactos.", "Background at 20% from Separate stems. Separation may leave vocal bleed or artifacts.")}</p>`
      : `<p class="field-note">${t("Exporta el instrumental de este vídeo desde Separar pistas.", "Export the instrumental for this video in Separate stems.")}</p>
        ${action("tool-module", t("Abrir Separar pistas", "Open Separate stems"), "arrow", "quiet", 'data-id="stems"')}`}
    </div>`;
  }

  function render() {
    const draft = hooks.draft();
    if (!draft) return "";
    const choice = selection();
    const job = matchingVoices()[0];
    const wav = completedVoice();
    const mux = videoJob();
    const { library } = hooks.snapshot();
    const audio = publishedTranslationFile(wav, library);
    const video = publishedTranslationFile(mux, library);
    const issues = timingIssues(job);
    const disabled = busy || hooks.hasEdits();
    const cue = Math.min(sampleCue, draft.count - 1);
    return `<section class="dub-expanded" aria-label="${t("Preparar la voz y el vídeo", "Prepare voice and video")}">
      ${!status ? `<p class="field-note" role="status">${busy ? t("Buscando voces locales…", "Finding local voices…") : t("Actualiza la lista de voces instaladas.", "Refresh installed voices.")}</p>` : !status.enabled || !status.available ? `<p class="field-note">${t("Activa Voz local en Ajustes y comprueba las voces de Windows.", "Enable Local voice in Settings and check Windows voices.")}</p>` : !choice ? `<p class="field-note">${t("Windows no tiene una voz local instalada para el idioma de destino.", "Windows has no installed local voice for the target language.")}</p>` : ""}
      ${!choice ? `<div class="button-group">${action("dub-settings", t("Ver componentes", "View components"), "settings", "secondary")}${action("dub-refresh", t("Actualizar voces", "Refresh voices"), "retry", "quiet", busy ? "disabled" : "")}</div>` : `
      <fieldset id="dub-controls" class="translation-fields" ${disabled ? "disabled" : ""}><legend>${t("1. Voz y muestra", "1. Voice and sample")}</legend>
        <label for="dub-voice">${t("Voz instalada", "Installed voice")}<select id="dub-voice">${voices().map(voice => option(voice.id, voice.name, choice.settings.voiceId)).join("")}</select></label>
        <label for="dub-rate">${t("Ritmo de voz", "Voice rate")}<select id="dub-rate">${[-3,-2,-1,0,1,2,3].map(value => option(String(value), value > 0 ? `+${value}` : String(value), String(rate))).join("")}</select></label>
        <label for="dub-volume">${t("Volumen (%)", "Volume (%)")}<input id="dub-volume" type="number" min="1" max="100" value="${volume}"></label>
        <label for="dub-cue">${t("Segmento de muestra", "Sample cue")}<input id="dub-cue" type="number" min="1" max="${draft.count}" value="${cue + 1}"></label>
        ${action("dub-sample", t("Escuchar muestra", "Play sample"), "play", "secondary")}
      </fieldset>
      ${sampleText ? `<div class="dub-sample"><span>${h(sampleText)}</span><div id="dub-sample-slot"></div></div>` : ""}
      ${job && job.id !== wav?.id ? issues.length ? timingReview(issues)
        : `<div class="dub-result" role="status"><span>${stateLabel(job.state, t)} · WAV</span>${job.error ? `<p>${h(job.error.message)}</p>` : ""}</div>` : ""}
      <div class="asset-destination dub-export"><span>${t("Destino", "Destination")}<span class="path field-note">${h(hooks.snapshot().settings.folder)}</span></span>${action("choose-folder", t("Cambiar carpeta", "Change folder"), "folder", "quiet", busy ? "disabled" : "")}
        ${audio ? "" : action("dub-queue-voice", t("Generar WAV sincronizado", "Generate synchronized WAV"), "audio", "primary", `id="dub-queue-voice" ${disabled || pending(job) ? "disabled" : ""}`)}</div>
      ${audio ? translationOutput(audio, t("Voz sincronizada · WAV", "Synchronized voice · WAV"), t, t("Escuchar WAV", "Play WAV")) : ""}
      ${wav ? `<fieldset id="dub-video-controls" class="translation-fields" ${disabled ? "disabled" : ""}><legend>${t("2. Copia de vídeo", "2. Video copy")}</legend>
        <label for="dub-video">${t("Vídeo de Biblioteca", "Library video")}<select id="dub-video">${option("", t("Selecciona el vídeo correspondiente", "Select the matching video"), videoChoice())}${videos().map(item => option(item.id, item.name, videoChoice())).join("")}</select></label>
        <label for="dub-mode">${t("Audio en la copia MKV", "Audio in the MKV copy")}<select id="dub-mode">${option("alternate", t("Pista alternativa", "Alternate track"), mode)}${option("replace", t("Solo voz traducida", "Translated voice only"), mode)}${option("mix", t("Voz + original al 20 %", "Voice + original at 20%"), mode)}${option("instrumental", t("Voz + instrumental", "Voice + instrumental"), mode)}</select></label>
        ${mode === "mix" && videoChoice() ? originalAudioControls() : ""}
        ${mode === "instrumental" && videoChoice() ? instrumentalControls() : ""}
        ${action("dub-queue-video", mux?.state === "error" || mux?.state === "cancelled" || mux?.state === "paused" ? t("Reintentar copia", "Retry copy") : t("Crear copia de vídeo", "Create video copy"), "video", "secondary", !videoChoice() || mode === "mix" && !mixAudio() || mode === "instrumental" && !instrumentalChoice() || pending(mux) && mux?.state !== "paused" ? "disabled" : "")}
      </fieldset>${video ? translationOutput(video, mode === "instrumental" ? t("Copia con instrumental · MKV", "Copy with instrumental · MKV") : t("Copia de vídeo · MKV", "Video copy · MKV"), t, t("Reproducir copia", "Play copy")) : mux ? `<p class="field-note" role="status">${stateLabel(mux.state, t)} · MKV ${mux.error ? `· ${h(mux.error.code === "dubbing_no_original"
        ? t("Este vídeo carece de audio original. El WAV sigue disponible.", "This video has no original audio. The WAV remains available.")
        : mux.error.code === "dubbing_audio_changed"
          ? t("La pista elegida cambió. Vuelve a revisar las pistas del vídeo. El WAV sigue disponible.", "The selected track changed. Review the video's tracks again. The WAV remains available.")
          : mux.error.code === "dubbing_instrumental_changed"
            ? t("El vídeo o el instrumental cambiaron. Exporta un nuevo instrumental para ese vídeo. El WAV sigue disponible.", "The video or instrumental changed. Export a fresh instrumental for this video. The WAV remains available.")
            : mux.error.message)}` : ""}</p>` : ""}` : ""}`}
      ${busy ? `<div class="button-group"><p class="field-note" role="status">${t("Preparando operación local…", "Preparing local operation…")}</p>
        ${requestId ? action("dub-cancel", t("Cancelar", "Cancel"), "close", "quiet") : ""}</div>` : ""}
      ${error ? `<p class="asset-error" role="alert">${h(error)}</p>` : ""}
      <p class="field-note">${t("El WAV respeta las pausas y avisa si una frase no cabe.", "The WAV preserves pauses and flags speech that does not fit.")}${mode === "mix" ? " " + t("La mezcla conserva el diálogo presente en la pista original.", "The mix keeps any dialogue in the original track.") : ""}</p>
    </section>`;
  }

  async function run(task: () => Promise<void>) {
    if (busy) return;
    busy = true; error = ""; hooks.render();
    try { await task(); }
    catch (cause) { if (!isCancelled(cause)) error = String(cause).slice(0, 600); }
    finally { busy = false; hooks.render(); }
  }

  async function load() {
    await run(async () => { status = await hooks.command<VoiceStatus>("voice_status"); });
  }

  async function click(actionName: string, button: HTMLElement): Promise<boolean> {
    if (actionName === "dub-refresh") {
      await load();
      requestAnimationFrame(() => document.getElementById("dub-voice")?.focus());
      return true;
    }
    if (actionName === "dub-settings") { hooks.components(); return true; }
    if (actionName === "dub-cancel") { stopSample(); hooks.render(); return true; }
    if (actionName === "dub-edit-cue") {
      const index = Number(button.dataset.index);
      if (!busy && timingIssues().includes(index)) await hooks.focusCue(index);
      return true;
    }
    if (actionName === "dub-review-audio") {
      const videoId = videoChoice();
      if (!videoId || mode !== "mix" || busy || hooks.hasEdits()) return true;
      const previousId = originalAudioId;
      await run(async () => {
        audioReview = null;
        originalAudioId = "";
        const id = Date.now() * 1000 + 918;
        requestId = id;
        hooks.render();
        try {
          const tracks = await hooks.command<OriginalAudio[]>("inspect_dub_video_audio", { videoId, requestId: id });
          if (requestId !== id || videoChoice() !== videoId) return;
          audioReview = { videoId, tracks };
          originalAudioId = tracks.some(track => track.id === previousId) ? previousId : tracks.length === 1 ? tracks[0].id : "";
        } finally { if (requestId === id) requestId = 0; }
      });
      (document.getElementById("dub-original-audio") ?? document.getElementById("dub-review-audio"))?.focus();
      return true;
    }
    if (actionName !== "dub-sample" && actionName !== "dub-queue-voice" && actionName !== "dub-queue-video") return false;
    const choice = selection(), draft = hooks.draft();
    if (!choice || !draft || hooks.hasEdits() || busy) return true;
    await run(async () => {
      if (actionName === "dub-sample") {
        stopSample();
        const id = Date.now() * 1000 + 917;
        requestId = id;
        hooks.render();
        try {
          const sample = await hooks.command<VoiceSample>("dub_sample", { selection: choice, index: Math.min(sampleCue, draft.count - 1), requestId: id });
          if (requestId !== id) return;
          sampleUrl = URL.createObjectURL(audioBlob(sample.wav)); sampleText = sample.segmentText;
          player = document.createElement("audio"); player.controls = true; player.src = sampleUrl;
          player.setAttribute("aria-label", t("Muestra de la voz traducida", "Translated voice sample"));
          hooks.render();
          requestAnimationFrame(() => { if (sampleUrl) void player?.play().catch(() => {}); });
        } finally { if (requestId === id) requestId = 0; }
      } else if (actionName === "dub-queue-voice") {
        await hooks.command("queue_dub_voice", { selection: choice, clientRequest: crypto.randomUUID() });
        await hooks.refresh();
      } else {
        const voiceJobId = completedVoice()?.id;
        const videoId = videoChoice(), previous = videoJob();
        const track = mode === "mix" ? mixAudio() : undefined;
        const instrumental = mode === "instrumental" ? instrumentalChoice() : undefined;
        if (!voiceJobId || !videoId || mode === "mix" && !track || mode === "instrumental" && !instrumental) return;
        if (previous && ["error", "cancelled", "paused"].includes(previous.state)) {
          await hooks.command("job_action", { ids: [previous.id], action: "retry" });
        } else {
          const options: DubVideoOptions = { voiceJobId, videoId, mode, ...(track ? { originalAudioId: track.id } : {}),
            ...(instrumental ? { instrumentalId: instrumental.id } : {}) };
          await hooks.command("queue_dub_video", { options, clientRequest: crypto.randomUUID() });
        }
        await hooks.refresh();
      }
    });
    return true;
  }

  function change(target: HTMLElement) {
    if (!(target instanceof HTMLSelectElement || target instanceof HTMLInputElement)) return false;
    switch (target.id) {
      case "dub-voice": voiceId = target.value; break;
      case "dub-rate": rate = Number(target.value); break;
      case "dub-volume": volume = Math.round(Math.max(1, Math.min(100, Number(target.value) || 100))); break;
      case "dub-cue": sampleCue = Math.max(0, Math.min((hooks.draft()?.count ?? 1) - 1, Math.floor(Number(target.value) || 1) - 1)); break;
      case "dub-video":
        videoId = target.value;
        audioReview = null;
        originalAudioId = "";
        instrumentalId = "";
        break;
      case "dub-mode": if (target.value === "replace" || target.value === "mix" || target.value === "alternate" || target.value === "instrumental") mode = target.value; break;
      case "dub-original-audio": originalAudioId = target.value; break;
      case "dub-instrumental": instrumentalId = target.value; break;
      default: return false;
    }
    stopSample(); hooks.render();
    requestAnimationFrame(() => document.getElementById(target.id)?.focus());
    return true;
  }

  function observe(job: Job) {
    if (job.source.kind !== "dubVoice" && job.source.kind !== "dubVideo") return;
    const change = `${job.id}:${job.state}`;
    if (change !== observed && (matchingVoices()[0]?.id === job.id || videoJob()?.id === job.id)) { observed = change; hooks.render(); }
  }

  return { bind, change, click, load, observe, render, resetDraft, stopSample, timingIssues };
}
