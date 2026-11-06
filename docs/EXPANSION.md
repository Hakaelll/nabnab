# Ampliación de fuentes y herramientas

El encargo se implementa sobre Tauri 2, Rust y TypeScript existentes. La referencia visual sigue siendo [design-reference.md](design-reference.md). Se conservan cola, sesiones protegidas, biblioteca, preparador de lotes y seguimiento existentes.

## Contratos y entregas

- A. Registro de operaciones por proveedor y catálogo conectado; gallery-dl opcional, exploración acotada, selección por adjunto, descarga por archivo y colección común.
- B. Proveedores de vídeo/audio, podcasts RSS con selección y OPML, directos limitados y detección de medios por acción explícita en la extensión.
- C. aria2 opcional, consentimiento P2P, metadatos antes de contenido, selección y publicación verificada en la cola.
- D. Telegram con credenciales propias, sesión protegida y navegación de medios autorizados.
- E. Adaptadores independientes de cursos, selección de materiales y estudio local.
- F. Herramientas locales y transcripción opcional, con salida en cola y biblioteca.

Un único integrador modifica cola, configuración y comandos compartidos. Los módulos independientes exponen sus contratos antes de integrarse. No se instalan componentes en el equipo durante el desarrollo, no se inician sesiones y no se descarga contenido personal.

Cada entrega debe pasar comprobación TypeScript, pruebas Rust relevantes y recorrido renderizado cuando afecte a la interfaz. Los commits usan mensajes breves en inglés y se suben al remoto existente por petición expresa del usuario. Ninguna cuenta sin probar se declara verificada. Una integración bloqueada no detiene las independientes.

## Estado

Los seis bloques tienen módulos integrados en la cola y biblioteca existentes. Además del primer bloque se añadieron Telegram, directos con límite y parada guardando el resultado, descargas HTTP con aria2, documentos PDF/EPUB, exportaciones de fuentes, seguimiento de galerías, autenticación acotada de Pixiv/Mastodon, edición de subtítulos, unión de clips, sustitución de audio, paletas y preescucha de audio.

La interfaz conserva la identidad monocromática. La navegación llega al borde superior; los controles de herramientas se agrupan con separadores en lugar de tarjetas repetidas. Se eliminó el nombre bajo el emblema. Diez activaciones del símbolo alternan el multicolor, también con teclado. El modo normal recibe oleadas suaves de tonos vecinos cada 25 segundos; el easter egg activa oleadas continuas solapadas. La reducción de movimiento desactiva las oleadas y el desplazamiento del color.

## Evidencia

- TypeScript, build Vite y comprobación de identidad correctos; 27 pruebas JavaScript y 224 pruebas Rust correctas. Otras 25 pruebas nativas requieren motores o servicios y están marcadas como ignoradas por defecto.
- Se ejecutaron expresamente las pruebas con archivos sintéticos reales de herramientas FFmpeg (incluidos WebP estático y animado), incrustación real de subtítulos con libass, PDF con MuPDF, galería HTML/hoja de contacto, unión y audio de vídeo, paletas, preescucha y descarga HTTP con aria2. Los originales se conservaron.
- El recorrido de navegador con fixtures IPC pasó a 1280×800, 1180×780 y 900×620, y con escalados 125 % y 150 %. Incluye navegación, formularios, biblioteca de 1000 elementos, foco, reducción de movimiento/transparencia y estados de descarga. No certifica servicios externos.
- La barra lateral y el easter egg se comprobaron además en la interfaz renderizada con puntero y teclado. Capturas actuales: [Inicio](interface-monochrome.png), [Multicolor](interface-color-egg.png), [Herramientas](interface-tools-refined.png). Las capturas expansion-* conservan el primer bloque como referencia anterior.

## Límites comprobables

No se han iniciado sesiones, instalado componentes en el sistema ni descargado archivos personales. Los motores utilizados en las pruebas están dentro del repositorio ignorado y se verificaron contra sus paquetes oficiales. No se declara verificado un proveedor que no se haya utilizado con una cuenta real.

Telegram ofrece TDLib como componente opcional verificado de NuGet, con instalación atómica y prueba local del esquema; la autorización de una cuenta sigue pendiente. Las cuentas de cursos, Pixiv y Mastodon necesitan autorización propia y validación en vivo. Los directos no se han grabado contra un servicio real. La transcripción se probó con Tiny multilingüe y una frase sintética en español, incluido su paso por la cola y búsqueda en Biblioteca; no certifica precisión general. Los extractores externos pueden cambiar o rechazar fuentes; las limitaciones aparecen como errores, nunca como resultados fabricados.

