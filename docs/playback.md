# Reproducción local

Biblioteca, descargas y marcadores usan una sesión con un elemento multimedia estable. La aplicación sirve los archivos autorizados por la ruta local `nabmedia`, que admite lectura por rangos sin copiar el archivo completo a memoria. El motor es el WebView2 instalado en Windows.

El vídeo abre una vista de borde a borde con ajuste de proporción, subtítulos locales, velocidad, volumen y pantalla completa. Los controles se ocultan durante la reproducción y vuelven con el puntero o el teclado. La barra distingue la preparación indeterminada del progreso temporal. Volver de esta vista pausa el vídeo y conserva su posición en la barra compacta. La cola y las preferencias se restauran en pausa al reiniciar. Las posiciones inferiores a tres segundos o dentro del tramo final del archivo no se ofrecen para reanudar.

| Muestra verificada en Windows | Resultado |
| --- | --- |
| MP4 vertical, H.264 y AAC, 360 × 640 | Vídeo, búsqueda temporal y reanudación |
| WebM ultrapanorámico, VP9 y Opus, 720 × 248 | Vídeo |
| MKV ultrapanorámico, VP9 y Opus, 720 × 248 | Vídeo en el WebView2 probado |
| MP4 con AV1, 256 × 144 | Vídeo en el WebView2 probado |
| MP3 con portada integrada | Audio clasificado correctamente y reproducido en la misma sesión |
| WAV PCM, M4A AAC, Ogg Opus y FLAC | Audio local |
| MP4 dañado | Error recuperable con conversión explícita o apertura en Windows |
| MP4 movido tras importarlo | Aviso de archivo no disponible |

La matriz corresponde a archivos reales. El MP4 de prueba incluía dos pistas AAC, en español e inglés. El WebView2 probado no expuso `audioTracks`, por lo que en ese equipo no apareció el selector de pistas integradas. Los subtítulos SRT y VTT locales se renderizan como texto, con ajuste de tamaño, contraste y desfase. ASS/SSA conserva las limitaciones del motor para estilos integrados.

Al detener el puntero sobre el progreso, FFmpeg extrae un fotograma del vídeo en el equipo. La petición anterior se cancela al mover el puntero o cambiar de archivo. La caché conserva hasta 28 JPEG de 48 KiB como máximo por fotograma, en memoria durante la sesión. El tiempo se muestra aunque el archivo tarde en producir una miniatura.

## Música inmersiva

Pulsa **Abrir visualización** o la portada en la barra compacta. El transporte queda centrado debajo de la escena, con el tiempo transcurrido y la duración a ambos lados de la línea de progreso. **Escenas** muestra Tinta, Vórtice, Seda, Interferencia, Órbita, Prisma, Lluvia y Aurora, junto con las miniaturas, favoritas y controles de rotación. El tratamiento sobrio es el inicial. Puedes elegir Color, ajustar la intensidad o seleccionar Imagen fija. La rotación automática se puede activar con un intervalo inicial de 45 segundos. El fundido gráfico dura 3,6 segundos y conserva el audio.

La vista de música conecta el elemento de la sesión a un solo `AudioContext`. La forma de onda, la energía por bandas y los ataques se leen antes de la ganancia de salida. Volver a la biblioteca mantiene la misma pista y detiene el dibujo. Se usa también el estado nativo de la ventana para detectar minimización. Un fallo gráfico conserva el audio y muestra la imagen local de la escena.

Las escenas usan un shader propio en `src/music-scenes.glsl`, con una función por escena y dos texturas para realimentar el fotograma anterior. El orden de las funciones debe corresponder a la tabla `scenes` de `src/music-visualizer.ts`. Las miniaturas de 384 × 216 se generan con el mismo renderizador en WebView2 mediante `NABNAB_RENDER_SCENES=1` al ejecutar `tests/native-playback.browser.mjs`. El script escribe los WebP en `artifacts/playback/previews`. Revisa los dos tratamientos antes de copiarlos a `public/playback`.

El lenguaje gráfico toma como referencia las estelas, deformaciones y simetrías de MilkDrop. Las ocho escenas y sus miniaturas son recursos propios. [Spotifast](https://spotifast.rocks/milkdrop/) · [projectM](https://github.com/projectM-visualizer/projectm) · [Butterchurn](https://github.com/jberg/butterchurn) · [Colección de presets de Butterchurn](https://github.com/jberg/butterchurn-presets)

## Verificación

La prueba `tests/native-playback.browser.mjs` genera las muestras con FFmpeg y, cuando está disponible, WebCodecs para AV1. Abre la aplicación con un perfil aislado y comprueba vídeo, subtítulos, teclado, pantalla completa y reanudación tras reiniciar. En música verifica continuidad entre vistas, respuesta a ritmos distintos, silencio real, señal antes del volumen maestro, minimización y pérdida y recuperación del contexto gráfico. El HEVC no se incluyó en la matriz porque el codificador Media Foundation no estaba disponible en el equipo de pruebas.

En Windows, compilación 26200, con Intel Iris Xe y ANGLE/D3D11, se observaron 60 FPS en Automática y 30 FPS en Ahorro, en la escena Órbita a 900 × 620 px CSS. Se midieron los fotogramas dibujados durante 2,3 segundos en cada modo, con la música activa y el selector de escenas abierto. Ahorro usó un lienzo interno de 610 × 420 px. Las cifras corresponden a ese equipo y esas condiciones.

El frontend de producción ocupa 2.221.082 bytes, 7.602 menos que `e191ff3` con el mismo compilador. Las 16 vistas previas bajaron de 65.850 a 55.240 bytes. El código de escenas se carga al abrir la vista musical. Se mantuvieron WebView2 y FFmpeg como motores existentes. La medida corresponde al frontend compilado. El instalador requiere su propia medición.

Las pruebas de navegador de audio y biblioteca cubren las listas, los capítulos, los marcadores y la búsqueda de subtítulos.

[Vídeo con controles](playback/video-controls.png) · [Vídeo con los controles ocultos](playback/video-clean.png) · [Previsualización al buscar](playback/video-seek.png) · [Música a 900 px](playback/music.png) · [Selector de escenas](playback/scenes.png) · [Barra compacta a 900 px](playback/compact.png)
