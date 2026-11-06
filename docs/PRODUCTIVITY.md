# Productividad local

El encargo implementa cinco recorridos sobre la cola y el repositorio JSON existentes. No cambia motores, identidad ni permisos de sesión. La instrucción actual autoriza commits y push.

## Contratos e integración

`options.rs` resuelve ajustes versionados y reglas. `batch.rs` conserva borradores y prepara elementos. `sources.rs` enumera fuentes y entrega selecciones a la misma cola. `scheduling.rs` decide cuándo puede comenzar un trabajo. `library` y `captions` conservan metadatos, segmentos y búsqueda local. `queue.rs` es el único responsable de ejecutar y publicar salidas.

Cada salida relacionada es un trabajo de la cola con un grupo común. Permite conservar salidas correctas y reintentar errores individuales. `Job.options` conserva una copia de `ResolvedOptions`; la identidad de salida incluye contenido, formato, idioma y recorte. Las nuevas propiedades JSON usan valores predeterminados y mantienen los registros anteriores.

Los controladores de interfaz se conectan desde `main.ts` mediante callbacks. Los módulos específicos poseen sus propios tipos y vistas. La navegación principal se conserva. Biblioteca usa Archivos, Colecciones, Fuentes y Para después.

## Entregas y comprobaciones

- [ ] A. Borradores, preparación progresiva, excepciones y ajustes. Comprobar lote mixto, recuperación y encolado seleccionado.
- [ ] B. Reglas, destinos seguros, prioridades, programación y salidas relacionadas. Comprobar precedencia, rutas, horarios e independencia de versiones.
- [ ] C. Fuentes de YouTube, paginación e incorporaciones. Comprobar línea base parcial, opt-in, reordenación e idempotencia.
- [ ] D. Subtítulos, capítulos, fragmentos, búsqueda, colecciones y marcadores. Comprobar tiempos, filtros y conservación de archivos.
- [ ] E. Paleta, extensión e integración visual. Comprobar intenciones y recepción sin descarga accidental, teclado y tamaños de ventana.

Por entrega: pruebas deterministas pertinentes, comprobación de TypeScript y compilación nativa. Al integrar: recorridos renderizados y capturas. Registrar límites reales en este archivo y decisiones en `docs/decisions.tsv`.

## Casos que requieren atención

Enlaces inválidos mezclados, cambios del ajuste después de encolar, rutas con enlaces simbólicos, historial parcial sin fechas, reinicios y cambios de horario, subtítulos de recortes y coincidencia entre encolado manual y automático.

## Documentación consultada

- [yt-dlp 2026.10.26](https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/README.md), versión fijada en el repositorio.
- [FFmpeg](https://ffmpeg.org/ffmpeg.html), versión fijada 9.0.2.
- [serde_json](https://docs.rs/serde_json/), persistencia existente sin SQLite.

La búsqueda utiliza segmentos locales en el repositorio JSON. No se añade otra base de datos por disponer de FTS.
