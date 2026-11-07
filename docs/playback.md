# Reproducción local

Biblioteca, descargas y marcadores usan una sesión con un elemento multimedia estable. La aplicación sirve los archivos autorizados por la ruta local `nabmedia`, que admite lectura por rangos sin copiar el archivo completo a memoria. El motor es el WebView2 instalado en Windows.

El vídeo abre una vista de borde a borde con ajuste de proporción, subtítulos locales, velocidad, volumen y pantalla completa. Los controles se ocultan durante la reproducción y vuelven con el puntero o el teclado. Volver de esta vista pausa el vídeo y conserva su posición en la barra compacta. La cola y las preferencias se restauran en pausa al reiniciar. Las posiciones inferiores a tres segundos o dentro del tramo final del archivo no se ofrecen para reanudar.

| Muestra verificada en Windows | Resultado |
| --- | --- |
| MP4 vertical, H.264 y AAC, 360 × 640 | Vídeo, búsqueda temporal y reanudación |
| WebM ultrapanorámico, VP9 y Opus, 720 × 248 | Vídeo |
| MKV ultrapanorámico, VP9 y Opus, 720 × 248 | Vídeo en el WebView2 probado |
| MP3 con portada integrada | Audio clasificado correctamente y reproducido en la misma sesión |
| MP4 dañado | Error recuperable con conversión explícita o apertura en Windows |

La matriz corresponde a archivos reales, no a todas las combinaciones que admiten esos contenedores. Las pistas integradas solo aparecen si WebView2 las expone. La vista previa de búsqueda muestra el tiempo. Los subtítulos SRT y VTT locales se renderizan como texto, con ajuste de tamaño, contraste y desfase.

La prueba `tests/native-playback.browser.mjs` genera sus muestras con los motores locales y abre la aplicación con un perfil aislado. Comprueba la composición de vídeo, los subtítulos, el teclado, la pantalla completa, la continuidad de la sesión y la reanudación tras reiniciar. Las pruebas `tests/audio-library.browser.mjs` y `tests/library-productivity.browser.mjs` cubren también las listas, los capítulos, los marcadores y la búsqueda de subtítulos.

[Vídeo con controles](playback/video-controls.png) · [Vídeo con los controles ocultos](playback/video-clean.png) · [Reproductor compacto a 900 px](playback/compact.png)
