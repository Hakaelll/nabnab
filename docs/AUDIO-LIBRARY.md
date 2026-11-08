# Biblioteca de audio

En **Biblioteca → Audio** puedes alternar entre Música, Podcasts, Audiolibros y Listas. En Música, elige una tarjeta de **Álbumes** para ver sus pistas, ordenadas por disco y número. La estantería usa portadas locales o incrustadas cuando están disponibles. También puedes filtrar por artista, álbum o género. Si un archivo antiguo aparece sin etiquetas, selecciónalo y usa **Leer etiquetas pendientes**.

El minirreproductor permanece visible al cambiar de sección. La cola **A continuación** es independiente de Descargas. En música, puedes activar el modo aleatorio y alternar entre repetir toda la cola o repetir una pista. Los controles multimedia de Windows muestran el título, el artista y la carátula disponible. Utilizan la misma cola aunque la ventana principal esté minimizada.

En **Ajustes → Sesiones y privacidad → Caché de miniaturas** puedes ver cuánto ocupan las carátulas y otras miniaturas generadas. nabnab retira las más antiguas al superar 512 imágenes o 128 MiB. **Vaciar miniaturas…** pide confirmación. Las imágenes se generan de nuevo al necesitarlas.

La **Estantería** de audiolibros agrupa capítulos de LibriVox y reconoce archivos M4B. **Continuar** recupera el punto guardado del capítulo o abre el siguiente si el anterior llegó al final. La cola de capítulos se limita al libro que estás escuchando. El minirreproductor ofrece velocidad, retroceso de 15 segundos y temporizador por minutos o al final del capítulo. Los marcadores se guardan en Biblioteca. Si hay capítulos incrustados, se puede saltar a ellos.

Selecciona pistas para crear una lista, añadirlas a una existente o editar sus etiquetas. Puedes cambiar el orden y quitar entradas desde **Listas → Editar lista**. La previsualización de etiquetas muestra los campos que cambiarán. **Guardar etiquetas** actualiza la base de nabnab. **Crear copias con etiquetas** usa la cola para generar archivos nuevos y compara la huella de los flujos de audio antes de incorporarlos a Biblioteca. Se probaron MP3, M4A, M4B, FLAC, OGG y Opus con los motores de producción. Los originales se conservan.

Desde la pista actual, **Elegir portada** permite asociar un JPEG, PNG o WebP local. Quitar la asociación vuelve a mostrar la portada incrustada cuando está disponible.

**Letras locales** lee el archivo LRC junto a la pista si está disponible. **Asociar archivo LRC** permite elegir otro archivo local. Cada línea permite saltar a su marca de tiempo.

En **Listas → Importar lista** puedes abrir M3U, M3U8, PLS o CSV en UTF-8. En CSV, confirma las columnas y la unidad de duración. nabnab reconoce identificadores y rutas de archivos que ya están en Biblioteca. Las coincidencias por título, artista, duración o versión son sugerencias. Puedes escoger la grabación o conservar la referencia remota o ausente. La lista conserva el orden y las URLs importadas permanecen como referencias.

La prueba de interfaz reproduce un WAV silencioso en Edge con almacenamiento de escritorio simulado. Comprueba el orden por disco y pista, la estantería con teclado, la continuidad del reproductor, el descanso de audiolibros, las letras y la edición de listas y etiquetas. La prueba nativa usa audio sintético y verifica que, después de reiniciar nabnab, **Continuar** abre el capítulo siguiente de un libro cuyo capítulo anterior terminó. La integración con Windows se comprueba mediante solicitudes del sistema a la sesión multimedia de prueba.

![Biblioteca de audio y minirreproductor persistente con datos de prueba](audio-library-preview.png)

![Estantería de audiolibros y continuación en el capítulo siguiente con datos de prueba](audio-bookshelf.png)

![Revisión de una lista importada y elección de una grabación local](audio-playlist-matching.png)

![Previsualización de etiquetas y creación explícita de copias](audio-tags-preview.png)
