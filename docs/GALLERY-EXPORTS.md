# Exportar galerías guardadas

Desde la selección de Biblioteca se pueden preparar dos exportaciones en la cola común. Los archivos originales se conservan.

La galería HTML contiene los archivos dentro del propio HTML mediante URLs de datos. Se puede mover sin una carpeta de recursos ni conexión. Incluye nombre, orden, autor e identificador del contenido cuando la biblioteca dispone de esos datos. Los enlaces de atribución muestran únicamente el origen público guardado, sin credenciales, rutas privadas, consultas ni URLs firmadas. No se inventa el enlace original de una publicación si la biblioteca no lo conserva.

El HTML no contiene scripts, fuentes externas, hojas de estilo remotas ni recursos remotos. Su política de contenido permite solamente imágenes y medios embebidos y el estilo local. Los enlaces de origen requieren una acción de quien lee la galería.

La hoja de contacto genera una imagen PNG con miniaturas de 240 × 180 píxeles, conservando proporciones dentro de cada celda. Usa FFmpeg y el primer fotograma de una imagen animada. No altera ni convierte los originales. Para exportar una colección con vídeo o audio, utiliza la galería HTML.

| Exportación | Entradas | Límites |
| --- | --- | --- |
| HTML | JPEG, PNG, GIF, WebP, BMP, MP4, WebM, MP3, M4A, Ogg/Opus y WAV | Hasta 200 archivos, 16 MiB por archivo y 128 MiB de contenido total antes de codificar. El HTML ocupa aproximadamente un tercio más que el contenido embebido. |
| Hoja de contacto | JPEG, PNG, GIF, WebP y BMP | Hasta 64 imágenes; mismos límites de entrada. Animaciones representadas por su primer fotograma. |

Un formato no admitido o un límite excedido devuelve un error antes de publicar. Las exportaciones se guardan como archivos nuevos en la biblioteca. La codificación HTML se escribe en bloques y comprueba cancelación; no mantiene toda la colección en memoria.

No se añaden dependencias. Las pruebas unitarias incluyen valores literales de Base64, escape de atribuciones y eliminación de credenciales. La prueba opcional con FFmpeg crea HTML y una hoja de contacto a partir de copias de los iconos del proyecto y comprueba dimensiones y conservación de entradas. Debe ejecutarse tras compilar el módulo; no sustituye una comprobación visual del archivo exportado.
