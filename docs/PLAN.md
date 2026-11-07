# Evolución a nabnab

Estado inicial: `7f533a8`, rama inicial `codex/nabnab`. El usuario pidió después integrar y subir directamente a `main`; ese es el destino vigente. El encargo completo está en `../artifacts/nabnab-spec.md` (copia local del encargo, excluida de Git). La dirección visual vigente, incluida la petición posterior explícita de liquid glass, se registra en `design-reference.md`; las referencias de 0.1 se conservan como historia.

## Auditoría inicial

- Tauri 2, Rust, TypeScript y Vite. Sin framework UI, reproductor ni motor 3D añadido.
- Una ventana con análisis cancelable, una descarga, perfiles MP4/MP3/M4A/Opus, directorio persistido como `destination.json` y actualización explícita de yt-dlp.
- Los recursos instalados resuelven yt-dlp, FFmpeg, ffprobe y Deno desde `resource_dir/engines`; Deno recibe una ruta explícita. No se usa PATH para motores.
- No existen historial, colecciones, credenciales, cola persistente ni extensión que migrar. No se inventarán ubicaciones antiguas.
- Identidad instalada `com.ytnabber.desktop`. Se conserva para acceso a configuración, motores actualizados y continuidad de NSIS. Marca visible y binario nuevos: `nabnab`.
- La copia de trabajo no contiene motores, dependencias JS ni artefactos compilados. Node no está en PATH; se utiliza el runtime de desarrollo de Codex. La primera prueba de Cargo requiere acceso al registro. Esto limita la medición inicial instalada.
- Omniget consultado en `b4c733f5a8a699634845e445fc8f162acde547c9`, GPL-3.0. Referencia conceptual de contratos, sin reutilizar código. Su cola/history no equivalen a una biblioteca local; su puente HTTP no cumple los requisitos de esta tarea.

## Arquitectura y propiedad de archivos

- `model.rs`, `providers.rs`, `engine.rs`, `media.rs`: URLs, metadatos y capacidades; procesos y verificación existentes conservados.
- `store.rs`, `library.rs`: repositorio JSON versionado y reemplazo atómico, bloqueo exclusivo, migración idempotente e índices de archivos autorizados. Un escritor bajo mutex. No se necesita SQLite para esta primera colección local.
- `queue.rs`: IDs estables y ejecuciones distintas; un planificador para red y procesamiento, límite por dominio, persistencia en transiciones y progreso efímero. Eventos antiguos descartados. Recuperación pausada.
- `tools.rs`: cuatro operaciones FFmpeg, argumentos estructurados, salida temporal y ffprobe. La cola publica resultados e indexa archivos.
- `main.rs`: frontera IPC y diálogos; valida entradas, autoriza archivos y adapta servicios.
- `src/`: navegación, captura, cola, biblioteca, herramientas y ajustes; tokens del encargo, emblema y campo de caracteres existentes.
- `browser-extension/` y puente local: solicitudes identificadas, emparejamiento explícito, revocación y permisos mínimos. Sin extracción en el navegador.

## Plan verificable

- [x] Leer encargo, Git, configuración, motores, persistencia y referencia.
- [x] Registrar comprobaciones iniciales y capturar UI anterior donde sea ejecutable.
- [x] Marca, ventana, navegación y migración con pruebas de arranque repetido.
- [x] Contratos de proveedores, lotes, perfiles y cola persistente con cancelación real.
- [x] Biblioteca autorizada, importación, colecciones, etiquetas e inspector con fallback local.
- [x] Convertir, extraer audio, recortar y comprimir conectados a cola y biblioteca.
- [x] Motores, ajustes, sesiones y privacidad con errores accionables.
- [x] Extensión y transporte local protegido; pruebas reales en Edge de reenvío y clientes no autorizados. Chrome, Firefox y el clic nativo del menú contextual siguen sin certificar.
- [x] Compilación, pruebas nativas/JS, recorridos reales con archivos autorizados, revisión visual y empaquetado. Las pruebas de instalación/actualización/desinstalación en VM se aplazaron expresamente.
- [x] Matriz de plataformas y mediciones reales; separar comprobado y pendiente.

Cada unidad se verifica antes de commit y push. No se certifica una plataforma mediante fixtures. Las pruebas de códecs usan archivos sintéticos locales autorizados; las de plataformas requieren fuentes autorizadas y se registran por separado.

## Ampliación de archivos y herramientas locales

- [x] A: fuentes públicas y conectores OAuth, selección y cola compartida. OAuth requiere una aplicación registrada y una cuenta autorizada; no se certifica acceso a cuentas mediante fixtures.
- [x] B: archivo web, lector/anotaciones, OCR y edición PDF; recorrido nativo con motores locales.
- [x] C: comparación y renombrado de archivos, assets de imágenes y colecciones portables; recorrido nativo con integridad de originales.
- [x] D: captura de pantalla, repetición y voz local. [Captura](verification/capture-native.json) y [voz](verification/voice-native.json) verificadas con perfiles aislados.
- [x] E: CLI/MCP, permisos y trabajador explícito sin ventana. [Recorrido autorizado](verification/automation-native.json) con un único propietario de la cola.

## Riesgos que deben probarse

1. Corte durante persistencia/publicación y segundo arranque sin duplicados.
2. Cancelación con procesos descendientes y resultados parciales.
3. URLs engañosas, red privada, consultas sensibles, symlinks y colisiones de nombres.
4. Metadatos fuera de orden y cambios de selección mientras hay trabajos activos.
5. Biblioteca grande, archivos desaparecidos, ventana pequeña, escalado y teclado.

La evidencia y las limitaciones se actualizarán en `NABNAB-VERIFICATION.md`; el registro de decisiones está en `decisions.tsv`.

## Validaciones aplazadas

El usuario pidió dejar pendientes los recorridos de VM y después informó de que había probado la aplicación en otro dispositivo y funcionaba bien. Se registra como evidencia aportada por el usuario; no se atribuye al agente ni a un hash concreto del instalador.

La continuación manual es: instalar el paquete final en un Windows de pruebas sin herramientas de desarrollo; comprobar motores y un archivo autorizado; actualizar una instalación 0.1.1 con destino y medios de prueba; abrir por segunda vez y revisar recuperación pausada; desinstalar comprobando que conserva medios y que solo elimina ajustes/caché si se selecciona esa opción. Quedan además las plataformas y los recorridos de navegador identificados como no certificados en sus matrices.

