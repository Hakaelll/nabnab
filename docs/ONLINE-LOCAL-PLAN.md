# Archivos online y herramientas locales

Especificación: el prompt del usuario del 6 de noviembre de 2026. Se conserva Tauri 2, Rust, TypeScript, la cola y la biblioteca existentes. La referencia visual canónica sigue siendo [design-reference.md](design-reference.md). El usuario autoriza commits cortos en inglés y push a `main`.

## Secuencia verificable

- A. Adaptadores independientes para Releases, Internet Archive y Commons; navegador de selección y descarga HTTP segura a la biblioteca. Conexiones Drive, OneDrive y Dropbox con OAuth de escritorio, tokens protegidos, navegación paginada y exportación por MIME. Las credenciales legítimas y las cuentas reales se identifican como requisitos externos.
- B. Acciones explícitas de extensión para artículo, selección y página estática; saneamiento, previsualización, lectura y anotaciones por versión. OCR opcional con idiomas, índice y copia PDF buscable; compresión, márgenes, numeración, agua y marcadores con originales intactos.
- C. Comparación exacta y perceptual, organización, renombrado reversible con colisiones, unión de capturas, iconos, atlas y colección portable con importación acotada.
- D. Captura explícita con indicador persistente, fuentes/audio separados, salida recuperable y búfer opt-in. Voz offline opcional, muestra y segmentos reanudables relacionados con el texto original.
- E. CLI y MCP stdio desactivado inicialmente, con catálogo cerrado, autorización local revocable, planes vinculados a parámetros/presupuesto/caducidad y propietario único de persistencia.

Cada entrega cierra su recorrido antes de pasar a la siguiente. No se considera completo un componente que solo tenga formulario. Se reutilizan servicios y migraciones aditivas, sin historiales paralelos.

## Contratos de A

`public_sources::inspect` devuelve páginas limitadas de releases/ítems o archivos con identidad estable. `resolve` vuelve a consultar esa identidad al ejecutar. `cloud_storage::Selection` conserva proveedor, cuenta, ID, versión y formato de exportación, nunca una URL preautorizada. La cola integra ambas clases y publica con las mismas reglas de no sobrescritura.

La transferencia pública amplía `direct.rs` para nombres explícitos de archivos elegidos, conservando DNS público fijado, redirecciones validadas, cancelación, límites, progreso y comprobación de integridad. La biblioteca conserva procedencia, derechos y atribución disponibles.

## Revisión y pruebas

Probar paginación/cancelación, archivos vacíos o truncados, nombres/rutas hostiles, origen restringido, metadatos HTML, hashes cambiados y enlaces temporales sin filtrar secretos. Ejecutar comprobaciones TypeScript, pruebas Node y Rust pertinentes. Verificar la interfaz renderizada en 1180 × 780 y 900 × 620, teclado y estados vacíos/error. Separar fixtures, acceso público real, acceso OAuth real y pruebas con hardware. Registrar evidencia y pendientes por entrega.

## Estado

A y B implementadas. A conserva el requisito externo de credenciales OAuth legítimas para probar cuentas reales. B integra archivo web, lectura/anotaciones, OCR opcional y operaciones PDF. Suite integrada: 269 pruebas nativas aprobadas, 32 opt-in omitidas y 27 JavaScript aprobadas. Las pruebas nativas adicionales verificaron OCR real, todos los cambios PDF y EPUB. Se completaron siete trabajos B en la app con datos aislados; se conservó el hash del original y se abrió la página OCR reconocida desde búsqueda global. Interfaz verificada en 1180 × 780 y 900 × 620, con comparaciones de apartados antes/después. La revisión corrigió capturas de texto editable, Markdown activo y colisiones de índices entre reintentos OCR. C, D y E pendientes. Ninguna cuenta OAuth ni captura de pantalla del usuario se activa sin su autorización explícita en la aplicación.
