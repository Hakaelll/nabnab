import { durationLabel } from "./presentation";
import { escapeHtml as h, sourceUrls, type Copy, type HomeState, type Snapshot } from "./contracts";
import { action, icon, option, unknownDuration } from "./views-common";

export function homeView(home: HomeState, snapshot: Snapshot, ready: boolean, desktop: boolean, t: Copy): string {
  const parsed = sourceUrls(home.input);
  const media = home.media;
  const entries = media?.entries.length ? media.entries : parsed.urls.length > 1 ? parsed.urls.map(url => ({ id: url, url, title: new URL(url).hostname, thumbnail: null, duration: null })) : [];
  const batch = entries.length > 0;
  const canSubmit = ready && !home.loading && !home.submitting && !parsed.invalid && (batch ? home.selectedUrls.size > 0 : media !== null);
  return `<div class="home-identity"><canvas id="emblem" aria-hidden="true"></canvas><span>nabnab</span></div>
    <div class="home-workspace">
      <form id="source-form" autocomplete="off" aria-busy="${home.loading}" class="source-form ${home.loading ? "is-loading" : ""}">
        <label class="sr-only" for="url">${t("Enlaces", "Links")}</label>
        <div class="url-control"><span class="source-emblem" aria-hidden="true"></span><textarea id="url" rows="2" maxlength="24000" placeholder="${t("Pega un enlace o arrastra un archivo", "Paste a link or drop a file")}" spellcheck="false">${h(home.input)}</textarea><button type="submit" id="load" class="icon-button" title="${t("Analizar enlace", "Analyze link")}" aria-label="${t("Analizar enlace", "Analyze link")}" ${!desktop || home.loading ? "disabled" : ""}>${icon("arrow")}</button></div>
      </form>
      <span id="source-status" class="sr-only" role="status" aria-live="polite">${home.loading ? t("Analizando el enlace.", "Analyzing the link.") : ""}</span>
      <div class="source-actions">${action("paste", t("Pegar", "Paste"), "paste", "quiet", !desktop ? "disabled" : "")}${action("url-file", t("Importar enlaces .txt", "Import links .txt"), "add", "quiet", !desktop ? "disabled" : "")}<span>${t("Enter para analizar", "Enter to analyze")}</span></div>
      ${!desktop ? `<div class="notice">${icon("info")}<p>${t("Abre la aplicación de escritorio nabnab para descargar o trabajar con tus archivos.", "Open the nabnab desktop app to download or work with your files.")}</p></div>` : !ready ? `<div class="notice">${icon("info")}<div><strong>${t("Prepara los motores para empezar", "Prepare engines to get started")}</strong><p>${t("Puedes usar la biblioteca y revisar los ajustes mientras tanto.", "You can use the library and review settings in the meantime.")}</p>${action("prepare-engines", t("Revisar motores", "Review engines"), undefined, "quiet")}</div></div>` : ""}
      ${parsed.invalid ? `<p class="inline-error">${icon("error")}${t("Incluye solo enlaces HTTP o HTTPS completos, sin credenciales.", "Use complete HTTP or HTTPS links without credentials.")}</p>` : ""}
      ${parsed.truncated ? `<p class="inline-error">${t("El límite es de 100 enlaces por lote. Divide el archivo antes de continuar.", "The limit is 100 links per batch. Split the file before continuing.")}</p>` : ""}
      ${media || batch ? `<section class="source-preview" aria-label="${t("Contenido y opciones", "Content and options")}">
        ${batch ? `<div class="section-heading"><div><h2>${h(media?.title ?? t("Lote de enlaces", "Link batch"))}</h2><p>${entries.length} ${t("elementos", "items")} · ${t("Las calidades se comprueban al iniciar cada elemento.", "Qualities are checked when each item starts.")}</p></div><label class="check-label"><input id="select-all-sources" type="checkbox" ${home.selectedUrls.size === entries.length ? "checked" : ""}>${t("Todos", "All")}</label></div>
          <ul class="source-batch">${entries.map(entry => `<li><label><input type="checkbox" data-source-url="${h(entry.url)}" ${home.selectedUrls.has(entry.url) ? "checked" : ""}><span><strong>${h(entry.title)}</strong><small>${h(new URL(entry.url).hostname)}${entry.duration !== null ? " · " + durationLabel(entry.duration) : ""}</small></span></label></li>`).join("")}</ul>` : media ? `<div class="preview-heading">${media.thumbnail ? `<img class="preview-image" src="${h(media.thumbnail)}" alt="" referrerpolicy="no-referrer">` : `<div class="preview-placeholder">${icon(media.hasVideo ? "video" : "audio")}</div>`}<div><span class="eyebrow">${t("Vista previa", "Preview")}</span><h2 id="video-title">${h(media.title)}</h2><p>${h(media.uploader ?? t("Origen", "Source"))}</p><p class="metadata">${unknownDuration(media.duration, t)} · ${t("Tamaño desconocido", "Size unknown")}</p></div></div>` : ""}
        <div class="output-options"><label>${t("Perfil de salida", "Output profile")}<select id="output-format">${[
          ["mp4", t("Vídeo compatible · H.264 / AAC", "Compatible video · H.264 / AAC"), !batch && !media?.hasVideo],
          ["source", t("Calidad original · sin recodificar", "Original quality · no re-encoding"), !batch && !media?.hasVideo],
          ["mp3", t("Audio compatible · MP3", "Compatible audio · MP3"), !batch && !media?.hasAudio],
          ["m4a", t("Audio compatible · M4A / AAC", "Compatible audio · M4A / AAC"), !batch && !media?.hasAudio],
          ["audio", t("Audio original · códec de origen", "Original audio · source codec"), !batch && !media?.hasAudio],
        ].map(([value, label, disabled]) => `<option value="${value}" ${home.format === value ? "selected" : ""} ${disabled ? "disabled" : ""}>${label}</option>`).join("")}</select></label>
        ${!batch && media?.hasVideo && (home.format === "mp4" || home.format === "source") ? `<label>${t("Calidad", "Quality")}<select id="quality">${option("", t("Mejor disponible", "Best available"), home.quality)}${media.heights.map(height => option(String(height), `${height}p`, home.quality)).join("")}</select></label>` : home.format === "mp3" ? `<label>${t("Bitrate", "Bitrate")}<select id="audio-bitrate">${[128,192,256,320].map(rate => option(String(rate), `${rate} kbps`, home.audioBitrate)).join("")}</select></label>` : ""}</div>
        <p class="field-note">${home.format === "mp4" ? t("Prioriza H.264/AAC. Los streams incompatibles se recodificarán antes de guardar.", "Prefers H.264/AAC. Incompatible streams will be re-encoded before saving.") : home.format === "source" ? t("Conserva los códecs de origen. El contenedor puede ser MP4, WebM o MKV.", "Keeps source codecs. The container may be MP4, WebM or MKV.") : home.format === "audio" ? t("Extrae la pista sin mejorar ni cambiar su códec. La extensión dependerá del original.", "Extracts the track without improving or changing its codec. The extension depends on the source.") : t("Convertir con pérdida no mejora el audio original.", "Lossy conversion does not improve the original audio.")}</p>
        <div class="destination-row"><span>${icon("folder")}<span><small>${t("Destino", "Destination")}</small><span title="${h(snapshot.settings.folder)}">${h(snapshot.settings.folder || t("Elige una carpeta", "Choose a folder"))}</span></span></span>${action("choose-folder", t("Cambiar", "Change"), undefined, "quiet")}</div>
        <div class="preview-submit">${action("queue-download", home.submitting ? t("Añadiendo…", "Adding…") : batch ? t("Añadir a la cola", "Add to queue") : t("Descargar", "Download"), "downloads", "primary", `id="download" ${!canSubmit || parsed.truncated ? "disabled" : ""}`)}</div>
      </section>` : ""}
    </div>`;
}

