# Escuchar una muestra de procesamiento

Normalizar sonoridad, eliminar silencios y reducir ruido permiten preparar una muestra de los primeros 20 segundos del primer archivo seleccionado. Si el archivo es más corto, se utiliza únicamente su duración disponible.

La muestra es una tarea real de la cola. FFmpeg extrae el fragmento a un temporal local y las herramientas existentes aplican la operación y el perfil elegidos. El resultado es una copia M4A/AAC cuyo nombre incluye `preview-first-20s`. Aparece en la biblioteca y se puede escuchar con el reproductor existente. El archivo original no se modifica.

Una muestra permite valorar cortes de voz o ruido antes de procesar el archivo completo. La normalización mide solo ese fragmento, por lo que su resultado puede diferir del procesamiento de toda la grabación. La eliminación de silencios puede producir una muestra de menos de 20 segundos. Si no queda audio válido, se devuelve un error en lugar de publicar un archivo vacío.

No se añaden componentes ni dependencias. La prueba opcional con los motores reales genera audio sintético de 30 segundos, procesa las tres operaciones, comprueba la duración de la muestra y verifica que la entrada permanece intacta.
