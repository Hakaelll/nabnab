# Reproducción local

Biblioteca, descargas y marcadores usan una sesión con un elemento multimedia estable. La aplicación sirve los archivos autorizados por la ruta local `nabmedia`, que admite lectura por rangos sin copiar el archivo completo a memoria. El motor es el WebView2 instalado en Windows.

El vídeo abre una vista de borde a borde con ajuste de proporción, subtítulos locales, velocidad, volumen y pantalla completa. Los controles se ocultan durante la reproducción y vuelven con el puntero o el teclado. La barra distingue la preparación indeterminada del progreso temporal. Volver de esta vista pausa el vídeo y conserva su posición en la barra compacta. La cola y las preferencias se restauran en pausa al reiniciar. Las posiciones inferiores a tres segundos o dentro del tramo final del archivo no se ofrecen para reanudar.

Abre el **minirreproductor** desde la barra o la vista grande. Se queda en una esquina mientras navegas por nabnab. El vídeo conserva la imagen y los controles básicos. El audio reúne portada, título, volumen y tiempo con una onda dibujada con la señal real de reproducción. [Referencia del mini de audio](https://spotifast.rocks/winamp/)

Pulsa **Sacar a una ventana** o minimiza nabnab con el mini abierto. La ventana flotante se queda sobre el escritorio y puedes arrastrarla desde el título de la pista. Al volver a nabnab, el mismo WebView2 conserva el archivo, la posición y la cola.

En Windows, pulsa **Ctrl + Mayús + M** para abrir el mini de audio o vídeo con un archivo cargado, aunque nabnab esté minimizado. Si el mini ya está flotando, el atajo le da el foco. Con **Ajustes → General → Mostrar icono en la bandeja** activado, también puedes elegir **Abrir minirreproductor** en el menú del icono junto al reloj.

Con una pista de audio en la sesión, mantén **Alt** y gira la rueda. En Windows funciona también con nabnab minimizado y otra aplicación en primer plano. Las portadas aparecen junto al puntero y giran en profundidad. Suelta **Alt** para reproducir la tarjeta central o pulsa **Esc** para cancelar. Puedes pulsar una tarjeta, usar **↑**, **↓** e **Intro**, o abrir **Salto rápido** desde el mini de audio. **Volver a la pista** deshace el último salto dentro de la misma cola. [Referencia de la rueda](https://github.com/ffoouuaadd/skip-dat)

Usa **↑** y **↓** para subir o bajar el volumen cinco puntos en vídeo o música. En la barra y el mini, el atajo funciona cuando el foco está en el reproductor. Los controles de formulario mantienen su manejo habitual con las flechas.

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

En **Opciones de reproducción → Pista de audio** aparecen el nombre y el idioma declarados por el archivo. Se usa el selector del motor cuando está disponible. En MP4 y M4V locales no fragmentados, nabnab también puede seleccionar audio cuando WebView2 no ofrece ese selector. La reproducción usa el mismo elemento y los datos del original. Al cambiar de pista hay una breve recarga y se recuperan la posición, la velocidad, el volumen y el estado de pausa. Los códecs disponibles dependen de WebView2. [Estructura de las pistas de un archivo MP4](https://developer.apple.com/documentation/quicktime-file-format/track_atom)

Los subtítulos SRT y VTT locales se renderizan como texto, con ajuste de tamaño, contraste y desfase.

En Windows, **Subtítulos → Cargar ASS o SSA con estilos** abre un archivo local en UTF-8 de hasta 2 MiB. Al abrir el panel en un MKV, el grupo **Integradas · con estilos** muestra las pistas ASS/SSA con el título e idioma que declare el archivo. Al elegir una, FFmpeg copia el subtítulo a memoria con el mismo límite de 2 MiB. Las otras pistas integradas dependen de lo que exponga WebView2. [Selección de pistas en FFmpeg](https://github.com/FFmpeg/FFmpeg/blob/master/doc/ffmpeg.texi)

El libass incluido dibuja posiciones, colores, capas y animaciones sobre el vídeo. El tamaño y el desfase se ajustan en el mismo panel. Las líneas de diálogo pueden subir al mostrar los controles, mientras las posiciones explícitas conservan el encuadre del autor. Se usan las fuentes disponibles en el equipo. Al salir del reproductor se cancela la lectura pendiente y se libera la pista. [Interfaz de libass](https://raw.githubusercontent.com/libass/libass/0.17.4/libass/ass.h)

El renderizado usa un hilo dedicado, un lienzo de hasta 480.000 píxeles y una solicitud de imagen a la vez desde el reproductor. El puente transmite el rectángulo de subtítulos solo cuando cambia. Los fotogramas se mantienen en memoria durante la vista activa.

Al detener el puntero sobre el progreso, FFmpeg extrae un fotograma del vídeo en el equipo. La petición anterior se cancela al mover el puntero o cambiar de archivo. La caché conserva hasta 28 JPEG de 48 KiB como máximo por fotograma, en memoria durante la sesión. El tiempo se muestra aunque el archivo tarde en producir una miniatura.

## Música inmersiva

Pulsa **Abrir visualización** o la portada en la barra compacta. El transporte queda centrado debajo de la escena, con el tiempo transcurrido y la duración a ambos lados de la línea de progreso. **Escenas** muestra 13 visualizaciones con miniaturas, favoritas y controles de rotación. El tratamiento sobrio es el inicial. Puedes elegir Color, ajustar la intensidad o seleccionar Imagen fija. La rotación automática se puede activar con un intervalo inicial de 45 segundos. El fundido gráfico dura 3,6 segundos y conserva el audio.

La vista de música conecta el elemento de la sesión a un solo `AudioContext`. La forma de onda, la energía por bandas y los ataques se leen antes de la ganancia de salida. Volver a la biblioteca mantiene la misma pista y detiene el dibujo. Se usa también el estado nativo de la ventana para detectar minimización. Un fallo gráfico conserva el audio y muestra la imagen local de la escena.

Las escenas usan un shader propio en `src/music-scenes.glsl`, con una función por escena y dos texturas para realimentar el fotograma anterior. El orden de las funciones debe corresponder a la tabla `scenes` de `src/music-visualizer.ts`. Las miniaturas de 384 × 216 se generan con el mismo renderizador en WebView2 mediante `NABNAB_RENDER_SCENES=1` al ejecutar `tests/native-playback.browser.mjs`. El script escribe los WebP en `artifacts/playback/previews`. Revisa los dos tratamientos antes de copiarlos a `public/playback`.

Las escenas y sus miniaturas son recursos propios. **Moiré** cruza dos ondas de interferencia. **Nácar** pliega bandas de luz con los graves y perfila los bordes con los agudos. **Vectorial** recorre un túnel geométrico que acelera con el ritmo. **Fósforo** dibuja dos trazas a partir de la forma de onda y ajusta su pantalla CRT al lienzo. **Casete** usa la energía de los graves y el volumen para mover los carretes y los surcos que llenan la escena. Las estelas y la lectura de bandas toman como referencia la guía de autoría de MilkDrop. El tratamiento de las líneas de barrido de Fósforo sigue la documentación de Libretro. [Autoría de MilkDrop](https://www.geisswerks.com/milkdrop/milkdrop_preset_authoring.html) · [Shaders CRT de Libretro](https://docs.libretro.com/shader/crt/)

## Verificación

La prueba `tests/native-playback.browser.mjs` genera las muestras con FFmpeg y, cuando está disponible, WebCodecs para AV1. Abre la aplicación con un perfil aislado y comprueba vídeo, subtítulos, búsquedas rápidas en pausa, cambio de archivo durante la carga, pantalla completa y reanudación tras reiniciar. Para los ASS/SSA externos, entrega una ruta de prueba en lugar de abrir el diálogo del sistema. Con un MKV verifica títulos e idiomas, rechaza seleccionar el flujo de vídeo como subtítulo y alterna dos pistas. Las pruebas de ambos orígenes usan el renderizador real y comprueban colores, capas, movimiento y conservación del elemento de vídeo. En música verifica tres cambios entre vistas con el mismo elemento, etiquetas ausentes, respuesta a ritmos distintos, silencio real, señal antes del volumen maestro, minimización y pérdida y recuperación del contexto gráfico. El reproductor y las escenas se probaron bloqueando peticiones externas del WebView. El codificador Media Foundation de HEVC no estaba disponible en el equipo de pruebas.

Con `NABNAB_PLAYBACK_LIVE_DOWNLOAD=1`, la misma prueba descarga una muestra pública a través de la cola nativa mientras suena un archivo local. Con la pila normal de Windows se guardaron 1.128.375 bytes y la pista avanzó 1,74 segundos entre el inicio de la transferencia y su final, sin cambiar de archivo ni pasar a pausa. [Muestra de vídeo de MDN](https://developer.mozilla.org/shared-assets/videos/flower.mp4)

Con `NABNAB_PLAYBACK_DESKTOP_GESTURE=1`, la prueba usa la rueda y el teclado de Windows en una ventana de prueba independiente. Comprueba el foco, la selección al soltar Alt, la cancelación con Esc y que el gesto deja de abrir la rueda al detener la sesión.

Con `NABNAB_PLAYBACK_DESKTOP_TRAY=1`, abre el menú nativo de bandeja con audio y vídeo en reproducción. Comprueba que el mini aparece con la ventana principal minimizada, que conserva el mismo elemento multimedia y que el menú desactiva la acción cuando se cierra la sesión.

Con `NABNAB_PLAYBACK_DESKTOP_MINI_KEY=1` y la bandeja desactivada, envía **Ctrl + Mayús + M** desde Windows y comprueba el mismo recorrido para audio y vídeo.

Con `NABNAB_PLAYBACK_WINDOWS_MEDIA=1`, la prueba busca la sesión multimedia de los archivos de prueba y verifica la carátula, la pausa, la reanudación, la búsqueda y los saltos de pista con la ventana minimizada. [Controles multimedia de Windows](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssession)

En Windows, compilación 26200, con Intel Iris Xe y ANGLE/D3D11, se observaron 60 FPS en Automática y 30 FPS en Ahorro, en la escena Órbita a 900 × 620 px CSS. Se midieron los fotogramas dibujados durante 2,3 segundos en cada modo, con la música activa y el selector de escenas abierto. Ahorro usó un lienzo interno de 610 × 420 px. Las cifras corresponden a ese equipo y esas condiciones.

El frontend de producción ocupa 2.221.082 bytes, 7.602 menos que `e191ff3` con el mismo compilador. Las 16 vistas previas bajaron de 65.850 a 55.240 bytes. El código de escenas se carga al abrir la vista musical. Se mantuvieron WebView2 y FFmpeg como motores existentes. La medida corresponde al frontend compilado. El instalador requiere su propia medición.

Las pruebas de navegador de audio y biblioteca cubren las listas, los capítulos, los marcadores y la búsqueda de subtítulos.

[Vídeo con controles](playback/video-controls.png) · [Pistas de audio](playback/video-audio-tracks.png) · [Vídeo con los controles ocultos](playback/video-clean.png) · [Subtítulos ASS externos](playback/video-ass.png) · [Subtítulos integrados en MKV](playback/video-embedded.png) · [Previsualización al buscar](playback/video-seek.png) · [Música a 900 px](playback/music.png) · [Fósforo](playback/phosphor-full.png) · [Selector de escenas](playback/scenes.png) · [Barra compacta a 900 px](playback/compact.png)

[Salto rápido sobre el escritorio](playback/quick-skip-desktop.png) · [Mini flotante de audio](playback/mini-audio.png) · [Mini flotante de vídeo](playback/mini-video.png)
