# Verificación de nabnab 0.2

Registro de trabajo del 5 de noviembre de 2026, Windows x64. Esta página se actualiza con evidencia ejecutada; los pendientes no son certificaciones.

## Línea base

Antes de cambiar la interfaz, `node --test tests/*.test.mjs` pasó 6 pruebas; `tsc --noEmit` y `vite build` terminaron correctamente. Rust pasó 16 pruebas con 10 integraciones omitidas. El renderer y los estados anteriores pasaron `tests/titlebar.browser.mjs` en cuatro tamaños y DPR 1/1.5/2 usando IPC simulado. Las capturas se conservan en `artifacts/optical` y describen únicamente la interfaz.

La copia inicial no tenía motores ni Node en PATH. Se instalaron las dependencias fijadas y se verificaron los motores de la versión ya instalada antes de copiarlos al directorio de desarrollo. El binario de yt-dlp coincide con el hash oficial comprometido; Deno se comparó con el ejecutable de su ZIP oficial verificado. Los motores y los fixtures permanecen excluidos de Git.

## Cambios integrados

Pruebas nativas iniciales de la nueva arquitectura: 55 correctas, 11 integraciones omitidas. Incluyen DPAPI, ACL privadas, copias/migración, bloqueo de escritor, importación, perfiles, URLs engañosas, límites del puente y rangos de reproducción. La ejecución dentro del sandbox produjo denegaciones en APIs de Windows; fuera de él pasó sin debilitar permisos.

La suite integrada, después de corregir concurrencia, recuperación y preferencias, pasó 67 pruebas con 12 integraciones omitidas. Las 11 pruebas JS, TypeScript y Vite también pasaron.

El pase nativo final pasó 108 pruebas, cero fallos y 16 integraciones omitidas. Incluye transporte HTTP directo, recuperación de recibos sin DNS, espera 429 compartida y persistida, publicación concurrente de un único resultado para bytes/perfil/destino idénticos, cancelación mientras se espera publicación, idioma de subtítulos y recepción TCP con el cliente retrasado. `cargo fmt` terminó correctamente. Las integraciones omitidas se ejecutan y documentan por separado; ese recuento no certifica plataformas remotas.

Se ejecutaron además [seis grupos de pruebas con motores reales](NATIVE-TESTS.md), todos correctos. El recorrido `tests/native-app.browser.mjs` pasó ocho comprobaciones con la aplicación Windows, WebView2, IPC y cola reales. Usó archivos sintéticos y datos aislados en `artifacts/native/run-bmJC1r`: importación e inspector, las cuatro herramientas, publicación/indexación, rangos de reproducción de 32 bytes, miniaturas, cancelación, segundo arranque, colecciones/etiquetas, retirada del índice sin borrar el original y puente HTTP autenticado con reenvíos deduplicados y revocación.

La nueva interfaz pasó `tests/nabnab.browser.mjs` en 1180 × 780, 900 × 620 y DPR 1.25/1.5, con IPC simulado. Verifica respuestas de análisis fuera de orden, ausencia de descarga automática, fases y eventos de ejecuciones antiguas, paginación de 60 elementos sobre 1000 registros, formularios persistentes, teclado y controles. Esto prueba la interfaz y se mantiene separado del recorrido nativo. Artefactos en `artifacts/nabnab`.

## Descargas y aplicación reales

El recorrido ampliado `tests/native-app.browser.mjs`, con `NABNAB_OPEN_SAMPLE=1`, pasó once comprobaciones en la aplicación Windows real. Datos aislados finales en `artifacts/native/run-pBxlPb`. Incluyó las cuatro herramientas, importación, reproducción por rangos, miniaturas, reinicio, conservación del original, organización, retirada del índice, autenticación del puente, revocación de dos clientes y reenvío deduplicado. Para HTTP se usó la interfaz real: pegar, esperar opciones y pulsar Descargar; el análisis no añadió trabajos y la entrada quedó libre después del envío. La salida fue MP4 H.264/AAC; otro trabajo generó MP3 a 128 kbps y un tercero reutilizó el resultado idéntico ya verificado. También se comprobó la etiqueta del proveedor `HTTP`. Las versiones de motor y resultados literales se guardaron en `results.json`.

