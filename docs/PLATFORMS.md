# Matriz de plataformas

La presencia de un extractor de yt-dlp y las pruebas con JSON simulado no certifican una plataforma. Esta matriz distingue implementación de pruebas reales. No se usan servicios de extracción intermediarios.

| Plataforma | Contrato y motor | Vídeo/audio | Lotes | Imagen/galería | Sesión opcional | Prueba real en 0.2 |
| --- | --- | --- | --- | --- | --- | --- |
| YouTube | Host oficial, yt-dlp + Deno/EJS local | Implementado | Selección hasta 200 | No ofrecido | DPAPI por sitio | Metadatos y vídeo H.264/AAC a 720p/1080p verificados con muestra abierta; audio independiente y subtítulos remotos sin certificar |
| X/Twitter | Host oficial, yt-dlp | Implementado según extractor | Según metadatos | No ofrecido | DPAPI por sitio | Pendiente |
| Instagram | Host oficial, yt-dlp | Implementado según extractor | Según metadatos | No ofrecido | DPAPI por sitio | Pendiente |
| Reddit | Host oficial, yt-dlp | Implementado según extractor | Según metadatos | No ofrecido | DPAPI por sitio | Pendiente |
| TikTok | Host oficial, yt-dlp | Implementado según extractor | Según metadatos | No ofrecido | DPAPI por sitio | Pendiente |
| Vimeo | Host oficial, yt-dlp | Implementado según extractor | Según metadatos | No ofrecido | DPAPI por sitio | Intento de metadatos fallido; no certificado |
| Twitch | Host oficial, yt-dlp | Vídeos finalizados | Según metadatos | No ofrecido | DPAPI por sitio | Pendiente |
| Pinterest | Detección de host | Sin certificar | Sin certificar | No ofrecido | DPAPI por sitio | Pendiente |
| HTTP/HTTPS directo | Cliente nativo con DNS fijado y validación por salto | MP4 y MP3 verificados con muestra pública; seis perfiles con fixture local real | Enlaces independientes | No ofrecido | No | Recorrido real de Inicio a archivo/biblioteca y reutilización de salida verificados |
| HLS/DASH arbitrario | Rechazado por el proveedor directo; genérico de yt-dlp deshabilitado | No ofrecido | No | No | No | Bloqueo de manifiestos y recursos locales probado |
| Archivos locales | FFmpeg/ffprobe y biblioteca | Cuatro herramientas verificadas | Trabajos separados | Importación local | No necesita | Aplicación Windows, IPC, cola y códecs reales verificados; no se anuncia reproducción universal |

No se anuncian galerías remotas ni un motor adicional sin una carencia y pruebas concretas. Los metadatos de listas conservan calidades desconocidas hasta que el motor analiza cada elemento. No se admiten directos ni emisiones programadas.

El proveedor HTTP directo valida cada salto y fija los destinos DNS aprobados. En proveedores de plataforma se valida el destino inicial; eso no certifica todos los recursos que yt-dlp decide abrir después. Véase la [política de red](NETWORK.md).

## Muestras autorizadas y resultados

Pruebas del 5 de noviembre de 2026, sin cookies. Motores: yt-dlp `2026.10.26`, Deno `2.9.8`, FFmpeg/ffprobe `9.0.2`. Solo se usaron medios sintéticos locales y fragmentos de **Big Buck Bunny**, Blender Foundation / www.bigbuckbunny.org, con licencia [CC BY 3.0 indicada por el proyecto](https://peach.blender.org/about/).

- [YouTube, publicación de Blender Foundation](https://www.youtube.com/watch?v=aqz-KE-bpKQ): metadatos reales, duración 635 s y alturas disponibles hasta 2160p. `smoke::real_engines_public_hd_samples` descargó dos fragmentos de dos segundos a 720p y 1080p, verificó H.264/AAC con ffprobe y decodificó cada archivo completo. Esta prueba no verifica listas, sesiones, audio independiente ni subtítulos de YouTube.
- [Vimeo, publicación de Blender](https://vimeo.com/1084537): el único intento de análisis terminó en error de extracción; el diagnóstico sanitizado no permite asignar una causa más precisa. No se repitió ni eludió el bloqueo. Artefacto local: `artifacts/platforms/open-license-metadata.json`.
- [Fragmento público HTTP de Big Buck Bunny](https://www.w3schools.com/html/mov_bbb.mp4): el recorrido de la aplicación real pegó el enlace, esperó la previsualización y pulsó Descargar. Generó MP4 H.264/AAC de 10.026667 s y 788493 bytes. Otro trabajo generó MP3 a 128 kbps de 10.005333 s y 160990 bytes. Un reenvío con solicitud distinta reutilizó el mismo resultado MP4 verificado sin añadir otro archivo de biblioteca. El proveedor del trabajo se comprobó como `HTTP`. Artefacto final: `artifacts/native/run-pBxlPb/results.json`.

Las demás plataformas no tienen una muestra abierta comprobada en este registro y permanecen sin certificar. Un contrato de host o una sesión protegida no sustituye la descarga real de un contenido autorizado.
