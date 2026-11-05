# Matriz de plataformas

La presencia de un extractor de yt-dlp y las pruebas con JSON simulado no certifican una plataforma. Esta matriz distingue implementación de pruebas reales. No se usan servicios de extracción intermediarios.

| Plataforma | Contrato y motor | Vídeo/audio | Lotes | Imagen/galería | Sesión opcional | Prueba real en 0.2 |
| --- | --- | --- | --- | --- | --- | --- |
| YouTube | Host oficial, yt-dlp + Deno/EJS local | Implementado | Selección hasta 200 | No ofrecido | DPAPI por sitio | Pendiente |
| X/Twitter | Host oficial, yt-dlp | Implementado según extractor | Según metadatos | No ofrecido | DPAPI por sitio | Pendiente |
| Instagram | Host oficial, yt-dlp | Implementado según extractor | Según metadatos | No ofrecido | DPAPI por sitio | Pendiente |
| Reddit | Host oficial, yt-dlp | Implementado según extractor | Según metadatos | No ofrecido | DPAPI por sitio | Pendiente |
| TikTok | Host oficial, yt-dlp | Implementado según extractor | Según metadatos | No ofrecido | DPAPI por sitio | Pendiente |
| Vimeo | Host oficial, yt-dlp | Implementado según extractor | Según metadatos | No ofrecido | DPAPI por sitio | Pendiente |
| Twitch | Host oficial, yt-dlp | Vídeos finalizados | Según metadatos | No ofrecido | DPAPI por sitio | Pendiente |
| Pinterest | Detección de host | Sin certificar | Sin certificar | No ofrecido | DPAPI por sitio | Pendiente |
| HTTP/HTTPS directo | Extractor genérico de yt-dlp | Sin certificar | No | No ofrecido | No | Pendiente |
| HLS/DASH | Motor subyacente, sin certificación de recursos/redirecciones | Sin certificar, sin DRM | No | No | No | Pendiente |
| Archivos locales | FFmpeg/ffprobe y biblioteca | Implementado | Trabajos separados | Importación local | No necesita | Pendiente de pruebas de salida |

No se anuncian galerías remotas ni un motor adicional sin una carencia y pruebas concretas. Los metadatos de listas conservan calidades desconocidas hasta que el motor analiza cada elemento. No se admiten directos ni emisiones programadas.

Las comprobaciones de red validan el destino inicial y su DNS. No certifican todos los recursos que yt-dlp decide abrir después. Véase la [política de red](NETWORK.md). Las pruebas reales que se ejecuten se anotarán con fecha, versión del motor, tipo, fuente autorizada y resultado en `NABNAB-VERIFICATION.md`.