YouTube devolvió metadatos reales de Big Buck Bunny y pasó `smoke::real_engines_public_hd_samples`: dos fragmentos de dos segundos a 720p y 1080p, H.264/AAC comprobado y decodificación completa. Vimeo falló su único intento de metadatos y sigue sin certificar. El test conjunto de metadatos terminó con código 1 porque exige que ambas plataformas pasen; no se oculta ese resultado. Fuentes, licencia, fechas y límites en la [matriz](PLATFORMS.md).

La extensión Edge desempaquetada pasó seis comprobaciones reales con la aplicación y un perfil de prueba aislado: emparejamiento, transporte autenticado, permiso activeTab, envío del popup, reenvío con el mismo ID y rechazo de credencial revocada. La [evidencia del navegador](BROWSER.md) distingue ese recorrido del clic nativo pendiente del menú contextual.

Una repetición bajo carga había detectado un 400 intermitente en el puente. Se aisló con un cliente TCP que envía después de conectar: el test falló antes del arreglo y pasó al configurar explícitamente el socket aceptado en modo bloqueante con timeout. La suite de 108 pruebas, el recorrido de aplicación de once comprobaciones y las seis comprobaciones reales de Edge pasaron después del arreglo. No se ampliaron permisos ni se relajó la validación HTTP.

## Acabado y accesibilidad

La versión final conserva campo y emblema y añade el glass solicitado en la capa funcional: sidebar de 200/64 px, material local de 6 px, controles de Inicio con 8 px y transiciones de 160/200 ms. La [fuente canónica](design-reference.md) registra la excepción posterior del usuario y las referencias Apple, Fluent y Linear. No hay nuevas fuentes, motor 3D ni biblioteca de animación.

Los cinco escenarios finales cubrieron 1280 × 800, 1180 × 780, 900 × 620 y 1180 × 780 con DPR 1.25/1.5. Comprobaron miniaturas apaisadas/retratos sin deformación, acción principal completa a las alturas estándar, biblioteca de 1000 registros con 60 elementos renderizados, foco en selección/acciones y al paginar a una última página de un elemento, anuncio persistente de análisis terminado y preferencias de movimiento/transparencia. La prueba de colores forzados se ejecutó en Edge. El detector mecánico de Impeccable terminó con código 0 y `[]`; no sustituye la inspección de las capturas.

Una revisión independiente midió 105 tramos de texto visibles sobre el fondo rasterizado final: mínimo observado 5.18:1 en Inicio y 5.72:1 en Biblioteca, Herramientas y Ajustes, a 1180 × 780. El material comprobado fue sidebar 6 px/40 % y controles 8 px/60 %. La medición usó canvas estático con movimiento reducido; no demuestra contraste de todos los fotogramas ni todos los estados. La transparencia reducida produjo superficies opacas sin reflejos ni blur. No se probó lectura de voz con NVDA/JAWS.

Capturas en `artifacts/nabnab`; matriz en `browser-final-outcome.json`. El paso auxiliar de vídeo falló inicialmente por el FFmpeg privado ausente de Playwright. Se corrigió el harness para usar frames reales de Edge CDP y el libopenh264 ya preparado, sin repetir la matriz. `glass-walkthrough.mp4` quedó verificado con ffprobe: H.264, 1280 × 800, 11.28 s y 1719864 bytes. Sus datos IPC son fixtures explícitos; el recorrido nativo se verifica por separado. Las muestras intercaladas de cadencia se guardaron, pero la compilación y otros recorridos compartían la máquina; no se infiere FPS, coste GPU ni mejora de rendimiento.

## Paquete y medidas

`npm run desktop:build` terminó correctamente con la revisión final del puente y generó `nabnab_0.2.0_x64-setup.exe`, 67546998 bytes (64.42 MiB). El ejecutable declara producto y descripción `nabnab`, versión `0.2.0`. SHA-256 del instalador: `8205506998c4e449e009b0037f9b09205c48ccb417e926b10421389aa680955c`. El trabajo no publica una release automáticamente.

`scripts/measure-release.mjs` registró 163471648 bytes de payload lógico: ejecutable 10255872, motores 152903506 y licencias/avisos 312270. Este inventario excluye desinstalador, asignación del sistema de archivos, WebView2 compartido y datos del usuario; no es una medición de instalación limpia. La línea base instalada incluía 162059598 bytes, de ellos 79583 de desinstalador; no se presenta una reducción ni una comparación de instaladores porque no se conservó el anterior.

