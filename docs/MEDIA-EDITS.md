# Unir clips y sustituir audio

Estas operaciones complementan convertir, recortar y comprimir. Utilizan el mismo componente FFmpeg y la cola común. Todos los resultados son archivos nuevos.

| Operación | Resultado | Comportamiento |
| --- | --- | --- |
| Unir copiando streams | MKV | Conserva vídeo y audio sin recodificar. Comprueba códecs, resolución, FPS, bases de tiempo, disposición de audio y datos de inicialización. Si los clips no coinciden, solicita recodificación. |
| Unir recodificando | MKV con H.264/AAC | Normaliza las dimensiones y FPS elegidos. Conserva proporciones mediante márgenes. Convierte audio a estéreo de 48 kHz y genera silencio cuando un clip carece de audio. |
| Sustituir audio | MKV | Copia el stream de vídeo y codifica la fuente de audio elegida como AAC. Completa con silencio el audio corto y recorta el largo a la duración del vídeo. |
| Quemar subtítulos | MP4 con H.264/AAC | Utiliza una pista local y el filtro `subtitles`. Requiere un componente FFmpeg con libass, incluido en la receta actual del motor. La interfaz comprueba el filtro real y deshabilita la opción si falta. |

Se pueden unir entre 2 y 20 clips en el orden elegido. La normalización permite dimensiones pares de 16 a 8192 píxeles y de 1 a 120 FPS. Estas opciones se muestran antes de preparar la tarea; recodificar puede cambiar la calidad y el tamaño.

El módulo solo admite archivos locales ya importados. Las listas de unión las escribe nabnab a partir de rutas autorizadas. No acepta manifiestos de red, filtros arbitrarios ni comandos introducidos por el usuario. Las rutas para filtros y listas se escapan para el parser de FFmpeg, sin pasar por una shell.

La verificación comprueba pistas y duración del resultado antes de devolverlo a publicación. La prueba opcional con el motor real genera clips y audio sintéticos, une streams compatibles, normaliza clips distintos y sustituye audio corto. Comprueba las dimensiones, FPS y audio de la unión normalizada, así como la conservación de todos los originales. Se ejecutó también la incrustación real de una pista sintética con el motor libass actualizado.
