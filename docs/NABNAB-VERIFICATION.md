# Verificación de nabnab 0.2

Registro de trabajo del 5 de noviembre de 2026, Windows x64. Esta página se actualiza con evidencia ejecutada; los pendientes no son certificaciones.

## Línea base

Antes de cambiar la interfaz, `node --test tests/*.test.mjs` pasó 6 pruebas; `tsc --noEmit` y `vite build` terminaron correctamente. Rust pasó 16 pruebas con 10 integraciones omitidas. El renderer y los estados anteriores pasaron `tests/titlebar.browser.mjs` en cuatro tamaños y DPR 1/1.5/2 usando IPC simulado. Las capturas se conservan en `artifacts/optical` y describen únicamente la interfaz.

La copia inicial no tenía motores ni Node en PATH. Se instalaron las dependencias fijadas y se verificaron los motores de la versión ya instalada antes de copiarlos al directorio de desarrollo. El binario de yt-dlp coincide con el hash oficial comprometido; Deno se comparó con el ejecutable de su ZIP oficial verificado. Los motores y los fixtures permanecen excluidos de Git.

## Cambios integrados

Pruebas nativas iniciales de la nueva arquitectura: 55 correctas, 11 integraciones omitidas. Incluyen DPAPI, ACL privadas, copias/migración, bloqueo de escritor, importación, perfiles, URLs engañosas, límites del puente y rangos de reproducción. La ejecución dentro del sandbox produjo denegaciones en APIs de Windows; fuera de él pasó sin debilitar permisos.

La suite integrada, después de corregir concurrencia, recuperación y preferencias, pasó 67 pruebas con 12 integraciones omitidas. Las 11 pruebas JS, TypeScript y Vite también pasaron.

Se ejecutaron además [seis grupos de pruebas con motores reales](NATIVE-TESTS.md), todos correctos. El recorrido `tests/native-app.browser.mjs` pasó ocho comprobaciones con la aplicación Windows, WebView2, IPC y cola reales. Usó archivos sintéticos y datos aislados en `artifacts/native/run-bmJC1r`: importación e inspector, las cuatro herramientas, publicación/indexación, rangos de reproducción de 32 bytes, miniaturas, cancelación, segundo arranque, colecciones/etiquetas, retirada del índice sin borrar el original y puente HTTP autenticado con reenvíos deduplicados y revocación.

La nueva interfaz pasó `tests/nabnab.browser.mjs` en 1180 × 780, 900 × 620 y DPR 1.25/1.5, con IPC simulado. Verifica respuestas de análisis fuera de orden, ausencia de descarga automática, fases y eventos de ejecuciones antiguas, paginación de 60 elementos sobre 1000 registros, formularios persistentes, teclado y controles. Esto prueba la interfaz y se mantiene separado del recorrido nativo. Artefactos en `artifacts/nabnab`.

## Pendiente de registrar

- Descarga real autorizada, resultados y versiones.
- Instalador, actualización desde 0.1.1, segundo arranque y desinstalación.
- Medidas antes/después comparables, biblioteca de 1000 elementos y límites de la extensión.