El instalador figura como `NotSigned` al consultar Authenticode; no se inventó una firma o un editor. `dumpbin /dependents` inspeccionó 17 archivos PE del ejecutable y los motores sin imports no resueltos entre el bundle y el sistema de esta máquina. Es una comprobación estática local, no una prueba de instalación limpia.

`tests/measure-native.ps1` tomó dos muestras de la app debug con datos/perfil aislados y Vite, en `artifacts/performance/debug-20261105-213424/native.json`. Observó la ventana a 249/238 ms, un árbol de procesos estable y working sets de 509652992/486879232 bytes. En ventanas de 5.37/5.40 s midió 2.359375/1.171875 s de CPU del árbol, equivalentes al 5.49/2.71 % de ocho procesadores lógicos. Se conservan las lecturas individuales sin redondear. La creación de ventana no es primer pintado; debug, tamaño, perfil, estado de caché y señal de arranque difieren de la línea base instalada. La máquina compartía otras tareas. No son pruebas de arranque frío/caliente ni una comparación controlada. GPU no medida.

Los informes sanitizados de [aplicación nativa](verification/native-app.json), [metadatos de plataformas](verification/platform-metadata.json), [interfaz](verification/interface.json) y [payload](verification/release-size.json) se conservan junto a esta página. Las capturas comparables anteriores y actuales están enlazadas desde el README.

## Reproducir los recorridos

Los comandos habituales están en el README y las integraciones locales en `NATIVE-TESTS.md`. Los harnesses de navegador requieren Edge y Playwright; esta sesión usó Playwright 1.62.1. Puede prepararse como herramienta de QA sin modificar los manifests del proyecto:

```powershell
npm install --no-save --package-lock=false playwright@1.62.1
npm run dev
```

Con Vite en otra terminal y los motores verificados, `node tests/nabnab.browser.mjs` ejecuta la matriz de interfaz con IPC simulado. `PLAYWRIGHT_MODULE` permite señalar una instalación existente del módulo. El vídeo utiliza el FFmpeg del proyecto; no requiere el codificador privado de Playwright.

Para los recorridos de aplicación real, se necesita la compilación debug y este fixture sintético de seis segundos. El directorio, los perfiles y las salidas de prueba están excluidos de Git:

```powershell
cargo build --manifest-path src-tauri/Cargo.toml
New-Item -ItemType Directory -Path artifacts/fixtures -Force
& ./src-tauri/engines/ffmpeg.exe -hide_banner -loglevel error -nostdin -n -f lavfi -i 'testsrc2=size=320x180:rate=12:duration=6' -f lavfi -i 'sine=frequency=440:sample_rate=48000:duration=6' -c:v libopenh264 -b:v 500k -pix_fmt yuv420p -c:a aac -b:a 128k -shortest './artifacts/fixtures/nabnab prueba.mp4'
$env:NABNAB_OPEN_SAMPLE='1'
node tests/native-app.browser.mjs
node tests/native-extension.browser.mjs
```

Estos dos recorridos se ejecutan de forma secuencial y usan datos/perfiles aislados del ejecutable debug. El segundo usa un perfil Edge temporal; el primero añade la muestra abierta de la matriz cuando se activa `NABNAB_OPEN_SAMPLE`. La app de pruebas necesita tener libre el puerto local 49587. La aplicación release no habilita la ruta de aislamiento debug.

## Pendiente

El usuario decidió dejar pendientes los recorridos de VM y comunicó después que había probado la aplicación en otro dispositivo y funcionaba bien. Se conserva como validación aportada por él, sin atribuirle un hash o alcance de prueba no especificado.

- Instalación en Windows limpio, actualización real desde 0.1.1 y desinstalación en una VM por parte del agente; el segundo arranque de datos aislados y la migración nativa sí se probaron. No se modificó la instalación personal anterior.
- Medidas de release antes/después comparables, primer pintado frío/caliente, GPU y carga real de varias descargas. La biblioteca de 1000 registros se comprobó con fixtures, no con 1000 archivos físicos.
- Plataformas no certificadas, subtítulos remotos, Chrome/Firefox y clic nativo del menú contextual. Los contratos implementados y sus límites permanecen en la matriz y `BROWSER.md`.