Consulta los documentos de cada módulo en este directorio para los formatos, recorridos, permisos y límites concretos. La compilación de FFmpeg comprueba las capacidades requeridas por su receta; una instalación antigua puede necesitar actualizar ese motor para WebP e incrustación de subtítulos.

## Fuentes y acceso

En Inicio se mantiene el flujo de pegar y descargar. Plataformas permite elegir los recorridos especializados. Esta tabla describe las rutas implementadas; ninguna cuenta de estos servicios se utilizó para validar acceso durante el desarrollo.

| Fuente | Contenido y acceso | Límite relevante |
| --- | --- | --- |
| YouTube | Vídeo, audio, listas, subtítulos; sesión opcional | yt-dlp y Deno; directos no DRM y exportación acotada de metadatos |
| Twitch | Vídeos accesibles y directos activos | Sesión según el origen; grabación con límite explícito |
| Bilibili | Vídeo accesible y XML danmaku | La API de comentarios puede exigir acceso o firma adicional |
| Dailymotion, Vimeo, TikTok | Vídeo/listas que yt-dlp enumere | Acceso y formatos reales del extractor; sin bypass de DRM |
| Instagram, Facebook, Threads | Adjuntos mediante gallery-dl; vídeo compatible mediante yt-dlp | Sesión según acceso; selección paginada y manifiestos no compatibles identificados |
| X / Twitter | Adjuntos y texto de conversación accesible | Sin mirrors; la conversación puede estar incompleta |
| Reddit | Adjuntos, texto y comentarios accesibles | Exportación acotada con aviso de partes no disponibles |
| Bluesky | Adjuntos enumerados por gallery-dl | Solo contenido accesible al extractor |
| Pinterest | Pines, tableros y secciones | gallery-dl opcional; máximo 50 archivos por bloque |
| ArtStation, DeviantArt | Obras, álbumes y perfiles seleccionados | Sesión según restricciones del origen; sin contenido de pago no autorizado |
| Pixiv | Obras accesibles y selección por archivo | Token de actualización propio protegido; cookies solas no bastan |
| Flickr, Tumblr, Imgur | Imágenes y medios que gallery-dl enumere | Originales conservados; permisos del origen |
| PeerTube | Vídeos/listas con archivo directo anunciado | Consentimiento por instancia pública; HLS exclusivo no implementado |
| Mastodon | Adjuntos de publicaciones accesibles | OAuth propio por instancia para contenido que requiera autorización |
| SoundCloud | Pistas/listas accesibles | Calidad disponible del stream, sesión según origen |
| Bandcamp | Pistas/álbumes públicos | El stream no equivale a formatos de compra |
| RSS | Enclosures de episodios, OPML y M3U | Feeds privados protegidos; importación no descarga episodios por sí sola |
| HTTP/HTTPS | Archivo original, rangos, reanudación y SHA-256 | aria2 opcional; URL pública validada, sin compartir cookies entre orígenes |
| BitTorrent | Magnet v1 y archivos torrent; selección de archivos | aria2 opcional y consentimiento P2P; sin seeding posterior por defecto |
| Telegram | Adjuntos de chats/mensajes autorizados | TDLib opcional de NuGet y cuenta propia; no contenido protegido ni efímero; pendiente prueba de cuenta real |
| Udemy | Estructura y materiales marcados descargables | Sesión propia, solo consultas; no matrícula automática ni DRM |
| Hotmart, Kiwify | Lecciones y adjuntos explícitamente descargables | Adaptadores separados, token de alumno propio; un stream no implica permiso de descarga |

Las herramientas y sus componentes se detallan en [TOOLS.md](TOOLS.md), [DOCUMENTS.md](DOCUMENTS.md), [TRANSCRIPTION.md](TRANSCRIPTION.md), [MEDIA-EDITS.md](MEDIA-EDITS.md), [CAPTION-EDITOR.md](CAPTION-EDITOR.md), [GALLERY-EXPORTS.md](GALLERY-EXPORTS.md), [PALETTES.md](PALETTES.md) y [AUDIO-PREVIEW.md](AUDIO-PREVIEW.md).

El componente Telegram se probó además desde Ajustes en una instancia nativa con datos aislados: instalación, inventario, estado instalado y desactivación; sin cuentas ni trabajos añadidos. Las pruebas locales cargaron realmente TDLib 1.8.69 y verificaron el rechazo de un paquete alterado. Captura: [Componente Telegram](telegram-components.png).

Pegar publicaciones de Instagram, X, Reddit y Bluesky en Inicio se comprobó en la aplicación nativa: abre el formulario de selección de adjuntos, conserva la URL y no añade trabajos ni consulta el origen hasta explorar. El selector de redes conserva una acción explícita «Solo vídeo en Inicio» para el flujo existente de yt-dlp.
